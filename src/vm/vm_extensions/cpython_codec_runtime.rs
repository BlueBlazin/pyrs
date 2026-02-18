use std::collections::HashMap;
use std::ffi::{c_char, c_void};

use crate::runtime::{BuiltinFunction, Object, Value};

use super::{
    CpythonMethodDef, METH_O, ModuleCapiContext, ObjRef, PyCodec_BackslashReplaceErrors,
    PyCodec_IgnoreErrors, PyCodec_NameReplaceErrors, PyCodec_ReplaceErrors, PyCodec_StrictErrors,
    PyCodec_XMLCharRefReplaceErrors, c_name_to_string, cpython_call_internal_in_context,
    cpython_getattr_in_context, cpython_set_error, value_to_int, with_active_cpython_context_mut,
};

pub(in crate::vm::vm_extensions) fn cpython_codec_required_name(
    name: *const c_char,
    api_name: &str,
) -> Result<String, String> {
    // SAFETY: C-API caller provides NUL-terminated string for non-null pointers.
    unsafe { c_name_to_string(name) }.map_err(|err| format!("{api_name} {err}"))
}

pub(in crate::vm::vm_extensions) fn cpython_codec_optional_name(
    name: *const c_char,
    api_name: &str,
) -> Result<Option<String>, String> {
    if name.is_null() {
        return Ok(None);
    }
    // SAFETY: C-API caller provides NUL-terminated string for non-null pointers.
    unsafe { c_name_to_string(name) }
        .map(Some)
        .map_err(|err| format!("{api_name} {err}"))
}

pub(in crate::vm::vm_extensions) fn cpython_codec_lookup_info_in_context(
    context: &mut ModuleCapiContext,
    encoding: &str,
) -> Result<Value, String> {
    cpython_call_internal_in_context(
        context,
        Value::Builtin(BuiltinFunction::CodecsLookup),
        vec![Value::Str(encoding.to_string())],
        HashMap::new(),
    )
}

pub(in crate::vm::vm_extensions) fn cpython_codec_lookup_attr_in_context(
    context: &mut ModuleCapiContext,
    encoding: &str,
    attr_name: &str,
) -> Result<Value, String> {
    let codec_info = cpython_codec_lookup_info_in_context(context, encoding)?;
    cpython_getattr_in_context(context, codec_info, attr_name)
}

pub(in crate::vm::vm_extensions) fn cpython_codec_call_callable_in_context(
    context: &mut ModuleCapiContext,
    callable: Value,
    args: Vec<Value>,
) -> Result<Value, String> {
    cpython_call_internal_in_context(context, callable, args, HashMap::new())
}

pub(in crate::vm::vm_extensions) fn cpython_codec_module_in_context(
    context: &mut ModuleCapiContext,
) -> Result<ObjRef, String> {
    if context.vm.is_null() {
        return Err("missing VM context for codecs module".to_string());
    }
    // SAFETY: VM pointer is valid for active context lifetime.
    let vm = unsafe { &mut *context.vm };
    if !vm.modules.contains_key("codecs") {
        vm.import_module("codecs").map_err(|err| err.message)?;
    }
    vm.modules
        .get("codecs")
        .cloned()
        .ok_or_else(|| "codecs module unavailable".to_string())
}

pub(in crate::vm::vm_extensions) fn cpython_codec_stream_fallback_in_context(
    context: &mut ModuleCapiContext,
    class_name: &str,
    stream: Value,
    errors: Option<&str>,
) -> Result<Value, String> {
    let codec_module = cpython_codec_module_in_context(context)?;
    let class_value = {
        let Object::Module(module_data) = &*codec_module.kind() else {
            return Err("invalid codecs module object".to_string());
        };
        module_data
            .globals
            .get(class_name)
            .cloned()
            .ok_or_else(|| format!("codecs.{class_name} unavailable"))?
    };
    let instance =
        cpython_call_internal_in_context(context, class_value, Vec::new(), HashMap::new())?;
    if let Value::Instance(instance_obj) = &instance
        && let Object::Instance(instance_data) = &mut *instance_obj.kind_mut()
    {
        instance_data
            .attrs
            .insert("stream".to_string(), stream.clone());
        if let Some(errors) = errors {
            instance_data
                .attrs
                .insert("errors".to_string(), Value::Str(errors.to_string()));
        }
    }
    Ok(instance)
}

