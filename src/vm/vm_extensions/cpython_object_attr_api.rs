use std::backtrace::Backtrace;
use std::ffi::{CString, c_char, c_void};

use crate::runtime::{Object, Value};

use super::{
    _Py_NoneStruct, BuiltinFunction, CPY_PROXY_PTR_ATTR, CpythonObjectHead, CpythonTypeObject,
    ModuleCapiContext, Py_DecRef, Py_IncRef, PyErr_BadInternalCall, PyErr_Clear,
    PyErr_ExceptionMatches, PyErr_Occurred, PyExc_AttributeError, PyExc_TypeError,
    PyObject_DelItem, PyObject_IsInstance, c_name_to_string, cpython_call_builtin,
    cpython_error_message_indicates_missing_attribute, cpython_is_reduce_probe_name,
    cpython_new_ptr_for_value, cpython_set_error, cpython_set_typed_error,
    cpython_trace_numpy_reduce_enabled, cpython_value_debug_tag, cpython_value_from_ptr,
    cpython_value_from_ptr_or_proxy, with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAttrString(
    object: *mut c_void,
    name: *const c_char,
) -> *mut c_void {
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let trace_reduce_attr =
        cpython_trace_numpy_reduce_enabled() && cpython_is_reduce_probe_name(&name);
    let trace_numpy_dtype_attr =
        std::env::var_os("PYRS_TRACE_NUMPY_DTYPE_ATTR").is_some() && name == "dtype";
    if trace_numpy_dtype_attr {
        eprintln!(
            "[numpy-dtype-attr] PyObject_GetAttrString object={:p}",
            object
        );
    }
    if trace_reduce_attr {
        eprintln!(
            "[numpy-reduce] PyObject_GetAttrString object={:p} attr={}",
            object, name
        );
    }
    let trace_numpy_attr = std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some()
        && matches!(
            name.as_str(),
            "__array_finalize__" | "__array_ufunc__" | "__array_function__" | "base" | "BoolDType"
        );
    let trace_seed_attrs = std::env::var_os("PYRS_TRACE_NUMPY_SEED_ATTRS").is_some()
        && matches!(
            name.as_str(),
            "BitGenerator" | "SeedSequence" | "SeedlessSeedSequence" | "generate_state"
        );
    let trace_getattr_slots = std::env::var_os("PYRS_TRACE_GETATTR_SLOTS").is_some();
    let trace_proxy_getattr = std::env::var_os("PYRS_TRACE_PROXY_GETATTR").is_some()
        && matches!(name.as_str(), "__repr__" | "__str__");
    let trace_dot_getattr =
        std::env::var_os("PYRS_TRACE_PROXY_GETATTR_DOT").is_some() && name == "dot";
    let trace_generate_state =
        std::env::var_os("PYRS_TRACE_GETATTR_GENERATE_STATE").is_some() && name == "generate_state";
    if trace_generate_state {
        let none_ptr = (&raw mut _Py_NoneStruct).cast::<c_void>();
        let target_kind = with_active_cpython_context_mut(|context| {
            context
                .cpython_value_from_ptr_or_proxy(object)
                .map(|value| match value {
                    Value::None => "None".to_string(),
                    Value::Class(_) => "Class".to_string(),
                    Value::Instance(_) => "Instance".to_string(),
                    Value::Builtin(_) => "Builtin".to_string(),
                    Value::Module(_) => "Module".to_string(),
                    Value::Function(_) => "Function".to_string(),
                    Value::BoundMethod(_) => "BoundMethod".to_string(),
                    _ => "Other".to_string(),
                })
                .unwrap_or_else(|| "<unresolved>".to_string())
        })
        .unwrap_or_else(|_| "<no-context>".to_string());
        eprintln!(
            "[cpy-getattr-generate-state] object={:p} is_none_ptr={} target_kind={}",
            object,
            object == none_ptr,
            target_kind
        );
    }
    if !object.is_null() {
        let native_result = with_active_cpython_context_mut(|context| {
            const MIN_VALID_PTR: usize = 0x1_0000_0000;
            if (object as usize) < MIN_VALID_PTR {
                return None;
            }
            let is_proxy_trace = name == "__array_finalize__"
                && std::env::var_os("PYRS_TRACE_CPY_PROXY_PTRS").is_some();
            if (object as usize) % std::mem::align_of::<CpythonObjectHead>() != 0 {
                if is_proxy_trace {
                    eprintln!(
                        "[cpy-proxy] native getattr skip: unaligned object_ptr={:p}",
                        object
                    );
                }
                return None;
            }
            let is_owned = context.owns_cpython_allocation_ptr(object);
            let is_known_compat = context.cpython_handle_from_ptr(object).is_some();
            let is_type_object = super::cpython_is_type_object_ptr(object);
            if is_proxy_trace {
                eprintln!(
                    "[cpy-proxy] native getattr check object_ptr={:p} owned={} known_compat={} is_type={}",
                    object, is_owned, is_known_compat, is_type_object
                );
            }
            if is_known_compat && is_owned && !is_type_object {
                return None;
            }
            // SAFETY: object pointer comes from extension code; type pointer access mirrors CPython.
            let type_ptr = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if is_proxy_trace {
                eprintln!("[cpy-proxy] native getattr type_ptr={:p}", type_ptr);
            }
            if type_ptr.is_null() {
                return None;
            }
            if (type_ptr as usize) < MIN_VALID_PTR {
                return None;
            }
            if (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0 {
                if is_proxy_trace {
                    eprintln!(
                        "[cpy-proxy] native getattr skip: unaligned type_ptr={:p}",
                        type_ptr
                    );
                }
                return None;
            }
            if !is_owned && is_proxy_trace {
                eprintln!(
                    "[cpy-proxy] native getattr external object_ptr={:p} attempting slot dispatch",
                    object
                );
            }
            // SAFETY: `type_ptr` is non-null and points to a CpythonTypeObject-compatible header.
            let tp_getattro = unsafe { (*type_ptr).tp_getattro };
            if is_proxy_trace {
                eprintln!(
                    "[cpy-proxy] native getattr slots tp_getattro={:p}",
                    tp_getattro
                );
            }
            if !tp_getattro.is_null() {
                // For foreign objects, call tp_getattro even when it resolves to
                // generic object attribute lookup so CPython instance-dict semantics
                // (e.g. module attrs like numpy.dot) stay authoritative.
                let is_generic_getattro = tp_getattro == PyObject_GetAttr as *mut c_void
                    || tp_getattro == PyObject_GenericGetAttr as *mut c_void;
                if is_owned && is_generic_getattro {
                    // Keep owned-compat objects on the internal fallback path to avoid
                    // recursive/self-referential generic getattr behavior.
                } else {
                if trace_getattr_slots {
                    eprintln!(
                        "[cpy-getattr-slot] branch=tp_getattro object={:p} attr={} type={:p} slot={:p} owned={} known={} generic={}",
                        object,
                        name,
                        type_ptr,
                        tp_getattro,
                        is_owned,
                        is_known_compat,
                        is_generic_getattro
                    );
                }
                if trace_dot_getattr {
                    eprintln!(
                        "[proxy-getattr-dot] branch=tp_getattro object={:p} is_owned={} is_known_compat={} tp_getattro={:p} generic={}",
                        object, is_owned, is_known_compat, tp_getattro, is_generic_getattro
                    );
                }
                if trace_proxy_getattr {
                    eprintln!(
                        "[proxy-getattr] branch=tp_getattro object={:p} attr={} tp_getattro={:p}",
                        object, name, tp_getattro
                    );
                }
                let name_ptr = context.alloc_cpython_ptr_for_value(Value::Str(name.clone()));
                if name_ptr.is_null() {
                    return Some(std::ptr::null_mut());
                }
                let getattro: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                    // SAFETY: tp_getattro follows the CPython `PyObject* (*)(PyObject*,PyObject*)` ABI.
                    unsafe { std::mem::transmute(tp_getattro) };
                return Some(unsafe { getattro(object, name_ptr) });
                }
            }
            // SAFETY: `type_ptr` is non-null and points to a CpythonTypeObject-compatible header.
            let tp_getattr = unsafe { (*type_ptr).tp_getattr };
            if is_proxy_trace {
                eprintln!(
                    "[cpy-proxy] native getattr slots tp_getattr={:p}",
                    tp_getattr
                );
            }
            if !tp_getattr.is_null() {
                if trace_getattr_slots {
                    eprintln!(
                        "[cpy-getattr-slot] branch=tp_getattr object={:p} attr={} type={:p} slot={:p} owned={} known={}",
                        object, name, type_ptr, tp_getattr, is_owned, is_known_compat
                    );
                }
                if trace_dot_getattr {
                    eprintln!(
                        "[proxy-getattr-dot] branch=tp_getattr object={:p} is_owned={} is_known_compat={} tp_getattr={:p}",
                        object, is_owned, is_known_compat, tp_getattr
                    );
                }
                if trace_proxy_getattr {
                    eprintln!(
                        "[proxy-getattr] branch=tp_getattr object={:p} attr={} tp_getattr={:p}",
                        object, name, tp_getattr
                    );
                }
                let name_cstr = match context.scratch_c_string_ptr(&name) {
                    Ok(ptr) => ptr,
                    Err(err) => {
                        context.set_error(err);
                        return Some(std::ptr::null_mut());
                    }
                };
                let getattr: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
                    // SAFETY: tp_getattr follows the CPython `char*` getattr ABI.
                    unsafe { std::mem::transmute(tp_getattr) };
                return Some(unsafe { getattr(object, name_cstr) });
            }
            if let Some(result) = context.lookup_type_attr_via_tp_dict(object, &name) {
                if trace_dot_getattr {
                    eprintln!(
                        "[proxy-getattr-dot] branch=tp_dict object={:p} is_owned={} is_known_compat={} result={:p}",
                        object, is_owned, is_known_compat, result
                    );
                }
                if trace_proxy_getattr {
                    eprintln!(
                        "[proxy-getattr] branch=tp_dict object={:p} attr={} result={:p}",
                        object, name, result
                    );
                }
                if is_proxy_trace {
                    eprintln!(
                        "[cpy-proxy] native getattr tp_dict hit object_ptr={:p} result_ptr={:p}",
                        object, result
                    );
                }
                return Some(result);
            }
            if is_proxy_trace {
                eprintln!("[cpy-proxy] native getattr no native path hit; falling back");
            }
            if trace_dot_getattr {
                eprintln!(
                    "[proxy-getattr-dot] branch=fallback object={:p} is_owned={} is_known_compat={}",
                    object, is_owned, is_known_compat
                );
            }
            if trace_proxy_getattr {
                eprintln!(
                    "[proxy-getattr] branch=fallback object={:p} attr={}",
                    object, name
                );
            }
            None
        })
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            Some(std::ptr::null_mut())
        });
        if let Some(result) = native_result {
            if trace_seed_attrs {
                const MIN_VALID_PTR: usize = 0x1_0000_0000;
                let valid_result_ptr = !result.is_null()
                    && (result as usize) >= MIN_VALID_PTR
                    && (result as usize) % std::mem::align_of::<usize>() == 0;
                let (result_type_ptr, result_type_name) = if valid_result_ptr {
                    // SAFETY: pointer provenance guarded by null/min-address/alignment checks.
                    unsafe {
                        let type_ptr = result
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                            .unwrap_or(std::ptr::null_mut());
                        let valid_type_ptr = !type_ptr.is_null()
                            && (type_ptr as usize) >= MIN_VALID_PTR
                            && (type_ptr as usize) % std::mem::align_of::<usize>() == 0;
                        let type_name = if !valid_type_ptr {
                            "<invalid-type>".to_string()
                        } else {
                            c_name_to_string((*type_ptr).tp_name)
                                .unwrap_or_else(|_| "<invalid>".to_string())
                        };
                        (type_ptr, type_name)
                    }
                } else {
                    (std::ptr::null_mut(), "<invalid-ptr>".to_string())
                };
                eprintln!(
                    "[numpy-seed-attr] source=native object={:p} attr={} result_ptr={:p} type={:p} type_name={}",
                    object, name, result, result_type_ptr, result_type_name
                );
            }
            if trace_proxy_getattr {
                eprintln!(
                    "[proxy-getattr] native-result object={:p} attr={} result={:p}",
                    object, name, result
                );
            }
            if trace_reduce_attr {
                eprintln!(
                    "[numpy-reduce] PyObject_GetAttrString native-result object={:p} attr={} result={:p}",
                    object, name, result
                );
            }
            if trace_numpy_attr {
                eprintln!(
                    "[numpy-init] PyObject_GetAttrString object={:p} name={} native_result={:p}",
                    object, name, result
                );
            }
            return result;
        }
    }
    let object_value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            if let Ok(Some(attr_ptr)) = with_active_cpython_context_mut(|context| {
                context
                    .lookup_type_attr_via_tp_dict(object, &name)
                    .filter(|ptr| !ptr.is_null())
            }) {
                return attr_ptr;
            }
            let (type_ptr, tp_getattro, tp_getattr, owned) =
                with_active_cpython_context_mut(|context| {
                    const MIN_VALID_PTR: usize = 0x1_0000_0000;
                    // SAFETY: best-effort diagnostics for unknown-pointer failures.
                    let type_ptr = unsafe {
                        object
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                            .unwrap_or(std::ptr::null_mut())
                    };
                    // SAFETY: type_ptr is either null or points to a type object header.
                    let (tp_getattro, tp_getattr) = if type_ptr.is_null()
                        || (type_ptr as usize) < MIN_VALID_PTR
                        || (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0
                    {
                        (std::ptr::null_mut(), std::ptr::null_mut())
                    } else {
                        unsafe { ((*type_ptr).tp_getattro, (*type_ptr).tp_getattr) }
                    };
                    (
                        type_ptr,
                        tp_getattro,
                        tp_getattr,
                        context.owns_cpython_allocation_ptr(object),
                    )
                })
                .unwrap_or((
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    false,
                ));
            cpython_set_error(format!(
                "{err} (PyObject_GetAttrString object={:p} attr={} owned={} type_ptr={:p} tp_getattro={:p} tp_getattr={:p})",
                object, name, owned, type_ptr, tp_getattro, tp_getattr
            ));
            return std::ptr::null_mut();
        }
    };
    if trace_numpy_dtype_attr
        && let Value::Module(module_obj) = &object_value
        && let Object::Module(module_data) = &*module_obj.kind()
    {
        let mut keys = module_data.globals.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        eprintln!(
            "[numpy-dtype-attr] module={} has_dtype={} globals_len={} keys={:?}",
            module_data.name,
            module_data.globals.contains_key("dtype"),
            module_data.globals.len(),
            keys
        );
    }
    if std::env::var_os("PYRS_TRACE_CPY_API").is_some() {
        let tag = cpython_value_debug_tag(&object_value);
        let (owned, known) = with_active_cpython_context_mut(|context| {
            (
                context.owns_cpython_allocation_ptr(object),
                context.cpython_handle_from_ptr(object).is_some(),
            )
        })
        .unwrap_or((false, false));
        eprintln!(
            "[cpy-api] PyObject_GetAttrString object_ptr={:p} object={} attr={} owned={} known={}",
            object, tag, name, owned, known
        );
    }
    if name == "__array_finalize__" && std::env::var_os("PYRS_TRACE_CPY_PROXY_PTRS").is_some() {
        match &object_value {
            Value::Class(class_obj) => {
                if let Object::Class(class_data) = &*class_obj.kind() {
                    eprintln!(
                        "[cpy-proxy] getattr __array_finalize__ object_ptr={:p} class={} id={} raw_ptr_attr={:?}",
                        object,
                        class_data.name,
                        class_obj.id(),
                        class_data.attrs.get(CPY_PROXY_PTR_ATTR)
                    );
                }
            }
            other => {
                eprintln!(
                    "[cpy-proxy] getattr __array_finalize__ non-class object_ptr={:p} tag={}",
                    object,
                    cpython_value_debug_tag(other)
                );
            }
        }
    }
    let object_value_for_debug = object_value.clone();
    match cpython_call_builtin(
        BuiltinFunction::GetAttr,
        vec![object_value, Value::Str(name.clone())],
    ) {
        Ok(value) => {
            let ptr = cpython_new_ptr_for_value(value);
            if trace_reduce_attr {
                eprintln!(
                    "[numpy-reduce] PyObject_GetAttrString builtin-result object={:p} attr={} result={:p}",
                    object, name, ptr
                );
            }
            if trace_numpy_attr {
                eprintln!(
                    "[numpy-init] PyObject_GetAttrString object={:p} name={} builtin_result={:p}",
                    object, name, ptr
                );
            }
            ptr
        }
        Err(err) => {
            if std::env::var_os("PYRS_TRACE_NONE_NAME_GETATTR").is_some()
                && name == "name"
                && matches!(object_value_for_debug, Value::None)
            {
                eprintln!(
                    "[cpy-none-name-getattr] object={:p} err={} bt={:?}",
                    object,
                    err,
                    Backtrace::force_capture()
                );
            }
            if trace_reduce_attr {
                eprintln!(
                    "[numpy-reduce] PyObject_GetAttrString error object={:p} attr={} err={}",
                    object, name, err
                );
            }
            if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some()
                && err.contains("attribute access unsupported type")
            {
                eprintln!(
                    "[cpy-attr-debug] getattr unsupported object_ptr={:p} object_tag={} attr={}",
                    object,
                    cpython_value_debug_tag(&object_value_for_debug),
                    name
                );
            }
            if std::env::var_os("PYRS_TRACE_CPY_ATTR_ERRORS").is_some() {
                eprintln!(
                    "[cpy-attr-error] getattr object_ptr={:p} object_tag={} attr={} err={}",
                    object,
                    cpython_value_debug_tag(&object_value_for_debug),
                    name,
                    err
                );
            }
            cpython_set_error(err.clone());
            if trace_numpy_attr {
                eprintln!(
                    "[numpy-init] PyObject_GetAttrString object={:p} name={} error={}",
                    object, name, err
                );
            }
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAttr(object: *mut c_void, name: *mut c_void) -> *mut c_void {
    let trace_generate_state = std::env::var_os("PYRS_TRACE_GETATTR_GENERATE_STATE").is_some()
        && with_active_cpython_context_mut(|context| {
            context
                .cpython_value_from_ptr_or_proxy(name)
                .and_then(|value| match value {
                    Value::Str(text) => Some(text == "generate_state"),
                    _ => None,
                })
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if trace_generate_state {
        let none_ptr = (&raw mut _Py_NoneStruct).cast::<c_void>();
        let target_kind = with_active_cpython_context_mut(|context| {
            context
                .cpython_value_from_ptr_or_proxy(object)
                .map(|value| match value {
                    Value::None => "None".to_string(),
                    Value::Class(_) => "Class".to_string(),
                    Value::Instance(_) => "Instance".to_string(),
                    Value::Builtin(_) => "Builtin".to_string(),
                    Value::Module(_) => "Module".to_string(),
                    Value::Function(_) => "Function".to_string(),
                    Value::BoundMethod(_) => "BoundMethod".to_string(),
                    _ => "Other".to_string(),
                })
                .unwrap_or_else(|| "<unresolved>".to_string())
        })
        .unwrap_or_else(|_| "<no-context>".to_string());
        eprintln!(
            "[cpy-getattr] attr=generate_state object={:p} name_ptr={:p} is_none_ptr={} target_kind={}",
            object,
            name,
            object == none_ptr,
            target_kind
        );
    }
    let trace_reduce_attr_name = if cpython_trace_numpy_reduce_enabled() {
        with_active_cpython_context_mut(|context| {
            context
                .cpython_value_from_ptr_or_proxy(name)
                .and_then(|value| match value {
                    Value::Str(text) if cpython_is_reduce_probe_name(&text) => Some(text),
                    _ => None,
                })
        })
        .ok()
        .flatten()
    } else {
        None
    };
    if let Some(attr_name) = trace_reduce_attr_name.as_deref() {
        eprintln!(
            "[numpy-reduce] PyObject_GetAttr object={:p} name_ptr={:p} attr={}",
            object, name, attr_name
        );
    }
    if !object.is_null() {
        let native_result = with_active_cpython_context_mut(|context| {
            const MIN_VALID_PTR: usize = 0x1_0000_0000;
            if (object as usize) < MIN_VALID_PTR {
                return None;
            }
            if (object as usize) % std::mem::align_of::<CpythonObjectHead>() != 0 {
                return None;
            }
            let is_known_compat = context.cpython_handle_from_ptr(object).is_some();
            let is_owned = context.owns_cpython_allocation_ptr(object);
            let is_type_object = super::cpython_is_type_object_ptr(object);
            if is_known_compat && is_owned && !is_type_object {
                return None;
            }
            // SAFETY: object pointer comes from extension code; type pointer access mirrors CPython.
            let type_ptr = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if type_ptr.is_null() {
                return None;
            }
            if (type_ptr as usize) < MIN_VALID_PTR {
                return None;
            }
            if (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0 {
                return None;
            }
            let attr_name =
                context
                    .cpython_value_from_ptr(name)
                    .and_then(|candidate| match candidate {
                        Value::Str(text) => Some(text),
                        _ => None,
                    });
            // SAFETY: `type_ptr` is non-null and points to a CpythonTypeObject-compatible header.
            let tp_getattro = unsafe { (*type_ptr).tp_getattro };
            if tp_getattro.is_null()
                || tp_getattro == PyObject_GetAttr as *mut c_void
                || tp_getattro == PyObject_GenericGetAttr as *mut c_void
            {
                if let Some(attr_name) = attr_name.as_ref()
                    && let Some(result) = context.lookup_type_attr_via_tp_dict(object, attr_name)
                {
                    return Some(result);
                }
                return None;
            }
            let getattro: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                // SAFETY: tp_getattro follows the CPython `PyObject* (*)(PyObject*,PyObject*)` ABI.
                unsafe { std::mem::transmute(tp_getattro) };
            Some(unsafe { getattro(object, name) })
        })
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            Some(std::ptr::null_mut())
        });
        if let Some(result) = native_result {
            if let Some(attr_name) = trace_reduce_attr_name.as_deref() {
                eprintln!(
                    "[numpy-reduce] PyObject_GetAttr native-result object={:p} attr={} result={:p}",
                    object, attr_name, result
                );
            }
            return result;
        }
    }
    let object_value = match cpython_value_from_ptr_or_proxy(object) {
        Ok(value) => value,
        Err(err) => {
            let (type_ptr, tp_getattro, owned) = with_active_cpython_context_mut(|context| {
                const MIN_VALID_PTR: usize = 0x1_0000_0000;
                // SAFETY: best-effort diagnostics for unknown-pointer failures.
                let type_ptr = unsafe {
                    object
                        .cast::<CpythonObjectHead>()
                        .as_ref()
                        .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                        .unwrap_or(std::ptr::null_mut())
                };
                let tp_getattro = if type_ptr.is_null()
                    || (type_ptr as usize) < MIN_VALID_PTR
                    || (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0
                {
                    std::ptr::null_mut()
                } else {
                    // SAFETY: type_ptr is non-null and points to a type object header.
                    unsafe { (*type_ptr).tp_getattro }
                };
                (
                    type_ptr,
                    tp_getattro,
                    context.owns_cpython_allocation_ptr(object),
                )
            })
            .unwrap_or((std::ptr::null_mut(), std::ptr::null_mut(), false));
            cpython_set_error(format!(
                "{err} (PyObject_GetAttr object={:p} name_ptr={:p} owned={} type_ptr={:p} tp_getattro={:p})",
                object, name, owned, type_ptr, tp_getattro
            ));
            if let Some(attr_name) = trace_reduce_attr_name.as_deref() {
                eprintln!(
                    "[numpy-reduce] PyObject_GetAttr error object={:p} attr={} err={}",
                    object, attr_name, err
                );
            }
            return std::ptr::null_mut();
        }
    };
    let name_value = match cpython_value_from_ptr(name) {
        Ok(value) => value,
        Err(err) => {
            let native_fallback = with_active_cpython_context_mut(|context| {
                const MIN_VALID_PTR: usize = 0x1_0000_0000;
                if object.is_null() || name.is_null() {
                    return None;
                }
                if (object as usize) < MIN_VALID_PTR
                    || (object as usize) % std::mem::align_of::<CpythonObjectHead>() != 0
                {
                    return None;
                }
                // SAFETY: best-effort C-ABI slot lookup using object header.
                let type_ptr = unsafe {
                    object
                        .cast::<CpythonObjectHead>()
                        .as_ref()
                        .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                        .unwrap_or(std::ptr::null_mut())
                };
                if type_ptr.is_null()
                    || (type_ptr as usize) < MIN_VALID_PTR
                    || (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0
                {
                    return None;
                }
                // SAFETY: validated `type_ptr` access.
                let tp_getattro = unsafe { (*type_ptr).tp_getattro };
                if tp_getattro.is_null()
                    || tp_getattro == PyObject_GetAttr as *mut c_void
                    || tp_getattro == PyObject_GenericGetAttr as *mut c_void
                {
                    return None;
                }
                if std::env::var_os("PYRS_TRACE_CPY_API").is_some() {
                    eprintln!(
                        "[cpy-api] PyObject_GetAttr native-fallback object={:p} name={:p} tp_getattro={:p}",
                        object, name, tp_getattro
                    );
                }
                let getattro: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                    // SAFETY: `tp_getattro` follows the CPython getattro ABI.
                    unsafe { std::mem::transmute(tp_getattro) };
                // Keep context pointer in scope for fallback call.
                let _ = context;
                Some(unsafe { getattro(object, name) })
            })
            .unwrap_or_else(|fallback_err| {
                cpython_set_error(fallback_err);
                None
            });
            if let Some(result) = native_fallback {
                return result;
            }
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::GetAttr, vec![object_value, name_value]) {
        Ok(value) => {
            let ptr = cpython_new_ptr_for_value(value);
            if let Some(attr_name) = trace_reduce_attr_name.as_deref() {
                eprintln!(
                    "[numpy-reduce] PyObject_GetAttr builtin-result object={:p} attr={} result={:p}",
                    object, attr_name, ptr
                );
            }
            ptr
        }
        Err(err) => {
            if let Some(attr_name) = trace_reduce_attr_name.as_deref() {
                eprintln!(
                    "[numpy-reduce] PyObject_GetAttr builtin-error object={:p} attr={} err={}",
                    object, attr_name, err
                );
            }
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericGetAttr(
    object: *mut c_void,
    name: *mut c_void,
) -> *mut c_void {
    unsafe { PyObject_GetAttr(object, name) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericSetAttr(
    object: *mut c_void,
    name: *mut c_void,
    value: *mut c_void,
) -> i32 {
    let object_value = match cpython_value_from_ptr_or_proxy(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let name_value = match cpython_value_from_ptr_or_proxy(name) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };

    let object_value_for_debug = object_value.clone();
    let name_value_for_debug = name_value.clone();
    let trace_common_setattr = std::env::var_os("PYRS_TRACE_PYX_CAPI").is_some()
        && matches!(
            &object_value_for_debug,
            Value::Module(module_obj)
                if matches!(
                    &*module_obj.kind(),
                    Object::Module(module_data)
                        if matches!(
                            module_data.globals.get("__name__"),
                            Some(Value::Str(name)) if name == "numpy.random._common"
                        )
                )
        )
        && !value.is_null();
    if trace_common_setattr {
        let (module_id, module_ptr) = match &object_value_for_debug {
            Value::Module(module_obj) => (module_obj.id(), object),
            _ => (0, object),
        };
        let name_debug = match &name_value_for_debug {
            Value::Str(text) => format!("Str({text})"),
            other => cpython_value_debug_tag(other),
        };
        eprintln!(
            "[pyx-capi] GenericSetAttr module=numpy.random._common id={} object={:p} name={} value_ptr={:p}",
            module_id, module_ptr, name_debug, value
        );
    }
    let result = if value.is_null() {
        cpython_call_builtin(BuiltinFunction::DelAttr, vec![object_value, name_value])
    } else {
        let attr_value = match cpython_value_from_ptr_or_proxy(value) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return -1;
            }
        };
        cpython_call_builtin(
            BuiltinFunction::SetAttr,
            vec![object_value, name_value, attr_value],
        )
    };
    match result {
        Ok(_) => {
            let _ = with_active_cpython_context_mut(|context| {
                if let Value::Module(module_obj) = &object_value_for_debug
                    && let Value::Str(attr_name) = &name_value_for_debug
                {
                    if value.is_null() {
                        let _ = context.sync_module_dict_del(module_obj, attr_name);
                    } else if let Some(attr_value) = context.cpython_value_from_ptr_or_proxy(value)
                    {
                        let _ = context.sync_module_dict_set(module_obj, attr_name, &attr_value);
                    }
                }
            });
            if trace_common_setattr {
                eprintln!(
                    "[pyx-capi] GenericSetAttr module=numpy.random._common status=0 object={:p}",
                    object
                );
            }
            0
        }
        Err(err) => {
            if trace_common_setattr {
                eprintln!(
                    "[pyx-capi] GenericSetAttr module=numpy.random._common status=-1 object={:p} err={}",
                    object, err
                );
            }
            if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some()
                && err.contains("attribute assignment unsupported type")
            {
                eprintln!(
                    "[cpy-attr-debug] setattr unsupported object_ptr={:p} object_tag={} attr={}",
                    object,
                    cpython_value_debug_tag(&object_value_for_debug),
                    cpython_value_debug_tag(&name_value_for_debug)
                );
            }
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericGetDict(
    object: *mut c_void,
    _context: *mut c_void,
) -> *mut c_void {
    unsafe { PyObject_GetAttrString(object, c"__dict__".as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericSetDict(
    object: *mut c_void,
    value: *mut c_void,
    _context: *mut c_void,
) -> i32 {
    if value.is_null() {
        cpython_set_error("PyObject_GenericSetDict does not support deleting __dict__");
        return -1;
    }
    unsafe { PyObject_SetAttrString(object, c"__dict__".as_ptr(), value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_GetDictPtr(object: *mut c_void) -> *mut *mut c_void {
    with_active_cpython_context_mut(|context| {
        if object.is_null() {
            return std::ptr::null_mut();
        }
        let mut dict_ptr = unsafe { PyObject_GetAttrString(object, c"__dict__".as_ptr()) };
        if dict_ptr.is_null() {
            // If `__dict__` is not present, try materializing one and attaching it.
            unsafe { PyErr_Clear() };
            if context.vm.is_null() {
                return std::ptr::null_mut();
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *context.vm };
            dict_ptr = context.alloc_cpython_ptr_for_value(vm.heap.alloc_dict(Vec::new()));
            if dict_ptr.is_null() {
                return std::ptr::null_mut();
            }
            let status = unsafe { PyObject_SetAttrString(object, c"__dict__".as_ptr(), dict_ptr) };
            if status != 0 {
                unsafe { PyErr_Clear() };
                return std::ptr::null_mut();
            }
        }
        let slot = context.alloc_aux_buffer(std::mem::size_of::<*mut c_void>());
        if slot.is_null() {
            return std::ptr::null_mut();
        }
        let slot_ptr = slot.cast::<*mut c_void>();
        // SAFETY: `slot_ptr` points to writable pointer-sized storage allocated above.
        unsafe {
            *slot_ptr = dict_ptr;
        }
        slot_ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetAttrString(
    object: *mut c_void,
    name: *const c_char,
    value: *mut c_void,
) -> i32 {
    let value_ptr = value;
    let name_text = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let trace_pyx_capi_attr =
        std::env::var_os("PYRS_TRACE_PYX_CAPI").is_some() && name_text == "__pyx_capi__";
    let trace_pybind11_attr =
        std::env::var_os("PYRS_TRACE_PYBIND11_ATTRS").is_some() && name_text.contains("__pybind11");
    if !object.is_null() {
        let trace_native_setattr = std::env::var_os("PYRS_TRACE_SETATTR_NATIVE").is_some();
        let attr_name = name_text.clone();
        let native_status = with_active_cpython_context_mut(|context| {
            const MIN_VALID_PTR: usize = 0x1_0000_0000;
            if (object as usize) < MIN_VALID_PTR {
                return None;
            }
            if (object as usize) % std::mem::align_of::<CpythonObjectHead>() != 0 {
                return None;
            }
            let is_probable_external =
                ModuleCapiContext::is_probable_external_cpython_object_ptr(object);
            if !is_probable_external {
                return None;
            }
            let is_known_compat = context.cpython_handle_from_ptr(object).is_some();
            let is_owned = context.owns_cpython_allocation_ptr(object);
            let is_type_object = super::cpython_is_type_object_ptr(object);
            // Keep owned known-compat non-type objects on builtin setattr path to avoid
            // recursive generic setattr behavior, but let type objects use native slots so
            // metatype/class dict writes (e.g. pybind11 enum __entries) stay authoritative.
            if is_known_compat && is_owned && !is_type_object {
                return None;
            }
            // SAFETY: object pointer comes from extension code; type pointer access mirrors CPython.
            let type_ptr = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if type_ptr.is_null() {
                return None;
            }
            if (type_ptr as usize) < MIN_VALID_PTR {
                return None;
            }
            if (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0 {
                return None;
            }
            // SAFETY: type pointer is non-null and follows CPython type layout.
            let tp_setattro = unsafe { (*type_ptr).tp_setattro };
            if trace_native_setattr {
                // SAFETY: guarded by type pointer checks above.
                let type_name = unsafe {
                    c_name_to_string((*type_ptr).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
                };
                eprintln!(
                    "[cpy-setattr-native] object={:p} known_compat={} owned={} probable_external={} type={:p} type_name={} tp_setattro={:p} tp_setattr={:p} value={:p}",
                    object,
                    is_known_compat,
                    is_owned,
                    is_probable_external,
                    type_ptr,
                    type_name,
                    tp_setattro,
                    unsafe { (*type_ptr).tp_setattr },
                    value,
                );
            }
            if !tp_setattro.is_null() {
                if (tp_setattro as usize) < MIN_VALID_PTR {
                    context.set_error("PyObject_SetAttrString rejected low tp_setattro pointer");
                    return Some(-1);
                }
                let attr_name_ptr =
                    context.alloc_cpython_ptr_for_value(Value::Str(attr_name.clone()));
                if attr_name_ptr.is_null() {
                    context.set_error("PyObject_SetAttrString failed to materialize attr name");
                    return Some(-1);
                }
                let setattro: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
                    // SAFETY: tp_setattro follows CPython setattro ABI.
                    unsafe { std::mem::transmute(tp_setattro) };
                return Some(unsafe { setattro(object, attr_name_ptr, value) });
            }
            // SAFETY: type pointer is non-null and follows CPython type layout.
            let tp_setattr = unsafe { (*type_ptr).tp_setattr };
            if !tp_setattr.is_null() {
                if (tp_setattr as usize) < MIN_VALID_PTR {
                    context.set_error("PyObject_SetAttrString rejected low tp_setattr pointer");
                    return Some(-1);
                }
                let c_name = match CString::new(attr_name.as_str()) {
                    Ok(name) => name,
                    Err(_) => {
                        context.set_error("attribute name contains interior NUL byte");
                        return Some(-1);
                    }
                };
                let setattr: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> i32 =
                    // SAFETY: tp_setattr follows CPython setattr ABI.
                    unsafe { std::mem::transmute(tp_setattr) };
                return Some(unsafe { setattr(object, c_name.as_ptr(), value) });
            }
            None
        })
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            Some(-1)
        });
        if let Some(status) = native_status {
            if trace_pybind11_attr {
                eprintln!(
                    "[pybind11-attr] branch=native object={:p} name={} value={:p} status={}",
                    object, name_text, value_ptr, status
                );
            }
            if trace_pyx_capi_attr {
                eprintln!(
                    "[pyx-capi] PyObject_SetAttrString native object={:p} value={:p} status={}",
                    object, value, status
                );
            }
            return status;
        }
    }
    let (object_value, value) = match with_active_cpython_context_mut(|context| {
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_SetAttrString received unknown object pointer");
            return None;
        };
        let Some(value) = context.cpython_value_from_ptr_or_proxy(value) else {
            context.set_error("PyObject_SetAttrString received unknown value pointer");
            return None;
        };
        Some((object_value, value))
    }) {
        Ok(Some(pair)) => pair,
        Ok(None) => return -1,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let object_tag = if trace_pyx_capi_attr {
        Some(cpython_value_debug_tag(&object_value))
    } else {
        None
    };
    let value_tag = if trace_pyx_capi_attr {
        Some(cpython_value_debug_tag(&value))
    } else {
        None
    };
    let object_value_for_sync = object_value.clone();
    let name_text_for_sync = name_text.clone();
    let value_for_sync = value.clone();
    match cpython_call_builtin(
        BuiltinFunction::SetAttr,
        vec![object_value, Value::Str(name_text), value],
    ) {
        Ok(_) => {
            let _ = with_active_cpython_context_mut(|context| {
                if let Value::Module(module_obj) = &object_value_for_sync {
                    let _ = context.sync_module_dict_set(
                        module_obj,
                        &name_text_for_sync,
                        &value_for_sync,
                    );
                }
            });
            if trace_pybind11_attr {
                eprintln!(
                    "[pybind11-attr] branch=builtin object={:p} name={} value={:p} status=0",
                    object, name_text_for_sync, value_ptr
                );
            }
            if trace_pyx_capi_attr {
                eprintln!(
                    "[pyx-capi] PyObject_SetAttrString builtin object={:p} value={:p} object_tag={} value_tag={} status=0",
                    object,
                    value_ptr,
                    object_tag.unwrap_or_default(),
                    value_tag.unwrap_or_default()
                );
            }
            0
        }
        Err(err) => {
            if trace_pybind11_attr {
                eprintln!(
                    "[pybind11-attr] branch=builtin object={:p} name={} value={:p} status=-1 err={}",
                    object, name_text_for_sync, value_ptr, err
                );
            }
            if trace_pyx_capi_attr {
                eprintln!(
                    "[pyx-capi] PyObject_SetAttrString builtin object={:p} value={:p} object_tag={} value_tag={} status=-1 err={}",
                    object,
                    value_ptr,
                    object_tag.unwrap_or_default(),
                    value_tag.unwrap_or_default(),
                    err
                );
            }
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetAttr(
    object: *mut c_void,
    name: *mut c_void,
    value: *mut c_void,
) -> i32 {
    if value.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let trace_pyx_capi = if std::env::var_os("PYRS_TRACE_PYX_CAPI").is_some() {
        with_active_cpython_context_mut(|context| {
            context
                .cpython_value_from_ptr_or_proxy(name)
                .and_then(|value| match value {
                    Value::Str(text) if text == "__pyx_capi__" => Some(text),
                    _ => None,
                })
        })
        .ok()
        .flatten()
        .is_some()
    } else {
        false
    };
    let status = unsafe { PyObject_GenericSetAttr(object, name, value) };
    if trace_pyx_capi {
        eprintln!(
            "[pyx-capi] PyObject_SetAttr object={:p} name={:p} value={:p} status={}",
            object, name, value, status
        );
    }
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelAttr(object: *mut c_void, name: *mut c_void) -> i32 {
    unsafe { PyObject_GenericSetAttr(object, name, std::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelAttrString(object: *mut c_void, name: *const c_char) -> i32 {
    let name_text = match unsafe { c_name_to_string(name) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let name_obj = cpython_new_ptr_for_value(Value::Str(name_text));
    if name_obj.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_DelAttr(object, name_obj) };
    unsafe { Py_DecRef(name_obj) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelItemString(object: *mut c_void, key: *const c_char) -> i32 {
    let key_text = match unsafe { c_name_to_string(key) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let key_obj = cpython_new_ptr_for_value(Value::Str(key_text));
    if key_obj.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_DelItem(object, key_obj) };
    unsafe { Py_DecRef(key_obj) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Type(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        cpython_set_error("PyObject_Type received null object");
        return std::ptr::null_mut();
    }
    let type_ptr = unsafe {
        object
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type)
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        cpython_set_error("PyObject_Type encountered object without type");
        return std::ptr::null_mut();
    }
    unsafe { Py_IncRef(type_ptr.cast()) };
    type_ptr.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_Type(object: *mut c_void) -> *mut c_void {
    unsafe { PyObject_Type(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetTypeData(
    object: *mut c_void,
    cls: *mut c_void,
) -> *mut c_void {
    if object.is_null() || cls.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let is_instance = unsafe { PyObject_IsInstance(object, cls) };
    if is_instance < 0 {
        return std::ptr::null_mut();
    }
    if is_instance == 0 {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "PyObject_GetTypeData called for unrelated type",
        );
        return std::ptr::null_mut();
    }
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_HasAttrString(object: *mut c_void, name: *const c_char) -> i32 {
    let status = unsafe { PyObject_HasAttrStringWithError(object, name) };
    if status < 0 {
        unsafe { PyErr_Clear() };
        0
    } else {
        status
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_HasAttr(object: *mut c_void, name: *mut c_void) -> i32 {
    let status = unsafe { PyObject_HasAttrWithError(object, name) };
    if status < 0 {
        unsafe { PyErr_Clear() };
        0
    } else {
        status
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_HasAttrWithError(object: *mut c_void, name: *mut c_void) -> i32 {
    let trace_enabled = cpython_trace_numpy_reduce_enabled();
    let mut trace_name: Option<String> = None;
    if trace_enabled
        && let Ok(value) = cpython_value_from_ptr(name)
        && let Value::Str(text) = value
    {
        trace_name = Some(text);
    }
    if trace_enabled {
        eprintln!(
            "[numpy-reduce] PyObject_HasAttrWithError object={:p} name_ptr={:p} attr={}",
            object,
            name,
            trace_name.as_deref().unwrap_or("<unmapped>")
        );
    }
    let attr = unsafe { PyObject_GetAttr(object, name) };
    if !attr.is_null() {
        unsafe { Py_DecRef(attr) };
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_HasAttrWithError hit object={:p} attr={}",
                object,
                trace_name.as_deref().unwrap_or("<unmapped>")
            );
        }
        return 1;
    }
    if unsafe { PyErr_Occurred() }.is_null() {
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_HasAttrWithError miss-noerr object={:p} attr={}",
                object,
                trace_name.as_deref().unwrap_or("<unmapped>")
            );
        }
        return 0;
    }
    if unsafe { PyErr_ExceptionMatches(PyExc_AttributeError) } != 0
        || cpython_error_message_indicates_missing_attribute()
    {
        unsafe { PyErr_Clear() };
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_HasAttrWithError miss object={:p} attr={}",
                object,
                trace_name.as_deref().unwrap_or("<unmapped>")
            );
        }
        return 0;
    }
    if trace_enabled {
        eprintln!(
            "[numpy-reduce] PyObject_HasAttrWithError error object={:p} attr={}",
            object,
            trace_name.as_deref().unwrap_or("<unmapped>")
        );
    }
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_HasAttrStringWithError(
    object: *mut c_void,
    name: *const c_char,
) -> i32 {
    let trace_enabled = cpython_trace_numpy_reduce_enabled();
    let trace_name = unsafe { c_name_to_string(name) }.ok();
    if trace_enabled {
        eprintln!(
            "[numpy-reduce] PyObject_HasAttrStringWithError object={:p} attr={}",
            object,
            trace_name.as_deref().unwrap_or("<invalid>")
        );
    }
    let attr = unsafe { PyObject_GetAttrString(object, name) };
    if !attr.is_null() {
        unsafe { Py_DecRef(attr) };
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_HasAttrStringWithError hit object={:p} attr={}",
                object,
                trace_name.as_deref().unwrap_or("<invalid>")
            );
        }
        return 1;
    }
    if unsafe { PyErr_Occurred() }.is_null() {
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_HasAttrStringWithError miss-noerr object={:p} attr={}",
                object,
                trace_name.as_deref().unwrap_or("<invalid>")
            );
        }
        return 0;
    }
    if unsafe { PyErr_ExceptionMatches(PyExc_AttributeError) } != 0
        || cpython_error_message_indicates_missing_attribute()
    {
        unsafe { PyErr_Clear() };
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_HasAttrStringWithError miss object={:p} attr={}",
                object,
                trace_name.as_deref().unwrap_or("<invalid>")
            );
        }
        return 0;
    }
    if trace_enabled {
        eprintln!(
            "[numpy-reduce] PyObject_HasAttrStringWithError error object={:p} attr={}",
            object,
            trace_name.as_deref().unwrap_or("<invalid>")
        );
    }
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetOptionalAttrString(
    object: *mut c_void,
    name: *const c_char,
    result: *mut *mut c_void,
) -> i32 {
    if result.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe { *result = std::ptr::null_mut() };
    let trace_enabled = cpython_trace_numpy_reduce_enabled();
    let trace_name = unsafe { c_name_to_string(name) }.ok();
    if trace_enabled {
        eprintln!(
            "[numpy-reduce] PyObject_GetOptionalAttrString object={:p} attr={}",
            object,
            trace_name.as_deref().unwrap_or("<invalid>")
        );
    }
    let value = unsafe { PyObject_GetAttrString(object, name) };
    if !value.is_null() {
        unsafe {
            *result = value;
        }
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_GetOptionalAttrString hit object={:p} attr={} result={:p}",
                object,
                trace_name.as_deref().unwrap_or("<invalid>"),
                value
            );
        }
        return 1;
    }
    if unsafe { PyErr_Occurred() }.is_null() {
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_GetOptionalAttrString miss-noerr object={:p} attr={}",
                object,
                trace_name.as_deref().unwrap_or("<invalid>")
            );
        }
        return 0;
    }
    if unsafe { PyErr_ExceptionMatches(PyExc_AttributeError) } != 0
        || cpython_error_message_indicates_missing_attribute()
    {
        unsafe { PyErr_Clear() };
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_GetOptionalAttrString miss object={:p} attr={}",
                object,
                trace_name.as_deref().unwrap_or("<invalid>")
            );
        }
        return 0;
    }
    if trace_enabled {
        eprintln!(
            "[numpy-reduce] PyObject_GetOptionalAttrString error object={:p} attr={}",
            object,
            trace_name.as_deref().unwrap_or("<invalid>")
        );
    }
    -1
}
