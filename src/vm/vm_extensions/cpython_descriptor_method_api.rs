use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_long, c_ulong, c_void};
use std::sync::atomic::Ordering;

use crate::runtime::{BuiltinFunction, Object, Value};

use super::{
    _Py_NoneStruct, CPY_PROXY_PTR_ATTR, CPYTHON_DESCRIPTOR_REGISTRY, CpythonCFunctionCompatObject,
    CpythonDescriptorKind, CpythonGetSetDef, CpythonMemberDef, CpythonMethodDef, CpythonObjectHead,
    CpythonTypeObject, METH_FASTCALL, METH_KEYWORDS, METH_METHOD, METH_NOARGS, METH_O,
    METH_VARARGS, ModuleCapiContext, PY_MEMBER_READONLY, PY_MEMBER_RELATIVE_OFFSET,
    PY_MEMBER_T_BOOL, PY_MEMBER_T_BYTE, PY_MEMBER_T_CHAR, PY_MEMBER_T_DOUBLE, PY_MEMBER_T_FLOAT,
    PY_MEMBER_T_INT, PY_MEMBER_T_LONG, PY_MEMBER_T_LONGLONG, PY_MEMBER_T_NONE, PY_MEMBER_T_OBJECT,
    PY_MEMBER_T_OBJECT_EX, PY_MEMBER_T_PYSSIZET, PY_MEMBER_T_SHORT, PY_MEMBER_T_STRING,
    PY_MEMBER_T_STRING_INPLACE, PY_MEMBER_T_UBYTE, PY_MEMBER_T_UINT, PY_MEMBER_T_ULONG,
    PY_MEMBER_T_ULONGLONG, PY_MEMBER_T_USHORT, Py_DecRef, Py_XDecRef, Py_XIncRef, PyBool_FromLong,
    PyCFunction_Type, PyClassMethodDescr_Type, PyDict_GetItemWithError, PyErr_BadArgument,
    PyErr_BadInternalCall, PyErr_Occurred, PyExc_AttributeError, PyExc_SystemError,
    PyExc_TypeError, PyExc_ValueError, PyFloat_AsDouble, PyFloat_FromDouble, PyGetSetDescr_Type,
    PyLong_AsLong, PyLong_AsLongLong, PyLong_AsSsize_t, PyLong_AsUnsignedLong,
    PyLong_AsUnsignedLongLong, PyLong_FromLong, PyLong_FromLongLong, PyLong_FromSsize_t,
    PyLong_FromUnsignedLong, PyLong_FromUnsignedLongLong, PyMemberDescr_Type, PyMethodDescr_Type,
    PyObject_Call, PyTuple_GetItem, PyTuple_New, PyTuple_SetItem, PyType_IsSubtype,
    PyUnicode_FromString, PyUnicode_FromStringAndSize, PyUnicode_InternFromString,
    TRACE_NUMPY_TYPEDICT_PTR, c_name_to_string, cpython_call_builtin,
    cpython_call_internal_in_context, cpython_debug_compare_value,
    cpython_debug_ufunc_attr_summary, cpython_getattr_in_context, cpython_is_type_object_ptr,
    cpython_keyword_args_from_dict_object, cpython_new_ptr_for_value,
    cpython_positional_args_from_tuple_object, cpython_ptr_is_type_object,
    cpython_safe_object_type_name, cpython_set_error, cpython_set_typed_error,
    cpython_type_name_for_object_ptr, cpython_value_debug_tag, cpython_value_from_ptr,
    value_to_int, with_active_cpython_context_mut,
};