pub(in crate::vm::vm_extensions) fn cpython_codec_exception_type_name_for_value(
    context: &mut ModuleCapiContext,
    value: &Value,
) -> Option<String> {
    match value {
        Value::Exception(err) => Some(err.name.clone()),
        Value::Instance(instance) => {
            if context.vm.is_null() {
                return None;
            }
            // SAFETY: VM pointer is valid for active context lifetime.
            let vm = unsafe { &*context.vm };
            vm.exception_class_name_for_instance(instance)
        }
        _ => None,
    }
}

pub(in crate::vm::vm_extensions) fn cpython_codec_exception_end_for_value(value: &Value) -> i64 {
    match value {
        Value::Exception(err) => {
            let attrs = err.attrs.borrow();
            attrs
                .get("end")
                .cloned()
                .and_then(|value| value_to_int(value).ok())
                .unwrap_or(0)
        }
        Value::Instance(instance) => {
            let Object::Instance(instance_data) = &*instance.kind() else {
                return 0;
            };
            instance_data
                .attrs
                .get("end")
                .cloned()
                .and_then(|value| value_to_int(value).ok())
                .unwrap_or(0)
        }
        _ => 0,
    }
}

pub(in crate::vm::vm_extensions) fn cpython_codec_error_info(
    context: &mut ModuleCapiContext,
    exc: *mut c_void,
) -> Result<(Value, String, i64), String> {
    if exc.is_null() {
        return Err("codec must pass exception instance".to_string());
    }
    let Some(value) = context.cpython_value_from_ptr_or_proxy(exc) else {
        return Err("codec must pass exception instance".to_string());
    };
    let Some(type_name) = cpython_codec_exception_type_name_for_value(context, &value) else {
        return Err(format!(
            "don't know how to handle {} in error callback",
            if context.vm.is_null() {
                "object".to_string()
            } else {
                // SAFETY: VM pointer is valid for active context lifetime.
                unsafe { (&*context.vm).value_type_name_for_error(&value) }
            }
        ));
    };
    let end = cpython_codec_exception_end_for_value(&value);
    Ok((value, type_name, end))
}

pub(in crate::vm::vm_extensions) fn cpython_codec_handler_tuple_result(
    replacement: String,
    end: i64,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("missing VM context for codec error handler");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let result = vm
            .heap
            .alloc_tuple(vec![Value::Str(replacement), Value::Int(end)]);
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

unsafe extern "C" fn cpython_codec_cfunc_strict(
    _self: *mut c_void,
    exc: *mut c_void,
) -> *mut c_void {
    unsafe { PyCodec_StrictErrors(exc) }
}

unsafe extern "C" fn cpython_codec_cfunc_ignore(
    _self: *mut c_void,
    exc: *mut c_void,
) -> *mut c_void {
    unsafe { PyCodec_IgnoreErrors(exc) }
}

unsafe extern "C" fn cpython_codec_cfunc_replace(
    _self: *mut c_void,
    exc: *mut c_void,
) -> *mut c_void {
    unsafe { PyCodec_ReplaceErrors(exc) }
}

unsafe extern "C" fn cpython_codec_cfunc_xmlcharrefreplace(
    _self: *mut c_void,
    exc: *mut c_void,
) -> *mut c_void {
    unsafe { PyCodec_XMLCharRefReplaceErrors(exc) }
}

unsafe extern "C" fn cpython_codec_cfunc_backslashreplace(
    _self: *mut c_void,
    exc: *mut c_void,
) -> *mut c_void {
    unsafe { PyCodec_BackslashReplaceErrors(exc) }
}

unsafe extern "C" fn cpython_codec_cfunc_namereplace(
    _self: *mut c_void,
    exc: *mut c_void,
) -> *mut c_void {
    unsafe { PyCodec_NameReplaceErrors(exc) }
}

static mut PYCODEC_STRICT_ERRORS_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: c"strict".as_ptr(),
    ml_meth: Some(cpython_codec_cfunc_strict),
    ml_flags: METH_O,
    ml_doc: c"PyCodec strict error handler".as_ptr(),
};

