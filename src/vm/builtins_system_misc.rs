use super::{
    BuiltinFunction, HashMap, InstanceObject, InternalCallOutcome, IpAddr, IteratorKind,
    IteratorObject, ObjRef, Object, Read, RuntimeError, SIGNAL_DEFAULT, SIGNAL_IGNORE,
    SIGNAL_SIGINT, SocketAddr, SystemTime, TimeParts, ToSocketAddrs, UNIX_EPOCH, Value, Vm,
    apply_uuid_variant, apply_uuid_version, bytes_like_from_value, day_of_year, days_from_civil,
    format_strftime, format_uuid_hex, format_uuid_hyphenated, is_truthy, parse_uuid_like_string,
    split_unix_timestamp, uuid_hash_mix_bytes, uuid_node_from_hostname, uuid_random_bytes,
    uuid_timestamp_100ns_since_gregorian, value_to_f64, value_to_int,
};

const DATETIME_MIN_YEAR: i64 = 1;
const DATETIME_MAX_YEAR: i64 = 9999;
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

impl Vm {
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
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuntimeError::new("system time before epoch"))?;
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
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuntimeError::new("system time before epoch"))?;
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
            seconds += 1;
            microsecond -= 1_000_000;
        } else if microsecond < 0 {
            seconds -= 1;
            microsecond += 1_000_000;
        }
        let tz_offset = tz
            .as_ref()
            .map(|value| self.datetime_timezone_offset_seconds(value))
            .transpose()?
            .unwrap_or(0);
        let parts = split_unix_timestamp(seconds + tz_offset);
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

        let day_us: i128 = 86_400_i128 * 1_000_000_i128;
        let mut total_microseconds = microseconds as i128;
        total_microseconds += milliseconds as i128 * 1_000_i128;
        total_microseconds += seconds as i128 * 1_000_000_i128;
        total_microseconds += minutes as i128 * 60_i128 * 1_000_000_i128;
        total_microseconds += hours as i128 * 3_600_i128 * 1_000_000_i128;
        total_microseconds += (days as i128 + weeks as i128 * 7_i128) * day_us;

        let normalized_days = total_microseconds.div_euclid(day_us);
        let rem = total_microseconds.rem_euclid(day_us);
        let normalized_seconds = rem / 1_000_000_i128;
        let normalized_microseconds = rem % 1_000_000_i128;

        let Object::Instance(instance_data) = &mut *receiver.kind_mut() else {
            return Err(RuntimeError::new(
                "timedelta.__init__() expects instance receiver",
            ));
        };
        instance_data
            .attrs
            .insert("days".to_string(), Value::Int(normalized_days as i64));
        instance_data
            .attrs
            .insert("seconds".to_string(), Value::Int(normalized_seconds as i64));
        instance_data.attrs.insert(
            "microseconds".to_string(),
            Value::Int(normalized_microseconds as i64),
        );
        Ok(Value::None)
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

        let (thread_ident, outcome) =
            self.call_internal_in_synthetic_thread(callable, call_args, call_kwargs)?;
        match outcome {
            InternalCallOutcome::Value(_) => Ok(Value::Int(thread_ident)),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(RuntimeError::new("start_new_thread() callable raised"))
            }
        }
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
        let group = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("group").unwrap_or(Value::None)
        };
        if group != Value::None {
            return Err(RuntimeError::new("group argument must be None for now"));
        }
        let target = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("target").unwrap_or(Value::None)
        };
        let name = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("name").unwrap_or(Value::None)
        };
        let call_args = if !args.is_empty() {
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
            _ => return Err(RuntimeError::new("Thread args must be a tuple")),
        };
        let kwargs_value = match call_kwargs {
            Value::Dict(dict) => Value::Dict(dict),
            _ => return Err(RuntimeError::new("Thread kwargs must be a dict")),
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
                _ => Vec::new(),
            };
            let call_kwargs = match Self::instance_attr_get(&instance, "_kwargs") {
                Some(Value::Dict(dict)) => match &*dict.kind() {
                    Object::Dict(entries) => {
                        let mut out = HashMap::new();
                        for (key, value) in entries {
                            if let Value::Str(name) = key {
                                out.insert(name.clone(), value.clone());
                            }
                        }
                        out
                    }
                    _ => HashMap::new(),
                },
                _ => HashMap::new(),
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
            let (_, outcome) =
                self.call_internal_in_synthetic_thread(target, call_args, call_kwargs)?;
            match outcome {
                InternalCallOutcome::Value(_) => {}
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("thread target raised"));
                }
            }
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
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("Event.wait() expects optional timeout"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Event.wait")?;
        if let Some(timeout) = args.first().cloned() {
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
            let timeout = value_to_f64(timeout)?;
            if timeout.is_sign_negative() {
                return Err(RuntimeError::value_error("timeout must be non-negative"));
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
                    Value::Builtin(BuiltinFunction::NoOp)
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
                    Value::Builtin(BuiltinFunction::NoOp)
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

    pub(super) fn builtin_socket_gethostname(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("gethostname() expects no arguments"));
        }
        let hostname = std::env::var("HOSTNAME")
            .ok()
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
        let fd = match fileno {
            Value::None => {
                let fd = self.next_fd;
                self.next_fd = self.next_fd.saturating_add(1);
                fd
            }
            value => value_to_int(value)?,
        };
        Self::instance_attr_set(&instance, "_family", Value::Int(value_to_int(family)?))?;
        Self::instance_attr_set(&instance, "_type", Value::Int(value_to_int(sock_type)?))?;
        Self::instance_attr_set(&instance, "_proto", Value::Int(value_to_int(proto)?))?;
        Self::instance_attr_set(&instance, "_fd", Value::Int(fd))?;
        Self::instance_attr_set(&instance, "_closed", Value::Bool(fd < 0))?;
        Ok(Value::None)
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
        Ok(Value::Int(uuid_node_from_hostname()))
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
            uuid_node_from_hostname()
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
        let node = (uuid_node_from_hostname() as u64) & 0x0000_FFFF_FFFF_FFFF;
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
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuntimeError::new("system time before epoch"))?;
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

    pub(super) fn builtin_warnings_warn(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if let Ok(module) = self.load_module("_py_warnings") {
            let callable = self.load_attr_module(&module, "warn")?;
            return match self.call_internal(callable, args, kwargs)? {
                InternalCallOutcome::Value(value) => Ok(value),
                InternalCallOutcome::CallerExceptionHandled => {
                    Err(self.runtime_error_from_active_exception("warn() raised exception"))
                }
            };
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
        if let Ok(module) = self.load_module("_py_warnings") {
            let callable = self.load_attr_module(&module, "warn_explicit")?;
            return match self.call_internal(callable, args, kwargs)? {
                InternalCallOutcome::Value(value) => Ok(value),
                InternalCallOutcome::CallerExceptionHandled => {
                    Err(self
                        .runtime_error_from_active_exception("warn_explicit() raised exception"))
                }
            };
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
}
