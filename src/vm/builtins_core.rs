#[cfg(target_arch = "wasm32")]
use super::wasm_c_float_format::format_float_with_c_pattern;
use super::{
    AttrAccessOutcome, AttrMutationOutcome, BYTES_BACKING_STORAGE_ATTR, BigInt, BoundMethod,
    BuiltinFunction, COMPLEX_BACKING_STORAGE_ATTR, ClassBuildOutcome, ClassObject, CodeObject,
    CompiledCodeMode, DICT_BACKING_STORAGE_ATTR, ExceptionObject, FLOAT_BACKING_STORAGE_ATTR,
    FROZENSET_BACKING_STORAGE_ATTR, Frame, GeneratorResumeKind, GeneratorResumeOutcome, HashMap,
    INSTANCE_DICT_STORAGE_ATTR, INT_BACKING_STORAGE_ATTR, InstanceObject, InternalCallOutcome,
    IteratorKind, IteratorObject, LIST_BACKING_STORAGE_ATTR, MAPPING_PROXY_STORAGE_ATTR,
    MONITORING_EVENT_BRANCH, MONITORING_EVENT_BRANCH_LEFT, MONITORING_EVENT_BRANCH_RIGHT,
    MONITORING_EVENT_C_RAISE, MONITORING_EVENT_C_RETURN, MONITORING_EVENT_CALL,
    MONITORING_EVENT_SET_MAX, MONITORING_LOCAL_EVENT_SET_MAX, MONITORING_MAX_USER_TOOL_ID,
    ModuleObject, NativeMethodKind, NativeMethodObject, ObjRef, Object, Ordering,
    PY_TPFLAGS_HEAPTYPE, PY_TPFLAGS_IMMUTABLETYPE, Rc, RuntimeError, SET_BACKING_STORAGE_ATTR,
    STR_BACKING_STORAGE_ATTR, SuperObject, TUPLE_BACKING_STORAGE_ATTR, Value, Vm, Write,
    add_values, bigint_from_bytes, bytes_like_from_value, call_builtin_with_kwargs,
    class_attr_lookup, class_attr_walk, compare_ge, compare_gt, compare_in, compare_le, compare_lt,
    compare_order, compiler, decode_text_bytes, dict_remove_value, dict_set_value,
    dict_set_value_checked, div_values, encode_text_bytes, format_float_hex, format_repr,
    format_value, frame_cell_value, invert_value, is_import_error_family,
    is_missing_attribute_error, is_os_error_family, is_runtime_type_name_marker, matmul_values,
    mul_values, neg_value, normalize_codec_encoding, normalize_codec_errors, or_values,
    ordering_from_cmp_value, parse_hex_float_literal, parser, pos_value, round_float_with_ndigits,
    runtime_error_matches_exception, sub_values, value_from_bigint, value_from_object_ref,
    value_to_bigint, value_to_f64, value_to_int, weakref_target_id, weakref_target_object,
    with_bytes_like_source, xor_values,
};
use crate::ast::{
    AssignTarget, AugOp as AstAugOp, BinaryOp as AstBinaryOp, BoolOp as AstBoolOp, CallArg,
    ComprehensionClause as AstComprehensionClause, Constant as AstConstant, DictEntry,
    ExceptHandler as AstExceptHandler, Expr, ExprKind, ImportAlias as AstImportAlias,
    MatchCase as AstMatchCase, Module as AstModule, Parameter as AstParameter,
    Pattern as AstPattern, Stmt, StmtKind, TypeParam as AstTypeParam,
    TypeParamKind as AstTypeParamKind, UnaryOp as AstUnaryOp,
};
use crate::runtime::value_lookup_hash;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

thread_local! {
    static TYPE_INSTANCECHECK_BYPASS_CUSTOM: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TYPE_SUBCLASSCHECK_BYPASS_CUSTOM: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static DIR_CUSTOM_LOOKUP_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Copy)]
struct NumericFormatSpec {
    fill: char,
    align: Option<char>,
    sign: char,
    alternate: bool,
    zero_pad: bool,
    width: Option<usize>,
    grouping: Option<char>,
    precision: Option<usize>,
    ty: Option<char>,
}

impl Default for NumericFormatSpec {
    fn default() -> Self {
        Self {
            fill: ' ',
            align: None,
            sign: '-',
            alternate: false,
            zero_pad: false,
            width: None,
            grouping: None,
            precision: None,
            ty: None,
        }
    }
}

