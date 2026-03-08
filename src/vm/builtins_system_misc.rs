use super::{
    BigInt, BuiltinFunction, HashMap, InstanceObject, InternalCallOutcome, IpAddr, IteratorKind,
    IteratorObject, ObjRef, Object, Read, RuntimeError, SIGNAL_DEFAULT, SIGNAL_IGNORE,
    SIGNAL_SIGINT, SocketAddr, TimeParts, ToSocketAddrs, Value, Vm, apply_uuid_variant,
    apply_uuid_version, bytes_like_from_value, day_of_year, days_from_civil, decode_text_bytes,
    dict_get_value, format_strftime, format_uuid_hex, format_uuid_hyphenated, is_truthy,
    parse_uuid_like_string, runtime_error_matches_exception, split_unix_timestamp,
    unix_time_now_duration, uuid_hash_mix_bytes, uuid_random_bytes,
    uuid_timestamp_100ns_since_gregorian, value_from_bigint, value_to_f64, value_to_int,
};
use std::rc::Rc;

const DATETIME_MIN_YEAR: i64 = 1;
const DATETIME_MAX_YEAR: i64 = 9999;
const DATETIME_MAX_DELTA_DAYS: i64 = 999_999_999;
const DATETIME_SECONDS_PER_DAY: i64 = 86_400;
const DATETIME_MICROSECONDS_PER_SECOND: i64 = 1_000_000;
const DATETIME_MICROSECONDS_PER_DAY: i64 =
    DATETIME_SECONDS_PER_DAY * DATETIME_MICROSECONDS_PER_SECOND;
const UNIX_EPOCH_ORDINAL: i64 = 719_163;

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}

fn parse_ascii_i64_component(text: &str) -> Option<i64> {
    if text.is_empty() || !text.as_bytes().iter().all(u8::is_ascii_digit) {
        return None;
    }
    text.parse::<i64>().ok()
}

fn escaped_single_quoted(input: &str) -> String {
    input.replace('\\', "\\\\").replace('\'', "\\'")
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> Option<u32> {
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => return None,
    })
}

