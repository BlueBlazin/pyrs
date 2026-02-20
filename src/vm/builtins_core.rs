use super::{
    AttrAccessOutcome, AttrMutationOutcome, BYTES_BACKING_STORAGE_ATTR, BigInt, BoundMethod,
    BuiltinFunction, COMPLEX_BACKING_STORAGE_ATTR, ClassBuildOutcome, ClassObject, CodeObject,
    DICT_BACKING_STORAGE_ATTR, FLOAT_BACKING_STORAGE_ATTR, FROZENSET_BACKING_STORAGE_ATTR, Frame,
    GeneratorResumeKind, GeneratorResumeOutcome, HashMap, HashSet, INT_BACKING_STORAGE_ATTR,
    InstanceObject, InternalCallOutcome, IteratorKind, IteratorObject, LIST_BACKING_STORAGE_ATTR,
    ModuleObject, NativeMethodKind, NativeMethodObject, ObjRef, Object, Ordering, Rc, RuntimeError,
    SET_BACKING_STORAGE_ATTR, STR_BACKING_STORAGE_ATTR, SuperObject, TUPLE_BACKING_STORAGE_ATTR,
    Value, Vm, Write, add_values, bigint_from_bytes, bytes_like_from_value,
    call_builtin_with_kwargs, class_attr_lookup, class_attr_walk, class_of_class, compare_ge,
    compare_gt, compare_in, compare_le, compare_lt, compare_order, compiler, decode_text_bytes,
    dedup_hashable_values, dict_remove_value, dict_set_value, dict_set_value_checked, div_values,
    encode_text_bytes, exception_type_is_subclass, format_float_hex, format_repr, format_value,
    frame_cell_value, invert_value, is_import_error_family, is_missing_attribute_error,
    is_os_error_family, is_runtime_type_name_marker, matmul_values, mul_values, neg_value,
    normalize_codec_encoding, normalize_codec_errors, or_values, ordering_from_cmp_value,
    parse_hex_float_literal, parser, pos_value, round_float_with_ndigits,
    runtime_error_matches_exception, sub_values, value_from_bigint, value_from_object_ref,
    value_to_bigint, value_to_f64, value_to_int, weakref_target_id, weakref_target_object,
    with_bytes_like_source, xor_values,
};
use crate::runtime::value_lookup_hash;

