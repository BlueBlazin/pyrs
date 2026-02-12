use super::*;

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
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("now() expects no arguments"));
        }
        Ok(Value::Str(current_utc_iso()))
    }

    pub(super) fn builtin_datetime_today(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("today() expects no arguments"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuntimeError::new("system time before epoch"))?;
        let days = (now.as_secs() / 86_400) as i64;
        let (year, month, day) = civil_from_days(days);
        Ok(Value::Str(format!("{year:04}-{month:02}-{day:02}")))
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

    pub(super) fn builtin_threading_get_ident(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("get_ident() expects no arguments"));
        }
        let mut hasher = DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        Ok(Value::Int((hasher.finish() & i64::MAX as u64) as i64))
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

        match self.call_internal(callable, call_args, call_kwargs)? {
            InternalCallOutcome::Value(_) => {
                self.builtin_threading_get_ident(Vec::new(), HashMap::new())
            }
            InternalCallOutcome::CallerExceptionHandled => {
                Err(RuntimeError::new("start_new_thread() callable raised"))
            }
        }
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
            {
                if let (Ok(fd), Ok(read_size)) = (
                    value_to_int(call_args[0].clone()),
                    value_to_int(call_args[1].clone()),
                ) {
                    if let Ok(mut file) = self.cloned_open_file_for_fd(fd) {
                        let read_size = read_size.max(0) as usize;
                        std::thread::spawn(move || {
                            let mut buf = vec![0u8; read_size.max(1)];
                            let _ = file.read(&mut buf);
                        });
                        Self::instance_attr_set(&instance, "_alive", Value::Bool(false))?;
                        return Ok(Value::None);
                    }
                }
            }
            match self.call_internal(target, call_args, call_kwargs)? {
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
                return Err(RuntimeError::new("timeout must be non-negative"));
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
                return Err(RuntimeError::new("timeout must be non-negative"));
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
        let lock_value = args.pop().unwrap_or(Value::None);
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
                return Err(RuntimeError::new("timeout must be non-negative"));
            }
        }
        Self::instance_attr_set(&instance, "_locked", Value::Bool(true))?;
        Ok(Value::Bool(true))
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
                return Err(RuntimeError::new("timeout must be non-negative"));
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
        Self::instance_attr_set(&instance, "_bound", Value::Int(value))?;
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
        let blocking = kwargs
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
        if blocking {
            Ok(Value::Bool(false))
        } else {
            Ok(Value::Bool(false))
        }
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
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_thread_semaphore_init(args, kwargs)
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
                return Err(RuntimeError::new("timeout must be non-negative"));
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
                return Err(RuntimeError::new("timeout must be non-negative"));
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
            .unwrap_or(Value::Int(SIGNAL_DEFAULT));
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
            .unwrap_or(Value::Int(SIGNAL_DEFAULT)))
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

    pub(super) fn socket_class_ref(&self) -> Result<ObjRef, RuntimeError> {
        let Some(module) = self.modules.get("_socket").cloned() else {
            return Err(RuntimeError::new("module '_socket' not found"));
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
                    return Err(RuntimeError::new("timeout must be non-negative"));
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
            return Err(RuntimeError::new("module 'uuid' not found"));
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
                _ => return Err(RuntimeError::new("name must be str or bytes")),
            },
            _ => return Err(RuntimeError::new("name must be str or bytes")),
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
                _ => return Err(RuntimeError::new("name must be str or bytes")),
            },
            _ => return Err(RuntimeError::new("name must be str or bytes")),
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
            return Err(RuntimeError::new("module '_colorize' not found"));
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
            return Err(RuntimeError::new("module '_colorize' not found"));
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
            return Err(RuntimeError::new("module '_colorize' not found"));
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