pub(in crate::vm::vm_extensions) fn cpython_invoke_method_from_values(
    context: &mut ModuleCapiContext,
    method_def: *mut CpythonMethodDef,
    self_obj: *mut c_void,
    class_obj: *mut c_void,
    args: Vec<Value>,
    kwargs: HashMap<String, Value>,
) -> *mut c_void {
    if method_def.is_null() {
        context.set_error("missing method definition");
        return std::ptr::null_mut();
    }
    // SAFETY: method pointer comes from an extension-provided PyMethodDef table.
    let Some(method) = (unsafe { (*method_def).ml_meth }) else {
        context.set_error("missing method callback");
        return std::ptr::null_mut();
    };
    let trace_calls = std::env::var_os("PYRS_TRACE_CPY_METHOD_CALLS").is_some();
    let trace_numpy_empty = std::env::var_os("PYRS_TRACE_NUMPY_EMPTY_CALL").is_some();
    let trace_numpy_result_type = std::env::var_os("PYRS_TRACE_NUMPY_RESULT_TYPE").is_some();
    let trace_set_typedict = std::env::var_os("PYRS_TRACE_NUMPY_TYPEDICT").is_some();
    let trace_numpy_subtract = std::env::var_os("PYRS_TRACE_NUMPY_SUBTRACT").is_some();
    let trace_method_precall = std::env::var_os("PYRS_TRACE_CPY_METHOD_PRECALL").is_some();
    let trace_array_function_dispatcher =
        std::env::var_os("PYRS_TRACE_ARRAY_FUNCTION_DISPATCHER").is_some();
    let method_name = if trace_calls
        || trace_numpy_empty
        || trace_numpy_result_type
        || trace_set_typedict
        || trace_numpy_subtract
        || trace_method_precall
        || trace_array_function_dispatcher
        || std::env::var_os("PYRS_TRACE_COPYTO_CALL").is_some()
    {
        // SAFETY: method definition pointer is valid for metadata reads.
        unsafe {
            c_name_to_string((*method_def).ml_name).unwrap_or_else(|_| "<invalid>".to_string())
        }
    } else {
        String::new()
    };
    if trace_numpy_subtract && method_name == "subtract" {
        let arg_summary = args
            .iter()
            .map(cpython_value_debug_tag)
            .collect::<Vec<_>>()
            .join(", ");
        let mut kwargs_sorted = kwargs.iter().collect::<Vec<_>>();
        kwargs_sorted.sort_by(|(left, _), (right, _)| left.cmp(right));
        let kw_summary = kwargs_sorted
            .into_iter()
            .map(|(name, value)| format!("{name}={}", cpython_value_debug_tag(value)))
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "[numpy-subtract] args=[{}] kwargs=[{}] self={:p} class={:p}",
            arg_summary, kw_summary, self_obj, class_obj
        );
    }
    if trace_array_function_dispatcher && method_name == "_ArrayFunctionDispatcher" {
        let arg_tags = args
            .iter()
            .map(cpython_value_debug_tag)
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "[array-func-dispatcher-call] self={:p} class={:p} flags={} args_len={} args=[{}] kwargs_len={}",
            self_obj,
            class_obj,
            // SAFETY: method definition layout follows CPython ABI.
            unsafe { (*method_def).ml_flags },
            args.len(),
            arg_tags,
            kwargs.len()
        );
    }
    // SAFETY: method definition layout follows CPython ABI.
    let flags = unsafe { (*method_def).ml_flags };
    if trace_method_precall {
        eprintln!(
            "[cpy-method-precall] name={} flags={} self={:p} class={:p} args_len={} kwargs_len={}",
            method_name,
            flags,
            self_obj,
            class_obj,
            args.len(),
            kwargs.len()
        );
    }
    if std::env::var_os("PYRS_TRACE_ADD_DOCSTRING").is_some()
        && method_name.contains("add_docstring")
    {
        let arg_tags = args
            .iter()
            .map(cpython_value_debug_tag)
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "[add-doc-call] name={} flags={} self={:p} class={:p} args_len={} kwargs_len={} args=[{}]",
            method_name,
            flags,
            self_obj,
            class_obj,
            args.len(),
            kwargs.len(),
            arg_tags
        );
    }
    if std::env::var_os("PYRS_TRACE_COPYTO_CALL").is_some() && method_name == "copyto" {
        let mut kw_names = kwargs.keys().cloned().collect::<Vec<_>>();
        kw_names.sort();
        let arg_tags = args
            .iter()
            .map(cpython_value_debug_tag)
            .collect::<Vec<_>>()
            .join(", ");
        let kw_entries = kwargs
            .iter()
            .map(|(name, value)| format!("{name}={}", cpython_value_debug_tag(value)))
            .collect::<Vec<_>>()
            .join(", ");
        let self_tag = context
            .cpython_value_from_ptr_or_proxy(self_obj)
            .map(|value| cpython_value_debug_tag(&value))
            .unwrap_or_else(|| "<unknown-self>".to_string());
        let self_type =
            cpython_safe_object_type_name(self_obj).unwrap_or_else(|| "<unknown-type>".to_string());
        eprintln!(
            "[copyto-call] flags={} self={:p} self_tag={} self_type={} class={:p} def={:p} args_len={} args=[{}] kwargs=[{}]",
            flags,
            self_obj,
            self_tag,
            self_type,
            class_obj,
            method_def,
            args.len(),
            arg_tags,
            kw_entries
        );
    }
    if std::env::var_os("PYRS_TRACE_NUMPY_METHOD_BINDING").is_some()
        && matches!(
            method_name.as_str(),
            "copyto" | "dot" | "arange" | "empty_like" | "empty" | "result_type"
        )
    {
        let arg_tags = args
            .iter()
            .map(cpython_value_debug_tag)
            .collect::<Vec<_>>()
            .join(", ");
        let mut kw_entries = kwargs
            .iter()
            .map(|(name, value)| format!("{name}={}", cpython_value_debug_tag(value)))
            .collect::<Vec<_>>();
        kw_entries.sort();
        let self_tag = context
            .cpython_value_from_ptr_or_proxy(self_obj)
            .map(|value| cpython_value_debug_tag(&value))
            .unwrap_or_else(|| "<unknown-self>".to_string());
        let self_type =
            cpython_safe_object_type_name(self_obj).unwrap_or_else(|| "<unknown-type>".to_string());
        let class_type = cpython_safe_object_type_name(class_obj)
            .unwrap_or_else(|| "<unknown-class>".to_string());
        eprintln!(
            "[numpy-method-binding] name={} flags={} self={:p} self_tag={} self_type={} class={:p} class_type={} args=[{}] kwargs=[{}]",
            method_name,
            flags,
            self_obj,
            self_tag,
            self_type,
            class_obj,
            class_type,
            arg_tags,
            kw_entries.join(", ")
        );
    }
    if flags & METH_METHOD != 0 {
        if flags & (METH_FASTCALL | METH_KEYWORDS) != (METH_FASTCALL | METH_KEYWORDS) {
            context.set_error("METH_METHOD requires METH_FASTCALL|METH_KEYWORDS");
            return std::ptr::null_mut();
        }
        if class_obj.is_null() {
            context.set_error("METH_METHOD call missing defining class");
            return std::ptr::null_mut();
        }
        let mut stack: Vec<*mut c_void> =
            Vec::with_capacity(args.len().saturating_add(kwargs.len()));
        for value in &args {
            let ptr = context.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                context.set_error("failed to materialize METH_METHOD positional argument");
                return std::ptr::null_mut();
            }
            stack.push(ptr);
        }
        let mut kw_name_ptrs: Vec<*mut c_void> = Vec::with_capacity(kwargs.len());
        for (name, value) in &kwargs {
            let c_name = match CString::new(name.as_str()) {
                Ok(c_name) => c_name,
                Err(_) => {
                    context.set_error("METH_METHOD keyword name contains interior NUL byte");
                    return std::ptr::null_mut();
                }
            };
            // SAFETY: C string is NUL-terminated and valid for this call.
            let name_ptr = unsafe { PyUnicode_InternFromString(c_name.as_ptr()) };
            if name_ptr.is_null() {
                context.set_error("failed to intern METH_METHOD keyword name");
                return std::ptr::null_mut();
            }
            kw_name_ptrs.push(name_ptr);
            let ptr = context.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                context.set_error("failed to materialize METH_METHOD keyword argument");
                return std::ptr::null_mut();
            }
            stack.push(ptr);
        }
        if context.vm.is_null() {
            context.set_error("METH_METHOD call missing VM context");
            return std::ptr::null_mut();
        }
        let kwnames_ptr = if kw_name_ptrs.is_empty() {
            std::ptr::null_mut()
        } else {
            // SAFETY: tuple allocation follows CPython tuple ABI.
            let tuple = unsafe { PyTuple_New(kw_name_ptrs.len() as isize) };
            if tuple.is_null() {
                context.set_error("failed to allocate METH_METHOD keyword names tuple");
                return std::ptr::null_mut();
            }
            for (index, name_ptr) in kw_name_ptrs.into_iter().enumerate() {
                // SAFETY: tuple is newly allocated and index is in-bounds.
                let status = unsafe { PyTuple_SetItem(tuple, index as isize, name_ptr) };
                if status != 0 {
                    // SAFETY: tuple owns any already-inserted references.
                    unsafe { Py_DecRef(tuple) };
                    context.set_error("failed to populate METH_METHOD keyword names tuple");
                    return std::ptr::null_mut();
                }
            }
            tuple
        };
        if !kwargs.is_empty() && kwnames_ptr.is_null() {
            context.set_error("failed to materialize METH_METHOD keyword names");
            return std::ptr::null_mut();
        }
        let call: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *const *mut c_void,
            usize,
            *mut c_void,
        ) -> *mut c_void =
            // SAFETY: flags indicate `PyCMethod`-compatible signature.
            unsafe { std::mem::transmute(method) };
        let args_ptr = if stack.is_empty() {
            std::ptr::null()
        } else {
            stack.as_ptr()
        };
        let result = unsafe { call(self_obj, class_obj, args_ptr, args.len(), kwnames_ptr) };
        if !kwnames_ptr.is_null() {
            // SAFETY: kwnames tuple is call-local materialization and no longer needed after call.
            unsafe { Py_DecRef(kwnames_ptr) };
        }
        if trace_calls {
            eprintln!(
                "[cpy-method-call] name={} flags={} cmethod nargs={} kwargs={} class={:p} result={:p}",
                method_name,
                flags,
                args.len(),
                kwargs.len(),
                class_obj,
                result
            );
        }
        return result;
    }
    if flags & METH_FASTCALL != 0 {
        if context.vm.is_null() {
            context.set_error("METH_FASTCALL call missing VM context");
            return std::ptr::null_mut();
        }
        let accepts_keywords = (flags & METH_KEYWORDS) != 0;
        if !accepts_keywords && !kwargs.is_empty() {
            context.set_error("METH_FASTCALL call does not accept keyword arguments");
            return std::ptr::null_mut();
        }
        if trace_numpy_empty && method_name == "empty" {
            let mut names: Vec<String> = kwargs.keys().cloned().collect();
            names.sort();
            eprintln!(
                "[numpy-empty] fastcall args_len={} kwargs={:?}",
                args.len(),
                names
            );
        }
        let mut stack: Vec<*mut c_void> =
            Vec::with_capacity(args.len().saturating_add(if accepts_keywords {
                kwargs.len()
            } else {
                0
            }));
        for value in &args {
            let ptr = context.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                context.set_error("failed to materialize FASTCALL positional argument");
                return std::ptr::null_mut();
            }
            stack.push(ptr);
        }
        let mut kw_name_ptrs: Vec<*mut c_void> = Vec::new();
        if accepts_keywords {
            kw_name_ptrs = Vec::with_capacity(kwargs.len());
            for (name, value) in &kwargs {
                let c_name = match CString::new(name.as_str()) {
                    Ok(c_name) => c_name,
                    Err(_) => {
                        context.set_error("FASTCALL keyword name contains interior NUL byte");
                        return std::ptr::null_mut();
                    }
                };
                // SAFETY: C string is NUL-terminated and valid for this call.
                let name_ptr = unsafe { PyUnicode_InternFromString(c_name.as_ptr()) };
                if name_ptr.is_null() {
                    context.set_error("failed to intern FASTCALL keyword name");
                    return std::ptr::null_mut();
                }
                kw_name_ptrs.push(name_ptr);
                let ptr = context.alloc_cpython_ptr_for_value(value.clone());
                if ptr.is_null() {
                    context.set_error("failed to materialize FASTCALL keyword argument");
                    return std::ptr::null_mut();
                }
                stack.push(ptr);
            }
        }
        let args_ptr = if stack.is_empty() {
            std::ptr::null()
        } else {
            stack.as_ptr()
        };
        let result = if accepts_keywords {
            let kwnames_ptr = if kw_name_ptrs.is_empty() {
                std::ptr::null_mut()
            } else {
                // SAFETY: tuple allocation follows CPython tuple ABI.
                let tuple = unsafe { PyTuple_New(kw_name_ptrs.len() as isize) };
                if tuple.is_null() {
                    context.set_error("failed to allocate FASTCALL keyword names tuple");
                    return std::ptr::null_mut();
                }
                for (index, name_ptr) in kw_name_ptrs.into_iter().enumerate() {
                    // SAFETY: tuple is newly allocated and index is in-bounds.
                    let status = unsafe { PyTuple_SetItem(tuple, index as isize, name_ptr) };
                    if status != 0 {
                        // SAFETY: tuple owns any already-inserted references.
                        unsafe { Py_DecRef(tuple) };
                        context.set_error("failed to populate FASTCALL keyword names tuple");
                        return std::ptr::null_mut();
                    }
                }
                tuple
            };
            if !kwargs.is_empty() && kwnames_ptr.is_null() {
                context.set_error("failed to materialize FASTCALL keyword names");
                return std::ptr::null_mut();
            }
            let call: unsafe extern "C" fn(*mut c_void, *const *mut c_void, usize, *mut c_void) -> *mut c_void =
                // SAFETY: method flags indicate FASTCALL|KEYWORDS signature.
                unsafe { std::mem::transmute(method) };
            let result = unsafe { call(self_obj, args_ptr, args.len(), kwnames_ptr) };
            if !kwnames_ptr.is_null() {
                // SAFETY: kwnames tuple is call-local materialization and no longer needed after call.
                unsafe { Py_DecRef(kwnames_ptr) };
            }
            result
        } else {
            let call: unsafe extern "C" fn(*mut c_void, *const *mut c_void, usize) -> *mut c_void =
                // SAFETY: method flags indicate FASTCALL-only signature.
                unsafe { std::mem::transmute(method) };
            unsafe { call(self_obj, args_ptr, args.len()) }
        };
        if trace_numpy_result_type && method_name == "result_type" {
            let mut kw_names = kwargs.keys().cloned().collect::<Vec<_>>();
            kw_names.sort();
            let mapped_summary = context
                .cpython_value_from_ptr_or_proxy(result)
                .map(|value| {
                    let raw = ModuleCapiContext::cpython_proxy_raw_ptr_from_value(&value)
                        .map(|ptr| format!("{:p}", ptr))
                        .unwrap_or_else(|| "<none>".to_string());
                    let class_raw = match &value {
                        Value::Instance(instance_obj) => match &*instance_obj.kind() {
                            Object::Instance(instance_data) => match &*instance_data.class.kind() {
                                Object::Class(class_data) => {
                                    match class_data.attrs.get(CPY_PROXY_PTR_ATTR) {
                                        Some(Value::Int(raw_ptr)) if *raw_ptr >= 0 => {
                                            format!("{:p}", *raw_ptr as usize as *mut c_void)
                                        }
                                        _ => "<none>".to_string(),
                                    }
                                }
                                _ => "<none>".to_string(),
                            },
                            _ => "<none>".to_string(),
                        },
                        _ => "<none>".to_string(),
                    };
                    let class_tp_name = if let Ok(class_ptr) =
                        usize::from_str_radix(class_raw.trim_start_matches("0x"), 16)
                    {
                        let class_ptr = class_ptr as *mut c_void;
                        if class_ptr.is_null() {
                            "<none>".to_string()
                        } else {
                            // SAFETY: diagnostics on proxy raw class pointer.
                            unsafe {
                                class_ptr
                                    .cast::<CpythonTypeObject>()
                                    .as_ref()
                                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                                    .unwrap_or_else(|| "<unknown>".to_string())
                            }
                        }
                    } else {
                        "<none>".to_string()
                    };
                    format!(
                        "value_type={} value_raw={} class_raw={} class_tp_name={}",
                        cpython_debug_ufunc_attr_summary(&value, 2),
                        raw,
                        class_raw,
                        class_tp_name
                    )
                })
                .unwrap_or_else(|| "<none>".to_string());
            eprintln!(
                "[numpy-result-type] flags={} nargs={} kwargs={:?} result_ptr={:p} result_ob_type={:p} result_type_name={} mapped={}",
                flags,
                args.len(),
                kw_names,
                result,
                if result.is_null() {
                    std::ptr::null_mut()
                } else {
                    // SAFETY: diagnostics only for returned PyObject pointer.
                    unsafe {
                        result
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type)
                            .unwrap_or(std::ptr::null_mut())
                    }
                },
                cpython_type_name_for_object_ptr(result),
                mapped_summary
            );
        }
        if trace_calls {
            eprintln!(
                "[cpy-method-call] name={} flags={} fastcall nargs={} kwargs={} result={:p}",
                method_name,
                flags,
                args.len(),
                kwargs.len(),
                result
            );
        }
        return result;
    }
    if flags & METH_KEYWORDS != 0 {
        if context.vm.is_null() {
            context.set_error("METH_KEYWORDS call missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let args_ptr = context.alloc_cpython_ptr_for_value(vm.heap.alloc_tuple(args));
        if args_ptr.is_null() {
            context.set_error("failed to materialize cfunction args tuple");
            return std::ptr::null_mut();
        }
        let kwargs_empty = kwargs.is_empty();
        let kwargs_ptr = if kwargs_empty {
            std::ptr::null_mut()
        } else {
            let entries = kwargs
                .into_iter()
                .map(|(name, value)| (Value::Str(name), value))
                .collect::<Vec<_>>();
            context.alloc_cpython_ptr_for_value(vm.heap.alloc_dict(entries))
        };
        if !kwargs_ptr.is_null() {
            // no-op: kwargs materialized successfully.
        } else if !kwargs_empty {
            context.set_error("failed to materialize cfunction kwargs dict");
            return std::ptr::null_mut();
        }
        let call: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: method flags indicate VARARGS|KEYWORDS signature.
            unsafe { std::mem::transmute(method) };
        return unsafe { call(self_obj, args_ptr, kwargs_ptr) };
    }
    if flags & METH_VARARGS != 0 {
        if !kwargs.is_empty() {
            context.set_error("METH_VARARGS call does not accept keyword arguments");
            return std::ptr::null_mut();
        }
        if trace_set_typedict && method_name == "set_typeDict" {
            let arg_len = args.len();
            if let Some(Value::Dict(dict_obj)) = args.first()
                && let Object::Dict(dict_data) = &*dict_obj.kind()
            {
                let sample = dict_data
                    .iter()
                    .take(8)
                    .map(|(k, _)| cpython_debug_compare_value(k))
                    .collect::<Vec<_>>()
                    .join(", ");
                let has_int8 = dict_data.contains_key(&Value::Str("int8".to_string()));
                let has_bool = dict_data.contains_key(&Value::Str("bool".to_string()));
                let has_float64 = dict_data.contains_key(&Value::Str("float64".to_string()));
                eprintln!(
                    "[numpy-typedict] incoming dict entries={} has_int8={} has_bool={} has_float64={} sample=[{}]",
                    dict_data.len(),
                    has_int8,
                    has_bool,
                    has_float64,
                    sample
                );
            } else {
                eprintln!(
                    "[numpy-typedict] incoming args_len={} first={}",
                    arg_len,
                    args.first()
                        .map(cpython_value_debug_tag)
                        .unwrap_or_else(|| "<none>".to_string())
                );
            }
        }
        if context.vm.is_null() {
            context.set_error("METH_VARARGS call missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let args_ptr = context.alloc_cpython_ptr_for_value(vm.heap.alloc_tuple(args));
        if args_ptr.is_null() {
            context.set_error("failed to materialize cfunction args tuple");
            return std::ptr::null_mut();
        }
        if trace_set_typedict && method_name == "set_typeDict" {
            let dict_ptr = unsafe { PyTuple_GetItem(args_ptr, 0) };
            TRACE_NUMPY_TYPEDICT_PTR.store(dict_ptr as usize, Ordering::Relaxed);
            let probe_key = unsafe { PyUnicode_FromString(c"int8".as_ptr()) };
            let probe_value = if probe_key.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { PyDict_GetItemWithError(dict_ptr, probe_key) }
            };
            let probe_error = unsafe { PyErr_Occurred() };
            if !probe_key.is_null() {
                unsafe { Py_DecRef(probe_key) };
            }
            eprintln!(
                "[numpy-typedict] c-arg tuple={:p} dict_ptr={:p} probe_int8={:p} probe_err={:p}",
                args_ptr, dict_ptr, probe_value, probe_error
            );
        }
        let call: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: method flags indicate VARARGS signature.
            unsafe { std::mem::transmute(method) };
        let result = unsafe { call(self_obj, args_ptr) };
        if trace_set_typedict && method_name == "set_typeDict" {
            eprintln!(
                "[numpy-typedict] result={:p} last_error={:?}",
                result, context.last_error
            );
        }
        return result;
    }
    if flags & METH_NOARGS != 0 {
        if !args.is_empty() || !kwargs.is_empty() {
            context.set_error("METH_NOARGS call expected no arguments");
            return std::ptr::null_mut();
        }
        let call: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: method flags indicate NOARGS signature.
            unsafe { std::mem::transmute(method) };
        return unsafe { call(self_obj, std::ptr::null_mut()) };
    }
    if flags & METH_O != 0 {
        if args.len() != 1 || !kwargs.is_empty() {
            context.set_error("METH_O call expected exactly one positional argument");
            return std::ptr::null_mut();
        }
        let arg_ptr = context.alloc_cpython_ptr_for_value(args[0].clone());
        if arg_ptr.is_null() {
            context.set_error("failed to materialize cfunction single argument");
            return std::ptr::null_mut();
        }
        let call: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: method flags indicate METH_O signature.
            unsafe { std::mem::transmute(method) };
        return unsafe { call(self_obj, arg_ptr) };
    }
    context.set_error(format!("unsupported cfunction method flags: {flags}"));
    std::ptr::null_mut()
}

fn cpython_method_call_flags_are_valid(flags: i32) -> bool {
    let call_flags =
        flags & (METH_VARARGS | METH_FASTCALL | METH_NOARGS | METH_O | METH_KEYWORDS | METH_METHOD);
    call_flags == METH_VARARGS
        || call_flags == (METH_VARARGS | METH_KEYWORDS)
        || call_flags == METH_FASTCALL
        || call_flags == (METH_FASTCALL | METH_KEYWORDS)
        || call_flags == METH_NOARGS
        || call_flags == METH_O
        || call_flags == (METH_METHOD | METH_FASTCALL | METH_KEYWORDS)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCMethod_New(
    method_def: *mut c_void,
    self_obj: *mut c_void,
    module_obj: *mut c_void,
    class_obj: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if method_def.is_null() {
            context.set_error("bad internal call");
            return std::ptr::null_mut();
        }
        let method_def = method_def.cast::<CpythonMethodDef>();
        // SAFETY: method table pointer is non-null and points to extension-owned definition.
        let flags = unsafe { (*method_def).ml_flags };
        let has_meth_method = (flags & METH_METHOD) != 0;
        if !cpython_method_call_flags_are_valid(flags) {
            // SAFETY: `ml_name` is expected NUL-terminated by PyMethodDef contract.
            let method_name = unsafe {
                c_name_to_string((*method_def).ml_name).unwrap_or_else(|_| "<unnamed>".to_string())
            };
            context.set_error(format!(
                "SystemError: {}() method: bad call flags",
                method_name
            ));
            return std::ptr::null_mut();
        }
        if has_meth_method && class_obj.is_null() {
            context.set_error(
                "SystemError: attempting to create PyCMethod with a METH_METHOD flag but no class",
            );
            return std::ptr::null_mut();
        }
        if !has_meth_method && !class_obj.is_null() {
            context.set_error(
                "SystemError: attempting to create PyCFunction with class but no METH_METHOD flag",
            );
            return std::ptr::null_mut();
        }
        context.alloc_cpython_method_cfunction_ptr(method_def, self_obj, module_obj, class_obj)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_NewEx(
    method_def: *mut c_void,
    self_obj: *mut c_void,
    module_obj: *mut c_void,
) -> *mut c_void {
    unsafe { PyCMethod_New(method_def, self_obj, module_obj, std::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_New(
    method_def: *mut c_void,
    self_obj: *mut c_void,
) -> *mut c_void {
    unsafe { PyCFunction_NewEx(method_def, self_obj, std::ptr::null_mut()) }
}

fn cpython_descriptor_owner_is_type(
    context: &mut ModuleCapiContext,
    type_obj: *mut c_void,
) -> bool {
    if cpython_ptr_is_type_object(type_obj) {
        return true;
    }
    matches!(
        context.cpython_value_from_ptr_or_proxy(type_obj),
        Some(Value::Class(_))
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewMethod(
    type_obj: *mut c_void,
    method: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if type_obj.is_null() || method.is_null() {
            context.set_error("bad internal call");
            return std::ptr::null_mut();
        }
        if !cpython_descriptor_owner_is_type(context, type_obj) {
            if std::env::var_os("PYRS_TRACE_CPY_DESCR_TYPE_CHECK").is_some() {
                // SAFETY: diagnostics for candidate type pointer during descriptor creation.
                unsafe {
                    let object_type = type_obj
                        .cast::<CpythonObjectHead>()
                        .as_ref()
                        .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                        .unwrap_or(std::ptr::null_mut());
                    let (
                        metatype_flags,
                        metatype_name,
                        metatype_base,
                        metatype_ob_type,
                        metatype_is_subtype_of_type,
                    ) =
                        if object_type.is_null() {
                            (
                                0usize,
                                "<null>".to_string(),
                                std::ptr::null_mut(),
                                std::ptr::null_mut(),
                                0,
                            )
                        } else {
                            (
                                (*object_type).tp_flags,
                                c_name_to_string((*object_type).tp_name)
                                    .unwrap_or_else(|_| "<invalid>".to_string()),
                                (*object_type).tp_base.cast::<c_void>(),
                                (*object_type).ob_type,
                                PyType_IsSubtype(
                                    object_type.cast::<c_void>(),
                                    std::ptr::addr_of_mut!(super::PyType_Type).cast::<c_void>(),
                                ),
                            )
                        };
                    let tp_name = if type_obj.is_null() {
                        "<null>".to_string()
                    } else {
                        c_name_to_string((*type_obj.cast::<CpythonTypeObject>()).tp_name)
                            .unwrap_or_else(|_| "<invalid>".to_string())
                    };
                    eprintln!(
                        "[cpy-descr] PyDescr_NewMethod non-type type_obj={:p} tp_name={} object_type={:p} metatype_name={} metatype_base={:p} metatype_ob_type={:p} metatype_is_subtype_of_type={} metatype_flags=0x{:x} PyType_Type={:p} PyBaseObject_Type={:p}",
                        type_obj,
                        tp_name,
                        object_type,
                        metatype_name,
                        metatype_base,
                        metatype_ob_type,
                        metatype_is_subtype_of_type,
                        metatype_flags,
                        std::ptr::addr_of_mut!(super::PyType_Type).cast::<c_void>(),
                        std::ptr::addr_of_mut!(super::PyBaseObject_Type).cast::<c_void>()
                    );
                }
            }
            context.set_error("PyDescr_NewMethod expected type object");
            return std::ptr::null_mut();
        }
        let owner_type = type_obj.cast::<CpythonTypeObject>();
        let method_def = method.cast::<CpythonMethodDef>();
        // SAFETY: method pointer is caller-owned and expected to outlive descriptor.
        let method_name_ptr = unsafe { (*method_def).ml_name };
        if method_name_ptr.is_null() {
            context.set_error("SystemError: <unnamed>() method: bad call flags");
            return std::ptr::null_mut();
        }
        // SAFETY: method definition pointer is non-null and points to extension-owned definition.
        let flags = unsafe { (*method_def).ml_flags };
        if !cpython_method_call_flags_are_valid(flags) {
            // SAFETY: `ml_name` is expected NUL-terminated by PyMethodDef contract.
            let method_name = unsafe { c_name_to_string(method_name_ptr) }
                .unwrap_or_else(|_| "<unnamed>".to_string());
            context.set_error(format!(
                "SystemError: {}() method: bad call flags",
                method_name
            ));
            return std::ptr::null_mut();
        }
        context.alloc_cpython_descriptor_ptr(
            std::ptr::addr_of_mut!(PyMethodDescr_Type),
            CpythonDescriptorKind::Method {
                owner_type,
                method_def,
                class_method: false,
            },
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewClassMethod(
    type_obj: *mut c_void,
    method: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if type_obj.is_null() || method.is_null() {
            context.set_error("bad internal call");
            return std::ptr::null_mut();
        }
        if !cpython_descriptor_owner_is_type(context, type_obj) {
            context.set_error("PyDescr_NewClassMethod expected type object");
            return std::ptr::null_mut();
        }
        let owner_type = type_obj.cast::<CpythonTypeObject>();
        let method_def = method.cast::<CpythonMethodDef>();
        // SAFETY: method pointer is caller-owned and expected to outlive descriptor.
        let method_name_ptr = unsafe { (*method_def).ml_name };
        if method_name_ptr.is_null() {
            context.set_error("SystemError: <unnamed>() method: bad call flags");
            return std::ptr::null_mut();
        }
        // CPython does not reject classmethod definitions here, but we retain
        // method call-flag validation to prevent invalid call ABI dispatch later.
        let flags = unsafe { (*method_def).ml_flags };
        if !cpython_method_call_flags_are_valid(flags) {
            // SAFETY: `ml_name` is expected NUL-terminated by PyMethodDef contract.
            let method_name = unsafe { c_name_to_string(method_name_ptr) }
                .unwrap_or_else(|_| "<unnamed>".to_string());
            context.set_error(format!(
                "SystemError: {}() method: bad call flags",
                method_name
            ));
            return std::ptr::null_mut();
        }
        context.alloc_cpython_descriptor_ptr(
            std::ptr::addr_of_mut!(PyClassMethodDescr_Type),
            CpythonDescriptorKind::Method {
                owner_type,
                method_def,
                class_method: true,
            },
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewMember(
    type_obj: *mut c_void,
    member: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if type_obj.is_null() || member.is_null() {
            context.set_error("bad internal call");
            return std::ptr::null_mut();
        }
        if !cpython_descriptor_owner_is_type(context, type_obj) {
            context.set_error("PyDescr_NewMember expected type object");
            return std::ptr::null_mut();
        }
        let owner_type = type_obj.cast::<CpythonTypeObject>();
        let member_def = member.cast::<CpythonMemberDef>();
        // SAFETY: member definition pointer is non-null and extension-owned.
        let member_name_ptr = unsafe { (*member_def).name };
        if member_name_ptr.is_null() {
            context.set_error("PyDescr_NewMember expected member name");
            return std::ptr::null_mut();
        }
        // SAFETY: member definition pointer is non-null and extension-owned.
        let flags = unsafe { (*member_def).flags };
        if (flags & PY_MEMBER_RELATIVE_OFFSET) != 0 {
            context.set_error("PyDescr_NewMember used with Py_RELATIVE_OFFSET");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_descriptor_ptr(
            std::ptr::addr_of_mut!(PyMemberDescr_Type),
            CpythonDescriptorKind::Member {
                owner_type,
                member: member_def,
            },
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMember_GetOne(
    obj_addr: *const c_char,
    member: *mut c_void,
) -> *mut c_void {
    if obj_addr.is_null() || member.is_null() {
        cpython_set_typed_error(unsafe { PyExc_SystemError }, "bad internal call");
        return std::ptr::null_mut();
    }
    let member_def = unsafe { &*member.cast::<CpythonMemberDef>() };
    if (member_def.flags & PY_MEMBER_RELATIVE_OFFSET) != 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyMember_GetOne used with Py_RELATIVE_OFFSET",
        );
        return std::ptr::null_mut();
    }
    let field_ptr =
        match ModuleCapiContext::member_field_ptr(obj_addr.cast_mut().cast(), member_def) {
            Ok(ptr) => ptr,
            Err(err) => {
                cpython_set_typed_error(unsafe { PyExc_SystemError }, err);
                return std::ptr::null_mut();
            }
        };
    let none_ptr = std::ptr::addr_of_mut!(_Py_NoneStruct).cast::<c_void>();
    match member_def.member_type {
        PY_MEMBER_T_BOOL => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_char>()) };
            unsafe { PyBool_FromLong((raw != 0) as c_long) }
        }
        PY_MEMBER_T_BYTE => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<i8>()) };
            unsafe { PyLong_FromLong(raw as i64) }
        }
        PY_MEMBER_T_UBYTE => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u8>()) };
            unsafe { PyLong_FromUnsignedLong(raw as u64) }
        }
        PY_MEMBER_T_SHORT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<i16>()) };
            unsafe { PyLong_FromLong(raw as i64) }
        }
        PY_MEMBER_T_USHORT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u16>()) };
            unsafe { PyLong_FromUnsignedLong(raw as u64) }
        }
        PY_MEMBER_T_INT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_int>()) };
            unsafe { PyLong_FromLong(raw as i64) }
        }
        PY_MEMBER_T_UINT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u32>()) };
            unsafe { PyLong_FromUnsignedLong(raw as u64) }
        }
        PY_MEMBER_T_LONG => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_long>()) };
            unsafe { PyLong_FromLong(raw as i64) }
        }
        PY_MEMBER_T_ULONG => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_ulong>()) };
            unsafe { PyLong_FromUnsignedLong(raw as u64) }
        }
        PY_MEMBER_T_PYSSIZET => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<isize>()) };
            unsafe { PyLong_FromSsize_t(raw) }
        }
        PY_MEMBER_T_FLOAT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<f32>()) };
            unsafe { PyFloat_FromDouble(raw as f64) }
        }
        PY_MEMBER_T_DOUBLE => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<f64>()) };
            unsafe { PyFloat_FromDouble(raw) }
        }
        PY_MEMBER_T_STRING => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*const c_char>()) };
            if raw.is_null() {
                none_ptr
            } else {
                unsafe { PyUnicode_FromString(raw) }
            }
        }
        PY_MEMBER_T_STRING_INPLACE => unsafe { PyUnicode_FromString(field_ptr.cast()) },
        PY_MEMBER_T_CHAR => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u8>()) };
            let text = [raw];
            unsafe { PyUnicode_FromStringAndSize(text.as_ptr().cast(), 1) }
        }
        PY_MEMBER_T_OBJECT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*mut c_void>()) };
            if raw.is_null() {
                none_ptr
            } else {
                unsafe { Py_XIncRef(raw) };
                raw
            }
        }
        PY_MEMBER_T_OBJECT_EX => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*mut c_void>()) };
            if raw.is_null() {
                let member_name = ModuleCapiContext::member_attr_name(member_def);
                cpython_set_typed_error(
                    unsafe { PyExc_AttributeError },
                    format!("attribute '{member_name}' is not set"),
                );
                std::ptr::null_mut()
            } else {
                unsafe { Py_XIncRef(raw) };
                raw
            }
        }
        PY_MEMBER_T_LONGLONG => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<i64>()) };
            unsafe { PyLong_FromLongLong(raw) }
        }
        PY_MEMBER_T_ULONGLONG => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u64>()) };
            unsafe { PyLong_FromUnsignedLongLong(raw) }
        }
        PY_MEMBER_T_NONE => none_ptr,
        _ => {
            cpython_set_typed_error(unsafe { PyExc_SystemError }, "bad memberdescr type");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMember_SetOne(
    obj_addr: *mut c_char,
    member: *mut c_void,
    value: *mut c_void,
) -> c_int {
    if obj_addr.is_null() || member.is_null() {
        cpython_set_typed_error(unsafe { PyExc_SystemError }, "bad internal call");
        return -1;
    }
    let member_def = unsafe { &*member.cast::<CpythonMemberDef>() };
    if (member_def.flags & PY_MEMBER_RELATIVE_OFFSET) != 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyMember_SetOne used with Py_RELATIVE_OFFSET",
        );
        return -1;
    }
    if (member_def.flags & PY_MEMBER_READONLY) != 0 {
        cpython_set_typed_error(unsafe { PyExc_AttributeError }, "readonly attribute");
        return -1;
    }
    let field_ptr = match ModuleCapiContext::member_field_ptr(obj_addr.cast(), member_def) {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_typed_error(unsafe { PyExc_SystemError }, err);
            return -1;
        }
    };
    let member_name = ModuleCapiContext::member_attr_name(member_def);
    if value.is_null() {
        if member_def.member_type == PY_MEMBER_T_OBJECT_EX {
            let current = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*mut c_void>()) };
            if current.is_null() {
                cpython_set_typed_error(unsafe { PyExc_AttributeError }, member_name);
                return -1;
            }
        } else if member_def.member_type != PY_MEMBER_T_OBJECT {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "can't delete numeric/char attribute",
            );
            return -1;
        }
    }
    match member_def.member_type {
        PY_MEMBER_T_BOOL => {
            let Ok(py_value) = cpython_value_from_ptr(value) else {
                cpython_set_typed_error(
                    unsafe { PyExc_TypeError },
                    "attribute value type must be bool",
                );
                return -1;
            };
            let Value::Bool(flag) = py_value else {
                cpython_set_typed_error(
                    unsafe { PyExc_TypeError },
                    "attribute value type must be bool",
                );
                return -1;
            };
            unsafe {
                std::ptr::write_unaligned(field_ptr.cast::<c_char>(), if flag { 1 } else { 0 })
            };
            0
        }
        PY_MEMBER_T_BYTE => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<i8>(), raw as i8) };
            0
        }
        PY_MEMBER_T_UBYTE => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<u8>(), raw as u8) };
            0
        }
        PY_MEMBER_T_SHORT => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<i16>(), raw as i16) };
            0
        }
        PY_MEMBER_T_USHORT => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<u16>(), raw as u16) };
            0
        }
        PY_MEMBER_T_INT => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<c_int>(), raw as c_int) };
            0
        }
        PY_MEMBER_T_UINT => {
            let numeric_value = match cpython_value_from_ptr(value) {
                Ok(v) => v,
                Err(err) => {
                    cpython_set_error(err);
                    return -1;
                }
            };
            let raw = if let Ok(signed) = value_to_int(numeric_value.clone()) {
                signed as u32
            } else {
                let unsigned = unsafe { PyLong_AsUnsignedLong(value) };
                if !unsafe { PyErr_Occurred() }.is_null() {
                    return -1;
                }
                unsigned as u32
            };
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<u32>(), raw) };
            0
        }
        PY_MEMBER_T_LONG => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<c_long>(), raw as c_long) };
            0
        }
        PY_MEMBER_T_ULONG => {
            let numeric_value = match cpython_value_from_ptr(value) {
                Ok(v) => v,
                Err(err) => {
                    cpython_set_error(err);
                    return -1;
                }
            };
            let raw = if let Ok(signed) = value_to_int(numeric_value.clone()) {
                signed as c_ulong
            } else {
                let unsigned = unsafe { PyLong_AsUnsignedLong(value) };
                if !unsafe { PyErr_Occurred() }.is_null() {
                    return -1;
                }
                unsigned as c_ulong
            };
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<c_ulong>(), raw) };
            0
        }
        PY_MEMBER_T_PYSSIZET => {
            let raw = unsafe { PyLong_AsSsize_t(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<isize>(), raw) };
            0
        }
        PY_MEMBER_T_FLOAT => {
            let raw = unsafe { PyFloat_AsDouble(value) };
            if raw == -1.0 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<f32>(), raw as f32) };
            0
        }
        PY_MEMBER_T_DOUBLE => {
            let raw = unsafe { PyFloat_AsDouble(value) };
            if raw == -1.0 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<f64>(), raw) };
            0
        }
        PY_MEMBER_T_OBJECT | PY_MEMBER_T_OBJECT_EX => {
            let previous = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*mut c_void>()) };
            if !value.is_null() {
                unsafe { Py_XIncRef(value) };
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<*mut c_void>(), value) };
            if !previous.is_null() {
                unsafe { Py_XDecRef(previous) };
            }
            0
        }
        PY_MEMBER_T_CHAR => {
            let Ok(py_value) = cpython_value_from_ptr(value) else {
                unsafe { PyErr_BadArgument() };
                return -1;
            };
            let Value::Str(text) = py_value else {
                unsafe { PyErr_BadArgument() };
                return -1;
            };
            if text.as_bytes().len() != 1 {
                unsafe { PyErr_BadArgument() };
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<u8>(), text.as_bytes()[0]) };
            0
        }
        PY_MEMBER_T_STRING | PY_MEMBER_T_STRING_INPLACE => {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, "readonly attribute");
            -1
        }
        PY_MEMBER_T_LONGLONG => {
            let raw = unsafe { PyLong_AsLongLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<i64>(), raw) };
            0
        }
        PY_MEMBER_T_ULONGLONG => {
            let numeric_value = match cpython_value_from_ptr(value) {
                Ok(v) => v,
                Err(err) => {
                    cpython_set_error(err);
                    return -1;
                }
            };
            let raw = if let Ok(signed) = value_to_int(numeric_value.clone()) {
                signed as u64
            } else {
                let unsigned = unsafe { PyLong_AsUnsignedLongLong(value) };
                if !unsafe { PyErr_Occurred() }.is_null() {
                    return -1;
                }
                unsigned
            };
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<u64>(), raw) };
            0
        }
        _ => {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                format!("bad memberdescr type for {member_name}"),
            );
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewGetSet(
    type_obj: *mut c_void,
    getset: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if type_obj.is_null() || getset.is_null() {
            context.set_error("bad internal call");
            return std::ptr::null_mut();
        }
        if !cpython_descriptor_owner_is_type(context, type_obj) {
            context.set_error("PyDescr_NewGetSet expected type object");
            return std::ptr::null_mut();
        }
        let owner_type = type_obj.cast::<CpythonTypeObject>();
        let getset_def = getset.cast::<CpythonGetSetDef>();
        // SAFETY: getset definition pointer is non-null and extension-owned.
        let getset_name_ptr = unsafe { (*getset_def).name };
        if getset_name_ptr.is_null() {
            context.set_error("PyDescr_NewGetSet expected getset name");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_descriptor_ptr(
            std::ptr::addr_of_mut!(PyGetSetDescr_Type),
            CpythonDescriptorKind::GetSet {
                owner_type,
                getset: getset_def,
            },
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyWrapper_New(
    descriptor: *mut c_void,
    self_obj: *mut c_void,
) -> *mut c_void {
    if descriptor.is_null() || self_obj.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        // SAFETY: self_obj points to a CPython-compatible object header.
        let object_type = unsafe {
            self_obj
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type)
                .unwrap_or(std::ptr::null_mut())
        };
        if object_type.is_null() {
            context.set_error("PyWrapper_New received object without type");
            return std::ptr::null_mut();
        }
        let is_type_object = cpython_is_type_object_ptr(self_obj);
        if let Some(bound) = context.resolve_descriptor_attr_ptr(
            descriptor,
            self_obj,
            object_type.cast(),
            is_type_object,
        ) {
            return bound;
        }

        let Some(descriptor_value) = context.cpython_value_from_ptr_or_proxy(descriptor) else {
            context.set_error("PyWrapper_New received unknown descriptor");
            return std::ptr::null_mut();
        };
        let Some(self_value) = context.cpython_value_from_ptr_or_proxy(self_obj) else {
            context.set_error("PyWrapper_New received unknown self object");
            return std::ptr::null_mut();
        };
        let getter = match cpython_getattr_in_context(context, descriptor_value, "__get__") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let owner_type_value = context
            .cpython_value_from_ptr_or_proxy(object_type.cast())
            .unwrap_or(Value::None);
        let bound = match cpython_call_internal_in_context(
            context,
            getter,
            vec![self_value, owner_type_value],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        context.alloc_cpython_ptr_for_value(bound)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

pub(in crate::vm::vm_extensions) unsafe extern "C" fn cpython_cfunction_tp_call(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if callable.is_null() {
            context.set_error("cfunction call received null callable");
            return std::ptr::null_mut();
        }
        let raw = callable.cast::<CpythonCFunctionCompatObject>();
        // SAFETY: `callable` is a cfunction object allocated by this runtime.
        let method_def = unsafe { (*raw).m_ml };
        if method_def.is_null() {
            context.set_error("cfunction call missing method definition");
            return std::ptr::null_mut();
        }
        // SAFETY: cfunction object layout is stable for this context.
        let self_obj = unsafe { (*raw).m_self };
        if self_obj == usize::MAX as *mut c_void {
            // SAFETY: method table pointer is validated above.
            let method_name = unsafe { c_name_to_string((*method_def).ml_name) }
                .unwrap_or_else(|_| "<unnamed>".to_string());
            context.set_error(format!(
                "invalid cfunction self sentinel for method '{}'",
                method_name
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: cfunction object layout is stable for this context.
        let class_obj = unsafe { (*raw).m_class };
        let positional = match cpython_positional_args_from_tuple_object(args) {
            Ok(values) => values,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let keyword_args = match cpython_keyword_args_from_dict_object(kwargs) {
            Ok(values) => values,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        cpython_invoke_method_from_values(
            context,
            method_def,
            self_obj,
            class_obj,
            positional,
            keyword_args,
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

pub(in crate::vm::vm_extensions) unsafe extern "C" fn cpython_method_descriptor_tp_call(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if callable.is_null() {
            context.set_error("method descriptor call received null callable");
            return std::ptr::null_mut();
        }
        let descriptor_key = callable as usize;
        let descriptor_kind = if let Some(kind) = context.cpython_descriptors.get(&descriptor_key) {
            Some(*kind)
        } else {
            CPYTHON_DESCRIPTOR_REGISTRY
                .with(|registry| registry.borrow().get(&descriptor_key).copied())
        };
        let Some(CpythonDescriptorKind::Method {
            owner_type,
            method_def,
            class_method,
        }) = descriptor_kind
        else {
            context.set_error("descriptor call expected method descriptor");
            return std::ptr::null_mut();
        };
        if owner_type.is_null() || method_def.is_null() {
            context.set_error("descriptor call has invalid method metadata");
            return std::ptr::null_mut();
        }
        let mut positional = match cpython_positional_args_from_tuple_object(args) {
            Ok(values) => values,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let keyword_args = match cpython_keyword_args_from_dict_object(kwargs) {
            Ok(values) => values,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        if positional.is_empty() {
            context.set_error("descriptor call requires an explicit receiver argument");
            return std::ptr::null_mut();
        }
        let receiver_value = positional.remove(0);
        let receiver_ptr = context.alloc_cpython_ptr_for_value(receiver_value);
        if receiver_ptr.is_null() {
            context.set_error("failed to materialize descriptor receiver argument");
            return std::ptr::null_mut();
        }
        // SAFETY: receiver pointer was materialized above.
        let receiver_type = unsafe {
            receiver_ptr
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if receiver_type.is_null()
            || unsafe {
                PyType_IsSubtype(receiver_type.cast::<c_void>(), owner_type.cast::<c_void>())
            } == 0
        {
            context.set_error("descriptor receiver is not an instance/subclass of owner type");
            return std::ptr::null_mut();
        }
        let flags = unsafe { (*method_def).ml_flags };
        let class_obj = if class_method {
            receiver_ptr
        } else if (flags & METH_METHOD) != 0 {
            owner_type.cast::<c_void>()
        } else {
            std::ptr::null_mut()
        };
        cpython_invoke_method_from_values(
            context,
            method_def,
            receiver_ptr,
            class_obj,
            positional,
            keyword_args,
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_cfunction_raw_object(
    context: &mut ModuleCapiContext,
    object: *mut c_void,
    api_name: &str,
) -> Option<*mut CpythonCFunctionCompatObject> {
    if object.is_null() {
        context.set_error(format!("{api_name} received null callable"));
        return None;
    }
    // SAFETY: `object` is a potential PyObject pointer; we only inspect head fields.
    let type_ptr = unsafe {
        object
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<c_void>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null()
        || unsafe {
            PyType_IsSubtype(
                type_ptr,
                std::ptr::addr_of_mut!(PyCFunction_Type).cast::<c_void>(),
            )
        } == 0
    {
        context.set_error("bad internal call");
        return None;
    }
    Some(object.cast::<CpythonCFunctionCompatObject>())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_GetFunction(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(raw) = cpython_cfunction_raw_object(context, object, "PyCFunction_GetFunction")
        else {
            return std::ptr::null_mut();
        };
        // SAFETY: raw object + method definition were validated above.
        unsafe {
            (*raw)
                .m_ml
                .as_ref()
                .and_then(|method| method.ml_meth)
                .map(|function| function as *mut c_void)
                .unwrap_or(std::ptr::null_mut())
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_GetSelf(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(raw) = cpython_cfunction_raw_object(context, object, "PyCFunction_GetSelf") else {
            return std::ptr::null_mut();
        };
        // SAFETY: raw object was validated above.
        unsafe { (*raw).m_self }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_GetFlags(object: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(raw) = cpython_cfunction_raw_object(context, object, "PyCFunction_GetFlags")
        else {
            return -1;
        };
        // SAFETY: raw object + method definition were validated above.
        unsafe {
            (*raw)
                .m_ml
                .as_ref()
                .map(|method| method.ml_flags)
                .unwrap_or(-1)
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_Call(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    unsafe { PyObject_Call(callable, args, kwargs) }
}

pub(in crate::vm::vm_extensions) unsafe extern "C" fn cpython_cfunction_tp_getattro(
    object: *mut c_void,
    name: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if object.is_null() {
            context.set_error("cfunction getattr received null object");
            return std::ptr::null_mut();
        }
        let attr_name = match context.cpython_value_from_ptr(name) {
            Some(Value::Str(text)) => text,
            _ => {
                context.set_error("cfunction getattr expected string attribute name");
                return std::ptr::null_mut();
            }
        };
        let raw = object.cast::<CpythonCFunctionCompatObject>();
        // SAFETY: `object` is expected to be a cfunction compat object.
        let method_def = unsafe { (*raw).m_ml };
        if method_def.is_null() {
            context.set_error("cfunction getattr missing method definition");
            return std::ptr::null_mut();
        }
        // SAFETY: method definition is extension-provided and pointer-stable.
        let method_name = unsafe { c_name_to_string((*method_def).ml_name) }
            .unwrap_or_else(|_| "method".to_string());
        match attr_name.as_str() {
            "__name__" | "__qualname__" => {
                context.alloc_cpython_ptr_for_value(Value::Str(method_name))
            }
            "__module__" => {
                // SAFETY: cfunction object layout is stable for this context.
                let self_obj = unsafe { (*raw).m_self };
                // SAFETY: cfunction object layout is stable for this context.
                let module_obj = unsafe { (*raw).m_module };
                let module_name = context
                    .cpython_value_from_ptr(module_obj)
                    .or_else(|| context.cpython_value_from_ptr(self_obj))
                    .and_then(|value| match value {
                        Value::Str(text) => Some(text),
                        Value::Module(module_obj) => match &*module_obj.kind() {
                            Object::Module(module_data) => Some(module_data.name.clone()),
                            _ => None,
                        },
                        _ => None,
                    })
                    .unwrap_or_else(|| "builtins".to_string());
                context.alloc_cpython_ptr_for_value(Value::Str(module_name))
            }
            "__doc__" => {
                // SAFETY: method definition is extension-provided and pointer-stable.
                let doc_ptr = unsafe { (*method_def).ml_doc };
                if doc_ptr.is_null() {
                    context.alloc_cpython_ptr_for_value(Value::None)
                } else {
                    // SAFETY: doc string is expected to be NUL-terminated by C-API contract.
                    let doc = unsafe { CStr::from_ptr(doc_ptr) }
                        .to_str()
                        .map(|text| text.to_string())
                        .unwrap_or_else(|_| String::new());
                    context.alloc_cpython_ptr_for_value(Value::Str(doc))
                }
            }
            _ => {
                context.set_error(format!(
                    "AttributeError: 'builtin_function_or_method' object has no attribute '{}'",
                    attr_name
                ));
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
pub unsafe extern "C" fn PySlice_New(
    start: *mut c_void,
    stop: *mut c_void,
    step: *mut c_void,
) -> *mut c_void {
    let start = if start.is_null() {
        Value::None
    } else {
        match cpython_value_from_ptr(start) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    let stop = if stop.is_null() {
        Value::None
    } else {
        match cpython_value_from_ptr(stop) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    let step = if step.is_null() {
        Value::None
    } else {
        match cpython_value_from_ptr(step) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    match cpython_call_builtin(BuiltinFunction::Slice, vec![start, stop, step]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_Unpack(
    slice: *mut c_void,
    start: *mut isize,
    stop: *mut isize,
    step: *mut isize,
) -> i32 {
    if start.is_null() || stop.is_null() || step.is_null() {
        cpython_set_error("PySlice_Unpack received null output pointer");
        return -1;
    }
    let Value::Slice(slice_value) = (match cpython_value_from_ptr(slice) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    }) else {
        cpython_set_error("PySlice_Unpack expected slice object");
        return -1;
    };
    unsafe {
        *start = slice_value.lower.unwrap_or(0) as isize;
        *stop = slice_value.upper.unwrap_or(isize::MAX as i64) as isize;
        *step = slice_value.step.unwrap_or(1) as isize;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_AdjustIndices(
    length: isize,
    start: *mut isize,
    stop: *mut isize,
    step: isize,
) -> isize {
    if start.is_null() || stop.is_null() || step == 0 {
        return 0;
    }
    // SAFETY: caller provided valid pointers.
    let mut s = unsafe { *start };
    // SAFETY: caller provided valid pointers.
    let mut e = unsafe { *stop };
    if s < 0 {
        s += length;
        if s < 0 {
            s = if step < 0 { -1 } else { 0 };
        }
    } else if s >= length {
        s = if step < 0 { length - 1 } else { length };
    }
    if e < 0 {
        e += length;
        if e < 0 {
            e = if step < 0 { -1 } else { 0 };
        }
    } else if e >= length {
        e = if step < 0 { length - 1 } else { length };
    }
    unsafe {
        *start = s;
        *stop = e;
    }
    if step < 0 {
        if e < s { (s - e - 1) / (-step) + 1 } else { 0 }
    } else if s < e {
        (e - s - 1) / step + 1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_GetIndices(
    slice: *mut c_void,
    length: isize,
    start: *mut isize,
    stop: *mut isize,
    step: *mut isize,
) -> i32 {
    if start.is_null() || stop.is_null() || step.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let Value::Slice(slice_value) = (match cpython_value_from_ptr(slice) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    }) else {
        cpython_set_error("PySlice_GetIndices expected slice object");
        return -1;
    };
    let raw_step = slice_value.step.unwrap_or(1) as isize;
    if raw_step == 0 {
        cpython_set_typed_error(unsafe { PyExc_ValueError }, "slice step cannot be zero");
        return -1;
    }
    let mut raw_start = match slice_value.lower {
        Some(value) => value as isize,
        None => {
            if raw_step < 0 {
                length.saturating_sub(1)
            } else {
                0
            }
        }
    };
    if slice_value.lower.is_some() && raw_start < 0 {
        raw_start += length;
    }
    let mut raw_stop = match slice_value.upper {
        Some(value) => value as isize,
        None => {
            if raw_step < 0 {
                -1
            } else {
                length
            }
        }
    };
    if slice_value.upper.is_some() && raw_stop < 0 {
        raw_stop += length;
    }
    if raw_stop > length || raw_start >= length {
        cpython_set_typed_error(unsafe { PyExc_ValueError }, "slice index out of range");
        return -1;
    }
    unsafe {
        *start = raw_start;
        *stop = raw_stop;
        *step = raw_step;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_GetIndicesEx(
    slice: *mut c_void,
    length: isize,
    start: *mut isize,
    stop: *mut isize,
    step: *mut isize,
    slice_length: *mut isize,
) -> i32 {
    if slice_length.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if unsafe { PySlice_Unpack(slice, start, stop, step) } < 0 {
        return -1;
    }
    // SAFETY: pointers validated by PySlice_Unpack and checked above.
    let adjusted = unsafe { PySlice_AdjustIndices(length, start, stop, *step) };
    unsafe {
        *slice_length = adjusted;
    }
    0
}