impl Vm {
    pub(super) fn value_type_name_for_error(&self, value: &Value) -> String {
        match value {
            Value::None => "NoneType".to_string(),
            Value::Bool(_) => "bool".to_string(),
            Value::Int(_) | Value::BigInt(_) => "int".to_string(),
            Value::Float(_) => "float".to_string(),
            Value::Complex { .. } => "complex".to_string(),
            Value::Str(_) => "str".to_string(),
            Value::List(_) => "list".to_string(),
            Value::Tuple(_) => "tuple".to_string(),
            Value::Dict(_) => "dict".to_string(),
            Value::DictKeys(_) => "dict_keys".to_string(),
            Value::Set(_) => "set".to_string(),
            Value::FrozenSet(_) => "frozenset".to_string(),
            Value::Bytes(_) => "bytes".to_string(),
            Value::ByteArray(_) => "bytearray".to_string(),
            Value::MemoryView(_) => "memoryview".to_string(),
            Value::Iterator(_) => "iterator".to_string(),
            Value::Generator(_) => "generator".to_string(),
            Value::Slice { .. } => "slice".to_string(),
            Value::Module(_) => "module".to_string(),
            Value::Super(_) => "super".to_string(),
            Value::BoundMethod(_) => "method".to_string(),
            Value::Exception(exc) => exc.name.clone(),
            Value::ExceptionType(name) => name.clone(),
            Value::Code(_) => "code".to_string(),
            Value::Function(_) => "function".to_string(),
            Value::Builtin(builtin) => self.builtin_type_name(*builtin).to_string(),
            Value::Class(class) => match &*class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "type".to_string(),
            },
            Value::Instance(instance) => {
                let instance_kind = instance.kind();
                match &*instance_kind {
                    Object::Instance(instance_data) => match &*instance_data.class.kind() {
                        Object::Class(class_data) => class_data.name.clone(),
                        _ => "object".to_string(),
                    },
                    _ => "object".to_string(),
                }
            }
            Value::Cell(_) => "cell".to_string(),
        }
    }

    fn truthy_from_len_result(&self, result: Value) -> Result<bool, RuntimeError> {
        match result {
            Value::Bool(flag) => Ok(flag),
            Value::Int(number) => {
                if number < 0 {
                    Err(RuntimeError::new("__len__() should return >= 0"))
                } else {
                    Ok(number != 0)
                }
            }
            Value::BigInt(number) => {
                if number.is_negative() {
                    Err(RuntimeError::new("__len__() should return >= 0"))
                } else {
                    Ok(!number.is_zero())
                }
            }
            other => Err(RuntimeError::new(format!(
                "'{}' object cannot be interpreted as an integer",
                self.value_type_name_for_error(&other)
            ))),
        }
    }

    pub(super) fn truthy_from_value(&mut self, value: &Value) -> Result<bool, RuntimeError> {
        match value {
            Value::None => Ok(false),
            Value::Bool(flag) => Ok(*flag),
            Value::Int(number) => Ok(*number != 0),
            Value::BigInt(number) => Ok(!number.is_zero()),
            Value::Float(number) => Ok(*number != 0.0),
            Value::Complex { real, imag } => Ok(*real != 0.0 || *imag != 0.0),
            Value::Str(text) => Ok(!text.is_empty()),
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => Ok(!values.is_empty()),
                _ => Ok(true),
            },
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => Ok(!values.is_empty()),
                _ => Ok(true),
            },
            Value::Dict(obj) => match &*obj.kind() {
                Object::Dict(values) => Ok(!values.is_empty()),
                _ => Ok(true),
            },
            Value::DictKeys(obj) => match &*obj.kind() {
                Object::DictKeysView(view) => match &*view.dict.kind() {
                    Object::Dict(values) => Ok(!values.is_empty()),
                    _ => Ok(true),
                },
                _ => Ok(true),
            },
            Value::Set(obj) => match &*obj.kind() {
                Object::Set(values) => Ok(!values.is_empty()),
                _ => Ok(true),
            },
            Value::FrozenSet(obj) => match &*obj.kind() {
                Object::FrozenSet(values) => Ok(!values.is_empty()),
                _ => Ok(true),
            },
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(values) => Ok(!values.is_empty()),
                _ => Ok(true),
            },
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(values) => Ok(!values.is_empty()),
                _ => Ok(true),
            },
            Value::MemoryView(obj) => match &*obj.kind() {
                Object::MemoryView(view) => {
                    Ok(
                        with_bytes_like_source(&view.source, |values| !values.is_empty())
                            .unwrap_or(true),
                    )
                }
                _ => Ok(true),
            },
            Value::Cell(obj) => match &*obj.kind() {
                Object::Cell(cell) => match &cell.value {
                    Some(inner) => self.truthy_from_value(inner),
                    None => Ok(false),
                },
                _ => Ok(true),
            },
            Value::Iterator(_) | Value::Generator(_) | Value::Slice { .. } => Ok(true),
            other => {
                if let Some(result) = self.cpython_proxy_truthy(other) {
                    return result;
                }
                if let Some(bool_method) = self.lookup_bound_special_method(other, "__bool__")? {
                    let bool_value =
                        match self.call_internal(bool_method, Vec::new(), HashMap::new())? {
                            InternalCallOutcome::Value(value) => value,
                            InternalCallOutcome::CallerExceptionHandled => {
                                return Err(
                                    self.runtime_error_from_active_exception("__bool__() failed")
                                );
                            }
                        };
                    return match bool_value {
                        Value::Bool(flag) => Ok(flag),
                        non_bool => Err(RuntimeError::new(format!(
                            "__bool__ should return bool, returned {}",
                            self.value_type_name_for_error(&non_bool)
                        ))),
                    };
                }
                if let Some(len_method) = self.lookup_bound_special_method(other, "__len__")? {
                    let len_value =
                        match self.call_internal(len_method, Vec::new(), HashMap::new())? {
                            InternalCallOutcome::Value(value) => value,
                            InternalCallOutcome::CallerExceptionHandled => {
                                return Err(
                                    self.runtime_error_from_active_exception("__len__() failed")
                                );
                            }
                        };
                    return self.truthy_from_len_result(len_value);
                }
                Ok(true)
            }
        }
    }

    pub(super) fn builtin_print(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let sep = match kwargs.remove("sep") {
            None | Some(Value::None) => " ".to_string(),
            Some(Value::Str(text)) => text,
            Some(other) => {
                return Err(RuntimeError::new(format!(
                    "TypeError: sep must be None or a string, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        let end = match kwargs.remove("end") {
            None | Some(Value::None) => "\n".to_string(),
            Some(Value::Str(text)) => text,
            Some(other) => {
                return Err(RuntimeError::new(format!(
                    "TypeError: end must be None or a string, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        let file = kwargs.remove("file").unwrap_or(Value::None);
        let flush_requested = match kwargs.remove("flush") {
            Some(value) => self.truthy_from_value(&value)?,
            None => false,
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "print() got an unexpected keyword argument",
            ));
        }

        let mut parts = Vec::with_capacity(args.len());
        for value in args {
            let rendered = match self.builtin_str(vec![value], HashMap::new())? {
                Value::Str(text) => text,
                _ => return Err(RuntimeError::new("str() returned non-string")),
            };
            parts.push(rendered);
        }
        let rendered = format!("{}{}", parts.join(&sep), end);

        if matches!(file, Value::None) {
            print!("{rendered}");
            if flush_requested {
                let _ = std::io::stdout().flush();
            }
            return Ok(Value::None);
        }

        let write = self.builtin_getattr(
            vec![file.clone(), Value::Str("write".to_string())],
            HashMap::new(),
        )?;
        match self.call_internal(write, vec![Value::Str(rendered)], HashMap::new())? {
            InternalCallOutcome::Value(_) => {}
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(self.runtime_error_from_active_exception("print() write failed"));
            }
        }
        if flush_requested
            && let Ok(flush) =
                self.builtin_getattr(vec![file, Value::Str("flush".to_string())], HashMap::new())
        {
            match self.call_internal(flush, Vec::new(), HashMap::new())? {
                InternalCallOutcome::Value(_) => {}
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception("print() flush failed"));
                }
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_input(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("input() expects an optional prompt"));
        }
        if let Some(prompt) = args.pop() {
            let text = match self.builtin_str(vec![prompt], HashMap::new())? {
                Value::Str(text) => text,
                _ => return Err(RuntimeError::new("str() returned non-string")),
            };
            print!("{text}");
            let _ = std::io::stdout().flush();
        }
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|err| RuntimeError::new(format!("input() failed: {err}")))?;
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        Ok(Value::Str(line))
    }

    pub(super) fn builtin_repr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("repr() expects one argument"));
        }
        let value = args.remove(0);
        let is_proxy = Self::cpython_proxy_raw_ptr_from_value(&value).is_some();
        if is_proxy {
            match self.cpython_proxy_repr(&value) {
                Some(Ok(text)) => return Ok(Value::Str(text)),
                Some(Err(_)) | None => return Ok(Value::Str(format_value(&value))),
            }
        }
        if let Value::Instance(instance) = &value
            && let Object::Instance(instance_data) = &*instance.kind()
        {
            let is_text_wrapper = matches!(
                &*instance_data.class.kind(),
                Object::Class(class_data) if class_data.name == "TextIOWrapper"
            );
            let is_uninitialized = matches!(
                instance_data.attrs.get("__pyrs_text_uninitialized"),
                Some(Value::Bool(true))
            ) || !instance_data.attrs.contains_key("buffer");
            if is_text_wrapper && is_uninitialized {
                return Err(RuntimeError::new(
                    "ValueError: I/O operation on uninitialized object",
                ));
            }
        }
        if matches!(&value, Value::Instance(_) | Value::Exception(_)) {
            match self.builtin_getattr(
                vec![value.clone(), Value::Str("__repr__".to_string())],
                HashMap::new(),
            ) {
                Ok(repr_method) => {
                    let is_recursive_builtin_repr = match &repr_method {
                        Value::BoundMethod(bound) => match &*bound.kind() {
                            Object::BoundMethod(bound_data) => match &*bound_data.function.kind() {
                                Object::NativeMethod(native) => matches!(
                                    native.kind,
                                    NativeMethodKind::Builtin(BuiltinFunction::Repr)
                                ),
                                _ => false,
                            },
                            _ => false,
                        },
                        Value::Builtin(BuiltinFunction::Repr) => true,
                        _ => false,
                    };
                    if !is_recursive_builtin_repr {
                        match self.call_internal(repr_method, Vec::new(), HashMap::new())? {
                            InternalCallOutcome::Value(Value::Str(text)) => {
                                return Ok(Value::Str(text));
                            }
                            InternalCallOutcome::Value(_) => {
                                return Err(RuntimeError::new("__repr__ returned non-string"));
                            }
                            InternalCallOutcome::CallerExceptionHandled => {
                                return Err(
                                    self.runtime_error_from_active_exception("repr() failed")
                                );
                            }
                        }
                    }
                }
                Err(err) => {
                    if !is_missing_attribute_error(&err) {
                        return Err(err);
                    }
                }
            }
        }
        let render_nested = |vm: &mut Vm, nested: Value| -> Result<String, RuntimeError> {
            match vm.builtin_repr(vec![nested], HashMap::new())? {
                Value::Str(text) => Ok(text),
                _ => Err(RuntimeError::new("__repr__ returned non-string")),
            }
        };

        let repr_guard = match &value {
            Value::List(obj) => Some((obj.id(), "[...]".to_string())),
            Value::Tuple(obj) => Some((obj.id(), "(...)".to_string())),
            Value::Dict(obj) => Some((obj.id(), "{...}".to_string())),
            Value::Set(obj) => Some((obj.id(), "{...}".to_string())),
            Value::FrozenSet(obj) => Some((obj.id(), "frozenset({...})".to_string())),
            _ => None,
        };
        if let Some((id, marker)) = &repr_guard {
            if self.repr_in_progress.contains(id) {
                return Ok(Value::Str(marker.clone()));
            }
            self.repr_in_progress.push(*id);
        }
        let rendered = match value {
            Value::List(obj) => {
                let values = match &*obj.kind() {
                    Object::List(values) => values.clone(),
                    _ => return Ok(Value::Str("<list>".to_string())),
                };
                let mut parts = Vec::with_capacity(values.len());
                for item in values {
                    parts.push(render_nested(self, item)?);
                }
                Ok(Value::Str(format!("[{}]", parts.join(", "))))
            }
            Value::Tuple(obj) => {
                let values = match &*obj.kind() {
                    Object::Tuple(values) => values.clone(),
                    _ => return Ok(Value::Str("<tuple>".to_string())),
                };
                let mut parts = Vec::with_capacity(values.len());
                for item in values {
                    parts.push(render_nested(self, item)?);
                }
                if parts.len() == 1 {
                    Ok(Value::Str(format!("({},)", parts[0])))
                } else {
                    Ok(Value::Str(format!("({})", parts.join(", "))))
                }
            }
            Value::Dict(obj) => {
                let values = match &*obj.kind() {
                    Object::Dict(values) => values.clone(),
                    _ => return Ok(Value::Str("<dict>".to_string())),
                };
                let mut parts = Vec::with_capacity(values.len());
                for (key, value) in values {
                    let key_repr = render_nested(self, key)?;
                    let value_repr = render_nested(self, value)?;
                    parts.push(format!("{key_repr}: {value_repr}"));
                }
                Ok(Value::Str(format!("{{{}}}", parts.join(", "))))
            }
            Value::Set(obj) => {
                let values = match &*obj.kind() {
                    Object::Set(values) => values.clone(),
                    _ => return Ok(Value::Str("<set>".to_string())),
                };
                if values.is_empty() {
                    return Ok(Value::Str("set()".to_string()));
                }
                let mut parts = Vec::with_capacity(values.len());
                for value in values {
                    parts.push(render_nested(self, value)?);
                }
                Ok(Value::Str(format!("{{{}}}", parts.join(", "))))
            }
            Value::FrozenSet(obj) => {
                let values = match &*obj.kind() {
                    Object::FrozenSet(values) => values.clone(),
                    _ => return Ok(Value::Str("<frozenset>".to_string())),
                };
                let mut parts = Vec::with_capacity(values.len());
                for value in values {
                    parts.push(render_nested(self, value)?);
                }
                Ok(Value::Str(format!("frozenset({{{}}})", parts.join(", "))))
            }
            Value::Instance(instance) => {
                if let Some(rendered) = self.io_instance_repr(&instance)? {
                    Ok(Value::Str(rendered))
                } else {
                    Ok(Value::Str(format_repr(&Value::Instance(instance))))
                }
            }
            other => Ok(Value::Str(format_repr(&other))),
        };
        if repr_guard.is_some() {
            self.repr_in_progress.pop();
        }
        rendered
    }

    fn io_instance_repr(&mut self, instance: &ObjRef) -> Result<Option<String>, RuntimeError> {
        let (class_ref, class_name) = match &*instance.kind() {
            Object::Instance(instance_data) => match &*instance_data.class.kind() {
                Object::Class(class_data) => (instance_data.class.clone(), class_data.name.clone()),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        let mro_names = class_attr_walk(&class_ref)
            .into_iter()
            .filter_map(|candidate| match &*candidate.kind() {
                Object::Class(class_data) => Some(class_data.name.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let has_mro = |needle: &str| mro_names.iter().any(|name| name == needle);
        let try_getattr =
            |vm: &mut Vm, target: Value, name: &str| -> Result<Option<Value>, RuntimeError> {
                match vm.builtin_getattr(vec![target, Value::Str(name.to_string())], HashMap::new())
                {
                    Ok(value) => Ok(Some(value)),
                    Err(err) if is_missing_attribute_error(&err) => Ok(None),
                    Err(err) => Err(err),
                }
            };
        let repr_text = |vm: &mut Vm, value: Value| -> Result<String, RuntimeError> {
            match vm.builtin_repr(vec![value], HashMap::new())? {
                Value::Str(text) => Ok(text),
                _ => Err(RuntimeError::new("__repr__ returned non-string")),
            }
        };

        if has_mro("BufferedReader")
            || has_mro("BufferedWriter")
            || has_mro("BufferedRandom")
            || has_mro("BufferedRWPair")
        {
            let mut parts = Vec::new();
            if let Some(raw) = try_getattr(self, Value::Instance(instance.clone()), "raw")?
                && !matches!(raw, Value::None)
                && let Some(name_value) = try_getattr(self, raw, "name")?
            {
                if matches!(&name_value, Value::Instance(name_obj) if name_obj.id() == instance.id())
                {
                    return Err(RuntimeError::new(
                        "maximum recursion depth exceeded while getting the repr of an object",
                    ));
                }
                parts.push(format!("name={}", repr_text(self, name_value)?));
            }
            let suffix = if parts.is_empty() {
                String::new()
            } else {
                format!(" {}", parts.join(" "))
            };
            return Ok(Some(format!("<{}{suffix}>", class_name)));
        }

        if has_mro("TextIOWrapper") {
            let mut parts = Vec::new();
            if let Some(raw) = try_getattr(self, Value::Instance(instance.clone()), "raw")?
                && !matches!(raw, Value::None)
                && let Some(name_value) = try_getattr(self, raw, "name")?
            {
                if matches!(&name_value, Value::Instance(name_obj) if name_obj.id() == instance.id())
                {
                    return Err(RuntimeError::new(
                        "maximum recursion depth exceeded while getting the repr of an object",
                    ));
                }
                parts.push(format!("name={}", repr_text(self, name_value)?));
            }
            if let Some(mode_value) = try_getattr(self, Value::Instance(instance.clone()), "mode")?
            {
                parts.push(format!("mode={}", repr_text(self, mode_value)?));
            }
            if let Some(encoding_value) =
                try_getattr(self, Value::Instance(instance.clone()), "encoding")?
            {
                parts.push(format!("encoding={}", repr_text(self, encoding_value)?));
            }
            let suffix = if parts.is_empty() {
                String::new()
            } else {
                format!(" {}", parts.join(" "))
            };
            return Ok(Some(format!("<{}{suffix}>", class_name)));
        }

        Ok(None)
    }

    pub(super) fn builtin_ascii(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let repr = self.builtin_repr(args, kwargs)?;
        let Value::Str(text) = repr else {
            return Err(RuntimeError::new("ascii() expects one argument"));
        };
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            if ch.is_ascii() {
                out.push(ch);
                continue;
            }
            let code = ch as u32;
            if code <= 0xFF {
                out.push_str(&format!("\\x{code:02x}"));
            } else if code <= 0xFFFF {
                out.push_str(&format!("\\u{code:04x}"));
            } else {
                out.push_str(&format!("\\U{code:08x}"));
            }
        }
        Ok(Value::Str(out))
    }

    pub(crate) fn render_value_repr_for_display(
        &mut self,
        value: Value,
    ) -> Result<String, RuntimeError> {
        if !self.frames.is_empty() {
            return match self.builtin_repr(vec![value], HashMap::new())? {
                Value::Str(text) => Ok(text),
                _ => Err(RuntimeError::new("__repr__ returned non-string")),
            };
        }
        const TEMP_REPR_NAME: &str = "__pyrs_repl_repr_target__";
        let previous = self.get_global(TEMP_REPR_NAME);
        self.set_global(TEMP_REPR_NAME, value);
        let render_result = (|| -> Result<String, RuntimeError> {
            let expr =
                parser::parse_expression("repr(__pyrs_repl_repr_target__)").map_err(|err| {
                    RuntimeError::new(format!("repr() parse failed: {}", err.message))
                })?;
            let code = compiler::compile_expression_with_filename(&expr, "<repl-repr>").map_err(
                |err| RuntimeError::new(format!("repr() compile failed: {}", err.message)),
            )?;
            match self.execute(&code)? {
                Value::Str(text) => Ok(text),
                _ => Err(RuntimeError::new("__repr__ returned non-string")),
            }
        })();
        match previous {
            Some(value) => self.set_global(TEMP_REPR_NAME, value),
            None => {
                self.remove_global(TEMP_REPR_NAME);
            }
        }
        render_result
    }

    pub(super) fn builtin_locals(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("locals() expects no arguments"));
        }
        let frame_index = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or_else(|| RuntimeError::new("no frame"))?;
        if self.frames[frame_index].is_module {
            let dict = self.ensure_frame_module_locals_dict(frame_index);
            return Ok(Value::Dict(dict));
        }
        let frame = &self.frames[frame_index];
        let mut map = frame.locals.clone();
        for (idx, slot) in frame.fast_locals.iter().enumerate() {
            if let Some(value) = slot
                && let Some(name) = frame.code.names.get(idx)
            {
                map.insert(name.clone(), value.clone());
            }
        }
        for (idx, name) in frame.code.cellvars.iter().enumerate() {
            if !map.contains_key(name)
                && let Some(cell) = frame.cells.get(idx)
                && let Object::Cell(cell_data) = &*cell.kind()
            {
                map.insert(name.clone(), cell_data.value.clone().unwrap_or(Value::None));
            }
        }
        let cell_offset = frame.code.cellvars.len();
        for (idx, name) in frame.code.freevars.iter().enumerate() {
            if !map.contains_key(name)
                && let Some(cell) = frame.cells.get(cell_offset + idx)
                && let Object::Cell(cell_data) = &*cell.kind()
            {
                map.insert(name.clone(), cell_data.value.clone().unwrap_or(Value::None));
            }
        }
        let mut entries = Vec::with_capacity(map.len());
        for (name, value) in map {
            entries.push((Value::Str(name), value));
        }
        Ok(self.heap.alloc_dict(entries))
    }

    pub(super) fn builtin_globals(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("globals() expects no arguments"));
        }
        let frame_index = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or_else(|| RuntimeError::new("no frame"))?;
        if self.frames[frame_index].is_module {
            let dict = self.ensure_frame_module_locals_dict(frame_index);
            return Ok(Value::Dict(dict));
        }
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| RuntimeError::new("no frame"))?;
        let globals_module = frame.function_globals.clone();
        if let Some(frame_index) = self
            .frames
            .iter()
            .rposition(|item| item.module.id() == globals_module.id())
        {
            let dict = self.ensure_frame_module_locals_dict(frame_index);
            return Ok(Value::Dict(dict));
        }
        if let Object::Module(module_data) = &*globals_module.kind() {
            let entries = module_data
                .globals
                .iter()
                .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                .collect::<Vec<_>>();
            Ok(self.heap.alloc_dict(entries))
        } else {
            Ok(self.heap.alloc_dict(Vec::new()))
        }
    }

    pub(super) fn builtin_vars(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: vars() got an unexpected keyword argument",
            ));
        }
        if args.len() > 1 {
            return Err(RuntimeError::new(
                "TypeError: vars() takes at most one argument",
            ));
        }
        if args.is_empty() {
            return self.builtin_locals(Vec::new(), HashMap::new());
        }
        let target = args.remove(0);
        match self.builtin_getattr(
            vec![target, Value::Str("__dict__".to_string())],
            HashMap::new(),
        ) {
            Ok(value) => Ok(value),
            Err(err) if is_missing_attribute_error(&err) => Err(RuntimeError::new(
                "TypeError: vars() argument must have __dict__ attribute",
            )),
            Err(err) => Err(err),
        }
    }

    pub(super) fn builtin_hash(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: hash() got an unexpected keyword argument",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error("hash() expects one argument"));
        }
        let target = args.remove(0);
        let hash_value = match target {
            Value::Instance(_) | Value::Class(_) | Value::Super(_) => {
                let Some(hash_method) = self.lookup_bound_special_method(&target, "__hash__")?
                else {
                    return Err(RuntimeError::new(format!(
                        "TypeError: unhashable type: '{}'",
                        self.value_type_name_for_error(&target)
                    )));
                };
                let result = match self.call_internal(hash_method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(value) => value_to_int(value)?,
                    InternalCallOutcome::CallerExceptionHandled => return Ok(Value::None),
                };
                if result == -1 { -2 } else { result }
            }
            _ => {
                let Some(hash_bits) = value_lookup_hash(&target) else {
                    return Err(RuntimeError::new(format!(
                        "TypeError: unhashable type: '{}'",
                        self.value_type_name_for_error(&target)
                    )));
                };
                let result = hash_bits as i64;
                if result == -1 { -2 } else { result }
            }
        };
        Ok(Value::Int(hash_value))
    }

    pub(super) fn builtin_breakpoint(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let breakpointhook =
            self.modules
                .get("sys")
                .and_then(|sys_module| match &*sys_module.kind() {
                    Object::Module(module_data) => {
                        module_data.globals.get("breakpointhook").cloned()
                    }
                    _ => None,
                });
        let Some(hook) = breakpointhook else {
            return Ok(Value::None);
        };
        match self.call_internal(hook, args, kwargs)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => Ok(Value::None),
        }
    }

    pub(super) fn exec_namespace_map_from_dict(
        &self,
        dict: &ObjRef,
        what: &str,
    ) -> Result<HashMap<String, Value>, RuntimeError> {
        let Object::Dict(entries) = &*dict.kind() else {
            return Err(RuntimeError::new(format!(
                "exec() {what} must be a dict or module"
            )));
        };
        let mut map = HashMap::new();
        for (key, value) in entries {
            if let Value::Str(name) = key {
                map.insert(name.clone(), value.clone());
            }
        }
        Ok(map)
    }

    pub(super) fn exec_namespace_map_from_module(
        &self,
        module: &ObjRef,
    ) -> Result<HashMap<String, Value>, RuntimeError> {
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("exec() internal module expected"));
        };
        Ok(module_data.globals.clone())
    }

    pub(super) fn alloc_exec_namespace_module(
        &self,
        name: &str,
        map: HashMap<String, Value>,
    ) -> ObjRef {
        let module = match self.heap.alloc_module(ModuleObject::new(name)) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data.globals = map;
        }
        module
    }

    pub(super) fn sync_exec_namespace_to_dict(
        &self,
        dict: &ObjRef,
        module: &ObjRef,
    ) -> Result<(), RuntimeError> {
        let new_values = self.exec_namespace_map_from_module(module)?;
        let Object::Dict(entries) = &mut *dict.kind_mut() else {
            return Err(RuntimeError::new("exec() internal dict expected"));
        };
        entries.retain(|(key, _)| !matches!(key, Value::Str(_)));
        for (name, value) in new_values {
            entries.push((Value::Str(name), value));
        }
        Ok(())
    }

    pub(super) fn exec_closure_cells(
        &self,
        code: &Rc<CodeObject>,
        closure: Option<Value>,
    ) -> Result<Vec<ObjRef>, RuntimeError> {
        let Some(value) = closure else {
            if code.freevars.is_empty() {
                return Ok(Vec::new());
            }
            return Err(RuntimeError::new(
                "exec() code object requires a closure of cell objects",
            ));
        };
        if value == Value::None {
            if code.freevars.is_empty() {
                return Ok(Vec::new());
            }
            return Err(RuntimeError::new(
                "exec() code object requires a closure of cell objects",
            ));
        }
        let tuple = match value {
            Value::Tuple(obj) => obj,
            _ => {
                return Err(RuntimeError::new(
                    "exec() closure must be a tuple of cell objects",
                ));
            }
        };
        let Object::Tuple(values) = &*tuple.kind() else {
            return Err(RuntimeError::new(
                "exec() closure must be a tuple of cell objects",
            ));
        };
        if values.len() != code.freevars.len() {
            return Err(RuntimeError::new(
                "exec() closure size does not match code free vars",
            ));
        }
        let mut cells = Vec::with_capacity(values.len());
        for value in values {
            match value {
                Value::Cell(cell) => cells.push(cell.clone()),
                _ => {
                    return Err(RuntimeError::new(
                        "exec() closure must contain only cell objects",
                    ));
                }
            }
        }
        Ok(cells)
    }

    pub(super) fn builtin_exec(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new(
                "exec() expects source plus optional globals and locals",
            ));
        }

        let source = args.remove(0);
        let globals_kw = kwargs.remove("globals");
        let locals_kw = kwargs.remove("locals");
        let closure_kw = kwargs.remove("closure");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "exec() got an unexpected keyword argument",
            ));
        }

        let globals_arg = if let Some(arg) = args.first().cloned() {
            if globals_kw.is_some() {
                return Err(RuntimeError::new("exec() got multiple values for globals"));
            }
            Some(arg)
        } else {
            globals_kw
        };
        let locals_arg = if let Some(arg) = args.get(1).cloned() {
            if locals_kw.is_some() {
                return Err(RuntimeError::new("exec() got multiple values for locals"));
            }
            Some(arg)
        } else {
            locals_kw
        };

        let code = match source {
            Value::Code(code) => code,
            Value::Str(source) => {
                let module_ast = parser::parse_module(&source).map_err(|err| {
                    RuntimeError::new(format!(
                        "exec() parse error at {}: {}",
                        err.offset, err.message
                    ))
                })?;
                Rc::new(
                    compiler::compile_module_with_filename(&module_ast, "<exec>").map_err(
                        |err| RuntimeError::new(format!("exec() compile error: {}", err.message)),
                    )?,
                )
            }
            _ => {
                return Err(RuntimeError::new(
                    "exec() source must be a string or code object",
                ));
            }
        };

        let caller_depth = self.frames.len();
        if caller_depth == 0 {
            return Err(RuntimeError::new("exec() requires an active frame"));
        }
        let caller_index = caller_depth - 1;
        let caller_globals = self.frames[caller_index].function_globals.clone();
        let caller_module = self.frames[caller_index].module.clone();
        let caller_is_module = self.frames[caller_index].is_module;
        let caller_locals = if caller_is_module {
            HashMap::new()
        } else {
            self.frames[caller_index].locals.clone()
        };

        let mut globals_module = caller_globals.clone();
        let mut globals_dict_writeback: Option<(ObjRef, ObjRef)> = None;
        let mut globals_explicit = false;
        if let Some(value) = globals_arg
            && value != Value::None
        {
            globals_explicit = true;
            match value {
                Value::Module(module) => {
                    globals_module = module;
                }
                Value::Dict(dict) => {
                    let module = self.alloc_exec_namespace_module(
                        "<exec_globals>",
                        self.exec_namespace_map_from_dict(&dict, "globals")?,
                    );
                    globals_dict_writeback = Some((dict, module.clone()));
                    globals_module = module;
                }
                _ => {
                    return Err(RuntimeError::new("exec() globals must be a dict or module"));
                }
            }
        }

        let mut locals_module = globals_module.clone();
        let mut locals_dict_writeback: Option<(ObjRef, ObjRef)> = None;
        let mut caller_locals_writeback: Option<ObjRef> = None;
        if let Some(value) = locals_arg {
            if value != Value::None {
                match value {
                    Value::Module(module) => {
                        locals_module = module;
                    }
                    Value::Dict(dict) => {
                        if let Some((global_dict, global_module)) = &globals_dict_writeback {
                            if global_dict.id() == dict.id() {
                                locals_module = global_module.clone();
                            } else {
                                let module = self.alloc_exec_namespace_module(
                                    "<exec_locals>",
                                    self.exec_namespace_map_from_dict(&dict, "locals")?,
                                );
                                locals_dict_writeback = Some((dict, module.clone()));
                                locals_module = module;
                            }
                        } else {
                            let module = self.alloc_exec_namespace_module(
                                "<exec_locals>",
                                self.exec_namespace_map_from_dict(&dict, "locals")?,
                            );
                            locals_dict_writeback = Some((dict, module.clone()));
                            locals_module = module;
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("exec() locals must be a dict or module"));
                    }
                }
            } else if !globals_explicit {
                if caller_is_module {
                    locals_module = caller_module;
                } else {
                    let module =
                        self.alloc_exec_namespace_module("<exec_locals>", caller_locals.clone());
                    caller_locals_writeback = Some(module.clone());
                    locals_module = module;
                }
            }
        } else if !globals_explicit {
            if caller_is_module {
                locals_module = caller_module;
            } else {
                let module = self.alloc_exec_namespace_module("<exec_locals>", caller_locals);
                caller_locals_writeback = Some(module.clone());
                locals_module = module;
            }
        }

        let closure_cells = self.exec_closure_cells(&code, closure_kw)?;
        let cells = self.build_cells(&code, closure_cells);
        let mut frame = Frame::new(code, locals_module.clone(), true, false, cells, None);
        frame.function_globals = globals_module.clone();
        if locals_module.id() != globals_module.id() {
            frame.globals_fallback = Some(globals_module.clone());
        }
        frame.discard_result = true;
        self.frames.push(Box::new(frame));

        let previous_stop = self.run_stop_depth;
        self.run_stop_depth = Some(caller_depth);
        let run_result = self.run();
        self.run_stop_depth = previous_stop;

        if let Some((dict, module)) = &globals_dict_writeback {
            self.sync_exec_namespace_to_dict(dict, module)?;
        }
        if let Some((dict, module)) = &locals_dict_writeback {
            self.sync_exec_namespace_to_dict(dict, module)?;
        }
        if let Some(module) = &caller_locals_writeback {
            let locals = self.exec_namespace_map_from_module(module)?;
            if let Some(frame) = self.frames.get_mut(caller_index) {
                frame.locals = locals;
            }
        }

        run_result?;
        Ok(Value::None)
    }

    pub(super) fn builtin_eval(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new(
                "eval() expects source plus optional globals and locals",
            ));
        }

        let source = args.remove(0);
        let globals_kw = kwargs.remove("globals");
        let locals_kw = kwargs.remove("locals");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "eval() got an unexpected keyword argument",
            ));
        }

        let globals_arg = if let Some(arg) = args.first().cloned() {
            if globals_kw.is_some() {
                return Err(RuntimeError::new("eval() got multiple values for globals"));
            }
            Some(arg)
        } else {
            globals_kw
        };
        let locals_arg = if let Some(arg) = args.get(1).cloned() {
            if locals_kw.is_some() {
                return Err(RuntimeError::new("eval() got multiple values for locals"));
            }
            Some(arg)
        } else {
            locals_kw
        };

        let code = match source {
            Value::Code(code) => code,
            source => {
                let source_text = match source {
                    Value::Str(text) => text,
                    other => {
                        let bytes = bytes_like_from_value(other)?;
                        String::from_utf8(bytes)
                            .map_err(|_| RuntimeError::new("eval() source is not valid UTF-8"))?
                    }
                };
                let expr_ast = parser::parse_expression(&source_text).map_err(|err| {
                    RuntimeError::new(format!(
                        "eval() parse error at {}: {}",
                        err.offset, err.message
                    ))
                })?;
                Rc::new(
                    compiler::compile_expression_with_filename(&expr_ast, "<eval>").map_err(
                        |err| RuntimeError::new(format!("eval() compile error: {}", err.message)),
                    )?,
                )
            }
        };
        if !code.freevars.is_empty() {
            return Err(RuntimeError::new(
                "eval() code object may not contain free variables",
            ));
        }

        let caller_depth = self.frames.len();
        if caller_depth == 0 {
            return Err(RuntimeError::new("eval() requires an active frame"));
        }
        let caller_index = caller_depth - 1;
        let caller_globals = self.frames[caller_index].function_globals.clone();
        let caller_module = self.frames[caller_index].module.clone();
        let caller_is_module = self.frames[caller_index].is_module;
        let caller_locals = if caller_is_module {
            HashMap::new()
        } else {
            self.frames[caller_index].locals.clone()
        };

        let mut globals_module = caller_globals.clone();
        let mut globals_dict_writeback: Option<(ObjRef, ObjRef)> = None;
        let mut globals_explicit = false;
        if let Some(value) = globals_arg
            && value != Value::None
        {
            globals_explicit = true;
            match value {
                Value::Module(module) => {
                    globals_module = module;
                }
                Value::Dict(dict) => {
                    let module = self.alloc_exec_namespace_module(
                        "<eval_globals>",
                        self.exec_namespace_map_from_dict(&dict, "globals")?,
                    );
                    globals_dict_writeback = Some((dict, module.clone()));
                    globals_module = module;
                }
                _ => {
                    return Err(RuntimeError::new("eval() globals must be a dict or module"));
                }
            }
        }

        let mut locals_module = globals_module.clone();
        let mut locals_dict_writeback: Option<(ObjRef, ObjRef)> = None;
        let mut caller_locals_writeback: Option<ObjRef> = None;
        if let Some(value) = locals_arg {
            if value != Value::None {
                match value {
                    Value::Module(module) => {
                        locals_module = module;
                    }
                    Value::Dict(dict) => {
                        if let Some((global_dict, global_module)) = &globals_dict_writeback {
                            if global_dict.id() == dict.id() {
                                locals_module = global_module.clone();
                            } else {
                                let module = self.alloc_exec_namespace_module(
                                    "<eval_locals>",
                                    self.exec_namespace_map_from_dict(&dict, "locals")?,
                                );
                                locals_dict_writeback = Some((dict, module.clone()));
                                locals_module = module;
                            }
                        } else {
                            let module = self.alloc_exec_namespace_module(
                                "<eval_locals>",
                                self.exec_namespace_map_from_dict(&dict, "locals")?,
                            );
                            locals_dict_writeback = Some((dict, module.clone()));
                            locals_module = module;
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("eval() locals must be a dict or module"));
                    }
                }
            } else if !globals_explicit {
                if caller_is_module {
                    locals_module = caller_module;
                } else {
                    let module =
                        self.alloc_exec_namespace_module("<eval_locals>", caller_locals.clone());
                    caller_locals_writeback = Some(module.clone());
                    locals_module = module;
                }
            }
        } else if !globals_explicit {
            if caller_is_module {
                locals_module = caller_module;
            } else {
                let module = self.alloc_exec_namespace_module("<eval_locals>", caller_locals);
                caller_locals_writeback = Some(module.clone());
                locals_module = module;
            }
        }

        let cells = self.build_cells(&code, Vec::new());
        let mut frame = Frame::new(code, locals_module.clone(), true, false, cells, None);
        frame.function_globals = globals_module.clone();
        if locals_module.id() != globals_module.id() {
            frame.globals_fallback = Some(globals_module.clone());
        }
        self.frames.push(Box::new(frame));

        let previous_stop = self.run_stop_depth;
        self.run_stop_depth = Some(caller_depth);
        let run_result = self.run();
        self.run_stop_depth = previous_stop;

        if let Some((dict, module)) = &globals_dict_writeback {
            self.sync_exec_namespace_to_dict(dict, module)?;
        }
        if let Some((dict, module)) = &locals_dict_writeback {
            self.sync_exec_namespace_to_dict(dict, module)?;
        }
        if let Some(module) = &caller_locals_writeback {
            let locals = self.exec_namespace_map_from_module(module)?;
            if let Some(frame) = self.frames.get_mut(caller_index) {
                frame.locals = locals;
            }
        }

        run_result?;
        let Some(caller_frame) = self.frames.get_mut(caller_index) else {
            return Err(RuntimeError::new("eval() caller frame unavailable"));
        };
        Ok(caller_frame.stack.pop().unwrap_or(Value::None))
    }

    pub(super) fn builtin_len(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if let Some(value) = kwargs.remove("obj") {
            if !args.is_empty() {
                return Err(RuntimeError::new("len() got multiple values"));
            }
            args.push(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "len() got an unexpected keyword argument",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new("len() expects one argument"));
        }
        if let Value::Instance(instance) = &args[0] {
            if let Some(values) = self.namedtuple_instance_values(instance) {
                return Ok(Value::Int(values.len() as i64));
            }
            if let Some(backing_list) = self.instance_backing_list(instance)
                && let Object::List(values) = &*backing_list.kind()
            {
                return Ok(Value::Int(values.len() as i64));
            }
            if let Some(backing_tuple) = self.instance_backing_tuple(instance)
                && let Object::Tuple(values) = &*backing_tuple.kind()
            {
                return Ok(Value::Int(values.len() as i64));
            }
            if let Some(backing_str) = self.instance_backing_str(instance) {
                return Ok(Value::Int(backing_str.chars().count() as i64));
            }
            if let Some(backing_dict) = self.instance_backing_dict(instance)
                && let Object::Dict(values) = &*backing_dict.kind()
            {
                return Ok(Value::Int(values.len() as i64));
            }
            if let Some(backing_set) = self.instance_backing_set(instance)
                && let Object::Set(values) = &*backing_set.kind()
            {
                return Ok(Value::Int(values.len() as i64));
            }
            if let Some(backing_frozenset) = self.instance_backing_frozenset(instance)
                && let Object::FrozenSet(values) = &*backing_frozenset.kind()
            {
                return Ok(Value::Int(values.len() as i64));
            }
        }
        if let Value::DictKeys(keys_view) = &args[0]
            && let Object::DictKeysView(view) = &*keys_view.kind()
            && let Object::Dict(values) = &*view.dict.kind()
        {
            return Ok(Value::Int(values.len() as i64));
        }
        if let Value::Iterator(iterator) = &args[0]
            && let Some((start, stop, step)) = self.range_object_parts(iterator)
        {
            return Ok(value_from_bigint(
                self.range_object_len_bigint(&start, &stop, &step),
            ));
        }
        let receiver = args
            .into_iter()
            .next()
            .ok_or_else(|| RuntimeError::new("len() expects one argument"))?;
        if let Some(proxy_result) = self.cpython_proxy_len(&receiver) {
            let raw = proxy_result?;
            return self.normalize_len_result(raw);
        }
        match BuiltinFunction::Len.call(&self.heap, vec![receiver.clone()]) {
            Ok(value) => Ok(value),
            Err(err) if err.message == "len() unsupported type" => {
                let Some(method) = self.lookup_bound_special_method(&receiver, "__len__")? else {
                    let type_name = self.value_type_name_for_error(&receiver);
                    return Err(RuntimeError::new(format!(
                        "TypeError: object of type '{type_name}' has no len()",
                    )));
                };
                let result = match self.call_internal(method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(self.runtime_error_from_active_exception("len() method raised"));
                    }
                };
                self.normalize_len_result(result)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn builtin_bool(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("bool() expects at most one argument"));
        }
        if args.is_empty() {
            return Ok(Value::Bool(false));
        }
        let value = args.remove(0);
        Ok(Value::Bool(self.truthy_from_value(&value)?))
    }

    pub(super) fn normalize_len_result(&self, result: Value) -> Result<Value, RuntimeError> {
        match result {
            Value::Bool(flag) => Ok(Value::Int(if flag { 1 } else { 0 })),
            Value::Int(number) => {
                if number < 0 {
                    Err(RuntimeError::new("__len__() should return >= 0"))
                } else {
                    Ok(Value::Int(number))
                }
            }
            Value::BigInt(number) => {
                if number.is_negative() {
                    return Err(RuntimeError::new("__len__() should return >= 0"));
                }
                let as_i64 = number
                    .to_i64()
                    .ok_or_else(|| RuntimeError::new("len() result does not fit in an index"))?;
                Ok(Value::Int(as_i64))
            }
            _ => Err(RuntimeError::new("__len__() should return an integer")),
        }
    }

    pub(super) fn builtin_collections_namedtuple_make(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("namedtuple._make() expects iterable"));
        }
        let class = match &args[0] {
            Value::Class(class) => class.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "namedtuple._make() requires class receiver",
                ));
            }
        };
        let Some(fields) = self.class_namedtuple_fields(&class) else {
            return Err(RuntimeError::new(
                "namedtuple._make() requires namedtuple class",
            ));
        };
        let values = self.collect_iterable_values(args[1].clone())?;
        if values.len() != fields.len() {
            return Err(RuntimeError::new(format!(
                "Expected {} arguments, got {}",
                fields.len(),
                values.len()
            )));
        }
        match self.call_internal(Value::Class(class), values, HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("namedtuple._make() failed"))
            }
        }
    }

    pub(super) fn builtin_collections_namedtuple(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let module_name = match kwargs.remove("module") {
            Some(Value::Str(name)) => Some(name),
            Some(Value::None) | None => None,
            Some(_) => return Err(RuntimeError::new("namedtuple() module must be string")),
        };
        let _rename = match kwargs.remove("rename") {
            Some(value) => self.truthy_from_value(&value)?,
            None => false,
        };
        let defaults = match kwargs.remove("defaults") {
            Some(Value::None) | None => Vec::new(),
            Some(value) => self.collect_iterable_values(value)?,
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "namedtuple() got an unexpected keyword argument",
            ));
        }

        let class_value = BuiltinFunction::CollectionsNamedTuple.call(&self.heap, args)?;
        let Value::Class(class) = class_value.clone() else {
            return Err(RuntimeError::new("namedtuple() internal class error"));
        };

        let Some(fields) = self.class_namedtuple_fields(&class) else {
            return Err(RuntimeError::new("namedtuple() internal field error"));
        };

        if defaults.len() > fields.len() {
            return Err(RuntimeError::new(
                "namedtuple() defaults length exceeds field count",
            ));
        }

        let defaults_tuple = self.heap.alloc_tuple(defaults.clone());
        let mut field_defaults = Vec::new();
        if !defaults.is_empty() {
            let first_default_index = fields.len() - defaults.len();
            for (offset, value) in defaults.into_iter().enumerate() {
                let field_name = fields[first_default_index + offset].clone();
                field_defaults.push((Value::Str(field_name), value));
            }
        }
        let field_defaults_dict = self.heap.alloc_dict(field_defaults);

        if let Object::Class(class_data) = &mut *class.kind_mut() {
            if let Some(module_name) = module_name {
                class_data
                    .attrs
                    .insert("__module__".to_string(), Value::Str(module_name));
            }
            class_data.attrs.insert(
                "__pyrs_namedtuple_defaults__".to_string(),
                defaults_tuple.clone(),
            );
            class_data
                .attrs
                .insert("_field_defaults".to_string(), field_defaults_dict);
            if !class_data.attrs.contains_key("__match_args__")
                && let Some(fields_value) = class_data.attrs.get("_fields").cloned()
            {
                class_data
                    .attrs
                    .insert("__match_args__".to_string(), fields_value);
            }
        }

        self.store_attr_builtin(
            BuiltinFunction::CollectionsNamedTupleNew,
            "__defaults__",
            defaults_tuple,
        )?;
        Ok(class_value)
    }

    pub(super) fn builtin_collections_namedtuple_new(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "namedtuple.__new__() requires class receiver",
            ));
        }
        let class = match args.remove(0) {
            Value::Class(class) => class,
            _ => {
                return Err(RuntimeError::new(
                    "namedtuple.__new__() requires class receiver",
                ));
            }
        };
        let Some(fields) = self.class_namedtuple_fields(&class) else {
            return Err(RuntimeError::new(
                "namedtuple.__new__() requires namedtuple class",
            ));
        };
        if args.len() > fields.len() {
            return Err(RuntimeError::new("namedtuple() argument count mismatch"));
        }
        let defaults = self.class_namedtuple_defaults(&class).unwrap_or_default();
        if defaults.len() > fields.len() {
            return Err(RuntimeError::new("namedtuple() argument count mismatch"));
        }
        let first_default = fields.len().saturating_sub(defaults.len());
        let mut bound_values: Vec<Option<Value>> = vec![None; fields.len()];
        for (index, value) in args.into_iter().enumerate() {
            bound_values[index] = Some(value);
        }
        for (name, value) in kwargs {
            let Some(index) = fields.iter().position(|field| field == &name) else {
                return Err(RuntimeError::new(
                    "namedtuple() got unexpected keyword argument",
                ));
            };
            if bound_values[index].is_some() {
                return Err(RuntimeError::new(
                    "namedtuple() got multiple values for field",
                ));
            }
            bound_values[index] = Some(value);
        }
        for index in 0..fields.len() {
            if bound_values[index].is_none() && index >= first_default {
                bound_values[index] = Some(defaults[index - first_default].clone());
            }
        }
        let Some(final_values) = bound_values.into_iter().collect::<Option<Vec<_>>>() else {
            return Err(RuntimeError::new("namedtuple() argument count mismatch"));
        };
        let instance = self.alloc_instance_for_class(&class);
        self.bind_namedtuple_instance_fields(&instance, &fields, final_values, HashMap::new())?;
        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_dir(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("dir() expects at most one argument"));
        }

        let mut names: Vec<String> = Vec::new();
        if let Some(target) = args.first() {
            match target {
                Value::Module(module) => {
                    if let Object::Module(module_data) = &*module.kind() {
                        names.extend(module_data.globals.keys().cloned());
                    }
                }
                Value::Class(class) => {
                    for entry in class_attr_walk(class) {
                        if let Object::Class(class_data) = &*entry.kind() {
                            names.extend(class_data.attrs.keys().cloned());
                        }
                    }
                }
                Value::Instance(instance) => {
                    if let Object::Instance(instance_data) = &*instance.kind() {
                        names.extend(instance_data.attrs.keys().cloned());
                        for entry in class_attr_walk(&instance_data.class) {
                            if let Object::Class(class_data) = &*entry.kind() {
                                names.extend(class_data.attrs.keys().cloned());
                            }
                        }
                    }
                }
                Value::Dict(dict) => {
                    if let Object::Dict(entries) = &*dict.kind() {
                        for (key, _) in entries {
                            if let Value::Str(name) = key {
                                names.push(name.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        } else {
            let frame = self
                .frames
                .last()
                .ok_or_else(|| RuntimeError::new("no frame"))?;
            if frame.is_module {
                if let Object::Module(module_data) = &*frame.module.kind() {
                    names.extend(module_data.globals.keys().cloned());
                }
            } else {
                names.extend(frame.locals.keys().cloned());
                for (idx, slot) in frame.fast_locals.iter().enumerate() {
                    if slot.is_some()
                        && let Some(name) = frame.code.names.get(idx)
                    {
                        names.push(name.clone());
                    }
                }
            }
        }

        names.sort();
        names.dedup();
        Ok(self
            .heap
            .alloc_list(names.into_iter().map(Value::Str).collect::<Vec<_>>()))
    }

    pub(super) fn builtin_sys_getframe(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new(
                "sys._getframe() expects at most one argument",
            ));
        }
        let depth = if args.is_empty() {
            0
        } else {
            value_to_int(args.remove(0))?
        };
        if depth < 0 {
            return Err(RuntimeError::new("call stack is not deep enough"));
        }
        let depth = depth as usize;
        if depth >= self.frames.len() {
            return Err(RuntimeError::new("call stack is not deep enough"));
        }
        let frame_index = self.frames.len() - 1 - depth;
        Ok(self.build_frame_proxy_value(frame_index))
    }

    pub(super) fn build_frame_proxy_value(&self, frame_index: usize) -> Value {
        let frame = &self.frames[frame_index];
        let locals_dict = if frame.is_module {
            if let Object::Module(module_data) = &*frame.module.kind() {
                let mut entries = Vec::with_capacity(module_data.globals.len());
                for (name, value) in module_data.globals.iter() {
                    entries.push((Value::Str(name.clone()), value.clone()));
                }
                self.heap.alloc_dict(entries)
            } else {
                self.heap.alloc_dict(Vec::new())
            }
        } else {
            // Keep f_locals lightweight and avoid retaining stale strong refs.
            // We expose the local names but not copied local values.
            let mut names = HashSet::new();
            names.extend(frame.locals.keys().cloned());
            for (idx, slot) in frame.fast_locals.iter().enumerate() {
                if slot.is_some()
                    && let Some(name) = frame.code.names.get(idx)
                {
                    names.insert(name.clone());
                }
            }
            names.extend(frame.code.cellvars.iter().cloned());
            names.extend(frame.code.freevars.iter().cloned());
            let mut entries = Vec::with_capacity(names.len());
            for name in names {
                entries.push((Value::Str(name), Value::None));
            }
            self.heap.alloc_dict(entries)
        };

        let globals_dict = if let Object::Module(module_data) = &*frame.function_globals.kind() {
            let mut entries = Vec::with_capacity(module_data.globals.len());
            for (name, value) in module_data.globals.iter() {
                entries.push((Value::Str(name.clone()), value.clone()));
            }
            self.heap.alloc_dict(entries)
        } else {
            self.heap.alloc_dict(Vec::new())
        };
        let location = frame.code.locations.get(frame.last_ip);
        let lineno = location.map(|loc| loc.line).unwrap_or(0);
        let f_back = if frame_index > 0 {
            self.build_frame_proxy_value(frame_index - 1)
        } else {
            Value::None
        };

        let frame_obj = match self
            .heap
            .alloc_module(ModuleObject::new("<frame>".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *frame_obj.kind_mut() {
            module_data
                .globals
                .insert("f_locals".to_string(), locals_dict);
            module_data
                .globals
                .insert("f_globals".to_string(), globals_dict);
            module_data
                .globals
                .insert("f_code".to_string(), Value::Code(frame.code.clone()));
            module_data
                .globals
                .insert("f_lineno".to_string(), Value::Int(lineno as i64));
            module_data.globals.insert("f_back".to_string(), f_back);
        }
        Value::Module(frame_obj)
    }

    pub(super) fn builtin_sys_exception(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("sys.exception() expects no arguments"));
        }
        for frame in self.frames.iter().rev() {
            if let Some(exc) = frame.active_exception.clone() {
                return Ok(exc);
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_exc_info(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("sys.exc_info() expects no arguments"));
        }
        for frame in self.frames.iter().rev() {
            if let Some(exc) = frame.active_exception.clone() {
                let exc_type = match &exc {
                    Value::Exception(exception) => Value::ExceptionType(exception.name.clone()),
                    Value::ExceptionType(name) => Value::ExceptionType(name.clone()),
                    _ => Value::None,
                };
                return Ok(self.heap.alloc_tuple(vec![exc_type, exc, Value::None]));
            }
        }
        Ok(self
            .heap
            .alloc_tuple(vec![Value::None, Value::None, Value::None]))
    }

    pub(super) fn builtin_sys_exit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("sys.exit() expects at most one argument"));
        }
        if let Some(value) = args.pop() {
            Err(RuntimeError::new(format!(
                "SystemExit: {}",
                format_value(&value)
            )))
        } else {
            Err(RuntimeError::new("SystemExit"))
        }
    }

    pub(super) fn builtin_sys_is_finalizing(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "sys.is_finalizing() expects no arguments",
            ));
        }
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_sys_getdefaultencoding(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "sys.getdefaultencoding() expects no arguments",
            ));
        }
        Ok(Value::Str("utf-8".to_string()))
    }

    pub(super) fn builtin_sys_getfilesystemencoding(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "sys.getfilesystemencoding() expects no arguments",
            ));
        }
        Ok(Value::Str("utf-8".to_string()))
    }

    pub(super) fn builtin_sys_getfilesystemencodeerrors(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "sys.getfilesystemencodeerrors() expects no arguments",
            ));
        }
        Ok(Value::Str("surrogateescape".to_string()))
    }

    pub(super) fn builtin_sys_getrefcount(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("sys.getrefcount() expects one argument"));
        }
        let count = match &args[0] {
            Value::List(obj)
            | Value::Tuple(obj)
            | Value::Dict(obj)
            | Value::DictKeys(obj)
            | Value::Set(obj)
            | Value::FrozenSet(obj)
            | Value::Bytes(obj)
            | Value::ByteArray(obj)
            | Value::MemoryView(obj)
            | Value::Iterator(obj)
            | Value::Generator(obj)
            | Value::Module(obj)
            | Value::Class(obj)
            | Value::Instance(obj)
            | Value::Super(obj)
            | Value::BoundMethod(obj)
            | Value::Function(obj)
            | Value::Cell(obj) => obj.strong_count() as i64 + 1,
            _ => 1,
        };
        Ok(Value::Int(count))
    }

    pub(super) fn builtin_sys_getrecursionlimit(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "sys.getrecursionlimit() expects no arguments",
            ));
        }
        Ok(Value::Int(self.recursion_limit))
    }

    pub(super) fn builtin_sys_setrecursionlimit(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "sys.setrecursionlimit() expects one argument",
            ));
        }
        let limit = value_to_int(args[0].clone())?;
        if limit < 1 {
            return Err(RuntimeError::new("recursion limit must be greater than 0"));
        }
        self.recursion_limit = limit;
        Ok(Value::None)
    }

    pub(super) fn builtin_memoryview(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("memoryview() expects one argument"));
        }
        let source = match args.remove(0) {
            Value::Bytes(obj) | Value::ByteArray(obj) => obj,
            Value::MemoryView(obj) => match &*obj.kind() {
                Object::MemoryView(view_data) => view_data.source.clone(),
                _ => return Err(RuntimeError::new("memoryview() expects bytes-like object")),
            },
            Value::Module(obj) => {
                let is_array = matches!(&*obj.kind(), Object::Module(module_data) if module_data.name == "__array__");
                if is_array {
                    obj
                } else {
                    return Err(RuntimeError::new("memoryview() expects bytes-like object"));
                }
            }
            Value::Instance(obj) => {
                {
                    let kind = obj.kind();
                    let Object::Instance(instance_data) = &*kind else {
                        return Err(RuntimeError::new("memoryview() expects bytes-like object"));
                    };
                    let is_picklebuffer = matches!(
                        &*instance_data.class.kind(),
                        Object::Class(class_data) if class_data.name == "PickleBuffer"
                    );
                    if is_picklebuffer {
                        if matches!(
                            instance_data.attrs.get("__pyrs_picklebuffer_released__"),
                            Some(Value::Bool(true))
                        ) {
                            return Err(RuntimeError::new(
                                "ValueError: operation forbidden on released PickleBuffer object",
                            ));
                        }
                        let source = instance_data
                            .attrs
                            .get("__pyrs_picklebuffer_source__")
                            .or_else(|| instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR))
                            .cloned()
                            .ok_or_else(|| {
                                RuntimeError::new("memoryview() expects bytes-like object")
                            })?;
                        let source = match source {
                            Value::Bytes(source)
                            | Value::ByteArray(source)
                            | Value::Instance(source) => source,
                            Value::MemoryView(view) => match &*view.kind() {
                                Object::MemoryView(view_data) => view_data.source.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "memoryview() expects bytes-like object",
                                    ));
                                }
                            },
                            _ => {
                                return Err(RuntimeError::new(
                                    "memoryview() expects bytes-like object",
                                ));
                            }
                        };
                        return Ok(self.heap.alloc_memoryview(source));
                    }
                }

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
                    match buffer_value {
                        Value::MemoryView(view_obj) => match &*view_obj.kind() {
                            Object::MemoryView(view_data) => view_data.source.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "memoryview() expects bytes-like object",
                                ));
                            }
                        },
                        Value::Bytes(obj) | Value::ByteArray(obj) => obj,
                        Value::Module(obj) => {
                            let is_array = matches!(
                                &*obj.kind(),
                                Object::Module(module_data) if module_data.name == "__array__"
                            );
                            if is_array {
                                obj
                            } else {
                                return Err(RuntimeError::new(
                                    "memoryview() expects bytes-like object",
                                ));
                            }
                        }
                        other => {
                            let payload = self.value_to_bytes_payload(other)?;
                            match self.heap.alloc_bytearray(payload) {
                                Value::ByteArray(obj) => obj,
                                _ => unreachable!(),
                            }
                        }
                    }
                } else if matches!(
                    &*obj.kind(),
                    Object::Instance(instance_data)
                        if instance_data.attrs.contains_key("__pyrs_bytes_storage__")
                ) {
                    obj
                } else {
                    return Err(RuntimeError::new("memoryview() expects bytes-like object"));
                }
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "memoryview() expects bytes-like object, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        Ok(self.heap.alloc_memoryview(source))
    }

    pub(super) fn builtin_sys_stream_write(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        stderr: bool,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("write() expects one argument"));
        }
        let text = format_value(&args[0]);
        if stderr {
            eprint!("{text}");
        } else {
            print!("{text}");
        }
        Ok(Value::Int(text.chars().count() as i64))
    }

    pub(super) fn builtin_sys_stream_buffer_write(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        stderr: bool,
    ) -> Result<Value, RuntimeError> {
        use std::io::Write;
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("write() expects one argument"));
        }
        let payload = bytes_like_from_value(args[0].clone())?;
        if stderr {
            std::io::stderr()
                .write_all(&payload)
                .map_err(|err| RuntimeError::new(format!("write failed: {err}")))?;
        } else {
            std::io::stdout()
                .write_all(&payload)
                .map_err(|err| RuntimeError::new(format!("write failed: {err}")))?;
        }
        Ok(Value::Int(payload.len() as i64))
    }

    pub(super) fn builtin_sys_stream_flush(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("flush() expects no arguments"));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_stdin_write(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("write() expects one argument"));
        }
        Err(RuntimeError::new("stdin is read-only"))
    }

    pub(super) fn builtin_sys_stream_isatty(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("isatty() expects no arguments"));
        }
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_float_fromhex(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("float.fromhex() expects one argument"));
        }
        let text = match args.remove(0) {
            Value::Str(text) => text,
            _ => return Err(RuntimeError::new("fromhex() argument must be str")),
        };
        let parsed = parse_hex_float_literal(&text)?;
        Ok(Value::Float(parsed))
    }

    pub(super) fn builtin_float_hex(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("float.hex() expects one argument"));
        }
        let value = value_to_f64(args.remove(0))?;
        Ok(Value::Str(format_float_hex(value)))
    }

    pub(super) fn builtin_str_maketrans(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new("str.maketrans() expects 1-3 arguments"));
        }

        let mapping = match args.len() {
            1 => match args.remove(0) {
                Value::Dict(dict) => match &*dict.kind() {
                    Object::Dict(entries) => {
                        let mut out = Vec::new();
                        for (key, value) in entries {
                            let key = match key {
                                Value::Int(_) => key.clone(),
                                Value::Str(text) => {
                                    let mut chars = text.chars();
                                    let Some(ch) = chars.next() else {
                                        return Err(RuntimeError::new(
                                            "maketrans() keys must be length 1 strings",
                                        ));
                                    };
                                    if chars.next().is_some() {
                                        return Err(RuntimeError::new(
                                            "maketrans() keys must be length 1 strings",
                                        ));
                                    }
                                    Value::Int(ch as i64)
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "maketrans() keys must be integers or strings",
                                    ));
                                }
                            };
                            out.push((key, value.clone()));
                        }
                        self.heap.alloc_dict(out)
                    }
                    _ => return Err(RuntimeError::new("maketrans() expects a dict")),
                },
                _ => return Err(RuntimeError::new("maketrans() expects a dict")),
            },
            2 | 3 => {
                let from = match args.remove(0) {
                    Value::Str(text) => text,
                    _ => return Err(RuntimeError::new("first maketrans arg must be str")),
                };
                let to = match args.remove(0) {
                    Value::Str(text) => text,
                    _ => return Err(RuntimeError::new("second maketrans arg must be str")),
                };
                if from.chars().count() != to.chars().count() {
                    return Err(RuntimeError::new(
                        "first two maketrans arguments must have equal length",
                    ));
                }
                let mut out = Vec::new();
                for (src, dst) in from.chars().zip(to.chars()) {
                    out.push((Value::Int(src as i64), Value::Int(dst as i64)));
                }
                if args.len() == 1 {
                    let deletions = match args.remove(0) {
                        Value::Str(text) => text,
                        _ => return Err(RuntimeError::new("third maketrans arg must be str")),
                    };
                    for ch in deletions.chars() {
                        out.push((Value::Int(ch as i64), Value::None));
                    }
                }
                self.heap.alloc_dict(out)
            }
            _ => unreachable!(),
        };

        Ok(mapping)
    }

    pub(super) fn builtin_int(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if let Some(Value::Class(class)) = args.first()
            && self.class_has_builtin_int_base(class)
        {
            let mut class = class.clone();
            args.remove(0);
            if let Some(Value::Class(explicit_class)) = args.first()
                && self.class_has_builtin_int_base(explicit_class)
            {
                class = explicit_class.clone();
                args.remove(0);
            }
            let int_value = if matches!(
                args.first(),
                Some(Value::Class(candidate)) if self.class_has_builtin_int_base(candidate)
            ) {
                if kwargs.is_empty() {
                    BuiltinFunction::Int.call(&self.heap, args)?
                } else {
                    call_builtin_with_kwargs(&self.heap, BuiltinFunction::Int, args, kwargs)?
                }
            } else {
                self.builtin_int(args, kwargs)?
            };
            let instance = self.alloc_instance_for_class(&class);
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(INT_BACKING_STORAGE_ATTR.to_string(), int_value);
            }
            return Ok(Value::Instance(instance));
        }
        if kwargs.is_empty() {
            if args.len() == 1 {
                let arg = args[0].clone();
                if let Value::Instance(instance) = &arg
                    && let Some(backing) = self.instance_backing_int(instance)
                {
                    return BuiltinFunction::Int.call(&self.heap, vec![backing]);
                }
                match BuiltinFunction::Int.call(&self.heap, args) {
                    Ok(value) => return Ok(value),
                    Err(err) if err.message == "int() unsupported type" => {
                        if let Some(method) = self.lookup_bound_special_method(&arg, "__int__")? {
                            return match self.call_internal(method, Vec::new(), HashMap::new())? {
                                InternalCallOutcome::Value(value) => match value {
                                    Value::Int(_) | Value::BigInt(_) | Value::Bool(_) => {
                                        BuiltinFunction::Int.call(&self.heap, vec![value])
                                    }
                                    _ => Err(RuntimeError::new("__int__ returned non-int")),
                                },
                                InternalCallOutcome::CallerExceptionHandled => {
                                    Err(RuntimeError::new("int() unsupported type"))
                                }
                            };
                        }
                        if let Some(proxy_result) = self.cpython_proxy_long(&arg) {
                            let proxy_value = proxy_result?;
                            return match proxy_value {
                                Value::Int(_) | Value::BigInt(_) | Value::Bool(_) => {
                                    BuiltinFunction::Int.call(&self.heap, vec![proxy_value])
                                }
                                _ => Err(RuntimeError::new("__int__ returned non-int")),
                            };
                        }
                        return Err(err);
                    }
                    Err(err) => return Err(err),
                }
            }
            return BuiltinFunction::Int.call(&self.heap, args);
        }

        call_builtin_with_kwargs(&self.heap, BuiltinFunction::Int, args, kwargs)
    }

    fn coerce_index_bigint_for_range(&mut self, value: Value) -> Result<BigInt, RuntimeError> {
        match value {
            Value::Int(_) | Value::Bool(_) | Value::BigInt(_) => value_to_bigint(value),
            other => {
                if let Some(index_method) = self.lookup_bound_special_method(&other, "__index__")? {
                    let index_value =
                        match self.call_internal(index_method, Vec::new(), HashMap::new())? {
                            InternalCallOutcome::Value(value) => value,
                            InternalCallOutcome::CallerExceptionHandled => {
                                return Err(self.runtime_error_from_active_exception(
                                    "range() index conversion failed",
                                ));
                            }
                        };
                    if matches!(
                        index_value,
                        Value::Int(_) | Value::Bool(_) | Value::BigInt(_)
                    ) {
                        return value_to_bigint(index_value);
                    }
                }
                if let Some(proxy_index) = self.cpython_proxy_long(&other)
                    && let Ok(index_value) = proxy_index
                    && matches!(
                        index_value,
                        Value::Int(_) | Value::Bool(_) | Value::BigInt(_)
                    )
                {
                    return value_to_bigint(index_value);
                }
                Err(RuntimeError::new("range() expects integers"))
            }
        }
    }

    pub(super) fn builtin_range(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let mut start = kwargs.remove("start");
        let mut stop = kwargs.remove("stop");
        let mut step = kwargs.remove("step");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "range() got an unexpected keyword argument",
            ));
        }
        match args.len() {
            0 => {}
            1 => {
                if stop.is_some() {
                    return Err(RuntimeError::new("range() got multiple values"));
                }
                stop = Some(args.remove(0));
            }
            2 => {
                if start.is_some() || stop.is_some() {
                    return Err(RuntimeError::new("range() got multiple values"));
                }
                start = Some(args.remove(0));
                stop = Some(args.remove(0));
            }
            3 => {
                if start.is_some() || stop.is_some() || step.is_some() {
                    return Err(RuntimeError::new("range() got multiple values"));
                }
                start = Some(args.remove(0));
                stop = Some(args.remove(0));
                step = Some(args.remove(0));
            }
            _ => return Err(RuntimeError::new("range() expects 1-3 arguments")),
        }

        let stop = stop.ok_or_else(|| RuntimeError::new("range() missing stop"))?;
        let start = start.unwrap_or(Value::Int(0));
        let step = step.unwrap_or(Value::Int(1));

        let start_big = self.coerce_index_bigint_for_range(start)?;
        let stop_big = self.coerce_index_bigint_for_range(stop)?;
        let step_big = self.coerce_index_bigint_for_range(step)?;
        if step_big.is_zero() {
            return Err(RuntimeError::new("range() step cannot be zero"));
        }

        Ok(Value::Iterator(self.heap.alloc(Object::Iterator(
            IteratorObject {
                kind: IteratorKind::RangeObject {
                    start: start_big,
                    stop: stop_big,
                    step: step_big,
                },
                index: 0,
            },
        ))))
    }

    pub(super) fn builtin_float(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if let Some(Value::Class(class)) = args.first()
            && self.class_has_builtin_float_base(class)
        {
            let mut class = class.clone();
            args.remove(0);
            if let Some(Value::Class(explicit_class)) = args.first()
                && self.class_has_builtin_float_base(explicit_class)
            {
                class = explicit_class.clone();
                args.remove(0);
            }
            let float_value = if matches!(
                args.first(),
                Some(Value::Class(candidate)) if self.class_has_builtin_float_base(candidate)
            ) {
                if kwargs.is_empty() {
                    BuiltinFunction::Float.call(&self.heap, args)?
                } else {
                    call_builtin_with_kwargs(&self.heap, BuiltinFunction::Float, args, kwargs)?
                }
            } else {
                self.builtin_float(args, kwargs)?
            };
            let instance = self.alloc_instance_for_class(&class);
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(FLOAT_BACKING_STORAGE_ATTR.to_string(), float_value);
            }
            return Ok(Value::Instance(instance));
        }
        if kwargs.is_empty() && args.len() == 1 {
            if let Value::Instance(instance) = &args[0]
                && let Some(backing) = self.instance_backing_float(instance)
            {
                return Ok(Value::Float(backing));
            }
            if let Some(proxy_result) = self.cpython_proxy_float(&args[0]) {
                return proxy_result;
            }
            if let Some(float_method) = self.lookup_bound_special_method(&args[0], "__float__")? {
                return match self.call_internal(float_method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(Value::Float(value)) => Ok(Value::Float(value)),
                    InternalCallOutcome::Value(_) => {
                        Err(RuntimeError::new("__float__ returned non-float"))
                    }
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(self.runtime_error_from_active_exception("float() failed"))
                    }
                };
            }
            return BuiltinFunction::Float.call(&self.heap, args);
        }
        if kwargs.is_empty() {
            return BuiltinFunction::Float.call(&self.heap, args);
        }
        call_builtin_with_kwargs(&self.heap, BuiltinFunction::Float, args, kwargs)
    }

    pub(super) fn builtin_complex(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if let Some(Value::Class(class)) = args.first()
            && self.class_has_builtin_complex_base(class)
        {
            let class = class.clone();
            args.remove(0);
            let complex_value = if kwargs.is_empty() {
                BuiltinFunction::Complex.call(&self.heap, args)?
            } else {
                call_builtin_with_kwargs(&self.heap, BuiltinFunction::Complex, args, kwargs)?
            };
            let instance = self.alloc_instance_for_class(&class);
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(COMPLEX_BACKING_STORAGE_ATTR.to_string(), complex_value);
            }
            return Ok(Value::Instance(instance));
        }
        if kwargs.is_empty() && args.len() == 1 {
            if let Value::Instance(instance) = &args[0]
                && let Some((real, imag)) = self.instance_backing_complex(instance)
            {
                return Ok(Value::Complex { real, imag });
            }
            return BuiltinFunction::Complex.call(&self.heap, args);
        }
        if kwargs.is_empty() {
            return BuiltinFunction::Complex.call(&self.heap, args);
        }
        call_builtin_with_kwargs(&self.heap, BuiltinFunction::Complex, args, kwargs)
    }

    pub(super) fn builtin_str(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let mut object = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut encoding = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut errors = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if !args.is_empty() {
            return Err(RuntimeError::new("str() expects at most three arguments"));
        }

        if let Some(value) = kwargs.remove("object") {
            if object.is_some() {
                return Err(RuntimeError::new(
                    "str() got multiple values for argument 'object'",
                ));
            }
            object = Some(value);
        }
        if let Some(value) = kwargs.remove("encoding") {
            if encoding.is_some() {
                return Err(RuntimeError::new(
                    "str() got multiple values for argument 'encoding'",
                ));
            }
            encoding = Some(value);
        }
        if let Some(value) = kwargs.remove("errors") {
            if errors.is_some() {
                return Err(RuntimeError::new(
                    "str() got multiple values for argument 'errors'",
                ));
            }
            errors = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "str() got an unexpected keyword argument",
            ));
        }

        let object = object.unwrap_or_else(|| Value::Str(String::new()));
        if encoding.is_none() && errors.is_none() {
            let is_proxy = Self::cpython_proxy_raw_ptr_from_value(&object).is_some();
            if is_proxy {
                match self.cpython_proxy_str(&object) {
                    Some(Ok(text)) => return Ok(Value::Str(text)),
                    Some(Err(_)) | None => return Ok(Value::Str(format_value(&object))),
                }
            }
            if matches!(object, Value::Builtin(_)) {
                return Ok(Value::Str(format_value(&object)));
            }
            if let Value::Instance(instance) = &object
                && let Some(backing) = self.instance_backing_str(instance)
            {
                return Ok(Value::Str(backing));
            }
            if matches!(object, Value::Class(_)) {
                return Ok(Value::Str(format_value(&object)));
            }
            if !matches!(object, Value::Str(_)) {
                let str_method = self.builtin_getattr(
                    vec![object.clone(), Value::Str("__str__".to_string())],
                    HashMap::new(),
                );
                match str_method {
                    Ok(str_method) => {
                        let is_recursive_builtin_str = match &str_method {
                            Value::BoundMethod(bound) => match &*bound.kind() {
                                Object::BoundMethod(bound_data) => {
                                    match &*bound_data.function.kind() {
                                        Object::NativeMethod(native) => matches!(
                                            native.kind,
                                            NativeMethodKind::Builtin(BuiltinFunction::Str)
                                        ),
                                        _ => false,
                                    }
                                }
                                _ => false,
                            },
                            Value::Builtin(BuiltinFunction::Str) => true,
                            _ => false,
                        };
                        if !is_recursive_builtin_str {
                            match self.call_internal(str_method, Vec::new(), HashMap::new())? {
                                InternalCallOutcome::Value(Value::Str(text)) => {
                                    return Ok(Value::Str(text));
                                }
                                InternalCallOutcome::Value(_) => {
                                    return Err(RuntimeError::new("__str__ returned non-string"));
                                }
                                InternalCallOutcome::CallerExceptionHandled => {
                                    return Err(
                                        self.runtime_error_from_active_exception("str() failed")
                                    );
                                }
                            }
                        }
                    }
                    Err(err) => {
                        if !runtime_error_matches_exception(&err, "AttributeError") {
                            return Err(err);
                        }
                    }
                }
            }
            return Ok(Value::Str(format_value(&object)));
        }

        let encoding =
            normalize_codec_encoding(encoding.unwrap_or(Value::Str("utf-8".to_string())))?;
        let errors = normalize_codec_errors(errors.unwrap_or(Value::Str("strict".to_string())))?;
        match object {
            Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                let bytes = bytes_like_from_value(object)?;
                let decoded = decode_text_bytes(&bytes, &encoding, &errors)?;
                Ok(Value::Str(decoded))
            }
            _ => Err(RuntimeError::new("decoding str is not supported")),
        }
    }

    fn parse_bytes_constructor_args(
        &self,
        constructor_name: &str,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<(Option<Value>, Option<Value>, Option<Value>), RuntimeError> {
        let mut object = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut encoding = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut errors = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if !args.is_empty() {
            return Err(RuntimeError::new(format!(
                "{constructor_name}() expects at most three arguments"
            )));
        }
        if let Some(value) = kwargs.remove("source") {
            if object.is_some() {
                return Err(RuntimeError::new(format!(
                    "{constructor_name}() got multiple values for argument 'source'"
                )));
            }
            object = Some(value);
        }
        if let Some(value) = kwargs.remove("encoding") {
            if encoding.is_some() {
                return Err(RuntimeError::new(format!(
                    "{constructor_name}() got multiple values for argument 'encoding'"
                )));
            }
            encoding = Some(value);
        }
        if let Some(value) = kwargs.remove("errors") {
            if errors.is_some() {
                return Err(RuntimeError::new(format!(
                    "{constructor_name}() got multiple values for argument 'errors'"
                )));
            }
            errors = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(format!(
                "{constructor_name}() got an unexpected keyword argument"
            )));
        }
        Ok((object, encoding, errors))
    }

    fn bytes_payload_from_constructor_parts(
        &mut self,
        constructor_name: &str,
        object: Option<Value>,
        encoding: Option<Value>,
        errors: Option<Value>,
    ) -> Result<Vec<u8>, RuntimeError> {
        if object.is_none() {
            if encoding.is_some() || errors.is_some() {
                return Err(RuntimeError::new(format!(
                    "{constructor_name}() argument 'encoding' without a string argument"
                )));
            }
            return Ok(Vec::new());
        }
        let object = object.unwrap_or(Value::None);

        if encoding.is_some() || errors.is_some() {
            let Value::Str(text) = object else {
                return Err(RuntimeError::new(format!(
                    "{constructor_name}() argument 'encoding' without a string argument"
                )));
            };
            let encoding =
                normalize_codec_encoding(encoding.unwrap_or(Value::Str("utf-8".to_string())))?;
            let errors =
                normalize_codec_errors(errors.unwrap_or(Value::Str("strict".to_string())))?;
            return encode_text_bytes(&text, &encoding, &errors);
        }

        match object {
            Value::Int(count) => {
                if count < 0 {
                    return Err(RuntimeError::new("negative count"));
                }
                Ok(vec![0; count as usize])
            }
            Value::Str(_) => Err(RuntimeError::new("string argument without an encoding")),
            Value::None => Err(RuntimeError::new(format!(
                "cannot convert '{}' object to bytes",
                self.value_type_name_for_error(&Value::None)
            ))),
            value => match self.value_to_bytes_payload(value.clone()) {
                Ok(payload) => Ok(payload),
                Err(_) => {
                    let mut out = Vec::new();
                    for item in self.collect_iterable_values(value)? {
                        let byte = value_to_int(item)?;
                        if !(0..=255).contains(&byte) {
                            return Err(RuntimeError::new("byte must be in range(0, 256)"));
                        }
                        out.push(byte as u8);
                    }
                    Ok(out)
                }
            },
        }
    }

    pub(super) fn builtin_bytes_constructor(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (object, encoding, errors) =
            self.parse_bytes_constructor_args("bytes", args, kwargs)?;
        let payload =
            self.bytes_payload_from_constructor_parts("bytes", object, encoding, errors)?;
        Ok(self.heap.alloc_bytes(payload))
    }

    pub(super) fn builtin_bytearray_constructor(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (object, encoding, errors) =
            self.parse_bytes_constructor_args("bytearray", args, kwargs)?;
        let payload =
            self.bytes_payload_from_constructor_parts("bytearray", object, encoding, errors)?;
        Ok(self.heap.alloc_bytearray(payload))
    }

    pub(super) fn builtin_bytes_maketrans(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("bytes.maketrans() expects two arguments"));
        }
        let from = bytes_like_from_value(args.remove(0))?;
        let to = bytes_like_from_value(args.remove(0))?;
        if from.len() != to.len() {
            return Err(RuntimeError::new(
                "first two maketrans arguments must have equal length",
            ));
        }
        let mut table = (0u16..=255).map(|value| value as u8).collect::<Vec<_>>();
        for (src, dst) in from.into_iter().zip(to.into_iter()) {
            table[src as usize] = dst;
        }
        Ok(self.heap.alloc_bytes(table))
    }

    pub(super) fn builtin_int_from_bytes(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 3 {
            return Err(RuntimeError::new(
                "int.from_bytes() expects bytes, byteorder, optional signed",
            ));
        }
        let bytes_kw = kwargs.remove("bytes");
        let byteorder_kw = kwargs.remove("byteorder");
        let signed_kw = kwargs.remove("signed");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "int.from_bytes() got an unexpected keyword argument",
            ));
        }
        if !args.is_empty() && bytes_kw.is_some() {
            return Err(RuntimeError::new(
                "int.from_bytes() got multiple values for bytes",
            ));
        }
        if args.len() > 1 && byteorder_kw.is_some() {
            return Err(RuntimeError::new(
                "int.from_bytes() got multiple values for byteorder",
            ));
        }
        if args.len() > 2 && signed_kw.is_some() {
            return Err(RuntimeError::new(
                "int.from_bytes() got multiple values for signed",
            ));
        }

        let bytes_arg = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = bytes_kw {
            value
        } else {
            return Err(RuntimeError::new("int.from_bytes() missing bytes argument"));
        };
        let byteorder_arg = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = byteorder_kw {
            value
        } else {
            Value::Str("big".to_string())
        };
        let signed_arg = if !args.is_empty() {
            args.remove(0)
        } else {
            signed_kw.unwrap_or(Value::Bool(false))
        };

        if matches!(bytes_arg, Value::Str(_)) {
            return Err(RuntimeError::new("expected bytes-like value"));
        }
        let bytes = self.value_to_bytes_payload(bytes_arg)?;
        let byteorder = match byteorder_arg {
            Value::Str(value) if value == "little" || value == "big" => value,
            _ => {
                return Err(RuntimeError::new(
                    "byteorder must be either 'little' or 'big'",
                ));
            }
        };
        let signed = self.truthy_from_value(&signed_arg)?;
        let value = bigint_from_bytes(&bytes, byteorder == "little", signed);
        Ok(value_from_bigint(value))
    }

    pub(super) fn builtin_compile(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 6 {
            return Err(RuntimeError::new("compile() expected at most 6 arguments"));
        }
        let source_kw = kwargs.remove("source");
        let filename_kw = kwargs.remove("filename");
        let mode_kw = kwargs.remove("mode");
        let flags_kw = kwargs.remove("flags");
        let dont_inherit_kw = kwargs.remove("dont_inherit");
        let optimize_kw = kwargs.remove("optimize");
        let feature_kw = kwargs.remove("_feature_version");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "compile() got an unexpected keyword argument",
            ));
        }

        if !args.is_empty() && source_kw.is_some() {
            return Err(RuntimeError::new(
                "compile() got multiple values for source",
            ));
        }
        if args.len() > 1 && filename_kw.is_some() {
            return Err(RuntimeError::new(
                "compile() got multiple values for filename",
            ));
        }
        if args.len() > 2 && mode_kw.is_some() {
            return Err(RuntimeError::new("compile() got multiple values for mode"));
        }
        if args.len() > 3 && flags_kw.is_some() {
            return Err(RuntimeError::new("compile() got multiple values for flags"));
        }
        if args.len() > 4 && dont_inherit_kw.is_some() {
            return Err(RuntimeError::new(
                "compile() got multiple values for dont_inherit",
            ));
        }
        if args.len() > 5 && optimize_kw.is_some() {
            return Err(RuntimeError::new(
                "compile() got multiple values for optimize",
            ));
        }

        let source_arg = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = source_kw {
            value
        } else {
            return Err(RuntimeError::new("compile() missing source argument"));
        };
        let filename_arg = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = filename_kw {
            value
        } else {
            return Err(RuntimeError::new("compile() missing filename argument"));
        };
        let mode_arg = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = mode_kw {
            value
        } else {
            return Err(RuntimeError::new("compile() missing mode argument"));
        };

        let flags_value = if !args.is_empty() {
            args.remove(0)
        } else {
            flags_kw.unwrap_or(Value::Int(0))
        };
        let dont_inherit_value = if !args.is_empty() {
            args.remove(0)
        } else {
            dont_inherit_kw.unwrap_or(Value::Bool(false))
        };
        let optimize_value = if !args.is_empty() {
            args.remove(0)
        } else {
            optimize_kw.unwrap_or(Value::Int(-1))
        };
        if !args.is_empty() {
            return Err(RuntimeError::new("compile() expected at most 6 arguments"));
        }

        let _ = value_to_int(flags_value)?;
        let _ = self.truthy_from_value(&dont_inherit_value)?;
        let _ = value_to_int(optimize_value)?;
        if let Some(value) = feature_kw {
            let _ = value_to_int(value)?;
        }

        let source_text = match source_arg {
            Value::Str(value) => value,
            other => {
                let bytes = bytes_like_from_value(other)?;
                String::from_utf8(bytes)
                    .map_err(|_| RuntimeError::new("compile() source is not valid UTF-8"))?
            }
        };
        let filename = match filename_arg {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("compile() filename must be str")),
        };
        let mode = match mode_arg {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("compile() mode must be str")),
        };
        if mode != "exec" && mode != "single" && mode != "eval" {
            return Err(RuntimeError::new(
                "compile() mode must be 'exec', 'eval', or 'single'",
            ));
        }
        let code = if mode == "eval" {
            let expr_ast = parser::parse_expression(&source_text).map_err(|err| {
                RuntimeError::new(format!(
                    "compile() parse error at {}: {}",
                    err.offset, err.message
                ))
            })?;
            compiler::compile_expression_with_filename(&expr_ast, &filename)
                .map_err(|err| RuntimeError::new(format!("compile() error: {}", err.message)))?
        } else {
            let module_ast = parser::parse_module(&source_text).map_err(|err| {
                RuntimeError::new(format!(
                    "compile() parse error at {}: {}",
                    err.offset, err.message
                ))
            })?;
            compiler::compile_module_with_filename(&module_ast, &filename)
                .map_err(|err| RuntimeError::new(format!("compile() error: {}", err.message)))?
        };
        Ok(Value::Code(Rc::new(code)))
    }

    pub(super) fn builtin_platform_libc_ver(
        &self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 4 {
            return Err(RuntimeError::new(
                "platform.libc_ver() expects up to 4 arguments",
            ));
        }

        let executable_kw = kwargs.remove("executable");
        let lib_kw = kwargs.remove("lib");
        let version_kw = kwargs.remove("version");
        let chunksize_kw = kwargs.remove("chunksize");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "platform.libc_ver() got an unexpected keyword argument",
            ));
        }

        if !args.is_empty() && executable_kw.is_some() {
            return Err(RuntimeError::new(
                "platform.libc_ver() got multiple values for executable",
            ));
        }
        if args.len() > 1 && lib_kw.is_some() {
            return Err(RuntimeError::new(
                "platform.libc_ver() got multiple values for lib",
            ));
        }
        if args.len() > 2 && version_kw.is_some() {
            return Err(RuntimeError::new(
                "platform.libc_ver() got multiple values for version",
            ));
        }
        if args.len() > 3 && chunksize_kw.is_some() {
            return Err(RuntimeError::new(
                "platform.libc_ver() got multiple values for chunksize",
            ));
        }

        let lib_value = args
            .get(1)
            .cloned()
            .or(lib_kw)
            .unwrap_or_else(|| Value::Str(String::new()));
        let version_value = args
            .get(2)
            .cloned()
            .or(version_kw)
            .unwrap_or_else(|| Value::Str(String::new()));
        let chunksize_value = args.get(3).cloned().or(chunksize_kw);

        let default_lib = match lib_value {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("platform.libc_ver() lib must be str")),
        };
        let default_version = match version_value {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("platform.libc_ver() version must be str")),
        };
        if let Some(value) = chunksize_value {
            value_to_int(value)?;
        }

        let detected_lib = if default_lib.is_empty() {
            if cfg!(all(target_os = "linux", target_env = "musl")) {
                "musl".to_string()
            } else if cfg!(target_os = "linux") {
                "glibc".to_string()
            } else {
                default_lib
            }
        } else {
            default_lib
        };

        Ok(self
            .heap
            .alloc_tuple(vec![Value::Str(detected_lib), Value::Str(default_version)]))
    }

    pub(super) fn builtin_platform_win32_is_iot(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "platform.win32_is_iot() expects no arguments",
            ));
        }
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_callable(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("callable() expects one argument"));
        }
        let value = args.remove(0);
        let callable = self.is_callable_value(&value);
        if !callable && std::env::var_os("PYRS_TRACE_CALLABLE_FALSE").is_some() {
            eprintln!(
                "[callable-false] type={} repr={}",
                self.value_type_name_for_error(&value),
                format_repr(&value)
            );
        }
        Ok(Value::Bool(callable))
    }

    pub(super) fn types_module_class(&self, name: &str) -> Option<ObjRef> {
        let module = self.modules.get("types")?;
        let Object::Module(module_data) = &*module.kind() else {
            return None;
        };
        match module_data.globals.get(name) {
            Some(Value::Class(class)) => Some(class.clone()),
            _ => None,
        }
    }

    fn fallback_none_type_class(&mut self) -> ObjRef {
        if let Some(module) = self.modules.get("builtins").cloned()
            && let Object::Module(module_data) = &*module.kind()
            && let Some(Value::Class(class)) = module_data.globals.get("__pyrs_none_type_class__")
        {
            return class.clone();
        }
        let class = self.alloc_synthetic_class("NoneType");
        if let Some(module) = self.modules.get("builtins").cloned()
            && let Object::Module(module_data) = &mut *module.kind_mut()
        {
            module_data.globals.insert(
                "__pyrs_none_type_class__".to_string(),
                Value::Class(class.clone()),
            );
        }
        class
    }

    fn bound_method_is_python_method(&self, method: &ObjRef) -> bool {
        let Object::BoundMethod(method_data) = &*method.kind() else {
            return false;
        };
        matches!(&*method_data.function.kind(), Object::Function(_))
    }

    pub(super) fn builtin_is_type_object(&self, builtin: BuiltinFunction) -> bool {
        matches!(
            builtin,
            BuiltinFunction::Type
                | BuiltinFunction::TypesMethodType
                | BuiltinFunction::Bool
                | BuiltinFunction::Int
                | BuiltinFunction::Float
                | BuiltinFunction::Str
                | BuiltinFunction::List
                | BuiltinFunction::Tuple
                | BuiltinFunction::Dict
                | BuiltinFunction::Set
                | BuiltinFunction::FrozenSet
                | BuiltinFunction::Bytes
                | BuiltinFunction::ByteArray
                | BuiltinFunction::MemoryView
                | BuiltinFunction::Complex
                | BuiltinFunction::Slice
                | BuiltinFunction::Range
                | BuiltinFunction::Enumerate
                | BuiltinFunction::Zip
                | BuiltinFunction::ClassMethod
                | BuiltinFunction::StaticMethod
                | BuiltinFunction::Property
                | BuiltinFunction::ObjectNew
                | BuiltinFunction::Super
                | BuiltinFunction::Map
                | BuiltinFunction::Filter
        )
    }

    pub(super) fn builtin_type(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() == 1 {
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "type() got an unexpected keyword argument",
                ));
            }
            let value = &args[0];
            let special = match value {
                Value::Class(class) => {
                    let metaclass = match &*class.kind() {
                        Object::Class(class_data) => class_data.metaclass.clone(),
                        _ => None,
                    };
                    metaclass.map(|meta| {
                        let is_builtin_type = self
                            .default_type_metaclass()
                            .map(|type_class| type_class.id() == meta.id())
                            .unwrap_or(false);
                        if is_builtin_type {
                            Value::Builtin(BuiltinFunction::Type)
                        } else {
                            Value::Class(meta)
                        }
                    })
                }
                Value::Function(_) => self.types_module_class("FunctionType").map(Value::Class),
                Value::BoundMethod(method) => {
                    if self.bound_method_is_python_method(method) {
                        Some(Value::Builtin(BuiltinFunction::TypesMethodType))
                    } else {
                        self.types_module_class("BuiltinMethodType")
                            .or_else(|| self.types_module_class("BuiltinFunctionType"))
                            .map(Value::Class)
                    }
                }
                Value::Builtin(builtin) => {
                    if self.builtin_is_type_object(*builtin) {
                        Some(Value::Builtin(BuiltinFunction::Type))
                    } else {
                        self.types_module_class("BuiltinFunctionType")
                            .map(Value::Class)
                    }
                }
                Value::Dict(dict) if self.defaultdict_factories.contains_key(&dict.id()) => {
                    Some(Value::Builtin(BuiltinFunction::CollectionsDefaultDict))
                }
                Value::Code(_) => self.types_module_class("CodeType").map(Value::Class),
                Value::None => Some(Value::Class(
                    self.types_module_class("NoneType")
                        .unwrap_or_else(|| self.fallback_none_type_class()),
                )),
                _ => None,
            };
            if let Some(marker) = special {
                return Ok(marker);
            }
            return BuiltinFunction::Type.call(&self.heap, args);
        }
        if args.len() == 4 || args.len() == 5 {
            let (metaclass_index, name_index) = if args.len() == 4 { (0, 1) } else { (1, 2) };
            let metaclass = match &args[metaclass_index] {
                Value::Class(class) => class.clone(),
                _ => {
                    return Err(RuntimeError::new(
                        "type.__new__() argument 1 must be a type",
                    ));
                }
            };
            let class_name = match &args[name_index] {
                Value::Str(name) => name.clone(),
                _ => return Err(RuntimeError::new("type() first argument must be string")),
            };
            let base_values = match &args[name_index + 1] {
                Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                    Object::Tuple(values) => values.clone(),
                    _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
                },
                Value::List(list_obj) => match &*list_obj.kind() {
                    Object::List(values) => values.clone(),
                    _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
                },
                _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
            };
            let mut base_classes = Vec::with_capacity(base_values.len());
            for base in base_values {
                base_classes.push(self.class_from_base_value(base)?);
            }
            let namespace = self.class_namespace_attrs_map(&args[name_index + 2])?;

            let class_value = self.build_default_class_value(
                class_name,
                namespace,
                base_classes,
                Some(metaclass),
            )?;
            if let Value::Class(class_ref) = &class_value
                && self.call_init_subclass_hook(class_ref, &kwargs)?
            {
                return Err(self.runtime_error_from_active_exception("type.__new__ failed"));
            }
            return Ok(class_value);
        }
        if args.len() == 3 {
            let mut class_keywords = kwargs;
            let explicit_metaclass = class_keywords
                .remove("metaclass")
                .filter(|value| !matches!(value, Value::None));
            let class_name = match &args[0] {
                Value::Str(name) => name.clone(),
                _ => return Err(RuntimeError::new("type() first argument must be string")),
            };
            let base_values = match &args[1] {
                Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                    Object::Tuple(values) => values.clone(),
                    _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
                },
                Value::List(list_obj) => match &*list_obj.kind() {
                    Object::List(values) => values.clone(),
                    _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
                },
                _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
            };
            let mut base_classes = Vec::with_capacity(base_values.len());
            for base in base_values {
                base_classes.push(self.class_from_base_value(base)?);
            }
            let namespace_value = args[2].clone();
            let namespace = self.class_namespace_attrs_map(&namespace_value)?;
            let class_module = match self.heap.alloc_module(ModuleObject::new(class_name)) {
                Value::Module(module) => module,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *class_module.kind_mut() {
                module_data.globals = namespace;
            }
            return match self.class_value_from_module(
                &class_module,
                base_classes,
                explicit_metaclass,
                class_keywords,
                Some(namespace_value),
            )? {
                ClassBuildOutcome::Value(value) => Ok(value),
                ClassBuildOutcome::ExceptionHandled => {
                    Err(self.runtime_error_from_active_exception("metaclass call failed"))
                }
            };
        }
        BuiltinFunction::Type.call(&self.heap, args)
    }

    pub(super) fn builtin_type_init(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        // CPython (Objects/typeobject.c:type_init) accepts payload arity 1 or 3,
        // where payload excludes the bound receiver (`cls`).
        let payload_len = args.len().saturating_sub(1);
        if !kwargs.is_empty() && payload_len == 1 {
            return Err(RuntimeError::new(
                "type.__init__() takes no keyword arguments",
            ));
        }
        if payload_len != 1 && payload_len != 3 {
            return Err(RuntimeError::new("type.__init__() takes 1 or 3 arguments"));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_type_annotations_get(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "__annotations__ descriptor __get__ expects 1-2 arguments",
            ));
        }

        let class = match args.first() {
            Some(Value::Class(class)) => class.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "__annotations__ descriptor requires a type object",
                ));
            }
        };

        let mut class_ref = class.kind_mut();
        let Object::Class(class_data) = &mut *class_ref else {
            return Err(RuntimeError::new(
                "__annotations__ descriptor requires a type object",
            ));
        };

        if let Some(existing) = class_data.attrs.get("__annotations__") {
            return match existing {
                Value::Dict(dict) => Ok(Value::Dict(dict.clone())),
                _ => Err(RuntimeError::new("__annotations__ must be a dict")),
            };
        }

        let annotations = match self.heap.alloc_dict(Vec::new()) {
            Value::Dict(dict) => dict,
            _ => unreachable!(),
        };
        class_data.attrs.insert(
            "__annotations__".to_string(),
            Value::Dict(annotations.clone()),
        );
        Ok(Value::Dict(annotations))
    }

    pub(super) fn builtin_dataclasses_field(
        &self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new(
                "field() accepts at most one positional argument",
            ));
        }
        let default = if let Some(value) = args.first() {
            value.clone()
        } else {
            kwargs.remove("default").unwrap_or(Value::None)
        };
        let default_factory = kwargs.remove("default_factory").unwrap_or(Value::None);
        let init = kwargs.remove("init").unwrap_or(Value::Bool(true));
        let repr = kwargs.remove("repr").unwrap_or(Value::Bool(true));
        let hash = kwargs.remove("hash").unwrap_or(Value::None);
        let compare = kwargs.remove("compare").unwrap_or(Value::Bool(true));
        let metadata = kwargs
            .remove("metadata")
            .unwrap_or_else(|| self.heap.alloc_dict(Vec::new()));
        let kw_only = kwargs.remove("kw_only").unwrap_or(Value::None);
        let doc = kwargs.remove("doc").unwrap_or(Value::None);
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "field() got an unexpected keyword argument",
            ));
        }
        Ok(self.heap.alloc_dict(vec![
            (Value::Str("default".to_string()), default),
            (Value::Str("default_factory".to_string()), default_factory),
            (Value::Str("init".to_string()), init),
            (Value::Str("repr".to_string()), repr),
            (Value::Str("hash".to_string()), hash),
            (Value::Str("compare".to_string()), compare),
            (Value::Str("metadata".to_string()), metadata),
            (Value::Str("kw_only".to_string()), kw_only),
            (Value::Str("doc".to_string()), doc),
        ]))
    }

    pub(super) fn builtin_dataclasses_is_dataclass(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("is_dataclass() expects one argument"));
        }
        let target = &args[0];
        match target {
            Value::Class(class) => {
                let Object::Class(class_data) = &*class.kind() else {
                    return Ok(Value::Bool(false));
                };
                Ok(Value::Bool(
                    class_data.attrs.contains_key("__dataclass_fields__")
                        || class_data.attrs.contains_key("__dataclass_params__"),
                ))
            }
            Value::Instance(instance) => {
                let Object::Instance(instance_data) = &*instance.kind() else {
                    return Ok(Value::Bool(false));
                };
                let Object::Class(class_data) = &*instance_data.class.kind() else {
                    return Ok(Value::Bool(false));
                };
                Ok(Value::Bool(
                    class_data.attrs.contains_key("__dataclass_fields__")
                        || class_data.attrs.contains_key("__dataclass_params__"),
                ))
            }
            _ => Ok(Value::Bool(false)),
        }
    }

    pub(super) fn dataclass_fields_value(&self, target: &Value) -> Option<Value> {
        match target {
            Value::Class(class) => {
                let Object::Class(class_data) = &*class.kind() else {
                    return None;
                };
                class_data.attrs.get("__dataclass_fields__").cloned()
            }
            Value::Instance(instance) => {
                let Object::Instance(instance_data) = &*instance.kind() else {
                    return None;
                };
                let Object::Class(class_data) = &*instance_data.class.kind() else {
                    return None;
                };
                class_data.attrs.get("__dataclass_fields__").cloned()
            }
            _ => None,
        }
    }

    pub(super) fn dataclass_field_names(
        &self,
        target: &Value,
    ) -> Result<Vec<String>, RuntimeError> {
        let Some(fields) = self.dataclass_fields_value(target) else {
            return Err(RuntimeError::new("target is not a dataclass"));
        };
        match fields {
            Value::Dict(dict) => {
                let Object::Dict(entries) = &*dict.kind() else {
                    return Err(RuntimeError::new("__dataclass_fields__ must be a dict"));
                };
                let mut names = Vec::new();
                for (key, _) in entries {
                    let Value::Str(name) = key else {
                        return Err(RuntimeError::new(
                            "__dataclass_fields__ keys must be strings",
                        ));
                    };
                    names.push(name.clone());
                }
                Ok(names)
            }
            _ => Err(RuntimeError::new("__dataclass_fields__ must be a dict")),
        }
    }

    pub(super) fn builtin_dataclasses_fields(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("fields() expects one dataclass argument"));
        }
        let Some(fields) = self.dataclass_fields_value(&args[0]) else {
            return Err(RuntimeError::new(
                "fields() expects a dataclass or dataclass instance",
            ));
        };
        match fields {
            Value::Dict(dict) => {
                let Object::Dict(entries) = &*dict.kind() else {
                    return Err(RuntimeError::new("__dataclass_fields__ must be a dict"));
                };
                let values = entries
                    .iter()
                    .map(|(_, value)| value.clone())
                    .collect::<Vec<_>>();
                Ok(self.heap.alloc_tuple(values))
            }
            _ => Err(RuntimeError::new("__dataclass_fields__ must be a dict")),
        }
    }

    pub(super) fn builtin_dataclasses_asdict(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("asdict() expects one dataclass instance"));
        }
        let instance = match &args[0] {
            Value::Instance(instance) => instance.clone(),
            _ => return Err(RuntimeError::new("asdict() expects a dataclass instance")),
        };
        let names = self.dataclass_field_names(&args[0])?;
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new("asdict() expects a dataclass instance"));
        };
        let mut entries = Vec::new();
        for name in names {
            entries.push((
                Value::Str(name.clone()),
                instance_data
                    .attrs
                    .get(&name)
                    .cloned()
                    .unwrap_or(Value::None),
            ));
        }
        Ok(self.heap.alloc_dict(entries))
    }

    pub(super) fn builtin_dataclasses_astuple(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "astuple() expects one dataclass instance",
            ));
        }
        let instance = match &args[0] {
            Value::Instance(instance) => instance.clone(),
            _ => return Err(RuntimeError::new("astuple() expects a dataclass instance")),
        };
        let names = self.dataclass_field_names(&args[0])?;
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new("astuple() expects a dataclass instance"));
        };
        let mut values = Vec::new();
        for name in names {
            values.push(
                instance_data
                    .attrs
                    .get(&name)
                    .cloned()
                    .unwrap_or(Value::None),
            );
        }
        Ok(self.heap.alloc_tuple(values))
    }

    pub(super) fn builtin_dataclasses_replace(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "replace() expects instance plus keyword replacements",
            ));
        }
        let instance = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("replace() expects a dataclass instance")),
        };
        let class = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return Err(RuntimeError::new("replace() expects a dataclass instance")),
        };
        let mut attrs = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.attrs.clone(),
            _ => HashMap::new(),
        };
        for (name, value) in kwargs {
            attrs.insert(name, value);
        }
        let new_instance = match self.heap.alloc_instance(InstanceObject::new(class)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *new_instance.kind_mut() {
            instance_data.attrs = attrs;
        }
        Ok(Value::Instance(new_instance))
    }

    pub(super) fn builtin_dataclasses_make_dataclass(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::new(
                "make_dataclass() expects class name and field iterable",
            ));
        }
        let class_name = match &args[0] {
            Value::Str(name) => name.clone(),
            _ => return Err(RuntimeError::new("make_dataclass() class name must be str")),
        };
        let field_values = self.collect_iterable_values(args[1].clone())?;
        let mut field_names = Vec::new();
        for item in field_values {
            match item {
                Value::Str(name) => field_names.push(name),
                Value::Tuple(tuple) => match &*tuple.kind() {
                    Object::Tuple(values) if !values.is_empty() => match &values[0] {
                        Value::Str(name) => field_names.push(name.clone()),
                        _ => {
                            return Err(RuntimeError::new(
                                "make_dataclass() field tuple must start with str name",
                            ));
                        }
                    },
                    _ => {
                        return Err(RuntimeError::new(
                            "make_dataclass() field tuple must not be empty",
                        ));
                    }
                },
                _ => {
                    return Err(RuntimeError::new(
                        "make_dataclass() fields must contain str or tuple entries",
                    ));
                }
            }
        }

        let bases = if let Some(value) = kwargs.remove("bases") {
            match value {
                Value::Tuple(tuple) => {
                    let Object::Tuple(values) = &*tuple.kind() else {
                        return Err(RuntimeError::new("make_dataclass() bases must be a tuple"));
                    };
                    let mut out = Vec::new();
                    for value in values {
                        match value {
                            Value::Class(class) => out.push(class.clone()),
                            _ => {
                                return Err(RuntimeError::new(
                                    "make_dataclass() bases must contain classes",
                                ));
                            }
                        }
                    }
                    out
                }
                _ => return Err(RuntimeError::new("make_dataclass() bases must be a tuple")),
            }
        } else {
            Vec::new()
        };
        let mut class = ClassObject::new(class_name, bases);
        let mut entries = Vec::new();
        for name in field_names {
            entries.push((Value::Str(name), Value::None));
        }
        class.attrs.insert(
            "__dataclass_fields__".to_string(),
            self.heap.alloc_dict(entries),
        );
        if let Some(namespace) = kwargs.remove("namespace")
            && let Value::Dict(dict) = namespace
            && let Object::Dict(entries) = &*dict.kind()
        {
            for (key, value) in entries {
                if let Value::Str(name) = key {
                    class.attrs.insert(name.clone(), value.clone());
                }
            }
        }
        if let Some(module_name) = kwargs.remove("module") {
            match module_name {
                Value::Str(name) => {
                    class
                        .attrs
                        .insert("__module__".to_string(), Value::Str(name));
                }
                _ => return Err(RuntimeError::new("make_dataclass() module must be str")),
            }
        }
        let class_value = self.heap.alloc_class(class);
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "make_dataclass() got an unexpected keyword argument",
            ));
        }
        Ok(class_value)
    }

    pub(super) fn builtin_isinstance(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("isinstance() expects two arguments"));
        }
        let value = args.remove(0);
        let classinfo = args.remove(0);
        Ok(Value::Bool(self.value_is_instance_of(&value, &classinfo)?))
    }

    pub(super) fn builtin_type_instancecheck(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "__instancecheck__() expects one argument",
            ));
        }
        let classinfo = args.remove(0);
        let value = args.remove(0);
        Ok(Value::Bool(self.value_is_instance_of(&value, &classinfo)?))
    }

    pub(super) fn builtin_issubclass(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("issubclass() expects two arguments"));
        }
        let candidate = args.remove(0);
        let classinfo = args.remove(0);
        Ok(Value::Bool(
            self.class_value_is_subclass_of(&candidate, &classinfo)?,
        ))
    }

    pub(super) fn builtin_type_subclasscheck(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "__subclasscheck__() expects one argument",
            ));
        }
        let classinfo = args.remove(0);
        let candidate = args.remove(0);
        Ok(Value::Bool(
            self.class_value_is_subclass_of(&candidate, &classinfo)?,
        ))
    }

    pub(super) fn builtin_property(
        &self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 4 {
            return Err(RuntimeError::new("property() expects up to four arguments"));
        }
        let mut fget = args.first().cloned().unwrap_or(Value::None);
        let mut fset = args.get(1).cloned().unwrap_or(Value::None);
        let mut fdel = args.get(2).cloned().unwrap_or(Value::None);
        let mut doc = args.get(3).cloned().unwrap_or(Value::None);

        if let Some(value) = kwargs.remove("fget") {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "property() got multiple values for argument 'fget'",
                ));
            }
            fget = value;
        }
        if let Some(value) = kwargs.remove("fset") {
            if args.len() > 1 {
                return Err(RuntimeError::new(
                    "property() got multiple values for argument 'fset'",
                ));
            }
            fset = value;
        }
        if let Some(value) = kwargs.remove("fdel") {
            if args.len() > 2 {
                return Err(RuntimeError::new(
                    "property() got multiple values for argument 'fdel'",
                ));
            }
            fdel = value;
        }
        if let Some(value) = kwargs.remove("doc") {
            if args.len() > 3 {
                return Err(RuntimeError::new(
                    "property() got multiple values for argument 'doc'",
                ));
            }
            doc = value;
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "property() got an unexpected keyword argument",
            ));
        }
        Ok(self.build_property_descriptor(fget, fset, fdel, doc, None))
    }

    pub(super) fn builtin_object_new(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "object.__new__() expects a class argument",
            ));
        }
        let class_value = args.remove(0);
        let mut class_ref = match class_value {
            Value::Class(class) => class,
            Value::Builtin(builtin) if self.builtin_is_type_object(builtin) => {
                self.class_from_base_value(Value::Builtin(builtin))?
            }
            _ => {
                return Err(RuntimeError::new(
                    "object.__new__(X): X is not a type object",
                ));
            }
        };
        // `super(...).__new__(cls)` can arrive here as a bound built-in call shape
        // where the explicit `cls` argument is still present in `args`.
        if let Some(explicit_class) = args.first().cloned() {
            match explicit_class {
                Value::Class(explicit_class) => {
                    class_ref = explicit_class;
                    args.remove(0);
                }
                Value::Builtin(builtin) if self.builtin_is_type_object(builtin) => {
                    class_ref = self.class_from_base_value(Value::Builtin(builtin))?;
                    args.remove(0);
                }
                _ => {}
            }
        }
        if let Some(message) = self.class_disallow_instantiation_message(&class_ref) {
            return Err(RuntimeError::new(message));
        }
        let instance = self.alloc_instance_for_class(&class_ref);
        if self.class_has_builtin_int_base(&class_ref) {
            let int_value = self.builtin_int(args, kwargs)?;
            let (Value::Int(_) | Value::BigInt(_) | Value::Bool(_)) = int_value else {
                return Err(RuntimeError::new("int constructor returned non-int"));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(INT_BACKING_STORAGE_ATTR.to_string(), int_value);
            }
            return Ok(Value::Instance(instance));
        }
        if self.class_has_builtin_float_base(&class_ref) {
            let float_value = self.builtin_float(args, kwargs)?;
            let Value::Float(_) = float_value else {
                return Err(RuntimeError::new("float constructor returned non-float"));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(FLOAT_BACKING_STORAGE_ATTR.to_string(), float_value);
            }
            return Ok(Value::Instance(instance));
        }
        if self.class_has_builtin_complex_base(&class_ref) {
            let complex_value = self.builtin_complex(args, kwargs)?;
            let Value::Complex { .. } = complex_value else {
                return Err(RuntimeError::new(
                    "complex constructor returned non-complex",
                ));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(COMPLEX_BACKING_STORAGE_ATTR.to_string(), complex_value);
            }
            return Ok(Value::Instance(instance));
        }
        if self.class_has_builtin_list_base(&class_ref) {
            let list_value = self.call_builtin(BuiltinFunction::List, args, kwargs)?;
            let Value::List(_) = list_value else {
                return Err(RuntimeError::new("list constructor returned non-list"));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(LIST_BACKING_STORAGE_ATTR.to_string(), list_value);
            }
            return Ok(Value::Instance(instance));
        }
        if self.class_has_builtin_tuple_base(&class_ref) {
            let tuple_value = self.call_builtin(BuiltinFunction::Tuple, args, kwargs)?;
            let Value::Tuple(_) = tuple_value else {
                return Err(RuntimeError::new("tuple constructor returned non-tuple"));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(TUPLE_BACKING_STORAGE_ATTR.to_string(), tuple_value);
            }
            return Ok(Value::Instance(instance));
        }
        if self.class_has_builtin_str_base(&class_ref) {
            let str_value = self.call_builtin(BuiltinFunction::Str, args, kwargs)?;
            let Value::Str(_) = str_value else {
                return Err(RuntimeError::new("str constructor returned non-str"));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(STR_BACKING_STORAGE_ATTR.to_string(), str_value);
            }
            return Ok(Value::Instance(instance));
        }
        if self.class_has_builtin_bytes_base(&class_ref) {
            let bytes_value = self.call_builtin(BuiltinFunction::Bytes, args, kwargs)?;
            let Value::Bytes(_) = bytes_value else {
                return Err(RuntimeError::new("bytes constructor returned non-bytes"));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(BYTES_BACKING_STORAGE_ATTR.to_string(), bytes_value);
            }
            return Ok(Value::Instance(instance));
        }
        if self.class_has_builtin_bytearray_base(&class_ref) {
            let bytearray_value = self.call_builtin(BuiltinFunction::ByteArray, args, kwargs)?;
            let Value::ByteArray(_) = bytearray_value else {
                return Err(RuntimeError::new(
                    "bytearray constructor returned non-bytearray",
                ));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(BYTES_BACKING_STORAGE_ATTR.to_string(), bytearray_value);
            }
            return Ok(Value::Instance(instance));
        }
        if self.class_has_builtin_dict_base(&class_ref) {
            let dict_value = self.call_builtin(BuiltinFunction::Dict, args, kwargs)?;
            let Value::Dict(_) = dict_value else {
                return Err(RuntimeError::new("dict constructor returned non-dict"));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(DICT_BACKING_STORAGE_ATTR.to_string(), dict_value);
            }
            return Ok(Value::Instance(instance));
        }
        if self.class_has_builtin_set_base(&class_ref) {
            let set_value = self.call_builtin(BuiltinFunction::Set, args, kwargs)?;
            let Value::Set(_) = set_value else {
                return Err(RuntimeError::new("set constructor returned non-set"));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(SET_BACKING_STORAGE_ATTR.to_string(), set_value);
            }
            return Ok(Value::Instance(instance));
        }
        if self.class_has_builtin_frozenset_base(&class_ref) {
            let frozenset_value = self.call_builtin(BuiltinFunction::FrozenSet, args, kwargs)?;
            let Value::FrozenSet(_) = frozenset_value else {
                return Err(RuntimeError::new(
                    "frozenset constructor returned non-frozenset",
                ));
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(FROZENSET_BACKING_STORAGE_ATTR.to_string(), frozenset_value);
            }
            return Ok(Value::Instance(instance));
        }
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "object.__new__() takes exactly one argument",
            ));
        }
        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_object_init(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        // `object.__init__` is exposed as a plain builtin in this VM, so
        // super() calls can reach it without an implicit `self` bind.
        if !kwargs.is_empty() || args.is_empty() {
            return Err(RuntimeError::new(
                "object.__init__() takes exactly one argument",
            ));
        }
        if args.len() > 1 {
            let allow_extra = match &args[0] {
                Value::Instance(instance) => match &*instance.kind() {
                    Object::Instance(instance_data) => {
                        self.class_allows_object_init_extra_args(&instance_data.class)
                    }
                    _ => false,
                },
                _ => false,
            };
            if !allow_extra {
                if std::env::var_os("PYRS_TRACE_OBJECT_INIT").is_some() {
                    let class_name = match &args[0] {
                        Value::Instance(instance) => match &*instance.kind() {
                            Object::Instance(instance_data) => match &*instance_data.class.kind() {
                                Object::Class(class_data) => class_data.name.clone(),
                                _ => "<non-class>".to_string(),
                            },
                            _ => "<non-instance>".to_string(),
                        },
                        other => self.value_type_name_for_error(other),
                    };
                    eprintln!(
                        "[object-init] rejecting extra args class={} argc={} kwargs={}",
                        class_name,
                        args.len(),
                        kwargs.len()
                    );
                }
                return Err(RuntimeError::new(
                    "object.__init__() takes exactly one argument",
                ));
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_exception_type_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "BaseException.__init__() takes no keyword arguments",
            ));
        }
        let receiver = args.remove(0);
        match receiver {
            Value::Instance(instance) => {
                let exception_name = match &*instance.kind() {
                    Object::Instance(instance_data) => {
                        if !self.class_is_exception_class(&instance_data.class) {
                            return Err(RuntimeError::new(
                                "descriptor '__init__' requires a 'BaseException' object",
                            ));
                        }
                        match &*instance_data.class.kind() {
                            Object::Class(class_data) => class_data.name.clone(),
                            _ => "BaseException".to_string(),
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "descriptor '__init__' requires a 'BaseException' object",
                        ));
                    }
                };
                let import_error_family = is_import_error_family(exception_name.as_str());
                let mut import_error_name = Value::None;
                let mut import_error_path = Value::None;
                if import_error_family {
                    if let Some(msg_kw) = kwargs.remove("msg") {
                        if args.is_empty() {
                            args.push(msg_kw);
                        } else {
                            return Err(RuntimeError::new(
                                "ImportError.__init__() got multiple values for argument 'msg'",
                            ));
                        }
                    }
                    if let Some(value) = kwargs.remove("name") {
                        import_error_name = value;
                    }
                    if let Some(value) = kwargs.remove("path") {
                        import_error_path = value;
                    }
                }
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "BaseException.__init__() takes no keyword arguments",
                    ));
                }
                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                    let is_stop_iteration = {
                        let class_kind = instance_data.class.kind();
                        matches!(
                            &*class_kind,
                            Object::Class(class_data)
                                if matches!(
                                    class_data.name.as_str(),
                                    "StopIteration" | "StopAsyncIteration"
                                )
                        )
                    };
                    instance_data
                        .attrs
                        .insert("args".to_string(), self.heap.alloc_tuple(args.clone()));
                    if is_stop_iteration {
                        let value = args.first().cloned().unwrap_or(Value::None);
                        instance_data.attrs.insert("value".to_string(), value);
                    }
                    if is_os_error_family(exception_name.as_str()) {
                        if let Some(errno) = args
                            .first()
                            .and_then(|value| value_to_int(value.clone()).ok())
                        {
                            instance_data
                                .attrs
                                .insert("errno".to_string(), Value::Int(errno));
                        }
                        if let Some(strerror) = args.get(1) {
                            instance_data
                                .attrs
                                .insert("strerror".to_string(), strerror.clone());
                        }
                        if let Some(third) = args.get(2) {
                            if exception_name == "BlockingIOError" {
                                instance_data
                                    .attrs
                                    .insert("characters_written".to_string(), third.clone());
                            } else {
                                instance_data
                                    .attrs
                                    .insert("filename".to_string(), third.clone());
                            }
                        }
                    }
                    if import_error_family {
                        instance_data.attrs.insert(
                            "msg".to_string(),
                            args.first().cloned().unwrap_or(Value::None),
                        );
                        instance_data
                            .attrs
                            .insert("name".to_string(), import_error_name);
                        instance_data
                            .attrs
                            .insert("path".to_string(), import_error_path);
                    }
                    return Ok(Value::None);
                }
                Err(RuntimeError::new(
                    "descriptor '__init__' requires a 'BaseException' object",
                ))
            }
            Value::Exception(mut exception) => {
                let import_error_family = is_import_error_family(exception.name.as_str());
                let mut import_error_name = Value::None;
                let mut import_error_path = Value::None;
                if import_error_family {
                    if let Some(msg_kw) = kwargs.remove("msg") {
                        if args.is_empty() {
                            args.push(msg_kw);
                        } else {
                            return Err(RuntimeError::new(
                                "ImportError.__init__() got multiple values for argument 'msg'",
                            ));
                        }
                    }
                    if let Some(value) = kwargs.remove("name") {
                        import_error_name = value;
                    }
                    if let Some(value) = kwargs.remove("path") {
                        import_error_path = value;
                    }
                }
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "BaseException.__init__() takes no keyword arguments",
                    ));
                }
                let mut attrs = exception.attrs.borrow_mut();
                attrs.insert("args".to_string(), self.heap.alloc_tuple(args.clone()));
                if matches!(
                    exception.name.as_str(),
                    "StopIteration" | "StopAsyncIteration"
                ) {
                    attrs.insert(
                        "value".to_string(),
                        args.first().cloned().unwrap_or(Value::None),
                    );
                }
                if args.len() == 1 {
                    exception.message = Some(format_value(&args[0]));
                } else if args.is_empty() {
                    exception.message = None;
                }
                if is_os_error_family(exception.name.as_str()) {
                    if let Some(errno) = args
                        .first()
                        .and_then(|value| value_to_int(value.clone()).ok())
                    {
                        attrs.insert("errno".to_string(), Value::Int(errno));
                    }
                    if let Some(strerror) = args.get(1) {
                        attrs.insert("strerror".to_string(), strerror.clone());
                    }
                    if let Some(third) = args.get(2) {
                        if exception.name == "BlockingIOError" {
                            attrs.insert("characters_written".to_string(), third.clone());
                        } else {
                            attrs.insert("filename".to_string(), third.clone());
                        }
                    }
                }
                if import_error_family {
                    attrs.insert(
                        "msg".to_string(),
                        args.first().cloned().unwrap_or(Value::None),
                    );
                    attrs.insert("name".to_string(), import_error_name);
                    attrs.insert("path".to_string(), import_error_path);
                }
                Ok(Value::None)
            }
            _ => Err(RuntimeError::new(
                "descriptor '__init__' requires a 'BaseException' object",
            )),
        }
    }

    pub(super) fn class_allows_object_init_extra_args(&self, class: &ObjRef) -> bool {
        // Match CPython object_init semantics from Objects/typeobject.c:
        // allow excess args only when __init__ resolves to object.__init__
        // and __new__ resolves to a non-object.__new__ implementation.
        let init_attr = class_attr_lookup(class, "__init__");
        let new_attr = class_attr_lookup(class, "__new__");
        let init_is_object_init = matches!(
            init_attr,
            None | Some(Value::Builtin(BuiltinFunction::ObjectInit))
        );
        if !init_is_object_init {
            return false;
        }
        if self.class_has_builtin_int_base(class)
            || self.class_has_builtin_float_base(class)
            || self.class_has_builtin_str_base(class)
            || self.class_has_builtin_list_base(class)
            || self.class_has_builtin_tuple_base(class)
            || self.class_has_builtin_dict_base(class)
            || self.class_has_builtin_set_base(class)
            || self.class_has_builtin_frozenset_base(class)
            || self.class_has_builtin_bytes_base(class)
            || self.class_has_builtin_bytearray_base(class)
            || self.class_has_builtin_complex_base(class)
        {
            return true;
        }
        if let Object::Class(class_data) = &*class.kind()
            && class_data.attrs.contains_key("__pyrs_cpython_proxy_ptr__")
        {
            return true;
        }
        if self
            .extension_cpython_ptr_by_object_id
            .contains_key(&class.id())
        {
            if std::env::var_os("PYRS_TRACE_OBJECT_INIT_CLASS").is_some()
                && let Object::Class(class_data) = &*class.kind()
            {
                eprintln!(
                    "[object-init-class] class={} id={} allow=extension-map",
                    class_data.name,
                    class.id()
                );
            }
            return true;
        }
        if std::env::var_os("PYRS_TRACE_OBJECT_INIT_CLASS").is_some()
            && let Object::Class(class_data) = &*class.kind()
        {
            eprintln!(
                "[object-init-class] class={} id={} allow=fallback new_attr={}",
                class_data.name,
                class.id(),
                !matches!(
                    new_attr,
                    None | Some(Value::Builtin(BuiltinFunction::ObjectNew))
                )
            );
        }
        !matches!(
            new_attr,
            None | Some(Value::Builtin(BuiltinFunction::ObjectNew))
        )
    }

    pub(super) fn builtin_object_getattribute(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "object.__getattribute__() expects two arguments",
            ));
        }
        let target = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("attribute name must be string")),
        };

        match target {
            Value::Instance(instance) => {
                match self.load_attr_instance_default(&instance, &name, false)? {
                    AttrAccessOutcome::Value(value) => Ok(value),
                    AttrAccessOutcome::ExceptionHandled => Err(self
                        .runtime_error_from_active_exception("object.__getattribute__() failed")),
                }
            }
            other => self.builtin_getattr(vec![other, Value::Str(name)], HashMap::new()),
        }
    }

    pub(super) fn builtin_object_format(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "TypeError: object.__format__() expects two arguments",
            ));
        }
        let target = args.remove(0);
        let format_spec = match args.remove(0) {
            Value::Str(spec) => spec,
            other => {
                return Err(RuntimeError::new(format!(
                    "TypeError: object.__format__() argument must be str, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        if !format_spec.is_empty() {
            return Err(RuntimeError::new(format!(
                "TypeError: unsupported format string passed to {}.__format__",
                self.value_type_name_for_error(&target)
            )));
        }
        self.builtin_str(vec![target], HashMap::new())
    }

    pub(super) fn builtin_object_setattr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(RuntimeError::new(
                "object.__setattr__() expects three arguments",
            ));
        }
        let target = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("attribute name must be string")),
        };
        let value = args.remove(0);
        match target {
            Value::Instance(instance) => {
                match self.store_attr_instance_direct(&instance, &name, value)? {
                    AttrMutationOutcome::Done => {}
                    AttrMutationOutcome::ExceptionHandled => {
                        return Err(
                            self.runtime_error_from_active_exception("object.__setattr__() failed")
                        );
                    }
                }
            }
            Value::Cell(cell) => self.store_attr_cell(&cell, &name, value)?,
            other => {
                if std::env::var_os("PYRS_TRACE_SETATTR_UNSUPPORTED").is_some() {
                    eprintln!(
                        "[object-setattr-unsupported] target={} name={}",
                        format_repr(&other),
                        name
                    );
                }
                return Err(RuntimeError::new("attribute assignment unsupported type"));
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_object_delattr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "object.__delattr__() expects two arguments",
            ));
        }
        let target = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("attribute name must be string")),
        };
        match target {
            Value::Instance(instance) => {
                match self.delete_attr_instance_direct(&instance, &name)? {
                    AttrMutationOutcome::Done => {}
                    AttrMutationOutcome::ExceptionHandled => {
                        return Err(
                            self.runtime_error_from_active_exception("object.__delattr__() failed")
                        );
                    }
                }
            }
            Value::Cell(cell) => self.delete_attr_cell(&cell, &name)?,
            _ => return Err(RuntimeError::new("attribute deletion unsupported type")),
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_list(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("list() expects at most one argument"));
        }
        let values = if args.is_empty() {
            Vec::new()
        } else {
            self.collect_iterable_values(args.remove(0))?
        };
        Ok(self.heap.alloc_list(values))
    }

    pub(super) fn builtin_tuple(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("tuple() expects at most one argument"));
        }
        let source = match args.len() {
            0 => None,
            1 => Some(args.remove(0)),
            2 => {
                let cls = args.remove(0);
                match cls {
                    Value::Builtin(BuiltinFunction::Tuple) => {}
                    Value::Class(class) => {
                        if !self.class_has_builtin_tuple_base(&class) {
                            let class_name = match &*class.kind() {
                                Object::Class(class_data) => class_data.name.clone(),
                                _ => "<class>".to_string(),
                            };
                            return Err(RuntimeError::new(format!(
                                "tuple.__new__({}): {} is not a subtype of tuple",
                                class_name, class_name
                            )));
                        }
                    }
                    _ => return Err(RuntimeError::new("tuple() expects at most one argument")),
                }
                Some(args.remove(0))
            }
            _ => return Err(RuntimeError::new("tuple() expects at most one argument")),
        };
        let values = if let Some(source) = source {
            match source {
                Value::Instance(instance) => {
                    if let Some(backing) = self.instance_backing_tuple(&instance) {
                        match &*backing.kind() {
                            Object::Tuple(values) => values.clone(),
                            _ => self.collect_iterable_values(Value::Instance(instance))?,
                        }
                    } else {
                        self.collect_iterable_values(Value::Instance(instance))?
                    }
                }
                other => self.collect_iterable_values(other)?,
            }
        } else {
            Vec::new()
        };
        Ok(self.heap.alloc_tuple(values))
    }

    pub(super) fn builtin_array_array(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("array.array() expects 1-2 arguments"));
        }
        let typecode_text = match args.remove(0) {
            Value::Str(text) => text,
            other => {
                return Err(RuntimeError::new(format!(
                    "array() argument 1 must be a unicode character, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        let mut typecode_chars = typecode_text.chars();
        let typecode = match (typecode_chars.next(), typecode_chars.next()) {
            (Some(ch), None) => ch,
            _ => {
                return Err(RuntimeError::new(format!(
                    "array() argument 1 must be a unicode character, not a string of length {}",
                    typecode_text.chars().count()
                )));
            }
        };
        let (itemsize, wide_char_typecode) = match typecode {
            'b' | 'B' => (1, false),
            'h' | 'H' => (2, false),
            'i' | 'I' | 'l' | 'L' | 'f' => (4, false),
            'q' | 'Q' | 'd' => (8, false),
            'u' | 'w' => (4, true),
            _ => {
                return Err(RuntimeError::new(
                    "bad typecode (must be b, B, u, w, h, H, i, I, l, L, q, Q, f or d)",
                ));
            }
        };
        let mut from_bytes_initializer = false;
        let mut values = if args.is_empty() {
            Vec::new()
        } else {
            let initializer = args.remove(0);
            match initializer {
                Value::None => Vec::new(),
                Value::Bytes(obj) | Value::ByteArray(obj) => {
                    from_bytes_initializer = true;
                    let bytes = match &*obj.kind() {
                        Object::Bytes(bytes) | Object::ByteArray(bytes) => bytes.clone(),
                        _ => Vec::new(),
                    };
                    bytes.iter().map(|byte| Value::Int(*byte as i64)).collect()
                }
                Value::Str(text) => {
                    if !wide_char_typecode {
                        return Err(RuntimeError::new(format!(
                            "cannot use a str to initialize an array with typecode '{}'",
                            typecode
                        )));
                    }
                    text.chars().map(|ch| Value::Str(ch.to_string())).collect()
                }
                other => self.collect_iterable_values(other)?,
            }
        };
        if wide_char_typecode {
            let mut normalized = Vec::with_capacity(values.len());
            for value in values {
                match value {
                    Value::Str(text) => {
                        if text.chars().count() != 1 {
                            return Err(RuntimeError::new(
                                "array item must be a unicode character",
                            ));
                        }
                        normalized.push(Value::Str(text));
                    }
                    other => {
                        return Err(RuntimeError::new(format!(
                            "array item must be a unicode character, not {}",
                            self.value_type_name_for_error(&other)
                        )));
                    }
                }
            }
            values = normalized;
        }
        let values = self.heap.alloc_list(values);
        let module = match self.heap.alloc_module(ModuleObject::new("__array__")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data
                .globals
                .insert("typecode".to_string(), Value::Str(typecode.to_string()));
            module_data
                .globals
                .insert("itemsize".to_string(), Value::Int(itemsize));
            module_data.globals.insert(
                "__pyrs_array_frombytes__".to_string(),
                Value::Bool(from_bytes_initializer),
            );
            module_data.globals.insert("values".to_string(), values);
        }
        Ok(Value::Module(module))
    }

    pub(super) fn builtin_dict(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new("dict() expects at most one argument"));
        }

        if args.len() == 2 {
            let cls = args.remove(0);
            match cls {
                Value::Builtin(BuiltinFunction::Dict) => {}
                Value::Class(class) => {
                    if !self.class_has_builtin_dict_base(&class) {
                        let class_name = match &*class.kind() {
                            Object::Class(class_data) => class_data.name.clone(),
                            _ => "<class>".to_string(),
                        };
                        return Err(RuntimeError::new(format!(
                            "dict.__new__({}): {} is not a subtype of dict",
                            class_name, class_name
                        )));
                    }
                }
                _ => return Err(RuntimeError::new("dict() expects at most one argument")),
            }
        }

        let dict_obj = match self.heap.alloc_dict(Vec::new()) {
            Value::Dict(obj) => obj,
            _ => unreachable!(),
        };

        if let Some(source) = args.pop() {
            match source {
                Value::Dict(obj) => {
                    if let Object::Dict(entries) = &*obj.kind() {
                        for (key, value) in entries {
                            dict_set_value_checked(&dict_obj, key.clone(), value.clone())?;
                        }
                    }
                }
                Value::Instance(instance) => {
                    if let Some(backing) = self.instance_backing_dict(&instance) {
                        if let Object::Dict(entries) = &*backing.kind() {
                            for (key, value) in entries {
                                dict_set_value_checked(&dict_obj, key.clone(), value.clone())?;
                            }
                        }
                    } else if !self.dict_extend_from_mapping_protocol(
                        &dict_obj,
                        &Value::Instance(instance.clone()),
                    )? {
                        self.dict_extend_from_iterable_pairs(&dict_obj, Value::Instance(instance))?;
                    }
                }
                other => {
                    if !self.dict_extend_from_mapping_protocol(&dict_obj, &other)? {
                        self.dict_extend_from_iterable_pairs(&dict_obj, other)?;
                    } else {
                        // handled by mapping protocol path
                    }
                }
            }
        }

        for (name, value) in kwargs {
            dict_set_value_checked(&dict_obj, Value::Str(name), value)?;
        }

        Ok(Value::Dict(dict_obj))
    }

    fn dict_extend_from_mapping_protocol(
        &mut self,
        dict_obj: &ObjRef,
        source: &Value,
    ) -> Result<bool, RuntimeError> {
        let keys_callable = match self.builtin_getattr(
            vec![source.clone(), Value::Str("keys".to_string())],
            HashMap::new(),
        ) {
            Ok(callable) => callable,
            Err(err) if runtime_error_matches_exception(&err, "AttributeError") => {
                return Ok(false);
            }
            Err(err) => return Err(err),
        };
        let keys_value = match self.call_internal(keys_callable, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(
                    self.runtime_error_from_active_exception("dict() mapping keys() call failed")
                );
            }
        };
        let keys = self.collect_iterable_values(keys_value)?;
        for key in keys {
            let value =
                self.builtin_operator_getitem(vec![source.clone(), key.clone()], HashMap::new())?;
            dict_set_value_checked(dict_obj, key, value)?;
        }
        Ok(true)
    }

    fn dict_extend_from_iterable_pairs(
        &mut self,
        dict_obj: &ObjRef,
        source: Value,
    ) -> Result<(), RuntimeError> {
        for item in self.collect_iterable_values(source)? {
            match item {
                Value::Tuple(pair) => match &*pair.kind() {
                    Object::Tuple(parts) if parts.len() == 2 => {
                        dict_set_value_checked(dict_obj, parts[0].clone(), parts[1].clone())?;
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "dict() sequence elements must be length 2",
                        ));
                    }
                },
                Value::List(pair) => match &*pair.kind() {
                    Object::List(parts) if parts.len() == 2 => {
                        dict_set_value_checked(dict_obj, parts[0].clone(), parts[1].clone())?;
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "dict() sequence elements must be length 2",
                        ));
                    }
                },
                _ => {
                    return Err(RuntimeError::new(
                        "dict() argument must be a mapping or iterable of pairs",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn builtin_dict_fromkeys(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new("dict.fromkeys() expects 1-2 arguments"));
        }
        let mut dict_subclass: Option<ObjRef> = None;
        if args.len() >= 2 {
            match args.first().cloned() {
                Some(Value::Builtin(BuiltinFunction::Dict)) => {
                    args.remove(0);
                }
                Some(Value::Class(class)) if self.class_has_builtin_dict_base(&class) => {
                    dict_subclass = Some(class);
                    args.remove(0);
                }
                _ => {}
            }
        }
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("dict.fromkeys() expects 1-2 arguments"));
        }
        let keys = self.collect_iterable_values(args[0].clone())?;
        let default = args.get(1).cloned().unwrap_or(Value::None);
        let dict_obj = match self.heap.alloc_dict(Vec::new()) {
            Value::Dict(obj) => obj,
            _ => unreachable!(),
        };
        for key in keys {
            dict_set_value_checked(&dict_obj, key, default.clone())?;
        }
        if let Some(class) = dict_subclass {
            let instance = match self.heap.alloc_instance(InstanceObject::new(class)) {
                Value::Instance(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(DICT_BACKING_STORAGE_ATTR.to_string(), Value::Dict(dict_obj));
            }
            return Ok(Value::Instance(instance));
        }
        Ok(Value::Dict(dict_obj))
    }

    pub(super) fn builtin_set(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("set() expects at most one argument"));
        }
        if args.len() == 2 {
            let cls = args.remove(0);
            match cls {
                Value::Builtin(BuiltinFunction::Set) => {}
                Value::Class(class) => {
                    if !self.class_has_builtin_set_base(&class) {
                        let class_name = match &*class.kind() {
                            Object::Class(class_data) => class_data.name.clone(),
                            _ => "<class>".to_string(),
                        };
                        return Err(RuntimeError::new(format!(
                            "set.__new__({}): {} is not a subtype of set",
                            class_name, class_name
                        )));
                    }
                }
                _ => return Err(RuntimeError::new("set() expects at most one argument")),
            }
        }
        let values = if args.is_empty() {
            Vec::new()
        } else if args.len() == 1 {
            match &args[0] {
                Value::Instance(instance) => {
                    if let Some(backing) = self.instance_backing_set(instance) {
                        match &*backing.kind() {
                            Object::Set(values) => values.to_vec(),
                            _ => self.collect_iterable_values(args.remove(0))?,
                        }
                    } else {
                        self.collect_iterable_values(args.remove(0))?
                    }
                }
                _ => self.collect_iterable_values(args.remove(0))?,
            }
        } else {
            return Err(RuntimeError::new("set() expects at most one argument"));
        };
        Ok(self.heap.alloc_set(dedup_hashable_values(values)?))
    }

    pub(super) fn builtin_frozenset(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "frozenset() expects at most one argument",
            ));
        }
        if args.len() == 2 {
            let cls = args.remove(0);
            match cls {
                Value::Builtin(BuiltinFunction::FrozenSet) => {}
                Value::Class(class) => {
                    if !self.class_has_builtin_frozenset_base(&class) {
                        let class_name = match &*class.kind() {
                            Object::Class(class_data) => class_data.name.clone(),
                            _ => "<class>".to_string(),
                        };
                        return Err(RuntimeError::new(format!(
                            "frozenset.__new__({}): {} is not a subtype of frozenset",
                            class_name, class_name
                        )));
                    }
                }
                _ => {
                    return Err(RuntimeError::new(
                        "frozenset() expects at most one argument",
                    ));
                }
            }
        }
        let values = if args.is_empty() {
            Vec::new()
        } else if args.len() == 1 {
            match &args[0] {
                Value::Instance(instance) => {
                    if let Some(backing) = self.instance_backing_frozenset(instance) {
                        match &*backing.kind() {
                            Object::FrozenSet(values) => values.to_vec(),
                            _ => self.collect_iterable_values(args.remove(0))?,
                        }
                    } else {
                        self.collect_iterable_values(args.remove(0))?
                    }
                }
                _ => self.collect_iterable_values(args.remove(0))?,
            }
        } else {
            return Err(RuntimeError::new(
                "frozenset() expects at most one argument",
            ));
        };
        Ok(self.heap.alloc_frozenset(dedup_hashable_values(values)?))
    }

    pub(super) fn builtin_set_reduce(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "set.__reduce__() takes exactly one argument",
            ));
        }
        let receiver = args[0].clone();
        let (constructor, values, state) = match &receiver {
            Value::Set(set_obj) => {
                let values = match &*set_obj.kind() {
                    Object::Set(entries) => entries.to_vec(),
                    _ => return Err(RuntimeError::new("set.__reduce__() invalid receiver")),
                };
                (Value::Builtin(BuiltinFunction::Set), values, Value::None)
            }
            Value::FrozenSet(set_obj) => {
                let values = match &*set_obj.kind() {
                    Object::FrozenSet(entries) => entries.to_vec(),
                    _ => return Err(RuntimeError::new("frozenset.__reduce__() invalid receiver")),
                };
                (
                    Value::Builtin(BuiltinFunction::FrozenSet),
                    values,
                    Value::None,
                )
            }
            Value::Instance(instance) => {
                if let Some(set_obj) = self.instance_backing_set(instance) {
                    let values = match &*set_obj.kind() {
                        Object::Set(entries) => entries.to_vec(),
                        _ => return Err(RuntimeError::new("set.__reduce__() invalid receiver")),
                    };
                    let constructor = self
                        .class_of_value(&receiver)
                        .map(Value::Class)
                        .unwrap_or(Value::Builtin(BuiltinFunction::Set));
                    let state = self.builtin_object_getstate(
                        vec![Value::Instance(instance.clone())],
                        HashMap::new(),
                    )?;
                    (constructor, values, state)
                } else if let Some(set_obj) = self.instance_backing_frozenset(instance) {
                    let values = match &*set_obj.kind() {
                        Object::FrozenSet(entries) => entries.to_vec(),
                        _ => {
                            return Err(RuntimeError::new(
                                "frozenset.__reduce__() invalid receiver",
                            ));
                        }
                    };
                    let constructor = self
                        .class_of_value(&receiver)
                        .map(Value::Class)
                        .unwrap_or(Value::Builtin(BuiltinFunction::FrozenSet));
                    let state = self.builtin_object_getstate(
                        vec![Value::Instance(instance.clone())],
                        HashMap::new(),
                    )?;
                    (constructor, values, state)
                } else {
                    return Err(RuntimeError::new("set.__reduce__() invalid receiver"));
                }
            }
            _ => return Err(RuntimeError::new("set.__reduce__() invalid receiver")),
        };
        let constructor_args = self.heap.alloc_tuple(vec![self.heap.alloc_list(values)]);
        Ok(self
            .heap
            .alloc_tuple(vec![constructor, constructor_args, state]))
    }

    pub(super) fn builtin_min(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_min_max(args, kwargs, Ordering::Less, "min")
    }

    pub(super) fn builtin_max(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_min_max(args, kwargs, Ordering::Greater, "max")
    }

    pub(super) fn builtin_sum(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("sum() expects 1-2 arguments"));
        }
        let start_kw = kwargs.remove("start");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "sum() got an unexpected keyword argument",
            ));
        }
        if start_kw.is_some() && args.len() > 1 {
            return Err(RuntimeError::new("sum() got multiple values"));
        }
        let start = if let Some(value) = start_kw {
            value
        } else if args.len() == 2 {
            args.remove(1)
        } else {
            Value::Int(0)
        };
        let values = self.collect_iterable_values(args.remove(0))?;
        let mut total = start;
        for value in values {
            total = self.binary_add_runtime(total, value)?;
        }
        Ok(total)
    }

    pub(super) fn builtin_round(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("round() expects 1-2 arguments"));
        }

        let ndigits_kw = kwargs.remove("ndigits");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "round() got an unexpected keyword argument",
            ));
        }
        if ndigits_kw.is_some() && args.len() == 2 {
            return Err(RuntimeError::new(
                "round() got multiple values for argument 'ndigits'",
            ));
        }

        let number = args.remove(0);
        let ndigits_value = ndigits_kw.or_else(|| args.pop());

        match number.clone() {
            Value::Int(_) | Value::Bool(_) | Value::BigInt(_) => {
                let value = value_to_bigint(number)?;
                if let Some(raw_digits) = ndigits_value {
                    let digits = value_to_int(raw_digits)?;
                    self.round_integral_with_ndigits(value, digits)
                } else {
                    Ok(value_from_bigint(value))
                }
            }
            Value::Float(value) => {
                if let Some(raw_digits) = ndigits_value {
                    let digits = value_to_int(raw_digits)?;
                    Ok(Value::Float(round_float_with_ndigits(value, digits)))
                } else {
                    self.round_float_to_int(value)
                }
            }
            other => self.call_round_dunder(other, ndigits_value),
        }
    }

    pub(super) fn parse_int_format_spec(
        &self,
        spec: &str,
    ) -> Result<(char, bool, bool, usize, char), RuntimeError> {
        if spec.is_empty() {
            return Ok(('-', false, false, 0, 'd'));
        }
        let mut chars = spec.chars().peekable();
        let mut sign_style = '-';
        if matches!(chars.peek(), Some('+') | Some('-') | Some(' ')) {
            sign_style = chars.next().unwrap_or('-');
        }
        let mut alternate = false;
        while matches!(chars.peek(), Some('#')) {
            alternate = true;
            chars.next();
        }
        let mut zero_pad = false;
        if matches!(chars.peek(), Some('0')) {
            zero_pad = true;
            chars.next();
        }
        let mut width = 0usize;
        while let Some(ch) = chars.peek().copied() {
            if ch.is_ascii_digit() {
                width = width
                    .saturating_mul(10)
                    .saturating_add((ch as u8 - b'0') as usize);
                chars.next();
            } else {
                break;
            }
        }
        let ty = chars.next().unwrap_or('d');
        if chars.next().is_some() || !matches!(ty, 'd' | 'o' | 'x' | 'X' | 'b') {
            return Err(RuntimeError::new(format!(
                "unsupported format string passed to int.__format__: '{spec}'"
            )));
        }
        Ok((sign_style, alternate, zero_pad, width, ty))
    }

    pub(super) fn format_bigint_with_spec(
        &self,
        value: &BigInt,
        spec: &str,
    ) -> Result<String, RuntimeError> {
        let (sign_style, alternate, zero_pad, width, ty) = self.parse_int_format_spec(spec)?;
        let is_negative = value.is_negative();
        let abs_value = value.abs();
        let mut digits = match ty {
            'd' => abs_value.to_string(),
            'o' => abs_value
                .to_str_radix(8)
                .ok_or_else(|| RuntimeError::new("failed to format integer"))?,
            'x' | 'X' => abs_value
                .to_str_radix(16)
                .ok_or_else(|| RuntimeError::new("failed to format integer"))?,
            'b' => abs_value
                .to_str_radix(2)
                .ok_or_else(|| RuntimeError::new("failed to format integer"))?,
            _ => unreachable!(),
        };
        if ty == 'X' {
            digits = digits.to_ascii_uppercase();
        }
        let sign = if is_negative {
            "-"
        } else {
            match sign_style {
                '+' => "+",
                ' ' => " ",
                _ => "",
            }
        };
        let prefix = if alternate {
            match ty {
                'o' => "0o",
                'x' => "0x",
                'X' => "0X",
                'b' => "0b",
                _ => "",
            }
        } else {
            ""
        };
        let base_len = sign.len() + prefix.len() + digits.len();
        if width <= base_len {
            return Ok(format!("{sign}{prefix}{digits}"));
        }
        let pad_len = width - base_len;
        if zero_pad {
            Ok(format!("{sign}{prefix}{}{digits}", "0".repeat(pad_len)))
        } else {
            Ok(format!("{}{sign}{prefix}{digits}", " ".repeat(pad_len)))
        }
    }

    pub(super) fn builtin_format(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("format() expects 1-2 arguments"));
        }
        let value = args.remove(0);
        let spec = if args.is_empty() {
            String::new()
        } else {
            match args.remove(0) {
                Value::Str(text) => text,
                _ => return Err(RuntimeError::new("format() argument 2 must be str")),
            }
        };
        if let Some(proxy_result) = self.cpython_proxy_format(&value, &spec) {
            return proxy_result.map(Value::Str);
        }

        let out = match &value {
            Value::Int(number) => {
                self.format_bigint_with_spec(&BigInt::from_i64(*number), &spec)?
            }
            Value::Bool(flag) => {
                let int_value = if *flag { 1 } else { 0 };
                self.format_bigint_with_spec(&BigInt::from_i64(int_value), &spec)?
            }
            Value::BigInt(number) => self.format_bigint_with_spec(number, &spec)?,
            Value::Str(text) => {
                if spec.is_empty() {
                    text.clone()
                } else {
                    return Err(RuntimeError::new(format!(
                        "unsupported format string passed to str.__format__: '{spec}'"
                    )));
                }
            }
            Value::Float(number) => {
                if spec.is_empty() {
                    format!("{number}")
                } else {
                    return Err(RuntimeError::new(format!(
                        "unsupported format string passed to float.__format__: '{spec}'"
                    )));
                }
            }
            Value::Instance(_)
            | Value::Class(_)
            | Value::Module(_)
            | Value::Function(_)
            | Value::BoundMethod(_)
            | Value::Generator(_)
            | Value::Iterator(_) => {
                let Some(method) = self.lookup_bound_special_method(&value, "__format__")? else {
                    return Err(RuntimeError::new("type doesn't define __format__ method"));
                };
                let formatted =
                    match self.call_internal(method, vec![Value::Str(spec)], HashMap::new())? {
                        InternalCallOutcome::Value(result) => result,
                        InternalCallOutcome::CallerExceptionHandled => {
                            return Err(self.runtime_error_from_active_exception("format() failed"));
                        }
                    };
                match formatted {
                    Value::Str(text) => text,
                    _ => {
                        return Err(RuntimeError::new(
                            "__format__ must return str, not non-str value",
                        ));
                    }
                }
            }
            _ => {
                if spec.is_empty() {
                    format_value(&value)
                } else {
                    return Err(RuntimeError::new(format!(
                        "unsupported format string passed to object.__format__: '{spec}'"
                    )));
                }
            }
        };
        Ok(Value::Str(out))
    }

    pub(super) fn round_integral_with_ndigits(
        &self,
        value: BigInt,
        ndigits: i64,
    ) -> Result<Value, RuntimeError> {
        if ndigits >= 0 {
            return Ok(value_from_bigint(value));
        }

        let factor = BigInt::from_i64(10).pow_u64((-ndigits) as u64);
        let is_negative = value.is_negative();
        let abs_value = value.abs();
        let (mut quotient, remainder) = abs_value
            .div_mod_floor(&factor)
            .ok_or_else(|| RuntimeError::new("round() internal error"))?;
        let should_increment = match remainder.mul_small(2).cmp_total(&factor) {
            Ordering::Greater => true,
            Ordering::Equal => self.bigint_is_odd(&quotient)?,
            Ordering::Less => false,
        };
        if should_increment {
            quotient = quotient.add(&BigInt::one());
        }

        let mut rounded = quotient.mul(&factor);
        if is_negative {
            rounded = rounded.negated();
        }
        Ok(value_from_bigint(rounded))
    }

    pub(super) fn round_float_to_int(&self, value: f64) -> Result<Value, RuntimeError> {
        if value.is_nan() {
            return Err(RuntimeError::new("cannot convert float NaN to integer"));
        }
        if value.is_infinite() {
            return Err(RuntimeError::new(
                "cannot convert float infinity to integer",
            ));
        }
        let rounded = value.round_ties_even();
        let bigint = BigInt::from_f64_integral(rounded)
            .ok_or_else(|| RuntimeError::new("cannot convert float to integer"))?;
        Ok(value_from_bigint(bigint))
    }

    pub(super) fn call_round_dunder(
        &mut self,
        number: Value,
        ndigits_value: Option<Value>,
    ) -> Result<Value, RuntimeError> {
        let Some(round_method) = self.lookup_bound_special_method(&number, "__round__")? else {
            return Err(RuntimeError::new("type doesn't define __round__ method"));
        };
        let mut call_args = Vec::new();
        if let Some(ndigits) = ndigits_value {
            call_args.push(ndigits);
        }
        match self.call_internal(round_method, call_args, HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("round() failed"))
            }
        }
    }

    pub(super) fn bigint_is_odd(&self, value: &BigInt) -> Result<bool, RuntimeError> {
        let divisor = BigInt::from_i64(2);
        let (_quotient, remainder) = value
            .div_mod_floor(&divisor)
            .ok_or_else(|| RuntimeError::new("round() internal error"))?;
        Ok(!remainder.is_zero())
    }

    pub(super) fn builtin_min_max(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
        preferred: Ordering,
        name: &str,
    ) -> Result<Value, RuntimeError> {
        let default = kwargs.remove("default");
        let key_func = kwargs.remove("key").unwrap_or(Value::None);
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(format!(
                "{name}() got an unexpected keyword argument"
            )));
        }
        if args.is_empty() {
            return Err(RuntimeError::new(format!(
                "{name}() expected at least 1 argument, got 0"
            )));
        }

        let values = if args.len() == 1 {
            self.collect_iterable_values(args.remove(0))?
        } else {
            if default.is_some() {
                return Err(RuntimeError::new(format!(
                    "Cannot specify a default for {name}() with multiple positional arguments"
                )));
            }
            args
        };

        if values.is_empty() {
            if let Some(default_value) = default {
                return Ok(default_value);
            }
            return Err(RuntimeError::new(format!(
                "{name}() arg is an empty sequence"
            )));
        }

        let mut iter = values.into_iter();
        let mut best_value = iter.next().expect("values is non-empty");
        let mut best_key = if matches!(key_func, Value::None) {
            best_value.clone()
        } else {
            match self.call_internal(key_func.clone(), vec![best_value.clone()], HashMap::new())? {
                InternalCallOutcome::Value(key) => key,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("key function raised"));
                }
            }
        };

        for value in iter {
            let key = if matches!(key_func, Value::None) {
                value.clone()
            } else {
                match self.call_internal(key_func.clone(), vec![value.clone()], HashMap::new())? {
                    InternalCallOutcome::Value(key) => key,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(RuntimeError::new("key function raised"));
                    }
                }
            };
            if self.compare_sort_keys(key.clone(), best_key.clone())? == preferred {
                best_value = value;
                best_key = key;
            }
        }

        Ok(best_value)
    }

    pub(super) fn builtin_sorted(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new("sorted() expects one iterable argument"));
        }
        let reverse = match kwargs.remove("reverse") {
            Some(value) => self.truthy_from_value(&value)?,
            None => false,
        };
        let key_func = kwargs.remove("key").unwrap_or(Value::None);
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "sorted() got an unexpected keyword argument",
            ));
        }

        let mut values = self.collect_iterable_values(args.remove(0))?;
        self.sort_values_with_optional_key(&mut values, &key_func, reverse)?;
        Ok(self.heap.alloc_list(values))
    }

    pub(super) fn sort_values_with_optional_key(
        &mut self,
        values: &mut Vec<Value>,
        key_func: &Value,
        reverse: bool,
    ) -> Result<(), RuntimeError> {
        if matches!(key_func, Value::None) {
            let mut compare_error: Option<RuntimeError> = None;
            values.sort_by(
                |left, right| match self.compare_order_with_fallback_ref(left, right) {
                    Ok(ordering) => ordering,
                    Err(err) => {
                        compare_error = Some(err);
                        Ordering::Equal
                    }
                },
            );
            if let Some(err) = compare_error {
                return Err(err);
            }
            if reverse {
                values.reverse();
            }
            return Ok(());
        }

        let mut slots: Vec<Option<Value>> = values.drain(..).map(Some).collect();
        let mut keys = Vec::with_capacity(slots.len());
        for slot in &slots {
            let value = slot.as_ref().expect("slot populated");
            let key =
                match self.call_internal(key_func.clone(), vec![value.clone()], HashMap::new())? {
                    InternalCallOutcome::Value(key) => key,
                    InternalCallOutcome::CallerExceptionHandled => {
                        *values = slots
                            .into_iter()
                            .map(|slot| slot.expect("slot populated"))
                            .collect();
                        return Err(RuntimeError::new("key function raised"));
                    }
                };
            keys.push(key);
        }

        let mut order: Vec<usize> = (0..keys.len()).collect();
        let mut compare_error: Option<RuntimeError> = None;
        order.sort_by(|left_idx, right_idx| {
            match self.compare_sort_keys_ref(&keys[*left_idx], &keys[*right_idx]) {
                Ok(ordering) => ordering,
                Err(err) => {
                    compare_error = Some(err);
                    Ordering::Equal
                }
            }
        });
        if let Some(err) = compare_error {
            *values = slots
                .into_iter()
                .map(|slot| slot.expect("slot populated"))
                .collect();
            return Err(err);
        }
        if reverse {
            order.reverse();
        }

        values.reserve(order.len());
        for index in order {
            values.push(slots[index].take().expect("slot populated"));
        }
        Ok(())
    }

    pub(super) fn compare_sort_keys(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Ordering, RuntimeError> {
        self.compare_sort_keys_ref(&left, &right)
    }

    pub(super) fn compare_sort_keys_ref(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Ordering, RuntimeError> {
        if let Some(ordering) = self.compare_cmp_to_key_wrappers(left, right)? {
            return Ok(ordering);
        }
        self.compare_order_with_fallback_ref(left, right)
    }

    pub(super) fn compare_order_with_fallback(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Ordering, RuntimeError> {
        self.compare_order_with_fallback_ref(&left, &right)
    }

    pub(super) fn compare_order_with_fallback_ref(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Ordering, RuntimeError> {
        match compare_order(left.clone(), right.clone()) {
            Ok(ordering) => Ok(ordering),
            Err(_) => {
                if let Some(ordering) = self.compare_sequence_order_for_values(left, right)? {
                    return Ok(ordering);
                }
                self.compare_order_via_richcmp(left.clone(), right.clone())?
                    .ok_or_else(|| RuntimeError::new("unsupported operand type for comparison"))
            }
        }
    }

    pub(super) fn compare_sequence_order_for_values(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Option<Ordering>, RuntimeError> {
        match (left, right) {
            (Value::Tuple(left_obj), Value::Tuple(right_obj)) => Ok(Some(
                self.compare_sequence_objects_order(left_obj, right_obj)?,
            )),
            (Value::List(left_obj), Value::List(right_obj)) => Ok(Some(
                self.compare_sequence_objects_order(left_obj, right_obj)?,
            )),
            _ => Ok(None),
        }
    }

    pub(super) fn compare_sequence_objects_order(
        &mut self,
        left_obj: &ObjRef,
        right_obj: &ObjRef,
    ) -> Result<Ordering, RuntimeError> {
        let (left_len, right_len) = {
            let left_kind = left_obj.kind();
            let right_kind = right_obj.kind();
            match (&*left_kind, &*right_kind) {
                (Object::List(left), Object::List(right)) => (left.len(), right.len()),
                (Object::Tuple(left), Object::Tuple(right)) => (left.len(), right.len()),
                _ => {
                    return Err(RuntimeError::new("unsupported operand type for comparison"));
                }
            }
        };

        for idx in 0..left_len.min(right_len) {
            let (left_item, right_item) = {
                let left_kind = left_obj.kind();
                let right_kind = right_obj.kind();
                match (&*left_kind, &*right_kind) {
                    (Object::List(left), Object::List(right)) => {
                        (left[idx].clone(), right[idx].clone())
                    }
                    (Object::Tuple(left), Object::Tuple(right)) => {
                        (left[idx].clone(), right[idx].clone())
                    }
                    _ => {
                        return Err(RuntimeError::new("unsupported operand type for comparison"));
                    }
                }
            };
            let ordering = self.compare_order_with_fallback_ref(&left_item, &right_item)?;
            if ordering != Ordering::Equal {
                return Ok(ordering);
            }
        }
        Ok(left_len.cmp(&right_len))
    }

    pub(super) fn compare_order_via_richcmp(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Option<Ordering>, RuntimeError> {
        const PY_LT: i32 = 0;
        const PY_GT: i32 = 4;
        if std::env::var_os("PYRS_TRACE_COMPARE_ORDER").is_some() {
            eprintln!(
                "[cmp-order] left_type={} right_type={} left_proxy={:p} right_proxy={:p}",
                self.value_type_name_for_error(&left),
                self.value_type_name_for_error(&right),
                Self::cpython_proxy_raw_ptr_from_value(&left).unwrap_or(std::ptr::null_mut()),
                Self::cpython_proxy_raw_ptr_from_value(&right).unwrap_or(std::ptr::null_mut())
            );
        }
        if let Some(ordering) = self.compare_order_via_proxy_numeric_bridge(&left, &right)? {
            return Ok(Some(ordering));
        }
        if let Some(result) = self.cpython_proxy_richcmp_bool(&left, &right, PY_LT) {
            match result {
                Ok(true) => return Ok(Some(Ordering::Less)),
                Ok(false) => {}
                Err(err) => {
                    if !runtime_error_matches_exception(&err, "TypeError") {
                        return Err(err);
                    }
                }
            }
        }
        if let Some(result) = self.cpython_proxy_richcmp_bool(&left, &right, PY_GT) {
            match result {
                Ok(true) => return Ok(Some(Ordering::Greater)),
                Ok(false) => {}
                Err(err) => {
                    if !runtime_error_matches_exception(&err, "TypeError") {
                        return Err(err);
                    }
                }
            }
        }
        if let Some(result) = self.cpython_proxy_richcmp_bool(&right, &left, PY_GT) {
            match result {
                Ok(true) => return Ok(Some(Ordering::Less)),
                Ok(false) => {}
                Err(err) => {
                    if !runtime_error_matches_exception(&err, "TypeError") {
                        return Err(err);
                    }
                }
            }
        }
        if let Some(result) = self.cpython_proxy_richcmp_bool(&right, &left, PY_LT) {
            match result {
                Ok(true) => return Ok(Some(Ordering::Greater)),
                Ok(false) => {}
                Err(err) => {
                    if !runtime_error_matches_exception(&err, "TypeError") {
                        return Err(err);
                    }
                }
            }
        }
        if let Some(result) =
            self.call_compare_method_bool(left.clone(), "__lt__", right.clone())?
            && result
        {
            return Ok(Some(Ordering::Less));
        }
        if let Some(result) =
            self.call_compare_method_bool(left.clone(), "__gt__", right.clone())?
            && result
        {
            return Ok(Some(Ordering::Greater));
        }
        if let Some(result) =
            self.call_compare_method_bool(right.clone(), "__gt__", left.clone())?
            && result
        {
            return Ok(Some(Ordering::Less));
        }
        if let Some(result) =
            self.call_compare_method_bool(right.clone(), "__lt__", left.clone())?
            && result
        {
            return Ok(Some(Ordering::Greater));
        }
        Ok(None)
    }

    fn is_runtime_numeric_for_compare(value: &Value) -> bool {
        matches!(
            value,
            Value::Bool(_) | Value::Int(_) | Value::BigInt(_) | Value::Float(_)
        )
    }

    fn normalize_proxy_numeric_for_compare(
        &mut self,
        value: &Value,
    ) -> Result<Option<Value>, RuntimeError> {
        if Self::is_runtime_numeric_for_compare(value) {
            return Ok(Some(value.clone()));
        }
        let Some(_) = Self::cpython_proxy_raw_ptr_from_value(value) else {
            return Ok(None);
        };
        let type_name = self.value_type_name_for_error(value).to_ascii_lowercase();
        let prefer_float =
            type_name.contains("float") || type_name.contains("double") || type_name == "half";
        let try_float = |vm: &mut Vm| match vm.builtin_float(vec![value.clone()], HashMap::new()) {
            Ok(normalized) if Self::is_runtime_numeric_for_compare(&normalized) => Some(normalized),
            Ok(_) | Err(_) => None,
        };
        let try_int = |vm: &mut Vm| match vm.builtin_int(vec![value.clone()], HashMap::new()) {
            Ok(normalized) if Self::is_runtime_numeric_for_compare(&normalized) => Some(normalized),
            Ok(_) | Err(_) => None,
        };

        let primary = if prefer_float {
            try_float(self)
        } else {
            try_int(self)
        };
        if let Some(normalized) = primary {
            return Ok(Some(normalized));
        }
        let secondary = if prefer_float {
            try_int(self)
        } else {
            try_float(self)
        };
        if let Some(normalized) = secondary {
            return Ok(Some(normalized));
        }
        if type_name == "bool"
            && let Ok(normalized) = self.builtin_bool(vec![value.clone()], HashMap::new())
            && Self::is_runtime_numeric_for_compare(&normalized)
        {
            return Ok(Some(normalized));
        }
        Ok(None)
    }

    fn compare_order_via_proxy_numeric_bridge(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Option<Ordering>, RuntimeError> {
        if Self::cpython_proxy_raw_ptr_from_value(left).is_none()
            && Self::cpython_proxy_raw_ptr_from_value(right).is_none()
        {
            return Ok(None);
        }
        let Some(left_normalized) = self.normalize_proxy_numeric_for_compare(left)? else {
            return Ok(None);
        };
        let Some(right_normalized) = self.normalize_proxy_numeric_for_compare(right)? else {
            return Ok(None);
        };
        if !Self::is_runtime_numeric_for_compare(&left_normalized)
            || !Self::is_runtime_numeric_for_compare(&right_normalized)
        {
            return Ok(None);
        }
        match compare_order(left_normalized, right_normalized) {
            Ok(ordering) => Ok(Some(ordering)),
            Err(_) => Ok(None),
        }
    }

    pub(super) fn call_compare_method_bool(
        &mut self,
        receiver: Value,
        method: &str,
        argument: Value,
    ) -> Result<Option<bool>, RuntimeError> {
        let Some(result) = self.call_compare_method_value(receiver, method, argument)? else {
            return Ok(None);
        };
        Ok(Some(self.truthy_from_value(&result)?))
    }

    pub(super) fn call_compare_method_value(
        &mut self,
        receiver: Value,
        method: &str,
        argument: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let callable = if method.starts_with("__") && method.ends_with("__") {
            match self.lookup_bound_special_method(&receiver, method)? {
                Some(callable) => callable,
                None => return Ok(None),
            }
        } else {
            let method_name = Value::Str(method.to_string());
            match self.builtin_getattr(vec![receiver, method_name], HashMap::new()) {
                Ok(callable) => callable,
                Err(err) => {
                    if runtime_error_matches_exception(&err, "AttributeError") {
                        return Ok(None);
                    }
                    return Err(err);
                }
            }
        };
        let result = match self.call_internal(callable, vec![argument], HashMap::new())? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                let active_exception = self
                    .frames
                    .last()
                    .and_then(|frame| frame.active_exception.clone());
                if let Some(active_exception) = active_exception
                    && self
                        .exception_matches(
                            &active_exception,
                            &Value::ExceptionType("TypeError".to_string()),
                        )
                        .unwrap_or(false)
                {
                    self.clear_active_exception();
                    return Ok(None);
                }
                return Err(self.runtime_error_from_active_exception("comparison method raised"));
            }
        };
        if self.is_not_implemented_singleton(&result) {
            return Ok(None);
        }
        Ok(Some(result))
    }

    pub(super) fn compare_lt_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        match compare_lt(left.clone(), right.clone()) {
            Ok(value) => Ok(value),
            Err(err) if err.message == "unsupported operand type for comparison" => Ok(
                Value::Bool(self.compare_order_with_fallback(left, right)? == Ordering::Less),
            ),
            Err(err) => Err(err),
        }
    }

    fn contains_via_iteration(
        &mut self,
        needle: &Value,
        iterator: Value,
    ) -> Result<bool, RuntimeError> {
        loop {
            match self.next_from_iterator_value(&iterator)? {
                GeneratorResumeOutcome::Yield(item) => {
                    let equals = self.compare_eq_runtime(item, needle.clone())?;
                    if self.truthy_from_value(&equals)? {
                        return Ok(true);
                    }
                }
                GeneratorResumeOutcome::Complete(_) => return Ok(false),
                GeneratorResumeOutcome::PropagatedException => {
                    return Err(self.iteration_error_from_state("membership iteration failed")?);
                }
            }
        }
    }

    pub(super) fn compare_in_runtime(
        &mut self,
        needle: Value,
        container: Value,
    ) -> Result<bool, RuntimeError> {
        match compare_in(&needle, &container) {
            Ok(found) => Ok(found),
            Err(err) if err.message == "unsupported operand type for in" => {
                if let Some(contains_method) =
                    self.lookup_bound_special_method(&container, "__contains__")?
                {
                    let contains_result =
                        match self.call_internal(contains_method, vec![needle], HashMap::new())? {
                            InternalCallOutcome::Value(value) => value,
                            InternalCallOutcome::CallerExceptionHandled => {
                                return Err(self
                                    .runtime_error_from_active_exception("__contains__() failed"));
                            }
                        };
                    return self.truthy_from_value(&contains_result);
                }
                let iterator = self.to_iterator_value(container.clone()).map_err(|_| {
                    RuntimeError::new(format!(
                        "TypeError: argument of type '{}' is not iterable",
                        self.value_type_name_for_error(&container)
                    ))
                })?;
                self.contains_via_iteration(&needle, iterator)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn call_binary_special_method(
        &mut self,
        receiver: &Value,
        method_name: &str,
        arg: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some(callable) = self.lookup_bound_special_method(receiver, method_name)? else {
            return Ok(None);
        };
        match self.call_internal(callable, vec![arg], HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(Some(value)),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self
                    .runtime_error_from_active_exception("binary operator special method raised"))
            }
        }
    }

    pub(super) fn call_unary_special_method(
        &mut self,
        receiver: &Value,
        method_name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some(callable) = self.lookup_bound_special_method(receiver, method_name)? else {
            return Ok(None);
        };
        match self.call_internal(callable, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(Some(value)),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self
                    .runtime_error_from_active_exception("unary operator special method raised"))
            }
        }
    }

    fn is_not_implemented_singleton(&self, value: &Value) -> bool {
        self.builtins
            .get("NotImplemented")
            .is_some_and(|not_implemented| value == not_implemented)
    }

    pub(super) fn unary_neg_runtime(&mut self, value: Value) -> Result<Value, RuntimeError> {
        match neg_value(value.clone()) {
            Ok(result) => Ok(result),
            Err(err) if err.message.contains("unsupported operand type") => {
                if let Some(proxy_result) = self.cpython_proxy_negative(&value) {
                    return proxy_result;
                }
                if let Some(result) = self.call_unary_special_method(&value, "__neg__")?
                    && !self.is_not_implemented_singleton(&result)
                {
                    return Ok(result);
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn unary_pos_runtime(&mut self, value: Value) -> Result<Value, RuntimeError> {
        match pos_value(value.clone()) {
            Ok(result) => Ok(result),
            Err(err) if err.message.contains("unsupported operand type") => {
                if let Some(proxy_result) = self.cpython_proxy_positive(&value) {
                    return proxy_result;
                }
                if let Some(result) = self.call_unary_special_method(&value, "__pos__")?
                    && !self.is_not_implemented_singleton(&result)
                {
                    return Ok(result);
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn unary_invert_runtime(&mut self, value: Value) -> Result<Value, RuntimeError> {
        match invert_value(value.clone()) {
            Ok(result) => Ok(result),
            Err(err) if err.message.contains("unsupported operand type") => {
                if let Some(proxy_result) = self.cpython_proxy_invert(&value) {
                    return proxy_result;
                }
                if let Some(result) = self.call_unary_special_method(&value, "__invert__")?
                    && !self.is_not_implemented_singleton(&result)
                {
                    return Ok(result);
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn binary_div_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        let trace = std::env::var_os("PYRS_TRACE_BINARY_DIV_RUNTIME").is_some();
        match div_values(left.clone(), right.clone()) {
            Ok(value) => Ok(value),
            Err(err)
                if err.message.contains("unsupported operand type")
                    && err.message.contains("for /") =>
            {
                if trace {
                    eprintln!(
                        "[bin-div] numeric fallback left_type={} right_type={} left={} right={}",
                        self.value_type_name_for_error(&left),
                        self.value_type_name_for_error(&right),
                        format_repr(&left),
                        format_repr(&right)
                    );
                }
                if let Some(proxy_result) = self.cpython_proxy_true_divide(&left, &right) {
                    if trace {
                        match &proxy_result {
                            Ok(value) => {
                                eprintln!(
                                    "[bin-div] used proxy numeric slot -> {}",
                                    format_repr(value)
                                )
                            }
                            Err(proxy_err) => {
                                eprintln!(
                                    "[bin-div] proxy numeric slot error -> {}",
                                    proxy_err.message
                                )
                            }
                        }
                    }
                    return proxy_result;
                }
                if let Some(value) =
                    self.call_binary_special_method(&left, "__truediv__", right.clone())?
                    && !self.is_not_implemented_singleton(&value)
                {
                    if trace {
                        eprintln!("[bin-div] used __truediv__ -> {}", format_repr(&value));
                    }
                    return Ok(value);
                }
                if let Some(value) =
                    self.call_binary_special_method(&right, "__rtruediv__", left)?
                    && !self.is_not_implemented_singleton(&value)
                {
                    if trace {
                        eprintln!("[bin-div] used __rtruediv__ -> {}", format_repr(&value));
                    }
                    return Ok(value);
                }
                if trace {
                    eprintln!("[bin-div] no usable __truediv__/__rtruediv__");
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn binary_add_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        match add_values(left.clone(), right.clone(), &self.heap) {
            Ok(value) => Ok(value),
            Err(err)
                if err.message.contains("unsupported operand type")
                    && err.message.contains("for +") =>
            {
                if let Some(proxy_result) = self.cpython_proxy_add(&left, &right) {
                    return proxy_result;
                }
                if let Some(value) =
                    self.call_binary_special_method(&left, "__add__", right.clone())?
                    && !self.is_not_implemented_singleton(&value)
                {
                    return Ok(value);
                }
                if let Some(value) = self.call_binary_special_method(&right, "__radd__", left)?
                    && !self.is_not_implemented_singleton(&value)
                {
                    return Ok(value);
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn binary_mul_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        let trace = std::env::var_os("PYRS_TRACE_BINARY_MUL_RUNTIME").is_some();
        match mul_values(left.clone(), right.clone(), &self.heap) {
            Ok(value) => Ok(value),
            Err(err)
                if err.message.contains("unsupported operand type")
                    && err.message.contains("for *") =>
            {
                if trace {
                    eprintln!(
                        "[bin-mul] numeric fallback left_type={} right_type={} left={} right={}",
                        self.value_type_name_for_error(&left),
                        self.value_type_name_for_error(&right),
                        format_repr(&left),
                        format_repr(&right)
                    );
                }
                if let Some(proxy_result) = self.cpython_proxy_multiply(&left, &right) {
                    if trace {
                        match &proxy_result {
                            Ok(value) => {
                                eprintln!(
                                    "[bin-mul] used proxy numeric slot -> {}",
                                    format_repr(value)
                                )
                            }
                            Err(proxy_err) => {
                                eprintln!(
                                    "[bin-mul] proxy numeric slot error -> {}",
                                    proxy_err.message
                                )
                            }
                        }
                    }
                    return proxy_result;
                }
                if let Some(value) =
                    self.call_binary_special_method(&left, "__mul__", right.clone())?
                    && !self.is_not_implemented_singleton(&value)
                {
                    if trace {
                        eprintln!("[bin-mul] used __mul__ -> {}", format_repr(&value));
                    }
                    return Ok(value);
                }
                if let Some(value) = self.call_binary_special_method(&right, "__rmul__", left)?
                    && !self.is_not_implemented_singleton(&value)
                {
                    if trace {
                        eprintln!("[bin-mul] used __rmul__ -> {}", format_repr(&value));
                    }
                    return Ok(value);
                }
                if trace {
                    eprintln!("[bin-mul] no usable __mul__/__rmul__");
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn binary_matmul_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        match matmul_values(left.clone(), right.clone()) {
            Ok(value) => Ok(value),
            Err(err)
                if err.message.contains("unsupported operand type")
                    && err.message.contains("for @") =>
            {
                if let Some(proxy_result) = self.cpython_proxy_matmul(&left, &right) {
                    return proxy_result;
                }
                if let Some(value) =
                    self.call_binary_special_method(&left, "__matmul__", right.clone())?
                    && !self.is_not_implemented_singleton(&value)
                {
                    return Ok(value);
                }
                if let Some(value) = self.call_binary_special_method(&right, "__rmatmul__", left)?
                    && !self.is_not_implemented_singleton(&value)
                {
                    return Ok(value);
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn binary_sub_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        let trace = std::env::var_os("PYRS_TRACE_BINARY_SUB_RUNTIME").is_some();
        match sub_values(left.clone(), right.clone(), &self.heap) {
            Ok(value) => Ok(value),
            Err(err)
                if err.message.contains("unsupported operand type")
                    && err.message.contains("for -") =>
            {
                if trace {
                    eprintln!(
                        "[bin-sub] numeric fallback left_type={} right_type={} left={} right={}",
                        self.value_type_name_for_error(&left),
                        self.value_type_name_for_error(&right),
                        format_repr(&left),
                        format_repr(&right)
                    );
                }
                if let Some(proxy_result) = self.cpython_proxy_subtract(&left, &right) {
                    if trace {
                        match &proxy_result {
                            Ok(value) => {
                                eprintln!(
                                    "[bin-sub] used proxy numeric slot -> {}",
                                    format_repr(value)
                                )
                            }
                            Err(err) => {
                                eprintln!("[bin-sub] proxy numeric slot error -> {}", err.message)
                            }
                        }
                    }
                    return proxy_result;
                }
                if let Some(value) =
                    self.call_binary_special_method(&left, "__sub__", right.clone())?
                    && !self.is_not_implemented_singleton(&value)
                {
                    if trace {
                        eprintln!("[bin-sub] used __sub__ -> {}", format_repr(&value));
                    }
                    return Ok(value);
                }
                if let Some(value) = self.call_binary_special_method(&right, "__rsub__", left)?
                    && !self.is_not_implemented_singleton(&value)
                {
                    if trace {
                        eprintln!("[bin-sub] used __rsub__ -> {}", format_repr(&value));
                    }
                    return Ok(value);
                }
                if trace {
                    eprintln!("[bin-sub] no usable __sub__/__rsub__");
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn binary_or_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        let trace = std::env::var_os("PYRS_TRACE_BINARY_OR_RUNTIME").is_some();
        match or_values(left.clone(), right.clone(), &self.heap) {
            Ok(value) => Ok(value),
            Err(err)
                if err.message.contains("unsupported operand type")
                    && err.message.contains("for |") =>
            {
                if trace {
                    eprintln!(
                        "[bin-or] unsupported fallback left={} right={}",
                        format_repr(&left),
                        format_repr(&right)
                    );
                }
                if let Some(value) =
                    self.call_binary_special_method(&left, "__or__", right.clone())?
                    && !self.is_not_implemented_singleton(&value)
                {
                    if trace {
                        eprintln!(
                            "[bin-or] used __or__ -> repr={} type={}",
                            format_repr(&value),
                            self.value_type_name_for_error(&value)
                        );
                    }
                    return Ok(value);
                }
                if trace {
                    eprintln!("[bin-or] __or__ missing/NotImplemented");
                }
                if let Some(value) = self.call_binary_special_method(&right, "__ror__", left)?
                    && !self.is_not_implemented_singleton(&value)
                {
                    if trace {
                        eprintln!(
                            "[bin-or] used __ror__ -> repr={} type={}",
                            format_repr(&value),
                            self.value_type_name_for_error(&value)
                        );
                    }
                    return Ok(value);
                }
                if trace {
                    eprintln!("[bin-or] __ror__ missing/NotImplemented");
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn binary_xor_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        let trace = std::env::var_os("PYRS_TRACE_BINARY_XOR_RUNTIME").is_some();
        match xor_values(left.clone(), right.clone(), &self.heap) {
            Ok(value) => Ok(value),
            Err(err)
                if err.message.contains("unsupported operand type")
                    && err.message.contains("for ^") =>
            {
                if trace {
                    eprintln!(
                        "[bin-xor] unsupported fallback left={} right={}",
                        format_repr(&left),
                        format_repr(&right)
                    );
                }
                if let Some(value) =
                    self.call_binary_special_method(&left, "__xor__", right.clone())?
                    && !self.is_not_implemented_singleton(&value)
                {
                    if trace {
                        eprintln!(
                            "[bin-xor] used __xor__ -> repr={} type={}",
                            format_repr(&value),
                            self.value_type_name_for_error(&value)
                        );
                    }
                    return Ok(value);
                }
                if trace {
                    eprintln!("[bin-xor] __xor__ missing/NotImplemented");
                }
                if let Some(value) = self.call_binary_special_method(&right, "__rxor__", left)?
                    && !self.is_not_implemented_singleton(&value)
                {
                    if trace {
                        eprintln!(
                            "[bin-xor] used __rxor__ -> repr={} type={}",
                            format_repr(&value),
                            self.value_type_name_for_error(&value)
                        );
                    }
                    return Ok(value);
                }
                if trace {
                    eprintln!("[bin-xor] __rxor__ missing/NotImplemented");
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn compare_eq_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        const PY_EQ: i32 = 2;
        let left_proxy_class =
            matches!(left, Value::Class(_)) && Self::cpython_proxy_raw_ptr_from_value(&left).is_some();
        let right_proxy_class =
            matches!(right, Value::Class(_)) && Self::cpython_proxy_raw_ptr_from_value(&right).is_some();
        if !left_proxy_class && !right_proxy_class
            && let Some(result) = self.cpython_proxy_richcmp_value(&left, &right, PY_EQ)
        {
            return result;
        }
        if let Some(result) = self.compare_eq_via_bound_method(&left, &right) {
            return Ok(Value::Bool(result));
        }
        if let Some(result) = self.compare_eq_via_int_backing(&left, &right) {
            return Ok(Value::Bool(result));
        }
        if let Some(result) = self.compare_eq_via_float_backing(&left, &right) {
            return Ok(Value::Bool(result));
        }
        if let Some(result) = self.compare_eq_via_complex_backing(&left, &right) {
            return Ok(Value::Bool(result));
        }
        if let Some(result) = self.compare_eq_via_str_backing(&left, &right) {
            return Ok(Value::Bool(result));
        }
        if let Some(result) = self.compare_eq_via_dict_backing(&left, &right) {
            return Ok(Value::Bool(result));
        }
        if let Some(result) = self.compare_eq_via_set_backing(&left, &right) {
            return Ok(Value::Bool(result));
        }
        if let Some(result) = self.compare_eq_via_tuple_backing(&left, &right)? {
            return Ok(Value::Bool(result));
        }
        if let Some(result) = self.compare_eq_via_list_backing(&left, &right)? {
            return Ok(Value::Bool(result));
        }
        if matches!(left, Value::Instance(_) | Value::Class(_))
            && let Some(result) =
                self.call_compare_method_value(left.clone(), "__eq__", right.clone())?
        {
            return Ok(result);
        }
        if matches!(right, Value::Instance(_) | Value::Class(_))
            && let Some(result) =
                self.call_compare_method_value(right.clone(), "__eq__", left.clone())?
        {
            return Ok(result);
        }
        Ok(Value::Bool(left == right))
    }

    pub(super) fn compare_ne_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        const PY_NE: i32 = 3;
        let left_proxy_class =
            matches!(left, Value::Class(_)) && Self::cpython_proxy_raw_ptr_from_value(&left).is_some();
        let right_proxy_class =
            matches!(right, Value::Class(_)) && Self::cpython_proxy_raw_ptr_from_value(&right).is_some();
        if !left_proxy_class && !right_proxy_class
            && let Some(result) = self.cpython_proxy_richcmp_value(&left, &right, PY_NE)
        {
            return result;
        }
        if let Some(result) = self.compare_eq_via_bound_method(&left, &right) {
            return Ok(Value::Bool(!result));
        }
        if let Some(result) = self.compare_eq_via_float_backing(&left, &right) {
            return Ok(Value::Bool(!result));
        }
        if let Some(result) = self.compare_eq_via_complex_backing(&left, &right) {
            return Ok(Value::Bool(!result));
        }
        if let Some(result) = self.compare_eq_via_str_backing(&left, &right) {
            return Ok(Value::Bool(!result));
        }
        if let Some(result) = self.compare_eq_via_dict_backing(&left, &right) {
            return Ok(Value::Bool(!result));
        }
        if let Some(result) = self.compare_eq_via_set_backing(&left, &right) {
            return Ok(Value::Bool(!result));
        }
        if let Some(result) = self.compare_eq_via_tuple_backing(&left, &right)? {
            return Ok(Value::Bool(!result));
        }
        if let Some(result) = self.compare_eq_via_list_backing(&left, &right)? {
            return Ok(Value::Bool(!result));
        }
        if matches!(left, Value::Instance(_) | Value::Class(_))
            && let Some(result) =
                self.call_compare_method_value(left.clone(), "__ne__", right.clone())?
        {
            return Ok(result);
        }
        if matches!(right, Value::Instance(_) | Value::Class(_))
            && let Some(result) =
                self.call_compare_method_value(right.clone(), "__ne__", left.clone())?
        {
            return Ok(result);
        }
        let eq = self.compare_eq_runtime(left, right)?;
        Ok(Value::Bool(!self.truthy_from_value(&eq)?))
    }

    pub(super) fn compare_eq_via_bound_method(&self, left: &Value, right: &Value) -> Option<bool> {
        let (Value::BoundMethod(left_method), Value::BoundMethod(right_method)) = (left, right)
        else {
            return None;
        };
        let left_kind = left_method.kind();
        let right_kind = right_method.kind();
        let (Object::BoundMethod(left_data), Object::BoundMethod(right_data)) =
            (&*left_kind, &*right_kind)
        else {
            return Some(false);
        };
        let function_equal = if left_data.function.id() == right_data.function.id() {
            true
        } else {
            match (&*left_data.function.kind(), &*right_data.function.kind()) {
                (Object::NativeMethod(left_native), Object::NativeMethod(right_native)) => {
                    left_native.kind == right_native.kind
                }
                _ => false,
            }
        };
        Some(function_equal && left_data.receiver.id() == right_data.receiver.id())
    }

    pub(super) fn compare_eq_via_int_backing(&self, left: &Value, right: &Value) -> Option<bool> {
        fn int_like(value: &Value) -> Option<BigInt> {
            match value {
                Value::Bool(flag) => Some(BigInt::from_i64(if *flag { 1 } else { 0 })),
                Value::Int(number) => Some(BigInt::from_i64(*number)),
                Value::BigInt(number) => Some((**number).clone()),
                _ => None,
            }
        }

        let int_like_value = |value: &Value| -> Option<BigInt> {
            match value {
                Value::Instance(instance) => {
                    let backing = self.instance_backing_int(instance)?;
                    int_like(&backing)
                }
                _ => int_like(value),
            }
        };

        let left_int = int_like_value(left)?;
        let right_int = int_like_value(right)?;
        Some(left_int == right_int)
    }

    pub(super) fn compare_eq_via_float_backing(&self, left: &Value, right: &Value) -> Option<bool> {
        let float_like = |value: &Value| -> Option<f64> {
            match value {
                Value::Float(number) => Some(*number),
                Value::Int(number) => Some(*number as f64),
                Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
                Value::Instance(instance) => self.instance_backing_float(instance),
                _ => None,
            }
        };
        let left_float = float_like(left)?;
        let right_float = float_like(right)?;
        Some(left_float == right_float)
    }

    pub(super) fn compare_eq_via_complex_backing(
        &self,
        left: &Value,
        right: &Value,
    ) -> Option<bool> {
        let complex_like = |value: &Value| -> Option<(f64, f64)> {
            match value {
                Value::Complex { real, imag } => Some((*real, *imag)),
                Value::Instance(instance) => self.instance_backing_complex(instance),
                _ => None,
            }
        };
        let (left_real, left_imag) = complex_like(left)?;
        let (right_real, right_imag) = complex_like(right)?;
        Some(
            left_real.to_bits() == right_real.to_bits()
                && left_imag.to_bits() == right_imag.to_bits(),
        )
    }

    pub(super) fn compare_eq_via_str_backing(&self, left: &Value, right: &Value) -> Option<bool> {
        let str_like = |value: &Value| -> Option<String> {
            match value {
                Value::Str(text) => Some(text.clone()),
                Value::Instance(instance) => self.instance_backing_str(instance),
                _ => None,
            }
        };
        let left_text = str_like(left)?;
        let right_text = str_like(right)?;
        Some(left_text == right_text)
    }

    pub(super) fn compare_eq_via_dict_backing(&self, left: &Value, right: &Value) -> Option<bool> {
        let dict_like = |value: &Value| -> Option<ObjRef> {
            match value {
                Value::Dict(dict) => Some(dict.clone()),
                Value::Instance(instance) => self.instance_backing_dict(instance),
                _ => None,
            }
        };
        let left_dict = dict_like(left)?;
        let right_dict = dict_like(right)?;
        Some(Value::Dict(left_dict) == Value::Dict(right_dict))
    }

    pub(super) fn compare_eq_via_set_backing(&self, left: &Value, right: &Value) -> Option<bool> {
        let set_like = |value: &Value| -> Option<Value> {
            match value {
                Value::Set(set) => Some(Value::Set(set.clone())),
                Value::FrozenSet(set) => Some(Value::FrozenSet(set.clone())),
                Value::Instance(instance) => {
                    if let Some(set) = self.instance_backing_set(instance) {
                        return Some(Value::Set(set));
                    }
                    self.instance_backing_frozenset(instance)
                        .map(Value::FrozenSet)
                }
                _ => None,
            }
        };
        let left_set = set_like(left)?;
        let right_set = set_like(right)?;
        Some(left_set == right_set)
    }

    pub(super) fn compare_eq_via_tuple_backing(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Option<bool>, RuntimeError> {
        let tuple_from_value = |value: &Value| -> Option<(u64, Vec<Value>)> {
            match value {
                Value::Tuple(tuple) => match &*tuple.kind() {
                    Object::Tuple(values) => Some((tuple.id(), values.clone())),
                    _ => None,
                },
                Value::Instance(instance) => {
                    let backing = self.instance_backing_tuple(instance)?;
                    match &*backing.kind() {
                        Object::Tuple(values) => Some((backing.id(), values.clone())),
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        let (Some((left_id, left_values)), Some((right_id, right_values))) =
            (tuple_from_value(left), tuple_from_value(right))
        else {
            return Ok(None);
        };
        if left_id == right_id {
            return Ok(Some(true));
        }
        if self
            .list_eq_in_progress
            .iter()
            .any(|(a, b)| (*a == left_id && *b == right_id) || (*a == right_id && *b == left_id))
        {
            return Ok(Some(true));
        }
        self.list_eq_in_progress.push((left_id, right_id));
        let result = (|| -> Result<Option<bool>, RuntimeError> {
            if left_values.len() != right_values.len() {
                return Ok(Some(false));
            }
            for (left_item, right_item) in left_values.into_iter().zip(right_values.into_iter()) {
                match self.compare_eq_runtime(left_item, right_item)? {
                    Value::Bool(true) => {}
                    Value::Bool(false) => return Ok(Some(false)),
                    _ => return Ok(None),
                }
            }
            Ok(Some(true))
        })();
        self.list_eq_in_progress.pop();
        result
    }

    pub(super) fn compare_eq_via_list_backing(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Option<bool>, RuntimeError> {
        let list_from_value = |value: &Value| -> Option<(u64, Vec<Value>)> {
            match value {
                Value::List(list) => match &*list.kind() {
                    Object::List(values) => Some((list.id(), values.clone())),
                    _ => None,
                },
                Value::Instance(instance) => {
                    let backing = self.instance_backing_list(instance)?;
                    match &*backing.kind() {
                        Object::List(values) => Some((backing.id(), values.clone())),
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        let (Some((left_id, left_values)), Some((right_id, right_values))) =
            (list_from_value(left), list_from_value(right))
        else {
            return Ok(None);
        };
        if left_id == right_id {
            return Ok(Some(true));
        }
        if self
            .list_eq_in_progress
            .iter()
            .any(|(a, b)| (*a == left_id && *b == right_id) || (*a == right_id && *b == left_id))
        {
            return Ok(Some(true));
        }
        self.list_eq_in_progress.push((left_id, right_id));
        let result = (|| -> Result<Option<bool>, RuntimeError> {
            if left_values.len() != right_values.len() {
                return Ok(Some(false));
            }
            for (left_item, right_item) in left_values.into_iter().zip(right_values.into_iter()) {
                match self.compare_eq_runtime(left_item, right_item)? {
                    Value::Bool(true) => {}
                    Value::Bool(false) => return Ok(Some(false)),
                    _ => return Ok(None),
                }
            }
            Ok(Some(true))
        })();
        self.list_eq_in_progress.pop();
        result
    }

    pub(super) fn compare_le_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        match compare_le(left.clone(), right.clone()) {
            Ok(value) => Ok(value),
            Err(err) if err.message == "unsupported operand type for comparison" => Ok(
                Value::Bool(self.compare_order_with_fallback(left, right)? != Ordering::Greater),
            ),
            Err(err) => Err(err),
        }
    }

    pub(super) fn compare_gt_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        match compare_gt(left.clone(), right.clone()) {
            Ok(value) => Ok(value),
            Err(err) if err.message == "unsupported operand type for comparison" => Ok(
                Value::Bool(self.compare_order_with_fallback(left, right)? == Ordering::Greater),
            ),
            Err(err) => Err(err),
        }
    }

    pub(super) fn compare_ge_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        match compare_ge(left.clone(), right.clone()) {
            Ok(value) => Ok(value),
            Err(err) if err.message == "unsupported operand type for comparison" => Ok(
                Value::Bool(self.compare_order_with_fallback(left, right)? != Ordering::Less),
            ),
            Err(err) => Err(err),
        }
    }

    pub(super) fn compare_cmp_to_key_wrappers(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Option<Ordering>, RuntimeError> {
        let Some((comparator, left_obj)) = self.cmp_to_key_wrapper_parts(left) else {
            return Ok(None);
        };
        let Some((_, right_obj)) = self.cmp_to_key_wrapper_parts(right) else {
            return Ok(None);
        };
        let result =
            match self.call_internal(comparator, vec![left_obj, right_obj], HashMap::new())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("cmp_to_key comparator raised"));
                }
            };
        Ok(Some(ordering_from_cmp_value(result)?))
    }

    pub(super) fn cmp_to_key_wrapper_parts(&self, value: &Value) -> Option<(Value, Value)> {
        let Value::Module(module) = value else {
            return None;
        };
        match &*module.kind() {
            Object::Module(module_data) if module_data.name == "__functools_cmp_key_item__" => {
                let comparator = module_data.globals.get("cmp")?.clone();
                let object = module_data.globals.get("obj")?.clone();
                Some((comparator, object))
            }
            _ => None,
        }
    }

    pub(super) fn builtin_all(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_all_any(args, kwargs, true)
    }

    pub(super) fn builtin_any(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_all_any(args, kwargs, false)
    }

    pub(super) fn builtin_all_any(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        expect_all: bool,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("all/any expects one argument"));
        }
        let iterable = args.remove(0);
        let iter = self
            .to_iterator_value(iterable)
            .map_err(|_| RuntimeError::new("all/any expects iterable"))?;

        let mut result = expect_all;
        match iter {
            Value::Iterator(iterator_ref) => {
                while let Some(value) = self.iterator_next_value(&iterator_ref)? {
                    let truthy = self.truthy_from_value(&value)?;
                    if expect_all {
                        if !truthy {
                            result = false;
                            break;
                        }
                    } else if truthy {
                        result = true;
                        break;
                    }
                }
            }
            Value::Generator(generator) => loop {
                match self.generator_for_iter_next(&generator)? {
                    GeneratorResumeOutcome::Yield(value) => {
                        let truthy = self.truthy_from_value(&value)?;
                        if expect_all {
                            if !truthy {
                                result = false;
                                break;
                            }
                        } else if truthy {
                            result = true;
                            break;
                        }
                    }
                    GeneratorResumeOutcome::Complete(_) => break,
                    GeneratorResumeOutcome::PropagatedException => {
                        return Err(self.iteration_error_from_state("iteration failed")?);
                    }
                }
            },
            _ => return Err(RuntimeError::new("all/any expects iterable")),
        }
        Ok(Value::Bool(result))
    }

    pub(super) fn builtin_reversed(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("reversed() expects one argument"));
        }
        let source = args.remove(0);
        let mut values = self.collect_iterable_values(source)?;
        values.reverse();
        Ok(self.heap.alloc_list(values))
    }

    pub(super) fn builtin_zip(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("zip() does not accept keyword arguments"));
        }
        if args.is_empty() {
            return Ok(self.heap.alloc_list(Vec::new()));
        }
        let mut columns = Vec::with_capacity(args.len());
        for source in args {
            columns.push(self.collect_iterable_values(source)?);
        }
        let len = columns.iter().map(Vec::len).min().unwrap_or(0);
        let mut rows = Vec::with_capacity(len);
        for idx in 0..len {
            let mut tuple_items = Vec::with_capacity(columns.len());
            for col in &columns {
                tuple_items.push(col[idx].clone());
            }
            rows.push(self.heap.alloc_tuple(tuple_items));
        }
        Ok(self.heap.alloc_list(rows))
    }

    pub(super) fn is_callable_value(&self, value: &Value) -> bool {
        match value {
            Value::Function(_)
            | Value::Builtin(_)
            | Value::BoundMethod(_)
            | Value::Class(_)
            | Value::ExceptionType(_) => true,
            Value::Module(module) => match &*module.kind() {
                Object::Module(module_data) if module_data.name == "__staticmethod__" => {
                    module_data
                        .globals
                        .get("__func__")
                        .is_some_and(|func| self.is_callable_value(func))
                }
                _ => false,
            },
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => {
                    class_attr_lookup(&instance_data.class, "__call__").is_some()
                }
                _ => false,
            },
            _ => false,
        }
    }

    pub(super) fn value_is_instance_of(
        &self,
        value: &Value,
        classinfo: &Value,
    ) -> Result<bool, RuntimeError> {
        match classinfo {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(items) => {
                    for item in items {
                        if self.value_is_instance_of(value, item)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                _ => Err(RuntimeError::new(
                    "isinstance() arg 2 must be a type or tuple of types",
                )),
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(items) => {
                    for item in items {
                        if self.value_is_instance_of(value, item)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                _ => Err(RuntimeError::new(
                    "isinstance() arg 2 must be a type or tuple of types",
                )),
            },
            Value::Class(expected) => {
                if let Object::Class(class_data) = &*expected.kind() {
                    if class_data.name == "PathLike" {
                        return Ok(self.value_has_fspath_protocol(value));
                    }
                    let marker_match = match class_data.name.as_str() {
                        "NoneType" => matches!(value, Value::None),
                        "function" => matches!(value, Value::Function(_)),
                        "method" => matches!(value, Value::BoundMethod(_)),
                        "builtin_function_or_method" => matches!(value, Value::Builtin(_)),
                        "code" => matches!(value, Value::Code(_)),
                        // CPython's numbers ABC tower treats these primitive
                        // runtime numerics as virtual subclasses.
                        "Number" | "Complex" => matches!(
                            value,
                            Value::Bool(_)
                                | Value::Int(_)
                                | Value::BigInt(_)
                                | Value::Float(_)
                                | Value::Complex { .. }
                        ),
                        "Real" => matches!(
                            value,
                            Value::Bool(_) | Value::Int(_) | Value::BigInt(_) | Value::Float(_)
                        ),
                        "Rational" | "Integral" => {
                            matches!(value, Value::Bool(_) | Value::Int(_) | Value::BigInt(_))
                        }
                        "Sequence" => match value {
                            Value::Instance(instance) => match &*instance.kind() {
                                Object::Instance(instance_data) => {
                                    class_attr_lookup(&instance_data.class, "__len__").is_some()
                                        && class_attr_lookup(&instance_data.class, "__getitem__")
                                            .is_some()
                                }
                                _ => false,
                            },
                            _ => false,
                        },
                        _ => false,
                    };
                    if marker_match {
                        return Ok(true);
                    }
                }
                match value {
                    Value::Instance(instance) => match &*instance.kind() {
                        Object::Instance(instance_data) => Ok(self
                            .class_mro_entries(&instance_data.class)
                            .iter()
                            .any(|entry| entry.id() == expected.id())),
                        _ => Ok(false),
                    },
                    Value::Class(class) => {
                        let Some(meta_class) = class_of_class(class) else {
                            return Ok(false);
                        };
                        Ok(self
                            .class_mro_entries(&meta_class)
                            .iter()
                            .any(|entry| entry.id() == expected.id()))
                    }
                    _ => Ok(false),
                }
            }
            Value::Builtin(builtin) => Ok(self.matches_builtin_type_marker(value, *builtin)),
            Value::ExceptionType(name) => match value {
                Value::Exception(exception) => {
                    Ok(exception_type_is_subclass(&exception.name, name))
                }
                Value::Instance(instance) => match &*instance.kind() {
                    Object::Instance(instance_data) => match &*instance_data.class.kind() {
                        Object::Class(class_data) => {
                            Ok(self.exception_inherits(&class_data.name, name))
                        }
                        _ => Ok(false),
                    },
                    _ => Ok(false),
                },
                Value::ExceptionType(candidate) => Ok(exception_type_is_subclass(candidate, name)),
                _ => Ok(false),
            },
            _ => Err(RuntimeError::new(
                "isinstance() arg 2 must be a type or tuple of types",
            )),
        }
    }

    pub(super) fn value_has_fspath_protocol(&self, value: &Value) -> bool {
        match value {
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => {
                    instance_data.attrs.contains_key("__fspath__")
                        || class_attr_lookup(&instance_data.class, "__fspath__").is_some()
                }
                _ => false,
            },
            Value::Class(class) => class_attr_lookup(class, "__fspath__").is_some(),
            _ => false,
        }
    }

    pub(super) fn class_value_is_subclass_of(
        &self,
        candidate: &Value,
        classinfo: &Value,
    ) -> Result<bool, RuntimeError> {
        match classinfo {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(items) => {
                    for item in items {
                        if self.class_value_is_subclass_of(candidate, item)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                _ => Err(RuntimeError::new(
                    "issubclass() arg 2 must be a type or tuple of types",
                )),
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(items) => {
                    for item in items {
                        if self.class_value_is_subclass_of(candidate, item)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                _ => Err(RuntimeError::new(
                    "issubclass() arg 2 must be a type or tuple of types",
                )),
            },
            Value::Class(expected) => match candidate {
                Value::Class(class) => {
                    if let Object::Class(expected_data) = &*expected.kind() {
                        if expected_data.name == "PathLike"
                            && class_attr_lookup(class, "__fspath__").is_some()
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "Sequence"
                            && class_attr_lookup(class, "__len__").is_some()
                            && class_attr_lookup(class, "__getitem__").is_some()
                        {
                            return Ok(true);
                        }
                    }
                    Ok(self
                        .class_mro_entries(class)
                        .iter()
                        .any(|entry| entry.id() == expected.id()))
                }
                Value::Exception(exception) => {
                    let Object::Class(class_data) = &*expected.kind() else {
                        return Ok(false);
                    };
                    Ok(self.exception_inherits(&exception.name, &class_data.name))
                }
                Value::ExceptionType(candidate_name) => {
                    let Object::Class(class_data) = &*expected.kind() else {
                        return Ok(false);
                    };
                    Ok(self.exception_inherits(candidate_name, &class_data.name))
                }
                Value::Builtin(_) => {
                    let Object::Class(class_data) = &*expected.kind() else {
                        return Ok(false);
                    };
                    Ok(match class_data.name.as_str() {
                        "object" => true,
                        "Number" | "Complex" => true,
                        "Real" => !matches!(candidate, Value::Builtin(BuiltinFunction::Complex)),
                        "Rational" | "Integral" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::Int | BuiltinFunction::Bool)
                        ),
                        _ => false,
                    })
                }
                _ => Err(RuntimeError::new("issubclass() arg 1 must be a class")),
            },
            Value::Builtin(expected_builtin) => match candidate {
                Value::Builtin(candidate_builtin) => Ok(candidate_builtin == expected_builtin),
                Value::Str(name) => {
                    if matches!(expected_builtin, BuiltinFunction::Type)
                        && is_runtime_type_name_marker(name)
                    {
                        Ok(false)
                    } else {
                        Err(RuntimeError::new("issubclass() arg 1 must be a class"))
                    }
                }
                Value::Class(class) => {
                    if !matches!(expected_builtin, BuiltinFunction::Type) {
                        return Ok(false);
                    }
                    Ok(self.class_mro_entries(class).iter().any(|entry| {
                        matches!(&*entry.kind(), Object::Class(class_data) if class_data.name == "type")
                    }))
                }
                Value::ExceptionType(_) => Ok(false),
                _ => Err(RuntimeError::new("issubclass() arg 1 must be a class")),
            },
            Value::ExceptionType(expected_name) => match candidate {
                Value::ExceptionType(candidate_name) => {
                    Ok(exception_type_is_subclass(candidate_name, expected_name))
                }
                Value::Class(class) => {
                    let Object::Class(class_data) = &*class.kind() else {
                        return Ok(false);
                    };
                    Ok(self.exception_inherits(&class_data.name, expected_name))
                }
                _ => Ok(false),
            },
            _ => Err(RuntimeError::new(
                "issubclass() arg 2 must be a type or tuple of types",
            )),
        }
    }

    pub(super) fn matches_builtin_type_marker(
        &self,
        value: &Value,
        builtin: BuiltinFunction,
    ) -> bool {
        match builtin {
            BuiltinFunction::Type => {
                matches!(
                    value,
                    Value::Class(_) | Value::ExceptionType(_) | Value::Builtin(_)
                )
            }
            BuiltinFunction::Bool => matches!(value, Value::Bool(_)),
            BuiltinFunction::Int => {
                matches!(value, Value::Int(_) | Value::BigInt(_) | Value::Bool(_))
            }
            BuiltinFunction::Float => matches!(value, Value::Float(_)),
            BuiltinFunction::Str => matches!(value, Value::Str(_)),
            BuiltinFunction::List => matches!(value, Value::List(_)),
            BuiltinFunction::Tuple => matches!(value, Value::Tuple(_)),
            BuiltinFunction::Dict => matches!(value, Value::Dict(_)),
            BuiltinFunction::CollectionsDefaultDict => matches!(
                value,
                Value::Dict(obj) if self.defaultdict_factories.contains_key(&obj.id())
            ),
            BuiltinFunction::Set => matches!(value, Value::Set(_)),
            BuiltinFunction::FrozenSet => matches!(value, Value::FrozenSet(_)),
            BuiltinFunction::Bytes => matches!(value, Value::Bytes(_)),
            BuiltinFunction::ByteArray => matches!(value, Value::ByteArray(_)),
            BuiltinFunction::MemoryView => matches!(value, Value::MemoryView(_)),
            BuiltinFunction::Complex => matches!(value, Value::Complex { .. }),
            BuiltinFunction::Slice => matches!(value, Value::Slice { .. }),
            BuiltinFunction::TypesModuleType => matches!(value, Value::Module(_)),
            BuiltinFunction::TypesMethodType => {
                matches!(value, Value::BoundMethod(method) if self.bound_method_is_python_method(method))
            }
            BuiltinFunction::Range => matches!(
                value,
                Value::Iterator(obj)
                    if matches!(
                        &*obj.kind(),
                        Object::Iterator(IteratorObject {
                            kind: IteratorKind::RangeObject { .. },
                            ..
                        })
                    )
            ),
            BuiltinFunction::Map => matches!(
                value,
                Value::Iterator(obj)
                    if matches!(
                        &*obj.kind(),
                        Object::Iterator(IteratorObject {
                            kind: IteratorKind::Map { .. },
                            ..
                        })
                    )
            ),
            _ => false,
        }
    }

    pub(super) fn builtin_iter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("iter() expects one argument"));
        }
        let source = args.remove(0);
        self.ensure_sync_iterator_target(&source)?;
        self.to_iterator_value(source)
            .map_err(|_| RuntimeError::new("object is not iterable"))
    }

    pub(super) fn builtin_next(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("next() expects 1-2 arguments"));
        }
        let default = if args.len() == 2 {
            Some(args.pop().expect("checked len"))
        } else {
            None
        };
        let target = args.pop().expect("checked len");
        if let Value::Iterator(iterator) = &target {
            let iterator_kind = iterator.kind();
            if matches!(
                &*iterator_kind,
                Object::Iterator(IteratorObject {
                    kind: IteratorKind::RangeObject { .. },
                    ..
                })
            ) {
                return Err(RuntimeError::new("'range' object is not an iterator"));
            }
        }
        self.ensure_sync_iterator_target(&target)?;
        let iterator = self
            .to_iterator_value(target)
            .map_err(|_| RuntimeError::new("next() argument is not iterable"))?;
        match iterator {
            Value::Generator(obj) => match self.generator_for_iter_next(&obj)? {
                GeneratorResumeOutcome::Yield(value) => Ok(value),
                GeneratorResumeOutcome::Complete(_) => {
                    if let Some(default) = default {
                        Ok(default)
                    } else {
                        Err(RuntimeError::new("StopIteration"))
                    }
                }
                GeneratorResumeOutcome::PropagatedException => {
                    Err(self.iteration_error_from_state("next() iteration failed")?)
                }
            },
            Value::Iterator(iterator_ref) => {
                if let Some(value) = self.iterator_next_value(&iterator_ref)? {
                    Ok(value)
                } else if let Some(default) = default {
                    Ok(default)
                } else {
                    Err(RuntimeError::new("StopIteration"))
                }
            }
            Value::Instance(_) => match self.next_from_iterator_value(&iterator)? {
                GeneratorResumeOutcome::Yield(value) => Ok(value),
                GeneratorResumeOutcome::Complete(_) => {
                    if let Some(default) = default {
                        Ok(default)
                    } else {
                        Err(RuntimeError::new("StopIteration"))
                    }
                }
                GeneratorResumeOutcome::PropagatedException => {
                    Err(self.iteration_error_from_state("next() iteration failed")?)
                }
            },
            _ => Err(RuntimeError::new("next() argument is not iterable")),
        }
    }

    pub(super) fn builtin_enumerate(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("enumerate() expects 1-2 arguments"));
        }
        let kw_start = kwargs.remove("start");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "enumerate() got an unexpected keyword argument",
            ));
        }
        if kw_start.is_some() && args.len() == 2 {
            return Err(RuntimeError::new("enumerate() got multiple values"));
        }

        let start = if let Some(value) = kw_start {
            value_to_int(value)?
        } else if args.len() == 2 {
            value_to_int(args.remove(1))?
        } else {
            0
        };
        let iterable = args.remove(0);
        let values = self
            .collect_iterable_values(iterable)
            .map_err(|_| RuntimeError::new("enumerate() expects iterable"))?;
        let mut out = Vec::with_capacity(values.len());
        for (offset, value) in values.into_iter().enumerate() {
            out.push(
                self.heap
                    .alloc_tuple(vec![Value::Int(start + offset as i64), value]),
            );
        }
        Ok(self.heap.alloc_list(out))
    }

    pub(super) fn builtin_weakref_ref(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "weakref helper expects object and optional callback",
            ));
        }

        if args.len() == 1
            && let Value::Module(wrapper) = &args[0]
            && let Object::Module(module_data) = &*wrapper.kind()
            && matches!(
                module_data.globals.get("__pyrs_weakref_ref__"),
                Some(Value::Bool(true))
            )
        {
            let target_id = match module_data.globals.get("target_id") {
                Some(Value::Int(value)) if *value >= 0 => *value as u64,
                _ => return Ok(Value::None),
            };
            // CPython clears weakrefs when object finalization begins.
            // Keep weakrefs dead even if the object is still temporarily reachable
            // during __del__ side-effects (e.g. warnings source payloads).
            if self.finalized_del_objects.contains(&target_id) {
                return Ok(Value::None);
            }
            let Some(obj) = self.heap.find_object_by_id(target_id) else {
                return Ok(Value::None);
            };
            return Ok(value_from_object_ref(obj).unwrap_or(Value::None));
        }

        let target = args.remove(0);
        let Some(target_id) = weakref_target_id(&target) else {
            return Ok(target);
        };
        let callback = args.into_iter().next().unwrap_or(Value::None);

        let wrapper = match self
            .heap
            .alloc_module(ModuleObject::new("__weakref_ref__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
            module_data
                .globals
                .insert("__pyrs_weakref_ref__".to_string(), Value::Bool(true));
            module_data
                .globals
                .insert("target_id".to_string(), Value::Int(target_id as i64));
            module_data.globals.insert("callback".to_string(), callback);
        }

        Ok(self.alloc_builtin_bound_method(BuiltinFunction::WeakRefRef, wrapper))
    }

    pub(super) fn builtin_weakref_proxy(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "weakref helper expects object and optional callback",
            ));
        }
        Ok(args.remove(0))
    }

    pub(super) fn builtin_weakref_finalize(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::new("finalize() expects object and callback"));
        }
        let object = args.remove(0);
        let callback = args.remove(0);
        let target = weakref_target_object(&object)
            .ok_or_else(|| RuntimeError::new("cannot create weak reference to object"))?;
        let target_id = target.id();
        let callback_args = self.heap.alloc_tuple(args);
        let callback_kwargs = self.heap.alloc_dict(
            kwargs
                .into_iter()
                .map(|(key, value)| (Value::Str(key), value))
                .collect(),
        );
        let finalizer = match self
            .heap
            .alloc_module(ModuleObject::new("__weakref_finalize__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        let detach = self
            .alloc_builtin_bound_method(BuiltinFunction::WeakRefFinalizeDetach, finalizer.clone());
        if let Object::Module(module_data) = &mut *finalizer.kind_mut() {
            module_data
                .globals
                .insert("__pyrs_weakref_finalize__".to_string(), Value::Bool(true));
            module_data
                .globals
                .insert("alive".to_string(), Value::Bool(true));
            module_data
                .globals
                .insert("atexit".to_string(), Value::Bool(true));
            module_data
                .globals
                .insert("target_id".to_string(), Value::Int(target_id as i64));
            module_data.globals.insert("_func".to_string(), callback);
            module_data
                .globals
                .insert("_args".to_string(), callback_args);
            module_data
                .globals
                .insert("_kwargs".to_string(), callback_kwargs);
            module_data.globals.insert("detach".to_string(), detach);
        }
        self.register_weakref_finalizer(&target, finalizer.clone());
        Ok(Value::Module(finalizer))
    }

    pub(super) fn builtin_weakref_finalize_detach(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("finalize.detach() expects no arguments"));
        }
        let finalizer = match &args[0] {
            Value::Module(obj) => obj.clone(),
            _ => return Err(RuntimeError::new("invalid finalize receiver")),
        };
        let mut obj = Value::None;
        let mut func = Value::None;
        let mut call_args = Value::None;
        let mut call_kwargs = Value::None;
        let mut alive = false;
        let mut target_id = None;
        if let Object::Module(module_data) = &mut *finalizer.kind_mut() {
            alive = matches!(module_data.globals.get("alive"), Some(Value::Bool(true)));
            if alive {
                module_data
                    .globals
                    .insert("alive".to_string(), Value::Bool(false));
                target_id = match module_data.globals.get("target_id") {
                    Some(Value::Int(value)) if *value >= 0 => Some(*value as u64),
                    _ => None,
                };
                func = module_data
                    .globals
                    .get("_func")
                    .cloned()
                    .unwrap_or(Value::None);
                call_args = module_data
                    .globals
                    .get("_args")
                    .cloned()
                    .unwrap_or_else(|| self.heap.alloc_tuple(Vec::new()));
                call_kwargs = module_data
                    .globals
                    .get("_kwargs")
                    .cloned()
                    .unwrap_or_else(|| self.heap.alloc_dict(Vec::new()));
            }
        }
        if let Some(id) = target_id {
            self.unregister_weakref_finalizer(id, finalizer.id());
            if let Some(target) = self
                .heap
                .find_object_by_id(id)
                .and_then(value_from_object_ref)
            {
                obj = target;
            }
        }
        if !alive || matches!(obj, Value::None) {
            return Ok(Value::None);
        }
        Ok(self
            .heap
            .alloc_tuple(vec![obj, func, call_args, call_kwargs]))
    }

    pub(super) fn builtin_map(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 {
            return Err(RuntimeError::new("map() expects at least two arguments"));
        }
        let func = args.remove(0);
        let sources = args.clone();
        let mut iterators = Vec::new();
        for source in args {
            self.ensure_sync_iterator_target(&source)?;
            let iterator = self
                .to_iterator_value(source)
                .map_err(|_| RuntimeError::new("map() argument is not iterable"))?;
            iterators.push(iterator);
        }

        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Map {
                values: Vec::new(),
                func,
                iterators,
                sources,
                exhausted: false,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_filter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("filter() expects two arguments"));
        }
        let predicate = args.remove(0);
        let source = args.remove(0);
        self.ensure_sync_iterator_target(&source)?;
        let iterator = self
            .to_iterator_value(source)
            .map_err(|_| RuntimeError::new("filter() argument 2 is not iterable"))?;
        let mut filtered = Vec::new();
        loop {
            let item = match self.next_from_iterator_value(&iterator)? {
                GeneratorResumeOutcome::Yield(value) => value,
                GeneratorResumeOutcome::Complete(_) => break,
                GeneratorResumeOutcome::PropagatedException => {
                    return Err(self.iteration_error_from_state("filter() iteration failed")?);
                }
            };

            let include = if matches!(predicate, Value::None) {
                self.truthy_from_value(&item)?
            } else {
                match self.call_internal(predicate.clone(), vec![item.clone()], HashMap::new())? {
                    InternalCallOutcome::Value(value) => self.truthy_from_value(&value)?,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(RuntimeError::new("filter() callable failed"));
                    }
                }
            };
            if include {
                filtered.push(item);
            }
        }
        Ok(self.heap.alloc_list(filtered))
    }

    pub(super) fn builtin_aiter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("aiter() expects one argument"));
        }
        let source = args.remove(0);
        let source_is_async_generator = if let Value::Generator(generator) = &source {
            matches!(&*generator.kind(), Object::Generator(state) if state.is_async_generator)
        } else {
            false
        };
        if source_is_async_generator {
            return Ok(source);
        }
        let method = self
            .lookup_bound_special_method(&source, "__aiter__")?
            .ok_or_else(|| RuntimeError::new("object is not async iterable"))?;
        match self.call_internal(method, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => Err(RuntimeError::new("aiter() failed")),
        }
    }

    pub(super) fn builtin_anext(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("anext() expects 1-2 arguments"));
        }
        let default = if args.len() == 2 {
            Some(args.pop().expect("checked len"))
        } else {
            None
        };
        let target = args.pop().expect("checked len");

        let target_is_async_generator = if let Value::Generator(generator) = &target {
            matches!(&*generator.kind(), Object::Generator(state) if state.is_async_generator)
        } else {
            false
        };

        if target_is_async_generator {
            let generator = match &target {
                Value::Generator(generator) => generator,
                _ => unreachable!(),
            };
            return match self.resume_generator(generator, None, None, GeneratorResumeKind::Next)? {
                GeneratorResumeOutcome::Yield(value) => Ok(self.make_immediate_coroutine(value)),
                GeneratorResumeOutcome::Complete(_) => {
                    if let Some(default) = default {
                        Ok(self.make_immediate_coroutine(default))
                    } else {
                        Err(RuntimeError::new("StopAsyncIteration"))
                    }
                }
                GeneratorResumeOutcome::PropagatedException => {
                    self.propagate_pending_generator_exception()?;
                    Err(RuntimeError::new("StopAsyncIteration"))
                }
            };
        }

        let method = self
            .lookup_bound_special_method(&target, "__anext__")?
            .ok_or_else(|| RuntimeError::new("object is not an async iterator"))?;
        match self.call_internal(method, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                if let Some(default) = default
                    && self.active_exception_is("StopAsyncIteration")
                {
                    self.clear_active_exception();
                    Ok(self.make_immediate_coroutine(default))
                } else {
                    Err(RuntimeError::new("anext() failed"))
                }
            }
        }
    }

    pub(super) fn builtin_getattr(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "getattr() got an unexpected keyword argument",
            ));
        }
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new("getattr() expects 2-3 arguments"));
        }

        let mut args_iter = args.into_iter();
        let target = args_iter
            .next()
            .ok_or_else(|| RuntimeError::new("getattr() expects 2-3 arguments"))?;
        let name = match args_iter
            .next()
            .ok_or_else(|| RuntimeError::new("getattr() expects 2-3 arguments"))?
        {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("attribute name must be string")),
        };
        let default = args_iter.next();
        // Preserve the caller's active exception context: getattr(..., default)
        // should not clobber an exception currently being handled by surrounding code.
        let saved_active_exception = self
            .frames
            .last()
            .and_then(|frame| frame.active_exception.clone());
        if name == "__class__" {
            return self.load_dunder_class_attr(&target);
        }
        if name == "generate_state"
            && std::env::var_os("PYRS_TRACE_GETATTR_GENERATE_STATE").is_some()
        {
            let target_tag = match &target {
                Value::None => "None".to_string(),
                Value::Class(_) => "Class".to_string(),
                Value::Instance(_) => "Instance".to_string(),
                Value::Builtin(_) => "Builtin".to_string(),
                Value::Module(_) => "Module".to_string(),
                Value::Function(_) => "Function".to_string(),
                Value::BoundMethod(_) => "BoundMethod".to_string(),
                _ => "Other".to_string(),
            };
            let stack = self
                .frames
                .iter()
                .rev()
                .take(10)
                .map(|frame| format!("{}@{}", frame.code.name, frame.code.filename))
                .collect::<Vec<_>>()
                .join(" <- ");
            eprintln!(
                "[getattr-generate-state] target={} stack={}",
                target_tag, stack
            );
        }

        let looked_up = match target {
            Value::Module(module) => self.load_attr_module(&module, &name),
            Value::Class(class) => match self.load_attr_class(&class, &name) {
                Ok(AttrAccessOutcome::Value(value)) => Ok(value),
                Ok(AttrAccessOutcome::ExceptionHandled) => {
                    Err(self.runtime_error_from_active_exception("getattr() failed"))
                }
                Err(err) => Err(err),
            },
            Value::Instance(instance) => match self.load_attr_instance(&instance, &name) {
                Ok(AttrAccessOutcome::Value(value)) => Ok(value),
                Ok(AttrAccessOutcome::ExceptionHandled) => {
                    Err(self.runtime_error_from_active_exception("getattr() failed"))
                }
                Err(err) => Err(err),
            },
            Value::Super(super_obj) => match self.load_attr_super(&super_obj, &name) {
                Ok(AttrAccessOutcome::Value(value)) => Ok(value),
                Ok(AttrAccessOutcome::ExceptionHandled) => {
                    Err(self.runtime_error_from_active_exception("getattr() failed"))
                }
                Err(err) => Err(err),
            },
            Value::None => match name.as_str() {
                "__doc__" => Ok(Value::Str("None".to_string())),
                _ => Err(RuntimeError::attribute_error(format!(
                    "NoneType has no attribute '{}'",
                    name
                ))),
            },
            Value::List(list) => self.load_attr_list_method(list, &name),
            Value::Tuple(tuple) => self.load_attr_tuple_method(tuple, &name),
            Value::Str(text) => self.load_attr_str_method(text, &name),
            Value::Bytes(bytes) => {
                let is_bytes = matches!(&*bytes.kind(), Object::Bytes(_));
                if !is_bytes {
                    return Err(RuntimeError::attribute_error("attribute access unsupported type"));
                }
                self.load_attr_bytes_method(Value::Bytes(bytes), &name)
            }
            Value::ByteArray(bytearray) => {
                let is_bytearray = matches!(&*bytearray.kind(), Object::ByteArray(_));
                if !is_bytearray {
                    return Err(RuntimeError::attribute_error("attribute access unsupported type"));
                }
                self.load_attr_bytes_method(Value::ByteArray(bytearray), &name)
            }
            Value::Iterator(iterator) => self.load_attr_iterator(iterator, &name),
            Value::MemoryView(view) => self.load_attr_memoryview(view, &name),
            Value::Set(set) => self.load_attr_set_method(set, &name),
            Value::FrozenSet(set) => self.load_attr_set_method(set, &name),
            Value::Dict(dict) => self.load_attr_dict_method(dict, &name),
            Value::Cell(cell) => self.load_attr_cell(cell, &name),
            Value::Builtin(builtin) => self.load_attr_builtin(builtin, &name),
            Value::Function(func) => self.load_attr_function(&func, &name),
            Value::BoundMethod(method) => self.load_attr_bound_method(&method, &name),
            Value::ExceptionType(exception_name) => {
                self.load_attr_exception_type(&exception_name, &name)
            }
            Value::Code(code) => self.load_attr_code(&code, &name),
            Value::Generator(generator) => {
                let (kind, type_name) = match &*generator.kind() {
                    Object::Generator(state) if state.is_async_generator => (
                        match name.as_str() {
                            "__aiter__" => Some(NativeMethodKind::GeneratorIter),
                            "__anext__" => Some(NativeMethodKind::GeneratorANext),
                            "asend" => Some(NativeMethodKind::GeneratorANext),
                            "athrow" => Some(NativeMethodKind::GeneratorThrow),
                            "aclose" => Some(NativeMethodKind::GeneratorClose),
                            "throw" => Some(NativeMethodKind::GeneratorThrow),
                            "close" => Some(NativeMethodKind::GeneratorClose),
                            _ => None,
                        },
                        "async_generator",
                    ),
                    Object::Generator(state) if state.is_coroutine => (
                        match name.as_str() {
                            "__await__" => Some(NativeMethodKind::GeneratorAwait),
                            "send" => Some(NativeMethodKind::GeneratorSend),
                            "throw" => Some(NativeMethodKind::GeneratorThrow),
                            "close" => Some(NativeMethodKind::GeneratorClose),
                            _ => None,
                        },
                        "coroutine",
                    ),
                    Object::Generator(_) => (
                        match name.as_str() {
                            "__iter__" => Some(NativeMethodKind::GeneratorIter),
                            "__next__" => Some(NativeMethodKind::GeneratorNext),
                            "send" => Some(NativeMethodKind::GeneratorSend),
                            "throw" => Some(NativeMethodKind::GeneratorThrow),
                            "close" => Some(NativeMethodKind::GeneratorClose),
                            _ => None,
                        },
                        "generator",
                    ),
                    _ => return Err(RuntimeError::attribute_error("attribute access unsupported type")),
                };
                if let Some(kind) = kind {
                    let native = self.heap.alloc_native_method(NativeMethodObject::new(kind));
                    let bound = BoundMethod::new(native, generator);
                    Ok(self.heap.alloc_bound_method(bound))
                } else {
                    Err(RuntimeError::new(format!(
                        "{type_name} has no attribute '{}'",
                        name
                    )))
                }
            }
            Value::Complex { real, imag } => match name.as_str() {
                "__reduce_ex__" | "__reduce__" => {
                    let wrapper = match self
                        .heap
                        .alloc_module(ModuleObject::new("__complex_reduce_ex__".to_string()))
                    {
                        Value::Module(obj) => obj,
                        _ => unreachable!(),
                    };
                    if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
                        module_data
                            .globals
                            .insert("value".to_string(), Value::Complex { real, imag });
                    }
                    Ok(self.alloc_native_bound_method(NativeMethodKind::ComplexReduceEx, wrapper))
                }
                "real" => Ok(Value::Float(real)),
                "imag" => Ok(Value::Float(imag)),
                _ => Err(RuntimeError::new(format!(
                    "complex has no attribute '{}'",
                    name
                ))),
            },
            Value::Exception(exception) => match name.as_str() {
                "__reduce_ex__" | "__reduce__" => {
                    Ok(self.alloc_reduce_ex_bound_method(Value::Exception(exception.clone())))
                }
                "with_traceback" => {
                    let wrapper = match self.heap.alloc_module(ModuleObject::new(
                        "__exception_with_traceback__".to_string(),
                    )) {
                        Value::Module(obj) => obj,
                        _ => unreachable!(),
                    };
                    if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
                        module_data
                            .globals
                            .insert("exception".to_string(), Value::Exception(exception.clone()));
                    }
                    Ok(self.alloc_native_bound_method(
                        NativeMethodKind::ExceptionWithTraceback,
                        wrapper,
                    ))
                }
                "add_note" => {
                    let wrapper = match self
                        .heap
                        .alloc_module(ModuleObject::new("__exception_add_note__".to_string()))
                    {
                        Value::Module(obj) => obj,
                        _ => unreachable!(),
                    };
                    if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
                        module_data
                            .globals
                            .insert("exception".to_string(), Value::Exception(exception.clone()));
                    }
                    Ok(self.alloc_native_bound_method(NativeMethodKind::ExceptionAddNote, wrapper))
                }
                "__notes__" => {
                    if exception.notes.is_empty() {
                        Ok(Value::None)
                    } else {
                        Ok(self
                            .heap
                            .alloc_list(exception.notes.iter().cloned().map(Value::Str).collect()))
                    }
                }
                "__cause__" => Ok(exception
                    .cause
                    .as_ref()
                    .map(|cause| Value::Exception(Box::new((**cause).clone())))
                    .unwrap_or(Value::None)),
                "__context__" => Ok(exception
                    .context
                    .as_ref()
                    .map(|context| Value::Exception(Box::new((**context).clone())))
                    .unwrap_or(Value::None)),
                "__traceback__" => Ok(Value::None),
                "__suppress_context__" => Ok(Value::Bool(exception.suppress_context)),
                "exceptions" => {
                    let members = exception
                        .exceptions
                        .iter()
                        .cloned()
                        .map(|member| Value::Exception(Box::new(member)))
                        .collect::<Vec<_>>();
                    Ok(self.heap.alloc_tuple(members))
                }
                _ => exception.attrs.borrow().get(&name).cloned().ok_or_else(|| {
                    RuntimeError::attribute_error(format!("exception has no attribute '{}'", name))
                }),
            },
            _ => Err(RuntimeError::attribute_error("attribute access unsupported type")),
        };

        match looked_up {
            Ok(value) => Ok(value),
            Err(err) => {
                if let Some(default) = default {
                    if is_missing_attribute_error(&err) {
                        if let Some(frame) = self.frames.last_mut() {
                            frame.active_exception = saved_active_exception;
                        }
                        Ok(default)
                    } else {
                        Err(err)
                    }
                } else {
                    Err(err)
                }
            }
        }
    }

    pub(super) fn builtin_setattr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "setattr() got an unexpected keyword argument",
            ));
        }
        if args.len() != 3 {
            return Err(RuntimeError::new("setattr() expects three arguments"));
        }

        let target = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("attribute name must be string")),
        };
        let value = args.remove(0);

        match target {
            Value::Module(module) => {
                let module_id = module.id();
                let active_frame_index = self
                    .frames
                    .iter()
                    .rposition(|frame| frame.is_module && frame.module.id() == module_id);
                if let Object::Module(module_data) = &mut *module.kind_mut() {
                    module_data.globals.insert(name.clone(), value.clone());
                }
                if let Some(frame_index) = active_frame_index {
                    let dict = self.ensure_frame_module_locals_dict(frame_index);
                    dict_set_value(&dict, Value::Str(name), value);
                }
            }
            Value::Instance(instance) => match self.store_attr_instance(&instance, &name, value)? {
                AttrMutationOutcome::Done => {}
                AttrMutationOutcome::ExceptionHandled => return Ok(Value::None),
            },
            Value::Class(class) => {
                if let Object::Class(class_data) = &mut *class.kind_mut() {
                    class_data.attrs.insert(name, value);
                }
            }
            Value::Function(func) => self.store_attr_function(&func, name, value)?,
            Value::Cell(cell) => self.store_attr_cell(&cell, &name, value)?,
            Value::Exception(mut exception) => {
                self.store_attr_exception(&mut exception, &name, value)?
            }
            Value::Builtin(builtin) => self.store_attr_builtin(builtin, &name, value)?,
            other => {
                if std::env::var_os("PYRS_TRACE_SETATTR_UNSUPPORTED").is_some() {
                    eprintln!(
                        "[setattr-unsupported] target={} name={}",
                        format_repr(&other),
                        name
                    );
                }
                return Err(RuntimeError::new("attribute assignment unsupported type"));
            }
        }

        Ok(Value::None)
    }

    pub(super) fn builtin_delattr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "delattr() got an unexpected keyword argument",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::new("delattr() expects two arguments"));
        }

        let target = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("attribute name must be string")),
        };

        match target {
            Value::Module(module) => {
                let module_id = module.id();
                let active_frame_index = self
                    .frames
                    .iter()
                    .rposition(|frame| frame.is_module && frame.module.id() == module_id);
                if let Object::Module(module_data) = &mut *module.kind_mut()
                    && module_data.globals.remove(&name).is_none()
                {
                    return Err(RuntimeError::new(format!(
                        "AttributeError: module attribute '{}' does not exist",
                        name
                    )));
                }
                if let Some(frame_index) = active_frame_index {
                    let dict = self.ensure_frame_module_locals_dict(frame_index);
                    let _ = dict_remove_value(&dict, &Value::Str(name.clone()));
                }
            }
            Value::Class(class) => {
                if let Object::Class(class_data) = &mut *class.kind_mut()
                    && class_data.attrs.remove(&name).is_none()
                {
                    return Err(RuntimeError::new(format!(
                        "AttributeError: class attribute '{}' does not exist",
                        name
                    )));
                }
            }
            Value::Instance(instance) => match self.delete_attr_instance(&instance, &name)? {
                AttrMutationOutcome::Done => {}
                AttrMutationOutcome::ExceptionHandled => return Ok(Value::None),
            },
            Value::Function(func) => self.delete_attr_function(&func, &name)?,
            Value::Cell(cell) => self.delete_attr_cell(&cell, &name)?,
            Value::Exception(exception) => self.delete_attr_exception(&exception, &name)?,
            Value::Builtin(builtin) => self.delete_attr_builtin(builtin, &name)?,
            _ => return Err(RuntimeError::new("attribute deletion unsupported type")),
        }

        Ok(Value::None)
    }

    pub(super) fn builtin_hasattr(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 2 {
            return Err(RuntimeError::new("hasattr() expects two arguments"));
        }
        match self.builtin_getattr(args, kwargs) {
            Ok(_) => Ok(Value::Bool(true)),
            Err(err) if is_missing_attribute_error(&err) => Ok(Value::Bool(false)),
            Err(err) => Err(err),
        }
    }

    pub(super) fn builtin_super(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "super() got an unexpected keyword argument",
            ));
        }
        let (start_class, object_value) = if args.is_empty() {
            let frame = self
                .frames
                .last()
                .ok_or_else(|| RuntimeError::new("super(): no active frame"))?;

            let object_value = frame
                .code
                .posonly_params
                .iter()
                .chain(frame.code.params.iter())
                .find_map(|name| {
                    Vm::frame_local_value(frame, name).or_else(|| frame_cell_value(frame, name))
                })
                .or_else(|| {
                    Vm::frame_local_value(frame, "self").or_else(|| frame_cell_value(frame, "self"))
                })
                .ok_or_else(|| {
                    RuntimeError::new(
                        "super(): unable to determine object for zero-argument super()",
                    )
                })?;

            let class_from_locals =
                Vm::frame_local_value(frame, "__class__").and_then(|value| match value {
                    Value::Class(class) => Some(class.clone()),
                    _ => None,
                });
            let class_from_cells =
                frame_cell_value(frame, "__class__").and_then(|value| match value {
                    Value::Class(class) => Some(class),
                    _ => None,
                });
            let class_from_owner = frame.owner_class.clone();
            let inferred = match &object_value {
                Value::Class(class) => match &*class.kind() {
                    Object::Class(class_data) => {
                        class_data.bases.first().cloned().or(Some(class.clone()))
                    }
                    _ => Some(class.clone()),
                },
                _ => self.class_of_value(&object_value),
            };
            let start_class = class_from_locals
                .or(class_from_cells)
                .or(class_from_owner)
                .or(inferred)
                .ok_or_else(|| {
                    RuntimeError::new(
                        "super(): unable to determine class for zero-argument super()",
                    )
                })?;
            (start_class, object_value)
        } else if args.len() == 2 {
            let start_class = match args.remove(0) {
                Value::Class(class) => class,
                _ => return Err(RuntimeError::new("super() first argument must be a class")),
            };
            let object_value = args.remove(0);
            (start_class, object_value)
        } else {
            return Err(RuntimeError::new("super() expects zero or two arguments"));
        };
        let object_ref = self.receiver_from_value(&object_value)?;
        let object_type = match &object_value {
            Value::Class(class) => {
                let class_mro = self.class_mro_entries(class);
                if class_mro.iter().any(|entry| entry.id() == start_class.id()) {
                    class.clone()
                } else if let Some(meta_class) = self.class_of_value(&object_value) {
                    let meta_mro = self.class_mro_entries(&meta_class);
                    if meta_mro.iter().any(|entry| entry.id() == start_class.id()) {
                        meta_class
                    } else {
                        class.clone()
                    }
                } else {
                    class.clone()
                }
            }
            _ => self.class_of_value(&object_value).ok_or_else(|| {
                RuntimeError::new("super() second argument must be an instance or subclass")
            })?,
        };

        let mro = self.class_mro_entries(&object_type);
        if !mro.iter().any(|entry| entry.id() == start_class.id()) {
            return Err(RuntimeError::new(
                "super(type, obj): obj must be an instance or subtype of type",
            ));
        }

        Ok(self
            .heap
            .alloc_super(SuperObject::new(start_class, object_ref, object_type)))
    }
}