impl Vm {
    fn exception_str_value(&self, exception: &ExceptionObject) -> String {
        if exception.name == "KeyError" {
            let keyerror_from_args = {
                let attrs = exception.attrs.borrow();
                if let Some(Value::Tuple(tuple)) = attrs.get("args") {
                    if let Object::Tuple(args) = &*tuple.kind() {
                        if args.len() == 1 {
                            Some(format_repr(&args[0]))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(rendered) = keyerror_from_args {
                return rendered;
            }
        }
        if self.exception_inherits(exception.name.as_str(), "BaseExceptionGroup") {
            let message = if let Some(message) = exception.message.clone() {
                message
            } else {
                let attrs = exception.attrs.borrow();
                match attrs.get("args") {
                    Some(Value::Tuple(tuple_obj)) => match &*tuple_obj.kind() {
                        Object::Tuple(args) if !args.is_empty() => format_value(&args[0]),
                        _ => String::new(),
                    },
                    _ => String::new(),
                }
            };
            let count = exception.exceptions.len();
            let suffix = if count == 1 {
                "1 sub-exception".to_string()
            } else {
                format!("{count} sub-exceptions")
            };
            return if message.is_empty() {
                format!(" ({suffix})")
            } else {
                format!("{message} ({suffix})")
            };
        }
        exception.message.clone().unwrap_or_default()
    }

    fn exception_repr_value(&self, exception: &ExceptionObject) -> String {
        let args = {
            let attrs = exception.attrs.borrow();
            if let Some(Value::Tuple(tuple_obj)) = attrs.get("args") {
                if let Object::Tuple(items) = &*tuple_obj.kind() {
                    Some(items.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(args) = args {
            if args.is_empty() {
                format!("{}()", exception.name)
            } else {
                format!(
                    "{}({})",
                    exception.name,
                    args.iter().map(format_repr).collect::<Vec<_>>().join(", ")
                )
            }
        } else if let Some(message) = &exception.message {
            format!(
                "{}({})",
                exception.name,
                format_repr(&Value::Str(message.clone()))
            )
        } else {
            format!("{}()", exception.name)
        }
    }

    fn value_to_str_text(&mut self, value: Value) -> Result<String, RuntimeError> {
        match self.builtin_str(vec![value], HashMap::new())? {
            Value::Str(text) => Ok(text),
            _ => Err(RuntimeError::type_error("__str__ returned non-string")),
        }
    }

    fn value_to_repr_text(&mut self, value: Value) -> Result<String, RuntimeError> {
        match self.builtin_repr(vec![value], HashMap::new())? {
            Value::Str(text) => Ok(text),
            _ => Err(RuntimeError::type_error("__repr__ returned non-string")),
        }
    }

    fn exception_instance_name_args_and_member_count(
        &self,
        instance: &ObjRef,
    ) -> Option<(String, Vec<Value>, Option<usize>)> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        if !self.class_is_exception_class(&instance_data.class) {
            return None;
        }
        let class_name = match &*instance_data.class.kind() {
            Object::Class(class_data) => class_data.name.clone(),
            _ => "BaseException".to_string(),
        };
        let args = match instance_data.attrs.get("args") {
            Some(Value::Tuple(tuple_obj)) => match &*tuple_obj.kind() {
                Object::Tuple(items) => items.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        let member_count = if self.exception_inherits(class_name.as_str(), "BaseExceptionGroup") {
            match instance_data.attrs.get("exceptions") {
                Some(Value::Tuple(tuple_obj)) => match &*tuple_obj.kind() {
                    Object::Tuple(items) => Some(items.len()),
                    _ => Some(0),
                },
                _ => None,
            }
        } else {
            None
        };
        Some((class_name, args, member_count))
    }

    fn exception_str_from_name_and_args(
        &mut self,
        class_name: &str,
        args: &[Value],
        member_count: Option<usize>,
    ) -> Result<String, RuntimeError> {
        if class_name == "KeyError" && args.len() == 1 {
            return Ok(format_repr(&args[0]));
        }
        if self.exception_inherits(class_name, "BaseExceptionGroup") {
            let message = if let Some(message) = args.first() {
                self.value_to_str_text(message.clone())?
            } else {
                String::new()
            };
            let count = member_count.unwrap_or_else(|| {
                args.get(1).map_or(0, |value| match value {
                    Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                        Object::Tuple(items) => items.len(),
                        _ => 0,
                    },
                    Value::List(list_obj) => match &*list_obj.kind() {
                        Object::List(items) => items.len(),
                        _ => 0,
                    },
                    _ => 0,
                })
            });
            let suffix = if count == 1 {
                "1 sub-exception".to_string()
            } else {
                format!("{count} sub-exceptions")
            };
            return Ok(if message.is_empty() {
                format!(" ({suffix})")
            } else {
                format!("{message} ({suffix})")
            });
        }
        match args.len() {
            0 => Ok(String::new()),
            1 => self.value_to_str_text(args[0].clone()),
            _ => self.value_to_repr_text(self.heap.alloc_tuple(args.to_vec())),
        }
    }

    fn exception_repr_from_name_and_args(&self, class_name: &str, args: &[Value]) -> String {
        if args.is_empty() {
            format!("{class_name}()")
        } else {
            format!(
                "{class_name}({})",
                args.iter().map(format_repr).collect::<Vec<_>>().join(", ")
            )
        }
    }

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

    fn runtime_format_type_name(&self, value: &Value) -> String {
        let Some(class_obj) = self.class_of_value(value) else {
            return self.value_type_name_for_error(value);
        };
        match &*class_obj.kind() {
            Object::Class(class_data) => class_data.name.clone(),
            _ => self.value_type_name_for_error(value),
        }
    }

    fn truthy_from_len_result(&self, result: Value) -> Result<bool, RuntimeError> {
        match result {
            Value::Bool(flag) => Ok(flag),
            Value::Int(number) => {
                if number < 0 {
                    Err(RuntimeError::value_error("__len__() should return >= 0"))
                } else {
                    Ok(number != 0)
                }
            }
            Value::BigInt(number) => {
                if number.is_negative() {
                    Err(RuntimeError::value_error("__len__() should return >= 0"))
                } else {
                    Ok(!number.is_zero())
                }
            }
            other => Err(RuntimeError::type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                self.value_type_name_for_error(&other)
            ))),
        }
    }

    pub(super) fn truthy_from_value(&mut self, value: &Value) -> Result<bool, RuntimeError> {
        if let Value::Instance(instance) = value {
            if let Some(values) = self.namedtuple_instance_values(instance) {
                return Ok(!values.is_empty());
            }
            if let Some(backing_list) = self.instance_backing_list(instance)
                && let Object::List(values) = &*backing_list.kind()
            {
                return Ok(!values.is_empty());
            }
            if let Some(backing_tuple) = self.instance_backing_tuple(instance)
                && let Object::Tuple(values) = &*backing_tuple.kind()
            {
                return Ok(!values.is_empty());
            }
            if let Some(backing_str) = self.instance_backing_str(instance) {
                return Ok(!backing_str.is_empty());
            }
            if let Some(backing_dict) = self.instance_backing_dict(instance)
                && let Object::Dict(values) = &*backing_dict.kind()
            {
                return Ok(!values.is_empty());
            }
            if let Some(backing_set) = self.instance_backing_set(instance)
                && let Object::Set(values) = &*backing_set.kind()
            {
                return Ok(!values.is_empty());
            }
            if let Some(backing_frozenset) = self.instance_backing_frozenset(instance)
                && let Object::FrozenSet(values) = &*backing_frozenset.kind()
            {
                return Ok(!values.is_empty());
            }
        }

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
            self.write_text_to_sys_stream_with_flush("stdout", &rendered, flush_requested)?;
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

    fn write_text_to_sys_stream_with_flush(
        &mut self,
        stream_name: &str,
        text: &str,
        flush_after_write: bool,
    ) -> Result<(), RuntimeError> {
        let stream = self.modules.get("sys").and_then(|sys_module| {
            if let Object::Module(module_data) = &*sys_module.kind() {
                module_data.globals.get(stream_name).cloned()
            } else {
                None
            }
        });
        let Some(stream) = stream else {
            return Err(RuntimeError::with_exception(
                "RuntimeError",
                Some(format!("lost sys.{stream_name}")),
            ));
        };
        let write = self.builtin_getattr(
            vec![stream.clone(), Value::Str("write".to_string())],
            HashMap::new(),
        )?;
        match self.call_internal(write, vec![Value::Str(text.to_string())], HashMap::new())? {
            InternalCallOutcome::Value(_) => {}
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(self.runtime_error_from_active_exception(&format!(
                    "sys.{stream_name}.write() failed"
                )));
            }
        }
        if flush_after_write
            && let Ok(flush) = self.builtin_getattr(
                vec![stream, Value::Str("flush".to_string())],
                HashMap::new(),
            )
        {
            match self.call_internal(flush, Vec::new(), HashMap::new())? {
                InternalCallOutcome::Value(_) => {}
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception(&format!(
                        "sys.{stream_name}.flush() failed"
                    )));
                }
            }
        }
        Ok(())
    }

    fn write_text_to_sys_stream(
        &mut self,
        stream_name: &str,
        text: &str,
    ) -> Result<(), RuntimeError> {
        self.write_text_to_sys_stream_with_flush(stream_name, text, true)
    }

    fn write_text_to_sys_stderr(&mut self, text: &str) -> Result<(), RuntimeError> {
        self.write_text_to_sys_stream("stderr", text)
    }

    fn set_builtins_global(&mut self, name: &str, value: Value) -> Result<(), RuntimeError> {
        let Some(builtins) = self.modules.get("builtins").cloned() else {
            return Err(RuntimeError::with_exception(
                "RuntimeError",
                Some("lost builtins".to_string()),
            ));
        };
        let Object::Module(module_data) = &mut *builtins.kind_mut() else {
            return Err(RuntimeError::with_exception(
                "RuntimeError",
                Some("builtins module is invalid".to_string()),
            ));
        };
        module_data.globals.insert(name.to_string(), value.clone());
        self.builtins.insert(name.to_string(), value);
        self.touch_builtins_version();
        Ok(())
    }

    fn write_text_to_sys_stderr_best_effort(&mut self, text: &str) {
        if self.write_text_to_sys_stderr(text).is_err() {
            self.clear_active_exception();
            eprint!("{text}");
        }
    }

    fn system_exit_code_from_args(&self, args: &[Value]) -> Value {
        match args {
            [] => Value::None,
            [single] => single.clone(),
            _ => self.heap.alloc_tuple(args.to_vec()),
        }
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
        if let Some(union_items) = self.union_args_from_value(&value) {
            let render_union_member = |vm: &mut Vm, item: Value| -> Result<String, RuntimeError> {
                match item {
                    Value::None => Ok("None".to_string()),
                    Value::Builtin(builtin) => {
                        let name = vm.builtin_attribute_name(builtin);
                        if name == "NoneType" {
                            return Ok("None".to_string());
                        }
                        let module_name = match vm.load_attr_builtin(builtin, "__module__") {
                            Ok(Value::Str(module_name)) => module_name,
                            _ => "builtins".to_string(),
                        };
                        if module_name == "builtins" {
                            Ok(name)
                        } else {
                            Ok(format!("{module_name}.{name}"))
                        }
                    }
                    Value::Class(class) => {
                        let class_kind = class.kind();
                        let Object::Class(class_data) = &*class_kind else {
                            let Value::Str(text) =
                                vm.builtin_repr(vec![Value::Class(class.clone())], HashMap::new())?
                            else {
                                return Err(RuntimeError::type_error(
                                    "__repr__ returned non-string",
                                ));
                            };
                            return Ok(text);
                        };
                        let module_name = match class_data.attrs.get("__module__") {
                            Some(Value::Str(name)) => name.clone(),
                            _ => "builtins".to_string(),
                        };
                        let qualname = match class_data.attrs.get("__qualname__") {
                            Some(Value::Str(name)) => name.clone(),
                            _ => class_data.name.clone(),
                        };
                        if qualname == "NoneType" {
                            return Ok("None".to_string());
                        }
                        if module_name == "builtins" {
                            Ok(qualname)
                        } else {
                            Ok(format!("{module_name}.{qualname}"))
                        }
                    }
                    other => {
                        let has_origin = vm
                            .optional_getattr_value(other.clone(), "__origin__")?
                            .is_some();
                        let has_args = vm
                            .optional_getattr_value(other.clone(), "__args__")?
                            .is_some();
                        if !(has_origin && has_args) {
                            let qualname =
                                match vm.optional_getattr_value(other.clone(), "__qualname__")? {
                                    Some(Value::Str(text)) => Some(text),
                                    _ => None,
                                };
                            let module_name =
                                match vm.optional_getattr_value(other.clone(), "__module__")? {
                                    Some(Value::Str(text)) => Some(text),
                                    _ => None,
                                };
                            if let (Some(qualname), Some(module_name)) = (qualname, module_name) {
                                if module_name == "builtins" {
                                    return Ok(qualname);
                                }
                                return Ok(format!("{module_name}.{qualname}"));
                            }
                        }
                        let Value::Str(text) = vm.builtin_repr(vec![other], HashMap::new())? else {
                            return Err(RuntimeError::type_error("__repr__ returned non-string"));
                        };
                        Ok(text)
                    }
                }
            };
            let mut rendered = Vec::with_capacity(union_items.len());
            for item in union_items {
                rendered.push(render_union_member(self, item)?);
            }
            return Ok(Value::Str(rendered.join(" | ")));
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
        if let Value::Exception(exception) = &value {
            return Ok(Value::Str(self.exception_repr_value(exception)));
        }
        if matches!(&value, Value::Class(_)) {
            if let Some(repr_method) = self.lookup_bound_special_method(&value, "__repr__")? {
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
                    let class_id = match &value {
                        Value::Class(class_ref) => class_ref.id(),
                        _ => unreachable!(),
                    };
                    if self.repr_in_progress.contains(&class_id) {
                        return Ok(Value::Str(format_repr(&value)));
                    }
                    self.repr_in_progress.push(class_id);
                    let repr_outcome = self.call_internal(repr_method, Vec::new(), HashMap::new());
                    self.repr_in_progress.pop();
                    match repr_outcome? {
                        InternalCallOutcome::Value(Value::Str(text)) => {
                            return Ok(Value::Str(text));
                        }
                        InternalCallOutcome::Value(_) => {
                            return Err(RuntimeError::type_error("__repr__ returned non-string"));
                        }
                        InternalCallOutcome::CallerExceptionHandled => {
                            return Err(self.runtime_error_from_active_exception("repr() failed"));
                        }
                    }
                }
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
                        let repr_outcome =
                            self.call_internal(repr_method, Vec::new(), HashMap::new());
                        match repr_outcome? {
                            InternalCallOutcome::Value(Value::Str(text)) => {
                                return Ok(Value::Str(text));
                            }
                            InternalCallOutcome::Value(_) => {
                                return Err(RuntimeError::type_error(
                                    "__repr__ returned non-string",
                                ));
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
                _ => Err(RuntimeError::type_error("__repr__ returned non-string")),
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
                _ => Err(RuntimeError::type_error("__repr__ returned non-string")),
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
                _ => Err(RuntimeError::type_error("__repr__ returned non-string")),
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
                _ => Err(RuntimeError::type_error("__repr__ returned non-string")),
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
            return Err(RuntimeError::type_error(
                "hash() got an unexpected keyword argument",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error("hash() expects one argument"));
        }
        let target = args.remove(0);
        let hash_value = self.hash_value_runtime(&target)?;
        Ok(Value::Int(hash_value))
    }

    pub(super) fn hash_value_runtime(&mut self, value: &Value) -> Result<i64, RuntimeError> {
        let hash = self.hash_value_runtime_u64(value)?;
        let result = hash as i64;
        Ok(if result == -1 { -2 } else { result })
    }

    fn instance_declares_hash_attr(&self, instance: &ObjRef) -> bool {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return false;
        };
        match &*instance_data.class.kind() {
            Object::Class(class_data) => class_data.attrs.contains_key("__hash__"),
            _ => false,
        }
    }

    fn instance_hashable_backing_value_for_runtime_hash(&self, instance: &ObjRef) -> Option<Value> {
        if let Some(backing) = self.instance_backing_int(instance) {
            return Some(backing);
        }
        if let Some(backing) = self.instance_backing_float(instance) {
            return Some(Value::Float(backing));
        }
        if let Some((real, imag)) = self.instance_backing_complex(instance) {
            return Some(Value::Complex { real, imag });
        }
        if let Some(backing) = self.instance_backing_str(instance) {
            return Some(Value::Str(backing));
        }
        if let Some(backing) = self.instance_backing_tuple(instance) {
            return Some(Value::Tuple(backing));
        }
        if let Some(backing) = self.instance_backing_frozenset(instance) {
            return Some(Value::FrozenSet(backing));
        }
        let instance_kind = instance.kind();
        let Object::Instance(instance_data) = &*instance_kind else {
            return None;
        };
        match instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR) {
            Some(Value::Bytes(backing)) => Some(Value::Bytes(backing.clone())),
            _ => None,
        }
    }

    fn hash_value_runtime_u64(&mut self, value: &Value) -> Result<u64, RuntimeError> {
        const HASH_CACHE_LIMIT: usize = 65_536;
        if let Some(union_args) = self.union_args_from_value(value) {
            return self.hash_union_args_runtime(value, &union_args);
        }
        if let Some((origin, args)) = self.generic_alias_parts_from_value(value) {
            return self.hash_generic_alias_parts_runtime(value, &origin, &args);
        }
        match value {
            Value::Instance(instance) => {
                if !self.instance_declares_hash_attr(instance)
                    && let Some(backing) =
                        self.instance_hashable_backing_value_for_runtime_hash(instance)
                {
                    return self.hash_value_runtime_u64(&backing);
                }
                if let Some(class_ref) = self.class_of_value(value)
                    && matches!(class_attr_lookup(&class_ref, "__hash__"), Some(Value::None))
                {
                    let type_name = if matches!(value, Value::Class(_)) {
                        match &*class_ref.kind() {
                            Object::Class(class_data) => class_data.name.clone(),
                            _ => self.value_type_name_for_error(value).to_string(),
                        }
                    } else {
                        self.value_type_name_for_error(value).to_string()
                    };
                    return Err(RuntimeError::type_error(format!(
                        "unhashable type: '{}'",
                        type_name
                    )));
                }
                let Some(hash_value) = self.call_special_method_with_fallback(
                    value,
                    "__hash__",
                    Vec::new(),
                    "hash special method raised",
                )?
                else {
                    return Err(RuntimeError::type_error(format!(
                        "unhashable type: '{}'",
                        self.value_type_name_for_error(value)
                    )));
                };
                let hash_value = value_to_int(hash_value)?;
                Ok(hash_value as u64)
            }
            Value::Class(_) | Value::Super(_) => {
                if let Some(class_ref) = self.class_of_value(value)
                    && matches!(class_attr_lookup(&class_ref, "__hash__"), Some(Value::None))
                {
                    let type_name = if matches!(value, Value::Class(_)) {
                        match &*class_ref.kind() {
                            Object::Class(class_data) => class_data.name.clone(),
                            _ => self.value_type_name_for_error(value).to_string(),
                        }
                    } else {
                        self.value_type_name_for_error(value).to_string()
                    };
                    return Err(RuntimeError::type_error(format!(
                        "unhashable type: '{}'",
                        type_name
                    )));
                }
                let Some(hash_value) = self.call_special_method_with_fallback(
                    value,
                    "__hash__",
                    Vec::new(),
                    "hash special method raised",
                )?
                else {
                    return Err(RuntimeError::type_error(format!(
                        "unhashable type: '{}'",
                        self.value_type_name_for_error(value)
                    )));
                };
                let hash_value = value_to_int(hash_value)?;
                Ok(hash_value as u64)
            }
            Value::Tuple(tuple) => {
                if let Some(cached) = self.hash_cache.get(&tuple.id()) {
                    return Ok(*cached);
                }
                let values = match &*tuple.kind() {
                    Object::Tuple(values) => values.clone(),
                    _ => {
                        return Err(RuntimeError::type_error(format!(
                            "unhashable type: '{}'",
                            self.value_type_name_for_error(value)
                        )));
                    }
                };
                let mut hasher = DefaultHasher::new();
                4u8.hash(&mut hasher);
                for item in values {
                    self.hash_value_runtime_u64(&item)?.hash(&mut hasher);
                }
                let hash = hasher.finish();
                if self.hash_cache.len() >= HASH_CACHE_LIMIT {
                    self.hash_cache.clear();
                }
                self.hash_cache.insert(tuple.id(), hash);
                Ok(hash)
            }
            Value::FrozenSet(set) => {
                if let Some(cached) = self.hash_cache.get(&set.id()) {
                    return Ok(*cached);
                }
                let values = match &*set.kind() {
                    Object::FrozenSet(values) => values.clone(),
                    _ => {
                        return Err(RuntimeError::type_error(format!(
                            "unhashable type: '{}'",
                            self.value_type_name_for_error(value)
                        )));
                    }
                };
                let mut hasher = DefaultHasher::new();
                5u8.hash(&mut hasher);
                let mut folded: u64 = 0;
                for item in values {
                    folded ^= self.hash_value_runtime_u64(&item)?;
                }
                folded.hash(&mut hasher);
                let hash = hasher.finish();
                if self.hash_cache.len() >= HASH_CACHE_LIMIT {
                    self.hash_cache.clear();
                }
                self.hash_cache.insert(set.id(), hash);
                Ok(hash)
            }
            _ => value_lookup_hash(value).ok_or_else(|| {
                RuntimeError::type_error(format!(
                    "unhashable type: '{}'",
                    self.value_type_name_for_error(value)
                ))
            }),
        }
    }

    fn hash_generic_alias_parts_runtime(
        &mut self,
        value: &Value,
        origin: &Value,
        args: &[Value],
    ) -> Result<u64, RuntimeError> {
        if let Some(literal_args) = self.literal_alias_args_from_value(value) {
            return self.literal_alias_args_hash_runtime(&literal_args);
        }
        let mut hasher = DefaultHasher::new();
        13u8.hash(&mut hasher);
        self.hash_value_runtime_u64(origin)?.hash(&mut hasher);
        args.len().hash(&mut hasher);
        for item in args {
            self.hash_value_runtime_u64(item)?.hash(&mut hasher);
        }
        if let Some(metadata) = self.annotated_alias_metadata_from_value(value) {
            metadata.len().hash(&mut hasher);
            for item in metadata {
                self.hash_value_runtime_u64(&item)?.hash(&mut hasher);
            }
        }
        Ok(hasher.finish())
    }

    fn hash_union_args_runtime(
        &mut self,
        value: &Value,
        args: &[Value],
    ) -> Result<u64, RuntimeError> {
        let cached_unhashable_count = if let Value::Instance(instance) = value {
            let instance_kind = instance.kind();
            if let Object::Instance(instance_data) = &*instance_kind {
                match instance_data.attrs.get("__pyrs_union_unhashable_count__") {
                    Some(Value::Int(count)) => *count,
                    _ => 0,
                }
            } else {
                0
            }
        } else {
            0
        };

        let mut element_hashes = Vec::with_capacity(args.len());
        let mut first_type_error_message = None::<String>;
        let mut type_error_count = 0i64;
        for item in args {
            match self.hash_value_runtime_u64(item) {
                Ok(hash) => element_hashes.push(hash),
                Err(err) if runtime_error_matches_exception(&err, "TypeError") => {
                    type_error_count += 1;
                    if first_type_error_message.is_none() {
                        first_type_error_message = Some(err.message);
                    }
                }
                Err(err) => return Err(err),
            }
        }

        if type_error_count == 0 && cached_unhashable_count > 1 {
            return Err(RuntimeError::type_error(format!(
                "union contains {} unhashable elements",
                cached_unhashable_count
            )));
        }

        if type_error_count > 0 {
            if type_error_count > 1
                && let Value::Instance(instance) = value
                && let Object::Instance(instance_data) = &mut *instance.kind_mut()
            {
                instance_data.attrs.insert(
                    "__pyrs_union_unhashable_count__".to_string(),
                    Value::Int(type_error_count),
                );
            }
            return Err(RuntimeError::type_error(
                first_type_error_message.unwrap_or_else(|| "unhashable union element".to_string()),
            ));
        }

        element_hashes.sort_unstable();

        let mut hasher = DefaultHasher::new();
        12u8.hash(&mut hasher);
        element_hashes.len().hash(&mut hasher);
        for hash in element_hashes {
            hash.hash(&mut hasher);
        }
        Ok(hasher.finish())
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

    pub(super) fn builtin_object_init_subclass(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "object.__init_subclass__() takes no keyword arguments",
            ));
        }
        match args.as_slice() {
            [] => Ok(Value::None),
            [Value::Class(_)] => Ok(Value::None),
            [..] => Err(RuntimeError::type_error(format!(
                "object.__init_subclass__() takes no arguments ({} given)",
                args.len()
            ))),
        }
    }

    pub(crate) fn dispatch_sys_audit_event(
        &mut self,
        event: &str,
        event_args: Vec<Value>,
    ) -> Result<(), RuntimeError> {
        if self.audit_hooks.is_empty() {
            return Ok(());
        }

        let mut index = 0usize;
        let event_name = Value::Str(event.to_string());
        let payload = self.heap.alloc_tuple(event_args);
        while index < self.audit_hooks.len() {
            let hook = self.audit_hooks[index].clone();
            index += 1;
            match self.call_internal(
                hook,
                vec![event_name.clone(), payload.clone()],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(_) | InternalCallOutcome::CallerExceptionHandled => {}
            }
        }
        Ok(())
    }

    pub(super) fn builtin_sys_addaudithook(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.addaudithook() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "sys.addaudithook() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let hook = args.remove(0);
        match self.dispatch_sys_audit_event("sys.addaudithook", Vec::new()) {
            Ok(()) => self.audit_hooks.push(hook),
            Err(err) => {
                if runtime_error_matches_exception(&err, "Exception") {
                    return Ok(Value::None);
                }
                return Err(err);
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_audit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.audit() takes no keyword arguments",
            ));
        }
        if args.is_empty() {
            return Err(RuntimeError::type_error(
                "audit expected at least 1 argument, got 0",
            ));
        }
        let event = args.remove(0);
        if !matches!(event, Value::Str(_)) {
            return Err(RuntimeError::type_error(format!(
                "audit() argument 1 must be str, not {}",
                self.value_type_name_for_error(&event)
            )));
        }
        let Value::Str(event_name) = event else {
            unreachable!("event type was validated above");
        };
        self.dispatch_sys_audit_event(&event_name, args)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_excepthook(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.excepthook() takes no keyword arguments",
            ));
        }
        if args.len() != 3 {
            return Err(RuntimeError::type_error(format!(
                "sys.excepthook() takes exactly 3 arguments ({} given)",
                args.len()
            )));
        }
        let value = args[1].clone();
        let normalized = match self.normalize_exception_value(value.clone()) {
            Ok(Value::Exception(exception)) => Value::Exception(exception),
            Ok(_) | Err(_) => {
                self.clear_active_exception();
                let rendered = format!(
                    "TypeError: print_exception(): Exception expected for value, {} found",
                    self.value_type_name_for_error(&value)
                );
                self.write_text_to_sys_stderr_best_effort(&format!("{rendered}\n"));
                return Ok(Value::None);
            }
        };
        let mut rendered = self.format_traceback(&[], &normalized);
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        self.write_text_to_sys_stderr_best_effort(&rendered);
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_displayhook(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.displayhook() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "sys.displayhook() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let value = args[0].clone();
        if matches!(value, Value::None) {
            return Ok(Value::None);
        }

        // Match CPython: clear builtins._ before repr/display.
        self.set_builtins_global("_", Value::None)?;
        let rendered = match self.builtin_repr(vec![value.clone()], HashMap::new())? {
            Value::Str(text) => text,
            other => {
                return Err(RuntimeError::type_error(format!(
                    "__repr__ returned non-string (type {})",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        self.write_text_to_sys_stream("stdout", &format!("{rendered}\n"))?;
        self.set_builtins_global("_", value)?;
        Ok(Value::None)
    }

    fn invoke_sys_displayhook(&mut self, value: Value) -> Result<(), RuntimeError> {
        let Some(sys_module) = self.modules.get("sys").cloned() else {
            return Err(RuntimeError::runtime_error("lost sys.displayhook"));
        };
        let displayhook = match self.builtin_getattr(
            vec![
                Value::Module(sys_module),
                Value::Str("displayhook".to_string()),
            ],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) if is_missing_attribute_error(&err) => {
                return Err(RuntimeError::runtime_error("lost sys.displayhook"));
            }
            Err(err) => return Err(err),
        };
        match self.call_internal(displayhook, vec![value], HashMap::new())? {
            InternalCallOutcome::Value(_) => Ok(()),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("sys.displayhook raised"))
            }
        }
    }

    pub(super) fn builtin_sys_clear_type_descriptors(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys._clear_type_descriptors() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "sys._clear_type_descriptors() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let target = args.remove(0);
        let Value::Class(class) = target else {
            return Err(RuntimeError::type_error(format!(
                "_clear_type_descriptors() argument must be type, not {}",
                self.value_type_name_for_error(&target)
            )));
        };
        let class_id = class.id();
        let class_kind = class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return Err(RuntimeError::type_error(
                "_clear_type_descriptors() argument must be type",
            ));
        };
        let flags = class_data
            .attrs
            .get("__flags__")
            .and_then(|value| match value {
                Value::Int(flags) => Some(*flags),
                _ => None,
            })
            .unwrap_or(0);
        drop(class_kind);
        let flags = self.cpython_proxy_type_flags(&class).unwrap_or(flags);
        if (flags & PY_TPFLAGS_HEAPTYPE) == 0 || (flags & PY_TPFLAGS_IMMUTABLETYPE) != 0 {
            return Err(RuntimeError::type_error("argument is immutable"));
        }
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data.attrs.remove("__dict__");
            class_data.attrs.remove("__weakref__");
        }
        self.touch_class_attr_version_by_id(class_id);
        Ok(Value::None)
    }

    fn runtime_warning(&mut self, message: String) -> Result<(), RuntimeError> {
        eprintln!("RuntimeWarning: {message}");
        Ok(())
    }

    pub(super) fn builtin_sys_breakpointhook(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let mut setting = self.host.env_var("PYTHONBREAKPOINT").unwrap_or_default();
        if setting.is_empty() {
            setting = "pdb.set_trace".to_string();
        } else if setting == "0" {
            return Ok(Value::None);
        }

        let (module_name, attr_name) = if let Some(last_dot) = setting.rfind('.') {
            if last_dot == 0 {
                let message = format!("Ignoring unimportable $PYTHONBREAKPOINT: \"{setting}\"");
                self.runtime_warning(message)?;
                return Ok(Value::None);
            }
            (&setting[..last_dot], &setting[last_dot + 1..])
        } else {
            ("builtins", setting.as_str())
        };

        let module = if module_name == "builtins" {
            self.modules
                .get("builtins")
                .cloned()
                .ok_or_else(|| RuntimeError::module_not_found_error("No module named 'builtins'"))?
        } else {
            match self.import_module_object(module_name) {
                Ok(module) => module,
                Err(err) => {
                    if runtime_error_matches_exception(&err, "ImportError") {
                        let message =
                            format!("Ignoring unimportable $PYTHONBREAKPOINT: \"{setting}\"");
                        self.runtime_warning(message)?;
                        return Ok(Value::None);
                    }
                    return Err(err);
                }
            }
        };

        let hook = match self.load_attr_module(&module, attr_name) {
            Ok(value) => value,
            Err(err) => {
                if runtime_error_matches_exception(&err, "AttributeError") {
                    let message = format!("Ignoring unimportable $PYTHONBREAKPOINT: \"{setting}\"");
                    self.runtime_warning(message)?;
                    return Ok(Value::None);
                }
                return Err(err);
            }
        };

        match self.call_internal(hook, args, kwargs)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self
                    .runtime_error_from_active_exception("sys.breakpointhook() raised exception"))
            }
        }
    }

    fn unraisable_args_from_value(
        &self,
        value: &Value,
    ) -> Result<(Value, Value, Value, Value, Value), RuntimeError> {
        let Value::Module(module) = value else {
            return Err(RuntimeError::type_error(
                "sys.unraisablehook argument type must be UnraisableHookArgs",
            ));
        };
        let module_kind = module.kind();
        let Object::Module(module_data) = &*module_kind else {
            return Err(RuntimeError::type_error(
                "sys.unraisablehook argument type must be UnraisableHookArgs",
            ));
        };
        if module_data.name != "__unraisable__" {
            return Err(RuntimeError::type_error(
                "sys.unraisablehook argument type must be UnraisableHookArgs",
            ));
        }
        let exc_type = module_data
            .globals
            .get("exc_type")
            .cloned()
            .ok_or_else(|| {
                RuntimeError::type_error(
                    "sys.unraisablehook argument type must be UnraisableHookArgs",
                )
            })?;
        let exc_value = module_data
            .globals
            .get("exc_value")
            .cloned()
            .ok_or_else(|| {
                RuntimeError::type_error(
                    "sys.unraisablehook argument type must be UnraisableHookArgs",
                )
            })?;
        let exc_traceback = module_data
            .globals
            .get("exc_traceback")
            .cloned()
            .ok_or_else(|| {
                RuntimeError::type_error(
                    "sys.unraisablehook argument type must be UnraisableHookArgs",
                )
            })?;
        let err_msg = module_data.globals.get("err_msg").cloned().ok_or_else(|| {
            RuntimeError::type_error("sys.unraisablehook argument type must be UnraisableHookArgs")
        })?;
        let object = module_data.globals.get("object").cloned().ok_or_else(|| {
            RuntimeError::type_error("sys.unraisablehook argument type must be UnraisableHookArgs")
        })?;
        Ok((exc_type, exc_value, exc_traceback, err_msg, object))
    }

    fn unraisable_type_name(&self, exc_type: &Value) -> String {
        match exc_type {
            Value::ExceptionType(name) => name.clone(),
            Value::Class(class) => match &*class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "<unknown>".to_string(),
            },
            Value::Exception(exception) => exception.name.clone(),
            _ => "<unknown>".to_string(),
        }
    }

    pub(super) fn builtin_sys_unraisablehook(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.unraisablehook() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "sys.unraisablehook() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let (exc_type, exc_value, _exc_traceback, err_msg, object) =
            self.unraisable_args_from_value(&args[0])?;

        if !matches!(object, Value::None) {
            if !matches!(err_msg, Value::None) {
                eprint!("{}: ", format_value(&err_msg));
            } else {
                eprint!("Exception ignored in: ");
            }
            eprintln!("{}", format_repr(&object));
        } else if !matches!(err_msg, Value::None) {
            eprintln!("{}:", format_value(&err_msg));
        }

        let type_name = self.unraisable_type_name(&exc_type);
        if matches!(exc_value, Value::None) {
            eprintln!("{type_name}");
        } else {
            eprintln!("{type_name}: {}", format_value(&exc_value));
        }
        Ok(Value::None)
    }

    fn sys_monitoring_parse_tool_id(&self, value: Value) -> Result<i64, RuntimeError> {
        let tool_id = value_to_int(value)?;
        if !(0..=MONITORING_MAX_USER_TOOL_ID).contains(&tool_id) {
            return Err(RuntimeError::value_error(format!(
                "invalid tool {tool_id} (must be between 0 and 5)"
            )));
        }
        Ok(tool_id)
    }

    fn sys_monitoring_clear_tool_state(&mut self, tool_id: i64) {
        self.monitoring_event_sets.remove(&tool_id);
        self.monitoring_local_event_sets
            .retain(|(stored_tool, _), _| *stored_tool != tool_id);
        self.monitoring_callbacks
            .retain(|(stored_tool, _), _| *stored_tool != tool_id);
    }

    fn monitoring_code_key_from_value(&self, code: Value) -> Result<usize, RuntimeError> {
        match code {
            Value::Code(code_obj) => Ok(Rc::as_ptr(&code_obj) as usize),
            _ => Err(RuntimeError::type_error("code must be a code object")),
        }
    }

    pub(super) fn builtin_sys_monitoring_get_tool(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.monitoring.get_tool() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "sys.monitoring.get_tool() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let tool_id = self.sys_monitoring_parse_tool_id(args.remove(0))?;
        Ok(self
            .monitoring_tool_names
            .get(&tool_id)
            .map(|name| Value::Str(name.clone()))
            .unwrap_or(Value::None))
    }

    pub(super) fn builtin_sys_monitoring_use_tool_id(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.monitoring.use_tool_id() takes no keyword arguments",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::type_error(format!(
                "use_tool_id expected 2 arguments, got {}",
                args.len()
            )));
        }
        let tool_id = self.sys_monitoring_parse_tool_id(args.remove(0))?;
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::value_error("tool name must be a str")),
        };
        if self.monitoring_tool_names.contains_key(&tool_id) {
            return Err(RuntimeError::value_error(format!(
                "tool {tool_id} is already in use"
            )));
        }
        self.monitoring_tool_names.insert(tool_id, name);
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_monitoring_clear_tool_id(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.monitoring.clear_tool_id() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "sys.monitoring.clear_tool_id() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let tool_id = self.sys_monitoring_parse_tool_id(args.remove(0))?;
        if self.monitoring_tool_names.contains_key(&tool_id) {
            self.sys_monitoring_clear_tool_state(tool_id);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_monitoring_free_tool_id(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.monitoring.free_tool_id() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "sys.monitoring.free_tool_id() takes exactly one argument ({} given)",
                args.len()
            )));
        }
        let tool_id = self.sys_monitoring_parse_tool_id(args.remove(0))?;
        if self.monitoring_tool_names.contains_key(&tool_id) {
            self.sys_monitoring_clear_tool_state(tool_id);
        }
        self.monitoring_tool_names.remove(&tool_id);
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_monitoring_register_callback(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.monitoring.register_callback() takes no keyword arguments",
            ));
        }
        if args.len() != 3 {
            return Err(RuntimeError::type_error(format!(
                "register_callback expected 3 arguments, got {}",
                args.len()
            )));
        }
        let tool_id = self.sys_monitoring_parse_tool_id(args.remove(0))?;
        let event = value_to_int(args.remove(0))?;
        if event <= 0 || (event as u64).count_ones() != 1 {
            return Err(RuntimeError::value_error(
                "The callback can only be set for one event at a time",
            ));
        }
        let event_id = (event as u64).trailing_zeros() as i64;
        if !(0..19).contains(&event_id) {
            return Err(RuntimeError::value_error(format!("invalid event {event}")));
        }
        let func = args.remove(0);
        self.builtin_sys_audit(
            vec![
                Value::Str("sys.monitoring.register_callback".to_string()),
                func.clone(),
            ],
            HashMap::new(),
        )?;
        let key = (tool_id, event_id);
        let previous = self
            .monitoring_callbacks
            .get(&key)
            .cloned()
            .unwrap_or(Value::None);
        if matches!(func, Value::None) {
            self.monitoring_callbacks.remove(&key);
        } else {
            self.monitoring_callbacks.insert(key, func);
        }
        Ok(previous)
    }

    pub(super) fn builtin_sys_monitoring_set_events(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.monitoring.set_events() takes no keyword arguments",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::type_error(format!(
                "set_events expected 2 arguments, got {}",
                args.len()
            )));
        }
        let tool_id = self.sys_monitoring_parse_tool_id(args.remove(0))?;
        let mut event_set = value_to_int(args.remove(0))?;
        if !(0..MONITORING_EVENT_SET_MAX).contains(&event_set) {
            return Err(RuntimeError::value_error(format!(
                "invalid event set 0x{:x}",
                event_set as u32
            )));
        }
        let c_return_events = MONITORING_EVENT_C_RETURN | MONITORING_EVENT_C_RAISE;
        let c_call_events = c_return_events | MONITORING_EVENT_CALL;
        if (event_set & c_return_events) != 0 && (event_set & c_call_events) != c_call_events {
            return Err(RuntimeError::value_error(
                "cannot set C_RETURN or C_RAISE events independently",
            ));
        }
        event_set &= !c_return_events;
        if (event_set & MONITORING_EVENT_BRANCH) != 0 {
            event_set &= !MONITORING_EVENT_BRANCH;
            event_set |= MONITORING_EVENT_BRANCH_LEFT | MONITORING_EVENT_BRANCH_RIGHT;
        }
        if !self.monitoring_tool_names.contains_key(&tool_id) {
            return Err(RuntimeError::value_error(format!(
                "tool {tool_id} is not in use"
            )));
        }
        self.monitoring_event_sets.insert(tool_id, event_set);
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_monitoring_get_events(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.monitoring.get_events() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "get_events expected 1 argument, got {}",
                args.len()
            )));
        }
        let tool_id = self.sys_monitoring_parse_tool_id(args.remove(0))?;
        let event_set = self
            .monitoring_event_sets
            .get(&tool_id)
            .copied()
            .unwrap_or(0);
        Ok(Value::Int(event_set))
    }

    pub(super) fn builtin_sys_monitoring_set_local_events(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.monitoring.set_local_events() takes no keyword arguments",
            ));
        }
        if args.len() != 3 {
            return Err(RuntimeError::type_error(format!(
                "set_local_events expected 3 arguments, got {}",
                args.len()
            )));
        }
        let tool = args.remove(0);
        let code_key = self.monitoring_code_key_from_value(args.remove(0))?;
        let tool_id = self.sys_monitoring_parse_tool_id(tool)?;
        let mut event_set = value_to_int(args.remove(0))?;
        let c_return_events = MONITORING_EVENT_C_RETURN | MONITORING_EVENT_C_RAISE;
        let c_call_events = c_return_events | MONITORING_EVENT_CALL;
        if (event_set & c_return_events) != 0 && (event_set & c_call_events) != c_call_events {
            return Err(RuntimeError::value_error(
                "cannot set C_RETURN or C_RAISE events independently",
            ));
        }
        event_set &= !c_return_events;
        if (event_set & MONITORING_EVENT_BRANCH) != 0 {
            event_set &= !MONITORING_EVENT_BRANCH;
            event_set |= MONITORING_EVENT_BRANCH_LEFT | MONITORING_EVENT_BRANCH_RIGHT;
        }
        if !(0..MONITORING_LOCAL_EVENT_SET_MAX).contains(&event_set) {
            return Err(RuntimeError::value_error(format!(
                "invalid local event set 0x{:x}",
                event_set as u32
            )));
        }
        if !self.monitoring_tool_names.contains_key(&tool_id) {
            return Err(RuntimeError::value_error(format!(
                "tool {tool_id} is not in use"
            )));
        }
        self.monitoring_local_event_sets
            .insert((tool_id, code_key), event_set);
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_monitoring_get_local_events(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.monitoring.get_local_events() takes no keyword arguments",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::type_error(format!(
                "get_local_events expected 2 arguments, got {}",
                args.len()
            )));
        }
        let tool = args.remove(0);
        let code_key = self.monitoring_code_key_from_value(args.remove(0))?;
        let tool_id = self.sys_monitoring_parse_tool_id(tool)?;
        let events = self
            .monitoring_local_event_sets
            .get(&(tool_id, code_key))
            .copied()
            .unwrap_or(0);
        Ok(Value::Int(events))
    }

    pub(super) fn builtin_sys_monitoring_restart_events(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "sys.monitoring.restart_events() takes no keyword arguments",
            ));
        }
        if !args.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "sys.monitoring.restart_events() takes no arguments ({} given)",
                args.len()
            )));
        }
        Ok(Value::None)
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
                let filename = "<string>";
                self.cache_source_text(filename, &source);
                let module_ast = parser::parse_module(&source).map_err(|err| {
                    self.compile_syntax_error_runtime_error(
                        err.message,
                        filename,
                        &source,
                        Some(err.line),
                        Some(err.column),
                    )
                })?;
                let code = Rc::new(
                    compiler::compile_module_with_filename(&module_ast, filename).map_err(
                        |err| {
                            self.compile_syntax_error_runtime_error(
                                err.message,
                                filename,
                                &source,
                                err.span.map(|span| span.line),
                                err.span.map(|span| span.column),
                            )
                        },
                    )?,
                );
                self.register_compiled_code_metadata(&code, CompiledCodeMode::Exec, Some(&source));
                code
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
        self.push_frame_checked(Box::new(frame))?;

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
                let filename = "<string>";
                self.cache_source_text(filename, &source_text);
                let expr_ast = parser::parse_expression(&source_text).map_err(|err| {
                    self.compile_syntax_error_runtime_error(
                        err.message,
                        filename,
                        &source_text,
                        Some(err.line),
                        Some(err.column),
                    )
                })?;
                let code = Rc::new(
                    compiler::compile_expression_with_filename(&expr_ast, filename).map_err(
                        |err| {
                            self.compile_syntax_error_runtime_error(
                                err.message,
                                filename,
                                &source_text,
                                err.span.map(|span| span.line),
                                err.span.map(|span| span.column),
                            )
                        },
                    )?,
                );
                self.register_compiled_code_metadata(
                    &code,
                    CompiledCodeMode::Eval,
                    Some(&source_text),
                );
                code
            }
        };
        if !code.freevars.is_empty() {
            return Err(RuntimeError::new(
                "eval() code object may not contain free variables",
            ));
        }

        let mut single_mode_displayhook = false;
        let single_mode_source = self.compiled_code_metadata(&code).and_then(|metadata| {
            if metadata.mode == CompiledCodeMode::Single {
                metadata.source.clone()
            } else {
                None
            }
        });
        let execution_code = if let Some(source_text) = single_mode_source.as_deref() {
            if let Ok(module_ast) = parser::parse_module(source_text) {
                if module_ast.body.len() == 1
                    && let StmtKind::Expr(expr) = &module_ast.body[0].node
                {
                    let expr_code =
                        compiler::compile_expression_with_filename(expr, &code.filename).map_err(
                            |err| {
                                self.compile_syntax_error_runtime_error(
                                    err.message,
                                    &code.filename,
                                    source_text,
                                    err.span.map(|span| span.line),
                                    err.span.map(|span| span.column),
                                )
                            },
                        )?;
                    single_mode_displayhook = true;
                    Rc::new(expr_code)
                } else {
                    code.clone()
                }
            } else {
                code.clone()
            }
        } else {
            code.clone()
        };

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
            let frame = &self.frames[caller_index];
            let mut locals = frame.locals.clone();
            for (idx, slot) in frame.fast_locals.iter().enumerate() {
                if let Some(value) = slot
                    && let Some(name) = frame.code.names.get(idx)
                {
                    locals.insert(name.clone(), value.clone());
                }
            }
            for name in &frame.code.cellvars {
                if let Some(value) = frame_cell_value(frame, name) {
                    locals.insert(name.clone(), value);
                }
            }
            for name in &frame.code.freevars {
                if let Some(value) = frame_cell_value(frame, name) {
                    locals.insert(name.clone(), value);
                }
            }
            locals
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
                Value::Instance(instance) => {
                    if let Some(dict) = self.instance_backing_dict(&instance) {
                        let module = self.alloc_exec_namespace_module(
                            "<eval_globals>",
                            self.exec_namespace_map_from_dict(&dict, "globals")?,
                        );
                        globals_dict_writeback = Some((dict, module.clone()));
                        globals_module = module;
                    } else {
                        return Err(RuntimeError::new("eval() globals must be a dict or module"));
                    }
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
                    Value::Instance(instance) => {
                        if let Some(dict) = self.instance_backing_dict(&instance) {
                            let mut namespace =
                                self.exec_namespace_map_from_dict(&dict, "locals")?;
                            for name in &execution_code.names {
                                if namespace.contains_key(name) {
                                    continue;
                                }
                                match self.getitem_value(
                                    Value::Instance(instance.clone()),
                                    Value::Str(name.clone()),
                                ) {
                                    Ok(value) => {
                                        namespace.insert(name.clone(), value);
                                    }
                                    Err(err)
                                        if runtime_error_matches_exception(&err, "KeyError") =>
                                    {
                                        self.clear_active_exception();
                                    }
                                    Err(err) => return Err(err),
                                }
                            }
                            let module =
                                self.alloc_exec_namespace_module("<eval_locals>", namespace);
                            locals_dict_writeback = Some((dict, module.clone()));
                            locals_module = module;
                        } else {
                            return Err(RuntimeError::new(
                                "eval() locals must be a dict or module",
                            ));
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

        let cells = self.build_cells(&execution_code, Vec::new());
        let mut frame = Frame::new(
            execution_code,
            locals_module.clone(),
            true,
            false,
            cells,
            None,
        );
        frame.function_globals = globals_module.clone();
        if locals_module.id() != globals_module.id() {
            frame.globals_fallback = Some(globals_module.clone());
        }
        self.push_frame_checked(Box::new(frame))?;

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
        let result = caller_frame.stack.pop().unwrap_or(Value::None);
        if single_mode_displayhook {
            self.invoke_sys_displayhook(result)?;
            Ok(Value::None)
        } else {
            Ok(result)
        }
    }

    pub(super) fn builtin_len(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error("len() takes no keyword arguments"));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error("len() expects one argument"));
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
            .ok_or_else(|| RuntimeError::type_error("len() expects one argument"))?;
        if let Some(proxy_result) = self.cpython_proxy_len(&receiver) {
            let raw = proxy_result?;
            return self.normalize_len_result(raw);
        }
        match BuiltinFunction::Len.call(&self.heap, vec![receiver.clone()]) {
            Ok(value) => Ok(value),
            Err(err) if runtime_error_matches_exception(&err, "TypeError") => {
                let Some(method) = self.lookup_bound_special_method(&receiver, "__len__")? else {
                    let type_name = self.value_type_name_for_error(&receiver);
                    return Err(RuntimeError::type_error(format!(
                        "object of type '{type_name}' has no len()",
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
                    Err(RuntimeError::value_error("__len__() should return >= 0"))
                } else {
                    Ok(Value::Int(number))
                }
            }
            Value::BigInt(number) => {
                if number.is_negative() {
                    return Err(RuntimeError::value_error("__len__() should return >= 0"));
                }
                let as_i64 = number.to_i64().ok_or_else(|| {
                    RuntimeError::overflow_error("len() result does not fit in an index")
                })?;
                Ok(Value::Int(as_i64))
            }
            other => Err(RuntimeError::type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                self.value_type_name_for_error(&other)
            ))),
        }
    }

    pub(super) fn builtin_collections_namedtuple_make(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "namedtuple._make() expects iterable",
            ));
        }
        let class = match &args[0] {
            Value::Class(class) => class.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "namedtuple._make() requires class receiver",
                ));
            }
        };
        let Some(fields) = self.class_namedtuple_fields(&class) else {
            return Err(RuntimeError::type_error(
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
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("dir() expects at most one argument"));
        }

        let mut names: Vec<String> = Vec::new();
        if let Some(target) = args.first() {
            let allow_custom_dir = DIR_CUSTOM_LOOKUP_DEPTH.with(|depth| depth.get() == 0);
            if allow_custom_dir
                && let Some(dir_method) = self.lookup_bound_special_method(target, "__dir__")?
            {
                struct DirLookupGuard;
                impl DirLookupGuard {
                    fn enter() -> Self {
                        DIR_CUSTOM_LOOKUP_DEPTH.with(|depth| {
                            depth.set(depth.get().saturating_add(1));
                        });
                        Self
                    }
                }
                impl Drop for DirLookupGuard {
                    fn drop(&mut self) {
                        DIR_CUSTOM_LOOKUP_DEPTH.with(|depth| {
                            depth.set(depth.get().saturating_sub(1));
                        });
                    }
                }
                let _guard = DirLookupGuard::enter();
                let dir_result = match self.call_internal(dir_method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(
                            self.runtime_error_from_active_exception("__dir__() call failed")
                        );
                    }
                };
                let dir_list = self.builtin_list(vec![dir_result], HashMap::new())?;
                let Value::List(dir_list_obj) = dir_list else {
                    return Err(RuntimeError::type_error(
                        "__dir__() must return an iterable of strings",
                    ));
                };
                let Object::List(dir_items) = &*dir_list_obj.kind() else {
                    return Err(RuntimeError::type_error(
                        "__dir__() must return an iterable of strings",
                    ));
                };
                for item in dir_items {
                    let Value::Str(name) = item else {
                        return Err(RuntimeError::type_error(
                            "__dir__() must return a list of strings",
                        ));
                    };
                    names.push(name.clone());
                }
                names.sort();
                names.dedup();
                return Ok(self
                    .heap
                    .alloc_list(names.into_iter().map(Value::Str).collect::<Vec<_>>()));
            }
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
                        names.extend(instance_data.attrs.keys().filter_map(|name| {
                            if matches!(
                                name.as_str(),
                                LIST_BACKING_STORAGE_ATTR
                                    | TUPLE_BACKING_STORAGE_ATTR
                                    | STR_BACKING_STORAGE_ATTR
                                    | BYTES_BACKING_STORAGE_ATTR
                                    | INT_BACKING_STORAGE_ATTR
                                    | FLOAT_BACKING_STORAGE_ATTR
                                    | COMPLEX_BACKING_STORAGE_ATTR
                                    | DICT_BACKING_STORAGE_ATTR
                                    | SET_BACKING_STORAGE_ATTR
                                    | FROZENSET_BACKING_STORAGE_ATTR
                                    | INSTANCE_DICT_STORAGE_ATTR
                                    | MAPPING_PROXY_STORAGE_ATTR
                            ) {
                                None
                            } else {
                                Some(name.clone())
                            }
                        }));
                        for entry in class_attr_walk(&instance_data.class) {
                            if let Object::Class(class_data) = &*entry.kind() {
                                names.extend(class_data.attrs.keys().filter_map(|name| {
                                    if matches!(
                                        name.as_str(),
                                        "__name__"
                                            | "__qualname__"
                                            | "__module__"
                                            | "__bases__"
                                            | "__mro__"
                                    ) {
                                        None
                                    } else {
                                        Some(name.clone())
                                    }
                                }));
                            }
                        }
                        if self.is_type_parameter_value(&Value::Instance(instance.clone()))
                            && matches!(
                                &*instance_data.class.kind(),
                                Object::Class(class_data) if class_data.name == "ParamSpec"
                            )
                        {
                            names.push("args".to_string());
                            names.push("kwargs".to_string());
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
        &mut self,
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
            return Err(RuntimeError::value_error("call stack is not deep enough"));
        }
        let depth = depth as usize;
        if depth >= self.frames.len() {
            return Err(RuntimeError::value_error("call stack is not deep enough"));
        }
        let frame_index = self.frames.len() - 1 - depth;
        Ok(self.build_frame_proxy_value(frame_index))
    }

    pub(super) fn builtin_sys_getframemodulename(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::type_error(
                "sys._getframemodulename() takes at most one argument",
            ));
        }
        let depth = if args.is_empty() {
            0
        } else {
            value_to_int(args.remove(0))?
        };
        // CPython accepts negative depth values and treats them as 0.
        let depth = depth.max(0) as usize;
        if depth >= self.frames.len() {
            return Ok(Value::None);
        }
        let frame_index = self.frames.len() - 1 - depth;
        let frame = &self.frames[frame_index];
        if let Object::Module(module_data) = &*frame.function_globals.kind() {
            if let Some(Value::Str(name)) = module_data.globals.get("__name__") {
                return Ok(Value::Str(name.clone()));
            }
            return Ok(Value::Str(module_data.name.clone()));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_current_frames(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::type_error(
                "sys._current_frames() takes no arguments",
            ));
        }
        let mut entries = Vec::new();
        if let Some(last_index) = self.frames.len().checked_sub(1) {
            entries.push((
                Value::Int(self.current_thread_ident_value()),
                self.build_frame_proxy_value(last_index),
            ));
        }
        Ok(self.heap.alloc_dict(entries))
    }

    fn current_frame_proxy_cache_key(&self) -> Vec<usize> {
        self.frames.iter().map(|frame| frame.frame_id).collect()
    }

    fn alloc_frame_proxy_module(&self) -> ObjRef {
        match self
            .heap
            .alloc_module(ModuleObject::new("<frame>".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        }
    }

    pub(super) fn builtins_mapping_value_from_dunder_builtins(
        &self,
        dunder_builtins: Option<Value>,
    ) -> Value {
        match dunder_builtins {
            Some(Value::Dict(dict)) => Value::Dict(dict),
            Some(Value::Module(module)) => {
                if let Object::Module(module_data) = &*module.kind() {
                    let mut entries = Vec::with_capacity(module_data.globals.len());
                    for (name, value) in module_data.globals.iter() {
                        entries.push((Value::Str(name.clone()), value.clone()));
                    }
                    self.heap.alloc_dict(entries)
                } else {
                    self.builtins_mapping_value_from_dunder_builtins(None)
                }
            }
            _ => {
                let mut entries = Vec::with_capacity(self.builtins.len());
                for (name, value) in self.builtins.iter() {
                    entries.push((Value::Str(name.clone()), value.clone()));
                }
                self.heap.alloc_dict(entries)
            }
        }
    }

    fn update_frame_proxy_module_for_frame(
        &mut self,
        frame_obj: &ObjRef,
        frame_index: usize,
        f_back: Value,
    ) {
        let (locals_value, globals_source, dunder_builtins, lineno, f_code) = {
            let frame = &self.frames[frame_index];
            let locals_value = if frame.is_module {
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
                let entries = Self::frame_trace(frame)
                    .local_values
                    .into_iter()
                    .map(|(name, value)| (Value::Str(name), value))
                    .collect::<Vec<_>>();
                let locals_dict = self.heap.alloc_dict(entries);
                let proxy_class = match self
                    .heap
                    .alloc_class(ClassObject::new("FrameLocalsProxy".to_string(), Vec::new()))
                {
                    Value::Class(class) => class,
                    _ => unreachable!(),
                };
                let proxy_instance =
                    match self.heap.alloc_instance(InstanceObject::new(proxy_class)) {
                        Value::Instance(instance) => instance,
                        _ => unreachable!(),
                    };
                if let Object::Instance(instance_data) = &mut *proxy_instance.kind_mut() {
                    instance_data
                        .attrs
                        .insert(DICT_BACKING_STORAGE_ATTR.to_string(), locals_dict);
                }
                Value::Instance(proxy_instance)
            };
            let globals_source = frame
                .globals_fallback
                .clone()
                .unwrap_or_else(|| frame.function_globals.clone());
            let dunder_builtins = if let Object::Module(module_data) = &*globals_source.kind() {
                module_data.globals.get("__builtins__").cloned()
            } else {
                None
            };
            let lineno = frame
                .code
                .locations
                .get(frame.last_ip)
                .map(|loc| loc.line)
                .unwrap_or(0);
            (
                locals_value,
                globals_source,
                dunder_builtins,
                lineno,
                Value::Code(frame.code.clone()),
            )
        };

        let globals_dict = if let Some(module_frame_index) = self
            .frames
            .iter()
            .rposition(|entry| entry.is_module && entry.module.id() == globals_source.id())
        {
            Value::Dict(self.ensure_frame_module_locals_dict(module_frame_index))
        } else if let Object::Module(module_data) = &mut *globals_source.kind_mut() {
            if let Some(mapping) = module_data
                .globals
                .get(Self::FUNCTION_GLOBALS_MAPPING_KEY)
                .cloned()
            {
                mapping
            } else {
                let mut entries = Vec::with_capacity(module_data.globals.len());
                for (name, value) in module_data.globals.iter() {
                    if name == Self::FUNCTION_GLOBALS_MAPPING_KEY {
                        continue;
                    }
                    entries.push((Value::Str(name.clone()), value.clone()));
                }
                let mapping = self.heap.alloc_dict(entries);
                module_data.globals.insert(
                    Self::FUNCTION_GLOBALS_MAPPING_KEY.to_string(),
                    mapping.clone(),
                );
                mapping
            }
        } else {
            self.heap.alloc_dict(Vec::new())
        };
        let builtins_dict = self.builtins_mapping_value_from_dunder_builtins(dunder_builtins);
        if let Object::Module(module_data) = &mut *frame_obj.kind_mut() {
            module_data.globals.clear();
            module_data
                .globals
                .insert("__pyrs_frame_proxy__".to_string(), Value::Bool(true));
            module_data
                .globals
                .insert("f_locals".to_string(), locals_value);
            module_data
                .globals
                .insert("f_globals".to_string(), globals_dict);
            module_data
                .globals
                .insert("f_builtins".to_string(), builtins_dict);
            module_data.globals.insert("f_code".to_string(), f_code);
            module_data
                .globals
                .insert("f_lineno".to_string(), Value::Int(lineno as i64));
            module_data.globals.insert("f_back".to_string(), f_back);
        }
    }

    pub(super) fn refresh_frame_proxy_cache(&mut self) {
        let key = self.current_frame_proxy_cache_key();
        if self.frame_proxy_cache_key.as_ref() != Some(&key)
            || self.frame_proxy_cache.len() != self.frames.len()
        {
            let mut existing_by_frame_id = HashMap::new();
            if let Some(existing_key) = &self.frame_proxy_cache_key {
                for (index, frame_id) in existing_key.iter().copied().enumerate() {
                    if let Some(proxy) = self.frame_proxy_cache.get(index) {
                        existing_by_frame_id
                            .entry(frame_id)
                            .or_insert(proxy.clone());
                    }
                }
            }
            let mut refreshed = Vec::with_capacity(key.len());
            for frame_id in &key {
                if let Some(proxy) = existing_by_frame_id.remove(frame_id) {
                    refreshed.push(proxy);
                } else {
                    refreshed.push(self.alloc_frame_proxy_module());
                }
            }
            self.frame_proxy_cache = refreshed;
        }

        for frame_index in 0..self.frames.len() {
            let f_back = if frame_index == 0 {
                Value::None
            } else {
                Value::Module(self.frame_proxy_cache[frame_index - 1].clone())
            };
            let frame_obj = self.frame_proxy_cache[frame_index].clone();
            self.update_frame_proxy_module_for_frame(&frame_obj, frame_index, f_back);
        }

        self.frame_proxy_cache_key = Some(key);
    }

    pub(super) fn refresh_frame_proxy_cache_if_active(&mut self, module: &ObjRef) {
        if self.frame_proxy_cache.is_empty() && self.frame_proxy_cache_key.is_none() {
            return;
        }
        if !self
            .frame_proxy_cache
            .iter()
            .any(|proxy| proxy.id() == module.id())
        {
            return;
        }
        self.refresh_frame_proxy_cache();
    }

    pub(super) fn build_frame_proxy_value(&mut self, frame_index: usize) -> Value {
        self.refresh_frame_proxy_cache();
        self.frame_proxy_cache
            .get(frame_index)
            .cloned()
            .map(Value::Module)
            .unwrap_or(Value::None)
    }

    fn preferred_active_exception(&self) -> Option<Value> {
        let mut fallback: Option<Value> = None;
        for frame in self.frames.iter().rev() {
            let Some(exc) = frame.active_exception.as_ref() else {
                continue;
            };
            if fallback.is_none() {
                fallback = Some(exc.clone());
            }
            let has_traceback = match exc {
                Value::Exception(exception) => {
                    !exception.traceback_frames.is_empty()
                        || exception.attrs.borrow().contains_key("__traceback__")
                }
                _ => false,
            };
            if has_traceback {
                return Some(exc.clone());
            }
        }
        fallback
    }

    pub(super) fn builtin_sys_exception(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("sys.exception() expects no arguments"));
        }
        Ok(self.preferred_active_exception().unwrap_or(Value::None))
    }

    pub(super) fn builtin_sys_exc_info(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("sys.exc_info() expects no arguments"));
        }
        if let Some(exc) = self.preferred_active_exception() {
            let exc_type = match &exc {
                Value::Exception(exception) => exception
                    .attrs
                    .borrow()
                    .get("__class__")
                    .and_then(|value| match value {
                        Value::Class(class) => Some(Value::Class(class.clone())),
                        _ => None,
                    })
                    .unwrap_or_else(|| Value::ExceptionType(exception.name.clone())),
                Value::ExceptionType(name) => Value::ExceptionType(name.clone()),
                _ => Value::None,
            };
            let exc_tb = match &exc {
                Value::Exception(exception) => {
                    if let Some(cached) = exception.attrs.borrow().get("__traceback__").cloned() {
                        cached
                    } else {
                        let traceback =
                            self.traceback_value_from_frames(&exception.traceback_frames);
                        exception
                            .attrs
                            .borrow_mut()
                            .insert("__traceback__".to_string(), traceback.clone());
                        traceback
                    }
                }
                _ => Value::None,
            };
            return Ok(self.heap.alloc_tuple(vec![exc_type, exc, exc_tb]));
        }
        Ok(self
            .heap
            .alloc_tuple(vec![Value::None, Value::None, Value::None]))
    }

    pub(super) fn builtin_sys_call_tracing(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "sys.call_tracing() expects callable and argument tuple",
            ));
        }
        let callable = args.remove(0);
        let call_args = match args.remove(0) {
            Value::Tuple(tuple) => match &*tuple.kind() {
                Object::Tuple(values) => values.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "sys.call_tracing() argument 2 must be tuple",
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::type_error(
                    "sys.call_tracing() argument 2 must be tuple",
                ));
            }
        };
        match self.call_internal(callable, call_args, HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("sys.call_tracing() failed"))
            }
        }
    }

    pub(super) fn builtin_sys_exit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::type_error(
                "sys.exit() takes at most one argument",
            ));
        }
        let call_args = match args.pop() {
            Some(Value::Tuple(tuple)) => {
                let tuple_values = {
                    let kind = tuple.kind();
                    match &*kind {
                        Object::Tuple(values) => Some(values.clone()),
                        _ => None,
                    }
                };
                tuple_values.unwrap_or_else(|| vec![Value::Tuple(tuple)])
            }
            Some(value) => vec![value],
            None => Vec::new(),
        };
        let code = self.system_exit_code_from_args(&call_args);
        let exception = ExceptionObject::new("SystemExit", call_args.first().map(format_value));
        {
            let mut attrs = exception.attrs.borrow_mut();
            attrs.insert("args".to_string(), self.heap.alloc_tuple(call_args));
            attrs.insert("code".to_string(), code);
        }
        Err(RuntimeError::from_exception(exception))
    }

    pub(super) fn builtin_sys_intern(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "intern() takes exactly one argument",
            ));
        }
        match args.remove(0) {
            Value::Str(text) => Ok(Value::Str(text)),
            _ => Err(RuntimeError::type_error("intern() argument must be str")),
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
        Ok(Value::Bool(self.is_finalizing))
    }

    pub(super) fn builtin_sys_is_gil_enabled(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::type_error(
                "sys._is_gil_enabled() takes no arguments",
            ));
        }
        Ok(Value::Bool(true))
    }

    pub(super) fn builtin_sys_is_remote_debug_enabled(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "sys.is_remote_debug_enabled() expects no arguments",
            ));
        }
        // pyrs does not currently expose remote debugger entrypoints.
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

    fn sizeof_hint_for_value(&self, value: &Value) -> i64 {
        const PTR: i64 = std::mem::size_of::<usize>() as i64;
        const BASE: i64 = 8 * PTR;
        match value {
            Value::None => BASE,
            Value::Bool(_) => BASE,
            Value::Int(_) | Value::Float(_) => BASE + 2 * PTR,
            Value::BigInt(value) => BASE + (value.bit_length() as i64 + 7) / 8,
            Value::Complex { .. } => BASE + 2 * (std::mem::size_of::<f64>() as i64),
            Value::Str(text) => BASE + text.len() as i64,
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => BASE + values.len() as i64 * PTR,
                _ => BASE,
            },
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => BASE + values.len() as i64 * PTR,
                _ => BASE,
            },
            Value::Dict(obj) => match &*obj.kind() {
                Object::Dict(values) => BASE + values.len() as i64 * (2 * PTR),
                _ => BASE,
            },
            Value::DictKeys(obj) => match &*obj.kind() {
                Object::DictKeysView(view) => match &*view.dict.kind() {
                    Object::Dict(values) => BASE + values.len() as i64 * PTR,
                    _ => BASE,
                },
                _ => BASE,
            },
            Value::Set(obj) => match &*obj.kind() {
                Object::Set(values) => BASE + values.len() as i64 * PTR,
                _ => BASE,
            },
            Value::FrozenSet(obj) => match &*obj.kind() {
                Object::FrozenSet(values) => BASE + values.len() as i64 * PTR,
                _ => BASE,
            },
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(values) => BASE + values.len() as i64,
                _ => BASE,
            },
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(values) => BASE + values.len() as i64,
                _ => BASE,
            },
            Value::MemoryView(obj) => match &*obj.kind() {
                Object::MemoryView(view) => {
                    let source_len = with_bytes_like_source(&view.source, |bytes| bytes.len())
                        .unwrap_or_default() as i64;
                    BASE + source_len
                }
                _ => BASE,
            },
            Value::Iterator(_) => BASE + 4 * PTR,
            Value::Generator(_) => BASE + 8 * PTR,
            Value::Module(obj) => match &*obj.kind() {
                Object::Module(module_data) => BASE + module_data.globals.len() as i64 * (2 * PTR),
                _ => BASE,
            },
            Value::Class(obj) => match &*obj.kind() {
                Object::Class(class_data) => BASE + class_data.attrs.len() as i64 * (2 * PTR),
                _ => BASE,
            },
            Value::Instance(obj) => match &*obj.kind() {
                Object::Instance(instance_data) => {
                    BASE + instance_data.attrs.len() as i64 * (2 * PTR)
                }
                _ => BASE,
            },
            Value::Super(_) | Value::BoundMethod(_) | Value::Function(_) | Value::Cell(_) => {
                BASE + 2 * PTR
            }
            Value::Exception(exception) => BASE + exception.attrs.borrow().len() as i64 * (2 * PTR),
            Value::ExceptionType(_) => BASE,
            Value::Slice(_) => BASE + 3 * PTR,
            Value::Code(code) => {
                BASE + (code.instructions.len() as i64
                    * std::mem::size_of::<crate::bytecode::Instruction>() as i64)
            }
            Value::Builtin(_) => BASE,
        }
    }

    pub(super) fn builtin_sys_getsizeof(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "sys.getsizeof() expects object and optional default",
            ));
        }
        let target = args.remove(0);
        let size = self.sizeof_hint_for_value(&target);
        if size >= 0 {
            return Ok(Value::Int(size));
        }
        if let Some(default) = args.pop() {
            Ok(default)
        } else {
            Err(RuntimeError::type_error(
                "sys.getsizeof() returned negative size",
            ))
        }
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
            return Err(RuntimeError::type_error(
                "sys.setrecursionlimit() expects one argument",
            ));
        }
        let limit = value_to_int(args[0].clone())?;
        if limit < 1 {
            return Err(RuntimeError::value_error(
                "recursion limit must be greater than 0",
            ));
        }
        self.recursion_limit = limit;
        Ok(Value::None)
    }

    pub(super) fn builtin_sys_getswitchinterval(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "sys.getswitchinterval() expects no arguments",
            ));
        }
        Ok(Value::Float(self.switch_interval))
    }

    pub(super) fn builtin_sys_setswitchinterval(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "sys.setswitchinterval() expects one argument",
            ));
        }
        let interval = match args[0].clone() {
            Value::Float(value) => value,
            Value::Int(value) => value as f64,
            Value::BigInt(value) => value.to_f64(),
            Value::Bool(value) => {
                if value {
                    1.0
                } else {
                    0.0
                }
            }
            other => {
                return Err(RuntimeError::type_error(format!(
                    "must be real number, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        if interval <= 0.0 {
            return Err(RuntimeError::value_error(
                "switch interval must be strictly positive",
            ));
        }
        self.switch_interval = interval;
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
                _ => {
                    return Err(RuntimeError::type_error(
                        "memoryview() expects bytes-like object",
                    ));
                }
            },
            Value::Module(obj) => {
                let is_array = matches!(&*obj.kind(), Object::Module(module_data) if module_data.name == "__array__");
                if is_array {
                    obj
                } else {
                    return Err(RuntimeError::type_error(
                        "memoryview() expects bytes-like object",
                    ));
                }
            }
            Value::Instance(obj) => {
                {
                    let kind = obj.kind();
                    let Object::Instance(instance_data) = &*kind else {
                        return Err(RuntimeError::type_error(
                            "memoryview() expects bytes-like object",
                        ));
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
                                RuntimeError::type_error("memoryview() expects bytes-like object")
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
                                return Err(RuntimeError::type_error(
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
                    return Err(RuntimeError::type_error(
                        "memoryview() expects bytes-like object",
                    ));
                }
            }
            other => {
                return Err(RuntimeError::type_error(format!(
                    "memoryview() expects bytes-like object, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        Ok(self.heap.alloc_memoryview(source))
    }

    pub(super) fn builtin_sys_stream_write(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        stderr: bool,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error("write() expects one argument"));
        }
        let text = format_value(&args[0]);
        if !self.capture_sys_stream_text(stderr, &text) {
            if stderr {
                eprint!("{text}");
            } else {
                print!("{text}");
            }
        }
        Ok(Value::Int(text.chars().count() as i64))
    }

    pub(super) fn builtin_sys_stream_buffer_write(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        stderr: bool,
    ) -> Result<Value, RuntimeError> {
        use std::io::Write;
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error("write() expects one argument"));
        }
        let payload = bytes_like_from_value(args[0].clone())?;
        if self.capture_sys_stream_output {
            let text = String::from_utf8_lossy(&payload);
            self.capture_sys_stream_text(stderr, &text);
        } else if stderr {
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
            return Err(RuntimeError::type_error("write() expects one argument"));
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

    pub(super) fn builtin_float_getformat(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "__getformat__() expects one argument",
            ));
        }
        let spec = match args.remove(0) {
            Value::Str(text) => text,
            other => {
                return Err(RuntimeError::type_error(format!(
                    "__getformat__() argument must be str, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        if spec != "double" && spec != "float" {
            return Err(RuntimeError::value_error(
                "__getformat__() argument 1 must be 'double' or 'float'",
            ));
        }
        #[cfg(target_endian = "little")]
        let format = "IEEE, little-endian";
        #[cfg(target_endian = "big")]
        let format = "IEEE, big-endian";
        #[cfg(not(any(target_endian = "little", target_endian = "big")))]
        let format = "unknown";
        Ok(Value::Str(format.to_string()))
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

    fn parse_fromhex_bytes(text: &str) -> Result<Vec<u8>, RuntimeError> {
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() / 2);
        let mut i = 0usize;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        while i < bytes.len() {
            let hi = match bytes[i] {
                b'0'..=b'9' => bytes[i] - b'0',
                b'a'..=b'f' => bytes[i] - b'a' + 10,
                b'A'..=b'F' => bytes[i] - b'A' + 10,
                _ => {
                    return Err(RuntimeError::value_error(format!(
                        "non-hexadecimal number found in fromhex() arg at position {i}"
                    )));
                }
            };
            i += 1;
            if i >= bytes.len() {
                return Err(RuntimeError::value_error(
                    "fromhex() arg must contain an even number of hexadecimal digits",
                ));
            }
            let lo = match bytes[i] {
                b'0'..=b'9' => bytes[i] - b'0',
                b'a'..=b'f' => bytes[i] - b'a' + 10,
                b'A'..=b'F' => bytes[i] - b'A' + 10,
                _ => {
                    return Err(RuntimeError::value_error(format!(
                        "non-hexadecimal number found in fromhex() arg at position {i}"
                    )));
                }
            };
            i += 1;
            out.push((hi << 4) | lo);
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
        }
        Ok(out)
    }

    pub(super) fn builtin_bytes_fromhex(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("fromhex() takes no keyword arguments"));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new("fromhex() expects one argument"));
        }
        let text = match args.remove(0) {
            Value::Str(text) => text,
            value => {
                return Err(RuntimeError::type_error(format!(
                    "fromhex() argument must be str, not {}",
                    self.value_type_name_for_error(&value)
                )));
            }
        };
        let bytes = Self::parse_fromhex_bytes(&text)?;
        Ok(self.heap.alloc_bytes(bytes))
    }

    pub(super) fn builtin_bytearray_fromhex(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("fromhex() takes no keyword arguments"));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new("fromhex() expects one argument"));
        }
        let text = match args.remove(0) {
            Value::Str(text) => text,
            value => {
                return Err(RuntimeError::type_error(format!(
                    "fromhex() argument must be str, not {}",
                    self.value_type_name_for_error(&value)
                )));
            }
        };
        let bytes = Self::parse_fromhex_bytes(&text)?;
        Ok(self.heap.alloc_bytearray(bytes))
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
                    Err(err)
                        if runtime_error_matches_exception(&err, "TypeError")
                            && err.message.contains("int() unsupported type") =>
                    {
                        if let Some(method) = self.lookup_bound_special_method(&arg, "__int__")? {
                            return match self.call_internal(method, Vec::new(), HashMap::new())? {
                                InternalCallOutcome::Value(value) => match value {
                                    Value::Int(_) | Value::BigInt(_) | Value::Bool(_) => {
                                        BuiltinFunction::Int.call(&self.heap, vec![value])
                                    }
                                    _ => Err(RuntimeError::new("__int__ returned non-int")),
                                },
                                InternalCallOutcome::CallerExceptionHandled => {
                                    Err(RuntimeError::type_error("int() unsupported type"))
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
                Err(RuntimeError::type_error("range() expects integers"))
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
            let keyword = kwargs
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "<unknown>".to_string());
            return Err(RuntimeError::type_error(format!(
                "range() got an unexpected keyword argument '{}'",
                keyword
            )));
        }
        match args.len() {
            0 => {}
            1 => {
                if stop.is_some() {
                    return Err(RuntimeError::type_error("range() got multiple values"));
                }
                stop = Some(args.remove(0));
            }
            2 => {
                if start.is_some() || stop.is_some() {
                    return Err(RuntimeError::type_error("range() got multiple values"));
                }
                start = Some(args.remove(0));
                stop = Some(args.remove(0));
            }
            3 => {
                if start.is_some() || stop.is_some() || step.is_some() {
                    return Err(RuntimeError::type_error("range() got multiple values"));
                }
                start = Some(args.remove(0));
                stop = Some(args.remove(0));
                step = Some(args.remove(0));
            }
            _ => return Err(RuntimeError::type_error("range() expects 1-3 arguments")),
        }

        let stop = stop
            .ok_or_else(|| RuntimeError::type_error("range expected at least 1 argument, got 0"))?;
        let start = start.unwrap_or(Value::Int(0));
        let step = step.unwrap_or(Value::Int(1));

        let start_big = self.coerce_index_bigint_for_range(start)?;
        let stop_big = self.coerce_index_bigint_for_range(stop)?;
        let step_big = self.coerce_index_bigint_for_range(step)?;
        if step_big.is_zero() {
            return Err(RuntimeError::value_error("range() step cannot be zero"));
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
            if let Value::Exception(exception) = &object {
                return Ok(Value::Str(self.exception_str_value(exception)));
            }
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
                            return Err(RuntimeError::value_error("byte must be in range(0, 256)"));
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

    fn ast_module_class(&self, class_name: &str) -> Option<ObjRef> {
        let module = self.modules.get("_ast")?.clone();
        let module_kind = module.kind();
        let module_data = match &*module_kind {
            Object::Module(module_data) => module_data,
            _ => return None,
        };
        match module_data.globals.get(class_name) {
            Some(Value::Class(class_ref)) => Some(class_ref.clone()),
            _ => None,
        }
    }

    fn build_ast_node(
        &mut self,
        class_name: &str,
        location: Option<(usize, usize)>,
        attrs: Vec<(&str, Value)>,
    ) -> Result<Value, RuntimeError> {
        let class_ref = self.ast_module_class(class_name).ok_or_else(|| {
            RuntimeError::new(format!("compile() missing _ast.{} support", class_name))
        })?;
        let mut instance = InstanceObject::new(class_ref);
        for (name, value) in attrs {
            instance.attrs.insert(name.to_string(), value);
        }
        if let Some((lineno, column)) = location {
            let col_offset = column.saturating_sub(1) as i64;
            instance
                .attrs
                .insert("lineno".to_string(), Value::Int(lineno as i64));
            instance
                .attrs
                .insert("col_offset".to_string(), Value::Int(col_offset));
            instance
                .attrs
                .insert("end_lineno".to_string(), Value::Int(lineno as i64));
            instance
                .attrs
                .insert("end_col_offset".to_string(), Value::Int(col_offset + 1));
        }
        Ok(self.heap.alloc_instance(instance))
    }

    fn build_ast_context_node(&mut self, class_name: &str) -> Result<Value, RuntimeError> {
        self.build_ast_node(class_name, None, Vec::new())
    }

    fn set_ast_node_end_location(
        &mut self,
        node: &Value,
        end_lineno: usize,
        end_col_offset: usize,
    ) {
        let Value::Instance(instance) = node else {
            return;
        };
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data
                .attrs
                .insert("end_lineno".to_string(), Value::Int(end_lineno as i64));
            instance_data.attrs.insert(
                "end_col_offset".to_string(),
                Value::Int(end_col_offset as i64),
            );
        }
    }

    fn infer_expr_end_location(expr: &Expr) -> Option<(usize, usize)> {
        let start = expr.span.column.saturating_sub(1);
        match &expr.node {
            ExprKind::Constant(constant) => {
                let width = match constant {
                    AstConstant::None => 4,
                    AstConstant::Bool(true) => 4,
                    AstConstant::Bool(false) => 5,
                    AstConstant::Int(value) => value.to_string().chars().count(),
                    AstConstant::Float(value) => value.value().to_string().chars().count(),
                    AstConstant::Str(text) => text.chars().count().saturating_add(2),
                };
                Some((expr.span.line, start + width.max(1)))
            }
            ExprKind::Name(name) => Some((expr.span.line, start + name.chars().count())),
            ExprKind::Attribute { value, name } => {
                let (value_end_line, value_end_col) = Self::infer_expr_end_location(value)?;
                if value_end_line == expr.span.line {
                    Some((value_end_line, value_end_col + 1 + name.chars().count()))
                } else {
                    Some((expr.span.line, start + 1))
                }
            }
            ExprKind::Call { func, args } => {
                let last_arg_end = args.last().and_then(|arg| match arg {
                    CallArg::Positional(value)
                    | CallArg::Star(value)
                    | CallArg::DoubleStar(value)
                    | CallArg::Keyword { value, .. } => Self::infer_expr_end_location(value),
                });
                if let Some((last_arg_end_line, last_arg_end_col)) = last_arg_end {
                    Some((last_arg_end_line, last_arg_end_col + 1))
                } else {
                    let (func_end_line, func_end_col) = Self::infer_expr_end_location(func)?;
                    Some((func_end_line, func_end_col + 2))
                }
            }
            _ => Some((expr.span.line, start + 1)),
        }
    }

    fn convert_ast_constant(&self, constant: &AstConstant) -> Value {
        match constant {
            AstConstant::None => Value::None,
            AstConstant::Bool(flag) => Value::Bool(*flag),
            AstConstant::Int(value) => Value::Int(*value),
            AstConstant::Float(value) => Value::Float(value.value()),
            AstConstant::Str(text) => Value::Str(text.clone()),
        }
    }

    fn ast_binary_operator_class_name(op: &AstBinaryOp) -> &'static str {
        match op {
            AstBinaryOp::Add => "Add",
            AstBinaryOp::Sub => "Sub",
            AstBinaryOp::Mul => "Mult",
            AstBinaryOp::MatMul => "MatMult",
            AstBinaryOp::Div => "Div",
            AstBinaryOp::Pow => "Pow",
            AstBinaryOp::FloorDiv => "FloorDiv",
            AstBinaryOp::Mod => "Mod",
            AstBinaryOp::LShift => "LShift",
            AstBinaryOp::RShift => "RShift",
            AstBinaryOp::BitAnd => "BitAnd",
            AstBinaryOp::BitXor => "BitXor",
            AstBinaryOp::BitOr => "BitOr",
            AstBinaryOp::Eq => "Eq",
            AstBinaryOp::Ne => "NotEq",
            AstBinaryOp::Lt => "Lt",
            AstBinaryOp::Le => "LtE",
            AstBinaryOp::Gt => "Gt",
            AstBinaryOp::Ge => "GtE",
            AstBinaryOp::In => "In",
            AstBinaryOp::NotIn => "NotIn",
            AstBinaryOp::Is => "Is",
            AstBinaryOp::IsNot => "IsNot",
        }
    }

    fn ast_binary_operator_is_compare(op: &AstBinaryOp) -> bool {
        matches!(
            op,
            AstBinaryOp::Eq
                | AstBinaryOp::Ne
                | AstBinaryOp::Lt
                | AstBinaryOp::Le
                | AstBinaryOp::Gt
                | AstBinaryOp::Ge
                | AstBinaryOp::In
                | AstBinaryOp::NotIn
                | AstBinaryOp::Is
                | AstBinaryOp::IsNot
        )
    }

    fn ast_unary_operator_class_name(op: &AstUnaryOp) -> &'static str {
        match op {
            AstUnaryOp::Neg => "USub",
            AstUnaryOp::Not => "Not",
            AstUnaryOp::Pos => "UAdd",
            AstUnaryOp::Invert => "Invert",
        }
    }

    fn ast_bool_operator_class_name(op: &AstBoolOp) -> &'static str {
        match op {
            AstBoolOp::And => "And",
            AstBoolOp::Or => "Or",
        }
    }

    fn ast_aug_operator_class_name(op: &AstAugOp) -> &'static str {
        match op {
            AstAugOp::Add => "Add",
            AstAugOp::Sub => "Sub",
            AstAugOp::Mul => "Mult",
            AstAugOp::MatMul => "MatMult",
            AstAugOp::Div => "Div",
            AstAugOp::Mod => "Mod",
            AstAugOp::FloorDiv => "FloorDiv",
            AstAugOp::Pow => "Pow",
            AstAugOp::LShift => "LShift",
            AstAugOp::RShift => "RShift",
            AstAugOp::BitAnd => "BitAnd",
            AstAugOp::BitXor => "BitXor",
            AstAugOp::BitOr => "BitOr",
        }
    }

    fn convert_stmt_list_to_ast_values(
        &mut self,
        statements: &[Stmt],
    ) -> Result<Vec<Value>, RuntimeError> {
        let mut out = Vec::with_capacity(statements.len());
        for statement in statements {
            out.push(self.convert_stmt_to_ast_node(statement)?);
        }
        Ok(out)
    }

    fn convert_import_alias_to_ast_node(
        &mut self,
        alias: &AstImportAlias,
        location: Option<(usize, usize)>,
    ) -> Result<Value, RuntimeError> {
        self.build_ast_node(
            "alias",
            location,
            vec![
                ("name", Value::Str(alias.name.clone())),
                (
                    "asname",
                    alias
                        .asname
                        .as_ref()
                        .map(|value| Value::Str(value.clone()))
                        .unwrap_or(Value::None),
                ),
            ],
        )
    }

    fn convert_except_handler_to_ast_node(
        &mut self,
        handler: &AstExceptHandler,
        location: Option<(usize, usize)>,
    ) -> Result<Value, RuntimeError> {
        let body = self.convert_stmt_list_to_ast_values(&handler.body)?;
        let type_node = match &handler.type_expr {
            Some(expr) => self.convert_expr_to_ast_node(expr)?,
            None => Value::None,
        };
        self.build_ast_node(
            "ExceptHandler",
            location,
            vec![
                ("type", type_node),
                (
                    "name",
                    handler
                        .name
                        .as_ref()
                        .map(|value| Value::Str(value.clone()))
                        .unwrap_or(Value::None),
                ),
                ("body", self.heap.alloc_list(body)),
            ],
        )
    }

    fn convert_parameter_to_ast_arg_node(
        &mut self,
        parameter: &AstParameter,
        location: Option<(usize, usize)>,
    ) -> Result<Value, RuntimeError> {
        let annotation = match &parameter.annotation {
            Some(annotation) => self.convert_expr_to_ast_node(annotation)?,
            None => Value::None,
        };
        self.build_ast_node(
            "arg",
            location,
            vec![
                ("arg", Value::Str(parameter.name.clone())),
                ("annotation", annotation),
                ("type_comment", Value::None),
            ],
        )
    }

    fn convert_type_params_to_ast_nodes(
        &mut self,
        type_params: &[AstTypeParam],
        location: Option<(usize, usize)>,
    ) -> Result<Vec<Value>, RuntimeError> {
        let mut nodes = Vec::with_capacity(type_params.len());
        for param in type_params {
            let (class_name, include_bound) = match param.kind {
                AstTypeParamKind::ParamSpec => ("ParamSpec", false),
                AstTypeParamKind::TypeVarTuple => ("TypeVarTuple", false),
                AstTypeParamKind::TypeVar => ("TypeVar", true),
            };
            let mut fields = vec![
                ("name", Value::Str(param.name.clone())),
                (
                    "default_value",
                    match &param.default {
                        Some(default) => self.convert_expr_to_ast_node(default)?,
                        None => Value::None,
                    },
                ),
            ];
            if include_bound {
                fields.insert(
                    1,
                    (
                        "bound",
                        match &param.bound {
                            Some(bound) => self.convert_expr_to_ast_node(bound)?,
                            None => Value::None,
                        },
                    ),
                );
            }
            nodes.push(self.build_ast_node(class_name, location, fields)?);
        }
        Ok(nodes)
    }

    fn convert_function_arguments_to_ast_node(
        &mut self,
        posonly_params: &[AstParameter],
        params: &[AstParameter],
        vararg: &Option<AstParameter>,
        kwonly_params: &[AstParameter],
        kwarg: &Option<AstParameter>,
        location: Option<(usize, usize)>,
    ) -> Result<Value, RuntimeError> {
        let mut posonlyargs = Vec::with_capacity(posonly_params.len());
        for param in posonly_params {
            posonlyargs.push(self.convert_parameter_to_ast_arg_node(param, location)?);
        }
        let mut args = Vec::with_capacity(params.len());
        for param in params {
            args.push(self.convert_parameter_to_ast_arg_node(param, location)?);
        }
        let vararg_node = match vararg {
            Some(param) => self.convert_parameter_to_ast_arg_node(param, location)?,
            None => Value::None,
        };
        let mut kwonlyargs = Vec::with_capacity(kwonly_params.len());
        let mut kw_defaults = Vec::with_capacity(kwonly_params.len());
        for param in kwonly_params {
            kwonlyargs.push(self.convert_parameter_to_ast_arg_node(param, location)?);
            kw_defaults.push(match &param.default {
                Some(default) => self.convert_expr_to_ast_node(default)?,
                None => Value::None,
            });
        }
        let kwarg_node = match kwarg {
            Some(param) => self.convert_parameter_to_ast_arg_node(param, location)?,
            None => Value::None,
        };
        let mut defaults = Vec::new();
        let positional_refs: Vec<&AstParameter> =
            posonly_params.iter().chain(params.iter()).collect();
        if let Some(first_default) = positional_refs
            .iter()
            .position(|param| param.default.is_some())
        {
            for param in positional_refs.into_iter().skip(first_default) {
                defaults.push(match &param.default {
                    Some(default) => self.convert_expr_to_ast_node(default)?,
                    None => Value::None,
                });
            }
        }
        self.build_ast_node(
            "arguments",
            None,
            vec![
                ("posonlyargs", self.heap.alloc_list(posonlyargs)),
                ("args", self.heap.alloc_list(args)),
                ("vararg", vararg_node),
                ("kwonlyargs", self.heap.alloc_list(kwonlyargs)),
                ("kw_defaults", self.heap.alloc_list(kw_defaults)),
                ("kwarg", kwarg_node),
                ("defaults", self.heap.alloc_list(defaults)),
            ],
        )
    }

    fn apply_decorators_to_ast_node(
        &mut self,
        node: Value,
        decorators: &[Expr],
    ) -> Result<Value, RuntimeError> {
        if decorators.is_empty() {
            return Ok(node);
        }
        let mut decorator_nodes = Vec::with_capacity(decorators.len());
        for decorator in decorators {
            decorator_nodes.push(self.convert_expr_to_ast_node(decorator)?);
        }
        let list_value = self.heap.alloc_list(decorator_nodes);
        if let Value::Instance(instance) = &node
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
            && instance_data.attrs.contains_key("decorator_list")
        {
            instance_data
                .attrs
                .insert("decorator_list".to_string(), list_value);
        }
        Ok(node)
    }

    fn convert_pattern_to_ast_node(
        &mut self,
        pattern: &AstPattern,
        location: Option<(usize, usize)>,
    ) -> Result<Value, RuntimeError> {
        match pattern {
            AstPattern::Wildcard => self.build_ast_node(
                "MatchAs",
                location,
                vec![("pattern", Value::None), ("name", Value::None)],
            ),
            AstPattern::Capture(name) => self.build_ast_node(
                "MatchAs",
                location,
                vec![("pattern", Value::None), ("name", Value::Str(name.clone()))],
            ),
            AstPattern::Constant(value) => match value {
                AstConstant::None | AstConstant::Bool(_) => self.build_ast_node(
                    "MatchSingleton",
                    location,
                    vec![("value", self.convert_ast_constant(value))],
                ),
                _ => {
                    let constant_node = self.build_ast_node(
                        "Constant",
                        location,
                        vec![("value", self.convert_ast_constant(value))],
                    )?;
                    self.build_ast_node("MatchValue", location, vec![("value", constant_node)])
                }
            },
            AstPattern::Value(expr) => {
                let value_node = self.convert_expr_to_ast_node(expr)?;
                self.build_ast_node("MatchValue", location, vec![("value", value_node)])
            }
            AstPattern::Sequence(items) => {
                let mut converted = Vec::with_capacity(items.len());
                for item in items {
                    converted.push(self.convert_pattern_to_ast_node(item, location)?);
                }
                self.build_ast_node(
                    "MatchSequence",
                    location,
                    vec![("patterns", self.heap.alloc_list(converted))],
                )
            }
            AstPattern::Mapping { entries, rest } => {
                let mut keys = Vec::with_capacity(entries.len());
                let mut patterns = Vec::with_capacity(entries.len());
                for (key, value_pattern) in entries {
                    keys.push(self.convert_expr_to_ast_node(key)?);
                    patterns.push(self.convert_pattern_to_ast_node(value_pattern, location)?);
                }
                self.build_ast_node(
                    "MatchMapping",
                    location,
                    vec![
                        ("keys", self.heap.alloc_list(keys)),
                        ("patterns", self.heap.alloc_list(patterns)),
                        (
                            "rest",
                            rest.as_ref()
                                .map(|name| Value::Str(name.clone()))
                                .unwrap_or(Value::None),
                        ),
                    ],
                )
            }
            AstPattern::Class {
                class,
                positional,
                keywords,
            } => {
                let cls = self.convert_expr_to_ast_node(class)?;
                let mut pos_patterns = Vec::with_capacity(positional.len());
                for value in positional {
                    pos_patterns.push(self.convert_pattern_to_ast_node(value, location)?);
                }
                let mut kwd_attrs = Vec::with_capacity(keywords.len());
                let mut kwd_patterns = Vec::with_capacity(keywords.len());
                for (name, value_pattern) in keywords {
                    kwd_attrs.push(Value::Str(name.clone()));
                    kwd_patterns.push(self.convert_pattern_to_ast_node(value_pattern, location)?);
                }
                self.build_ast_node(
                    "MatchClass",
                    location,
                    vec![
                        ("cls", cls),
                        ("patterns", self.heap.alloc_list(pos_patterns)),
                        ("kwd_attrs", self.heap.alloc_list(kwd_attrs)),
                        ("kwd_patterns", self.heap.alloc_list(kwd_patterns)),
                    ],
                )
            }
            AstPattern::Or(patterns) => {
                let mut converted = Vec::with_capacity(patterns.len());
                for value in patterns {
                    converted.push(self.convert_pattern_to_ast_node(value, location)?);
                }
                self.build_ast_node(
                    "MatchOr",
                    location,
                    vec![("patterns", self.heap.alloc_list(converted))],
                )
            }
            AstPattern::As { pattern, name } => {
                let pattern_node = self.convert_pattern_to_ast_node(pattern, location)?;
                self.build_ast_node(
                    "MatchAs",
                    location,
                    vec![
                        ("pattern", pattern_node),
                        ("name", Value::Str(name.clone())),
                    ],
                )
            }
            AstPattern::Star(name) => self.build_ast_node(
                "MatchStar",
                location,
                vec![(
                    "name",
                    name.as_ref()
                        .map(|value| Value::Str(value.clone()))
                        .unwrap_or(Value::None),
                )],
            ),
        }
    }

    fn convert_match_case_to_ast_node(
        &mut self,
        case: &AstMatchCase,
        location: Option<(usize, usize)>,
    ) -> Result<Value, RuntimeError> {
        let pattern = self.convert_pattern_to_ast_node(&case.pattern, location)?;
        let guard = match &case.guard {
            Some(expr) => self.convert_expr_to_ast_node(expr)?,
            None => Value::None,
        };
        let body = self.convert_stmt_list_to_ast_values(&case.body)?;
        self.build_ast_node(
            "match_case",
            None,
            vec![
                ("pattern", pattern),
                ("guard", guard),
                ("body", self.heap.alloc_list(body)),
            ],
        )
    }

    fn convert_comprehension_clauses_to_ast_nodes(
        &mut self,
        clauses: &[AstComprehensionClause],
        location: Option<(usize, usize)>,
    ) -> Result<Vec<Value>, RuntimeError> {
        let mut out = Vec::with_capacity(clauses.len());
        for clause in clauses {
            let target = self.convert_assign_target_to_ast_expr(&clause.target)?;
            let iter = self.convert_expr_to_ast_node(&clause.iter)?;
            let mut ifs = Vec::with_capacity(clause.ifs.len());
            for predicate in &clause.ifs {
                ifs.push(self.convert_expr_to_ast_node(predicate)?);
            }
            out.push(self.build_ast_node(
                "comprehension",
                location,
                vec![
                    ("target", target),
                    ("iter", iter),
                    ("ifs", self.heap.alloc_list(ifs)),
                    ("is_async", Value::Int(if clause.is_async { 1 } else { 0 })),
                ],
            )?);
        }
        Ok(out)
    }

    fn convert_assign_target_to_ast_expr(
        &mut self,
        target: &AssignTarget,
    ) -> Result<Value, RuntimeError> {
        match target {
            AssignTarget::Name(name) => {
                let ctx = self.build_ast_context_node("Store")?;
                self.build_ast_node(
                    "Name",
                    None,
                    vec![("id", Value::Str(name.clone())), ("ctx", ctx)],
                )
            }
            AssignTarget::Starred(inner) => {
                let value = self.convert_assign_target_to_ast_expr(inner)?;
                let ctx = self.build_ast_context_node("Store")?;
                self.build_ast_node("Starred", None, vec![("value", value), ("ctx", ctx)])
            }
            AssignTarget::Tuple(items) => {
                let mut converted = Vec::with_capacity(items.len());
                for item in items {
                    converted.push(self.convert_assign_target_to_ast_expr(item)?);
                }
                let ctx = self.build_ast_context_node("Store")?;
                self.build_ast_node(
                    "Tuple",
                    None,
                    vec![("elts", self.heap.alloc_list(converted)), ("ctx", ctx)],
                )
            }
            AssignTarget::List(items) => {
                let mut converted = Vec::with_capacity(items.len());
                for item in items {
                    converted.push(self.convert_assign_target_to_ast_expr(item)?);
                }
                let ctx = self.build_ast_context_node("Store")?;
                self.build_ast_node(
                    "List",
                    None,
                    vec![("elts", self.heap.alloc_list(converted)), ("ctx", ctx)],
                )
            }
            AssignTarget::Subscript { value, index } => {
                let value_node = self.convert_expr_to_ast_node(value)?;
                let index_node = self.convert_expr_to_ast_node(index)?;
                let ctx = self.build_ast_context_node("Store")?;
                self.build_ast_node(
                    "Subscript",
                    None,
                    vec![("value", value_node), ("slice", index_node), ("ctx", ctx)],
                )
            }
            AssignTarget::Attribute { value, name } => {
                let value_node = self.convert_expr_to_ast_node(value)?;
                let ctx = self.build_ast_context_node("Store")?;
                self.build_ast_node(
                    "Attribute",
                    None,
                    vec![
                        ("value", value_node),
                        ("attr", Value::Str(name.clone())),
                        ("ctx", ctx),
                    ],
                )
            }
        }
    }

    fn convert_expr_to_ast_node(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        let location = Some((expr.span.line, expr.span.column));
        match &expr.node {
            ExprKind::Name(name) => {
                let ctx = self.build_ast_context_node("Load")?;
                let node = self.build_ast_node(
                    "Name",
                    location,
                    vec![("id", Value::Str(name.clone())), ("ctx", ctx)],
                )?;
                if let Some((end_line, end_col_offset)) = Self::infer_expr_end_location(expr) {
                    self.set_ast_node_end_location(&node, end_line, end_col_offset);
                }
                Ok(node)
            }
            ExprKind::Constant(value) => self.build_ast_node(
                "Constant",
                location,
                vec![("value", self.convert_ast_constant(value))],
            ),
            ExprKind::Binary { left, op, right } => {
                let left_node = self.convert_expr_to_ast_node(left)?;
                let right_node = self.convert_expr_to_ast_node(right)?;
                let op_name = Self::ast_binary_operator_class_name(op);
                let op_node = self.build_ast_node(op_name, None, Vec::new())?;
                if Self::ast_binary_operator_is_compare(op) {
                    self.build_ast_node(
                        "Compare",
                        location,
                        vec![
                            ("left", left_node),
                            ("ops", self.heap.alloc_list(vec![op_node])),
                            ("comparators", self.heap.alloc_list(vec![right_node])),
                        ],
                    )
                } else {
                    self.build_ast_node(
                        "BinOp",
                        location,
                        vec![("left", left_node), ("op", op_node), ("right", right_node)],
                    )
                }
            }
            ExprKind::Unary { op, operand } => {
                let operand_node = self.convert_expr_to_ast_node(operand)?;
                let op_name = Self::ast_unary_operator_class_name(op);
                let op_node = self.build_ast_node(op_name, None, Vec::new())?;
                self.build_ast_node(
                    "UnaryOp",
                    location,
                    vec![("op", op_node), ("operand", operand_node)],
                )
            }
            ExprKind::BoolOp { op, left, right } => {
                let op_name = Self::ast_bool_operator_class_name(op);
                let op_node = self.build_ast_node(op_name, None, Vec::new())?;
                let left_node = self.convert_expr_to_ast_node(left)?;
                let right_node = self.convert_expr_to_ast_node(right)?;
                self.build_ast_node(
                    "BoolOp",
                    location,
                    vec![
                        ("op", op_node),
                        ("values", self.heap.alloc_list(vec![left_node, right_node])),
                    ],
                )
            }
            ExprKind::IfExpr { test, body, orelse } => {
                let test_node = self.convert_expr_to_ast_node(test)?;
                let body_node = self.convert_expr_to_ast_node(body)?;
                let orelse_node = self.convert_expr_to_ast_node(orelse)?;
                self.build_ast_node(
                    "IfExp",
                    location,
                    vec![
                        ("test", test_node),
                        ("body", body_node),
                        ("orelse", orelse_node),
                    ],
                )
            }
            ExprKind::NamedExpr { target, value } => {
                let value_node = self.convert_expr_to_ast_node(value)?;
                let store_ctx = self.build_ast_context_node("Store")?;
                let target_node = self.build_ast_node(
                    "Name",
                    None,
                    vec![("id", Value::Str(target.clone())), ("ctx", store_ctx)],
                )?;
                self.build_ast_node(
                    "NamedExpr",
                    location,
                    vec![("target", target_node), ("value", value_node)],
                )
            }
            ExprKind::Lambda {
                posonly_params,
                params,
                vararg,
                kwarg,
                kwonly_params,
                body,
            } => {
                let args_node = self.convert_function_arguments_to_ast_node(
                    posonly_params,
                    params,
                    vararg,
                    kwonly_params,
                    kwarg,
                    location,
                )?;
                let body_node = self.convert_expr_to_ast_node(body)?;
                self.build_ast_node(
                    "Lambda",
                    location,
                    vec![("args", args_node), ("body", body_node)],
                )
            }
            ExprKind::Await { value } => {
                let value_node = self.convert_expr_to_ast_node(value)?;
                self.build_ast_node("Await", location, vec![("value", value_node)])
            }
            ExprKind::ListComp { elt, clauses } => {
                let elt_node = self.convert_expr_to_ast_node(elt)?;
                let generators =
                    self.convert_comprehension_clauses_to_ast_nodes(clauses, location)?;
                self.build_ast_node(
                    "ListComp",
                    location,
                    vec![
                        ("elt", elt_node),
                        ("generators", self.heap.alloc_list(generators)),
                    ],
                )
            }
            ExprKind::SetComp { elt, clauses } => {
                let elt_node = self.convert_expr_to_ast_node(elt)?;
                let generators =
                    self.convert_comprehension_clauses_to_ast_nodes(clauses, location)?;
                self.build_ast_node(
                    "SetComp",
                    location,
                    vec![
                        ("elt", elt_node),
                        ("generators", self.heap.alloc_list(generators)),
                    ],
                )
            }
            ExprKind::DictComp {
                key,
                value,
                clauses,
            } => {
                let key_node = self.convert_expr_to_ast_node(key)?;
                let value_node = self.convert_expr_to_ast_node(value)?;
                let generators =
                    self.convert_comprehension_clauses_to_ast_nodes(clauses, location)?;
                self.build_ast_node(
                    "DictComp",
                    location,
                    vec![
                        ("key", key_node),
                        ("value", value_node),
                        ("generators", self.heap.alloc_list(generators)),
                    ],
                )
            }
            ExprKind::GeneratorExp { elt, clauses } => {
                let elt_node = self.convert_expr_to_ast_node(elt)?;
                let generators =
                    self.convert_comprehension_clauses_to_ast_nodes(clauses, location)?;
                self.build_ast_node(
                    "GeneratorExp",
                    location,
                    vec![
                        ("elt", elt_node),
                        ("generators", self.heap.alloc_list(generators)),
                    ],
                )
            }
            ExprKind::Yield { value } => {
                let value_node = match value {
                    Some(expr) => self.convert_expr_to_ast_node(expr)?,
                    None => Value::None,
                };
                self.build_ast_node("Yield", location, vec![("value", value_node)])
            }
            ExprKind::YieldFrom { value } => {
                let value_node = self.convert_expr_to_ast_node(value)?;
                self.build_ast_node("YieldFrom", location, vec![("value", value_node)])
            }
            ExprKind::Call { func, args } => {
                let mut positional = Vec::new();
                let mut keywords = Vec::new();
                for arg in args {
                    match arg {
                        CallArg::Positional(value) => {
                            positional.push(self.convert_expr_to_ast_node(value)?);
                        }
                        CallArg::Keyword { name, value } => {
                            let converted = self.convert_expr_to_ast_node(value)?;
                            keywords.push(self.build_ast_node(
                                "keyword",
                                location,
                                vec![("arg", Value::Str(name.clone())), ("value", converted)],
                            )?);
                        }
                        CallArg::Star(value) => {
                            let converted = self.convert_expr_to_ast_node(value)?;
                            let ctx = self.build_ast_context_node("Load")?;
                            positional.push(self.build_ast_node(
                                "Starred",
                                None,
                                vec![("value", converted), ("ctx", ctx)],
                            )?);
                        }
                        CallArg::DoubleStar(value) => {
                            let converted = self.convert_expr_to_ast_node(value)?;
                            keywords.push(self.build_ast_node(
                                "keyword",
                                location,
                                vec![("arg", Value::None), ("value", converted)],
                            )?);
                        }
                    }
                }
                let func_value = self.convert_expr_to_ast_node(func)?;
                let node = self.build_ast_node(
                    "Call",
                    location,
                    vec![
                        ("func", func_value),
                        ("args", self.heap.alloc_list(positional)),
                        ("keywords", self.heap.alloc_list(keywords)),
                    ],
                )?;
                if let Some((end_line, end_col_offset)) = Self::infer_expr_end_location(expr) {
                    self.set_ast_node_end_location(&node, end_line, end_col_offset);
                }
                Ok(node)
            }
            ExprKind::Attribute { value, name } => {
                let value_node = self.convert_expr_to_ast_node(value)?;
                let ctx = self.build_ast_context_node("Load")?;
                self.build_ast_node(
                    "Attribute",
                    location,
                    vec![
                        ("value", value_node),
                        ("attr", Value::Str(name.clone())),
                        ("ctx", ctx),
                    ],
                )
            }
            ExprKind::Tuple(items) => {
                let mut converted = Vec::with_capacity(items.len());
                for item in items {
                    converted.push(self.convert_expr_to_ast_node(item)?);
                }
                let ctx = self.build_ast_context_node("Load")?;
                self.build_ast_node(
                    "Tuple",
                    location,
                    vec![("elts", self.heap.alloc_list(converted)), ("ctx", ctx)],
                )
            }
            ExprKind::List(items) => {
                let mut converted = Vec::with_capacity(items.len());
                for item in items {
                    converted.push(self.convert_expr_to_ast_node(item)?);
                }
                let ctx = self.build_ast_context_node("Load")?;
                self.build_ast_node(
                    "List",
                    location,
                    vec![("elts", self.heap.alloc_list(converted)), ("ctx", ctx)],
                )
            }
            ExprKind::Dict(entries) => {
                let mut keys = Vec::new();
                let mut values = Vec::new();
                for entry in entries {
                    match entry {
                        DictEntry::Pair(key, value) => {
                            keys.push(self.convert_expr_to_ast_node(key)?);
                            values.push(self.convert_expr_to_ast_node(value)?);
                        }
                        DictEntry::Unpack(value) => {
                            keys.push(Value::None);
                            values.push(self.convert_expr_to_ast_node(value)?);
                        }
                    }
                }
                self.build_ast_node(
                    "Dict",
                    location,
                    vec![
                        ("keys", self.heap.alloc_list(keys)),
                        ("values", self.heap.alloc_list(values)),
                    ],
                )
            }
            ExprKind::Subscript { value, index } => {
                let value_node = self.convert_expr_to_ast_node(value)?;
                let index_node = self.convert_expr_to_ast_node(index)?;
                let ctx = self.build_ast_context_node("Load")?;
                self.build_ast_node(
                    "Subscript",
                    location,
                    vec![("value", value_node), ("slice", index_node), ("ctx", ctx)],
                )
            }
            ExprKind::TemplateLiteral {
                strings,
                interpolations,
            } => {
                let mut values = Vec::new();
                for (idx, text) in strings.iter().enumerate() {
                    if !text.is_empty() {
                        values.push(self.build_ast_node(
                            "Constant",
                            location,
                            vec![("value", Value::Str(text.clone()))],
                        )?);
                    }
                    if let Some(interpolation) = interpolations.get(idx) {
                        let value_node = self.convert_expr_to_ast_node(&interpolation.value)?;
                        let conversion = interpolation.conversion.map(|ch| ch as i64).unwrap_or(-1);
                        let format_spec = if interpolation
                            .format_spec
                            .as_ref()
                            .is_some_and(|spec| !spec.is_empty())
                        {
                            self.build_ast_node(
                                "Constant",
                                location,
                                vec![(
                                    "value",
                                    Value::Str(
                                        interpolation
                                            .format_spec
                                            .as_ref()
                                            .cloned()
                                            .unwrap_or_default(),
                                    ),
                                )],
                            )?
                        } else {
                            Value::None
                        };
                        values.push(self.build_ast_node(
                            "Interpolation",
                            location,
                            vec![
                                ("value", value_node),
                                ("str", Value::Str(interpolation.expression.clone())),
                                ("conversion", Value::Int(conversion)),
                                ("format_spec", format_spec),
                            ],
                        )?);
                    }
                }
                self.build_ast_node(
                    "TemplateStr",
                    location,
                    vec![("values", self.heap.alloc_list(values))],
                )
            }
            ExprKind::Slice { lower, upper, step } => {
                let lower_node = match lower {
                    Some(value) => self.convert_expr_to_ast_node(value)?,
                    None => Value::None,
                };
                let upper_node = match upper {
                    Some(value) => self.convert_expr_to_ast_node(value)?,
                    None => Value::None,
                };
                let step_node = match step {
                    Some(value) => self.convert_expr_to_ast_node(value)?,
                    None => Value::None,
                };
                self.build_ast_node(
                    "Slice",
                    location,
                    vec![
                        ("lower", lower_node),
                        ("upper", upper_node),
                        ("step", step_node),
                    ],
                )
            }
        }
    }

    fn convert_stmt_to_ast_node(&mut self, stmt: &Stmt) -> Result<Value, RuntimeError> {
        let location = Some((stmt.span.line, stmt.span.column));
        match &stmt.node {
            StmtKind::FunctionDef {
                name,
                type_params,
                is_async,
                posonly_params,
                params,
                vararg,
                kwarg,
                kwonly_params,
                returns,
                body,
            } => {
                let args_node = self.convert_function_arguments_to_ast_node(
                    posonly_params,
                    params,
                    vararg,
                    kwonly_params,
                    kwarg,
                    location,
                )?;
                let body_nodes = self.convert_stmt_list_to_ast_values(body)?;
                let returns_node = match returns {
                    Some(expr) => self.convert_expr_to_ast_node(expr)?,
                    None => Value::None,
                };
                let type_params_nodes =
                    self.convert_type_params_to_ast_nodes(type_params, location)?;
                let class_name = if *is_async {
                    "AsyncFunctionDef"
                } else {
                    "FunctionDef"
                };
                self.build_ast_node(
                    class_name,
                    location,
                    vec![
                        ("name", Value::Str(name.clone())),
                        ("args", args_node),
                        ("body", self.heap.alloc_list(body_nodes)),
                        ("decorator_list", self.heap.alloc_list(Vec::new())),
                        ("returns", returns_node),
                        ("type_comment", Value::None),
                        ("type_params", self.heap.alloc_list(type_params_nodes)),
                    ],
                )
            }
            StmtKind::ClassDef {
                name,
                type_params,
                bases,
                metaclass,
                keywords,
                body,
            } => {
                let mut base_nodes = Vec::with_capacity(bases.len());
                for base in bases {
                    match base {
                        CallArg::Positional(expr) => {
                            base_nodes.push(self.convert_expr_to_ast_node(expr)?);
                        }
                        CallArg::Star(expr) => {
                            let value = self.convert_expr_to_ast_node(expr)?;
                            let ctx = self.build_ast_context_node("Load")?;
                            base_nodes.push(self.build_ast_node(
                                "Starred",
                                None,
                                vec![("value", value), ("ctx", ctx)],
                            )?);
                        }
                        CallArg::Keyword { .. } | CallArg::DoubleStar(_) => {
                            return Err(RuntimeError::new("invalid class base argument"));
                        }
                    }
                }
                let mut keyword_nodes =
                    Vec::with_capacity(keywords.len() + usize::from(metaclass.is_some()));
                if let Some(meta) = metaclass {
                    let meta_node = self.convert_expr_to_ast_node(meta)?;
                    keyword_nodes.push(self.build_ast_node(
                        "keyword",
                        location,
                        vec![
                            ("arg", Value::Str("metaclass".to_string())),
                            ("value", meta_node),
                        ],
                    )?);
                }
                for (name, value) in keywords {
                    let keyword_value = self.convert_expr_to_ast_node(value)?;
                    keyword_nodes.push(self.build_ast_node(
                        "keyword",
                        location,
                        vec![("arg", Value::Str(name.clone())), ("value", keyword_value)],
                    )?);
                }
                let body_nodes = self.convert_stmt_list_to_ast_values(body)?;
                let type_params_nodes =
                    self.convert_type_params_to_ast_nodes(type_params, location)?;
                self.build_ast_node(
                    "ClassDef",
                    location,
                    vec![
                        ("name", Value::Str(name.clone())),
                        ("bases", self.heap.alloc_list(base_nodes)),
                        ("keywords", self.heap.alloc_list(keyword_nodes)),
                        ("body", self.heap.alloc_list(body_nodes)),
                        ("decorator_list", self.heap.alloc_list(Vec::new())),
                        ("type_params", self.heap.alloc_list(type_params_nodes)),
                    ],
                )
            }
            StmtKind::TypeAlias {
                name,
                type_params,
                value,
            } => {
                let type_params_nodes =
                    self.convert_type_params_to_ast_nodes(type_params, location)?;
                let name_node =
                    self.convert_assign_target_to_ast_expr(&AssignTarget::Name(name.clone()))?;
                let value_node = self.convert_expr_to_ast_node(value)?;
                self.build_ast_node(
                    "TypeAlias",
                    location,
                    vec![
                        ("name", name_node),
                        ("type_params", self.heap.alloc_list(type_params_nodes)),
                        ("value", value_node),
                    ],
                )
            }
            StmtKind::Decorated { decorators, stmt } => {
                let base_node = self.convert_stmt_to_ast_node(stmt)?;
                self.apply_decorators_to_ast_node(base_node, decorators)
            }
            StmtKind::AugAssign { target, op, value } => {
                let target_node = self.convert_assign_target_to_ast_expr(target)?;
                let op_node =
                    self.build_ast_node(Self::ast_aug_operator_class_name(op), None, Vec::new())?;
                let value_node = self.convert_expr_to_ast_node(value)?;
                self.build_ast_node(
                    "AugAssign",
                    location,
                    vec![
                        ("target", target_node),
                        ("op", op_node),
                        ("value", value_node),
                    ],
                )
            }
            StmtKind::AnnAssign {
                target,
                annotation,
                value,
                simple,
            } => {
                let target_node = self.convert_assign_target_to_ast_expr(target)?;
                let annotation_node = self.convert_expr_to_ast_node(annotation)?;
                let value_node = match value {
                    Some(expr) => self.convert_expr_to_ast_node(expr)?,
                    None => Value::None,
                };
                self.build_ast_node(
                    "AnnAssign",
                    location,
                    vec![
                        ("target", target_node),
                        ("annotation", annotation_node),
                        ("value", value_node),
                        ("simple", Value::Int(if *simple { 1 } else { 0 })),
                    ],
                )
            }
            StmtKind::Assign { targets, value } => {
                let mut converted_targets = Vec::with_capacity(targets.len());
                for target in targets {
                    converted_targets.push(self.convert_assign_target_to_ast_expr(target)?);
                }
                let value_node = self.convert_expr_to_ast_node(value)?;
                self.build_ast_node(
                    "Assign",
                    location,
                    vec![
                        ("targets", self.heap.alloc_list(converted_targets)),
                        ("value", value_node),
                        ("type_comment", Value::None),
                    ],
                )
            }
            StmtKind::Delete { targets } => {
                let mut converted_targets = Vec::with_capacity(targets.len());
                for target in targets {
                    converted_targets.push(self.convert_assign_target_to_ast_expr(target)?);
                }
                self.build_ast_node(
                    "Delete",
                    location,
                    vec![("targets", self.heap.alloc_list(converted_targets))],
                )
            }
            StmtKind::Return { value } => {
                let value_node = match value {
                    Some(value) => self.convert_expr_to_ast_node(value)?,
                    None => Value::None,
                };
                self.build_ast_node("Return", location, vec![("value", value_node)])
            }
            StmtKind::Raise { value, cause } => {
                let exc = match value {
                    Some(value) => self.convert_expr_to_ast_node(value)?,
                    None => Value::None,
                };
                let cause = match cause {
                    Some(cause) => self.convert_expr_to_ast_node(cause)?,
                    None => Value::None,
                };
                self.build_ast_node("Raise", location, vec![("exc", exc), ("cause", cause)])
            }
            StmtKind::Assert { test, message } => {
                let test_node = self.convert_expr_to_ast_node(test)?;
                let message_node = match message {
                    Some(value) => self.convert_expr_to_ast_node(value)?,
                    None => Value::None,
                };
                self.build_ast_node(
                    "Assert",
                    location,
                    vec![("test", test_node), ("msg", message_node)],
                )
            }
            StmtKind::If { test, body, orelse } => {
                let test_node = self.convert_expr_to_ast_node(test)?;
                let body_nodes = self.convert_stmt_list_to_ast_values(body)?;
                let orelse_nodes = self.convert_stmt_list_to_ast_values(orelse)?;
                self.build_ast_node(
                    "If",
                    location,
                    vec![
                        ("test", test_node),
                        ("body", self.heap.alloc_list(body_nodes)),
                        ("orelse", self.heap.alloc_list(orelse_nodes)),
                    ],
                )
            }
            StmtKind::While { test, body, orelse } => {
                let test_node = self.convert_expr_to_ast_node(test)?;
                let body_nodes = self.convert_stmt_list_to_ast_values(body)?;
                let orelse_nodes = self.convert_stmt_list_to_ast_values(orelse)?;
                self.build_ast_node(
                    "While",
                    location,
                    vec![
                        ("test", test_node),
                        ("body", self.heap.alloc_list(body_nodes)),
                        ("orelse", self.heap.alloc_list(orelse_nodes)),
                    ],
                )
            }
            StmtKind::For {
                is_async,
                target,
                iter,
                body,
                orelse,
            } => {
                let class_name = if *is_async { "AsyncFor" } else { "For" };
                let target_node = self.convert_assign_target_to_ast_expr(target)?;
                let iter_node = self.convert_expr_to_ast_node(iter)?;
                let body_nodes = self.convert_stmt_list_to_ast_values(body)?;
                let orelse_nodes = self.convert_stmt_list_to_ast_values(orelse)?;
                self.build_ast_node(
                    class_name,
                    location,
                    vec![
                        ("target", target_node),
                        ("iter", iter_node),
                        ("body", self.heap.alloc_list(body_nodes)),
                        ("orelse", self.heap.alloc_list(orelse_nodes)),
                        ("type_comment", Value::None),
                    ],
                )
            }
            StmtKind::Match { subject, cases } => {
                let subject_node = self.convert_expr_to_ast_node(subject)?;
                let mut case_nodes = Vec::with_capacity(cases.len());
                for case in cases {
                    case_nodes.push(self.convert_match_case_to_ast_node(case, location)?);
                }
                self.build_ast_node(
                    "Match",
                    location,
                    vec![
                        ("subject", subject_node),
                        ("cases", self.heap.alloc_list(case_nodes)),
                    ],
                )
            }
            StmtKind::With {
                is_async,
                context,
                target,
                body,
            } => {
                let class_name = if *is_async { "AsyncWith" } else { "With" };
                let context_expr = self.convert_expr_to_ast_node(context)?;
                let optional_vars = match target {
                    Some(value) => self.convert_assign_target_to_ast_expr(value)?,
                    None => Value::None,
                };
                let with_item = self.build_ast_node(
                    "withitem",
                    None,
                    vec![
                        ("context_expr", context_expr),
                        ("optional_vars", optional_vars),
                    ],
                )?;
                let body_nodes = self.convert_stmt_list_to_ast_values(body)?;
                self.build_ast_node(
                    class_name,
                    location,
                    vec![
                        ("items", self.heap.alloc_list(vec![with_item])),
                        ("body", self.heap.alloc_list(body_nodes)),
                        ("type_comment", Value::None),
                    ],
                )
            }
            StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                let class_name = if handlers.iter().any(|handler| handler.is_star) {
                    "TryStar"
                } else {
                    "Try"
                };
                let body_nodes = self.convert_stmt_list_to_ast_values(body)?;
                let mut handler_nodes = Vec::with_capacity(handlers.len());
                for handler in handlers {
                    handler_nodes.push(self.convert_except_handler_to_ast_node(handler, location)?);
                }
                let orelse_nodes = self.convert_stmt_list_to_ast_values(orelse)?;
                let final_nodes = self.convert_stmt_list_to_ast_values(finalbody)?;
                self.build_ast_node(
                    class_name,
                    location,
                    vec![
                        ("body", self.heap.alloc_list(body_nodes)),
                        ("handlers", self.heap.alloc_list(handler_nodes)),
                        ("orelse", self.heap.alloc_list(orelse_nodes)),
                        ("finalbody", self.heap.alloc_list(final_nodes)),
                    ],
                )
            }
            StmtKind::Import { names } => {
                let mut converted = Vec::with_capacity(names.len());
                for name in names {
                    converted.push(self.convert_import_alias_to_ast_node(name, location)?);
                }
                self.build_ast_node(
                    "Import",
                    location,
                    vec![("names", self.heap.alloc_list(converted))],
                )
            }
            StmtKind::ImportFrom {
                module,
                names,
                level,
            } => {
                let mut converted = Vec::with_capacity(names.len());
                for name in names {
                    converted.push(self.convert_import_alias_to_ast_node(name, location)?);
                }
                self.build_ast_node(
                    "ImportFrom",
                    location,
                    vec![
                        (
                            "module",
                            module
                                .as_ref()
                                .map(|value| Value::Str(value.clone()))
                                .unwrap_or(Value::None),
                        ),
                        ("names", self.heap.alloc_list(converted)),
                        ("level", Value::Int(*level as i64)),
                    ],
                )
            }
            StmtKind::Global { names } => self.build_ast_node(
                "Global",
                location,
                vec![(
                    "names",
                    self.heap
                        .alloc_list(names.iter().cloned().map(Value::Str).collect()),
                )],
            ),
            StmtKind::Nonlocal { names } => self.build_ast_node(
                "Nonlocal",
                location,
                vec![(
                    "names",
                    self.heap
                        .alloc_list(names.iter().cloned().map(Value::Str).collect()),
                )],
            ),
            StmtKind::Expr(value) => {
                let value_node = self.convert_expr_to_ast_node(value)?;
                self.build_ast_node("Expr", location, vec![("value", value_node)])
            }
            StmtKind::Pass => self.build_ast_node("Pass", location, Vec::new()),
            StmtKind::Break => self.build_ast_node("Break", location, Vec::new()),
            StmtKind::Continue => self.build_ast_node("Continue", location, Vec::new()),
        }
    }

    fn convert_module_to_ast_node(
        &mut self,
        module_ast: &AstModule,
    ) -> Result<Value, RuntimeError> {
        let mut body = Vec::with_capacity(module_ast.body.len());
        for stmt in &module_ast.body {
            body.push(self.convert_stmt_to_ast_node(stmt)?);
        }
        self.build_ast_node(
            "Module",
            None,
            vec![
                ("body", self.heap.alloc_list(body)),
                ("type_ignores", self.heap.alloc_list(Vec::new())),
            ],
        )
    }

    fn convert_expression_to_ast_node(&mut self, expr_ast: &Expr) -> Result<Value, RuntimeError> {
        let body = self.convert_expr_to_ast_node(expr_ast)?;
        self.build_ast_node("Expression", None, vec![("body", body)])
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

        const PYCF_ONLY_AST: i64 = 1024;
        let flags = value_to_int(flags_value)?;
        let _ = self.truthy_from_value(&dont_inherit_value)?;
        let _ = value_to_int(optimize_value)?;
        if let Some(value) = feature_kw {
            let _ = value_to_int(value)?;
        }
        let request_ast = (flags & PYCF_ONLY_AST) != 0;

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
        let compiled_mode = match mode.as_str() {
            "eval" => CompiledCodeMode::Eval,
            "single" => CompiledCodeMode::Single,
            _ => CompiledCodeMode::Exec,
        };
        let cacheable_filename = !filename.is_empty()
            && !(filename.starts_with('<')
                && filename.ends_with('>')
                && !filename.starts_with("<frozen "));
        if cacheable_filename {
            self.cache_source_text(&filename, &source_text);
        }
        if mode == "eval" {
            let expr_ast = parser::parse_expression(&source_text).map_err(|err| {
                self.compile_syntax_error_runtime_error(
                    err.message,
                    &filename,
                    &source_text,
                    Some(err.line),
                    Some(err.column),
                )
            })?;
            if request_ast {
                return self.convert_expression_to_ast_node(&expr_ast);
            }
            let code = compiler::compile_expression_with_filename(&expr_ast, &filename).map_err(
                |err| {
                    self.compile_syntax_error_runtime_error(
                        err.message,
                        &filename,
                        &source_text,
                        err.span.map(|span| span.line),
                        err.span.map(|span| span.column),
                    )
                },
            )?;
            let code = Rc::new(code);
            self.register_compiled_code_metadata(&code, compiled_mode, Some(&source_text));
            return Ok(Value::Code(code));
        }

        let module_ast = parser::parse_module(&source_text).map_err(|err| {
            self.compile_syntax_error_runtime_error(
                err.message,
                &filename,
                &source_text,
                Some(err.line),
                Some(err.column),
            )
        })?;
        if request_ast {
            return self.convert_module_to_ast_node(&module_ast);
        }
        let code =
            compiler::compile_module_with_filename(&module_ast, &filename).map_err(|err| {
                self.compile_syntax_error_runtime_error(
                    err.message,
                    &filename,
                    &source_text,
                    err.span.map(|span| span.line),
                    err.span.map(|span| span.column),
                )
            })?;
        let code = Rc::new(code);
        self.register_compiled_code_metadata(&code, compiled_mode, Some(&source_text));
        Ok(Value::Code(code))
    }

    fn compile_syntax_error_runtime_error(
        &mut self,
        message: String,
        filename: &str,
        source_text: &str,
        line: Option<usize>,
        column: Option<usize>,
    ) -> RuntimeError {
        let line_end_column = |line_no: usize| -> Option<usize> {
            source_text
                .lines()
                .nth(line_no.saturating_sub(1))
                .map(|text| text.chars().count().saturating_add(1))
        };
        let detect_unexpected_top_level_indent = || -> Option<usize> {
            for (idx, line_text) in source_text.lines().enumerate() {
                if line_text.trim().is_empty() {
                    continue;
                }
                if line_text
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_whitespace())
                {
                    return Some(idx + 1);
                }
                break;
            }
            None
        };
        let detect_last_unclosed_delimiter = || -> Option<(usize, usize)> {
            let mut stack: Vec<(char, usize, usize)> = Vec::new();
            let mut line_no = 1usize;
            let mut col_no = 0usize;
            let mut in_single_quote = false;
            let mut in_double_quote = false;
            let mut escaped = false;
            let mut in_comment = false;
            for ch in source_text.chars() {
                if ch == '\n' {
                    line_no += 1;
                    col_no = 0;
                    in_comment = false;
                    escaped = false;
                    continue;
                }
                col_no += 1;
                if in_comment {
                    continue;
                }
                if escaped {
                    escaped = false;
                    continue;
                }
                if in_single_quote {
                    match ch {
                        '\\' => escaped = true,
                        '\'' => in_single_quote = false,
                        _ => {}
                    }
                    continue;
                }
                if in_double_quote {
                    match ch {
                        '\\' => escaped = true,
                        '"' => in_double_quote = false,
                        _ => {}
                    }
                    continue;
                }
                match ch {
                    '#' => in_comment = true,
                    '\'' => in_single_quote = true,
                    '"' => in_double_quote = true,
                    '(' | '[' | '{' => stack.push((ch, line_no, col_no)),
                    ')' => {
                        if matches!(stack.last(), Some(('(', _, _))) {
                            stack.pop();
                        }
                    }
                    ']' => {
                        if matches!(stack.last(), Some(('[', _, _))) {
                            stack.pop();
                        }
                    }
                    '}' => {
                        if matches!(stack.last(), Some(('{', _, _))) {
                            stack.pop();
                        }
                    }
                    _ => {}
                }
            }
            stack.last().map(|(_, line, col)| (*line, *col))
        };
        let lower = message.to_ascii_lowercase();
        let mut error_type = "SyntaxError";
        let mut normalized_message =
            if message == "expected expression" || message.starts_with("unexpected character:") {
                "invalid syntax".to_string()
            } else {
                message
            };
        let mut detail_line = line;
        let mut detail_column = column;
        if let Some(indent_line) = detect_unexpected_top_level_indent() {
            if detail_line.is_none() || detail_line == Some(indent_line) {
                error_type = "IndentationError";
                normalized_message = "unexpected indent".to_string();
                // Match CPython: show offending line without a caret.
                detail_column = Some(0);
            }
        } else if lower.starts_with("expected indent") {
            error_type = "IndentationError";
            normalized_message = "expected an indented block".to_string();
        } else if lower.contains("indentation does not match any outer level")
            || lower.contains("unindent does not match any outer indentation level")
            || lower.starts_with("expected dedent")
        {
            error_type = "IndentationError";
            normalized_message = "unindent does not match any outer indentation level".to_string();
            if let Some(line_no) = detail_line {
                detail_column = line_end_column(line_no).or(detail_column);
            }
        } else if lower.starts_with("unexpected indent") {
            error_type = "IndentationError";
            normalized_message = "unexpected indent".to_string();
            if detail_line == Some(1) {
                // Match CPython top-level indentation errors: show source line without caret.
                detail_column = Some(0);
            }
        }
        if lower.starts_with("expected expression")
            && let Some((open_line, open_column)) = detect_last_unclosed_delimiter()
            && detail_line.is_none_or(|existing| existing >= open_line)
        {
            detail_line = Some(open_line);
            detail_column = Some(open_column);
        }
        let infer_genexp_range_end = |line_text: &str, start_column: usize| -> Option<usize> {
            if start_column == 0 {
                return Some(0);
            }
            let chars = line_text.chars().collect::<Vec<_>>();
            let start = start_column.saturating_sub(1);
            if start >= chars.len() {
                return None;
            }
            let mut depth = 0usize;
            for (idx, ch) in chars.iter().enumerate().skip(start) {
                match *ch {
                    '(' | '[' | '{' => depth = depth.saturating_add(1),
                    ')' | ']' | '}' => {
                        if depth == 0 {
                            return Some(idx + 1);
                        }
                        depth = depth.saturating_sub(1);
                    }
                    ',' if depth == 0 => return Some(idx + 1),
                    _ => {}
                }
            }
            Some(chars.len().saturating_add(1))
        };
        let exception = ExceptionObject::new(error_type, Some(normalized_message.clone()));
        let mut call_args = vec![Value::Str(normalized_message)];
        if let (Some(line), Some(column)) = (detail_line, detail_column) {
            let source_line = source_text
                .lines()
                .nth(line.saturating_sub(1))
                .unwrap_or("")
                .to_string();
            let mut end_column = if column == 0 {
                0
            } else {
                column.saturating_add(1)
            };
            if call_args
                .first()
                .is_some_and(|value| matches!(value, Value::Str(text) if text == "Generator expression must be parenthesized"))
                && let Some(inferred_end) = infer_genexp_range_end(&source_line, column)
            {
                end_column = inferred_end.max(column.saturating_add(1));
            }
            let line_value = i64::try_from(line).unwrap_or(i64::MAX);
            let column_value = i64::try_from(column).unwrap_or(i64::MAX);
            let end_column_value = i64::try_from(end_column).unwrap_or(i64::MAX);
            let detail = self.heap.alloc_tuple(vec![
                Value::Str(filename.to_string()),
                Value::Int(line_value),
                Value::Int(column_value),
                Value::Str(source_line),
                Value::Int(line_value),
                Value::Int(end_column_value),
            ]);
            call_args.push(detail);
        }
        {
            let mut attrs = exception.attrs.borrow_mut();
            attrs.insert("args".to_string(), self.heap.alloc_tuple(call_args.clone()));
            self.populate_syntax_error_attrs(&mut attrs, &call_args);
        }
        RuntimeError::from_exception(exception)
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
        if !callable && self.host.env_var_os("PYRS_TRACE_CALLABLE_FALSE").is_some() {
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

    pub(super) fn types_module_or_private_class(&self, name: &str) -> Option<ObjRef> {
        self.types_module_class(name)
            .or_else(|| {
                let module = self.modules.get("_types")?;
                let Object::Module(module_data) = &*module.kind() else {
                    return None;
                };
                match module_data.globals.get(name) {
                    Some(Value::Class(class)) => Some(class.clone()),
                    _ => None,
                }
            })
            .or_else(|| {
                let builtins = self.modules.get("builtins")?;
                let Object::Module(module_data) = &*builtins.kind() else {
                    return None;
                };
                let cache_key = format!("__pyrs_types_class_{name}");
                match module_data.globals.get(&cache_key) {
                    Some(Value::Class(class)) => Some(class.clone()),
                    _ => None,
                }
            })
    }

    pub(super) fn fallback_none_type_class(&mut self) -> ObjRef {
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

    fn mark_re_runtime_type_class(&mut self, class: &ObjRef, class_name: &str) {
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("re".to_string()));
            class_data
                .attrs
                .insert("__name__".to_string(), Value::Str(class_name.to_string()));
            class_data.attrs.insert(
                "__qualname__".to_string(),
                Value::Str(class_name.to_string()),
            );
            class_data.attrs.insert(
                "__pyrs_disallow_subclassing__".to_string(),
                Value::Bool(true),
            );
        }
    }

    pub(super) fn ensure_re_runtime_type_class(&mut self, class_name: &str) -> ObjRef {
        if let Some(re_module) = self.modules.get("re").cloned()
            && let Object::Module(module_data) = &*re_module.kind()
            && let Some(Value::Class(class)) = module_data.globals.get(class_name)
        {
            let class = class.clone();
            self.mark_re_runtime_type_class(&class, class_name);
            return class;
        }
        let class = self.alloc_synthetic_class(class_name);
        self.mark_re_runtime_type_class(&class, class_name);
        if let Some(re_module) = self.modules.get("re").cloned()
            && let Object::Module(module_data) = &mut *re_module.kind_mut()
        {
            module_data
                .globals
                .insert(class_name.to_string(), Value::Class(class.clone()));
        }
        class
    }

    pub(super) fn fallback_function_type_class(&mut self) -> ObjRef {
        if let Some(module) = self.modules.get("builtins").cloned()
            && let Object::Module(module_data) = &*module.kind()
            && let Some(Value::Class(class)) =
                module_data.globals.get("__pyrs_function_type_class__")
        {
            return class.clone();
        }
        let class = self.alloc_synthetic_class("function");
        let getset_descriptor_class = self.alloc_synthetic_class("getset_descriptor");
        let member_descriptor_class = self.alloc_synthetic_class("member_descriptor");
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data.attrs.insert(
                "__new__".to_string(),
                Value::Builtin(BuiltinFunction::TypesFunctionType),
            );
            class_data.attrs.insert(
                "__code__".to_string(),
                self.heap
                    .alloc_instance(InstanceObject::new(getset_descriptor_class)),
            );
            class_data.attrs.insert(
                "__globals__".to_string(),
                self.heap
                    .alloc_instance(InstanceObject::new(member_descriptor_class)),
            );
        }
        if let Some(module) = self.modules.get("builtins").cloned()
            && let Object::Module(module_data) = &mut *module.kind_mut()
        {
            module_data.globals.insert(
                "__pyrs_function_type_class__".to_string(),
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

    fn bound_method_is_builtin_unbound_descriptor(&self, method: &ObjRef) -> bool {
        let Object::BoundMethod(method_data) = &*method.kind() else {
            return false;
        };
        let Object::Module(module_data) = &*method_data.receiver.kind() else {
            return false;
        };
        module_data.name.ends_with("_unbound_method__")
    }

    fn bound_method_is_builtin_unbound_slot_wrapper(&self, method: &ObjRef) -> bool {
        let Object::BoundMethod(method_data) = &*method.kind() else {
            return false;
        };
        if !matches!(&*method_data.receiver.kind(), Object::Class(_)) {
            return false;
        }
        let Object::NativeMethod(native) = &*method_data.function.kind() else {
            return false;
        };
        matches!(
            native.kind,
            NativeMethodKind::Builtin(BuiltinFunction::Repr | BuiltinFunction::Str)
        )
    }

    fn bound_method_is_builtin_slot_wrapper(&self, method: &ObjRef) -> bool {
        let Object::BoundMethod(method_data) = &*method.kind() else {
            return false;
        };
        let receiver_is_slot_wrapper = matches!(&*method_data.receiver.kind(), Object::Instance(_))
            || matches!(
                &*method_data.receiver.kind(),
                Object::Module(module_data) if module_data.name == "__int_method__"
            );
        if !receiver_is_slot_wrapper {
            return false;
        }
        let Object::NativeMethod(native) = &*method_data.function.kind() else {
            return false;
        };
        matches!(
            native.kind,
            NativeMethodKind::Builtin(
                BuiltinFunction::Repr
                    | BuiltinFunction::Str
                    | BuiltinFunction::ObjectInit
                    | BuiltinFunction::OperatorLt
            )
        )
    }

    fn builtin_is_classmethod_descriptor(&self, builtin: BuiltinFunction) -> bool {
        matches!(
            builtin,
            BuiltinFunction::DictFromKeys | BuiltinFunction::IntFromBytes
        )
    }

    pub(super) fn builtin_is_type_object(&self, builtin: BuiltinFunction) -> bool {
        matches!(
            builtin,
            BuiltinFunction::Type
                | BuiltinFunction::TypesMethodType
                | BuiltinFunction::TypesModuleType
                | BuiltinFunction::Bool
                | BuiltinFunction::Int
                | BuiltinFunction::Float
                | BuiltinFunction::Str
                | BuiltinFunction::List
                | BuiltinFunction::Tuple
                | BuiltinFunction::Dict
                | BuiltinFunction::CollectionsDefaultDict
                | BuiltinFunction::CollectionsOrderedDict
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
                | BuiltinFunction::GeneratorType
                | BuiltinFunction::CoroutineType
                | BuiltinFunction::AsyncGeneratorType
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
                Value::Function(_) => Some(Value::Class(
                    self.types_module_or_private_class("FunctionType")
                        .unwrap_or_else(|| self.fallback_function_type_class()),
                )),
                Value::BoundMethod(method) => {
                    if self.bound_method_is_python_method(method) {
                        Some(
                            self.types_module_or_private_class("MethodType")
                                .map(Value::Class)
                                .unwrap_or(Value::Builtin(BuiltinFunction::TypesMethodType)),
                        )
                    } else if self.bound_method_is_builtin_unbound_slot_wrapper(method) {
                        self.types_module_or_private_class("WrapperDescriptorType")
                            .or_else(|| self.types_module_or_private_class("BuiltinFunctionType"))
                            .map(Value::Class)
                    } else if self.bound_method_is_builtin_unbound_descriptor(method) {
                        self.types_module_or_private_class("MethodDescriptorType")
                            .or_else(|| self.types_module_or_private_class("BuiltinMethodType"))
                            .or_else(|| self.types_module_or_private_class("BuiltinFunctionType"))
                            .map(Value::Class)
                    } else if self.bound_method_is_builtin_slot_wrapper(method) {
                        self.types_module_or_private_class("MethodWrapperType")
                            .or_else(|| self.types_module_or_private_class("BuiltinMethodType"))
                            .or_else(|| self.types_module_or_private_class("BuiltinFunctionType"))
                            .map(Value::Class)
                    } else {
                        self.types_module_or_private_class("BuiltinMethodType")
                            .or_else(|| self.types_module_or_private_class("BuiltinFunctionType"))
                            .map(Value::Class)
                    }
                }
                Value::Builtin(builtin) => {
                    if self.builtin_is_type_object(*builtin) {
                        Some(Value::Builtin(BuiltinFunction::Type))
                    } else if matches!(builtin, BuiltinFunction::ListAppendDescriptor) {
                        self.types_module_or_private_class("MethodDescriptorType")
                            .or_else(|| self.types_module_or_private_class("BuiltinFunctionType"))
                            .map(Value::Class)
                    } else if self.builtin_is_classmethod_descriptor(*builtin) {
                        self.types_module_or_private_class("ClassMethodDescriptorType")
                            .or_else(|| self.types_module_or_private_class("BuiltinFunctionType"))
                            .map(Value::Class)
                    } else if matches!(
                        builtin,
                        BuiltinFunction::ObjectInit
                            | BuiltinFunction::ObjectNew
                            | BuiltinFunction::OperatorLt
                    ) {
                        self.types_module_or_private_class("WrapperDescriptorType")
                            .or_else(|| self.types_module_or_private_class("BuiltinFunctionType"))
                            .map(Value::Class)
                    } else {
                        self.types_module_or_private_class("BuiltinFunctionType")
                            .map(Value::Class)
                    }
                }
                Value::Dict(dict) if self.defaultdict_factories.contains_key(&dict.id()) => {
                    Some(Value::Builtin(BuiltinFunction::CollectionsDefaultDict))
                }
                Value::Dict(dict) if self.ordered_dict_instances.contains(&dict.id()) => {
                    Some(Value::Builtin(BuiltinFunction::CollectionsOrderedDict))
                }
                Value::Code(_) => self
                    .types_module_or_private_class("CodeType")
                    .map(Value::Class),
                Value::Generator(generator) => {
                    let builtin = match &*generator.kind() {
                        Object::Generator(state) if state.is_async_generator => {
                            BuiltinFunction::AsyncGeneratorType
                        }
                        Object::Generator(state) if state.is_coroutine => {
                            BuiltinFunction::CoroutineType
                        }
                        _ => BuiltinFunction::GeneratorType,
                    };
                    Some(Value::Builtin(builtin))
                }
                Value::Instance(instance)
                    if self
                        .builtins
                        .get("NotImplemented")
                        .and_then(|value| match value {
                            Value::Instance(singleton) => Some(singleton.id() == instance.id()),
                            _ => None,
                        })
                        .unwrap_or(false) =>
                {
                    self.types_module_or_private_class("NotImplementedType")
                        .map(Value::Class)
                }
                Value::Cell(_) => self
                    .types_module_or_private_class("CellType")
                    .map(Value::Class),
                Value::Module(module) => match &*module.kind() {
                    Object::Module(module_data) if module_data.name == "__staticmethod__" => {
                        Some(Value::Builtin(BuiltinFunction::StaticMethod))
                    }
                    Object::Module(module_data) if module_data.name == "__classmethod__" => {
                        Some(Value::Builtin(BuiltinFunction::ClassMethod))
                    }
                    Object::Module(module_data) if module_data.name == "__re_pattern__" => {
                        Some(Value::Class(self.ensure_re_runtime_type_class("Pattern")))
                    }
                    Object::Module(module_data) if module_data.name == "__re_match__" => {
                        Some(Value::Class(self.ensure_re_runtime_type_class("Match")))
                    }
                    _ => self
                        .types_module_or_private_class("ModuleType")
                        .map(Value::Class)
                        .or(Some(Value::Builtin(BuiltinFunction::TypesModuleType))),
                },
                Value::None => Some(Value::Class(
                    self.types_module_or_private_class("NoneType")
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
                    return Err(RuntimeError::type_error(
                        "type.__new__() argument 1 must be a type",
                    ));
                }
            };
            let class_name = match &args[name_index] {
                Value::Str(name) => name.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "type() first argument must be string",
                    ));
                }
            };
            let base_values = self.builtin_type_base_values(&args[name_index + 1])?;
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
                None,
            )?;
            self.preserve_type_bases_tuple_subclass(&class_value, &args[name_index + 1]);
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
                _ => {
                    return Err(RuntimeError::type_error(
                        "type() first argument must be string",
                    ));
                }
            };
            let base_values = self.builtin_type_base_values(&args[1])?;
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
                None,
                explicit_metaclass,
                class_keywords,
                Some(namespace_value),
                None,
                None,
                false,
            )? {
                ClassBuildOutcome::Value(value) => {
                    self.preserve_type_bases_tuple_subclass(&value, &args[1]);
                    Ok(value)
                }
                ClassBuildOutcome::ExceptionHandled => {
                    Err(self.runtime_error_from_active_exception("metaclass call failed"))
                }
            };
        }
        BuiltinFunction::Type.call(&self.heap, args)
    }

    fn builtin_type_base_values(&self, bases: &Value) -> Result<Vec<Value>, RuntimeError> {
        match bases {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(values) => Ok(values.clone()),
                _ => Err(RuntimeError::type_error("type() bases must be tuple/list")),
            },
            Value::List(list_obj) => match &*list_obj.kind() {
                Object::List(values) => Ok(values.clone()),
                _ => Err(RuntimeError::type_error("type() bases must be tuple/list")),
            },
            Value::Instance(instance) => {
                if let Some(backing_tuple) = self.instance_backing_tuple(instance) {
                    if let Object::Tuple(values) = &*backing_tuple.kind() {
                        return Ok(values.clone());
                    }
                }
                if let Some(backing_list) = self.instance_backing_list(instance) {
                    if let Object::List(values) = &*backing_list.kind() {
                        return Ok(values.clone());
                    }
                }
                Err(RuntimeError::type_error("type() bases must be tuple/list"))
            }
            _ => Err(RuntimeError::type_error("type() bases must be tuple/list")),
        }
    }

    fn preserve_type_bases_tuple_subclass(&mut self, class_value: &Value, bases_value: &Value) {
        let Value::Class(class_ref) = class_value else {
            return;
        };
        let Value::Instance(instance) = bases_value else {
            return;
        };
        if self.instance_backing_tuple(instance).is_none() {
            return;
        }
        let tuple_subclass = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return,
        };
        let is_exact_tuple = matches!(
            &*tuple_subclass.kind(),
            Object::Class(class_data) if class_data.name == "tuple"
        );
        if is_exact_tuple || !self.class_has_builtin_tuple_base(&tuple_subclass) {
            return;
        }
        if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
            class_data
                .attrs
                .insert("__bases__".to_string(), bases_value.clone());
        }
    }

    pub(super) fn builtin_type_call(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let Some((receiver, call_args)) = args.split_first() else {
            return Err(RuntimeError::type_error(
                "type.__call__() requires a type object",
            ));
        };
        let Value::Class(class) = receiver else {
            return Err(RuntimeError::type_error(
                "type.__call__() requires a type object",
            ));
        };
        self.suppress_metaclass_dispatch_depth += 1;
        let outcome = self.call_internal(Value::Class(class.clone()), call_args.to_vec(), kwargs);
        self.suppress_metaclass_dispatch_depth =
            self.suppress_metaclass_dispatch_depth.saturating_sub(1);
        match outcome? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("type.__call__ failed"))
            }
        }
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

    pub(super) fn builtin_type_mro(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("mro() does not accept keyword arguments"));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new("mro() takes no arguments"));
        }
        let class = match &args[0] {
            Value::Class(class) => class.clone(),
            _ => return Err(RuntimeError::new("mro() expected class receiver")),
        };
        let entries = self
            .class_mro_entries(&class)
            .into_iter()
            .map(Value::Class)
            .collect::<Vec<_>>();
        Ok(self.heap.alloc_list(entries))
    }

    pub(super) fn builtin_type_prepare(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 3 {
            return Err(RuntimeError::new(
                "__prepare__() missing required positional arguments",
            ));
        }
        let _ = kwargs;
        Ok(self.heap.alloc_dict(Vec::new()))
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
            Some(Value::Builtin(builtin)) if self.builtin_is_type_object(*builtin) => {
                return Err(RuntimeError::attribute_error(format!(
                    "type object '{}' has no attribute '__annotations__'",
                    self.builtin_type_name(*builtin)
                )));
            }
            _ => {
                return Err(RuntimeError::new(
                    "__annotations__ descriptor requires a type object",
                ));
            }
        };

        let existing_annotations = {
            let class_ref = class.kind();
            let Object::Class(class_data) = &*class_ref else {
                return Err(RuntimeError::new(
                    "__annotations__ descriptor requires a type object",
                ));
            };
            class_data
                .attrs
                .get("__annotations__")
                .cloned()
                .or_else(|| class_data.attrs.get("__annotations_cache__").cloned())
        };
        if let Some(existing) = existing_annotations {
            return match existing {
                Value::Dict(dict) => Ok(Value::Dict(dict)),
                _ => Err(RuntimeError::new("__annotations__ must be a dict")),
            };
        }

        let annotate_callable = {
            let class_ref = class.kind();
            let Object::Class(class_data) = &*class_ref else {
                return Err(RuntimeError::new(
                    "__annotations__ descriptor requires a type object",
                ));
            };
            class_data
                .attrs
                .get("__annotate__")
                .cloned()
                .or_else(|| class_data.attrs.get("__annotate_func__").cloned())
        };
        let annotations = if let Some(annotate_callable) = annotate_callable {
            if self.is_callable_value(&annotate_callable) {
                let mut annotate_format = Value::Int(1);
                if let Some(annotationlib) = self.modules.get("annotationlib").cloned()
                    && let Ok(format_enum) = self.builtin_getattr(
                        vec![
                            Value::Module(annotationlib),
                            Value::Str("Format".to_string()),
                        ],
                        HashMap::new(),
                    )
                    && let Ok(value_enum) = self.builtin_getattr(
                        vec![format_enum, Value::Str("VALUE".to_string())],
                        HashMap::new(),
                    )
                {
                    annotate_format = value_enum;
                }
                match self.call_internal(
                    annotate_callable,
                    vec![annotate_format],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(Value::Dict(dict)) => dict,
                    InternalCallOutcome::Value(other) => {
                        return Err(RuntimeError::type_error(format!(
                            "__annotate__ returned non-dict of type '{}'",
                            self.value_type_name_for_error(&other)
                        )));
                    }
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(
                            self.runtime_error_from_active_exception("class.__annotate__ failed")
                        );
                    }
                }
            } else {
                match self.heap.alloc_dict(Vec::new()) {
                    Value::Dict(dict) => dict,
                    _ => unreachable!(),
                }
            }
        } else {
            match self.heap.alloc_dict(Vec::new()) {
                Value::Dict(dict) => dict,
                _ => unreachable!(),
            }
        };

        let mut class_ref = class.kind_mut();
        let Object::Class(class_data) = &mut *class_ref else {
            return Err(RuntimeError::new(
                "__annotations__ descriptor requires a type object",
            ));
        };
        class_data.attrs.insert(
            "__annotations_cache__".to_string(),
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
        &mut self,
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
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        struct TypeInstancecheckBypassGuard;
        impl TypeInstancecheckBypassGuard {
            fn enter() -> Self {
                TYPE_INSTANCECHECK_BYPASS_CUSTOM.with(|depth| {
                    depth.set(depth.get().saturating_add(1));
                });
                Self
            }
        }
        impl Drop for TypeInstancecheckBypassGuard {
            fn drop(&mut self) {
                TYPE_INSTANCECHECK_BYPASS_CUSTOM.with(|depth| {
                    depth.set(depth.get().saturating_sub(1));
                });
            }
        }
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "__instancecheck__() expects one argument",
            ));
        }
        let _bypass_guard = TypeInstancecheckBypassGuard::enter();
        let classinfo = args.remove(0);
        let value = args.remove(0);
        // CPython type.__instancecheck__ is defined in terms of
        // issubclass(type(instance), cls).
        let candidate_type = self.builtin_type(vec![value], HashMap::new())?;
        Ok(Value::Bool(
            self.class_value_is_subclass_of(&candidate_type, &classinfo)?,
        ))
    }

    pub(super) fn builtin_issubclass(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "issubclass() expects two arguments",
            ));
        }
        let candidate = args.remove(0);
        let classinfo = args.remove(0);
        Ok(Value::Bool(
            self.class_value_is_subclass_of(&candidate, &classinfo)?,
        ))
    }

    pub(super) fn builtin_type_subclasscheck(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        struct TypeSubclasscheckBypassGuard;
        impl TypeSubclasscheckBypassGuard {
            fn enter() -> Self {
                TYPE_SUBCLASSCHECK_BYPASS_CUSTOM.with(|depth| {
                    depth.set(depth.get().saturating_add(1));
                });
                Self
            }
        }
        impl Drop for TypeSubclasscheckBypassGuard {
            fn drop(&mut self) {
                TYPE_SUBCLASSCHECK_BYPASS_CUSTOM.with(|depth| {
                    depth.set(depth.get().saturating_sub(1));
                });
            }
        }
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "__subclasscheck__() expects one argument",
            ));
        }
        let _bypass_guard = TypeSubclasscheckBypassGuard::enter();
        let classinfo = args.remove(0);
        let candidate = args.remove(0);
        Ok(Value::Bool(
            self.class_value_is_subclass_of(&candidate, &classinfo)?,
        ))
    }

    fn class_value_is_subclass_of_without_custom(
        &mut self,
        candidate: &Value,
        classinfo: &Value,
    ) -> Result<bool, RuntimeError> {
        struct TypeSubclasscheckBypassGuard;
        impl TypeSubclasscheckBypassGuard {
            fn enter() -> Self {
                TYPE_SUBCLASSCHECK_BYPASS_CUSTOM.with(|depth| {
                    depth.set(depth.get().saturating_add(1));
                });
                Self
            }
        }
        impl Drop for TypeSubclasscheckBypassGuard {
            fn drop(&mut self) {
                TYPE_SUBCLASSCHECK_BYPASS_CUSTOM.with(|depth| {
                    depth.set(depth.get().saturating_sub(1));
                });
            }
        }

        let _bypass_guard = TypeSubclasscheckBypassGuard::enter();
        self.class_value_is_subclass_of(candidate, classinfo)
    }

    fn abc_type_identity_eq(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Class(left_class), Value::Class(right_class)) => {
                left_class.id() == right_class.id()
            }
            (Value::Builtin(left_builtin), Value::Builtin(right_builtin)) => {
                left_builtin == right_builtin
            }
            (Value::ExceptionType(left_name), Value::ExceptionType(right_name)) => {
                left_name == right_name
            }
            (Value::Str(left_name), Value::Str(right_name)) => {
                is_runtime_type_name_marker(left_name)
                    && is_runtime_type_name_marker(right_name)
                    && left_name == right_name
            }
            _ => false,
        }
    }

    fn abc_vec_contains(values: &[Value], candidate: &Value) -> bool {
        values
            .iter()
            .any(|value| Self::abc_type_identity_eq(value, candidate))
    }

    fn abc_vec_insert_unique(values: &mut Vec<Value>, candidate: Value) {
        if !Self::abc_vec_contains(values, &candidate) {
            values.push(candidate);
        }
    }

    fn abc_is_type_value(&self, value: &Value) -> bool {
        match value {
            Value::Class(_) | Value::ExceptionType(_) => true,
            Value::Builtin(builtin) => self.builtin_is_type_object(*builtin),
            Value::Str(name) => is_runtime_type_name_marker(name),
            _ => false,
        }
    }

    fn class_uses_abc_meta(&self, class: &ObjRef) -> bool {
        let Some(meta_class) = self.class_of_value(&Value::Class(class.clone())) else {
            return false;
        };
        self.class_mro_entries(&meta_class).iter().any(|entry| {
            let Object::Class(class_data) = &*entry.kind() else {
                return false;
            };
            class_data.name == "ABCMeta"
                && matches!(
                    class_data.attrs.get("__module__"),
                    Some(Value::Str(module_name)) if module_name == "abc"
                )
        })
    }

    fn abc_abstract_method_names(&self, value: &Value) -> Vec<String> {
        let mut names = match value {
            Value::Set(obj) => match &*obj.kind() {
                Object::Set(values) => values
                    .iter()
                    .filter_map(|item| match item {
                        Value::Str(name) => Some(name.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            },
            Value::FrozenSet(obj) => match &*obj.kind() {
                Object::FrozenSet(values) => values
                    .iter()
                    .filter_map(|item| match item {
                        Value::Str(name) => Some(name.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            },
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values
                    .iter()
                    .filter_map(|item| match item {
                        Value::Str(name) => Some(name.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => values
                    .iter()
                    .filter_map(|item| match item {
                        Value::Str(name) => Some(name.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        names.sort();
        names.dedup();
        names
    }

    fn abc_class_ref_arg<'a>(
        &self,
        value: &'a Value,
        name: &str,
    ) -> Result<&'a ObjRef, RuntimeError> {
        match value {
            Value::Class(class) => Ok(class),
            _ => Err(RuntimeError::type_error(format!("{name} must be a class"))),
        }
    }

    fn abc_subclasscheck_internal(
        &mut self,
        cls: &Value,
        subclass: &Value,
    ) -> Result<bool, RuntimeError> {
        let cls_ref = self.abc_class_ref_arg(cls, "_abc_subclasscheck() arg 1")?;
        if !self.abc_is_type_value(subclass) {
            return Err(RuntimeError::type_error(
                "issubclass() arg 1 must be a class",
            ));
        }
        let cls_id = cls_ref.id();
        let subclass_is_marker =
            matches!(subclass, Value::Str(name) if is_runtime_type_name_marker(name));
        if let Some(cache) = self.abc_cache.get(&cls_id)
            && Self::abc_vec_contains(cache, subclass)
        {
            return Ok(true);
        }

        let stale_negative_cache = self
            .abc_negative_cache_version
            .get(&cls_id)
            .copied()
            .unwrap_or(self.abc_invalidation_counter)
            < self.abc_invalidation_counter;
        if stale_negative_cache {
            self.abc_negative_cache.insert(cls_id, Vec::new());
            self.abc_negative_cache_version
                .insert(cls_id, self.abc_invalidation_counter);
        } else if let Some(negative_cache) = self.abc_negative_cache.get(&cls_id)
            && Self::abc_vec_contains(negative_cache, subclass)
        {
            return Ok(false);
        }

        if !subclass_is_marker {
            let subclasshook = match self.builtin_getattr(
                vec![cls.clone(), Value::Str("__subclasshook__".to_string())],
                HashMap::new(),
            ) {
                Ok(value) => Some(value),
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => None,
                Err(err) => return Err(err),
            };
            if let Some(subclasshook) = subclasshook {
                let hook_result = match self.call_internal(
                    subclasshook,
                    vec![subclass.clone()],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(
                            self.runtime_error_from_active_exception("_abc_subclasscheck failed")
                        );
                    }
                };
                if !self.is_not_implemented_singleton(&hook_result) {
                    let accepted = self.truthy_from_value(&hook_result)?;
                    if accepted {
                        let cache = self.abc_cache.entry(cls_id).or_default();
                        Self::abc_vec_insert_unique(cache, subclass.clone());
                    } else {
                        let negative_cache = self.abc_negative_cache.entry(cls_id).or_default();
                        Self::abc_vec_insert_unique(negative_cache, subclass.clone());
                    }
                    return Ok(accepted);
                }
            }
        }

        if !subclass_is_marker && self.class_value_is_subclass_of_without_custom(subclass, cls)? {
            let cache = self.abc_cache.entry(cls_id).or_default();
            Self::abc_vec_insert_unique(cache, subclass.clone());
            return Ok(true);
        }

        if let Some(registry) = self.abc_registry.get(&cls_id).cloned() {
            for registered in registry {
                if Self::abc_type_identity_eq(subclass, &registered) {
                    let cache = self.abc_cache.entry(cls_id).or_default();
                    Self::abc_vec_insert_unique(cache, subclass.clone());
                    return Ok(true);
                }
                let registered_is_marker =
                    matches!(&registered, Value::Str(name) if is_runtime_type_name_marker(name));
                if !subclass_is_marker
                    && !registered_is_marker
                    && self.class_value_is_subclass_of(subclass, &registered)?
                {
                    let cache = self.abc_cache.entry(cls_id).or_default();
                    Self::abc_vec_insert_unique(cache, subclass.clone());
                    return Ok(true);
                }
            }
        }

        let negative_cache = self.abc_negative_cache.entry(cls_id).or_default();
        Self::abc_vec_insert_unique(negative_cache, subclass.clone());
        Ok(false)
    }

    pub(super) fn builtin_abc_get_cache_token(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("get_cache_token() expects no arguments"));
        }
        Ok(Value::Int(self.abc_invalidation_counter as i64))
    }

    pub(super) fn builtin_abc_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("_abc_init() expects one argument"));
        }
        let cls = args.remove(0);
        let cls_ref = self.abc_class_ref_arg(&cls, "_abc_init() arg 1")?.clone();
        let cls_value = Value::Class(cls_ref.clone());
        let cls_id = cls_ref.id();
        self.abc_registry.entry(cls_id).or_default();
        self.abc_cache.entry(cls_id).or_default();
        self.abc_negative_cache.entry(cls_id).or_default();
        self.abc_negative_cache_version
            .insert(cls_id, self.abc_invalidation_counter);

        let mut abstract_names = Vec::<String>::new();
        let base_classes = match &*cls_ref.kind() {
            Object::Class(class_data) => {
                for (name, value) in &class_data.attrs {
                    if let Some(is_abstract) =
                        self.optional_getattr_value(value.clone(), "__isabstractmethod__")?
                        && self.truthy_from_value(&is_abstract)?
                    {
                        if !abstract_names.contains(name) {
                            abstract_names.push(name.clone());
                        }
                    }
                }
                class_data.bases.clone()
            }
            _ => Vec::new(),
        };
        for base in base_classes {
            let Some(base_methods_value) =
                self.optional_getattr_value(Value::Class(base), "__abstractmethods__")?
            else {
                continue;
            };
            for name in self.abc_abstract_method_names(&base_methods_value) {
                let Some(attr_value) =
                    self.optional_getattr_value(cls_value.clone(), name.as_str())?
                else {
                    continue;
                };
                if let Some(is_abstract) =
                    self.optional_getattr_value(attr_value, "__isabstractmethod__")?
                    && self.truthy_from_value(&is_abstract)?
                    && !abstract_names.contains(&name)
                {
                    abstract_names.push(name);
                }
            }
        }
        abstract_names.sort();
        let abstract_values = abstract_names
            .into_iter()
            .map(Value::Str)
            .collect::<Vec<_>>();
        if let Object::Class(class_data) = &mut *cls_ref.kind_mut() {
            class_data.attrs.insert(
                "__abstractmethods__".to_string(),
                self.heap.alloc_frozenset(abstract_values),
            );
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_abc_register(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("_abc_register() expects two arguments"));
        }
        let cls = args.remove(0);
        let subclass = args.remove(0);
        let cls_ref = self.abc_class_ref_arg(&cls, "_abc_register() arg 1")?;
        if !self.abc_is_type_value(&subclass) {
            return Err(RuntimeError::type_error("Can only register classes"));
        }
        let subclass_is_marker =
            matches!(&subclass, Value::Str(name) if is_runtime_type_name_marker(name));
        if !subclass_is_marker {
            if self.class_value_is_subclass_of(&subclass, &cls)? {
                return Ok(subclass);
            }
            if self.class_value_is_subclass_of_without_custom(&cls, &subclass)? {
                return Err(RuntimeError::runtime_error(
                    "Refusing to create an inheritance cycle",
                ));
            }
        }

        let mut registry_targets = Vec::new();
        registry_targets.push(cls_ref.id());
        for base in self.class_mro_entries(cls_ref) {
            if self.class_uses_abc_meta(&base) {
                registry_targets.push(base.id());
            }
        }
        registry_targets.sort_unstable();
        registry_targets.dedup();
        for cls_id in registry_targets {
            let registry = self.abc_registry.entry(cls_id).or_default();
            Self::abc_vec_insert_unique(registry, subclass.clone());
        }

        self.abc_invalidation_counter = self.abc_invalidation_counter.saturating_add(1);
        Ok(subclass)
    }

    pub(super) fn builtin_abc_instancecheck(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "_abc_instancecheck() expects two arguments",
            ));
        }
        let cls = args.remove(0);
        let instance = args.remove(0);
        let subclass = self.call_builtin(BuiltinFunction::Type, vec![instance], HashMap::new())?;
        Ok(Value::Bool(
            self.abc_subclasscheck_internal(&cls, &subclass)?,
        ))
    }

    pub(super) fn builtin_abc_subclasscheck(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "_abc_subclasscheck() expects two arguments",
            ));
        }
        let cls = args.remove(0);
        let subclass = args.remove(0);
        Ok(Value::Bool(
            self.abc_subclasscheck_internal(&cls, &subclass)?,
        ))
    }

    pub(super) fn builtin_abc_get_dump(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("_get_dump() expects one argument"));
        }
        let cls = args.remove(0);
        let cls_ref = self.abc_class_ref_arg(&cls, "_get_dump() arg 1")?;
        let cls_id = cls_ref.id();
        let registry = self.abc_registry.get(&cls_id).cloned().unwrap_or_default();
        let cache = self.abc_cache.get(&cls_id).cloned().unwrap_or_default();
        let negative_cache = self
            .abc_negative_cache
            .get(&cls_id)
            .cloned()
            .unwrap_or_default();
        let version = self
            .abc_negative_cache_version
            .get(&cls_id)
            .copied()
            .unwrap_or(self.abc_invalidation_counter);
        Ok(self.heap.alloc_tuple(vec![
            self.heap.alloc_set(registry),
            self.heap.alloc_set(cache),
            self.heap.alloc_set(negative_cache),
            Value::Int(version as i64),
        ]))
    }

    pub(super) fn builtin_abc_reset_registry(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("_reset_registry() expects one argument"));
        }
        let cls = args.remove(0);
        let cls_ref = self.abc_class_ref_arg(&cls, "_reset_registry() arg 1")?;
        self.abc_registry.insert(cls_ref.id(), Vec::new());
        Ok(Value::None)
    }

    pub(super) fn builtin_abc_reset_caches(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("_reset_caches() expects one argument"));
        }
        let cls = args.remove(0);
        let cls_ref = self.abc_class_ref_arg(&cls, "_reset_caches() arg 1")?;
        let cls_id = cls_ref.id();
        self.abc_cache.insert(cls_id, Vec::new());
        self.abc_negative_cache.insert(cls_id, Vec::new());
        self.abc_negative_cache_version
            .insert(cls_id, self.abc_invalidation_counter);
        Ok(Value::None)
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
            return Err(RuntimeError::type_error(
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
                return Err(RuntimeError::type_error(
                    "object.__new__(X): X is not a type object",
                ));
            }
        };
        // `super(...).__new__(cls)` can arrive here as a bound built-in call shape
        // where the explicit `cls` argument is still present in `args`.
        if let Some(explicit_class) = args.first().cloned() {
            let explicit_class_ref = match explicit_class {
                Value::Class(explicit_class) => Some(explicit_class),
                Value::Builtin(builtin) if self.builtin_is_type_object(builtin) => {
                    Some(self.class_from_base_value(Value::Builtin(builtin))?)
                }
                _ => None,
            };
            if let Some(explicit_class_ref) = explicit_class_ref {
                let explicit_is_compatible = explicit_class_ref.id() == class_ref.id()
                    || self
                        .class_mro_entries(&explicit_class_ref)
                        .iter()
                        .any(|entry| entry.id() == class_ref.id());
                if explicit_is_compatible {
                    class_ref = explicit_class_ref;
                    args.remove(0);
                }
            }
        }
        if self.class_has_generic_alias_base(&class_ref) {
            return self.instantiate_generic_alias_class(class_ref, args, kwargs);
        }
        if let Some(message) = self.class_disallow_instantiation_message(&class_ref) {
            return Err(RuntimeError::type_error(message));
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
            return Err(RuntimeError::type_error(
                "object.__new__() takes exactly one argument",
            ));
        }
        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_traceback_type_new(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "TracebackType() takes no keyword arguments",
            ));
        }
        if args.is_empty() {
            return Err(RuntimeError::type_error(
                "TracebackType() expects a class argument",
            ));
        }
        let class_ref = match args.remove(0) {
            Value::Class(class) => class,
            Value::Builtin(builtin) if self.builtin_is_type_object(builtin) => {
                self.class_from_base_value(Value::Builtin(builtin))?
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "TracebackType.__new__(X): X is not a type object",
                ));
            }
        };

        if args.len() != 4 {
            return Err(RuntimeError::type_error(format!(
                "TracebackType() takes exactly 4 arguments ({} given)",
                args.len()
            )));
        }

        let tb_next = args.remove(0);
        let tb_frame = args.remove(0);
        let tb_lasti = value_to_int(args.remove(0))?;
        let tb_lineno = value_to_int(args.remove(0))?;

        if !matches!(tb_next, Value::None) {
            let is_traceback = match &tb_next {
                Value::Instance(instance) => match &*instance.kind() {
                    Object::Instance(instance_data) => {
                        matches!(
                            instance_data.attrs.get("__pyrs_traceback_marker__"),
                            Some(Value::Bool(true))
                        )
                    }
                    _ => false,
                },
                _ => false,
            };
            if !is_traceback {
                return Err(RuntimeError::type_error(
                    "tb_next must be a traceback object",
                ));
            }
        }

        let (f_code, f_lineno_opt, f_globals_opt, f_locals_opt, f_builtins_opt, f_back_opt) =
            match &tb_frame {
                Value::Module(module) => match &*module.kind() {
                    Object::Module(module_data) => (
                        module_data.globals.get("f_code").cloned(),
                        module_data.globals.get("f_lineno").cloned(),
                        module_data.globals.get("f_globals").cloned(),
                        module_data.globals.get("f_locals").cloned(),
                        module_data.globals.get("f_builtins").cloned(),
                        module_data.globals.get("f_back").cloned(),
                    ),
                    _ => (None, None, None, None, None, None),
                },
                Value::Instance(instance) => match &*instance.kind() {
                    Object::Instance(instance_data) => (
                        instance_data.attrs.get("f_code").cloned(),
                        instance_data.attrs.get("f_lineno").cloned(),
                        instance_data.attrs.get("f_globals").cloned(),
                        instance_data.attrs.get("f_locals").cloned(),
                        instance_data.attrs.get("f_builtins").cloned(),
                        instance_data.attrs.get("f_back").cloned(),
                    ),
                    _ => (None, None, None, None, None, None),
                },
                _ => (None, None, None, None, None, None),
            };

        let Some(Value::Code(code)) = f_code else {
            return Err(RuntimeError::type_error("tb_frame must be a frame object"));
        };
        let filename = code.filename.clone();
        let name = code.name.clone();
        let frame_lineno = match f_lineno_opt {
            Some(Value::Int(line)) if line >= 0 => line,
            _ => 0,
        };
        let frame_code_value = if tb_lineno >= 0 {
            let mut synthetic = CodeObject::new(code.name.clone(), code.filename.clone());
            synthetic.first_line = tb_lineno.max(1) as usize;
            synthetic
                .instructions
                .push(crate::bytecode::Instruction::new(
                    crate::bytecode::Opcode::Nop,
                    None,
                ));
            synthetic
                .locations
                .push(crate::bytecode::Location::with_end(
                    tb_lineno as usize,
                    1,
                    tb_lineno as usize,
                    1,
                ));
            Value::Code(Rc::new(synthetic))
        } else {
            Value::Code(code.clone())
        };
        let frame_value = match tb_frame {
            Value::Module(_) => {
                let frame_class = self
                    .types_module_or_private_class("FrameType")
                    .unwrap_or_else(|| {
                        match self
                            .heap
                            .alloc_class(ClassObject::new("frame".to_string(), Vec::new()))
                        {
                            Value::Class(class) => class,
                            _ => unreachable!(),
                        }
                    });
                let frame_instance = match self
                    .heap
                    .alloc_instance(InstanceObject::new(frame_class.clone()))
                {
                    Value::Instance(instance) => instance,
                    _ => unreachable!(),
                };
                if let Object::Instance(instance_data) = &mut *frame_instance.kind_mut() {
                    instance_data
                        .attrs
                        .insert("f_code".to_string(), frame_code_value);
                    instance_data.attrs.insert(
                        "f_globals".to_string(),
                        f_globals_opt.unwrap_or_else(|| self.heap.alloc_dict(Vec::new())),
                    );
                    instance_data.attrs.insert(
                        "f_locals".to_string(),
                        f_locals_opt.unwrap_or_else(|| self.heap.alloc_dict(Vec::new())),
                    );
                    instance_data.attrs.insert(
                        "f_builtins".to_string(),
                        self.builtins_mapping_value_from_dunder_builtins(f_builtins_opt),
                    );
                    instance_data
                        .attrs
                        .insert("f_lineno".to_string(), Value::Int(frame_lineno));
                    instance_data
                        .attrs
                        .insert("f_back".to_string(), f_back_opt.unwrap_or(Value::None));
                }
                Value::Instance(frame_instance)
            }
            other => other,
        };

        let traceback = match self.heap.alloc_instance(InstanceObject::new(class_ref)) {
            Value::Instance(instance) => instance,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *traceback.kind_mut() {
            instance_data
                .attrs
                .insert("__pyrs_traceback_marker__".to_string(), Value::Bool(true));
            instance_data
                .attrs
                .insert("__pyrs_tb_filename__".to_string(), Value::Str(filename));
            instance_data
                .attrs
                .insert("__pyrs_tb_name__".to_string(), Value::Str(name));
            instance_data
                .attrs
                .insert("__pyrs_tb_column__".to_string(), Value::Int(0));
            instance_data.attrs.insert(
                "__pyrs_tb_end_line__".to_string(),
                Value::Int(tb_lineno.max(frame_lineno)),
            );
            instance_data
                .attrs
                .insert("__pyrs_tb_end_column__".to_string(), Value::Int(0));
            instance_data.attrs.insert(
                "__pyrs_tb_frame_id__".to_string(),
                Value::Int(self.id_of(&frame_value) as i64),
            );
            instance_data
                .attrs
                .insert("tb_lineno".to_string(), Value::Int(tb_lineno));
            instance_data
                .attrs
                .insert("tb_lasti".to_string(), Value::Int(tb_lasti));
            instance_data
                .attrs
                .insert("tb_frame".to_string(), frame_value);
            instance_data.attrs.insert("tb_next".to_string(), tb_next);
        }

        Ok(Value::Instance(traceback))
    }

    pub(super) fn builtin_object_init(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        // `object.__init__` is exposed as a plain builtin in this VM, so
        // super() calls can reach it without an implicit `self` bind.
        if !kwargs.is_empty() || args.is_empty() {
            return Err(RuntimeError::type_error(
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
                if self.host.env_var_os("PYRS_TRACE_OBJECT_INIT").is_some() {
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
                return Err(RuntimeError::type_error(
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
                let group_members_tuple =
                    if self.exception_inherits(exception_name.as_str(), "BaseExceptionGroup") {
                        let members_source = args
                            .get(1)
                            .cloned()
                            .unwrap_or_else(|| self.heap.alloc_tuple(Vec::new()));
                        let members = self.exception_members_from_value(&members_source)?;
                        let member_values = members
                            .into_iter()
                            .map(|member| Value::Exception(Box::new(member)))
                            .collect::<Vec<_>>();
                        Some(self.heap.alloc_tuple(member_values))
                    } else {
                        None
                    };
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
                    let is_system_exit =
                        self.exception_inherits(exception_name.as_str(), "SystemExit");
                    instance_data
                        .attrs
                        .insert("args".to_string(), self.heap.alloc_tuple(args.clone()));
                    instance_data
                        .attrs
                        .entry("__traceback__".to_string())
                        .or_insert(Value::None);
                    instance_data
                        .attrs
                        .entry("__cause__".to_string())
                        .or_insert(Value::None);
                    instance_data
                        .attrs
                        .entry("__context__".to_string())
                        .or_insert(Value::None);
                    instance_data
                        .attrs
                        .entry("__suppress_context__".to_string())
                        .or_insert(Value::Bool(false));
                    if is_stop_iteration {
                        let value = args.first().cloned().unwrap_or(Value::None);
                        instance_data.attrs.insert("value".to_string(), value);
                    }
                    if is_system_exit {
                        let code = self.system_exit_code_from_args(&args);
                        instance_data.attrs.insert("code".to_string(), code);
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
                    if self.exception_inherits(exception_name.as_str(), "SyntaxError") {
                        self.populate_syntax_error_attrs(&mut instance_data.attrs, &args);
                    }
                    if let Some(group_members_tuple) = group_members_tuple {
                        instance_data
                            .attrs
                            .insert("exceptions".to_string(), group_members_tuple);
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
                let group_members =
                    if self.exception_inherits(exception.name.as_str(), "BaseExceptionGroup") {
                        let members_source = args
                            .get(1)
                            .cloned()
                            .unwrap_or_else(|| self.heap.alloc_tuple(Vec::new()));
                        Some(self.exception_members_from_value(&members_source)?)
                    } else {
                        None
                    };
                let mut attrs = exception.attrs.borrow_mut();
                attrs.insert("args".to_string(), self.heap.alloc_tuple(args.clone()));
                attrs
                    .entry("__traceback__".to_string())
                    .or_insert(Value::None);
                attrs.entry("__cause__".to_string()).or_insert(Value::None);
                attrs
                    .entry("__context__".to_string())
                    .or_insert(Value::None);
                attrs
                    .entry("__suppress_context__".to_string())
                    .or_insert(Value::Bool(false));
                if matches!(
                    exception.name.as_str(),
                    "StopIteration" | "StopAsyncIteration"
                ) {
                    attrs.insert(
                        "value".to_string(),
                        args.first().cloned().unwrap_or(Value::None),
                    );
                }
                if self.exception_inherits(exception.name.as_str(), "SystemExit") {
                    attrs.insert("code".to_string(), self.system_exit_code_from_args(&args));
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
                if self.exception_inherits(exception.name.as_str(), "SyntaxError") {
                    self.populate_syntax_error_attrs(&mut attrs, &args);
                }
                if let Some(group_members) = group_members {
                    let member_values = group_members
                        .iter()
                        .cloned()
                        .map(|member| Value::Exception(Box::new(member)))
                        .collect::<Vec<_>>();
                    attrs.insert(
                        "exceptions".to_string(),
                        self.heap.alloc_tuple(member_values),
                    );
                    exception.exceptions = group_members;
                    exception.message = args.first().map(format_value);
                }
                Ok(Value::None)
            }
            _ => Err(RuntimeError::new(
                "descriptor '__init__' requires a 'BaseException' object",
            )),
        }
    }

    pub(super) fn builtin_exception_type_str(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "__str__() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error("__str__() takes no arguments"));
        }
        let receiver = args.remove(0);
        let rendered = match receiver {
            Value::Exception(exception) => self.exception_str_value(&exception),
            Value::Instance(instance) => {
                let Some((class_name, args, member_count)) =
                    self.exception_instance_name_args_and_member_count(&instance)
                else {
                    return Err(RuntimeError::type_error(
                        "descriptor '__str__' requires a 'BaseException' object",
                    ));
                };
                self.exception_str_from_name_and_args(class_name.as_str(), &args, member_count)?
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "descriptor '__str__' requires a 'BaseException' object",
                ));
            }
        };
        Ok(Value::Str(rendered))
    }

    pub(super) fn builtin_exception_type_repr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "__repr__() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error("__repr__() takes no arguments"));
        }
        let receiver = args.remove(0);
        let rendered = match receiver {
            Value::Exception(exception) => self.exception_repr_value(&exception),
            Value::Instance(instance) => {
                let Some((class_name, args, _)) =
                    self.exception_instance_name_args_and_member_count(&instance)
                else {
                    return Err(RuntimeError::type_error(
                        "descriptor '__repr__' requires a 'BaseException' object",
                    ));
                };
                self.exception_repr_from_name_and_args(class_name.as_str(), &args)
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "descriptor '__repr__' requires a 'BaseException' object",
                ));
            }
        };
        Ok(Value::Str(rendered))
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
            if self
                .host
                .env_var_os("PYRS_TRACE_OBJECT_INIT_CLASS")
                .is_some()
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
        if self
            .host
            .env_var_os("PYRS_TRACE_OBJECT_INIT_CLASS")
            .is_some()
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
            _ => return Err(RuntimeError::type_error("attribute name must be string")),
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
            let type_name = self.runtime_format_type_name(&target);
            return Err(RuntimeError::type_error(format!(
                "unsupported format string passed to {}.__format__",
                type_name
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
            _ => return Err(RuntimeError::type_error("attribute name must be string")),
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
                if self
                    .host
                    .env_var_os("PYRS_TRACE_SETATTR_UNSUPPORTED")
                    .is_some()
                {
                    eprintln!(
                        "[object-setattr-unsupported] target={} name={}",
                        format_repr(&other),
                        name
                    );
                }
                return Err(RuntimeError::type_error(
                    "attribute assignment unsupported type",
                ));
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
            _ => return Err(RuntimeError::type_error("attribute name must be string")),
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
            _ => {
                return Err(RuntimeError::type_error(
                    "attribute deletion unsupported type",
                ));
            }
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
            match self.collect_iterable_values(args.remove(0)) {
                Ok(values) => values,
                Err(err)
                    if runtime_error_matches_exception(&err, "TypeError")
                        && err.message.contains("expected iterable") =>
                {
                    return Err(RuntimeError::type_error("object is not iterable"));
                }
                Err(err) => return Err(err),
            }
        };
        Ok(self.heap.alloc_list(values))
    }

    pub(super) fn builtin_tuple(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "tuple() expects at most one argument",
            ));
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
                    _ => {
                        return Err(RuntimeError::type_error(
                            "tuple() expects at most one argument",
                        ));
                    }
                }
                Some(args.remove(0))
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "tuple() expects at most one argument",
                ));
            }
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

    pub(super) fn builtin_dict_with_order(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        kwargs_order: Option<Vec<String>>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::type_error(
                "dict() expects at most one argument",
            ));
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
                _ => {
                    return Err(RuntimeError::type_error(
                        "dict() expects at most one argument",
                    ));
                }
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

        let mut kwargs = kwargs;
        if let Some(order) = kwargs_order {
            for name in order {
                if let Some(value) = kwargs.remove(&name) {
                    dict_set_value_checked(&dict_obj, Value::Str(name), value)?;
                }
            }
        }
        for (name, value) in kwargs {
            dict_set_value_checked(&dict_obj, Value::Str(name), value)?;
        }

        Ok(Value::Dict(dict_obj))
    }

    pub(super) fn builtin_dict(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_dict_with_order(args, kwargs, None)
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
            let parts = match self.collect_iterable_values(item) {
                Ok(values) => values,
                Err(err) if runtime_error_matches_exception(&err, "TypeError") => {
                    return Err(RuntimeError::new(
                        "dict() argument must be a mapping or iterable of pairs",
                    ));
                }
                Err(err) => return Err(err),
            };
            if parts.len() != 2 {
                return Err(RuntimeError::new(
                    "dict() sequence elements must be length 2",
                ));
            }
            dict_set_value_checked(dict_obj, parts[0].clone(), parts[1].clone())?;
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
            self.dict_set_value_checked_runtime(&dict_obj, key, default.clone())?;
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
            return Err(RuntimeError::type_error(
                "set() expects at most one argument",
            ));
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
                _ => {
                    return Err(RuntimeError::type_error(
                        "set() expects at most one argument",
                    ));
                }
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
            return Err(RuntimeError::type_error(
                "set() expects at most one argument",
            ));
        };
        let deduped = self.dedup_hashable_values_runtime(values)?;
        Ok(self.heap.alloc_set(deduped))
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
        let deduped = self.dedup_hashable_values_runtime(values)?;
        Ok(self.heap.alloc_frozenset(deduped))
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

    fn parse_decimal_format_number(
        &self,
        chars: &[char],
        idx: &mut usize,
    ) -> Result<Option<usize>, RuntimeError> {
        let start = *idx;
        let mut value = 0usize;
        while *idx < chars.len() && chars[*idx].is_ascii_digit() {
            let digit = (chars[*idx] as u8 - b'0') as usize;
            value = value
                .checked_mul(10)
                .and_then(|current| current.checked_add(digit))
                .ok_or_else(|| {
                    RuntimeError::value_error("Too many decimal digits in format string")
                })?;
            *idx += 1;
        }
        if *idx == start {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    fn parse_numeric_format_spec(&self, spec: &str) -> Result<NumericFormatSpec, RuntimeError> {
        if spec.is_empty() {
            return Ok(NumericFormatSpec::default());
        }

        let chars: Vec<char> = spec.chars().collect();
        let mut idx = 0usize;
        let mut parsed = NumericFormatSpec::default();
        let is_align = |ch: char| matches!(ch, '<' | '>' | '=' | '^');

        if idx + 1 < chars.len() && is_align(chars[idx + 1]) {
            parsed.fill = chars[idx];
            parsed.align = Some(chars[idx + 1]);
            idx += 2;
        } else if idx < chars.len() && is_align(chars[idx]) {
            parsed.align = Some(chars[idx]);
            idx += 1;
        }

        if idx < chars.len() && matches!(chars[idx], '+' | '-' | ' ') {
            parsed.sign = chars[idx];
            idx += 1;
        }

        if idx < chars.len() && chars[idx] == '#' {
            parsed.alternate = true;
            idx += 1;
        }

        if idx < chars.len() && chars[idx] == '0' {
            parsed.zero_pad = true;
            idx += 1;
        }

        parsed.width = self.parse_decimal_format_number(&chars, &mut idx)?;

        if idx < chars.len() && matches!(chars[idx], ',' | '_') {
            parsed.grouping = Some(chars[idx]);
            idx += 1;
        }

        if idx < chars.len() && chars[idx] == '.' {
            idx += 1;
            parsed.precision = Some(
                self.parse_decimal_format_number(&chars, &mut idx)?
                    .ok_or_else(|| {
                        RuntimeError::value_error("Format specifier missing precision")
                    })?,
            );
        }

        if idx < chars.len() {
            parsed.ty = Some(chars[idx]);
            idx += 1;
        }

        if idx != chars.len() {
            return Err(RuntimeError::value_error("Invalid format specifier"));
        }

        Ok(parsed)
    }

    fn resolve_fill_align(&self, spec: &NumericFormatSpec, numeric: bool) -> (char, char) {
        let mut fill = spec.fill;
        let mut align = spec.align.unwrap_or('>');
        if spec.zero_pad && spec.align.is_none() {
            fill = '0';
            align = if numeric { '=' } else { '>' };
        }
        (fill, align)
    }

    fn apply_alignment(
        &self,
        text: String,
        width: Option<usize>,
        fill: char,
        align: char,
    ) -> String {
        let Some(width) = width else {
            return text;
        };
        let len = text.chars().count();
        if len >= width {
            return text;
        }
        let pad_len = width - len;
        match align {
            '<' => {
                let mut out = text;
                out.extend(std::iter::repeat_n(fill, pad_len));
                out
            }
            '^' => {
                let left = pad_len / 2;
                let right = pad_len - left;
                let mut out = String::new();
                out.extend(std::iter::repeat_n(fill, left));
                out.push_str(&text);
                out.extend(std::iter::repeat_n(fill, right));
                out
            }
            _ => {
                let mut out = String::new();
                out.extend(std::iter::repeat_n(fill, pad_len));
                out.push_str(&text);
                out
            }
        }
    }

    fn apply_numeric_alignment_with_prefix(
        &self,
        prefix: &str,
        body: &str,
        width: Option<usize>,
        fill: char,
        align: char,
    ) -> String {
        let combined = format!("{prefix}{body}");
        if align != '=' {
            return self.apply_alignment(combined, width, fill, align);
        }
        let Some(width) = width else {
            return combined;
        };
        let len = combined.chars().count();
        if len >= width {
            return combined;
        }
        let mut out = String::new();
        out.push_str(prefix);
        out.extend(std::iter::repeat_n(fill, width - len));
        out.push_str(body);
        out
    }

    fn group_digits(&self, digits: &str, separator: char, group_size: usize) -> String {
        if digits.is_empty() || group_size == 0 {
            return digits.to_string();
        }
        let chars: Vec<char> = digits.chars().collect();
        if chars.len() <= group_size {
            return digits.to_string();
        }
        let mut out = String::new();
        for (idx, ch) in chars.iter().enumerate() {
            if idx > 0 && (chars.len() - idx) % group_size == 0 {
                out.push(separator);
            }
            out.push(*ch);
        }
        out
    }

    fn grouped_len(&self, digits_len: usize, group_size: usize) -> usize {
        if digits_len == 0 || group_size == 0 {
            return digits_len;
        }
        digits_len + (digits_len.saturating_sub(1) / group_size)
    }

    fn padded_digits_for_grouped_width(
        &self,
        target_len: usize,
        min_digits_len: usize,
        group_size: usize,
    ) -> usize {
        let mut digits_len = min_digits_len;
        while self.grouped_len(digits_len, group_size) < target_len {
            digits_len += 1;
        }
        digits_len
    }

    fn format_double_with_snprintf(
        &self,
        format_text: &str,
        value: f64,
    ) -> Result<String, RuntimeError> {
        #[cfg(target_arch = "wasm32")]
        {
            format_float_with_c_pattern(format_text, value)
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::ffi::CString;
            use std::os::raw::{c_char, c_int};

            unsafe extern "C" {
                fn snprintf(buffer: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
            }

            let c_format = CString::new(format_text)
                .map_err(|_| RuntimeError::value_error("invalid format string"))?;
            // SAFETY: `c_format` is NUL-terminated and valid for both calls.
            let needed = unsafe { snprintf(std::ptr::null_mut(), 0, c_format.as_ptr(), value) };
            if needed < 0 {
                return Err(RuntimeError::value_error("failed to format float"));
            }
            let mut buffer = vec![0u8; needed as usize + 1];
            // SAFETY: buffer is writable and large enough for output including trailing NUL.
            let wrote = unsafe {
                snprintf(
                    buffer.as_mut_ptr().cast::<c_char>(),
                    buffer.len(),
                    c_format.as_ptr(),
                    value,
                )
            };
            if wrote < 0 {
                return Err(RuntimeError::value_error("failed to format float"));
            }
            buffer.truncate(wrote as usize);
            Ok(String::from_utf8_lossy(&buffer).into_owned())
        }
    }

    fn apply_grouping_to_float_text(
        &self,
        text: &str,
        separator: char,
        width: Option<usize>,
        fill: char,
        align: char,
    ) -> String {
        let (sign_prefix, unsigned) =
            if text.starts_with('-') || text.starts_with('+') || text.starts_with(' ') {
                (&text[..1], &text[1..])
            } else {
                ("", text)
            };

        let (unsigned_core, percent_suffix) = if let Some(stripped) = unsigned.strip_suffix('%') {
            (stripped, "%")
        } else {
            (unsigned, "")
        };

        let exponent_pos = unsigned_core.find(['e', 'E']);
        let (mantissa, exponent_suffix) = if let Some(pos) = exponent_pos {
            (&unsigned_core[..pos], &unsigned_core[pos..])
        } else {
            (unsigned_core, "")
        };

        let (integer_part, fractional_suffix) = if let Some(dot_pos) = mantissa.find('.') {
            (&mantissa[..dot_pos], &mantissa[dot_pos..])
        } else {
            (mantissa, "")
        };

        if !integer_part.chars().all(|ch| ch.is_ascii_digit()) {
            return self.apply_numeric_alignment_with_prefix(
                sign_prefix,
                unsigned,
                width,
                fill,
                align,
            );
        }

        let grouped_integer = if align == '=' && fill == '0' {
            let Some(width) = width else {
                return format!(
                    "{sign_prefix}{}{}{}{}",
                    self.group_digits(integer_part, separator, 3),
                    fractional_suffix,
                    exponent_suffix,
                    percent_suffix
                );
            };
            let sign_len = sign_prefix.chars().count();
            let suffix_len = fractional_suffix.chars().count()
                + exponent_suffix.chars().count()
                + percent_suffix.chars().count();
            if width <= sign_len + suffix_len {
                self.group_digits(integer_part, separator, 3)
            } else {
                let target = width - sign_len - suffix_len;
                let padded_len =
                    self.padded_digits_for_grouped_width(target, integer_part.len(), 3);
                let mut padded = "0".repeat(padded_len.saturating_sub(integer_part.len()));
                padded.push_str(integer_part);
                self.group_digits(&padded, separator, 3)
            }
        } else {
            self.group_digits(integer_part, separator, 3)
        };

        let body = format!("{grouped_integer}{fractional_suffix}{exponent_suffix}{percent_suffix}");
        self.apply_numeric_alignment_with_prefix(sign_prefix, &body, width, fill, align)
    }

    fn locale_format_string(
        &mut self,
        format_pattern: String,
        value: Value,
    ) -> Result<String, RuntimeError> {
        let locale_module = self.import_module_object("locale")?;
        let formatter = self.load_attr_module(&locale_module, "format_string")?;
        let mut kwargs = HashMap::new();
        kwargs.insert("grouping".to_string(), Value::Bool(true));
        let result =
            match self.call_internal(formatter, vec![Value::Str(format_pattern), value], kwargs)? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(
                        self.runtime_error_from_active_exception("locale.format_string() failed")
                    );
                }
            };
        match result {
            Value::Str(text) => Ok(text),
            _ => Err(RuntimeError::type_error(
                "locale.format_string() returned non-str value",
            )),
        }
    }

    fn format_float_with_spec(
        &mut self,
        value: f64,
        spec: &NumericFormatSpec,
        type_code: char,
    ) -> Result<String, RuntimeError> {
        if type_code == 'n' && spec.grouping.is_some() {
            let group = spec.grouping.unwrap_or(',');
            return Err(RuntimeError::value_error(format!(
                "Cannot specify '{group}' with 'n'."
            )));
        }

        let (fill, align) = self.resolve_fill_align(spec, true);

        if type_code == 'n' {
            let mut pattern = String::from("%");
            if spec.sign == '+' {
                pattern.push('+');
            } else if spec.sign == ' ' {
                pattern.push(' ');
            }
            if spec.alternate {
                pattern.push('#');
            }
            if let Some(precision) = spec.precision {
                pattern.push('.');
                pattern.push_str(&precision.to_string());
            }
            pattern.push('g');
            let rendered = self.locale_format_string(pattern, Value::Float(value))?;
            return Ok(
                self.apply_numeric_alignment_with_prefix("", &rendered, spec.width, fill, align)
            );
        }

        let mut c_pattern = String::from("%");
        if spec.sign == '+' {
            c_pattern.push('+');
        } else if spec.sign == ' ' {
            c_pattern.push(' ');
        }
        if spec.alternate {
            c_pattern.push('#');
        }
        if let Some(precision) = spec.precision {
            c_pattern.push('.');
            c_pattern.push_str(&precision.to_string());
        }

        let rendered = if type_code == '%' {
            c_pattern.push('f');
            let mut out = self.format_double_with_snprintf(&c_pattern, value * 100.0)?;
            out.push('%');
            out
        } else {
            c_pattern.push(type_code);
            self.format_double_with_snprintf(&c_pattern, value)?
        };

        if let Some(group) = spec.grouping {
            return Ok(self.apply_grouping_to_float_text(&rendered, group, spec.width, fill, align));
        }

        if rendered.starts_with('-') || rendered.starts_with('+') || rendered.starts_with(' ') {
            let prefix = &rendered[..1];
            let body = &rendered[1..];
            Ok(self.apply_numeric_alignment_with_prefix(prefix, body, spec.width, fill, align))
        } else {
            Ok(self.apply_numeric_alignment_with_prefix("", &rendered, spec.width, fill, align))
        }
    }

    pub(super) fn format_bigint_with_spec(
        &mut self,
        value: &BigInt,
        spec_text: &str,
    ) -> Result<String, RuntimeError> {
        let spec = self.parse_numeric_format_spec(spec_text)?;
        let ty = spec.ty.unwrap_or('d');

        if matches!(ty, 'e' | 'E' | 'f' | 'F' | 'g' | 'G' | '%') {
            let float_value = value.to_f64();
            if !float_value.is_finite() {
                return Err(RuntimeError::overflow_error(
                    "int too large to convert to float",
                ));
            }
            return self.format_float_with_spec(float_value, &spec, ty);
        }

        if spec.precision.is_some() {
            return Err(RuntimeError::value_error(
                "Precision not allowed in integer format specifier",
            ));
        }

        if ty == 'c' {
            if spec.sign != '-' {
                return Err(RuntimeError::value_error(
                    "Sign not allowed with integer format specifier 'c'",
                ));
            }
            if spec.alternate {
                return Err(RuntimeError::value_error(
                    "Alternate form (#) not allowed with integer format specifier 'c'",
                ));
            }
            if let Some(group) = spec.grouping {
                return Err(RuntimeError::value_error(format!(
                    "Cannot specify '{group}' with 'c'."
                )));
            }

            let Some(codepoint) = value.to_i64() else {
                return Err(RuntimeError::overflow_error(
                    "%c arg not in range(0x110000)",
                ));
            };
            if !(0..=0x10ffff).contains(&codepoint) {
                return Err(RuntimeError::overflow_error(
                    "%c arg not in range(0x110000)",
                ));
            }
            let ch = char::from_u32(codepoint as u32)
                .ok_or_else(|| RuntimeError::overflow_error("%c arg not in range(0x110000)"))?;
            let (fill, align) = self.resolve_fill_align(&spec, false);
            return Ok(self.apply_alignment(ch.to_string(), spec.width, fill, align));
        }

        if ty == 'n' {
            if let Some(group) = spec.grouping {
                return Err(RuntimeError::value_error(format!(
                    "Cannot specify '{group}' with 'n'."
                )));
            }
            let mut pattern = String::from("%");
            if spec.sign == '+' {
                pattern.push('+');
            } else if spec.sign == ' ' {
                pattern.push(' ');
            }
            pattern.push('d');
            let rendered = self.locale_format_string(pattern, value_from_bigint(value.clone()))?;
            let (fill, align) = self.resolve_fill_align(&spec, true);
            if rendered.starts_with('-') || rendered.starts_with('+') || rendered.starts_with(' ') {
                return Ok(self.apply_numeric_alignment_with_prefix(
                    &rendered[..1],
                    &rendered[1..],
                    spec.width,
                    fill,
                    align,
                ));
            }
            return Ok(
                self.apply_numeric_alignment_with_prefix("", &rendered, spec.width, fill, align)
            );
        }

        if !matches!(ty, 'd' | 'b' | 'o' | 'x' | 'X') {
            return Err(RuntimeError::value_error(format!(
                "Unknown format code '{ty}' for object of type 'int'"
            )));
        }

        if spec.grouping == Some(',') && !matches!(ty, 'd') {
            return Err(RuntimeError::value_error(format!(
                "Cannot specify ',' with '{ty}'."
            )));
        }

        let abs_value = value.abs();
        let mut digits = match ty {
            'd' => abs_value.to_string(),
            'b' => abs_value
                .to_str_radix(2)
                .ok_or_else(|| RuntimeError::value_error("failed to format integer"))?,
            'o' => abs_value
                .to_str_radix(8)
                .ok_or_else(|| RuntimeError::value_error("failed to format integer"))?,
            'x' | 'X' => abs_value
                .to_str_radix(16)
                .ok_or_else(|| RuntimeError::value_error("failed to format integer"))?,
            _ => unreachable!(),
        };
        if ty == 'X' {
            digits = digits.to_ascii_uppercase();
        }

        let sign_prefix = if value.is_negative() {
            "-"
        } else if spec.sign == '+' {
            "+"
        } else if spec.sign == ' ' {
            " "
        } else {
            ""
        };

        let base_prefix = if spec.alternate {
            match ty {
                'b' => "0b",
                'o' => "0o",
                'x' => "0x",
                'X' => "0X",
                _ => "",
            }
        } else {
            ""
        };
        let prefix = format!("{sign_prefix}{base_prefix}");
        let (fill, align) = self.resolve_fill_align(&spec, true);

        let grouped_digits = if let Some(group) = spec.grouping {
            let group_size = if group == '_' && matches!(ty, 'b' | 'o' | 'x' | 'X') {
                4
            } else {
                3
            };
            if align == '=' && fill == '0' {
                if let Some(width) = spec.width {
                    let prefix_len = prefix.chars().count();
                    let target = width.saturating_sub(prefix_len);
                    let padded_len =
                        self.padded_digits_for_grouped_width(target, digits.len(), group_size);
                    let mut padded = "0".repeat(padded_len.saturating_sub(digits.len()));
                    padded.push_str(&digits);
                    self.group_digits(&padded, group, group_size)
                } else {
                    self.group_digits(&digits, group, group_size)
                }
            } else {
                self.group_digits(&digits, group, group_size)
            }
        } else {
            digits
        };

        Ok(self.apply_numeric_alignment_with_prefix(
            &prefix,
            &grouped_digits,
            spec.width,
            fill,
            align,
        ))
    }

    fn format_str_with_spec(&self, text: &str, spec: &str) -> Result<String, RuntimeError> {
        if spec.is_empty() || spec == "s" {
            return Ok(text.to_string());
        }
        let chars: Vec<char> = spec.chars().collect();
        let mut index = 0usize;
        let mut fill = ' ';
        let mut align = None;

        if chars.len() >= 2 && matches!(chars[1], '<' | '>' | '^' | '=') {
            fill = chars[0];
            align = Some(chars[1]);
            index = 2;
        } else if chars
            .first()
            .is_some_and(|ch| matches!(ch, '<' | '>' | '^' | '='))
        {
            align = chars.first().copied();
            index = 1;
        }

        let width_start = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
        let width = if index > width_start {
            chars[width_start..index]
                .iter()
                .collect::<String>()
                .parse::<usize>()
                .ok()
        } else {
            None
        };

        let mut precision = None;
        if index < chars.len() && chars[index] == '.' {
            index += 1;
            let precision_start = index;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
            if index == precision_start {
                return Err(RuntimeError::value_error(format!(
                    "unsupported format string passed to str.__format__: '{spec}'"
                )));
            }
            precision = chars[precision_start..index]
                .iter()
                .collect::<String>()
                .parse::<usize>()
                .ok();
        }

        let ty = if index < chars.len() {
            let ty = chars[index];
            index += 1;
            Some(ty)
        } else {
            None
        };
        if index != chars.len() || matches!(ty, Some(ch) if ch != 's') || matches!(align, Some('='))
        {
            return Err(RuntimeError::value_error(format!(
                "unsupported format string passed to str.__format__: '{spec}'"
            )));
        }

        let mut rendered = match precision {
            Some(limit) => text.chars().take(limit).collect::<String>(),
            None => text.to_string(),
        };
        if let Some(target_width) = width {
            let current = rendered.chars().count();
            if target_width > current {
                let padding = target_width - current;
                let align = align.unwrap_or('<');
                let left_padding = match align {
                    '<' => 0,
                    '>' => padding,
                    '^' => padding / 2,
                    _ => {
                        return Err(RuntimeError::value_error(format!(
                            "unsupported format string passed to str.__format__: '{spec}'"
                        )));
                    }
                };
                let right_padding = padding.saturating_sub(left_padding);
                let mut out = String::new();
                out.push_str(&fill.to_string().repeat(left_padding));
                out.push_str(&rendered);
                out.push_str(&fill.to_string().repeat(right_padding));
                rendered = out;
            }
        }
        Ok(rendered)
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
                _ => return Err(RuntimeError::type_error("format() argument 2 must be str")),
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
                if spec.is_empty() {
                    if *flag {
                        "True".to_string()
                    } else {
                        "False".to_string()
                    }
                } else {
                    let int_value = if *flag { 1 } else { 0 };
                    self.format_bigint_with_spec(&BigInt::from_i64(int_value), &spec)?
                }
            }
            Value::BigInt(number) => self.format_bigint_with_spec(number, &spec)?,
            Value::Str(text) => self.format_str_with_spec(text, &spec)?,
            Value::Float(number) => {
                if spec.is_empty() {
                    format_value(&value)
                } else {
                    let parsed = self.parse_numeric_format_spec(&spec)?;
                    let ty = parsed.ty.unwrap_or('g');
                    if !matches!(ty, 'e' | 'E' | 'f' | 'F' | 'g' | 'G' | 'n' | '%') {
                        return Err(RuntimeError::value_error(format!(
                            "Unknown format code '{ty}' for object of type 'float'"
                        )));
                    }
                    self.format_float_with_spec(*number, &parsed, ty)?
                }
            }
            Value::Builtin(builtin) if self.builtin_is_type_object(*builtin) => {
                if spec.is_empty() {
                    format_value(&value)
                } else {
                    return Err(RuntimeError::type_error(format!(
                        "unsupported format string passed to type.__format__: '{spec}'"
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
                    if spec.is_empty() {
                        return Ok(Value::Str(format_value(&value)));
                    }
                    let type_name = self.runtime_format_type_name(&value);
                    return Err(RuntimeError::type_error(format!(
                        "unsupported format string passed to {type_name}.__format__"
                    )));
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
        if let Some(ordering) = self.compare_cmp_to_key_wrappers(left, right)? {
            return Ok(ordering);
        }
        match compare_order(left.clone(), right.clone()) {
            Ok(ordering) => Ok(ordering),
            Err(_) => {
                if let Some(ordering) = self.compare_sequence_order_for_values(left, right)? {
                    return Ok(ordering);
                }
                self.compare_order_via_richcmp(left.clone(), right.clone())?
                    .ok_or_else(|| {
                        RuntimeError::type_error("unsupported operand type for comparison")
                    })
            }
        }
    }

    fn sequence_order_value(&mut self, value: &Value) -> Option<Value> {
        match value {
            Value::Tuple(_) | Value::List(_) => Some(value.clone()),
            Value::Instance(instance) => {
                if let Some(values) = self.namedtuple_instance_values(instance) {
                    return Some(self.heap.alloc_tuple(values));
                }
                if let Some(tuple) = self.instance_backing_tuple(instance) {
                    return Some(Value::Tuple(tuple));
                }
                if let Some(list) = self.instance_backing_list(instance) {
                    return Some(Value::List(list));
                }
                None
            }
            _ => None,
        }
    }

    pub(super) fn compare_sequence_order_for_values(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Option<Ordering>, RuntimeError> {
        let Some(left_sequence) = self.sequence_order_value(left) else {
            return Ok(None);
        };
        let Some(right_sequence) = self.sequence_order_value(right) else {
            return Ok(None);
        };
        match (&left_sequence, &right_sequence) {
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
                    return Err(RuntimeError::type_error(
                        "unsupported operand type for comparison",
                    ));
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
                        return Err(RuntimeError::type_error(
                            "unsupported operand type for comparison",
                        ));
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
        if self.host.env_var_os("PYRS_TRACE_COMPARE_ORDER").is_some() {
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
        let mut saw_order_method = false;
        if let Some(result) =
            self.call_compare_method_bool(left.clone(), "__lt__", right.clone())?
        {
            saw_order_method = true;
            if result {
                return Ok(Some(Ordering::Less));
            }
        }
        if let Some(result) =
            self.call_compare_method_bool(left.clone(), "__gt__", right.clone())?
        {
            saw_order_method = true;
            if result {
                return Ok(Some(Ordering::Greater));
            }
        }
        if let Some(result) =
            self.call_compare_method_bool(right.clone(), "__gt__", left.clone())?
        {
            saw_order_method = true;
            if result {
                return Ok(Some(Ordering::Less));
            }
        }
        if let Some(result) =
            self.call_compare_method_bool(right.clone(), "__lt__", left.clone())?
        {
            saw_order_method = true;
            if result {
                return Ok(Some(Ordering::Greater));
            }
        }
        if saw_order_method {
            return Ok(Some(Ordering::Equal));
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
            Err(err) if Self::is_unsupported_comparison_type_error(&err) => match self
                .compare_order_with_fallback(left.clone(), right.clone())
            {
                Ok(ordering) => Ok(Value::Bool(ordering == Ordering::Less)),
                Err(fallback_err) if Self::is_unsupported_comparison_type_error(&fallback_err) => {
                    Err(self.unsupported_comparison_between_instances_error("<", &left, &right))
                }
                Err(fallback_err) => Err(fallback_err),
            },
            Err(err) => Err(err),
        }
    }

    fn is_unsupported_comparison_type_error(err: &RuntimeError) -> bool {
        runtime_error_matches_exception(err, "TypeError")
            && err
                .message
                .contains("unsupported operand type for comparison")
    }

    fn unsupported_comparison_between_instances_error(
        &self,
        operator: &str,
        left: &Value,
        right: &Value,
    ) -> RuntimeError {
        RuntimeError::type_error(format!(
            "'{operator}' not supported between instances of '{}' and '{}'",
            self.value_type_name_for_error(left),
            self.value_type_name_for_error(right)
        ))
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
        if let Value::List(obj) = &container
            && let Object::List(values) = &*obj.kind()
        {
            for item in values {
                let equals = self.compare_eq_runtime(item.clone(), needle.clone())?;
                if self.truthy_from_value(&equals)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if let Value::Tuple(obj) = &container
            && let Object::Tuple(values) = &*obj.kind()
        {
            for item in values {
                let equals = self.compare_eq_runtime(item.clone(), needle.clone())?;
                if self.truthy_from_value(&equals)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if let Value::Dict(dict) = &container {
            return self.dict_contains_key_checked_runtime(dict, &needle);
        }
        if let Value::Instance(instance) = &container
            && let Some(backing_dict) = self.instance_backing_dict(instance)
        {
            return self.dict_contains_key_checked_runtime(&backing_dict, &needle);
        }
        if let Value::Instance(instance) = &container
            && let Some(backing_list) = self.instance_backing_list(instance)
            && let Object::List(values) = &*backing_list.kind()
        {
            for item in values {
                let equals = self.compare_eq_runtime(item.clone(), needle.clone())?;
                if self.truthy_from_value(&equals)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if let Value::Instance(instance) = &container
            && let Some(backing_tuple) = self.instance_backing_tuple(instance)
            && let Object::Tuple(values) = &*backing_tuple.kind()
        {
            for item in values {
                let equals = self.compare_eq_runtime(item.clone(), needle.clone())?;
                if self.truthy_from_value(&equals)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        match compare_in(&needle, &container) {
            Ok(found) => Ok(found),
            Err(err) if runtime_error_matches_exception(&err, "TypeError") => {
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
        self.call_special_method_with_fallback(
            receiver,
            method_name,
            vec![arg],
            "binary operator special method raised",
        )
    }

    pub(super) fn call_unary_special_method(
        &mut self,
        receiver: &Value,
        method_name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        self.call_special_method_with_fallback(
            receiver,
            method_name,
            Vec::new(),
            "unary operator special method raised",
        )
    }

    fn call_special_method_with_fallback(
        &mut self,
        receiver: &Value,
        method_name: &str,
        mut args: Vec<Value>,
        error_context: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some(class_ref) = self.class_of_value(receiver) else {
            return Ok(None);
        };
        let Some(method) = class_attr_lookup(&class_ref, method_name) else {
            return Ok(None);
        };
        let callable = if let Some(bound) = self.bind_descriptor_method(method.clone(), receiver)? {
            bound
        } else {
            args.insert(0, receiver.clone());
            method
        };
        match self.call_internal(callable, args, HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(Some(value)),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception(error_context))
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
        let trace = self
            .host
            .env_var_os("PYRS_TRACE_BINARY_DIV_RUNTIME")
            .is_some();
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

    pub(super) fn binary_inplace_add_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        if let Some(value) = self.call_binary_special_method(&left, "__iadd__", right.clone())?
            && !self.is_not_implemented_singleton(&value)
        {
            return Ok(value);
        }
        self.binary_add_runtime(left, right)
    }

    pub(super) fn binary_mul_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        let trace = self
            .host
            .env_var_os("PYRS_TRACE_BINARY_MUL_RUNTIME")
            .is_some();
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
        let trace = self
            .host
            .env_var_os("PYRS_TRACE_BINARY_SUB_RUNTIME")
            .is_some();
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
        let trace = self
            .host
            .env_var_os("PYRS_TRACE_BINARY_OR_RUNTIME")
            .is_some();
        match or_values(left.clone(), right.clone(), &self.heap) {
            Ok(value) => {
                if matches!(value, Value::Tuple(_))
                    && (self.union_operand_value(&left)
                        || self.union_operand_value(&right)
                        || matches!(left, Value::None)
                        || matches!(right, Value::None))
                {
                    return self.build_union_value_from_pair(left, right);
                }
                Ok(value)
            }
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
                match self.build_union_value_from_pair(left.clone(), right.clone()) {
                    Ok(value) => return Ok(value),
                    Err(union_err)
                        if !union_err.message.contains("unsupported operand type for |") =>
                    {
                        return Err(union_err);
                    }
                    Err(_) => {}
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
        let trace = self
            .host
            .env_var_os("PYRS_TRACE_BINARY_XOR_RUNTIME")
            .is_some();
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
        if let Some(ordering) = self.compare_cmp_to_key_wrappers(&left, &right)? {
            return Ok(Value::Bool(ordering == Ordering::Equal));
        }
        if let Some(result) = self.compare_eq_via_union_values(&left, &right)? {
            return Ok(Value::Bool(result));
        }
        if let Some(result) = self.compare_eq_via_paramspec_attr_values(&left, &right)? {
            return Ok(Value::Bool(result));
        }
        if let Some(result) = self.compare_eq_via_generic_alias_values(&left, &right)? {
            return Ok(Value::Bool(result));
        }
        if let (Value::Class(left_class), Value::Class(right_class)) = (&left, &right) {
            if let (Some(left_ptr), Some(right_ptr)) = (
                Self::cpython_proxy_raw_ptr_from_value(&left),
                Self::cpython_proxy_raw_ptr_from_value(&right),
            ) {
                return Ok(Value::Bool(left_ptr == right_ptr));
            }
            if Self::cpython_proxy_raw_ptr_from_value(&left).is_some()
                || Self::cpython_proxy_raw_ptr_from_value(&right).is_some()
            {
                return Ok(Value::Bool(false));
            }
            if left_class.id() == right_class.id() {
                return Ok(Value::Bool(true));
            }
            if self.class_has_default_type_metaclass(left_class)
                && self.class_has_default_type_metaclass(right_class)
            {
                return Ok(Value::Bool(false));
            }
        }
        const PY_EQ: i32 = 2;
        if !matches!(left, Value::Class(_))
            && !matches!(right, Value::Class(_))
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
        if let Some(result) = self.compare_eq_via_dict_backing(&left, &right)? {
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
        if let Some(ordering) = self.compare_cmp_to_key_wrappers(&left, &right)? {
            return Ok(Value::Bool(ordering != Ordering::Equal));
        }
        if let (Value::Class(left_class), Value::Class(right_class)) = (&left, &right) {
            if let (Some(left_ptr), Some(right_ptr)) = (
                Self::cpython_proxy_raw_ptr_from_value(&left),
                Self::cpython_proxy_raw_ptr_from_value(&right),
            ) {
                return Ok(Value::Bool(left_ptr != right_ptr));
            }
            if Self::cpython_proxy_raw_ptr_from_value(&left).is_some()
                || Self::cpython_proxy_raw_ptr_from_value(&right).is_some()
            {
                return Ok(Value::Bool(true));
            }
            if left_class.id() == right_class.id() {
                return Ok(Value::Bool(false));
            }
            if self.class_has_default_type_metaclass(left_class)
                && self.class_has_default_type_metaclass(right_class)
            {
                return Ok(Value::Bool(true));
            }
        }
        const PY_NE: i32 = 3;
        if !matches!(left, Value::Class(_))
            && !matches!(right, Value::Class(_))
            && let Some(result) = self.cpython_proxy_richcmp_value(&left, &right, PY_NE)
        {
            return result;
        }
        if let Some(result) = self.compare_eq_via_bound_method(&left, &right) {
            return Ok(Value::Bool(!result));
        }
        if let Some(result) = self.compare_eq_via_int_backing(&left, &right) {
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
        if let Some(result) = self.compare_eq_via_dict_backing(&left, &right)? {
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

    fn class_has_default_type_metaclass(&self, class_ref: &ObjRef) -> bool {
        let metaclass = match &*class_ref.kind() {
            Object::Class(class_data) => class_data.metaclass.clone(),
            _ => None,
        }
        .or_else(|| self.default_type_metaclass());
        let Some(metaclass) = metaclass else {
            return true;
        };
        matches!(
            &*metaclass.kind(),
            Object::Class(class_data) if class_data.name == "type"
        )
    }

    fn compare_eq_via_union_values(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Option<bool>, RuntimeError> {
        let Some(left_args) = self.union_args_from_value(left) else {
            return Ok(None);
        };
        let Some(right_args) = self.union_args_from_value(right) else {
            return Ok(Some(false));
        };
        if left_args.len() != right_args.len() {
            return Ok(Some(false));
        }

        let mut matched = vec![false; right_args.len()];
        for left_arg in left_args {
            let mut found = false;
            for (index, right_arg) in right_args.iter().enumerate() {
                if matched[index] {
                    continue;
                }
                let eq = self.compare_eq_runtime(left_arg.clone(), right_arg.clone())?;
                if self.truthy_from_value(&eq)? {
                    matched[index] = true;
                    found = true;
                    break;
                }
            }
            if !found {
                return Ok(Some(false));
            }
        }
        Ok(Some(true))
    }

    fn paramspec_attr_origin_info(&self, value: &Value) -> Option<(&'static str, Value)> {
        let Value::Instance(instance) = value else {
            return None;
        };
        let instance_kind = instance.kind();
        let Object::Instance(instance_data) = &*instance_kind else {
            return None;
        };
        let class_kind = instance_data.class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return None;
        };
        let module_name = match class_data.attrs.get("__module__") {
            Some(Value::Str(module_name)) => module_name.as_str(),
            _ => return None,
        };
        if module_name != "typing" && module_name != "_typing" {
            return None;
        }
        let kind = match class_data.name.as_str() {
            "ParamSpecArgs" => "args",
            "ParamSpecKwargs" => "kwargs",
            _ => return None,
        };
        let origin = instance_data.attrs.get("__origin__")?.clone();
        Some((kind, origin))
    }

    fn compare_eq_via_paramspec_attr_values(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Option<bool>, RuntimeError> {
        let Some((left_kind, left_origin)) = self.paramspec_attr_origin_info(left) else {
            return Ok(None);
        };
        let Some((right_kind, right_origin)) = self.paramspec_attr_origin_info(right) else {
            return Ok(Some(false));
        };
        if left_kind != right_kind {
            return Ok(Some(false));
        }
        let origin_eq = self.compare_eq_runtime(left_origin, right_origin)?;
        Ok(Some(self.truthy_from_value(&origin_eq)?))
    }

    fn compare_eq_via_generic_alias_values(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Option<bool>, RuntimeError> {
        if let (Some(left_literal_args), Some(right_literal_args)) = (
            self.literal_alias_args_from_value(left),
            self.literal_alias_args_from_value(right),
        ) {
            return Ok(Some(self.literal_alias_args_equal_runtime(
                &left_literal_args,
                &right_literal_args,
            )?));
        }
        let Some((left_origin, left_args)) = self.generic_alias_parts_from_value(left) else {
            return Ok(None);
        };
        let Some((right_origin, right_args)) = self.generic_alias_parts_from_value(right) else {
            return Ok(Some(false));
        };
        let left_is_types_alias = self.is_exact_types_generic_alias_value(left);
        let right_is_types_alias = self.is_exact_types_generic_alias_value(right);
        if left_is_types_alias != right_is_types_alias {
            return Ok(Some(false));
        }
        if left_is_types_alias {
            let left_unpacked = match self.optional_getattr_value(left.clone(), "__unpacked__")? {
                Some(flag) => self.truthy_from_value(&flag)?,
                None => false,
            };
            let right_unpacked = match self.optional_getattr_value(right.clone(), "__unpacked__")? {
                Some(flag) => self.truthy_from_value(&flag)?,
                None => false,
            };
            if left_unpacked != right_unpacked {
                return Ok(Some(false));
            }
        }
        match (
            self.annotated_alias_metadata_from_value(left),
            self.annotated_alias_metadata_from_value(right),
        ) {
            (Some(left_metadata), Some(right_metadata)) => {
                if left_metadata.len() != right_metadata.len() {
                    return Ok(Some(false));
                }
                for (left_item, right_item) in
                    left_metadata.into_iter().zip(right_metadata.into_iter())
                {
                    let eq = self.compare_eq_runtime(left_item, right_item)?;
                    if !self.truthy_from_value(&eq)? {
                        return Ok(Some(false));
                    }
                }
            }
            (Some(_), None) | (None, Some(_)) => return Ok(Some(false)),
            (None, None) => {}
        }
        if left_args.len() != right_args.len() {
            return Ok(Some(false));
        }
        let origin_eq = self.compare_eq_runtime(left_origin, right_origin)?;
        if !self.truthy_from_value(&origin_eq)? {
            return Ok(Some(false));
        }
        for (left_arg, right_arg) in left_args.into_iter().zip(right_args.into_iter()) {
            let arg_eq = self.compare_eq_runtime(left_arg.clone(), right_arg.clone())?;
            if !self.truthy_from_value(&arg_eq)? {
                return Ok(Some(false));
            }
        }
        Ok(Some(true))
    }

    fn annotated_alias_metadata_from_value(&self, value: &Value) -> Option<Vec<Value>> {
        let Value::Instance(instance) = value else {
            return None;
        };
        let instance_kind = instance.kind();
        let Object::Instance(instance_data) = &*instance_kind else {
            return None;
        };
        let class_kind = instance_data.class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return None;
        };
        let module_name = match class_data.attrs.get("__module__") {
            Some(Value::Str(module)) => module.as_str(),
            _ => return None,
        };
        if module_name != "typing" || class_data.name != "_AnnotatedAlias" {
            return None;
        }
        let Value::Tuple(metadata_obj) = instance_data.attrs.get("__metadata__")? else {
            return None;
        };
        let Object::Tuple(items) = &*metadata_obj.kind() else {
            return None;
        };
        Some(items.clone())
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
                    let (instance_is_enum, instance_has_int_base) = match &*instance.kind() {
                        Object::Instance(instance_data) => {
                            let is_enum =
                                self.class_mro_entries(&instance_data.class)
                                    .iter()
                                    .any(|entry| {
                                        matches!(
                                            &*entry.kind(),
                                            Object::Class(class_data) if class_data.name == "Enum"
                                        )
                                    });
                            (
                                is_enum,
                                self.class_has_builtin_int_base(&instance_data.class),
                            )
                        }
                        _ => (false, false),
                    };
                    if instance_is_enum && !instance_has_int_base {
                        return None;
                    }
                    if instance_is_enum
                        && instance_has_int_base
                        && let Object::Instance(instance_data) = &*instance.kind()
                        && let Some(raw) = instance_data
                            .attrs
                            .get("_value_")
                            .or_else(|| instance_data.attrs.get("value"))
                    {
                        if let Some(value) = int_like(raw) {
                            return Some(value);
                        }
                    }
                    if let Some(backing) = self.instance_backing_int(instance) {
                        return int_like(&backing);
                    }
                    None
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

    pub(super) fn compare_eq_via_dict_backing(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Result<Option<bool>, RuntimeError> {
        let dict_like = |value: &Value| -> Option<ObjRef> {
            match value {
                Value::Dict(dict) => Some(dict.clone()),
                Value::Instance(instance) => self.instance_backing_dict(instance),
                _ => None,
            }
        };
        let Some(left_dict) = dict_like(left) else {
            return Ok(None);
        };
        let Some(right_dict) = dict_like(right) else {
            return Ok(None);
        };
        if left_dict.id() == right_dict.id() {
            return Ok(Some(true));
        }
        let (left_entries, right_entries) = match (&*left_dict.kind(), &*right_dict.kind()) {
            (Object::Dict(left_entries), Object::Dict(right_entries)) => {
                (left_entries.to_vec(), right_entries.to_vec())
            }
            _ => return Ok(Some(false)),
        };
        if left_entries.len() != right_entries.len() {
            return Ok(Some(false));
        }
        let mut matched_right = vec![false; right_entries.len()];
        for (left_key, left_value) in &left_entries {
            let mut found = None;
            for (index, (right_key, _)) in right_entries.iter().enumerate() {
                if matched_right[index] {
                    continue;
                }
                let keys_equal = self.compare_eq_runtime(left_key.clone(), right_key.clone())?;
                if self.truthy_from_value(&keys_equal)? {
                    found = Some(index);
                    break;
                }
            }
            let Some(index) = found else {
                return Ok(Some(false));
            };
            let right_value = &right_entries[index].1;
            let values_equal = self.compare_eq_runtime(left_value.clone(), right_value.clone())?;
            if !self.truthy_from_value(&values_equal)? {
                return Ok(Some(false));
            }
            matched_right[index] = true;
        }
        Ok(Some(true))
    }

    pub(super) fn compare_eq_via_set_backing(&self, left: &Value, right: &Value) -> Option<bool> {
        let set_like_values = |value: &Value| -> Option<Vec<Value>> {
            match value {
                Value::Set(set) => match &*set.kind() {
                    Object::Set(values) => Some(values.to_vec()),
                    _ => None,
                },
                Value::FrozenSet(set) => match &*set.kind() {
                    Object::FrozenSet(values) => Some(values.to_vec()),
                    _ => None,
                },
                Value::DictKeys(keys_view) => match &*keys_view.kind() {
                    Object::DictKeysView(view) => match &*view.dict.kind() {
                        Object::Dict(entries) => {
                            Some(entries.iter().map(|(key, _)| key.clone()).collect())
                        }
                        _ => None,
                    },
                    _ => None,
                },
                Value::Instance(instance) => {
                    if let Some(set) = self.instance_backing_set(instance) {
                        return match &*set.kind() {
                            Object::Set(values) => Some(values.to_vec()),
                            _ => None,
                        };
                    }
                    self.instance_backing_frozenset(instance).and_then(
                        |frozenset| match &*frozenset.kind() {
                            Object::FrozenSet(values) => Some(values.to_vec()),
                            _ => None,
                        },
                    )
                }
                _ => None,
            }
        };
        let left_values = set_like_values(left)?;
        let right_values = set_like_values(right)?;
        if left_values.len() != right_values.len() {
            return Some(false);
        }
        Some(left_values.iter().all(|left_item| {
            right_values
                .iter()
                .any(|right_item| right_item == left_item)
        }))
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
            return Err(RuntimeError::with_exception(
                "RecursionError",
                Some("maximum recursion depth exceeded in comparison".to_string()),
            ));
        }
        self.list_eq_in_progress.push((left_id, right_id));
        let result = (|| -> Result<Option<bool>, RuntimeError> {
            if left_values.len() != right_values.len() {
                return Ok(Some(false));
            }
            for (left_item, right_item) in left_values.into_iter().zip(right_values.into_iter()) {
                let item_eq = self.compare_eq_runtime(left_item, right_item)?;
                if !self.truthy_from_value(&item_eq)? {
                    return Ok(Some(false));
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
            return Err(RuntimeError::with_exception(
                "RecursionError",
                Some("maximum recursion depth exceeded in comparison".to_string()),
            ));
        }
        self.list_eq_in_progress.push((left_id, right_id));
        let result = (|| -> Result<Option<bool>, RuntimeError> {
            if left_values.len() != right_values.len() {
                return Ok(Some(false));
            }
            for (left_item, right_item) in left_values.into_iter().zip(right_values.into_iter()) {
                let item_eq = self.compare_eq_runtime(left_item, right_item)?;
                if !self.truthy_from_value(&item_eq)? {
                    return Ok(Some(false));
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
        if let Some(result) =
            self.call_compare_method_value(left.clone(), "__le__", right.clone())?
        {
            return Ok(result);
        }
        if let Some(result) =
            self.call_compare_method_value(right.clone(), "__ge__", left.clone())?
        {
            return Ok(result);
        }
        match compare_le(left.clone(), right.clone()) {
            Ok(value) => Ok(value),
            Err(err) if Self::is_unsupported_comparison_type_error(&err) => match self
                .compare_order_with_fallback(left.clone(), right.clone())
            {
                Ok(ordering) => Ok(Value::Bool(ordering != Ordering::Greater)),
                Err(fallback_err) if Self::is_unsupported_comparison_type_error(&fallback_err) => {
                    Err(self.unsupported_comparison_between_instances_error("<=", &left, &right))
                }
                Err(fallback_err) => Err(fallback_err),
            },
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
            Err(err) if Self::is_unsupported_comparison_type_error(&err) => match self
                .compare_order_with_fallback(left.clone(), right.clone())
            {
                Ok(ordering) => Ok(Value::Bool(ordering == Ordering::Greater)),
                Err(fallback_err) if Self::is_unsupported_comparison_type_error(&fallback_err) => {
                    Err(self.unsupported_comparison_between_instances_error(">", &left, &right))
                }
                Err(fallback_err) => Err(fallback_err),
            },
            Err(err) => Err(err),
        }
    }

    pub(super) fn compare_ge_runtime(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        if let Some(result) =
            self.call_compare_method_value(left.clone(), "__ge__", right.clone())?
        {
            return Ok(result);
        }
        if let Some(result) =
            self.call_compare_method_value(right.clone(), "__le__", left.clone())?
        {
            return Ok(result);
        }
        match compare_ge(left.clone(), right.clone()) {
            Ok(value) => Ok(value),
            Err(err) if Self::is_unsupported_comparison_type_error(&err) => match self
                .compare_order_with_fallback(left.clone(), right.clone())
            {
                Ok(ordering) => Ok(Value::Bool(ordering != Ordering::Less)),
                Err(fallback_err) if Self::is_unsupported_comparison_type_error(&fallback_err) => {
                    Err(self.unsupported_comparison_between_instances_error(">=", &left, &right))
                }
                Err(fallback_err) => Err(fallback_err),
            },
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
            return Err(RuntimeError::type_error("all/any expects one argument"));
        }
        let iterable = args.remove(0);
        let iter = self
            .to_iterator_value(iterable)
            .map_err(|_| RuntimeError::type_error("all/any expects iterable"))?;

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
            _ => return Err(RuntimeError::type_error("all/any expects iterable")),
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
        let reversed_list = self.heap.alloc_list(values);
        self.to_iterator_value(reversed_list)
    }

    pub(super) fn builtin_zip(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let strict = if let Some(strict_value) = kwargs.remove("strict") {
            self.truthy_from_value(&strict_value)?
        } else {
            false
        };
        if !kwargs.is_empty() {
            let mut keys = kwargs.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let key = keys.first().cloned().unwrap_or_default();
            return Err(RuntimeError::type_error(format!(
                "zip() got an unexpected keyword argument '{}'",
                key
            )));
        }
        let mut iterators = Vec::with_capacity(args.len());
        for source in args {
            iterators.push(self.to_iterator_value(source)?);
        }
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Zip {
                iterators,
                strict,
                exhausted: false,
            },
            index: 0,
        }))
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
                        || self.is_types_generic_alias_value(value)
                        || Self::cpython_proxy_raw_ptr_from_value(value)
                            .is_some_and(Self::cpython_proxy_raw_ptr_is_callable)
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn try_custom_instancecheck(
        &mut self,
        class: &ObjRef,
        value: &Value,
    ) -> Result<Option<bool>, RuntimeError> {
        if TYPE_INSTANCECHECK_BYPASS_CUSTOM.with(|depth| depth.get() > 0) {
            return Ok(None);
        }
        let Some(meta_class) = self.class_of_value(&Value::Class(class.clone())) else {
            return Ok(None);
        };
        if let Object::Class(class_data) = &*meta_class.kind()
            && class_data.name == "type"
        {
            return Ok(None);
        }
        let Some(raw_instancecheck) = class_attr_lookup(&meta_class, "__instancecheck__") else {
            return Ok(None);
        };
        if matches!(
            raw_instancecheck,
            Value::Builtin(BuiltinFunction::TypeInstanceCheck)
        ) {
            return Ok(None);
        }
        let Some(instancecheck) =
            self.lookup_bound_special_method(&Value::Class(class.clone()), "__instancecheck__")?
        else {
            return Ok(None);
        };
        match self.call_internal(instancecheck, vec![value.clone()], HashMap::new())? {
            InternalCallOutcome::Value(result) => Ok(Some(self.truthy_from_value(&result)?)),
            InternalCallOutcome::CallerExceptionHandled => Err(self
                .runtime_error_from_active_exception(
                    "isinstance() custom __instancecheck__ failed",
                )),
        }
    }

    fn try_custom_subclasscheck(
        &mut self,
        class: &ObjRef,
        candidate: &Value,
    ) -> Result<Option<bool>, RuntimeError> {
        if TYPE_SUBCLASSCHECK_BYPASS_CUSTOM.with(|depth| depth.get() > 0) {
            return Ok(None);
        }
        let Some(meta_class) = self.class_of_value(&Value::Class(class.clone())) else {
            return Ok(None);
        };
        if let Object::Class(class_data) = &*meta_class.kind()
            && class_data.name == "type"
        {
            return Ok(None);
        }
        let Some(raw_subclasscheck) = class_attr_lookup(&meta_class, "__subclasscheck__") else {
            return Ok(None);
        };
        if matches!(
            raw_subclasscheck,
            Value::Builtin(BuiltinFunction::TypeSubclassCheck)
        ) {
            return Ok(None);
        }
        let Some(subclasscheck) =
            self.lookup_bound_special_method(&Value::Class(class.clone()), "__subclasscheck__")?
        else {
            return Ok(None);
        };
        match self.call_internal(subclasscheck, vec![candidate.clone()], HashMap::new())? {
            InternalCallOutcome::Value(result) => Ok(Some(self.truthy_from_value(&result)?)),
            InternalCallOutcome::CallerExceptionHandled => Err(self
                .runtime_error_from_active_exception(
                    "issubclass() custom __subclasscheck__ failed",
                )),
        }
    }

    pub(super) fn value_is_instance_of(
        &mut self,
        value: &Value,
        classinfo: &Value,
    ) -> Result<bool, RuntimeError> {
        thread_local! {
            static ISINSTANCE_RECURSION_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        }
        struct IsInstanceDepthGuard;
        impl IsInstanceDepthGuard {
            fn enter() -> Result<Self, RuntimeError> {
                let overflow = ISINSTANCE_RECURSION_DEPTH.with(|depth| {
                    let next = depth.get().saturating_add(1);
                    depth.set(next);
                    next > 2048
                });
                if overflow {
                    ISINSTANCE_RECURSION_DEPTH
                        .with(|depth| depth.set(depth.get().saturating_sub(1)));
                    return Err(RuntimeError::runtime_error(
                        "maximum recursion depth exceeded in isinstance()",
                    ));
                }
                Ok(Self)
            }
        }
        impl Drop for IsInstanceDepthGuard {
            fn drop(&mut self) {
                ISINSTANCE_RECURSION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
            }
        }
        let _depth_guard = IsInstanceDepthGuard::enter()?;

        if let Some(items) = self.union_args_from_value(classinfo) {
            for item in items {
                if self.value_is_instance_of(value, &item)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

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
                _ => Err(RuntimeError::type_error(
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
                _ => Err(RuntimeError::type_error(
                    "isinstance() arg 2 must be a type or tuple of types",
                )),
            },
            Value::Class(expected) => {
                if let Some(custom_result) = self.try_custom_instancecheck(expected, value)? {
                    return Ok(custom_result);
                }
                if let Object::Class(class_data) = &*expected.kind() {
                    if matches!(value, Value::ExceptionType(_)) {
                        return Ok(matches!(class_data.name.as_str(), "type" | "object"));
                    }
                    let trace_seed_instance = self
                        .host
                        .env_var_os("PYRS_TRACE_ISINSTANCE_CLASS")
                        .is_some()
                        && matches!(value, Value::None)
                        && (class_data.name.contains("SeedSequence")
                            || class_data.name.contains("BitGenerator"));
                    if trace_seed_instance {
                        eprintln!(
                            "[isinstance-class] value=None class={} class_id={}",
                            class_data.name,
                            expected.id()
                        );
                    }
                    if class_data.name == "PathLike" {
                        return Ok(self.value_has_fspath_protocol(value));
                    }
                    let marker_match = match class_data.name.as_str() {
                        "NoneType" => matches!(value, Value::None),
                        "NotImplementedType" => self
                            .builtins
                            .get("NotImplemented")
                            .and_then(|singleton| match singleton {
                                Value::Instance(obj) => Some(matches!(
                                    value,
                                    Value::Instance(candidate) if candidate.id() == obj.id()
                                )),
                                _ => None,
                            })
                            .unwrap_or(false),
                        "function" => matches!(value, Value::Function(_)),
                        "method" => matches!(value, Value::BoundMethod(_)),
                        "builtin_function_or_method" => {
                            matches!(value, Value::Builtin(_))
                                || matches!(
                                    value,
                                    Value::BoundMethod(method)
                                        if !self.bound_method_is_python_method(method)
                                            && !self.bound_method_is_builtin_unbound_descriptor(method)
                                            && !self.bound_method_is_builtin_slot_wrapper(method)
                                )
                        }
                        "method_descriptor" => {
                            matches!(
                                value,
                                Value::BoundMethod(method)
                                    if self.bound_method_is_builtin_unbound_descriptor(method)
                            ) || matches!(
                                value,
                                Value::Builtin(BuiltinFunction::ListAppendDescriptor)
                            )
                        }
                        "method-wrapper" => matches!(
                            value,
                            Value::BoundMethod(method)
                                if self.bound_method_is_builtin_slot_wrapper(method)
                        ),
                        "wrapper_descriptor" => {
                            matches!(
                                value,
                                Value::Builtin(
                                    BuiltinFunction::ObjectInit
                                        | BuiltinFunction::ObjectNew
                                        | BuiltinFunction::OperatorLt
                                )
                            ) || matches!(
                                value,
                                Value::BoundMethod(method)
                                    if self.bound_method_is_builtin_unbound_slot_wrapper(method)
                            )
                        }
                        "classmethod_descriptor" => matches!(
                            value,
                            Value::Builtin(builtin)
                                if self.builtin_is_classmethod_descriptor(*builtin)
                        ),
                        "code" => matches!(value, Value::Code(_)),
                        "cell" => matches!(value, Value::Cell(_)),
                        "traceback" | "frame" | "getset_descriptor" | "member_descriptor" => {
                            matches!(
                                value,
                                Value::Instance(instance)
                                    if matches!(
                                        &*instance.kind(),
                                        Object::Instance(instance_data)
                                            if matches!(
                                                &*instance_data.class.kind(),
                                                Object::Class(instance_class)
                                                    if instance_class.name == class_data.name
                                            )
                                    )
                            )
                        }
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
                        "Awaitable" => self.value_matches_awaitable_protocol(value),
                        "Coroutine" => self.value_matches_coroutine_protocol(value),
                        "Generator" => self.value_matches_generator_protocol(value),
                        "Hashable" => self.hash_value_runtime(value).is_ok(),
                        "AsyncIterable" => self.value_supports_attr_runtime(value, "__aiter__"),
                        "AsyncIterator" => {
                            self.value_supports_attr_runtime(value, "__aiter__")
                                && self.value_supports_attr_runtime(value, "__anext__")
                        }
                        "AsyncGenerator" => {
                            self.value_supports_attr_runtime(value, "__aiter__")
                                && self.value_supports_attr_runtime(value, "__anext__")
                                && self.value_supports_attr_runtime(value, "asend")
                                && self.value_supports_attr_runtime(value, "athrow")
                                && self.value_supports_attr_runtime(value, "aclose")
                        }
                        "Iterable" => self.value_has_iter_protocol(value),
                        "Sized" => self.value_has_len_protocol(value),
                        "Container" => self.value_has_contains_protocol(value),
                        "Collection" => {
                            self.value_has_len_protocol(value)
                                && self.value_has_iter_protocol(value)
                                && self.value_has_contains_protocol(value)
                        }
                        "Sequence" => {
                            self.value_has_len_protocol(value)
                                && self.value_has_getitem_protocol(value)
                        }
                        "Callable" => self.is_callable_value(value),
                        "Mapping" => matches!(value, Value::Dict(_)),
                        "Set" => matches!(value, Value::Set(_) | Value::FrozenSet(_)),
                        "MutableSet" => matches!(value, Value::Set(_)),
                        "MutableMapping" => matches!(value, Value::Dict(_)),
                        "MutableSequence" => matches!(value, Value::List(_)),
                        "ByteString" => matches!(value, Value::Bytes(_) | Value::ByteArray(_)),
                        "SupportsInt" => self.value_supports_attr_runtime(value, "__int__"),
                        "SupportsFloat" => self.value_supports_attr_runtime(value, "__float__"),
                        "SupportsComplex" => self.value_supports_attr_runtime(value, "__complex__"),
                        "SupportsBytes" => self.value_supports_attr_runtime(value, "__bytes__"),
                        "SupportsAbs" => self.value_supports_attr_runtime(value, "__abs__"),
                        "SupportsRound" => self.value_supports_attr_runtime(value, "__round__"),
                        "SupportsIndex" => self.value_supports_attr_runtime(value, "__index__"),
                        _ => false,
                    };
                    if marker_match {
                        if trace_seed_instance {
                            eprintln!(
                                "[isinstance-class] marker-match class={} result=true",
                                class_data.name
                            );
                        }
                        return Ok(true);
                    }
                    if let Value::Exception(exception) = value {
                        return Ok(self.exception_inherits(&exception.name, &class_data.name));
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
                    Value::Module(module) => match &*module.kind() {
                        Object::Module(module_data) => {
                            if let Some(Value::Class(module_class)) =
                                module_data.globals.get("__class__")
                            {
                                return Ok(self
                                    .class_mro_entries(module_class)
                                    .iter()
                                    .any(|entry| entry.id() == expected.id()));
                            }
                            Ok(false)
                        }
                        _ => Ok(false),
                    },
                    Value::Class(class) => {
                        let Some(meta_class) = self.class_of_value(&Value::Class(class.clone()))
                        else {
                            return Ok(false);
                        };
                        self.class_value_is_subclass_of(
                            &Value::Class(meta_class),
                            &Value::Class(expected.clone()),
                        )
                    }
                    _ => Ok(false),
                }
            }
            Value::Builtin(builtin) => Ok(self.matches_builtin_type_marker(value, *builtin)),
            Value::ExceptionType(name) => match value {
                Value::Exception(exception) => Ok(self.exception_inherits(&exception.name, name)),
                Value::Instance(instance) => match &*instance.kind() {
                    Object::Instance(instance_data) => match &*instance_data.class.kind() {
                        Object::Class(class_data) => {
                            Ok(self.exception_inherits(&class_data.name, name))
                        }
                        _ => Ok(false),
                    },
                    _ => Ok(false),
                },
                Value::ExceptionType(_) => Ok(false),
                _ => Ok(false),
            },
            other => {
                if let Some(instancecheck) =
                    self.lookup_bound_special_method(other, "__instancecheck__")?
                {
                    return match self.call_internal(
                        instancecheck,
                        vec![value.clone()],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(result) => Ok(self.truthy_from_value(&result)?),
                        InternalCallOutcome::CallerExceptionHandled => Err(self
                            .runtime_error_from_active_exception(
                                "isinstance() custom __instancecheck__ failed",
                            )),
                    };
                }
                Err(RuntimeError::type_error(
                    "isinstance() arg 2 must be a type or tuple of types",
                ))
            }
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

    fn value_supports_attr_runtime(&mut self, value: &Value, attr_name: &str) -> bool {
        self.optional_getattr_value(value.clone(), attr_name)
            .ok()
            .flatten()
            .is_some()
    }

    pub(super) fn value_matches_awaitable_protocol(&mut self, value: &Value) -> bool {
        self.value_supports_attr_runtime(value, "__await__")
    }

    pub(super) fn value_matches_coroutine_protocol(&mut self, value: &Value) -> bool {
        self.value_supports_attr_runtime(value, "__await__")
            && self.value_supports_attr_runtime(value, "send")
            && self.value_supports_attr_runtime(value, "throw")
            && self.value_supports_attr_runtime(value, "close")
    }

    pub(super) fn value_matches_generator_protocol(&mut self, value: &Value) -> bool {
        self.value_supports_attr_runtime(value, "__iter__")
            && self.value_supports_attr_runtime(value, "__next__")
            && self.value_supports_attr_runtime(value, "send")
            && self.value_supports_attr_runtime(value, "throw")
            && self.value_supports_attr_runtime(value, "close")
    }

    pub(super) fn value_has_iter_protocol(&self, value: &Value) -> bool {
        match value {
            Value::List(_)
            | Value::Tuple(_)
            | Value::Dict(_)
            | Value::DictKeys(_)
            | Value::Set(_)
            | Value::FrozenSet(_)
            | Value::Bytes(_)
            | Value::ByteArray(_)
            | Value::MemoryView(_)
            | Value::Iterator(_)
            | Value::Generator(_)
            | Value::Str(_) => true,
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => {
                    class_attr_lookup(&instance_data.class, "__iter__").is_some()
                }
                _ => false,
            },
            _ => Self::cpython_proxy_has_iternext(value).unwrap_or(false),
        }
    }

    pub(super) fn value_has_len_protocol(&self, value: &Value) -> bool {
        match value {
            Value::List(_)
            | Value::Tuple(_)
            | Value::Dict(_)
            | Value::DictKeys(_)
            | Value::Set(_)
            | Value::FrozenSet(_)
            | Value::Bytes(_)
            | Value::ByteArray(_)
            | Value::MemoryView(_)
            | Value::Str(_) => true,
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => {
                    class_attr_lookup(&instance_data.class, "__len__").is_some()
                }
                _ => false,
            },
            _ => false,
        }
    }

    pub(super) fn value_has_getitem_protocol(&self, value: &Value) -> bool {
        match value {
            Value::List(_)
            | Value::Tuple(_)
            | Value::Bytes(_)
            | Value::ByteArray(_)
            | Value::MemoryView(_)
            | Value::Str(_) => true,
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => {
                    class_attr_lookup(&instance_data.class, "__getitem__").is_some()
                }
                _ => false,
            },
            _ => false,
        }
    }

    pub(super) fn value_has_contains_protocol(&self, value: &Value) -> bool {
        match value {
            Value::List(_)
            | Value::Tuple(_)
            | Value::Dict(_)
            | Value::DictKeys(_)
            | Value::Set(_)
            | Value::FrozenSet(_)
            | Value::Bytes(_)
            | Value::ByteArray(_)
            | Value::MemoryView(_)
            | Value::Str(_) => true,
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => {
                    class_attr_lookup(&instance_data.class, "__contains__").is_some()
                }
                _ => false,
            },
            _ => false,
        }
    }

    pub(super) fn class_value_is_subclass_of(
        &mut self,
        candidate: &Value,
        classinfo: &Value,
    ) -> Result<bool, RuntimeError> {
        thread_local! {
            static ISSUBCLASS_RECURSION_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        }
        struct IsSubclassDepthGuard;
        impl IsSubclassDepthGuard {
            fn enter() -> Result<Self, RuntimeError> {
                let overflow = ISSUBCLASS_RECURSION_DEPTH.with(|depth| {
                    let next = depth.get().saturating_add(1);
                    depth.set(next);
                    next > 2048
                });
                if overflow {
                    ISSUBCLASS_RECURSION_DEPTH
                        .with(|depth| depth.set(depth.get().saturating_sub(1)));
                    return Err(RuntimeError::runtime_error(
                        "maximum recursion depth exceeded in issubclass()",
                    ));
                }
                Ok(Self)
            }
        }
        impl Drop for IsSubclassDepthGuard {
            fn drop(&mut self) {
                ISSUBCLASS_RECURSION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
            }
        }
        let _depth_guard = IsSubclassDepthGuard::enter()?;

        if let Some(items) = self.union_args_from_value(classinfo) {
            for item in items {
                if self.class_value_is_subclass_of(candidate, &item)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

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
                _ => Err(RuntimeError::type_error(
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
                _ => Err(RuntimeError::type_error(
                    "issubclass() arg 2 must be a type or tuple of types",
                )),
            },
            Value::Class(expected) => {
                let custom_candidate = match candidate {
                    Value::Builtin(builtin)
                        if matches!(
                            builtin,
                            BuiltinFunction::Bool
                                | BuiltinFunction::Int
                                | BuiltinFunction::Float
                                | BuiltinFunction::Complex
                                | BuiltinFunction::Str
                                | BuiltinFunction::List
                                | BuiltinFunction::Tuple
                                | BuiltinFunction::Dict
                                | BuiltinFunction::Set
                                | BuiltinFunction::FrozenSet
                                | BuiltinFunction::Bytes
                                | BuiltinFunction::ByteArray
                                | BuiltinFunction::MemoryView
                        ) =>
                    {
                        Value::Class(self.class_from_base_value(Value::Builtin(*builtin))?)
                    }
                    Value::Builtin(_) => candidate.clone(),
                    _ => candidate.clone(),
                };
                let candidate_allows_custom_subclasscheck = matches!(
                    &custom_candidate,
                    Value::Class(_)
                        | Value::Builtin(_)
                        | Value::Exception(_)
                        | Value::ExceptionType(_)
                ) || matches!(&custom_candidate, Value::Str(name) if is_runtime_type_name_marker(name));
                if candidate_allows_custom_subclasscheck
                    && let Some(custom_result) =
                    self.try_custom_subclasscheck(expected, &custom_candidate)?
                {
                    return Ok(custom_result);
                }
                match candidate {
                Value::Class(class) => {
                    if let Object::Class(expected_data) = &*expected.kind() {
                        if expected_data.name == "PathLike"
                            && class_attr_lookup(class, "__fspath__").is_some()
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "Awaitable"
                            && class_attr_lookup(class, "__await__").is_some()
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "Coroutine"
                            && class_attr_lookup(class, "__await__").is_some()
                            && class_attr_lookup(class, "send").is_some()
                            && class_attr_lookup(class, "throw").is_some()
                            && class_attr_lookup(class, "close").is_some()
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "Generator"
                            && class_attr_lookup(class, "__iter__").is_some()
                            && class_attr_lookup(class, "__next__").is_some()
                            && class_attr_lookup(class, "send").is_some()
                            && class_attr_lookup(class, "throw").is_some()
                            && class_attr_lookup(class, "close").is_some()
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "AsyncIterable"
                            && class_attr_lookup(class, "__aiter__").is_some()
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "AsyncIterator"
                            && class_attr_lookup(class, "__aiter__").is_some()
                            && class_attr_lookup(class, "__anext__").is_some()
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "AsyncGenerator"
                            && class_attr_lookup(class, "__aiter__").is_some()
                            && class_attr_lookup(class, "__anext__").is_some()
                            && class_attr_lookup(class, "asend").is_some()
                            && class_attr_lookup(class, "athrow").is_some()
                            && class_attr_lookup(class, "aclose").is_some()
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "Sized"
                            && (class_attr_lookup(class, "__len__").is_some()
                                || self.class_has_builtin_list_base(class)
                                || self.class_has_builtin_tuple_base(class)
                                || self.class_has_builtin_dict_base(class)
                                || self.class_has_builtin_set_base(class)
                                || self.class_has_builtin_frozenset_base(class)
                                || self.class_has_builtin_str_base(class)
                                || self.class_has_builtin_bytes_base(class)
                                || self.class_has_builtin_bytearray_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "Hashable" {
                            let explicit_hash = match &*class.kind() {
                                Object::Class(class_data) => {
                                    class_data.attrs.get("__hash__").cloned()
                                }
                                _ => None,
                            };
                            if let Some(hash_attr) = explicit_hash {
                                return Ok(!matches!(hash_attr, Value::None));
                            }
                            let inherits_builtin_unhashable_base =
                                self.class_has_builtin_list_base(class)
                                    || self.class_has_builtin_dict_base(class)
                                    || self.class_has_builtin_defaultdict_base(class)
                                    || self.class_has_builtin_ordereddict_base(class)
                                    || self.class_has_builtin_set_base(class)
                                    || self.class_has_builtin_bytearray_base(class);
                            if inherits_builtin_unhashable_base {
                                return Ok(false);
                            }
                            if matches!(class_attr_lookup(class, "__hash__"), Some(value) if !matches!(value, Value::None))
                            {
                                return Ok(true);
                            }
                        }
                        if expected_data.name == "Sequence"
                            && ((class_attr_lookup(class, "__len__").is_some()
                                && class_attr_lookup(class, "__getitem__").is_some())
                                || self.class_has_builtin_list_base(class)
                                || self.class_has_builtin_tuple_base(class)
                                || self.class_has_builtin_str_base(class)
                                || self.class_has_builtin_bytes_base(class)
                                || self.class_has_builtin_bytearray_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "Callable" {
                            if class_attr_lookup(class, "__call__").is_some() {
                                return Ok(true);
                            }
                            let class_is_builtin_callable_type = matches!(
                                &*class.kind(),
                                Object::Class(class_data)
                                    if matches!(
                                        class_data.name.as_str(),
                                        "function"
                                            | "builtin_function_or_method"
                                            | "method_descriptor"
                                            | "method-wrapper"
                                    ) && matches!(
                                        class_data.attrs.get("__module__"),
                                        Some(Value::Str(module_name))
                                            if matches!(module_name.as_str(), "types" | "_types")
                                    )
                            );
                            if class_is_builtin_callable_type {
                                return Ok(true);
                            }
                        }
                        if expected_data.name == "Container"
                            && (class_attr_lookup(class, "__contains__").is_some()
                                || self.class_has_builtin_list_base(class)
                                || self.class_has_builtin_tuple_base(class)
                                || self.class_has_builtin_dict_base(class)
                                || self.class_has_builtin_set_base(class)
                                || self.class_has_builtin_frozenset_base(class)
                                || self.class_has_builtin_str_base(class)
                                || self.class_has_builtin_bytes_base(class)
                                || self.class_has_builtin_bytearray_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "Collection"
                            && ((class_attr_lookup(class, "__len__").is_some()
                                && class_attr_lookup(class, "__iter__").is_some()
                                && class_attr_lookup(class, "__contains__").is_some())
                                || self.class_has_builtin_list_base(class)
                                || self.class_has_builtin_tuple_base(class)
                                || self.class_has_builtin_dict_base(class)
                                || self.class_has_builtin_set_base(class)
                                || self.class_has_builtin_frozenset_base(class)
                                || self.class_has_builtin_str_base(class)
                                || self.class_has_builtin_bytes_base(class)
                                || self.class_has_builtin_bytearray_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "Set"
                            && (self.class_has_builtin_set_base(class)
                                || self.class_has_builtin_frozenset_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "MutableSet"
                            && self.class_has_builtin_set_base(class)
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "Mapping"
                            && self.class_has_builtin_dict_base(class)
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "MutableMapping"
                            && self.class_has_builtin_dict_base(class)
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "MutableSequence"
                            && self.class_has_builtin_list_base(class)
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "ByteString"
                            && (self.class_has_builtin_bytes_base(class)
                                || self.class_has_builtin_bytearray_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "SupportsInt"
                            && (class_attr_lookup(class, "__int__").is_some()
                                || self.class_has_builtin_int_base(class)
                                || self.class_has_builtin_float_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "SupportsFloat"
                            && (class_attr_lookup(class, "__float__").is_some()
                                || self.class_has_builtin_int_base(class)
                                || self.class_has_builtin_float_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "SupportsComplex"
                            && (class_attr_lookup(class, "__complex__").is_some()
                                || self.class_has_builtin_int_base(class)
                                || self.class_has_builtin_float_base(class)
                                || self.class_has_builtin_complex_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "SupportsBytes"
                            && (class_attr_lookup(class, "__bytes__").is_some()
                                || self.class_has_builtin_bytes_base(class)
                                || self.class_has_builtin_bytearray_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "SupportsAbs"
                            && (class_attr_lookup(class, "__abs__").is_some()
                                || self.class_has_builtin_int_base(class)
                                || self.class_has_builtin_float_base(class)
                                || self.class_has_builtin_complex_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "SupportsRound"
                            && (class_attr_lookup(class, "__round__").is_some()
                                || self.class_has_builtin_int_base(class)
                                || self.class_has_builtin_float_base(class))
                        {
                            return Ok(true);
                        }
                        if expected_data.name == "SupportsIndex"
                            && (class_attr_lookup(class, "__index__").is_some()
                                || self.class_has_builtin_int_base(class))
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
                        "Awaitable" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::CoroutineType)
                        ),
                        "Coroutine" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::CoroutineType)
                        ),
                        "Generator" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::GeneratorType)
                        ),
                        "Hashable" => !matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::List
                                    | BuiltinFunction::Dict
                                    | BuiltinFunction::Set
                                    | BuiltinFunction::ByteArray
                                    | BuiltinFunction::CollectionsOrderedDict
                                    | BuiltinFunction::CollectionsDefaultDict
                            )
                        ),
                        "AsyncIterable" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::AsyncGeneratorType)
                        ),
                        "AsyncIterator" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::AsyncGeneratorType)
                        ),
                        "AsyncGenerator" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::AsyncGeneratorType)
                        ),
                        "Iterable" => matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::List
                                    | BuiltinFunction::Tuple
                                    | BuiltinFunction::Set
                                    | BuiltinFunction::FrozenSet
                                    | BuiltinFunction::Dict
                                    | BuiltinFunction::Str
                                    | BuiltinFunction::Bytes
                                    | BuiltinFunction::ByteArray
                            )
                        ),
                        "Container" => matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::List
                                    | BuiltinFunction::Tuple
                                    | BuiltinFunction::Set
                                    | BuiltinFunction::FrozenSet
                                    | BuiltinFunction::Dict
                                    | BuiltinFunction::Str
                                    | BuiltinFunction::Bytes
                                    | BuiltinFunction::ByteArray
                            )
                        ),
                        "Collection" => matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::List
                                    | BuiltinFunction::Tuple
                                    | BuiltinFunction::Set
                                    | BuiltinFunction::FrozenSet
                                    | BuiltinFunction::Dict
                                    | BuiltinFunction::Str
                                    | BuiltinFunction::Bytes
                                    | BuiltinFunction::ByteArray
                            )
                        ),
                        "Sized" => matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::List
                                    | BuiltinFunction::Tuple
                                    | BuiltinFunction::Dict
                                    | BuiltinFunction::Set
                                    | BuiltinFunction::FrozenSet
                                    | BuiltinFunction::Str
                                    | BuiltinFunction::Bytes
                                    | BuiltinFunction::ByteArray
                                    | BuiltinFunction::MemoryView
                            )
                        ),
                        "Sequence" => matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::List
                                    | BuiltinFunction::Tuple
                                    | BuiltinFunction::Str
                                    | BuiltinFunction::Bytes
                                    | BuiltinFunction::ByteArray
                                    | BuiltinFunction::MemoryView
                            )
                        ),
                        "Callable" => {
                            matches!(candidate, Value::Builtin(BuiltinFunction::TypesFunctionType))
                        }
                        "Set" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::Set | BuiltinFunction::FrozenSet)
                        ),
                        "MutableSet" => matches!(candidate, Value::Builtin(BuiltinFunction::Set)),
                        "Mapping" => {
                            matches!(candidate, Value::Builtin(BuiltinFunction::Dict))
                        }
                        "MutableMapping" => {
                            matches!(candidate, Value::Builtin(BuiltinFunction::Dict))
                        }
                        "MutableSequence" => {
                            matches!(candidate, Value::Builtin(BuiltinFunction::List))
                        }
                        "ByteString" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::Bytes | BuiltinFunction::ByteArray)
                        ),
                        "SupportsInt" => matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::Int
                                    | BuiltinFunction::Bool
                                    | BuiltinFunction::Float
                            )
                        ),
                        "SupportsFloat" => matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::Int
                                    | BuiltinFunction::Bool
                                    | BuiltinFunction::Float
                            )
                        ),
                        "SupportsComplex" => matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::Int
                                    | BuiltinFunction::Bool
                                    | BuiltinFunction::Float
                                    | BuiltinFunction::Complex
                            )
                        ),
                        "SupportsBytes" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::Bytes | BuiltinFunction::ByteArray)
                        ),
                        "SupportsAbs" => matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::Int
                                    | BuiltinFunction::Bool
                                    | BuiltinFunction::Float
                                    | BuiltinFunction::Complex
                            )
                        ),
                        "SupportsRound" => matches!(
                            candidate,
                            Value::Builtin(
                                BuiltinFunction::Int
                                    | BuiltinFunction::Bool
                                    | BuiltinFunction::Float
                            )
                        ),
                        "SupportsIndex" => matches!(
                            candidate,
                            Value::Builtin(BuiltinFunction::Int | BuiltinFunction::Bool)
                        ),
                        _ => false,
                    })
                }
                _ => Err(RuntimeError::type_error("issubclass() arg 1 must be a class")),
            }
            }
            Value::Builtin(expected_builtin) => match candidate {
                Value::Builtin(candidate_builtin) => Ok(
                    candidate_builtin == expected_builtin
                        || matches!(
                            (candidate_builtin, expected_builtin),
                            (BuiltinFunction::Bool, BuiltinFunction::Int)
                        ),
                ),
                Value::Str(name) => {
                    if matches!(expected_builtin, BuiltinFunction::Type)
                        && is_runtime_type_name_marker(name)
                    {
                        Ok(false)
                    } else {
                        Err(RuntimeError::type_error("issubclass() arg 1 must be a class"))
                    }
                }
                Value::Class(class) => Ok(match expected_builtin {
                    BuiltinFunction::Type => self.class_has_builtin_type_base(class),
                    BuiltinFunction::Bool => {
                        self.class_mro_entries(class).iter().any(|entry| {
                            matches!(&*entry.kind(), Object::Class(class_data) if class_data.name == "bool")
                        })
                    }
                    BuiltinFunction::Int => self.class_has_builtin_int_base(class),
                    BuiltinFunction::Float => self.class_has_builtin_float_base(class),
                    BuiltinFunction::Complex => self.class_has_builtin_complex_base(class),
                    BuiltinFunction::Str => self.class_has_builtin_str_base(class),
                    BuiltinFunction::List => self.class_has_builtin_list_base(class),
                    BuiltinFunction::Tuple => self.class_has_builtin_tuple_base(class),
                    BuiltinFunction::Dict => self.class_has_builtin_dict_base(class),
                    BuiltinFunction::CollectionsDefaultDict => {
                        self.class_has_builtin_defaultdict_base(class)
                    }
                    BuiltinFunction::CollectionsOrderedDict => {
                        self.class_has_builtin_ordereddict_base(class)
                    }
                    BuiltinFunction::Set => self.class_has_builtin_set_base(class),
                    BuiltinFunction::FrozenSet => self.class_has_builtin_frozenset_base(class),
                    BuiltinFunction::Bytes => self.class_has_builtin_bytes_base(class),
                    BuiltinFunction::ByteArray => self.class_has_builtin_bytearray_base(class),
                    BuiltinFunction::TypingTypeVar
                    | BuiltinFunction::TypingParamSpec
                    | BuiltinFunction::TypingTypeVarTuple => {
                        let expected_name = match expected_builtin {
                            BuiltinFunction::TypingTypeVar => "TypeVar",
                            BuiltinFunction::TypingParamSpec => "ParamSpec",
                            BuiltinFunction::TypingTypeVarTuple => "TypeVarTuple",
                            _ => unreachable!(),
                        };
                        self.class_mro_entries(class).iter().any(|entry| {
                            let Object::Class(class_data) = &*entry.kind() else {
                                return false;
                            };
                            class_data.name == expected_name
                                && matches!(
                                    class_data.attrs.get("__module__"),
                                    Some(Value::Str(module))
                                        if matches!(module.as_str(), "typing" | "_typing")
                                )
                        })
                    }
                    _ => false,
                }),
                Value::ExceptionType(_) => Ok(false),
                _ => Err(RuntimeError::type_error("issubclass() arg 1 must be a class")),
            },
            Value::ExceptionType(expected_name) => match candidate {
                Value::ExceptionType(candidate_name) => {
                    Ok(self.exception_inherits(candidate_name, expected_name))
                }
                Value::Class(class) => {
                    let Object::Class(class_data) = &*class.kind() else {
                        return Ok(false);
                    };
                    Ok(self.exception_inherits(&class_data.name, expected_name))
                }
                _ => Ok(false),
            },
            other => {
                if let Some(subclasscheck) =
                    self.lookup_bound_special_method(other, "__subclasscheck__")?
                {
                    return match self.call_internal(
                        subclasscheck,
                        vec![candidate.clone()],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(result) => Ok(self.truthy_from_value(&result)?),
                        InternalCallOutcome::CallerExceptionHandled => Err(self
                            .runtime_error_from_active_exception(
                                "issubclass() custom __subclasscheck__ failed",
                            )),
                    };
                }
                Err(RuntimeError::type_error(
                    "issubclass() arg 2 must be a type or tuple of types",
                ))
            }
        }
    }

    pub(super) fn matches_builtin_type_marker(
        &self,
        value: &Value,
        builtin: BuiltinFunction,
    ) -> bool {
        match builtin {
            BuiltinFunction::Type => match value {
                Value::Class(_) | Value::ExceptionType(_) => true,
                Value::Builtin(builtin) => self.builtin_is_type_object(*builtin),
                _ => false,
            },
            BuiltinFunction::Bool => matches!(value, Value::Bool(_)),
            BuiltinFunction::Int => {
                matches!(value, Value::Int(_) | Value::BigInt(_) | Value::Bool(_))
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if self.instance_backing_int(instance).is_some()
                                || matches!(
                                    &*instance.kind(),
                                    Object::Instance(instance_data)
                                        if self.class_has_builtin_int_base(&instance_data.class)
                                )
                    )
            }
            BuiltinFunction::Float => {
                matches!(value, Value::Float(_))
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if self.instance_backing_float(instance).is_some()
                                || matches!(
                                    &*instance.kind(),
                                    Object::Instance(instance_data)
                                        if self.class_has_builtin_float_base(&instance_data.class)
                                )
                    )
            }
            BuiltinFunction::Str => {
                matches!(value, Value::Str(_))
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if self.instance_backing_str(instance).is_some()
                                || matches!(
                                    &*instance.kind(),
                                    Object::Instance(instance_data)
                                        if self.class_has_builtin_str_base(&instance_data.class)
                                )
                    )
            }
            BuiltinFunction::List => {
                matches!(value, Value::List(_))
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if self.instance_backing_list(instance).is_some()
                                || matches!(
                                    &*instance.kind(),
                                    Object::Instance(instance_data)
                                        if self.class_has_builtin_list_base(&instance_data.class)
                                )
                    )
            }
            BuiltinFunction::Tuple => {
                matches!(value, Value::Tuple(_))
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if self.instance_backing_tuple(instance).is_some()
                                || matches!(
                                    &*instance.kind(),
                                    Object::Instance(instance_data)
                                        if self.class_has_builtin_tuple_base(&instance_data.class)
                                )
                    )
            }
            BuiltinFunction::Dict => {
                matches!(value, Value::Dict(_))
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if self.instance_backing_dict(instance).is_some()
                                || matches!(
                                    &*instance.kind(),
                                    Object::Instance(instance_data)
                                        if self.class_has_builtin_dict_base(&instance_data.class)
                                )
                    )
            }
            BuiltinFunction::CollectionsDefaultDict => matches!(
                value,
                Value::Dict(obj) if self.defaultdict_factories.contains_key(&obj.id())
            ),
            BuiltinFunction::CollectionsOrderedDict => matches!(
                value,
                Value::Dict(obj) if self.ordered_dict_instances.contains(&obj.id())
            ),
            BuiltinFunction::Set => {
                matches!(value, Value::Set(_))
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if self.instance_backing_set(instance).is_some()
                                || matches!(
                                    &*instance.kind(),
                                    Object::Instance(instance_data)
                                        if self.class_has_builtin_set_base(&instance_data.class)
                                )
                    )
            }
            BuiltinFunction::FrozenSet => {
                matches!(value, Value::FrozenSet(_))
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if self.instance_backing_frozenset(instance).is_some()
                                || matches!(
                                    &*instance.kind(),
                                    Object::Instance(instance_data)
                                        if self.class_has_builtin_frozenset_base(&instance_data.class)
                                )
                    )
            }
            BuiltinFunction::Bytes => {
                matches!(value, Value::Bytes(_))
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if matches!(
                                &*instance.kind(),
                                Object::Instance(instance_data)
                                    if self.class_has_builtin_bytes_base(&instance_data.class)
                                        && matches!(
                                            instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR),
                                            Some(Value::Bytes(_))
                                        )
                            )
                    )
            }
            BuiltinFunction::ByteArray => {
                matches!(value, Value::ByteArray(_))
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if matches!(
                                &*instance.kind(),
                                Object::Instance(instance_data)
                                    if self.class_has_builtin_bytearray_base(&instance_data.class)
                                        && matches!(
                                            instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR),
                                            Some(Value::ByteArray(_))
                                        )
                            )
                    )
            }
            BuiltinFunction::MemoryView => matches!(value, Value::MemoryView(_)),
            BuiltinFunction::Complex => {
                matches!(value, Value::Complex { .. })
                    || matches!(
                        value,
                        Value::Instance(instance)
                            if self.instance_backing_complex(instance).is_some()
                                || matches!(
                                    &*instance.kind(),
                                    Object::Instance(instance_data)
                                        if self.class_has_builtin_complex_base(&instance_data.class)
                                )
                    )
            }
            BuiltinFunction::Slice => matches!(value, Value::Slice { .. }),
            BuiltinFunction::TypesModuleType => matches!(value, Value::Module(_)),
            BuiltinFunction::TypesMethodType => {
                matches!(value, Value::BoundMethod(method) if self.bound_method_is_python_method(method))
            }
            BuiltinFunction::TypingTypeVar
            | BuiltinFunction::TypingParamSpec
            | BuiltinFunction::TypingTypeVarTuple => {
                let expected_name = match builtin {
                    BuiltinFunction::TypingTypeVar => "TypeVar",
                    BuiltinFunction::TypingParamSpec => "ParamSpec",
                    BuiltinFunction::TypingTypeVarTuple => "TypeVarTuple",
                    _ => unreachable!(),
                };
                matches!(
                    value,
                    Value::Instance(instance)
                        if matches!(
                            &*instance.kind(),
                            Object::Instance(instance_data)
                                if self.class_mro_entries(&instance_data.class).iter().any(|entry| {
                                    matches!(
                                        &*entry.kind(),
                                        Object::Class(class_data)
                                            if class_data.name == expected_name
                                                && matches!(
                                                    class_data.attrs.get("__module__"),
                                                    Some(Value::Str(module))
                                                        if matches!(module.as_str(), "typing" | "_typing")
                                                )
                                    )
                                })
                        )
                )
            }
            BuiltinFunction::GeneratorType => {
                matches!(
                    value,
                    Value::Generator(generator)
                        if matches!(
                            &*generator.kind(),
                            Object::Generator(state) if !state.is_coroutine && !state.is_async_generator
                        )
                )
            }
            BuiltinFunction::CoroutineType => {
                matches!(
                    value,
                    Value::Generator(generator)
                        if matches!(
                            &*generator.kind(),
                            Object::Generator(state) if state.is_coroutine
                        )
                )
            }
            BuiltinFunction::AsyncGeneratorType => {
                matches!(
                    value,
                    Value::Generator(generator)
                        if matches!(
                            &*generator.kind(),
                            Object::Generator(state) if state.is_async_generator
                        )
                )
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
            .map_err(|_| RuntimeError::type_error("object is not iterable"))
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
                GeneratorResumeOutcome::Complete(value) => {
                    if let Some(default) = default {
                        Ok(default)
                    } else {
                        Err(self.stop_iteration_runtime_error(value))
                    }
                }
                GeneratorResumeOutcome::PropagatedException => {
                    self.propagate_pending_generator_exception()?;
                    Ok(Value::None)
                }
            },
            Value::Iterator(iterator_ref) => {
                if let Some(value) = self.iterator_next_value(&iterator_ref)? {
                    Ok(value)
                } else if let Some(default) = default {
                    Ok(default)
                } else {
                    Err(self.stop_iteration_runtime_error(Value::None))
                }
            }
            Value::Instance(_) => match self.next_from_iterator_value(&iterator)? {
                GeneratorResumeOutcome::Yield(value) => Ok(value),
                GeneratorResumeOutcome::Complete(value) => {
                    if let Some(default) = default {
                        Ok(default)
                    } else {
                        Err(self.stop_iteration_runtime_error(value))
                    }
                }
                GeneratorResumeOutcome::PropagatedException => {
                    self.propagate_pending_generator_exception()?;
                    Ok(Value::None)
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
            .map_err(|_| RuntimeError::type_error("enumerate() expects iterable"))?;
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
            if self.finalized_del_objects.contains(&target_id)
                || self.cleared_weakref_objects.contains(&target_id)
            {
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
            _ => return Err(RuntimeError::type_error("attribute name must be string")),
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
            && self
                .host
                .env_var_os("PYRS_TRACE_GETATTR_GENERATE_STATE")
                .is_some()
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
                "__new__" => {
                    let none_type = self
                        .types_module_class("NoneType")
                        .unwrap_or_else(|| self.fallback_none_type_class());
                    Ok(self.alloc_builtin_bound_method(BuiltinFunction::ObjectNew, none_type))
                }
                _ => Err(RuntimeError::attribute_error(format!(
                    "NoneType has no attribute '{}'",
                    name
                ))),
            },
            Value::List(list) => self.load_attr_list_method(list, &name),
            Value::Tuple(tuple) => self.load_attr_tuple_method(tuple, &name),
            Value::Int(value) => self.load_attr_int_method(Value::Int(value), &name),
            Value::BigInt(value) => self.load_attr_int_method(Value::BigInt(value), &name),
            Value::Bool(value) => self.load_attr_int_method(Value::Bool(value), &name),
            Value::Float(value) => self.load_attr_float_method(value, &name),
            Value::Str(text) => self.load_attr_str_method(text, &name),
            Value::Bytes(bytes) => {
                let is_bytes = matches!(&*bytes.kind(), Object::Bytes(_));
                if !is_bytes {
                    return Err(RuntimeError::attribute_error(
                        "attribute access unsupported type",
                    ));
                }
                self.load_attr_bytes_method(Value::Bytes(bytes), &name)
            }
            Value::ByteArray(bytearray) => {
                let is_bytearray = matches!(&*bytearray.kind(), Object::ByteArray(_));
                if !is_bytearray {
                    return Err(RuntimeError::attribute_error(
                        "attribute access unsupported type",
                    ));
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
                if let Some(value) = self.load_attr_generator_property(&generator, &name) {
                    return Ok(value);
                }
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
                    _ => {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    }
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
                "__notes__" => Ok(exception
                    .attrs
                    .borrow()
                    .get("__notes__")
                    .cloned()
                    .unwrap_or(Value::None)),
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
                "__traceback__" => {
                    if let Some(cached) = exception.attrs.borrow().get("__traceback__").cloned() {
                        Ok(cached)
                    } else {
                        let traceback =
                            self.traceback_value_from_frames(&exception.traceback_frames);
                        exception
                            .attrs
                            .borrow_mut()
                            .insert("__traceback__".to_string(), traceback.clone());
                        Ok(traceback)
                    }
                }
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
            _ => Err(RuntimeError::attribute_error(
                "attribute access unsupported type",
            )),
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
            _ => return Err(RuntimeError::type_error("attribute name must be string")),
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
                let metaclass_descriptor = self
                    .class_of_value(&Value::Class(class.clone()))
                    .and_then(|metaclass| class_attr_lookup(&metaclass, &name));
                if let Some(descriptor) = metaclass_descriptor {
                    let (_getter, setter, _deleter) = self.descriptor_hooks(&descriptor)?;
                    if let Some(setter) = setter {
                        match self.call_internal(
                            setter,
                            vec![Value::Class(class.clone()), value.clone()],
                            HashMap::new(),
                        )? {
                            InternalCallOutcome::Value(_) => return Ok(Value::None),
                            InternalCallOutcome::CallerExceptionHandled => return Ok(Value::None),
                        }
                    }
                    if let Value::Instance(descriptor_instance) = &descriptor
                        && let Object::Instance(instance_data) = &*descriptor_instance.kind()
                        && let Object::Class(class_data) = &*instance_data.class.kind()
                        && class_data.name == "property"
                    {
                        return Err(RuntimeError::attribute_error("readonly attribute"));
                    }
                }
                let (flags, class_name) = match &*class.kind() {
                    Object::Class(class_data) => (
                        class_data
                            .attrs
                            .get("__flags__")
                            .and_then(|value| match value {
                                Value::Int(flags) => Some(*flags),
                                _ => None,
                            }),
                        class_data.name.clone(),
                    ),
                    _ => (None, "type".to_string()),
                };
                let flags = flags.or_else(|| self.cpython_proxy_type_flags(&class));
                if let Some(flags) = flags
                    && ((flags & PY_TPFLAGS_HEAPTYPE) == 0
                        || (flags & PY_TPFLAGS_IMMUTABLETYPE) != 0)
                {
                    return Err(RuntimeError::type_error(format!(
                        "cannot set attribute '{}' of immutable type '{}'",
                        name, class_name
                    )));
                }
                if name == "__bases__" {
                    self.update_class_bases_attr(&class, value)?;
                } else if let Object::Class(class_data) = &mut *class.kind_mut() {
                    class_data.attrs.insert(name.clone(), value);
                }
                self.normalize_class_annotations_after_attr_set(&class, &name);
            }
            Value::Function(func) => self.store_attr_function(&func, name, value)?,
            Value::Cell(cell) => self.store_attr_cell(&cell, &name, value)?,
            Value::Exception(mut exception) => {
                self.store_attr_exception(&mut exception, &name, value)?
            }
            Value::Builtin(builtin) => self.store_attr_builtin(builtin, &name, value)?,
            other => {
                if self
                    .host
                    .env_var_os("PYRS_TRACE_SETATTR_UNSUPPORTED")
                    .is_some()
                {
                    eprintln!(
                        "[setattr-unsupported] target={} name={}",
                        format_repr(&other),
                        name
                    );
                }
                return Err(RuntimeError::type_error(
                    "attribute assignment unsupported type",
                ));
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
            _ => return Err(RuntimeError::type_error("attribute name must be string")),
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
                let (flags, class_name) = match &*class.kind() {
                    Object::Class(class_data) => (
                        class_data
                            .attrs
                            .get("__flags__")
                            .and_then(|value| match value {
                                Value::Int(flags) => Some(*flags),
                                _ => None,
                            }),
                        class_data.name.clone(),
                    ),
                    _ => (None, "type".to_string()),
                };
                let flags = flags.or_else(|| self.cpython_proxy_type_flags(&class));
                if let Some(flags) = flags
                    && ((flags & PY_TPFLAGS_HEAPTYPE) == 0
                        || (flags & PY_TPFLAGS_IMMUTABLETYPE) != 0)
                {
                    return Err(RuntimeError::type_error(format!(
                        "cannot delete attribute '{}' of immutable type '{}'",
                        name, class_name
                    )));
                }
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
            _ => {
                return Err(RuntimeError::type_error(
                    "attribute deletion unsupported type",
                ));
            }
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
            let frame_name = frame.code.name.clone();

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
            let no_explicit_class = class_from_locals.is_none()
                && class_from_cells.is_none()
                && class_from_owner.is_none();
            let inferred = match &object_value {
                Value::Class(class) => match &*class.kind() {
                    Object::Class(class_data) => {
                        class_data.bases.first().cloned().or(Some(class.clone()))
                    }
                    _ => Some(class.clone()),
                },
                _ => self.class_of_value(&object_value),
            };
            let inferred = if no_explicit_class {
                let is_generic_proxy = inferred.as_ref().is_some_and(|class| {
                    matches!(
                        &*class.kind(),
                        Object::Class(class_data)
                            if class_data.name == "__pyrs_cpython_proxy__"
                    )
                });
                if is_generic_proxy {
                    self.load_cpython_proxy_attr_for_value(&object_value, "__class__")
                        .and_then(|value| match value {
                            Value::Class(class) => Some(class),
                            _ => None,
                        })
                        .or(inferred)
                } else {
                    inferred
                }
            } else {
                inferred
            };
            if self.host.env_var_os("PYRS_TRACE_SUPER_DTYPE").is_some() {
                let class_name = |class: &ObjRef| match &*class.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "<non-class>".to_string(),
                };
                let loc = class_from_locals
                    .as_ref()
                    .map(class_name)
                    .unwrap_or_else(|| "<none>".to_string());
                let cell = class_from_cells
                    .as_ref()
                    .map(class_name)
                    .unwrap_or_else(|| "<none>".to_string());
                let owner = class_from_owner
                    .as_ref()
                    .map(class_name)
                    .unwrap_or_else(|| "<none>".to_string());
                let inf = inferred
                    .as_ref()
                    .map(class_name)
                    .unwrap_or_else(|| "<none>".to_string());
                eprintln!(
                    "[super-build] func={} locals={} cells={} owner={} inferred={} object_type={}",
                    frame_name,
                    loc,
                    cell,
                    owner,
                    inf,
                    self.value_type_name_for_error(&object_value)
                );
            }
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
            _ => self
                .class_of_value(&object_value)
                .ok_or_else(|| {
                    RuntimeError::new("super() second argument must be an instance or subclass")
                })
                .and_then(|class| {
                    let is_generic_proxy = matches!(
                        &*class.kind(),
                        Object::Class(class_data) if class_data.name == "__pyrs_cpython_proxy__"
                    );
                    if !is_generic_proxy {
                        return Ok(class);
                    }
                    if let Some(Value::Class(proxy_class)) =
                        self.load_cpython_proxy_attr_for_value(&object_value, "__class__")
                    {
                        return Ok(proxy_class);
                    }
                    Ok(class)
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
