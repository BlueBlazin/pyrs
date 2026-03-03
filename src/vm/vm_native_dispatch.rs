//! Native builtin/stdlib method dispatch implementations.
//!
//! This module hosts CPython-parity method bodies for builtin/native receiver
//! types and returns either a value or explicit propagated-exception markers to
//! integrate with VM-level unwind logic.

use std::cell::Cell;

use super::{
    BigInt, BoundMethod, BuiltinFunction, CodeObject, ExceptionObject, FormatterFieldKey, Frame,
    GeneratorObject, GeneratorResumeKind, GeneratorResumeOutcome, HashMap, InstanceObject,
    Instruction, InternalCallOutcome, IteratorKind, IteratorObject, MAPPING_PROXY_STORAGE_ATTR,
    ModuleObject, NativeCallResult, NativeMethodKind, ObjRef, Object, Opcode, Ordering,
    PY_TPFLAGS_DISALLOW_INSTANTIATION, Rc, ReMode, RePatternValue, RuntimeError, Value, Vm,
    bigint_to_fixed_bytes, bytes_like_from_value, call_builtin_with_kwargs, class_attr_lookup,
    class_name_for_instance, decode_text_bytes, dict_get_value, dict_remove_value, dict_set_value,
    dict_set_value_checked, encode_text_bytes, ensure_hashable, exception_is_named,
    find_bytes_subslice, format_value, is_truthy, memoryview_bounds, memoryview_decode_tolist,
    memoryview_format_for_view, memoryview_shape_and_strides_from_parts, normalize_codec_encoding,
    normalize_codec_errors, parse_memoryview_cast_format, parse_string_formatter,
    py_rsplit_whitespace, py_split_whitespace, py_splitlines, re_pattern_from_compiled_module,
    runtime_error_matches_exception, split_formatter_field_name, value_from_bigint,
    value_to_bigint, value_to_int, with_bytes_like_source,
};
use crate::ast::{Constant, ExprKind};
use crate::parser;

unsafe extern "C" {
    fn PyErr_Clear();
}

thread_local! {
    static CALL_BUILTIN_DEPTH: Cell<usize> = const { Cell::new(0) };
    static CALL_NATIVE_METHOD_DEPTH: Cell<usize> = const { Cell::new(0) };
}

const SIMPLE_QUEUE_STORAGE_ATTR: &str = "__pyrs_simple_queue_items__";

/// Parse and validate the optional `shape=` argument for `memoryview.cast`.
fn parse_memoryview_cast_shape(value: &Value) -> Result<Vec<usize>, RuntimeError> {
    let shape_items = match value {
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => values.clone(),
            _ => {
                return Err(RuntimeError::new("shape must be a list or a tuple"));
            }
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => values.clone(),
            _ => {
                return Err(RuntimeError::new("shape must be a list or a tuple"));
            }
        },
        _ => {
            return Err(RuntimeError::new("shape must be a list or a tuple"));
        }
    };
    let mut shape = Vec::with_capacity(shape_items.len());
    for item in shape_items {
        let dim = value_to_int(item).map_err(|_| {
            RuntimeError::new("memoryview.cast(): elements of shape must be integers")
        })?;
        if dim <= 0 {
            return Err(RuntimeError::new(
                "memoryview.cast(): elements of shape must be integers > 0",
            ));
        }
        shape.push(dim as usize);
    }
    Ok(shape)
}

fn shape_product_matches_buffer_len(shape: &[usize], itemsize: usize, buffer_len: usize) -> bool {
    let mut total_elems = 1usize;
    for dim in shape {
        let Some(next_total) = total_elems.checked_mul(*dim) else {
            return false;
        };
        total_elems = next_total;
    }
    let Some(expected_len) = total_elems.checked_mul(itemsize.max(1)) else {
        return false;
    };
    expected_len == buffer_len
}

fn c_contiguous_strides_for_shape(
    shape: &[usize],
    itemsize: usize,
) -> Result<Vec<isize>, RuntimeError> {
    if shape.is_empty() {
        return Ok(Vec::new());
    }
    let mut strides = vec![0isize; shape.len()];
    let mut stride = itemsize.max(1);
    for index in (0..shape.len()).rev() {
        let stride_isize = isize::try_from(stride)
            .map_err(|_| RuntimeError::new("memoryview.cast() shape is too large"))?;
        strides[index] = stride_isize;
        stride = stride
            .checked_mul(shape[index])
            .ok_or_else(|| RuntimeError::new("memoryview.cast() shape is too large"))?;
    }
    Ok(strides)
}

impl Vm {
    fn extract_float_receiver_value_for_method_call(
        &self,
        receiver: &ObjRef,
        args: &mut Vec<Value>,
        method_name: &str,
    ) -> Result<f64, RuntimeError> {
        match &*receiver.kind() {
            Object::Module(module_data) if module_data.name == "__float_method__" => {
                if !args.is_empty() {
                    return Err(RuntimeError::new(format!(
                        "{method_name}() expects no arguments"
                    )));
                }
                match module_data.globals.get("value") {
                    Some(Value::Float(value)) => Ok(*value),
                    _ => Err(RuntimeError::type_error("float receiver is invalid")),
                }
            }
            Object::Module(module_data) if module_data.name == "__float_unbound_method__" => {
                if args.is_empty() {
                    return Err(RuntimeError::type_error(format!(
                        "unbound method float.{method_name}() needs an argument"
                    )));
                }
                let receiver_value = args.remove(0);
                let Value::Float(value) = receiver_value else {
                    return Err(RuntimeError::type_error(format!(
                        "descriptor '{}' for 'float' objects doesn't apply to a '{}' object",
                        method_name,
                        self.value_type_name_for_error(&receiver_value)
                    )));
                };
                if !args.is_empty() {
                    return Err(RuntimeError::new(format!(
                        "{method_name}() expects no arguments"
                    )));
                }
                Ok(value)
            }
            _ => Err(RuntimeError::type_error("float receiver is invalid")),
        }
    }

    pub(super) fn stop_iteration_runtime_error(&mut self, value: Value) -> RuntimeError {
        let message = if matches!(value, Value::None) {
            None
        } else {
            Some(format_value(&value))
        };
        let exception = ExceptionObject::new("StopIteration", message);
        {
            let mut attrs = exception.attrs.borrow_mut();
            let args = if matches!(value, Value::None) {
                self.heap.alloc_tuple(Vec::new())
            } else {
                self.heap.alloc_tuple(vec![value.clone()])
            };
            attrs.insert("args".to_string(), args);
            attrs.insert("value".to_string(), value);
        }
        RuntimeError::from_exception(exception)
    }

    fn stop_iteration_value_from_value(&self, value: &Value) -> Option<Value> {
        if !exception_is_named(value, "StopIteration") {
            return None;
        }
        match value {
            Value::Exception(exception) => {
                let attrs = exception.attrs.borrow();
                attrs
                    .get("value")
                    .cloned()
                    .or_else(|| match attrs.get("args") {
                        Some(Value::Tuple(args_obj)) => match &*args_obj.kind() {
                            Object::Tuple(items) => items.first().cloned(),
                            _ => None,
                        },
                        _ => None,
                    })
            }
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => {
                    instance_data.attrs.get("value").cloned().or_else(|| {
                        match instance_data.attrs.get("args") {
                            Some(Value::Tuple(args_obj)) => match &*args_obj.kind() {
                                Object::Tuple(items) => items.first().cloned(),
                                _ => None,
                            },
                            _ => None,
                        }
                    })
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn stop_iteration_value_from_active_exception(&self) -> Option<Value> {
        self.frames
            .last()
            .and_then(|frame| frame.active_exception.as_ref())
            .and_then(|value| self.stop_iteration_value_from_value(value))
    }

    fn str_predicate_receiver_text(
        &self,
        receiver: &ObjRef,
        args: &mut Vec<Value>,
        method_name: &str,
    ) -> Result<String, RuntimeError> {
        let Object::Module(module_data) = &*receiver.kind() else {
            return Err(RuntimeError::type_error("str receiver is invalid"));
        };
        if let Some(Value::Str(value)) = module_data.globals.get("value") {
            if !args.is_empty() {
                return Err(RuntimeError::new(format!(
                    "{method_name}() expects no arguments"
                )));
            }
            return Ok(value.clone());
        }
        if matches!(
            module_data.globals.get("owner"),
            Some(Value::Builtin(BuiltinFunction::Str))
        ) {
            if args.len() != 1 {
                return Err(RuntimeError::new(format!(
                    "{method_name}() descriptor requires a str argument"
                )));
            }
            return match args.remove(0) {
                Value::Str(value) => Ok(value),
                Value::Instance(instance) => self
                    .instance_backing_str(&instance)
                    .ok_or_else(|| RuntimeError::type_error("str receiver is invalid")),
                _ => Err(RuntimeError::type_error("str receiver is invalid")),
            };
        }
        Err(RuntimeError::type_error("str receiver is invalid"))
    }

    /// Dispatch a native receiver method using CPython argument contracts.
    ///
    /// Return shape:
    /// - `NativeCallResult::Value` for normal completion.
    /// - `NativeCallResult::PropagatedException` when the callee has already
    ///   installed frame-local exception state and unwind should continue.
    pub(super) fn call_native_method(
        &mut self,
        kind: NativeMethodKind,
        receiver: ObjRef,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<NativeCallResult, RuntimeError> {
        let depth = CALL_NATIVE_METHOD_DEPTH.with(|depth| {
            let next = depth.get().saturating_add(1);
            depth.set(next);
            next
        });
        struct CallNativeMethodDepthGuard;
        impl Drop for CallNativeMethodDepthGuard {
            fn drop(&mut self) {
                CALL_NATIVE_METHOD_DEPTH.with(|depth| {
                    depth.set(depth.get().saturating_sub(1));
                });
            }
        }
        let _depth_guard = CallNativeMethodDepthGuard;
        let hard_limit = (self.recursion_limit.max(1) as usize).saturating_mul(4);
        if self
            .host
            .env_var_os("PYRS_TRACE_NATIVE_CALL_DEPTH")
            .is_some()
            && depth >= hard_limit.saturating_sub(16)
        {
            let receiver_name = match &*receiver.kind() {
                Object::Class(class_data) => format!("class:{}", class_data.name),
                Object::Instance(instance_data) => match &*instance_data.class.kind() {
                    Object::Class(class_data) => format!("instance:{}", class_data.name),
                    _ => "instance:<unknown>".to_string(),
                },
                other => format!("{other:?}"),
            };
            eprintln!(
                "[native-depth] depth={} limit={} kind={:?} receiver={} argc={} kwargc={}",
                depth,
                hard_limit,
                kind,
                receiver_name,
                args.len(),
                kwargs.len()
            );
        }
        if depth > hard_limit {
            return Err(self.recursion_limit_error());
        }
        if !kwargs.is_empty()
            && !matches!(
                kind,
                NativeMethodKind::FunctoolsPartialCall
                    | NativeMethodKind::ExtensionFunctionCall(_)
                    | NativeMethodKind::DictUpdateMethod
                    | NativeMethodKind::IntToBytes
                    | NativeMethodKind::StrFormat
                    | NativeMethodKind::StrSplit
                    | NativeMethodKind::StrSplitLines
                    | NativeMethodKind::StrRSplit
                    | NativeMethodKind::StrEncode
                    | NativeMethodKind::StrDecode
                    | NativeMethodKind::StrCount
                    | NativeMethodKind::StrFind
                    | NativeMethodKind::StrIndex
                    | NativeMethodKind::StrRFind
                    | NativeMethodKind::QueueSimpleQueuePut
                    | NativeMethodKind::QueueSimpleQueueGet
                    | NativeMethodKind::BytesDecode
                    | NativeMethodKind::BytesHex
                    | NativeMethodKind::BytesCount
                    | NativeMethodKind::BytesSplit
                    | NativeMethodKind::BytesTranslate
                    | NativeMethodKind::ListSort
                    | NativeMethodKind::MemoryViewCast
                    | NativeMethodKind::CodecsIncrementalEncoderFactoryCall
                    | NativeMethodKind::CodecsIncrementalDecoderFactoryCall
                    | NativeMethodKind::CodecsIncrementalEncoderEncode
                    | NativeMethodKind::CodecsIncrementalDecoderDecode
                    | NativeMethodKind::CodecsIncrementalEncoderSetState
                    | NativeMethodKind::CodecsIncrementalDecoderSetState
                    | NativeMethodKind::CodeReplace
                    | NativeMethodKind::RePatternSearch
                    | NativeMethodKind::RePatternMatch
                    | NativeMethodKind::RePatternFullMatch
                    | NativeMethodKind::FunctionAnnotate
                    | NativeMethodKind::GenericAliasCall
                    | NativeMethodKind::Builtin(_)
            )
        {
            if self
                .host
                .env_var_os("PYRS_TRACE_NATIVE_KW_REJECT")
                .is_some()
            {
                let mut kw_names = kwargs.keys().cloned().collect::<Vec<_>>();
                kw_names.sort();
                let receiver_type = match &*receiver.kind() {
                    Object::List(_) => "list",
                    Object::Tuple(_) => "tuple",
                    Object::Dict(_) => "dict",
                    Object::Set(_) => "set",
                    Object::FrozenSet(_) => "frozenset",
                    Object::Instance(_) => "instance",
                    Object::Class(_) => "class",
                    Object::Function(_) => "function",
                    Object::Module(_) => "module",
                    _ => "object",
                };
                eprintln!(
                    "[native-kw-reject] kind={kind:?} kwargs={kw_names:?} receiver_type={}",
                    receiver_type
                );
            }
            return Err(RuntimeError::new(
                "TypeError: native methods do not accept keywords",
            ));
        }
        match kind {
            NativeMethodKind::Builtin(builtin) => {
                let prepend_receiver = !matches!(
                    builtin,
                    BuiltinFunction::DictFromKeys
                        | BuiltinFunction::BytesMakeTrans
                        | BuiltinFunction::StrMakeTrans
                );
                let mut call_args =
                    Vec::with_capacity(args.len() + if prepend_receiver { 1 } else { 0 });
                if prepend_receiver {
                    call_args.push(self.bound_method_reduce_receiver_value(&receiver)?);
                }
                call_args.extend(args);
                let value = self.call_builtin(builtin, call_args, kwargs)?;
                Ok(NativeCallResult::Value(value))
            }
            NativeMethodKind::ExtensionFunctionCall(function_id) => {
                self.call_extension_callable(function_id, args, kwargs)
            }
            NativeMethodKind::GeneratorIter => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("__iter__() expects no arguments"));
                }
                Ok(NativeCallResult::Value(Value::Generator(receiver)))
            }
            NativeMethodKind::GeneratorAwait => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("__await__() expects no arguments"));
                }
                let is_coroutine = match &*receiver.kind() {
                    Object::Generator(state) => state.is_coroutine,
                    _ => false,
                };
                if is_coroutine {
                    Ok(NativeCallResult::Value(Value::Generator(receiver)))
                } else {
                    Err(RuntimeError::type_error("object is not awaitable"))
                }
            }
            NativeMethodKind::GeneratorANext => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("__anext__() expects no arguments"));
                }
                match &*receiver.kind() {
                    Object::Generator(state) if state.is_async_generator => {}
                    _ => return Err(RuntimeError::new("object is not an async generator")),
                }
                match self.resume_generator(&receiver, None, None, GeneratorResumeKind::Next)? {
                    GeneratorResumeOutcome::Yield(value) => Ok(NativeCallResult::Value(
                        self.make_immediate_coroutine(value),
                    )),
                    GeneratorResumeOutcome::Complete(_) => {
                        Err(RuntimeError::new("StopAsyncIteration"))
                    }
                    GeneratorResumeOutcome::PropagatedException => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::GeneratorNext => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("__next__() expects no arguments"));
                }
                match self.resume_generator(&receiver, None, None, GeneratorResumeKind::Next)? {
                    GeneratorResumeOutcome::Yield(value) => Ok(NativeCallResult::Value(value)),
                    GeneratorResumeOutcome::Complete(value) => {
                        Err(self.stop_iteration_runtime_error(value))
                    }
                    GeneratorResumeOutcome::PropagatedException => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::GeneratorSend => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("send() expects one argument"));
                }
                let sent = args.into_iter().next();
                match self.resume_generator(&receiver, sent, None, GeneratorResumeKind::Next)? {
                    GeneratorResumeOutcome::Yield(value) => Ok(NativeCallResult::Value(value)),
                    GeneratorResumeOutcome::Complete(value) => {
                        Err(self.stop_iteration_runtime_error(value))
                    }
                    GeneratorResumeOutcome::PropagatedException => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::GeneratorThrow => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("throw() expects 1-2 arguments"));
                }
                let exc = args.into_iter().next().expect("checked len");
                let exc = match exc {
                    Value::Exception(_) | Value::ExceptionType(_) => exc,
                    Value::Class(class) if self.class_is_exception_class(&class) => {
                        Value::Class(class)
                    }
                    Value::Instance(instance)
                        if self.exception_class_name_for_instance(&instance).is_some() =>
                    {
                        Value::Instance(instance)
                    }
                    _ => return Err(RuntimeError::new("throw() expects an exception type/value")),
                };
                match self.resume_generator(
                    &receiver,
                    None,
                    Some(exc),
                    GeneratorResumeKind::Throw,
                )? {
                    GeneratorResumeOutcome::Yield(value) => Ok(NativeCallResult::Value(value)),
                    GeneratorResumeOutcome::Complete(value) => {
                        Err(self.stop_iteration_runtime_error(value))
                    }
                    GeneratorResumeOutcome::PropagatedException => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::GeneratorClose => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("close() expects no arguments"));
                }
                match &*receiver.kind() {
                    Object::Generator(state) if state.closed => {
                        return Ok(NativeCallResult::Value(Value::None));
                    }
                    Object::Generator(_) => {}
                    _ => return Err(RuntimeError::type_error("object is not a generator")),
                }
                let close_exc = Value::ExceptionType("GeneratorExit".to_string());
                match self.resume_generator(
                    &receiver,
                    None,
                    Some(close_exc),
                    GeneratorResumeKind::Close,
                ) {
                    Ok(GeneratorResumeOutcome::Yield(_)) => Err(RuntimeError::runtime_error(
                        "generator ignored GeneratorExit",
                    )),
                    Ok(GeneratorResumeOutcome::Complete(_)) => {
                        self.set_generator_closed(&receiver, true)?;
                        Ok(NativeCallResult::Value(Value::None))
                    }
                    Ok(GeneratorResumeOutcome::PropagatedException) => {
                        if self
                            .pending_generator_exception
                            .as_ref()
                            .map(|exc| exception_is_named(exc, "GeneratorExit"))
                            .unwrap_or(false)
                        {
                            self.pending_generator_exception = None;
                            self.set_generator_closed(&receiver, true)?;
                            Ok(NativeCallResult::Value(Value::None))
                        } else if self.active_exception_is("GeneratorExit") {
                            self.clear_active_exception();
                            self.set_generator_closed(&receiver, true)?;
                            Ok(NativeCallResult::Value(Value::None))
                        } else {
                            Ok(NativeCallResult::PropagatedException)
                        }
                    }
                    Err(err) => {
                        if err.message.contains("GeneratorExit") {
                            self.set_generator_closed(&receiver, true)?;
                            Ok(NativeCallResult::Value(Value::None))
                        } else {
                            Err(err)
                        }
                    }
                }
            }
            NativeMethodKind::IteratorIter => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("__iter__() expects no arguments"));
                }
                let receiver_value = Value::Iterator(receiver.clone());
                let iter_value = self.to_iterator_value(receiver_value)?;
                Ok(NativeCallResult::Value(iter_value))
            }
            NativeMethodKind::IteratorNext => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("__next__() expects no arguments"));
                }
                if !matches!(&*receiver.kind(), Object::Iterator(_)) {
                    return Err(RuntimeError::new("__next__() expects iterator"));
                }
                match self.iterator_next_value(&receiver)? {
                    Some(value) => Ok(NativeCallResult::Value(value)),
                    None => Err(self.stop_iteration_runtime_error(Value::None)),
                }
            }
            NativeMethodKind::DictKeys => {
                let dict_receiver = match &*receiver.kind() {
                    Object::Dict(_) => {
                        if !args.is_empty() {
                            return Err(RuntimeError::new("dict.keys() expects no arguments"));
                        }
                        receiver.clone()
                    }
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.len() != 1 {
                            return Err(RuntimeError::new("dict.keys() expects one argument"));
                        }
                        match args.remove(0) {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.keys() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new("dict.keys() receiver must be dict"));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("dict.keys() receiver must be dict"));
                    }
                };
                Ok(NativeCallResult::Value(
                    self.heap.alloc_dict_keys_view(dict_receiver),
                ))
            }
            NativeMethodKind::DictValues => {
                let dict_receiver = match &*receiver.kind() {
                    Object::Dict(_) => {
                        if !args.is_empty() {
                            return Err(RuntimeError::new("dict.values() expects no arguments"));
                        }
                        receiver.clone()
                    }
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.len() != 1 {
                            return Err(RuntimeError::new("dict.values() expects one argument"));
                        }
                        match args.remove(0) {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.values() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.values() receiver must be dict",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("dict.values() receiver must be dict"));
                    }
                };
                let Object::Dict(entries) = &*dict_receiver.kind() else {
                    return Err(RuntimeError::new("dict.values() receiver must be dict"));
                };
                let values = entries.iter().map(|(_, value)| value.clone()).collect();
                Ok(NativeCallResult::Value(self.heap.alloc_list(values)))
            }
            NativeMethodKind::DictItems => {
                let dict_receiver = match &*receiver.kind() {
                    Object::Dict(_) => {
                        if !args.is_empty() {
                            return Err(RuntimeError::new("dict.items() expects no arguments"));
                        }
                        receiver.clone()
                    }
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.len() != 1 {
                            return Err(RuntimeError::new("dict.items() expects one argument"));
                        }
                        match args.remove(0) {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.items() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.items() receiver must be dict",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("dict.items() receiver must be dict"));
                    }
                };
                let Object::Dict(entries) = &*dict_receiver.kind() else {
                    return Err(RuntimeError::new("dict.items() receiver must be dict"));
                };
                let values = entries
                    .iter()
                    .map(|(key, value)| self.heap.alloc_tuple(vec![key.clone(), value.clone()]))
                    .collect();
                Ok(NativeCallResult::Value(self.heap.alloc_list(values)))
            }
            NativeMethodKind::DictCopy => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new("dict.copy() expects no arguments"));
                }
                let dict_receiver = match &*receiver.kind() {
                    Object::Dict(_) => {
                        if !args.is_empty() {
                            return Err(RuntimeError::new("dict.copy() expects no arguments"));
                        }
                        receiver.clone()
                    }
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.len() != 1 {
                            return Err(RuntimeError::new("dict.copy() expects one argument"));
                        }
                        match args.remove(0) {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.copy() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new("dict.copy() receiver must be dict"));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("dict.copy() receiver must be dict"));
                    }
                };
                let Object::Dict(entries) = &*dict_receiver.kind() else {
                    return Err(RuntimeError::new("dict.copy() receiver must be dict"));
                };
                Ok(NativeCallResult::Value(
                    self.heap.alloc_dict(entries.to_vec()),
                ))
            }
            NativeMethodKind::DictInit => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "dict.__init__() expects at most one argument",
                    ));
                }
                if !matches!(&*receiver.kind(), Object::Dict(_)) {
                    return Err(RuntimeError::new("dict.__init__() receiver must be dict"));
                }
                {
                    let mut receiver_kind = receiver.kind_mut();
                    let Object::Dict(entries) = &mut *receiver_kind else {
                        unreachable!();
                    };
                    entries.clear();
                }
                match self.call_native_method(
                    NativeMethodKind::DictUpdateMethod,
                    receiver,
                    args,
                    kwargs,
                )? {
                    NativeCallResult::Value(_) => Ok(NativeCallResult::Value(Value::None)),
                    NativeCallResult::PropagatedException => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::DictClear => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new("dict.clear() expects no arguments"));
                }
                let dict_receiver = match &*receiver.kind() {
                    Object::Dict(_) => {
                        if !args.is_empty() {
                            return Err(RuntimeError::new("dict.clear() expects no arguments"));
                        }
                        receiver.clone()
                    }
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.len() != 1 {
                            return Err(RuntimeError::new("dict.clear() expects one argument"));
                        }
                        match args.remove(0) {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.clear() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.clear() receiver must be dict",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("dict.clear() receiver must be dict"));
                    }
                };
                let Object::Dict(entries) = &mut *dict_receiver.kind_mut() else {
                    return Err(RuntimeError::new("dict.clear() receiver must be dict"));
                };
                entries.clear();
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::DictUpdateMethod => {
                let dict_receiver = match &*receiver.kind() {
                    Object::Dict(_) => receiver.clone(),
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.is_empty() {
                            return Err(RuntimeError::new("dict.update() expects an argument"));
                        }
                        match args.remove(0) {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.update() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.update() receiver must be dict",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("dict.update() receiver must be dict"));
                    }
                };
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "dict.update() expects at most one argument",
                    ));
                }
                if let Some(source) = args.first() {
                    if let Value::Dict(other) = source {
                        let Object::Dict(entries) = &*other.kind() else {
                            return Err(RuntimeError::new("dict.update() expects dict"));
                        };
                        if other.id() == dict_receiver.id() {
                            for (key, value) in entries.to_vec() {
                                self.dict_set_value_checked_runtime(&dict_receiver, key, value)?;
                            }
                        } else {
                            for (key, value) in entries.iter() {
                                self.dict_set_value_checked_runtime(
                                    &dict_receiver,
                                    key.clone(),
                                    value.clone(),
                                )?;
                            }
                        }
                    } else {
                        let mut incoming = Vec::new();
                        let keys_callable = self
                            .builtin_getattr(
                                vec![source.clone(), Value::Str("keys".to_string())],
                                HashMap::new(),
                            )
                            .ok();
                        if let Some(keys_callable) = keys_callable {
                            let keys_value = match self.call_internal(
                                keys_callable,
                                Vec::new(),
                                HashMap::new(),
                            )? {
                                InternalCallOutcome::Value(value) => value,
                                InternalCallOutcome::CallerExceptionHandled => {
                                    return Ok(NativeCallResult::PropagatedException);
                                }
                            };
                            for key in self.collect_iterable_values(keys_value)? {
                                let value = self.getitem_value(source.clone(), key.clone())?;
                                incoming.push((key, value));
                            }
                        } else {
                            let pairs = self.collect_iterable_values(source.clone())?;
                            for pair in pairs {
                                let pair_tuple = match pair {
                                    Value::Tuple(obj) => match &*obj.kind() {
                                        Object::Tuple(values) if values.len() == 2 => {
                                            (values[0].clone(), values[1].clone())
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "dict.update() expects mapping or iterable of pairs",
                                            ));
                                        }
                                    },
                                    Value::List(obj) => match &*obj.kind() {
                                        Object::List(values) if values.len() == 2 => {
                                            (values[0].clone(), values[1].clone())
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "dict.update() expects mapping or iterable of pairs",
                                            ));
                                        }
                                    },
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "dict.update() expects mapping or iterable of pairs",
                                        ));
                                    }
                                };
                                incoming.push(pair_tuple);
                            }
                        }
                        for (key, value) in incoming {
                            self.dict_set_value_checked_runtime(&dict_receiver, key, value)?;
                        }
                    }
                }
                for (name, value) in kwargs.drain() {
                    self.dict_set_value_checked_runtime(&dict_receiver, Value::Str(name), value)?;
                }
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::DictSetDefault => {
                let dict_receiver = match &*receiver.kind() {
                    Object::Dict(_) => receiver.clone(),
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.is_empty() {
                            return Err(RuntimeError::new("dict.setdefault() expects an argument"));
                        }
                        match args.remove(0) {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.setdefault() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.setdefault() receiver must be dict",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("dict.setdefault() receiver must be dict"));
                    }
                };
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("dict.setdefault() expects 1-2 arguments"));
                }
                let key = args.first().cloned().expect("checked len");
                let default = args.get(1).cloned().unwrap_or(Value::None);
                if let Some(value) = self.dict_get_value_runtime(&dict_receiver, &key)? {
                    return Ok(NativeCallResult::Value(value));
                }
                self.dict_set_value_checked_runtime(&dict_receiver, key, default.clone())?;
                Ok(NativeCallResult::Value(default))
            }
            NativeMethodKind::DictGet => {
                let dict_receiver = match &*receiver.kind() {
                    Object::Dict(_) => receiver.clone(),
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.is_empty() {
                            return Err(RuntimeError::new("dict.get() expects an argument"));
                        }
                        match args.remove(0) {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.get() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new("dict.get() receiver must be dict"));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("dict.get() receiver must be dict"));
                    }
                };
                if args.is_empty() || args.len() > 2 || !kwargs.is_empty() {
                    return Err(RuntimeError::new("dict.get() expects 1-2 arguments"));
                }
                let key = args.first().cloned().expect("checked len");
                let default = args.get(1).cloned().unwrap_or(Value::None);
                if let Some(value) = self.dict_get_value_runtime(&dict_receiver, &key)? {
                    return Ok(NativeCallResult::Value(value));
                }
                Ok(NativeCallResult::Value(default))
            }
            NativeMethodKind::ContextVarGetMethod => {
                if args.len() > 1 || !kwargs.is_empty() {
                    return Err(RuntimeError::new("ContextVar.get() expects 0-1 arguments"));
                }
                if !matches!(&*receiver.kind(), Object::Dict(_)) {
                    return Err(RuntimeError::new(
                        "ContextVar.get() receiver must be contextvar",
                    ));
                }
                let marker =
                    dict_get_value(&receiver, &Value::Str("__pyrs_contextvar__".to_string()));
                if !matches!(marker, Some(Value::Bool(true))) {
                    return Err(RuntimeError::new(
                        "ContextVar.get() receiver must be contextvar",
                    ));
                }
                if let Some(value) = dict_get_value(&receiver, &Value::Str("value".to_string())) {
                    return Ok(NativeCallResult::Value(value));
                }
                if let Some(explicit_default) = args.first().cloned() {
                    return Ok(NativeCallResult::Value(explicit_default));
                }
                if let Some(default) = dict_get_value(&receiver, &Value::Str("default".to_string()))
                {
                    return Ok(NativeCallResult::Value(default));
                }
                Err(RuntimeError::new("LookupError"))
            }
            NativeMethodKind::ContextVarSetMethod => {
                if args.len() != 1 || !kwargs.is_empty() {
                    return Err(RuntimeError::new("ContextVar.set() expects one argument"));
                }
                if !matches!(&*receiver.kind(), Object::Dict(_)) {
                    return Err(RuntimeError::new(
                        "ContextVar.set() receiver must be contextvar",
                    ));
                }
                let marker =
                    dict_get_value(&receiver, &Value::Str("__pyrs_contextvar__".to_string()));
                if !matches!(marker, Some(Value::Bool(true))) {
                    return Err(RuntimeError::new(
                        "ContextVar.set() receiver must be contextvar",
                    ));
                }
                let previous = dict_get_value(&receiver, &Value::Str("value".to_string()));
                let had_value = previous.is_some();
                dict_set_value(&receiver, Value::Str("value".to_string()), args[0].clone());
                let mut token_entries = vec![
                    (
                        Value::Str("__pyrs_contextvar_token__".to_string()),
                        Value::Bool(true),
                    ),
                    (
                        Value::Str("contextvar".to_string()),
                        Value::Dict(receiver.clone()),
                    ),
                    (Value::Str("had_value".to_string()), Value::Bool(had_value)),
                ];
                if let Some(previous) = previous {
                    token_entries.push((Value::Str("value".to_string()), previous));
                }
                Ok(NativeCallResult::Value(self.heap.alloc_dict(token_entries)))
            }
            NativeMethodKind::ContextVarResetMethod => {
                if args.len() != 1 || !kwargs.is_empty() {
                    return Err(RuntimeError::new("ContextVar.reset() expects one argument"));
                }
                if !matches!(&*receiver.kind(), Object::Dict(_)) {
                    return Err(RuntimeError::new(
                        "ContextVar.reset() receiver must be contextvar",
                    ));
                }
                let marker =
                    dict_get_value(&receiver, &Value::Str("__pyrs_contextvar__".to_string()));
                if !matches!(marker, Some(Value::Bool(true))) {
                    return Err(RuntimeError::new(
                        "ContextVar.reset() receiver must be contextvar",
                    ));
                }
                let Value::Dict(token_dict) = &args[0] else {
                    return Err(RuntimeError::new(
                        "ContextVar.reset() received invalid token",
                    ));
                };
                let token_marker = dict_get_value(
                    token_dict,
                    &Value::Str("__pyrs_contextvar_token__".to_string()),
                );
                if !matches!(token_marker, Some(Value::Bool(true))) {
                    return Err(RuntimeError::new(
                        "ContextVar.reset() received invalid token",
                    ));
                }
                let Some(Value::Dict(token_contextvar)) =
                    dict_get_value(token_dict, &Value::Str("contextvar".to_string()))
                else {
                    return Err(RuntimeError::new(
                        "ContextVar.reset() received invalid token",
                    ));
                };
                if token_contextvar.id() != receiver.id() {
                    return Err(RuntimeError::new(
                        "ContextVar.reset() token was created by a different ContextVar",
                    ));
                }
                let had_value = matches!(
                    dict_get_value(token_dict, &Value::Str("had_value".to_string())),
                    Some(Value::Bool(true))
                );
                if had_value {
                    if let Some(previous) =
                        dict_get_value(token_dict, &Value::Str("value".to_string()))
                    {
                        dict_set_value(&receiver, Value::Str("value".to_string()), previous);
                    } else {
                        dict_remove_value(&receiver, &Value::Str("value".to_string()));
                    }
                } else {
                    dict_remove_value(&receiver, &Value::Str("value".to_string()));
                }
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::DictGetItem => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new("dict.__getitem__() expects one argument"));
                }
                let (dict_receiver, key, missing_owner) = match &*receiver.kind() {
                    Object::Dict(_) => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(
                                "dict.__getitem__() expects one argument",
                            ));
                        }
                        (
                            receiver.clone(),
                            args.first().cloned().expect("checked len"),
                            None,
                        )
                    }
                    Object::Module(module_data) if module_data.name == "__dict_method__" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(
                                "dict.__getitem__() expects one argument",
                            ));
                        }
                        let dict_receiver = match module_data.globals.get("dict") {
                            Some(Value::Dict(dict_obj)) => dict_obj.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.__getitem__() receiver must be dict",
                                ));
                            }
                        };
                        let missing_owner = module_data.globals.get("owner").cloned();
                        (
                            dict_receiver,
                            args.first().cloned().expect("checked len"),
                            missing_owner,
                        )
                    }
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.len() != 2 {
                            return Err(RuntimeError::new(
                                "dict.__getitem__() expects two arguments",
                            ));
                        }
                        let dict_receiver = match args.first().cloned().expect("checked len") {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.__getitem__() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.__getitem__() receiver must be dict",
                                ));
                            }
                        };
                        let missing_owner = module_data.globals.get("owner").cloned();
                        (
                            dict_receiver,
                            args.get(1).cloned().expect("checked len"),
                            missing_owner,
                        )
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "dict.__getitem__() receiver must be dict",
                        ));
                    }
                };
                if let Some(value) = self.dict_get_value_runtime(&dict_receiver, &key)? {
                    return Ok(NativeCallResult::Value(value));
                }
                if let Some(owner) = missing_owner
                    && let Some(missing) =
                        self.lookup_bound_special_method(&owner, "__missing__")?
                {
                    return match self.call_internal(missing, vec![key], HashMap::new())? {
                        InternalCallOutcome::Value(value) => Ok(NativeCallResult::Value(value)),
                        InternalCallOutcome::CallerExceptionHandled => {
                            Err(self.runtime_error_from_active_exception("__missing__() failed"))
                        }
                    };
                }
                Err(RuntimeError::key_error("key not found"))
            }
            NativeMethodKind::DictSetItem => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "dict.__setitem__() expects two arguments",
                    ));
                }
                let (dict_receiver, key, value) = match &*receiver.kind() {
                    Object::Dict(_) => {
                        if args.len() != 2 {
                            return Err(RuntimeError::new(
                                "dict.__setitem__() expects two arguments",
                            ));
                        }
                        (
                            receiver.clone(),
                            args.first().cloned().expect("checked len"),
                            args.get(1).cloned().expect("checked len"),
                        )
                    }
                    Object::Module(module_data) if module_data.name == "__dict_method__" => {
                        if args.len() != 2 {
                            return Err(RuntimeError::new(
                                "dict.__setitem__() expects two arguments",
                            ));
                        }
                        let dict_receiver = match module_data.globals.get("dict") {
                            Some(Value::Dict(dict_obj)) => dict_obj.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.__setitem__() receiver must be dict",
                                ));
                            }
                        };
                        (
                            dict_receiver,
                            args.first().cloned().expect("checked len"),
                            args.get(1).cloned().expect("checked len"),
                        )
                    }
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.len() != 3 {
                            return Err(RuntimeError::new(
                                "dict.__setitem__() expects three arguments",
                            ));
                        }
                        let dict_receiver = match args.first().cloned().expect("checked len") {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.__setitem__() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.__setitem__() receiver must be dict",
                                ));
                            }
                        };
                        (
                            dict_receiver,
                            args.get(1).cloned().expect("checked len"),
                            args.get(2).cloned().expect("checked len"),
                        )
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "dict.__setitem__() receiver must be dict",
                        ));
                    }
                };
                self.dict_set_value_checked_runtime(&dict_receiver, key, value)?;
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::DictDelItem => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new("dict.__delitem__() expects one argument"));
                }
                let (dict_receiver, key) = match &*receiver.kind() {
                    Object::Dict(_) => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(
                                "dict.__delitem__() expects one argument",
                            ));
                        }
                        (
                            receiver.clone(),
                            args.first().cloned().expect("checked len"),
                        )
                    }
                    Object::Module(module_data) if module_data.name == "__dict_method__" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(
                                "dict.__delitem__() expects one argument",
                            ));
                        }
                        let dict_receiver = match module_data.globals.get("dict") {
                            Some(Value::Dict(dict_obj)) => dict_obj.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.__delitem__() receiver must be dict",
                                ));
                            }
                        };
                        (dict_receiver, args.first().cloned().expect("checked len"))
                    }
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.len() != 2 {
                            return Err(RuntimeError::new(
                                "dict.__delitem__() expects two arguments",
                            ));
                        }
                        let dict_receiver = match args.first().cloned().expect("checked len") {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.__delitem__() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "dict.__delitem__() receiver must be dict",
                                ));
                            }
                        };
                        (dict_receiver, args.get(1).cloned().expect("checked len"))
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "dict.__delitem__() receiver must be dict",
                        ));
                    }
                };
                if self
                    .dict_remove_value_runtime(&dict_receiver, &key)?
                    .is_none()
                {
                    return Err(RuntimeError::key_error("key not found"));
                }
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::DictPop => {
                let dict_receiver = match &*receiver.kind() {
                    Object::Dict(_) => receiver.clone(),
                    Object::Module(module_data)
                        if module_data.name == "__dict_unbound_method__" =>
                    {
                        if args.is_empty() {
                            return Err(RuntimeError::new("dict.pop() expects an argument"));
                        }
                        match args.remove(0) {
                            Value::Dict(dict_obj) => dict_obj,
                            Value::Instance(instance) => {
                                self.instance_backing_dict(&instance).ok_or_else(|| {
                                    RuntimeError::new("dict.pop() receiver must be dict")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new("dict.pop() receiver must be dict"));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new("dict.pop() receiver must be dict"));
                    }
                };
                if args.is_empty() || args.len() > 2 || !kwargs.is_empty() {
                    return Err(RuntimeError::new("dict.pop() expects 1-2 arguments"));
                }
                let key = args.first().cloned().expect("checked len");
                let default = args.get(1).cloned();
                if let Some(value) = self.dict_remove_value_runtime(&dict_receiver, &key)? {
                    return Ok(NativeCallResult::Value(value));
                }
                if let Some(default) = default {
                    return Ok(NativeCallResult::Value(default));
                }
                Err(RuntimeError::key_error("key not found"))
            }
            NativeMethodKind::ListAppend => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("list.append() expects one argument"));
                }
                let value = args.first().cloned().expect("checked len");
                let mut receiver_kind = receiver.kind_mut();
                let Object::List(values) = &mut *receiver_kind else {
                    return Err(RuntimeError::new("list.append() receiver must be list"));
                };
                values.push(value);
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::ListInit => {
                if args.len() > 1 || !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "list.__init__() expects at most one argument",
                    ));
                }
                let mut incoming = if let Some(iterable) = args.first() {
                    self.collect_iterable_values(iterable.clone())?
                } else {
                    Vec::new()
                };
                let mut receiver_kind = receiver.kind_mut();
                let Object::List(values) = &mut *receiver_kind else {
                    return Err(RuntimeError::new("list.__init__() receiver must be list"));
                };
                values.clear();
                values.append(&mut incoming);
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::ListExtend => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("list.extend() expects one argument"));
                }
                let iter = args.first().cloned().expect("checked len");
                let extra = self.collect_iterable_values(iter)?;
                let mut receiver_kind = receiver.kind_mut();
                let Object::List(values) = &mut *receiver_kind else {
                    return Err(RuntimeError::new("list.extend() receiver must be list"));
                };
                values.extend(extra);
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::ListInsert => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("list.insert() expects two arguments"));
                }
                let idx = value_to_int(args.first().cloned().expect("checked len"))?;
                let value = args.get(1).cloned().expect("checked len");
                let mut receiver_kind = receiver.kind_mut();
                let Object::List(values) = &mut *receiver_kind else {
                    return Err(RuntimeError::new("list.insert() receiver must be list"));
                };
                let len = values.len() as i64;
                let mut insert_idx = idx;
                if insert_idx < 0 {
                    insert_idx += len;
                    if insert_idx < 0 {
                        insert_idx = 0;
                    }
                }
                if insert_idx > len {
                    insert_idx = len;
                }
                values.insert(insert_idx as usize, value);
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::ListRemove => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("list.remove() expects one argument"));
                }
                let target = args.first().cloned().expect("checked len");
                let mut receiver_kind = receiver.kind_mut();
                let Object::List(values) = &mut *receiver_kind else {
                    return Err(RuntimeError::new("list.remove() receiver must be list"));
                };
                let Some(index) = values.iter().position(|value| *value == target) else {
                    return Err(RuntimeError::value_error("list.remove(x): x not in list"));
                };
                values.remove(index);
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::ListPop => {
                let (list_receiver, method_args) = match &*receiver.kind() {
                    Object::List(_) => (receiver.clone(), args),
                    Object::Module(module_data)
                        if module_data.name == "__list_unbound_method__" =>
                    {
                        if args.is_empty() {
                            return Err(RuntimeError::type_error(
                                "descriptor 'pop' for 'list' objects needs an argument",
                            ));
                        }
                        let target = args.remove(0);
                        let list_receiver = match target {
                            Value::List(obj) => obj,
                            Value::Instance(instance) => {
                                self.instance_backing_list(&instance).ok_or_else(|| {
                                    RuntimeError::type_error(
                                        "descriptor 'pop' for 'list' objects doesn't apply to this object",
                                    )
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "descriptor 'pop' for 'list' objects doesn't apply to this object",
                                ));
                            }
                        };
                        (list_receiver, args)
                    }
                    _ => {
                        return Err(RuntimeError::new("list.pop() receiver must be list"));
                    }
                };
                if method_args.len() > 1 {
                    return Err(RuntimeError::new("list.pop() expects at most one argument"));
                }
                let mut receiver_kind = list_receiver.kind_mut();
                let Object::List(values) = &mut *receiver_kind else {
                    return Err(RuntimeError::new("list.pop() receiver must be list"));
                };
                if values.is_empty() {
                    return Err(RuntimeError::new("pop from empty list"));
                }
                let idx = if method_args.is_empty() {
                    values.len() as i64 - 1
                } else {
                    value_to_int(method_args.first().cloned().expect("checked len"))?
                };
                let len = values.len() as i64;
                let mut normalized = idx;
                if normalized < 0 {
                    normalized += len;
                }
                if normalized < 0 || normalized >= len {
                    return Err(RuntimeError::new("pop index out of range"));
                }
                Ok(NativeCallResult::Value(values.remove(normalized as usize)))
            }
            NativeMethodKind::QueueSimpleQueuePut => {
                let mut method_args = args;
                let queue_receiver = match &*receiver.kind() {
                    Object::Instance(_) => receiver.clone(),
                    Object::Module(module_data)
                        if module_data.name == "__queue_unbound_method__" =>
                    {
                        if method_args.is_empty() {
                            return Err(RuntimeError::type_error(
                                "descriptor 'put' for '_queue.SimpleQueue' objects needs an argument",
                            ));
                        }
                        match method_args.remove(0) {
                            Value::Instance(instance) => instance,
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "put() receiver must be _queue.SimpleQueue",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "put() receiver must be _queue.SimpleQueue",
                        ));
                    }
                };
                if method_args.is_empty() {
                    return Err(RuntimeError::type_error(
                        "put() missing required argument 'item' (pos 1)",
                    ));
                }
                if method_args.len() > 3 {
                    return Err(RuntimeError::type_error(format!(
                        "put() takes at most 3 positional arguments ({} given)",
                        method_args.len()
                    )));
                }
                let allowed_kwargs = ["block", "timeout"];
                for name in kwargs.keys() {
                    if !allowed_kwargs.contains(&name.as_str()) {
                        return Err(RuntimeError::type_error(format!(
                            "put() got an unexpected keyword argument '{}'",
                            name
                        )));
                    }
                }
                let item = method_args.remove(0);
                let storage = {
                    let mut receiver_kind = queue_receiver.kind_mut();
                    let Object::Instance(instance_data) = &mut *receiver_kind else {
                        return Err(RuntimeError::type_error(
                            "put() receiver must be _queue.SimpleQueue",
                        ));
                    };
                    if let Some(existing) =
                        instance_data.attrs.get(SIMPLE_QUEUE_STORAGE_ATTR).cloned()
                    {
                        existing
                    } else {
                        let list = self.heap.alloc_list(Vec::new());
                        instance_data
                            .attrs
                            .insert(SIMPLE_QUEUE_STORAGE_ATTR.to_string(), list.clone());
                        list
                    }
                };
                let Value::List(storage_list) = storage else {
                    return Err(RuntimeError::type_error("SimpleQueue storage is invalid"));
                };
                let mut storage_kind = storage_list.kind_mut();
                let Object::List(values) = &mut *storage_kind else {
                    return Err(RuntimeError::type_error("SimpleQueue storage is invalid"));
                };
                values.push(item);
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::QueueSimpleQueueGet => {
                let mut method_args = args;
                let queue_receiver = match &*receiver.kind() {
                    Object::Instance(_) => receiver.clone(),
                    Object::Module(module_data)
                        if module_data.name == "__queue_unbound_method__" =>
                    {
                        if method_args.is_empty() {
                            return Err(RuntimeError::type_error(
                                "descriptor 'get' for '_queue.SimpleQueue' objects needs an argument",
                            ));
                        }
                        match method_args.remove(0) {
                            Value::Instance(instance) => instance,
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "get() receiver must be _queue.SimpleQueue",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "get() receiver must be _queue.SimpleQueue",
                        ));
                    }
                };
                if method_args.len() > 2 {
                    return Err(RuntimeError::type_error(format!(
                        "get() takes at most 2 positional arguments ({} given)",
                        method_args.len()
                    )));
                }
                let allowed_kwargs = ["block", "timeout"];
                for name in kwargs.keys() {
                    if !allowed_kwargs.contains(&name.as_str()) {
                        return Err(RuntimeError::type_error(format!(
                            "get() got an unexpected keyword argument '{}'",
                            name
                        )));
                    }
                }
                let block = if let Some(value) = kwargs.remove("block") {
                    if !method_args.is_empty() {
                        return Err(RuntimeError::type_error(
                            "get() got multiple values for argument 'block'",
                        ));
                    }
                    self.truthy_from_value(&value)?
                } else if let Some(value) = method_args.first() {
                    self.truthy_from_value(value)?
                } else {
                    true
                };
                if kwargs.contains_key("timeout") && method_args.len() > 1 {
                    return Err(RuntimeError::type_error(
                        "get() got multiple values for argument 'timeout'",
                    ));
                }
                let _timeout = kwargs
                    .remove("timeout")
                    .or_else(|| method_args.get(1).cloned());
                let storage = {
                    let mut receiver_kind = queue_receiver.kind_mut();
                    let Object::Instance(instance_data) = &mut *receiver_kind else {
                        return Err(RuntimeError::type_error(
                            "get() receiver must be _queue.SimpleQueue",
                        ));
                    };
                    if let Some(existing) =
                        instance_data.attrs.get(SIMPLE_QUEUE_STORAGE_ATTR).cloned()
                    {
                        existing
                    } else {
                        let list = self.heap.alloc_list(Vec::new());
                        instance_data
                            .attrs
                            .insert(SIMPLE_QUEUE_STORAGE_ATTR.to_string(), list.clone());
                        list
                    }
                };
                let Value::List(storage_list) = storage else {
                    return Err(RuntimeError::type_error("SimpleQueue storage is invalid"));
                };
                let mut storage_kind = storage_list.kind_mut();
                let Object::List(values) = &mut *storage_kind else {
                    return Err(RuntimeError::type_error("SimpleQueue storage is invalid"));
                };
                if !values.is_empty() {
                    return Ok(NativeCallResult::Value(values.remove(0)));
                }
                let _ = block;
                Err(RuntimeError::from_exception(ExceptionObject::new(
                    "Empty", None,
                )))
            }
            NativeMethodKind::QueueSimpleQueueGetNowait => {
                let mut method_args = args;
                let queue_receiver = match &*receiver.kind() {
                    Object::Instance(_) => receiver.clone(),
                    Object::Module(module_data)
                        if module_data.name == "__queue_unbound_method__" =>
                    {
                        if method_args.is_empty() {
                            return Err(RuntimeError::type_error(
                                "descriptor 'get_nowait' for '_queue.SimpleQueue' objects needs an argument",
                            ));
                        }
                        match method_args.remove(0) {
                            Value::Instance(instance) => instance,
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "get_nowait() receiver must be _queue.SimpleQueue",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "get_nowait() receiver must be _queue.SimpleQueue",
                        ));
                    }
                };
                if !kwargs.is_empty() || !method_args.is_empty() {
                    return Err(RuntimeError::type_error("get_nowait() takes no arguments"));
                }
                let storage = {
                    let mut receiver_kind = queue_receiver.kind_mut();
                    let Object::Instance(instance_data) = &mut *receiver_kind else {
                        return Err(RuntimeError::type_error(
                            "get_nowait() receiver must be _queue.SimpleQueue",
                        ));
                    };
                    if let Some(existing) =
                        instance_data.attrs.get(SIMPLE_QUEUE_STORAGE_ATTR).cloned()
                    {
                        existing
                    } else {
                        let list = self.heap.alloc_list(Vec::new());
                        instance_data
                            .attrs
                            .insert(SIMPLE_QUEUE_STORAGE_ATTR.to_string(), list.clone());
                        list
                    }
                };
                let Value::List(storage_list) = storage else {
                    return Err(RuntimeError::type_error("SimpleQueue storage is invalid"));
                };
                let mut storage_kind = storage_list.kind_mut();
                let Object::List(values) = &mut *storage_kind else {
                    return Err(RuntimeError::type_error("SimpleQueue storage is invalid"));
                };
                if values.is_empty() {
                    return Err(RuntimeError::from_exception(ExceptionObject::new(
                        "Empty", None,
                    )));
                }
                Ok(NativeCallResult::Value(values.remove(0)))
            }
            NativeMethodKind::QueueSimpleQueueEmpty => {
                let mut method_args = args;
                let queue_receiver = match &*receiver.kind() {
                    Object::Instance(_) => receiver.clone(),
                    Object::Module(module_data)
                        if module_data.name == "__queue_unbound_method__" =>
                    {
                        if method_args.is_empty() {
                            return Err(RuntimeError::type_error(
                                "descriptor 'empty' for '_queue.SimpleQueue' objects needs an argument",
                            ));
                        }
                        match method_args.remove(0) {
                            Value::Instance(instance) => instance,
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "empty() receiver must be _queue.SimpleQueue",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "empty() receiver must be _queue.SimpleQueue",
                        ));
                    }
                };
                if !kwargs.is_empty() || !method_args.is_empty() {
                    return Err(RuntimeError::type_error("empty() takes no arguments"));
                }
                let storage = {
                    let receiver_kind = queue_receiver.kind();
                    let Object::Instance(instance_data) = &*receiver_kind else {
                        return Err(RuntimeError::type_error(
                            "empty() receiver must be _queue.SimpleQueue",
                        ));
                    };
                    instance_data.attrs.get(SIMPLE_QUEUE_STORAGE_ATTR).cloned()
                };
                let is_empty = match storage {
                    Some(Value::List(storage_list)) => match &*storage_list.kind() {
                        Object::List(values) => values.is_empty(),
                        _ => {
                            return Err(RuntimeError::type_error("SimpleQueue storage is invalid"));
                        }
                    },
                    Some(_) => {
                        return Err(RuntimeError::type_error("SimpleQueue storage is invalid"));
                    }
                    None => true,
                };
                Ok(NativeCallResult::Value(Value::Bool(is_empty)))
            }
            NativeMethodKind::ListCount => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("list.count() expects one argument"));
                }
                let target = args.first().cloned().expect("checked len");
                let receiver_kind = receiver.kind();
                let Object::List(values) = &*receiver_kind else {
                    return Err(RuntimeError::new("list.count() receiver must be list"));
                };
                let count = values.iter().filter(|value| **value == target).count() as i64;
                Ok(NativeCallResult::Value(Value::Int(count)))
            }
            NativeMethodKind::ListCopy => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("list.copy() expects no arguments"));
                }
                let receiver_kind = receiver.kind();
                let Object::List(values) = &*receiver_kind else {
                    return Err(RuntimeError::new("list.copy() receiver must be list"));
                };
                Ok(NativeCallResult::Value(
                    self.heap.alloc_list(values.clone()),
                ))
            }
            NativeMethodKind::ListClear => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("list.clear() expects no arguments"));
                }
                let mut receiver_kind = receiver.kind_mut();
                let Object::List(values) = &mut *receiver_kind else {
                    return Err(RuntimeError::new("list.clear() receiver must be list"));
                };
                values.clear();
                Ok(NativeCallResult::Value(Value::None))
            }
            cmp_kind @ (NativeMethodKind::ListEq | NativeMethodKind::ListNe) => {
                if args.len() != 1 {
                    let method_name = if matches!(cmp_kind, NativeMethodKind::ListEq) {
                        "list.__eq__"
                    } else {
                        "list.__ne__"
                    };
                    return Err(RuntimeError::new(format!(
                        "{method_name}() expects one argument"
                    )));
                }
                let left_values = {
                    let receiver_kind = receiver.kind();
                    let Object::List(values) = &*receiver_kind else {
                        return Err(RuntimeError::new("list comparison receiver must be list"));
                    };
                    values.clone()
                };
                let right_values = match args.remove(0) {
                    Value::List(list_obj) => {
                        let list_kind = list_obj.kind();
                        let Object::List(values) = &*list_kind else {
                            return Err(RuntimeError::new("list comparison argument must be list"));
                        };
                        Some(values.clone())
                    }
                    Value::Instance(instance) => {
                        let Some(backing) = self.instance_backing_list(&instance) else {
                            return Ok(NativeCallResult::Value(Value::Bool(!matches!(
                                cmp_kind,
                                NativeMethodKind::ListEq
                            ))));
                        };
                        let list_kind = backing.kind();
                        let Object::List(values) = &*list_kind else {
                            return Ok(NativeCallResult::Value(Value::Bool(!matches!(
                                cmp_kind,
                                NativeMethodKind::ListEq
                            ))));
                        };
                        Some(values.clone())
                    }
                    _ => None,
                };
                let equals = if let Some(right_values) = right_values {
                    if left_values.len() != right_values.len() {
                        false
                    } else {
                        let mut all_equal = true;
                        for (left, right) in left_values.iter().zip(right_values.iter()) {
                            let result = self.compare_eq_runtime(left.clone(), right.clone())?;
                            if !self.truthy_from_value(&result)? {
                                all_equal = false;
                                break;
                            }
                        }
                        all_equal
                    }
                } else {
                    false
                };
                let value = if matches!(cmp_kind, NativeMethodKind::ListEq) {
                    equals
                } else {
                    !equals
                };
                Ok(NativeCallResult::Value(Value::Bool(value)))
            }
            NativeMethodKind::TupleCount => {
                if args.is_empty() {
                    return Err(RuntimeError::type_error(
                        "tuple.count() expects one argument",
                    ));
                }
                match &*receiver.kind() {
                    Object::Tuple(values) => {
                        if args.len() != 1 {
                            return Err(RuntimeError::type_error(
                                "tuple.count() expects one argument",
                            ));
                        }
                        let target = args.remove(0);
                        let count = values.iter().filter(|value| **value == target).count() as i64;
                        Ok(NativeCallResult::Value(Value::Int(count)))
                    }
                    Object::Module(module_data) => {
                        let tuple_obj =
                            if let Some(Value::Tuple(tuple)) = module_data.globals.get("value") {
                                tuple.clone()
                            } else {
                                if args.len() < 2 {
                                    return Err(RuntimeError::new(
                                        "tuple.count() expects one argument",
                                    ));
                                }
                                match args.remove(0) {
                                    Value::Tuple(tuple) => tuple,
                                    Value::Instance(instance) => {
                                        self.instance_backing_tuple(&instance).ok_or_else(|| {
                                            RuntimeError::type_error(
                                                "tuple.count() receiver must be tuple",
                                            )
                                        })?
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "tuple.count() receiver must be tuple",
                                        ));
                                    }
                                }
                            };
                        if args.len() != 1 {
                            return Err(RuntimeError::type_error(
                                "tuple.count() expects one argument",
                            ));
                        }
                        let target = args.remove(0);
                        let tuple_kind = tuple_obj.kind();
                        let Object::Tuple(values) = &*tuple_kind else {
                            return Err(RuntimeError::type_error(
                                "tuple.count() receiver must be tuple",
                            ));
                        };
                        let count = values.iter().filter(|value| **value == target).count() as i64;
                        Ok(NativeCallResult::Value(Value::Int(count)))
                    }
                    _ => Err(RuntimeError::type_error(
                        "tuple.count() receiver must be tuple",
                    )),
                }
            }
            cmp_kind @ (NativeMethodKind::TupleEq | NativeMethodKind::TupleNe) => {
                if args.len() != 1 {
                    let method_name = if matches!(cmp_kind, NativeMethodKind::TupleEq) {
                        "tuple.__eq__"
                    } else {
                        "tuple.__ne__"
                    };
                    return Err(RuntimeError::new(format!(
                        "{method_name}() expects one argument"
                    )));
                }
                let left_values = {
                    let receiver_kind = receiver.kind();
                    let Object::Tuple(values) = &*receiver_kind else {
                        return Err(RuntimeError::new("tuple comparison receiver must be tuple"));
                    };
                    values.clone()
                };
                let right_values = match args.remove(0) {
                    Value::Tuple(tuple_obj) => {
                        let tuple_kind = tuple_obj.kind();
                        let Object::Tuple(values) = &*tuple_kind else {
                            return Err(RuntimeError::new(
                                "tuple comparison argument must be tuple",
                            ));
                        };
                        Some(values.clone())
                    }
                    Value::Instance(instance) => {
                        let Some(backing) = self.instance_backing_tuple(&instance) else {
                            return Ok(NativeCallResult::Value(Value::Bool(!matches!(
                                cmp_kind,
                                NativeMethodKind::TupleEq
                            ))));
                        };
                        let tuple_kind = backing.kind();
                        let Object::Tuple(values) = &*tuple_kind else {
                            return Ok(NativeCallResult::Value(Value::Bool(!matches!(
                                cmp_kind,
                                NativeMethodKind::TupleEq
                            ))));
                        };
                        Some(values.clone())
                    }
                    _ => None,
                };
                let equals = if let Some(right_values) = right_values {
                    if left_values.len() != right_values.len() {
                        false
                    } else {
                        let mut all_equal = true;
                        for (left, right) in left_values.iter().zip(right_values.iter()) {
                            let result = self.compare_eq_runtime(left.clone(), right.clone())?;
                            if !self.truthy_from_value(&result)? {
                                all_equal = false;
                                break;
                            }
                        }
                        all_equal
                    }
                } else {
                    false
                };
                let value = if matches!(cmp_kind, NativeMethodKind::TupleEq) {
                    equals
                } else {
                    !equals
                };
                Ok(NativeCallResult::Value(Value::Bool(value)))
            }
            NativeMethodKind::TupleIndex => {
                let find_index = |values: &[Value],
                                  remaining_args: &mut Vec<Value>|
                 -> Result<Option<i64>, RuntimeError> {
                    if !(1..=3).contains(&remaining_args.len()) {
                        return Err(RuntimeError::new(
                            "tuple.index() expects one to three arguments",
                        ));
                    }
                    let target = remaining_args.remove(0);
                    let len = values.len() as i64;
                    let mut start = if remaining_args.is_empty() {
                        0
                    } else {
                        value_to_int(remaining_args.remove(0))?
                    };
                    let mut stop = if remaining_args.is_empty() {
                        len
                    } else {
                        value_to_int(remaining_args.remove(0))?
                    };
                    if start < 0 {
                        start += len;
                    }
                    if stop < 0 {
                        stop += len;
                    }
                    start = start.clamp(0, len);
                    stop = stop.clamp(0, len);
                    if stop < start {
                        stop = start;
                    }
                    for (idx, value) in values
                        .iter()
                        .enumerate()
                        .take(stop as usize)
                        .skip(start as usize)
                    {
                        if *value == target {
                            return Ok(Some(idx as i64));
                        }
                    }
                    Ok(None)
                };
                match &*receiver.kind() {
                    Object::Tuple(values) => {
                        let mut remaining_args = args;
                        if let Some(index) = find_index(values, &mut remaining_args)? {
                            Ok(NativeCallResult::Value(Value::Int(index)))
                        } else {
                            Err(RuntimeError::value_error("tuple.index(x): x not in tuple"))
                        }
                    }
                    Object::Module(module_data) => {
                        let tuple_obj =
                            if let Some(Value::Tuple(tuple)) = module_data.globals.get("value") {
                                tuple.clone()
                            } else {
                                if args.is_empty() {
                                    return Err(RuntimeError::new(
                                        "tuple.index() expects one argument",
                                    ));
                                }
                                match args.remove(0) {
                                    Value::Tuple(tuple) => tuple,
                                    Value::Instance(instance) => {
                                        self.instance_backing_tuple(&instance).ok_or_else(|| {
                                            RuntimeError::type_error(
                                                "tuple.index() receiver must be tuple",
                                            )
                                        })?
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "tuple.index() receiver must be tuple",
                                        ));
                                    }
                                }
                            };
                        let mut remaining_args = args;
                        let tuple_kind = tuple_obj.kind();
                        let Object::Tuple(values) = &*tuple_kind else {
                            return Err(RuntimeError::type_error(
                                "tuple.index() receiver must be tuple",
                            ));
                        };
                        if let Some(index) = find_index(values, &mut remaining_args)? {
                            Ok(NativeCallResult::Value(Value::Int(index)))
                        } else {
                            Err(RuntimeError::value_error("tuple.index(x): x not in tuple"))
                        }
                    }
                    _ => Err(RuntimeError::type_error(
                        "tuple.index() receiver must be tuple",
                    )),
                }
            }
            NativeMethodKind::ListIndex => {
                if !(1..=3).contains(&args.len()) {
                    return Err(RuntimeError::new(
                        "list.index() expects one to three arguments",
                    ));
                }
                let target = args.remove(0);
                let receiver_kind = receiver.kind();
                let Object::List(values) = &*receiver_kind else {
                    return Err(RuntimeError::new("list.index() receiver must be list"));
                };
                let len = values.len() as i64;
                let mut start = if args.is_empty() {
                    0
                } else {
                    value_to_int(args.remove(0))?
                };
                let mut stop = if args.is_empty() {
                    len
                } else {
                    value_to_int(args.remove(0))?
                };
                if start < 0 {
                    start += len;
                }
                if stop < 0 {
                    stop += len;
                }
                start = start.clamp(0, len);
                stop = stop.clamp(0, len);
                if stop < start {
                    stop = start;
                }
                for (idx, value) in values
                    .iter()
                    .enumerate()
                    .take(stop as usize)
                    .skip(start as usize)
                {
                    if *value == target {
                        return Ok(NativeCallResult::Value(Value::Int(idx as i64)));
                    }
                }
                Err(RuntimeError::new("list.index(x): x not in list"))
            }
            NativeMethodKind::ListReverse => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("list.reverse() expects no arguments"));
                }
                let mut receiver_kind = receiver.kind_mut();
                let Object::List(values) = &mut *receiver_kind else {
                    return Err(RuntimeError::new("list.reverse() receiver must be list"));
                };
                values.reverse();
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::ListSort => {
                if !args.is_empty() {
                    return Err(RuntimeError::new(
                        "list.sort() expects no positional arguments",
                    ));
                }
                let reverse = kwargs
                    .remove("reverse")
                    .map(|value| is_truthy(&value))
                    .unwrap_or(false);
                let key_func = kwargs.remove("key").unwrap_or(Value::None);
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "list.sort() got an unexpected keyword argument",
                    ));
                }

                // Follow CPython-style in-place semantics by temporarily taking the list
                // contents out of the receiver object and restoring them after sorting.
                let mut working = {
                    let mut receiver_kind = receiver.kind_mut();
                    let Object::List(values) = &mut *receiver_kind else {
                        return Err(RuntimeError::new("list.sort() receiver must be list"));
                    };
                    std::mem::take(values)
                };
                if let Err(err) =
                    self.sort_values_with_optional_key(&mut working, &key_func, reverse)
                {
                    let mut receiver_kind = receiver.kind_mut();
                    let Object::List(values) = &mut *receiver_kind else {
                        return Err(RuntimeError::new("list.sort() receiver must be list"));
                    };
                    if values.is_empty() {
                        *values = working;
                    }
                    return Err(err);
                }

                let mut receiver_kind = receiver.kind_mut();
                let Object::List(values) = &mut *receiver_kind else {
                    return Err(RuntimeError::new("list.sort() receiver must be list"));
                };
                let modified_during_sort = !values.is_empty();
                *values = working;
                if modified_during_sort {
                    return Err(RuntimeError::new("list modified during sort"));
                }
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::IntToBytes => {
                if args.len() > 3 {
                    return Err(RuntimeError::new(
                        "to_bytes() takes at most 3 positional arguments",
                    ));
                }
                let mut length_arg = Value::Int(1);
                let mut byteorder_arg = Value::Str("big".to_string());
                let mut signed_arg = Value::Bool(false);
                if let Some(value) = args.first() {
                    length_arg = value.clone();
                }
                if let Some(value) = args.get(1) {
                    byteorder_arg = value.clone();
                }
                if let Some(value) = args.get(2) {
                    signed_arg = value.clone();
                }
                if let Some(value) = kwargs.remove("length") {
                    if !args.is_empty() {
                        return Err(RuntimeError::new(
                            "to_bytes() got multiple values for argument 'length'",
                        ));
                    }
                    length_arg = value;
                }
                if let Some(value) = kwargs.remove("byteorder") {
                    if args.len() > 1 {
                        return Err(RuntimeError::new(
                            "to_bytes() got multiple values for argument 'byteorder'",
                        ));
                    }
                    byteorder_arg = value;
                }
                if let Some(value) = kwargs.remove("signed") {
                    if args.len() > 2 {
                        return Err(RuntimeError::new(
                            "to_bytes() got multiple values for argument 'signed'",
                        ));
                    }
                    signed_arg = value;
                }
                if let Some(unexpected) = kwargs.keys().next().cloned() {
                    return Err(RuntimeError::new(format!(
                        "to_bytes() got an unexpected keyword argument '{}'",
                        unexpected
                    )));
                }
                let value = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(value) => value_to_bigint(value.clone())?,
                        _ => return Err(RuntimeError::type_error("int receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("int receiver is invalid")),
                };
                let length = value_to_int(length_arg)?;
                if length < 0 {
                    return Err(RuntimeError::new("length argument must be non-negative"));
                }
                let byteorder = match &byteorder_arg {
                    Value::Str(order) if order == "little" || order == "big" => order.clone(),
                    _ => {
                        return Err(RuntimeError::new(
                            "byteorder argument must be either 'little' or 'big'",
                        ));
                    }
                };
                let signed = is_truthy(&signed_arg);
                let bytes =
                    bigint_to_fixed_bytes(&value, length as usize, byteorder == "little", signed)?;
                Ok(NativeCallResult::Value(self.heap.alloc_bytes(bytes)))
            }
            NativeMethodKind::IntBitLengthMethod => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("bit_length() expects no arguments"));
                }
                let value = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(value) => value_to_bigint(value.clone())?,
                        _ => return Err(RuntimeError::type_error("int receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("int receiver is invalid")),
                };
                Ok(NativeCallResult::Value(Value::Int(
                    value.bit_length() as i64
                )))
            }
            NativeMethodKind::IntIndexMethod => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("__index__() expects no arguments"));
                }
                let value = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Int(value)) => Value::Int(*value),
                        Some(Value::Bool(value)) => Value::Int(if *value { 1 } else { 0 }),
                        Some(Value::BigInt(value)) => Value::BigInt(value.clone()),
                        _ => return Err(RuntimeError::type_error("int receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("int receiver is invalid")),
                };
                Ok(NativeCallResult::Value(value))
            }
            NativeMethodKind::FloatAsIntegerRatioMethod => {
                let value = self.extract_float_receiver_value_for_method_call(
                    &receiver,
                    &mut args,
                    "as_integer_ratio",
                )?;

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
                    return Ok(NativeCallResult::Value(
                        self.heap.alloc_tuple(vec![Value::Int(0), Value::Int(1)]),
                    ));
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

                Ok(NativeCallResult::Value(self.heap.alloc_tuple(vec![
                    value_from_bigint(numerator),
                    value_from_bigint(denominator),
                ])))
            }
            NativeMethodKind::FloatIsIntegerMethod => {
                let value = self.extract_float_receiver_value_for_method_call(
                    &receiver,
                    &mut args,
                    "is_integer",
                )?;
                Ok(NativeCallResult::Value(Value::Bool(
                    value.is_finite() && value.fract() == 0.0,
                )))
            }
            NativeMethodKind::FloatConjugateMethod => {
                let value = self.extract_float_receiver_value_for_method_call(
                    &receiver,
                    &mut args,
                    "conjugate",
                )?;
                Ok(NativeCallResult::Value(Value::Float(value)))
            }
            NativeMethodKind::StrStartsWith | NativeMethodKind::StrEndsWith => {
                let method_name = if matches!(kind, NativeMethodKind::StrStartsWith) {
                    "startswith"
                } else {
                    "endswith"
                };
                if args.is_empty() || args.len() > 3 {
                    return Err(RuntimeError::new(format!(
                        "{method_name}() expects prefix/suffix, optional start, optional end"
                    )));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let char_len = text.chars().count();
                let len = char_len as i64;
                let mut start = if let Some(value) = args.get(1) {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let mut end = if let Some(value) = args.get(2) {
                    value_to_int(value.clone())?
                } else {
                    len
                };
                if start < 0 {
                    start += len;
                }
                if end < 0 {
                    end += len;
                }
                start = start.clamp(0, len);
                end = end.clamp(0, len);
                if end < start {
                    return Ok(NativeCallResult::Value(Value::Bool(false)));
                }
                let start_idx = start as usize;
                let end_idx = end as usize;
                let mut byte_offsets: Vec<usize> =
                    text.char_indices().map(|(idx, _)| idx).collect();
                byte_offsets.push(text.len());
                if start_idx > char_len || end_idx > char_len {
                    return Ok(NativeCallResult::Value(Value::Bool(false)));
                }
                let start_byte = byte_offsets[start_idx];
                let end_byte = byte_offsets[end_idx];
                let Some(slice) = text.get(start_byte..end_byte) else {
                    return Ok(NativeCallResult::Value(Value::Bool(false)));
                };
                let match_candidate = |candidate: &str| {
                    if matches!(kind, NativeMethodKind::StrStartsWith) {
                        slice.starts_with(candidate)
                    } else {
                        slice.ends_with(candidate)
                    }
                };
                let matches = match args.first().expect("checked len") {
                    Value::Str(prefix_or_suffix) => match_candidate(prefix_or_suffix),
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(items) => {
                            let mut any = false;
                            for item in items {
                                if let Value::Str(text) = item {
                                    if match_candidate(text) {
                                        any = true;
                                        break;
                                    }
                                } else {
                                    return Err(RuntimeError::type_error(format!(
                                        "{method_name}() tuple entries must be str"
                                    )));
                                }
                            }
                            any
                        }
                        _ => {
                            return Err(RuntimeError::type_error(format!(
                                "{method_name}() argument must be str or tuple of str"
                            )));
                        }
                    },
                    _ => {
                        return Err(RuntimeError::type_error(format!(
                            "{method_name}() argument must be str or tuple of str"
                        )));
                    }
                };
                Ok(NativeCallResult::Value(Value::Bool(matches)))
            }
            NativeMethodKind::StrReplace => {
                if args.len() < 2 || args.len() > 3 {
                    return Err(RuntimeError::new(
                        "replace() expects old, new, optional count",
                    ));
                }
                let old = match args.first().expect("checked len") {
                    Value::Str(value) => value.clone(),
                    _ => return Err(RuntimeError::new("replace() old must be str")),
                };
                let new = match args.get(1).expect("checked len") {
                    Value::Str(value) => value.clone(),
                    _ => return Err(RuntimeError::new("replace() new must be str")),
                };
                let count = if let Some(value) = args.get(2) {
                    value_to_int(value.clone())?
                } else {
                    -1
                };
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                if old.is_empty() {
                    return Ok(NativeCallResult::Value(Value::Str(text)));
                }
                if count == 0 {
                    return Ok(NativeCallResult::Value(Value::Str(text)));
                }
                let mut remaining = text.as_str();
                let mut out = String::new();
                let mut replaced = 0i64;
                while let Some(idx) = remaining.find(&old) {
                    if count >= 0 && replaced >= count {
                        break;
                    }
                    out.push_str(&remaining[..idx]);
                    out.push_str(&new);
                    remaining = &remaining[idx + old.len()..];
                    replaced += 1;
                }
                out.push_str(remaining);
                Ok(NativeCallResult::Value(Value::Str(out)))
            }
            NativeMethodKind::StrUpper => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "upper")?;
                Ok(NativeCallResult::Value(Value::Str(text.to_uppercase())))
            }
            NativeMethodKind::StrLower => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "lower")?;
                Ok(NativeCallResult::Value(Value::Str(text.to_lowercase())))
            }
            NativeMethodKind::StrSwapCase => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "swapcase")?;
                let mut out = String::new();
                for ch in text.chars() {
                    if ch.is_lowercase() {
                        out.extend(ch.to_uppercase());
                    } else if ch.is_uppercase() {
                        out.extend(ch.to_lowercase());
                    } else {
                        out.push(ch);
                    }
                }
                Ok(NativeCallResult::Value(Value::Str(out)))
            }
            NativeMethodKind::StrCapitalize => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("capitalize() expects no arguments"));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let mut chars = text.chars();
                let Some(first) = chars.next() else {
                    return Ok(NativeCallResult::Value(Value::Str(String::new())));
                };
                let mut out = String::new();
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str().to_lowercase().as_str());
                Ok(NativeCallResult::Value(Value::Str(out)))
            }
            NativeMethodKind::StrTitle => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "title")?;
                let mut out = String::new();
                let mut previous_is_cased = false;
                for ch in text.chars() {
                    if ch.is_lowercase() || ch.is_uppercase() {
                        if previous_is_cased {
                            out.extend(ch.to_lowercase());
                        } else {
                            out.extend(ch.to_uppercase());
                        }
                        previous_is_cased = true;
                    } else {
                        out.push(ch);
                        previous_is_cased = false;
                    }
                }
                Ok(NativeCallResult::Value(Value::Str(out)))
            }
            NativeMethodKind::StrEncode => {
                let mut kw_encoding = None;
                let mut kw_errors = None;
                for (name, value) in kwargs {
                    match name.as_str() {
                        "encoding" => {
                            if kw_encoding.replace(value).is_some() {
                                return Err(RuntimeError::new(
                                    "encode() got multiple values for argument 'encoding'",
                                ));
                            }
                        }
                        "errors" => {
                            if kw_errors.replace(value).is_some() {
                                return Err(RuntimeError::new(
                                    "encode() got multiple values for argument 'errors'",
                                ));
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(format!(
                                "encode() got an unexpected keyword argument '{}'",
                                name
                            )));
                        }
                    }
                }
                if args.len() > 2 {
                    return Err(RuntimeError::new(
                        "encode() expects optional encoding and errors",
                    ));
                }
                if args.len() >= 1 && kw_encoding.is_some() {
                    return Err(RuntimeError::new(
                        "encode() got multiple values for argument 'encoding'",
                    ));
                }
                if args.len() >= 2 && kw_errors.is_some() {
                    return Err(RuntimeError::new(
                        "encode() got multiple values for argument 'errors'",
                    ));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let encoding = normalize_codec_encoding(
                    args.first()
                        .cloned()
                        .or(kw_encoding)
                        .unwrap_or(Value::Str("utf-8".to_string())),
                )?;
                let errors = normalize_codec_errors(
                    args.get(1)
                        .cloned()
                        .or(kw_errors)
                        .unwrap_or(Value::Str("strict".to_string())),
                )?;
                Ok(NativeCallResult::Value(self.heap.alloc_bytes(
                    encode_text_bytes(&text, &encoding, &errors)?,
                )))
            }
            NativeMethodKind::StrDecode => {
                let mut kw_encoding = None;
                let mut kw_errors = None;
                for (name, value) in kwargs {
                    match name.as_str() {
                        "encoding" => {
                            if kw_encoding.replace(value).is_some() {
                                return Err(RuntimeError::new(
                                    "decode() got multiple values for argument 'encoding'",
                                ));
                            }
                        }
                        "errors" => {
                            if kw_errors.replace(value).is_some() {
                                return Err(RuntimeError::new(
                                    "decode() got multiple values for argument 'errors'",
                                ));
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(format!(
                                "decode() got an unexpected keyword argument '{}'",
                                name
                            )));
                        }
                    }
                }
                if args.len() > 2 {
                    return Err(RuntimeError::new(
                        "decode() expects optional encoding and errors",
                    ));
                }
                if args.len() >= 1 && kw_encoding.is_some() {
                    return Err(RuntimeError::new(
                        "decode() got multiple values for argument 'encoding'",
                    ));
                }
                if args.len() >= 2 && kw_errors.is_some() {
                    return Err(RuntimeError::new(
                        "decode() got multiple values for argument 'errors'",
                    ));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                if let Some(value) = args.first().cloned().or(kw_encoding) {
                    let _ = normalize_codec_encoding(value.clone())?;
                }
                if let Some(value) = args.get(1).cloned().or(kw_errors) {
                    let _ = normalize_codec_errors(value)?;
                }
                Ok(NativeCallResult::Value(Value::Str(text)))
            }
            NativeMethodKind::BytesDecode => {
                let mut kw_encoding = None;
                let mut kw_errors = None;
                for (name, value) in kwargs {
                    match name.as_str() {
                        "encoding" => {
                            if kw_encoding.replace(value).is_some() {
                                return Err(RuntimeError::new(
                                    "decode() got multiple values for argument 'encoding'",
                                ));
                            }
                        }
                        "errors" => {
                            if kw_errors.replace(value).is_some() {
                                return Err(RuntimeError::new(
                                    "decode() got multiple values for argument 'errors'",
                                ));
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(format!(
                                "decode() got an unexpected keyword argument '{}'",
                                name
                            )));
                        }
                    }
                }
                if args.len() > 2 {
                    return Err(RuntimeError::new(
                        "decode() expects optional encoding and errors",
                    ));
                }
                if args.len() >= 1 && kw_encoding.is_some() {
                    return Err(RuntimeError::new(
                        "decode() got multiple values for argument 'encoding'",
                    ));
                }
                if args.len() >= 2 && kw_errors.is_some() {
                    return Err(RuntimeError::new(
                        "decode() got multiple values for argument 'errors'",
                    ));
                }
                let bytes = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(value) => bytes_like_from_value(value.clone())?,
                        None => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let encoding = normalize_codec_encoding(
                    args.first()
                        .cloned()
                        .or(kw_encoding)
                        .unwrap_or(Value::Str("utf-8".to_string())),
                )?;
                let errors = normalize_codec_errors(
                    args.get(1)
                        .cloned()
                        .or(kw_errors)
                        .unwrap_or(Value::Str("strict".to_string())),
                )?;
                let text = decode_text_bytes(&bytes, &encoding, &errors)?;
                Ok(NativeCallResult::Value(Value::Str(text)))
            }
            NativeMethodKind::BytesHex => {
                let mut kw_sep = None;
                let mut kw_bytes_per_sep = None;
                for (name, value) in kwargs {
                    match name.as_str() {
                        "sep" => {
                            if kw_sep.replace(value).is_some() {
                                return Err(RuntimeError::new(
                                    "hex() got multiple values for argument 'sep'",
                                ));
                            }
                        }
                        "bytes_per_sep" => {
                            if kw_bytes_per_sep.replace(value).is_some() {
                                return Err(RuntimeError::new(
                                    "hex() got multiple values for argument 'bytes_per_sep'",
                                ));
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(format!(
                                "hex() got an unexpected keyword argument '{}'",
                                name
                            )));
                        }
                    }
                }
                if args.len() > 2 {
                    return Err(RuntimeError::new(
                        "TypeError: hex() takes at most 2 arguments",
                    ));
                }
                if !args.is_empty() && kw_sep.is_some() {
                    return Err(RuntimeError::new(
                        "hex() got multiple values for argument 'sep'",
                    ));
                }
                if args.len() > 1 && kw_bytes_per_sep.is_some() {
                    return Err(RuntimeError::new(
                        "hex() got multiple values for argument 'bytes_per_sep'",
                    ));
                }
                let bytes = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(value) => bytes_like_from_value(value.clone())?,
                        None => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let sep_value = args.first().cloned().or(kw_sep);
                let has_separator_arg = sep_value.is_some();
                let bytes_per_sep_value = args.get(1).cloned().or(kw_bytes_per_sep);
                let separator = if let Some(value) = sep_value {
                    match value {
                        Value::Str(sep) => {
                            let mut chars = sep.chars();
                            let Some(ch) = chars.next() else {
                                return Err(RuntimeError::value_error(
                                    "sep must be length 1 and ASCII",
                                ));
                            };
                            if chars.next().is_some() || (ch as u32) > 0x7f {
                                return Err(RuntimeError::value_error(
                                    "sep must be length 1 and ASCII",
                                ));
                            }
                            ch
                        }
                        Value::Bytes(obj) => {
                            let Object::Bytes(values) = &*obj.kind() else {
                                return Err(RuntimeError::type_error("sep must be str or bytes"));
                            };
                            if values.len() != 1 || values[0] > 0x7f {
                                return Err(RuntimeError::value_error(
                                    "sep must be length 1 and ASCII",
                                ));
                            }
                            values[0] as char
                        }
                        Value::ByteArray(obj) => {
                            let Object::ByteArray(values) = &*obj.kind() else {
                                return Err(RuntimeError::type_error("sep must be str or bytes"));
                            };
                            if values.len() != 1 || values[0] > 0x7f {
                                return Err(RuntimeError::value_error(
                                    "sep must be length 1 and ASCII",
                                ));
                            }
                            values[0] as char
                        }
                        _ => return Err(RuntimeError::type_error("sep must be str or bytes")),
                    }
                } else if bytes_per_sep_value.is_some() {
                    return Err(RuntimeError::type_error("sep must be str or bytes"));
                } else {
                    '\0'
                };
                let bytes_per_sep = if let Some(value) = bytes_per_sep_value {
                    value_to_int(value)?
                } else {
                    1
                };
                const HEX: &[u8; 16] = b"0123456789abcdef";
                let mut out = String::with_capacity(bytes.len() * 3);
                let group = bytes_per_sep.unsigned_abs() as usize;
                let use_separator = has_separator_arg && bytes_per_sep != 0 && group > 0;
                for (idx, byte) in bytes.iter().copied().enumerate() {
                    if use_separator && idx > 0 {
                        let insert = if bytes_per_sep > 0 {
                            (bytes.len() - idx) % group == 0
                        } else {
                            idx % group == 0
                        };
                        if insert {
                            out.push(separator);
                        }
                    }
                    out.push(HEX[(byte >> 4) as usize] as char);
                    out.push(HEX[(byte & 0x0f) as usize] as char);
                }
                Ok(NativeCallResult::Value(Value::Str(out)))
            }
            NativeMethodKind::BytesStartsWith | NativeMethodKind::BytesEndsWith => {
                let method_name = if matches!(kind, NativeMethodKind::BytesStartsWith) {
                    "startswith"
                } else {
                    "endswith"
                };
                if args.is_empty() || args.len() > 3 {
                    return Err(RuntimeError::new(format!(
                        "{method_name}() expects prefix/suffix, optional start, optional end"
                    )));
                }
                let bytes = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(value) => bytes_like_from_value(value.clone())?,
                        None => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let len = bytes.len() as i64;
                let mut start = if let Some(value) = args.get(1) {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let mut end = if let Some(value) = args.get(2) {
                    value_to_int(value.clone())?
                } else {
                    len
                };
                if start < 0 {
                    start += len;
                }
                if end < 0 {
                    end += len;
                }
                start = start.clamp(0, len);
                end = end.clamp(0, len);
                if end < start {
                    return Ok(NativeCallResult::Value(Value::Bool(false)));
                }
                let slice = &bytes[start as usize..end as usize];
                let match_candidate = |candidate: &[u8]| {
                    if matches!(kind, NativeMethodKind::BytesStartsWith) {
                        slice.starts_with(candidate)
                    } else {
                        slice.ends_with(candidate)
                    }
                };
                let matches = match args.first().expect("checked len") {
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(items) => {
                            let mut any = false;
                            for item in items {
                                let candidate = bytes_like_from_value(item.clone())?;
                                if match_candidate(&candidate) {
                                    any = true;
                                    break;
                                }
                            }
                            any
                        }
                        _ => {
                            return Err(RuntimeError::new(format!(
                                "{method_name}() argument must be bytes-like or tuple of bytes-like"
                            )));
                        }
                    },
                    value => {
                        let candidate = bytes_like_from_value(value.clone())?;
                        match_candidate(&candidate)
                    }
                };
                Ok(NativeCallResult::Value(Value::Bool(matches)))
            }
            NativeMethodKind::BytesCount => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::type_error(
                        "count() takes no keyword arguments",
                    ));
                }
                if args.is_empty() || args.len() > 3 {
                    return Err(RuntimeError::new(
                        "count() expects sub, optional start, optional end",
                    ));
                }
                let bytes = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(value) => bytes_like_from_value(value.clone())?,
                        None => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let needle = match args.remove(0) {
                    Value::Int(value) => {
                        if !(0..=255).contains(&value) {
                            return Err(RuntimeError::value_error("byte must be in range(0, 256)"));
                        }
                        vec![value as u8]
                    }
                    Value::BigInt(value) => {
                        let Some(value) = value.to_i64() else {
                            return Err(RuntimeError::value_error("byte must be in range(0, 256)"));
                        };
                        if !(0..=255).contains(&value) {
                            return Err(RuntimeError::value_error("byte must be in range(0, 256)"));
                        }
                        vec![value as u8]
                    }
                    Value::Bool(value) => vec![if value { 1 } else { 0 }],
                    other => bytes_like_from_value(other)?,
                };
                let len = bytes.len() as i64;
                let mut start = if let Some(value) = args.first() {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let mut end = if let Some(value) = args.get(1) {
                    value_to_int(value.clone())?
                } else {
                    len
                };
                if start < 0 {
                    start += len;
                }
                if end < 0 {
                    end += len;
                }
                start = start.clamp(0, len);
                end = end.clamp(0, len);
                if end < start {
                    return Ok(NativeCallResult::Value(Value::Int(0)));
                }
                let haystack = &bytes[start as usize..end as usize];
                if needle.is_empty() {
                    return Ok(NativeCallResult::Value(Value::Int(
                        haystack.len() as i64 + 1,
                    )));
                }
                let mut remaining = haystack;
                let mut count = 0i64;
                while let Some(index) = find_bytes_subslice(remaining, &needle) {
                    count += 1;
                    let next = index + needle.len();
                    remaining = &remaining[next..];
                }
                Ok(NativeCallResult::Value(Value::Int(count)))
            }
            NativeMethodKind::BytesFind | NativeMethodKind::BytesIndex => {
                if args.is_empty() || args.len() > 3 {
                    let method_name = if matches!(kind, NativeMethodKind::BytesIndex) {
                        "index"
                    } else {
                        "find"
                    };
                    return Err(RuntimeError::new(format!(
                        "{method_name}() expects sub, optional start, optional end",
                    )));
                }
                let bytes = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(value) => bytes_like_from_value(value.clone())?,
                        None => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let needle = match args.remove(0) {
                    Value::Int(value) => {
                        if !(0..=255).contains(&value) {
                            return Err(RuntimeError::value_error("byte must be in range(0, 256)"));
                        }
                        vec![value as u8]
                    }
                    Value::BigInt(value) => {
                        let Some(value) = value.to_i64() else {
                            return Err(RuntimeError::value_error("byte must be in range(0, 256)"));
                        };
                        if !(0..=255).contains(&value) {
                            return Err(RuntimeError::value_error("byte must be in range(0, 256)"));
                        }
                        vec![value as u8]
                    }
                    Value::Bool(value) => vec![if value { 1 } else { 0 }],
                    other => bytes_like_from_value(other)?,
                };
                let len = bytes.len() as i64;
                let mut start = if let Some(value) = args.first() {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let mut end = if let Some(value) = args.get(1) {
                    value_to_int(value.clone())?
                } else {
                    len
                };
                if start < 0 {
                    start += len;
                }
                if end < 0 {
                    end += len;
                }
                start = start.clamp(0, len);
                end = end.clamp(0, len);
                if end < start {
                    return Ok(NativeCallResult::Value(Value::Int(-1)));
                }
                let haystack = &bytes[start as usize..end as usize];
                let found = if needle.is_empty() {
                    Some(0usize)
                } else {
                    find_bytes_subslice(haystack, &needle)
                };
                if let Some(found) = found {
                    let index = found as i64 + start;
                    Ok(NativeCallResult::Value(Value::Int(index)))
                } else if matches!(kind, NativeMethodKind::BytesIndex) {
                    Err(RuntimeError::value_error("subsection not found"))
                } else {
                    Ok(NativeCallResult::Value(Value::Int(-1)))
                }
            }
            NativeMethodKind::BytesSplit => {
                let mut sep_kw = kwargs.remove("sep");
                let mut maxsplit_kw = kwargs.remove("maxsplit");
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "split() got an unexpected keyword argument",
                    ));
                }
                if args.len() > 2 {
                    return Err(RuntimeError::new("split() expects at most 2 arguments"));
                }
                let sep_arg = if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                };
                if sep_arg.is_some() && sep_kw.is_some() {
                    return Err(RuntimeError::new("split() got multiple values for sep"));
                }
                let sep_value = sep_arg.or_else(|| sep_kw.take());

                let maxsplit_arg = if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                };
                if maxsplit_arg.is_some() && maxsplit_kw.is_some() {
                    return Err(RuntimeError::new(
                        "split() got multiple values for maxsplit",
                    ));
                }
                let maxsplit = if let Some(value) = maxsplit_arg.or_else(|| maxsplit_kw.take()) {
                    value_to_int(value)?
                } else {
                    -1
                };

                let receiver_value = match &*receiver.kind() {
                    Object::Module(module_data) => module_data
                        .globals
                        .get("value")
                        .cloned()
                        .ok_or_else(|| RuntimeError::type_error("bytes receiver is invalid"))?,
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let bytes = bytes_like_from_value(receiver_value.clone())?;
                let output_bytearray = matches!(receiver_value, Value::ByteArray(_));

                let parts = match sep_value {
                    Some(Value::None) | None => {
                        let len = bytes.len();
                        let mut idx = 0usize;
                        while idx < len && bytes[idx].is_ascii_whitespace() {
                            idx += 1;
                        }
                        if idx == len {
                            Vec::new()
                        } else if maxsplit == 0 {
                            vec![bytes[idx..].to_vec()]
                        } else {
                            let mut out = Vec::new();
                            while idx < len {
                                if maxsplit >= 0 && out.len() as i64 == maxsplit {
                                    out.push(bytes[idx..].to_vec());
                                    break;
                                }
                                let start = idx;
                                while idx < len && !bytes[idx].is_ascii_whitespace() {
                                    idx += 1;
                                }
                                out.push(bytes[start..idx].to_vec());
                                while idx < len && bytes[idx].is_ascii_whitespace() {
                                    idx += 1;
                                }
                            }
                            out
                        }
                    }
                    Some(value) => {
                        let sep = bytes_like_from_value(value)?;
                        if sep.is_empty() {
                            return Err(RuntimeError::value_error("empty separator"));
                        }
                        if maxsplit == 0 {
                            vec![bytes]
                        } else {
                            let mut out = Vec::new();
                            let mut start = 0usize;
                            let mut splits = 0i64;
                            while start <= bytes.len() {
                                if maxsplit >= 0 && splits >= maxsplit {
                                    break;
                                }
                                let Some(rel) = find_bytes_subslice(&bytes[start..], &sep) else {
                                    break;
                                };
                                let split_at = start + rel;
                                out.push(bytes[start..split_at].to_vec());
                                start = split_at + sep.len();
                                splits += 1;
                            }
                            out.push(bytes[start..].to_vec());
                            out
                        }
                    }
                };

                let values = parts
                    .into_iter()
                    .map(|part| {
                        if output_bytearray {
                            self.heap.alloc_bytearray(part)
                        } else {
                            self.heap.alloc_bytes(part)
                        }
                    })
                    .collect();
                Ok(NativeCallResult::Value(self.heap.alloc_list(values)))
            }
            NativeMethodKind::BytesSplitLines => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "splitlines() expects at most one argument",
                    ));
                }
                let keepends = args
                    .first()
                    .map(|value| self.truthy_from_value(value))
                    .transpose()?
                    .unwrap_or(false);
                let (bytes, output_bytearray) = match &*receiver.kind() {
                    Object::Module(module_data) => {
                        let Some(value) = module_data.globals.get("value").cloned() else {
                            return Err(RuntimeError::type_error("bytes receiver is invalid"));
                        };
                        let bytes = bytes_like_from_value(value.clone())?;
                        let output_bytearray = matches!(value, Value::ByteArray(_));
                        (bytes, output_bytearray)
                    }
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let mut lines = Vec::new();
                let mut start = 0usize;
                let mut idx = 0usize;
                while idx < bytes.len() {
                    let byte = bytes[idx];
                    if byte == b'\n' || byte == b'\r' {
                        let mut break_end = idx + 1;
                        if byte == b'\r' && break_end < bytes.len() && bytes[break_end] == b'\n' {
                            break_end += 1;
                        }
                        let line_end = if keepends { break_end } else { idx };
                        let line = bytes[start..line_end].to_vec();
                        if output_bytearray {
                            lines.push(self.heap.alloc_bytearray(line));
                        } else {
                            lines.push(self.heap.alloc_bytes(line));
                        }
                        start = break_end;
                        idx = break_end;
                        continue;
                    }
                    idx += 1;
                }
                if start < bytes.len() {
                    let tail = bytes[start..].to_vec();
                    if output_bytearray {
                        lines.push(self.heap.alloc_bytearray(tail));
                    } else {
                        lines.push(self.heap.alloc_bytes(tail));
                    }
                }
                Ok(NativeCallResult::Value(self.heap.alloc_list(lines)))
            }
            NativeMethodKind::BytesTranslate => {
                let delete_kw = kwargs.remove("delete");
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "translate() got an unexpected keyword argument",
                    ));
                }
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new(
                        "translate() expects table and optional delete",
                    ));
                }
                if delete_kw.is_some() && args.len() > 1 {
                    return Err(RuntimeError::new(
                        "translate() got multiple values for delete",
                    ));
                }

                let table_arg = args.remove(0);
                let delete_arg = if let Some(value) = delete_kw {
                    value
                } else if let Some(value) = args.pop() {
                    value
                } else {
                    self.heap.alloc_bytes(Vec::new())
                };

                let receiver_value = match &*receiver.kind() {
                    Object::Module(module_data) => module_data
                        .globals
                        .get("value")
                        .cloned()
                        .ok_or_else(|| RuntimeError::type_error("bytes receiver is invalid"))?,
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let source = bytes_like_from_value(receiver_value.clone())?;
                let delete = bytes_like_from_value(delete_arg)?;

                let table = if matches!(table_arg, Value::None) {
                    None
                } else {
                    let table = bytes_like_from_value(table_arg)?;
                    if table.len() != 256 {
                        return Err(RuntimeError::new(
                            "translation table must be 256 characters long",
                        ));
                    }
                    Some(table)
                };

                let mut out = Vec::with_capacity(source.len());
                for byte in source {
                    if delete.contains(&byte) {
                        continue;
                    }
                    let mapped = if let Some(table) = &table {
                        table[byte as usize]
                    } else {
                        byte
                    };
                    out.push(mapped);
                }

                let translated = match receiver_value {
                    Value::ByteArray(_) => self.heap.alloc_bytearray(out),
                    _ => self.heap.alloc_bytes(out),
                };
                Ok(NativeCallResult::Value(translated))
            }
            NativeMethodKind::BytesJoin => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("join() expects one argument"));
                }
                let separator = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(value) => bytes_like_from_value(value.clone())?,
                        None => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let values = self.collect_iterable_values(args.remove(0))?;
                let mut output = Vec::new();
                for (idx, value) in values.into_iter().enumerate() {
                    let bytes = match bytes_like_from_value(value) {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            return Err(RuntimeError::new(
                                "sequence item is not a bytes-like object",
                            ));
                        }
                    };
                    if idx > 0 && !separator.is_empty() {
                        output.extend_from_slice(&separator);
                    }
                    output.extend_from_slice(&bytes);
                }
                Ok(NativeCallResult::Value(self.heap.alloc_bytes(output)))
            }
            NativeMethodKind::BytesLJust => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "ljust() got an unexpected keyword argument",
                    ));
                }
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new(
                        "ljust() expects width and optional fillbyte",
                    ));
                }
                let receiver_value = match &*receiver.kind() {
                    Object::Module(module_data) => module_data
                        .globals
                        .get("value")
                        .cloned()
                        .ok_or_else(|| RuntimeError::type_error("bytes receiver is invalid"))?,
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let bytes = bytes_like_from_value(receiver_value.clone())?;
                let width = value_to_int(args.remove(0))?;
                let width = if width <= 0 {
                    0usize
                } else {
                    usize::try_from(width)
                        .map_err(|_| RuntimeError::new("ljust() width is too large"))?
                };
                let fillbyte = if args.is_empty() {
                    b' '
                } else {
                    let fill = bytes_like_from_value(args.remove(0)).map_err(|_| {
                        RuntimeError::new(
                            "ljust() argument 2 must be a bytes-like object of length 1",
                        )
                    })?;
                    if fill.len() != 1 {
                        return Err(RuntimeError::new(
                            "ljust() argument 2 must be a bytes-like object of length 1",
                        ));
                    }
                    fill[0]
                };
                if width <= bytes.len() {
                    return Ok(NativeCallResult::Value(match receiver_value {
                        Value::ByteArray(_) => self.heap.alloc_bytearray(bytes),
                        _ => self.heap.alloc_bytes(bytes),
                    }));
                }
                let mut out = bytes;
                out.resize(width, fillbyte);
                Ok(NativeCallResult::Value(match receiver_value {
                    Value::ByteArray(_) => self.heap.alloc_bytearray(out),
                    _ => self.heap.alloc_bytes(out),
                }))
            }
            NativeMethodKind::BytesRStrip => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("rstrip() expects at most one argument"));
                }
                let receiver_value = match &*receiver.kind() {
                    Object::Module(module_data) => module_data
                        .globals
                        .get("value")
                        .cloned()
                        .ok_or_else(|| RuntimeError::type_error("bytes receiver is invalid"))?,
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let bytes = bytes_like_from_value(receiver_value.clone())?;
                let chars = if args.is_empty() || matches!(args[0], Value::None) {
                    None
                } else {
                    Some(bytes_like_from_value(args.remove(0))?)
                };
                let mut end = bytes.len();
                if let Some(chars) = chars {
                    while end > 0 && chars.contains(&bytes[end - 1]) {
                        end -= 1;
                    }
                } else {
                    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
                        end -= 1;
                    }
                }
                let out = bytes[..end].to_vec();
                Ok(NativeCallResult::Value(match receiver_value {
                    Value::ByteArray(_) => self.heap.alloc_bytearray(out),
                    _ => self.heap.alloc_bytes(out),
                }))
            }
            NativeMethodKind::BytesLStrip => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("lstrip() expects at most one argument"));
                }
                let receiver_value = match &*receiver.kind() {
                    Object::Module(module_data) => module_data
                        .globals
                        .get("value")
                        .cloned()
                        .ok_or_else(|| RuntimeError::type_error("bytes receiver is invalid"))?,
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let bytes = bytes_like_from_value(receiver_value.clone())?;
                let chars = if args.is_empty() || matches!(args[0], Value::None) {
                    None
                } else {
                    Some(bytes_like_from_value(args.remove(0))?)
                };
                let mut start = 0usize;
                if let Some(chars) = chars {
                    while start < bytes.len() && chars.contains(&bytes[start]) {
                        start += 1;
                    }
                } else {
                    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
                        start += 1;
                    }
                }
                let out = bytes[start..].to_vec();
                Ok(NativeCallResult::Value(match receiver_value {
                    Value::ByteArray(_) => self.heap.alloc_bytearray(out),
                    _ => self.heap.alloc_bytes(out),
                }))
            }
            NativeMethodKind::BytesStrip => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("strip() expects at most one argument"));
                }
                let receiver_value = match &*receiver.kind() {
                    Object::Module(module_data) => module_data
                        .globals
                        .get("value")
                        .cloned()
                        .ok_or_else(|| RuntimeError::type_error("bytes receiver is invalid"))?,
                    _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
                };
                let bytes = bytes_like_from_value(receiver_value.clone())?;
                let chars = if args.is_empty() || matches!(args[0], Value::None) {
                    None
                } else {
                    Some(bytes_like_from_value(args.remove(0))?)
                };
                let mut start = 0usize;
                let mut end = bytes.len();
                if let Some(chars) = chars {
                    while start < end && chars.contains(&bytes[start]) {
                        start += 1;
                    }
                    while end > start && chars.contains(&bytes[end - 1]) {
                        end -= 1;
                    }
                } else {
                    while start < end && bytes[start].is_ascii_whitespace() {
                        start += 1;
                    }
                    while end > start && bytes[end - 1].is_ascii_whitespace() {
                        end -= 1;
                    }
                }
                let out = bytes[start..end].to_vec();
                Ok(NativeCallResult::Value(match receiver_value {
                    Value::ByteArray(_) => self.heap.alloc_bytearray(out),
                    _ => self.heap.alloc_bytes(out),
                }))
            }
            NativeMethodKind::CodecsIncrementalEncoderFactoryCall => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "IncrementalEncoder() expects optional errors argument",
                    ));
                }
                let mut errors_arg = if args.is_empty() {
                    None
                } else {
                    Some(args.remove(0))
                };
                if let Some(value) = kwargs.remove("errors") {
                    if errors_arg.is_some() {
                        return Err(RuntimeError::new(
                            "IncrementalEncoder() got multiple values for errors",
                        ));
                    }
                    errors_arg = Some(value);
                }
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "IncrementalEncoder() got an unexpected keyword argument",
                    ));
                }
                let encoding = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("encoding") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::new("incremental encoder factory invalid")),
                    },
                    _ => return Err(RuntimeError::new("incremental encoder factory invalid")),
                };
                let errors =
                    normalize_codec_errors(errors_arg.unwrap_or(Value::Str("strict".to_string())))?;
                let encoder_class = self
                    .modules
                    .get("codecs")
                    .and_then(|module| match &*module.kind() {
                        Object::Module(module_data) => {
                            match module_data.globals.get("IncrementalEncoder") {
                                Some(Value::Class(class_obj)) => Some(class_obj.clone()),
                                _ => None,
                            }
                        }
                        _ => None,
                    })
                    .ok_or_else(|| RuntimeError::new("codecs.IncrementalEncoder unavailable"))?;
                let encoder = match self.heap.alloc_instance(InstanceObject::new(encoder_class)) {
                    Value::Instance(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Instance(instance_data) = &mut *encoder.kind_mut() {
                    instance_data
                        .attrs
                        .insert("__pyrs_codec_encoding__".to_string(), Value::Str(encoding));
                    instance_data
                        .attrs
                        .insert("__pyrs_codec_errors__".to_string(), Value::Str(errors));
                }
                Ok(NativeCallResult::Value(Value::Instance(encoder)))
            }
            NativeMethodKind::CodecsIncrementalDecoderFactoryCall => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "IncrementalDecoder() expects optional errors argument",
                    ));
                }
                let mut errors_arg = if args.is_empty() {
                    None
                } else {
                    Some(args.remove(0))
                };
                if let Some(value) = kwargs.remove("errors") {
                    if errors_arg.is_some() {
                        return Err(RuntimeError::new(
                            "IncrementalDecoder() got multiple values for errors",
                        ));
                    }
                    errors_arg = Some(value);
                }
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "IncrementalDecoder() got an unexpected keyword argument",
                    ));
                }
                let encoding = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("encoding") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::new("incremental decoder factory invalid")),
                    },
                    _ => return Err(RuntimeError::new("incremental decoder factory invalid")),
                };
                let errors =
                    normalize_codec_errors(errors_arg.unwrap_or(Value::Str("strict".to_string())))?;
                let decoder_class = self
                    .modules
                    .get("codecs")
                    .and_then(|module| match &*module.kind() {
                        Object::Module(module_data) => {
                            match module_data.globals.get("IncrementalDecoder") {
                                Some(Value::Class(class_obj)) => Some(class_obj.clone()),
                                _ => None,
                            }
                        }
                        _ => None,
                    })
                    .ok_or_else(|| RuntimeError::new("codecs.IncrementalDecoder unavailable"))?;
                let decoder = match self.heap.alloc_instance(InstanceObject::new(decoder_class)) {
                    Value::Instance(obj) => obj,
                    _ => unreachable!(),
                };
                let pending = self.heap.alloc_bytes(Vec::new());
                if let Object::Instance(instance_data) = &mut *decoder.kind_mut() {
                    instance_data
                        .attrs
                        .insert("__pyrs_codec_encoding__".to_string(), Value::Str(encoding));
                    instance_data
                        .attrs
                        .insert("__pyrs_codec_errors__".to_string(), Value::Str(errors));
                    instance_data
                        .attrs
                        .insert("__pyrs_codec_pending__".to_string(), pending);
                    instance_data
                        .attrs
                        .insert("__pyrs_codec_state_flag__".to_string(), Value::Int(0));
                }
                Ok(NativeCallResult::Value(Value::Instance(decoder)))
            }
            NativeMethodKind::CodecsIncrementalEncoderEncode => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new(
                        "encode() expects input and optional final argument",
                    ));
                }
                let mut final_arg = if args.len() == 2 {
                    Some(args.remove(1))
                } else {
                    None
                };
                if let Some(value) = kwargs.remove("final") {
                    if final_arg.is_some() {
                        return Err(RuntimeError::new("encode() got multiple values for final"));
                    }
                    final_arg = Some(value);
                }
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "encode() got an unexpected keyword argument",
                    ));
                }
                if let Some(value) = final_arg {
                    let _ = is_truthy(&value);
                }
                let input = args.remove(0);
                let text = match input {
                    Value::Str(text) => text,
                    _ => return Err(RuntimeError::new("encoder input must be str")),
                };
                let (encoding, errors) = match &*receiver.kind() {
                    Object::Module(module_data) => {
                        let encoding = match module_data.globals.get("encoding") {
                            Some(Value::Str(value)) => value.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "incremental encoder object is invalid",
                                ));
                            }
                        };
                        let errors = match module_data.globals.get("errors") {
                            Some(Value::Str(value)) => value.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "incremental encoder object is invalid",
                                ));
                            }
                        };
                        (encoding, errors)
                    }
                    _ => return Err(RuntimeError::new("incremental encoder object is invalid")),
                };
                let encoded = encode_text_bytes(&text, &encoding, &errors)?;
                Ok(NativeCallResult::Value(self.heap.alloc_bytes(encoded)))
            }
            NativeMethodKind::CodecsIncrementalEncoderReset => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("reset() expects no arguments"));
                }
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::CodecsIncrementalEncoderGetState => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("getstate() expects no arguments"));
                }
                Ok(NativeCallResult::Value(Value::Int(0)))
            }
            NativeMethodKind::CodecsIncrementalEncoderSetState => {
                let mut state_arg = if args.is_empty() {
                    None
                } else if args.len() == 1 {
                    Some(args.remove(0))
                } else {
                    return Err(RuntimeError::type_error("setstate() expects one argument"));
                };
                if let Some(value) = kwargs.remove("state") {
                    if state_arg.is_some() {
                        return Err(RuntimeError::new(
                            "setstate() got multiple values for state",
                        ));
                    }
                    state_arg = Some(value);
                }
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "setstate() got an unexpected keyword argument",
                    ));
                }
                let state = state_arg
                    .ok_or_else(|| RuntimeError::type_error("setstate() expects one argument"))?;
                let _ = value_to_int(state)?;
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::CodecsIncrementalDecoderDecode => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new(
                        "decode() expects input and optional final argument",
                    ));
                }
                let mut final_arg = if args.len() == 2 {
                    Some(args.remove(1))
                } else {
                    None
                };
                if let Some(value) = kwargs.remove("final") {
                    if final_arg.is_some() {
                        return Err(RuntimeError::new("decode() got multiple values for final"));
                    }
                    final_arg = Some(value);
                }
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "decode() got an unexpected keyword argument",
                    ));
                }
                let final_decode = if let Some(value) = final_arg {
                    is_truthy(&value)
                } else {
                    false
                };
                let input = bytes_like_from_value(args.remove(0))?;
                let (encoding, errors, pending, state_flag) = match &*receiver.kind() {
                    Object::Module(module_data) => {
                        let encoding = match module_data.globals.get("encoding") {
                            Some(Value::Str(value)) => value.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "incremental decoder object is invalid",
                                ));
                            }
                        };
                        let errors = match module_data.globals.get("errors") {
                            Some(Value::Str(value)) => value.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "incremental decoder object is invalid",
                                ));
                            }
                        };
                        let pending = match module_data.globals.get("pending") {
                            Some(value) => bytes_like_from_value(value.clone())?,
                            None => Vec::new(),
                        };
                        let state_flag = match module_data.globals.get("state_flag") {
                            Some(Value::Int(value)) => *value,
                            _ => 0,
                        };
                        (encoding, errors, pending, state_flag)
                    }
                    _ => return Err(RuntimeError::new("incremental decoder object is invalid")),
                };
                let mut combined = pending;
                combined.extend_from_slice(&input);
                let decode_result = if final_decode {
                    decode_text_bytes(&combined, &encoding, &errors).map(|text| (text, Vec::new()))
                } else {
                    let max_tail = match encoding.as_str() {
                        "utf-8" => 3usize,
                        "utf-16" | "utf-16-le" | "utf-16-be" => 1usize,
                        "utf-32" | "utf-32-le" | "utf-32-be" => 3usize,
                        _ => 0usize,
                    };
                    let mut success = None;
                    let max_try = max_tail.min(combined.len());
                    for tail_len in 0..=max_try {
                        let split_at = combined.len() - tail_len;
                        match decode_text_bytes(&combined[..split_at], &encoding, &errors) {
                            Ok(text) => {
                                success = Some((text, combined[split_at..].to_vec()));
                                break;
                            }
                            Err(_) => continue,
                        }
                    }
                    success.ok_or_else(|| RuntimeError::new("codec decode failed"))
                }?;
                let pending_value = self.heap.alloc_bytes(decode_result.1);
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("pending".to_string(), pending_value);
                    module_data
                        .globals
                        .insert("state_flag".to_string(), Value::Int(state_flag));
                }
                Ok(NativeCallResult::Value(Value::Str(decode_result.0)))
            }
            NativeMethodKind::CodecsIncrementalDecoderReset => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("reset() expects no arguments"));
                }
                let pending = self.heap.alloc_bytes(Vec::new());
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data.globals.insert("pending".to_string(), pending);
                    module_data
                        .globals
                        .insert("state_flag".to_string(), Value::Int(0));
                }
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::CodecsIncrementalDecoderGetState => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("getstate() expects no arguments"));
                }
                let (pending, state_flag) = match &*receiver.kind() {
                    Object::Module(module_data) => {
                        let pending = module_data
                            .globals
                            .get("pending")
                            .cloned()
                            .unwrap_or_else(|| self.heap.alloc_bytes(Vec::new()));
                        let state_flag = match module_data.globals.get("state_flag") {
                            Some(Value::Int(value)) => *value,
                            _ => 0,
                        };
                        (pending, state_flag)
                    }
                    _ => return Err(RuntimeError::new("incremental decoder object is invalid")),
                };
                Ok(NativeCallResult::Value(
                    self.heap.alloc_tuple(vec![pending, Value::Int(state_flag)]),
                ))
            }
            NativeMethodKind::CodecsIncrementalDecoderSetState => {
                let mut state_arg = if args.is_empty() {
                    None
                } else if args.len() == 1 {
                    Some(args.remove(0))
                } else {
                    return Err(RuntimeError::type_error("setstate() expects one argument"));
                };
                if let Some(value) = kwargs.remove("state") {
                    if state_arg.is_some() {
                        return Err(RuntimeError::new(
                            "setstate() got multiple values for state",
                        ));
                    }
                    state_arg = Some(value);
                }
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "setstate() got an unexpected keyword argument",
                    ));
                }
                let state = state_arg
                    .ok_or_else(|| RuntimeError::type_error("setstate() expects one argument"))?;
                let tuple_values = match state {
                    Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                        Object::Tuple(values) => values.clone(),
                        _ => return Err(RuntimeError::new("state must be a tuple")),
                    },
                    _ => return Err(RuntimeError::new("state must be a tuple")),
                };
                if tuple_values.len() != 2 {
                    return Err(RuntimeError::new("state must be a (buffer, flag) tuple"));
                }
                let pending_bytes = bytes_like_from_value(tuple_values[0].clone())?;
                let state_flag = value_to_int(tuple_values[1].clone())?;
                let pending = self.heap.alloc_bytes(pending_bytes);
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data.globals.insert("pending".to_string(), pending);
                    module_data
                        .globals
                        .insert("state_flag".to_string(), Value::Int(state_flag));
                }
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::ByteArrayAppend => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("append() takes exactly one argument"));
                }
                let item = self.io_index_arg_to_int(args.remove(0))?;
                if !(0..=255).contains(&item) {
                    return Err(RuntimeError::new(
                        "ValueError: byte must be in range(0, 256)",
                    ));
                }
                let buffer = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::ByteArray(obj)) => obj.clone(),
                        _ => return Err(RuntimeError::type_error("bytearray receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("bytearray receiver is invalid")),
                };
                let has_exports = self.heap.count_live_buffer_exports_for_source(&buffer) > 0;
                let Object::ByteArray(values) = &mut *buffer.kind_mut() else {
                    return Err(RuntimeError::type_error("bytearray receiver is invalid"));
                };
                if has_exports {
                    return Err(RuntimeError::new(
                        "BufferError: Existing exports of data: object cannot be re-sized",
                    ));
                }
                values.push(item as u8);
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::ByteArrayExtend => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("extend() takes exactly one argument"));
                }
                let source = args.remove(0);
                let mut extension = if matches!(source, Value::Int(_)) {
                    return Err(RuntimeError::new(
                        "TypeError: can't extend bytearray with int",
                    ));
                } else if matches!(source, Value::Str(_)) {
                    return Err(RuntimeError::new(
                        "TypeError: expected iterable of integers; got: 'str'",
                    ));
                } else {
                    self.value_to_bytes_payload(source)?
                };
                if extension.is_empty() {
                    return Ok(NativeCallResult::Value(Value::None));
                }
                let buffer = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::ByteArray(obj)) => obj.clone(),
                        _ => return Err(RuntimeError::type_error("bytearray receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("bytearray receiver is invalid")),
                };
                let has_exports = self.heap.count_live_buffer_exports_for_source(&buffer) > 0;
                let Object::ByteArray(values) = &mut *buffer.kind_mut() else {
                    return Err(RuntimeError::type_error("bytearray receiver is invalid"));
                };
                if has_exports {
                    return Err(RuntimeError::new(
                        "BufferError: Existing exports of data: object cannot be re-sized",
                    ));
                }
                values.append(&mut extension);
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::ByteArrayClear => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("clear() expects no arguments"));
                }
                let buffer = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::ByteArray(obj)) => obj.clone(),
                        _ => return Err(RuntimeError::type_error("bytearray receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("bytearray receiver is invalid")),
                };
                let has_exports = self.heap.count_live_buffer_exports_for_source(&buffer) > 0;
                let Object::ByteArray(values) = &mut *buffer.kind_mut() else {
                    return Err(RuntimeError::type_error("bytearray receiver is invalid"));
                };
                if !values.is_empty() && has_exports {
                    return Err(RuntimeError::new(
                        "BufferError: Existing exports of data: object cannot be re-sized",
                    ));
                }
                values.clear();
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::ByteArrayResize => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("resize() takes exactly one argument"));
                }
                let buffer = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::ByteArray(obj)) => obj.clone(),
                        _ => return Err(RuntimeError::type_error("bytearray receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("bytearray receiver is invalid")),
                };
                let new_size = self.io_index_arg_to_int(args.remove(0))?;
                if new_size < 0 {
                    return Err(RuntimeError::new(
                        "ValueError: new size must be non-negative",
                    ));
                }
                let has_exports = self.heap.count_live_buffer_exports_for_source(&buffer) > 0;
                let Object::ByteArray(values) = &mut *buffer.kind_mut() else {
                    return Err(RuntimeError::type_error("bytearray receiver is invalid"));
                };
                if has_exports && values.len() != new_size as usize {
                    return Err(RuntimeError::new(
                        "BufferError: Existing exports of data: object cannot be re-sized",
                    ));
                }
                values.resize(new_size as usize, 0);
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::MemoryViewEnter => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("__enter__() expects no arguments"));
                }
                Ok(NativeCallResult::Value(Value::MemoryView(receiver)))
            }
            NativeMethodKind::MemoryViewExit => {
                if !args.is_empty() && args.len() != 3 {
                    return Err(RuntimeError::new("__exit__() expects 3 arguments"));
                }
                if let Object::MemoryView(view) = &mut *receiver.kind_mut() {
                    if !view.released {
                        view.released = true;
                        view.export_owner = None;
                    }
                } else {
                    return Err(RuntimeError::type_error("memoryview receiver is invalid"));
                }
                Ok(NativeCallResult::Value(Value::Bool(false)))
            }
            NativeMethodKind::MemoryViewToReadOnly => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("toreadonly() expects no arguments"));
                }
                let (itemsize, format) = match &*receiver.kind() {
                    Object::MemoryView(view_data) => (view_data.itemsize, view_data.format.clone()),
                    _ => return Err(RuntimeError::type_error("memoryview receiver is invalid")),
                };
                let bytes = self.value_to_bytes_payload(Value::MemoryView(receiver.clone()))?;
                let source = match self.heap.alloc_bytes(bytes) {
                    Value::Bytes(obj) => obj,
                    _ => unreachable!(),
                };
                Ok(NativeCallResult::Value(
                    self.heap.alloc_memoryview_with(source, itemsize, format),
                ))
            }
            NativeMethodKind::MemoryViewCast => {
                let total_args = args.len() + kwargs.len();
                if total_args > 2 {
                    if args.is_empty() {
                        return Err(RuntimeError::type_error(format!(
                            "cast() takes at most 2 keyword arguments ({} given)",
                            kwargs.len()
                        )));
                    }
                    return Err(RuntimeError::type_error(format!(
                        "cast() takes at most 2 arguments ({} given)",
                        total_args
                    )));
                }
                let mut format_arg = args.first().cloned();
                let mut shape_arg = args.get(1).cloned();
                for (name, value) in kwargs {
                    match name.as_str() {
                        "format" => {
                            if format_arg.is_some() {
                                return Err(RuntimeError::type_error(
                                    "argument for cast() given by name ('format') and position (1)",
                                ));
                            }
                            format_arg = Some(value);
                        }
                        "shape" => {
                            if shape_arg.is_some() {
                                return Err(RuntimeError::type_error(
                                    "argument for cast() given by name ('shape') and position (2)",
                                ));
                            }
                            shape_arg = Some(value);
                        }
                        _ => {
                            return Err(RuntimeError::type_error(format!(
                                "cast() got an unexpected keyword argument '{name}'"
                            )));
                        }
                    }
                }
                let format = match format_arg {
                    Some(Value::Str(value)) => value,
                    Some(_) => {
                        return Err(RuntimeError::type_error(
                            "memoryview.cast() format must be str",
                        ));
                    }
                    None => {
                        return Err(RuntimeError::type_error(
                            "cast() missing required argument 'format' (pos 1)",
                        ));
                    }
                };
                let cast_format = parse_memoryview_cast_format(&format)
                    .ok_or_else(|| RuntimeError::new("memoryview.cast() unsupported format"))?;
                let itemsize = cast_format.itemsize();
                let (source, start, length, contiguous) = match &*receiver.kind() {
                    Object::MemoryView(view_data) => (
                        view_data.source.clone(),
                        view_data.start,
                        view_data.length,
                        view_data.contiguous,
                    ),
                    _ => return Err(RuntimeError::type_error("memoryview receiver is invalid")),
                };
                if !contiguous {
                    return Err(RuntimeError::new(
                        "memoryview: casts are restricted to C-contiguous views",
                    ));
                }
                let byte_len = with_bytes_like_source(&source, |values| {
                    let (start, end) = memoryview_bounds(start, length, values.len());
                    end.saturating_sub(start)
                })
                .ok_or_else(|| RuntimeError::type_error("memoryview receiver is invalid"))?;
                let shape_override = if let Some(shape_value) = shape_arg {
                    let shape_dims = parse_memoryview_cast_shape(&shape_value)?;
                    if shape_dims.is_empty() {
                        return Err(RuntimeError::new(
                            "memoryview: product(shape) * itemsize != buffer size",
                        ));
                    }
                    if !shape_product_matches_buffer_len(&shape_dims, itemsize, byte_len) {
                        return Err(RuntimeError::new(
                            "memoryview: product(shape) * itemsize != buffer size",
                        ));
                    }
                    Some((
                        shape_dims
                            .iter()
                            .map(|dim| *dim as isize)
                            .collect::<Vec<isize>>(),
                        c_contiguous_strides_for_shape(&shape_dims, itemsize)?,
                    ))
                } else {
                    if byte_len % itemsize != 0 {
                        return Err(RuntimeError::new(
                            "memoryview.cast() length is not a multiple of itemsize",
                        ));
                    }
                    None
                };
                let view = self
                    .heap
                    .alloc_memoryview_with(source, itemsize, Some(format));
                if let Value::MemoryView(view_obj) = &view
                    && let Object::MemoryView(view_data) = &mut *view_obj.kind_mut()
                {
                    view_data.start = start;
                    view_data.length = length;
                    if let Some((shape, strides)) = shape_override {
                        view_data.shape = Some(shape);
                        view_data.strides = Some(strides);
                    }
                }
                Ok(NativeCallResult::Value(view))
            }
            NativeMethodKind::MemoryViewToList => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("tolist() expects no arguments"));
                }
                let (source, start, length, itemsize, format, shape, strides) = match &*receiver
                    .kind()
                {
                    Object::MemoryView(view_data) => (
                        view_data.source.clone(),
                        view_data.start,
                        view_data.length,
                        view_data.itemsize.max(1),
                        view_data.format.clone(),
                        view_data.shape.clone(),
                        view_data.strides.clone(),
                    ),
                    _ => return Err(RuntimeError::type_error("memoryview receiver is invalid")),
                };
                let cast_format =
                    memoryview_format_for_view(itemsize, format.as_deref()).map_err(|_| {
                        RuntimeError::not_implemented_error("memoryview: unsupported format")
                    })?;
                with_bytes_like_source(&source, |values| {
                    let (shape, strides) = memoryview_shape_and_strides_from_parts(
                        start,
                        length,
                        shape.as_ref(),
                        strides.as_ref(),
                        itemsize,
                        values.len(),
                    )
                    .ok_or_else(|| {
                        RuntimeError::not_implemented_error("memoryview: unsupported format")
                    })?;
                    memoryview_decode_tolist(
                        values,
                        start,
                        itemsize,
                        cast_format,
                        &shape,
                        &strides,
                        &self.heap,
                    )
                })
                .unwrap_or_else(|| {
                    Err(RuntimeError::not_implemented_error(
                        "memoryview: unsupported format",
                    ))
                })
                .map(NativeCallResult::Value)
            }
            NativeMethodKind::MemoryViewRelease => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("release() expects no arguments"));
                }
                if let Object::MemoryView(view) = &mut *receiver.kind_mut() {
                    if !view.released {
                        view.released = true;
                        view.export_owner = None;
                    }
                } else {
                    return Err(RuntimeError::type_error("memoryview receiver is invalid"));
                }
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::StrRemovePrefix => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("removeprefix() expects one argument"));
                }
                let prefix = match args.first().expect("checked len") {
                    Value::Str(value) => value.as_str(),
                    _ => return Err(RuntimeError::new("removeprefix() argument must be str")),
                };
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let value = match text.strip_prefix(prefix) {
                    Some(stripped) => stripped.to_string(),
                    None => text,
                };
                Ok(NativeCallResult::Value(Value::Str(value)))
            }
            NativeMethodKind::StrRemoveSuffix => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("removesuffix() expects one argument"));
                }
                let suffix = match args.first().expect("checked len") {
                    Value::Str(value) => value.as_str(),
                    _ => return Err(RuntimeError::new("removesuffix() argument must be str")),
                };
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let value = match text.strip_suffix(suffix) {
                    Some(stripped) => stripped.to_string(),
                    None => text,
                };
                Ok(NativeCallResult::Value(Value::Str(value)))
            }
            NativeMethodKind::StrFormat => {
                let template = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let parsed = parse_string_formatter(&template)?;
                let mut out = String::new();
                let mut auto_index = 0usize;

                for (literal, field_name, format_spec, conversion) in parsed {
                    out.push_str(&literal);
                    let Some(field_name) = field_name else {
                        continue;
                    };

                    let (first, rest) = split_formatter_field_name(&field_name)?;
                    let mut value = match first {
                        FormatterFieldKey::Int(idx) => {
                            if idx < 0 {
                                return Err(RuntimeError::new(
                                    "negative format field indexes are not supported",
                                ));
                            }
                            args.get(idx as usize).cloned().ok_or_else(|| {
                                RuntimeError::new("format field index out of range")
                            })?
                        }
                        FormatterFieldKey::Str(name) => {
                            if name.is_empty() {
                                let value = args.get(auto_index).cloned().ok_or_else(|| {
                                    RuntimeError::new("format field index out of range")
                                })?;
                                auto_index += 1;
                                value
                            } else {
                                kwargs.get(&name).cloned().ok_or_else(|| {
                                    RuntimeError::new("missing format keyword argument")
                                })?
                            }
                        }
                    };

                    for (is_attr, key) in rest {
                        if is_attr {
                            let name = match key {
                                FormatterFieldKey::Int(idx) => idx.to_string(),
                                FormatterFieldKey::Str(name) => name,
                            };
                            value = self
                                .builtin_getattr(vec![value, Value::Str(name)], HashMap::new())?;
                        } else {
                            let index = match key {
                                FormatterFieldKey::Int(idx) => Value::Int(idx),
                                FormatterFieldKey::Str(name) => Value::Str(name),
                            };
                            value = self.getitem_value(value, index)?;
                        }
                    }

                    if let Some(conv) = conversion {
                        value = match conv.as_str() {
                            "" | "s" => self.builtin_str(vec![value], HashMap::new())?,
                            "r" => self.builtin_repr(vec![value], HashMap::new())?,
                            "a" => self.builtin_ascii(vec![value], HashMap::new())?,
                            _ => {
                                return Err(RuntimeError::new(
                                    "unsupported format conversion specifier",
                                ));
                            }
                        };
                    }
                    let rendered = match self.builtin_format(
                        vec![value, Value::Str(format_spec.unwrap_or_default())],
                        HashMap::new(),
                    )? {
                        Value::Str(text) => text,
                        _ => return Err(RuntimeError::new("format() returned non-string")),
                    };
                    out.push_str(&rendered);
                }
                Ok(NativeCallResult::Value(Value::Str(out)))
            }
            NativeMethodKind::StrIsUpper => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "isupper")?;
                let mut has_upper = false;
                for ch in text.chars() {
                    if ch.is_lowercase() {
                        return Ok(NativeCallResult::Value(Value::Bool(false)));
                    }
                    if ch.is_uppercase() {
                        has_upper = true;
                    }
                }
                Ok(NativeCallResult::Value(Value::Bool(has_upper)))
            }
            NativeMethodKind::StrIsLower => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "islower")?;
                let mut has_lower = false;
                for ch in text.chars() {
                    if ch.is_uppercase() {
                        return Ok(NativeCallResult::Value(Value::Bool(false)));
                    }
                    if ch.is_lowercase() {
                        has_lower = true;
                    }
                }
                Ok(NativeCallResult::Value(Value::Bool(has_lower)))
            }
            NativeMethodKind::StrIsAscii => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "isascii")?;
                Ok(NativeCallResult::Value(Value::Bool(text.is_ascii())))
            }
            NativeMethodKind::StrIsAlpha => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "isalpha")?;
                let is_alpha = !text.is_empty() && text.chars().all(|ch| ch.is_alphabetic());
                Ok(NativeCallResult::Value(Value::Bool(is_alpha)))
            }
            NativeMethodKind::StrIsDigit => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "isdigit")?;
                let is_digit = !text.is_empty() && text.chars().all(|ch| ch.is_numeric());
                Ok(NativeCallResult::Value(Value::Bool(is_digit)))
            }
            NativeMethodKind::StrIsAlNum => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "isalnum")?;
                let is_alnum = !text.is_empty() && text.chars().all(|ch| ch.is_alphanumeric());
                Ok(NativeCallResult::Value(Value::Bool(is_alnum)))
            }
            NativeMethodKind::StrIsSpace => {
                let text = self.str_predicate_receiver_text(&receiver, &mut args, "isspace")?;
                let is_space = !text.is_empty() && text.chars().all(|ch| ch.is_whitespace());
                Ok(NativeCallResult::Value(Value::Bool(is_space)))
            }
            NativeMethodKind::StrIsIdentifier => {
                let text =
                    self.str_predicate_receiver_text(&receiver, &mut args, "isidentifier")?;
                let mut chars = text.chars();
                let Some(first) = chars.next() else {
                    return Ok(NativeCallResult::Value(Value::Bool(false)));
                };
                if first != '_' && !first.is_alphabetic() {
                    return Ok(NativeCallResult::Value(Value::Bool(false)));
                }
                let is_identifier = chars.all(|ch| ch == '_' || ch.is_alphanumeric());
                Ok(NativeCallResult::Value(Value::Bool(is_identifier)))
            }
            NativeMethodKind::StrJoin => {
                let Object::Module(module_data) = &*receiver.kind() else {
                    return Err(RuntimeError::type_error("str receiver is invalid"));
                };
                let (separator, iterable_arg) = if let Some(Value::Str(value)) =
                    module_data.globals.get("value")
                {
                    if args.len() != 1 {
                        return Err(RuntimeError::new("join() expects one argument"));
                    }
                    (value.clone(), args.remove(0))
                } else if matches!(
                    module_data.globals.get("owner"),
                    Some(Value::Builtin(BuiltinFunction::Str))
                ) {
                    if args.len() != 2 {
                        return Err(RuntimeError::new(
                            "join() descriptor requires a str argument",
                        ));
                    }
                    let separator = match args.remove(0) {
                        Value::Str(value) => value,
                        Value::Instance(instance) => self
                            .instance_backing_str(&instance)
                            .ok_or_else(|| RuntimeError::type_error("str receiver is invalid"))?,
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    };
                    (separator, args.remove(0))
                } else {
                    return Err(RuntimeError::type_error("str receiver is invalid"));
                };
                let values = self.collect_iterable_values(iterable_arg)?;
                let mut parts = Vec::with_capacity(values.len());
                for (index, value) in values.into_iter().enumerate() {
                    match value {
                        Value::Str(text) => parts.push(text),
                        Value::Instance(instance) => {
                            if let Some(text) = self.instance_backing_str(&instance) {
                                parts.push(text);
                            } else {
                                return Err(RuntimeError::type_error(format!(
                                    "sequence item {index}: expected str instance, {} found",
                                    self.value_type_name_for_error(&Value::Instance(instance))
                                )));
                            }
                        }
                        _ => {
                            return Err(RuntimeError::type_error(format!(
                                "sequence item {index}: expected str instance, {} found",
                                self.value_type_name_for_error(&value)
                            )));
                        }
                    }
                }
                Ok(NativeCallResult::Value(Value::Str(parts.join(&separator))))
            }
            NativeMethodKind::StrSplit => {
                let sep_kw = kwargs.remove("sep");
                let maxsplit_kw = kwargs.remove("maxsplit");
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "split() got an unexpected keyword argument",
                    ));
                }
                if args.len() > 2 {
                    return Err(RuntimeError::new("split() expects at most two arguments"));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                if sep_kw.is_some() && !args.is_empty() {
                    return Err(RuntimeError::new("split() got multiple values for sep"));
                }
                if maxsplit_kw.is_some() && args.len() > 1 {
                    return Err(RuntimeError::new(
                        "split() got multiple values for maxsplit",
                    ));
                }
                let sep_arg = if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    sep_kw
                };
                let maxsplit_arg = if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    maxsplit_kw
                };
                let sep = match sep_arg {
                    Some(Value::Str(value)) => Some(value),
                    Some(Value::None) | None => None,
                    Some(_) => return Err(RuntimeError::new("split() separator must be str")),
                };
                let maxsplit = if let Some(value) = maxsplit_arg {
                    value_to_int(value)?
                } else {
                    -1
                };
                let parts: Vec<Value> = if let Some(sep) = sep {
                    if sep.is_empty() {
                        return Err(RuntimeError::new("empty separator"));
                    }
                    if maxsplit < 0 {
                        text.split(&sep)
                            .map(|part| Value::Str(part.to_string()))
                            .collect()
                    } else {
                        text.splitn((maxsplit + 1) as usize, &sep)
                            .map(|part| Value::Str(part.to_string()))
                            .collect()
                    }
                } else {
                    py_split_whitespace(&text, maxsplit)
                        .into_iter()
                        .map(Value::Str)
                        .collect()
                };
                Ok(NativeCallResult::Value(self.heap.alloc_list(parts)))
            }
            NativeMethodKind::StrSplitLines => {
                let keepends_kw = kwargs.remove("keepends");
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "splitlines() got an unexpected keyword argument",
                    ));
                }
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "splitlines() expects at most one argument",
                    ));
                }
                if keepends_kw.is_some() && !args.is_empty() {
                    return Err(RuntimeError::new(
                        "splitlines() got multiple values for keepends",
                    ));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let keepends = match args.into_iter().next().or(keepends_kw) {
                    Some(value) => is_truthy(&value),
                    None => false,
                };
                let parts = py_splitlines(&text, keepends)
                    .into_iter()
                    .map(Value::Str)
                    .collect::<Vec<_>>();
                Ok(NativeCallResult::Value(self.heap.alloc_list(parts)))
            }
            NativeMethodKind::StrRSplit => {
                let sep_kw = kwargs.remove("sep");
                let maxsplit_kw = kwargs.remove("maxsplit");
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(
                        "rsplit() got an unexpected keyword argument",
                    ));
                }
                if args.len() > 2 {
                    return Err(RuntimeError::new("rsplit() expects at most two arguments"));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                if sep_kw.is_some() && !args.is_empty() {
                    return Err(RuntimeError::new("rsplit() got multiple values for sep"));
                }
                if maxsplit_kw.is_some() && args.len() > 1 {
                    return Err(RuntimeError::new(
                        "rsplit() got multiple values for maxsplit",
                    ));
                }
                let sep_arg = if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    sep_kw
                };
                let maxsplit_arg = if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    maxsplit_kw
                };
                let sep = match sep_arg {
                    Some(Value::Str(value)) => Some(value),
                    Some(Value::None) | None => None,
                    Some(_) => return Err(RuntimeError::new("rsplit() separator must be str")),
                };
                let maxsplit = if let Some(value) = maxsplit_arg {
                    value_to_int(value)?
                } else {
                    -1
                };
                let parts: Vec<Value> = if let Some(sep) = sep {
                    if sep.is_empty() {
                        return Err(RuntimeError::new("empty separator"));
                    }
                    if maxsplit < 0 {
                        text.split(&sep)
                            .map(|part| Value::Str(part.to_string()))
                            .collect()
                    } else {
                        let mut reversed: Vec<Value> = text
                            .rsplitn((maxsplit + 1) as usize, &sep)
                            .map(|part| Value::Str(part.to_string()))
                            .collect();
                        reversed.reverse();
                        reversed
                    }
                } else {
                    py_rsplit_whitespace(&text, maxsplit)
                        .into_iter()
                        .map(Value::Str)
                        .collect()
                };
                Ok(NativeCallResult::Value(self.heap.alloc_list(parts)))
            }
            NativeMethodKind::StrPartition => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("partition() expects one argument"));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let sep = match &args[0] {
                    Value::Str(value) => value.clone(),
                    _ => return Err(RuntimeError::new("partition() separator must be str")),
                };
                if sep.is_empty() {
                    return Err(RuntimeError::new("empty separator"));
                }
                let parts = if let Some(index) = text.find(&sep) {
                    vec![
                        Value::Str(text[..index].to_string()),
                        Value::Str(sep.clone()),
                        Value::Str(text[index + sep.len()..].to_string()),
                    ]
                } else {
                    vec![
                        Value::Str(text),
                        Value::Str(String::new()),
                        Value::Str(String::new()),
                    ]
                };
                Ok(NativeCallResult::Value(self.heap.alloc_tuple(parts)))
            }
            NativeMethodKind::StrRPartition => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("rpartition() expects one argument"));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let sep = match &args[0] {
                    Value::Str(value) => value.clone(),
                    _ => return Err(RuntimeError::new("rpartition() separator must be str")),
                };
                if sep.is_empty() {
                    return Err(RuntimeError::new("empty separator"));
                }
                let parts = if let Some(index) = text.rfind(&sep) {
                    vec![
                        Value::Str(text[..index].to_string()),
                        Value::Str(sep.clone()),
                        Value::Str(text[index + sep.len()..].to_string()),
                    ]
                } else {
                    vec![
                        Value::Str(String::new()),
                        Value::Str(String::new()),
                        Value::Str(text),
                    ]
                };
                Ok(NativeCallResult::Value(self.heap.alloc_tuple(parts)))
            }
            NativeMethodKind::StrCount => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::type_error(
                        "count() takes no keyword arguments",
                    ));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => {
                        if let Some(Value::Str(value)) = module_data.globals.get("value") {
                            value.clone()
                        } else {
                            if args.is_empty() || args.len() > 4 {
                                return Err(RuntimeError::new(
                                    "count() expects sub, optional start, optional end",
                                ));
                            }
                            match args.remove(0) {
                                Value::Str(value) => value,
                                Value::Instance(instance) => {
                                    self.instance_backing_str(&instance).ok_or_else(|| {
                                        RuntimeError::type_error("str receiver is invalid")
                                    })?
                                }
                                _ => {
                                    return Err(RuntimeError::type_error(
                                        "str receiver is invalid",
                                    ));
                                }
                            }
                        }
                    }
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                if args.is_empty() || args.len() > 3 {
                    return Err(RuntimeError::new(
                        "count() expects sub, optional start, optional end",
                    ));
                }
                let needle = match &args[0] {
                    Value::Str(value) => value.clone(),
                    _ => return Err(RuntimeError::new("count() substring must be str")),
                };
                let char_len = text.chars().count();
                let len = char_len as i64;
                let mut start = if let Some(value) = args.get(1) {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let mut end = if let Some(value) = args.get(2) {
                    value_to_int(value.clone())?
                } else {
                    len
                };
                if start < 0 {
                    start += len;
                }
                if end < 0 {
                    end += len;
                }
                start = start.clamp(0, len);
                end = end.clamp(0, len);
                if end < start {
                    return Ok(NativeCallResult::Value(Value::Int(0)));
                }
                let start_idx = start as usize;
                let end_idx = end as usize;
                let Some(slice) = text.get(start_idx..end_idx) else {
                    return Ok(NativeCallResult::Value(Value::Int(0)));
                };
                if needle.is_empty() {
                    let count = slice.chars().count() as i64 + 1;
                    return Ok(NativeCallResult::Value(Value::Int(count)));
                }
                let mut remaining = slice;
                let mut count = 0i64;
                while let Some(index) = remaining.find(&needle) {
                    count += 1;
                    let next = index + needle.len();
                    remaining = &remaining[next..];
                }
                Ok(NativeCallResult::Value(Value::Int(count)))
            }
            NativeMethodKind::StrFind | NativeMethodKind::StrIndex | NativeMethodKind::StrRFind => {
                let method_name = match kind {
                    NativeMethodKind::StrFind => "find",
                    NativeMethodKind::StrIndex => "index",
                    NativeMethodKind::StrRFind => "rfind",
                    _ => unreachable!(),
                };
                if !kwargs.is_empty() {
                    return Err(RuntimeError::type_error(format!(
                        "{}() takes no keyword arguments",
                        method_name
                    )));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => {
                        if let Some(Value::Str(value)) = module_data.globals.get("value") {
                            value.clone()
                        } else {
                            if args.is_empty() {
                                return Err(RuntimeError::new(format!(
                                    "{}() expects sub, optional start, optional end",
                                    method_name
                                )));
                            }
                            match args.remove(0) {
                                Value::Str(value) => value,
                                _ => {
                                    return Err(RuntimeError::type_error(
                                        "str receiver is invalid",
                                    ));
                                }
                            }
                        }
                    }
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                if args.is_empty() || args.len() > 3 {
                    return Err(RuntimeError::new(format!(
                        "{}() expects sub, optional start, optional end",
                        method_name
                    )));
                }
                let needle = match &args[0] {
                    Value::Str(value) => value.clone(),
                    _ => {
                        return Err(RuntimeError::new(format!(
                            "{}() substring must be str",
                            method_name
                        )));
                    }
                };
                let char_len = text.chars().count();
                let len = char_len as i64;
                let mut start = if let Some(value) = args.get(1) {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let mut end = if let Some(value) = args.get(2) {
                    value_to_int(value.clone())?
                } else {
                    len
                };
                if start < 0 {
                    start += len;
                }
                if end < 0 {
                    end += len;
                }
                start = start.clamp(0, len);
                end = end.clamp(0, len);
                if end < start {
                    return Ok(NativeCallResult::Value(Value::Int(-1)));
                }
                let start_idx = start as usize;
                let end_idx = end as usize;
                let mut byte_offsets: Vec<usize> =
                    text.char_indices().map(|(idx, _)| idx).collect();
                byte_offsets.push(text.len());
                if start_idx > char_len || end_idx > char_len {
                    return Ok(NativeCallResult::Value(Value::Int(-1)));
                }
                let start_byte = byte_offsets[start_idx];
                let end_byte = byte_offsets[end_idx];
                let Some(slice) = text.get(start_byte..end_byte) else {
                    return Ok(NativeCallResult::Value(Value::Int(-1)));
                };
                let found = if matches!(kind, NativeMethodKind::StrRFind) {
                    slice.rfind(&needle)
                } else {
                    slice.find(&needle)
                };
                let found = found
                    .map(|idx| {
                        let absolute_byte = start_byte + idx;
                        text[..absolute_byte].chars().count() as i64
                    })
                    .unwrap_or(-1);
                if matches!(kind, NativeMethodKind::StrIndex) && found < 0 {
                    return Err(RuntimeError::value_error("substring not found"));
                }
                Ok(NativeCallResult::Value(Value::Int(found)))
            }
            NativeMethodKind::StrTranslate => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("translate() expects one argument"));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let table = args.remove(0);
                let mut out = String::with_capacity(text.len());
                for ch in text.chars() {
                    let code = ch as u32 as i64;
                    let mapped = match &table {
                        Value::Dict(dict_obj) => dict_get_value(dict_obj, &Value::Int(code))
                            .or_else(|| dict_get_value(dict_obj, &Value::Str(ch.to_string()))),
                        _ => match self.getitem_value(table.clone(), Value::Int(code)) {
                            Ok(value) => Some(value),
                            Err(err) if runtime_error_matches_exception(&err, "KeyError") => None,
                            Err(err) => return Err(err),
                        },
                    };
                    let Some(mapped) = mapped else {
                        out.push(ch);
                        continue;
                    };
                    match mapped {
                        Value::None => {}
                        Value::Str(fragment) => out.push_str(&fragment),
                        Value::Int(number) => {
                            if !(0..=0x10FFFF).contains(&number) {
                                return Err(RuntimeError::new(
                                    "character mapping must be in range(0x110000)",
                                ));
                            }
                            let Some(mapped_char) = char::from_u32(number as u32) else {
                                return Err(RuntimeError::new(
                                    "character mapping must be in range(0x110000)",
                                ));
                            };
                            out.push(mapped_char);
                        }
                        Value::BigInt(number) => {
                            let Some(number) = number.to_i64() else {
                                return Err(RuntimeError::new(
                                    "character mapping must be in range(0x110000)",
                                ));
                            };
                            if !(0..=0x10FFFF).contains(&number) {
                                return Err(RuntimeError::new(
                                    "character mapping must be in range(0x110000)",
                                ));
                            }
                            let Some(mapped_char) = char::from_u32(number as u32) else {
                                return Err(RuntimeError::new(
                                    "character mapping must be in range(0x110000)",
                                ));
                            };
                            out.push(mapped_char);
                        }
                        Value::Bool(value) => {
                            let mapped_char = if value { '\u{1}' } else { '\0' };
                            out.push(mapped_char);
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "character mapping must return integer, str or None",
                            ));
                        }
                    }
                }
                Ok(NativeCallResult::Value(Value::Str(out)))
            }
            NativeMethodKind::StrLStrip => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("lstrip() expects at most one argument"));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let stripped = match args.first() {
                    None | Some(Value::None) => text.trim_start().to_string(),
                    Some(Value::Str(chars)) => {
                        if chars.is_empty() {
                            text
                        } else {
                            text.trim_start_matches(|ch| chars.contains(ch)).to_string()
                        }
                    }
                    Some(_) => return Err(RuntimeError::new("lstrip() chars must be str or None")),
                };
                Ok(NativeCallResult::Value(Value::Str(stripped)))
            }
            NativeMethodKind::StrLJust => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new(
                        "ljust() expects width and optional fillchar",
                    ));
                }
                let width = value_to_int(args[0].clone())?;
                let fillchar = if args.len() == 2 {
                    match &args[1] {
                        Value::Str(text) => {
                            let mut chars = text.chars();
                            let Some(ch) = chars.next() else {
                                return Err(RuntimeError::new(
                                    "The fill character must be exactly one character long",
                                ));
                            };
                            if chars.next().is_some() {
                                return Err(RuntimeError::new(
                                    "The fill character must be exactly one character long",
                                ));
                            }
                            ch
                        }
                        _ => return Err(RuntimeError::new("ljust() fillchar must be str")),
                    }
                } else {
                    ' '
                };
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let text_len = text.chars().count() as i64;
                if width <= text_len {
                    return Ok(NativeCallResult::Value(Value::Str(text)));
                }
                let pad_len = usize::try_from(width - text_len)
                    .map_err(|_| RuntimeError::new("ljust() width is too large"))?;
                let mut out = String::with_capacity(text.len() + pad_len * fillchar.len_utf8());
                out.push_str(&text);
                for _ in 0..pad_len {
                    out.push(fillchar);
                }
                Ok(NativeCallResult::Value(Value::Str(out)))
            }
            NativeMethodKind::StrCenter => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new(
                        "center() expects width and optional fillchar",
                    ));
                }
                let width = value_to_int(args[0].clone())?;
                let fillchar = if args.len() == 2 {
                    match &args[1] {
                        Value::Str(text) => {
                            let mut chars = text.chars();
                            let Some(ch) = chars.next() else {
                                return Err(RuntimeError::new(
                                    "The fill character must be exactly one character long",
                                ));
                            };
                            if chars.next().is_some() {
                                return Err(RuntimeError::new(
                                    "The fill character must be exactly one character long",
                                ));
                            }
                            ch
                        }
                        _ => return Err(RuntimeError::new("center() fillchar must be str")),
                    }
                } else {
                    ' '
                };
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let text_len = text.chars().count() as i64;
                if width <= text_len {
                    return Ok(NativeCallResult::Value(Value::Str(text)));
                }
                let total_pad = usize::try_from(width - text_len)
                    .map_err(|_| RuntimeError::new("center() width is too large"))?;
                let left_pad = total_pad / 2;
                let right_pad = total_pad - left_pad;
                let mut out = String::with_capacity(
                    text.len() + (left_pad + right_pad) * fillchar.len_utf8(),
                );
                for _ in 0..left_pad {
                    out.push(fillchar);
                }
                out.push_str(&text);
                for _ in 0..right_pad {
                    out.push(fillchar);
                }
                Ok(NativeCallResult::Value(Value::Str(out)))
            }
            NativeMethodKind::StrRStrip => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("rstrip() expects at most one argument"));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let stripped = match args.first() {
                    None | Some(Value::None) => text.trim_end().to_string(),
                    Some(Value::Str(chars)) => {
                        if chars.is_empty() {
                            text
                        } else {
                            text.trim_end_matches(|ch| chars.contains(ch)).to_string()
                        }
                    }
                    Some(_) => return Err(RuntimeError::new("rstrip() chars must be str or None")),
                };
                Ok(NativeCallResult::Value(Value::Str(stripped)))
            }
            NativeMethodKind::StrStrip => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("strip() expects at most one argument"));
                }
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let stripped = match args.first() {
                    None | Some(Value::None) => text.trim().to_string(),
                    Some(Value::Str(chars)) => {
                        if chars.is_empty() {
                            text
                        } else {
                            text.trim_matches(|ch| chars.contains(ch)).to_string()
                        }
                    }
                    Some(_) => return Err(RuntimeError::new("strip() chars must be str or None")),
                };
                Ok(NativeCallResult::Value(Value::Str(stripped)))
            }
            NativeMethodKind::StrExpandTabs => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "expandtabs() expects at most one argument",
                    ));
                }
                let tabsize = if let Some(value) = args.first() {
                    value_to_int(value.clone())?
                } else {
                    8
                };
                let tabsize = if tabsize < 0 { 0 } else { tabsize as usize };
                let text = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::type_error("str receiver is invalid")),
                };
                let mut out = String::with_capacity(text.len());
                let mut column = 0usize;
                for ch in text.chars() {
                    match ch {
                        '\t' => {
                            if tabsize > 0 {
                                let spaces = tabsize - (column % tabsize);
                                for _ in 0..spaces {
                                    out.push(' ');
                                }
                                column += spaces;
                            }
                        }
                        '\n' | '\r' => {
                            out.push(ch);
                            column = 0;
                        }
                        _ => {
                            out.push(ch);
                            column += 1;
                        }
                    }
                }
                Ok(NativeCallResult::Value(Value::Str(out)))
            }
            NativeMethodKind::CodeReplace => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("replace() takes no positional arguments"));
                }
                let code_obj = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Code(code_obj)) => code_obj.clone(),
                        _ => return Err(RuntimeError::new("code receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::new("code receiver is invalid")),
                };
                let mut replaced = (*code_obj).clone();
                let parse_str_sequence =
                    |value: Value, field_name: &str| -> Result<Vec<String>, RuntimeError> {
                        let items = match value {
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(items) => items.clone(),
                                _ => {
                                    return Err(RuntimeError::new(format!(
                                        "replace() {field_name} must be a tuple"
                                    )));
                                }
                            },
                            Value::List(obj) => match &*obj.kind() {
                                Object::List(items) => items.clone(),
                                _ => {
                                    return Err(RuntimeError::new(format!(
                                        "replace() {field_name} must be a tuple"
                                    )));
                                }
                            },
                            _ => {
                                return Err(RuntimeError::new(format!(
                                    "replace() {field_name} must be a tuple"
                                )));
                            }
                        };
                        let mut out = Vec::with_capacity(items.len());
                        for item in items {
                            match item {
                                Value::Str(text) => out.push(text),
                                _ => {
                                    return Err(RuntimeError::new(format!(
                                        "replace() {field_name} entries must be str"
                                    )));
                                }
                            }
                        }
                        Ok(out)
                    };
                for (name, value) in kwargs {
                    match name.as_str() {
                        "co_filename" => match value {
                            Value::Str(text) => replaced.filename = text,
                            _ => {
                                return Err(RuntimeError::new("replace() co_filename must be str"));
                            }
                        },
                        "co_name" | "co_qualname" => match value {
                            Value::Str(text) => replaced.name = text,
                            _ => {
                                return Err(RuntimeError::new("replace() co_name must be str"));
                            }
                        },
                        "co_names" => {
                            replaced.names = parse_str_sequence(value, "co_names")?;
                        }
                        "co_freevars" => {
                            replaced.freevars = parse_str_sequence(value, "co_freevars")?;
                        }
                        "co_cellvars" => {
                            replaced.cellvars = parse_str_sequence(value, "co_cellvars")?;
                        }
                        "co_varnames" => {
                            let _ = parse_str_sequence(value, "co_varnames")?;
                        }
                        "co_consts" => {
                            let items = match value {
                                Value::Tuple(obj) => match &*obj.kind() {
                                    Object::Tuple(items) => items.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "replace() co_consts must be a tuple",
                                        ));
                                    }
                                },
                                _ => {
                                    return Err(RuntimeError::new(
                                        "replace() co_consts must be a tuple",
                                    ));
                                }
                            };
                            replaced.constants = items;
                        }
                        "co_flags" => {
                            let flags = value_to_int(value)?;
                            if flags < 0 {
                                return Err(RuntimeError::new("replace() co_flags must be >= 0"));
                            }
                            replaced.is_generator = (flags & 0x0020) != 0;
                            replaced.is_coroutine = (flags & 0x0080) != 0;
                            replaced.is_iterable_coroutine = (flags & 0x0100) != 0;
                            replaced.is_async_generator = (flags & 0x0200) != 0;
                        }
                        "co_argcount" | "co_posonlyargcount" | "co_kwonlyargcount"
                        | "co_nlocals" | "co_stacksize" => {
                            let value = value_to_int(value)?;
                            if value < 0 {
                                return Err(RuntimeError::new(format!(
                                    "replace() {name} must be >= 0"
                                )));
                            }
                        }
                        "co_code" | "co_linetable" | "co_lnotab" | "co_exceptiontable" => {
                            match value {
                                Value::Bytes(_) | Value::ByteArray(_) => {}
                                _ => {
                                    return Err(RuntimeError::new(format!(
                                        "replace() {name} must be bytes",
                                    )));
                                }
                            }
                        }
                        "co_firstlineno" => {
                            let line = value_to_int(value)?;
                            if line <= 0 {
                                return Err(RuntimeError::new(
                                    "replace() co_firstlineno must be >= 1",
                                ));
                            }
                            let line = line as usize;
                            replaced.first_line = line;
                            if let Some(first) = replaced.locations.first_mut() {
                                first.line = line;
                            } else {
                                replaced
                                    .locations
                                    .push(crate::bytecode::Location::new(line, 0));
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(format!(
                                "replace() got an unexpected keyword argument '{}'",
                                name
                            )));
                        }
                    }
                }
                replaced.rebuild_layout_indexes();
                Ok(NativeCallResult::Value(Value::Code(Rc::new(replaced))))
            }
            NativeMethodKind::CodeCoPositions => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("co_positions() takes no arguments"));
                }
                let code_obj = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Code(code_obj)) => code_obj.clone(),
                        _ => return Err(RuntimeError::new("code receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::new("code receiver is invalid")),
                };
                let mut entries = Vec::with_capacity(code_obj.locations.len());
                for location in &code_obj.locations {
                    let start_line = if location.line == 0 {
                        Value::None
                    } else {
                        Value::Int(location.line as i64)
                    };
                    let end_line = if location.end_line == 0 {
                        Value::None
                    } else {
                        Value::Int(location.end_line as i64)
                    };
                    let start_column = if !self.traceback_caret_enabled || location.column == 0 {
                        Value::None
                    } else {
                        Value::Int(location.column.saturating_sub(1) as i64)
                    };
                    let end_column = if !self.traceback_caret_enabled || location.end_column == 0 {
                        Value::None
                    } else {
                        Value::Int(location.end_column.saturating_sub(1) as i64)
                    };
                    entries.push(self.heap.alloc_tuple(vec![
                        start_line,
                        end_line,
                        start_column,
                        end_column,
                    ]));
                }
                let iterator = self.call_builtin(
                    BuiltinFunction::Iter,
                    vec![self.heap.alloc_list(entries)],
                    HashMap::new(),
                )?;
                Ok(NativeCallResult::Value(iterator))
            }
            NativeMethodKind::CodeCoLines => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("co_lines() takes no arguments"));
                }
                let code_obj = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("value") {
                        Some(Value::Code(code_obj)) => code_obj.clone(),
                        _ => return Err(RuntimeError::new("code receiver is invalid")),
                    },
                    _ => return Err(RuntimeError::new("code receiver is invalid")),
                };
                let mut entries = Vec::with_capacity(code_obj.locations.len());
                for (index, location) in code_obj.locations.iter().enumerate() {
                    let start_offset = Value::Int((index * 2) as i64);
                    let end_offset = Value::Int(((index + 1) * 2) as i64);
                    let line = if location.line == 0 {
                        Value::None
                    } else {
                        Value::Int(location.line as i64)
                    };
                    entries.push(self.heap.alloc_tuple(vec![start_offset, end_offset, line]));
                }
                let iterator = self.call_builtin(
                    BuiltinFunction::Iter,
                    vec![self.heap.alloc_list(entries)],
                    HashMap::new(),
                )?;
                Ok(NativeCallResult::Value(iterator))
            }
            NativeMethodKind::FrameClear => {
                if !kwargs.is_empty() || !args.is_empty() {
                    return Err(RuntimeError::new("frame.clear() takes no arguments"));
                }
                let locals_value = {
                    let Object::Instance(instance_data) = &mut *receiver.kind_mut() else {
                        return Err(RuntimeError::new("frame.clear() receiver must be a frame"));
                    };
                    let locals_value = instance_data.attrs.get("f_locals").cloned();
                    // Drop backward links to break traceback reference cycles.
                    instance_data
                        .attrs
                        .insert("f_back".to_string(), Value::None);
                    locals_value
                };
                let clear_dict = |dict_obj: &ObjRef| {
                    if let Object::Dict(entries) = &mut *dict_obj.kind_mut() {
                        entries.clear();
                    }
                };
                if let Some(locals) = locals_value {
                    match locals {
                        Value::Dict(dict_obj) => clear_dict(&dict_obj),
                        Value::Instance(instance) => {
                            if let Some(dict_obj) = self.instance_backing_dict(&instance) {
                                clear_dict(&dict_obj);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::RePatternSearch
            | NativeMethodKind::RePatternMatch
            | NativeMethodKind::RePatternFullMatch => {
                if args.is_empty() || args.len() > 3 {
                    return Err(RuntimeError::new(
                        "TypeError: pattern method expects string and optional pos/endpos",
                    ));
                }
                let mut keyword_pos = None;
                let mut keyword_endpos = None;
                for (name, value) in kwargs {
                    match name.as_str() {
                        "pos" => {
                            if keyword_pos.is_some() {
                                return Err(RuntimeError::new(
                                    "TypeError: pattern method got multiple values for argument 'pos'",
                                ));
                            }
                            keyword_pos = Some(value);
                        }
                        "endpos" => {
                            if keyword_endpos.is_some() {
                                return Err(RuntimeError::new(
                                    "TypeError: pattern method got multiple values for argument 'endpos'",
                                ));
                            }
                            keyword_endpos = Some(value);
                        }
                        _ => {
                            return Err(RuntimeError::new(format!(
                                "TypeError: pattern method got an unexpected keyword argument '{}'",
                                name
                            )));
                        }
                    }
                }
                if args.len() >= 2 && keyword_pos.is_some() {
                    return Err(RuntimeError::new(
                        "TypeError: pattern method got multiple values for argument 'pos'",
                    ));
                }
                if args.len() >= 3 && keyword_endpos.is_some() {
                    return Err(RuntimeError::new(
                        "TypeError: pattern method got multiple values for argument 'endpos'",
                    ));
                }
                let mode = match kind {
                    NativeMethodKind::RePatternSearch => ReMode::Search,
                    NativeMethodKind::RePatternMatch => ReMode::Match,
                    NativeMethodKind::RePatternFullMatch => ReMode::FullMatch,
                    _ => unreachable!(),
                };
                let mut forwarded = vec![Value::Module(receiver.clone())];
                forwarded.push(args[0].clone());
                let pos = args.get(1).cloned().or(keyword_pos);
                let endpos = args.get(2).cloned().or(keyword_endpos);
                if let Some(pos) = pos {
                    forwarded.push(pos);
                }
                if let Some(endpos) = endpos {
                    if forwarded.len() == 2 {
                        forwarded.push(Value::Int(0));
                    }
                    forwarded.push(endpos);
                }
                Ok(NativeCallResult::Value(self.builtin_re_match_mode(
                    forwarded,
                    HashMap::new(),
                    mode,
                )?))
            }
            NativeMethodKind::RePatternSub => {
                if args.len() < 2 || args.len() > 3 {
                    return Err(RuntimeError::new(
                        "sub() expects replacement, string, optional count",
                    ));
                }
                let pattern = re_pattern_from_compiled_module(&receiver)?;
                let count = if let Some(value) = args.get(2) {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let max_replacements = if count <= 0 {
                    None
                } else {
                    Some(count as usize)
                };
                match pattern {
                    RePatternValue::Str(pattern_text) => {
                        let replacement = args[0].clone();
                        let replacement_is_callable = self.is_callable_value(&replacement);
                        let replacement_template = if replacement_is_callable {
                            None
                        } else {
                            match &replacement {
                                Value::Str(value) => Some(value.clone()),
                                _ => return Err(RuntimeError::new("replacement must be string")),
                            }
                        };
                        let text = match &args[1] {
                            Value::Str(value) => value.clone(),
                            _ => return Err(RuntimeError::new("string must be string")),
                        };
                        if pattern_text.is_empty() {
                            return Ok(NativeCallResult::Value(Value::Str(text)));
                        }
                        let mut cursor = 0usize;
                        let mut out = String::new();
                        let mut replaced = 0usize;
                        loop {
                            if max_replacements.is_some_and(|limit| replaced >= limit) {
                                break;
                            }
                            let match_value = self.builtin_re_match_mode(
                                vec![
                                    Value::Module(receiver.clone()),
                                    Value::Str(text.clone()),
                                    Value::Int(cursor as i64),
                                ],
                                HashMap::new(),
                                ReMode::Search,
                            )?;
                            let Value::Module(match_module) = match_value else {
                                break;
                            };
                            let (match_start, match_end, groups) = {
                                let Object::Module(module_data) = &*match_module.kind() else {
                                    return Err(RuntimeError::new("re match receiver is invalid"));
                                };
                                let match_start = match module_data.globals.get("_start") {
                                    Some(Value::Int(value)) if *value >= 0 => *value as usize,
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "re match receiver is invalid",
                                        ));
                                    }
                                };
                                let match_end = match module_data.globals.get("_end") {
                                    Some(Value::Int(value)) if *value >= 0 => *value as usize,
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "re match receiver is invalid",
                                        ));
                                    }
                                };
                                let groups = match module_data.globals.get("_groups") {
                                    Some(Value::Tuple(obj)) => match &*obj.kind() {
                                        Object::Tuple(values) => values.clone(),
                                        _ => Vec::new(),
                                    },
                                    Some(Value::List(obj)) => match &*obj.kind() {
                                        Object::List(values) => values.clone(),
                                        _ => Vec::new(),
                                    },
                                    _ => Vec::new(),
                                };
                                (match_start, match_end, groups)
                            };
                            if match_start < cursor
                                || match_start > text.len()
                                || match_end > text.len()
                                || match_end < match_start
                            {
                                return Err(RuntimeError::new("invalid regex match bounds"));
                            }
                            out.push_str(&text[cursor..match_start]);
                            let replacement_text = if replacement_is_callable {
                                let replacement_value = match self.call_internal(
                                    replacement.clone(),
                                    vec![Value::Module(match_module.clone())],
                                    HashMap::new(),
                                )? {
                                    InternalCallOutcome::Value(value) => value,
                                    InternalCallOutcome::CallerExceptionHandled => {
                                        return Err(self.runtime_error_from_active_exception(
                                            "replacement callable failed",
                                        ));
                                    }
                                };
                                match replacement_value {
                                    Value::Str(value) => value,
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "repl function must return a string",
                                        ));
                                    }
                                }
                            } else {
                                let template = replacement_template
                                    .as_ref()
                                    .expect("replacement template must exist");
                                let whole_match = &text[match_start..match_end];
                                let mut expanded = String::new();
                                let chars = template.chars().collect::<Vec<_>>();
                                let mut idx = 0usize;
                                while idx < chars.len() {
                                    let ch = chars[idx];
                                    if ch != '\\' {
                                        expanded.push(ch);
                                        idx += 1;
                                        continue;
                                    }
                                    idx += 1;
                                    if idx >= chars.len() {
                                        expanded.push('\\');
                                        break;
                                    }
                                    let escape = chars[idx];
                                    if escape.is_ascii_digit() && escape != '0' {
                                        let mut number = 0usize;
                                        while idx < chars.len() && chars[idx].is_ascii_digit() {
                                            number = number
                                                .saturating_mul(10)
                                                .saturating_add((chars[idx] as u8 - b'0') as usize);
                                            idx += 1;
                                        }
                                        if let Some(group) = groups.get(number.saturating_sub(1)) {
                                            match group {
                                                Value::Str(text) => expanded.push_str(text),
                                                Value::None => {}
                                                _ => {}
                                            }
                                        }
                                        continue;
                                    }
                                    idx += 1;
                                    match escape {
                                        '\\' => expanded.push('\\'),
                                        'g' => {
                                            expanded.push('\\');
                                            expanded.push('g');
                                        }
                                        _ => {
                                            expanded.push(escape);
                                        }
                                    }
                                }
                                if expanded.is_empty() && template == "\\0" {
                                    whole_match.to_string()
                                } else {
                                    expanded
                                }
                            };
                            out.push_str(&replacement_text);
                            replaced += 1;
                            if match_end == cursor {
                                if cursor >= text.len() {
                                    break;
                                }
                                let step = text[cursor..]
                                    .chars()
                                    .next()
                                    .map(|ch| ch.len_utf8())
                                    .unwrap_or(1);
                                cursor = (cursor + step).min(text.len());
                            } else {
                                cursor = match_end;
                            }
                        }
                        out.push_str(&text[cursor..]);
                        Ok(NativeCallResult::Value(Value::Str(out)))
                    }
                    RePatternValue::Bytes(pattern_bytes) => {
                        let replacement = bytes_like_from_value(args[0].clone())?;
                        let text = bytes_like_from_value(args[1].clone())?;
                        if pattern_bytes.is_empty() {
                            return Ok(NativeCallResult::Value(self.heap.alloc_bytes(text)));
                        }
                        let mut remaining: &[u8] = &text;
                        let mut out: Vec<u8> = Vec::new();
                        let mut replaced = 0usize;
                        while let Some(idx) = find_bytes_subslice(remaining, &pattern_bytes) {
                            if max_replacements.is_some_and(|limit| replaced >= limit) {
                                break;
                            }
                            out.extend_from_slice(&remaining[..idx]);
                            out.extend_from_slice(&replacement);
                            remaining = &remaining[idx + pattern_bytes.len()..];
                            replaced += 1;
                        }
                        out.extend_from_slice(remaining);
                        Ok(NativeCallResult::Value(self.heap.alloc_bytes(out)))
                    }
                }
            }
            NativeMethodKind::ReMatchGroup => Ok(NativeCallResult::Value(
                self.native_re_match_group(&receiver, args)?,
            )),
            NativeMethodKind::ReMatchGroups => Ok(NativeCallResult::Value(
                self.native_re_match_groups(&receiver, args)?,
            )),
            NativeMethodKind::ReMatchGroupDict => Ok(NativeCallResult::Value(
                self.native_re_match_groupdict(&receiver, args)?,
            )),
            NativeMethodKind::ReMatchStart => Ok(NativeCallResult::Value(
                self.native_re_match_start(&receiver, args)?,
            )),
            NativeMethodKind::ReMatchEnd => Ok(NativeCallResult::Value(
                self.native_re_match_end(&receiver, args)?,
            )),
            NativeMethodKind::ReMatchSpan => Ok(NativeCallResult::Value(
                self.native_re_match_span(&receiver, args)?,
            )),
            NativeMethodKind::ExceptionWithTraceback => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("with_traceback() expects one argument"));
                }
                let traceback_frames = self.traceback_frames_from_value(args.remove(0))?;
                let mut receiver_kind = receiver.kind_mut();
                let Object::Module(module_data) = &mut *receiver_kind else {
                    return Err(RuntimeError::type_error("exception receiver is invalid"));
                };
                let Some(Value::Exception(exception)) = module_data.globals.get_mut("exception")
                else {
                    return Err(RuntimeError::type_error("exception receiver is invalid"));
                };
                exception.traceback_frames = traceback_frames.unwrap_or_default();
                Ok(NativeCallResult::Value(Value::Exception(exception.clone())))
            }
            NativeMethodKind::ExceptionAddNote => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("add_note() expects one argument"));
                }
                let note = match args.remove(0) {
                    Value::Str(value) => value,
                    _ => return Err(RuntimeError::new("note must be str")),
                };
                let mut receiver_kind = receiver.kind_mut();
                let Object::Module(module_data) = &mut *receiver_kind else {
                    return Err(RuntimeError::type_error("exception receiver is invalid"));
                };
                let Some(Value::Exception(exception)) = module_data.globals.get_mut("exception")
                else {
                    return Err(RuntimeError::type_error("exception receiver is invalid"));
                };
                let note_value = Value::Str(note.clone());
                let default_notes_list = self.heap.alloc_list(vec![note_value.clone()]);
                let append_note_to_candidate = |candidate: &mut ExceptionObject| {
                    let mut attrs = candidate.attrs.borrow_mut();
                    if let Some(existing) = attrs.get_mut("__notes__") {
                        let Value::List(list_obj) = existing else {
                            return Err(RuntimeError::type_error(
                                "Cannot add note: __notes__ is not a list",
                            ));
                        };
                        let mut list_kind = list_obj.kind_mut();
                        let Object::List(items) = &mut *list_kind else {
                            return Err(RuntimeError::type_error(
                                "Cannot add note: __notes__ is not a list",
                            ));
                        };
                        items.push(note_value.clone());
                    } else {
                        attrs.insert("__notes__".to_string(), default_notes_list.clone());
                    }
                    candidate.notes.push(note.clone());
                    Ok::<(), RuntimeError>(())
                };
                append_note_to_candidate(exception.as_mut())?;
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::DescriptorReduceTypeError => Err(RuntimeError::new(
                "TypeError: cannot pickle descriptor objects",
            )),
            NativeMethodKind::BoundMethodDescriptorGet => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("__get__() expects 1-2 arguments"));
                }
                let descriptor = {
                    let descriptor_kind = receiver.kind();
                    let Object::BoundMethod(method_data) = &*descriptor_kind else {
                        return Err(RuntimeError::new("invalid method descriptor"));
                    };
                    method_data.clone()
                };
                let obj = args.remove(0);
                if matches!(obj, Value::None) {
                    return Ok(NativeCallResult::Value(Value::BoundMethod(receiver)));
                }
                if let Object::Module(module_data) = &*descriptor.receiver.kind()
                    && let Some(owner) = module_data.globals.get("owner")
                {
                    let owner_is_classinfo = matches!(
                        owner,
                        Value::Class(_) | Value::Tuple(_) | Value::List(_)
                    ) || matches!(owner, Value::Builtin(builtin) if self.builtin_is_type_object(*builtin));
                    if owner_is_classinfo && !self.value_is_instance_of(&obj, owner)? {
                        let owner_name = match owner {
                            Value::Builtin(builtin) => self.builtin_type_name(*builtin).to_string(),
                            Value::Class(class_ref) => match &*class_ref.kind() {
                                Object::Class(class_data) => class_data.name.clone(),
                                _ => self.value_type_name_for_error(owner),
                            },
                            _ => self.value_type_name_for_error(owner),
                        };
                        let object_name = self.value_type_name_for_error(&obj);
                        return Err(RuntimeError::type_error(format!(
                            "descriptor for '{}' objects doesn't apply to a '{}' object",
                            owner_name, object_name
                        )));
                    }
                }
                if let Object::Module(module_data) = &*descriptor.receiver.kind()
                    && module_data.name == "__str_unbound_method__"
                {
                    let text = match &obj {
                        Value::Str(value) => Some(value.clone()),
                        Value::Instance(instance) => self.instance_backing_str(instance),
                        _ => None,
                    };
                    if let Some(text) = text {
                        let bound_receiver = match self
                            .heap
                            .alloc_module(ModuleObject::new("__str_method__".to_string()))
                        {
                            Value::Module(obj) => obj,
                            _ => unreachable!(),
                        };
                        if let Object::Module(bound_receiver_data) = &mut *bound_receiver.kind_mut()
                        {
                            bound_receiver_data
                                .globals
                                .insert("value".to_string(), Value::Str(text));
                        }
                        return Ok(NativeCallResult::Value(self.heap.alloc_bound_method(
                            BoundMethod::new(descriptor.function, bound_receiver),
                        )));
                    }
                }
                let bound_receiver = self.receiver_from_value(&obj)?;
                Ok(NativeCallResult::Value(self.heap.alloc_bound_method(
                    BoundMethod::new(descriptor.function, bound_receiver),
                )))
            }
            NativeMethodKind::FunctionDescriptorGet => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("__get__() expects 1-2 arguments"));
                }
                if let Object::Module(module_data) = &*receiver.kind()
                    && module_data.name == "__builtin_descriptor__"
                {
                    let Some(Value::Builtin(builtin)) = module_data.globals.get("builtin").cloned()
                    else {
                        return Err(RuntimeError::new("invalid builtin descriptor"));
                    };
                    let obj = args.remove(0);
                    if matches!(obj, Value::None) {
                        return Ok(NativeCallResult::Value(Value::Builtin(builtin)));
                    }
                    let bound_receiver = self.receiver_from_value(&obj)?;
                    return Ok(NativeCallResult::Value(
                        self.alloc_builtin_bound_method(builtin, bound_receiver),
                    ));
                }
                let obj = args.remove(0);
                if matches!(obj, Value::None) {
                    return Ok(NativeCallResult::Value(Value::Function(receiver)));
                }
                let bound_receiver = self.receiver_from_value(&obj)?;
                Ok(NativeCallResult::Value(self.heap.alloc_bound_method(
                    BoundMethod::new(receiver, bound_receiver),
                )))
            }
            NativeMethodKind::ClassMethodDescriptorGet => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("__get__() expects 1-2 arguments"));
                }
                let obj = args.remove(0);
                let owner_hint = args.first().cloned().unwrap_or(Value::None);
                let descriptor = Value::Module(receiver.clone());
                let Some(unwrapped) = self.unwrap_classmethod_attr(&descriptor) else {
                    return Err(RuntimeError::new("invalid classmethod descriptor"));
                };

                let owner_class = match owner_hint {
                    Value::Class(class_ref) => Some(class_ref),
                    Value::None => self.class_of_value(&obj),
                    other => self.class_of_value(&other),
                };
                let Some(owner_class) = owner_class else {
                    return Ok(NativeCallResult::Value(unwrapped));
                };

                let bound = match unwrapped {
                    Value::Function(function) => self
                        .heap
                        .alloc_bound_method(BoundMethod::new(function, owner_class)),
                    Value::Builtin(builtin) => {
                        self.alloc_builtin_bound_method(builtin, owner_class)
                    }
                    other => other,
                };
                Ok(NativeCallResult::Value(bound))
            }
            NativeMethodKind::StaticMethodDescriptorGet => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("__get__() expects 1-2 arguments"));
                }
                let descriptor = Value::Module(receiver);
                let Some(unwrapped) = self.unwrap_staticmethod_attr(&descriptor) else {
                    return Err(RuntimeError::new("invalid staticmethod descriptor"));
                };
                Ok(NativeCallResult::Value(unwrapped))
            }
            NativeMethodKind::FunctionAnnotate => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "__annotate__() takes at most one positional argument",
                    ));
                }
                let format = if let Some(value) = args.pop() {
                    let mut parsed = value_to_int(value.clone())?;
                    if parsed == 0
                        && let Value::Instance(instance) = value
                        && let Object::Instance(instance_data) = &*instance.kind()
                        && let Some(raw_value) = instance_data
                            .attrs
                            .get("_value_")
                            .or_else(|| instance_data.attrs.get("value"))
                        && let Ok(unwrapped) = value_to_int(raw_value.clone())
                    {
                        parsed = unwrapped;
                    }
                    parsed
                } else {
                    1
                };
                if !matches!(format, 1..=4) {
                    return Err(RuntimeError::value_error(format!(
                        "invalid annotation format {}",
                        format
                    )));
                }
                let receiver_kind = receiver.kind();
                let Object::Module(module_data) = &*receiver_kind else {
                    return Err(RuntimeError::new("function annotate receiver is invalid"));
                };
                let class_annotation_wrapper = module_data.name == "__class_annotate__";
                let Some(Value::Function(function_obj)) =
                    module_data.globals.get("function").cloned()
                else {
                    return Err(RuntimeError::new("function annotate receiver is invalid"));
                };
                let explicit_owner = module_data
                    .globals
                    .get("owner")
                    .cloned()
                    .filter(|owner| !matches!(owner, Value::None));
                let call_owner = if class_annotation_wrapper {
                    self.annotation_call_owner_from_call_stack()
                } else {
                    None
                };
                let forward_ref_owner = if class_annotation_wrapper {
                    explicit_owner
                        .clone()
                        .or_else(|| call_owner.clone())
                        .unwrap_or(Value::None)
                } else {
                    explicit_owner
                        .or_else(|| self.annotation_owner_from_call_stack())
                        .unwrap_or_else(|| Value::Function(function_obj.clone()))
                };
                let (annotations, function_module, annotation_locals, future_annotations_import) = {
                    let mut function_ref = function_obj.kind_mut();
                    let Object::Function(func_data) = &mut *function_ref else {
                        return Err(RuntimeError::new("function annotate receiver is invalid"));
                    };
                    let annotations = if let Some(existing) = func_data.annotations.clone() {
                        existing
                    } else {
                        let dict = self.heap.alloc_dict(Vec::new());
                        let Value::Dict(dict_obj) = dict else {
                            unreachable!()
                        };
                        func_data.annotations = Some(dict_obj.clone());
                        dict_obj
                    };
                    let module = func_data.module.clone();
                    let locals = func_data
                        .dict
                        .as_ref()
                        .and_then(|dict| {
                            dict_get_value(
                                dict,
                                &Value::Str("__pyrs_annotation_locals__".to_string()),
                            )
                        })
                        .and_then(|value| match value {
                            Value::Dict(dict) => Some(dict),
                            _ => None,
                        });
                    (
                        annotations,
                        module,
                        locals,
                        func_data.code.future_annotations_import,
                    )
                };

                let mut resolved_entries = Vec::new();
                if let Object::Dict(entries) = &*annotations.kind() {
                    for (key, value) in entries.iter() {
                        let Value::Str(name) = key else {
                            continue;
                        };
                        let resolved_value = match format {
                            1 | 2 => match value {
                                Value::Str(text)
                                    if class_annotation_wrapper && future_annotations_import =>
                                {
                                    if let Some(owner) = call_owner.clone() {
                                        let module_name =
                                            self.annotation_module_name_for_owner(&owner);
                                        self.function_annotation_forward_ref(
                                            text,
                                            owner,
                                            module_name,
                                        )?
                                    } else {
                                        Value::Str(text.clone())
                                    }
                                }
                                Value::Str(text) if class_annotation_wrapper => self
                                    .class_annotation_eval_value_dynamic(
                                        text,
                                        &function_module,
                                        annotation_locals.as_ref(),
                                    )?,
                                Value::Str(text) if !future_annotations_import => self
                                    .function_annotation_eval_value(
                                        text,
                                        &function_module,
                                        annotation_locals.as_ref(),
                                    )?,
                                other => other.clone(),
                            },
                            3 => match value {
                                Value::Str(text)
                                    if class_annotation_wrapper && future_annotations_import =>
                                {
                                    let module_name =
                                        self.annotation_module_name_for_owner(&forward_ref_owner);
                                    self.function_annotation_forward_ref(
                                        text,
                                        forward_ref_owner.clone(),
                                        module_name,
                                    )?
                                }
                                Value::Str(text) => self.function_annotation_eval_forward_ref(
                                    text,
                                    &function_module,
                                    annotation_locals.as_ref(),
                                    forward_ref_owner.clone(),
                                )?,
                                other => other.clone(),
                            },
                            4 => match value {
                                Value::Str(text) => {
                                    Value::Str(self.function_annotation_format_string(text))
                                }
                                other => self.builtin_repr(vec![other.clone()], HashMap::new())?,
                            },
                            _ => unreachable!(),
                        };
                        resolved_entries.push((Value::Str(name.clone()), resolved_value));
                    }
                }
                Ok(NativeCallResult::Value(
                    self.heap.alloc_dict(resolved_entries),
                ))
            }
            NativeMethodKind::ObjectReduceExBound => {
                let receiver_kind = receiver.kind();
                let Object::Module(module_data) = &*receiver_kind else {
                    return Err(RuntimeError::new("object reduce receiver is invalid"));
                };
                let Some(stored_value) = module_data.globals.get("value").cloned() else {
                    return Err(RuntimeError::new("object reduce receiver is invalid"));
                };
                let mut protocol = 0;
                let mut value = stored_value.clone();
                let explicit_object_base_call =
                    matches!(stored_value, Value::Builtin(BuiltinFunction::ObjectNew));

                if explicit_object_base_call {
                    if args.is_empty() {
                        return Err(RuntimeError::new(
                            "__reduce_ex__() missing required argument 'self'",
                        ));
                    }
                    value = args.remove(0);
                }
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "__reduce_ex__() takes at most one protocol argument",
                    ));
                }
                if let Some(protocol_arg) = args.first() {
                    protocol = value_to_int(protocol_arg.clone())?;
                }

                let reduced =
                    self.object_reduce_ex_for_value(value, protocol, !explicit_object_base_call)?;
                Ok(NativeCallResult::Value(reduced))
            }
            NativeMethodKind::BoundMethodReduceEx => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "__reduce_ex__() takes at most one protocol argument",
                    ));
                }
                let receiver_kind = receiver.kind();
                let Object::Module(module_data) = &*receiver_kind else {
                    return Err(RuntimeError::new("method reduce receiver is invalid"));
                };
                let Some(Value::BoundMethod(method)) = module_data.globals.get("method").cloned()
                else {
                    return Err(RuntimeError::new("method reduce receiver is invalid"));
                };
                let (function, method_receiver) = match &*method.kind() {
                    Object::BoundMethod(method_data) => {
                        (method_data.function.clone(), method_data.receiver.clone())
                    }
                    _ => return Err(RuntimeError::new("method reduce receiver is invalid")),
                };
                let function_name = match &*function.kind() {
                    Object::Function(function_data) => function_data.code.name.clone(),
                    Object::NativeMethod(native_data) => self
                        .native_method_pickle_name(native_data.kind)
                        .map(str::to_string)
                        .ok_or_else(|| RuntimeError::new("method is not picklable"))?,
                    _ => return Err(RuntimeError::new("method is not picklable")),
                };
                let receiver_value = self.bound_method_reduce_receiver_value(&method_receiver)?;
                let reduce_args = self
                    .heap
                    .alloc_tuple(vec![receiver_value, Value::Str(function_name)]);
                Ok(NativeCallResult::Value(self.heap.alloc_tuple(vec![
                    Value::Builtin(BuiltinFunction::GetAttr),
                    reduce_args,
                ])))
            }
            NativeMethodKind::ComplexReduceEx => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "__reduce_ex__() takes at most one protocol argument",
                    ));
                }
                let receiver_kind = receiver.kind();
                let Object::Module(module_data) = &*receiver_kind else {
                    return Err(RuntimeError::new("complex reduce receiver is invalid"));
                };
                let Some(value) = module_data.globals.get("value").cloned() else {
                    return Err(RuntimeError::new("complex reduce receiver is invalid"));
                };
                let mut forwarded = vec![value];
                if let Some(protocol) = args.first() {
                    forwarded.push(protocol.clone());
                }
                let reduced = self.builtin_object_reduce_ex(forwarded, HashMap::new())?;
                Ok(NativeCallResult::Value(reduced))
            }
            NativeMethodKind::GenericAliasRepr => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("__repr__() takes no arguments"));
                }
                let rendered = self.generic_alias_repr(&receiver)?;
                Ok(NativeCallResult::Value(Value::Str(rendered)))
            }
            NativeMethodKind::GenericAliasCall => {
                let receiver_value = Value::Instance(receiver.clone());
                if !self.is_types_generic_alias_value(&receiver_value) {
                    return Err(RuntimeError::new("GenericAlias call receiver is invalid"));
                }
                let Some((origin, _)) = self.generic_alias_parts_from_value(&receiver_value) else {
                    return Err(RuntimeError::new("GenericAlias call receiver is invalid"));
                };
                match self.call_internal(origin, args, kwargs)? {
                    InternalCallOutcome::Value(value) => {
                        match self.builtin_setattr(
                            vec![
                                value.clone(),
                                Value::Str("__orig_class__".to_string()),
                                receiver_value,
                            ],
                            HashMap::new(),
                        ) {
                            Ok(_) => {}
                            Err(err)
                                if runtime_error_matches_exception(&err, "AttributeError")
                                    || runtime_error_matches_exception(&err, "TypeError") => {}
                            Err(err) => return Err(err),
                        }
                        Ok(NativeCallResult::Value(value))
                    }
                    InternalCallOutcome::CallerExceptionHandled => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::GenericAliasReduceEx => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "__reduce_ex__() takes at most one protocol argument",
                    ));
                }
                if self
                    .generic_alias_parts_from_value(&Value::Instance(receiver.clone()))
                    .is_none()
                {
                    return Err(RuntimeError::new("GenericAlias reduce receiver is invalid"));
                }
                let receiver_kind = receiver.kind();
                let Object::Instance(instance_data) = &*receiver_kind else {
                    return Err(RuntimeError::new("GenericAlias reduce receiver is invalid"));
                };
                let origin = instance_data
                    .attrs
                    .get("__origin__")
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("GenericAlias reduce receiver is invalid"))?;
                let args_value = instance_data
                    .attrs
                    .get("__args__")
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("GenericAlias reduce receiver is invalid"))?;
                let preserve_unpacked_marker = instance_data
                    .attrs
                    .get("__unpacked__")
                    .cloned()
                    .map(|flag| self.truthy_from_value(&flag))
                    .transpose()?
                    .unwrap_or(false);
                let unpacked_tuple_args = instance_data
                    .attrs
                    .get("__typing_unpacked_tuple_args__")
                    .cloned();
                let is_typing_annotated_alias = match &*instance_data.class.kind() {
                    Object::Class(class_data) => {
                        if class_data.name != "_AnnotatedAlias" {
                            false
                        } else {
                            matches!(
                                class_data.attrs.get("__module__"),
                                Some(Value::Str(module_name))
                                    if module_name == "typing" || module_name == "_typing"
                            )
                        }
                    }
                    _ => false,
                };
                let is_collections_callable_alias = match &*instance_data.class.kind() {
                    Object::Class(class_data) => {
                        if class_data.name != "_CallableGenericAlias" {
                            false
                        } else {
                            matches!(
                                class_data.attrs.get("__module__"),
                                Some(Value::Str(module_name))
                                    if module_name == "collections.abc"
                                        || module_name == "_collections_abc"
                            )
                        }
                    }
                    _ => false,
                };
                if is_typing_annotated_alias {
                    let metadata_values = match instance_data.attrs.get("__metadata__") {
                        Some(Value::Tuple(metadata_obj)) => match &*metadata_obj.kind() {
                            Object::Tuple(values) => values.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "GenericAlias reduce receiver is invalid",
                                ));
                            }
                        },
                        Some(other) => vec![other.clone()],
                        None => Vec::new(),
                    };
                    let typing_module = if let Some(module) = self.modules.get("typing").cloned() {
                        module
                    } else {
                        self.load_module("typing")?
                    };
                    let annotated = self.builtin_getattr(
                        vec![
                            Value::Module(typing_module),
                            Value::Str("Annotated".to_string()),
                        ],
                        HashMap::new(),
                    )?;
                    let mut annotated_args = Vec::with_capacity(metadata_values.len() + 1);
                    annotated_args.push(origin);
                    annotated_args.extend(metadata_values);
                    let params = self.heap.alloc_tuple(annotated_args);
                    let ctor_args = self.heap.alloc_tuple(vec![annotated, params]);
                    return Ok(NativeCallResult::Value(self.heap.alloc_tuple(vec![
                        Value::Builtin(BuiltinFunction::OperatorGetItem),
                        ctor_args,
                    ])));
                }
                let args_value = match args_value {
                    Value::Tuple(_) => args_value,
                    other => self.heap.alloc_tuple(vec![other]),
                };
                let args_value = if is_collections_callable_alias {
                    let flattened = match &args_value {
                        Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                            Object::Tuple(items) => items.clone(),
                            _ => Vec::new(),
                        },
                        _ => Vec::new(),
                    };
                    let first_is_param_expr = flattened.first().is_some_and(|first| {
                        if matches!(first, Value::List(_)) {
                            return true;
                        }
                        if self
                            .builtins
                            .get("Ellipsis")
                            .is_some_and(|ellipsis| ellipsis == first)
                        {
                            return true;
                        }
                        if self.typing_param_kind_name(first) == Some("ParamSpec") {
                            return true;
                        }
                        let Value::Instance(instance) = first else {
                            return false;
                        };
                        let instance_kind = instance.kind();
                        let Object::Instance(instance_data) = &*instance_kind else {
                            return false;
                        };
                        let class_kind = instance_data.class.kind();
                        let Object::Class(class_data) = &*class_kind else {
                            return false;
                        };
                        class_data.name == "_ConcatenateGenericAlias"
                            && matches!(
                                class_data.attrs.get("__module__"),
                                Some(Value::Str(module_name))
                                    if module_name == "typing" || module_name == "_typing"
                            )
                    });
                    if flattened.len() == 2 && first_is_param_expr {
                        args_value
                    } else if let Some(result) = flattened.last().cloned() {
                        let call_args = self
                            .heap
                            .alloc_list(flattened[..flattened.len().saturating_sub(1)].to_vec());
                        self.heap.alloc_tuple(vec![call_args, result])
                    } else {
                        args_value
                    }
                } else {
                    args_value
                };
                let ctor = Value::Class(instance_data.class.clone());
                let ctor_args = self.heap.alloc_tuple(vec![origin, args_value]);
                if preserve_unpacked_marker || unpacked_tuple_args.is_some() {
                    let mut state_entries = Vec::new();
                    if preserve_unpacked_marker {
                        state_entries
                            .push((Value::Str("__unpacked__".to_string()), Value::Bool(true)));
                    }
                    if let Some(unpacked_args) = unpacked_tuple_args {
                        state_entries.push((
                            Value::Str("__typing_unpacked_tuple_args__".to_string()),
                            unpacked_args,
                        ));
                    }
                    let state = self.heap.alloc_dict(state_entries);
                    return Ok(NativeCallResult::Value(
                        self.heap.alloc_tuple(vec![ctor, ctor_args, state]),
                    ));
                }
                Ok(NativeCallResult::Value(
                    self.heap.alloc_tuple(vec![ctor, ctor_args]),
                ))
            }
            NativeMethodKind::GenericAliasMroEntries => {
                if args.len() != 1 {
                    return Err(RuntimeError::type_error(
                        "__mro_entries__() takes exactly one argument",
                    ));
                }
                let receiver_value = Value::Instance(receiver.clone());
                if !self.is_types_generic_alias_value(&receiver_value) {
                    return Err(RuntimeError::new("GenericAlias mro receiver is invalid"));
                }
                let receiver_kind = receiver.kind();
                let Object::Instance(instance_data) = &*receiver_kind else {
                    return Err(RuntimeError::new("GenericAlias mro receiver is invalid"));
                };
                let origin = instance_data
                    .attrs
                    .get("__origin__")
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("GenericAlias mro receiver is invalid"))?;
                Ok(NativeCallResult::Value(self.heap.alloc_tuple(vec![origin])))
            }
            NativeMethodKind::TypeParamCopy => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "__deepcopy__() takes at most one argument",
                    ));
                }
                if !self.is_type_parameter_value(&Value::Instance(receiver.clone())) {
                    return Err(RuntimeError::new("type parameter copy receiver is invalid"));
                }
                Ok(NativeCallResult::Value(Value::Instance(receiver)))
            }
            NativeMethodKind::TypeParamRepr => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("__repr__() takes no arguments"));
                }
                let receiver_value = Value::Instance(receiver.clone());
                let Some(kind) = self.typing_param_kind_name(&receiver_value) else {
                    return Err(RuntimeError::new("type parameter repr receiver is invalid"));
                };
                let name = match self.builtin_getattr(
                    vec![receiver_value.clone(), Value::Str("__name__".to_string())],
                    HashMap::new(),
                ) {
                    Ok(Value::Str(name)) => name,
                    _ => {
                        return Err(RuntimeError::new("type parameter repr receiver is invalid"));
                    }
                };
                let rendered = match kind {
                    "TypeVar" => {
                        let receiver_kind = receiver.kind();
                        let Object::Instance(instance_data) = &*receiver_kind else {
                            return Err(RuntimeError::new(
                                "type parameter repr receiver is invalid",
                            ));
                        };
                        let covariant = matches!(
                            instance_data.attrs.get("__covariant__"),
                            Some(Value::Bool(true))
                        );
                        let contravariant = matches!(
                            instance_data.attrs.get("__contravariant__"),
                            Some(Value::Bool(true))
                        );
                        let prefix = if covariant {
                            "+"
                        } else if contravariant {
                            "-"
                        } else {
                            "~"
                        };
                        format!("{prefix}{name}")
                    }
                    "ParamSpec" => format!("~{name}"),
                    "TypeVarTuple" => name,
                    _ => return Err(RuntimeError::new("type parameter repr receiver is invalid")),
                };
                Ok(NativeCallResult::Value(Value::Str(rendered)))
            }
            NativeMethodKind::TypeParamReduceEx => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "__reduce_ex__() takes at most one protocol argument",
                    ));
                }
                if !self.is_type_parameter_value(&Value::Instance(receiver.clone())) {
                    return Err(RuntimeError::new(
                        "type parameter reduce receiver is invalid",
                    ));
                }
                let receiver_kind = receiver.kind();
                let Object::Instance(instance_data) = &*receiver_kind else {
                    return Err(RuntimeError::new(
                        "type parameter reduce receiver is invalid",
                    ));
                };
                let Some(Value::Str(name)) = instance_data.attrs.get("__name__") else {
                    return Err(RuntimeError::new(
                        "type parameter reduce receiver is invalid",
                    ));
                };
                Ok(NativeCallResult::Value(Value::Str(name.clone())))
            }
            NativeMethodKind::MappingProxyContains => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("__contains__() expects one argument"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let target = args.remove(0);
                if let Some(contains_method) =
                    self.lookup_bound_special_method(&mapping, "__contains__")?
                {
                    return match self.call_internal(
                        contains_method,
                        vec![target],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(value) => Ok(NativeCallResult::Value(
                            Value::Bool(self.truthy_from_value(&value)?),
                        )),
                        InternalCallOutcome::CallerExceptionHandled => {
                            Ok(NativeCallResult::PropagatedException)
                        }
                    };
                }
                let contains = self.compare_in_runtime(target, mapping)?;
                Ok(NativeCallResult::Value(Value::Bool(contains)))
            }
            NativeMethodKind::MappingProxyGetItem => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("__getitem__() expects one argument"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let key = args.remove(0);
                let value = if let Some(getitem_method) =
                    self.lookup_bound_special_method(&mapping, "__getitem__")?
                {
                    match self.call_internal(getitem_method, vec![key.clone()], HashMap::new())? {
                        InternalCallOutcome::Value(value) => value,
                        InternalCallOutcome::CallerExceptionHandled => {
                            return Ok(NativeCallResult::PropagatedException);
                        }
                    }
                } else {
                    self.call_builtin(
                        BuiltinFunction::OperatorGetItem,
                        vec![mapping, key],
                        HashMap::new(),
                    )?
                };
                Ok(NativeCallResult::Value(value))
            }
            NativeMethodKind::MappingProxyIor => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("__ior__() expects one argument"));
                }
                Err(RuntimeError::type_error(
                    "'|=' is not supported by mappingproxy; use '|' instead",
                ))
            }
            NativeMethodKind::MappingProxyIter => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("__iter__() expects no arguments"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let iter = if let Some(iter_method) =
                    self.lookup_bound_special_method(&mapping, "__iter__")?
                {
                    match self.call_internal(iter_method, Vec::new(), HashMap::new())? {
                        InternalCallOutcome::Value(value) => self.to_iterator_value(value)?,
                        InternalCallOutcome::CallerExceptionHandled => {
                            return Ok(NativeCallResult::PropagatedException);
                        }
                    }
                } else {
                    self.to_iterator_value(mapping)?
                };
                Ok(NativeCallResult::Value(iter))
            }
            NativeMethodKind::MappingProxyLen => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("__len__() expects no arguments"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let len = if let Some(len_method) =
                    self.lookup_bound_special_method(&mapping, "__len__")?
                {
                    match self.call_internal(len_method, Vec::new(), HashMap::new())? {
                        InternalCallOutcome::Value(value) => Value::Int(value_to_int(value)?),
                        InternalCallOutcome::CallerExceptionHandled => {
                            return Ok(NativeCallResult::PropagatedException);
                        }
                    }
                } else {
                    self.call_builtin(BuiltinFunction::Len, vec![mapping], HashMap::new())?
                };
                Ok(NativeCallResult::Value(len))
            }
            NativeMethodKind::MappingProxyOr => {
                if args.len() != 1 {
                    return Err(RuntimeError::type_error("__or__() expects one argument"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let right = args.remove(0);
                let merged = self.binary_or_runtime(mapping, right)?;
                Ok(NativeCallResult::Value(merged))
            }
            NativeMethodKind::MappingProxyReversed => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error(
                        "__reversed__() expects no arguments",
                    ));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let reversed =
                    self.call_builtin(BuiltinFunction::Reversed, vec![mapping], HashMap::new())?;
                Ok(NativeCallResult::Value(reversed))
            }
            NativeMethodKind::MappingProxyRor => {
                if args.len() != 1 {
                    return Err(RuntimeError::type_error("__ror__() expects one argument"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let left = args.remove(0);
                let merged = self.binary_or_runtime(left, mapping)?;
                Ok(NativeCallResult::Value(merged))
            }
            NativeMethodKind::MappingProxyCopy => {
                self.mappingproxy_call_mapping_method(&receiver, "copy", args)
            }
            NativeMethodKind::MappingProxyGet => {
                self.mappingproxy_call_mapping_method(&receiver, "get", args)
            }
            NativeMethodKind::MappingProxyItems => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("items() expects no arguments"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                if matches!(mapping, Value::Dict(_)) {
                    return Ok(NativeCallResult::Value(
                        self.mappingproxy_alloc_view_instance(mapping, true),
                    ));
                }
                self.mappingproxy_call_mapping_method(&receiver, "items", args)
            }
            NativeMethodKind::MappingProxyKeys => {
                self.mappingproxy_call_mapping_method(&receiver, "keys", args)
            }
            NativeMethodKind::MappingProxyValues => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("values() expects no arguments"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                if matches!(mapping, Value::Dict(_)) {
                    return Ok(NativeCallResult::Value(
                        self.mappingproxy_alloc_view_instance(mapping, false),
                    ));
                }
                self.mappingproxy_call_mapping_method(&receiver, "values", args)
            }
            NativeMethodKind::MappingProxyEq => {
                if args.len() != 1 {
                    return Err(RuntimeError::type_error("__eq__() expects one argument"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let other = args.remove(0);
                let equals = self.compare_eq_runtime(mapping, other)?;
                Ok(NativeCallResult::Value(equals))
            }
            NativeMethodKind::MappingProxyNe => {
                if args.len() != 1 {
                    return Err(RuntimeError::type_error("__ne__() expects one argument"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let other = args.remove(0);
                let equals = self.compare_eq_runtime(mapping, other)?;
                let truthy = self.truthy_from_value(&equals)?;
                Ok(NativeCallResult::Value(Value::Bool(!truthy)))
            }
            NativeMethodKind::MappingProxyHash => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("__hash__() expects no arguments"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let hash = self.hash_value_runtime(&mapping)?;
                Ok(NativeCallResult::Value(Value::Int(hash)))
            }
            NativeMethodKind::MappingProxyRepr => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("__repr__() expects no arguments"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let repr =
                    self.call_builtin(BuiltinFunction::Repr, vec![mapping], HashMap::new())?;
                let Value::Str(rendered) = repr else {
                    return Err(RuntimeError::type_error("__repr__() must return str"));
                };
                Ok(NativeCallResult::Value(Value::Str(format!(
                    "mappingproxy({rendered})"
                ))))
            }
            NativeMethodKind::MappingProxyClassGetItem => {
                if args.len() != 1 {
                    return Err(RuntimeError::type_error(
                        "__class_getitem__() expects one argument",
                    ));
                }
                Ok(NativeCallResult::Value(self.alloc_generic_alias_instance(
                    Value::Class(receiver),
                    args.remove(0),
                )))
            }
            NativeMethodKind::MappingProxyValuesViewIter => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("__iter__() expects no arguments"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let Value::Dict(dict_obj) = mapping else {
                    return Err(RuntimeError::type_error(
                        "mappingproxy values view is invalid",
                    ));
                };
                let values = match &*dict_obj.kind() {
                    Object::Dict(entries) => entries
                        .iter()
                        .map(|(_key, value)| value.clone())
                        .collect::<Vec<_>>(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "mappingproxy values view is invalid",
                        ));
                    }
                };
                let as_list = self.heap.alloc_list(values);
                let iter = self.to_iterator_value(as_list)?;
                Ok(NativeCallResult::Value(iter))
            }
            NativeMethodKind::MappingProxyItemsViewIter => {
                if !args.is_empty() {
                    return Err(RuntimeError::type_error("__iter__() expects no arguments"));
                }
                let mapping = self.mappingproxy_mapping_value(&receiver)?;
                let Value::Dict(dict_obj) = mapping else {
                    return Err(RuntimeError::type_error(
                        "mappingproxy items view is invalid",
                    ));
                };
                let items = match &*dict_obj.kind() {
                    Object::Dict(entries) => entries
                        .iter()
                        .map(|(key, value)| self.heap.alloc_tuple(vec![key.clone(), value.clone()]))
                        .collect::<Vec<_>>(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "mappingproxy items view is invalid",
                        ));
                    }
                };
                let as_list = self.heap.alloc_list(items);
                let iter = self.to_iterator_value(as_list)?;
                Ok(NativeCallResult::Value(iter))
            }
            NativeMethodKind::SetContains => {
                let receiver_is_set = {
                    let receiver_kind = receiver.kind();
                    matches!(&*receiver_kind, Object::Set(_) | Object::FrozenSet(_))
                };
                let receiver_is_module = {
                    let receiver_kind = receiver.kind();
                    matches!(&*receiver_kind, Object::Module(_))
                };
                let (container, target) = if receiver_is_set {
                    if args.len() != 1 {
                        return Err(RuntimeError::new("__contains__() expects one argument"));
                    }
                    (receiver.clone(), args.remove(0))
                } else if receiver_is_module {
                    if args.len() != 2 {
                        return Err(RuntimeError::new("__contains__() expects one argument"));
                    }
                    let receiver_value = args.remove(0);
                    let target = args.remove(0);
                    let container = self.receiver_from_value(&receiver_value)?;
                    let container_is_set = {
                        let container_kind = container.kind();
                        matches!(&*container_kind, Object::Set(_) | Object::FrozenSet(_))
                    };
                    if !container_is_set {
                        return Err(RuntimeError::new("__contains__() receiver must be set"));
                    }
                    (container, target)
                } else {
                    return Err(RuntimeError::new("__contains__() receiver must be set"));
                };
                ensure_hashable(&target)?;
                let contains = self.set_contains_runtime(&container, &target)?;
                Ok(NativeCallResult::Value(Value::Bool(contains)))
            }
            NativeMethodKind::SetAdd => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("add() expects one argument"));
                }
                let item = args.first().cloned().expect("checked len");
                self.set_insert_checked_runtime(&receiver, item)?;
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::SetDiscard => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("discard() expects one argument"));
                }
                let item = args.first().cloned().expect("checked len");
                let _ = self.set_remove_checked_runtime(&receiver, &item)?;
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::SetRemove => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("remove() expects one argument"));
                }
                let item = args.first().cloned().expect("checked len");
                if self.set_remove_checked_runtime(&receiver, &item)? {
                    Ok(NativeCallResult::Value(Value::None))
                } else {
                    Err(RuntimeError::key_error("key not found"))
                }
            }
            NativeMethodKind::SetPop => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("pop() expects no arguments"));
                }
                let mut receiver_kind = receiver.kind_mut();
                let Object::Set(values) = &mut *receiver_kind else {
                    return Err(RuntimeError::new("pop() receiver must be set"));
                };
                if values.is_empty() {
                    return Err(RuntimeError::key_error("pop from an empty set"));
                }
                let item = values.remove(values.len() - 1);
                Ok(NativeCallResult::Value(item))
            }
            NativeMethodKind::SetUpdate => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("update() expects one argument"));
                }
                let items = self.collect_iterable_values(args[0].clone())?;
                for item in items {
                    self.set_insert_checked_runtime(&receiver, item)?;
                }
                Ok(NativeCallResult::Value(Value::None))
            }
            NativeMethodKind::SetUnion => {
                let mut out = match &*receiver.kind() {
                    Object::Set(values) | Object::FrozenSet(values) => values.to_vec(),
                    _ => return Err(RuntimeError::new("union() receiver must be set")),
                };
                out = self.dedup_hashable_values_runtime(out)?;
                for iterable in args {
                    for item in self.collect_iterable_values(iterable)? {
                        ensure_hashable(&item)?;
                        if !self.sequence_contains_runtime_value(&out, &item)? {
                            out.push(item);
                        }
                    }
                }
                if matches!(&*receiver.kind(), Object::FrozenSet(_)) {
                    Ok(NativeCallResult::Value(self.heap.alloc_frozenset(out)))
                } else {
                    Ok(NativeCallResult::Value(self.heap.alloc_set(out)))
                }
            }
            NativeMethodKind::SetIntersection => {
                let mut out = self.dedup_hashable_values_runtime(match &*receiver.kind() {
                    Object::Set(values) | Object::FrozenSet(values) => values.to_vec(),
                    _ => return Err(RuntimeError::new("intersection() receiver must be set")),
                })?;
                for iterable in args {
                    let iterable_items = self.collect_iterable_values(iterable)?;
                    let other = self.dedup_hashable_values_runtime(iterable_items)?;
                    let mut filtered = Vec::new();
                    for item in out.into_iter() {
                        if self.sequence_contains_runtime_value(&other, &item)? {
                            filtered.push(item);
                        }
                    }
                    out = filtered;
                }
                if matches!(&*receiver.kind(), Object::FrozenSet(_)) {
                    Ok(NativeCallResult::Value(self.heap.alloc_frozenset(out)))
                } else {
                    Ok(NativeCallResult::Value(self.heap.alloc_set(out)))
                }
            }
            NativeMethodKind::SetDifference => {
                let mut out = self.dedup_hashable_values_runtime(match &*receiver.kind() {
                    Object::Set(values) | Object::FrozenSet(values) => values.to_vec(),
                    _ => return Err(RuntimeError::new("difference() receiver must be set")),
                })?;
                for iterable in args {
                    let iterable_items = self.collect_iterable_values(iterable)?;
                    let other = self.dedup_hashable_values_runtime(iterable_items)?;
                    let mut filtered = Vec::new();
                    for item in out.into_iter() {
                        if !self.sequence_contains_runtime_value(&other, &item)? {
                            filtered.push(item);
                        }
                    }
                    out = filtered;
                }
                if matches!(&*receiver.kind(), Object::FrozenSet(_)) {
                    Ok(NativeCallResult::Value(self.heap.alloc_frozenset(out)))
                } else {
                    Ok(NativeCallResult::Value(self.heap.alloc_set(out)))
                }
            }
            NativeMethodKind::SetIsSuperset => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("issuperset() expects one argument"));
                }
                let other_items = self.collect_iterable_values(args[0].clone())?;
                let other_values = self.dedup_hashable_values_runtime(other_items)?;
                let receiver_values = match &*receiver.kind() {
                    Object::Set(values) | Object::FrozenSet(values) => values.to_vec(),
                    _ => return Err(RuntimeError::new("issuperset() receiver must be set")),
                };
                let mut is_superset = true;
                for item in &other_values {
                    if !self.sequence_contains_runtime_value(&receiver_values, item)? {
                        is_superset = false;
                        break;
                    }
                }
                Ok(NativeCallResult::Value(Value::Bool(is_superset)))
            }
            NativeMethodKind::SetIsSubset => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("issubset() expects one argument"));
                }
                let other_items = self.collect_iterable_values(args[0].clone())?;
                let other = self.dedup_hashable_values_runtime(other_items)?;
                let receiver_values = match &*receiver.kind() {
                    Object::Set(values) | Object::FrozenSet(values) => values.to_vec(),
                    _ => return Err(RuntimeError::new("issubset() receiver must be set")),
                };
                let mut is_subset = true;
                for item in &receiver_values {
                    if !self.sequence_contains_runtime_value(&other, item)? {
                        is_subset = false;
                        break;
                    }
                }
                Ok(NativeCallResult::Value(Value::Bool(is_subset)))
            }
            NativeMethodKind::SetIsDisjoint => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("isdisjoint() expects one argument"));
                }
                let other_items = self.collect_iterable_values(args[0].clone())?;
                let other = self.dedup_hashable_values_runtime(other_items)?;
                let receiver_values = match &*receiver.kind() {
                    Object::Set(values) | Object::FrozenSet(values) => values.to_vec(),
                    _ => return Err(RuntimeError::new("isdisjoint() receiver must be set")),
                };
                let mut is_disjoint = true;
                for item in &receiver_values {
                    if self.sequence_contains_runtime_value(&other, item)? {
                        is_disjoint = false;
                        break;
                    }
                }
                Ok(NativeCallResult::Value(Value::Bool(is_disjoint)))
            }
            NativeMethodKind::ClassRegister => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("register() expects one argument"));
                }
                Ok(NativeCallResult::Value(
                    args.first().cloned().expect("checked len"),
                ))
            }
            NativeMethodKind::PropertyGet => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("__get__() expects 2 arguments"));
                }
                let obj = args.first().cloned().expect("checked len");
                if matches!(obj, Value::None) {
                    return Ok(NativeCallResult::Value(Value::Instance(receiver)));
                }
                let Some((fget, _, _, _, _)) = self.property_descriptor_parts(&receiver) else {
                    return Err(RuntimeError::type_error("property receiver is invalid"));
                };
                if matches!(fget, Value::None) {
                    return Err(RuntimeError::attribute_error("unreadable attribute"));
                }
                match self.call_internal(fget, vec![obj], HashMap::new())? {
                    InternalCallOutcome::Value(value) => Ok(NativeCallResult::Value(value)),
                    InternalCallOutcome::CallerExceptionHandled => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::PropertySet => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("__set__() expects 2 arguments"));
                }
                let obj = args.first().cloned().expect("checked len");
                let value = args.get(1).cloned().expect("checked len");
                let Some((_, fset, _, _, _)) = self.property_descriptor_parts(&receiver) else {
                    return Err(RuntimeError::type_error("property receiver is invalid"));
                };
                if matches!(fset, Value::None) {
                    return Err(RuntimeError::attribute_error("can't set attribute"));
                }
                match self.call_internal(fset, vec![obj, value], HashMap::new())? {
                    InternalCallOutcome::Value(_) => Ok(NativeCallResult::Value(Value::None)),
                    InternalCallOutcome::CallerExceptionHandled => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::PropertyDelete => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("__delete__() expects 1 argument"));
                }
                let obj = args.first().cloned().expect("checked len");
                let Some((_, _, fdel, _, _)) = self.property_descriptor_parts(&receiver) else {
                    return Err(RuntimeError::type_error("property receiver is invalid"));
                };
                if matches!(fdel, Value::None) {
                    return Err(RuntimeError::attribute_error("can't delete attribute"));
                }
                match self.call_internal(fdel, vec![obj], HashMap::new())? {
                    InternalCallOutcome::Value(_) => Ok(NativeCallResult::Value(Value::None)),
                    InternalCallOutcome::CallerExceptionHandled => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::PropertyGetter => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("getter() expects one argument"));
                }
                let getter = args.first().cloned().expect("checked len");
                if !matches!(getter, Value::None) && !self.is_callable_value(&getter) {
                    return Err(RuntimeError::new("getter() argument must be callable"));
                }
                let updated = self.clone_property_descriptor_with(
                    &receiver,
                    Some(getter),
                    None,
                    None,
                    None,
                    None,
                )?;
                Ok(NativeCallResult::Value(updated))
            }
            NativeMethodKind::PropertySetter => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("setter() expects one argument"));
                }
                let setter = args.first().cloned().expect("checked len");
                if !matches!(setter, Value::None) && !self.is_callable_value(&setter) {
                    return Err(RuntimeError::new("setter() argument must be callable"));
                }
                let updated = self.clone_property_descriptor_with(
                    &receiver,
                    None,
                    Some(setter),
                    None,
                    None,
                    None,
                )?;
                Ok(NativeCallResult::Value(updated))
            }
            NativeMethodKind::PropertyDeleter => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("deleter() expects one argument"));
                }
                let deleter = args.first().cloned().expect("checked len");
                if !matches!(deleter, Value::None) && !self.is_callable_value(&deleter) {
                    return Err(RuntimeError::new("deleter() argument must be callable"));
                }
                let updated = self.clone_property_descriptor_with(
                    &receiver,
                    None,
                    None,
                    Some(deleter),
                    None,
                    None,
                )?;
                Ok(NativeCallResult::Value(updated))
            }
            NativeMethodKind::PropertySetName => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("__set_name__() expects 2 arguments"));
                }
                let explicit_name = args.get(1).cloned().expect("checked len");
                if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
                    instance_data
                        .attrs
                        .insert("__name__".to_string(), explicit_name);
                    return Ok(NativeCallResult::Value(Value::None));
                }
                Err(RuntimeError::type_error("property receiver is invalid"))
            }
            NativeMethodKind::CachedPropertyGet => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("__get__() expects 1-2 arguments"));
                }
                let obj = args.first().cloned().expect("checked len");
                let cache_is_class_dict = matches!(obj, Value::Class(_));
                if matches!(obj, Value::None) {
                    return Ok(NativeCallResult::Value(Value::Instance(receiver)));
                }
                let type_name = match &obj {
                    Value::Instance(instance) => {
                        class_name_for_instance(instance).unwrap_or_else(|| "object".to_string())
                    }
                    Value::Class(class_ref) => match &*class_ref.kind() {
                        Object::Class(class_data) => class_data
                            .metaclass
                            .as_ref()
                            .and_then(|meta| match &*meta.kind() {
                                Object::Class(meta_data) => Some(meta_data.name.clone()),
                                _ => None,
                            })
                            .unwrap_or_else(|| "type".to_string()),
                        _ => "type".to_string(),
                    },
                    _ => self
                        .class_of_value(&obj)
                        .and_then(|class| match &*class.kind() {
                            Object::Class(class_data) => Some(class_data.name.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "object".to_string()),
                };
                let Some((func, attr_name, _doc)) =
                    self.cached_property_descriptor_parts(&receiver)
                else {
                    return Err(RuntimeError::new("cached_property receiver is invalid"));
                };
                let Some(attr_name) = attr_name else {
                    return Err(RuntimeError::type_error(
                        "Cannot use cached_property instance without calling __set_name__ on it.",
                    ));
                };
                let cache = match self.builtin_getattr(
                    vec![obj.clone(), Value::Str("__dict__".to_string())],
                    HashMap::new(),
                ) {
                    Ok(Value::Dict(dict)) => dict,
                    Ok(_) | Err(_) if matches!(obj, Value::Class(_)) => {
                        return Err(RuntimeError::type_error(format!(
                            "The '__dict__' attribute on '{}' instance does not support item assignment for caching '{}' property.",
                            type_name, attr_name
                        )));
                    }
                    Ok(_) => {
                        return Err(RuntimeError::type_error(format!(
                            "The '__dict__' attribute on '{}' instance does not support item assignment for caching '{}' property.",
                            type_name, attr_name
                        )));
                    }
                    Err(err) if runtime_error_matches_exception(&err, "AttributeError") => {
                        return Err(RuntimeError::type_error(format!(
                            "No '__dict__' attribute on '{}' instance to cache '{}' property.",
                            type_name, attr_name
                        )));
                    }
                    Err(err) => return Err(err),
                };
                if let Some(existing) = dict_get_value(&cache, &Value::Str(attr_name.clone())) {
                    return Ok(NativeCallResult::Value(existing));
                }
                let value = match self.call_internal(func, vec![obj], HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Ok(NativeCallResult::PropagatedException);
                    }
                };
                if cache_is_class_dict {
                    return Err(RuntimeError::type_error(format!(
                        "The '__dict__' attribute on '{}' instance does not support item assignment for caching '{}' property.",
                        type_name, attr_name
                    )));
                }
                dict_set_value_checked(&cache, Value::Str(attr_name), value.clone())?;
                Ok(NativeCallResult::Value(value))
            }
            NativeMethodKind::CachedPropertySetName => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("__set_name__() expects 2 arguments"));
                }
                let name = match args.get(1).cloned() {
                    Some(Value::Str(name)) => name,
                    _ => return Err(RuntimeError::type_error("__set_name__() name must be str")),
                };
                let Some((_func, existing_attr_name, _doc)) =
                    self.cached_property_descriptor_parts(&receiver)
                else {
                    return Err(RuntimeError::new("cached_property receiver is invalid"));
                };
                if let Some(existing) = existing_attr_name
                    && existing != name
                {
                    return Err(RuntimeError::type_error(format!(
                        "Cannot assign the same cached_property to two different names ('{}' and '{}').",
                        existing, name
                    )));
                }
                if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
                    instance_data
                        .attrs
                        .insert("attrname".to_string(), Value::Str(name));
                    Ok(NativeCallResult::Value(Value::None))
                } else {
                    Err(RuntimeError::new("cached_property receiver is invalid"))
                }
            }
            NativeMethodKind::OperatorItemGetterCall => {
                if !kwargs.is_empty() || args.len() != 1 {
                    return Err(RuntimeError::new("itemgetter expected 1 argument"));
                }
                let items = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("items") {
                        Some(Value::List(obj)) => match &*obj.kind() {
                            Object::List(values) => values.clone(),
                            _ => Vec::new(),
                        },
                        _ => Vec::new(),
                    },
                    _ => return Err(RuntimeError::new("itemgetter receiver is invalid")),
                };
                if items.is_empty() {
                    return Err(RuntimeError::new("itemgetter receiver is invalid"));
                }
                let target = args.first().cloned().expect("checked len");
                if items.len() == 1 {
                    let value = self.getitem_value(target, items[0].clone())?;
                    return Ok(NativeCallResult::Value(value));
                }
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.getitem_value(target.clone(), item)?);
                }
                Ok(NativeCallResult::Value(self.heap.alloc_tuple(out)))
            }
            NativeMethodKind::OperatorAttrGetterCall => {
                if !kwargs.is_empty() || args.len() != 1 {
                    return Err(RuntimeError::new("attrgetter expected 1 argument"));
                }
                let attrs = match &*receiver.kind() {
                    Object::Module(module_data) => match module_data.globals.get("attrs") {
                        Some(Value::List(obj)) => match &*obj.kind() {
                            Object::List(values) => values.clone(),
                            _ => Vec::new(),
                        },
                        _ => Vec::new(),
                    },
                    _ => return Err(RuntimeError::new("attrgetter receiver is invalid")),
                };
                if attrs.is_empty() {
                    return Err(RuntimeError::new("attrgetter receiver is invalid"));
                }
                let target = args.first().cloned().expect("checked len");
                let mut out = Vec::with_capacity(attrs.len());
                for attr in attrs {
                    let Value::Str(path) = attr else {
                        return Err(RuntimeError::new("attribute name must be a string"));
                    };
                    let mut current = target.clone();
                    for part in path.split('.') {
                        current = self.builtin_getattr(
                            vec![current, Value::Str(part.to_string())],
                            HashMap::new(),
                        )?;
                    }
                    out.push(current);
                }
                if out.len() == 1 {
                    Ok(NativeCallResult::Value(out.remove(0)))
                } else {
                    Ok(NativeCallResult::Value(self.heap.alloc_tuple(out)))
                }
            }
            NativeMethodKind::OperatorMethodCallerCall => {
                if !kwargs.is_empty() || args.len() != 1 {
                    return Err(RuntimeError::new("methodcaller expected 1 argument"));
                }
                let (method_name, call_args, call_kwargs) = match &*receiver.kind() {
                    Object::Module(module_data) => {
                        let method_name = match module_data.globals.get("name") {
                            Some(Value::Str(name)) => name.clone(),
                            _ => return Err(RuntimeError::new("methodcaller receiver is invalid")),
                        };
                        let call_args = match module_data.globals.get("args") {
                            Some(Value::List(obj)) => match &*obj.kind() {
                                Object::List(values) => values.clone(),
                                _ => Vec::new(),
                            },
                            _ => Vec::new(),
                        };
                        let mut call_kwargs = HashMap::new();
                        if let Some(Value::Dict(obj)) = module_data.globals.get("kwargs")
                            && let Object::Dict(entries) = &*obj.kind()
                        {
                            for (key, value) in entries {
                                if let Value::Str(name) = key {
                                    call_kwargs.insert(name.clone(), value.clone());
                                }
                            }
                        }
                        (method_name, call_args, call_kwargs)
                    }
                    _ => return Err(RuntimeError::new("methodcaller receiver is invalid")),
                };
                let method = self.builtin_getattr(
                    vec![
                        args.first().cloned().expect("checked len"),
                        Value::Str(method_name),
                    ],
                    HashMap::new(),
                )?;
                match self.call_internal(method, call_args, call_kwargs)? {
                    InternalCallOutcome::Value(value) => Ok(NativeCallResult::Value(value)),
                    InternalCallOutcome::CallerExceptionHandled => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::FunctoolsWrapsDecorator => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("wraps() decorator expects one argument"));
                }
                let wrapped = match &*receiver.kind() {
                    Object::Module(module_data) => module_data
                        .globals
                        .get("wrapped")
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("wraps receiver is invalid"))?,
                    _ => return Err(RuntimeError::new("wraps receiver is invalid")),
                };
                let wrapper = args.first().cloned().expect("checked len");
                self.apply_functools_wraps_metadata(&wrapper, &wrapped)?;
                Ok(NativeCallResult::Value(wrapper))
            }
            NativeMethodKind::FunctoolsPartialCall => {
                let (callable, frozen_args, frozen_kwargs) = match &*receiver.kind() {
                    Object::Module(module_data) => {
                        let callable = module_data
                            .globals
                            .get("callable")
                            .cloned()
                            .ok_or_else(|| RuntimeError::new("partial callable missing"))?;
                        let frozen_args = match module_data.globals.get("args") {
                            Some(Value::List(obj)) => match &*obj.kind() {
                                Object::List(values) => values.clone(),
                                _ => Vec::new(),
                            },
                            _ => Vec::new(),
                        };
                        let mut frozen_kwargs = HashMap::new();
                        if let Some(Value::Dict(obj)) = module_data.globals.get("kwargs")
                            && let Object::Dict(entries) = &*obj.kind()
                        {
                            for (key, value) in entries {
                                if let Value::Str(name) = key {
                                    frozen_kwargs.insert(name.clone(), value.clone());
                                }
                            }
                        }
                        (callable, frozen_args, frozen_kwargs)
                    }
                    _ => return Err(RuntimeError::new("partial receiver is invalid")),
                };
                let mut combined_args = frozen_args;
                combined_args.extend(args);
                let mut combined_kwargs = frozen_kwargs;
                for (name, value) in kwargs.drain() {
                    combined_kwargs.insert(name, value);
                }
                match self.call_internal(callable, combined_args, combined_kwargs)? {
                    InternalCallOutcome::Value(value) => Ok(NativeCallResult::Value(value)),
                    InternalCallOutcome::CallerExceptionHandled => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::FunctoolsCmpToKeyCall => {
                if !kwargs.is_empty() || args.len() != 1 {
                    return Err(RuntimeError::new(
                        "cmp_to_key() callable expects one argument",
                    ));
                }
                let comparator = match &*receiver.kind() {
                    Object::Module(module_data) => module_data
                        .globals
                        .get("cmp")
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("cmp_to_key receiver is invalid"))?,
                    _ => return Err(RuntimeError::new("cmp_to_key receiver is invalid")),
                };
                let key_obj = match self
                    .heap
                    .alloc_module(ModuleObject::new("__functools_cmp_key_item__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *key_obj.kind_mut() {
                    module_data.globals.insert("cmp".to_string(), comparator);
                    module_data.globals.insert(
                        "obj".to_string(),
                        args.first().cloned().expect("checked len"),
                    );
                }
                Ok(NativeCallResult::Value(Value::Module(key_obj)))
            }
        }
    }

    pub(super) fn make_immediate_coroutine(&mut self, value: Value) -> Value {
        let mut code = CodeObject::new("<awaitable>", "<builtin>");
        let const_idx = code.add_const(value);
        code.instructions
            .push(Instruction::new(Opcode::LoadConst, Some(const_idx)));
        code.instructions
            .push(Instruction::new(Opcode::ReturnValue, None));
        code.is_generator = true;
        code.is_coroutine = true;
        code.is_async_generator = false;
        let code = Rc::new(code);
        let module = self
            .frames
            .last()
            .map(|frame| frame.module.clone())
            .unwrap_or_else(|| self.main_module.clone());
        let mut frame = Box::new(Frame::new(
            code.clone(),
            module,
            false,
            false,
            Vec::new(),
            None,
        ));
        let generator =
            match self
                .heap
                .alloc_generator(GeneratorObject::new(code.clone(), true, false))
            {
                Value::Generator(obj) => obj,
                _ => unreachable!(),
            };
        frame.generator_owner = Some(generator.clone());
        self.generator_states.insert(generator.id(), frame);
        Value::Generator(generator)
    }

    pub(super) fn awaitable_from_value(&mut self, value: Value) -> Result<Value, RuntimeError> {
        match value {
            Value::Generator(generator) => {
                let (is_coroutine, is_async_generator, is_iterable_coroutine) =
                    match &*generator.kind() {
                        Object::Generator(state) => (
                            state.is_coroutine,
                            state.is_async_generator,
                            state.code.is_iterable_coroutine,
                        ),
                        _ => (false, false, false),
                    };
                if is_coroutine || is_iterable_coroutine {
                    Ok(Value::Generator(generator))
                } else if is_async_generator {
                    Err(RuntimeError::type_error(
                        "async generator object is not awaitable",
                    ))
                } else {
                    Err(RuntimeError::type_error("object is not awaitable"))
                }
            }
            Value::Iterator(_) => Err(RuntimeError::type_error("object is not awaitable")),
            other => {
                let method = self
                    .lookup_bound_special_method(&other, "__await__")?
                    .ok_or_else(|| RuntimeError::type_error("object is not awaitable"))?;
                match self.call_internal(method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(awaitable) => {
                        let iterator = match self.to_iterator_value(awaitable) {
                            Ok(value) => value,
                            Err(_) => {
                                return Err(RuntimeError::type_error(
                                    "__await__() returned non-iterator",
                                ));
                            }
                        };
                        if let Value::Generator(generator) = &iterator
                            && let Object::Generator(state) = &*generator.kind()
                            && state.is_async_generator
                        {
                            return Err(RuntimeError::new(
                                "__await__() returned an async generator",
                            ));
                        }
                        Ok(iterator)
                    }
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(RuntimeError::new("__await__() failed"))
                    }
                }
            }
        }
    }

    pub(super) fn run_awaitable(&mut self, awaitable: Value) -> Result<Value, RuntimeError> {
        let iterator = self.awaitable_from_value(awaitable)?;
        loop {
            match self.next_from_iterator_value(&iterator)? {
                GeneratorResumeOutcome::Yield(_) => {}
                GeneratorResumeOutcome::Complete(value) => return Ok(value),
                GeneratorResumeOutcome::PropagatedException => {
                    self.propagate_pending_generator_exception()?;
                    return Err(RuntimeError::new("awaitable execution failed"));
                }
            }
        }
    }

    pub(super) fn is_awaitable_value(&self, value: &Value) -> bool {
        match value {
            Value::Generator(generator) => match &*generator.kind() {
                Object::Generator(state) => state.is_coroutine || state.code.is_iterable_coroutine,
                _ => false,
            },
            Value::Iterator(_) => false,
            _ => self
                .class_of_value(value)
                .and_then(|class| class_attr_lookup(&class, "__await__"))
                .is_some(),
        }
    }

    pub(super) fn ensure_sync_iterator_target(&self, value: &Value) -> Result<(), RuntimeError> {
        if let Value::Generator(generator) = value
            && let Object::Generator(state) = &*generator.kind()
            && (state.is_coroutine || state.is_async_generator)
        {
            return Err(RuntimeError::type_error("object is not iterable"));
        }
        Ok(())
    }

    pub(super) fn generator_for_iter_next(
        &mut self,
        generator: &ObjRef,
    ) -> Result<GeneratorResumeOutcome, RuntimeError> {
        self.resume_generator(generator, None, None, GeneratorResumeKind::Next)
    }

    pub(super) fn sequence_iterator_via_getitem(
        &mut self,
        target: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some(getitem) = self.lookup_bound_special_method(&target, "__getitem__")? else {
            return Ok(None);
        };
        Ok(Some(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::SequenceGetItem { target, getitem },
            index: 0,
        })))
    }

    fn origin_is_tuple_alias_target(origin: &Value) -> bool {
        match origin {
            Value::Builtin(BuiltinFunction::Tuple) => true,
            Value::Class(class) => match &*class.kind() {
                Object::Class(class_data) => class_data.name == "tuple",
                _ => false,
            },
            _ => false,
        }
    }

    fn generic_alias_type_class_display_name(class: &ObjRef) -> String {
        let class_kind = class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return "<class ?>".to_string();
        };
        if class_data.name == "NoneType" {
            return "None".to_string();
        }
        let module = class_data
            .attrs
            .get("__module__")
            .and_then(|value| match value {
                Value::Str(name) => Some(name.as_str()),
                _ => None,
            })
            .unwrap_or("builtins");
        let qualname = class_data
            .attrs
            .get("__qualname__")
            .and_then(|value| match value {
                Value::Str(name) => Some(name.as_str()),
                _ => None,
            })
            .unwrap_or(class_data.name.as_str());
        if module == "builtins" {
            qualname.to_string()
        } else {
            format!("{module}.{qualname}")
        }
    }

    fn generic_alias_builtin_type_name(builtin: BuiltinFunction) -> Option<&'static str> {
        match builtin {
            BuiltinFunction::Type => Some("type"),
            BuiltinFunction::Bool => Some("bool"),
            BuiltinFunction::Int => Some("int"),
            BuiltinFunction::Float => Some("float"),
            BuiltinFunction::Complex => Some("complex"),
            BuiltinFunction::Str => Some("str"),
            BuiltinFunction::List => Some("list"),
            BuiltinFunction::Tuple => Some("tuple"),
            BuiltinFunction::Dict => Some("dict"),
            BuiltinFunction::Set => Some("set"),
            BuiltinFunction::FrozenSet => Some("frozenset"),
            BuiltinFunction::Bytes => Some("bytes"),
            BuiltinFunction::ByteArray => Some("bytearray"),
            BuiltinFunction::MemoryView => Some("memoryview"),
            BuiltinFunction::Slice => Some("slice"),
            BuiltinFunction::Range => Some("range"),
            _ => None,
        }
    }

    fn generic_alias_type_repr(&mut self, value: Value) -> Result<String, RuntimeError> {
        if self
            .builtins
            .get("Ellipsis")
            .is_some_and(|ellipsis| match (&value, ellipsis) {
                (Value::Instance(left), Value::Instance(right)) => left.id() == right.id(),
                _ => &value == ellipsis,
            })
        {
            return Ok("...".to_string());
        }
        match value {
            Value::None => Ok("None".to_string()),
            Value::Class(class) => Ok(Self::generic_alias_type_class_display_name(&class)),
            Value::Builtin(builtin) => Ok(Self::generic_alias_builtin_type_name(builtin)
                .map(str::to_string)
                .unwrap_or_else(|| format_value(&Value::Builtin(builtin)))),
            other => match self.builtin_repr(vec![other], HashMap::new())? {
                Value::Str(text) => Ok(text),
                repr_value => Ok(format_value(&repr_value)),
            },
        }
    }

    fn generic_alias_list_repr(&mut self, list_obj: &ObjRef) -> Result<String, RuntimeError> {
        let len = {
            let list_kind = list_obj.kind();
            let Object::List(values) = &*list_kind else {
                return Err(RuntimeError::new(
                    "GenericAlias repr list argument is invalid",
                ));
            };
            values.len()
        };
        let mut rendered = String::new();
        rendered.push('[');
        for index in 0..len {
            if index > 0 {
                rendered.push_str(", ");
            }
            let item = {
                let list_kind = list_obj.kind();
                let Object::List(values) = &*list_kind else {
                    return Err(RuntimeError::new(
                        "GenericAlias repr list argument is invalid",
                    ));
                };
                values
                    .get(index)
                    .cloned()
                    .ok_or_else(|| RuntimeError::index_error("list index out of range"))?
            };
            rendered.push_str(&self.generic_alias_type_repr(item)?);
        }
        rendered.push(']');
        Ok(rendered)
    }

    fn generic_alias_repr(&mut self, alias: &ObjRef) -> Result<String, RuntimeError> {
        let alias_value = Value::Instance(alias.clone());
        let Some((origin, args)) = self.generic_alias_parts_from_value(&alias_value) else {
            return Err(RuntimeError::new("GenericAlias repr receiver is invalid"));
        };
        let is_unpacked = self
            .optional_getattr_value(alias_value, "__unpacked__")?
            .map(|flag| self.truthy_from_value(&flag))
            .transpose()?
            .unwrap_or(false);

        let mut rendered = String::new();
        if is_unpacked {
            rendered.push('*');
        }
        rendered.push_str(&self.generic_alias_type_repr(origin)?);
        rendered.push('[');
        if args.is_empty() {
            rendered.push_str("()");
        } else {
            for (index, arg) in args.iter().enumerate() {
                if index > 0 {
                    rendered.push_str(", ");
                }
                let arg_text = if let Value::List(list_obj) = arg {
                    self.generic_alias_list_repr(list_obj)?
                } else {
                    self.generic_alias_type_repr(arg.clone())?
                };
                rendered.push_str(&arg_text);
            }
        }
        rendered.push(']');
        Ok(rendered)
    }

    fn typing_alias_unpack_iterator(&mut self, alias: &ObjRef) -> Option<Value> {
        let (origin, args) = {
            let alias_kind = alias.kind();
            let Object::Instance(instance_data) = &*alias_kind else {
                return None;
            };
            let class_kind = instance_data.class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return None;
            };
            if !matches!(class_data.name.as_str(), "GenericAlias" | "_GenericAlias") {
                return None;
            }
            let origin = instance_data.attrs.get("__origin__")?.clone();
            let args = instance_data.attrs.get("__args__")?.clone();
            (origin, args)
        };

        if !Self::origin_is_tuple_alias_target(&origin) {
            let unpack = self.typing_helper_callable("Unpack").ok()?;
            let unpacked = self
                .getitem_value(unpack, Value::Instance(alias.clone()))
                .ok()?;
            let list = match self.heap.alloc_list(vec![unpacked]) {
                Value::List(list) => list,
                _ => unreachable!(),
            };
            return Some(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::List(list),
                index: 0,
            }));
        }

        let unpacked = self.alloc_generic_alias_instance(origin.clone(), args.clone());
        if let Value::Instance(unpacked_instance) = &unpacked
            && let Object::Instance(unpacked_data) = &mut *unpacked_instance.kind_mut()
        {
            unpacked_data
                .attrs
                .insert("__unpacked__".to_string(), Value::Bool(true));
            if Self::origin_is_tuple_alias_target(&origin) {
                unpacked_data
                    .attrs
                    .insert("__typing_unpacked_tuple_args__".to_string(), args);
            }
        }

        let list = match self.heap.alloc_list(vec![unpacked]) {
            Value::List(list) => list,
            _ => unreachable!(),
        };
        Some(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::List(list),
            index: 0,
        }))
    }

    fn typing_no_default_marker(&mut self) -> Option<Value> {
        for module_name in ["_typing", "typing"] {
            let Some(module) = self.modules.get(module_name).cloned() else {
                continue;
            };
            let Object::Module(module_data) = &*module.kind() else {
                continue;
            };
            if let Some(value) = module_data.globals.get("NoDefault").cloned() {
                return Some(value);
            }
        }
        None
    }

    fn builtin_typing_nodefault_new(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error("NoDefaultType takes no arguments"));
        }
        self.typing_no_default_marker()
            .ok_or_else(|| RuntimeError::new("NoDefault singleton is unavailable"))
    }

    fn builtin_typing_nodefault_repr(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "NoDefaultType.__repr__() takes no arguments",
            ));
        }
        Ok(Value::Str("typing.NoDefault".to_string()))
    }

    fn builtin_typing_nodefault_reduce(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "NoDefaultType.__reduce__() takes no arguments",
            ));
        }
        Ok(Value::Str("NoDefault".to_string()))
    }

    fn typing_param_kind_name(&self, value: &Value) -> Option<&'static str> {
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
            Some(Value::Str(name)) => Some(name.as_str()),
            _ => None,
        };
        if !matches!(module_name, Some("typing" | "_typing")) {
            return None;
        }
        match class_data.name.as_str() {
            "TypeVar" => Some("TypeVar"),
            "ParamSpec" => Some("ParamSpec"),
            "TypeVarTuple" => Some("TypeVarTuple"),
            _ => None,
        }
    }

    pub(super) fn typing_helper_callable(
        &mut self,
        helper_name: &str,
    ) -> Result<Value, RuntimeError> {
        let module = if let Some(module) = self.modules.get("typing").cloned() {
            module
        } else {
            self.import_module_object("typing")?
        };
        self.builtin_getattr(
            vec![Value::Module(module), Value::Str(helper_name.to_string())],
            HashMap::new(),
        )
    }

    fn call_typing_helper(
        &mut self,
        helper_name: &str,
        helper_args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let callable = self.typing_helper_callable(helper_name)?;
        match self.call_internal(callable, helper_args, HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("typing helper call failed"))
            }
        }
    }

    fn typing_sequence_items(value: &Value) -> Result<Vec<Value>, RuntimeError> {
        match value {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(items) => Ok(items.clone()),
                _ => Err(RuntimeError::type_error(
                    "typing parameters must be a sequence",
                )),
            },
            Value::List(list_obj) => match &*list_obj.kind() {
                Object::List(items) => Ok(items.clone()),
                _ => Err(RuntimeError::type_error(
                    "typing parameters must be a sequence",
                )),
            },
            _ => Err(RuntimeError::type_error(
                "typing parameters must be a sequence",
            )),
        }
    }

    fn typing_param_default(&self, value: &Value) -> Option<Value> {
        let Value::Instance(instance) = value else {
            return None;
        };
        let instance_kind = instance.kind();
        let Object::Instance(instance_data) = &*instance_kind else {
            return None;
        };
        instance_data.attrs.get("__default__").cloned()
    }

    fn typing_marker_is_same(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Class(left), Value::Class(right)) => left.id() == right.id(),
            (Value::Instance(left), Value::Instance(right)) => left.id() == right.id(),
            (Value::Module(left), Value::Module(right)) => left.id() == right.id(),
            _ => left == right,
        }
    }

    fn typing_typevar_subst_fallback(&mut self, mut arg: Value) -> Result<Value, RuntimeError> {
        let type_check_message = "Parameters to generic types must be types.";
        arg = match self.call_typing_helper(
            "_type_check",
            vec![arg.clone(), Value::Str(type_check_message.to_string())],
        ) {
            Ok(value) => value,
            Err(err) if runtime_error_matches_exception(&err, "AttributeError") => arg,
            Err(err) => return Err(err),
        };

        let class_name = match &arg {
            Value::Instance(instance) => class_name_for_instance(instance),
            _ => None,
        };

        let mut reject = false;
        if class_name.as_deref() == Some("_GenericAlias")
            && let Some(origin) = self.optional_getattr_value(arg.clone(), "__origin__")?
        {
            if let Ok(unpack) = self.typing_helper_callable("Unpack")
                && Self::typing_marker_is_same(&origin, &unpack)
            {
                reject = true;
            }
        } else if class_name.as_deref() == Some("GenericAlias")
            && let Some(unpacked) = self.optional_getattr_value(arg.clone(), "__unpacked__")?
            && self.truthy_from_value(&unpacked)?
        {
            reject = true;
        }

        if reject {
            return Err(RuntimeError::type_error(format!(
                "{} is not valid as type argument",
                format_value(&arg)
            )));
        }

        Ok(arg)
    }

    fn parse_name_error_missing_name(message: &str) -> Option<String> {
        let marker = "name '";
        let start = message.find(marker)? + marker.len();
        let tail = &message[start..];
        let end = tail.find('\'')?;
        Some(tail[..end].to_string())
    }

    fn name_error_missing_name(&self, err: &RuntimeError) -> Option<String> {
        if !runtime_error_matches_exception(err, "NameError") {
            return None;
        }
        if let Some(exception) = err.exception.as_ref() {
            if let Some(Value::Str(name)) = exception.attrs.borrow().get("name").cloned() {
                return Some(name);
            }
            if let Some(message) = exception.message.as_ref()
                && let Some(name) = Self::parse_name_error_missing_name(message)
            {
                return Some(name);
            }
        }
        Self::parse_name_error_missing_name(&err.message)
    }

    fn function_annotation_locals_from_dict(
        &self,
        annotation_locals: Option<&ObjRef>,
    ) -> HashMap<String, Value> {
        let mut out = HashMap::new();
        let Some(locals_dict) = annotation_locals else {
            return out;
        };
        let Object::Dict(entries) = &*locals_dict.kind() else {
            return out;
        };
        for (key, value) in entries {
            if let Value::Str(name) = key {
                out.insert(name.clone(), value.clone());
            }
        }
        out
    }

    fn annotation_owner_from_call_stack(&self) -> Option<Value> {
        for frame in self.frames.iter().rev() {
            if let Some(owner_index) = frame.code.name_to_index.get("owner").copied()
                && owner_index < frame.fast_locals.len()
                && let Some(owner) = frame.fast_locals[owner_index].clone()
                && !matches!(owner, Value::None)
            {
                return Some(owner);
            }
            if let Some(owner) = frame.locals.get("owner").cloned()
                && !matches!(owner, Value::None)
            {
                return Some(owner);
            }
        }
        None
    }

    fn annotation_call_owner_from_call_stack(&self) -> Option<Value> {
        for frame in self.frames.iter().rev() {
            if frame.code.name != "call_annotate_function" {
                continue;
            }
            if let Some(owner_index) = frame.code.name_to_index.get("owner").copied()
                && owner_index < frame.fast_locals.len()
                && let Some(owner) = frame.fast_locals[owner_index].clone()
                && !matches!(owner, Value::None)
            {
                return Some(owner);
            }
            if let Some(owner) = frame.locals.get("owner").cloned()
                && !matches!(owner, Value::None)
            {
                return Some(owner);
            }
            return None;
        }
        None
    }

    fn function_annotation_forward_ref(
        &mut self,
        name: &str,
        function_owner: Value,
        module_name: Option<String>,
    ) -> Result<Value, RuntimeError> {
        let annotationlib = if let Some(module) = self.modules.get("annotationlib").cloned() {
            module
        } else {
            self.load_module("annotationlib")?
        };
        let forward_ref_ctor = self.builtin_getattr(
            vec![
                Value::Module(annotationlib),
                Value::Str("ForwardRef".to_string()),
            ],
            HashMap::new(),
        )?;
        let mut kwargs = HashMap::new();
        kwargs.insert("owner".to_string(), function_owner);
        if let Some(module_name) = module_name {
            kwargs.insert("module".to_string(), Value::Str(module_name));
        }
        match self.call_internal(forward_ref_ctor, vec![Value::Str(name.to_string())], kwargs)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("annotationlib.ForwardRef() failed"))
            }
        }
    }

    fn function_annotation_eval_forward_ref(
        &mut self,
        text: &str,
        function_module: &ObjRef,
        annotation_locals: Option<&ObjRef>,
        function_owner: Value,
    ) -> Result<Value, RuntimeError> {
        let mut locals = self.function_annotation_locals_from_dict(annotation_locals);
        let module_name = self.annotation_module_name_for_owner(&function_owner);
        for _ in 0..32 {
            let mut eval_args = vec![
                Value::Str(text.to_string()),
                Value::Module(function_module.clone()),
            ];
            if !locals.is_empty() {
                let entries = locals
                    .iter()
                    .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                    .collect::<Vec<_>>();
                eval_args.push(self.heap.alloc_dict(entries));
            }
            match self.builtin_eval(eval_args, HashMap::new()) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    let Some(name) = self.name_error_missing_name(&err) else {
                        return Ok(Value::Str(text.to_string()));
                    };
                    if locals.contains_key(&name) {
                        return Ok(Value::Str(text.to_string()));
                    }
                    let forward_ref = self.function_annotation_forward_ref(
                        &name,
                        function_owner.clone(),
                        module_name.clone(),
                    )?;
                    locals.insert(name, forward_ref);
                }
            }
        }
        Ok(Value::Str(text.to_string()))
    }

    fn annotation_module_name_for_owner(&self, owner: &Value) -> Option<String> {
        match owner {
            Value::Class(class) => match &*class.kind() {
                Object::Class(class_data) => class_data.attrs.get("__module__").and_then(|value| {
                    if let Value::Str(name) = value {
                        Some(name.clone())
                    } else {
                        None
                    }
                }),
                _ => None,
            },
            _ => None,
        }
    }

    fn function_annotation_eval_value(
        &mut self,
        text: &str,
        function_module: &ObjRef,
        annotation_locals: Option<&ObjRef>,
    ) -> Result<Value, RuntimeError> {
        let mut eval_args = vec![
            Value::Str(text.to_string()),
            Value::Module(function_module.clone()),
        ];
        if let Some(annotation_locals) = annotation_locals {
            eval_args.push(Value::Dict(annotation_locals.clone()));
        }
        self.builtin_eval(eval_args, HashMap::new())
    }

    fn class_annotation_eval_value_dynamic(
        &mut self,
        text: &str,
        function_module: &ObjRef,
        annotation_locals: Option<&ObjRef>,
    ) -> Result<Value, RuntimeError> {
        let mut locals = self.function_annotation_locals_from_dict(annotation_locals);
        for _ in 0..32 {
            let mut eval_args = vec![
                Value::Str(text.to_string()),
                Value::Module(function_module.clone()),
            ];
            if !locals.is_empty() {
                let entries = locals
                    .iter()
                    .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                    .collect::<Vec<_>>();
                eval_args.push(self.heap.alloc_dict(entries));
            }
            match self.builtin_eval(eval_args, HashMap::new()) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    let Some(name) = self.name_error_missing_name(&err) else {
                        return Err(err);
                    };
                    if locals.contains_key(&name) {
                        return Err(err);
                    }
                    let Some(value) = self.annotation_value_from_call_stack(&name) else {
                        return Err(err);
                    };
                    locals.insert(name, value);
                }
            }
        }
        Err(RuntimeError::new(
            "annotation evaluation exceeded lookup retries",
        ))
    }

    fn annotation_value_from_call_stack(&self, name: &str) -> Option<Value> {
        for frame in self.frames.iter().rev() {
            if let Some(slot_index) = frame.code.name_to_index.get(name).copied()
                && slot_index < frame.fast_locals.len()
                && let Some(value) = frame.fast_locals[slot_index].clone()
            {
                return Some(value);
            }
            if let Some(value) = frame.locals.get(name).cloned() {
                return Some(value);
            }
            if let Some(fallback) = &frame.locals_fallback
                && let Some(value) = fallback.get(name).cloned()
            {
                return Some(value);
            }
        }
        None
    }

    fn function_annotation_format_string(&self, text: &str) -> String {
        if let Ok(expr) = parser::parse_expression(text)
            && let ExprKind::Constant(Constant::Str(value)) = expr.node
        {
            return value;
        }
        text.to_string()
    }

    fn builtin_typing_typeparam_has_default(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        kwargs.clear();
        if args.len() != 1 {
            return Err(RuntimeError::type_error(
                "has_default() expects one argument",
            ));
        }
        let type_param = args.remove(0);
        let Some(default_value) = self.typing_param_default(&type_param) else {
            return Ok(Value::Bool(false));
        };
        if let Some(marker) = self.typing_no_default_marker()
            && Self::typing_marker_is_same(&default_value, &marker)
        {
            return Ok(Value::Bool(false));
        }
        Ok(Value::Bool(true))
    }

    fn builtin_typing_generic_class_getitem(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let helper = match self.typing_helper_callable("_generic_class_getitem") {
            Ok(value) => value,
            Err(err) if runtime_error_matches_exception(&err, "AttributeError") => {
                return Err(RuntimeError::type_error(
                    "typing._generic_class_getitem is unavailable",
                ));
            }
            Err(err) => return Err(err),
        };
        match self.call_internal(helper, args, kwargs)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("typing helper call failed"))
            }
        }
    }

    fn builtin_typing_generic_init_subclass(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let helper = match self.typing_helper_callable("_generic_init_subclass") {
            Ok(value) => value,
            Err(err) if runtime_error_matches_exception(&err, "AttributeError") => {
                return Ok(Value::None);
            }
            Err(err) => return Err(err),
        };
        match self.call_internal(helper, args, kwargs)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("typing helper call failed"))
            }
        }
    }

    fn builtin_typing_typeparam_subst(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        kwargs.clear();
        if args.len() != 2 {
            return Err(RuntimeError::type_error(
                "__typing_subst__() expects self and arg",
            ));
        }
        let type_param = args.remove(0);
        let arg = args.remove(0);
        match self.typing_param_kind_name(&type_param) {
            Some("TypeVar") => {
                match self.call_typing_helper("_typevar_subst", vec![type_param, arg.clone()]) {
                    Ok(value) => Ok(value),
                    Err(err) if runtime_error_matches_exception(&err, "AttributeError") => {
                        self.typing_typevar_subst_fallback(arg)
                    }
                    Err(err) => Err(err),
                }
            }
            Some("ParamSpec") => self.call_typing_helper("_paramspec_subst", vec![type_param, arg]),
            Some("TypeVarTuple") => Err(RuntimeError::type_error(
                "Substitution of bare TypeVarTuple is not supported",
            )),
            _ => Err(RuntimeError::type_error(
                "__typing_subst__() receiver must be a type parameter",
            )),
        }
    }

    fn builtin_typing_typeparam_prepare_subst(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        kwargs.clear();
        if args.len() != 3 {
            return Err(RuntimeError::type_error(
                "__typing_prepare_subst__() expects self, alias, and args",
            ));
        }
        let type_param = args.remove(0);
        let alias = args.remove(0);
        let subst_args = args.remove(0);
        match self.typing_param_kind_name(&type_param) {
            Some("TypeVar") => {
                let params_value = self.builtin_getattr(
                    vec![alias.clone(), Value::Str("__parameters__".to_string())],
                    HashMap::new(),
                )?;
                let params = Self::typing_sequence_items(&params_value)?;
                let mut index = None;
                for (i, param) in params.iter().enumerate() {
                    if param == &type_param {
                        index = Some(i);
                        break;
                    }
                }
                let Some(index) = index else {
                    return Err(RuntimeError::type_error(
                        "__typing_prepare_subst__() receiver missing from alias parameters",
                    ));
                };
                let mut subst_items = Self::typing_sequence_items(&subst_args)?;
                if index < subst_items.len() {
                    return Ok(subst_args);
                }
                if index == subst_items.len() {
                    let has_default = self.builtin_typing_typeparam_has_default(
                        vec![type_param.clone()],
                        HashMap::new(),
                    )?;
                    if matches!(has_default, Value::Bool(true))
                        && let Some(default_value) = self.typing_param_default(&type_param)
                    {
                        subst_items.push(default_value);
                        return Ok(self.heap.alloc_tuple(subst_items));
                    }
                }
                Err(RuntimeError::type_error(format!(
                    "Too few arguments for {}; actual {}, expected at least {}",
                    format_value(&alias),
                    subst_items.len(),
                    index + 1
                )))
            }
            Some("ParamSpec") => self.call_typing_helper(
                "_paramspec_prepare_subst",
                vec![type_param, alias, subst_args],
            ),
            Some("TypeVarTuple") => self.call_typing_helper(
                "_typevartuple_prepare_subst",
                vec![type_param, alias, subst_args],
            ),
            _ => Err(RuntimeError::type_error(
                "__typing_prepare_subst__() receiver must be a type parameter",
            )),
        }
    }

    fn mappingproxy_mapping_value(&self, receiver: &ObjRef) -> Result<Value, RuntimeError> {
        let receiver_kind = receiver.kind();
        let Object::Instance(instance_data) = &*receiver_kind else {
            return Err(RuntimeError::type_error("mappingproxy receiver is invalid"));
        };
        instance_data
            .attrs
            .get(MAPPING_PROXY_STORAGE_ATTR)
            .cloned()
            .ok_or_else(|| RuntimeError::type_error("mappingproxy receiver is invalid"))
    }

    fn mappingproxy_call_mapping_method(
        &mut self,
        receiver: &ObjRef,
        method_name: &str,
        args: Vec<Value>,
    ) -> Result<NativeCallResult, RuntimeError> {
        let mapping = self.mappingproxy_mapping_value(receiver)?;
        if let Value::Instance(instance) = &mapping
            && let Object::Instance(instance_data) = &*instance.kind()
            && let Some(method) = class_attr_lookup(&instance_data.class, method_name)
        {
            let callable =
                if let Some(bound) = self.bind_descriptor_method(method.clone(), &mapping)? {
                    bound
                } else {
                    method
                };
            return match self.call_internal(callable, args, HashMap::new())? {
                InternalCallOutcome::Value(value) => Ok(NativeCallResult::Value(value)),
                InternalCallOutcome::CallerExceptionHandled => {
                    Ok(NativeCallResult::PropagatedException)
                }
            };
        }
        let callable = self.builtin_getattr(
            vec![mapping, Value::Str(method_name.to_string())],
            HashMap::new(),
        )?;
        match self.call_internal(callable, args, HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(NativeCallResult::Value(value)),
            InternalCallOutcome::CallerExceptionHandled => {
                Ok(NativeCallResult::PropagatedException)
            }
        }
    }

    fn mappingproxy_view_class(&mut self, items: bool) -> ObjRef {
        let class = if items {
            self.alloc_synthetic_class("_mappingproxy_items_view")
        } else {
            self.alloc_synthetic_class("_mappingproxy_values_view")
        };
        if let Object::Class(class_data) = &mut *class.kind_mut()
            && !class_data.attrs.contains_key("__iter__")
        {
            let kind = if items {
                NativeMethodKind::MappingProxyItemsViewIter
            } else {
                NativeMethodKind::MappingProxyValuesViewIter
            };
            class_data.attrs.insert(
                "__iter__".to_string(),
                Value::Function(
                    self.heap
                        .alloc_native_method(crate::runtime::NativeMethodObject::new(kind)),
                ),
            );
        }
        class
    }

    fn mappingproxy_alloc_view_instance(&mut self, mapping: Value, items: bool) -> Value {
        let class = self.mappingproxy_view_class(items);
        let instance = self.alloc_instance_for_class(&class);
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data
                .attrs
                .insert(MAPPING_PROXY_STORAGE_ATTR.to_string(), mapping);
        }
        Value::Instance(instance)
    }

    pub(super) fn to_iterator_value(&mut self, source: Value) -> Result<Value, RuntimeError> {
        let is_typing_marker_instance = |instance: &ObjRef| -> bool {
            let instance_kind = instance.kind();
            let Object::Instance(instance_data) = &*instance_kind else {
                return false;
            };
            let class_kind = instance_data.class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return false;
            };
            let class_name = class_data.name.as_str();
            if matches!(
                class_name,
                "GenericAlias"
                    | "_GenericAlias"
                    | "UnionType"
                    | "TypeVar"
                    | "TypeVarTuple"
                    | "ParamSpec"
                    | "TypeAliasType"
                    | "ForwardRef"
            ) {
                return true;
            }
            matches!(
                class_data.attrs.get("__module__"),
                Some(Value::Str(module_name))
                    if matches!(module_name.as_str(), "typing" | "_typing" | "types")
            ) && (class_name.contains("GenericAlias")
                || class_name.contains("SpecialForm")
                || class_name.contains("SpecialGenericAlias")
                || class_name.contains("LiteralGenericAlias")
                || matches!(
                    class_name,
                    "Union"
                        | "NewType"
                        | "_SpecialForm"
                        | "_TypedCacheSpecialForm"
                        | "_AnyMeta"
                        | "_TupleType"
                        | "_TypingEllipsis"
                        | "_CallableType"
                        | "_CallableGenericAlias"
                        | "_AnnotatedAlias"
                ))
        };
        let is_typing_marker_class = |class: &ObjRef| -> bool {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return false;
            };
            matches!(
                class_data.attrs.get("__module__"),
                Some(Value::Str(module_name)) if matches!(module_name.as_str(), "typing" | "_typing")
            ) && matches!(class_data.name.as_str(), "Any" | "Union")
        };
        match source {
            Value::Iterator(obj) => {
                let range_parts = {
                    let iter_kind = obj.kind();
                    match &*iter_kind {
                        Object::Iterator(state) => match &state.kind {
                            IteratorKind::RangeObject { start, stop, step } => {
                                Some((start.clone(), stop.clone(), step.clone()))
                            }
                            _ => None,
                        },
                        _ => return Err(RuntimeError::type_error("yield from expects iterable")),
                    }
                };
                if let Some((start, stop, step)) = range_parts {
                    Ok(self.heap.alloc_iterator(IteratorObject {
                        kind: IteratorKind::Range {
                            current: start,
                            stop,
                            step,
                        },
                        index: 0,
                    }))
                } else {
                    Ok(Value::Iterator(obj))
                }
            }
            Value::Generator(_) => Ok(source),
            Value::DictKeys(keys_view) => match &*keys_view.kind() {
                Object::DictKeysView(view) => {
                    self.to_iterator_value(Value::Dict(view.dict.clone()))
                }
                _ => Err(RuntimeError::type_error("yield from expects iterable")),
            },
            Value::Instance(instance) => {
                if let Some(values) = self.namedtuple_instance_values(&instance) {
                    return self.to_iterator_value(self.heap.alloc_tuple(values));
                }
                if let Some(backing_list) = self.instance_backing_list(&instance) {
                    return self.to_iterator_value(Value::List(backing_list));
                }
                if let Some(backing_tuple) = self.instance_backing_tuple(&instance) {
                    return self.to_iterator_value(Value::Tuple(backing_tuple));
                }
                if let Some(backing_str) = self.instance_backing_str(&instance) {
                    return self.to_iterator_value(Value::Str(backing_str));
                }
                if let Some(backing_dict) = self.instance_backing_dict(&instance) {
                    return self.to_iterator_value(Value::Dict(backing_dict));
                }
                if let Some(backing_set) = self.instance_backing_set(&instance) {
                    return self.to_iterator_value(Value::Set(backing_set));
                }
                if let Some(backing_frozenset) = self.instance_backing_frozenset(&instance) {
                    return self.to_iterator_value(Value::FrozenSet(backing_frozenset));
                }
                if let Some(iterator) = self.typing_alias_unpack_iterator(&instance) {
                    return Ok(iterator);
                }
                let other = Value::Instance(instance.clone());
                if self.typing_param_kind_name(&other) == Some("TypeVarTuple") {
                    // CPython TypeVarTuple iteration compatibility yields a single
                    // `typing.Unpack[Ts]` item so starred-unpack syntax (`*Ts`)
                    // expands through normal iterable-unpack paths.
                    let unpack = self.typing_helper_callable("Unpack")?;
                    let unpacked = self.getitem_value(unpack, other.clone())?;
                    return self.to_iterator_value(self.heap.alloc_tuple(vec![unpacked]));
                }
                if is_typing_marker_instance(&instance) {
                    return Err(RuntimeError::type_error("object is not iterable"));
                }
                if let Some(proxy_iter_result) = self.cpython_proxy_get_iter(&other) {
                    match proxy_iter_result {
                        Ok(iterable) => {
                            let same_proxy_identity = match (
                                Vm::cpython_proxy_raw_ptr_from_value(&other),
                                Vm::cpython_proxy_raw_ptr_from_value(&iterable),
                            ) {
                                (Some(left), Some(right)) => left == right,
                                _ => false,
                            };
                            if same_proxy_identity
                                || Vm::cpython_proxy_has_iternext(&iterable).unwrap_or(false)
                            {
                                return Ok(iterable);
                            }
                            return self.to_iterator_value(iterable);
                        }
                        Err(err) => {
                            if !err.message.contains("not iterable") {
                                return Err(err);
                            }
                        }
                    }
                }
                let maybe_next = self.lookup_bound_special_method(&other, "__next__")?;
                let Some(iter_method) = self.lookup_bound_special_method(&other, "__iter__")?
                else {
                    if maybe_next.is_some() {
                        // `next(obj)` accepts iterator-like objects that only provide
                        // `__next__` even when `__iter__` is absent.
                        return Ok(other);
                    }
                    if let Some(iterator) = self.sequence_iterator_via_getitem(other.clone())? {
                        return Ok(iterator);
                    }
                    return Err(RuntimeError::type_error("yield from expects iterable"));
                };
                match self.call_internal(iter_method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(iterable) => match iterable {
                        Value::Iterator(_) | Value::Generator(_) => Ok(iterable),
                        Value::List(_)
                        | Value::Tuple(_)
                        | Value::Str(_)
                        | Value::Dict(_)
                        | Value::Set(_)
                        | Value::FrozenSet(_)
                        | Value::Bytes(_)
                        | Value::ByteArray(_)
                        | Value::MemoryView(_)
                        | Value::Module(_) => self.to_iterator_value(iterable),
                        Value::Instance(_) => {
                            if self
                                .lookup_bound_special_method(&iterable, "__next__")?
                                .is_some()
                            {
                                Ok(iterable)
                            } else {
                                Err(RuntimeError::type_error("__iter__() returned non-iterator"))
                            }
                        }
                        _ => Err(RuntimeError::type_error("__iter__() returned non-iterator")),
                    },
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(self.runtime_error_from_active_exception("__iter__() failed"))
                    }
                }
            }
            Value::List(obj) => {
                let is_list = matches!(&*obj.kind(), Object::List(_));
                if is_list {
                    Ok(self.heap.alloc_iterator(IteratorObject {
                        kind: IteratorKind::List(obj),
                        index: 0,
                    }))
                } else {
                    Err(RuntimeError::type_error("yield from expects iterable"))
                }
            }
            Value::Tuple(obj) => {
                let is_tuple = matches!(&*obj.kind(), Object::Tuple(_));
                if is_tuple {
                    Ok(self.heap.alloc_iterator(IteratorObject {
                        kind: IteratorKind::Tuple(obj),
                        index: 0,
                    }))
                } else {
                    Err(RuntimeError::type_error("yield from expects iterable"))
                }
            }
            Value::Str(value) => Ok(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::Str(value),
                index: 0,
            })),
            Value::Dict(obj) => {
                let is_dict = matches!(&*obj.kind(), Object::Dict(_));
                if is_dict {
                    Ok(self.heap.alloc_iterator(IteratorObject {
                        kind: IteratorKind::Dict(obj),
                        index: 0,
                    }))
                } else {
                    Err(RuntimeError::type_error("yield from expects iterable"))
                }
            }
            Value::Set(obj) | Value::FrozenSet(obj) => {
                Ok(self.heap.alloc_iterator(IteratorObject {
                    kind: IteratorKind::Set(obj),
                    index: 0,
                }))
            }
            Value::Bytes(obj) => Ok(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::Bytes(obj),
                index: 0,
            })),
            Value::ByteArray(obj) => Ok(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::ByteArray(obj),
                index: 0,
            })),
            Value::MemoryView(obj) => Ok(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::MemoryView(obj),
                index: 0,
            })),
            Value::Module(module) => {
                let array_values = {
                    let module_kind = module.kind();
                    match &*module_kind {
                        Object::Module(module_data) if module_data.name == "__array__" => {
                            module_data.globals.get("values").cloned()
                        }
                        _ => None,
                    }
                };
                if let Some(values) = array_values {
                    return self.to_iterator_value(values);
                }
                let other = Value::Module(module);
                let Some(iter_method) = self.lookup_bound_special_method(&other, "__iter__")?
                else {
                    if let Some(iterator) = self.sequence_iterator_via_getitem(other.clone())? {
                        return Ok(iterator);
                    }
                    return Err(RuntimeError::type_error("yield from expects iterable"));
                };

                match self.call_internal(iter_method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(iterable) => match iterable {
                        Value::Iterator(_) | Value::Generator(_) => Ok(iterable),
                        Value::List(_)
                        | Value::Tuple(_)
                        | Value::Str(_)
                        | Value::Dict(_)
                        | Value::Set(_)
                        | Value::FrozenSet(_)
                        | Value::Bytes(_)
                        | Value::ByteArray(_)
                        | Value::MemoryView(_)
                        | Value::Module(_) => self.to_iterator_value(iterable),
                        Value::Instance(_) => {
                            if self
                                .lookup_bound_special_method(&iterable, "__next__")?
                                .is_some()
                            {
                                Ok(iterable)
                            } else {
                                Err(RuntimeError::type_error("__iter__() returned non-iterator"))
                            }
                        }
                        _ => Err(RuntimeError::type_error("__iter__() returned non-iterator")),
                    },
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(RuntimeError::new("__iter__() failed"))
                    }
                }
            }
            other => {
                if let Value::Class(class) = &other
                    && is_typing_marker_class(class)
                {
                    return Err(RuntimeError::type_error("object is not iterable"));
                }
                if let Value::Class(class) = &other
                    && let Some(iterator) = self.class_fallback_iterator(class)
                {
                    return Ok(iterator);
                }

                let Some(iter_method) = self.lookup_bound_special_method(&other, "__iter__")?
                else {
                    if let Some(iterator) = self.sequence_iterator_via_getitem(other.clone())? {
                        return Ok(iterator);
                    }
                    return Err(RuntimeError::type_error("yield from expects iterable"));
                };

                match self.call_internal(iter_method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(iterable) => match iterable {
                        Value::Iterator(_) | Value::Generator(_) => Ok(iterable),
                        Value::List(_)
                        | Value::Tuple(_)
                        | Value::Str(_)
                        | Value::Dict(_)
                        | Value::Set(_)
                        | Value::FrozenSet(_)
                        | Value::Bytes(_)
                        | Value::ByteArray(_)
                        | Value::MemoryView(_) => self.to_iterator_value(iterable),
                        Value::Instance(_) => {
                            if self
                                .lookup_bound_special_method(&iterable, "__next__")?
                                .is_some()
                            {
                                Ok(iterable)
                            } else {
                                Err(RuntimeError::type_error("__iter__() returned non-iterator"))
                            }
                        }
                        _ => Err(RuntimeError::type_error("__iter__() returned non-iterator")),
                    },
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(RuntimeError::new("__iter__() failed"))
                    }
                }
            }
        }
    }

    pub(super) fn class_namedtuple_fields(&self, class: &ObjRef) -> Option<Vec<String>> {
        let value = class_attr_lookup(class, "__pyrs_namedtuple_fields__")?;
        match value {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values
                    .iter()
                    .map(|value| match value {
                        Value::Str(name) => Some(name.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => None,
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => values
                    .iter()
                    .map(|value| match value {
                        Value::Str(name) => Some(name.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => None,
            },
            _ => None,
        }
    }

    pub(super) fn class_namedtuple_defaults(&self, class: &ObjRef) -> Option<Vec<Value>> {
        let value = class_attr_lookup(class, "__pyrs_namedtuple_defaults__")?;
        match value {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => Some(values.clone()),
                _ => None,
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => Some(values.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub(super) fn namedtuple_instance_values(&self, instance: &ObjRef) -> Option<Vec<Value>> {
        let instance_ref = instance.kind();
        let Object::Instance(instance_data) = &*instance_ref else {
            return None;
        };
        let fields = self.class_namedtuple_fields(&instance_data.class)?;
        fields
            .iter()
            .map(|field| instance_data.attrs.get(field).cloned())
            .collect()
    }

    fn abstract_method_names_from_value(&self, value: &Value) -> Vec<String> {
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

    pub(super) fn class_disallow_instantiation_message(&self, class: &ObjRef) -> Option<String> {
        let class_kind = class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return None;
        };
        if let Some(abstract_methods) = class_data.attrs.get("__abstractmethods__") {
            let names = self.abstract_method_names_from_value(abstract_methods);
            if let Some(first) = names.first() {
                let methods = if names.len() == 1 {
                    format!("'{first}'")
                } else {
                    names
                        .iter()
                        .map(|name| format!("'{name}'"))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                return Some(format!(
                    "Can't instantiate abstract class {} without an implementation for abstract method{} {}",
                    class_data.name,
                    if names.len() == 1 { "" } else { "s" },
                    methods
                ));
            }
            if is_truthy(abstract_methods) {
                return Some(format!(
                    "Can't instantiate abstract class {}",
                    class_data.name
                ));
            }
        }
        let runtime_disallow = matches!(
            class_data.attrs.get("__pyrs_disallow_instantiation__"),
            Some(Value::Bool(true))
        );
        let proxy_disallow = self
            .cpython_proxy_type_flags(class)
            .map(|flags| (flags & PY_TPFLAGS_DISALLOW_INSTANTIATION) != 0)
            .unwrap_or(false);
        if !runtime_disallow && !proxy_disallow {
            return None;
        }
        let module_name = match class_data.attrs.get("__module__") {
            Some(Value::Str(name)) => name.clone(),
            _ => "builtins".to_string(),
        };
        let qualified_name = if module_name == "builtins" {
            class_data.name.clone()
        } else {
            format!("{}.{}", module_name, class_data.name)
        };
        Some(format!("cannot create '{}' instances", qualified_name))
    }

    pub(super) fn class_fallback_iterator(&self, class: &ObjRef) -> Option<Value> {
        let member_values = {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return None;
            };

            if let Some(Value::Dict(members)) = class_data.attrs.get("__members__") {
                let members_kind = members.kind();
                let Object::Dict(entries) = &*members_kind else {
                    return None;
                };
                entries
                    .iter()
                    .map(|(_, value)| value.clone())
                    .collect::<Vec<_>>()
            } else {
                let mut values = Vec::new();
                for (name, value) in class_data.attrs.iter() {
                    if name.starts_with('_') {
                        continue;
                    }
                    let is_enum_style = name.chars().any(|ch| ch.is_ascii_uppercase())
                        && name
                            .chars()
                            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_');
                    if !is_enum_style {
                        return None;
                    }
                    let rank = match value {
                        Value::Instance(instance) => match &*instance.kind() {
                            Object::Instance(instance_data) => {
                                match instance_data.attrs.get("_value_") {
                                    Some(Value::Int(value)) => *value,
                                    Some(Value::Bool(value)) => {
                                        if *value {
                                            1
                                        } else {
                                            0
                                        }
                                    }
                                    _ => i64::MAX,
                                }
                            }
                            _ => i64::MAX,
                        },
                        _ => i64::MAX,
                    };
                    values.push((rank, name.clone(), value.clone()));
                }
                if values.is_empty() {
                    return None;
                }
                values
                    .sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
                values.into_iter().map(|(_, _, value)| value).collect()
            }
        };

        let list = match self.heap.alloc_list(member_values) {
            Value::List(list) => list,
            _ => unreachable!(),
        };
        Some(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::List(list),
            index: 0,
        }))
    }

    pub(super) fn next_from_iterator_value(
        &mut self,
        iterator: &Value,
    ) -> Result<GeneratorResumeOutcome, RuntimeError> {
        match iterator {
            Value::Generator(obj) => self.generator_for_iter_next(obj),
            Value::Iterator(iterator_ref) => {
                let next_value = self.iterator_next_value(iterator_ref)?;
                if let Some(value) = next_value {
                    Ok(GeneratorResumeOutcome::Yield(value))
                } else {
                    Ok(GeneratorResumeOutcome::Complete(Value::None))
                }
            }
            Value::Instance(instance) => {
                let receiver = Value::Instance(instance.clone());
                let is_cpython_proxy_iterator = Vm::cpython_proxy_raw_ptr_from_value(&receiver)
                    .is_some()
                    && Vm::cpython_proxy_has_iternext(&receiver).unwrap_or(false);
                let method = self
                    .lookup_bound_special_method(&receiver, "__next__")?
                    .ok_or_else(|| RuntimeError::type_error("yield from expects iterable"))?;
                match self.call_internal(method, Vec::new(), HashMap::new()) {
                    Ok(InternalCallOutcome::Value(value)) => {
                        if exception_is_named(&value, "StopIteration")
                            || (is_cpython_proxy_iterator
                                && exception_is_named(&value, "IndexError"))
                        {
                            unsafe { PyErr_Clear() };
                            Ok(GeneratorResumeOutcome::Complete(Value::None))
                        } else {
                            Ok(GeneratorResumeOutcome::Yield(value))
                        }
                    }
                    Ok(InternalCallOutcome::CallerExceptionHandled) => {
                        let active_exception = self
                            .frames
                            .last()
                            .and_then(|frame| frame.active_exception.clone());
                        if let Some(exception) = active_exception
                            && (exception_is_named(&exception, "StopIteration")
                                || (is_cpython_proxy_iterator
                                    && exception_is_named(&exception, "IndexError")))
                        {
                            unsafe { PyErr_Clear() };
                            self.clear_active_exception();
                            return Ok(GeneratorResumeOutcome::Complete(Value::None));
                        }
                        Ok(GeneratorResumeOutcome::PropagatedException)
                    }
                    Err(err) => {
                        if runtime_error_matches_exception(&err, "StopIteration")
                            || (is_cpython_proxy_iterator
                                && runtime_error_matches_exception(&err, "IndexError"))
                        {
                            unsafe { PyErr_Clear() };
                            Ok(GeneratorResumeOutcome::Complete(Value::None))
                        } else {
                            Err(err)
                        }
                    }
                }
            }
            _ => Err(RuntimeError::type_error("yield from expects iterable")),
        }
    }

    pub(super) fn delegate_yield_from(
        &mut self,
        iterator: &Value,
        sent: Value,
        thrown: Option<Value>,
        resume_kind: GeneratorResumeKind,
    ) -> Result<GeneratorResumeOutcome, RuntimeError> {
        if let Some(exc) = thrown {
            return match iterator {
                Value::Generator(obj) => {
                    let delegated_kind = if resume_kind == GeneratorResumeKind::Close
                        && exception_is_named(&exc, "GeneratorExit")
                    {
                        GeneratorResumeKind::Close
                    } else {
                        GeneratorResumeKind::Throw
                    };
                    let outcome =
                        self.resume_generator(obj, None, Some(exc.clone()), delegated_kind)?;
                    if resume_kind == GeneratorResumeKind::Close
                        && exception_is_named(&exc, "GeneratorExit")
                    {
                        match outcome {
                            GeneratorResumeOutcome::Yield(_) => Err(RuntimeError::runtime_error(
                                "generator ignored GeneratorExit",
                            )),
                            GeneratorResumeOutcome::Complete(_) => {
                                self.raise_exception(exc)?;
                                Ok(GeneratorResumeOutcome::PropagatedException)
                            }
                            GeneratorResumeOutcome::PropagatedException => {
                                if self.active_exception_is("GeneratorExit") {
                                    self.clear_active_exception();
                                    self.raise_exception(exc)?;
                                }
                                Ok(GeneratorResumeOutcome::PropagatedException)
                            }
                        }
                    } else {
                        Ok(outcome)
                    }
                }
                Value::Iterator(_) => {
                    self.raise_exception(exc)?;
                    Ok(GeneratorResumeOutcome::PropagatedException)
                }
                _ => Err(RuntimeError::type_error("yield from expects iterable")),
            };
        }

        if sent != Value::None {
            return match iterator {
                Value::Generator(obj) => {
                    self.resume_generator(obj, Some(sent), None, GeneratorResumeKind::Next)
                }
                Value::Instance(instance) => {
                    let receiver = Value::Instance(instance.clone());
                    let Some(send_method) = self.optional_getattr_value(receiver, "send")? else {
                        return Err(RuntimeError::type_error("yield from expects iterable"));
                    };
                    match self.call_internal(send_method, vec![sent], HashMap::new()) {
                        Ok(InternalCallOutcome::Value(value)) => {
                            if exception_is_named(&value, "StopIteration") {
                                Ok(GeneratorResumeOutcome::Complete(
                                    self.stop_iteration_value_from_value(&value)
                                        .unwrap_or(Value::None),
                                ))
                            } else {
                                Ok(GeneratorResumeOutcome::Yield(value))
                            }
                        }
                        Ok(InternalCallOutcome::CallerExceptionHandled) => {
                            if self.active_exception_is("StopIteration") {
                                let value = self
                                    .stop_iteration_value_from_active_exception()
                                    .unwrap_or(Value::None);
                                self.clear_active_exception();
                                Ok(GeneratorResumeOutcome::Complete(value))
                            } else {
                                Ok(GeneratorResumeOutcome::PropagatedException)
                            }
                        }
                        Err(err) => {
                            if runtime_error_matches_exception(&err, "StopIteration") {
                                let value = self
                                    .stop_iteration_value_from_active_exception()
                                    .unwrap_or(Value::None);
                                self.clear_active_exception();
                                Ok(GeneratorResumeOutcome::Complete(value))
                            } else {
                                Err(err)
                            }
                        }
                    }
                }
                Value::Iterator(_) => Err(RuntimeError::attribute_error(format!(
                    "'{}' object has no attribute 'send'",
                    self.iterator_type_name(iterator)
                ))),
                _ => Err(RuntimeError::type_error("yield from expects iterable")),
            };
        }

        self.next_from_iterator_value(iterator)
    }

    pub(super) fn iterator_type_name(&self, iterator: &Value) -> &'static str {
        match iterator {
            Value::Iterator(obj) => match &*obj.kind() {
                Object::Iterator(state) => match state.kind {
                    IteratorKind::List(_) => "list_iterator",
                    IteratorKind::Tuple(_) => "tuple_iterator",
                    IteratorKind::Str(_) => "str_iterator",
                    IteratorKind::Dict(_) => "dict_keyiterator",
                    IteratorKind::Set(_) => "set_iterator",
                    IteratorKind::Bytes(_) => "bytes_iterator",
                    IteratorKind::ByteArray(_) => "bytearray_iterator",
                    IteratorKind::MemoryView(_) => "memoryview_iterator",
                    IteratorKind::Cycle { .. } => "cycle",
                    IteratorKind::Count { .. } => "count",
                    IteratorKind::Map { .. } => "map",
                    IteratorKind::Zip { .. } => "zip",
                    IteratorKind::RangeObject { .. } => "range",
                    IteratorKind::Range { .. } => "range_iterator",
                    IteratorKind::SequenceGetItem { .. } => "iterator",
                    IteratorKind::CpythonSequence { .. } => "iterator",
                    IteratorKind::CallIter { .. } => "callable_iterator",
                },
                _ => "iterator",
            },
            Value::Generator(_) => "generator",
            _ => "object",
        }
    }

    pub(super) fn iterator_next_value(
        &mut self,
        iterator_ref: &ObjRef,
    ) -> Result<Option<Value>, RuntimeError> {
        enum PendingIteratorStep {
            MapEvaluate {
                func: Value,
                iterators: Vec<Value>,
            },
            ZipEvaluate {
                iterators: Vec<Value>,
                strict: bool,
            },
            SequenceGetItem {
                target: Value,
                getitem: Value,
                index: i64,
            },
            CpythonSequence {
                target: Value,
                index: i64,
            },
            CallIter {
                callable: Value,
                sentinel: Value,
            },
            CycleAdvance {
                source: Value,
            },
        }

        let pending_step;
        {
            let mut iter = iterator_ref.kind_mut();
            let Object::Iterator(state) = &mut *iter else {
                return Ok(None);
            };
            match &mut state.kind {
                IteratorKind::List(list) => {
                    return Ok(match &*list.kind() {
                        Object::List(values) => {
                            if state.index >= values.len() {
                                None
                            } else {
                                let value = values[state.index].clone();
                                state.index += 1;
                                Some(value)
                            }
                        }
                        _ => None,
                    });
                }
                IteratorKind::Tuple(list) => {
                    return Ok(match &*list.kind() {
                        Object::Tuple(values) => {
                            if state.index >= values.len() {
                                None
                            } else {
                                let value = values[state.index].clone();
                                state.index += 1;
                                Some(value)
                            }
                        }
                        _ => None,
                    });
                }
                IteratorKind::Str(text) => {
                    let chars: Vec<char> = text.chars().collect();
                    return Ok(if state.index >= chars.len() {
                        None
                    } else {
                        let ch = chars[state.index];
                        state.index += 1;
                        Some(Value::Str(ch.to_string()))
                    });
                }
                IteratorKind::Dict(dict) => {
                    return Ok(match &*dict.kind() {
                        Object::Dict(entries) => {
                            if state.index >= entries.len() {
                                None
                            } else {
                                let value = entries[state.index].0.clone();
                                state.index += 1;
                                Some(value)
                            }
                        }
                        _ => None,
                    });
                }
                IteratorKind::Set(set) => {
                    return Ok(match &*set.kind() {
                        Object::Set(values) | Object::FrozenSet(values) => {
                            if state.index >= values.len() {
                                None
                            } else {
                                let value = values[state.index].clone();
                                state.index += 1;
                                Some(value)
                            }
                        }
                        _ => None,
                    });
                }
                IteratorKind::Bytes(bytes) => {
                    return Ok(match &*bytes.kind() {
                        Object::Bytes(values) => {
                            if state.index >= values.len() {
                                None
                            } else {
                                let value = Value::Int(values[state.index] as i64);
                                state.index += 1;
                                Some(value)
                            }
                        }
                        _ => None,
                    });
                }
                IteratorKind::ByteArray(bytes) => {
                    return Ok(match &*bytes.kind() {
                        Object::ByteArray(values) => {
                            if state.index >= values.len() {
                                None
                            } else {
                                let value = Value::Int(values[state.index] as i64);
                                state.index += 1;
                                Some(value)
                            }
                        }
                        _ => None,
                    });
                }
                IteratorKind::MemoryView(view_ref) => {
                    return match &*view_ref.kind() {
                        Object::MemoryView(view) => with_bytes_like_source(&view.source, |values| {
                            let Some((shape, _strides)) = memoryview_shape_and_strides_from_parts(
                                view.start,
                                view.length,
                                view.shape.as_ref(),
                                view.strides.as_ref(),
                                view.itemsize,
                                values.len(),
                            ) else {
                                return Ok(None);
                            };
                            if shape.len() > 1 {
                                return Err(RuntimeError::new(
                                    "NotImplementedError: multi-dimensional sub-views are not implemented",
                                ));
                            }
                            let itemsize = view.itemsize.max(1);
                            let format = memoryview_format_for_view(itemsize, view.format.as_deref())?;
                            let Some((origin, logical_len, stride, _)) =
                                super::memoryview_layout_1d(view, values.len())
                            else {
                                return Ok(None);
                            };
                            if state.index >= logical_len {
                                return Ok(None);
                            }
                            let offset = super::memoryview_element_offset(
                                origin,
                                logical_len,
                                stride,
                                state.index as isize,
                            )
                            .ok_or_else(|| RuntimeError::index_error("index out of range"))?;
                            let end = offset
                                .checked_add(itemsize)
                                .ok_or_else(|| RuntimeError::index_error("index out of range"))?;
                            let chunk = values
                                .get(offset..end)
                                .ok_or_else(|| RuntimeError::index_error("index out of range"))?;
                            let value =
                                super::memoryview_decode_element(chunk, format, itemsize, &self.heap)?;
                            state.index += 1;
                            Ok(Some(value))
                        })
                        .unwrap_or_else(|| Ok(None)),
                        _ => Ok(None),
                    };
                }
                IteratorKind::Cycle {
                    source,
                    values,
                    source_exhausted,
                } => {
                    if !*source_exhausted {
                        pending_step = PendingIteratorStep::CycleAdvance {
                            source: source.clone(),
                        };
                    } else {
                        if values.is_empty() {
                            return Ok(None);
                        }
                        let index = state.index % values.len();
                        let value = values[index].clone();
                        state.index = state.index.wrapping_add(1);
                        return Ok(Some(value));
                    }
                }
                IteratorKind::Count { current, step } => {
                    let value = *current;
                    *current = current.saturating_add(*step);
                    return Ok(Some(Value::Int(value)));
                }
                IteratorKind::Map {
                    values,
                    func,
                    iterators,
                    exhausted,
                    ..
                } => {
                    if state.index < values.len() {
                        let value = values[state.index].clone();
                        state.index += 1;
                        return Ok(Some(value));
                    }
                    if *exhausted {
                        return Ok(None);
                    }
                    pending_step = PendingIteratorStep::MapEvaluate {
                        func: func.clone(),
                        iterators: iterators.clone(),
                    };
                }
                IteratorKind::Zip {
                    iterators,
                    strict,
                    exhausted,
                } => {
                    if *exhausted {
                        return Ok(None);
                    }
                    pending_step = PendingIteratorStep::ZipEvaluate {
                        iterators: iterators.clone(),
                        strict: *strict,
                    };
                }
                IteratorKind::RangeObject { start, stop, step } => {
                    if step.is_zero() {
                        return Err(RuntimeError::value_error("range() arg 3 must not be zero"));
                    }
                    let offset = step.mul(&BigInt::from_u64(state.index as u64));
                    let current = start.add(&offset);
                    let done = if step.is_negative() {
                        current.cmp_total(stop) != Ordering::Greater
                    } else {
                        current.cmp_total(stop) != Ordering::Less
                    };
                    if done {
                        return Ok(None);
                    }
                    state.index = state.index.saturating_add(1);
                    return Ok(Some(value_from_bigint(current)));
                }
                IteratorKind::Range {
                    current,
                    stop,
                    step,
                } => {
                    let done = if step.is_negative() {
                        current.cmp_total(stop) != Ordering::Greater
                    } else {
                        current.cmp_total(stop) != Ordering::Less
                    };
                    if done {
                        return Ok(None);
                    }
                    let value = current.clone();
                    *current = current.add(step);
                    return Ok(Some(value_from_bigint(value)));
                }
                IteratorKind::SequenceGetItem { target, getitem } => {
                    if state.index > i64::MAX as usize {
                        return Err(RuntimeError::new("iterator index overflow"));
                    }
                    pending_step = PendingIteratorStep::SequenceGetItem {
                        target: target.clone(),
                        getitem: getitem.clone(),
                        index: state.index as i64,
                    };
                }
                IteratorKind::CpythonSequence { target } => {
                    if state.index > i64::MAX as usize {
                        return Err(RuntimeError::new("iterator index overflow"));
                    }
                    pending_step = PendingIteratorStep::CpythonSequence {
                        target: target.clone(),
                        index: state.index as i64,
                    };
                }
                IteratorKind::CallIter { callable, sentinel } => {
                    pending_step = PendingIteratorStep::CallIter {
                        callable: callable.clone(),
                        sentinel: sentinel.clone(),
                    };
                }
            }
        }

        match pending_step {
            PendingIteratorStep::MapEvaluate { func, iterators } => {
                let mut call_args = Vec::with_capacity(iterators.len());
                for iterator in &iterators {
                    match self.next_from_iterator_value(iterator)? {
                        GeneratorResumeOutcome::Yield(value) => call_args.push(value),
                        GeneratorResumeOutcome::Complete(_) => {
                            let mut iter = iterator_ref.kind_mut();
                            if let Object::Iterator(state) = &mut *iter
                                && let IteratorKind::Map { exhausted, .. } = &mut state.kind
                            {
                                *exhausted = true;
                            }
                            return Ok(None);
                        }
                        GeneratorResumeOutcome::PropagatedException => {
                            return Err(self.iteration_error_from_state("map() iteration failed")?);
                        }
                    }
                }

                let value = match self.call_internal(func, call_args, HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(RuntimeError::new("map() callable failed"));
                    }
                };

                let mut iter = iterator_ref.kind_mut();
                if let Object::Iterator(state) = &mut *iter
                    && let IteratorKind::Map { values, .. } = &mut state.kind
                {
                    values.push(value.clone());
                    state.index += 1;
                    return Ok(Some(value));
                }
                Ok(None)
            }
            PendingIteratorStep::ZipEvaluate { iterators, strict } => {
                if iterators.is_empty() {
                    let mut iter = iterator_ref.kind_mut();
                    if let Object::Iterator(state) = &mut *iter
                        && let IteratorKind::Zip { exhausted, .. } = &mut state.kind
                    {
                        *exhausted = true;
                    }
                    return Ok(None);
                }

                let mut tuple_items = Vec::with_capacity(iterators.len());
                for (idx, iterator) in iterators.iter().enumerate() {
                    match self.next_from_iterator_value(iterator)? {
                        GeneratorResumeOutcome::Yield(value) => tuple_items.push(value),
                        GeneratorResumeOutcome::Complete(_) => {
                            let mut iter = iterator_ref.kind_mut();
                            if let Object::Iterator(state) = &mut *iter
                                && let IteratorKind::Zip { exhausted, .. } = &mut state.kind
                            {
                                *exhausted = true;
                            }
                            if !strict {
                                return Ok(None);
                            }

                            if idx > 0 {
                                let plural = if idx == 1 { " " } else { "s 1-" };
                                return Err(RuntimeError::value_error(format!(
                                    "zip() argument {} is shorter than argument{}{}",
                                    idx + 1,
                                    plural,
                                    idx
                                )));
                            }

                            for follow_idx in 1..iterators.len() {
                                match self.next_from_iterator_value(&iterators[follow_idx])? {
                                    GeneratorResumeOutcome::Yield(_) => {
                                        let plural = if follow_idx == 1 { " " } else { "s 1-" };
                                        return Err(RuntimeError::value_error(format!(
                                            "zip() argument {} is longer than argument{}{}",
                                            follow_idx + 1,
                                            plural,
                                            follow_idx
                                        )));
                                    }
                                    GeneratorResumeOutcome::Complete(_) => {}
                                    GeneratorResumeOutcome::PropagatedException => {
                                        return Err(self.iteration_error_from_state(
                                            "zip() iteration failed",
                                        )?);
                                    }
                                }
                            }
                            return Ok(None);
                        }
                        GeneratorResumeOutcome::PropagatedException => {
                            return Err(self.iteration_error_from_state("zip() iteration failed")?);
                        }
                    }
                }

                let value = self.heap.alloc_tuple(tuple_items);
                let mut iter = iterator_ref.kind_mut();
                if let Object::Iterator(state) = &mut *iter
                    && let IteratorKind::Zip { .. } = &mut state.kind
                {
                    state.index = state.index.saturating_add(1);
                    return Ok(Some(value));
                }
                Ok(None)
            }
            PendingIteratorStep::SequenceGetItem {
                target,
                getitem,
                index,
            } => {
                let index_value = Value::Int(index);
                let call_result = self.call_internal(getitem, vec![index_value], HashMap::new());
                match call_result {
                    Ok(InternalCallOutcome::Value(value)) => {
                        {
                            let mut iter = iterator_ref.kind_mut();
                            if let Object::Iterator(state) = &mut *iter
                                && let IteratorKind::SequenceGetItem { .. } = &mut state.kind
                            {
                                state.index += 1;
                            }
                        }
                        Ok(Some(value))
                    }
                    Ok(InternalCallOutcome::CallerExceptionHandled) => {
                        if self.active_exception_is("IndexError") {
                            self.clear_active_exception();
                            unsafe { PyErr_Clear() };
                            return Ok(None);
                        }
                        let _ = target;
                        let err = self.runtime_error_from_active_exception("__getitem__() failed");
                        if runtime_error_matches_exception(&err, "IndexError")
                            || err.message.contains("index out of range")
                            || err.message.contains("out of bounds for axis")
                        {
                            self.clear_active_exception();
                            unsafe { PyErr_Clear() };
                            return Ok(None);
                        }
                        Err(err)
                    }
                    Err(err) => {
                        if runtime_error_matches_exception(&err, "IndexError")
                            || err.message.contains("index out of range")
                            || err.message.contains("out of bounds for axis")
                        {
                            self.clear_active_exception();
                            unsafe { PyErr_Clear() };
                            return Ok(None);
                        }
                        Err(err)
                    }
                }
            }
            PendingIteratorStep::CpythonSequence { target, index } => {
                let index_value = Value::Int(index);
                if let Some(proxy_result) =
                    self.cpython_proxy_get_item(&target, index_value.clone())
                {
                    match proxy_result {
                        Ok(value) => {
                            {
                                let mut iter = iterator_ref.kind_mut();
                                if let Object::Iterator(state) = &mut *iter
                                    && let IteratorKind::CpythonSequence { .. } = &mut state.kind
                                {
                                    state.index += 1;
                                }
                            }
                            return Ok(Some(value));
                        }
                        Err(err) => {
                            let treat_as_end = runtime_error_matches_exception(&err, "IndexError")
                                || err.message.contains("index out of range")
                                || err.message.contains("out of bounds for axis");
                            if treat_as_end {
                                self.clear_active_exception();
                                unsafe { PyErr_Clear() };
                                return Ok(None);
                            }
                            return Err(err);
                        }
                    }
                }
                match self.getitem_value(target.clone(), index_value) {
                    Ok(value) => {
                        {
                            let mut iter = iterator_ref.kind_mut();
                            if let Object::Iterator(state) = &mut *iter
                                && let IteratorKind::CpythonSequence { .. } = &mut state.kind
                            {
                                state.index += 1;
                            }
                        }
                        Ok(Some(value))
                    }
                    Err(err) => {
                        let treat_as_end = runtime_error_matches_exception(&err, "IndexError")
                            || err.message.contains("index out of range")
                            || err.message.contains("out of bounds for axis");
                        if treat_as_end {
                            self.clear_active_exception();
                            unsafe { PyErr_Clear() };
                            return Ok(None);
                        }
                        Err(err)
                    }
                }
            }
            PendingIteratorStep::CallIter { callable, sentinel } => {
                let produced = match self.call_internal(callable, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(RuntimeError::new("callable iterator target failed"));
                    }
                };
                let should_stop = match self.compare_eq_runtime(produced.clone(), sentinel)? {
                    Value::Bool(flag) => flag,
                    _ => false,
                };
                if should_stop {
                    Ok(None)
                } else {
                    Ok(Some(produced))
                }
            }
            PendingIteratorStep::CycleAdvance { source } => {
                match self.next_from_iterator_value(&source)? {
                    GeneratorResumeOutcome::Yield(value) => {
                        let mut iter = iterator_ref.kind_mut();
                        if let Object::Iterator(state) = &mut *iter
                            && let IteratorKind::Cycle { values, .. } = &mut state.kind
                        {
                            values.push(value.clone());
                            return Ok(Some(value));
                        }
                        Ok(None)
                    }
                    GeneratorResumeOutcome::Complete(_) => {
                        let mut iter = iterator_ref.kind_mut();
                        if let Object::Iterator(state) = &mut *iter
                            && let IteratorKind::Cycle {
                                values,
                                source_exhausted,
                                ..
                            } = &mut state.kind
                        {
                            *source_exhausted = true;
                            if values.is_empty() {
                                return Ok(None);
                            }
                            let value = values[0].clone();
                            state.index = 1;
                            return Ok(Some(value));
                        }
                        Ok(None)
                    }
                    GeneratorResumeOutcome::PropagatedException => {
                        Err(self.iteration_error_from_state("cycle() iteration failed")?)
                    }
                }
            }
        }
    }

    pub(super) fn resume_generator(
        &mut self,
        generator: &ObjRef,
        sent: Option<Value>,
        thrown: Option<Value>,
        kind: GeneratorResumeKind,
    ) -> Result<GeneratorResumeOutcome, RuntimeError> {
        let (started, running, closed) = match &*generator.kind() {
            Object::Generator(state) => (state.started, state.running, state.closed),
            _ => return Err(RuntimeError::type_error("object is not a generator")),
        };
        if running {
            return Err(RuntimeError::value_error("generator already executing"));
        }
        if closed {
            let value = self
                .generator_returns
                .get(&generator.id())
                .cloned()
                .unwrap_or(Value::None);
            return Ok(GeneratorResumeOutcome::Complete(value));
        }
        if thrown.is_none()
            && !started
            && let Some(value) = &sent
            && *value != Value::None
        {
            return Err(RuntimeError::type_error(
                "can't send non-None value to a just-started generator",
            ));
        }

        let mut frame = self
            .generator_states
            .remove(&generator.id())
            .ok_or_else(|| RuntimeError::new("generator has no suspended frame"))?;
        frame.generator_resume_value = sent;
        frame.generator_pending_throw = thrown;
        frame.generator_resume_kind = Some(kind);
        self.set_generator_running(generator, true)?;
        self.set_generator_started(generator, true)?;

        let previous_active = self.active_generator_resume;
        let previous_boundary = self.active_generator_resume_boundary;
        let previous_outcome = self.generator_resume_outcome.take();
        let previous_pending = self.pending_generator_exception.take();

        self.active_generator_resume = Some(generator.id());
        self.active_generator_resume_boundary = Some(self.frames.len());
        self.generator_resume_outcome = None;
        self.pending_generator_exception = None;
        self.push_frame_checked(frame)?;
        let run_result = self.run();
        let outcome = self.generator_resume_outcome.take();
        let pending = self.pending_generator_exception.take();
        self.active_generator_resume = previous_active;
        self.active_generator_resume_boundary = previous_boundary;
        self.generator_resume_outcome = previous_outcome;
        self.pending_generator_exception = pending.or(previous_pending);

        match run_result {
            Ok(_) => {
                if let Some(outcome) = outcome {
                    Ok(outcome)
                } else {
                    let value = self
                        .generator_returns
                        .get(&generator.id())
                        .cloned()
                        .unwrap_or(Value::None);
                    Ok(GeneratorResumeOutcome::Complete(value))
                }
            }
            Err(err) => {
                let _ = self.set_generator_running(generator, false);
                Err(err)
            }
        }
    }

    pub(super) fn finish_generator_resume(&mut self, owner: ObjRef, value: Value) {
        self.generator_states.remove(&owner.id());
        self.generator_returns.insert(owner.id(), value.clone());
        let _ = self.set_generator_running(&owner, false);
        let _ = self.set_generator_started(&owner, true);
        let _ = self.set_generator_closed(&owner, true);
        if self.active_generator_resume == Some(owner.id()) {
            self.generator_resume_outcome = Some(GeneratorResumeOutcome::Complete(value));
        }
    }

    pub(super) fn set_generator_started(
        &self,
        generator: &ObjRef,
        started: bool,
    ) -> Result<(), RuntimeError> {
        match &mut *generator.kind_mut() {
            Object::Generator(state) => {
                state.started = started;
                Ok(())
            }
            _ => Err(RuntimeError::type_error("object is not a generator")),
        }
    }

    pub(super) fn set_generator_running(
        &self,
        generator: &ObjRef,
        running: bool,
    ) -> Result<(), RuntimeError> {
        match &mut *generator.kind_mut() {
            Object::Generator(state) => {
                state.running = running;
                Ok(())
            }
            _ => Err(RuntimeError::type_error("object is not a generator")),
        }
    }

    pub(super) fn set_generator_closed(
        &self,
        generator: &ObjRef,
        closed: bool,
    ) -> Result<(), RuntimeError> {
        match &mut *generator.kind_mut() {
            Object::Generator(state) => {
                state.closed = closed;
                if closed {
                    state.running = false;
                }
                Ok(())
            }
            _ => Err(RuntimeError::type_error("object is not a generator")),
        }
    }

    pub(super) fn active_exception_is(&self, name: &str) -> bool {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.active_exception.as_ref())
            .and_then(|value| match value {
                Value::Exception(exc) => Some(exc.name.as_str()),
                _ => None,
            })
            .map(|exc_name| exc_name == name)
            .unwrap_or(false)
    }

    pub(super) fn clear_active_exception(&mut self) {
        if let Some(frame) = self
            .frames
            .iter_mut()
            .rev()
            .find(|frame| frame.active_exception.is_some())
        {
            frame.active_exception = None;
        }
    }

    pub(super) fn propagate_pending_generator_exception(&mut self) -> Result<(), RuntimeError> {
        if let Some(exc) = self.pending_generator_exception.take() {
            self.raise_exception(exc)?;
        }
        Ok(())
    }

    fn builtin_setattr_with_class_version(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let class_target = args.first().and_then(|value| match value {
            Value::Class(class) => Some(class.clone()),
            _ => None,
        });
        let result = self.builtin_setattr(args, kwargs);
        if result.is_ok()
            && let Some(class) = class_target
        {
            self.touch_class_attr_version(&class);
        }
        result
    }

    fn builtin_delattr_with_class_version(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let class_target = args.first().and_then(|value| match value {
            Value::Class(class) => Some(class.clone()),
            _ => None,
        });
        let result = self.builtin_delattr(args, kwargs);
        if result.is_ok()
            && let Some(class) = class_target
        {
            self.touch_class_attr_version(&class);
        }
        result
    }

    pub(super) fn call_builtin(
        &mut self,
        builtin: BuiltinFunction,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let depth = CALL_BUILTIN_DEPTH.with(|depth| {
            let next = depth.get().saturating_add(1);
            depth.set(next);
            next
        });
        struct CallBuiltinDepthGuard;
        impl Drop for CallBuiltinDepthGuard {
            fn drop(&mut self) {
                CALL_BUILTIN_DEPTH.with(|depth| {
                    depth.set(depth.get().saturating_sub(1));
                });
            }
        }
        let _guard = CallBuiltinDepthGuard;
        if self
            .host
            .env_var_os("PYRS_DEBUG_CALL_BUILTIN_DEPTH")
            .is_some()
            && depth > 256
        {
            panic!("call_builtin recursion depth exceeded at depth={depth} builtin={builtin:?}");
        }
        match builtin {
            BuiltinFunction::Print => self.builtin_print(args, kwargs),
            BuiltinFunction::Input => self.builtin_input(args, kwargs),
            BuiltinFunction::Repr => self.builtin_repr(args, kwargs),
            BuiltinFunction::Ascii => self.builtin_ascii(args, kwargs),
            BuiltinFunction::Len => self.builtin_len(args, kwargs),
            BuiltinFunction::Locals => self.builtin_locals(args, kwargs),
            BuiltinFunction::Globals => self.builtin_globals(args, kwargs),
            BuiltinFunction::Vars => self.builtin_vars(args, kwargs),
            BuiltinFunction::GcCollect => self.builtin_gc_collect(args, kwargs),
            BuiltinFunction::GcEnable => self.builtin_gc_enable(args, kwargs),
            BuiltinFunction::GcDisable => self.builtin_gc_disable(args, kwargs),
            BuiltinFunction::GcIsEnabled => self.builtin_gc_is_enabled(args, kwargs),
            BuiltinFunction::GcGetThreshold => self.builtin_gc_get_threshold(args, kwargs),
            BuiltinFunction::GcSetThreshold => self.builtin_gc_set_threshold(args, kwargs),
            BuiltinFunction::GcGetCount => self.builtin_gc_get_count(args, kwargs),
            BuiltinFunction::TraceMallocStart => self.builtin_tracemalloc_start(args, kwargs),
            BuiltinFunction::TraceMallocStop => self.builtin_tracemalloc_stop(args, kwargs),
            BuiltinFunction::TraceMallocIsTracing => {
                self.builtin_tracemalloc_is_tracing(args, kwargs)
            }
            BuiltinFunction::TraceMallocGetTracebackLimit => {
                self.builtin_tracemalloc_get_traceback_limit(args, kwargs)
            }
            BuiltinFunction::TraceMallocGetTracedMemory => {
                self.builtin_tracemalloc_get_traced_memory(args, kwargs)
            }
            BuiltinFunction::TraceMallocGetTraceMallocMemory => {
                self.builtin_tracemalloc_get_tracemalloc_memory(args, kwargs)
            }
            BuiltinFunction::TraceMallocResetPeak => {
                self.builtin_tracemalloc_reset_peak(args, kwargs)
            }
            BuiltinFunction::TraceMallocClearTraces => {
                self.builtin_tracemalloc_clear_traces(args, kwargs)
            }
            BuiltinFunction::TraceMallocGetTraces => {
                self.builtin_tracemalloc_get_traces(args, kwargs)
            }
            BuiltinFunction::TraceMallocGetObjectTraceback => {
                self.builtin_tracemalloc_get_object_traceback(args, kwargs)
            }
            BuiltinFunction::Dir => self.builtin_dir(args, kwargs),
            BuiltinFunction::Hash => self.builtin_hash(args, kwargs),
            BuiltinFunction::Breakpoint => self.builtin_breakpoint(args, kwargs),
            BuiltinFunction::SysGetFrame => self.builtin_sys_getframe(args, kwargs),
            BuiltinFunction::SysGetFrameModuleName => {
                self.builtin_sys_getframemodulename(args, kwargs)
            }
            BuiltinFunction::SysCurrentFrames => self.builtin_sys_current_frames(args, kwargs),
            BuiltinFunction::SysException => self.builtin_sys_exception(args, kwargs),
            BuiltinFunction::SysExcInfo => self.builtin_sys_exc_info(args, kwargs),
            BuiltinFunction::SysCallTracing => self.builtin_sys_call_tracing(args, kwargs),
            BuiltinFunction::SysExit => self.builtin_sys_exit(args, kwargs),
            BuiltinFunction::SysIntern => self.builtin_sys_intern(args, kwargs),
            BuiltinFunction::SysIsFinalizing => self.builtin_sys_is_finalizing(args, kwargs),
            BuiltinFunction::SysIsGilEnabled => self.builtin_sys_is_gil_enabled(args, kwargs),
            BuiltinFunction::SysIsRemoteDebugEnabled => {
                self.builtin_sys_is_remote_debug_enabled(args, kwargs)
            }
            BuiltinFunction::SysGetDefaultEncoding => {
                self.builtin_sys_getdefaultencoding(args, kwargs)
            }
            BuiltinFunction::SysGetFilesystemEncoding => {
                self.builtin_sys_getfilesystemencoding(args, kwargs)
            }
            BuiltinFunction::SysGetFilesystemEncodeErrors => {
                self.builtin_sys_getfilesystemencodeerrors(args, kwargs)
            }
            BuiltinFunction::SysGetRefCount => self.builtin_sys_getrefcount(args, kwargs),
            BuiltinFunction::SysGetSizeOf => self.builtin_sys_getsizeof(args, kwargs),
            BuiltinFunction::SysGetRecursionLimit => {
                self.builtin_sys_getrecursionlimit(args, kwargs)
            }
            BuiltinFunction::SysSetRecursionLimit => {
                self.builtin_sys_setrecursionlimit(args, kwargs)
            }
            BuiltinFunction::SysGetSwitchInterval => {
                self.builtin_sys_getswitchinterval(args, kwargs)
            }
            BuiltinFunction::SysSetSwitchInterval => {
                self.builtin_sys_setswitchinterval(args, kwargs)
            }
            BuiltinFunction::SysExcepthook => self.builtin_sys_excepthook(args, kwargs),
            BuiltinFunction::SysDisplayHook => self.builtin_sys_displayhook(args, kwargs),
            BuiltinFunction::SysAudit => self.builtin_sys_audit(args, kwargs),
            BuiltinFunction::SysAddAuditHook => self.builtin_sys_addaudithook(args, kwargs),
            BuiltinFunction::SysClearTypeDescriptors => {
                self.builtin_sys_clear_type_descriptors(args, kwargs)
            }
            BuiltinFunction::SysUnraisableHook => self.builtin_sys_unraisablehook(args, kwargs),
            BuiltinFunction::SysBreakpointHook => self.builtin_sys_breakpointhook(args, kwargs),
            BuiltinFunction::SysMonitoringGetTool => {
                self.builtin_sys_monitoring_get_tool(args, kwargs)
            }
            BuiltinFunction::SysMonitoringUseToolId => {
                self.builtin_sys_monitoring_use_tool_id(args, kwargs)
            }
            BuiltinFunction::SysMonitoringClearToolId => {
                self.builtin_sys_monitoring_clear_tool_id(args, kwargs)
            }
            BuiltinFunction::SysMonitoringFreeToolId => {
                self.builtin_sys_monitoring_free_tool_id(args, kwargs)
            }
            BuiltinFunction::SysMonitoringRegisterCallback => {
                self.builtin_sys_monitoring_register_callback(args, kwargs)
            }
            BuiltinFunction::SysMonitoringGetEvents => {
                self.builtin_sys_monitoring_get_events(args, kwargs)
            }
            BuiltinFunction::SysMonitoringSetEvents => {
                self.builtin_sys_monitoring_set_events(args, kwargs)
            }
            BuiltinFunction::SysMonitoringGetLocalEvents => {
                self.builtin_sys_monitoring_get_local_events(args, kwargs)
            }
            BuiltinFunction::SysMonitoringSetLocalEvents => {
                self.builtin_sys_monitoring_set_local_events(args, kwargs)
            }
            BuiltinFunction::SysMonitoringRestartEvents => {
                self.builtin_sys_monitoring_restart_events(args, kwargs)
            }
            BuiltinFunction::SysStdoutWrite => self.builtin_sys_stream_write(args, kwargs, false),
            BuiltinFunction::SysStdoutBufferWrite => {
                self.builtin_sys_stream_buffer_write(args, kwargs, false)
            }
            BuiltinFunction::SysStdoutFlush => self.builtin_sys_stream_flush(args, kwargs),
            BuiltinFunction::SysStderrWrite => self.builtin_sys_stream_write(args, kwargs, true),
            BuiltinFunction::SysStderrBufferWrite => {
                self.builtin_sys_stream_buffer_write(args, kwargs, true)
            }
            BuiltinFunction::SysStderrFlush => self.builtin_sys_stream_flush(args, kwargs),
            BuiltinFunction::SysStdinWrite => self.builtin_sys_stdin_write(args, kwargs),
            BuiltinFunction::SysStdinFlush => self.builtin_sys_stream_flush(args, kwargs),
            BuiltinFunction::SysStreamIsATty => self.builtin_sys_stream_isatty(args, kwargs),
            BuiltinFunction::SysconfigGetDataName => {
                self.builtin_sysconfig_get_data_name(args, kwargs)
            }
            BuiltinFunction::Int => self.builtin_int(args, kwargs),
            BuiltinFunction::Bool => self.builtin_bool(args, kwargs),
            BuiltinFunction::Float => self.builtin_float(args, kwargs),
            BuiltinFunction::Complex => self.builtin_complex(args, kwargs),
            BuiltinFunction::Str => self.builtin_str(args, kwargs),
            BuiltinFunction::Bytes => self.builtin_bytes_constructor(args, kwargs),
            BuiltinFunction::ByteArray => self.builtin_bytearray_constructor(args, kwargs),
            BuiltinFunction::MemoryView => self.builtin_memoryview(args, kwargs),
            BuiltinFunction::FloatGetFormat => self.builtin_float_getformat(args, kwargs),
            BuiltinFunction::FloatFromHex => self.builtin_float_fromhex(args, kwargs),
            BuiltinFunction::FloatHex => self.builtin_float_hex(args, kwargs),
            BuiltinFunction::BytesFromHex => self.builtin_bytes_fromhex(args, kwargs),
            BuiltinFunction::ByteArrayFromHex => self.builtin_bytearray_fromhex(args, kwargs),
            BuiltinFunction::StrMakeTrans => self.builtin_str_maketrans(args, kwargs),
            BuiltinFunction::BytesMakeTrans => self.builtin_bytes_maketrans(args, kwargs),
            BuiltinFunction::IntFromBytes => self.builtin_int_from_bytes(args, kwargs),
            BuiltinFunction::Compile => self.builtin_compile(args, kwargs),
            BuiltinFunction::PlatformLibcVer => self.builtin_platform_libc_ver(args, kwargs),
            BuiltinFunction::PlatformWin32IsIot => self.builtin_platform_win32_is_iot(args, kwargs),
            BuiltinFunction::GetAttr => self.builtin_getattr(args, kwargs),
            BuiltinFunction::SetAttr => self.builtin_setattr_with_class_version(args, kwargs),
            BuiltinFunction::DelAttr => self.builtin_delattr_with_class_version(args, kwargs),
            BuiltinFunction::HasAttr => self.builtin_hasattr(args, kwargs),
            BuiltinFunction::Callable => self.builtin_callable(args, kwargs),
            BuiltinFunction::Type => self.builtin_type(args, kwargs),
            BuiltinFunction::TypeCall => self.builtin_type_call(args, kwargs),
            BuiltinFunction::TypeInit => self.builtin_type_init(args, kwargs),
            BuiltinFunction::TypeMro => self.builtin_type_mro(args, kwargs),
            BuiltinFunction::TypePrepare => self.builtin_type_prepare(args, kwargs),
            BuiltinFunction::IsInstance => self.builtin_isinstance(args, kwargs),
            BuiltinFunction::IsSubclass => self.builtin_issubclass(args, kwargs),
            BuiltinFunction::TypeInstanceCheck => self.builtin_type_instancecheck(args, kwargs),
            BuiltinFunction::TypeSubclassCheck => self.builtin_type_subclasscheck(args, kwargs),
            BuiltinFunction::Property => self.builtin_property(args, kwargs),
            BuiltinFunction::ObjectNew => self.builtin_object_new(args, kwargs),
            BuiltinFunction::ObjectEq => self.builtin_object_eq(args, kwargs),
            BuiltinFunction::ObjectNe => self.builtin_object_ne(args, kwargs),
            BuiltinFunction::TracebackTypeNew => self.builtin_traceback_type_new(args, kwargs),
            BuiltinFunction::ObjectInit => self.builtin_object_init(args, kwargs),
            BuiltinFunction::ObjectInitSubclass => self.builtin_object_init_subclass(args, kwargs),
            BuiltinFunction::ExceptionTypeInit => self.builtin_exception_type_init(args, kwargs),
            BuiltinFunction::ExceptionTypeStr => self.builtin_exception_type_str(args, kwargs),
            BuiltinFunction::ExceptionTypeRepr => self.builtin_exception_type_repr(args, kwargs),
            BuiltinFunction::ObjectGetAttribute => self.builtin_object_getattribute(args, kwargs),
            BuiltinFunction::ObjectFormat => self.builtin_object_format(args, kwargs),
            BuiltinFunction::ObjectGetState => self.builtin_object_getstate(args, kwargs),
            BuiltinFunction::ObjectSetState => self.builtin_object_setstate(args, kwargs),
            BuiltinFunction::ObjectReduce => self.builtin_object_reduce(args, kwargs),
            BuiltinFunction::ObjectReduceEx => self.builtin_object_reduce_ex(args, kwargs),
            BuiltinFunction::ObjectSetAttr => self.builtin_object_setattr(args, kwargs),
            BuiltinFunction::ObjectDelAttr => self.builtin_object_delattr(args, kwargs),
            BuiltinFunction::List => self.builtin_list(args, kwargs),
            BuiltinFunction::Tuple => self.builtin_tuple(args, kwargs),
            BuiltinFunction::ArrayArray => self.builtin_array_array(args, kwargs),
            BuiltinFunction::Dict => self.builtin_dict(args, kwargs),
            BuiltinFunction::DictFromKeys => self.builtin_dict_fromkeys(args, kwargs),
            BuiltinFunction::Set => self.builtin_set(args, kwargs),
            BuiltinFunction::SetReduce => self.builtin_set_reduce(args, kwargs),
            BuiltinFunction::FrozenSet => self.builtin_frozenset(args, kwargs),
            BuiltinFunction::Min => self.builtin_min(args, kwargs),
            BuiltinFunction::Max => self.builtin_max(args, kwargs),
            BuiltinFunction::Sum => self.builtin_sum(args, kwargs),
            BuiltinFunction::Round => self.builtin_round(args, kwargs),
            BuiltinFunction::Format => self.builtin_format(args, kwargs),
            BuiltinFunction::Sorted => self.builtin_sorted(args, kwargs),
            BuiltinFunction::All => self.builtin_all(args, kwargs),
            BuiltinFunction::Any => self.builtin_any(args, kwargs),
            BuiltinFunction::Enumerate => self.builtin_enumerate(args, kwargs),
            BuiltinFunction::WeakRefRef => self.builtin_weakref_ref(args, kwargs),
            BuiltinFunction::WeakRefProxy => self.builtin_weakref_proxy(args, kwargs),
            BuiltinFunction::WeakRefFinalize => self.builtin_weakref_finalize(args, kwargs),
            BuiltinFunction::WeakRefFinalizeDetach => {
                self.builtin_weakref_finalize_detach(args, kwargs)
            }
            BuiltinFunction::Filter => self.builtin_filter(args, kwargs),
            BuiltinFunction::Reversed => self.builtin_reversed(args, kwargs),
            BuiltinFunction::Zip => self.builtin_zip(args, kwargs),
            BuiltinFunction::Iter => self.builtin_iter(args, kwargs),
            BuiltinFunction::Next => self.builtin_next(args, kwargs),
            BuiltinFunction::Map => self.builtin_map(args, kwargs),
            BuiltinFunction::AIter => self.builtin_aiter(args, kwargs),
            BuiltinFunction::ANext => self.builtin_anext(args, kwargs),
            BuiltinFunction::Super => self.builtin_super(args, kwargs),
            BuiltinFunction::Import => self.builtin_import(args, kwargs),
            BuiltinFunction::Exec => self.builtin_exec(args, kwargs),
            BuiltinFunction::Eval => self.builtin_eval(args, kwargs),
            BuiltinFunction::ImportModule => self.builtin_import_module(args, kwargs),
            BuiltinFunction::PkgutilGetData => self.builtin_pkgutil_get_data(args, kwargs),
            BuiltinFunction::PkgutilIterModules => self.builtin_pkgutil_iter_modules(args, kwargs),
            BuiltinFunction::PkgutilWalkPackages => {
                self.builtin_pkgutil_walk_packages(args, kwargs)
            }
            BuiltinFunction::PkgutilResolveName => self.builtin_pkgutil_resolve_name(args, kwargs),
            BuiltinFunction::ImportlibPathHook => self.builtin_importlib_path_hook(args, kwargs),
            BuiltinFunction::ImportlibFileFinderFindSpec => {
                self.builtin_importlib_file_finder_find_spec(args, kwargs)
            }
            BuiltinFunction::FindSpec => self.builtin_find_spec(args, kwargs),
            BuiltinFunction::ImportlibInvalidateCaches => {
                self.builtin_importlib_invalidate_caches(args, kwargs)
            }
            BuiltinFunction::ImportlibSourceFromCache => {
                self.builtin_importlib_source_from_cache(args, kwargs)
            }
            BuiltinFunction::ImportlibCacheFromSource => {
                self.builtin_importlib_cache_from_source(args, kwargs)
            }
            BuiltinFunction::ImportlibSpecFromFileLocation => {
                self.builtin_importlib_spec_from_file_location(args, kwargs)
            }
            BuiltinFunction::ImportlibModuleFromSpec => {
                self.builtin_importlib_module_from_spec(args, kwargs)
            }
            BuiltinFunction::FrozenImportlibSpecFromLoader => {
                self.builtin_frozen_importlib_spec_from_loader(args, kwargs)
            }
            BuiltinFunction::FrozenImportlibVerboseMessage => {
                self.builtin_frozen_importlib_verbose_message(args, kwargs)
            }
            BuiltinFunction::FrozenImportlibExternalPathJoin => {
                self.builtin_frozen_importlib_external_path_join(args, kwargs)
            }
            BuiltinFunction::FrozenImportlibExternalPathSplit => {
                self.builtin_frozen_importlib_external_path_split(args, kwargs)
            }
            BuiltinFunction::FrozenImportlibExternalPathStat => {
                self.builtin_frozen_importlib_external_path_stat(args, kwargs)
            }
            BuiltinFunction::FrozenImportlibExternalPackUint32 => {
                self.builtin_frozen_importlib_external_pack_uint32(args, kwargs)
            }
            BuiltinFunction::FrozenImportlibExternalUnpackUint16 => {
                self.builtin_frozen_importlib_external_unpack_uint16(args, kwargs)
            }
            BuiltinFunction::FrozenImportlibExternalUnpackUint32 => {
                self.builtin_frozen_importlib_external_unpack_uint32(args, kwargs)
            }
            BuiltinFunction::FrozenImportlibExternalUnpackUint64 => {
                self.builtin_frozen_importlib_external_unpack_uint64(args, kwargs)
            }
            BuiltinFunction::OpcodeStackEffect => self.builtin_opcode_stack_effect(args, kwargs),
            BuiltinFunction::OpcodeHasArg => self.builtin_opcode_has_arg(args, kwargs),
            BuiltinFunction::OpcodeHasConst => self.builtin_opcode_has_const(args, kwargs),
            BuiltinFunction::OpcodeHasName => self.builtin_opcode_has_name(args, kwargs),
            BuiltinFunction::OpcodeHasJump => self.builtin_opcode_has_jump(args, kwargs),
            BuiltinFunction::OpcodeHasFree => self.builtin_opcode_has_free(args, kwargs),
            BuiltinFunction::OpcodeHasLocal => self.builtin_opcode_has_local(args, kwargs),
            BuiltinFunction::OpcodeHasExc => self.builtin_opcode_has_exc(args, kwargs),
            BuiltinFunction::OpcodeGetExecutor => self.builtin_opcode_get_executor(args, kwargs),
            BuiltinFunction::RandomSeed => self.builtin_random_seed(args, kwargs),
            BuiltinFunction::RandomRandom => self.builtin_random_random(args, kwargs),
            BuiltinFunction::RandomRandRange => self.builtin_random_randrange(args, kwargs),
            BuiltinFunction::RandomRandInt => self.builtin_random_randint(args, kwargs),
            BuiltinFunction::RandomGetRandBits => self.builtin_random_getrandbits(args, kwargs),
            BuiltinFunction::RandomChoice => self.builtin_random_choice(args, kwargs),
            BuiltinFunction::RandomChoices => self.builtin_random_choices(args, kwargs),
            BuiltinFunction::RandomShuffle => self.builtin_random_shuffle(args, kwargs),
            BuiltinFunction::DecimalGetContext => self.builtin_decimal_getcontext(args, kwargs),
            BuiltinFunction::DecimalSetContext => self.builtin_decimal_setcontext(args, kwargs),
            BuiltinFunction::DecimalLocalContext => self.builtin_decimal_localcontext(args, kwargs),
            BuiltinFunction::DecimalContextEnter => {
                self.builtin_decimal_context_enter(args, kwargs)
            }
            BuiltinFunction::DecimalContextExit => self.builtin_decimal_context_exit(args, kwargs),
            BuiltinFunction::MathSqrt => self.builtin_math_sqrt(args, kwargs),
            BuiltinFunction::MathCopySign => self.builtin_math_copysign(args, kwargs),
            BuiltinFunction::MathFloor => self.builtin_math_floor(args, kwargs),
            BuiltinFunction::MathCeil => self.builtin_math_ceil(args, kwargs),
            BuiltinFunction::MathTrunc => self.builtin_math_trunc(args, kwargs),
            BuiltinFunction::MathIsFinite => self.builtin_math_isfinite(args, kwargs),
            BuiltinFunction::MathIsInf => self.builtin_math_isinf(args, kwargs),
            BuiltinFunction::MathIsNaN => self.builtin_math_isnan(args, kwargs),
            BuiltinFunction::MathLdExp => self.builtin_math_ldexp(args, kwargs),
            BuiltinFunction::MathHypot => self.builtin_math_hypot(args, kwargs),
            BuiltinFunction::MathFAbs => self.builtin_math_fabs(args, kwargs),
            BuiltinFunction::MathExp => self.builtin_math_exp(args, kwargs),
            BuiltinFunction::MathErfc => self.builtin_math_erfc(args, kwargs),
            BuiltinFunction::MathLog => self.builtin_math_log(args, kwargs),
            BuiltinFunction::MathLog2 => self.builtin_math_log2(args, kwargs),
            BuiltinFunction::MathLGamma => self.builtin_math_lgamma(args, kwargs),
            BuiltinFunction::MathFSum => self.builtin_math_fsum(args, kwargs),
            BuiltinFunction::MathSumProd => self.builtin_math_sumprod(args, kwargs),
            BuiltinFunction::MathCos => self.builtin_math_cos(args, kwargs),
            BuiltinFunction::MathSin => self.builtin_math_sin(args, kwargs),
            BuiltinFunction::MathTan => self.builtin_math_tan(args, kwargs),
            BuiltinFunction::MathCosh => self.builtin_math_cosh(args, kwargs),
            BuiltinFunction::MathAsin => self.builtin_math_asin(args, kwargs),
            BuiltinFunction::MathAtan => self.builtin_math_atan(args, kwargs),
            BuiltinFunction::MathAcos => self.builtin_math_acos(args, kwargs),
            BuiltinFunction::MathIsClose => self.builtin_math_isclose(args, kwargs),
            BuiltinFunction::MathFactorial => self.builtin_math_factorial(args, kwargs),
            BuiltinFunction::MathGcd => self.builtin_math_gcd(args, kwargs),
            BuiltinFunction::TimeTime => self.builtin_time_time(args, kwargs),
            BuiltinFunction::TimeTimeNs => self.builtin_time_time_ns(args, kwargs),
            BuiltinFunction::TimeLocalTime => self.builtin_time_localtime(args, kwargs),
            BuiltinFunction::TimeGmTime => self.builtin_time_gmtime(args, kwargs),
            BuiltinFunction::TimeStrFTime => self.builtin_time_strftime(args, kwargs),
            BuiltinFunction::TimeMonotonic => self.builtin_time_monotonic(args, kwargs),
            BuiltinFunction::TimeSleep => self.builtin_time_sleep(args, kwargs),
            BuiltinFunction::OsGetPid => self.builtin_os_getpid(args, kwargs),
            BuiltinFunction::OsChDir => self.builtin_os_chdir(args, kwargs),
            BuiltinFunction::OsGetCwd => self.builtin_os_getcwd(args, kwargs),
            BuiltinFunction::OsCpuCount => self.builtin_os_cpu_count(args, kwargs),
            BuiltinFunction::OsUname => self.builtin_os_uname(args, kwargs),
            BuiltinFunction::OsUnameIter => self.builtin_os_uname_iter(args, kwargs),
            BuiltinFunction::OsGetEnv => self.builtin_os_getenv(args, kwargs),
            BuiltinFunction::OsPutEnv => self.builtin_os_putenv(args, kwargs),
            BuiltinFunction::OsUnsetEnv => self.builtin_os_unsetenv(args, kwargs),
            BuiltinFunction::OsGetTerminalSize => self.builtin_os_get_terminal_size(args, kwargs),
            BuiltinFunction::OsTerminalSize => self.builtin_os_terminal_size(args, kwargs),
            BuiltinFunction::OsOpen => self.builtin_os_open(args, kwargs),
            BuiltinFunction::OsPipe => self.builtin_os_pipe(args, kwargs),
            BuiltinFunction::OsRead => self.builtin_os_read(args, kwargs),
            BuiltinFunction::OsReadInto => self.builtin_os_readinto(args, kwargs),
            BuiltinFunction::OsWrite => self.builtin_os_write(args, kwargs),
            BuiltinFunction::OsDup => self.builtin_os_dup(args, kwargs),
            BuiltinFunction::OsLSeek => self.builtin_os_lseek(args, kwargs),
            BuiltinFunction::OsFTruncate => self.builtin_os_ftruncate(args, kwargs),
            BuiltinFunction::OsClose => self.builtin_os_close(args, kwargs),
            BuiltinFunction::OsKill => self.builtin_os_kill(args, kwargs),
            BuiltinFunction::OsIsATty => self.builtin_os_isatty(args, kwargs),
            BuiltinFunction::OsSetInheritable => self.builtin_os_set_inheritable(args, kwargs),
            BuiltinFunction::OsGetInheritable => self.builtin_os_get_inheritable(args, kwargs),
            BuiltinFunction::OsURandom => self.builtin_os_urandom(args, kwargs),
            BuiltinFunction::OsStat => self.builtin_os_stat(args, kwargs),
            BuiltinFunction::OsLStat => self.builtin_os_lstat(args, kwargs),
            BuiltinFunction::OsMkdir => self.builtin_os_mkdir(args, kwargs),
            BuiltinFunction::OsChmod => self.builtin_os_chmod(args, kwargs),
            BuiltinFunction::OsRmdir => self.builtin_os_rmdir(args, kwargs),
            BuiltinFunction::OsUTime => self.builtin_os_utime(args, kwargs),
            BuiltinFunction::OsScandir => self.builtin_os_scandir(args, kwargs),
            BuiltinFunction::OsScandirIter => self.builtin_os_scandir_iter(args, kwargs),
            BuiltinFunction::OsScandirNext => self.builtin_os_scandir_next(args, kwargs),
            BuiltinFunction::OsScandirEnter => self.builtin_os_scandir_enter(args, kwargs),
            BuiltinFunction::OsScandirExit => self.builtin_os_scandir_exit(args, kwargs),
            BuiltinFunction::OsScandirClose => self.builtin_os_scandir_close(args, kwargs),
            BuiltinFunction::OsDirEntryIsDir => self.builtin_os_direntry_is_dir(args, kwargs),
            BuiltinFunction::OsDirEntryIsFile => self.builtin_os_direntry_is_file(args, kwargs),
            BuiltinFunction::OsDirEntryIsSymlink => {
                self.builtin_os_direntry_is_symlink(args, kwargs)
            }
            BuiltinFunction::OsWalk => self.builtin_os_walk(args, kwargs),
            BuiltinFunction::OsWIfStopped => self.builtin_os_wifstopped(args, kwargs),
            BuiltinFunction::OsWStopSig => self.builtin_os_wstopsig(args, kwargs),
            BuiltinFunction::OsWIfSignaled => self.builtin_os_wifsignaled(args, kwargs),
            BuiltinFunction::OsWTermSig => self.builtin_os_wtermsig(args, kwargs),
            BuiltinFunction::OsWIfExited => self.builtin_os_wifexited(args, kwargs),
            BuiltinFunction::OsWExitStatus => self.builtin_os_wexitstatus(args, kwargs),
            BuiltinFunction::OsListDir => self.builtin_os_listdir(args, kwargs),
            BuiltinFunction::OsAccess => self.builtin_os_access(args, kwargs),
            BuiltinFunction::OsFspath => self.builtin_os_fspath(args, kwargs),
            BuiltinFunction::OsFsEncode => self.builtin_os_fsencode(args, kwargs),
            BuiltinFunction::OsFsDecode => self.builtin_os_fsdecode(args, kwargs),
            BuiltinFunction::OsRemove => self.builtin_os_remove(args, kwargs),
            BuiltinFunction::OsPopen => self.builtin_os_popen(args, kwargs),
            BuiltinFunction::OsWaitStatusToExitCode => {
                self.builtin_os_waitstatus_to_exitcode(args, kwargs)
            }
            BuiltinFunction::OsWaitPid => self.builtin_os_waitpid(args, kwargs),
            BuiltinFunction::OsPathExists => self.builtin_os_path_exists(args, kwargs),
            BuiltinFunction::OsPathJoin => self.builtin_os_path_join(args, kwargs),
            BuiltinFunction::OsPathNormPath => self.builtin_os_path_normpath(args, kwargs),
            BuiltinFunction::OsPathNormCase => self.builtin_os_path_normcase(args, kwargs),
            BuiltinFunction::OsPathSplitDrive => self.builtin_os_path_splitdrive(args, kwargs),
            BuiltinFunction::OsPathSplitRootEx => self.builtin_os_path_splitroot_ex(args, kwargs),
            BuiltinFunction::OsPathSplit => self.builtin_os_path_split(args, kwargs),
            BuiltinFunction::OsPathDirName => self.builtin_os_path_dirname(args, kwargs),
            BuiltinFunction::OsPathBaseName => self.builtin_os_path_basename(args, kwargs),
            BuiltinFunction::OsPathIsAbs => self.builtin_os_path_isabs(args, kwargs),
            BuiltinFunction::OsPathIsDir => self.builtin_os_path_isdir(args, kwargs),
            BuiltinFunction::OsPathIsFile => self.builtin_os_path_isfile(args, kwargs),
            BuiltinFunction::OsPathIsLink => self.builtin_os_path_islink(args, kwargs),
            BuiltinFunction::OsPathIsJunction => self.builtin_os_path_isjunction(args, kwargs),
            BuiltinFunction::OsPathSplitExt => self.builtin_os_path_splitext(args, kwargs),
            BuiltinFunction::OsPathAbsPath => self.builtin_os_path_abspath(args, kwargs),
            BuiltinFunction::OsPathExpandUser => self.builtin_os_path_expanduser(args, kwargs),
            BuiltinFunction::OsPathRealPath => self.builtin_os_path_realpath(args, kwargs),
            BuiltinFunction::OsPathRelPath => self.builtin_os_path_relpath(args, kwargs),
            BuiltinFunction::OsPathCommonPrefix => self.builtin_os_path_commonprefix(args, kwargs),
            BuiltinFunction::PathlibPathInit => self.builtin_pathlib_path_init(args, kwargs),
            BuiltinFunction::PathlibPathJoinPath => {
                self.builtin_pathlib_path_joinpath(args, kwargs)
            }
            BuiltinFunction::PathlibPathStr => self.builtin_pathlib_path_str(args, kwargs),
            BuiltinFunction::PwdGetPwAll => self.builtin_pwd_getpwall(args, kwargs),
            BuiltinFunction::PwdGetPwNam => self.builtin_pwd_getpwnam(args, kwargs),
            BuiltinFunction::PwdGetPwUid => self.builtin_pwd_getpwuid(args, kwargs),
            BuiltinFunction::PosixSubprocessForkExec => {
                self.builtin_posixsubprocess_fork_exec(args, kwargs)
            }
            BuiltinFunction::SubprocessPopenInit => {
                self.builtin_subprocess_popen_init(args, kwargs)
            }
            BuiltinFunction::SubprocessPopenCommunicate => {
                self.builtin_subprocess_popen_communicate(args, kwargs)
            }
            BuiltinFunction::SubprocessPopenWait => {
                self.builtin_subprocess_popen_wait(args, kwargs)
            }
            BuiltinFunction::SubprocessPopenKill => {
                self.builtin_subprocess_popen_kill(args, kwargs)
            }
            BuiltinFunction::SubprocessPopenPoll => {
                self.builtin_subprocess_popen_poll(args, kwargs)
            }
            BuiltinFunction::SubprocessPopenEnter => {
                self.builtin_subprocess_popen_enter(args, kwargs)
            }
            BuiltinFunction::SubprocessPopenExit => {
                self.builtin_subprocess_popen_exit(args, kwargs)
            }
            BuiltinFunction::SubprocessPipeRead => self.builtin_subprocess_pipe_read(args, kwargs),
            BuiltinFunction::SubprocessPipeReadline => {
                self.builtin_subprocess_pipe_readline(args, kwargs)
            }
            BuiltinFunction::SubprocessPipeWrite => {
                self.builtin_subprocess_pipe_write(args, kwargs)
            }
            BuiltinFunction::SubprocessPipeFlush => {
                self.builtin_subprocess_pipe_flush(args, kwargs)
            }
            BuiltinFunction::SubprocessPipeClose => {
                self.builtin_subprocess_pipe_close(args, kwargs)
            }
            BuiltinFunction::SubprocessCleanup => self.builtin_subprocess_cleanup(args, kwargs),
            BuiltinFunction::SubprocessRun => self.builtin_subprocess_run(args, kwargs),
            BuiltinFunction::SubprocessCheckCall => {
                self.builtin_subprocess_check_call(args, kwargs)
            }
            BuiltinFunction::SubprocessCompletedProcessInit => {
                self.builtin_subprocess_completed_process_init(args, kwargs)
            }
            BuiltinFunction::JsonDumps => self.builtin_json_dumps(args, kwargs),
            BuiltinFunction::JsonLoads => self.builtin_json_loads(args, kwargs),
            BuiltinFunction::JsonEncodeBaseString => {
                self.builtin_json_encode_basestring(args, kwargs)
            }
            BuiltinFunction::JsonEncodeBaseStringAscii => {
                self.builtin_json_encode_basestring_ascii(args, kwargs)
            }
            BuiltinFunction::JsonMakeEncoder => self.builtin_json_make_encoder(args, kwargs),
            BuiltinFunction::JsonMakeEncoderCall => {
                self.builtin_json_make_encoder_call(args, kwargs)
            }
            BuiltinFunction::SqliteConnect => self.builtin_sqlite_connect(args, kwargs),
            BuiltinFunction::SqliteCompleteStatement => {
                self.builtin_sqlite_complete_statement(args, kwargs)
            }
            BuiltinFunction::SqliteRegisterAdapter => {
                self.builtin_sqlite_register_adapter(args, kwargs)
            }
            BuiltinFunction::SqliteRegisterConverter => {
                self.builtin_sqlite_register_converter(args, kwargs)
            }
            BuiltinFunction::SqliteEnableCallbackTracebacks => {
                self.builtin_sqlite_enable_callback_tracebacks(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionInit => {
                self.builtin_sqlite_connection_init(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionDel => {
                self.builtin_sqlite_connection_del(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionGetAttribute => {
                self.builtin_sqlite_connection_getattribute(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionSetAttribute => {
                self.builtin_sqlite_connection_setattr(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionDelAttribute => {
                self.builtin_sqlite_connection_delattr(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionCursor => {
                self.builtin_sqlite_connection_cursor(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionClose => {
                self.builtin_sqlite_connection_close(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionEnter => {
                self.builtin_sqlite_connection_enter(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionExit => {
                self.builtin_sqlite_connection_exit(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionExecute => {
                self.builtin_sqlite_connection_execute(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionExecuteMany => {
                self.builtin_sqlite_connection_executemany(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionExecuteScript => {
                self.builtin_sqlite_connection_executescript(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionCommit => {
                self.builtin_sqlite_connection_commit(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionRollback => {
                self.builtin_sqlite_connection_rollback(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionInterrupt => {
                self.builtin_sqlite_connection_interrupt(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionIterDump => {
                self.builtin_sqlite_connection_iterdump(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionCreateFunction => {
                self.builtin_sqlite_connection_create_function(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionCreateAggregate => {
                self.builtin_sqlite_connection_create_aggregate(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionCreateWindowFunction => {
                self.builtin_sqlite_connection_create_window_function(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionSetTraceCallback => {
                self.builtin_sqlite_connection_set_trace_callback(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionCreateCollation => {
                self.builtin_sqlite_connection_create_collation(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionSetAuthorizer => {
                self.builtin_sqlite_connection_set_authorizer(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionSetProgressHandler => {
                self.builtin_sqlite_connection_set_progress_handler(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionGetLimit => {
                self.builtin_sqlite_connection_getlimit(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionSetLimit => {
                self.builtin_sqlite_connection_setlimit(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionGetConfig => {
                self.builtin_sqlite_connection_getconfig(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionSetConfig => {
                self.builtin_sqlite_connection_setconfig(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionBlobOpen => {
                self.builtin_sqlite_connection_blobopen(args, kwargs)
            }
            BuiltinFunction::SqliteConnectionBackup => {
                self.builtin_sqlite_connection_backup(args, kwargs)
            }
            BuiltinFunction::SqliteCursorInit => self.builtin_sqlite_cursor_init(args, kwargs),
            BuiltinFunction::SqliteCursorSetAttribute => {
                self.builtin_sqlite_cursor_setattr(args, kwargs)
            }
            BuiltinFunction::SqliteCursorSetInputSizes => {
                self.builtin_sqlite_cursor_setinputsizes(args, kwargs)
            }
            BuiltinFunction::SqliteCursorSetOutputSize => {
                self.builtin_sqlite_cursor_setoutputsize(args, kwargs)
            }
            BuiltinFunction::SqliteCursorExecute => {
                self.builtin_sqlite_cursor_execute(args, kwargs)
            }
            BuiltinFunction::SqliteCursorExecuteMany => {
                self.builtin_sqlite_cursor_executemany(args, kwargs)
            }
            BuiltinFunction::SqliteCursorExecuteScript => {
                self.builtin_sqlite_cursor_executescript(args, kwargs)
            }
            BuiltinFunction::SqliteCursorFetchOne => {
                self.builtin_sqlite_cursor_fetchone(args, kwargs)
            }
            BuiltinFunction::SqliteCursorFetchMany => {
                self.builtin_sqlite_cursor_fetchmany(args, kwargs)
            }
            BuiltinFunction::SqliteCursorFetchAll => {
                self.builtin_sqlite_cursor_fetchall(args, kwargs)
            }
            BuiltinFunction::SqliteCursorClose => self.builtin_sqlite_cursor_close(args, kwargs),
            BuiltinFunction::SqliteCursorIter => self.builtin_sqlite_cursor_iter(args, kwargs),
            BuiltinFunction::SqliteCursorNext => self.builtin_sqlite_cursor_next(args, kwargs),
            BuiltinFunction::SqliteBlobClose => self.builtin_sqlite_blob_close(args, kwargs),
            BuiltinFunction::SqliteBlobRead => self.builtin_sqlite_blob_read(args, kwargs),
            BuiltinFunction::SqliteBlobWrite => self.builtin_sqlite_blob_write(args, kwargs),
            BuiltinFunction::SqliteBlobSeek => self.builtin_sqlite_blob_seek(args, kwargs),
            BuiltinFunction::SqliteBlobTell => self.builtin_sqlite_blob_tell(args, kwargs),
            BuiltinFunction::SqliteBlobEnter => self.builtin_sqlite_blob_enter(args, kwargs),
            BuiltinFunction::SqliteBlobExit => self.builtin_sqlite_blob_exit(args, kwargs),
            BuiltinFunction::SqliteBlobLen => self.builtin_sqlite_blob_len(args, kwargs),
            BuiltinFunction::SqliteBlobGetItem => self.builtin_sqlite_blob_getitem(args, kwargs),
            BuiltinFunction::SqliteBlobSetItem => self.builtin_sqlite_blob_setitem(args, kwargs),
            BuiltinFunction::SqliteBlobDelItem => self.builtin_sqlite_blob_delitem(args, kwargs),
            BuiltinFunction::SqliteBlobIter => self.builtin_sqlite_blob_iter(args, kwargs),
            BuiltinFunction::SqliteRowInit => self.builtin_sqlite_row_init(args, kwargs),
            BuiltinFunction::SqliteRowKeys => self.builtin_sqlite_row_keys(args, kwargs),
            BuiltinFunction::SqliteRowLen => self.builtin_sqlite_row_len(args, kwargs),
            BuiltinFunction::SqliteRowGetItem => self.builtin_sqlite_row_getitem(args, kwargs),
            BuiltinFunction::SqliteRowIter => self.builtin_sqlite_row_iter(args, kwargs),
            BuiltinFunction::SqliteRowEq => self.builtin_sqlite_row_eq(args, kwargs),
            BuiltinFunction::SqliteRowHash => self.builtin_sqlite_row_hash(args, kwargs),
            BuiltinFunction::HashlibMd5 => self.builtin_hashlib_md5(args, kwargs),
            BuiltinFunction::HashlibSha1 => self.builtin_hashlib_sha1(args, kwargs),
            BuiltinFunction::HashlibSha224 => self.builtin_hashlib_sha224(args, kwargs),
            BuiltinFunction::HashlibSha256 => self.builtin_hashlib_sha256(args, kwargs),
            BuiltinFunction::HashlibSha384 => self.builtin_hashlib_sha384(args, kwargs),
            BuiltinFunction::HashlibSha512 => self.builtin_hashlib_sha512(args, kwargs),
            BuiltinFunction::HashlibBlake2b => self.builtin_hashlib_blake2b(args, kwargs),
            BuiltinFunction::HashlibBlake2s => self.builtin_hashlib_blake2s(args, kwargs),
            BuiltinFunction::HashlibSha3_224 => self.builtin_hashlib_sha3_224(args, kwargs),
            BuiltinFunction::HashlibSha3_256 => self.builtin_hashlib_sha3_256(args, kwargs),
            BuiltinFunction::HashlibSha3_384 => self.builtin_hashlib_sha3_384(args, kwargs),
            BuiltinFunction::HashlibSha3_512 => self.builtin_hashlib_sha3_512(args, kwargs),
            BuiltinFunction::HashlibShake128 => self.builtin_hashlib_shake128(args, kwargs),
            BuiltinFunction::HashlibShake256 => self.builtin_hashlib_shake256(args, kwargs),
            BuiltinFunction::HashlibNew => self.builtin_hashlib_new(args, kwargs),
            BuiltinFunction::HashlibPbkdf2Hmac => self.builtin_hashlib_pbkdf2_hmac(args, kwargs),
            BuiltinFunction::HashlibScrypt => self.builtin_hashlib_scrypt(args, kwargs),
            BuiltinFunction::HashlibHmacNew => self.builtin_hashlib_hmac_new(args, kwargs),
            BuiltinFunction::HashlibHmacDigest => self.builtin_hashlib_hmac_digest(args, kwargs),
            BuiltinFunction::HashlibHmacUpdate => self.builtin_hashlib_hmac_update(args, kwargs),
            BuiltinFunction::HashlibHmacObjDigest => {
                self.builtin_hashlib_hmac_obj_digest(args, kwargs)
            }
            BuiltinFunction::HashlibHmacObjHexDigest => {
                self.builtin_hashlib_hmac_obj_hexdigest(args, kwargs)
            }
            BuiltinFunction::HashlibHmacCopy => self.builtin_hashlib_hmac_copy(args, kwargs),
            BuiltinFunction::HashlibHmacRepr => self.builtin_hashlib_hmac_repr(args, kwargs),
            BuiltinFunction::HashlibHashUpdate => self.builtin_hashlib_hash_update(args, kwargs),
            BuiltinFunction::HashlibHashDigest => self.builtin_hashlib_hash_digest(args, kwargs),
            BuiltinFunction::HashlibHashHexDigest => {
                self.builtin_hashlib_hash_hexdigest(args, kwargs)
            }
            BuiltinFunction::HashlibHashCopy => self.builtin_hashlib_hash_copy(args, kwargs),
            BuiltinFunction::ZlibCompress => self.builtin_zlib_compress(args, kwargs),
            BuiltinFunction::ZlibDecompress => self.builtin_zlib_decompress(args, kwargs),
            BuiltinFunction::ZlibCompressObj => self.builtin_zlib_compressobj(args, kwargs),
            BuiltinFunction::ZlibDecompressObj => self.builtin_zlib_decompressobj(args, kwargs),
            BuiltinFunction::ZlibCrc32 => self.builtin_zlib_crc32(args, kwargs),
            BuiltinFunction::ZlibCompressObjectCompress => {
                self.builtin_zlib_compress_object_compress(args, kwargs)
            }
            BuiltinFunction::ZlibCompressObjectFlush => {
                self.builtin_zlib_compress_object_flush(args, kwargs)
            }
            BuiltinFunction::ZlibDecompressObjectDecompress => {
                self.builtin_zlib_decompress_object_decompress(args, kwargs)
            }
            BuiltinFunction::ZlibDecompressObjectFlush => {
                self.builtin_zlib_decompress_object_flush(args, kwargs)
            }
            BuiltinFunction::Bz2CompressorInit => self.builtin_bz2_compressor_init(args, kwargs),
            BuiltinFunction::Bz2CompressorCompress => {
                self.builtin_bz2_compressor_compress(args, kwargs)
            }
            BuiltinFunction::Bz2CompressorFlush => self.builtin_bz2_compressor_flush(args, kwargs),
            BuiltinFunction::Bz2DecompressorInit => {
                self.builtin_bz2_decompressor_init(args, kwargs)
            }
            BuiltinFunction::Bz2DecompressorDecompress => {
                self.builtin_bz2_decompressor_decompress(args, kwargs)
            }
            BuiltinFunction::LzmaCompressorInit => self.builtin_lzma_compressor_init(args, kwargs),
            BuiltinFunction::LzmaCompressorCompress => {
                self.builtin_lzma_compressor_compress(args, kwargs)
            }
            BuiltinFunction::LzmaCompressorFlush => {
                self.builtin_lzma_compressor_flush(args, kwargs)
            }
            BuiltinFunction::LzmaDecompressorInit => {
                self.builtin_lzma_decompressor_init(args, kwargs)
            }
            BuiltinFunction::LzmaDecompressorDecompress => {
                self.builtin_lzma_decompressor_decompress(args, kwargs)
            }
            BuiltinFunction::LzmaIsCheckSupported => {
                self.builtin_lzma_is_check_supported(args, kwargs)
            }
            BuiltinFunction::LzmaEncodeFilterProperties => {
                self.builtin_lzma_encode_filter_properties(args, kwargs)
            }
            BuiltinFunction::LzmaDecodeFilterProperties => {
                self.builtin_lzma_decode_filter_properties(args, kwargs)
            }
            BuiltinFunction::SslTxt2Obj => self.builtin_ssl_txt2obj(args, kwargs),
            BuiltinFunction::SslNid2Obj => self.builtin_ssl_nid2obj(args, kwargs),
            BuiltinFunction::SslRandStatus => self.builtin_ssl_rand_status(args, kwargs),
            BuiltinFunction::SslRandAdd => self.builtin_ssl_rand_add(args, kwargs),
            BuiltinFunction::SslRandBytes => self.builtin_ssl_rand_bytes(args, kwargs),
            BuiltinFunction::SslRandEgd => self.builtin_ssl_rand_egd(args, kwargs),
            BuiltinFunction::SslContextNew => self.builtin_ssl_context_new(args, kwargs),
            BuiltinFunction::SslContextInit => self.builtin_ssl_context_init(args, kwargs),
            BuiltinFunction::SslCreateDefaultContext => {
                self.builtin_ssl_create_default_context(args, kwargs)
            }
            BuiltinFunction::PyExpatParserCreate => {
                self.builtin_pyexpat_parser_create(args, kwargs)
            }
            BuiltinFunction::PyExpatParserParse => self.builtin_pyexpat_parser_parse(args, kwargs),
            BuiltinFunction::PyExpatParserGetReparseDeferralEnabled => {
                self.builtin_pyexpat_parser_get_reparse_deferral_enabled(args, kwargs)
            }
            BuiltinFunction::PyExpatParserSetReparseDeferralEnabled => {
                self.builtin_pyexpat_parser_set_reparse_deferral_enabled(args, kwargs)
            }
            BuiltinFunction::PickleDump => self.builtin_pickle_dump(args, kwargs),
            BuiltinFunction::PickleDumps => self.builtin_pickle_dumps(args, kwargs),
            BuiltinFunction::PickleLoad => self.builtin_pickle_load(args, kwargs),
            BuiltinFunction::PickleLoads => self.builtin_pickle_loads(args, kwargs),
            BuiltinFunction::PickleModuleGetAttr => {
                self.builtin_pickle_module_getattr(args, kwargs)
            }
            BuiltinFunction::PicklePicklerInit => self.builtin_pickle_pickler_init(args, kwargs),
            BuiltinFunction::PicklePicklerDump => self.builtin_pickle_pickler_dump(args, kwargs),
            BuiltinFunction::PickleCPicklerSaveReduceHook => {
                self.builtin_pickle_c_pickler_save_reduce_hook(args, kwargs)
            }
            BuiltinFunction::PicklePicklerClearMemo => {
                self.builtin_pickle_pickler_clear_memo(args, kwargs)
            }
            BuiltinFunction::PicklePicklerPersistentId => {
                self.builtin_pickle_pickler_persistent_id(args, kwargs)
            }
            BuiltinFunction::PickleUnpicklerInit => {
                self.builtin_pickle_unpickler_init(args, kwargs)
            }
            BuiltinFunction::PickleUnpicklerLoad => {
                self.builtin_pickle_unpickler_load(args, kwargs)
            }
            BuiltinFunction::PickleUnpicklerPersistentLoad => {
                self.builtin_pickle_unpickler_persistent_load(args, kwargs)
            }
            BuiltinFunction::PickleBufferInit => self.builtin_picklebuffer_init(args, kwargs),
            BuiltinFunction::PickleBufferRaw => self.builtin_picklebuffer_raw(args, kwargs),
            BuiltinFunction::PickleBufferRelease => self.builtin_picklebuffer_release(args, kwargs),
            BuiltinFunction::CopyregReconstructor => {
                self.builtin_copyreg_reconstructor(args, kwargs)
            }
            BuiltinFunction::CopyregNewObj => self.builtin_copyreg_newobj(args, kwargs),
            BuiltinFunction::CopyregNewObjEx => self.builtin_copyreg_newobj_ex(args, kwargs),
            BuiltinFunction::JsonScannerMakeScanner => {
                self.builtin_json_scanner_make_scanner(args, kwargs)
            }
            BuiltinFunction::JsonScannerPyMakeScanner => {
                self.builtin_json_scanner_make_scanner(args, kwargs)
            }
            BuiltinFunction::JsonScannerScanOnce => {
                self.builtin_json_scanner_scan_once(args, kwargs)
            }
            BuiltinFunction::JsonDecoderScanString => {
                self.builtin_json_decoder_scanstring(args, kwargs)
            }
            BuiltinFunction::PyLongIntToDecimalString => {
                self.builtin_pylong_int_to_decimal_string(args, kwargs)
            }
            BuiltinFunction::PyLongIntDivMod => self.builtin_pylong_int_divmod(args, kwargs),
            BuiltinFunction::PyLongIntFromString => {
                self.builtin_pylong_int_from_string(args, kwargs)
            }
            BuiltinFunction::PyLongComputePowers => {
                self.builtin_pylong_compute_powers(args, kwargs)
            }
            BuiltinFunction::PyLongDecStrToIntInner => {
                self.builtin_pylong_dec_str_to_int_inner(args, kwargs)
            }
            BuiltinFunction::CodecsEncode => self.builtin_codecs_encode(args, kwargs),
            BuiltinFunction::CodecsDecode => self.builtin_codecs_decode(args, kwargs),
            BuiltinFunction::CodecsEscapeDecode => self.builtin_codecs_escape_decode(args, kwargs),
            BuiltinFunction::CodecsMakeIdentityDict => {
                self.builtin_codecs_make_identity_dict(args, kwargs)
            }
            BuiltinFunction::CodecsLookup => self.builtin_codecs_lookup(args, kwargs),
            BuiltinFunction::CodecsRegister => self.builtin_codecs_register(args, kwargs),
            BuiltinFunction::CodecsUnregister => self.builtin_codecs_unregister(args, kwargs),
            BuiltinFunction::CodecsCodecInfoInit => {
                self.builtin_codecs_codecinfo_init(args, kwargs)
            }
            BuiltinFunction::CodecsGetIncrementalEncoder => {
                self.builtin_codecs_getincrementalencoder(args, kwargs)
            }
            BuiltinFunction::CodecsGetIncrementalDecoder => {
                self.builtin_codecs_getincrementaldecoder(args, kwargs)
            }
            BuiltinFunction::CodecsIncrementalEncoderInit => {
                self.builtin_codecs_incremental_encoder_init(args, kwargs)
            }
            BuiltinFunction::CodecsIncrementalEncoderEncode => {
                self.builtin_codecs_incremental_encoder_encode(args, kwargs)
            }
            BuiltinFunction::CodecsIncrementalEncoderReset => {
                self.builtin_codecs_incremental_encoder_reset(args, kwargs)
            }
            BuiltinFunction::CodecsIncrementalEncoderGetState => {
                self.builtin_codecs_incremental_encoder_getstate(args, kwargs)
            }
            BuiltinFunction::CodecsIncrementalEncoderSetState => {
                self.builtin_codecs_incremental_encoder_setstate(args, kwargs)
            }
            BuiltinFunction::CodecsIncrementalDecoderInit => {
                self.builtin_codecs_incremental_decoder_init(args, kwargs)
            }
            BuiltinFunction::CodecsIncrementalDecoderDecode => {
                self.builtin_codecs_incremental_decoder_decode(args, kwargs)
            }
            BuiltinFunction::CodecsIncrementalDecoderReset => {
                self.builtin_codecs_incremental_decoder_reset(args, kwargs)
            }
            BuiltinFunction::CodecsIncrementalDecoderGetState => {
                self.builtin_codecs_incremental_decoder_getstate(args, kwargs)
            }
            BuiltinFunction::CodecsIncrementalDecoderSetState => {
                self.builtin_codecs_incremental_decoder_setstate(args, kwargs)
            }
            BuiltinFunction::UnicodedataNormalize => {
                self.builtin_unicodedata_normalize(args, kwargs)
            }
            BuiltinFunction::UnicodedataEastAsianWidth => {
                self.builtin_unicodedata_east_asian_width(args, kwargs)
            }
            BuiltinFunction::UnicodedataCategory => self.builtin_unicodedata_category(args, kwargs),
            BuiltinFunction::UnicodedataBidirectional => {
                self.builtin_unicodedata_bidirectional(args, kwargs)
            }
            BuiltinFunction::UnicodedataLegacyCategory => {
                self.builtin_unicodedata_legacy_category(args, kwargs)
            }
            BuiltinFunction::UnicodedataLegacyBidirectional => {
                self.builtin_unicodedata_legacy_bidirectional(args, kwargs)
            }
            BuiltinFunction::SelectSelect => self.builtin_select_select(args, kwargs),
            BuiltinFunction::ReSearch => self.builtin_re_search(args, kwargs),
            BuiltinFunction::ReMatch => self.builtin_re_match(args, kwargs),
            BuiltinFunction::ReFullMatch => self.builtin_re_fullmatch(args, kwargs),
            BuiltinFunction::ReCompile => self.builtin_re_compile(args, kwargs),
            BuiltinFunction::ReEscape => self.builtin_re_escape(args, kwargs),
            BuiltinFunction::SreCompile => self.builtin_sre_compile(args, kwargs),
            BuiltinFunction::SreTemplate => self.builtin_sre_template(args, kwargs),
            BuiltinFunction::SreAsciiIsCased => self.builtin_sre_ascii_iscased(args, kwargs),
            BuiltinFunction::SreAsciiToLower => self.builtin_sre_ascii_tolower(args, kwargs),
            BuiltinFunction::SreUnicodeIsCased => self.builtin_sre_unicode_iscased(args, kwargs),
            BuiltinFunction::SreUnicodeToLower => self.builtin_sre_unicode_tolower(args, kwargs),
            BuiltinFunction::RePatternFindAll => self.builtin_re_pattern_findall(args, kwargs),
            BuiltinFunction::RePatternFindIter => self.builtin_re_pattern_finditer(args, kwargs),
            BuiltinFunction::RePatternSplit => self.builtin_re_pattern_split(args, kwargs),
            BuiltinFunction::OperatorAdd => self.builtin_operator_add(args, kwargs),
            BuiltinFunction::OperatorSub => self.builtin_operator_sub(args, kwargs),
            BuiltinFunction::OperatorMul => self.builtin_operator_mul(args, kwargs),
            BuiltinFunction::OperatorMod => self.builtin_operator_mod(args, kwargs),
            BuiltinFunction::OperatorPow => self.builtin_operator_pow(args, kwargs),
            BuiltinFunction::OperatorAnd => self.builtin_operator_and(args, kwargs),
            BuiltinFunction::OperatorOr => self.builtin_operator_or(args, kwargs),
            BuiltinFunction::OperatorXor => self.builtin_operator_xor(args, kwargs),
            BuiltinFunction::OperatorLShift => self.builtin_operator_lshift(args, kwargs),
            BuiltinFunction::OperatorRShift => self.builtin_operator_rshift(args, kwargs),
            BuiltinFunction::OperatorMatMul => self.builtin_operator_matmul(args, kwargs),
            BuiltinFunction::OperatorNeg => self.builtin_operator_neg(args, kwargs),
            BuiltinFunction::OperatorPos => self.builtin_operator_pos(args, kwargs),
            BuiltinFunction::OperatorInvert => self.builtin_operator_invert(args, kwargs),
            BuiltinFunction::OperatorTrueDiv => self.builtin_operator_truediv(args, kwargs),
            BuiltinFunction::OperatorFloorDiv => self.builtin_operator_floordiv(args, kwargs),
            BuiltinFunction::OperatorIndex => self.builtin_operator_index(args, kwargs),
            BuiltinFunction::OperatorEq => self.builtin_operator_eq(args, kwargs),
            BuiltinFunction::OperatorNe => self.builtin_operator_ne(args, kwargs),
            BuiltinFunction::OperatorLt => self.builtin_operator_lt(args, kwargs),
            BuiltinFunction::OperatorLe => self.builtin_operator_le(args, kwargs),
            BuiltinFunction::OperatorGt => self.builtin_operator_gt(args, kwargs),
            BuiltinFunction::OperatorGe => self.builtin_operator_ge(args, kwargs),
            BuiltinFunction::OperatorContains => self.builtin_operator_contains(args, kwargs),
            BuiltinFunction::OperatorGetItem => self.builtin_operator_getitem(args, kwargs),
            BuiltinFunction::OperatorItemGetter => self.builtin_operator_itemgetter(args, kwargs),
            BuiltinFunction::OperatorAttrGetter => self.builtin_operator_attrgetter(args, kwargs),
            BuiltinFunction::OperatorMethodCaller => {
                self.builtin_operator_methodcaller(args, kwargs)
            }
            BuiltinFunction::OperatorCompareDigest => {
                self.builtin_operator_compare_digest(args, kwargs)
            }
            BuiltinFunction::ItertoolsChain => self.builtin_itertools_chain(args, kwargs),
            BuiltinFunction::ItertoolsChainFromIterable => {
                self.builtin_itertools_chain_from_iterable(args, kwargs)
            }
            BuiltinFunction::ItertoolsAccumulate => self.builtin_itertools_accumulate(args, kwargs),
            BuiltinFunction::ItertoolsCombinations => {
                self.builtin_itertools_combinations(args, kwargs)
            }
            BuiltinFunction::ItertoolsCombinationsWithReplacement => {
                self.builtin_itertools_combinations_with_replacement(args, kwargs)
            }
            BuiltinFunction::ItertoolsCompress => self.builtin_itertools_compress(args, kwargs),
            BuiltinFunction::ItertoolsCount => self.builtin_itertools_count(args, kwargs),
            BuiltinFunction::ItertoolsCycle => self.builtin_itertools_cycle(args, kwargs),
            BuiltinFunction::ItertoolsDropWhile => self.builtin_itertools_dropwhile(args, kwargs),
            BuiltinFunction::ItertoolsFilterFalse => {
                self.builtin_itertools_filterfalse(args, kwargs)
            }
            BuiltinFunction::ItertoolsGroupBy => self.builtin_itertools_groupby(args, kwargs),
            BuiltinFunction::ItertoolsISlice => self.builtin_itertools_islice(args, kwargs),
            BuiltinFunction::ItertoolsPairwise => self.builtin_itertools_pairwise(args, kwargs),
            BuiltinFunction::ItertoolsRepeat => self.builtin_itertools_repeat(args, kwargs),
            BuiltinFunction::ItertoolsStarMap => self.builtin_itertools_starmap(args, kwargs),
            BuiltinFunction::ItertoolsTakeWhile => self.builtin_itertools_takewhile(args, kwargs),
            BuiltinFunction::ItertoolsTee => self.builtin_itertools_tee(args, kwargs),
            BuiltinFunction::ItertoolsZipLongest => {
                self.builtin_itertools_zip_longest(args, kwargs)
            }
            BuiltinFunction::ItertoolsBatched => self.builtin_itertools_batched(args, kwargs),
            BuiltinFunction::ItertoolsPermutations => {
                self.builtin_itertools_permutations(args, kwargs)
            }
            BuiltinFunction::ItertoolsProduct => self.builtin_itertools_product(args, kwargs),
            BuiltinFunction::FunctoolsReduce => self.builtin_functools_reduce(args, kwargs),
            BuiltinFunction::FunctoolsSingleDispatch => {
                self.builtin_functools_singledispatch(args, kwargs)
            }
            BuiltinFunction::FunctoolsSingleDispatchMethod => {
                self.builtin_functools_singledispatch(args, kwargs)
            }
            BuiltinFunction::FunctoolsSingleDispatchRegister => {
                self.builtin_functools_singledispatch_register(args, kwargs)
            }
            BuiltinFunction::FunctoolsWraps => self.builtin_functools_wraps(args, kwargs),
            BuiltinFunction::FunctoolsPartial => self.builtin_functools_partial(args, kwargs),
            BuiltinFunction::FunctoolsCmpToKey => self.builtin_functools_cmp_to_key(args, kwargs),
            BuiltinFunction::FunctoolsCachedProperty => {
                self.builtin_functools_cached_property(args, kwargs)
            }
            BuiltinFunction::CollectionsCounter => self.builtin_collections_counter(args, kwargs),
            BuiltinFunction::CollectionsDeque => self.builtin_collections_deque(args, kwargs),
            BuiltinFunction::CollectionsDequeInit => {
                self.builtin_collections_deque_init(args, kwargs)
            }
            BuiltinFunction::CollectionsDequeAppend => {
                self.builtin_collections_deque_append(args, kwargs)
            }
            BuiltinFunction::CollectionsDequeAppendLeft => {
                self.builtin_collections_deque_appendleft(args, kwargs)
            }
            BuiltinFunction::CollectionsDequePop => {
                self.builtin_collections_deque_pop(args, kwargs)
            }
            BuiltinFunction::CollectionsDequePopleft => {
                self.builtin_collections_deque_popleft(args, kwargs)
            }
            BuiltinFunction::CollectionsDequeClear => {
                self.builtin_collections_deque_clear(args, kwargs)
            }
            BuiltinFunction::CollectionsDequeExtend => {
                self.builtin_collections_deque_extend(args, kwargs)
            }
            BuiltinFunction::CollectionsDequeExtendLeft => {
                self.builtin_collections_deque_extendleft(args, kwargs)
            }
            BuiltinFunction::CollectionsDequeLen => {
                self.builtin_collections_deque_len(args, kwargs)
            }
            BuiltinFunction::CollectionsDequeIter => {
                self.builtin_collections_deque_iter(args, kwargs)
            }
            BuiltinFunction::CollectionsOrderedDict => {
                self.builtin_collections_ordereddict(args, kwargs)
            }
            BuiltinFunction::CollectionsChainMapInit => {
                self.builtin_collections_chainmap_init(args, kwargs)
            }
            BuiltinFunction::CollectionsChainMapNewChild => {
                self.builtin_collections_chainmap_new_child(args, kwargs)
            }
            BuiltinFunction::CollectionsChainMapRepr => {
                self.builtin_collections_chainmap_repr(args, kwargs)
            }
            BuiltinFunction::CollectionsChainMapItems => {
                self.builtin_collections_chainmap_items(args, kwargs)
            }
            BuiltinFunction::CollectionsChainMapGet => {
                self.builtin_collections_chainmap_get(args, kwargs)
            }
            BuiltinFunction::CollectionsChainMapGetItem => {
                self.builtin_collections_chainmap_getitem(args, kwargs)
            }
            BuiltinFunction::CollectionsChainMapSetItem => {
                self.builtin_collections_chainmap_setitem(args, kwargs)
            }
            BuiltinFunction::CollectionsChainMapDelItem => {
                self.builtin_collections_chainmap_delitem(args, kwargs)
            }
            BuiltinFunction::CollectionsDefaultDict => {
                self.builtin_collections_defaultdict(args, kwargs)
            }
            BuiltinFunction::CollectionsCountElements => {
                self.builtin_collections_count_elements(args, kwargs)
            }
            BuiltinFunction::InspectSignature => self.builtin_inspect_signature(args, kwargs),
            BuiltinFunction::InspectSignatureInit => {
                self.builtin_inspect_signature_init(args, kwargs)
            }
            BuiltinFunction::InspectSignatureStr => {
                self.builtin_inspect_signature_str(args, kwargs)
            }
            BuiltinFunction::InspectSignatureRepr => {
                self.builtin_inspect_signature_repr(args, kwargs)
            }
            BuiltinFunction::InspectSignatureEq => self.builtin_inspect_signature_eq(args, kwargs),
            BuiltinFunction::InspectSignatureReplace => {
                self.builtin_inspect_signature_replace(args, kwargs)
            }
            BuiltinFunction::InspectSignatureBind => {
                self.builtin_inspect_signature_bind(args, kwargs, false)
            }
            BuiltinFunction::InspectSignatureBindPartial => {
                self.builtin_inspect_signature_bind(args, kwargs, true)
            }
            BuiltinFunction::InspectParameterInit => {
                self.builtin_inspect_parameter_init(args, kwargs)
            }
            BuiltinFunction::InspectParameterReplace => {
                self.builtin_inspect_parameter_replace(args, kwargs)
            }
            BuiltinFunction::InspectGetModule => self.builtin_inspect_getmodule(args, kwargs),
            BuiltinFunction::InspectGetFile => self.builtin_inspect_getfile(args, kwargs),
            BuiltinFunction::InspectGetDoc => self.builtin_inspect_getdoc(args, kwargs),
            BuiltinFunction::InspectGetSourceFile => {
                self.builtin_inspect_getsourcefile(args, kwargs)
            }
            BuiltinFunction::InspectCleanDoc => self.builtin_inspect_cleandoc(args, kwargs),
            BuiltinFunction::InspectIsAbstract => self.builtin_inspect_isabstract(args, kwargs),
            BuiltinFunction::InspectIsFunction => self.builtin_inspect_isfunction(args, kwargs),
            BuiltinFunction::InspectIsMethod => self.builtin_inspect_ismethod(args, kwargs),
            BuiltinFunction::InspectIsRoutine => self.builtin_inspect_isroutine(args, kwargs),
            BuiltinFunction::InspectIsMethodDescriptor => {
                self.builtin_inspect_ismethoddescriptor(args, kwargs)
            }
            BuiltinFunction::InspectIsDataDescriptor => {
                self.builtin_inspect_isdatadescriptor(args, kwargs)
            }
            BuiltinFunction::InspectIsMethodWrapper => {
                self.builtin_inspect_ismethodwrapper(args, kwargs)
            }
            BuiltinFunction::InspectIsTraceback => self.builtin_inspect_istraceback(args, kwargs),
            BuiltinFunction::InspectIsFrame => self.builtin_inspect_isframe(args, kwargs),
            BuiltinFunction::InspectIsCode => self.builtin_inspect_iscode(args, kwargs),
            BuiltinFunction::InspectUnwrap => self.builtin_inspect_unwrap(args, kwargs),
            BuiltinFunction::InspectGetAttrStatic => {
                self.builtin_inspect_getattr_static(args, kwargs)
            }
            BuiltinFunction::InspectIsClass => self.builtin_inspect_isclass(args, kwargs),
            BuiltinFunction::InspectIsModule => self.builtin_inspect_ismodule(args, kwargs),
            BuiltinFunction::InspectIsGenerator => self.builtin_inspect_isgenerator(args, kwargs),
            BuiltinFunction::InspectIsCoroutine => self.builtin_inspect_iscoroutine(args, kwargs),
            BuiltinFunction::InspectIsCoroutineFunction => {
                self.builtin_inspect_iscoroutinefunction(args, kwargs)
            }
            BuiltinFunction::InspectIsAwaitable => self.builtin_inspect_isawaitable(args, kwargs),
            BuiltinFunction::InspectIsAsyncGen => self.builtin_inspect_isasyncgen(args, kwargs),
            BuiltinFunction::InspectMarkCoroutineFunction => {
                self.builtin_inspect_markcoroutinefunction(args, kwargs)
            }
            BuiltinFunction::InspectStaticGetMro => {
                self.builtin_inspect_static_getmro(args, kwargs)
            }
            BuiltinFunction::InspectGetDunderDictOfClass => {
                self.builtin_inspect_get_dunder_dict_of_class(args, kwargs)
            }
            BuiltinFunction::TypesModuleType => self.builtin_types_moduletype(args, kwargs),
            BuiltinFunction::TypesMappingProxy => self.builtin_types_mappingproxy(args, kwargs),
            BuiltinFunction::SimpleNamespaceTypeRepr => {
                self.builtin_types_simplenamespace_repr(args, kwargs)
            }
            BuiltinFunction::SimpleNamespaceInit => {
                self.builtin_types_simplenamespace_init(args, kwargs)
            }
            BuiltinFunction::SimpleNamespaceEq => {
                self.builtin_types_simplenamespace_eq(args, kwargs)
            }
            BuiltinFunction::SimpleNamespaceReduce => {
                self.builtin_types_simplenamespace_reduce(args, kwargs)
            }
            BuiltinFunction::SimpleNamespaceReplace => {
                self.builtin_types_simplenamespace_replace(args, kwargs)
            }
            BuiltinFunction::TypesFunctionType => self.builtin_types_functiontype(args, kwargs),
            BuiltinFunction::TypesMethodType => self.builtin_types_methodtype(args, kwargs),
            BuiltinFunction::TypesCoroutine => self.builtin_types_coroutine(args, kwargs),
            BuiltinFunction::TypesNewClass => self.builtin_types_new_class(args, kwargs),
            BuiltinFunction::EnumConvert => self.builtin_enum_convert(args, kwargs),
            BuiltinFunction::TypeAnnotationsGet => self.builtin_type_annotations_get(args, kwargs),
            BuiltinFunction::TestCapiExceptionPrint => {
                self.builtin_testcapi_exception_print(args, kwargs)
            }
            BuiltinFunction::TestCapiConfigGet => self.builtin_testcapi_config_get(args, kwargs),
            BuiltinFunction::TestCapiPyObjectVectorcall => {
                self.builtin_testcapi_pyobject_vectorcall(args, kwargs)
            }
            BuiltinFunction::TestInternalCapiGetRecursionDepth => {
                self.builtin_testinternalcapi_get_recursion_depth(args, kwargs)
            }
            BuiltinFunction::DataclassesField => self.builtin_dataclasses_field(args, kwargs),
            BuiltinFunction::DataclassesIsDataclass => {
                self.builtin_dataclasses_is_dataclass(args, kwargs)
            }
            BuiltinFunction::DataclassesFields => self.builtin_dataclasses_fields(args, kwargs),
            BuiltinFunction::DataclassesAsDict => self.builtin_dataclasses_asdict(args, kwargs),
            BuiltinFunction::DataclassesAsTuple => self.builtin_dataclasses_astuple(args, kwargs),
            BuiltinFunction::DataclassesReplace => self.builtin_dataclasses_replace(args, kwargs),
            BuiltinFunction::DataclassesMakeDataclass => {
                self.builtin_dataclasses_make_dataclass(args, kwargs)
            }
            BuiltinFunction::IoOpen => self.builtin_io_open(args, kwargs),
            BuiltinFunction::IoOpenCode => self.builtin_io_open_code(args, kwargs),
            BuiltinFunction::IoReadText => self.builtin_io_read_text(args, kwargs),
            BuiltinFunction::IoWriteText => self.builtin_io_write_text(args, kwargs),
            BuiltinFunction::IoTextEncoding => self.builtin_io_text_encoding(args, kwargs),
            BuiltinFunction::IoIncrementalNewlineDecoderInit => {
                self.builtin_io_incremental_newline_decoder_init(args, kwargs)
            }
            BuiltinFunction::IoIncrementalNewlineDecoderDecode => {
                self.builtin_io_incremental_newline_decoder_decode(args, kwargs)
            }
            BuiltinFunction::IoIncrementalNewlineDecoderGetState => {
                self.builtin_io_incremental_newline_decoder_getstate(args, kwargs)
            }
            BuiltinFunction::IoIncrementalNewlineDecoderSetState => {
                self.builtin_io_incremental_newline_decoder_setstate(args, kwargs)
            }
            BuiltinFunction::IoIncrementalNewlineDecoderReset => {
                self.builtin_io_incremental_newline_decoder_reset(args, kwargs)
            }
            BuiltinFunction::IoTextIOWrapperInit => {
                self.builtin_io_textiowrapper_init(args, kwargs)
            }
            BuiltinFunction::IoFileInit => self.builtin_io_file_init(args, kwargs),
            BuiltinFunction::IoFileRead => self.builtin_io_file_read(args, kwargs),
            BuiltinFunction::IoFileReadLine => self.builtin_io_file_readline(args, kwargs),
            BuiltinFunction::IoFileReadInto => self.builtin_io_file_readinto(args, kwargs),
            BuiltinFunction::IoFileReadLines => self.builtin_io_file_readlines(args, kwargs),
            BuiltinFunction::IoFileWrite => self.builtin_io_file_write(args, kwargs),
            BuiltinFunction::IoFileWriteLines => self.builtin_io_file_writelines(args, kwargs),
            BuiltinFunction::IoFileTruncate => self.builtin_io_file_truncate(args, kwargs),
            BuiltinFunction::IoFileSeek => self.builtin_io_file_seek(args, kwargs),
            BuiltinFunction::IoFileTell => self.builtin_io_file_tell(args, kwargs),
            BuiltinFunction::IoFileClose => self.builtin_io_file_close(args, kwargs),
            BuiltinFunction::IoFileFlush => self.builtin_io_file_flush(args, kwargs),
            BuiltinFunction::IoFileIter => self.builtin_io_file_iter(args, kwargs),
            BuiltinFunction::IoFileNext => self.builtin_io_file_next(args, kwargs),
            BuiltinFunction::IoFileEnter => self.builtin_io_file_enter(args, kwargs),
            BuiltinFunction::IoFileExit => self.builtin_io_file_exit(args, kwargs),
            BuiltinFunction::IoFileFileno => self.builtin_io_file_fileno(args, kwargs),
            BuiltinFunction::IoFileDetach => self.builtin_io_file_detach(args, kwargs),
            BuiltinFunction::IoFileReadable => self.builtin_io_file_readable(args, kwargs),
            BuiltinFunction::IoFileWritable => self.builtin_io_file_writable(args, kwargs),
            BuiltinFunction::IoFileSeekable => self.builtin_io_file_seekable(args, kwargs),
            BuiltinFunction::IoBufferedInit => self.builtin_io_buffered_init(args, kwargs),
            BuiltinFunction::IoBufferedRead => self.builtin_io_buffered_read(args, kwargs),
            BuiltinFunction::IoBufferedRead1 => self.builtin_io_buffered_read1(args, kwargs),
            BuiltinFunction::IoBufferedReadLine => self.builtin_io_buffered_readline(args, kwargs),
            BuiltinFunction::IoBufferedWrite => self.builtin_io_buffered_write(args, kwargs),
            BuiltinFunction::IoBufferedFlush => self.builtin_io_buffered_flush(args, kwargs),
            BuiltinFunction::IoBufferedClose => self.builtin_io_buffered_close(args, kwargs),
            BuiltinFunction::IoBufferedDetach => self.builtin_io_buffered_detach(args, kwargs),
            BuiltinFunction::IoBufferedFileno => self.builtin_io_buffered_fileno(args, kwargs),
            BuiltinFunction::IoBufferedSeek => self.builtin_io_buffered_seek(args, kwargs),
            BuiltinFunction::IoBufferedTell => self.builtin_io_buffered_tell(args, kwargs),
            BuiltinFunction::IoBufferedTruncate => self.builtin_io_buffered_truncate(args, kwargs),
            BuiltinFunction::IoBufferedReadInto => self.builtin_io_buffered_readinto(args, kwargs),
            BuiltinFunction::IoBufferedReadInto1 => {
                self.builtin_io_buffered_readinto1(args, kwargs)
            }
            BuiltinFunction::IoBufferedPeek => self.builtin_io_buffered_peek(args, kwargs),
            BuiltinFunction::IoBufferedReadable => self.builtin_io_buffered_readable(args, kwargs),
            BuiltinFunction::IoBufferedWritable => self.builtin_io_buffered_writable(args, kwargs),
            BuiltinFunction::IoBufferedSeekable => self.builtin_io_buffered_seekable(args, kwargs),
            BuiltinFunction::IoBufferedRWPairInit => {
                self.builtin_io_buffered_rwpair_init(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairRead => {
                self.builtin_io_buffered_rwpair_read(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairReadLine => {
                self.builtin_io_buffered_rwpair_readline(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairRead1 => {
                self.builtin_io_buffered_rwpair_read1(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairReadInto => {
                self.builtin_io_buffered_rwpair_readinto(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairReadInto1 => {
                self.builtin_io_buffered_rwpair_readinto1(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairWrite => {
                self.builtin_io_buffered_rwpair_write(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairFlush => {
                self.builtin_io_buffered_rwpair_flush(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairClose => {
                self.builtin_io_buffered_rwpair_close(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairReadable => {
                self.builtin_io_buffered_rwpair_readable(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairWritable => {
                self.builtin_io_buffered_rwpair_writable(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairIsAtty => {
                self.builtin_io_buffered_rwpair_isatty(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairSeekable => {
                self.builtin_io_buffered_rwpair_seekable(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairDetach => {
                self.builtin_io_buffered_rwpair_detach(args, kwargs)
            }
            BuiltinFunction::IoBufferedRWPairPeek => {
                self.builtin_io_buffered_rwpair_peek(args, kwargs)
            }
            BuiltinFunction::IoRawRead => self.builtin_io_raw_read(args, kwargs),
            BuiltinFunction::IoRawReadAll => self.builtin_io_raw_readall(args, kwargs),
            BuiltinFunction::IoBaseReadLine => self.builtin_iobase_readline(args, kwargs),
            BuiltinFunction::IoBaseReadLines => self.builtin_iobase_readlines(args, kwargs),
            BuiltinFunction::IoBaseWriteLines => self.builtin_iobase_writelines(args, kwargs),
            BuiltinFunction::IoBaseEnter => self.builtin_iobase_enter(args, kwargs),
            BuiltinFunction::IoBaseExit => self.builtin_iobase_exit(args, kwargs),
            BuiltinFunction::IoBaseIter => self.builtin_iobase_iter(args, kwargs),
            BuiltinFunction::IoBaseNext => self.builtin_iobase_next(args, kwargs),
            BuiltinFunction::IoBaseClose => self.builtin_iobase_close(args, kwargs),
            BuiltinFunction::IoBaseFlush => self.builtin_iobase_flush(args, kwargs),
            BuiltinFunction::IoBaseDel => self.builtin_iobase_del(args, kwargs),
            BuiltinFunction::StringIOInit => self.builtin_stringio_init(args, kwargs),
            BuiltinFunction::StringIOWrite => self.builtin_stringio_write(args, kwargs),
            BuiltinFunction::StringIORead => self.builtin_stringio_read(args, kwargs),
            BuiltinFunction::StringIOReadLine => self.builtin_stringio_readline(args, kwargs),
            BuiltinFunction::StringIOReadLines => self.builtin_stringio_readlines(args, kwargs),
            BuiltinFunction::StringIOGetValue => self.builtin_stringio_getvalue(args, kwargs),
            BuiltinFunction::StringIOGetState => self.builtin_stringio_getstate(args, kwargs),
            BuiltinFunction::StringIOSetState => self.builtin_stringio_setstate(args, kwargs),
            BuiltinFunction::StringIOSeek => self.builtin_stringio_seek(args, kwargs),
            BuiltinFunction::StringIOTell => self.builtin_stringio_tell(args, kwargs),
            BuiltinFunction::StringIOWriteLines => self.builtin_stringio_writelines(args, kwargs),
            BuiltinFunction::StringIOTruncate => self.builtin_stringio_truncate(args, kwargs),
            BuiltinFunction::StringIODetach => self.builtin_stringio_detach(args, kwargs),
            BuiltinFunction::StringIOIter => self.builtin_stringio_iter(args, kwargs),
            BuiltinFunction::StringIONext => self.builtin_stringio_next(args, kwargs),
            BuiltinFunction::StringIOEnter => self.builtin_stringio_enter(args, kwargs),
            BuiltinFunction::StringIOExit => self.builtin_stringio_exit(args, kwargs),
            BuiltinFunction::StringIOClose => self.builtin_stringio_close(args, kwargs),
            BuiltinFunction::StringIOFlush => self.builtin_stringio_flush(args, kwargs),
            BuiltinFunction::StringIOIsAtty => self.builtin_stringio_isatty(args, kwargs),
            BuiltinFunction::StringIOFileno => self.builtin_stringio_fileno(args, kwargs),
            BuiltinFunction::StringIOReadable => self.builtin_stringio_readable(args, kwargs),
            BuiltinFunction::StringIOWritable => self.builtin_stringio_writable(args, kwargs),
            BuiltinFunction::StringIOSeekable => self.builtin_stringio_seekable(args, kwargs),
            BuiltinFunction::BytesIOInit => self.builtin_bytesio_init(args, kwargs),
            BuiltinFunction::BytesIOWrite => self.builtin_bytesio_write(args, kwargs),
            BuiltinFunction::BytesIOWriteLines => self.builtin_bytesio_writelines(args, kwargs),
            BuiltinFunction::BytesIOTruncate => self.builtin_bytesio_truncate(args, kwargs),
            BuiltinFunction::BytesIORead => self.builtin_bytesio_read(args, kwargs),
            BuiltinFunction::BytesIORead1 => self.builtin_bytesio_read1(args, kwargs),
            BuiltinFunction::BytesIOReadLine => self.builtin_bytesio_readline(args, kwargs),
            BuiltinFunction::BytesIOReadLines => self.builtin_bytesio_readlines(args, kwargs),
            BuiltinFunction::BytesIOReadInto => self.builtin_bytesio_readinto(args, kwargs),
            BuiltinFunction::BytesIOGetValue => self.builtin_bytesio_getvalue(args, kwargs),
            BuiltinFunction::BytesIOGetBuffer => self.builtin_bytesio_getbuffer(args, kwargs),
            BuiltinFunction::BytesIOGetState => self.builtin_bytesio_getstate(args, kwargs),
            BuiltinFunction::BytesIOSetState => self.builtin_bytesio_setstate(args, kwargs),
            BuiltinFunction::BytesIODetach => self.builtin_bytesio_detach(args, kwargs),
            BuiltinFunction::BytesIOSeek => self.builtin_bytesio_seek(args, kwargs),
            BuiltinFunction::BytesIOTell => self.builtin_bytesio_tell(args, kwargs),
            BuiltinFunction::BytesIOFlush => self.builtin_bytesio_flush(args, kwargs),
            BuiltinFunction::BytesIOIsAtty => self.builtin_bytesio_isatty(args, kwargs),
            BuiltinFunction::BytesIOIter => self.builtin_bytesio_iter(args, kwargs),
            BuiltinFunction::BytesIONext => self.builtin_bytesio_next(args, kwargs),
            BuiltinFunction::BytesIOEnter => self.builtin_bytesio_enter(args, kwargs),
            BuiltinFunction::BytesIOExit => self.builtin_bytesio_exit(args, kwargs),
            BuiltinFunction::BytesIOClose => self.builtin_bytesio_close(args, kwargs),
            BuiltinFunction::BytesIOReadable => self.builtin_bytesio_readable(args, kwargs),
            BuiltinFunction::BytesIOWritable => self.builtin_bytesio_writable(args, kwargs),
            BuiltinFunction::BytesIOSeekable => self.builtin_bytesio_seekable(args, kwargs),
            BuiltinFunction::BytesIOFileno => self.builtin_bytesio_fileno(args, kwargs),
            BuiltinFunction::StructCalcSize => self.builtin_struct_calcsize(args, kwargs),
            BuiltinFunction::StructPack => self.builtin_struct_pack(args, kwargs),
            BuiltinFunction::StructUnpack => self.builtin_struct_unpack(args, kwargs),
            BuiltinFunction::StructIterUnpack => self.builtin_struct_iter_unpack(args, kwargs),
            BuiltinFunction::StructPackInto => self.builtin_struct_pack_into(args, kwargs),
            BuiltinFunction::StructUnpackFrom => self.builtin_struct_unpack_from(args, kwargs),
            BuiltinFunction::StringFormatterParser => {
                self.builtin_string_formatter_parser(args, kwargs)
            }
            BuiltinFunction::StringFormatterFieldNameSplit => {
                self.builtin_string_formatter_field_name_split(args, kwargs)
            }
            BuiltinFunction::StructClassInit => self.builtin_struct_class_init(args, kwargs),
            BuiltinFunction::StructClassPack => self.builtin_struct_class_pack(args, kwargs),
            BuiltinFunction::StructClassUnpack => self.builtin_struct_class_unpack(args, kwargs),
            BuiltinFunction::StructClassIterUnpack => {
                self.builtin_struct_class_iter_unpack(args, kwargs)
            }
            BuiltinFunction::StructClassPackInto => {
                self.builtin_struct_class_pack_into(args, kwargs)
            }
            BuiltinFunction::StructClassUnpackFrom => {
                self.builtin_struct_class_unpack_from(args, kwargs)
            }
            BuiltinFunction::DateTimeNow => self.builtin_datetime_now(args, kwargs),
            BuiltinFunction::DateToday => self.builtin_datetime_today(args, kwargs),
            BuiltinFunction::DateTimeInit => self.builtin_datetime_init(args, kwargs),
            BuiltinFunction::DateTimeFromTimestamp => {
                self.builtin_datetime_fromtimestamp(args, kwargs)
            }
            BuiltinFunction::DateTimeFromIsoCalendar => {
                self.builtin_datetime_fromisocalendar(args, kwargs)
            }
            BuiltinFunction::DateTimeAstimezone => self.builtin_datetime_astimezone(args, kwargs),
            BuiltinFunction::DateTimeReplace => self.builtin_datetime_replace(args, kwargs),
            BuiltinFunction::DateInit => self.builtin_date_init(args, kwargs),
            BuiltinFunction::DateReplace => self.builtin_date_replace(args, kwargs),
            BuiltinFunction::DateFromIsoCalendar => self.builtin_date_fromisocalendar(args, kwargs),
            BuiltinFunction::DateTimeDeltaInit => self.builtin_datetime_delta_init(args, kwargs),
            BuiltinFunction::DateTimeTimezoneInit => {
                self.builtin_datetime_timezone_init(args, kwargs)
            }
            BuiltinFunction::DateToOrdinal => self.builtin_date_toordinal(args, kwargs),
            BuiltinFunction::DateWeekday => self.builtin_date_weekday(args, kwargs),
            BuiltinFunction::DateIsoWeekday => self.builtin_date_isoweekday(args, kwargs),
            BuiltinFunction::DateIsoFormat => self.builtin_date_isoformat(args, kwargs),
            BuiltinFunction::DateStrFTime => self.builtin_date_strftime(args, kwargs),
            BuiltinFunction::TimeInit => self.builtin_time_init(args, kwargs),
            BuiltinFunction::TimeReplace => self.builtin_time_replace(args, kwargs),
            BuiltinFunction::AsyncioRun => self.builtin_asyncio_run(args, kwargs),
            BuiltinFunction::AsyncioSleep => self.builtin_asyncio_sleep(args, kwargs),
            BuiltinFunction::AsyncioCreateTask => self.builtin_asyncio_create_task(args, kwargs),
            BuiltinFunction::AsyncioGather => self.builtin_asyncio_gather(args, kwargs),
            BuiltinFunction::ThreadingExcepthook => self.builtin_threading_excepthook(args, kwargs),
            BuiltinFunction::ThreadingGetIdent => self.builtin_threading_get_ident(args, kwargs),
            BuiltinFunction::ThreadStartNewThread => {
                self.builtin_thread_start_new_thread(args, kwargs)
            }
            BuiltinFunction::ThreadLockEnter => self.builtin_thread_lock_enter(args, kwargs),
            BuiltinFunction::ThreadLockExit => self.builtin_thread_lock_exit(args, kwargs),
            BuiltinFunction::ThreadLockAcquire => self.builtin_thread_lock_acquire(args, kwargs),
            BuiltinFunction::ThreadLockRelease => self.builtin_thread_lock_release(args, kwargs),
            BuiltinFunction::ThreadLockLocked => self.builtin_thread_lock_locked(args, kwargs),
            BuiltinFunction::ThreadingCurrentThread => {
                self.builtin_threading_current_thread(args, kwargs)
            }
            BuiltinFunction::ThreadingMainThread => {
                self.builtin_threading_main_thread(args, kwargs)
            }
            BuiltinFunction::ThreadingActiveCount => {
                self.builtin_threading_active_count(args, kwargs)
            }
            BuiltinFunction::ThreadingRegisterAtexit => {
                self.builtin_threading_register_atexit(args, kwargs)
            }
            BuiltinFunction::ThreadClassInit => self.builtin_thread_class_init(args, kwargs),
            BuiltinFunction::ThreadClassStart => self.builtin_thread_class_start(args, kwargs),
            BuiltinFunction::ThreadClassJoin => self.builtin_thread_class_join(args, kwargs),
            BuiltinFunction::ThreadClassIsAlive => self.builtin_thread_class_is_alive(args, kwargs),
            BuiltinFunction::ThreadEventInit => self.builtin_thread_event_init(args, kwargs),
            BuiltinFunction::ThreadEventClear => self.builtin_thread_event_clear(args, kwargs),
            BuiltinFunction::ThreadEventIsSet => self.builtin_thread_event_is_set(args, kwargs),
            BuiltinFunction::ThreadEventSet => self.builtin_thread_event_set(args, kwargs),
            BuiltinFunction::ThreadEventWait => self.builtin_thread_event_wait(args, kwargs),
            BuiltinFunction::ThreadConditionInit => {
                self.builtin_thread_condition_init(args, kwargs)
            }
            BuiltinFunction::ThreadConditionAcquire => {
                self.builtin_thread_condition_acquire(args, kwargs)
            }
            BuiltinFunction::ThreadConditionEnter => {
                self.builtin_thread_condition_enter(args, kwargs)
            }
            BuiltinFunction::ThreadConditionNotify => {
                self.builtin_thread_condition_notify(args, kwargs)
            }
            BuiltinFunction::ThreadConditionNotifyAll => {
                self.builtin_thread_condition_notify_all(args, kwargs)
            }
            BuiltinFunction::ThreadConditionRelease => {
                self.builtin_thread_condition_release(args, kwargs)
            }
            BuiltinFunction::ThreadConditionExit => {
                self.builtin_thread_condition_exit(args, kwargs)
            }
            BuiltinFunction::ThreadConditionWait => {
                self.builtin_thread_condition_wait(args, kwargs)
            }
            BuiltinFunction::ThreadSemaphoreInit => {
                self.builtin_thread_semaphore_init(args, kwargs)
            }
            BuiltinFunction::ThreadSemaphoreAcquire => {
                self.builtin_thread_semaphore_acquire(args, kwargs)
            }
            BuiltinFunction::ThreadSemaphoreRelease => {
                self.builtin_thread_semaphore_release(args, kwargs)
            }
            BuiltinFunction::ThreadBoundedSemaphoreInit => {
                self.builtin_thread_bounded_semaphore_init(args, kwargs)
            }
            BuiltinFunction::ThreadBarrierInit => self.builtin_thread_barrier_init(args, kwargs),
            BuiltinFunction::ThreadBarrierAbort => self.builtin_thread_barrier_abort(args, kwargs),
            BuiltinFunction::ThreadBarrierReset => self.builtin_thread_barrier_reset(args, kwargs),
            BuiltinFunction::ThreadBarrierWait => self.builtin_thread_barrier_wait(args, kwargs),
            BuiltinFunction::SignalSignal => self.builtin_signal_signal(args, kwargs),
            BuiltinFunction::SignalGetSignal => self.builtin_signal_getsignal(args, kwargs),
            BuiltinFunction::SignalRaiseSignal => self.builtin_signal_raise_signal(args, kwargs),
            BuiltinFunction::LocaleSetLocale => self.builtin_locale_setlocale(args, kwargs),
            BuiltinFunction::LocaleLocaleConv => self.builtin_locale_localeconv(args, kwargs),
            BuiltinFunction::SocketGetHostName => self.builtin_socket_gethostname(args, kwargs),
            BuiltinFunction::SocketGetHostByName => self.builtin_socket_gethostbyname(args, kwargs),
            BuiltinFunction::SocketGetAddrInfo => self.builtin_socket_getaddrinfo(args, kwargs),
            BuiltinFunction::SocketFromFd => self.builtin_socket_fromfd(args, kwargs),
            BuiltinFunction::SocketGetDefaultTimeout => {
                self.builtin_socket_getdefaulttimeout(args, kwargs)
            }
            BuiltinFunction::SocketSetDefaultTimeout => {
                self.builtin_socket_setdefaulttimeout(args, kwargs)
            }
            BuiltinFunction::SocketNtoHs => self.builtin_socket_ntohs(args, kwargs),
            BuiltinFunction::SocketNtoHl => self.builtin_socket_ntohl(args, kwargs),
            BuiltinFunction::SocketHtoNs => self.builtin_socket_htons(args, kwargs),
            BuiltinFunction::SocketHtoNl => self.builtin_socket_htonl(args, kwargs),
            BuiltinFunction::SocketObjectInit => self.builtin_socket_object_init(args, kwargs),
            BuiltinFunction::SocketObjectClose => self.builtin_socket_object_close(args, kwargs),
            BuiltinFunction::SocketObjectDetach => self.builtin_socket_object_detach(args, kwargs),
            BuiltinFunction::SocketObjectFileno => self.builtin_socket_object_fileno(args, kwargs),
            BuiltinFunction::ScproxyGetProxySettings => {
                self.builtin_scproxy_get_proxy_settings(args, kwargs)
            }
            BuiltinFunction::ScproxyGetProxies => self.builtin_scproxy_get_proxies(args, kwargs),
            BuiltinFunction::UuidClassInit => self.builtin_uuid_class_init(args, kwargs),
            BuiltinFunction::UuidGetNode => self.builtin_uuid_getnode(args, kwargs),
            BuiltinFunction::Uuid1 => self.builtin_uuid1(args, kwargs),
            BuiltinFunction::Uuid3 => self.builtin_uuid3(args, kwargs),
            BuiltinFunction::Uuid4 => self.builtin_uuid4(args, kwargs),
            BuiltinFunction::Uuid5 => self.builtin_uuid5(args, kwargs),
            BuiltinFunction::Uuid6 => self.builtin_uuid6(args, kwargs),
            BuiltinFunction::Uuid7 => self.builtin_uuid7(args, kwargs),
            BuiltinFunction::Uuid8 => self.builtin_uuid8(args, kwargs),
            BuiltinFunction::BinasciiCrc32 => self.builtin_binascii_crc32(args, kwargs),
            BuiltinFunction::BinasciiB2aBase64 => self.builtin_binascii_b2a_base64(args, kwargs),
            BuiltinFunction::BinasciiA2bBase64 => self.builtin_binascii_a2b_base64(args, kwargs),
            BuiltinFunction::BinasciiHexlify => self.builtin_binascii_hexlify(args, kwargs),
            BuiltinFunction::BinasciiUnhexlify => self.builtin_binascii_unhexlify(args, kwargs),
            BuiltinFunction::CsvReader => self.builtin_csv_reader(args, kwargs),
            BuiltinFunction::CsvWriter => self.builtin_csv_writer(args, kwargs),
            BuiltinFunction::CsvWriterRow => self.builtin_csv_writerow(args, kwargs),
            BuiltinFunction::CsvWriterRows => self.builtin_csv_writerows(args, kwargs),
            BuiltinFunction::CsvReaderIter => self.builtin_csv_reader_iter(args, kwargs),
            BuiltinFunction::CsvReaderNext => self.builtin_csv_reader_next(args, kwargs),
            BuiltinFunction::CsvRegisterDialect => self.builtin_csv_register_dialect(args, kwargs),
            BuiltinFunction::CsvUnregisterDialect => {
                self.builtin_csv_unregister_dialect(args, kwargs)
            }
            BuiltinFunction::CsvGetDialect => self.builtin_csv_get_dialect(args, kwargs),
            BuiltinFunction::CsvListDialects => self.builtin_csv_list_dialects(args, kwargs),
            BuiltinFunction::CsvFieldSizeLimit => self.builtin_csv_field_size_limit(args, kwargs),
            BuiltinFunction::CsvDialectValidate => self.builtin_csv_dialect_validate(args, kwargs),
            BuiltinFunction::CollectionsNamedTuple => {
                self.builtin_collections_namedtuple(args, kwargs)
            }
            BuiltinFunction::CollectionsNamedTupleMake => {
                self.builtin_collections_namedtuple_make(args, kwargs)
            }
            BuiltinFunction::CollectionsNamedTupleNew => {
                self.builtin_collections_namedtuple_new(args, kwargs)
            }
            BuiltinFunction::AtexitRegister => self.builtin_atexit_register(args, kwargs),
            BuiltinFunction::AtexitUnregister => self.builtin_atexit_unregister(args, kwargs),
            BuiltinFunction::AtexitRunExitFuncs => self.builtin_atexit_run_exitfuncs(args, kwargs),
            BuiltinFunction::AtexitClear => self.builtin_atexit_clear(args, kwargs),
            BuiltinFunction::ColorizeCanColorize => {
                self.builtin_colorize_can_colorize(args, kwargs)
            }
            BuiltinFunction::ColorizeGetTheme => self.builtin_colorize_get_theme(args, kwargs),
            BuiltinFunction::ColorizeGetColors => self.builtin_colorize_get_colors(args, kwargs),
            BuiltinFunction::ColorizeSetTheme => self.builtin_colorize_set_theme(args, kwargs),
            BuiltinFunction::ColorizeDecolor => self.builtin_colorize_decolor(args, kwargs),
            BuiltinFunction::ColorizeThemeItems => self.builtin_colorize_theme_items(args, kwargs),
            BuiltinFunction::WarningsWarn => self.builtin_warnings_warn(args, kwargs),
            BuiltinFunction::WarningsWarnExplicit => {
                self.builtin_warnings_warn_explicit(args, kwargs)
            }
            BuiltinFunction::WarningsFiltersMutated => {
                self.builtin_warnings_filters_mutated(args, kwargs)
            }
            BuiltinFunction::WarningsAcquireLock => {
                self.builtin_warnings_acquire_lock(args, kwargs)
            }
            BuiltinFunction::WarningsReleaseLock => {
                self.builtin_warnings_release_lock(args, kwargs)
            }
            BuiltinFunction::TypingTypeVar
            | BuiltinFunction::TypingParamSpec
            | BuiltinFunction::TypingTypeVarTuple
            | BuiltinFunction::TypingTypeAliasType => {
                let mut args = args;
                let mut kwargs = kwargs;
                let helper_name = match builtin {
                    BuiltinFunction::TypingTypeVar => "TypeVar",
                    BuiltinFunction::TypingParamSpec => "ParamSpec",
                    BuiltinFunction::TypingTypeVarTuple => "TypeVarTuple",
                    BuiltinFunction::TypingTypeAliasType => "TypeAliasType",
                    _ => unreachable!(),
                };
                if let Some(name_kw) = kwargs.remove("name") {
                    if !args.is_empty() {
                        return Err(RuntimeError::type_error(format!(
                            "{helper_name}() got multiple values for argument 'name'"
                        )));
                    }
                    args.push(name_kw);
                }
                let typevar_constraints =
                    if matches!(builtin, BuiltinFunction::TypingTypeVar) && args.len() > 1 {
                        Some(args[1..].to_vec())
                    } else {
                        None
                    };
                let marker = builtin.call(&self.heap, args)?;
                let module_attr = self.typing_param_caller_module_attr();
                if let Value::Instance(instance) = &marker
                    && let Object::Instance(instance_data) = &mut *instance.kind_mut()
                {
                    instance_data
                        .attrs
                        .insert("__module__".to_string(), module_attr);
                    if !instance_data.attrs.contains_key("__default__") {
                        let default_marker = self.typing_no_default_marker().unwrap_or(Value::None);
                        instance_data
                            .attrs
                            .insert("__default__".to_string(), default_marker);
                    }
                    if matches!(builtin, BuiltinFunction::TypingTypeVar) {
                        if typevar_constraints
                            .as_ref()
                            .is_some_and(|constraints| constraints.len() == 1)
                        {
                            return Err(RuntimeError::type_error(
                                "A single constraint is not allowed",
                            ));
                        }
                        let bound_value = kwargs.remove("bound");
                        if typevar_constraints.is_some() && bound_value.is_some() {
                            return Err(RuntimeError::type_error(
                                "Constraints cannot be combined with bound=...",
                            ));
                        }
                        if let Some(constraints) = typevar_constraints {
                            instance_data.attrs.insert(
                                "__constraints__".to_string(),
                                self.heap.alloc_tuple(constraints),
                            );
                        }
                        if let Some(bound) = bound_value {
                            let checked_bound = match self.call_typing_helper(
                                "_type_check",
                                vec![
                                    bound.clone(),
                                    Value::Str("Bound must be a type.".to_string()),
                                ],
                            ) {
                                Ok(value) => value,
                                Err(err)
                                    if runtime_error_matches_exception(&err, "AttributeError") =>
                                {
                                    bound
                                }
                                Err(err) => return Err(err),
                            };
                            instance_data
                                .attrs
                                .insert("__bound__".to_string(), checked_bound);
                        }
                        let mut pop_bool_kw = |name: &str| -> Result<bool, RuntimeError> {
                            match kwargs.remove(name) {
                                Some(Value::Bool(flag)) => Ok(flag),
                                Some(_) => Err(RuntimeError::type_error(format!(
                                    "TypeVar() argument '{name}' must be a bool"
                                ))),
                                None => Ok(false),
                            }
                        };
                        let covariant = pop_bool_kw("covariant")?;
                        let contravariant = pop_bool_kw("contravariant")?;
                        let infer_variance = pop_bool_kw("infer_variance")?;
                        if covariant && contravariant {
                            return Err(RuntimeError::value_error(
                                "Bivariant types are not supported.",
                            ));
                        }
                        if infer_variance && (covariant || contravariant) {
                            return Err(RuntimeError::value_error(
                                "Variance cannot be specified with infer_variance.",
                            ));
                        }
                        instance_data
                            .attrs
                            .insert("__covariant__".to_string(), Value::Bool(covariant));
                        instance_data
                            .attrs
                            .insert("__contravariant__".to_string(), Value::Bool(contravariant));
                        instance_data.attrs.insert(
                            "__infer_variance__".to_string(),
                            Value::Bool(infer_variance),
                        );
                    } else {
                        instance_data
                            .attrs
                            .insert("__covariant__".to_string(), Value::Bool(false));
                        instance_data
                            .attrs
                            .insert("__contravariant__".to_string(), Value::Bool(false));
                        instance_data
                            .attrs
                            .insert("__infer_variance__".to_string(), Value::Bool(false));
                    }
                    if let Some(default_value) = kwargs.remove("default") {
                        instance_data
                            .attrs
                            .insert("__default__".to_string(), default_value);
                    }
                }
                if let Some(unexpected) = kwargs.keys().next().cloned() {
                    return Err(RuntimeError::type_error(format!(
                        "{helper_name}() got an unexpected keyword argument '{unexpected}'"
                    )));
                }
                Ok(marker)
            }
            BuiltinFunction::TypingNoDefaultNew => self.builtin_typing_nodefault_new(args, kwargs),
            BuiltinFunction::TypingNoDefaultRepr => {
                self.builtin_typing_nodefault_repr(args, kwargs)
            }
            BuiltinFunction::TypingNoDefaultReduce => {
                self.builtin_typing_nodefault_reduce(args, kwargs)
            }
            BuiltinFunction::TypingTypeParamSubst => {
                self.builtin_typing_typeparam_subst(args, kwargs)
            }
            BuiltinFunction::TypingTypeParamPrepareSubst => {
                self.builtin_typing_typeparam_prepare_subst(args, kwargs)
            }
            BuiltinFunction::TypingTypeParamHasDefault => {
                self.builtin_typing_typeparam_has_default(args, kwargs)
            }
            BuiltinFunction::TypingGenericClassGetItem => {
                self.builtin_typing_generic_class_getitem(args, kwargs)
            }
            BuiltinFunction::TypingGenericInitSubclass => {
                self.builtin_typing_generic_init_subclass(args, kwargs)
            }
            BuiltinFunction::AbcGetCacheToken => self.builtin_abc_get_cache_token(args, kwargs),
            BuiltinFunction::AbcInit => self.builtin_abc_init(args, kwargs),
            BuiltinFunction::AbcRegister => self.builtin_abc_register(args, kwargs),
            BuiltinFunction::AbcInstanceCheck => self.builtin_abc_instancecheck(args, kwargs),
            BuiltinFunction::AbcSubclassCheck => self.builtin_abc_subclasscheck(args, kwargs),
            BuiltinFunction::AbcGetDump => self.builtin_abc_get_dump(args, kwargs),
            BuiltinFunction::AbcResetRegistry => self.builtin_abc_reset_registry(args, kwargs),
            BuiltinFunction::AbcResetCaches => self.builtin_abc_reset_caches(args, kwargs),
            BuiltinFunction::Range => self.builtin_range(args, kwargs),
            _ => {
                if kwargs.is_empty() {
                    builtin.call(&self.heap, args)
                } else {
                    call_builtin_with_kwargs(&self.heap, builtin, args, kwargs)
                }
            }
        }
    }
}
