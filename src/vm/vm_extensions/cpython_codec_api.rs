use std::collections::HashMap;
use std::ffi::{c_char, c_void};

use crate::runtime::{BuiltinFunction, Value};

use super::{
    PyCallable_Check, PyErr_BadArgument, PyErr_SetObject, PyExc_TypeError, cpython_call_builtin,
    cpython_call_internal_in_context, cpython_codec_builtin_handler_ptr,
    cpython_codec_call_callable_in_context, cpython_codec_error_info,
    cpython_codec_handler_tuple_result, cpython_codec_lookup_attr_in_context,
    cpython_codec_optional_name, cpython_codec_required_name,
    cpython_codec_stream_fallback_in_context, cpython_new_ptr_for_value, cpython_set_error,
    cpython_set_typed_error, cpython_value_from_ptr, with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Register(search_function: *mut c_void) -> i32 {
    if search_function.is_null() {
        let _ = unsafe { PyErr_BadArgument() };
        return -1;
    }
    if unsafe { PyCallable_Check(search_function) } == 0 {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "argument must be callable");
        -1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Unregister(search_function: *mut c_void) -> i32 {
    if search_function.is_null() {
        let _ = unsafe { PyErr_BadArgument() };
        return -1;
    }
    if unsafe { PyCallable_Check(search_function) } == 0 {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "argument must be callable");
        -1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_KnownEncoding(encoding: *const c_char) -> i32 {
    let Ok(encoding) = cpython_codec_required_name(encoding, "PyCodec_KnownEncoding") else {
        return 0;
    };
    if cpython_call_builtin(BuiltinFunction::CodecsLookup, vec![Value::Str(encoding)]).is_ok() {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Encode(
    object: *mut c_void,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    if object.is_null() {
        let _ = unsafe { PyErr_BadArgument() };
        return std::ptr::null_mut();
    }
    let object_value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(format!("PyCodec_Encode {err}"));
            return std::ptr::null_mut();
        }
    };
    let encoding = match cpython_codec_required_name(encoding, "PyCodec_Encode") {
        Ok(encoding) => encoding,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let errors = match cpython_codec_optional_name(errors, "PyCodec_Encode") {
        Ok(errors) => errors,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let mut args = vec![object_value, Value::Str(encoding)];
    if let Some(errors) = errors {
        args.push(Value::Str(errors));
    }
    match cpython_call_builtin(BuiltinFunction::CodecsEncode, args) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Decode(
    object: *mut c_void,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    if object.is_null() {
        let _ = unsafe { PyErr_BadArgument() };
        return std::ptr::null_mut();
    }
    let object_value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(format!("PyCodec_Decode {err}"));
            return std::ptr::null_mut();
        }
    };
    let encoding = match cpython_codec_required_name(encoding, "PyCodec_Decode") {
        Ok(encoding) => encoding,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let errors = match cpython_codec_optional_name(errors, "PyCodec_Decode") {
        Ok(errors) => errors,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let mut args = vec![object_value, Value::Str(encoding)];
    if let Some(errors) = errors {
        args.push(Value::Str(errors));
    }
    match cpython_call_builtin(BuiltinFunction::CodecsDecode, args) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Encoder(encoding: *const c_char) -> *mut c_void {
    let encoding = match cpython_codec_required_name(encoding, "PyCodec_Encoder") {
        Ok(encoding) => encoding,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        match cpython_codec_lookup_attr_in_context(context, &encoding, "encode") {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Decoder(encoding: *const c_char) -> *mut c_void {
    let encoding = match cpython_codec_required_name(encoding, "PyCodec_Decoder") {
        Ok(encoding) => encoding,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        match cpython_codec_lookup_attr_in_context(context, &encoding, "decode") {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_IncrementalEncoder(
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    let encoding = match cpython_codec_required_name(encoding, "PyCodec_IncrementalEncoder") {
        Ok(encoding) => encoding,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let errors = match cpython_codec_optional_name(errors, "PyCodec_IncrementalEncoder") {
        Ok(errors) => errors,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let factory = match cpython_call_internal_in_context(
            context,
            Value::Builtin(BuiltinFunction::CodecsGetIncrementalEncoder),
            vec![Value::Str(encoding.clone())],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error_from_runtime_error(err);
                return std::ptr::null_mut();
            }
        };
        let mut args = Vec::new();
        if let Some(errors) = errors {
            args.push(Value::Str(errors));
        }
        match cpython_codec_call_callable_in_context(context, factory, args) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_IncrementalDecoder(
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    let encoding = match cpython_codec_required_name(encoding, "PyCodec_IncrementalDecoder") {
        Ok(encoding) => encoding,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let errors = match cpython_codec_optional_name(errors, "PyCodec_IncrementalDecoder") {
        Ok(errors) => errors,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let factory = match cpython_call_internal_in_context(
            context,
            Value::Builtin(BuiltinFunction::CodecsGetIncrementalDecoder),
            vec![Value::Str(encoding.clone())],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error_from_runtime_error(err);
                return std::ptr::null_mut();
            }
        };
        let mut args = Vec::new();
        if let Some(errors) = errors {
            args.push(Value::Str(errors));
        }
        match cpython_codec_call_callable_in_context(context, factory, args) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_StreamReader(
    encoding: *const c_char,
    stream: *mut c_void,
    errors: *const c_char,
) -> *mut c_void {
    if stream.is_null() {
        let _ = unsafe { PyErr_BadArgument() };
        return std::ptr::null_mut();
    }
    let stream_value = match cpython_value_from_ptr(stream) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(format!("PyCodec_StreamReader {err}"));
            return std::ptr::null_mut();
        }
    };
    let encoding = match cpython_codec_required_name(encoding, "PyCodec_StreamReader") {
        Ok(encoding) => encoding,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let errors = match cpython_codec_optional_name(errors, "PyCodec_StreamReader") {
        Ok(errors) => errors,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let factory = match cpython_codec_lookup_attr_in_context(context, &encoding, "streamreader")
        {
            Ok(value) if !matches!(value, Value::None) => value,
            _ => {
                return match cpython_codec_stream_fallback_in_context(
                    context,
                    "StreamReader",
                    stream_value.clone(),
                    errors.as_deref(),
                ) {
                    Ok(value) => context.alloc_cpython_ptr_for_value(value),
                    Err(err) => {
                        context.set_error(err);
                        std::ptr::null_mut()
                    }
                };
            }
        };
        let mut args = vec![stream_value];
        if let Some(errors) = errors {
            args.push(Value::Str(errors));
        }
        match cpython_codec_call_callable_in_context(context, factory, args) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_StreamWriter(
    encoding: *const c_char,
    stream: *mut c_void,
    errors: *const c_char,
) -> *mut c_void {
    if stream.is_null() {
        let _ = unsafe { PyErr_BadArgument() };
        return std::ptr::null_mut();
    }
    let stream_value = match cpython_value_from_ptr(stream) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(format!("PyCodec_StreamWriter {err}"));
            return std::ptr::null_mut();
        }
    };
    let encoding = match cpython_codec_required_name(encoding, "PyCodec_StreamWriter") {
        Ok(encoding) => encoding,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let errors = match cpython_codec_optional_name(errors, "PyCodec_StreamWriter") {
        Ok(errors) => errors,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let factory = match cpython_codec_lookup_attr_in_context(context, &encoding, "streamwriter")
        {
            Ok(value) if !matches!(value, Value::None) => value,
            _ => {
                return match cpython_codec_stream_fallback_in_context(
                    context,
                    "StreamWriter",
                    stream_value.clone(),
                    errors.as_deref(),
                ) {
                    Ok(value) => context.alloc_cpython_ptr_for_value(value),
                    Err(err) => {
                        context.set_error(err);
                        std::ptr::null_mut()
                    }
                };
            }
        };
        let mut args = vec![stream_value];
        if let Some(errors) = errors {
            args.push(Value::Str(errors));
        }
        match cpython_codec_call_callable_in_context(context, factory, args) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_RegisterError(name: *const c_char, error: *mut c_void) -> i32 {
    if error.is_null() {
        let _ = unsafe { PyErr_BadArgument() };
        return -1;
    }
    let name = match cpython_codec_required_name(name, "PyCodec_RegisterError") {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    if unsafe { PyCallable_Check(error) } == 0 {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "handler must be callable");
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        context.codec_error_handlers.insert(name, error as usize);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_LookupError(name: *const c_char) -> *mut c_void {
    let name = if name.is_null() {
        "strict".to_string()
    } else {
        match cpython_codec_required_name(name, "PyCodec_LookupError") {
            Ok(name) => name,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    with_active_cpython_context_mut(|context| {
        if let Some(handler_ptr) = context.codec_error_handlers.get(&name).copied() {
            return handler_ptr as *mut c_void;
        }
        match cpython_codec_builtin_handler_ptr(context, &name) {
            Ok(handler_ptr) => handler_ptr,
            Err(_) => {
                context.set_error(format!("LookupError: unknown error handler name '{name}'"));
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_StrictErrors(exc: *mut c_void) -> *mut c_void {
    if exc.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "codec must pass exception instance",
        );
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| match cpython_codec_error_info(context, exc) {
        Ok(_) => {
            // SAFETY: raising the passed exception object mirrors CPython strict handler behavior.
            unsafe { PyErr_SetObject(std::ptr::null_mut(), exc) };
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, err);
            std::ptr::null_mut()
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_IgnoreErrors(exc: *mut c_void) -> *mut c_void {
    match with_active_cpython_context_mut(|context| cpython_codec_error_info(context, exc)) {
        Ok(Ok((_, _, end))) => cpython_codec_handler_tuple_result(String::new(), end),
        Ok(Err(err)) => {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, err);
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_ReplaceErrors(exc: *mut c_void) -> *mut c_void {
    match with_active_cpython_context_mut(|context| cpython_codec_error_info(context, exc)) {
        Ok(Ok((_, exc_name, end))) => {
            let replacement = if exc_name == "UnicodeEncodeError" {
                "?".to_string()
            } else {
                "\u{fffd}".to_string()
            };
            cpython_codec_handler_tuple_result(replacement, end)
        }
        Ok(Err(err)) => {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, err);
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_XMLCharRefReplaceErrors(exc: *mut c_void) -> *mut c_void {
    match with_active_cpython_context_mut(|context| cpython_codec_error_info(context, exc)) {
        Ok(Ok((_, _, end))) => cpython_codec_handler_tuple_result("&#xfffd;".to_string(), end),
        Ok(Err(err)) => {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, err);
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_BackslashReplaceErrors(exc: *mut c_void) -> *mut c_void {
    match with_active_cpython_context_mut(|context| cpython_codec_error_info(context, exc)) {
        Ok(Ok((_, _, end))) => cpython_codec_handler_tuple_result("\\x3f".to_string(), end),
        Ok(Err(err)) => {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, err);
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_NameReplaceErrors(exc: *mut c_void) -> *mut c_void {
    match with_active_cpython_context_mut(|context| cpython_codec_error_info(context, exc)) {
        Ok(Ok((_, _, end))) => {
            cpython_codec_handler_tuple_result("\\N{REPLACEMENT CHARACTER}".to_string(), end)
        }
        Ok(Err(err)) => {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, err);
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}