impl Vm {
    fn uuid_node_from_host(&self) -> i64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let host_name = self
            .host
            .env_var("HOSTNAME")
            .unwrap_or_else(|| "localhost".to_string());
        std::hash::Hash::hash(&host_name, &mut hasher);
        let mut node = std::hash::Hasher::finish(&hasher) & 0x0000_FFFF_FFFF_FFFF;
        node |= 0x0000_0100_0000_0000;
        node as i64
    }

    fn warnings_is_internal_traceback_file(filename: &str) -> bool {
        filename.ends_with("_py_warnings.py")
    }

    fn warnings_infer_call_span_end_column(
        &mut self,
        filename: &str,
        line: usize,
        column: usize,
    ) -> Option<usize> {
        if line == 0 || column == 0 {
            return None;
        }
        let source_line = self.traceback_source_line(filename, line)?;
        let chars = source_line.chars().collect::<Vec<_>>();
        let start = column.saturating_sub(1);
        if start >= chars.len() {
            return None;
        }
        let segment = chars[start..].iter().collect::<String>();
        let segment = segment.trim_end();
        if segment.contains('#') || !segment.ends_with(')') {
            return None;
        }
        let first = segment.chars().next()?;
        if !(first == '_' || first.is_ascii_alphabetic()) {
            return None;
        }
        let open = segment.find('(')?;
        if open == 0 {
            return None;
        }
        Some(
            start
                .saturating_add(segment.chars().count())
                .saturating_add(1),
        )
    }

    fn warnings_normalize_active_exception_traceback(&mut self) {
        let Some(frame_index) = (0..self.frames.len())
            .rev()
            .find(|idx| self.frames[*idx].active_exception.is_some())
        else {
            return;
        };
        let Some(mut active) = self
            .frames
            .get_mut(frame_index)
            .and_then(|frame| frame.active_exception.take())
        else {
            return;
        };
        if let Value::Exception(exception) = &mut active {
            Rc::make_mut(&mut exception.traceback_frames)
                .retain(|frame| !Self::warnings_is_internal_traceback_file(&frame.filename));
            for idx in 0..exception.traceback_frames.len() {
                let (filename, line, column, end_line, end_column) = {
                    let frame = &exception.traceback_frames[idx];
                    (
                        frame.filename.clone(),
                        frame.line,
                        frame.column,
                        frame.end_line,
                        frame.end_column,
                    )
                };
                if end_line == 0
                    && end_column == 0
                    && let Some(inferred_end) =
                        self.warnings_infer_call_span_end_column(&filename, line, column)
                    && let Some(frame) = Rc::make_mut(&mut exception.traceback_frames).get_mut(idx)
                {
                    frame.end_line = line;
                    frame.end_column = inferred_end;
                }
            }
        }
        if let Some(frame) = self.frames.get_mut(frame_index) {
            frame.active_exception = Some(active);
        }
    }

    fn warnings_normalize_runtime_error_traceback(&mut self, err: &mut RuntimeError) {
        let Some(exception) = err.exception.as_mut() else {
            return;
        };
        Rc::make_mut(&mut exception.traceback_frames)
            .retain(|frame| !Self::warnings_is_internal_traceback_file(&frame.filename));
        for idx in 0..exception.traceback_frames.len() {
            let (filename, line, column, end_line, end_column) = {
                let frame = &exception.traceback_frames[idx];
                (
                    frame.filename.clone(),
                    frame.line,
                    frame.column,
                    frame.end_line,
                    frame.end_column,
                )
            };
            if end_line == 0
                && end_column == 0
                && let Some(inferred_end) =
                    self.warnings_infer_call_span_end_column(&filename, line, column)
                && let Some(frame) = Rc::make_mut(&mut exception.traceback_frames).get_mut(idx)
            {
                frame.end_line = line;
                frame.end_column = inferred_end;
            }
        }
        err.message =
            self.format_traceback(&[], &Value::Exception(Box::new((**exception).clone())));
    }

    pub(super) fn builtin_threading_excepthook(
        &mut self,
        _args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "excepthook() got unexpected keyword arguments",
            ));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_struct_calcsize(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("calcsize() expects one argument"));
        }
        let format = self.struct_format_from_value(args.remove(0), "calcsize")?;
        let spec = self.parse_struct_format(&format)?;
        Ok(Value::Int(spec.size as i64))
    }

    pub(super) fn builtin_struct_pack(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() {
            return Err(RuntimeError::new("pack() expects format string"));
        }
        let format = self.struct_format_from_value(args.remove(0), "pack")?;
        let spec = self.parse_struct_format(&format)?;
        let packed = self.struct_pack_format_values(&spec, &args)?;
        Ok(self.heap.alloc_bytes(packed))
    }

    pub(super) fn builtin_struct_unpack(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("unpack() expects format and buffer"));
        }
        let format = self.struct_format_from_value(args.remove(0), "unpack")?;
        let spec = self.parse_struct_format(&format)?;
        let buffer = bytes_like_from_value(args.remove(0))?;
        let values = self.struct_unpack_format_bytes(&spec, &buffer)?;
        Ok(self.heap.alloc_tuple(values))
    }

    pub(super) fn builtin_struct_iter_unpack(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("iter_unpack() expects format and buffer"));
        }
        let format = self.struct_format_from_value(args.remove(0), "iter_unpack")?;
        let spec = self.parse_struct_format(&format)?;
        if spec.size == 0 {
            return Err(RuntimeError::new(
                "iter_unpack() requires a non-empty format",
            ));
        }
        let buffer = bytes_like_from_value(args.remove(0))?;
        if buffer.len() % spec.size != 0 {
            return Err(RuntimeError::new(
                "iter_unpack() buffer size must be a multiple of format size",
            ));
        }
        let mut rows = Vec::new();
        let mut pos = 0usize;
        while pos < buffer.len() {
            let chunk = &buffer[pos..pos + spec.size];
            let unpacked = self.struct_unpack_format_bytes(&spec, chunk)?;
            rows.push(self.heap.alloc_tuple(unpacked));
            pos += spec.size;
        }
        let list = match self.heap.alloc_list(rows) {
            Value::List(obj) => obj,
            _ => unreachable!(),
        };
        Ok(Value::Iterator(self.heap.alloc(Object::Iterator(
            IteratorObject {
                kind: IteratorKind::List(list),
                index: 0,
            },
        ))))
    }

    pub(super) fn builtin_struct_pack_into(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 3 {
            return Err(RuntimeError::new(
                "pack_into() expects format, buffer, offset, and values",
            ));
        }
        let format = self.struct_format_from_value(args.remove(0), "pack_into")?;
        let spec = self.parse_struct_format(&format)?;
        let target = args.remove(0);
        let offset = value_to_int(args.remove(0))?;
        let packed = self.struct_pack_format_values(&spec, &args)?;
        match target {
            Value::ByteArray(obj) => {
                let Object::ByteArray(values) = &mut *obj.kind_mut() else {
                    return Err(RuntimeError::new("pack_into() requires writable buffer"));
                };
                let start = self.struct_normalize_offset(offset, values.len(), packed.len())?;
                values[start..start + packed.len()].copy_from_slice(&packed);
                Ok(Value::None)
            }
            Value::MemoryView(view_obj) => {
                let source = match &*view_obj.kind() {
                    Object::MemoryView(view) => view.source.clone(),
                    _ => return Err(RuntimeError::new("pack_into() requires writable buffer")),
                };
                let Object::ByteArray(values) = &mut *source.kind_mut() else {
                    return Err(RuntimeError::new("pack_into() requires writable buffer"));
                };
                let start = self.struct_normalize_offset(offset, values.len(), packed.len())?;
                values[start..start + packed.len()].copy_from_slice(&packed);
                Ok(Value::None)
            }
            _ => Err(RuntimeError::new("pack_into() requires writable buffer")),
        }
    }

    pub(super) fn builtin_struct_unpack_from(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "unpack_from() expects format, buffer, and optional offset",
            ));
        }
        let format = self.struct_format_from_value(args.remove(0), "unpack_from")?;
        let spec = self.parse_struct_format(&format)?;
        let buffer = bytes_like_from_value(args.remove(0))?;
        let offset = if args.is_empty() {
            0
        } else {
            value_to_int(args.remove(0))?
        };
        let start = self.struct_normalize_offset(offset, buffer.len(), spec.size)?;
        let values = self.struct_unpack_format_bytes(&spec, &buffer[start..start + spec.size])?;
        Ok(self.heap.alloc_tuple(values))
    }

    pub(super) fn struct_format_from_receiver(
        &self,
        receiver: &ObjRef,
    ) -> Result<String, RuntimeError> {
        let Object::Instance(instance_data) = &*receiver.kind() else {
            return Err(RuntimeError::new("Struct method expects struct instance"));
        };
        match instance_data.attrs.get("format") {
            Some(Value::Str(format)) => Ok(format.clone()),
            _ => Err(RuntimeError::new("Struct instance is missing format")),
        }
    }

    pub(super) fn struct_format_from_value(
        &self,
        value: Value,
        func_name: &str,
    ) -> Result<String, RuntimeError> {
        match value {
            Value::Str(text) => Ok(text),
            other => {
                let bytes = bytes_like_from_value(other)
                    .map_err(|_| RuntimeError::new(format!("{func_name}() format must be str")))?;
                String::from_utf8(bytes)
                    .map_err(|_| RuntimeError::new(format!("{func_name}() format must be str")))
            }
        }
    }

    pub(super) fn forward_struct_method(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        target: BuiltinFunction,
        method_name: &str,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(format!(
                "Struct.{method_name}() missing receiver"
            )));
        }
        let receiver = self.receiver_from_value(&args[0])?;
        let format = self.struct_format_from_receiver(&receiver)?;
        args.remove(0);
        let mut forwarded = Vec::with_capacity(args.len() + 1);
        forwarded.push(Value::Str(format));
        forwarded.extend(args);
        self.call_builtin(target, forwarded, kwargs)
    }

    pub(super) fn builtin_struct_class_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("Struct() takes no keyword arguments"));
        }
        if args.len() != 2 {
            return Err(RuntimeError::new("Struct() expects format string"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let format = self.struct_format_from_value(args.remove(0), "Struct")?;
        let size = self
            .call_builtin(
                BuiltinFunction::StructCalcSize,
                vec![Value::Str(format.clone())],
                HashMap::new(),
            )
            .unwrap_or(Value::Int(0));
        let Object::Instance(instance_data) = &mut *receiver.kind_mut() else {
            return Err(RuntimeError::new("Struct() expects instance receiver"));
        };
        instance_data
            .attrs
            .insert("format".to_string(), Value::Str(format));
        instance_data.attrs.insert("size".to_string(), size);
        Ok(Value::None)
    }

    pub(super) fn builtin_struct_class_pack(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.forward_struct_method(args, kwargs, BuiltinFunction::StructPack, "pack")
    }

    pub(super) fn builtin_struct_class_unpack(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.forward_struct_method(args, kwargs, BuiltinFunction::StructUnpack, "unpack")
    }

    pub(super) fn builtin_struct_class_iter_unpack(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.forward_struct_method(
            args,
            kwargs,
            BuiltinFunction::StructIterUnpack,
            "iter_unpack",
        )
    }

    pub(super) fn builtin_struct_class_pack_into(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.forward_struct_method(args, kwargs, BuiltinFunction::StructPackInto, "pack_into")
    }

    pub(super) fn builtin_struct_class_unpack_from(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.forward_struct_method(
            args,
            kwargs,
            BuiltinFunction::StructUnpackFrom,
            "unpack_from",
        )
    }

    pub(super) fn builtin_datetime_now(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("now() expects no arguments"));
        }
        let class = if !args.is_empty() {
            self.receiver_from_value(&args.remove(0))?
        } else {
            self.datetime_default_class()?
        };
        let now = unix_time_now_duration()?;
        let parts = split_unix_timestamp(now.as_secs() as i64);
        self.datetime_instance_from_parts(
            class,
            parts.year as i64,
            parts.month,
            parts.day,
            parts.hour,
            parts.minute,
            parts.second,
            now.subsec_micros() as i64,
            None,
        )
    }

    pub(super) fn builtin_datetime_today(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("today() expects no arguments"));
        }
        let class = if !args.is_empty() {
            self.receiver_from_value(&args.remove(0))?
        } else {
            self.date_default_class()?
        };
        let now = unix_time_now_duration()?;
        let parts = split_unix_timestamp(now.as_secs() as i64);
        let is_datetime_class = matches!(
            &*class.kind(),
            Object::Class(class_data) if class_data.name == "datetime"
        );
        if is_datetime_class {
            self.datetime_instance_from_parts(
                class,
                parts.year as i64,
                parts.month,
                parts.day,
                parts.hour,
                parts.minute,
                parts.second,
                now.subsec_micros() as i64,
                None,
            )
        } else {
            self.date_instance_from_parts(class, parts.year as i64, parts.month, parts.day)
        }
    }

    fn datetime_timezone_offset_seconds(&self, tzinfo: &Value) -> Result<i64, RuntimeError> {
        match tzinfo {
            Value::None => Ok(0),
            Value::Instance(instance) => {
                let Object::Instance(instance_data) = &*instance.kind() else {
                    return Ok(0);
                };
                let Some(offset) = instance_data.attrs.get("offset") else {
                    return Ok(0);
                };
                match offset {
                    Value::Int(_) | Value::BigInt(_) | Value::Bool(_) => {
                        value_to_int(offset.clone())
                    }
                    Value::Instance(delta) => {
                        let Object::Instance(delta_data) = &*delta.kind() else {
                            return Ok(0);
                        };
                        let days = delta_data
                            .attrs
                            .get("days")
                            .cloned()
                            .map(value_to_int)
                            .transpose()?
                            .unwrap_or(0);
                        let seconds = delta_data
                            .attrs
                            .get("seconds")
                            .cloned()
                            .map(value_to_int)
                            .transpose()?
                            .unwrap_or(0);
                        let microseconds = delta_data
                            .attrs
                            .get("microseconds")
                            .cloned()
                            .map(value_to_int)
                            .transpose()?
                            .unwrap_or(0);
                        Ok(days * 86_400 + seconds + microseconds / 1_000_000)
                    }
                    _ => Ok(0),
                }
            }
            _ => Err(RuntimeError::new(
                "tz argument must be an instance of tzinfo",
            )),
        }
    }

    fn datetime_instance_from_parts(
        &mut self,
        class: ObjRef,
        year: i64,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        microsecond: i64,
        tzinfo: Option<Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.alloc_instance_for_class(&class);
        {
            let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                return Err(RuntimeError::new("datetime construction failed"));
            };
            instance_data
                .attrs
                .insert("year".to_string(), Value::Int(year));
            instance_data
                .attrs
                .insert("month".to_string(), Value::Int(month as i64));
            instance_data
                .attrs
                .insert("day".to_string(), Value::Int(day as i64));
            instance_data
                .attrs
                .insert("hour".to_string(), Value::Int(hour as i64));
            instance_data
                .attrs
                .insert("minute".to_string(), Value::Int(minute as i64));
            instance_data
                .attrs
                .insert("second".to_string(), Value::Int(second as i64));
            instance_data
                .attrs
                .insert("microsecond".to_string(), Value::Int(microsecond));
            if let Some(tzinfo) = tzinfo {
                instance_data.attrs.insert("tzinfo".to_string(), tzinfo);
            }
        }
        Ok(Value::Instance(instance))
    }

    fn datetime_default_class(&self) -> Result<ObjRef, RuntimeError> {
        let module = self
            .modules
            .get("datetime")
            .ok_or_else(|| RuntimeError::new("datetime module not initialized"))?;
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("datetime module not initialized"));
        };
        let Some(Value::Class(class)) = module_data.globals.get("datetime") else {
            return Err(RuntimeError::new("datetime.datetime is unavailable"));
        };
        Ok(class.clone())
    }

    fn date_default_class(&self) -> Result<ObjRef, RuntimeError> {
        let module = self
            .modules
            .get("datetime")
            .ok_or_else(|| RuntimeError::new("datetime module not initialized"))?;
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("datetime module not initialized"));
        };
        let Some(Value::Class(class)) = module_data.globals.get("date") else {
            return Err(RuntimeError::new("datetime.date is unavailable"));
        };
        Ok(class.clone())
    }

    fn timedelta_default_class(&self) -> Result<ObjRef, RuntimeError> {
        let module = self
            .modules
            .get("datetime")
            .ok_or_else(|| RuntimeError::new("datetime module not initialized"))?;
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("datetime module not initialized"));
        };
        let Some(Value::Class(class)) = module_data.globals.get("timedelta") else {
            return Err(RuntimeError::new("datetime.timedelta is unavailable"));
        };
        Ok(class.clone())
    }

    fn timedelta_not_implemented(&self) -> Value {
        self.builtins
            .get("NotImplemented")
            .cloned()
            .unwrap_or(Value::None)
    }

    fn timedelta_integer_factor(value: &Value) -> Option<BigInt> {
        match value {
            Value::Int(number) => Some(BigInt::from_i64(*number)),
            Value::Bool(flag) => Some(BigInt::from_i64(if *flag { 1 } else { 0 })),
            Value::BigInt(number) => Some((**number).clone()),
            _ => None,
        }
    }

    fn timedelta_float_ratio(value: f64) -> Result<(BigInt, BigInt), RuntimeError> {
        if value.is_nan() {
            return Err(RuntimeError::value_error(
                "cannot convert NaN to integer ratio",
            ));
        }
        if value.is_infinite() {
            return Err(RuntimeError::overflow_error(
                "cannot convert Infinity to integer ratio",
            ));
        }
        if value == 0.0 {
            return Ok((BigInt::zero(), BigInt::one()));
        }

        let bits = value.to_bits();
        let negative = (bits >> 63) != 0;
        let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
        let fraction = bits & ((1u64 << 52) - 1);

        let mut mantissa = if exponent_bits == 0 {
            fraction
        } else {
            (1u64 << 52) | fraction
        };
        let mut exponent = if exponent_bits == 0 {
            1 - 1023 - 52
        } else {
            exponent_bits - 1023 - 52
        };

        if exponent < 0 {
            let reduce = mantissa.trailing_zeros().min((-exponent) as u32);
            mantissa >>= reduce;
            exponent += reduce as i32;
        }

        let mut numerator = BigInt::from_u64(mantissa);
        if negative {
            numerator = numerator.negated();
        }
        let denominator = if exponent >= 0 {
            numerator = numerator.shl_bits(exponent as usize);
            BigInt::one()
        } else {
            BigInt::one().shl_bits((-exponent) as usize)
        };
        Ok((numerator, denominator))
    }

    fn timedelta_read_parts(
        instance: &ObjRef,
        method_name: &str,
    ) -> Result<(i64, i64, i64), RuntimeError> {
        let read_part = |name: &str| -> Result<i64, RuntimeError> {
            Self::instance_attr_get(instance, name)
                .ok_or_else(|| RuntimeError::new(format!("{method_name}() missing {name}")))
                .and_then(value_to_int)
        };
        Ok((
            read_part("days")?,
            read_part("seconds")?,
            read_part("microseconds")?,
        ))
    }

    fn normalized_timedelta_parts_from_total_microseconds(
        total_microseconds: &BigInt,
    ) -> Result<(i64, i64, i64), RuntimeError> {
        let day_us = BigInt::from_i64(DATETIME_MICROSECONDS_PER_DAY);
        let us_per_second = BigInt::from_i64(DATETIME_MICROSECONDS_PER_SECOND);
        let max_days = BigInt::from_i64(DATETIME_MAX_DELTA_DAYS);
        let min_days = max_days.negated();
        let (days, remainder) = total_microseconds
            .div_mod_floor(&day_us)
            .expect("timedelta day divisor must be non-zero");
        if days.cmp_total(&min_days) == std::cmp::Ordering::Less
            || days.cmp_total(&max_days) == std::cmp::Ordering::Greater
        {
            return Err(RuntimeError::overflow_error(format!(
                "days={days}; must have magnitude <= {DATETIME_MAX_DELTA_DAYS}"
            )));
        }
        let (seconds, microseconds) = remainder
            .div_mod_floor(&us_per_second)
            .expect("timedelta second divisor must be non-zero");
        let days = days
            .to_i64()
            .expect("timedelta day count should fit after range check");
        let seconds = seconds
            .to_i64()
            .expect("timedelta seconds remainder should fit in i64");
        let microseconds = microseconds
            .to_i64()
            .expect("timedelta microseconds remainder should fit in i64");
        Ok((days, seconds, microseconds))
    }

    fn timedelta_apply_normalized_parts(
        instance: &ObjRef,
        days: i64,
        seconds: i64,
        microseconds: i64,
    ) -> Result<(), RuntimeError> {
        Self::instance_attr_set(instance, "days", Value::Int(days))?;
        Self::instance_attr_set(instance, "seconds", Value::Int(seconds))?;
        Self::instance_attr_set(instance, "microseconds", Value::Int(microseconds))?;
        Ok(())
    }

    fn timedelta_total_microseconds_from_instance(
        instance: &ObjRef,
        method_name: &str,
    ) -> Result<BigInt, RuntimeError> {
        let (days, seconds, microseconds) = Self::timedelta_read_parts(instance, method_name)?;
        let day_part = BigInt::from_i64(days).mul(&BigInt::from_i64(DATETIME_MICROSECONDS_PER_DAY));
        let second_part =
            BigInt::from_i64(seconds).mul(&BigInt::from_i64(DATETIME_MICROSECONDS_PER_SECOND));
        Ok(day_part
            .add(&second_part)
            .add(&BigInt::from_i64(microseconds)))
    }

    fn timedelta_instance_arg(&mut self, value: &Value) -> Result<Option<ObjRef>, RuntimeError> {
        let Value::Instance(instance) = value else {
            return Ok(None);
        };
        let timedelta_class = self.timedelta_default_class()?;
        if self.value_is_instance_of(value, &Value::Class(timedelta_class))? {
            Ok(Some(instance.clone()))
        } else {
            Ok(None)
        }
    }

    fn timedelta_instance_from_total_microseconds(
        &mut self,
        total_microseconds: BigInt,
    ) -> Result<Value, RuntimeError> {
        let class = self.timedelta_default_class()?;
        let instance = match self.heap.alloc_instance(InstanceObject::new(class)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        let (days, seconds, microseconds) =
            Self::normalized_timedelta_parts_from_total_microseconds(&total_microseconds)?;
        Self::timedelta_apply_normalized_parts(&instance, days, seconds, microseconds)?;
        Ok(Value::Instance(instance))
    }

    fn timedelta_divide_nearest(
        &self,
        numerator: &BigInt,
        denominator: &BigInt,
    ) -> Result<BigInt, RuntimeError> {
        if denominator.is_zero() {
            return Err(RuntimeError::zero_division_error("division by zero"));
        }

        let numerator_abs = numerator.abs();
        let denominator_abs = denominator.abs();
        let (mut quotient, remainder) = numerator_abs
            .div_mod_floor(&denominator_abs)
            .ok_or_else(|| RuntimeError::zero_division_error("division by zero"))?;
        let doubled_remainder = remainder.mul_small(2);
        match doubled_remainder.cmp_total(&denominator_abs) {
            std::cmp::Ordering::Greater => {
                quotient = quotient.add(&BigInt::one());
            }
            std::cmp::Ordering::Equal if self.bigint_is_odd(&quotient)? => {
                quotient = quotient.add(&BigInt::one());
            }
            _ => {}
        }

        let same_sign = numerator.is_negative() == denominator.is_negative();
        Ok(if same_sign || quotient.is_zero() {
            quotient
        } else {
            quotient.negated()
        })
    }

    fn timedelta_binary_pair_totals(
        &mut self,
        left: &ObjRef,
        right: &Value,
        method_name: &str,
    ) -> Result<Option<(BigInt, BigInt)>, RuntimeError> {
        let Some(right) = self.timedelta_instance_arg(right)? else {
            return Ok(None);
        };
        let left_total = Self::timedelta_total_microseconds_from_instance(left, method_name)?;
        let right_total = Self::timedelta_total_microseconds_from_instance(&right, method_name)?;
        Ok(Some((left_total, right_total)))
    }

    fn timedelta_multiply_total_by_factor(
        &mut self,
        total_microseconds: BigInt,
        factor: &Value,
    ) -> Result<Value, RuntimeError> {
        if let Some(factor) = Self::timedelta_integer_factor(factor) {
            return self
                .timedelta_instance_from_total_microseconds(total_microseconds.mul(&factor));
        }
        if let Value::Float(value) = factor {
            let (numerator, denominator) = Self::timedelta_float_ratio(*value)?;
            let scaled = total_microseconds.mul(&numerator);
            let rounded = self.timedelta_divide_nearest(&scaled, &denominator)?;
            return self.timedelta_instance_from_total_microseconds(rounded);
        }
        Ok(self.timedelta_not_implemented())
    }

    fn timedelta_divide_by_value(
        &mut self,
        total_microseconds: BigInt,
        divisor: &Value,
    ) -> Result<Value, RuntimeError> {
        if let Some(divisor) = Self::timedelta_integer_factor(divisor) {
            let rounded = self.timedelta_divide_nearest(&total_microseconds, &divisor)?;
            return self.timedelta_instance_from_total_microseconds(rounded);
        }
        match divisor {
            Value::Float(value) => {
                let (numerator, denominator) = Self::timedelta_float_ratio(*value)?;
                let scaled = total_microseconds.mul(&denominator);
                let rounded = self.timedelta_divide_nearest(&scaled, &numerator)?;
                self.timedelta_instance_from_total_microseconds(rounded)
            }
            _ => Ok(self.timedelta_not_implemented()),
        }
    }

    fn timedelta_multiply_instance(
        &mut self,
        instance: &ObjRef,
        factor: &Value,
        method_name: &str,
    ) -> Result<Value, RuntimeError> {
        let total_microseconds =
            Self::timedelta_total_microseconds_from_instance(instance, method_name)?;
        self.timedelta_multiply_total_by_factor(total_microseconds, factor)
    }

    fn date_instance_from_parts(
        &mut self,
        class: ObjRef,
        year: i64,
        month: u32,
        day: u32,
    ) -> Result<Value, RuntimeError> {
        let instance = self.alloc_instance_for_class(&class);
        {
            let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                return Err(RuntimeError::new("date construction failed"));
            };
            instance_data
                .attrs
                .insert("year".to_string(), Value::Int(year));
            instance_data
                .attrs
                .insert("month".to_string(), Value::Int(month as i64));
            instance_data
                .attrs
                .insert("day".to_string(), Value::Int(day as i64));
        }
        Ok(Value::Instance(instance))
    }

    fn time_instance_from_parts(
        &mut self,
        class: ObjRef,
        hour: i64,
        minute: i64,
        second: i64,
        microsecond: i64,
        tzinfo: Option<Value>,
        fold: i64,
    ) -> Result<Value, RuntimeError> {
        let instance = self.alloc_instance_for_class(&class);
        {
            let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                return Err(RuntimeError::new("time construction failed"));
            };
            instance_data
                .attrs
                .insert("hour".to_string(), Value::Int(hour));
            instance_data
                .attrs
                .insert("minute".to_string(), Value::Int(minute));
            instance_data
                .attrs
                .insert("second".to_string(), Value::Int(second));
            instance_data
                .attrs
                .insert("microsecond".to_string(), Value::Int(microsecond));
            if let Some(tzinfo) = tzinfo {
                instance_data.attrs.insert("tzinfo".to_string(), tzinfo);
            }
            instance_data
                .attrs
                .insert("fold".to_string(), Value::Int(fold));
        }
        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_datetime_fromtimestamp(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let class = if let Some(Value::Class(_)) = args.first() {
            self.receiver_from_value(&args.remove(0))?
        } else {
            self.datetime_default_class()?
        };
        let mut timestamp = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut tz = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if !args.is_empty() {
            return Err(RuntimeError::new(
                "fromtimestamp() takes at most 2 positional arguments",
            ));
        }
        if let Some(value) = kwargs.remove("timestamp") {
            if timestamp.is_some() {
                return Err(RuntimeError::new(
                    "fromtimestamp() got multiple values for argument 'timestamp'",
                ));
            }
            timestamp = Some(value);
        }
        if let Some(value) = kwargs.remove("tz") {
            if tz.is_some() {
                return Err(RuntimeError::new(
                    "fromtimestamp() got multiple values for argument 'tz'",
                ));
            }
            tz = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "fromtimestamp() got an unexpected keyword argument",
            ));
        }
        let timestamp =
            timestamp.ok_or_else(|| RuntimeError::new("fromtimestamp() missing timestamp"))?;
        let timestamp = value_to_f64(timestamp)?;
        let mut seconds = timestamp.floor() as i64;
        let mut microsecond = ((timestamp - seconds as f64) * 1_000_000.0).round() as i64;
        if microsecond >= 1_000_000 {
            seconds = seconds
                .checked_add(1)
                .ok_or_else(|| RuntimeError::overflow_error("timestamp out of range"))?;
            microsecond -= 1_000_000;
        } else if microsecond < 0 {
            seconds = seconds
                .checked_sub(1)
                .ok_or_else(|| RuntimeError::overflow_error("timestamp out of range"))?;
            microsecond += 1_000_000;
        }
        let tz_offset = tz
            .as_ref()
            .map(|value| self.datetime_timezone_offset_seconds(value))
            .transpose()?
            .unwrap_or(0);
        let adjusted_seconds = seconds
            .checked_add(tz_offset)
            .ok_or_else(|| RuntimeError::overflow_error("timestamp out of range"))?;
        let parts = split_unix_timestamp(adjusted_seconds);
        self.datetime_instance_from_parts(
            class,
            parts.year as i64,
            parts.month,
            parts.day,
            parts.hour,
            parts.minute,
            parts.second,
            microsecond,
            tz,
        )
    }

    pub(super) fn builtin_datetime_astimezone(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("astimezone() missing instance"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "datetime.astimezone")?;
        let mut tz = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if !args.is_empty() {
            return Err(RuntimeError::new(
                "astimezone() takes at most one positional argument",
            ));
        }
        if let Some(value) = kwargs.remove("tz") {
            if tz.is_some() {
                return Err(RuntimeError::new(
                    "astimezone() got multiple values for argument 'tz'",
                ));
            }
            tz = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "astimezone() got an unexpected keyword argument",
            ));
        }

        let (class, year, month, day, hour, minute, second, microsecond, current_tz) = {
            let Object::Instance(instance_data) = &*instance.kind() else {
                return Err(RuntimeError::new(
                    "astimezone() expects datetime instance receiver",
                ));
            };
            let year = instance_data
                .attrs
                .get("year")
                .cloned()
                .map(value_to_int)
                .transpose()?
                .ok_or_else(|| RuntimeError::new("astimezone() missing year"))?;
            let month = instance_data
                .attrs
                .get("month")
                .cloned()
                .map(value_to_int)
                .transpose()?
                .ok_or_else(|| RuntimeError::new("astimezone() missing month"))?
                as u32;
            let day = instance_data
                .attrs
                .get("day")
                .cloned()
                .map(value_to_int)
                .transpose()?
                .ok_or_else(|| RuntimeError::new("astimezone() missing day"))?
                as u32;
            let hour = instance_data
                .attrs
                .get("hour")
                .cloned()
                .map(value_to_int)
                .transpose()?
                .unwrap_or(0) as u32;
            let minute = instance_data
                .attrs
                .get("minute")
                .cloned()
                .map(value_to_int)
                .transpose()?
                .unwrap_or(0) as u32;
            let second = instance_data
                .attrs
                .get("second")
                .cloned()
                .map(value_to_int)
                .transpose()?
                .unwrap_or(0) as u32;
            let microsecond = instance_data
                .attrs
                .get("microsecond")
                .cloned()
                .map(value_to_int)
                .transpose()?
                .unwrap_or(0);
            (
                instance_data.class.clone(),
                year,
                month,
                day,
                hour,
                minute,
                second,
                microsecond,
                instance_data.attrs.get("tzinfo").cloned(),
            )
        };

        let current_offset = current_tz
            .as_ref()
            .map(|value| self.datetime_timezone_offset_seconds(value))
            .transpose()?
            .unwrap_or(0);
        let target_tz = tz.or(current_tz);
        let target_offset = target_tz
            .as_ref()
            .map(|value| self.datetime_timezone_offset_seconds(value))
            .transpose()?
            .unwrap_or(current_offset);
        let days = days_from_civil(year, month, day);
        let utc_seconds = days * 86_400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64
            - current_offset;
        let local = split_unix_timestamp(utc_seconds + target_offset);
        self.datetime_instance_from_parts(
            class,
            local.year as i64,
            local.month,
            local.day,
            local.hour,
            local.minute,
            local.second,
            microsecond,
            target_tz,
        )
    }

    pub(super) fn builtin_datetime_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("datetime.__init__() missing instance"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        if args.len() > 7 {
            return Err(RuntimeError::new(
                "datetime.__init__() expects year, month, day and optional time fields",
            ));
        }

        let mut year = None;
        let mut month = None;
        let mut day = None;
        let mut hour = Some(0_i64);
        let mut minute = Some(0_i64);
        let mut second = Some(0_i64);
        let mut microsecond = Some(0_i64);
        let mut tzinfo = None;

        if let Some(value) = args.first() {
            year = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(1) {
            month = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(2) {
            day = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(3) {
            hour = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(4) {
            minute = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(5) {
            second = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(6) {
            microsecond = Some(value_to_int(value.clone())?);
        }

        if let Some(value) = kwargs.remove("year") {
            year = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("month") {
            month = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("day") {
            day = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("hour") {
            hour = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("minute") {
            minute = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("second") {
            second = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("microsecond") {
            microsecond = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("tzinfo") {
            tzinfo = Some(value);
        }
        if kwargs.contains_key("fold") {
            let _ = kwargs.remove("fold");
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "datetime.__init__() got unexpected keyword",
            ));
        }

        let (year, month, day) = match (year, month, day) {
            (Some(year), Some(month), Some(day)) => (year, month, day),
            _ => {
                return Err(RuntimeError::new(
                    "datetime.__init__() missing required year/month/day",
                ));
            }
        };

        let Object::Instance(instance_data) = &mut *receiver.kind_mut() else {
            return Err(RuntimeError::new(
                "datetime.__init__() expects instance receiver",
            ));
        };
        instance_data
            .attrs
            .insert("year".to_string(), Value::Int(year));
        instance_data
            .attrs
            .insert("month".to_string(), Value::Int(month));
        instance_data
            .attrs
            .insert("day".to_string(), Value::Int(day));
        instance_data
            .attrs
            .insert("hour".to_string(), Value::Int(hour.unwrap_or(0)));
        instance_data
            .attrs
            .insert("minute".to_string(), Value::Int(minute.unwrap_or(0)));
        instance_data
            .attrs
            .insert("second".to_string(), Value::Int(second.unwrap_or(0)));
        instance_data.attrs.insert(
            "microsecond".to_string(),
            Value::Int(microsecond.unwrap_or(0)),
        );
        if let Some(tzinfo) = tzinfo {
            instance_data.attrs.insert("tzinfo".to_string(), tzinfo);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_datetime_replace(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "datetime.replace")?;
        if args.len() > 8 {
            return Err(RuntimeError::new(
                "replace() takes at most 8 positional arguments",
            ));
        }

        let (
            class,
            mut year,
            mut month,
            mut day,
            mut hour,
            mut minute,
            mut second,
            mut microsecond,
            mut tzinfo,
            mut fold,
        ) = {
            let Object::Instance(instance_data) = &*instance.kind() else {
                return Err(RuntimeError::new(
                    "datetime.replace() expects instance receiver",
                ));
            };
            let read_field = |name: &str| -> Result<i64, RuntimeError> {
                instance_data
                    .attrs
                    .get(name)
                    .cloned()
                    .map(value_to_int)
                    .transpose()?
                    .ok_or_else(|| RuntimeError::new(format!("datetime.replace() missing {name}")))
            };
            (
                instance_data.class.clone(),
                read_field("year")?,
                read_field("month")?,
                read_field("day")?,
                read_field("hour").unwrap_or(0),
                read_field("minute").unwrap_or(0),
                read_field("second").unwrap_or(0),
                read_field("microsecond").unwrap_or(0),
                instance_data.attrs.get("tzinfo").cloned(),
                instance_data
                    .attrs
                    .get("fold")
                    .cloned()
                    .map(value_to_int)
                    .transpose()?
                    .unwrap_or(0),
            )
        };

        let positional_count = args.len();
        let mut set_int_field =
            |name: &str, index: usize, target: &mut i64| -> Result<(), RuntimeError> {
                if let Some(value) = args.get(index).cloned() {
                    if !matches!(value, Value::None) {
                        *target = value_to_int(value)?;
                    }
                }
                if let Some(value) = kwargs.remove(name) {
                    if positional_count > index {
                        return Err(RuntimeError::new(format!(
                            "replace() got multiple values for argument '{name}'"
                        )));
                    }
                    if !matches!(value, Value::None) {
                        *target = value_to_int(value)?;
                    }
                }
                Ok(())
            };

        set_int_field("year", 0, &mut year)?;
        set_int_field("month", 1, &mut month)?;
        set_int_field("day", 2, &mut day)?;
        set_int_field("hour", 3, &mut hour)?;
        set_int_field("minute", 4, &mut minute)?;
        set_int_field("second", 5, &mut second)?;
        set_int_field("microsecond", 6, &mut microsecond)?;

        if let Some(value) = args.get(7).cloned() {
            tzinfo = if value == Value::Bool(true) {
                tzinfo
            } else if value == Value::None {
                None
            } else {
                Some(value)
            };
        }
        if let Some(value) = kwargs.remove("tzinfo") {
            if positional_count > 7 {
                return Err(RuntimeError::new(
                    "replace() got multiple values for argument 'tzinfo'",
                ));
            }
            tzinfo = if value == Value::Bool(true) {
                tzinfo
            } else if value == Value::None {
                None
            } else {
                Some(value)
            };
        }
        if let Some(value) = kwargs.remove("fold")
            && !matches!(value, Value::None)
        {
            fold = value_to_int(value)?;
        }

        if !kwargs.is_empty() {
            let mut keys: Vec<_> = kwargs.keys().cloned().collect();
            keys.sort();
            return Err(RuntimeError::new(format!(
                "replace() got an unexpected keyword argument '{}'",
                keys[0]
            )));
        }

        let replaced = self.datetime_instance_from_parts(
            class,
            year,
            month as u32,
            day as u32,
            hour as u32,
            minute as u32,
            second as u32,
            microsecond,
            tzinfo,
        )?;
        if let Value::Instance(obj) = &replaced {
            Self::instance_attr_set(obj, "fold", Value::Int(fold))?;
        }
        Ok(replaced)
    }

    pub(super) fn builtin_datetime_repr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "__repr__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "datetime.__repr__")?;
        if !args.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "__repr__() takes no arguments ({} given)",
                args.len()
            )));
        }
        let read_part = |name: &str| -> Result<i64, RuntimeError> {
            Self::instance_attr_get(&instance, name)
                .ok_or_else(|| RuntimeError::new(format!("datetime.__repr__() missing {name}")))
                .and_then(value_to_int)
        };
        let year = read_part("year")?;
        let month = read_part("month")?;
        let day = read_part("day")?;
        let hour = read_part("hour").unwrap_or(0);
        let minute = read_part("minute").unwrap_or(0);
        let second = read_part("second").unwrap_or(0);
        let microsecond = read_part("microsecond").unwrap_or(0);
        let tzinfo = Self::instance_attr_get(&instance, "tzinfo");
        let fold = Self::instance_attr_get(&instance, "fold")
            .map(value_to_int)
            .transpose()?
            .unwrap_or(0);

        let mut parts = vec![
            year.to_string(),
            month.to_string(),
            day.to_string(),
            hour.to_string(),
            minute.to_string(),
        ];
        if second != 0 || microsecond != 0 || tzinfo.is_some() || fold != 0 {
            parts.push(second.to_string());
        }
        if microsecond != 0 || tzinfo.is_some() || fold != 0 {
            parts.push(microsecond.to_string());
        }
        if let Some(tzinfo_value) = tzinfo {
            let Value::Str(tzinfo_repr) = self.builtin_repr(vec![tzinfo_value], HashMap::new())?
            else {
                return Err(RuntimeError::type_error("__repr__ returned non-string"));
            };
            parts.push(format!("tzinfo={tzinfo_repr}"));
        }
        if fold != 0 {
            parts.push(format!("fold={fold}"));
        }
        Ok(Value::Str(format!(
            "datetime.datetime({})",
            parts.join(", ")
        )))
    }

    pub(super) fn builtin_datetime_str(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "__str__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "datetime.__str__")?;
        if !args.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "__str__() takes no arguments ({} given)",
                args.len()
            )));
        }
        self.builtin_date_isoformat(
            vec![Value::Instance(instance), Value::Str(" ".to_string())],
            HashMap::new(),
        )
    }

    pub(super) fn builtin_date_repr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "__repr__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "date.__repr__")?;
        if !args.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "__repr__() takes no arguments ({} given)",
                args.len()
            )));
        }
        let read_part = |name: &str| -> Result<i64, RuntimeError> {
            Self::instance_attr_get(&instance, name)
                .ok_or_else(|| RuntimeError::new(format!("date.__repr__() missing {name}")))
                .and_then(value_to_int)
        };
        let year = read_part("year")?;
        let month = read_part("month")?;
        let day = read_part("day")?;
        Ok(Value::Str(format!("datetime.date({year}, {month}, {day})")))
    }

    pub(super) fn builtin_date_str(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "__str__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "date.__str__")?;
        if !args.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "__str__() takes no arguments ({} given)",
                args.len()
            )));
        }
        self.builtin_date_isoformat(vec![Value::Instance(instance)], HashMap::new())
    }

    pub(super) fn builtin_date_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("date.__init__() missing instance"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        if args.len() > 3 {
            return Err(RuntimeError::new(
                "date.__init__() expects year, month, day",
            ));
        }

        let mut year = None;
        let mut month = None;
        let mut day = None;
        if let Some(value) = args.first() {
            year = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(1) {
            month = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(2) {
            day = Some(value_to_int(value.clone())?);
        }

        if let Some(value) = kwargs.remove("year") {
            year = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("month") {
            month = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("day") {
            day = Some(value_to_int(value)?);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("date.__init__() got unexpected keyword"));
        }

        let (year, month, day) = match (year, month, day) {
            (Some(year), Some(month), Some(day)) => (year, month, day),
            _ => {
                return Err(RuntimeError::new(
                    "date.__init__() missing required year/month/day",
                ));
            }
        };

        let Object::Instance(instance_data) = &mut *receiver.kind_mut() else {
            return Err(RuntimeError::new(
                "date.__init__() expects instance receiver",
            ));
        };
        instance_data
            .attrs
            .insert("year".to_string(), Value::Int(year));
        instance_data
            .attrs
            .insert("month".to_string(), Value::Int(month));
        instance_data
            .attrs
            .insert("day".to_string(), Value::Int(day));
        Ok(Value::None)
    }

    pub(super) fn builtin_date_replace(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "date.replace")?;
        if args.len() > 3 {
            return Err(RuntimeError::new(
                "replace() takes at most 3 positional arguments",
            ));
        }

        let (class, mut year, mut month, mut day) = {
            let Object::Instance(instance_data) = &*instance.kind() else {
                return Err(RuntimeError::new(
                    "date.replace() expects instance receiver",
                ));
            };
            let read_field = |name: &str| -> Result<i64, RuntimeError> {
                instance_data
                    .attrs
                    .get(name)
                    .cloned()
                    .map(value_to_int)
                    .transpose()?
                    .ok_or_else(|| RuntimeError::new(format!("date.replace() missing {name}")))
            };
            (
                instance_data.class.clone(),
                read_field("year")?,
                read_field("month")?,
                read_field("day")?,
            )
        };

        let positional_count = args.len();
        let mut set_int_field =
            |name: &str, index: usize, target: &mut i64| -> Result<(), RuntimeError> {
                if let Some(value) = args.get(index).cloned() {
                    if !matches!(value, Value::None) {
                        *target = value_to_int(value)?;
                    }
                }
                if let Some(value) = kwargs.remove(name) {
                    if positional_count > index {
                        return Err(RuntimeError::new(format!(
                            "replace() got multiple values for argument '{name}'"
                        )));
                    }
                    if !matches!(value, Value::None) {
                        *target = value_to_int(value)?;
                    }
                }
                Ok(())
            };
        set_int_field("year", 0, &mut year)?;
        set_int_field("month", 1, &mut month)?;
        set_int_field("day", 2, &mut day)?;

        if !kwargs.is_empty() {
            let mut keys: Vec<_> = kwargs.keys().cloned().collect();
            keys.sort();
            return Err(RuntimeError::new(format!(
                "replace() got an unexpected keyword argument '{}'",
                keys[0]
            )));
        }

        self.date_instance_from_parts(class, year, month as u32, day as u32)
    }

    pub(super) fn builtin_date_toordinal(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("date.toordinal() takes no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "date.toordinal")?;
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new(
                "date.toordinal() expects date/datetime instance receiver",
            ));
        };
        let year = match instance_data.attrs.get("year") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.toordinal() missing year")),
        };
        let month = match instance_data.attrs.get("month") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.toordinal() missing month")),
        };
        let day = match instance_data.attrs.get("day") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.toordinal() missing day")),
        };
        let days = days_from_civil(year, month as u32, day as u32);
        Ok(Value::Int(days + UNIX_EPOCH_ORDINAL))
    }

    fn parse_fromisocalendar_args(
        &self,
        args: &mut Vec<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<(i64, i64, i64), RuntimeError> {
        let mut year = args.first().cloned();
        let mut week = args.get(1).cloned();
        let mut day = args.get(2).cloned();
        if args.len() > 3 {
            return Err(RuntimeError::new(
                "fromisocalendar() takes exactly 3 positional arguments",
            ));
        }
        args.clear();

        if let Some(value) = kwargs.remove("year") {
            if year.is_some() {
                return Err(RuntimeError::new(
                    "fromisocalendar() got multiple values for argument 'year'",
                ));
            }
            year = Some(value);
        }
        if let Some(value) = kwargs.remove("week") {
            if week.is_some() {
                return Err(RuntimeError::new(
                    "fromisocalendar() got multiple values for argument 'week'",
                ));
            }
            week = Some(value);
        }
        if let Some(value) = kwargs.remove("day") {
            if day.is_some() {
                return Err(RuntimeError::new(
                    "fromisocalendar() got multiple values for argument 'day'",
                ));
            }
            day = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "fromisocalendar() got an unexpected keyword argument",
            ));
        }

        let year = year.ok_or_else(|| RuntimeError::new("fromisocalendar() missing year"))?;
        let week = week.ok_or_else(|| RuntimeError::new("fromisocalendar() missing week"))?;
        let day = day.ok_or_else(|| RuntimeError::new("fromisocalendar() missing day"))?;
        Ok((value_to_int(year)?, value_to_int(week)?, value_to_int(day)?))
    }

    fn parse_date_fromisoformat_components(
        &self,
        dtstr: &str,
    ) -> Result<(i64, u32, u32), RuntimeError> {
        let invalid = || {
            RuntimeError::value_error(format!(
                "Invalid isoformat string: '{}'",
                escaped_single_quoted(dtstr)
            ))
        };

        let parse_ymd =
            |year: &str, month: &str, day: &str| -> Result<(i64, u32, u32), RuntimeError> {
                let year = parse_ascii_i64_component(year).ok_or_else(invalid)?;
                let month = parse_ascii_i64_component(month).ok_or_else(invalid)?;
                let day = parse_ascii_i64_component(day).ok_or_else(invalid)?;

                if !(DATETIME_MIN_YEAR..=DATETIME_MAX_YEAR).contains(&year) {
                    return Err(RuntimeError::value_error(format!(
                        "year must be in {}..{}, not {}",
                        DATETIME_MIN_YEAR, DATETIME_MAX_YEAR, year
                    )));
                }
                if !(1..=12).contains(&month) {
                    return Err(RuntimeError::value_error(format!(
                        "month must be in 1..12, not {}",
                        month
                    )));
                }
                let month_u32 = month as u32;
                let max_day = days_in_month(year, month_u32).ok_or_else(invalid)?;
                if day < 1 || day > max_day as i64 {
                    return Err(RuntimeError::value_error(format!(
                        "day {} must be in range 1..{} for month {} in year {}",
                        day, max_day, month, year
                    )));
                }
                Ok((year, month_u32, day as u32))
            };

        if dtstr.len() == 10
            && dtstr.as_bytes().get(4) == Some(&b'-')
            && dtstr.as_bytes().get(7) == Some(&b'-')
        {
            return parse_ymd(&dtstr[0..4], &dtstr[5..7], &dtstr[8..10]);
        }
        if dtstr.len() == 8 && dtstr.as_bytes().iter().all(u8::is_ascii_digit) {
            return parse_ymd(&dtstr[0..4], &dtstr[4..6], &dtstr[6..8]);
        }

        let (iso_year, iso_week, iso_day) = if dtstr.len() == 10
            && dtstr.as_bytes().get(4) == Some(&b'-')
            && dtstr.as_bytes().get(5) == Some(&b'W')
            && dtstr.as_bytes().get(8) == Some(&b'-')
        {
            (
                parse_ascii_i64_component(&dtstr[0..4]).ok_or_else(invalid)?,
                parse_ascii_i64_component(&dtstr[6..8]).ok_or_else(invalid)?,
                parse_ascii_i64_component(&dtstr[9..10]).ok_or_else(invalid)?,
            )
        } else if dtstr.len() == 8 && dtstr.as_bytes().get(4) == Some(&b'W') {
            (
                parse_ascii_i64_component(&dtstr[0..4]).ok_or_else(invalid)?,
                parse_ascii_i64_component(&dtstr[5..7]).ok_or_else(invalid)?,
                parse_ascii_i64_component(&dtstr[7..8]).ok_or_else(invalid)?,
            )
        } else if dtstr.len() == 8
            && dtstr.as_bytes().get(4) == Some(&b'-')
            && dtstr.as_bytes().get(5) == Some(&b'W')
        {
            (
                parse_ascii_i64_component(&dtstr[0..4]).ok_or_else(invalid)?,
                parse_ascii_i64_component(&dtstr[6..8]).ok_or_else(invalid)?,
                1,
            )
        } else if dtstr.len() == 7 && dtstr.as_bytes().get(4) == Some(&b'W') {
            (
                parse_ascii_i64_component(&dtstr[0..4]).ok_or_else(invalid)?,
                parse_ascii_i64_component(&dtstr[5..7]).ok_or_else(invalid)?,
                1,
            )
        } else {
            return Err(invalid());
        };

        self.iso_to_ymd(iso_year, iso_week, iso_day)
            .map_err(|_| invalid())
    }

    fn iso_week1_monday_ordinal(&self, year: i64) -> i64 {
        let first_day = days_from_civil(year, 1, 1) + UNIX_EPOCH_ORDINAL;
        let first_weekday = (first_day + 6).rem_euclid(7);
        let mut week1_monday = first_day - first_weekday;
        if first_weekday > 3 {
            week1_monday += 7;
        }
        week1_monday
    }

    fn iso_to_ymd(
        &self,
        iso_year: i64,
        iso_week: i64,
        iso_day: i64,
    ) -> Result<(i64, u32, u32), RuntimeError> {
        if !(DATETIME_MIN_YEAR..=DATETIME_MAX_YEAR).contains(&iso_year) {
            return Err(RuntimeError::new(format!(
                "year must be in {}..{}, not {}",
                DATETIME_MIN_YEAR, DATETIME_MAX_YEAR, iso_year
            )));
        }

        if iso_week <= 0 || iso_week >= 53 {
            let mut out_of_range = true;
            if iso_week == 53 {
                let first_weekday = (days_from_civil(iso_year, 1, 1) + 3).rem_euclid(7);
                if first_weekday == 3
                    || (first_weekday == 2 && day_of_year(iso_year as i32, 12, 31) == 366)
                {
                    out_of_range = false;
                }
            }
            if out_of_range {
                return Err(RuntimeError::new(format!("Invalid week: {}", iso_week)));
            }
        }

        if iso_day <= 0 || iso_day >= 8 {
            return Err(RuntimeError::new(format!(
                "Invalid weekday: {} (range is [1, 7])",
                iso_day
            )));
        }

        let day_1 = self.iso_week1_monday_ordinal(iso_year);
        let day_offset = (iso_week - 1) * 7 + iso_day - 1;
        let (year, month, day) = civil_from_days(day_1 + day_offset - UNIX_EPOCH_ORDINAL);
        Ok((year, month, day))
    }

    pub(super) fn builtin_date_fromisocalendar(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let class = if let Some(Value::Class(_)) = args.first() {
            self.receiver_from_value(&args.remove(0))?
        } else {
            self.date_default_class()?
        };
        let (iso_year, iso_week, iso_day) =
            self.parse_fromisocalendar_args(&mut args, &mut kwargs)?;
        let (year, month, day) = self.iso_to_ymd(iso_year, iso_week, iso_day)?;
        self.date_instance_from_parts(class, year, month, day)
    }

    pub(super) fn builtin_date_fromisoformat(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let class = if let Some(Value::Class(_)) = args.first() {
            self.receiver_from_value(&args.remove(0))?
        } else {
            self.date_default_class()?
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "date.fromisoformat() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "date.fromisoformat() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let dtstr = match args.remove(0) {
            Value::Str(value) => value,
            _ => {
                return Err(RuntimeError::type_error(
                    "fromisoformat: argument must be str",
                ));
            }
        };
        let (year, month, day) = self.parse_date_fromisoformat_components(&dtstr)?;
        self.date_instance_from_parts(class, year, month, day)
    }

    pub(super) fn builtin_datetime_fromisocalendar(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let class = if let Some(Value::Class(_)) = args.first() {
            self.receiver_from_value(&args.remove(0))?
        } else {
            self.datetime_default_class()?
        };
        let (iso_year, iso_week, iso_day) =
            self.parse_fromisocalendar_args(&mut args, &mut kwargs)?;
        let (year, month, day) = self.iso_to_ymd(iso_year, iso_week, iso_day)?;
        self.datetime_instance_from_parts(class, year, month, day, 0, 0, 0, 0, None)
    }

    pub(super) fn builtin_date_weekday(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("date.weekday() takes no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "date.weekday")?;
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new(
                "date.weekday() expects date/datetime instance receiver",
            ));
        };
        let year = match instance_data.attrs.get("year") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.weekday() missing year")),
        };
        let month = match instance_data.attrs.get("month") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.weekday() missing month")),
        };
        let day = match instance_data.attrs.get("day") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.weekday() missing day")),
        };
        let days = days_from_civil(year, month as u32, day as u32);
        Ok(Value::Int((days + 3).rem_euclid(7)))
    }

    pub(super) fn builtin_date_isoweekday(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("date.isoweekday() takes no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "date.isoweekday")?;
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new(
                "date.isoweekday() expects date/datetime instance receiver",
            ));
        };
        let year = match instance_data.attrs.get("year") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.isoweekday() missing year")),
        };
        let month = match instance_data.attrs.get("month") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.isoweekday() missing month")),
        };
        let day = match instance_data.attrs.get("day") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.isoweekday() missing day")),
        };
        let days = days_from_civil(year, month as u32, day as u32);
        Ok(Value::Int((days + 3).rem_euclid(7) + 1))
    }

    pub(super) fn builtin_date_isoformat(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("isoformat() missing instance"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "date.isoformat")?;
        let mut sep = "T".to_string();
        if let Some(value) = kwargs.remove("sep") {
            sep = match value {
                Value::Str(text) => text,
                _ => return Err(RuntimeError::new("isoformat() sep must be str")),
            };
        }
        let _timespec = kwargs.remove("timespec");
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("isoformat() got invalid arguments"));
        }
        if let Some(value) = args.pop() {
            sep = match value {
                Value::Str(text) => text,
                _ => return Err(RuntimeError::new("isoformat() sep must be str")),
            };
        }
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new(
                "isoformat() expects date/datetime instance receiver",
            ));
        };
        let year = instance_data
            .attrs
            .get("year")
            .cloned()
            .map(value_to_int)
            .transpose()?
            .ok_or_else(|| RuntimeError::new("isoformat() missing year"))?;
        let month = instance_data
            .attrs
            .get("month")
            .cloned()
            .map(value_to_int)
            .transpose()?
            .ok_or_else(|| RuntimeError::new("isoformat() missing month"))?;
        let day = instance_data
            .attrs
            .get("day")
            .cloned()
            .map(value_to_int)
            .transpose()?
            .ok_or_else(|| RuntimeError::new("isoformat() missing day"))?;
        let mut out = format!("{year:04}-{month:02}-{day:02}");
        if let Some(hour_value) = instance_data.attrs.get("hour").cloned() {
            let hour = value_to_int(hour_value)?;
            let minute = instance_data
                .attrs
                .get("minute")
                .cloned()
                .map(value_to_int)
                .transpose()?
                .unwrap_or(0);
            let second = instance_data
                .attrs
                .get("second")
                .cloned()
                .map(value_to_int)
                .transpose()?
                .unwrap_or(0);
            let microsecond = instance_data
                .attrs
                .get("microsecond")
                .cloned()
                .map(value_to_int)
                .transpose()?
                .unwrap_or(0);
            out.push_str(&format!("{sep}{hour:02}:{minute:02}:{second:02}"));
            if microsecond != 0 {
                out.push_str(&format!(".{microsecond:06}"));
            }
        }
        Ok(Value::Str(out))
    }

    pub(super) fn builtin_date_strftime(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("strftime() expects format argument"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "date.strftime")?;
        let format = match args.remove(0) {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("strftime() format must be str")),
        };
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new(
                "date.strftime() expects date/datetime instance receiver",
            ));
        };
        let year = match instance_data.attrs.get("year") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.strftime() missing year")),
        };
        let month = match instance_data.attrs.get("month") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.strftime() missing month")),
        };
        let day = match instance_data.attrs.get("day") {
            Some(value) => value_to_int(value.clone())?,
            None => return Err(RuntimeError::new("date.strftime() missing day")),
        };
        let hour = instance_data
            .attrs
            .get("hour")
            .cloned()
            .map(value_to_int)
            .transpose()?
            .unwrap_or(0);
        let minute = instance_data
            .attrs
            .get("minute")
            .cloned()
            .map(value_to_int)
            .transpose()?
            .unwrap_or(0);
        let second = instance_data
            .attrs
            .get("second")
            .cloned()
            .map(value_to_int)
            .transpose()?
            .unwrap_or(0);
        let days = days_from_civil(year, month as u32, day as u32);
        let weekday = (days + 3).rem_euclid(7) as u32; // Monday=0
        let yearday = day_of_year(year as i32, month as u32, day as u32);
        let utc_offset_seconds = instance_data
            .attrs
            .get("tzinfo")
            .map(|value| self.datetime_timezone_offset_seconds(value))
            .transpose()?
            .map(|value| value.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
        let parts = TimeParts {
            year: year as i32,
            month: month as u32,
            day: day as u32,
            hour: hour as u32,
            minute: minute as u32,
            second: second as u32,
            weekday,
            yearday,
            isdst: -1,
            utc_offset_seconds,
        };
        Ok(Value::Str(format_strftime(&format, parts)))
    }

    pub(super) fn builtin_datetime_timezone_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("timezone.__init__() missing instance"));
        }
        let instance = self.receiver_from_value(&args.remove(0))?;
        let mut offset = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut name = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if !args.is_empty() {
            return Err(RuntimeError::new(
                "timezone.__init__() takes at most 2 positional arguments",
            ));
        }
        if let Some(value) = kwargs.remove("offset") {
            if offset.is_some() {
                return Err(RuntimeError::new(
                    "timezone.__init__() got multiple values for argument 'offset'",
                ));
            }
            offset = Some(value);
        }
        if let Some(value) = kwargs.remove("name") {
            if name.is_some() {
                return Err(RuntimeError::new(
                    "timezone.__init__() got multiple values for argument 'name'",
                ));
            }
            name = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "timezone.__init__() got an unexpected keyword argument",
            ));
        }
        let Some(offset) = offset else {
            return Err(RuntimeError::new(
                "timezone.__init__() missing required argument 'offset'",
            ));
        };
        let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
            return Err(RuntimeError::new(
                "timezone.__init__() expects instance receiver",
            ));
        };
        instance_data
            .attrs
            .insert("offset".to_string(), offset.clone());
        instance_data.attrs.insert(
            "name".to_string(),
            name.unwrap_or(Value::Str("UTC".to_string())),
        );
        Ok(Value::None)
    }

    pub(super) fn builtin_datetime_delta_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("timedelta.__init__() missing instance"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        if args.len() > 3 {
            return Err(RuntimeError::new(
                "timedelta.__init__() takes at most 3 positional arguments",
            ));
        }

        let mut days = if let Some(value) = args.first() {
            value_to_int(value.clone())?
        } else {
            0
        };
        let mut seconds = if let Some(value) = args.get(1) {
            value_to_int(value.clone())?
        } else {
            0
        };
        let mut microseconds = if let Some(value) = args.get(2) {
            value_to_int(value.clone())?
        } else {
            0
        };

        let mut weeks = 0_i64;
        let mut hours = 0_i64;
        let mut minutes = 0_i64;
        let mut milliseconds = 0_i64;

        if let Some(value) = kwargs.remove("days") {
            days = value_to_int(value)?;
        }
        if let Some(value) = kwargs.remove("seconds") {
            seconds = value_to_int(value)?;
        }
        if let Some(value) = kwargs.remove("microseconds") {
            microseconds = value_to_int(value)?;
        }
        if let Some(value) = kwargs.remove("weeks") {
            weeks = value_to_int(value)?;
        }
        if let Some(value) = kwargs.remove("hours") {
            hours = value_to_int(value)?;
        }
        if let Some(value) = kwargs.remove("minutes") {
            minutes = value_to_int(value)?;
        }
        if let Some(value) = kwargs.remove("milliseconds") {
            milliseconds = value_to_int(value)?;
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "timedelta.__init__() got an unexpected keyword argument",
            ));
        }

        let total_microseconds = BigInt::from_i64(microseconds)
            .add(&BigInt::from_i64(milliseconds).mul(&BigInt::from_i64(1_000)))
            .add(
                &BigInt::from_i64(seconds).mul(&BigInt::from_i64(DATETIME_MICROSECONDS_PER_SECOND)),
            )
            .add(
                &BigInt::from_i64(minutes)
                    .mul(&BigInt::from_i64(60 * DATETIME_MICROSECONDS_PER_SECOND)),
            )
            .add(
                &BigInt::from_i64(hours)
                    .mul(&BigInt::from_i64(3_600 * DATETIME_MICROSECONDS_PER_SECOND)),
            )
            .add(
                &BigInt::from_i64(days)
                    .add(&BigInt::from_i64(weeks).mul(&BigInt::from_i64(7)))
                    .mul(&BigInt::from_i64(DATETIME_MICROSECONDS_PER_DAY)),
            );

        let (normalized_days, normalized_seconds, normalized_microseconds) =
            Self::normalized_timedelta_parts_from_total_microseconds(&total_microseconds)?;
        Self::timedelta_apply_normalized_parts(
            &receiver,
            normalized_days,
            normalized_seconds,
            normalized_microseconds,
        )?;
        Ok(Value::None)
    }

    pub(super) fn builtin_datetime_delta_repr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "__repr__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "timedelta.__repr__")?;
        if !args.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "__repr__() takes no arguments ({} given)",
                args.len()
            )));
        }
        let read_part = |name: &str| -> Result<i64, RuntimeError> {
            Self::instance_attr_get(&instance, name)
                .ok_or_else(|| RuntimeError::new(format!("timedelta.__repr__() missing {name}")))
                .and_then(value_to_int)
        };
        let days = read_part("days")?;
        let seconds = read_part("seconds")?;
        let microseconds = read_part("microseconds")?;

        let mut parts = Vec::new();
        if days != 0 {
            parts.push(format!("days={days}"));
        }
        if seconds != 0 {
            parts.push(format!("seconds={seconds}"));
        }
        if microseconds != 0 {
            parts.push(format!("microseconds={microseconds}"));
        }
        if parts.is_empty() {
            parts.push("0".to_string());
        }
        Ok(Value::Str(format!(
            "datetime.timedelta({})",
            parts.join(", ")
        )))
    }

    pub(super) fn builtin_datetime_delta_str(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "__str__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "timedelta.__str__")?;
        if !args.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "__str__() takes no arguments ({} given)",
                args.len()
            )));
        }
        let read_part = |name: &str| -> Result<i64, RuntimeError> {
            Self::instance_attr_get(&instance, name)
                .ok_or_else(|| RuntimeError::new(format!("timedelta.__str__() missing {name}")))
                .and_then(value_to_int)
        };
        let days = read_part("days")?;
        let total_seconds = read_part("seconds")?;
        let microseconds = read_part("microseconds")?;
        let hours = total_seconds.div_euclid(3600);
        let minutes = total_seconds.rem_euclid(3600).div_euclid(60);
        let seconds = total_seconds.rem_euclid(60);

        let rendered = if days != 0 {
            let day_suffix = if days == 1 || days == -1 { "" } else { "s" };
            if microseconds != 0 {
                format!(
                    "{days} day{day_suffix}, {hours}:{minutes:02}:{seconds:02}.{microseconds:06}"
                )
            } else {
                format!("{days} day{day_suffix}, {hours}:{minutes:02}:{seconds:02}")
            }
        } else if microseconds != 0 {
            format!("{hours}:{minutes:02}:{seconds:02}.{microseconds:06}")
        } else {
            format!("{hours}:{minutes:02}:{seconds:02}")
        };
        Ok(Value::Str(rendered))
    }

    fn timedelta_unary_method_args(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        method_name: &str,
    ) -> Result<ObjRef, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "{method_name}() takes no keyword arguments"
            )));
        }
        let instance = self.take_bound_instance_arg(&mut args, method_name)?;
        if !args.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "{method_name}() takes no arguments ({} given)",
                args.len()
            )));
        }
        Ok(instance)
    }

    fn builtin_datetime_delta_mul_common(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        method_name: &str,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "{method_name}() takes no keyword arguments"
            )));
        }
        let instance = self.take_bound_instance_arg(&mut args, method_name)?;
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "{method_name}() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        self.timedelta_multiply_instance(&instance, &args[0], method_name)
    }

    fn builtin_datetime_delta_add_common(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        method_name: &str,
        reflected: bool,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "{method_name}() takes no keyword arguments"
            )));
        }
        let instance = self.take_bound_instance_arg(&mut args, method_name)?;
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "{method_name}() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let arg = &args[0];
        if reflected {
            return match self.timedelta_binary_pair_totals(&instance, arg, method_name)? {
                Some((left, right)) => {
                    self.timedelta_instance_from_total_microseconds(right.add(&left))
                }
                None => Ok(self.timedelta_not_implemented()),
            };
        }
        match self.timedelta_binary_pair_totals(&instance, arg, method_name)? {
            Some((left, right)) => {
                self.timedelta_instance_from_total_microseconds(left.add(&right))
            }
            None => Ok(self.timedelta_not_implemented()),
        }
    }

    fn builtin_datetime_delta_sub_common(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        method_name: &str,
        reflected: bool,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "{method_name}() takes no keyword arguments"
            )));
        }
        let instance = self.take_bound_instance_arg(&mut args, method_name)?;
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "{method_name}() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let arg = &args[0];
        if reflected {
            return match self.timedelta_binary_pair_totals(&instance, arg, method_name)? {
                Some((left, right)) => {
                    self.timedelta_instance_from_total_microseconds(right.sub(&left))
                }
                None => Ok(self.timedelta_not_implemented()),
            };
        }
        match self.timedelta_binary_pair_totals(&instance, arg, method_name)? {
            Some((left, right)) => {
                self.timedelta_instance_from_total_microseconds(left.sub(&right))
            }
            None => Ok(self.timedelta_not_implemented()),
        }
    }

    pub(super) fn builtin_datetime_delta_add(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_datetime_delta_add_common(args, kwargs, "timedelta.__add__", false)
    }

    pub(super) fn builtin_datetime_delta_radd(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_datetime_delta_add_common(args, kwargs, "timedelta.__radd__", true)
    }

    pub(super) fn builtin_datetime_delta_sub(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_datetime_delta_sub_common(args, kwargs, "timedelta.__sub__", false)
    }

    pub(super) fn builtin_datetime_delta_rsub(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_datetime_delta_sub_common(args, kwargs, "timedelta.__rsub__", true)
    }

    pub(super) fn builtin_datetime_delta_neg(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.timedelta_unary_method_args(args, kwargs, "timedelta.__neg__")?;
        let total_microseconds =
            Self::timedelta_total_microseconds_from_instance(&instance, "timedelta.__neg__")?;
        self.timedelta_instance_from_total_microseconds(total_microseconds.negated())
    }

    pub(super) fn builtin_datetime_delta_pos(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.timedelta_unary_method_args(args, kwargs, "timedelta.__pos__")?;
        let total_microseconds =
            Self::timedelta_total_microseconds_from_instance(&instance, "timedelta.__pos__")?;
        self.timedelta_instance_from_total_microseconds(total_microseconds)
    }

    pub(super) fn builtin_datetime_delta_abs(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.timedelta_unary_method_args(args, kwargs, "timedelta.__abs__")?;
        let total_microseconds =
            Self::timedelta_total_microseconds_from_instance(&instance, "timedelta.__abs__")?;
        let total_microseconds = if total_microseconds.is_negative() {
            total_microseconds.negated()
        } else {
            total_microseconds
        };
        self.timedelta_instance_from_total_microseconds(total_microseconds)
    }

    pub(super) fn builtin_datetime_delta_bool(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.timedelta_unary_method_args(args, kwargs, "timedelta.__bool__")?;
        let (days, seconds, microseconds) =
            Self::timedelta_read_parts(&instance, "timedelta.__bool__")?;
        Ok(Value::Bool(days != 0 || seconds != 0 || microseconds != 0))
    }

    pub(super) fn builtin_datetime_delta_mul(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_datetime_delta_mul_common(args, kwargs, "timedelta.__mul__")
    }

    pub(super) fn builtin_datetime_delta_rmul(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_datetime_delta_mul_common(args, kwargs, "timedelta.__rmul__")
    }

    pub(super) fn builtin_datetime_delta_floordiv(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "timedelta.__floordiv__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "timedelta.__floordiv__")?;
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "timedelta.__floordiv__() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let total_microseconds =
            Self::timedelta_total_microseconds_from_instance(&instance, "timedelta.__floordiv__")?;
        if let Some(divisor) = Self::timedelta_integer_factor(&args[0]) {
            let (quotient, _) = total_microseconds
                .div_mod_floor(&divisor)
                .ok_or_else(|| RuntimeError::zero_division_error("division by zero"))?;
            return self.timedelta_instance_from_total_microseconds(quotient);
        }
        if let Some(other) = self.timedelta_instance_arg(&args[0])? {
            let other_total =
                Self::timedelta_total_microseconds_from_instance(&other, "timedelta.__floordiv__")?;
            let (quotient, _) = total_microseconds
                .div_mod_floor(&other_total)
                .ok_or_else(|| RuntimeError::zero_division_error("division by zero"))?;
            return Ok(value_from_bigint(quotient));
        }
        Ok(self.timedelta_not_implemented())
    }

    pub(super) fn builtin_datetime_delta_truediv(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "timedelta.__truediv__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "timedelta.__truediv__")?;
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "timedelta.__truediv__() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let total_microseconds =
            Self::timedelta_total_microseconds_from_instance(&instance, "timedelta.__truediv__")?;
        if let Some(other) = self.timedelta_instance_arg(&args[0])? {
            let other_total =
                Self::timedelta_total_microseconds_from_instance(&other, "timedelta.__truediv__")?;
            if other_total.is_zero() {
                return Err(RuntimeError::zero_division_error("division by zero"));
            }
            return Ok(Value::Float(
                total_microseconds.to_f64() / other_total.to_f64(),
            ));
        }
        self.timedelta_divide_by_value(total_microseconds, &args[0])
    }

    pub(super) fn builtin_datetime_delta_mod(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "timedelta.__mod__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "timedelta.__mod__")?;
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "timedelta.__mod__() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        match self.timedelta_binary_pair_totals(&instance, &args[0], "timedelta.__mod__")? {
            Some((left, right)) => {
                let (_, remainder) = left
                    .div_mod_floor(&right)
                    .ok_or_else(|| RuntimeError::zero_division_error("division by zero"))?;
                self.timedelta_instance_from_total_microseconds(remainder)
            }
            None => Ok(self.timedelta_not_implemented()),
        }
    }

    pub(super) fn builtin_datetime_delta_divmod(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "timedelta.__divmod__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "timedelta.__divmod__")?;
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "timedelta.__divmod__() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        match self.timedelta_binary_pair_totals(&instance, &args[0], "timedelta.__divmod__")? {
            Some((left, right)) => {
                let (quotient, remainder) = left
                    .div_mod_floor(&right)
                    .ok_or_else(|| RuntimeError::zero_division_error("division by zero"))?;
                let remainder = self.timedelta_instance_from_total_microseconds(remainder)?;
                Ok(self
                    .heap
                    .alloc_tuple(vec![value_from_bigint(quotient), remainder]))
            }
            None => Ok(self.timedelta_not_implemented()),
        }
    }

    pub(super) fn builtin_time_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("time.__init__() missing instance"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        if args.len() > 4 {
            return Err(RuntimeError::new(
                "time.__init__() expects hour, minute, second and optional microsecond",
            ));
        }

        let mut hour = Some(0_i64);
        let mut minute = Some(0_i64);
        let mut second = Some(0_i64);
        let mut microsecond = Some(0_i64);
        let mut tzinfo = None;
        if let Some(value) = args.first() {
            hour = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(1) {
            minute = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(2) {
            second = Some(value_to_int(value.clone())?);
        }
        if let Some(value) = args.get(3) {
            microsecond = Some(value_to_int(value.clone())?);
        }

        if let Some(value) = kwargs.remove("hour") {
            hour = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("minute") {
            minute = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("second") {
            second = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("microsecond") {
            microsecond = Some(value_to_int(value)?);
        }
        if let Some(value) = kwargs.remove("tzinfo") {
            tzinfo = Some(value);
        }
        if kwargs.contains_key("fold") {
            let _ = kwargs.remove("fold");
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("time.__init__() got unexpected keyword"));
        }

        let Object::Instance(instance_data) = &mut *receiver.kind_mut() else {
            return Err(RuntimeError::new(
                "time.__init__() expects instance receiver",
            ));
        };
        instance_data
            .attrs
            .insert("hour".to_string(), Value::Int(hour.unwrap_or(0)));
        instance_data
            .attrs
            .insert("minute".to_string(), Value::Int(minute.unwrap_or(0)));
        instance_data
            .attrs
            .insert("second".to_string(), Value::Int(second.unwrap_or(0)));
        instance_data.attrs.insert(
            "microsecond".to_string(),
            Value::Int(microsecond.unwrap_or(0)),
        );
        if let Some(tzinfo) = tzinfo {
            instance_data.attrs.insert("tzinfo".to_string(), tzinfo);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_time_replace(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "time.replace")?;
        if args.len() > 5 {
            return Err(RuntimeError::new(
                "replace() takes at most 5 positional arguments",
            ));
        }

        let (class, mut hour, mut minute, mut second, mut microsecond, mut tzinfo, mut fold) = {
            let Object::Instance(instance_data) = &*instance.kind() else {
                return Err(RuntimeError::new(
                    "time.replace() expects instance receiver",
                ));
            };
            let read_field = |name: &str| -> Result<i64, RuntimeError> {
                instance_data
                    .attrs
                    .get(name)
                    .cloned()
                    .map(value_to_int)
                    .transpose()?
                    .ok_or_else(|| RuntimeError::new(format!("time.replace() missing {name}")))
            };
            (
                instance_data.class.clone(),
                read_field("hour").unwrap_or(0),
                read_field("minute").unwrap_or(0),
                read_field("second").unwrap_or(0),
                read_field("microsecond").unwrap_or(0),
                instance_data.attrs.get("tzinfo").cloned(),
                instance_data
                    .attrs
                    .get("fold")
                    .cloned()
                    .map(value_to_int)
                    .transpose()?
                    .unwrap_or(0),
            )
        };

        let positional_count = args.len();
        let mut set_int_field =
            |name: &str, index: usize, target: &mut i64| -> Result<(), RuntimeError> {
                if let Some(value) = args.get(index).cloned() {
                    if !matches!(value, Value::None) {
                        *target = value_to_int(value)?;
                    }
                }
                if let Some(value) = kwargs.remove(name) {
                    if positional_count > index {
                        return Err(RuntimeError::new(format!(
                            "replace() got multiple values for argument '{name}'"
                        )));
                    }
                    if !matches!(value, Value::None) {
                        *target = value_to_int(value)?;
                    }
                }
                Ok(())
            };
        set_int_field("hour", 0, &mut hour)?;
        set_int_field("minute", 1, &mut minute)?;
        set_int_field("second", 2, &mut second)?;
        set_int_field("microsecond", 3, &mut microsecond)?;

        if let Some(value) = args.get(4).cloned() {
            tzinfo = if value == Value::Bool(true) {
                tzinfo
            } else if value == Value::None {
                None
            } else {
                Some(value)
            };
        }
        if let Some(value) = kwargs.remove("tzinfo") {
            if positional_count > 4 {
                return Err(RuntimeError::new(
                    "replace() got multiple values for argument 'tzinfo'",
                ));
            }
            tzinfo = if value == Value::Bool(true) {
                tzinfo
            } else if value == Value::None {
                None
            } else {
                Some(value)
            };
        }
        if let Some(value) = kwargs.remove("fold")
            && !matches!(value, Value::None)
        {
            fold = value_to_int(value)?;
        }

        if !kwargs.is_empty() {
            let mut keys: Vec<_> = kwargs.keys().cloned().collect();
            keys.sort();
            return Err(RuntimeError::new(format!(
                "replace() got an unexpected keyword argument '{}'",
                keys[0]
            )));
        }

        self.time_instance_from_parts(class, hour, minute, second, microsecond, tzinfo, fold)
    }

    pub(super) fn builtin_asyncio_run(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("run() expects one awaitable argument"));
        }
        let awaitable = args.remove(0);
        self.run_awaitable(awaitable)
    }

    pub(super) fn builtin_asyncio_sleep(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "sleep() expects delay and optional result",
            ));
        }
        let seconds = value_to_f64(args.remove(0))?;
        if seconds < 0.0 {
            return Err(RuntimeError::new("sleep length must be non-negative"));
        }
        let result = if args.is_empty() {
            Value::None
        } else {
            args.remove(0)
        };
        Ok(self.make_immediate_coroutine(result))
    }

    pub(super) fn builtin_asyncio_create_task(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "create_task() expects one awaitable argument",
            ));
        }
        self.awaitable_from_value(args.remove(0))
    }

    pub(super) fn builtin_asyncio_gather(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "gather() keyword arguments are not supported",
            ));
        }
        let mut results = Vec::with_capacity(args.len());
        for awaitable in args {
            results.push(self.run_awaitable(awaitable)?);
        }
        Ok(self.make_immediate_coroutine(self.heap.alloc_list(results)))
    }

    pub(super) fn take_bound_instance_arg(
        &self,
        args: &mut Vec<Value>,
        method_name: &str,
    ) -> Result<ObjRef, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(format!(
                "{method_name}() missing bound instance"
            )));
        }
        match args.remove(0) {
            Value::Instance(instance) => Ok(instance),
            _ => Err(RuntimeError::new(format!(
                "{method_name}() descriptor requires an instance"
            ))),
        }
    }

    pub(super) fn instance_attr_get(instance: &ObjRef, name: &str) -> Option<Value> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        instance_data.attrs.get(name).cloned()
    }

    pub(super) fn instance_attr_set(
        instance: &ObjRef,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
            return Err(RuntimeError::new("expected instance object"));
        };
        instance_data.attrs.insert(name.to_string(), value);
        Ok(())
    }

    fn thread_lock_set_state(lock_value: &Value, locked: bool) {
        let Value::Instance(lock_instance) = lock_value else {
            return;
        };
        if let Object::Instance(lock_data) = &mut *lock_instance.kind_mut() {
            lock_data
                .attrs
                .insert("_locked".to_string(), Value::Bool(locked));
        }
    }

    pub(super) fn builtin_context_run(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::new(
                "Context.run() expects at least a callable argument",
            ));
        }
        let _context = self.take_bound_instance_arg(&mut args, "Context.run")?;
        let callable = args.remove(0);
        if !self.is_callable_value(&callable) {
            return Err(RuntimeError::type_error(
                "Context.run() first argument must be callable",
            ));
        }
        match self.call_internal(callable, args, kwargs)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("Context.run() failed"))
            }
        }
    }

    pub(super) fn builtin_context_copy_context(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("copy_context() expects no arguments"));
        }
        let context_class = {
            let module = self
                .modules
                .get("_contextvars")
                .ok_or_else(|| RuntimeError::new("module '_contextvars' not found"))?;
            let Object::Module(module_data) = &*module.kind() else {
                return Err(RuntimeError::new("module '_contextvars' is invalid"));
            };
            let Some(Value::Class(class)) = module_data.globals.get("Context").cloned() else {
                return Err(RuntimeError::new(
                    "module '_contextvars' is missing Context",
                ));
            };
            class
        };
        let instance = match self.heap.alloc_instance(InstanceObject::new(context_class)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        Ok(Value::Instance(instance))
    }

    fn thread_handle_class(&self) -> Option<ObjRef> {
        let module = self.modules.get("_thread")?;
        let Object::Module(module_data) = &*module.kind() else {
            return None;
        };
        let Value::Class(class) = module_data.globals.get("_ThreadHandle")? else {
            return None;
        };
        Some(class.clone())
    }

    fn thread_handle_instance_from_value(&self, value: &Value) -> Option<ObjRef> {
        let Value::Instance(instance) = value else {
            return None;
        };
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        let handle_class = self.thread_handle_class()?;
        if instance_data.class.id() == handle_class.id() {
            Some(instance.clone())
        } else {
            None
        }
    }

    fn alloc_thread_handle_instance(
        &mut self,
        ident: Option<i64>,
        done: bool,
    ) -> Result<ObjRef, RuntimeError> {
        let handle_class = self
            .thread_handle_class()
            .ok_or_else(|| RuntimeError::new("_thread._ThreadHandle is unavailable"))?;
        let instance = match self.heap.alloc_instance(InstanceObject::new(handle_class)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        let ident_value = ident.map(Value::Int).unwrap_or(Value::None);
        Self::instance_attr_set(&instance, "ident", ident_value)?;
        Self::instance_attr_set(&instance, "_done", Value::Bool(done))?;
        Ok(instance)
    }

    pub(super) fn builtin_threading_get_ident(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("get_ident() expects no arguments"));
        }
        Ok(Value::Int(self.current_thread_ident_value()))
    }

    pub(super) fn builtin_thread_start_new_thread(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "start_new_thread() expects callable, args tuple, and optional kwargs dict",
            ));
        }
        let callable = args.remove(0);
        if !self.is_callable_value(&callable) {
            return Err(RuntimeError::new(
                "start_new_thread() first argument must be callable",
            ));
        }
        let call_args = match args.remove(0) {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::new("start_new_thread() args must be tuple")),
            },
            _ => return Err(RuntimeError::new("start_new_thread() args must be tuple")),
        };
        let call_kwargs = if !args.is_empty() {
            match args.remove(0) {
                Value::Dict(obj) => match &*obj.kind() {
                    Object::Dict(entries) => {
                        let mut out = HashMap::new();
                        for (key, value) in entries {
                            let Value::Str(name) = key else {
                                return Err(RuntimeError::new(
                                    "start_new_thread() kwargs keys must be strings",
                                ));
                            };
                            out.insert(name.clone(), value.clone());
                        }
                        out
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "start_new_thread() kwargs must be a dict",
                        ));
                    }
                },
                _ => {
                    return Err(RuntimeError::new(
                        "start_new_thread() kwargs must be a dict",
                    ));
                }
            }
        } else {
            HashMap::new()
        };

        let (thread_ident, _outcome) =
            self.call_internal_in_synthetic_thread(callable, call_args, call_kwargs)?;
        Ok(Value::Int(thread_ident))
    }

    pub(super) fn builtin_thread_start_joinable_thread(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 3
            || kwargs
                .keys()
                .any(|key| key != "function" && key != "handle" && key != "daemon")
        {
            return Err(RuntimeError::type_error(
                "start_joinable_thread() got unexpected arguments",
            ));
        }
        let callable = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("function").ok_or_else(|| {
                RuntimeError::type_error("start_joinable_thread() missing required argument")
            })?
        };
        if !self.is_callable_value(&callable) {
            return Err(RuntimeError::type_error(
                "start_joinable_thread() first argument must be callable",
            ));
        }
        let handle_value = kwargs
            .remove("handle")
            .or_else(|| (!args.is_empty()).then(|| args.remove(0)))
            .unwrap_or(Value::None);
        let daemon_value = kwargs
            .remove("daemon")
            .or_else(|| (!args.is_empty()).then(|| args.remove(0)))
            .unwrap_or(Value::Bool(true));
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "start_joinable_thread() got unexpected arguments",
            ));
        }
        let _daemon = is_truthy(&daemon_value);
        let mut created_handle = false;
        let handle = if matches!(handle_value, Value::None) {
            created_handle = true;
            self.alloc_thread_handle_instance(None, false)?
        } else if let Some(instance) = self.thread_handle_instance_from_value(&handle_value) {
            instance
        } else {
            return Err(RuntimeError::type_error("'handle' must be a _ThreadHandle"));
        };
        let (thread_ident, _outcome) =
            self.call_internal_in_synthetic_thread(callable, Vec::new(), HashMap::new())?;
        Self::instance_attr_set(&handle, "ident", Value::Int(thread_ident))?;
        // Synthetic-thread execution is synchronous today, so the handle is done
        // when start_joinable_thread() returns.
        Self::instance_attr_set(&handle, "_done", Value::Bool(true))?;
        if created_handle {
            Ok(Value::Instance(handle))
        } else {
            Ok(Value::None)
        }
    }

    pub(super) fn builtin_thread_daemon_threads_allowed(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "daemon_threads_allowed() expects no arguments",
            ));
        }
        Ok(Value::Bool(true))
    }

    pub(super) fn builtin_thread_stack_size(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new(
                "stack_size() expects zero or one positional argument",
            ));
        }
        if let Some(requested) = args.pop() {
            let size = value_to_int(requested)?;
            if size < 0 {
                return Err(RuntimeError::value_error("size must be non-negative"));
            }
            // Current synthetic threading model does not configure per-thread host
            // stack size. Keep CPython-compatible call shape and report previous
            // baseline value.
            return Ok(Value::Int(0));
        }
        Ok(Value::Int(0))
    }

    pub(super) fn builtin_thread_shutdown(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("_shutdown() expects no arguments"));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_make_thread_handle(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_make_thread_handle() expects one argument",
            ));
        }
        let ident = value_to_int(args.remove(0))?;
        Ok(Value::Instance(
            self.alloc_thread_handle_instance(Some(ident), false)?,
        ))
    }

    pub(super) fn builtin_thread_get_main_thread_ident(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "_get_main_thread_ident() expects no arguments",
            ));
        }
        Ok(Value::Int(self.current_thread_ident_value()))
    }

    pub(super) fn builtin_thread_is_main_interpreter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "_is_main_interpreter() expects no arguments",
            ));
        }
        Ok(Value::Bool(true))
    }

    pub(super) fn builtin_thread_handle_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_ThreadHandle.__init__() expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "_ThreadHandle.__init__")?;
        Self::instance_attr_set(&instance, "ident", Value::None)?;
        Self::instance_attr_set(&instance, "_done", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_handle_join(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 || kwargs.keys().any(|key| key != "timeout") {
            return Err(RuntimeError::new(
                "_ThreadHandle.join() expects optional timeout",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "_ThreadHandle.join")?;
        let _timeout = kwargs.remove("timeout").or_else(|| args.pop());
        let _ = instance;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_handle_is_done(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_ThreadHandle.is_done() expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "_ThreadHandle.is_done")?;
        Ok(Value::Bool(matches!(
            Self::instance_attr_get(&instance, "_done"),
            Some(Value::Bool(true))
        )))
    }

    pub(super) fn builtin_thread_handle_set_done(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_ThreadHandle._set_done() expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "_ThreadHandle._set_done")?;
        Self::instance_attr_set(&instance, "_done", Value::Bool(true))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_lock_enter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_thread_lock_acquire(args, kwargs)
    }

    pub(super) fn builtin_thread_lock_exit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 4 {
            return Err(RuntimeError::new(
                "_thread.lock.__exit__() expects optional exception triple",
            ));
        }
        self.builtin_thread_lock_release(vec![args.remove(0)], HashMap::new())?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_lock_acquire(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let invalid_call = kwargs
            .keys()
            .any(|key| key != "blocking" && key != "timeout")
            || args.is_empty()
            || args.len() > 3;
        if invalid_call {
            return Err(RuntimeError::new(
                "_thread.lock.acquire() got unexpected arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "_thread.lock.acquire")?;
        let blocking = kwargs
            .remove("blocking")
            .or_else(|| args.first().cloned())
            .map(|value| is_truthy(&value))
            .unwrap_or(true);
        if let Some(timeout) = kwargs.remove("timeout").or_else(|| args.get(1).cloned()) {
            let timeout = value_to_f64(timeout)?;
            if timeout.is_sign_negative() {
                return Err(RuntimeError::value_error("timeout must be non-negative"));
            }
        }
        let currently_locked = matches!(
            Self::instance_attr_get(&instance, "_locked"),
            Some(Value::Bool(true))
        );
        if currently_locked && !blocking {
            return Ok(Value::Bool(false));
        }
        Self::instance_attr_set(&instance, "_locked", Value::Bool(true))?;
        Ok(Value::Bool(true))
    }

    pub(super) fn builtin_thread_lock_release(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_thread.lock.release() expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "_thread.lock.release")?;
        Self::instance_attr_set(&instance, "_locked", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_lock_locked(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_thread.lock.locked() expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "_thread.lock.locked")?;
        Ok(Value::Bool(matches!(
            Self::instance_attr_get(&instance, "_locked"),
            Some(Value::Bool(true))
        )))
    }

    pub(super) fn builtin_threading_current_thread(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("current_thread() expects no arguments"));
        }
        self.thread_info_dict("MainThread")
    }

    pub(super) fn builtin_threading_main_thread(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("main_thread() expects no arguments"));
        }
        self.thread_info_dict("MainThread")
    }

    pub(super) fn builtin_threading_active_count(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("active_count() expects no arguments"));
        }
        Ok(Value::Int(1))
    }

    pub(super) fn builtin_threading_register_atexit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "_register_atexit() missing required callable argument",
            ));
        }
        let callable = args.remove(0);
        if !self.is_callable_value(&callable) {
            return Err(RuntimeError::new(
                "_register_atexit() first argument must be callable",
            ));
        }
        // Compatibility baseline: accept and record shutdown callbacks as no-op.
        // CPython executes these during threading shutdown; current runtime does
        // not model interpreter-finalization callback ordering yet.
        let _ = args;
        let _ = kwargs;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_class_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "Thread.__init__")?;
        if args.len() > 6 {
            return Err(RuntimeError::new(
                "Thread.__init__() expects up to 6 positional arguments",
            ));
        }
        let mut group = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("group").unwrap_or(Value::None)
        };
        let mut target = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("target").unwrap_or(Value::None)
        };
        let name = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("name").unwrap_or(Value::None)
        };
        let mut call_args = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs
                .remove("args")
                .unwrap_or_else(|| self.heap.alloc_tuple(Vec::new()))
        };
        let call_kwargs = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs
                .remove("kwargs")
                .unwrap_or_else(|| self.heap.alloc_dict(Vec::new()))
        };
        let daemon = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("daemon").unwrap_or(Value::None)
        };

        if group != Value::None {
            let kwargs_is_empty_dict = matches!(
                &call_kwargs,
                Value::Dict(dict) if matches!(&*dict.kind(), Object::Dict(entries) if entries.is_empty())
            );
            let call_args_is_empty_tuple = matches!(
                &call_args,
                Value::Tuple(tuple) if matches!(&*tuple.kind(), Object::Tuple(values) if values.is_empty())
            );
            let shifted_target_args_pair = matches!(target, Value::Tuple(_) | Value::List(_))
                && matches!(name, Value::None)
                && kwargs_is_empty_dict
                && call_args_is_empty_tuple
                && matches!(daemon, Value::None);
            if shifted_target_args_pair {
                call_args = target;
                target = group;
                group = Value::None;
            }
        }
        if group != Value::None {
            return Err(RuntimeError::new("group argument must be None for now"));
        }

        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "Thread.__init__() got unexpected arguments",
            ));
        }
        let name_value = match name {
            Value::None => Value::Str("Thread-1".to_string()),
            Value::Str(text) => Value::Str(text),
            _ => return Err(RuntimeError::new("Thread name must be str or None")),
        };
        let args_value = match call_args {
            Value::Tuple(tuple) => Value::Tuple(tuple),
            Value::List(list) => Value::List(list),
            Value::None => self.heap.alloc_tuple(Vec::new()),
            other => other,
        };
        let kwargs_value = match call_kwargs {
            Value::Dict(dict) => Value::Dict(dict),
            Value::None => self.heap.alloc_dict(Vec::new()),
            other => other,
        };
        Self::instance_attr_set(&instance, "_target", target)?;
        Self::instance_attr_set(&instance, "_name", name_value)?;
        Self::instance_attr_set(&instance, "_args", args_value)?;
        Self::instance_attr_set(&instance, "_kwargs", kwargs_value)?;
        Self::instance_attr_set(&instance, "_daemon", daemon)?;
        Self::instance_attr_set(&instance, "_started", Value::Bool(false))?;
        Self::instance_attr_set(&instance, "_alive", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_class_start(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("Thread.start() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Thread.start")?;
        if matches!(
            Self::instance_attr_get(&instance, "_started"),
            Some(Value::Bool(true))
        ) {
            return Err(RuntimeError::new("threads can only be started once"));
        }
        Self::instance_attr_set(&instance, "_started", Value::Bool(true))?;
        Self::instance_attr_set(&instance, "_alive", Value::Bool(true))?;
        let target = Self::instance_attr_get(&instance, "_target").unwrap_or(Value::None);
        if target != Value::None {
            let call_args = match Self::instance_attr_get(&instance, "_args") {
                Some(Value::Tuple(tuple)) => match &*tuple.kind() {
                    Object::Tuple(values) => values.clone(),
                    _ => Vec::new(),
                },
                Some(Value::List(list)) => match &*list.kind() {
                    Object::List(values) => values.clone(),
                    _ => Vec::new(),
                },
                Some(_) => {
                    return Err(RuntimeError::type_error(
                        "argument after * must be an iterable",
                    ));
                }
                None => Vec::new(),
            };
            let call_kwargs = match Self::instance_attr_get(&instance, "_kwargs") {
                Some(Value::Dict(dict)) => match &*dict.kind() {
                    Object::Dict(entries) => {
                        let mut out = HashMap::new();
                        for (key, value) in entries {
                            let Value::Str(name) = key else {
                                return Err(RuntimeError::type_error(
                                    "keyword name must be string",
                                ));
                            };
                            out.insert(name.clone(), value.clone());
                        }
                        out
                    }
                    _ => HashMap::new(),
                },
                Some(Value::None) | None => HashMap::new(),
                Some(_) => {
                    return Err(RuntimeError::type_error(
                        "argument after ** must be a mapping",
                    ));
                }
            };
            if matches!(target, Value::Builtin(BuiltinFunction::OsRead))
                && call_kwargs.is_empty()
                && call_args.len() == 2
                && let (Ok(fd), Ok(read_size)) = (
                    value_to_int(call_args[0].clone()),
                    value_to_int(call_args[1].clone()),
                )
                && let Ok(mut file) = self.cloned_open_file_for_fd(fd)
            {
                let read_size = read_size.max(0) as usize;
                std::thread::spawn(move || {
                    let mut buf = vec![0u8; read_size.max(1)];
                    let _ = file.read(&mut buf);
                });
                Self::instance_attr_set(&instance, "_alive", Value::Bool(false))?;
                return Ok(Value::None);
            }
            let _ = self.call_internal_in_synthetic_thread(target, call_args, call_kwargs);
        }
        Self::instance_attr_set(&instance, "_alive", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_class_join(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("Thread.join() expects optional timeout"));
        }
        let _instance = self.take_bound_instance_arg(&mut args, "Thread.join")?;
        if let Some(timeout) = args.first().cloned() {
            let timeout = value_to_f64(timeout)?;
            if timeout.is_sign_negative() {
                return Err(RuntimeError::value_error("timeout must be non-negative"));
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_class_is_alive(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("Thread.is_alive() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Thread.is_alive")?;
        Ok(Value::Bool(matches!(
            Self::instance_attr_get(&instance, "_alive"),
            Some(Value::Bool(true))
        )))
    }

    pub(super) fn builtin_thread_event_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("Event.__init__() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Event.__init__")?;
        Self::instance_attr_set(&instance, "_flag", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_event_clear(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("Event.clear() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Event.clear")?;
        Self::instance_attr_set(&instance, "_flag", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_event_is_set(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("Event.is_set() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Event.is_set")?;
        Ok(Value::Bool(matches!(
            Self::instance_attr_get(&instance, "_flag"),
            Some(Value::Bool(true))
        )))
    }

    pub(super) fn builtin_thread_event_set(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("Event.set() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Event.set")?;
        Self::instance_attr_set(&instance, "_flag", Value::Bool(true))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_event_wait(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if kwargs.keys().any(|key| key != "timeout") || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("Event.wait() expects optional timeout"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Event.wait")?;
        let timeout = match (kwargs.remove("timeout"), args.pop()) {
            (Some(_), Some(_)) => {
                return Err(RuntimeError::type_error(
                    "Event.wait() got multiple values for argument 'timeout'",
                ));
            }
            (Some(timeout), None) | (None, Some(timeout)) => Some(timeout),
            (None, None) => None,
        };
        if let Some(timeout) = timeout {
            let timeout = value_to_f64(timeout)?;
            if timeout.is_sign_negative() {
                return Err(RuntimeError::value_error("timeout must be non-negative"));
            }
        }
        Ok(Value::Bool(matches!(
            Self::instance_attr_get(&instance, "_flag"),
            Some(Value::Bool(true))
        )))
    }

    pub(super) fn builtin_thread_condition_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "Condition.__init__() expects optional lock",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Condition.__init__")?;
        let lock_value = match args.pop() {
            Some(value) if !matches!(value, Value::None) => value,
            _ => self.call_builtin(BuiltinFunction::ThreadRLock, Vec::new(), HashMap::new())?,
        };
        Self::instance_attr_set(&instance, "_lock", lock_value)?;
        Self::instance_attr_set(&instance, "_locked", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_condition_acquire(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if kwargs
            .keys()
            .any(|key| key != "blocking" && key != "timeout")
            || args.is_empty()
            || args.len() > 3
        {
            return Err(RuntimeError::new(
                "Condition.acquire() got unexpected arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Condition.acquire")?;
        if let Some(timeout) = kwargs.get("timeout").cloned() {
            let timeout = value_to_f64(timeout)?;
            if timeout.is_sign_negative() {
                return Err(RuntimeError::value_error("timeout must be non-negative"));
            }
        }
        if let Some(lock_value) = Self::instance_attr_get(&instance, "_lock") {
            Self::thread_lock_set_state(&lock_value, true);
        }
        Self::instance_attr_set(&instance, "_locked", Value::Bool(true))?;
        Ok(Value::Bool(true))
    }

    pub(super) fn builtin_thread_condition_enter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_thread_condition_acquire(args, kwargs)
    }

    pub(super) fn builtin_thread_condition_notify(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "Condition.notify() expects optional count",
            ));
        }
        let _instance = self.take_bound_instance_arg(&mut args, "Condition.notify")?;
        if let Some(count) = args.first().cloned() {
            let count = value_to_int(count)?;
            if count < 0 {
                return Err(RuntimeError::new("notify count must be non-negative"));
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_condition_notify_all(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "Condition.notify_all() expects no arguments",
            ));
        }
        let _instance = self.take_bound_instance_arg(&mut args, "Condition.notify_all")?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_condition_release(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "Condition.release() expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Condition.release")?;
        if let Some(lock_value) = Self::instance_attr_get(&instance, "_lock") {
            Self::thread_lock_set_state(&lock_value, false);
        }
        Self::instance_attr_set(&instance, "_locked", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_condition_exit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 4 {
            return Err(RuntimeError::new(
                "Condition.__exit__() expects exc_type/exc/tb or no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Condition.__exit__")?;
        if let Some(lock_value) = Self::instance_attr_get(&instance, "_lock") {
            Self::thread_lock_set_state(&lock_value, false);
        }
        Self::instance_attr_set(&instance, "_locked", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_condition_wait(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if kwargs.keys().any(|key| key != "timeout") || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "Condition.wait() expects optional timeout",
            ));
        }
        let _instance = self.take_bound_instance_arg(&mut args, "Condition.wait")?;
        let timeout = kwargs.remove("timeout").or_else(|| args.pop());
        if let Some(timeout) = timeout {
            if !matches!(timeout, Value::None) {
                let timeout = value_to_f64(timeout)?;
                if timeout.is_sign_negative() {
                    return Err(RuntimeError::value_error("timeout must be non-negative"));
                }
            }
        }
        Ok(Value::Bool(true))
    }

    pub(super) fn builtin_thread_semaphore_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 || kwargs.keys().any(|key| key != "value") {
            return Err(RuntimeError::new(
                "Semaphore.__init__() expects optional value",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Semaphore.__init__")?;
        let value = kwargs
            .remove("value")
            .or_else(|| args.pop())
            .unwrap_or(Value::Int(1));
        let value = value_to_int(value)?;
        if value < 0 {
            return Err(RuntimeError::new("semaphore initial value must be >= 0"));
        }
        Self::instance_attr_set(&instance, "_value", Value::Int(value))?;
        // Plain Semaphore has no upper release bound; BoundedSemaphore sets this explicitly.
        Self::instance_attr_set(&instance, "_bound", Value::Int(i64::MAX))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_semaphore_acquire(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if kwargs
            .keys()
            .any(|key| key != "blocking" && key != "timeout")
            || args.is_empty()
            || args.len() > 3
        {
            return Err(RuntimeError::new(
                "Semaphore.acquire() got unexpected arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Semaphore.acquire")?;
        let _blocking = kwargs
            .get("blocking")
            .cloned()
            .or_else(|| args.first().cloned())
            .map(|value| is_truthy(&value))
            .unwrap_or(true);
        let current = match Self::instance_attr_get(&instance, "_value") {
            Some(Value::Int(value)) => value,
            _ => 0,
        };
        if current > 0 {
            Self::instance_attr_set(&instance, "_value", Value::Int(current - 1))?;
            return Ok(Value::Bool(true));
        }
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_thread_semaphore_release(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 || kwargs.keys().any(|key| key != "n") {
            return Err(RuntimeError::new(
                "Semaphore.release() expects optional n argument",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Semaphore.release")?;
        let increment = kwargs
            .remove("n")
            .or_else(|| args.pop())
            .unwrap_or(Value::Int(1));
        let increment = value_to_int(increment)?;
        if increment < 0 {
            return Err(RuntimeError::new("release increment must be >= 0"));
        }
        let current = match Self::instance_attr_get(&instance, "_value") {
            Some(Value::Int(value)) => value,
            _ => 0,
        };
        let bound = match Self::instance_attr_get(&instance, "_bound") {
            Some(Value::Int(value)) => value,
            _ => i64::MAX,
        };
        let new_value = current.saturating_add(increment);
        if bound != i64::MAX && new_value > bound {
            return Err(RuntimeError::new("Semaphore released too many times"));
        }
        Self::instance_attr_set(&instance, "_value", Value::Int(new_value))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_bounded_semaphore_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_thread_semaphore_init(args.clone(), kwargs.clone())?;
        let instance = self.take_bound_instance_arg(&mut args, "BoundedSemaphore.__init__")?;
        let value_arg = kwargs
            .remove("value")
            .or_else(|| args.pop())
            .unwrap_or(Value::Int(1));
        let bound = value_to_int(value_arg)?;
        Self::instance_attr_set(&instance, "_bound", Value::Int(bound))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_barrier_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 4 {
            return Err(RuntimeError::new(
                "Barrier.__init__() expects parties and optional action/timeout",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Barrier.__init__")?;
        let parties_value = args.remove(0);
        let parties = value_to_int(parties_value)?;
        if parties <= 0 {
            return Err(RuntimeError::new("parties must be > 0"));
        }
        let action = kwargs
            .remove("action")
            .or_else(|| {
                if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            })
            .unwrap_or(Value::None);
        let timeout = kwargs
            .remove("timeout")
            .or_else(|| {
                if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            })
            .unwrap_or(Value::None);
        if timeout != Value::None {
            let timeout = value_to_f64(timeout)?;
            if timeout.is_sign_negative() {
                return Err(RuntimeError::value_error("timeout must be non-negative"));
            }
        }
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "Barrier.__init__() got unexpected arguments",
            ));
        }
        Self::instance_attr_set(&instance, "_parties", Value::Int(parties))?;
        Self::instance_attr_set(&instance, "_action", action)?;
        Self::instance_attr_set(&instance, "_n_waiting", Value::Int(0))?;
        Self::instance_attr_set(&instance, "_broken", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_barrier_abort(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("Barrier.abort() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Barrier.abort")?;
        Self::instance_attr_set(&instance, "_broken", Value::Bool(true))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_barrier_reset(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("Barrier.reset() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Barrier.reset")?;
        Self::instance_attr_set(&instance, "_broken", Value::Bool(false))?;
        Self::instance_attr_set(&instance, "_n_waiting", Value::Int(0))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_thread_barrier_wait(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if kwargs.keys().any(|key| key != "timeout") || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("Barrier.wait() expects optional timeout"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Barrier.wait")?;
        let timeout = kwargs.remove("timeout").or_else(|| args.pop());
        if let Some(timeout) = timeout {
            let timeout = value_to_f64(timeout)?;
            if timeout.is_sign_negative() {
                return Err(RuntimeError::value_error("timeout must be non-negative"));
            }
        }
        if matches!(
            Self::instance_attr_get(&instance, "_broken"),
            Some(Value::Bool(true))
        ) {
            return Err(RuntimeError::new("barrier is broken"));
        }
        let parties = match Self::instance_attr_get(&instance, "_parties") {
            Some(Value::Int(value)) => value.max(1),
            _ => 1,
        };
        let waiting = match Self::instance_attr_get(&instance, "_n_waiting") {
            Some(Value::Int(value)) => value,
            _ => 0,
        };
        let next_waiting = waiting + 1;
        if next_waiting >= parties {
            Self::instance_attr_set(&instance, "_n_waiting", Value::Int(0))?;
            return Ok(Value::Int(0));
        }
        Self::instance_attr_set(&instance, "_n_waiting", Value::Int(next_waiting))?;
        Ok(Value::Int(next_waiting))
    }

    pub(super) fn builtin_signal_signal(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("signal() expects signum and handler"));
        }
        let signum = value_to_int(args.remove(0))?;
        let handler = args.remove(0);
        let previous = self
            .signal_handlers
            .insert(signum, handler)
            .unwrap_or_else(|| {
                if signum == SIGNAL_SIGINT {
                    Value::Builtin(BuiltinFunction::SignalDefaultIntHandler)
                } else {
                    Value::Int(SIGNAL_DEFAULT)
                }
            });
        Ok(previous)
    }

    pub(super) fn builtin_signal_getsignal(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("getsignal() expects one signum argument"));
        }
        let signum = value_to_int(args.remove(0))?;
        Ok(self
            .signal_handlers
            .get(&signum)
            .cloned()
            .unwrap_or_else(|| {
                if signum == SIGNAL_SIGINT {
                    Value::Builtin(BuiltinFunction::SignalDefaultIntHandler)
                } else {
                    Value::Int(SIGNAL_DEFAULT)
                }
            }))
    }

    pub(super) fn builtin_signal_raise_signal(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "raise_signal() expects one signum argument",
            ));
        }
        let signum = value_to_int(args.remove(0))?;
        let handler = self
            .signal_handlers
            .get(&signum)
            .cloned()
            .unwrap_or(Value::Int(SIGNAL_DEFAULT));
        match handler {
            Value::Int(code) if code == SIGNAL_IGNORE => Ok(Value::None),
            Value::Int(code) if code == SIGNAL_DEFAULT => {
                if signum == SIGNAL_SIGINT {
                    Err(RuntimeError::new("KeyboardInterrupt"))
                } else {
                    Ok(Value::None)
                }
            }
            callable => {
                match self.call_internal(
                    callable,
                    vec![Value::Int(signum), Value::None],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(_) => Ok(Value::None),
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(RuntimeError::new("signal handler raised"))
                    }
                }
            }
        }
    }

    pub(super) fn builtin_signal_default_int_handler(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "default_int_handler() expects optional signum and frame",
            ));
        }
        Err(RuntimeError::new("KeyboardInterrupt"))
    }

    pub(super) fn builtin_tokenize_tokenizer_iter(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::type_error(
                "TokenizerIter() expects one source callable",
            ));
        }
        let source = args.remove(0);
        if !self.is_callable_value(&source) {
            return Err(RuntimeError::type_error(
                "TokenizerIter() source must be callable",
            ));
        }
        let encoding = match kwargs.remove("encoding") {
            Some(Value::Str(name)) => Some(name),
            Some(Value::None) => None,
            Some(_) => {
                return Err(RuntimeError::type_error(
                    "TokenizerIter() encoding must be str or None",
                ));
            }
            None => None,
        };
        let _extra_tokens = match kwargs.remove("extra_tokens") {
            Some(value) => self.truthy_from_value(&value)?,
            None => false,
        };
        if let Some(unexpected) = kwargs.keys().next().cloned() {
            return Err(RuntimeError::type_error(format!(
                "TokenizerIter() got an unexpected keyword argument '{unexpected}'"
            )));
        }

        let mut source_lines = Vec::new();
        loop {
            let next_line = match self.call_internal(source.clone(), Vec::new(), HashMap::new()) {
                Ok(InternalCallOutcome::Value(value)) => value,
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    let err =
                        self.runtime_error_from_active_exception("TokenizerIter source failed");
                    if runtime_error_matches_exception(&err, "StopIteration") {
                        self.clear_active_exception();
                        break;
                    }
                    return Err(err);
                }
                Err(err) => {
                    if runtime_error_matches_exception(&err, "StopIteration") {
                        self.clear_active_exception();
                        break;
                    }
                    return Err(err);
                }
            };

            let line = match next_line {
                Value::Str(text) => text,
                Value::Bytes(_) | Value::ByteArray(_) => {
                    let bytes = bytes_like_from_value(next_line)?;
                    if bytes.is_empty() {
                        break;
                    }
                    let encoding_name = encoding.as_deref().unwrap_or("utf-8");
                    decode_text_bytes(&bytes, encoding_name, "strict")?
                }
                _ => {
                    return Err(RuntimeError::type_error(
                        "readline() should return str or bytes",
                    ));
                }
            };
            if line.is_empty() {
                break;
            }
            source_lines.push(line);
        }

        let source_text = source_lines.concat();
        let mut lexer = crate::parser::lexer::Lexer::new(&source_text);
        let tokens = lexer.tokenize().map_err(|err| {
            RuntimeError::with_exception(
                "SyntaxError",
                Some(format!(
                    "{} (line {}, column {})",
                    err.message, err.line, err.column
                )),
            )
        })?;

        let physical_lines = if source_text.is_empty() {
            vec![String::new()]
        } else {
            source_text
                .split_inclusive('\n')
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
        };
        let mut token_rows = Vec::with_capacity(tokens.len());
        for token in tokens {
            let token_type = match &token.kind {
                crate::parser::token::TokenKind::EndMarker => 0,
                crate::parser::token::TokenKind::Name
                | crate::parser::token::TokenKind::Keyword(_) => 1,
                crate::parser::token::TokenKind::Number => 2,
                crate::parser::token::TokenKind::String
                | crate::parser::token::TokenKind::Bytes
                | crate::parser::token::TokenKind::FString
                | crate::parser::token::TokenKind::TemplateString => 3,
                crate::parser::token::TokenKind::Newline => 4,
                crate::parser::token::TokenKind::Indent => 5,
                crate::parser::token::TokenKind::Dedent => 6,
                crate::parser::token::TokenKind::LParen => 7,
                crate::parser::token::TokenKind::RParen => 8,
                crate::parser::token::TokenKind::LBracket => 9,
                crate::parser::token::TokenKind::RBracket => 10,
                crate::parser::token::TokenKind::Colon => 11,
                crate::parser::token::TokenKind::Comma => 12,
                crate::parser::token::TokenKind::Semicolon => 13,
                crate::parser::token::TokenKind::Plus => 14,
                crate::parser::token::TokenKind::Minus => 15,
                crate::parser::token::TokenKind::Star => 16,
                crate::parser::token::TokenKind::Slash => 17,
                crate::parser::token::TokenKind::Pipe => 18,
                crate::parser::token::TokenKind::Ampersand => 19,
                crate::parser::token::TokenKind::Less => 20,
                crate::parser::token::TokenKind::Greater => 21,
                crate::parser::token::TokenKind::Equal => 22,
                crate::parser::token::TokenKind::Dot => 23,
                crate::parser::token::TokenKind::Percent => 24,
                crate::parser::token::TokenKind::LBrace => 25,
                crate::parser::token::TokenKind::RBrace => 26,
                crate::parser::token::TokenKind::DoubleEqual => 27,
                crate::parser::token::TokenKind::NotEqual => 28,
                crate::parser::token::TokenKind::LessEqual => 29,
                crate::parser::token::TokenKind::GreaterEqual => 30,
                crate::parser::token::TokenKind::Tilde => 31,
                crate::parser::token::TokenKind::Caret => 32,
                crate::parser::token::TokenKind::LeftShift => 33,
                crate::parser::token::TokenKind::RightShift => 34,
                crate::parser::token::TokenKind::DoubleStar => 35,
                crate::parser::token::TokenKind::PlusEqual => 36,
                crate::parser::token::TokenKind::MinusEqual => 37,
                crate::parser::token::TokenKind::StarEqual => 38,
                crate::parser::token::TokenKind::SlashEqual => 39,
                crate::parser::token::TokenKind::PercentEqual => 40,
                crate::parser::token::TokenKind::AmpersandEqual => 41,
                crate::parser::token::TokenKind::PipeEqual => 42,
                crate::parser::token::TokenKind::CaretEqual => 43,
                crate::parser::token::TokenKind::LeftShiftEqual => 44,
                crate::parser::token::TokenKind::RightShiftEqual => 45,
                crate::parser::token::TokenKind::DoubleStarEqual => 46,
                crate::parser::token::TokenKind::DoubleSlash => 47,
                crate::parser::token::TokenKind::DoubleSlashEqual => 48,
                crate::parser::token::TokenKind::At => 49,
                crate::parser::token::TokenKind::AtEqual => 50,
                crate::parser::token::TokenKind::Arrow => 51,
                crate::parser::token::TokenKind::Ellipsis => 52,
                crate::parser::token::TokenKind::ColonEqual => 53,
            };

            let line_text = physical_lines
                .get(token.line.saturating_sub(1))
                .cloned()
                .unwrap_or_default();
            let start_col = token.column.saturating_sub(1);
            let end_col = start_col.saturating_add(token.lexeme.chars().count());
            token_rows.push(self.heap.alloc_tuple(vec![
                Value::Int(token_type),
                Value::Str(token.lexeme),
                self.heap.alloc_tuple(vec![
                    Value::Int(token.line as i64),
                    Value::Int(start_col as i64),
                ]),
                self.heap.alloc_tuple(vec![
                    Value::Int(token.line as i64),
                    Value::Int(end_col as i64),
                ]),
                Value::Str(line_text),
            ]));
        }

        let list = match self.heap.alloc_list(token_rows) {
            Value::List(obj) => obj,
            _ => unreachable!(),
        };
        Ok(Value::Iterator(self.heap.alloc(Object::Iterator(
            IteratorObject {
                kind: IteratorKind::List(list),
                index: 0,
            },
        ))))
    }

    fn locale_module_ref(&self) -> Result<ObjRef, RuntimeError> {
        let Some(module) = self.modules.get("_locale").cloned() else {
            return Err(RuntimeError::module_not_found_error(
                "module '_locale' not found",
            ));
        };
        Ok(module)
    }

    pub(super) fn builtin_locale_setlocale(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::type_error(
                "setlocale() takes from 1 to 2 positional arguments but more were given",
            ));
        }
        let category = if let Some(value) = kwargs.remove("category") {
            if !args.is_empty() {
                return Err(RuntimeError::type_error(
                    "setlocale() got multiple values for argument 'category'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::type_error(
                "setlocale() missing required argument 'category' (pos 1)",
            ));
        };
        let locale_value = if let Some(value) = kwargs.remove("locale") {
            if !args.is_empty() {
                return Err(RuntimeError::type_error(
                    "setlocale() got multiple values for argument 'locale'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            Value::None
        };
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::type_error(
                "setlocale() got unexpected keyword argument",
            ));
        }

        let category = value_to_int(category)?;
        if !(0..=6).contains(&category) {
            return Err(RuntimeError::with_exception(
                "Error",
                Some("unsupported locale setting".to_string()),
            ));
        }

        let module = self.locale_module_ref()?;
        let Object::Module(module_data) = &mut *module.kind_mut() else {
            return Err(RuntimeError::new("module '_locale' is invalid"));
        };

        let current = module_data
            .globals
            .get("_pyrs_current_locale")
            .cloned()
            .unwrap_or_else(|| Value::Str("C".to_string()));

        match locale_value {
            Value::None => Ok(current),
            Value::Str(text) => {
                let normalized = if text.is_empty() {
                    self.host
                        .env_var("LC_ALL")
                        .or_else(|| self.host.env_var("LC_CTYPE"))
                        .or_else(|| self.host.env_var("LANG"))
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "C".to_string())
                } else {
                    text
                };
                module_data.globals.insert(
                    "_pyrs_current_locale".to_string(),
                    Value::Str(normalized.clone()),
                );
                Ok(Value::Str(normalized))
            }
            _ => Err(RuntimeError::type_error(
                "setlocale() argument 2 must be str or None",
            )),
        }
    }

    pub(super) fn builtin_locale_localeconv(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::type_error("localeconv() takes no arguments"));
        }
        Ok(self.heap.alloc_dict(vec![
            (
                Value::Str("decimal_point".to_string()),
                Value::Str(".".to_string()),
            ),
            (
                Value::Str("thousands_sep".to_string()),
                Value::Str(String::new()),
            ),
            (
                Value::Str("grouping".to_string()),
                self.heap.alloc_list(Vec::new()),
            ),
            (
                Value::Str("int_curr_symbol".to_string()),
                Value::Str(String::new()),
            ),
            (
                Value::Str("currency_symbol".to_string()),
                Value::Str(String::new()),
            ),
            (
                Value::Str("mon_decimal_point".to_string()),
                Value::Str(String::new()),
            ),
            (
                Value::Str("mon_thousands_sep".to_string()),
                Value::Str(String::new()),
            ),
            (
                Value::Str("mon_grouping".to_string()),
                self.heap.alloc_list(Vec::new()),
            ),
            (
                Value::Str("positive_sign".to_string()),
                Value::Str(String::new()),
            ),
            (
                Value::Str("negative_sign".to_string()),
                Value::Str(String::new()),
            ),
            (Value::Str("int_frac_digits".to_string()), Value::Int(127)),
            (Value::Str("frac_digits".to_string()), Value::Int(127)),
            (Value::Str("p_cs_precedes".to_string()), Value::Int(127)),
            (Value::Str("p_sep_by_space".to_string()), Value::Int(127)),
            (Value::Str("n_cs_precedes".to_string()), Value::Int(127)),
            (Value::Str("n_sep_by_space".to_string()), Value::Int(127)),
            (Value::Str("p_sign_posn".to_string()), Value::Int(127)),
            (Value::Str("n_sign_posn".to_string()), Value::Int(127)),
        ]))
    }

    pub(super) fn builtin_locale_strxfrm(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "strxfrm() takes exactly one argument",
            ));
        }
        let Value::Str(text) = args.remove(0) else {
            return Err(RuntimeError::type_error("strxfrm() argument must be str"));
        };
        // Locale-aware transform is host-dependent; keep deterministic passthrough
        // semantics until host collation substrate is wired.
        Ok(Value::Str(text))
    }

    pub(super) fn builtin_locale_strcoll(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "strcoll() takes exactly two arguments",
            ));
        }
        let Value::Str(left) = args.remove(0) else {
            return Err(RuntimeError::type_error("strcoll() arguments must be str"));
        };
        let Value::Str(right) = args.remove(0) else {
            return Err(RuntimeError::type_error("strcoll() arguments must be str"));
        };
        let result = match left.cmp(&right) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        Ok(Value::Int(result))
    }

    pub(super) fn builtin_locale_nl_langinfo(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "nl_langinfo() takes exactly one argument",
            ));
        }
        let item = value_to_int(args.remove(0))?;
        let module = self.locale_module_ref()?;
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("module '_locale' is invalid"));
        };
        let codeset = module_data
            .globals
            .get("CODESET")
            .and_then(|value| match value {
                Value::Int(v) => Some(*v),
                _ => None,
            })
            .unwrap_or(14);
        if item == codeset {
            return Ok(Value::Str("utf-8".to_string()));
        }
        Err(RuntimeError::with_exception(
            "ValueError",
            Some("unsupported langinfo constant".to_string()),
        ))
    }

    pub(super) fn builtin_sysconfig_get_data_name(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "_get_sysconfigdata_name() expects no arguments",
            ));
        }
        let mut names = self
            .modules
            .keys()
            .filter(|name| name.starts_with("_sysconfigdata__"))
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        let preferred = names
            .iter()
            .find(|name| name.ends_with('_'))
            .cloned()
            .or_else(|| names.first().cloned())
            .unwrap_or_else(|| "_sysconfigdata__pyrs".to_string());
        Ok(Value::Str(preferred))
    }

    pub(super) fn builtin_osx_support_customize_config_vars(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "customize_config_vars() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "customize_config_vars() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        // CPython returns the same mapping object after environment-aware
        // macOS-specific adjustments. pyrs keeps this as a pass-through
        // fallback while preserving call contract and identity.
        Ok(args.remove(0))
    }

    pub(super) fn socket_class_ref(&self) -> Result<ObjRef, RuntimeError> {
        let Some(module) = self.modules.get("_socket").cloned() else {
            return Err(RuntimeError::module_not_found_error(
                "module '_socket' not found",
            ));
        };
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("module '_socket' is invalid"));
        };
        let Some(Value::Class(class_ref)) = module_data.globals.get("socket").cloned() else {
            return Err(RuntimeError::new("module '_socket' has no socket class"));
        };
        Ok(class_ref)
    }

    pub(super) fn alloc_socket_instance_with_fd(&self, fd: i64) -> Result<Value, RuntimeError> {
        let class_ref = self.socket_class_ref()?;
        let instance = match self.heap.alloc_instance(InstanceObject::new(class_ref)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data
                .attrs
                .insert("_fd".to_string(), Value::Int(fd));
            instance_data
                .attrs
                .insert("_closed".to_string(), Value::Bool(fd < 0));
        }
        Ok(Value::Instance(instance))
    }

    fn socket_instance_fd(instance: &ObjRef) -> Result<i32, RuntimeError> {
        match Self::instance_attr_get(instance, "_fd") {
            Some(Value::Int(fd)) if fd >= 0 => Ok(fd as i32),
            _ => Err(RuntimeError::os_error("[Errno 9] Bad file descriptor")),
        }
    }

    pub(super) fn builtin_socket_socketpair(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 3 {
            return Err(RuntimeError::new(
                "socketpair() expects optional family, type, and proto",
            ));
        }
        let family = kwargs
            .remove("family")
            .or_else(|| {
                if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            })
            .unwrap_or(Value::Int(1));
        let sock_type = kwargs
            .remove("type")
            .or_else(|| {
                if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            })
            .unwrap_or(Value::Int(1));
        let proto = kwargs
            .remove("proto")
            .or_else(|| {
                if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            })
            .unwrap_or(Value::Int(0));
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("socketpair() got unexpected arguments"));
        }
        let family = value_to_int(family)?;
        let sock_type = value_to_int(sock_type)?;
        let proto = value_to_int(proto)?;

        #[cfg(unix)]
        {
            let mut fds = [-1i32; 2];
            // SAFETY: pointers are valid for exactly two file descriptors.
            let status = unsafe {
                libc::socketpair(
                    family as libc::c_int,
                    sock_type as libc::c_int,
                    proto as libc::c_int,
                    fds.as_mut_ptr(),
                )
            };
            if status != 0 {
                let err = std::io::Error::last_os_error();
                return Err(RuntimeError::os_error(format!(
                    "socketpair() failed: {err}"
                )));
            }
            let left = self.alloc_socket_instance_with_fd(fds[0] as i64)?;
            let right = self.alloc_socket_instance_with_fd(fds[1] as i64)?;
            for socket in [&left, &right] {
                if let Value::Instance(instance) = socket {
                    Self::instance_attr_set(instance, "_family", Value::Int(family))?;
                    Self::instance_attr_set(instance, "_type", Value::Int(sock_type))?;
                    Self::instance_attr_set(instance, "_proto", Value::Int(proto))?;
                }
            }
            return Ok(self.heap.alloc_tuple(vec![left, right]));
        }
        #[cfg(not(unix))]
        {
            let _ = (family, sock_type, proto);
            Err(RuntimeError::new(
                "socketpair() is not supported on this platform",
            ))
        }
    }

    pub(super) fn builtin_socket_gethostname(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("gethostname() expects no arguments"));
        }
        let hostname = self
            .host
            .env_var("HOSTNAME")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "localhost".to_string());
        Ok(Value::Str(hostname))
    }

    pub(super) fn builtin_socket_gethostbyname(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "gethostbyname() expects one host argument",
            ));
        }
        let host = match args.remove(0) {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("host must be a string")),
        };
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(Value::Str(ip.to_string()));
        }
        let addrs = (host.as_str(), 0u16)
            .to_socket_addrs()
            .map_err(|err| RuntimeError::new(format!("name resolution failed: {err}")))?;
        if let Some(ip) = addrs.into_iter().find_map(|addr| match addr {
            SocketAddr::V4(v4) => Some(v4.ip().to_string()),
            SocketAddr::V6(_) => None,
        }) {
            return Ok(Value::Str(ip));
        }
        Err(RuntimeError::new("name resolution failed"))
    }

    pub(super) fn builtin_socket_getaddrinfo(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 6 {
            return Err(RuntimeError::new(
                "getaddrinfo() expects host, port and optional hints",
            ));
        }

        let host = args.remove(0);
        let port = args.remove(0);
        let family_hint = if let Some(value) = args.first().cloned() {
            value_to_int(value)?
        } else {
            0
        };
        let socktype_hint = if let Some(value) = args.get(1).cloned() {
            value_to_int(value)?
        } else {
            0
        };
        let proto_hint = if let Some(value) = args.get(2).cloned() {
            value_to_int(value)?
        } else {
            0
        };

        let host_name = match host {
            Value::None => "0.0.0.0".to_string(),
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("host must be string or None")),
        };
        let port_value = match port {
            Value::Int(value) => value,
            Value::Bool(value) => {
                if value {
                    1
                } else {
                    0
                }
            }
            Value::Str(value) => value
                .parse::<i64>()
                .map_err(|_| RuntimeError::new("port must be int-compatible"))?,
            _ => return Err(RuntimeError::new("port must be int-compatible")),
        };
        if !(0..=u16::MAX as i64).contains(&port_value) {
            return Err(RuntimeError::new("port out of range"));
        }
        let port = port_value as u16;

        let resolved = (host_name.as_str(), port)
            .to_socket_addrs()
            .map_err(|err| RuntimeError::new(format!("name resolution failed: {err}")))?;
        let mut entries = Vec::new();
        for addr in resolved {
            let family = match addr {
                SocketAddr::V4(_) => 2,
                SocketAddr::V6(_) => 10,
            };
            if family_hint != 0 && family_hint != family {
                continue;
            }
            let sockaddr = match addr {
                SocketAddr::V4(v4) => self.heap.alloc_tuple(vec![
                    Value::Str(v4.ip().to_string()),
                    Value::Int(v4.port() as i64),
                ]),
                SocketAddr::V6(v6) => self.heap.alloc_tuple(vec![
                    Value::Str(v6.ip().to_string()),
                    Value::Int(v6.port() as i64),
                    Value::Int(v6.flowinfo() as i64),
                    Value::Int(v6.scope_id() as i64),
                ]),
            };
            let entry = self.heap.alloc_tuple(vec![
                Value::Int(family),
                Value::Int(if socktype_hint == 0 { 1 } else { socktype_hint }),
                Value::Int(proto_hint),
                Value::Str(String::new()),
                sockaddr,
            ]);
            entries.push(entry);
        }
        Ok(self.heap.alloc_list(entries))
    }

    pub(super) fn builtin_socket_fromfd(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || (args.len() != 3 && args.len() != 4) {
            return Err(RuntimeError::new(
                "fromfd() expects fd, family, type and optional proto",
            ));
        }
        let fd = value_to_int(args.remove(0))?;
        let _family = value_to_int(args.remove(0))?;
        let _sock_type = value_to_int(args.remove(0))?;
        if !args.is_empty() {
            let _ = value_to_int(args.remove(0))?;
        }
        self.alloc_socket_instance_with_fd(fd)
    }

    pub(super) fn builtin_socket_getdefaulttimeout(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "getdefaulttimeout() expects no arguments",
            ));
        }
        match self.socket_default_timeout {
            Some(timeout) => Ok(Value::Float(timeout)),
            None => Ok(Value::None),
        }
    }

    pub(super) fn builtin_socket_setdefaulttimeout(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "setdefaulttimeout() expects one timeout argument",
            ));
        }
        match args.remove(0) {
            Value::None => self.socket_default_timeout = None,
            value => {
                let timeout = value_to_f64(value)?;
                if timeout.is_sign_negative() {
                    return Err(RuntimeError::value_error("timeout must be non-negative"));
                }
                self.socket_default_timeout = Some(timeout);
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_socket_ntohs(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("ntohs() expects one argument"));
        }
        let value = value_to_int(args.remove(0))?;
        let value = u16::try_from(value).map_err(|_| RuntimeError::new("value out of range"))?;
        Ok(Value::Int(u16::from_be(value) as i64))
    }

    pub(super) fn builtin_socket_ntohl(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("ntohl() expects one argument"));
        }
        let value = value_to_int(args.remove(0))?;
        let value = u32::try_from(value).map_err(|_| RuntimeError::new("value out of range"))?;
        Ok(Value::Int(u32::from_be(value) as i64))
    }

    pub(super) fn builtin_socket_htons(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("htons() expects one argument"));
        }
        let value = value_to_int(args.remove(0))?;
        let value = u16::try_from(value).map_err(|_| RuntimeError::new("value out of range"))?;
        Ok(Value::Int(value.to_be() as i64))
    }

    pub(super) fn builtin_socket_htonl(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("htonl() expects one argument"));
        }
        let value = value_to_int(args.remove(0))?;
        let value = u32::try_from(value).map_err(|_| RuntimeError::new("value out of range"))?;
        Ok(Value::Int(value.to_be() as i64))
    }

    pub(super) fn builtin_socket_object_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 5 {
            return Err(RuntimeError::new(
                "socket.__init__() expects optional family, type, proto, fileno",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "socket.__init__")?;
        let family = kwargs
            .remove("family")
            .or_else(|| {
                if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            })
            .unwrap_or(Value::Int(2));
        let sock_type = kwargs
            .remove("type")
            .or_else(|| {
                if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            })
            .unwrap_or(Value::Int(1));
        let proto = kwargs
            .remove("proto")
            .or_else(|| {
                if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            })
            .unwrap_or(Value::Int(0));
        let fileno = kwargs
            .remove("fileno")
            .or_else(|| {
                if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            })
            .unwrap_or(Value::None);
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "socket.__init__() got unexpected arguments",
            ));
        }
        let family = value_to_int(family)?;
        let sock_type = value_to_int(sock_type)?;
        let proto = value_to_int(proto)?;
        let fd = match fileno {
            Value::None => {
                #[cfg(unix)]
                {
                    // SAFETY: libc::socket follows OS contract for parameters.
                    let created = unsafe {
                        libc::socket(
                            family as libc::c_int,
                            sock_type as libc::c_int,
                            proto as libc::c_int,
                        )
                    };
                    if created < 0 {
                        let err = std::io::Error::last_os_error();
                        return Err(RuntimeError::os_error(format!(
                            "socket() creation failed: {err}"
                        )));
                    }
                    created as i64
                }
                #[cfg(not(unix))]
                {
                    let fd = self.next_fd;
                    self.next_fd = self.next_fd.saturating_add(1);
                    fd
                }
            }
            value => value_to_int(value)?,
        };
        Self::instance_attr_set(&instance, "_family", Value::Int(family))?;
        Self::instance_attr_set(&instance, "_type", Value::Int(sock_type))?;
        Self::instance_attr_set(&instance, "_proto", Value::Int(proto))?;
        Self::instance_attr_set(&instance, "_fd", Value::Int(fd))?;
        Self::instance_attr_set(&instance, "_closed", Value::Bool(fd < 0))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_socket_object_setblocking(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "socket.setblocking() expects one argument",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "socket.setblocking")?;
        let blocking = is_truthy(&args.remove(0));
        let fd = Self::socket_instance_fd(&instance)?;
        #[cfg(unix)]
        {
            // SAFETY: fcntl is called with a valid file descriptor from socket object state.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 {
                let err = std::io::Error::last_os_error();
                return Err(RuntimeError::os_error(format!(
                    "setblocking() failed to read flags: {err}"
                )));
            }
            let new_flags = if blocking {
                flags & !libc::O_NONBLOCK
            } else {
                flags | libc::O_NONBLOCK
            };
            // SAFETY: fcntl updates file status flags on a valid descriptor.
            if unsafe { libc::fcntl(fd, libc::F_SETFL, new_flags) } < 0 {
                let err = std::io::Error::last_os_error();
                return Err(RuntimeError::os_error(format!(
                    "setblocking() failed to update flags: {err}"
                )));
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (fd, blocking);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_socket_object_recv(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "socket.recv() expects buffersize and optional flags",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "socket.recv")?;
        let bufsize = value_to_int(args.remove(0))?;
        if bufsize < 0 {
            return Err(RuntimeError::value_error("negative buffersize in recv()"));
        }
        let flags = if let Some(value) = args.pop() {
            value_to_int(value)? as i32
        } else {
            0
        };
        let fd = Self::socket_instance_fd(&instance)?;
        #[cfg(unix)]
        {
            let mut buffer = vec![0u8; bufsize as usize];
            // SAFETY: buffer pointer/length are valid for writes; fd comes from socket state.
            let received = unsafe {
                libc::recv(
                    fd,
                    buffer.as_mut_ptr() as *mut libc::c_void,
                    buffer.len(),
                    flags,
                )
            };
            if received < 0 {
                let err = std::io::Error::last_os_error();
                return Err(match err.kind() {
                    std::io::ErrorKind::WouldBlock => RuntimeError::new("BlockingIOError"),
                    std::io::ErrorKind::Interrupted => RuntimeError::new("InterruptedError"),
                    _ => RuntimeError::os_error(format!("recv() failed: {err}")),
                });
            }
            buffer.truncate(received as usize);
            return Ok(self.heap.alloc_bytes(buffer));
        }
        #[cfg(not(unix))]
        {
            let _ = (fd, flags);
            Err(RuntimeError::new(
                "recv() is not supported on this platform",
            ))
        }
    }

    pub(super) fn builtin_socket_object_send(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "socket.send() expects data and optional flags",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "socket.send")?;
        let payload = bytes_like_from_value(args.remove(0))?;
        let flags = if let Some(value) = args.pop() {
            value_to_int(value)? as i32
        } else {
            0
        };
        let fd = Self::socket_instance_fd(&instance)?;
        #[cfg(unix)]
        {
            // SAFETY: payload buffer pointer/length are valid for reads; fd comes from socket state.
            let sent = unsafe {
                libc::send(
                    fd,
                    payload.as_ptr() as *const libc::c_void,
                    payload.len(),
                    flags,
                )
            };
            if sent < 0 {
                let err = std::io::Error::last_os_error();
                return Err(match err.kind() {
                    std::io::ErrorKind::WouldBlock => RuntimeError::new("BlockingIOError"),
                    std::io::ErrorKind::Interrupted => RuntimeError::new("InterruptedError"),
                    _ => RuntimeError::os_error(format!("send() failed: {err}")),
                });
            }
            return Ok(Value::Int(sent as i64));
        }
        #[cfg(not(unix))]
        {
            let _ = (fd, payload, flags);
            Err(RuntimeError::new(
                "send() is not supported on this platform",
            ))
        }
    }

    pub(super) fn builtin_socket_object_close(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("socket.close() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "socket.close")?;
        #[cfg(unix)]
        if let Some(Value::Int(fd)) = Self::instance_attr_get(&instance, "_fd")
            && fd >= 0
        {
            // SAFETY: closing an owned file descriptor from socket state.
            let _ = unsafe { libc::close(fd as i32) };
        }
        Self::instance_attr_set(&instance, "_closed", Value::Bool(true))?;
        Self::instance_attr_set(&instance, "_fd", Value::Int(-1))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_socket_object_detach(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("socket.detach() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "socket.detach")?;
        let fd = match Self::instance_attr_get(&instance, "_fd") {
            Some(Value::Int(value)) => value,
            _ => -1,
        };
        Self::instance_attr_set(&instance, "_closed", Value::Bool(true))?;
        Self::instance_attr_set(&instance, "_fd", Value::Int(-1))?;
        Ok(Value::Int(fd))
    }

    pub(super) fn builtin_socket_object_fileno(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("socket.fileno() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "socket.fileno")?;
        Ok(match Self::instance_attr_get(&instance, "_fd") {
            Some(Value::Int(value)) => Value::Int(value),
            _ => Value::Int(-1),
        })
    }

    pub(super) fn builtin_scproxy_get_proxy_settings(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "_get_proxy_settings() expects no arguments",
            ));
        }
        Ok(self.heap.alloc_dict(vec![
            (Value::Str("exclude_simple".to_string()), Value::Bool(false)),
            (
                Value::Str("exceptions".to_string()),
                self.heap.alloc_tuple(Vec::new()),
            ),
        ]))
    }

    pub(super) fn builtin_scproxy_get_proxies(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("_get_proxies() expects no arguments"));
        }
        Ok(self.heap.alloc_dict(Vec::new()))
    }

    pub(super) fn uuid_class_ref(&self) -> Result<ObjRef, RuntimeError> {
        let Some(module) = self.modules.get("uuid").cloned() else {
            return Err(RuntimeError::module_not_found_error(
                "module 'uuid' not found",
            ));
        };
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("module 'uuid' is invalid"));
        };
        match module_data.globals.get("UUID") {
            Some(Value::Class(class_ref)) => Ok(class_ref.clone()),
            _ => Err(RuntimeError::new("module 'uuid' missing UUID class")),
        }
    }

    pub(super) fn uuid_value_to_bytes(&self, value: Value) -> Result<[u8; 16], RuntimeError> {
        match value {
            Value::Instance(instance) => match Self::instance_attr_get(&instance, "__uuid_bytes__")
            {
                Some(Value::Bytes(bytes_obj)) => match &*bytes_obj.kind() {
                    Object::Bytes(bytes) if bytes.len() == 16 => {
                        let mut out = [0u8; 16];
                        out.copy_from_slice(bytes);
                        Ok(out)
                    }
                    _ => Err(RuntimeError::new("UUID object missing bytes payload")),
                },
                Some(Value::Str(text)) => parse_uuid_like_string(&text),
                _ => Err(RuntimeError::new("expected UUID instance")),
            },
            Value::Str(text) => parse_uuid_like_string(&text),
            Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(bytes) if bytes.len() == 16 => {
                    let mut out = [0u8; 16];
                    out.copy_from_slice(bytes);
                    Ok(out)
                }
                _ => Err(RuntimeError::new("UUID bytes argument must be length 16")),
            },
            _ => Err(RuntimeError::new("expected UUID-compatible value")),
        }
    }

    pub(super) fn make_uuid_instance_from_bytes(
        &self,
        mut bytes: [u8; 16],
    ) -> Result<Value, RuntimeError> {
        apply_uuid_variant(&mut bytes);
        let class_ref = self.uuid_class_ref()?;
        let instance = match self.heap.alloc_instance(InstanceObject::new(class_ref)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        self.populate_uuid_instance(&instance, bytes)?;
        Ok(Value::Instance(instance))
    }

    pub(super) fn populate_uuid_instance(
        &self,
        instance: &ObjRef,
        bytes: [u8; 16],
    ) -> Result<(), RuntimeError> {
        let text = format_uuid_hyphenated(bytes);
        let hex = format_uuid_hex(bytes);
        let bytes_value = self.heap.alloc_bytes(bytes.to_vec());
        let fields = self.heap.alloc_tuple(vec![
            Value::Int(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64),
            Value::Int(u16::from_be_bytes([bytes[4], bytes[5]]) as i64),
            Value::Int(u16::from_be_bytes([bytes[6], bytes[7]]) as i64),
            Value::Int(bytes[8] as i64),
            Value::Int(bytes[9] as i64),
            Value::Int(
                (((bytes[10] as u64) << 40)
                    | ((bytes[11] as u64) << 32)
                    | ((bytes[12] as u64) << 24)
                    | ((bytes[13] as u64) << 16)
                    | ((bytes[14] as u64) << 8)
                    | (bytes[15] as u64)) as i64,
            ),
        ]);
        Self::instance_attr_set(instance, "__uuid_bytes__", bytes_value.clone())?;
        Self::instance_attr_set(instance, "bytes", bytes_value)?;
        Self::instance_attr_set(instance, "hex", Value::Str(hex.clone()))?;
        Self::instance_attr_set(instance, "urn", Value::Str(format!("urn:uuid:{text}")))?;
        Self::instance_attr_set(instance, "__str__", Value::Str(text))?;
        Self::instance_attr_set(instance, "fields", fields)?;
        Self::instance_attr_set(
            instance,
            "version",
            Value::Int(((bytes[6] >> 4) & 0x0f) as i64),
        )?;
        Self::instance_attr_set(
            instance,
            "variant",
            Value::Str("specified in RFC 4122".to_string()),
        )?;
        Self::instance_attr_set(instance, "is_safe", Value::None)?;
        Ok(())
    }

    pub(super) fn builtin_uuid_class_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("UUID.__init__() missing instance"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "UUID.__init__")?;
        let source = kwargs
            .remove("hex")
            .or_else(|| kwargs.remove("bytes"))
            .or_else(|| kwargs.remove("int"))
            .or_else(|| {
                if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            });
        let version = kwargs.remove("version").or_else(|| {
            if !args.is_empty() {
                Some(args.remove(0))
            } else {
                None
            }
        });
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "UUID.__init__() got unexpected arguments",
            ));
        }
        let mut bytes = if let Some(source) = source {
            match source {
                Value::Int(value) => {
                    if value < 0 {
                        return Err(RuntimeError::new("UUID int value must be non-negative"));
                    }
                    let mut out = [0u8; 16];
                    out[8..].copy_from_slice(&(value as u64).to_be_bytes());
                    out
                }
                other => self.uuid_value_to_bytes(other)?,
            }
        } else {
            uuid_random_bytes(&mut self.random)
        };
        if let Some(version) = version {
            let version = value_to_int(version)?;
            if !(1..=8).contains(&version) {
                return Err(RuntimeError::new("UUID version must be in [1, 8]"));
            }
            apply_uuid_version(&mut bytes, version as u8);
        } else {
            apply_uuid_variant(&mut bytes);
        }
        self.populate_uuid_instance(&instance, bytes)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_uuid_getnode(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("uuid.getnode() expects no arguments"));
        }
        Ok(Value::Int(self.uuid_node_from_host()))
    }

    pub(super) fn builtin_uuid1(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if kwargs.keys().any(|key| key != "node" && key != "clock_seq") || args.len() > 2 {
            return Err(RuntimeError::new(
                "uuid1() expects optional node and clock_seq",
            ));
        }
        let node = if let Some(value) = kwargs
            .get("node")
            .cloned()
            .or_else(|| args.first().cloned())
        {
            value_to_int(value)?
        } else {
            self.uuid_node_from_host()
        } as u64
            & 0x0000_FFFF_FFFF_FFFF;
        let clock_seq = if let Some(value) = kwargs
            .get("clock_seq")
            .cloned()
            .or_else(|| args.get(1).cloned())
        {
            value_to_int(value)? as u16
        } else {
            (self.random.next_u32() as u16) & 0x3fff
        };
        let timestamp = uuid_timestamp_100ns_since_gregorian()?;
        let mut bytes = [0u8; 16];
        let time_low = (timestamp & 0xffff_ffff) as u32;
        let time_mid = ((timestamp >> 32) & 0xffff) as u16;
        let time_hi = ((timestamp >> 48) & 0x0fff) as u16;
        bytes[0..4].copy_from_slice(&time_low.to_be_bytes());
        bytes[4..6].copy_from_slice(&time_mid.to_be_bytes());
        bytes[6..8].copy_from_slice(&(time_hi | (1 << 12)).to_be_bytes());
        bytes[8] = ((clock_seq >> 8) as u8 & 0x3f) | 0x80;
        bytes[9] = (clock_seq & 0xff) as u8;
        bytes[10..16].copy_from_slice(&node.to_be_bytes()[2..]);
        self.make_uuid_instance_from_bytes(bytes)
    }

    pub(super) fn builtin_uuid3(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "uuid3() expects namespace and name arguments",
            ));
        }
        let namespace = self.uuid_value_to_bytes(args.remove(0))?;
        let name = match args.remove(0) {
            Value::Str(text) => text.into_bytes(),
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(bytes) => bytes.clone(),
                _ => return Err(RuntimeError::type_error("name must be str or bytes")),
            },
            _ => return Err(RuntimeError::type_error("name must be str or bytes")),
        };
        let mut bytes = uuid_hash_mix_bytes(3, namespace, &name);
        apply_uuid_version(&mut bytes, 3);
        self.make_uuid_instance_from_bytes(bytes)
    }

    pub(super) fn builtin_uuid4(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("uuid4() expects no arguments"));
        }
        let mut bytes = uuid_random_bytes(&mut self.random);
        apply_uuid_version(&mut bytes, 4);
        self.make_uuid_instance_from_bytes(bytes)
    }

    pub(super) fn builtin_uuid5(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "uuid5() expects namespace and name arguments",
            ));
        }
        let namespace = self.uuid_value_to_bytes(args.remove(0))?;
        let name = match args.remove(0) {
            Value::Str(text) => text.into_bytes(),
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(bytes) => bytes.clone(),
                _ => return Err(RuntimeError::type_error("name must be str or bytes")),
            },
            _ => return Err(RuntimeError::type_error("name must be str or bytes")),
        };
        let mut bytes = uuid_hash_mix_bytes(5, namespace, &name);
        apply_uuid_version(&mut bytes, 5);
        self.make_uuid_instance_from_bytes(bytes)
    }

    pub(super) fn builtin_uuid6(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("uuid6() expects no arguments"));
        }
        let timestamp = uuid_timestamp_100ns_since_gregorian()?;
        let mut bytes = [0u8; 16];
        bytes[0] = (timestamp >> 52) as u8;
        bytes[1] = (timestamp >> 44) as u8;
        bytes[2] = (timestamp >> 36) as u8;
        bytes[3] = (timestamp >> 28) as u8;
        bytes[4] = (timestamp >> 20) as u8;
        bytes[5] = (timestamp >> 12) as u8;
        bytes[6] = ((timestamp >> 4) as u8) & 0x0f;
        bytes[7] = ((timestamp & 0x0f) as u8) << 4;
        let rand = self.random.next_u32() as u64;
        bytes[8] = ((rand >> 24) as u8 & 0x3f) | 0x80;
        bytes[9] = (rand >> 16) as u8;
        let node = (self.uuid_node_from_host() as u64) & 0x0000_FFFF_FFFF_FFFF;
        bytes[10..16].copy_from_slice(&node.to_be_bytes()[2..]);
        apply_uuid_version(&mut bytes, 6);
        self.make_uuid_instance_from_bytes(bytes)
    }

    pub(super) fn builtin_uuid7(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("uuid7() expects no arguments"));
        }
        let now = unix_time_now_duration()?;
        let millis = now.as_millis() as u64;
        let mut bytes = [0u8; 16];
        bytes[0] = (millis >> 40) as u8;
        bytes[1] = (millis >> 32) as u8;
        bytes[2] = (millis >> 24) as u8;
        bytes[3] = (millis >> 16) as u8;
        bytes[4] = (millis >> 8) as u8;
        bytes[5] = millis as u8;
        let rand_a = self.random.next_u32() as u16;
        bytes[6] = (rand_a >> 8) as u8;
        bytes[7] = rand_a as u8;
        let mut rand_b = [0u8; 8];
        rand_b[..4].copy_from_slice(&self.random.next_u32().to_be_bytes());
        rand_b[4..].copy_from_slice(&self.random.next_u32().to_be_bytes());
        bytes[8..16].copy_from_slice(&rand_b);
        apply_uuid_version(&mut bytes, 7);
        self.make_uuid_instance_from_bytes(bytes)
    }

    pub(super) fn builtin_uuid8(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new(
                "uuid8() expects up to three integer components",
            ));
        }
        let a = args
            .first()
            .cloned()
            .map(value_to_int)
            .transpose()?
            .unwrap_or(self.random.next_u32() as i64) as u32;
        let b = args
            .get(1)
            .cloned()
            .map(value_to_int)
            .transpose()?
            .unwrap_or(self.random.next_u32() as i64) as u32;
        let c = args
            .get(2)
            .cloned()
            .map(value_to_int)
            .transpose()?
            .unwrap_or(self.random.next_u32() as i64) as u64;
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&a.to_be_bytes());
        bytes[4..8].copy_from_slice(&b.to_be_bytes());
        let lower = c.to_be_bytes();
        bytes[8..16].copy_from_slice(&lower);
        apply_uuid_version(&mut bytes, 8);
        self.make_uuid_instance_from_bytes(bytes)
    }

    pub(super) fn builtin_colorize_can_colorize(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty() || kwargs.keys().any(|key| key != "file") {
            return Err(RuntimeError::new(
                "can_colorize() accepts only optional 'file' keyword",
            ));
        }
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_colorize_get_theme(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty()
            || kwargs
                .keys()
                .any(|key| !matches!(key.as_str(), "force_color" | "force_no_color" | "tty_file"))
        {
            return Err(RuntimeError::new("get_theme() received invalid arguments"));
        }
        let Some(module) = self.modules.get("_colorize").cloned() else {
            return Err(RuntimeError::module_not_found_error(
                "module '_colorize' not found",
            ));
        };
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("module '_colorize' is invalid"));
        };
        Ok(module_data
            .globals
            .get("_theme")
            .cloned()
            .unwrap_or(Value::None))
    }

    pub(super) fn builtin_colorize_get_colors(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty()
            || kwargs
                .keys()
                .any(|key| !matches!(key.as_str(), "colorize" | "file" | "tty_file"))
        {
            return Err(RuntimeError::new("get_colors() received invalid arguments"));
        }
        let Some(module) = self.modules.get("_colorize").cloned() else {
            return Err(RuntimeError::module_not_found_error(
                "module '_colorize' not found",
            ));
        };
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("module '_colorize' is invalid"));
        };
        Ok(module_data
            .globals
            .get("_ansi")
            .cloned()
            .unwrap_or(Value::None))
    }

    pub(super) fn builtin_colorize_set_theme(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let theme = if let Some(value) = kwargs.remove("theme") {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "set_theme() got multiple values for theme",
                ));
            }
            value
        } else if args.len() == 1 {
            args.remove(0)
        } else {
            return Err(RuntimeError::new("set_theme() expects one theme argument"));
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "set_theme() got unexpected keyword arguments",
            ));
        }
        let Some(module) = self.modules.get("_colorize").cloned() else {
            return Err(RuntimeError::module_not_found_error(
                "module '_colorize' not found",
            ));
        };
        let Object::Module(module_data) = &mut *module.kind_mut() else {
            return Err(RuntimeError::new("module '_colorize' is invalid"));
        };
        module_data.globals.insert("_theme".to_string(), theme);
        Ok(Value::None)
    }

    pub(super) fn builtin_colorize_decolor(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("decolor() expects one string argument"));
        }
        let text = match args.remove(0) {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("decolor() expects one string argument")),
        };
        let mut output = text;
        for code in [
            "\u{1b}[0m",
            "\u{1b}[30m",
            "\u{1b}[34m",
            "\u{1b}[36m",
            "\u{1b}[32m",
            "\u{1b}[90m",
            "\u{1b}[35m",
            "\u{1b}[31m",
            "\u{1b}[37m",
            "\u{1b}[33m",
            "\u{1b}[1m",
            "\u{1b}[1;30m",
            "\u{1b}[1;34m",
            "\u{1b}[1;36m",
            "\u{1b}[1;32m",
            "\u{1b}[1;35m",
            "\u{1b}[1;31m",
            "\u{1b}[1;37m",
            "\u{1b}[1;33m",
            "\u{1b}[94m",
            "\u{1b}[96m",
            "\u{1b}[92m",
            "\u{1b}[95m",
            "\u{1b}[91m",
            "\u{1b}[97m",
            "\u{1b}[93m",
            "\u{1b}[40m",
            "\u{1b}[44m",
            "\u{1b}[46m",
            "\u{1b}[42m",
            "\u{1b}[45m",
            "\u{1b}[41m",
            "\u{1b}[47m",
            "\u{1b}[43m",
            "\u{1b}[100m",
            "\u{1b}[104m",
            "\u{1b}[106m",
            "\u{1b}[102m",
            "\u{1b}[105m",
            "\u{1b}[101m",
            "\u{1b}[107m",
            "\u{1b}[103m",
        ] {
            output = output.replace(code, "");
        }
        Ok(Value::Str(output))
    }

    pub(super) fn builtin_colorize_theme_items(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("theme.items() expects no arguments"));
        }
        let theme_section = match args.remove(0) {
            Value::Module(module) => module,
            _ => {
                return Err(RuntimeError::type_error(
                    "theme.items() receiver must be module",
                ));
            }
        };
        let Object::Module(module_data) = &*theme_section.kind() else {
            return Err(RuntimeError::type_error(
                "theme.items() receiver must be module",
            ));
        };
        let mut names = module_data
            .globals
            .keys()
            .filter(|name| !name.starts_with("__") && name.as_str() != "items")
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        let items = names
            .into_iter()
            .map(|name| {
                self.heap.alloc_tuple(vec![
                    Value::Str(name.clone()),
                    module_data
                        .globals
                        .get(&name)
                        .cloned()
                        .unwrap_or(Value::None),
                ])
            })
            .collect::<Vec<_>>();
        Ok(self.heap.alloc_list(items))
    }

    pub(super) fn builtin_warnings_warn(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.warnings_ensure_defaultaction();
        self.warnings_ensure_filters();
        self.warnings_ensure_onceregistry();
        self.warnings_ensure_showwarnmsg();
        if let Some(callable) =
            self.warnings_python_dispatch_callable("warn", BuiltinFunction::WarningsWarn)
        {
            return self.call_warnings_python_callable(
                callable,
                args,
                kwargs,
                "warn() raised exception",
            );
        }
        if kwargs.keys().any(|key| {
            !matches!(
                key.as_str(),
                "message" | "category" | "stacklevel" | "source"
            )
        }) {
            return Err(RuntimeError::new(
                "warn() got an unexpected keyword argument",
            ));
        }
        if args.is_empty() && !kwargs.contains_key("message") {
            return Err(RuntimeError::new("warn() missing message argument"));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_warnings_warn_explicit(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.warnings_ensure_defaultaction();
        self.warnings_ensure_filters();
        self.warnings_ensure_onceregistry();
        self.warnings_ensure_showwarnmsg();
        self.warnings_bless_module_globals_for_warn_explicit(&args, &kwargs)?;
        if let Some(callable) = self.warnings_python_dispatch_callable(
            "warn_explicit",
            BuiltinFunction::WarningsWarnExplicit,
        ) {
            return self.call_warnings_python_callable(
                callable,
                args,
                kwargs,
                "warn_explicit() raised exception",
            );
        }
        if kwargs
            .keys()
            .any(|key| !matches!(key.as_str(), "message" | "category" | "filename" | "lineno"))
        {
            return Err(RuntimeError::new(
                "warn_explicit() got an unexpected keyword argument",
            ));
        }
        if args.len() + kwargs.len() < 4 {
            return Err(RuntimeError::new(
                "warn_explicit() missing required arguments",
            ));
        }
        Ok(Value::None)
    }

    fn warnings_python_dispatch_callable(
        &mut self,
        attr_name: &str,
        recursive_builtin: BuiltinFunction,
    ) -> Option<Value> {
        if let Some(modules_dict) = self.sys_dict_obj("modules")
            && let Some(Value::Module(warnings_module)) =
                dict_get_value(&modules_dict, &Value::Str("warnings".to_string()))
        {
            if let Ok(callable) = self.load_attr_module(&warnings_module, attr_name) {
                if !matches!(callable, Value::Builtin(builtin) if builtin == recursive_builtin) {
                    return Some(callable);
                }
                // CPython's `warnings` module binds `warn`/`warn_explicit` to `_warnings`
                // when available. If that resolves back to this builtin, continue probing
                // `_py_warnings` so we still get the full Python warning machinery.
            }
            if let Ok(set_module) = self.load_attr_module(&warnings_module, "_set_module") {
                let _ = self.call_internal_preserving_caller(
                    set_module,
                    vec![Value::Module(warnings_module.clone())],
                    HashMap::new(),
                );
                self.clear_active_exception();
            }
        }
        if let Ok(module) = self.load_module("_py_warnings")
            && let Ok(callable) = self.load_attr_module(&module, attr_name)
            && !matches!(callable, Value::Builtin(builtin) if builtin == recursive_builtin)
        {
            return Some(callable);
        }
        None
    }

    fn warnings_ensure_defaultaction(&mut self) {
        let Some(modules_dict) = self.sys_dict_obj("modules") else {
            return;
        };
        let Some(Value::Module(warnings_module)) =
            dict_get_value(&modules_dict, &Value::Str("warnings".to_string()))
        else {
            return;
        };
        if let Object::Module(module_data) = &mut *warnings_module.kind_mut() {
            module_data
                .globals
                .entry("defaultaction".to_string())
                .or_insert_with(|| Value::Str("default".to_string()));
        }
    }

    fn warnings_ensure_filters(&mut self) {
        let Some(modules_dict) = self.sys_dict_obj("modules") else {
            return;
        };
        let Some(Value::Module(warnings_module)) =
            dict_get_value(&modules_dict, &Value::Str("warnings".to_string()))
        else {
            return;
        };

        let fallback_from_builtin = self
            .modules
            .get("_warnings")
            .and_then(|module| match &*module.kind() {
                Object::Module(module_data) => module_data.globals.get("filters").cloned(),
                _ => None,
            });

        if let Object::Module(module_data) = &mut *warnings_module.kind_mut() {
            if let Some(filters) = module_data.globals.get("filters").cloned() {
                module_data
                    .globals
                    .insert("__pyrs_warning_filters_fallback__".to_string(), filters);
                return;
            }
            if let Some(saved) = module_data
                .globals
                .get("__pyrs_warning_filters_fallback__")
                .cloned()
            {
                module_data.globals.insert("filters".to_string(), saved);
                return;
            }
            if let Some(filters) = fallback_from_builtin {
                module_data.globals.insert("filters".to_string(), filters);
            }
        }
    }

    fn warnings_ensure_onceregistry(&mut self) {
        let Some(modules_dict) = self.sys_dict_obj("modules") else {
            return;
        };
        let Some(Value::Module(warnings_module)) =
            dict_get_value(&modules_dict, &Value::Str("warnings".to_string()))
        else {
            return;
        };

        let fallback_from_builtin = self
            .modules
            .get("_warnings")
            .and_then(|module| match &*module.kind() {
                Object::Module(module_data) => module_data.globals.get("_onceregistry").cloned(),
                _ => None,
            });

        if let Object::Module(module_data) = &mut *warnings_module.kind_mut() {
            if let Some(onceregistry) = module_data.globals.get("onceregistry").cloned() {
                module_data.globals.insert(
                    "__pyrs_warning_onceregistry_fallback__".to_string(),
                    onceregistry,
                );
                return;
            }
            if let Some(saved) = module_data
                .globals
                .get("__pyrs_warning_onceregistry_fallback__")
                .cloned()
            {
                module_data
                    .globals
                    .insert("onceregistry".to_string(), saved);
                return;
            }
            if let Some(onceregistry) = fallback_from_builtin {
                module_data
                    .globals
                    .insert("onceregistry".to_string(), onceregistry);
            } else {
                module_data
                    .globals
                    .insert("onceregistry".to_string(), self.heap.alloc_dict(Vec::new()));
            }
        }
    }

    fn warnings_ensure_showwarnmsg(&mut self) {
        let Some(modules_dict) = self.sys_dict_obj("modules") else {
            return;
        };
        let Some(Value::Module(warnings_module)) =
            dict_get_value(&modules_dict, &Value::Str("warnings".to_string()))
        else {
            return;
        };
        if let Object::Module(module_data) = &*warnings_module.kind()
            && module_data.globals.contains_key("_showwarnmsg")
        {
            return;
        }

        let fallback_showwarnmsg = if let Ok(filterwarnings_callable) =
            self.load_attr_module(&warnings_module, "filterwarnings")
            && let Value::Function(function_obj) = filterwarnings_callable
            && let Object::Function(function_data) = &*function_obj.kind()
        {
            self.load_attr_module(&function_data.module, "_showwarnmsg")
                .ok()
        } else {
            None
        };

        if let Some(showwarnmsg) = fallback_showwarnmsg
            && let Object::Module(module_data) = &mut *warnings_module.kind_mut()
        {
            module_data
                .globals
                .insert("_showwarnmsg".to_string(), showwarnmsg);
        }
    }

    fn call_warnings_python_callable(
        &mut self,
        callable: Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        error_context: &str,
    ) -> Result<Value, RuntimeError> {
        match self.call_internal_preserving_caller(callable, args, kwargs) {
            Ok(InternalCallOutcome::Value(value)) => Ok(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                self.warnings_normalize_active_exception_traceback();
                let mut err = self.runtime_error_from_active_exception(error_context);
                self.warnings_normalize_runtime_error_traceback(&mut err);
                self.clear_active_exception();
                Err(err)
            }
            Err(mut err) => {
                self.warnings_normalize_runtime_error_traceback(&mut err);
                Err(err)
            }
        }
    }

    fn warnings_warn_explicit_module_globals_arg(
        &self,
        _args: &[Value],
        kwargs: &HashMap<String, Value>,
    ) -> Option<Value> {
        kwargs.get("module_globals").cloned()
    }

    fn warnings_bless_module_globals_for_warn_explicit(
        &mut self,
        args: &[Value],
        kwargs: &HashMap<String, Value>,
    ) -> Result<(), RuntimeError> {
        let Some(module_globals) = self.warnings_warn_explicit_module_globals_arg(args, kwargs)
        else {
            return Ok(());
        };
        if matches!(module_globals, Value::None) {
            return Ok(());
        }
        if !matches!(module_globals, Value::Dict(_)) {
            return Err(RuntimeError::type_error(format!(
                "module_globals must be a dict, not '{}'",
                self.value_type_name_for_error(&module_globals)
            )));
        }
        if self.warnings_bless_my_loader_depth > 0 {
            return Ok(());
        }

        self.warnings_bless_my_loader_depth += 1;
        let bless_result = (|| {
            let external = match self.load_module("importlib._bootstrap_external") {
                Ok(module) => module,
                Err(err)
                    if runtime_error_matches_exception(&err, "ImportError")
                        || runtime_error_matches_exception(&err, "ModuleNotFoundError") =>
                {
                    return Ok(());
                }
                Err(err) => return Err(err),
            };
            let bless = match self.load_attr_module(&external, "_bless_my_loader") {
                Ok(callable) => callable,
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => {
                    return Ok(());
                }
                Err(err) => return Err(err),
            };
            match self.call_internal_preserving_caller(bless, vec![module_globals], HashMap::new())
            {
                Ok(InternalCallOutcome::Value(_)) => Ok(()),
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    self.warnings_normalize_active_exception_traceback();
                    let mut err = self
                        .runtime_error_from_active_exception("_bless_my_loader() raised exception");
                    self.warnings_normalize_runtime_error_traceback(&mut err);
                    Err(err)
                }
                Err(mut err) => {
                    self.warnings_normalize_runtime_error_traceback(&mut err);
                    Err(err)
                }
            }
        })();
        self.warnings_bless_my_loader_depth = self.warnings_bless_my_loader_depth.saturating_sub(1);
        bless_result
    }

    pub(super) fn builtin_testinternalcapi_get_recursion_depth(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "get_recursion_depth() takes no arguments",
            ));
        }
        Ok(Value::Int(self.frames.len().max(1) as i64))
    }

    pub(super) fn builtin_testcapi_exception_print(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::type_error(format!(
                "exception_print() takes from 1 to 2 positional arguments but {} were given",
                args.len()
            )));
        }

        let legacy = if args.len() == 2 {
            is_truthy(&args.remove(1))
        } else {
            false
        };
        let input_exc = args.remove(0);

        let normalized = match &input_exc {
            Value::Exception(exception) => Value::Exception(exception.clone()),
            Value::Instance(instance)
                if self.exception_class_name_for_instance(instance).is_some() =>
            {
                self.normalize_exception_value(input_exc.clone())?
            }
            _ => {
                self.clear_active_exception();
                let rendered = format!(
                    "TypeError: print_exception(): Exception expected for value, {} found",
                    self.value_type_name_for_error(&input_exc)
                );
                let _ = self.call_builtin(
                    BuiltinFunction::SysStderrWrite,
                    vec![Value::Str(format!("{rendered}\n"))],
                    HashMap::new(),
                );
                return Ok(Value::None);
            }
        };

        let traceback = self.load_module("traceback")?;
        let print_exception = self.load_attr_module(&traceback, "print_exception")?;
        let call_args = if legacy {
            let Value::Exception(exception) = &normalized else {
                unreachable!("normalized exception must be exception object");
            };
            let traceback_value =
                if let Some(cached) = exception.attrs.borrow().get("__traceback__").cloned() {
                    cached
                } else {
                    self.traceback_value_from_frames(&exception.traceback_frames)
                };
            vec![
                Value::ExceptionType(exception.name.clone()),
                normalized.clone(),
                traceback_value,
            ]
        } else {
            vec![normalized]
        };
        match self.call_internal(print_exception, call_args, HashMap::new())? {
            InternalCallOutcome::Value(_) => Ok(Value::None),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("exception_print() failed"))
            }
        }
    }

    pub(super) fn builtin_testcapi_config_get(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new(
                "config_get() expects zero or one argument",
            ));
        }
        if args.is_empty() {
            return Ok(self.heap.alloc_dict(vec![(
                Value::Str("code_debug_ranges".to_string()),
                Value::Int(0),
            )]));
        }
        let key = match args.remove(0) {
            Value::Str(key) => key,
            _ => {
                return Err(RuntimeError::type_error("config_get() key must be string"));
            }
        };
        match key.as_str() {
            "code_debug_ranges" => Ok(Value::Int(0)),
            _ => Ok(Value::None),
        }
    }

    pub(super) fn builtin_testcapi_pyobject_vectorcall(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(RuntimeError::type_error(
                "pyobject_vectorcall() expects callable, tuple-or-None, dict-or-None",
            ));
        }
        let callable = args.remove(0);
        let positional_args = match args.remove(0) {
            Value::None => Vec::new(),
            Value::Tuple(tuple) => match &*tuple.kind() {
                Object::Tuple(values) => values.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "pyobject_vectorcall() positional args must be tuple or None",
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::type_error(
                    "pyobject_vectorcall() positional args must be tuple or None",
                ));
            }
        };
        let keyword_args = match args.remove(0) {
            Value::None => HashMap::new(),
            Value::Dict(dict) => match &*dict.kind() {
                Object::Dict(entries) => {
                    let mut collected = HashMap::with_capacity(entries.len());
                    for (key, value) in entries {
                        let Value::Str(name) = key else {
                            return Err(RuntimeError::type_error(
                                "pyobject_vectorcall() keyword names must be strings",
                            ));
                        };
                        collected.insert(name.clone(), value.clone());
                    }
                    collected
                }
                _ => {
                    return Err(RuntimeError::type_error(
                        "pyobject_vectorcall() keyword args must be dict or None",
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::type_error(
                    "pyobject_vectorcall() keyword args must be dict or None",
                ));
            }
        };
        match self.call_internal(callable, positional_args, keyword_args)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("pyobject_vectorcall() failed"))
            }
        }
    }
}
