use super::*;

const IO_BUFFERED_ATTR_READ_BUF: &str = "__pyrs_buffered_read_buf";
const IO_BUFFERED_ATTR_BUF_SIZE: &str = "__pyrs_buffer_size";
const IO_BUFFERED_DEFAULT_SIZE: i64 = 8192;

impl Vm {
    pub(super) fn builtin_io_open(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 8 {
            return Err(RuntimeError::new("open() expected at most 8 arguments"));
        }

        let file_arg = args.remove(0);
        let mut mode_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut buffering_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut encoding_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut errors_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut newline_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut closefd_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut opener_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if !args.is_empty() {
            return Err(RuntimeError::new("open() expected at most 8 arguments"));
        }

        if let Some(value) = kwargs.remove("mode") {
            if mode_arg.is_some() {
                return Err(RuntimeError::new("open() got multiple values for mode"));
            }
            mode_arg = Some(value);
        }
        if let Some(value) = kwargs.remove("buffering") {
            if buffering_arg.is_some() {
                return Err(RuntimeError::new(
                    "open() got multiple values for buffering",
                ));
            }
            buffering_arg = Some(value);
        }
        if let Some(value) = kwargs.remove("encoding") {
            if encoding_arg.is_some() {
                return Err(RuntimeError::new("open() got multiple values for encoding"));
            }
            encoding_arg = Some(value);
        }
        if let Some(value) = kwargs.remove("errors") {
            if errors_arg.is_some() {
                return Err(RuntimeError::new("open() got multiple values for errors"));
            }
            errors_arg = Some(value);
        }
        if let Some(value) = kwargs.remove("newline") {
            if newline_arg.is_some() {
                return Err(RuntimeError::new("open() got multiple values for newline"));
            }
            newline_arg = Some(value);
        }
        if let Some(value) = kwargs.remove("closefd") {
            if closefd_arg.is_some() {
                return Err(RuntimeError::new("open() got multiple values for closefd"));
            }
            closefd_arg = Some(value);
        }
        if let Some(value) = kwargs.remove("opener") {
            if opener_arg.is_some() {
                return Err(RuntimeError::new("open() got multiple values for opener"));
            }
            opener_arg = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "open() got an unexpected keyword argument",
            ));
        }

        let mut mode = match mode_arg.unwrap_or(Value::Str("r".to_string())) {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("open() mode must be str")),
        };
        // Keep mode validation close to CPython's _io_open_impl in Modules/_io/_iomodule.c.
        let mut creating = false;
        let mut reading = false;
        let mut writing = false;
        let mut appending = false;
        let mut updating = false;
        let mut text_mode = false;
        let mut binary_mode = false;
        for ch in mode.chars() {
            match ch {
                'x' => {
                    if creating {
                        return Err(RuntimeError::new(format!("invalid mode: '{mode}'")));
                    }
                    creating = true;
                }
                'r' => {
                    if reading {
                        return Err(RuntimeError::new(format!("invalid mode: '{mode}'")));
                    }
                    reading = true;
                }
                'w' => {
                    if writing {
                        return Err(RuntimeError::new(format!("invalid mode: '{mode}'")));
                    }
                    writing = true;
                }
                'a' => {
                    if appending {
                        return Err(RuntimeError::new(format!("invalid mode: '{mode}'")));
                    }
                    appending = true;
                }
                '+' => {
                    if updating {
                        return Err(RuntimeError::new(format!("invalid mode: '{mode}'")));
                    }
                    updating = true;
                }
                't' => {
                    if text_mode {
                        return Err(RuntimeError::new(format!("invalid mode: '{mode}'")));
                    }
                    text_mode = true;
                }
                'b' => {
                    if binary_mode {
                        return Err(RuntimeError::new(format!("invalid mode: '{mode}'")));
                    }
                    binary_mode = true;
                }
                _ => return Err(RuntimeError::new(format!("invalid mode: '{mode}'"))),
            }
        }

        if text_mode && binary_mode {
            return Err(RuntimeError::new("can't have text and binary mode at once"));
        }
        let mode_kind_count = creating as u8 + reading as u8 + writing as u8 + appending as u8;
        if mode_kind_count != 1 {
            return Err(RuntimeError::new(
                "must have exactly one of create/read/write/append mode",
            ));
        }
        let mode_kind = if creating {
            'x'
        } else if reading {
            'r'
        } else if writing {
            'w'
        } else {
            'a'
        };
        if binary_mode && updating {
            mode = match mode_kind {
                'a' => "ab+".to_string(),
                'x' => "xb+".to_string(),
                _ => "rb+".to_string(),
            };
        }

        let mut buffering = value_to_int(buffering_arg.unwrap_or(Value::Int(-1)))?;
        if buffering < -1 {
            return Err(RuntimeError::new("invalid buffering size"));
        }
        if binary_mode && buffering == 1 {
            // CPython emits RuntimeWarning and falls back to default buffering.
            buffering = -1;
        }
        if buffering == 0 && !binary_mode {
            return Err(RuntimeError::new("can't have unbuffered text I/O"));
        }

        let encoding = match encoding_arg.unwrap_or(Value::None) {
            Value::None => None,
            Value::Str(value) => Some(value),
            _ => return Err(RuntimeError::new("open() encoding must be str or None")),
        };
        let errors = match errors_arg.unwrap_or(Value::None) {
            Value::None => None,
            Value::Str(value) => Some(value),
            _ => return Err(RuntimeError::new("open() errors must be str or None")),
        };
        let newline = match newline_arg.unwrap_or(Value::None) {
            Value::None => None,
            Value::Str(value) => Some(value),
            _ => return Err(RuntimeError::new("open() newline must be str or None")),
        };
        if let Some(value) = newline.as_deref() {
            if !matches!(value, "" | "\n" | "\r" | "\r\n") {
                return Err(RuntimeError::new(format!(
                    "ValueError: illegal newline value: {value}",
                )));
            }
        }
        if binary_mode && encoding.is_some() {
            return Err(RuntimeError::new(
                "binary mode doesn't take an encoding argument",
            ));
        }
        if binary_mode && errors.is_some() {
            return Err(RuntimeError::new(
                "binary mode doesn't take an errors argument",
            ));
        }
        if binary_mode && newline.is_some() {
            return Err(RuntimeError::new(
                "binary mode doesn't take a newline argument",
            ));
        }
        if let Some(encoding_name) = encoding.as_ref() {
            self.ensure_known_text_encoding(encoding_name)?;
        }
        let closefd = is_truthy(&closefd_arg.unwrap_or(Value::Bool(true)));
        let opener = opener_arg.unwrap_or(Value::None);

        let opener_value = if matches!(opener, Value::None) {
            None
        } else {
            Some(opener)
        };

        let fd = match file_arg {
            Value::Int(fd) => {
                if fd < 0 {
                    return Err(RuntimeError::new(
                        "OSError: bad file descriptor (os error 9)",
                    ));
                }
                let Some(resolved_fd) = self.resolve_open_file_fd(fd).or_else(|| {
                    if (0..=2).contains(&fd) {
                        Some(fd)
                    } else {
                        None
                    }
                }) else {
                    return Err(RuntimeError::new(
                        "OSError: bad file descriptor (os error 9)",
                    ));
                };
                resolved_fd
            }
            Value::Bool(flag) => {
                let fd = if flag { 1 } else { 0 };
                self.resolve_open_file_fd(fd).unwrap_or(fd)
            }
            pathlike => {
                if !closefd {
                    return Err(RuntimeError::new("Cannot use closefd=False with file name"));
                }
                let path = self.io_open_path_from_value(pathlike)?;
                if let Some(opener) = opener_value {
                    let mut flags = if updating {
                        2
                    } else if mode_kind == 'r' {
                        0
                    } else {
                        1
                    };
                    match mode_kind {
                        'w' => {
                            flags |= 64 | 512;
                        }
                        'a' => {
                            flags |= 64 | 1024;
                        }
                        'x' => {
                            flags |= 64 | 128;
                        }
                        _ => {}
                    }
                    let opener_result = match self.call_internal(
                        opener,
                        vec![Value::Str(path.clone()), Value::Int(flags)],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(value) => value,
                        InternalCallOutcome::CallerExceptionHandled => {
                            return Err(RuntimeError::new("open failed: opener callback raised"));
                        }
                    };
                    let fd = value_to_int(opener_result)?;
                    if fd < 0 {
                        return Err(RuntimeError::new(format!("opener returned {fd}")));
                    }
                    let Some(resolved_fd) = self.resolve_open_file_fd(fd).or_else(|| {
                        if (0..=2).contains(&fd) {
                            Some(fd)
                        } else {
                            None
                        }
                    }) else {
                        return Err(RuntimeError::new(
                            "OSError: bad file descriptor (os error 9)",
                        ));
                    };
                    resolved_fd
                } else {
                    let mut options = fs::OpenOptions::new();
                    match mode_kind {
                        'r' => {
                            options.read(true);
                            if updating {
                                options.write(true);
                            }
                        }
                        'w' => {
                            options.write(true).create(true).truncate(true);
                            if updating {
                                options.read(true);
                            }
                        }
                        'a' => {
                            options.write(true).append(true).create(true);
                            if updating {
                                options.read(true);
                            }
                        }
                        'x' => {
                            options.write(true).create_new(true);
                            if updating {
                                options.read(true);
                            }
                        }
                        _ => return Err(RuntimeError::new("invalid mode")),
                    }
                    let file = options
                        .open(path)
                        .map_err(|err| RuntimeError::new(format!("open() failed: {err}")))?;
                    self.alloc_open_fd(file)
                }
            }
        };
        if mode_kind == 'a' {
            if let Some(file) = self.find_open_file_mut(fd) {
                let _ = file.seek(SeekFrom::End(0));
            }
        }

        let raw_mode = if binary_mode {
            mode.clone()
        } else {
            let mut raw_mode = mode.chars().filter(|ch| *ch != 't').collect::<String>();
            if !raw_mode.contains('b') {
                raw_mode.push('b');
            }
            raw_mode
        };
        let raw_instance =
            self.alloc_io_file_instance("FileIO", fd, &raw_mode, true, closefd, None, None, None)?;
        let buffered_instance = if buffering == 0 {
            raw_instance.clone()
        } else {
            let buffered_class = if mode_kind == 'r' && !updating {
                "BufferedReader"
            } else if (mode_kind == 'w' || mode_kind == 'x' || mode_kind == 'a') && !updating {
                "BufferedWriter"
            } else {
                "BufferedRandom"
            };
            let instance = self.alloc_io_file_instance(
                buffered_class,
                fd,
                &raw_mode,
                true,
                closefd,
                None,
                None,
                None,
            )?;
            if let Value::Instance(buffer_ref) = &instance {
                if let Value::Instance(raw_ref) = &raw_instance {
                    Self::instance_attr_set(buffer_ref, "raw", Value::Instance(raw_ref.clone()))?;
                }
                let buffer_size = if buffering > 0 {
                    buffering
                } else {
                    IO_BUFFERED_DEFAULT_SIZE
                };
                Self::instance_attr_set(
                    buffer_ref,
                    IO_BUFFERED_ATTR_BUF_SIZE,
                    Value::Int(buffer_size),
                )?;
                self.io_buffered_store_read_buffer(buffer_ref, Vec::new())?;
            }
            instance
        };
        if binary_mode {
            return Ok(buffered_instance);
        }
        let text_instance = self.alloc_io_file_instance(
            "TextIOWrapper",
            fd,
            &mode,
            false,
            closefd,
            encoding,
            errors,
            newline,
        )?;
        let line_buffering = buffering == 1;
        if let (Value::Instance(text_ref), Value::Instance(raw_ref), Value::Instance(buffer_ref)) =
            (&text_instance, &raw_instance, &buffered_instance)
        {
            Self::instance_attr_set(text_ref, "buffer", Value::Instance(buffer_ref.clone()))?;
            Self::instance_attr_set(text_ref, "raw", Value::Instance(raw_ref.clone()))?;
            Self::instance_attr_set(text_ref, "_line_buffering", Value::Bool(line_buffering))?;
        }
        Ok(text_instance)
    }

    pub(super) fn ensure_known_text_encoding(
        &mut self,
        encoding: &str,
    ) -> Result<(), RuntimeError> {
        match self.call_builtin(
            BuiltinFunction::CodecsLookup,
            vec![Value::Str(encoding.to_string())],
            HashMap::new(),
        ) {
            Ok(_) => Ok(()),
            Err(_) => Err(RuntimeError::new(format!(
                "LookupError: unknown encoding: {}",
                encoding
            ))),
        }
    }

    pub(super) fn io_open_path_from_value(&mut self, value: Value) -> Result<String, RuntimeError> {
        let validate_path = |path: String| -> Result<String, RuntimeError> {
            if path.contains('\0') {
                return Err(RuntimeError::new(
                    "ValueError: embedded null character in path",
                ));
            }
            Ok(path)
        };
        match value {
            Value::Str(_) | Value::Bytes(_) => validate_path(value_to_path(&value)?),
            other => {
                let Some(fspath) = self.lookup_bound_special_method(&other, "__fspath__")? else {
                    return Err(RuntimeError::new(
                        "TypeError: expected str, bytes or os.PathLike object",
                    ));
                };
                let path_value = match self.call_internal(fspath, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(self.runtime_error_from_active_exception("__fspath__() failed"));
                    }
                };
                match path_value {
                    Value::Str(_) | Value::Bytes(_) => validate_path(value_to_path(&path_value)?),
                    _ => Err(RuntimeError::new(
                        "TypeError: __fspath__() must return str or bytes",
                    )),
                }
            }
        }
    }

    pub(super) fn builtin_io_read_text(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("read_text() expects one argument"));
        }
        let path = self.path_arg_to_string(args[0].clone())?;
        let text = fs::read_to_string(path)
            .map_err(|err| RuntimeError::new(format!("read_text failed: {err}")))?;
        Ok(Value::Str(text))
    }

    pub(super) fn builtin_io_write_text(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("write_text() expects path and text"));
        }
        let path = self.path_arg_to_string(args[0].clone())?;
        let text = match &args[1] {
            Value::Str(text) => text.clone(),
            other => format_value(other),
        };
        fs::write(path, text)
            .map_err(|err| RuntimeError::new(format!("write_text failed: {err}")))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_io_text_encoding(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let mut encoding: Option<Value> = None;
        let mut _stacklevel: Option<Value> = None;

        if let Some(value) = kwargs.get("encoding") {
            encoding = Some(value.clone());
        }
        if let Some(value) = kwargs.get("stacklevel") {
            _stacklevel = Some(value.clone());
        }
        for name in kwargs.keys() {
            if name != "encoding" && name != "stacklevel" {
                return Err(RuntimeError::new(
                    "text_encoding() got unexpected keyword argument",
                ));
            }
        }

        if encoding.is_none() && !args.is_empty() {
            encoding = Some(args.remove(0));
        }
        if _stacklevel.is_none() && !args.is_empty() {
            _stacklevel = Some(args.remove(0));
        }
        if !args.is_empty() {
            return Err(RuntimeError::new(
                "text_encoding() takes at most 2 arguments",
            ));
        }

        match encoding.unwrap_or(Value::None) {
            Value::None => Ok(Value::Str("utf-8".to_string())),
            Value::Str(value) => Ok(Value::Str(value)),
            _ => Err(RuntimeError::new(
                "text_encoding() argument 'encoding' must be str or None",
            )),
        }
    }

    pub(super) fn builtin_io_textiowrapper_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = match args.first() {
            Some(Value::Instance(instance)) => instance.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "TextIOWrapper.__init__() expects self and buffer",
                ));
            }
        };
        args.remove(0);
        let buffer_value = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = kwargs.remove("buffer") {
            value
        } else {
            return Err(RuntimeError::new(
                "TextIOWrapper.__init__() missing required buffer argument",
            ));
        };
        let buffer_instance = match buffer_value {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "TextIOWrapper() argument 1 must be file object",
                ));
            }
        };
        let buffer_fd = match Self::instance_attr_get(&buffer_instance, "_fd") {
            Some(Value::Int(fd)) => Some(fd),
            _ => None,
        };
        let buffer_is_bytesio = matches!(
            Self::instance_attr_get(&buffer_instance, "_value"),
            Some(Value::ByteArray(_)) | Some(Value::Bytes(_))
        );
        let buffer_is_file_like = self
            .builtin_getattr(
                vec![
                    Value::Instance(buffer_instance.clone()),
                    Value::Str("readable".to_string()),
                ],
                HashMap::new(),
            )
            .is_ok()
            && self
                .builtin_getattr(
                    vec![
                        Value::Instance(buffer_instance.clone()),
                        Value::Str("writable".to_string()),
                    ],
                    HashMap::new(),
                )
                .is_ok();
        if buffer_fd.is_none() && !buffer_is_bytesio && !buffer_is_file_like {
            return Err(RuntimeError::new(
                "TextIOWrapper() argument 1 must be file object",
            ));
        }

        let encoding = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("encoding")
        };
        let errors = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("errors")
        };
        let newline = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("newline")
        };
        if !args.is_empty() {
            args.remove(0);
        }
        if !args.is_empty() {
            args.remove(0);
        }
        for key in kwargs.keys() {
            if !matches!(
                key.as_str(),
                "encoding" | "errors" | "newline" | "line_buffering" | "write_through" | "buffer"
            ) {
                return Err(RuntimeError::new(
                    "TextIOWrapper.__init__() got unexpected keyword argument",
                ));
            }
        }

        let source_mode = if let Some(mode) = Self::io_file_mode(&buffer_instance) {
            mode
        } else {
            let infer_ability = |vm: &mut Vm, method_name: &str| -> bool {
                let method = match vm.builtin_getattr(
                    vec![
                        Value::Instance(buffer_instance.clone()),
                        Value::Str(method_name.to_string()),
                    ],
                    HashMap::new(),
                ) {
                    Ok(value) => value,
                    Err(_) => return false,
                };
                match vm.call_internal(method, Vec::new(), HashMap::new()) {
                    Ok(InternalCallOutcome::Value(value)) => is_truthy(&value),
                    Ok(InternalCallOutcome::CallerExceptionHandled) => {
                        vm.clear_active_exception();
                        false
                    }
                    Err(_) => false,
                }
            };
            let readable = infer_ability(self, "readable");
            let writable = infer_ability(self, "writable");
            match (readable, writable) {
                (true, true) => "r+".to_string(),
                (true, false) => "r".to_string(),
                (false, true) => "w".to_string(),
                (false, false) => {
                    if buffer_fd.is_some() {
                        "r".to_string()
                    } else {
                        "r+".to_string()
                    }
                }
            }
        };
        let mut mode = source_mode.replace('b', "");
        if mode.is_empty() {
            mode = "r".to_string();
        }
        let closefd = !matches!(
            Self::instance_attr_get(&buffer_instance, "_closefd"),
            Some(Value::Bool(false))
        );
        let closed = matches!(
            Self::instance_attr_get(&buffer_instance, "_closed"),
            Some(Value::Bool(true))
        );

        if let Some(fd) = buffer_fd {
            Self::instance_attr_set(&instance, "_fd", Value::Int(fd))?;
        }
        Self::instance_attr_set(&instance, "_mode", Value::Str(mode))?;
        Self::instance_attr_set(&instance, "_binary", Value::Bool(false))?;
        Self::instance_attr_set(&instance, "_closed", Value::Bool(closed))?;
        Self::instance_attr_set(&instance, "closed", Value::Bool(closed))?;
        Self::instance_attr_set(
            &instance,
            "_closefd",
            Value::Bool(closefd && buffer_fd.is_some()),
        )?;
        let encoding_value = match encoding.unwrap_or(Value::None) {
            Value::None => Value::Str("utf-8".to_string()),
            Value::Str(value) => {
                self.ensure_known_text_encoding(&value)?;
                Value::Str(value)
            }
            _ => {
                return Err(RuntimeError::new(
                    "TextIOWrapper encoding must be str or None",
                ));
            }
        };
        Self::instance_attr_set(&instance, "_encoding", encoding_value.clone())?;
        Self::instance_attr_set(&instance, "encoding", encoding_value)?;
        let errors_value = match errors.unwrap_or(Value::None) {
            Value::None => Value::Str("strict".to_string()),
            Value::Str(value) => Value::Str(value),
            _ => {
                return Err(RuntimeError::new(
                    "TextIOWrapper errors must be str or None",
                ));
            }
        };
        Self::instance_attr_set(&instance, "_errors", errors_value.clone())?;
        Self::instance_attr_set(&instance, "errors", errors_value)?;
        let newline_value = match newline.unwrap_or(Value::None) {
            Value::None => Value::None,
            Value::Str(value) => {
                if !matches!(value.as_str(), "" | "\n" | "\r" | "\r\n") {
                    return Err(RuntimeError::new(format!(
                        "ValueError: illegal newline value: {value}",
                    )));
                }
                Value::Str(value)
            }
            _ => {
                return Err(RuntimeError::new(
                    "TextIOWrapper newline must be str or None",
                ));
            }
        };
        Self::instance_attr_set(&instance, "_newline", newline_value.clone())?;
        let observed_newlines = match &newline_value {
            Value::None => Value::Str(if cfg!(windows) { "\r\n" } else { "\n" }.to_string()),
            Value::Str(value) if value.is_empty() => {
                Value::Str(if cfg!(windows) { "\r\n" } else { "\n" }.to_string())
            }
            Value::Str(value) => Value::Str(value.clone()),
            _ => Value::None,
        };
        Self::instance_attr_set(&instance, "newlines", observed_newlines)?;
        Self::instance_attr_set(
            &instance,
            "buffer",
            Value::Instance(buffer_instance.clone()),
        )?;
        Self::instance_attr_set(&instance, "raw", Value::Instance(buffer_instance))?;
        Ok(Value::None)
    }

    pub(super) fn io_class_ref(&self, class_name: &str) -> Result<ObjRef, RuntimeError> {
        let module = self
            .modules
            .get("_io")
            .cloned()
            .ok_or_else(|| RuntimeError::new("module '_io' not found"))?;
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("module '_io' is invalid"));
        };
        match module_data.globals.get(class_name) {
            Some(Value::Class(class_ref)) => Ok(class_ref.clone()),
            _ => Err(RuntimeError::new(format!(
                "module '_io' has no {} class",
                class_name
            ))),
        }
    }

    pub(super) fn install_iobase_methods(class_data: &mut ClassObject) {
        class_data.attrs.insert(
            "readline".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseReadLine),
        );
        class_data.attrs.insert(
            "readlines".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseReadLines),
        );
        class_data.attrs.insert(
            "writelines".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseWriteLines),
        );
        class_data.attrs.insert(
            "__iter__".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseIter),
        );
        class_data.attrs.insert(
            "__next__".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseNext),
        );
        class_data.attrs.insert(
            "close".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseClose),
        );
        class_data.attrs.insert(
            "flush".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseFlush),
        );
        class_data.attrs.insert(
            "__del__".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseDel),
        );
        class_data.attrs.insert(
            "__enter__".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseEnter),
        );
        class_data.attrs.insert(
            "__exit__".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseExit),
        );
    }

    pub(super) fn install_io_file_methods(class_data: &mut ClassObject) {
        class_data.attrs.insert(
            "read".to_string(),
            Value::Builtin(BuiltinFunction::IoFileRead),
        );
        class_data.attrs.insert(
            "readline".to_string(),
            Value::Builtin(BuiltinFunction::IoFileReadLine),
        );
        class_data.attrs.insert(
            "readinto".to_string(),
            Value::Builtin(BuiltinFunction::IoFileReadInto),
        );
        class_data.attrs.insert(
            "readlines".to_string(),
            Value::Builtin(BuiltinFunction::IoFileReadLines),
        );
        class_data.attrs.insert(
            "write".to_string(),
            Value::Builtin(BuiltinFunction::IoFileWrite),
        );
        class_data.attrs.insert(
            "writelines".to_string(),
            Value::Builtin(BuiltinFunction::IoFileWriteLines),
        );
        class_data.attrs.insert(
            "truncate".to_string(),
            Value::Builtin(BuiltinFunction::IoFileTruncate),
        );
        class_data.attrs.insert(
            "seek".to_string(),
            Value::Builtin(BuiltinFunction::IoFileSeek),
        );
        class_data.attrs.insert(
            "tell".to_string(),
            Value::Builtin(BuiltinFunction::IoFileTell),
        );
        class_data.attrs.insert(
            "close".to_string(),
            Value::Builtin(BuiltinFunction::IoFileClose),
        );
        class_data.attrs.insert(
            "__del__".to_string(),
            Value::Builtin(BuiltinFunction::IoBaseDel),
        );
        class_data.attrs.insert(
            "flush".to_string(),
            Value::Builtin(BuiltinFunction::IoFileFlush),
        );
        class_data.attrs.insert(
            "__iter__".to_string(),
            Value::Builtin(BuiltinFunction::IoFileIter),
        );
        class_data.attrs.insert(
            "__next__".to_string(),
            Value::Builtin(BuiltinFunction::IoFileNext),
        );
        class_data.attrs.insert(
            "__enter__".to_string(),
            Value::Builtin(BuiltinFunction::IoFileEnter),
        );
        class_data.attrs.insert(
            "__exit__".to_string(),
            Value::Builtin(BuiltinFunction::IoFileExit),
        );
        class_data.attrs.insert(
            "fileno".to_string(),
            Value::Builtin(BuiltinFunction::IoFileFileno),
        );
        class_data.attrs.insert(
            "detach".to_string(),
            Value::Builtin(BuiltinFunction::IoFileDetach),
        );
        class_data.attrs.insert(
            "readable".to_string(),
            Value::Builtin(BuiltinFunction::IoFileReadable),
        );
        class_data.attrs.insert(
            "writable".to_string(),
            Value::Builtin(BuiltinFunction::IoFileWritable),
        );
        class_data.attrs.insert(
            "seekable".to_string(),
            Value::Builtin(BuiltinFunction::IoFileSeekable),
        );
    }

    pub(super) fn install_stringio_methods(class_data: &mut ClassObject) {
        class_data.attrs.insert(
            "__iter__".to_string(),
            Value::Builtin(BuiltinFunction::StringIOIter),
        );
        class_data.attrs.insert(
            "__next__".to_string(),
            Value::Builtin(BuiltinFunction::StringIONext),
        );
        class_data.attrs.insert(
            "__enter__".to_string(),
            Value::Builtin(BuiltinFunction::StringIOEnter),
        );
        class_data.attrs.insert(
            "__exit__".to_string(),
            Value::Builtin(BuiltinFunction::StringIOExit),
        );
        class_data.attrs.insert(
            "write".to_string(),
            Value::Builtin(BuiltinFunction::StringIOWrite),
        );
        class_data.attrs.insert(
            "read".to_string(),
            Value::Builtin(BuiltinFunction::StringIORead),
        );
        class_data.attrs.insert(
            "readline".to_string(),
            Value::Builtin(BuiltinFunction::StringIOReadLine),
        );
        class_data.attrs.insert(
            "readlines".to_string(),
            Value::Builtin(BuiltinFunction::StringIOReadLines),
        );
        class_data.attrs.insert(
            "getvalue".to_string(),
            Value::Builtin(BuiltinFunction::StringIOGetValue),
        );
        class_data.attrs.insert(
            "__getstate__".to_string(),
            Value::Builtin(BuiltinFunction::StringIOGetState),
        );
        class_data.attrs.insert(
            "__setstate__".to_string(),
            Value::Builtin(BuiltinFunction::StringIOSetState),
        );
        class_data.attrs.insert(
            "seek".to_string(),
            Value::Builtin(BuiltinFunction::StringIOSeek),
        );
        class_data.attrs.insert(
            "tell".to_string(),
            Value::Builtin(BuiltinFunction::StringIOTell),
        );
        class_data.attrs.insert(
            "writelines".to_string(),
            Value::Builtin(BuiltinFunction::StringIOWriteLines),
        );
        class_data.attrs.insert(
            "truncate".to_string(),
            Value::Builtin(BuiltinFunction::StringIOTruncate),
        );
        class_data.attrs.insert(
            "detach".to_string(),
            Value::Builtin(BuiltinFunction::StringIODetach),
        );
        class_data.attrs.insert(
            "close".to_string(),
            Value::Builtin(BuiltinFunction::StringIOClose),
        );
        class_data.attrs.insert(
            "flush".to_string(),
            Value::Builtin(BuiltinFunction::StringIOFlush),
        );
        class_data.attrs.insert(
            "isatty".to_string(),
            Value::Builtin(BuiltinFunction::StringIOIsAtty),
        );
        class_data.attrs.insert(
            "fileno".to_string(),
            Value::Builtin(BuiltinFunction::StringIOFileno),
        );
        class_data.attrs.insert(
            "readable".to_string(),
            Value::Builtin(BuiltinFunction::StringIOReadable),
        );
        class_data.attrs.insert(
            "writable".to_string(),
            Value::Builtin(BuiltinFunction::StringIOWritable),
        );
        class_data.attrs.insert(
            "seekable".to_string(),
            Value::Builtin(BuiltinFunction::StringIOSeekable),
        );
        class_data.attrs.insert(
            "__init__".to_string(),
            Value::Builtin(BuiltinFunction::StringIOInit),
        );
    }

    pub(super) fn install_bytesio_methods(class_data: &mut ClassObject) {
        class_data.attrs.insert(
            "__iter__".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOIter),
        );
        class_data.attrs.insert(
            "__next__".to_string(),
            Value::Builtin(BuiltinFunction::BytesIONext),
        );
        class_data.attrs.insert(
            "__enter__".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOEnter),
        );
        class_data.attrs.insert(
            "__exit__".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOExit),
        );
        class_data.attrs.insert(
            "close".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOClose),
        );
        class_data.attrs.insert(
            "write".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOWrite),
        );
        class_data.attrs.insert(
            "writelines".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOWriteLines),
        );
        class_data.attrs.insert(
            "truncate".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOTruncate),
        );
        class_data.attrs.insert(
            "read".to_string(),
            Value::Builtin(BuiltinFunction::BytesIORead),
        );
        class_data.attrs.insert(
            "read1".to_string(),
            Value::Builtin(BuiltinFunction::BytesIORead1),
        );
        class_data.attrs.insert(
            "readline".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOReadLine),
        );
        class_data.attrs.insert(
            "readlines".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOReadLines),
        );
        class_data.attrs.insert(
            "readinto".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOReadInto),
        );
        class_data.attrs.insert(
            "getvalue".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOGetValue),
        );
        class_data.attrs.insert(
            "getbuffer".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOGetBuffer),
        );
        class_data.attrs.insert(
            "__getstate__".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOGetState),
        );
        class_data.attrs.insert(
            "__setstate__".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOSetState),
        );
        class_data.attrs.insert(
            "detach".to_string(),
            Value::Builtin(BuiltinFunction::BytesIODetach),
        );
        class_data.attrs.insert(
            "seek".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOSeek),
        );
        class_data.attrs.insert(
            "tell".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOTell),
        );
        class_data.attrs.insert(
            "flush".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOFlush),
        );
        class_data.attrs.insert(
            "isatty".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOIsAtty),
        );
        class_data.attrs.insert(
            "fileno".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOFileno),
        );
        class_data.attrs.insert(
            "readable".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOReadable),
        );
        class_data.attrs.insert(
            "writable".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOWritable),
        );
        class_data.attrs.insert(
            "seekable".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOSeekable),
        );
        class_data.attrs.insert(
            "__init__".to_string(),
            Value::Builtin(BuiltinFunction::BytesIOInit),
        );
    }

    fn fileio_force_binary_mode(mode: &str) -> Result<String, RuntimeError> {
        if mode.chars().any(|ch| ch == 't') {
            return Err(RuntimeError::new(format!("invalid mode: '{mode}'")));
        }
        if mode.chars().any(|ch| ch == 'b') {
            return Ok(mode.to_string());
        }
        if let Some(prefix) = mode.strip_suffix('+') {
            return Ok(format!("{prefix}b+"));
        }
        Ok(format!("{mode}b"))
    }

    pub(super) fn builtin_io_file_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let receiver = self.take_bound_instance_arg(&mut args, "FileIO.__init__")?;

        let mut file_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut mode_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut closefd_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut opener_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if !args.is_empty() {
            return Err(RuntimeError::new("FileIO() expected at most 4 arguments"));
        }

        if let Some(value) = kwargs.remove("file") {
            if file_arg.is_some() {
                return Err(RuntimeError::new("FileIO() got multiple values for file"));
            }
            file_arg = Some(value);
        }
        if let Some(value) = kwargs.remove("mode") {
            if mode_arg.is_some() {
                return Err(RuntimeError::new("FileIO() got multiple values for mode"));
            }
            mode_arg = Some(value);
        }
        if let Some(value) = kwargs.remove("closefd") {
            if closefd_arg.is_some() {
                return Err(RuntimeError::new(
                    "FileIO() got multiple values for closefd",
                ));
            }
            closefd_arg = Some(value);
        }
        if let Some(value) = kwargs.remove("opener") {
            if opener_arg.is_some() {
                return Err(RuntimeError::new("FileIO() got multiple values for opener"));
            }
            opener_arg = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "FileIO() got an unexpected keyword argument",
            ));
        }

        let file = file_arg
            .ok_or_else(|| RuntimeError::new("FileIO() missing required argument 'file'"))?;
        let mode = match mode_arg.unwrap_or(Value::Str("r".to_string())) {
            Value::Str(mode) => mode,
            _ => return Err(RuntimeError::new("FileIO() mode must be str")),
        };
        let normalized_mode = Self::fileio_force_binary_mode(&mode)?;

        let mut open_kwargs = HashMap::new();
        if let Some(closefd) = closefd_arg {
            open_kwargs.insert("closefd".to_string(), closefd);
        }
        if let Some(opener) = opener_arg {
            open_kwargs.insert("opener".to_string(), opener);
        }

        let opened = self.builtin_io_open(
            vec![file, Value::Str(normalized_mode), Value::Int(0)],
            open_kwargs,
        )?;
        let opened_instance = match opened {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("FileIO initialization failed")),
        };
        let attrs = match &*opened_instance.kind() {
            Object::Instance(instance_data) => instance_data.attrs.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "FileIO initialization produced invalid object",
                ));
            }
        };
        let Object::Instance(receiver_data) = &mut *receiver.kind_mut() else {
            return Err(RuntimeError::new("FileIO receiver must be instance"));
        };
        receiver_data.attrs = attrs;
        Ok(Value::None)
    }

    pub(super) fn alloc_io_file_instance(
        &self,
        class_name: &str,
        fd: i64,
        mode: &str,
        binary: bool,
        closefd: bool,
        encoding: Option<String>,
        errors: Option<String>,
        newline: Option<String>,
    ) -> Result<Value, RuntimeError> {
        let class_ref = self.io_class_ref(class_name)?;
        let instance = match self.heap.alloc_instance(InstanceObject::new(class_ref)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        let instance_value = Value::Instance(instance.clone());
        let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
            return Err(RuntimeError::new("expected io instance"));
        };
        instance_data
            .attrs
            .insert("_fd".to_string(), Value::Int(fd));
        instance_data
            .attrs
            .insert("_mode".to_string(), Value::Str(mode.to_string()));
        instance_data
            .attrs
            .insert("mode".to_string(), Value::Str(mode.to_string()));
        instance_data
            .attrs
            .insert("_binary".to_string(), Value::Bool(binary));
        instance_data
            .attrs
            .insert("_closed".to_string(), Value::Bool(false));
        instance_data
            .attrs
            .insert("closed".to_string(), Value::Bool(false));
        instance_data
            .attrs
            .insert("_closefd".to_string(), Value::Bool(closefd));
        instance_data
            .attrs
            .insert("closefd".to_string(), Value::Bool(closefd));
        instance_data
            .attrs
            .insert("name".to_string(), Value::Int(fd));
        let encoding_value = if binary {
            encoding.map(Value::Str).unwrap_or(Value::None)
        } else {
            Value::Str(encoding.unwrap_or_else(|| "utf-8".to_string()))
        };
        let errors_value = if binary {
            errors.map(Value::Str).unwrap_or(Value::None)
        } else {
            Value::Str(errors.unwrap_or_else(|| "strict".to_string()))
        };
        let newline_value = newline.map(Value::Str).unwrap_or(Value::None);
        instance_data
            .attrs
            .insert("_encoding".to_string(), encoding_value.clone());
        instance_data
            .attrs
            .insert("_errors".to_string(), errors_value.clone());
        instance_data
            .attrs
            .insert("_newline".to_string(), newline_value.clone());
        if !binary {
            let observed_newlines = match &newline_value {
                Value::None => Value::Str(if cfg!(windows) { "\r\n" } else { "\n" }.to_string()),
                Value::Str(value) if value.is_empty() => {
                    Value::Str(if cfg!(windows) { "\r\n" } else { "\n" }.to_string())
                }
                Value::Str(value) => Value::Str(value.clone()),
                _ => Value::None,
            };
            instance_data
                .attrs
                .insert("encoding".to_string(), encoding_value);
            instance_data
                .attrs
                .insert("errors".to_string(), errors_value);
            instance_data
                .attrs
                .insert("newlines".to_string(), observed_newlines);
        }
        if !binary {
            instance_data
                .attrs
                .insert("buffer".to_string(), instance_value.clone());
            instance_data
                .attrs
                .insert("raw".to_string(), instance_value.clone());
        }
        Ok(instance_value)
    }

    pub(super) fn io_file_fd_from_instance(&self, instance: &ObjRef) -> Result<i64, RuntimeError> {
        self.io_file_fd_from_instance_inner(instance, 0)
    }

    fn io_buffered_raw_from_instance(instance: &ObjRef) -> Result<Value, RuntimeError> {
        Self::instance_attr_get(instance, "raw")
            .ok_or_else(|| RuntimeError::new("buffered stream is uninitialized"))
    }

    fn io_buffered_is_uninitialized(instance: &ObjRef) -> bool {
        matches!(
            Self::instance_attr_get(instance, "raw"),
            None | Some(Value::None)
        )
    }

    fn io_buffered_read_buffer(instance: &ObjRef) -> Result<Vec<u8>, RuntimeError> {
        let Some(value) = Self::instance_attr_get(instance, IO_BUFFERED_ATTR_READ_BUF) else {
            return Ok(Vec::new());
        };
        bytes_like_from_value(value)
            .map_err(|_| RuntimeError::new("TypeError: internal buffered data must be bytes-like"))
    }

    fn io_buffered_store_read_buffer(
        &mut self,
        instance: &ObjRef,
        bytes: Vec<u8>,
    ) -> Result<(), RuntimeError> {
        Self::instance_attr_set(
            instance,
            IO_BUFFERED_ATTR_READ_BUF,
            self.heap.alloc_bytes(bytes),
        )
    }

    fn io_buffered_buffer_size(instance: &ObjRef) -> usize {
        match Self::instance_attr_get(instance, IO_BUFFERED_ATTR_BUF_SIZE) {
            Some(Value::Int(value)) if value > 0 => value as usize,
            Some(Value::Bool(true)) => 1,
            Some(Value::BigInt(value)) => value
                .to_i64()
                .filter(|size| *size > 0)
                .map(|size| size as usize)
                .unwrap_or(IO_BUFFERED_DEFAULT_SIZE as usize),
            _ => IO_BUFFERED_DEFAULT_SIZE as usize,
        }
    }

    fn io_value_is_user_rawio_instance(value: &Value) -> bool {
        let Value::Instance(instance) = value else {
            return false;
        };
        let Object::Instance(instance_data) = &*instance.kind() else {
            return false;
        };
        let Object::Class(class_data) = &*instance_data.class.kind() else {
            return false;
        };
        let is_user_class = matches!(
            class_data.attrs.get("__pyrs_user_class__"),
            Some(Value::Bool(true))
        );
        if !is_user_class {
            return false;
        }
        class_attr_walk(&instance_data.class)
            .into_iter()
            .any(|candidate| matches!(&*candidate.kind(), Object::Class(class_data) if class_data.name == "RawIOBase"))
    }

    fn io_buffered_readinto_chunk(
        &mut self,
        instance: &ObjRef,
        read_size: usize,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        let target = self.heap.alloc_bytearray(vec![0; read_size]);
        let read_result = self.io_buffered_delegate_method(
            instance,
            "readinto",
            vec![target.clone()],
            HashMap::new(),
            "buffered read failed",
        )?;
        let count = match read_result {
            Value::None => return Ok(None),
            Value::Int(value) => value,
            Value::Bool(value) => i64::from(value),
            Value::BigInt(value) => value.to_i64().ok_or_else(|| {
                RuntimeError::new("OSError: raw readinto() returned invalid length")
            })?,
            other => {
                return Err(RuntimeError::new(format!(
                    "TypeError: readinto() should return integer or None, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        if count < 0 || (count as usize) > read_size {
            return Err(RuntimeError::new(
                "OSError: raw readinto() returned invalid length",
            ));
        }
        let bytes = match target {
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(values) => values[..count as usize].to_vec(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        Ok(Some(bytes))
    }

    fn io_buffered_mark_closed(instance: &ObjRef) -> Result<(), RuntimeError> {
        Self::instance_attr_set(instance, "__IOBase_closed", Value::Bool(true))?;
        Self::instance_attr_set(instance, "_closed", Value::Bool(true))?;
        Self::instance_attr_set(instance, "closed", Value::Bool(true))?;
        Ok(())
    }

    fn io_buffered_rwpair_endpoint(instance: &ObjRef, attr: &str) -> Result<Value, RuntimeError> {
        Self::instance_attr_get(instance, attr)
            .ok_or_else(|| RuntimeError::new("BufferedRWPair is uninitialized"))
    }

    fn io_buffered_delegate_method(
        &mut self,
        instance: &ObjRef,
        method_name: &str,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        error_context: &str,
    ) -> Result<Value, RuntimeError> {
        let raw = Self::io_buffered_raw_from_instance(instance)?;
        let method = self.builtin_getattr(
            vec![raw, Value::Str(method_name.to_string())],
            HashMap::new(),
        )?;
        match self.call_internal_preserving_caller(method, args, kwargs)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception(error_context))
            }
        }
    }

    fn io_buffered_rwpair_delegate_method(
        &mut self,
        instance: &ObjRef,
        endpoint_attr: &str,
        method_name: &str,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        error_context: &str,
    ) -> Result<Value, RuntimeError> {
        let endpoint = Self::io_buffered_rwpair_endpoint(instance, endpoint_attr)?;
        let method = self.builtin_getattr(
            vec![endpoint, Value::Str(method_name.to_string())],
            HashMap::new(),
        )?;
        match self.call_internal(method, args, kwargs)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception(error_context))
            }
        }
    }

    fn io_buffered_probe_raw_readinto_for_readline(
        &mut self,
        instance: &ObjRef,
    ) -> Result<(), RuntimeError> {
        let Some(Value::Instance(raw)) = Self::instance_attr_get(instance, "raw") else {
            return Ok(());
        };
        let readinto = self.builtin_getattr(
            vec![Value::Instance(raw), Value::Str("readinto".to_string())],
            HashMap::new(),
        )?;
        let probe = match self.heap.alloc_bytearray(Vec::new()) {
            Value::ByteArray(obj) => Value::ByteArray(obj),
            _ => unreachable!(),
        };
        let result = match self.call_internal(readinto, vec![probe], HashMap::new())? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(self.runtime_error_from_active_exception("RawIOBase.readinto failed"));
            }
        };
        match result {
            Value::None => Ok(()),
            Value::Int(value) => {
                if value == 0 {
                    Ok(())
                } else {
                    Err(RuntimeError::new(format!(
                        "ValueError: readinto returned {value} outside buffer size 0",
                    )))
                }
            }
            Value::Bool(value) => {
                if !value {
                    Ok(())
                } else {
                    Err(RuntimeError::new(
                        "ValueError: readinto returned 1 outside buffer size 0",
                    ))
                }
            }
            Value::BigInt(value) => {
                let read_count = value.to_i64().ok_or_else(|| {
                    RuntimeError::new("ValueError: readinto returned value outside buffer size")
                })?;
                if read_count == 0 {
                    Ok(())
                } else {
                    Err(RuntimeError::new(format!(
                        "ValueError: readinto returned {read_count} outside buffer size 0",
                    )))
                }
            }
            other => Err(RuntimeError::new(format!(
                "TypeError: readinto() should return integer or None, not {}",
                self.value_type_name_for_error(&other)
            ))),
        }
    }

    fn io_exception_object_from_runtime_error(
        &mut self,
        err: RuntimeError,
    ) -> Result<ExceptionObject, RuntimeError> {
        let classified = classify_runtime_error(&err.message);
        let exception_type = if classified == "RuntimeError" {
            extract_runtime_error_exception_name(&err.message)
                .unwrap_or_else(|| classified.to_string())
        } else {
            classified.to_string()
        };
        let mut exception_message = Some(err.message.clone());
        if let Some(from_traceback) =
            extract_runtime_error_final_message(&err.message, &exception_type)
        {
            exception_message = from_traceback;
        } else if let Some(from_prefixed) =
            extract_prefixed_exception_message(&err.message, &exception_type)
        {
            exception_message = from_prefixed;
        }
        let exception = ExceptionObject::new(exception_type.clone(), exception_message);
        if is_os_error_family(exception_type.as_str()) {
            if let Some(errno) =
                extract_os_error_errno(&err.message).or_else(|| infer_os_error_errno(&err.message))
            {
                exception
                    .attrs
                    .borrow_mut()
                    .insert("errno".to_string(), Value::Int(errno));
            }
            if let Some(strerror) = extract_os_error_strerror(&err.message) {
                exception
                    .attrs
                    .borrow_mut()
                    .insert("strerror".to_string(), Value::Str(strerror));
            }
        }
        let args = if is_os_error_family(exception_type.as_str()) {
            if let Some(errno) = exception.attrs.borrow().get("errno").cloned() {
                let mut items = vec![errno];
                if let Some(strerror) = exception.attrs.borrow().get("strerror").cloned() {
                    items.push(strerror);
                }
                self.heap.alloc_tuple(items)
            } else if let Some(message) = &exception.message {
                self.heap.alloc_tuple(vec![Value::Str(message.clone())])
            } else {
                self.heap.alloc_tuple(Vec::new())
            }
        } else if let Some(message) = &exception.message {
            self.heap.alloc_tuple(vec![Value::Str(message.clone())])
        } else {
            self.heap.alloc_tuple(Vec::new())
        };
        exception
            .attrs
            .borrow_mut()
            .insert("args".to_string(), args);
        Ok(exception)
    }

    fn io_exception_value_from_runtime_error(
        &mut self,
        err: RuntimeError,
    ) -> Result<Value, RuntimeError> {
        Ok(Value::Exception(Box::new(
            self.io_exception_object_from_runtime_error(err)?,
        )))
    }

    fn io_take_active_exception_value(&mut self, fallback: &str) -> Result<Value, RuntimeError> {
        let value = self
            .frames
            .last_mut()
            .and_then(|frame| frame.active_exception.take())
            .ok_or_else(|| RuntimeError::new(fallback))?;
        self.normalize_exception_value(value)
            .map_err(|_| RuntimeError::new(fallback))
    }

    fn io_runtime_error_from_exception_value(&self, exc: &Value, fallback: &str) -> RuntimeError {
        match exc {
            Value::Exception(exception) => {
                let mut line = exception.name.clone();
                if let Some(message) = &exception.message {
                    if !message.is_empty() {
                        line.push_str(": ");
                        line.push_str(message);
                    }
                }
                RuntimeError::new(line)
            }
            _ => RuntimeError::new(fallback),
        }
    }

    pub(super) fn builtin_io_buffered_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.__init__")?;
        let owner_name = match &*instance.kind() {
            Object::Instance(instance_data) => match &*instance_data.class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "BufferedIOBase".to_string(),
            },
            _ => "BufferedIOBase".to_string(),
        };
        if args.is_empty() {
            return Err(RuntimeError::new(format!(
                "{owner_name}() missing required argument 'raw'"
            )));
        }
        let raw = args.remove(0);
        let mut buffer_size = IO_BUFFERED_DEFAULT_SIZE;
        if let Some(buffer_size_value) = kwargs.remove("buffer_size") {
            buffer_size = self.io_index_arg_to_int(buffer_size_value)?;
        } else if !args.is_empty() {
            buffer_size = self.io_index_arg_to_int(args.remove(0))?;
        }
        if buffer_size <= 0 {
            let _ = Self::instance_attr_set(&instance, "raw", Value::None);
            let _ = self.io_buffered_store_read_buffer(&instance, Vec::new());
            return Err(RuntimeError::new(
                "ValueError: buffer_size must be positive",
            ));
        }
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new(format!(
                "{owner_name}() received unexpected arguments"
            )));
        }
        for required in ["read", "readline", "write", "seek", "tell"] {
            let method = self.builtin_getattr(
                vec![raw.clone(), Value::Str(required.to_string())],
                HashMap::new(),
            )?;
            if !self.is_callable_value(&method) {
                return Err(RuntimeError::new(
                    "TypeError: raw stream does not provide required I/O methods",
                ));
            }
        }
        Self::instance_attr_set(&instance, "raw", raw)?;
        Self::instance_attr_set(&instance, "__IOBase_closed", Value::Bool(false))?;
        Self::instance_attr_set(&instance, "_closed", Value::Bool(false))?;
        Self::instance_attr_set(&instance, "closed", Value::Bool(false))?;
        Self::instance_attr_set(
            &instance,
            IO_BUFFERED_ATTR_BUF_SIZE,
            Value::Int(buffer_size),
        )?;
        self.io_buffered_store_read_buffer(&instance, Vec::new())?;
        Ok(Value::None)
    }

    pub(super) fn builtin_io_buffered_read(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "BufferedIOBase.read expects 0-1 arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.read")?;
        if Self::io_buffered_is_uninitialized(&instance) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on uninitialized object",
            ));
        }
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new("ValueError: read of closed file"));
        }
        let readable = self.builtin_io_buffered_readable(
            vec![Value::Instance(instance.clone())],
            HashMap::new(),
        )?;
        if !is_truthy(&readable) {
            return Err(RuntimeError::new("UnsupportedOperation: not readable"));
        }
        let size = match args.pop() {
            None | Some(Value::None) => -1,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        if size < -1 {
            return Err(RuntimeError::new(
                "ValueError: read length must be non-negative",
            ));
        }
        if size == 0 {
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }

        let mut cached = Self::io_buffered_read_buffer(&instance)?;
        let mut output = Vec::new();
        if size < 0 {
            if !cached.is_empty() {
                output.extend_from_slice(&cached);
                cached.clear();
            }
            loop {
                let value = self.io_buffered_delegate_method(
                    &instance,
                    "read",
                    Vec::new(),
                    HashMap::new(),
                    "buffered read failed",
                )?;
                if matches!(value, Value::None) {
                    if output.is_empty() {
                        return Ok(Value::None);
                    }
                    break;
                }
                let chunk = bytes_like_from_value(value)
                    .map_err(|_| RuntimeError::new("TypeError: read() should return bytes"))?;
                if chunk.is_empty() {
                    break;
                }
                output.extend_from_slice(&chunk);
            }
            self.io_buffered_store_read_buffer(&instance, cached)?;
            return Ok(self.heap.alloc_bytes(output));
        }

        let requested = size as usize;
        if cached.len() >= requested {
            let remainder = cached.split_off(requested);
            output.extend_from_slice(&cached);
            self.io_buffered_store_read_buffer(&instance, remainder)?;
            return Ok(self.heap.alloc_bytes(output));
        }

        if !cached.is_empty() {
            output.extend_from_slice(&cached);
            cached.clear();
        }

        let buffer_size = Self::io_buffered_buffer_size(&instance).max(1);
        let use_rawio_readinto = Self::io_buffered_raw_from_instance(&instance)
            .map(|raw| Self::io_value_is_user_rawio_instance(&raw))
            .unwrap_or(false);
        while output.len() < requested {
            let remaining = requested - output.len();
            if use_rawio_readinto {
                let read_size = buffer_size.max(remaining);
                let Some(chunk) = self.io_buffered_readinto_chunk(&instance, read_size)? else {
                    if output.is_empty() {
                        self.io_buffered_store_read_buffer(&instance, cached)?;
                        return Ok(Value::None);
                    }
                    break;
                };
                if chunk.is_empty() {
                    break;
                }
                if chunk.len() <= remaining {
                    output.extend_from_slice(&chunk);
                    continue;
                }
                output.extend_from_slice(&chunk[..remaining]);
                cached.extend_from_slice(&chunk[remaining..]);
                break;
            }
            let read_size = buffer_size.max(remaining);
            let value = self.io_buffered_delegate_method(
                &instance,
                "read",
                vec![Value::Int(read_size as i64)],
                HashMap::new(),
                "buffered read failed",
            )?;
            if matches!(value, Value::None) {
                if output.is_empty() {
                    self.io_buffered_store_read_buffer(&instance, cached)?;
                    return Ok(Value::None);
                }
                break;
            }
            let chunk = bytes_like_from_value(value)
                .map_err(|_| RuntimeError::new("TypeError: read() should return bytes"))?;
            if chunk.len() > read_size {
                return Err(RuntimeError::new(
                    "OSError: raw read() returned invalid length",
                ));
            }
            if chunk.is_empty() {
                break;
            }
            if chunk.len() <= remaining {
                output.extend_from_slice(&chunk);
                continue;
            }
            output.extend_from_slice(&chunk[..remaining]);
            cached.extend_from_slice(&chunk[remaining..]);
            break;
        }
        self.io_buffered_store_read_buffer(&instance, cached)?;
        Ok(self.heap.alloc_bytes(output))
    }

    pub(super) fn builtin_io_buffered_read1(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "BufferedIOBase.read1 expects 0-1 arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.read1")?;
        if Self::io_buffered_is_uninitialized(&instance) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on uninitialized object",
            ));
        }
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new("ValueError: read of closed file"));
        }
        let readable = self.builtin_io_buffered_readable(
            vec![Value::Instance(instance.clone())],
            HashMap::new(),
        )?;
        if !is_truthy(&readable) {
            return Err(RuntimeError::new("UnsupportedOperation: not readable"));
        }
        let size = match args.pop() {
            None | Some(Value::None) => -1,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        if size < -1 {
            return Err(RuntimeError::new(
                "ValueError: read length must be non-negative",
            ));
        }
        if size == 0 {
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }

        let mut cached = Self::io_buffered_read_buffer(&instance)?;
        if !cached.is_empty() {
            if size < 0 || cached.len() <= size as usize {
                self.io_buffered_store_read_buffer(&instance, Vec::new())?;
                return Ok(self.heap.alloc_bytes(cached));
            }
            let requested = size as usize;
            let remainder = cached.split_off(requested);
            self.io_buffered_store_read_buffer(&instance, remainder)?;
            return Ok(self.heap.alloc_bytes(cached));
        }

        let read_size = if size < 0 {
            Self::io_buffered_buffer_size(&instance).max(1)
        } else {
            size as usize
        };
        let use_rawio_readinto = Self::io_buffered_raw_from_instance(&instance)
            .map(|raw| Self::io_value_is_user_rawio_instance(&raw))
            .unwrap_or(false);
        let chunk = if use_rawio_readinto {
            match self.io_buffered_readinto_chunk(&instance, read_size)? {
                Some(chunk) => chunk,
                None => return Ok(self.heap.alloc_bytes(Vec::new())),
            }
        } else {
            let value = self.io_buffered_delegate_method(
                &instance,
                "read",
                vec![Value::Int(read_size as i64)],
                HashMap::new(),
                "buffered read failed",
            )?;
            if matches!(value, Value::None) {
                return Ok(self.heap.alloc_bytes(Vec::new()));
            }
            let chunk = bytes_like_from_value(value)
                .map_err(|_| RuntimeError::new("TypeError: read() should return bytes"))?;
            if chunk.len() > read_size {
                return Err(RuntimeError::new(
                    "OSError: raw read() returned invalid length",
                ));
            }
            chunk
        };

        if chunk.is_empty() {
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }

        if size >= 0 && chunk.len() > size as usize {
            let requested = size as usize;
            self.io_buffered_store_read_buffer(&instance, chunk[requested..].to_vec())?;
            return Ok(self.heap.alloc_bytes(chunk[..requested].to_vec()));
        }

        Ok(self.heap.alloc_bytes(chunk))
    }

    pub(super) fn builtin_io_buffered_peek(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "BufferedIOBase.peek expects 0-1 arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.peek")?;
        if Self::io_buffered_is_uninitialized(&instance) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on uninitialized object",
            ));
        }
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new("ValueError: peek of closed file"));
        }
        let readable = self.builtin_io_buffered_readable(
            vec![Value::Instance(instance.clone())],
            HashMap::new(),
        )?;
        if !is_truthy(&readable) {
            return Err(RuntimeError::new("UnsupportedOperation: not readable"));
        }
        let hint = match args.pop() {
            None | Some(Value::None) => 0,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        let min_read = if hint <= 0 { 1 } else { hint as usize };
        let mut cached = Self::io_buffered_read_buffer(&instance)?;
        if cached.is_empty() {
            let read_size = Self::io_buffered_buffer_size(&instance)
                .max(min_read)
                .max(1);
            let use_rawio_readinto = Self::io_buffered_raw_from_instance(&instance)
                .map(|raw| Self::io_value_is_user_rawio_instance(&raw))
                .unwrap_or(false);
            let chunk = if use_rawio_readinto {
                self.io_buffered_readinto_chunk(&instance, read_size)?
            } else {
                let value = self.io_buffered_delegate_method(
                    &instance,
                    "read",
                    vec![Value::Int(read_size as i64)],
                    HashMap::new(),
                    "buffered peek failed",
                )?;
                if matches!(value, Value::None) {
                    None
                } else {
                    let chunk = bytes_like_from_value(value)
                        .map_err(|_| RuntimeError::new("TypeError: read() should return bytes"))?;
                    if chunk.len() > read_size {
                        return Err(RuntimeError::new(
                            "OSError: raw read() returned invalid length",
                        ));
                    }
                    Some(chunk)
                }
            };
            if let Some(bytes) = chunk {
                cached.extend_from_slice(&bytes);
            }
            self.io_buffered_store_read_buffer(&instance, cached.clone())?;
        }
        Ok(self.heap.alloc_bytes(cached))
    }

    pub(super) fn builtin_io_buffered_readline(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.readline")?;
        let readable = self.builtin_io_buffered_readable(
            vec![Value::Instance(instance.clone())],
            HashMap::new(),
        )?;
        if !is_truthy(&readable) {
            return Err(RuntimeError::new("UnsupportedOperation: not readable"));
        }
        if let Err(err) = self.io_buffered_probe_raw_readinto_for_readline(&instance) {
            if let Some(cause_message) = err.message.strip_prefix("TypeError: ") {
                let cause = Value::Exception(Box::new(ExceptionObject::new(
                    "TypeError".to_string(),
                    Some(cause_message.trim().to_string()),
                )));
                let os_error = Value::Exception(Box::new(ExceptionObject::new(
                    "OSError".to_string(),
                    Some(cause_message.trim().to_string()),
                )));
                self.raise_exception_with_cause(os_error, Some(cause))?;
                return Err(self.runtime_error_from_active_exception("buffered readline failed"));
            }
            if let Some(message) = err.message.strip_prefix("ValueError: ") {
                let os_error = Value::Exception(Box::new(ExceptionObject::new(
                    "OSError".to_string(),
                    Some(message.trim().to_string()),
                )));
                self.raise_exception(os_error)?;
                return Err(self.runtime_error_from_active_exception("buffered readline failed"));
            }
            return Err(err);
        }
        self.io_buffered_delegate_method(
            &instance,
            "readline",
            args,
            kwargs,
            "buffered readline failed",
        )
    }

    pub(super) fn builtin_io_buffered_write(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.write")?;
        let writable = self.builtin_io_buffered_writable(
            vec![Value::Instance(instance.clone())],
            HashMap::new(),
        )?;
        if !is_truthy(&writable) {
            return Err(RuntimeError::new("UnsupportedOperation: not writable"));
        }
        self.io_buffered_delegate_method(&instance, "write", args, kwargs, "buffered write failed")
    }

    pub(super) fn builtin_io_buffered_flush(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedIOBase.flush expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.flush")?;
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new("I/O operation on closed file."));
        }
        self.io_buffered_delegate_method(
            &instance,
            "flush",
            Vec::new(),
            HashMap::new(),
            "buffered flush failed",
        )
    }

    pub(super) fn builtin_io_buffered_close(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedIOBase.close expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.close")?;
        if Self::iobase_is_closed(&instance) {
            return Ok(Value::None);
        }
        let flush_method = self.builtin_getattr(
            vec![
                Value::Instance(instance.clone()),
                Value::Str("flush".to_string()),
            ],
            HashMap::new(),
        )?;
        let flush_error =
            match self.call_internal_preserving_caller(flush_method, Vec::new(), HashMap::new()) {
                Ok(InternalCallOutcome::Value(_)) => None,
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    Some(self.io_take_active_exception_value("buffered close failed")?)
                }
                Err(err) => Some(self.io_exception_value_from_runtime_error(err)?),
            };
        let raw = Self::io_buffered_raw_from_instance(&instance)?;
        let close_method = self.builtin_getattr(
            vec![raw.clone(), Value::Str("close".to_string())],
            HashMap::new(),
        )?;
        let close_error =
            match self.call_internal_preserving_caller(close_method, Vec::new(), HashMap::new()) {
                Ok(InternalCallOutcome::Value(_)) => None,
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    Some(self.io_take_active_exception_value("buffered close failed")?)
                }
                Err(err) => Some(self.io_exception_value_from_runtime_error(err)?),
            };
        let raw_closed = self
            .builtin_getattr(
                vec![raw.clone(), Value::Str("closed".to_string())],
                HashMap::new(),
            )
            .map(|value| is_truthy(&value))
            .unwrap_or(false);
        if raw_closed {
            Self::io_buffered_mark_closed(&instance)?;
        }
        if let Some(mut close_exc) = close_error {
            if let (Value::Exception(close_exception), Some(Value::Exception(flush_exception))) =
                (&mut close_exc, flush_error.clone())
            {
                close_exception.context = Some(flush_exception);
            }
            if let Some(frame) = self.frames.last_mut() {
                frame.active_exception = Some(close_exc.clone());
            }
            return Err(
                self.io_runtime_error_from_exception_value(&close_exc, "buffered close failed")
            );
        }
        if !raw_closed {
            Self::io_buffered_mark_closed(&instance)?;
        }
        if let Some(flush_exc) = flush_error {
            if let Some(frame) = self.frames.last_mut() {
                frame.active_exception = Some(flush_exc.clone());
            }
            return Err(
                self.io_runtime_error_from_exception_value(&flush_exc, "buffered close failed")
            );
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_io_buffered_detach(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedIOBase.detach expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.detach")?;
        let raw = match Self::instance_attr_get(&instance, "raw") {
            Some(Value::None) => {
                return Err(RuntimeError::new("ValueError: raw stream already detached"));
            }
            Some(value) => value,
            None => return Err(RuntimeError::new("buffered stream is uninitialized")),
        };
        let flush_method = self.builtin_getattr(
            vec![
                Value::Instance(instance.clone()),
                Value::Str("flush".to_string()),
            ],
            HashMap::new(),
        )?;
        match self.call_internal_preserving_caller(flush_method, Vec::new(), HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => {}
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                return Err(self.runtime_error_from_active_exception("buffered detach failed"));
            }
            Err(err) => return Err(err),
        }
        Self::instance_attr_set(&instance, "raw", Value::None)?;
        self.io_buffered_store_read_buffer(&instance, Vec::new())?;
        Ok(raw)
    }

    pub(super) fn builtin_io_buffered_fileno(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedIOBase.fileno expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.fileno")?;
        match self.io_buffered_delegate_method(
            &instance,
            "fileno",
            Vec::new(),
            HashMap::new(),
            "buffered fileno failed",
        ) {
            Ok(value) => Ok(value),
            Err(_) => Err(RuntimeError::new("OSError: fileno")),
        }
    }

    pub(super) fn builtin_io_buffered_seek(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.seek")?;
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on closed file.",
            ));
        }
        let seekable = self.builtin_io_buffered_seekable(
            vec![Value::Instance(instance.clone())],
            HashMap::new(),
        )?;
        if !is_truthy(&seekable) {
            return Err(RuntimeError::new("OSError: not seekable"));
        }
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: seek expected at least 1 argument",
            ));
        }
        if args.len() == 1 {
            args.push(Value::Int(0));
        }
        let whence = value_to_int(args[1].clone())?;
        if !matches!(whence, 0..=2) {
            return Err(RuntimeError::new("ValueError: invalid whence"));
        }
        if whence == 1 {
            let offset = value_to_int(args[0].clone())?;
            let cached = Self::io_buffered_read_buffer(&instance)?;
            args[0] = Value::Int(offset - cached.len() as i64);
        }
        let value = self.io_buffered_delegate_method(
            &instance,
            "seek",
            args,
            kwargs,
            "buffered seek failed",
        )?;
        let position = value_to_int(value)
            .map_err(|_| RuntimeError::new("OSError: seek() returned an invalid position"))?;
        if position < 0 {
            return Err(RuntimeError::new(
                "OSError: seek() returned an invalid position",
            ));
        }
        self.io_buffered_store_read_buffer(&instance, Vec::new())?;
        Ok(Value::Int(position))
    }

    pub(super) fn builtin_io_buffered_tell(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.tell")?;
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on closed file.",
            ));
        }
        let seekable = self.builtin_io_buffered_seekable(
            vec![Value::Instance(instance.clone())],
            HashMap::new(),
        )?;
        if !is_truthy(&seekable) {
            return Err(RuntimeError::new("OSError: not seekable"));
        }
        let raw_position = self.io_buffered_delegate_method(
            &instance,
            "tell",
            args,
            kwargs,
            "buffered tell failed",
        )?;
        let raw_pos = value_to_int(raw_position)
            .map_err(|_| RuntimeError::new("OSError: tell() returned an invalid position"))?;
        if raw_pos < 0 {
            return Err(RuntimeError::new(
                "OSError: tell() returned an invalid position",
            ));
        }
        let cached = Self::io_buffered_read_buffer(&instance)?;
        let adjusted = raw_pos - cached.len() as i64;
        Ok(Value::Int(adjusted.max(0)))
    }

    pub(super) fn builtin_io_buffered_truncate(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.truncate")?;
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on closed file.",
            ));
        }
        let writable = self.builtin_io_buffered_writable(
            vec![Value::Instance(instance.clone())],
            HashMap::new(),
        )?;
        if !is_truthy(&writable) {
            return Err(RuntimeError::new("UnsupportedOperation: truncate"));
        }
        let seekable = self.builtin_io_buffered_seekable(
            vec![Value::Instance(instance.clone())],
            HashMap::new(),
        )?;
        if !is_truthy(&seekable) {
            return Err(RuntimeError::new("OSError: not seekable"));
        }
        self.io_buffered_delegate_method(
            &instance,
            "truncate",
            args,
            kwargs,
            "buffered truncate failed",
        )
    }

    pub(super) fn builtin_io_buffered_readable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedIOBase.readable expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.readable")?;
        let class_name = match &*instance.kind() {
            Object::Instance(instance_data) => match &*instance_data.class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        if matches!(class_name.as_str(), "BufferedIOBase" | "BufferedWriter") {
            return Ok(Value::Bool(false));
        }
        match self.io_buffered_delegate_method(
            &instance,
            "readable",
            Vec::new(),
            HashMap::new(),
            "buffered readable failed",
        ) {
            Ok(value) => Ok(Value::Bool(is_truthy(&value))),
            Err(_) => Ok(Value::Bool(false)),
        }
    }

    pub(super) fn builtin_io_buffered_writable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedIOBase.writable expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.writable")?;
        let class_name = match &*instance.kind() {
            Object::Instance(instance_data) => match &*instance_data.class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        if matches!(class_name.as_str(), "BufferedIOBase" | "BufferedReader") {
            return Ok(Value::Bool(false));
        }
        match self.io_buffered_delegate_method(
            &instance,
            "writable",
            Vec::new(),
            HashMap::new(),
            "buffered writable failed",
        ) {
            Ok(value) => Ok(Value::Bool(is_truthy(&value))),
            Err(_) => Ok(Value::Bool(false)),
        }
    }

    pub(super) fn builtin_io_buffered_seekable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedIOBase.seekable expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.seekable")?;
        let class_name = match &*instance.kind() {
            Object::Instance(instance_data) => match &*instance_data.class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        if class_name == "BufferedIOBase" {
            return Ok(Value::Bool(false));
        }
        match self.io_buffered_delegate_method(
            &instance,
            "seekable",
            Vec::new(),
            HashMap::new(),
            "buffered seekable failed",
        ) {
            Ok(value) => Ok(Value::Bool(is_truthy(&value))),
            Err(_) => Ok(Value::Bool(false)),
        }
    }

    pub(super) fn builtin_io_buffered_rwpair_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedRWPair.__init__")?;
        if args.len() < 2 {
            return Err(RuntimeError::new(
                "BufferedRWPair.__init__() missing required arguments",
            ));
        }
        let reader = args.remove(0);
        let writer = args.remove(0);
        if let Some(buffer_size) = kwargs.remove("buffer_size") {
            let _ = value_to_int(buffer_size)?;
        } else if !args.is_empty() {
            let _ = value_to_int(args.remove(0))?;
        }
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "BufferedRWPair.__init__() received unexpected arguments",
            ));
        }
        let reader_readable = self.builtin_getattr(
            vec![reader.clone(), Value::Str("readable".to_string())],
            HashMap::new(),
        )?;
        let reader_readable =
            match self.call_internal(reader_readable, Vec::new(), HashMap::new())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception(
                        "BufferedRWPair readable check failed",
                    ));
                }
            };
        if !is_truthy(&reader_readable) {
            return Err(RuntimeError::new("OSError: readable stream expected"));
        }
        let writer_writable = self.builtin_getattr(
            vec![writer.clone(), Value::Str("writable".to_string())],
            HashMap::new(),
        )?;
        let writer_writable =
            match self.call_internal(writer_writable, Vec::new(), HashMap::new())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception(
                        "BufferedRWPair writable check failed",
                    ));
                }
            };
        if !is_truthy(&writer_writable) {
            return Err(RuntimeError::new("OSError: writable stream expected"));
        }
        Self::instance_attr_set(&instance, "__pyrs_rwpair_reader", reader)?;
        Self::instance_attr_set(&instance, "__pyrs_rwpair_writer", writer)?;
        Self::instance_attr_set(&instance, "__IOBase_closed", Value::Bool(false))?;
        Self::instance_attr_set(&instance, "_closed", Value::Bool(false))?;
        Self::instance_attr_set(&instance, "closed", Value::Bool(false))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_io_buffered_rwpair_read(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedRWPair.read")?;
        self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_reader",
            "read",
            args,
            kwargs,
            "BufferedRWPair.read failed",
        )
    }

    pub(super) fn builtin_io_buffered_rwpair_readline(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedRWPair.readline")?;
        self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_reader",
            "readline",
            args,
            kwargs,
            "BufferedRWPair.readline failed",
        )
    }

    pub(super) fn builtin_io_buffered_rwpair_read1(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedRWPair.read1")?;
        match self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_reader",
            "read1",
            args.clone(),
            kwargs.clone(),
            "BufferedRWPair.read1 failed",
        ) {
            Ok(value) => Ok(value),
            Err(_) => self.io_buffered_rwpair_delegate_method(
                &instance,
                "__pyrs_rwpair_reader",
                "read",
                args,
                kwargs,
                "BufferedRWPair.read1 failed",
            ),
        }
    }

    pub(super) fn builtin_io_buffered_rwpair_readinto(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedRWPair.readinto")?;
        self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_reader",
            "readinto",
            args,
            kwargs,
            "BufferedRWPair.readinto failed",
        )
    }

    pub(super) fn builtin_io_buffered_rwpair_readinto1(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedRWPair.readinto1")?;
        match self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_reader",
            "readinto1",
            args.clone(),
            kwargs.clone(),
            "BufferedRWPair.readinto1 failed",
        ) {
            Ok(value) => Ok(value),
            Err(_) => self.io_buffered_rwpair_delegate_method(
                &instance,
                "__pyrs_rwpair_reader",
                "readinto",
                args,
                kwargs,
                "BufferedRWPair.readinto1 failed",
            ),
        }
    }

    pub(super) fn builtin_io_buffered_rwpair_write(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "BufferedRWPair.write")?;
        self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_writer",
            "write",
            args,
            kwargs,
            "BufferedRWPair.write failed",
        )
    }

    pub(super) fn builtin_io_buffered_rwpair_flush(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedRWPair.flush expects no arguments",
            ));
        }
        let instance = self.receiver_from_value(&args.remove(0))?;
        self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_writer",
            "flush",
            Vec::new(),
            HashMap::new(),
            "BufferedRWPair.flush failed",
        )
    }

    pub(super) fn builtin_io_buffered_rwpair_close(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedRWPair.close expects no arguments",
            ));
        }
        let instance = self.receiver_from_value(&args.remove(0))?;
        if Self::iobase_is_closed(&instance) {
            return Ok(Value::None);
        }
        let writer = Self::io_buffered_rwpair_endpoint(&instance, "__pyrs_rwpair_writer")?;
        let reader = Self::io_buffered_rwpair_endpoint(&instance, "__pyrs_rwpair_reader")?;
        let close_endpoint = |vm: &mut Vm, endpoint: Value, label: &str| -> Option<RuntimeError> {
            let method = match vm.builtin_getattr(
                vec![endpoint, Value::Str("close".to_string())],
                HashMap::new(),
            ) {
                Ok(value) => value,
                Err(err) => return Some(err),
            };
            match vm.call_internal(method, Vec::new(), HashMap::new()) {
                Ok(InternalCallOutcome::Value(_)) => None,
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    Some(vm.runtime_error_from_active_exception(label))
                }
                Err(err) => Some(err),
            }
        };
        let writer_error = close_endpoint(self, writer, "BufferedRWPair.close writer failed");
        let reader_error = close_endpoint(self, reader, "BufferedRWPair.close reader failed");
        if writer_error.is_none() {
            Self::iobase_mark_closed(&instance)?;
        }
        if let Some(err) = writer_error {
            return Err(err);
        }
        if let Some(err) = reader_error {
            Self::iobase_mark_closed(&instance)?;
            return Err(err);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_io_buffered_rwpair_readable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedRWPair.readable expects no arguments",
            ));
        }
        let instance = self.receiver_from_value(&args.remove(0))?;
        let value = self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_reader",
            "readable",
            Vec::new(),
            HashMap::new(),
            "BufferedRWPair.readable failed",
        )?;
        Ok(Value::Bool(is_truthy(&value)))
    }

    pub(super) fn builtin_io_buffered_rwpair_writable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedRWPair.writable expects no arguments",
            ));
        }
        let instance = self.receiver_from_value(&args.remove(0))?;
        let value = self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_writer",
            "writable",
            Vec::new(),
            HashMap::new(),
            "BufferedRWPair.writable failed",
        )?;
        Ok(Value::Bool(is_truthy(&value)))
    }

    pub(super) fn builtin_io_buffered_rwpair_seekable(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedRWPair.seekable expects no arguments",
            ));
        }
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_io_buffered_rwpair_detach(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BufferedRWPair.detach expects no arguments",
            ));
        }
        Err(RuntimeError::new("UnsupportedOperation: detach"))
    }

    pub(super) fn builtin_io_buffered_rwpair_peek(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "BufferedRWPair.peek expects at most one argument",
            ));
        }
        let instance = self.receiver_from_value(&args.remove(0))?;
        let size = if args.is_empty() {
            0
        } else {
            value_to_int(args.remove(0))?
        };
        let request = if size <= 0 { 1 } else { size };
        if let Ok(value) = self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_reader",
            "peek",
            vec![Value::Int(request)],
            HashMap::new(),
            "BufferedRWPair.peek failed",
        ) {
            return Ok(value);
        }
        let reader = Self::io_buffered_rwpair_endpoint(&instance, "__pyrs_rwpair_reader")?;
        let tell = self.builtin_getattr(
            vec![reader.clone(), Value::Str("tell".to_string())],
            HashMap::new(),
        );
        let seek = self.builtin_getattr(
            vec![reader.clone(), Value::Str("seek".to_string())],
            HashMap::new(),
        );
        if let (Ok(tell_method), Ok(seek_method)) = (tell, seek) {
            let pos_value = match self.call_internal(tell_method, Vec::new(), HashMap::new())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(
                        self.runtime_error_from_active_exception("BufferedRWPair.peek failed")
                    );
                }
            };
            let pos = value_to_int(pos_value)?;
            let read_value = self.io_buffered_rwpair_delegate_method(
                &instance,
                "__pyrs_rwpair_reader",
                "read",
                vec![Value::Int(request)],
                HashMap::new(),
                "BufferedRWPair.peek failed",
            )?;
            let _ = self.call_internal(
                seek_method,
                vec![Value::Int(pos), Value::Int(0)],
                HashMap::new(),
            );
            return Ok(read_value);
        }
        self.io_buffered_rwpair_delegate_method(
            &instance,
            "__pyrs_rwpair_reader",
            "read",
            vec![Value::Int(request)],
            HashMap::new(),
            "BufferedRWPair.peek failed",
        )
    }

    pub(super) fn io_writable_buffer_len(&self, target: &Value) -> Result<usize, RuntimeError> {
        match target {
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(values) => Ok(values.len()),
                _ => Err(RuntimeError::new(
                    "TypeError: readinto() argument must be read-write bytes-like object",
                )),
            },
            Value::MemoryView(view_obj) => {
                let (source, start, length) = match &*view_obj.kind() {
                    Object::MemoryView(view) => (view.source.clone(), view.start, view.length),
                    _ => {
                        return Err(RuntimeError::new(
                            "TypeError: readinto() argument must be read-write bytes-like object",
                        ));
                    }
                };
                match &*source.kind() {
                    Object::ByteArray(values) => {
                        let (start, end) = memoryview_bounds(start, length, values.len());
                        Ok(end.saturating_sub(start))
                    }
                    _ => Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    )),
                }
            }
            Value::Module(module_obj) => {
                let Object::Module(module_data) = &*module_obj.kind() else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                if module_data.name != "__array__" {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                }
                let Some(Value::List(values_obj)) = module_data.globals.get("values") else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                let Object::List(values) = &*values_obj.kind() else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                Ok(values.len())
            }
            _ => Err(RuntimeError::new(
                "TypeError: readinto() argument must be read-write bytes-like object",
            )),
        }
    }

    pub(super) fn io_copy_into_writable_buffer(
        &mut self,
        target: Value,
        payload: &[u8],
    ) -> Result<usize, RuntimeError> {
        match target {
            Value::ByteArray(obj) => {
                let Object::ByteArray(values) = &mut *obj.kind_mut() else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                let count = values.len().min(payload.len());
                values[..count].copy_from_slice(&payload[..count]);
                Ok(count)
            }
            Value::MemoryView(view_obj) => {
                let (source, start, length) = match &*view_obj.kind() {
                    Object::MemoryView(view) => (view.source.clone(), view.start, view.length),
                    _ => {
                        return Err(RuntimeError::new(
                            "TypeError: readinto() argument must be read-write bytes-like object",
                        ));
                    }
                };
                let Object::ByteArray(values) = &mut *source.kind_mut() else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                let (start, end) = memoryview_bounds(start, length, values.len());
                let span = end.saturating_sub(start);
                let count = span.min(payload.len());
                values[start..start + count].copy_from_slice(&payload[..count]);
                Ok(count)
            }
            Value::Module(module_obj) => {
                let Object::Module(module_data) = &mut *module_obj.kind_mut() else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                if module_data.name != "__array__" {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                }
                let Some(Value::List(values_obj)) = module_data.globals.get_mut("values") else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                let Object::List(values) = &mut *values_obj.kind_mut() else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                let count = values.len().min(payload.len());
                for (slot, byte) in values.iter_mut().zip(payload.iter()).take(count) {
                    *slot = Value::Int(*byte as i64);
                }
                Ok(count)
            }
            _ => Err(RuntimeError::new(
                "TypeError: readinto() argument must be read-write bytes-like object",
            )),
        }
    }

    fn builtin_io_buffered_readinto_impl(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        method_name: &str,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "BufferedIOBase.readinto expects 1 argument",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "BufferedIOBase.readinto")?;
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on closed file.",
            ));
        }
        let target = args.remove(0);
        let request = self.io_writable_buffer_len(&target)?;
        if request == 0 {
            return Ok(Value::Int(0));
        }
        if method_name == "read1" && !Self::io_buffered_is_uninitialized(&instance) {
            let payload = self.io_buffered_readinto1_payload(&instance, request)?;
            let copied = self.io_copy_into_writable_buffer(target, &payload)?;
            return Ok(Value::Int(copied as i64));
        }
        let read_method = match self.builtin_getattr(
            vec![
                Value::Instance(instance.clone()),
                Value::Str(method_name.to_string()),
            ],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(_err) if method_name == "read1" => self.builtin_getattr(
                vec![Value::Instance(instance), Value::Str("read".to_string())],
                HashMap::new(),
            )?,
            Err(err) => return Err(err),
        };
        let payload_value = match self.call_internal(
            read_method,
            vec![Value::Int(request as i64)],
            HashMap::new(),
        )? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(
                    self.runtime_error_from_active_exception("BufferedIOBase.readinto failed")
                );
            }
        };
        let payload = bytes_like_from_value(payload_value)
            .map_err(|_| RuntimeError::new("TypeError: read() should return bytes"))?;
        let copied = self.io_copy_into_writable_buffer(target, &payload)?;
        Ok(Value::Int(copied as i64))
    }

    fn io_buffered_readinto1_payload(
        &mut self,
        instance: &ObjRef,
        request: usize,
    ) -> Result<Vec<u8>, RuntimeError> {
        let mut payload = Vec::new();
        let mut cached = Self::io_buffered_read_buffer(instance)?;
        if !cached.is_empty() {
            let take = cached.len().min(request);
            payload.extend_from_slice(&cached[..take]);
            cached.drain(..take);
            self.io_buffered_store_read_buffer(instance, cached)?;
            if payload.len() == request {
                return Ok(payload);
            }
        }

        let remaining = request - payload.len();
        if remaining == 0 {
            return Ok(payload);
        }

        let buffer_size = Self::io_buffered_buffer_size(instance).max(1);
        let use_rawio_readinto = Self::io_buffered_raw_from_instance(instance)
            .map(|raw| Self::io_value_is_user_rawio_instance(&raw))
            .unwrap_or(false);

        let read_from_raw =
            |vm: &mut Self, read_size: usize| -> Result<Option<Vec<u8>>, RuntimeError> {
                if use_rawio_readinto {
                    return vm.io_buffered_readinto_chunk(instance, read_size);
                }
                let value = vm.io_buffered_delegate_method(
                    instance,
                    "read",
                    vec![Value::Int(read_size as i64)],
                    HashMap::new(),
                    "buffered readinto1 failed",
                )?;
                if matches!(value, Value::None) {
                    return Ok(None);
                }
                let chunk = bytes_like_from_value(value)
                    .map_err(|_| RuntimeError::new("TypeError: read() should return bytes"))?;
                if chunk.len() > read_size {
                    return Err(RuntimeError::new(
                        "OSError: raw read() returned invalid length",
                    ));
                }
                Ok(Some(chunk))
            };

        if remaining > buffer_size {
            if let Some(chunk) = read_from_raw(self, remaining)? {
                payload.extend_from_slice(&chunk);
            }
            return Ok(payload);
        }

        if payload.is_empty() {
            let Some(chunk) = read_from_raw(self, buffer_size)? else {
                return Ok(payload);
            };
            if !chunk.is_empty() {
                let take = chunk.len().min(remaining);
                payload.extend_from_slice(&chunk[..take]);
                if chunk.len() > take {
                    self.io_buffered_store_read_buffer(instance, chunk[take..].to_vec())?;
                }
            }
        }

        Ok(payload)
    }

    pub(super) fn builtin_io_buffered_readinto(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_io_buffered_readinto_impl(args, kwargs, "read")
    }

    pub(super) fn builtin_io_buffered_readinto1(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_io_buffered_readinto_impl(args, kwargs, "read1")
    }

    fn io_raw_readinto_chunk(
        &mut self,
        receiver: &ObjRef,
        size: usize,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        if size == 0 {
            return Ok(Some(Vec::new()));
        }
        let buffer_obj = match self.heap.alloc_bytearray(vec![0; size]) {
            Value::ByteArray(obj) => obj,
            _ => unreachable!(),
        };
        let readinto = self.builtin_getattr(
            vec![
                Value::Instance(receiver.clone()),
                Value::Str("readinto".to_string()),
            ],
            HashMap::new(),
        )?;
        let result = match self.call_internal(
            readinto,
            vec![Value::ByteArray(buffer_obj.clone())],
            HashMap::new(),
        )? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(self.runtime_error_from_active_exception("RawIOBase.readinto failed"));
            }
        };
        let Some(read_count) = (match result {
            Value::None => None,
            Value::Int(value) => Some(value),
            Value::Bool(value) => Some(i64::from(value)),
            Value::BigInt(value) => Some(value.to_i64().ok_or_else(|| {
                RuntimeError::new("ValueError: readinto returned value outside buffer size")
            })?),
            other => {
                return Err(RuntimeError::new(format!(
                    "TypeError: readinto() should return integer or None, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        }) else {
            return Ok(None);
        };
        if read_count < 0 || read_count as usize > size {
            return Err(RuntimeError::new(format!(
                "ValueError: readinto returned {read_count} outside buffer size {size}",
            )));
        }
        let Object::ByteArray(values) = &*buffer_obj.kind() else {
            return Err(RuntimeError::new(
                "RawIOBase.readinto produced invalid buffer",
            ));
        };
        Ok(Some(values[..read_count as usize].to_vec()))
    }

    pub(super) fn builtin_io_raw_read(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("RawIOBase.read expects 0-1 arguments"));
        }
        let receiver = self.take_bound_instance_arg(&mut args, "RawIOBase.read")?;
        let size = match args.pop() {
            None | Some(Value::None) => -1,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        if size < 0 {
            return self.builtin_io_raw_readall(vec![Value::Instance(receiver)], HashMap::new());
        }
        let Some(payload) = self.io_raw_readinto_chunk(&receiver, size as usize)? else {
            return Ok(Value::None);
        };
        Ok(self.heap.alloc_bytes(payload))
    }

    pub(super) fn builtin_io_raw_readall(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("RawIOBase.readall expects no arguments"));
        }
        let receiver = self.take_bound_instance_arg(&mut args, "RawIOBase.readall")?;
        let read = self.builtin_getattr(
            vec![
                Value::Instance(receiver.clone()),
                Value::Str("read".to_string()),
            ],
            HashMap::new(),
        )?;
        let mut out = Vec::new();
        loop {
            let chunk_value =
                match self.call_internal(read.clone(), vec![Value::Int(8192)], HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(
                            self.runtime_error_from_active_exception("RawIOBase.readall failed")
                        );
                    }
                };
            match chunk_value {
                Value::None => {
                    if out.is_empty() {
                        return Ok(Value::None);
                    }
                    break;
                }
                Value::Bytes(obj) => {
                    let Object::Bytes(values) = &*obj.kind() else {
                        return Err(RuntimeError::new(
                            "RawIOBase.read() returned invalid bytes payload",
                        ));
                    };
                    if values.is_empty() {
                        break;
                    }
                    out.extend_from_slice(values);
                }
                Value::ByteArray(_) | Value::MemoryView(_) => {
                    let values = bytes_like_from_value(chunk_value)
                        .map_err(|_| RuntimeError::new("TypeError: read() should return bytes"))?;
                    if values.is_empty() {
                        break;
                    }
                    out.extend(values);
                }
                other => {
                    return Err(RuntimeError::new(format!(
                        "TypeError: read() should return bytes or None, not {}",
                        self.value_type_name_for_error(&other)
                    )));
                }
            }
        }
        Ok(self.heap.alloc_bytes(out))
    }

    pub(super) fn io_file_fd_from_instance_inner(
        &self,
        instance: &ObjRef,
        depth: usize,
    ) -> Result<i64, RuntimeError> {
        if depth > 8 {
            return Err(RuntimeError::new("invalid file object"));
        }
        let closed = matches!(
            Self::instance_attr_get(instance, "_closed"),
            Some(Value::Bool(true))
        );
        if closed {
            return Err(RuntimeError::new("I/O operation on closed file"));
        }
        match Self::instance_attr_get(instance, "_fd") {
            Some(Value::Int(fd)) => Ok(fd),
            _ => {
                for key in ["buffer", "raw"] {
                    if let Some(Value::Instance(inner)) = Self::instance_attr_get(instance, key) {
                        if inner.id() != instance.id() {
                            return self.io_file_fd_from_instance_inner(&inner, depth + 1);
                        }
                    }
                }
                Err(RuntimeError::new("invalid file object"))
            }
        }
    }

    pub(super) fn io_file_is_binary(instance: &ObjRef) -> bool {
        matches!(
            Self::instance_attr_get(instance, "_binary"),
            Some(Value::Bool(true))
        )
    }

    pub(super) fn io_file_mode(instance: &ObjRef) -> Option<String> {
        match Self::instance_attr_get(instance, "_mode") {
            Some(Value::Str(mode)) => Some(mode),
            _ => None,
        }
    }

    pub(super) fn io_file_newline(instance: &ObjRef) -> Option<String> {
        match Self::instance_attr_get(instance, "_newline") {
            Some(Value::Str(newline)) => Some(newline),
            _ => None,
        }
    }

    pub(super) fn io_file_text_buffer_bytesio(instance: &ObjRef) -> Option<ObjRef> {
        if Self::io_file_is_binary(instance) {
            return None;
        }
        let Some(Value::Instance(buffer)) = Self::instance_attr_get(instance, "buffer") else {
            return None;
        };
        if buffer.id() == instance.id() {
            return None;
        }
        if matches!(
            Self::instance_attr_get(&buffer, "_value"),
            Some(Value::ByteArray(_)) | Some(Value::Bytes(_))
        ) {
            return Some(buffer);
        }
        None
    }

    pub(super) fn io_normalize_universal_newlines(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\r' {
                if chars.peek() == Some(&'\n') {
                    let _ = chars.next();
                }
                out.push('\n');
            } else {
                out.push(ch);
            }
        }
        out
    }

    pub(super) fn io_translate_write_newlines(mut text: String, newline: Option<&str>) -> String {
        match newline {
            Some("") | Some("\n") => text,
            Some("\r") => text.replace('\n', "\r"),
            Some("\r\n") => text.replace('\n', "\r\n"),
            None => {
                if cfg!(windows) {
                    text = text.replace('\n', "\r\n");
                }
                text
            }
            Some(_) => text,
        }
    }

    pub(super) fn io_file_read_bytes(
        &mut self,
        fd: i64,
        size: Option<usize>,
    ) -> Result<Vec<u8>, RuntimeError> {
        let file = self
            .open_files
            .get_mut(&fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        let mut out = Vec::new();
        match size {
            Some(limit) => {
                let mut buf = vec![0u8; limit];
                let count = file
                    .read(&mut buf)
                    .map_err(|err| RuntimeError::new(format!("read failed: {err}")))?;
                buf.truncate(count);
                out = buf;
            }
            None => {
                file.read_to_end(&mut out)
                    .map_err(|err| RuntimeError::new(format!("read failed: {err}")))?;
            }
        }
        Ok(out)
    }

    pub(super) fn io_file_close_instance(&mut self, instance: &ObjRef) -> Result<(), RuntimeError> {
        if matches!(
            Self::instance_attr_get(instance, "_closed"),
            Some(Value::Bool(true))
        ) {
            return Ok(());
        }

        let mut linked = Vec::new();
        if let Some(Value::Instance(buffer)) = Self::instance_attr_get(instance, "buffer") {
            if buffer.id() != instance.id() {
                linked.push(buffer);
            }
        }
        if let Some(Value::Instance(raw)) = Self::instance_attr_get(instance, "raw") {
            if raw.id() != instance.id() && !linked.iter().any(|item| item.id() == raw.id()) {
                linked.push(raw);
            }
        }

        if let Some(Value::Int(fd)) = Self::instance_attr_get(instance, "_fd") {
            let closefd = !matches!(
                Self::instance_attr_get(instance, "_closefd"),
                Some(Value::Bool(false))
            );
            if closefd {
                self.open_files.remove(&fd);
            }
        }
        Self::instance_attr_set(instance, "_closed", Value::Bool(true))?;
        Self::instance_attr_set(instance, "closed", Value::Bool(true))?;
        for nested in linked {
            self.io_file_close_instance(&nested)?;
        }
        Ok(())
    }

    pub(super) fn builtin_io_file_read(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("read() expects optional size"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "read")?;
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on closed file.",
            ));
        }
        let mode = Self::io_file_mode(&instance).unwrap_or_else(|| "r".to_string());
        if !(mode.starts_with('r') || mode.contains('+')) {
            return Err(RuntimeError::new("UnsupportedOperation: not readable"));
        }
        let size = if let Some(value) = args.pop() {
            let size = value_to_int(value)?;
            if size < 0 { None } else { Some(size as usize) }
        } else {
            None
        };
        if let Some(buffer) = Self::io_file_text_buffer_bytesio(&instance) {
            let mut read_args = vec![Value::Instance(buffer)];
            if let Some(limit) = size {
                read_args.push(Value::Int(limit as i64));
            }
            let bytes_value = self.builtin_bytesio_read(read_args, HashMap::new())?;
            let bytes = bytes_like_from_value(bytes_value)?;
            let text = String::from_utf8(bytes)
                .map_err(|_| RuntimeError::new("read() encountered non-UTF-8 bytes"))?;
            let newline = Self::io_file_newline(&instance);
            let normalized = if newline.is_none() {
                Self::io_normalize_universal_newlines(&text)
            } else {
                text
            };
            return Ok(Value::Str(normalized));
        }
        let fd = self.io_file_fd_from_instance(&instance)?;
        let bytes = self.io_file_read_bytes(fd, size)?;
        if Self::io_file_is_binary(&instance) {
            Ok(self.heap.alloc_bytes(bytes))
        } else {
            let text = String::from_utf8(bytes)
                .map_err(|_| RuntimeError::new("read() encountered non-UTF-8 bytes"))?;
            let newline = Self::io_file_newline(&instance);
            let normalized = if newline.is_none() {
                Self::io_normalize_universal_newlines(&text)
            } else {
                text
            };
            Ok(Value::Str(normalized))
        }
    }

    pub(super) fn builtin_io_file_readinto(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("readinto() expects one argument"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "readinto")?;
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on closed file.",
            ));
        }
        let mode = Self::io_file_mode(&instance).unwrap_or_else(|| "r".to_string());
        if !(mode.starts_with('r') || mode.contains('+')) {
            return Err(RuntimeError::new("UnsupportedOperation: not readable"));
        }
        let target = args.remove(0);
        let request = self.io_writable_buffer_len(&target)?;
        if request == 0 {
            return Ok(Value::Int(0));
        }
        if let Some(buffer) = Self::io_file_text_buffer_bytesio(&instance) {
            return self
                .builtin_bytesio_readinto(vec![Value::Instance(buffer), target], HashMap::new());
        }
        let fd = self.io_file_fd_from_instance(&instance)?;
        let payload = self.io_file_read_bytes(fd, Some(request))?;
        let copied = self.io_copy_into_writable_buffer(target, &payload)?;
        Ok(Value::Int(copied as i64))
    }

    pub(super) fn builtin_io_file_readline(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("readline() expects optional size"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "readline")?;
        if Self::iobase_is_closed(&instance) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on closed file.",
            ));
        }
        let mode = Self::io_file_mode(&instance).unwrap_or_else(|| "r".to_string());
        if !(mode.starts_with('r') || mode.contains('+')) {
            return Err(RuntimeError::new("UnsupportedOperation: not readable"));
        }
        let limit = if let Some(value) = args.pop() {
            if matches!(value, Value::None) {
                None
            } else {
                let value = value_to_int(value)?;
                if value < 0 {
                    None
                } else {
                    Some(value as usize)
                }
            }
        } else {
            None
        };
        if let Some(buffer) = Self::io_file_text_buffer_bytesio(&instance) {
            let mut read_args = vec![Value::Instance(buffer)];
            if let Some(max_len) = limit {
                read_args.push(Value::Int(max_len as i64));
            }
            let bytes_value = self.builtin_bytesio_readline(read_args, HashMap::new())?;
            let bytes = bytes_like_from_value(bytes_value)?;
            let text = String::from_utf8(bytes)
                .map_err(|_| RuntimeError::new("readline() encountered non-UTF-8 bytes"))?;
            let newline = Self::io_file_newline(&instance);
            let normalized = if newline.is_none() {
                Self::io_normalize_universal_newlines(&text)
            } else {
                text
            };
            return Ok(Value::Str(normalized));
        }
        let fd = self.io_file_fd_from_instance(&instance)?;
        let binary = Self::io_file_is_binary(&instance);
        let newline = if binary {
            None
        } else {
            Self::io_file_newline(&instance)
        };
        let file = self
            .open_files
            .get_mut(&fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        let mut out = Vec::new();
        while limit.map(|max| out.len() < max).unwrap_or(true) {
            let mut byte = [0u8; 1];
            let count = file
                .read(&mut byte)
                .map_err(|err| RuntimeError::new(format!("readline failed: {err}")))?;
            if count == 0 {
                break;
            }
            out.push(byte[0]);
            if byte[0] == b'\n' {
                if binary || newline.as_deref() != Some("\r") {
                    break;
                }
            }
            if !binary {
                match newline.as_deref() {
                    None | Some("") => {
                        if byte[0] == b'\n' {
                            break;
                        }
                        if byte[0] == b'\r' {
                            if limit.map(|max| out.len() < max).unwrap_or(true) {
                                let mut next = [0u8; 1];
                                let next_count = file.read(&mut next).map_err(|err| {
                                    RuntimeError::new(format!("readline failed: {err}"))
                                })?;
                                if next_count == 1 {
                                    if next[0] == b'\n' {
                                        out.push(next[0]);
                                    } else {
                                        file.seek(SeekFrom::Current(-1)).map_err(|err| {
                                            RuntimeError::new(format!("readline failed: {err}"))
                                        })?;
                                    }
                                }
                            }
                            break;
                        }
                    }
                    Some("\n") => {
                        if byte[0] == b'\n' {
                            break;
                        }
                    }
                    Some("\r") => {
                        if byte[0] == b'\r' {
                            break;
                        }
                    }
                    Some("\r\n") => {
                        if byte[0] == b'\r' {
                            if limit.map(|max| out.len() < max).unwrap_or(true) {
                                let mut next = [0u8; 1];
                                let next_count = file.read(&mut next).map_err(|err| {
                                    RuntimeError::new(format!("readline failed: {err}"))
                                })?;
                                if next_count == 1 {
                                    if next[0] == b'\n' {
                                        out.push(next[0]);
                                        break;
                                    }
                                    file.seek(SeekFrom::Current(-1)).map_err(|err| {
                                        RuntimeError::new(format!("readline failed: {err}"))
                                    })?;
                                }
                            }
                        }
                    }
                    Some(_) => {}
                }
            }
        }
        if binary {
            Ok(self.heap.alloc_bytes(out))
        } else {
            let text = String::from_utf8(out)
                .map_err(|_| RuntimeError::new("readline() encountered non-UTF-8 bytes"))?;
            let normalized = if newline.is_none() {
                Self::io_normalize_universal_newlines(&text)
            } else {
                text
            };
            Ok(Value::Str(normalized))
        }
    }

    pub(super) fn builtin_io_file_readlines(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("readlines() expects optional hint"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "readlines")?;
        let hint = if let Some(value) = args.pop() {
            let parsed = value_to_int(value)?;
            if parsed <= 0 {
                None
            } else {
                Some(parsed as usize)
            }
        } else {
            None
        };
        let mut lines = Vec::new();
        let mut consumed = 0usize;
        loop {
            let line = self.builtin_io_file_readline(
                vec![Value::Instance(instance.clone())],
                HashMap::new(),
            )?;
            let bytes = match &line {
                Value::Str(text) => text.as_bytes().len(),
                Value::Bytes(obj) => match &*obj.kind() {
                    Object::Bytes(values) => values.len(),
                    _ => 0,
                },
                _ => 0,
            };
            if bytes == 0 {
                break;
            }
            consumed = consumed.saturating_add(bytes);
            lines.push(line);
            if hint.map(|limit| consumed >= limit).unwrap_or(false) {
                break;
            }
        }
        Ok(self.heap.alloc_list(lines))
    }

    pub(super) fn builtin_io_file_write(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("write() expects one argument"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "write")?;
        let mode = Self::io_file_mode(&instance).unwrap_or_else(|| "r".to_string());
        if !(mode.starts_with('w')
            || mode.starts_with('a')
            || mode.starts_with('x')
            || mode.contains('+'))
        {
            return Err(RuntimeError::new("UnsupportedOperation: not writable"));
        }
        if let Some(buffer) = Self::io_file_text_buffer_bytesio(&instance) {
            let text = match args.remove(0) {
                Value::Str(text) => text,
                _ => return Err(RuntimeError::new("write() argument must be str")),
            };
            let newline = Self::io_file_newline(&instance);
            let translated = Self::io_translate_write_newlines(text.clone(), newline.as_deref());
            let payload = self.heap.alloc_bytes(translated.into_bytes());
            let _ =
                self.builtin_bytesio_write(vec![Value::Instance(buffer), payload], HashMap::new())?;
            return Ok(Value::Int(text.chars().count() as i64));
        }
        let fd = self.io_file_fd_from_instance(&instance)?;
        let payload = if Self::io_file_is_binary(&instance) {
            self.value_to_bytes_payload(args.remove(0))?
        } else {
            match args.remove(0) {
                Value::Str(text) => {
                    let newline = Self::io_file_newline(&instance);
                    let translated = Self::io_translate_write_newlines(text, newline.as_deref());
                    translated.into_bytes()
                }
                _ => return Err(RuntimeError::new("write() argument must be str")),
            }
        };
        let file = self
            .open_files
            .get_mut(&fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        file.write_all(&payload)
            .map_err(|err| RuntimeError::new(format!("write failed: {err}")))?;
        Ok(Value::Int(payload.len() as i64))
    }

    pub(super) fn builtin_io_file_writelines(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "writelines() expects one iterable argument",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "writelines")?;
        let mode = Self::io_file_mode(&instance).unwrap_or_else(|| "r".to_string());
        if !(mode.starts_with('w')
            || mode.starts_with('a')
            || mode.starts_with('x')
            || mode.contains('+'))
        {
            return Err(RuntimeError::new("UnsupportedOperation: not writable"));
        }
        let values = self.collect_iterable_values(args.remove(0))?;
        for value in values {
            let _ = self.builtin_io_file_write(
                vec![Value::Instance(instance.clone()), value],
                HashMap::new(),
            )?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_io_file_truncate(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("truncate() expects optional size"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "truncate")?;
        let mode = Self::io_file_mode(&instance).unwrap_or_else(|| "r".to_string());
        if !(mode.starts_with('w')
            || mode.starts_with('a')
            || mode.starts_with('x')
            || mode.contains('+'))
        {
            return Err(RuntimeError::new("UnsupportedOperation: not writable"));
        }
        let seekable =
            self.builtin_io_file_seekable(vec![Value::Instance(instance.clone())], HashMap::new())?;
        if !is_truthy(&seekable) {
            return Err(RuntimeError::new("OSError: not seekable"));
        }
        if let Some(buffer) = Self::io_file_text_buffer_bytesio(&instance) {
            let mut truncate_args = vec![Value::Instance(buffer)];
            if !args.is_empty() {
                truncate_args.push(args.remove(0));
            }
            return self.builtin_bytesio_truncate(truncate_args, HashMap::new());
        }
        let fd = self.io_file_fd_from_instance(&instance)?;
        let file = self
            .open_files
            .get_mut(&fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        let target_size = if args.is_empty() {
            file.stream_position()
                .map_err(|err| RuntimeError::new(format!("OSError: {err}")))? as i64
        } else {
            value_to_int(args.remove(0))?
        };
        if target_size < 0 {
            return Err(RuntimeError::new("truncate() size must be non-negative"));
        }
        file.set_len(target_size as u64)
            .map_err(|err| RuntimeError::new(format!("OSError: {err}")))?;
        Ok(Value::Int(target_size))
    }

    pub(super) fn builtin_io_file_seek(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new(
                "seek() expects offset and optional whence",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "seek")?;
        if args.is_empty() {
            return Err(RuntimeError::new("seek() missing offset argument"));
        }
        let offset = value_to_int(args.remove(0))?;
        let whence = if args.is_empty() {
            0
        } else {
            value_to_int(args.remove(0))?
        };
        let seekable =
            self.builtin_io_file_seekable(vec![Value::Instance(instance.clone())], HashMap::new())?;
        if !is_truthy(&seekable) {
            return Err(RuntimeError::new("OSError: not seekable"));
        }
        if !Self::io_file_is_binary(&instance) && whence != 0 && offset != 0 {
            return Err(RuntimeError::new(
                "UnsupportedOperation: can't do nonzero cur-relative or end-relative seeks",
            ));
        }
        if let Some(buffer) = Self::io_file_text_buffer_bytesio(&instance) {
            return self.builtin_bytesio_seek(
                vec![
                    Value::Instance(buffer),
                    Value::Int(offset),
                    Value::Int(whence),
                ],
                HashMap::new(),
            );
        }
        let fd = self.io_file_fd_from_instance(&instance)?;
        let file = self
            .open_files
            .get_mut(&fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        let position = match whence {
            0 => {
                if offset < 0 {
                    return Err(RuntimeError::new("negative seek position"));
                }
                file.seek(SeekFrom::Start(offset as u64))
            }
            1 => file.seek(SeekFrom::Current(offset)),
            2 => file.seek(SeekFrom::End(offset)),
            _ => return Err(RuntimeError::new("invalid whence")),
        }
        .map_err(|err| RuntimeError::new(format!("OSError: {err}")))?;
        Ok(Value::Int(position as i64))
    }

    pub(super) fn builtin_io_file_tell(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("tell() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "tell")?;
        let seekable =
            self.builtin_io_file_seekable(vec![Value::Instance(instance.clone())], HashMap::new())?;
        if !is_truthy(&seekable) {
            return Err(RuntimeError::new("OSError: not seekable"));
        }
        if let Some(buffer) = Self::io_file_text_buffer_bytesio(&instance) {
            return self.builtin_bytesio_tell(vec![Value::Instance(buffer)], HashMap::new());
        }
        let fd = self.io_file_fd_from_instance(&instance)?;
        let file = self
            .open_files
            .get_mut(&fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        let position = file
            .stream_position()
            .map_err(|err| RuntimeError::new(format!("OSError: {err}")))?;
        Ok(Value::Int(position as i64))
    }

    pub(super) fn builtin_io_file_close(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("close() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "close")?;
        if matches!(
            Self::instance_attr_get(&instance, "_closed"),
            Some(Value::Bool(true))
        ) {
            return Ok(Value::None);
        }
        let flush_method = self.builtin_getattr(
            vec![
                Value::Instance(instance.clone()),
                Value::Str("flush".to_string()),
            ],
            HashMap::new(),
        )?;
        let flush_error = match self.call_internal(flush_method, Vec::new(), HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => None,
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                Some(self.runtime_error_from_active_exception("close() failed"))
            }
            Err(err) => Some(err),
        };
        if let Some(buffer) = Self::io_file_text_buffer_bytesio(&instance) {
            let _ = self.builtin_bytesio_close(vec![Value::Instance(buffer)], HashMap::new())?;
            Self::instance_attr_set(&instance, "_closed", Value::Bool(true))?;
            Self::instance_attr_set(&instance, "closed", Value::Bool(true))?;
            if let Some(err) = flush_error {
                return Err(err);
            }
            return Ok(Value::None);
        }
        self.io_file_close_instance(&instance)?;
        if let Some(err) = flush_error {
            return Err(err);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_io_file_flush(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("flush() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "flush")?;
        if Self::io_file_text_buffer_bytesio(&instance).is_some() {
            return Ok(Value::None);
        }
        let mode = Self::io_file_mode(&instance).unwrap_or_else(|| "r".to_string());
        if !(mode.starts_with('w')
            || mode.starts_with('a')
            || mode.starts_with('x')
            || mode.contains('+'))
        {
            return Ok(Value::None);
        }
        let fd = self.io_file_fd_from_instance(&instance)?;
        let file = self
            .open_files
            .get_mut(&fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        file.flush()
            .map_err(|err| RuntimeError::new(format!("flush failed: {err}")))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_io_file_iter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("__iter__() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "__iter__")?;
        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_io_file_next(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("__next__() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "__next__")?;
        let line =
            self.builtin_io_file_readline(vec![Value::Instance(instance)], HashMap::new())?;
        let is_empty = match &line {
            Value::Str(text) => text.is_empty(),
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(bytes) => bytes.is_empty(),
                _ => false,
            },
            _ => false,
        };
        if is_empty {
            Err(RuntimeError::new("StopIteration"))
        } else {
            Ok(line)
        }
    }

    pub(super) fn builtin_io_file_enter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("__enter__() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "__enter__")?;
        if matches!(
            Self::instance_attr_get(&instance, "_closed"),
            Some(Value::Bool(true))
        ) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on closed file",
            ));
        }
        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_io_file_exit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 4 {
            return Err(RuntimeError::new("__exit__() expects up to 3 arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "__exit__")?;
        self.io_file_close_instance(&instance)?;
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_io_file_fileno(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("fileno() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "fileno")?;
        if Self::io_file_text_buffer_bytesio(&instance).is_some() {
            return Err(RuntimeError::new("OSError: fileno"));
        }
        let fd = self.io_file_fd_from_instance(&instance)?;
        #[cfg(unix)]
        if let Some(file) = self.open_files.get(&fd) {
            return Ok(Value::Int(file.as_raw_fd() as i64));
        }
        Ok(Value::Int(fd))
    }

    pub(super) fn builtin_io_file_detach(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("detach() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "detach")?;
        if let Some(Value::Instance(buffer)) = Self::instance_attr_get(&instance, "buffer") {
            if buffer.id() != instance.id() {
                Self::instance_attr_set(&instance, "_closed", Value::Bool(true))?;
                Self::instance_attr_set(&instance, "closed", Value::Bool(true))?;
                Self::instance_attr_set(&instance, "buffer", Value::None)?;
                Self::instance_attr_set(&instance, "raw", Value::None)?;
                return Ok(Value::Instance(buffer));
            }
        }
        Err(RuntimeError::new("detach"))
    }

    pub(super) fn builtin_io_file_readable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("readable() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "readable")?;
        let mode = Self::io_file_mode(&instance).unwrap_or_else(|| "r".to_string());
        Ok(Value::Bool(mode.starts_with('r') || mode.contains('+')))
    }

    pub(super) fn builtin_io_file_writable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("writable() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "writable")?;
        let mode = Self::io_file_mode(&instance).unwrap_or_else(|| "r".to_string());
        Ok(Value::Bool(
            mode.starts_with('w')
                || mode.starts_with('a')
                || mode.starts_with('x')
                || mode.contains('+'),
        ))
    }

    pub(super) fn builtin_io_file_seekable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("seekable() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "seekable")?;
        if let Some(Value::Instance(buffer)) = Self::instance_attr_get(&instance, "buffer") {
            if buffer.id() != instance.id() {
                let seekable = self.builtin_getattr(
                    vec![Value::Instance(buffer), Value::Str("seekable".to_string())],
                    HashMap::new(),
                )?;
                return match self.call_internal(seekable, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(value) => Ok(Value::Bool(is_truthy(&value))),
                    InternalCallOutcome::CallerExceptionHandled => {
                        self.clear_active_exception();
                        Ok(Value::Bool(false))
                    }
                };
            }
        }
        let fd = self.io_file_fd_from_instance(&instance)?;
        let file = self
            .open_files
            .get_mut(&fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        Ok(Value::Bool(file.stream_position().is_ok()))
    }

    pub(super) fn builtin_iobase_iter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("__iter__() expects no arguments"));
        }
        Ok(args.remove(0))
    }

    pub(super) fn builtin_iobase_readline(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("readline() expects at most one argument"));
        }
        let receiver_value = args.remove(0);
        let receiver = self.receiver_from_value(&receiver_value)?;
        if Self::iobase_is_closed(&receiver) {
            return Err(RuntimeError::new("I/O operation on closed file."));
        }
        let size = if args.is_empty() || matches!(args[0], Value::None) {
            -1
        } else {
            value_to_int(args.remove(0))?
        };
        let read = self.builtin_getattr(
            vec![receiver_value.clone(), Value::Str("read".to_string())],
            HashMap::new(),
        )?;
        if size == 0 {
            return Ok(
                match self.call_internal(read.clone(), vec![Value::Int(0)], HashMap::new())? {
                    InternalCallOutcome::Value(Value::Str(_)) => Value::Str(String::new()),
                    InternalCallOutcome::Value(_) => self.heap.alloc_bytes(Vec::new()),
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(self.runtime_error_from_active_exception("readline() failed"));
                    }
                },
            );
        }
        let mut remaining = size;
        let mut text_chunks = String::new();
        let mut binary_chunks: Vec<u8> = Vec::new();
        let mut text_mode: Option<bool> = None;
        while remaining < 0 || remaining > 0 {
            let request = if remaining < 0 { 1 } else { remaining.min(1) };
            let chunk = match self.call_internal(
                read.clone(),
                vec![Value::Int(request)],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception("readline() failed"));
                }
            };
            match chunk {
                Value::None => break,
                Value::Str(text) => {
                    if text_mode == Some(false) {
                        return Err(RuntimeError::new(
                            "readline() mixed text and binary read results",
                        ));
                    }
                    text_mode = Some(true);
                    if text.is_empty() {
                        break;
                    }
                    let consumed = text.chars().count() as i64;
                    text_chunks.push_str(&text);
                    if text.ends_with('\n') {
                        break;
                    }
                    if remaining > 0 {
                        remaining = remaining.saturating_sub(consumed);
                    }
                }
                other => {
                    if text_mode == Some(true) {
                        return Err(RuntimeError::new(
                            "readline() mixed text and binary read results",
                        ));
                    }
                    text_mode = Some(false);
                    let chunk_bytes = bytes_like_from_value(other)
                        .map_err(|_| RuntimeError::new("read() should return bytes-like object"))?;
                    if chunk_bytes.is_empty() {
                        break;
                    }
                    let consumed = chunk_bytes.len() as i64;
                    binary_chunks.extend(chunk_bytes.iter());
                    if chunk_bytes.last() == Some(&b'\n') {
                        break;
                    }
                    if remaining > 0 {
                        remaining = remaining.saturating_sub(consumed);
                    }
                }
            }
        }
        if text_mode == Some(true) {
            Ok(Value::Str(text_chunks))
        } else {
            Ok(self.heap.alloc_bytes(binary_chunks))
        }
    }

    pub(super) fn builtin_iobase_readlines(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "readlines() expects at most one argument",
            ));
        }
        let receiver = args.remove(0);
        let hint = if args.is_empty() || matches!(args[0], Value::None) {
            0
        } else {
            value_to_int(args.remove(0))?
        };
        if hint <= 0 {
            let values = self.collect_iterable_values(receiver)?;
            return Ok(self.heap.alloc_list(values));
        }
        let iterator = self.to_iterator_value(receiver)?;
        let mut lines = Vec::new();
        let mut total_size = 0i64;
        loop {
            let line = match self.next_from_iterator_value(&iterator)? {
                GeneratorResumeOutcome::Yield(value) => value,
                GeneratorResumeOutcome::Complete(_) => break,
                GeneratorResumeOutcome::PropagatedException => {
                    return Err(self.iteration_error_from_state("readlines() iteration failed")?);
                }
            };
            let line_len =
                match self.call_builtin(BuiltinFunction::Len, vec![line.clone()], HashMap::new()) {
                    Ok(value) => value_to_int(value)?,
                    Err(_) => return Err(RuntimeError::new("TypeError: object has no len()")),
                };
            lines.push(line);
            total_size = total_size.saturating_add(line_len);
            if total_size >= hint {
                break;
            }
        }
        Ok(self.heap.alloc_list(lines))
    }

    pub(super) fn builtin_iobase_writelines(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("writelines() expects one argument"));
        }
        let receiver_value = args.remove(0);
        let receiver = self.receiver_from_value(&receiver_value)?;
        if Self::iobase_is_closed(&receiver) {
            return Err(RuntimeError::new("I/O operation on closed file."));
        }
        let lines = self.collect_iterable_values(args.remove(0))?;
        let write = self.builtin_getattr(
            vec![receiver_value.clone(), Value::Str("write".to_string())],
            HashMap::new(),
        )?;
        for line in lines {
            match self.call_internal(write.clone(), vec![line], HashMap::new())? {
                InternalCallOutcome::Value(_) => {}
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception("writelines() failed"));
                }
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_iobase_enter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("__enter__() expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args[0])?;
        if Self::iobase_is_closed(&receiver) {
            return Err(RuntimeError::new("I/O operation on closed file."));
        }
        Ok(args.remove(0))
    }

    pub(super) fn builtin_iobase_exit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() {
            return Err(RuntimeError::new("__exit__() expects 3 arguments"));
        }
        let receiver = args.remove(0);
        let close = self.builtin_getattr(
            vec![receiver, Value::Str("close".to_string())],
            HashMap::new(),
        )?;
        match self.call_internal(close, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(_) => Ok(Value::None),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("__exit__() failed"))
            }
        }
    }

    fn iobase_is_closed(instance: &ObjRef) -> bool {
        matches!(
            Self::instance_attr_get(instance, "closed"),
            Some(Value::Bool(true))
        ) || matches!(
            Self::instance_attr_get(instance, "__IOBase_closed"),
            Some(Value::Bool(true))
        ) || matches!(
            Self::instance_attr_get(instance, "_closed"),
            Some(Value::Bool(true))
        )
    }

    fn iobase_mark_closed(instance: &ObjRef) -> Result<(), RuntimeError> {
        Self::instance_attr_set(instance, "__IOBase_closed", Value::Bool(true))?;
        Self::instance_attr_set(instance, "_closed", Value::Bool(true))?;
        Self::instance_attr_set(instance, "closed", Value::Bool(true))?;
        Ok(())
    }

    fn iobase_emit_unclosed_resource_warning(&mut self, receiver: &ObjRef) {
        let has_direct_fd = Self::instance_attr_get(receiver, "_fd").is_some();
        let has_raw_fd = match Self::instance_attr_get(receiver, "raw") {
            Some(Value::Instance(raw)) => Self::instance_attr_get(&raw, "_fd").is_some(),
            _ => false,
        };
        if !has_direct_fd && !has_raw_fd {
            return;
        }
        let Some(warnings_module) = self.modules.get("warnings").cloned() else {
            return;
        };
        let warn = match self.builtin_getattr(
            vec![
                Value::Module(warnings_module),
                Value::Str("warn".to_string()),
            ],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(_) => return,
        };
        let category = self
            .builtins
            .get("ResourceWarning")
            .cloned()
            .unwrap_or_else(|| Value::ExceptionType("ResourceWarning".to_string()));
        let _ = self.call_internal(
            warn,
            vec![Value::Str("unclosed file".to_string()), category],
            HashMap::new(),
        );
    }

    pub(super) fn builtin_iobase_flush(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("flush() expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        if Self::iobase_is_closed(&receiver) {
            return Err(RuntimeError::new("I/O operation on closed file."));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_iobase_close(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("close() expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        if Self::iobase_is_closed(&receiver) {
            return Ok(Value::None);
        }
        let flush = self.builtin_getattr(
            vec![
                Value::Instance(receiver.clone()),
                Value::Str("flush".to_string()),
            ],
            HashMap::new(),
        )?;
        let flush_error =
            match self.call_internal_preserving_caller(flush, Vec::new(), HashMap::new()) {
                Ok(InternalCallOutcome::Value(_)) => None,
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    Some(self.runtime_error_from_active_exception("close() failed"))
                }
                Err(err) => Some(err),
            };
        Self::iobase_mark_closed(&receiver)?;
        if let Some(err) = flush_error {
            return Err(err);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_iobase_del(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("__del__() expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        if Self::iobase_is_closed(&receiver) {
            return Ok(Value::None);
        }
        self.iobase_emit_unclosed_resource_warning(&receiver);
        let close = match self.builtin_getattr(
            vec![
                Value::Instance(receiver.clone()),
                Value::Str("close".to_string()),
            ],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(_) => return Ok(Value::None),
        };
        match self.call_internal_preserving_caller(close, Vec::new(), HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => Ok(Value::None),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                Err(self.runtime_error_from_active_exception("__del__() close failed"))
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn builtin_iobase_next(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("__next__() expects no arguments"));
        }
        let receiver = args.remove(0);
        let readline = self.builtin_getattr(
            vec![receiver.clone(), Value::Str("readline".to_string())],
            HashMap::new(),
        )?;
        let line = match self.call_internal(readline, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(self.runtime_error_from_active_exception("__next__() iteration failed"));
            }
        };
        let is_empty = match &line {
            Value::Str(text) => text.is_empty(),
            Value::Bytes(obj) => matches!(&*obj.kind(), Object::Bytes(values) if values.is_empty()),
            _ => {
                return Err(RuntimeError::new(
                    "TypeError: readline() should return bytes-like object or str",
                ));
            }
        };
        if is_empty {
            Err(RuntimeError::new("StopIteration"))
        } else {
            Ok(line)
        }
    }

    pub(super) fn stringio_buffer_from_instance(
        &self,
        instance: &ObjRef,
    ) -> Result<(Vec<char>, usize), RuntimeError> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new("StringIO receiver must be instance"));
        };
        let text = match instance_data.attrs.get("_value") {
            Some(Value::Str(value)) => value.chars().collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let pos = match instance_data.attrs.get("_pos") {
            Some(Value::Int(value)) if *value >= 0 => *value as usize,
            _ => 0,
        };
        Ok((text, pos))
    }

    fn stringio_is_closed(instance: &ObjRef) -> bool {
        matches!(
            Self::instance_attr_get(instance, "_closed"),
            Some(Value::Bool(true))
        )
    }

    fn stringio_newline(instance: &ObjRef) -> Option<String> {
        match Self::instance_attr_get(instance, "_newline") {
            Some(Value::Str(newline)) => Some(newline),
            _ => None,
        }
    }

    fn stringio_detect_newline_markers(text: &str) -> (bool, bool, bool) {
        let chars: Vec<char> = text.chars().collect();
        let mut saw_cr = false;
        let mut saw_lf = false;
        let mut saw_crlf = false;
        let mut idx = 0usize;
        while idx < chars.len() {
            match chars[idx] {
                '\r' => {
                    if idx + 1 < chars.len() && chars[idx + 1] == '\n' {
                        saw_crlf = true;
                        idx += 2;
                    } else {
                        saw_cr = true;
                        idx += 1;
                    }
                }
                '\n' => {
                    saw_lf = true;
                    idx += 1;
                }
                _ => idx += 1,
            }
        }
        (saw_cr, saw_lf, saw_crlf)
    }

    fn stringio_newlines_markers(instance: &ObjRef) -> (bool, bool, bool) {
        let Some(value) = Self::instance_attr_get(instance, "newlines") else {
            return (false, false, false);
        };
        match value {
            Value::None => (false, false, false),
            Value::Str(kind) => match kind.as_str() {
                "\r" => (true, false, false),
                "\n" => (false, true, false),
                "\r\n" => (false, false, true),
                _ => (false, false, false),
            },
            Value::Tuple(obj) => {
                let Object::Tuple(items) = &*obj.kind() else {
                    return (false, false, false);
                };
                let mut saw_cr = false;
                let mut saw_lf = false;
                let mut saw_crlf = false;
                for item in items {
                    if let Value::Str(kind) = item {
                        match kind.as_str() {
                            "\r" => saw_cr = true,
                            "\n" => saw_lf = true,
                            "\r\n" => saw_crlf = true,
                            _ => {}
                        }
                    }
                }
                (saw_cr, saw_lf, saw_crlf)
            }
            _ => (false, false, false),
        }
    }

    fn stringio_set_newlines_markers(
        &mut self,
        instance: &ObjRef,
        saw_cr: bool,
        saw_lf: bool,
        saw_crlf: bool,
    ) -> Result<(), RuntimeError> {
        let value = match (saw_cr, saw_lf, saw_crlf) {
            (false, false, false) => Value::None,
            (true, false, false) => Value::Str("\r".to_string()),
            (false, true, false) => Value::Str("\n".to_string()),
            (false, false, true) => Value::Str("\r\n".to_string()),
            _ => {
                let mut items = Vec::new();
                if saw_cr {
                    items.push(Value::Str("\r".to_string()));
                }
                if saw_lf {
                    items.push(Value::Str("\n".to_string()));
                }
                if saw_crlf {
                    items.push(Value::Str("\r\n".to_string()));
                }
                self.heap.alloc_tuple(items)
            }
        };
        Self::instance_attr_set(instance, "newlines", value)
    }

    fn stringio_update_newlines_from_text(
        &mut self,
        instance: &ObjRef,
        text: &str,
    ) -> Result<(), RuntimeError> {
        let (current_cr, current_lf, current_crlf) = Self::stringio_newlines_markers(instance);
        let (new_cr, new_lf, new_crlf) = Self::stringio_detect_newline_markers(text);
        self.stringio_set_newlines_markers(
            instance,
            current_cr || new_cr,
            current_lf || new_lf,
            current_crlf || new_crlf,
        )
    }

    fn stringio_ensure_open(instance: &ObjRef) -> Result<(), RuntimeError> {
        if Self::stringio_is_closed(instance) {
            Err(RuntimeError::new("I/O operation on closed file"))
        } else {
            Ok(())
        }
    }

    fn stringio_mark_closed(&mut self, instance: &ObjRef) -> Result<(), RuntimeError> {
        self.stringio_store_buffer(instance, Vec::new(), 0)?;
        Self::instance_attr_set(instance, "_closed", Value::Bool(true))?;
        Self::instance_attr_set(instance, "closed", Value::Bool(true))?;
        Ok(())
    }

    fn stringio_translate_text(text: String, newline: Option<&str>) -> String {
        match newline {
            None => Self::io_normalize_universal_newlines(&text),
            Some("") | Some("\n") => text,
            Some("\r") => text.replace('\n', "\r"),
            Some("\r\n") => text.replace('\n', "\r\n"),
            Some(_) => text,
        }
    }

    pub(super) fn io_index_arg_to_int(&mut self, value: Value) -> Result<i64, RuntimeError> {
        match value {
            Value::Int(_) | Value::Bool(_) | Value::BigInt(_) => value_to_int(value),
            other => {
                let Some(index_method) = self.lookup_bound_special_method(&other, "__index__")?
                else {
                    return Err(RuntimeError::new("unsupported operand type"));
                };
                let indexed = match self.call_internal(index_method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(
                            self.runtime_error_from_active_exception("__index__() call failed")
                        );
                    }
                };
                match indexed {
                    Value::Int(_) | Value::Bool(_) | Value::BigInt(_) => value_to_int(indexed),
                    _ => Err(RuntimeError::new("TypeError: __index__ returned non-int")),
                }
            }
        }
    }

    pub(super) fn stringio_store_buffer(
        &mut self,
        instance: &ObjRef,
        text: Vec<char>,
        pos: usize,
    ) -> Result<(), RuntimeError> {
        let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
            return Err(RuntimeError::new("StringIO receiver must be instance"));
        };
        instance_data
            .attrs
            .insert("_value".to_string(), Value::Str(text.iter().collect()));
        instance_data
            .attrs
            .insert("_pos".to_string(), Value::Int(pos as i64));
        Ok(())
    }

    pub(super) fn stringio_next_line_end(
        buffer: &[char],
        pos: usize,
        limit: Option<usize>,
        newline: Option<&str>,
    ) -> usize {
        let Some(max_len) = limit else {
            return Self::stringio_next_line_end(buffer, pos, Some(usize::MAX), newline);
        };
        if max_len == 0 {
            return pos;
        }
        let is_universal = newline.is_none() || newline == Some("");
        if is_universal {
            let mut end = pos;
            while end < buffer.len() {
                if end - pos >= max_len {
                    break;
                }
                let ch = buffer[end];
                if ch == '\n' {
                    end += 1;
                    break;
                }
                if ch == '\r' {
                    end += 1;
                    if end < buffer.len() && buffer[end] == '\n' {
                        if end - pos >= max_len {
                            break;
                        }
                        end += 1;
                    }
                    break;
                }
                end += 1;
            }
            return end;
        }

        let mut end = pos;
        while end < buffer.len() && end - pos < max_len {
            match newline {
                Some("\n") | None => {
                    if buffer[end] == '\n' {
                        end += 1;
                        break;
                    }
                    end += 1;
                }
                Some("\r") => {
                    if buffer[end] == '\r' {
                        end += 1;
                        break;
                    }
                    end += 1;
                }
                Some("\r\n") => {
                    if end + 1 < buffer.len() && buffer[end] == '\r' && buffer[end + 1] == '\n' {
                        if end - pos + 2 > max_len {
                            end = pos + max_len;
                        } else {
                            end += 2;
                        }
                        break;
                    }
                    end += 1;
                }
                Some(_) => {
                    if buffer[end] == '\n' {
                        end += 1;
                        break;
                    }
                    end += 1;
                }
            }
        }
        end
    }

    pub(super) fn builtin_stringio_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("StringIO.__init__ expects instance"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let initial = args.pop().or_else(|| kwargs.remove("initial_value"));
        let newline_arg = kwargs.remove("newline");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("StringIO.__init__ unexpected keyword"));
        }

        let newline = match newline_arg.unwrap_or(Value::Str("\n".to_string())) {
            Value::None => None,
            Value::Str(value) if matches!(value.as_str(), "" | "\n" | "\r" | "\r\n") => Some(value),
            Value::Str(value) => {
                return Err(RuntimeError::new(format!(
                    "ValueError: illegal newline value: {value:?}"
                )));
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "TypeError: newline must be str or None, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };

        let initial_raw_text = match &initial {
            Some(Value::Str(value)) => Some(value.clone()),
            _ => None,
        };
        let initial_text = match initial {
            None => Vec::new(),
            Some(Value::None) => Vec::new(),
            Some(Value::Str(value)) => Self::stringio_translate_text(value, newline.as_deref())
                .chars()
                .collect(),
            Some(other) => {
                return Err(RuntimeError::new(format!(
                    "TypeError: initial_value must be str or None, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        self.stringio_store_buffer(&receiver, initial_text, 0)?;
        Self::instance_attr_set(&receiver, "_closed", Value::Bool(false))?;
        Self::instance_attr_set(&receiver, "closed", Value::Bool(false))?;
        let newline_value = newline.clone().map(Value::Str).unwrap_or(Value::None);
        Self::instance_attr_set(&receiver, "_newline", newline_value)?;
        Self::instance_attr_set(&receiver, "newlines", Value::None)?;
        Self::instance_attr_set(&receiver, "encoding", Value::None)?;
        Self::instance_attr_set(&receiver, "errors", Value::None)?;
        Self::instance_attr_set(&receiver, "line_buffering", Value::Bool(false))?;
        if let (None, Some(initial_value)) = (newline, initial_raw_text) {
            self.stringio_update_newlines_from_text(&receiver, &initial_value)?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_stringio_write(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("StringIO.write expects 1 argument"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let input = match args.remove(0) {
            Value::Str(value) => value,
            _ => {
                return Err(RuntimeError::new(
                    "TypeError: string argument expected, got non-str",
                ));
            }
        };
        let newline = Self::stringio_newline(&receiver);
        if newline.is_none() {
            self.stringio_update_newlines_from_text(&receiver, &input)?;
        }
        let translated = Self::stringio_translate_text(input.clone(), newline.as_deref());
        let mut insert = input.chars().collect::<Vec<_>>();
        if translated != input {
            insert = translated.chars().collect::<Vec<_>>();
        }
        let (buffer, pos) = self.stringio_buffer_from_instance(&receiver)?;
        let insert_len = insert.len();
        if insert_len == 0 {
            return Ok(Value::Int(0));
        }
        let mut new_buf = Vec::new();
        if pos > buffer.len() {
            new_buf.extend_from_slice(&buffer);
            new_buf.resize(pos, '\0');
            new_buf.append(&mut insert);
        } else {
            let tail_start = pos.saturating_add(insert_len);
            new_buf.extend_from_slice(&buffer[..pos]);
            new_buf.append(&mut insert);
            if tail_start < buffer.len() {
                new_buf.extend_from_slice(&buffer[tail_start..]);
            }
        }
        let new_pos = pos + insert_len;
        self.stringio_store_buffer(&receiver, new_buf, new_pos)?;
        Ok(Value::Int(input.chars().count() as i64))
    }

    pub(super) fn builtin_stringio_read(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("StringIO.read expects 0-1 arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let size = match args.pop() {
            None | Some(Value::None) => -1,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        let (buffer, pos) = self.stringio_buffer_from_instance(&receiver)?;
        if pos >= buffer.len() {
            return Ok(Value::Str(String::new()));
        }
        let end = if size < 0 {
            buffer.len()
        } else {
            (pos + size as usize).min(buffer.len())
        };
        let out: String = buffer[pos..end].iter().collect();
        self.stringio_store_buffer(&receiver, buffer, end)?;
        Ok(Value::Str(out))
    }

    pub(super) fn builtin_stringio_readline(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("StringIO.readline expects 0-1 arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let limit = match args.pop() {
            None | Some(Value::None) => -1,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        let (buffer, pos) = self.stringio_buffer_from_instance(&receiver)?;
        if pos >= buffer.len() {
            return Ok(Value::Str(String::new()));
        }
        let max = if limit < 0 {
            None
        } else {
            Some(limit as usize)
        };
        let newline = Self::stringio_newline(&receiver);
        let end = Self::stringio_next_line_end(&buffer, pos, max, newline.as_deref());
        let out: String = buffer[pos..end].iter().collect();
        self.stringio_store_buffer(&receiver, buffer, end)?;
        Ok(Value::Str(out))
    }

    pub(super) fn builtin_stringio_readlines(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "StringIO.readlines expects 0-1 arguments",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let hint = match args.pop() {
            None | Some(Value::None) => -1,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        let mut lines = Vec::new();
        let mut consumed = 0i64;
        loop {
            let line = self.builtin_stringio_readline(
                vec![Value::Instance(receiver.clone())],
                HashMap::new(),
            )?;
            let Value::Str(text) = line else {
                break;
            };
            if text.is_empty() {
                break;
            }
            consumed += text.chars().count() as i64;
            lines.push(Value::Str(text));
            if hint > 0 && consumed >= hint {
                break;
            }
        }
        Ok(self.heap.alloc_list(lines))
    }

    pub(super) fn builtin_stringio_getvalue(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.getvalue expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let (buffer, _pos) = self.stringio_buffer_from_instance(&receiver)?;
        Ok(Value::Str(buffer.iter().collect()))
    }

    pub(super) fn builtin_stringio_getstate(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "StringIO.__getstate__ expects no arguments",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let (buffer, pos) = self.stringio_buffer_from_instance(&receiver)?;
        let text: String = buffer.iter().collect();
        let newline = Self::instance_attr_get(&receiver, "_newline").unwrap_or(Value::None);
        let instance_dict = match &*receiver.kind() {
            Object::Instance(instance_data) => {
                let mut entries = Vec::new();
                for (name, value) in &instance_data.attrs {
                    if matches!(
                        name.as_str(),
                        "_value"
                            | "_pos"
                            | "_closed"
                            | "closed"
                            | "_newline"
                            | "newlines"
                            | "encoding"
                            | "errors"
                            | "line_buffering"
                    ) {
                        continue;
                    }
                    entries.push((Value::Str(name.clone()), value.clone()));
                }
                if entries.is_empty() {
                    Value::None
                } else {
                    self.heap.alloc_dict(entries)
                }
            }
            _ => return Err(RuntimeError::new("StringIO receiver must be instance")),
        };
        Ok(self.heap.alloc_tuple(vec![
            Value::Str(text),
            newline,
            Value::Int(pos as i64),
            instance_dict,
        ]))
    }

    pub(super) fn builtin_stringio_setstate(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "StringIO.__setstate__ expects one argument",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let state = args.remove(0);
        let items = match state {
            Value::Tuple(tuple_obj) => {
                let Object::Tuple(items) = &*tuple_obj.kind() else {
                    return Err(RuntimeError::new("StringIO.__setstate__ state is invalid"));
                };
                items.clone()
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "TypeError: StringIO.__setstate__ argument should be 4-tuple, got {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        if items.len() < 4 {
            return Err(RuntimeError::new(
                "TypeError: StringIO.__setstate__ argument should be 4-tuple, got tuple",
            ));
        }

        let initial_text = match &items[0] {
            Value::None => String::new(),
            Value::Str(text) => text.clone(),
            other => {
                return Err(RuntimeError::new(format!(
                    "TypeError: first item of state should be a str, got {}",
                    self.value_type_name_for_error(other)
                )));
            }
        };

        let mut init_kwargs = HashMap::new();
        init_kwargs.insert("newline".to_string(), items[1].clone());
        let _ = self.builtin_stringio_init(
            vec![Value::Instance(receiver.clone()), Value::None],
            init_kwargs,
        )?;
        self.stringio_store_buffer(&receiver, initial_text.chars().collect(), 0)?;

        let position = match &items[2] {
            Value::Int(value) => *value,
            Value::Bool(value) => i64::from(*value),
            Value::BigInt(value) => value.to_i64().ok_or_else(|| {
                RuntimeError::new("OverflowError: position out of range for this platform")
            })?,
            other => {
                return Err(RuntimeError::new(format!(
                    "TypeError: third item of state must be an integer, got {}",
                    self.value_type_name_for_error(other)
                )));
            }
        };
        if position < 0 {
            return Err(RuntimeError::new(
                "ValueError: position value cannot be negative",
            ));
        }
        let (buffer, _old_pos) = self.stringio_buffer_from_instance(&receiver)?;
        self.stringio_store_buffer(&receiver, buffer, position as usize)?;

        match &items[3] {
            Value::None => {}
            Value::Dict(dict_obj) => {
                let updates = match &*dict_obj.kind() {
                    Object::Dict(dict) => dict.to_vec(),
                    _ => return Err(RuntimeError::new("fourth item of state should be a dict")),
                };
                let Object::Instance(instance_data) = &mut *receiver.kind_mut() else {
                    return Err(RuntimeError::new("StringIO receiver must be instance"));
                };
                for (key, value) in updates {
                    let Value::Str(name) = key else {
                        return Err(RuntimeError::new(
                            "TypeError: fourth item of state should be a dict",
                        ));
                    };
                    if matches!(
                        name.as_str(),
                        "_value"
                            | "_pos"
                            | "_closed"
                            | "closed"
                            | "_newline"
                            | "newlines"
                            | "encoding"
                            | "errors"
                            | "line_buffering"
                    ) {
                        continue;
                    }
                    instance_data.attrs.insert(name, value);
                }
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "TypeError: fourth item of state should be a dict, got a {}",
                    self.value_type_name_for_error(other)
                )));
            }
        }

        Ok(Value::None)
    }

    pub(super) fn builtin_stringio_seek(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new("StringIO.seek expects 1-2 arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let offset = self.io_index_arg_to_int(args.remove(0))?;
        let whence = if args.is_empty() {
            0
        } else {
            self.io_index_arg_to_int(args.remove(0))?
        };
        let (buffer, pos) = self.stringio_buffer_from_instance(&receiver)?;
        if !matches!(whence, 0 | 1 | 2) {
            return Err(RuntimeError::new(format!(
                "ValueError: Invalid whence ({whence}, should be 0, 1 or 2)"
            )));
        }
        if whence == 0 && offset < 0 {
            return Err(RuntimeError::new(format!(
                "ValueError: Negative seek position {offset}"
            )));
        }
        if whence != 0 && offset != 0 {
            return Err(RuntimeError::new(
                "OSError: Can't do nonzero cur-relative seeks",
            ));
        }
        let new_pos = match whence {
            0 => offset as usize,
            1 => pos,
            2 => buffer.len(),
            _ => unreachable!(),
        };
        self.stringio_store_buffer(&receiver, buffer, new_pos)?;
        Ok(Value::Int(new_pos as i64))
    }

    pub(super) fn builtin_stringio_tell(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.tell expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let (_buffer, pos) = self.stringio_buffer_from_instance(&receiver)?;
        Ok(Value::Int(pos as i64))
    }

    pub(super) fn builtin_stringio_writelines(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("StringIO.writelines expects 1 argument"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let values = self.collect_iterable_values(args.remove(0))?;
        for item in values {
            let _ = self.builtin_stringio_write(
                vec![Value::Instance(receiver.clone()), item],
                HashMap::new(),
            )?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_stringio_truncate(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("StringIO.truncate expects 0-1 arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let (mut buffer, pos) = self.stringio_buffer_from_instance(&receiver)?;
        let size = match args.pop() {
            None | Some(Value::None) => pos as i64,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        if size < 0 {
            return Err(RuntimeError::new("ValueError: negative size value"));
        }
        let size = size as usize;
        if buffer.len() > size {
            buffer.truncate(size);
        }
        self.stringio_store_buffer(&receiver, buffer, pos)?;
        Ok(Value::Int(size as i64))
    }

    pub(super) fn builtin_stringio_detach(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.detach expects no arguments"));
        }
        Err(RuntimeError::new("UnsupportedOperation: detach"))
    }

    pub(super) fn builtin_stringio_flush(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.flush expects no arguments"));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_stringio_isatty(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.isatty expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_stringio_fileno(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.fileno expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        Err(RuntimeError::new("OSError: fileno"))
    }

    pub(super) fn builtin_stringio_iter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.__iter__ expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        if Self::stringio_is_closed(&receiver) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on closed file.",
            ));
        }
        Ok(Value::Instance(receiver))
    }

    pub(super) fn builtin_stringio_next(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.__next__ expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        let (buffer, pos) = self.stringio_buffer_from_instance(&receiver)?;
        if pos >= buffer.len() {
            return Err(RuntimeError::new("StopIteration"));
        }
        let newline = Self::stringio_newline(&receiver);
        let end = Self::stringio_next_line_end(&buffer, pos, None, newline.as_deref());
        let out: String = buffer[pos..end].iter().collect();
        self.stringio_store_buffer(&receiver, buffer, end)?;
        Ok(Value::Str(out))
    }

    pub(super) fn builtin_stringio_enter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.__enter__ expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        if Self::stringio_is_closed(&receiver) {
            return Err(RuntimeError::new(
                "ValueError: I/O operation on closed file.",
            ));
        }
        Ok(Value::Instance(receiver))
    }

    pub(super) fn builtin_stringio_exit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 4 {
            return Err(RuntimeError::new(
                "StringIO.__exit__ expects up to 3 arguments",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.stringio_mark_closed(&receiver)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_stringio_close(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.close expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.stringio_mark_closed(&receiver)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_stringio_readable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.readable expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        Ok(Value::Bool(true))
    }

    pub(super) fn builtin_stringio_writable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.writable expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        Ok(Value::Bool(true))
    }

    pub(super) fn builtin_stringio_seekable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("StringIO.seekable expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        Self::stringio_ensure_open(&receiver)?;
        Ok(Value::Bool(true))
    }

    pub(super) fn bytesio_state_from_instance(
        &mut self,
        instance: &ObjRef,
    ) -> Result<(ObjRef, usize, bool), RuntimeError> {
        let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
            return Err(RuntimeError::new("BytesIO receiver must be instance"));
        };
        let pos = match instance_data.attrs.get("_pos") {
            Some(Value::Int(value)) if *value >= 0 => *value as usize,
            _ => 0,
        };
        let closed = matches!(instance_data.attrs.get("_closed"), Some(Value::Bool(true)));
        let current_value = instance_data.attrs.get("_value").cloned();
        let (value_obj, needs_store_value) = match current_value {
            Some(Value::ByteArray(obj)) => (obj, false),
            Some(Value::Bytes(obj)) => {
                let bytes = match &*obj.kind() {
                    Object::Bytes(values) => values.clone(),
                    _ => Vec::new(),
                };
                match self.heap.alloc_bytearray(bytes) {
                    Value::ByteArray(obj) => (obj, true),
                    _ => unreachable!(),
                }
            }
            Some(other) => {
                let bytes = self
                    .value_to_bytes_payload(other)
                    .map_err(|_| RuntimeError::new("BytesIO internal buffer is invalid"))?;
                match self.heap.alloc_bytearray(bytes) {
                    Value::ByteArray(obj) => (obj, true),
                    _ => unreachable!(),
                }
            }
            None => match self.heap.alloc_bytearray(Vec::new()) {
                Value::ByteArray(obj) => (obj, true),
                _ => unreachable!(),
            },
        };
        if needs_store_value {
            if let Some(slot) = instance_data.attrs.get_mut("_value") {
                *slot = Value::ByteArray(value_obj.clone());
            } else {
                instance_data
                    .attrs
                    .insert("_value".to_string(), Value::ByteArray(value_obj.clone()));
            }
        }
        Ok((value_obj, pos, closed))
    }

    pub(super) fn bytesio_store_state(
        &mut self,
        instance: &ObjRef,
        value_obj: ObjRef,
        pos: usize,
        closed: bool,
    ) -> Result<(), RuntimeError> {
        let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
            return Err(RuntimeError::new("BytesIO receiver must be instance"));
        };
        if let Some(slot) = instance_data.attrs.get_mut("_value") {
            *slot = Value::ByteArray(value_obj);
        } else {
            instance_data
                .attrs
                .insert("_value".to_string(), Value::ByteArray(value_obj));
        }
        if let Some(slot) = instance_data.attrs.get_mut("_pos") {
            *slot = Value::Int(pos as i64);
        } else {
            instance_data
                .attrs
                .insert("_pos".to_string(), Value::Int(pos as i64));
        }
        if let Some(slot) = instance_data.attrs.get_mut("_closed") {
            *slot = Value::Bool(closed);
        } else {
            instance_data
                .attrs
                .insert("_closed".to_string(), Value::Bool(closed));
        }
        if let Some(slot) = instance_data.attrs.get_mut("closed") {
            *slot = Value::Bool(closed);
        } else {
            instance_data
                .attrs
                .insert("closed".to_string(), Value::Bool(closed));
        }
        Ok(())
    }

    pub(super) fn bytesio_ensure_open(&self, instance: &ObjRef) -> Result<(), RuntimeError> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new("BytesIO receiver must be instance"));
        };
        if matches!(instance_data.attrs.get("_closed"), Some(Value::Bool(true))) {
            Err(RuntimeError::new("I/O operation on closed file."))
        } else {
            Ok(())
        }
    }

    fn bytesio_export_count(&self, value_obj: &ObjRef) -> usize {
        self.heap
            .count_live_memoryview_exports_for_source(value_obj)
    }

    fn bytesio_ensure_resizable(&self, value_obj: &ObjRef) -> Result<(), RuntimeError> {
        if self.bytesio_export_count(value_obj) > 0 {
            return Err(RuntimeError::new(
                "BufferError: Existing exports of data: object cannot be re-sized",
            ));
        }
        Ok(())
    }

    fn bytesio_payload_from_value(&mut self, value: Value) -> Result<Vec<u8>, RuntimeError> {
        match value {
            Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                bytes_like_from_value(value)
            }
            Value::Module(obj) => {
                let is_array =
                    matches!(&*obj.kind(), Object::Module(module) if module.name == "__array__");
                if is_array {
                    bytes_like_from_value(Value::Module(obj))
                } else {
                    Err(RuntimeError::new(format!(
                        "TypeError: a bytes-like object is required, not '{}'",
                        self.value_type_name_for_error(&Value::Module(obj))
                    )))
                }
            }
            Value::Instance(obj) => {
                let has_storage = matches!(
                    &*obj.kind(),
                    Object::Instance(instance_data)
                        if instance_data.attrs.contains_key("__pyrs_bytes_storage__")
                );
                if has_storage {
                    bytes_like_from_value(Value::Instance(obj))
                } else {
                    let receiver = Value::Instance(obj.clone());
                    if let Some(buffer_method) =
                        self.lookup_bound_special_method(&receiver, "__buffer__")?
                    {
                        let buffer_value = match self.call_internal(
                            buffer_method,
                            vec![Value::Int(0)],
                            HashMap::new(),
                        )? {
                            InternalCallOutcome::Value(value) => value,
                            InternalCallOutcome::CallerExceptionHandled => {
                                return Err(RuntimeError::new("__buffer__() raised an exception"));
                            }
                        };
                        bytes_like_from_value(buffer_value)
                    } else {
                        Err(RuntimeError::new(format!(
                            "TypeError: a bytes-like object is required, not '{}'",
                            self.value_type_name_for_error(&Value::Instance(obj))
                        )))
                    }
                }
            }
            other => Err(RuntimeError::new(format!(
                "TypeError: a bytes-like object is required, not '{}'",
                self.value_type_name_for_error(&other)
            ))),
        }
    }

    pub(super) fn builtin_bytesio_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("BytesIO.__init__ expects instance"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let initial = args.pop().or_else(|| kwargs.remove("initial_bytes"));
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: BytesIO.__init__() got an unexpected keyword argument",
            ));
        }
        let bytes = match initial {
            None | Some(Value::None) => Vec::new(),
            Some(value) => self.bytesio_payload_from_value(value)?,
        };
        let value_obj = match self.heap.alloc_bytearray(bytes) {
            Value::ByteArray(obj) => obj,
            _ => unreachable!(),
        };
        self.bytesio_store_state(&receiver, value_obj, 0, false)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_bytesio_write(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("BytesIO.write expects 1 argument"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let payload = self.bytesio_payload_from_value(args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let written = payload.len();
        let (value_obj, mut pos, closed) = self.bytesio_state_from_instance(&receiver)?;
        if written == 0 {
            return Ok(Value::Int(0));
        }
        let needs_resize = {
            let Object::ByteArray(buffer) = &*value_obj.kind() else {
                return Err(RuntimeError::new("BytesIO internal buffer is invalid"));
            };
            let start = pos.max(buffer.len());
            start.saturating_add(written) > buffer.len()
        };
        if needs_resize {
            self.bytesio_ensure_resizable(&value_obj)?;
        }
        pos = {
            let Object::ByteArray(buffer) = &mut *value_obj.kind_mut() else {
                return Err(RuntimeError::new("BytesIO internal buffer is invalid"));
            };
            if pos > buffer.len() {
                buffer.resize(pos, 0);
            }
            let end = pos.saturating_add(written);
            if end > buffer.len() {
                buffer.resize(end, 0);
            }
            buffer[pos..end].copy_from_slice(&payload);
            end
        };
        self.bytesio_store_state(&receiver, value_obj, pos, closed)?;
        Ok(Value::Int(written as i64))
    }

    pub(super) fn builtin_bytesio_writelines(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "BytesIO.writelines expects one iterable argument",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let values = self.collect_iterable_values(args.remove(0))?;
        for value in values {
            let _ = self.builtin_bytesio_write(
                vec![Value::Instance(receiver.clone()), value],
                HashMap::new(),
            )?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_bytesio_truncate(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("BytesIO.truncate expects optional size"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let (value_obj, pos, closed) = self.bytesio_state_from_instance(&receiver)?;
        let target_size = match args.pop() {
            None | Some(Value::None) => pos as i64,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        if target_size < 0 {
            return Err(RuntimeError::new("ValueError: negative size value"));
        }
        let current_len = {
            let Object::ByteArray(buffer) = &*value_obj.kind() else {
                return Err(RuntimeError::new("BytesIO internal buffer is invalid"));
            };
            buffer.len()
        };
        if target_size as usize != current_len {
            self.bytesio_ensure_resizable(&value_obj)?;
        }
        {
            let Object::ByteArray(buffer) = &mut *value_obj.kind_mut() else {
                return Err(RuntimeError::new("BytesIO internal buffer is invalid"));
            };
            let size = target_size as usize;
            if size < buffer.len() {
                buffer.truncate(size);
            } else if size > buffer.len() {
                buffer.resize(size, 0);
            }
        }
        self.bytesio_store_state(&receiver, value_obj, pos, closed)?;
        Ok(Value::Int(target_size))
    }

    pub(super) fn builtin_bytesio_read(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("BytesIO.read expects 0-1 arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let size = match args.pop() {
            None | Some(Value::None) => -1,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        let (value_obj, pos, closed) = self.bytesio_state_from_instance(&receiver)?;
        let (out, end) = {
            let Object::ByteArray(buffer) = &*value_obj.kind() else {
                return Err(RuntimeError::new("BytesIO internal buffer is invalid"));
            };
            if pos >= buffer.len() {
                return Ok(self.heap.alloc_bytes(Vec::new()));
            }
            let end = if size < 0 {
                buffer.len()
            } else {
                (pos + size as usize).min(buffer.len())
            };
            (buffer[pos..end].to_vec(), end)
        };
        self.bytesio_store_state(&receiver, value_obj, end, closed)?;
        Ok(self.heap.alloc_bytes(out))
    }

    pub(super) fn builtin_bytesio_read1(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_bytesio_read(args, kwargs)
    }

    pub(super) fn builtin_bytesio_readline(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("BytesIO.readline expects 0-1 arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let limit = match args.pop() {
            None | Some(Value::None) => -1,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        if limit == 0 {
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }
        let (value_obj, mut pos, closed) = self.bytesio_state_from_instance(&receiver)?;
        let line_out = {
            let Object::ByteArray(buffer) = &*value_obj.kind() else {
                return Err(RuntimeError::new("BytesIO internal buffer is invalid"));
            };
            if pos > buffer.len() {
                pos = buffer.len();
            }
            if pos >= buffer.len() {
                None
            } else {
                let mut end = pos;
                while end < buffer.len() {
                    if buffer[end] == b'\n' {
                        end += 1;
                        break;
                    }
                    end += 1;
                    if limit >= 0 && (end - pos) as i64 >= limit {
                        break;
                    }
                }
                Some((buffer[pos..end].to_vec(), end))
            }
        };
        let Some((out, end)) = line_out else {
            return Ok(self.heap.alloc_bytes(Vec::new()));
        };
        self.bytesio_store_state(&receiver, value_obj, end, closed)?;
        Ok(self.heap.alloc_bytes(out))
    }

    pub(super) fn builtin_bytesio_readlines(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("BytesIO.readlines expects 0-1 arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let hint = match args.pop() {
            None | Some(Value::None) => -1,
            Some(value) => self.io_index_arg_to_int(value)?,
        };
        let mut lines = Vec::new();
        let mut consumed = 0i64;
        loop {
            let line = self.builtin_bytesio_readline(
                vec![Value::Instance(receiver.clone())],
                HashMap::new(),
            )?;
            let Value::Bytes(bytes_obj) = line else {
                break;
            };
            let line_len = {
                let Object::Bytes(values) = &*bytes_obj.kind() else {
                    break;
                };
                values.len()
            };
            if line_len == 0 {
                break;
            }
            consumed += line_len as i64;
            lines.push(Value::Bytes(bytes_obj));
            if hint > 0 && consumed >= hint {
                break;
            }
        }
        Ok(self.heap.alloc_list(lines))
    }

    pub(super) fn builtin_bytesio_readinto(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("BytesIO.readinto expects 1 argument"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let target = args.remove(0);
        let (value_obj, mut pos, closed) = self.bytesio_state_from_instance(&receiver)?;
        let remaining = {
            let Object::ByteArray(buffer) = &*value_obj.kind() else {
                return Err(RuntimeError::new("BytesIO internal buffer is invalid"));
            };
            if pos > buffer.len() {
                pos = buffer.len();
            }
            buffer[pos..].to_vec()
        };
        let copied = match target {
            Value::ByteArray(obj) => {
                let Object::ByteArray(values) = &mut *obj.kind_mut() else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                let count = values.len().min(remaining.len());
                values[..count].copy_from_slice(&remaining[..count]);
                count
            }
            Value::MemoryView(view_obj) => {
                let source = match &*view_obj.kind() {
                    Object::MemoryView(view) => view.source.clone(),
                    _ => {
                        return Err(RuntimeError::new(
                            "TypeError: readinto() argument must be read-write bytes-like object",
                        ));
                    }
                };
                let Object::ByteArray(values) = &mut *source.kind_mut() else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                let count = values.len().min(remaining.len());
                values[..count].copy_from_slice(&remaining[..count]);
                count
            }
            Value::Module(module_obj) => {
                let Object::Module(module_data) = &mut *module_obj.kind_mut() else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                if module_data.name != "__array__" {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                }
                let Some(Value::List(values_obj)) = module_data.globals.get_mut("values") else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                let Object::List(values) = &mut *values_obj.kind_mut() else {
                    return Err(RuntimeError::new(
                        "TypeError: readinto() argument must be read-write bytes-like object",
                    ));
                };
                let count = values.len().min(remaining.len());
                for (slot, byte) in values.iter_mut().zip(remaining.iter()).take(count) {
                    *slot = Value::Int(*byte as i64);
                }
                count
            }
            _ => {
                return Err(RuntimeError::new(
                    "TypeError: readinto() argument must be read-write bytes-like object",
                ));
            }
        };
        self.bytesio_store_state(&receiver, value_obj, pos + copied, closed)?;
        Ok(Value::Int(copied as i64))
    }

    pub(super) fn builtin_bytesio_getvalue(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.getvalue expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let (value_obj, _pos, _closed) = self.bytesio_state_from_instance(&receiver)?;
        let Object::ByteArray(buffer) = &*value_obj.kind() else {
            return Err(RuntimeError::new("BytesIO internal buffer is invalid"));
        };
        Ok(self.heap.alloc_bytes(buffer.to_vec()))
    }

    pub(super) fn builtin_bytesio_getbuffer(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.getbuffer expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let (value_obj, _pos, _closed) = self.bytesio_state_from_instance(&receiver)?;
        let view = match self.heap.alloc_memoryview(value_obj) {
            Value::MemoryView(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::MemoryView(view_data) = &mut *view.kind_mut() {
            view_data.export_owner = Some(receiver);
            view_data.released = false;
        }
        Ok(Value::MemoryView(view))
    }

    pub(super) fn builtin_bytesio_getstate(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "BytesIO.__getstate__ expects no arguments",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let (value_obj, pos, _closed) = self.bytesio_state_from_instance(&receiver)?;
        let payload = match &*value_obj.kind() {
            Object::ByteArray(values) => values.clone(),
            _ => return Err(RuntimeError::new("BytesIO internal buffer is invalid")),
        };
        let instance_dict = match &*receiver.kind() {
            Object::Instance(instance_data) => {
                let mut entries = Vec::new();
                for (name, value) in &instance_data.attrs {
                    if matches!(name.as_str(), "_value" | "_pos" | "_closed" | "closed") {
                        continue;
                    }
                    entries.push((Value::Str(name.clone()), value.clone()));
                }
                if entries.is_empty() {
                    Value::None
                } else {
                    self.heap.alloc_dict(entries)
                }
            }
            _ => return Err(RuntimeError::new("BytesIO receiver must be instance")),
        };
        Ok(self.heap.alloc_tuple(vec![
            self.heap.alloc_bytes(payload),
            Value::Int(pos as i64),
            instance_dict,
        ]))
    }

    pub(super) fn builtin_bytesio_setstate(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "BytesIO.__setstate__ expects one argument",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let state = args.remove(0);
        let (items, state_type_name) = match state {
            Value::Tuple(tuple_obj) => {
                let Object::Tuple(items) = &*tuple_obj.kind() else {
                    return Err(RuntimeError::new("BytesIO.__setstate__ state is invalid"));
                };
                (items.clone(), "tuple")
            }
            other => {
                let type_name = self.value_type_name_for_error(&other);
                return Err(RuntimeError::new(format!(
                    "TypeError: BytesIO.__setstate__ argument should be 3-tuple, got {type_name}",
                )));
            }
        };
        if items.len() < 3 {
            return Err(RuntimeError::new(format!(
                "TypeError: BytesIO.__setstate__ argument should be 3-tuple, got {state_type_name}",
            )));
        }
        let (value_obj, _pos, _closed) = self.bytesio_state_from_instance(&receiver)?;
        self.bytesio_ensure_resizable(&value_obj)?;
        let payload = self
            .bytesio_payload_from_value(items[0].clone())
            .map_err(|_| {
                RuntimeError::new("TypeError: first item of state should be a bytes-like object")
            })?;
        let position_value = items[1].clone();
        let position = match position_value {
            Value::Int(value) => value,
            Value::Bool(value) => i64::from(value),
            Value::BigInt(ref value) => value.to_i64().ok_or_else(|| {
                RuntimeError::new("OverflowError: position out of range for this platform")
            })?,
            other => {
                return Err(RuntimeError::new(format!(
                    "TypeError: second item of state must be an integer, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        if position < 0 {
            return Err(RuntimeError::new(
                "ValueError: position value cannot be negative",
            ));
        }
        self.bytesio_store_state(&receiver, value_obj.clone(), 0, false)?;
        {
            let Object::ByteArray(values) = &mut *value_obj.kind_mut() else {
                return Err(RuntimeError::new("BytesIO internal buffer is invalid"));
            };
            values.clear();
            values.extend_from_slice(&payload);
        }
        self.bytesio_store_state(&receiver, value_obj, position as usize, false)?;
        let dict_state = items[2].clone();
        match dict_state {
            Value::None => {}
            Value::Dict(dict_obj) => {
                let updates = match &*dict_obj.kind() {
                    Object::Dict(dict) => dict.to_vec(),
                    _ => return Err(RuntimeError::new("third item of state should be a dict")),
                };
                let Object::Instance(instance_data) = &mut *receiver.kind_mut() else {
                    return Err(RuntimeError::new("BytesIO receiver must be instance"));
                };
                for (key, value) in updates {
                    let Value::Str(name) = key else {
                        return Err(RuntimeError::new(
                            "TypeError: third item of state should be a dict",
                        ));
                    };
                    if matches!(name.as_str(), "_value" | "_pos" | "_closed" | "closed") {
                        continue;
                    }
                    instance_data.attrs.insert(name, value);
                }
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "TypeError: third item of state should be a dict, got a {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_bytesio_detach(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.detach expects no arguments"));
        }
        Err(RuntimeError::new("UnsupportedOperation: detach"))
    }

    pub(super) fn builtin_bytesio_seek(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new("BytesIO.seek expects 1-2 arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let offset = self.io_index_arg_to_int(args.remove(0))?;
        let whence = if args.is_empty() {
            0
        } else {
            self.io_index_arg_to_int(args.remove(0))?
        };
        let (value_obj, pos, closed) = self.bytesio_state_from_instance(&receiver)?;
        let new_pos = {
            let Object::ByteArray(buffer) = &*value_obj.kind() else {
                return Err(RuntimeError::new("BytesIO internal buffer is invalid"));
            };
            let new_pos = match whence {
                0 => {
                    if offset < 0 {
                        return Err(RuntimeError::new("ValueError: negative seek value"));
                    }
                    offset
                }
                1 => (pos as i64 + offset).max(0),
                2 => (buffer.len() as i64 + offset).max(0),
                _ => return Err(RuntimeError::new("ValueError: invalid whence")),
            };
            new_pos as usize
        };
        self.bytesio_store_state(&receiver, value_obj, new_pos, closed)?;
        Ok(Value::Int(new_pos as i64))
    }

    pub(super) fn builtin_bytesio_tell(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.tell expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        let (_value_obj, pos, _closed) = self.bytesio_state_from_instance(&receiver)?;
        Ok(Value::Int(pos as i64))
    }

    pub(super) fn builtin_bytesio_flush(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.flush expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_bytesio_isatty(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.isatty expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_bytesio_fileno(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.fileno expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        Err(RuntimeError::new("OSError: fileno"))
    }

    pub(super) fn builtin_bytesio_iter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.__iter__ expects no arguments"));
        }
        Ok(args.remove(0))
    }

    pub(super) fn builtin_bytesio_next(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.__next__ expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let line =
            self.builtin_bytesio_readline(vec![Value::Instance(receiver)], HashMap::new())?;
        let is_empty = if let Value::Bytes(obj) = &line {
            matches!(&*obj.kind(), Object::Bytes(values) if values.is_empty())
        } else {
            false
        };
        if is_empty {
            return Err(RuntimeError::new("StopIteration"));
        }
        Ok(line)
    }

    pub(super) fn builtin_bytesio_enter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.__enter__ expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        Ok(Value::Instance(receiver))
    }

    pub(super) fn builtin_bytesio_exit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 4 {
            return Err(RuntimeError::new(
                "BytesIO.__exit__ expects up to 3 arguments",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let _ = self.builtin_bytesio_close(vec![Value::Instance(receiver)], HashMap::new())?;
        Ok(Value::None)
    }

    pub(super) fn builtin_bytesio_close(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.close expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let (value_obj, _pos, _closed) = self.bytesio_state_from_instance(&receiver)?;
        if self.bytesio_export_count(&value_obj) > 0 {
            return Err(RuntimeError::new(
                "BufferError: Existing exports of data: object cannot be re-sized",
            ));
        }
        if let Object::ByteArray(values) = &mut *value_obj.kind_mut() {
            values.clear();
        }
        self.bytesio_store_state(&receiver, value_obj, 0, true)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_bytesio_readable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.readable expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        Ok(Value::Bool(true))
    }

    pub(super) fn builtin_bytesio_writable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.writable expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        Ok(Value::Bool(true))
    }

    pub(super) fn builtin_bytesio_seekable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("BytesIO.seekable expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        self.bytesio_ensure_open(&receiver)?;
        Ok(Value::Bool(true))
    }

    pub(super) fn parse_struct_format(
        &self,
        format: &str,
    ) -> Result<StructFormatSpec, RuntimeError> {
        let mut chars = format.chars().peekable();
        let mut endian = StructEndian::Little;
        if let Some(prefix) = chars.peek().copied() {
            match prefix {
                '<' => {
                    endian = StructEndian::Little;
                    chars.next();
                }
                '>' | '!' => {
                    endian = StructEndian::Big;
                    chars.next();
                }
                '@' | '=' => {
                    endian = StructEndian::Little;
                    chars.next();
                }
                _ => {}
            }
        }

        let mut fields = Vec::new();
        let mut size = 0usize;
        let mut value_count = 0usize;
        while let Some(ch) = chars.next() {
            let mut count = if ch.is_ascii_digit() {
                let mut digits = String::new();
                digits.push(ch);
                while let Some(next) = chars.peek().copied() {
                    if next.is_ascii_digit() {
                        digits.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                digits
                    .parse::<usize>()
                    .map_err(|_| RuntimeError::new("bad struct format"))?
            } else {
                1
            };
            let code = if ch.is_ascii_digit() {
                chars
                    .next()
                    .ok_or_else(|| RuntimeError::new("bad struct format"))?
            } else {
                ch
            };
            if count == 0 && code != 's' {
                continue;
            }
            let (kind, unit_size, takes_value, repeat_values) = match code {
                'x' => (StructFieldKind::Pad, 1usize, false, false),
                's' => (StructFieldKind::Bytes, 1usize, true, false),
                'c' => (StructFieldKind::Char, 1usize, true, true),
                '?' => (StructFieldKind::Bool, 1usize, true, true),
                'b' => (StructFieldKind::I8, 1usize, true, true),
                'B' => (StructFieldKind::U8, 1usize, true, true),
                'h' => (StructFieldKind::I16, 2usize, true, true),
                'H' => (StructFieldKind::U16, 2usize, true, true),
                'i' | 'l' => (StructFieldKind::I32, 4usize, true, true),
                'I' | 'L' => (StructFieldKind::U32, 4usize, true, true),
                'q' => (StructFieldKind::I64, 8usize, true, true),
                'Q' => (StructFieldKind::U64, 8usize, true, true),
                'f' => (StructFieldKind::F32, 4usize, true, true),
                'd' => (StructFieldKind::F64, 8usize, true, true),
                _ => {
                    return Err(RuntimeError::new(format!(
                        "bad char in struct format: {code}"
                    )));
                }
            };
            if code == 's' && ch.is_ascii_digit() {
                // count is already parsed from prefix digits.
            } else if code == 's' && count == 0 {
                count = 0;
            }
            fields.push(StructFieldSpec { kind, count });
            let field_size = if code == 's' {
                count
            } else {
                unit_size.saturating_mul(count)
            };
            size = size.saturating_add(field_size);
            if takes_value {
                value_count = value_count.saturating_add(if repeat_values { count } else { 1 });
            }
        }
        Ok(StructFormatSpec {
            endian,
            fields,
            size,
            value_count,
        })
    }

    pub(super) fn struct_pack_format_values(
        &mut self,
        spec: &StructFormatSpec,
        values: &[Value],
    ) -> Result<Vec<u8>, RuntimeError> {
        if values.len() != spec.value_count {
            return Err(RuntimeError::new(format!(
                "pack expected {} items for packing (got {})",
                spec.value_count,
                values.len()
            )));
        }
        let mut result = Vec::with_capacity(spec.size);
        let mut value_idx = 0usize;
        for field in &spec.fields {
            match field.kind {
                StructFieldKind::Pad => {
                    result.extend(std::iter::repeat(0u8).take(field.count));
                }
                StructFieldKind::Bytes => {
                    let bytes = bytes_like_from_value(values[value_idx].clone())?;
                    value_idx += 1;
                    if bytes.len() >= field.count {
                        result.extend_from_slice(&bytes[..field.count]);
                    } else {
                        result.extend_from_slice(&bytes);
                        result.extend(std::iter::repeat(0u8).take(field.count - bytes.len()));
                    }
                }
                StructFieldKind::Char => {
                    for _ in 0..field.count {
                        let bytes = bytes_like_from_value(values[value_idx].clone())?;
                        value_idx += 1;
                        if bytes.len() != 1 {
                            return Err(RuntimeError::new(
                                "char format requires a bytes object of length 1",
                            ));
                        }
                        result.push(bytes[0]);
                    }
                }
                StructFieldKind::Bool => {
                    for _ in 0..field.count {
                        let flag = is_truthy(&values[value_idx]);
                        value_idx += 1;
                        result.push(if flag { 1 } else { 0 });
                    }
                }
                StructFieldKind::I8 => {
                    for _ in 0..field.count {
                        let raw = value_to_int(values[value_idx].clone())?;
                        value_idx += 1;
                        let value = i8::try_from(raw).map_err(|_| {
                            RuntimeError::new("byte format requires -128 <= number <= 127")
                        })?;
                        result.push(value as u8);
                    }
                }
                StructFieldKind::U8 => {
                    for _ in 0..field.count {
                        let raw = value_to_int(values[value_idx].clone())?;
                        value_idx += 1;
                        let value = u8::try_from(raw).map_err(|_| {
                            RuntimeError::new("ubyte format requires 0 <= number <= 255")
                        })?;
                        result.push(value);
                    }
                }
                StructFieldKind::I16 => {
                    for _ in 0..field.count {
                        let raw = value_to_int(values[value_idx].clone())?;
                        value_idx += 1;
                        let value = i16::try_from(raw).map_err(|_| {
                            RuntimeError::new("short format requires -32768 <= number <= 32767")
                        })?;
                        let bytes = match spec.endian {
                            StructEndian::Little => value.to_le_bytes(),
                            StructEndian::Big => value.to_be_bytes(),
                        };
                        result.extend_from_slice(&bytes);
                    }
                }
                StructFieldKind::U16 => {
                    for _ in 0..field.count {
                        let raw = value_to_int(values[value_idx].clone())?;
                        value_idx += 1;
                        let value = u16::try_from(raw).map_err(|_| {
                            RuntimeError::new("ushort format requires 0 <= number <= 65535")
                        })?;
                        let bytes = match spec.endian {
                            StructEndian::Little => value.to_le_bytes(),
                            StructEndian::Big => value.to_be_bytes(),
                        };
                        result.extend_from_slice(&bytes);
                    }
                }
                StructFieldKind::I32 => {
                    for _ in 0..field.count {
                        let raw = value_to_int(values[value_idx].clone())?;
                        value_idx += 1;
                        let value = i32::try_from(raw).map_err(|_| {
                            RuntimeError::new(
                                "int format requires -2147483648 <= number <= 2147483647",
                            )
                        })?;
                        let bytes = match spec.endian {
                            StructEndian::Little => value.to_le_bytes(),
                            StructEndian::Big => value.to_be_bytes(),
                        };
                        result.extend_from_slice(&bytes);
                    }
                }
                StructFieldKind::U32 => {
                    for _ in 0..field.count {
                        let raw = value_to_int(values[value_idx].clone())?;
                        value_idx += 1;
                        let value = u32::try_from(raw).map_err(|_| {
                            RuntimeError::new("uint format requires 0 <= number <= 4294967295")
                        })?;
                        let bytes = match spec.endian {
                            StructEndian::Little => value.to_le_bytes(),
                            StructEndian::Big => value.to_be_bytes(),
                        };
                        result.extend_from_slice(&bytes);
                    }
                }
                StructFieldKind::I64 => {
                    for _ in 0..field.count {
                        let value = value_to_int(values[value_idx].clone())?;
                        value_idx += 1;
                        let bytes = match spec.endian {
                            StructEndian::Little => value.to_le_bytes(),
                            StructEndian::Big => value.to_be_bytes(),
                        };
                        result.extend_from_slice(&bytes);
                    }
                }
                StructFieldKind::U64 => {
                    for _ in 0..field.count {
                        let raw = value_to_int(values[value_idx].clone())?;
                        value_idx += 1;
                        let value = u64::try_from(raw)
                            .map_err(|_| RuntimeError::new("argument out of range"))?;
                        let bytes = match spec.endian {
                            StructEndian::Little => value.to_le_bytes(),
                            StructEndian::Big => value.to_be_bytes(),
                        };
                        result.extend_from_slice(&bytes);
                    }
                }
                StructFieldKind::F32 => {
                    for _ in 0..field.count {
                        let value = value_to_f64(values[value_idx].clone())? as f32;
                        value_idx += 1;
                        let bytes = match spec.endian {
                            StructEndian::Little => value.to_le_bytes(),
                            StructEndian::Big => value.to_be_bytes(),
                        };
                        result.extend_from_slice(&bytes);
                    }
                }
                StructFieldKind::F64 => {
                    for _ in 0..field.count {
                        let value = value_to_f64(values[value_idx].clone())?;
                        value_idx += 1;
                        let bytes = match spec.endian {
                            StructEndian::Little => value.to_le_bytes(),
                            StructEndian::Big => value.to_be_bytes(),
                        };
                        result.extend_from_slice(&bytes);
                    }
                }
            }
        }
        Ok(result)
    }

    pub(super) fn struct_unpack_format_bytes(
        &mut self,
        spec: &StructFormatSpec,
        bytes: &[u8],
    ) -> Result<Vec<Value>, RuntimeError> {
        if bytes.len() != spec.size {
            return Err(RuntimeError::new(format!(
                "unpack requires a buffer of {} bytes",
                spec.size
            )));
        }
        let mut values = Vec::with_capacity(spec.value_count);
        let mut pos = 0usize;
        for field in &spec.fields {
            match field.kind {
                StructFieldKind::Pad => {
                    pos += field.count;
                }
                StructFieldKind::Bytes => {
                    values.push(
                        self.heap
                            .alloc_bytes(bytes[pos..pos + field.count].to_vec()),
                    );
                    pos += field.count;
                }
                StructFieldKind::Char => {
                    for _ in 0..field.count {
                        values.push(self.heap.alloc_bytes(vec![bytes[pos]]));
                        pos += 1;
                    }
                }
                StructFieldKind::Bool => {
                    for _ in 0..field.count {
                        values.push(Value::Bool(bytes[pos] != 0));
                        pos += 1;
                    }
                }
                StructFieldKind::I8 => {
                    for _ in 0..field.count {
                        values.push(Value::Int((bytes[pos] as i8) as i64));
                        pos += 1;
                    }
                }
                StructFieldKind::U8 => {
                    for _ in 0..field.count {
                        values.push(Value::Int(bytes[pos] as i64));
                        pos += 1;
                    }
                }
                StructFieldKind::I16 => {
                    for _ in 0..field.count {
                        let value = match spec.endian {
                            StructEndian::Little => {
                                i16::from_le_bytes([bytes[pos], bytes[pos + 1]])
                            }
                            StructEndian::Big => i16::from_be_bytes([bytes[pos], bytes[pos + 1]]),
                        };
                        values.push(Value::Int(value as i64));
                        pos += 2;
                    }
                }
                StructFieldKind::U16 => {
                    for _ in 0..field.count {
                        let value = match spec.endian {
                            StructEndian::Little => {
                                u16::from_le_bytes([bytes[pos], bytes[pos + 1]])
                            }
                            StructEndian::Big => u16::from_be_bytes([bytes[pos], bytes[pos + 1]]),
                        };
                        values.push(Value::Int(value as i64));
                        pos += 2;
                    }
                }
                StructFieldKind::I32 => {
                    for _ in 0..field.count {
                        let value = match spec.endian {
                            StructEndian::Little => i32::from_le_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                            ]),
                            StructEndian::Big => i32::from_be_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                            ]),
                        };
                        values.push(Value::Int(value as i64));
                        pos += 4;
                    }
                }
                StructFieldKind::U32 => {
                    for _ in 0..field.count {
                        let value = match spec.endian {
                            StructEndian::Little => u32::from_le_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                            ]),
                            StructEndian::Big => u32::from_be_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                            ]),
                        };
                        values.push(Value::Int(value as i64));
                        pos += 4;
                    }
                }
                StructFieldKind::I64 => {
                    for _ in 0..field.count {
                        let value = match spec.endian {
                            StructEndian::Little => i64::from_le_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                                bytes[pos + 4],
                                bytes[pos + 5],
                                bytes[pos + 6],
                                bytes[pos + 7],
                            ]),
                            StructEndian::Big => i64::from_be_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                                bytes[pos + 4],
                                bytes[pos + 5],
                                bytes[pos + 6],
                                bytes[pos + 7],
                            ]),
                        };
                        values.push(Value::Int(value));
                        pos += 8;
                    }
                }
                StructFieldKind::U64 => {
                    for _ in 0..field.count {
                        let value = match spec.endian {
                            StructEndian::Little => u64::from_le_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                                bytes[pos + 4],
                                bytes[pos + 5],
                                bytes[pos + 6],
                                bytes[pos + 7],
                            ]),
                            StructEndian::Big => u64::from_be_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                                bytes[pos + 4],
                                bytes[pos + 5],
                                bytes[pos + 6],
                                bytes[pos + 7],
                            ]),
                        };
                        if value <= i64::MAX as u64 {
                            values.push(Value::Int(value as i64));
                        } else {
                            values.push(Value::BigInt(Box::new(BigInt::from_u64(value))));
                        }
                        pos += 8;
                    }
                }
                StructFieldKind::F32 => {
                    for _ in 0..field.count {
                        let value = match spec.endian {
                            StructEndian::Little => f32::from_le_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                            ]),
                            StructEndian::Big => f32::from_be_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                            ]),
                        };
                        values.push(Value::Float(value as f64));
                        pos += 4;
                    }
                }
                StructFieldKind::F64 => {
                    for _ in 0..field.count {
                        let value = match spec.endian {
                            StructEndian::Little => f64::from_le_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                                bytes[pos + 4],
                                bytes[pos + 5],
                                bytes[pos + 6],
                                bytes[pos + 7],
                            ]),
                            StructEndian::Big => f64::from_be_bytes([
                                bytes[pos],
                                bytes[pos + 1],
                                bytes[pos + 2],
                                bytes[pos + 3],
                                bytes[pos + 4],
                                bytes[pos + 5],
                                bytes[pos + 6],
                                bytes[pos + 7],
                            ]),
                        };
                        values.push(Value::Float(value));
                        pos += 8;
                    }
                }
            }
        }
        Ok(values)
    }

    pub(super) fn struct_normalize_offset(
        &self,
        offset: i64,
        buffer_len: usize,
        needed: usize,
    ) -> Result<usize, RuntimeError> {
        let mut start = offset;
        if start < 0 {
            start += buffer_len as i64;
        }
        if start < 0 {
            return Err(RuntimeError::new("offset out of range"));
        }
        let start = start as usize;
        if start > buffer_len || start.saturating_add(needed) > buffer_len {
            return Err(RuntimeError::new(
                "unpack_from requires a buffer of sufficient size",
            ));
        }
        Ok(start)
    }
}