static mut PYCODEC_IGNORE_ERRORS_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: c"ignore".as_ptr(),
    ml_meth: Some(cpython_codec_cfunc_ignore),
    ml_flags: METH_O,
    ml_doc: c"PyCodec ignore error handler".as_ptr(),
};

static mut PYCODEC_REPLACE_ERRORS_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: c"replace".as_ptr(),
    ml_meth: Some(cpython_codec_cfunc_replace),
    ml_flags: METH_O,
    ml_doc: c"PyCodec replace error handler".as_ptr(),
};

static mut PYCODEC_XMLCHARREFREPLACE_ERRORS_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: c"xmlcharrefreplace".as_ptr(),
    ml_meth: Some(cpython_codec_cfunc_xmlcharrefreplace),
    ml_flags: METH_O,
    ml_doc: c"PyCodec xmlcharrefreplace error handler".as_ptr(),
};

static mut PYCODEC_BACKSLASHREPLACE_ERRORS_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: c"backslashreplace".as_ptr(),
    ml_meth: Some(cpython_codec_cfunc_backslashreplace),
    ml_flags: METH_O,
    ml_doc: c"PyCodec backslashreplace error handler".as_ptr(),
};

static mut PYCODEC_NAMEREPLACE_ERRORS_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: c"namereplace".as_ptr(),
    ml_meth: Some(cpython_codec_cfunc_namereplace),
    ml_flags: METH_O,
    ml_doc: c"PyCodec namereplace error handler".as_ptr(),
};

pub(in crate::vm::vm_extensions) fn cpython_codec_builtin_handler_method_def(
    name: &str,
) -> Option<*mut CpythonMethodDef> {
    match name {
        "strict" => Some(std::ptr::addr_of_mut!(PYCODEC_STRICT_ERRORS_METHOD_DEF)),
        "ignore" => Some(std::ptr::addr_of_mut!(PYCODEC_IGNORE_ERRORS_METHOD_DEF)),
        "replace" => Some(std::ptr::addr_of_mut!(PYCODEC_REPLACE_ERRORS_METHOD_DEF)),
        "xmlcharrefreplace" => Some(std::ptr::addr_of_mut!(
            PYCODEC_XMLCHARREFREPLACE_ERRORS_METHOD_DEF
        )),
        "backslashreplace" => Some(std::ptr::addr_of_mut!(
            PYCODEC_BACKSLASHREPLACE_ERRORS_METHOD_DEF
        )),
        "namereplace" => Some(std::ptr::addr_of_mut!(
            PYCODEC_NAMEREPLACE_ERRORS_METHOD_DEF
        )),
        _ => None,
    }
}

pub(in crate::vm::vm_extensions) fn cpython_codec_builtin_handler_ptr(
    context: &mut ModuleCapiContext,
    name: &str,
) -> Result<*mut c_void, String> {
    let Some(method_def) = cpython_codec_builtin_handler_method_def(name) else {
        return Err(format!("unknown built-in codec error handler '{name}'"));
    };
    let ptr = context.alloc_cpython_method_cfunction_ptr(
        method_def,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    );
    if ptr.is_null() {
        return Err(format!(
            "failed to allocate codec error handler callable for '{name}'"
        ));
    }
    Ok(ptr)
}
