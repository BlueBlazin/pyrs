use std::backtrace::Backtrace;
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::Ordering;

use crate::runtime::{BuiltinFunction, Object, Value};
use crate::vm::Vm;

use super::{
    _PyErr_BadInternalCall, CpythonBuffer, CpythonObjectHead, CpythonTypeObject,
    CpythonVarObjectHead, ModuleCapiContext, PYBYTES_ASSTRING_MISMATCH_BT_COUNT, Py_DecRef,
    Py_XDecRef, Py_XIncRef, PyBuffer_Release, PyByteArray_Type, PyBytes_Type, PyObject_GetBuffer,
    PyType_IsSubtype, c_name_to_string, cpython_bytes_data_ptr, cpython_call_builtin,
    cpython_new_bytes_ptr, cpython_new_ptr_for_value, cpython_set_error,
    cpython_type_name_for_object_ptr, cpython_value_from_ptr, with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_FromStringAndSize(
    value: *const c_char,
    len: isize,
) -> *mut c_void {
    if len < 0 {
        cpython_set_error("PyBytes_FromStringAndSize received negative length");
        return std::ptr::null_mut();
    }
    let bytes = if len == 0 {
        Vec::new()
    } else if value.is_null() {
        // CPython allows NULL input to allocate an uninitialized bytes buffer.
        // We materialize a zero-filled payload for deterministic safety.
        vec![0u8; len as usize]
    } else {
        // SAFETY: caller guarantees `value` points to at least `len` bytes.
        unsafe { std::slice::from_raw_parts(value.cast::<u8>(), len as usize).to_vec() }
    };
    cpython_new_bytes_ptr(bytes)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_FromString(value: *const c_char) -> *mut c_void {
    if value.is_null() {
        cpython_set_error("PyBytes_FromString received null pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: pointer must be NUL-terminated C string.
    let bytes = unsafe { CStr::from_ptr(value).to_bytes().to_vec() };
    cpython_new_bytes_ptr(bytes)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_FromObject(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        unsafe { _PyErr_BadInternalCall(std::ptr::null(), 0) };
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyBytes_FromObject received unknown object pointer");
            return std::ptr::null_mut();
        };
        if matches!(value, Value::Bytes(_)) {
            unsafe { Py_XIncRef(object) };
            return object;
        }
        if matches!(
            value,
            Value::Int(_) | Value::BigInt(_) | Value::Bool(_) | Value::Str(_)
        ) {
            context.set_error(format!(
                "TypeError: cannot convert '{}' object to bytes",
                cpython_type_name_for_object_ptr(object)
            ));
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            context.set_error("PyBytes_FromObject missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_builtin(BuiltinFunction::Bytes, vec![value], HashMap::new()) {
            Ok(bytes_value @ Value::Bytes(_)) => context.alloc_cpython_ptr_for_value(bytes_value),
            Ok(_) => {
                context.set_error("PyBytes_FromObject expected bytes result");
                std::ptr::null_mut()
            }
            Err(err) => {
                context.set_error(err.message);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

unsafe fn cpython_bytes_join_impl(separator: *mut c_void, iterable: *mut c_void) -> *mut c_void {
    fn bytes_join_value_to_bytes(
        vm: &mut Vm,
        context: &mut ModuleCapiContext,
        value: Value,
        label: &str,
    ) -> Result<Vec<u8>, ()> {
        match value {
            Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(values) => Ok(values.clone()),
                _ => {
                    context.set_error(format!(
                        "PyBytes_Join encountered invalid {label} bytes storage"
                    ));
                    Err(())
                }
            },
            Value::ByteArray(bytearray_obj) => match &*bytearray_obj.kind() {
                Object::ByteArray(values) => Ok(values.clone()),
                _ => {
                    context.set_error(format!(
                        "PyBytes_Join encountered invalid {label} bytearray storage"
                    ));
                    Err(())
                }
            },
            other => match vm.call_builtin(BuiltinFunction::Bytes, vec![other], HashMap::new()) {
                Ok(Value::Bytes(bytes_obj)) => match &*bytes_obj.kind() {
                    Object::Bytes(values) => Ok(values.clone()),
                    _ => {
                        context.set_error(format!(
                            "PyBytes_Join encountered invalid {label} bytes conversion"
                        ));
                        Err(())
                    }
                },
                Ok(_) | Err(_) => {
                    context.set_error(format!(
                        "TypeError: sequence item for PyBytes_Join is not bytes-like ({label})"
                    ));
                    Err(())
                }
            },
        }
    }

    if separator.is_null() || iterable.is_null() {
        unsafe { _PyErr_BadInternalCall(std::ptr::null(), 0) };
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyBytes_Join missing VM context");
            return std::ptr::null_mut();
        }
        let Some(separator_value) = context.cpython_value_from_ptr_or_proxy(separator) else {
            context.set_error("PyBytes_Join received unknown separator pointer");
            return std::ptr::null_mut();
        };
        let Some(iterable_value) = context.cpython_value_from_ptr_or_proxy(iterable) else {
            context.set_error("PyBytes_Join received unknown iterable pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let separator_bytes =
            match bytes_join_value_to_bytes(vm, context, separator_value, "separator") {
                Ok(values) => values,
                Err(()) => return std::ptr::null_mut(),
            };
        let values = match vm.collect_iterable_values(iterable_value) {
            Ok(values) => values,
            Err(err) => {
                context.set_error(err.message);
                return std::ptr::null_mut();
            }
        };
        let mut output = Vec::new();
        for (idx, value) in values.into_iter().enumerate() {
            let bytes = match bytes_join_value_to_bytes(vm, context, value, "item") {
                Ok(values) => values,
                Err(()) => return std::ptr::null_mut(),
            };
            if idx > 0 && !separator_bytes.is_empty() {
                output.extend_from_slice(&separator_bytes);
            }
            output.extend_from_slice(&bytes);
        }
        let joined = vm.heap.alloc(Object::Bytes(output));
        context.alloc_cpython_ptr_for_value(Value::Bytes(joined))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_Join(
    separator: *mut c_void,
    iterable: *mut c_void,
) -> *mut c_void {
    unsafe { cpython_bytes_join_impl(separator, iterable) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyBytes_Join(
    separator: *mut c_void,
    iterable: *mut c_void,
) -> *mut c_void {
    unsafe { cpython_bytes_join_impl(separator, iterable) }
}

unsafe fn cpython_clear_pyobject_ref(slot: *mut *mut c_void) {
    if slot.is_null() {
        return;
    }
    // SAFETY: caller guarantees `slot` is a valid writable pointer location.
    let current = unsafe { *slot };
    if !current.is_null() {
        unsafe { Py_XDecRef(current) };
    }
    // SAFETY: caller guarantees `slot` is writable.
    unsafe {
        *slot = std::ptr::null_mut();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_Concat(pv: *mut *mut c_void, w: *mut c_void) {
    if pv.is_null() {
        cpython_set_error("PyBytes_Concat requires non-null output pointer");
        return;
    }
    // SAFETY: `pv` is checked non-null.
    let left_ptr = unsafe { *pv };
    if left_ptr.is_null() {
        return;
    }
    if w.is_null() {
        unsafe { cpython_clear_pyobject_ref(pv) };
        return;
    }
    let result = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyBytes_Concat missing VM context");
            return std::ptr::null_mut();
        }
        let left_value = match context.cpython_value_from_ptr(left_ptr) {
            Some(value) => value,
            None => {
                context.set_error("PyBytes_Concat received unknown left pointer");
                return std::ptr::null_mut();
            }
        };
        let right_value = match context.cpython_value_from_ptr_or_proxy(w) {
            Some(value) => value,
            None => {
                context.set_error("PyBytes_Concat received unknown right pointer");
                return std::ptr::null_mut();
            }
        };
        let left_bytes = match left_value {
            Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(values) => values.clone(),
                _ => {
                    context.set_error("PyBytes_Concat encountered invalid left bytes storage");
                    return std::ptr::null_mut();
                }
            },
            _ => {
                context.set_error(format!(
                    "TypeError: can't concat {} to {}",
                    cpython_type_name_for_object_ptr(w),
                    cpython_type_name_for_object_ptr(left_ptr)
                ));
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let right_bytes =
            match vm.call_builtin(BuiltinFunction::Bytes, vec![right_value], HashMap::new()) {
                Ok(Value::Bytes(bytes_obj)) => match &*bytes_obj.kind() {
                    Object::Bytes(values) => values.clone(),
                    _ => {
                        context.set_error("PyBytes_Concat encountered invalid right bytes storage");
                        return std::ptr::null_mut();
                    }
                },
                Ok(_) => {
                    context.set_error(format!(
                        "TypeError: can't concat {} to {}",
                        cpython_type_name_for_object_ptr(w),
                        cpython_type_name_for_object_ptr(left_ptr)
                    ));
                    return std::ptr::null_mut();
                }
                Err(_) => {
                    context.set_error(format!(
                        "TypeError: can't concat {} to {}",
                        cpython_type_name_for_object_ptr(w),
                        cpython_type_name_for_object_ptr(left_ptr)
                    ));
                    return std::ptr::null_mut();
                }
            };
        let mut merged = left_bytes;
        merged.extend(right_bytes);
        let merged_obj = vm.heap.alloc(Object::Bytes(merged));
        context.alloc_cpython_ptr_for_value(Value::Bytes(merged_obj))
    });
    let new_ptr = match result {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    };
    if new_ptr.is_null() {
        unsafe { cpython_clear_pyobject_ref(pv) };
        return;
    }
    unsafe {
        Py_XDecRef(left_ptr);
        *pv = new_ptr;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_ConcatAndDel(pv: *mut *mut c_void, w: *mut c_void) {
    unsafe { PyBytes_Concat(pv, w) };
    unsafe { Py_XDecRef(w) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_Resize(pv: *mut *mut c_void, requested_size: isize) -> i32 {
    if pv.is_null() {
        unsafe { _PyErr_BadInternalCall(std::ptr::null(), 0) };
        return -1;
    }
    // SAFETY: `pv` is checked non-null.
    let current = unsafe { *pv };
    if current.is_null() {
        unsafe { _PyErr_BadInternalCall(std::ptr::null(), 0) };
        return -1;
    }
    if requested_size < 0 {
        cpython_set_error("PyBytes_Resize received negative size");
        unsafe { cpython_clear_pyobject_ref(pv) };
        return -1;
    }
    let result = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyBytes_Resize missing VM context");
            return std::ptr::null_mut();
        }
        let Some(current_value) = context.cpython_value_from_ptr(current) else {
            context.set_error("PyBytes_Resize received unknown object pointer");
            return std::ptr::null_mut();
        };
        let current_bytes = match current_value {
            Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(values) => values.clone(),
                _ => {
                    context.set_error("PyBytes_Resize encountered invalid bytes storage");
                    return std::ptr::null_mut();
                }
            },
            _ => {
                context.set_error("PyBytes_Resize expected bytes object");
                return std::ptr::null_mut();
            }
        };
        let mut resized = current_bytes;
        resized.resize(requested_size as usize, 0);
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let bytes_obj = match vm.heap.alloc_bytes(resized) {
            Value::Bytes(obj) => obj,
            _ => unreachable!("heap.alloc_bytes must produce Value::Bytes"),
        };
        context.alloc_cpython_ptr_for_value(Value::Bytes(bytes_obj))
    });
    let new_ptr = match result {
        Ok(ptr) if !ptr.is_null() => ptr,
        Ok(_) => {
            unsafe { cpython_clear_pyobject_ref(pv) };
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            unsafe { cpython_clear_pyobject_ref(pv) };
            return -1;
        }
    };
    unsafe {
        Py_XDecRef(current);
        *pv = new_ptr;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyBytes_Resize(pv: *mut *mut c_void, requested_size: isize) -> i32 {
    unsafe { PyBytes_Resize(pv, requested_size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_Size(object: *mut c_void) -> isize {
    let foreign_bytes_len = |object: *mut c_void| -> Option<isize> {
        if object.is_null() {
            return None;
        }
        // SAFETY: pointer is treated as foreign PyObject; we only inspect fixed headers.
        let head = unsafe { object.cast::<CpythonVarObjectHead>().as_ref() }?;
        let ty = head.ob_base.ob_type.cast::<CpythonTypeObject>();
        if ty.is_null() {
            return None;
        }
        let is_bytes = ty == std::ptr::addr_of_mut!(PyBytes_Type)
            // SAFETY: type pointers are valid for subtype checks.
            || unsafe {
                PyType_IsSubtype(
                    ty.cast::<c_void>(),
                    std::ptr::addr_of_mut!(PyBytes_Type).cast::<c_void>(),
                ) != 0
            };
        if is_bytes {
            return Some(head.ob_size.max(0));
        }
        None
    };
    match cpython_value_from_ptr(object) {
        Ok(Value::Bytes(bytes_obj)) | Ok(Value::ByteArray(bytes_obj)) => match &*bytes_obj.kind() {
            Object::Bytes(values) | Object::ByteArray(values) => values.len() as isize,
            _ => {
                cpython_set_error("PyBytes_Size encountered invalid bytes storage");
                -1
            }
        },
        Ok(_) => {
            if let Some(len) = foreign_bytes_len(object) {
                return len;
            }
            cpython_set_error("PyBytes_Size expected bytes-compatible object");
            -1
        }
        Err(err) => {
            if let Some(len) = foreign_bytes_len(object) {
                return len;
            }
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_AsString(object: *mut c_void) -> *mut c_char {
    let foreign_bytes_payload = |object: *mut c_void| -> Option<*mut c_char> {
        if object.is_null() {
            return None;
        }
        // SAFETY: pointer is treated as foreign PyObject; we only inspect fixed headers.
        let head = unsafe { object.cast::<CpythonVarObjectHead>().as_ref() }?;
        let ty = head.ob_base.ob_type.cast::<CpythonTypeObject>();
        if ty.is_null() {
            return None;
        }
        let is_bytes = ty == std::ptr::addr_of_mut!(PyBytes_Type)
            // SAFETY: type pointers are valid for subtype checks.
            || unsafe {
                PyType_IsSubtype(
                    ty.cast::<c_void>(),
                    std::ptr::addr_of_mut!(PyBytes_Type).cast::<c_void>(),
                ) != 0
            };
        if !is_bytes {
            return None;
        }
        // CPython bytes layout: PyObject_VAR_HEAD + ob_shash + ob_sval[...].
        Some(unsafe {
            object
                .cast::<u8>()
                .add(std::mem::size_of::<CpythonVarObjectHead>() + std::mem::size_of::<isize>())
                .cast::<c_char>()
        })
    };
    match cpython_value_from_ptr(object) {
        Ok(Value::Bytes(bytes_obj)) | Ok(Value::ByteArray(bytes_obj)) => {
            if let Ok(true) = with_active_cpython_context_mut(|context| {
                context.owns_cpython_allocation_ptr(object)
            }) {
                // SAFETY: owned bytes-compatible pointers use CPython bytes layout.
                return unsafe { cpython_bytes_data_ptr(object) };
            }
            match &*bytes_obj.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => {
                    values.as_ptr().cast_mut().cast()
                }
                _ => {
                    cpython_set_error("PyBytes_AsString encountered invalid bytes storage");
                    std::ptr::null_mut()
                }
            }
        }
        Ok(_) => {
            if let Some(ptr) = foreign_bytes_payload(object) {
                return ptr;
            }
            if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() && !object.is_null() {
                // SAFETY: candidate object pointer for diagnostics only.
                let ty = unsafe { (*object.cast::<CpythonObjectHead>()).ob_type };
                let ty_name = unsafe {
                    ty.cast::<CpythonTypeObject>()
                        .as_ref()
                        .and_then(|raw| c_name_to_string(raw.tp_name).ok())
                        .unwrap_or_else(|| "<unknown>".to_string())
                };
                eprintln!(
                    "[cpy-bytes] as_string mismatch object={:p} type={:p} type_name={}",
                    object, ty, ty_name
                );
                if std::env::var_os("PYRS_TRACE_PYBYTES_CALLER_BT").is_some() {
                    let seen = PYBYTES_ASSTRING_MISMATCH_BT_COUNT.fetch_add(1, Ordering::Relaxed);
                    if seen < 8 {
                        eprintln!("[cpy-bytes] mismatch backtrace #{}:", seen + 1);
                        eprintln!("{}", Backtrace::force_capture());
                    }
                }
            }
            cpython_set_error("PyBytes_AsString expected bytes object");
            std::ptr::null_mut()
        }
        Err(err) => {
            if let Some(ptr) = foreign_bytes_payload(object) {
                return ptr;
            }
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_AsStringAndSize(
    object: *mut c_void,
    out_buffer: *mut *mut c_char,
    out_len: *mut isize,
) -> i32 {
    if out_buffer.is_null() || out_len.is_null() {
        cpython_set_error("PyBytes_AsStringAndSize requires non-null out pointers");
        return -1;
    }
    let ptr = unsafe { PyBytes_AsString(object) };
    if ptr.is_null() {
        return -1;
    }
    let len = unsafe { PyBytes_Size(object) };
    if len < 0 {
        return -1;
    }
    // SAFETY: caller provided valid pointers.
    unsafe {
        *out_buffer = ptr;
        *out_len = len;
    }
    0
}

fn cpython_bytes_repr_text(values: &[u8], smartquotes: bool) -> String {
    let use_double_quotes = smartquotes && values.contains(&b'\'') && !values.contains(&b'"');
    let quote = if use_double_quotes { '"' } else { '\'' };
    let mut out = String::with_capacity(values.len() + 8);
    out.push('b');
    out.push(quote);
    for byte in values {
        match *byte {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            b'\'' if quote == '\'' => out.push_str("\\'"),
            b'"' if quote == '"' => out.push_str("\\\""),
            32..=126 => out.push(*byte as char),
            _ => out.push_str(&format!("\\x{:02x}", byte)),
        }
    }
    out.push(quote);
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_Repr(object: *mut c_void, smartquotes: i32) -> *mut c_void {
    let values = match cpython_value_from_ptr(object) {
        Ok(Value::Bytes(obj)) => match &*obj.kind() {
            Object::Bytes(values) => values.clone(),
            _ => {
                cpython_set_error("PyBytes_Repr encountered invalid bytes storage");
                return std::ptr::null_mut();
            }
        },
        Ok(_) => {
            cpython_set_error("PyBytes_Repr expected bytes object");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_new_ptr_for_value(Value::Str(cpython_bytes_repr_text(
        &values,
        smartquotes != 0,
    )))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_DecodeEscape(
    s: *const c_char,
    len: isize,
    errors: *const c_char,
    _unicode: isize,
    _recode_encoding: *const c_char,
) -> *mut c_void {
    if s.is_null() {
        cpython_set_error("PyBytes_DecodeEscape received null input buffer");
        return std::ptr::null_mut();
    }
    if len < 0 {
        cpython_set_error("PyBytes_DecodeEscape received negative length");
        return std::ptr::null_mut();
    }
    let error_mode = if errors.is_null() {
        "strict".to_string()
    } else {
        match unsafe { c_name_to_string(errors) } {
            Ok(name) => name,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    let input_bytes = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len as usize).to_vec() };
    let source_ptr = cpython_new_bytes_ptr(input_bytes);
    if source_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let source_value = match cpython_value_from_ptr(source_ptr) {
        Ok(value) => value,
        Err(err) => {
            unsafe { Py_DecRef(source_ptr) };
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    unsafe { Py_DecRef(source_ptr) };

    let decoded = match cpython_call_builtin(
        BuiltinFunction::CodecsEscapeDecode,
        vec![source_value, Value::Str(error_mode)],
    ) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match decoded {
        Value::Tuple(items) => match &*items.kind() {
            Object::Tuple(values) if !values.is_empty() => {
                cpython_new_ptr_for_value(values[0].clone())
            }
            _ => {
                cpython_set_error("PyBytes_DecodeEscape expected tuple(bytes, consumed)");
                std::ptr::null_mut()
            }
        },
        _ => {
            cpython_set_error("PyBytes_DecodeEscape expected tuple result");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_FromStringAndSize(
    bytes: *const c_char,
    size: isize,
) -> *mut c_void {
    if size < 0 {
        cpython_set_error("Negative size passed to PyByteArray_FromStringAndSize");
        return std::ptr::null_mut();
    }
    let payload = if size == 0 {
        Vec::new()
    } else if bytes.is_null() {
        vec![0; size as usize]
    } else {
        // SAFETY: caller guarantees `bytes` points to at least `size` bytes.
        unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), size as usize).to_vec() }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyByteArray_FromStringAndSize missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        context.alloc_cpython_ptr_for_value(vm.heap.alloc_bytearray(payload))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_FromObject(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::ByteArray, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_Size(object: *mut c_void) -> isize {
    let foreign_bytearray_len = |object: *mut c_void| -> Option<isize> {
        if object.is_null() {
            return None;
        }
        // SAFETY: pointer is treated as foreign PyObject; we only inspect fixed headers.
        let head = unsafe { object.cast::<CpythonVarObjectHead>().as_ref() }?;
        let ty = head.ob_base.ob_type.cast::<CpythonTypeObject>();
        if ty.is_null() {
            return None;
        }
        let is_bytearray = ty == std::ptr::addr_of_mut!(PyByteArray_Type)
            // SAFETY: type pointers are valid for subtype checks.
            || unsafe {
                PyType_IsSubtype(
                    ty.cast::<c_void>(),
                    std::ptr::addr_of_mut!(PyByteArray_Type).cast::<c_void>(),
                ) != 0
            };
        if is_bytearray {
            return Some(head.ob_size.max(0));
        }
        None
    };
    match cpython_value_from_ptr(object) {
        Ok(Value::ByteArray(bytearray_obj)) => match &*bytearray_obj.kind() {
            Object::ByteArray(values) => values.len() as isize,
            _ => {
                cpython_set_error("PyByteArray_Size encountered invalid bytearray storage");
                -1
            }
        },
        Ok(_) => {
            if let Some(len) = foreign_bytearray_len(object) {
                return len;
            }
            cpython_set_error("PyByteArray_Size expected bytearray object");
            -1
        }
        Err(err) => {
            if let Some(len) = foreign_bytearray_len(object) {
                return len;
            }
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_AsString(object: *mut c_void) -> *mut c_char {
    match cpython_value_from_ptr(object) {
        Ok(Value::ByteArray(bytearray_obj)) => {
            if let Ok(true) = with_active_cpython_context_mut(|context| {
                context.owns_cpython_allocation_ptr(object)
            }) {
                // SAFETY: owned bytearray-compatible pointers use bytes-like payload layout.
                return unsafe { cpython_bytes_data_ptr(object) };
            }
            let mut bytes_kind = bytearray_obj.kind_mut();
            match &mut *bytes_kind {
                Object::ByteArray(values) => values.as_mut_ptr().cast(),
                _ => {
                    cpython_set_error("PyByteArray_AsString encountered invalid bytearray storage");
                    std::ptr::null_mut()
                }
            }
        }
        Ok(_) => {
            cpython_set_error("PyByteArray_AsString expected bytearray object");
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_Resize(object: *mut c_void, requested_size: isize) -> i32 {
    if requested_size < 0 {
        cpython_set_error(format!(
            "Can only resize to positive sizes, got {}",
            requested_size
        ));
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyByteArray_Resize missing VM context");
            return -1;
        }
        let Some(handle) = context.cpython_handle_from_ptr(object) else {
            context.set_error("PyByteArray_Resize received unknown object pointer");
            return -1;
        };
        let Some(value) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyByteArray_Resize received unknown object pointer");
            return -1;
        };
        let Value::ByteArray(bytearray_obj) = value else {
            context.set_error("PyByteArray_Resize expected bytearray object");
            return -1;
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        if vm.heap.external_buffer_pin_count_for_source(&bytearray_obj) > 0 {
            context.set_error("BufferError: Existing exports of data: object cannot be re-sized");
            return -1;
        }
        let mut bytearray_kind = bytearray_obj.kind_mut();
        let Object::ByteArray(values) = &mut *bytearray_kind else {
            context.set_error("PyByteArray_Resize encountered invalid bytearray storage");
            return -1;
        };
        let target = requested_size as usize;
        if target >= values.len() {
            values.resize(target, 0);
        } else {
            values.truncate(target);
        }
        drop(bytearray_kind);
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_Concat(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    let mut left_view = CpythonBuffer {
        buf: std::ptr::null_mut(),
        obj: std::ptr::null_mut(),
        len: 0,
        itemsize: 0,
        readonly: 0,
        ndim: 0,
        format: std::ptr::null_mut(),
        shape: std::ptr::null_mut(),
        strides: std::ptr::null_mut(),
        suboffsets: std::ptr::null_mut(),
        internal: std::ptr::null_mut(),
    };
    let mut right_view = CpythonBuffer {
        buf: std::ptr::null_mut(),
        obj: std::ptr::null_mut(),
        len: 0,
        itemsize: 0,
        readonly: 0,
        ndim: 0,
        format: std::ptr::null_mut(),
        shape: std::ptr::null_mut(),
        strides: std::ptr::null_mut(),
        suboffsets: std::ptr::null_mut(),
        internal: std::ptr::null_mut(),
    };
    if unsafe { PyObject_GetBuffer(left, &mut left_view, 0) } != 0
        || unsafe { PyObject_GetBuffer(right, &mut right_view, 0) } != 0
    {
        if !left_view.obj.is_null() {
            unsafe { PyBuffer_Release((&mut left_view as *mut CpythonBuffer).cast()) };
        }
        if !right_view.obj.is_null() {
            unsafe { PyBuffer_Release((&mut right_view as *mut CpythonBuffer).cast()) };
        }
        cpython_set_error(format!(
            "TypeError: can't concat {} to {}",
            cpython_type_name_for_object_ptr(right),
            cpython_type_name_for_object_ptr(left)
        ));
        return std::ptr::null_mut();
    }
    if left_view.len < 0 || right_view.len < 0 {
        unsafe { PyBuffer_Release((&mut left_view as *mut CpythonBuffer).cast()) };
        unsafe { PyBuffer_Release((&mut right_view as *mut CpythonBuffer).cast()) };
        cpython_set_error("PyByteArray_Concat received invalid negative buffer size");
        return std::ptr::null_mut();
    }
    let left_len = left_view.len as usize;
    let right_len = right_view.len as usize;
    let total_len = match left_len.checked_add(right_len) {
        Some(len) => len,
        None => {
            unsafe { PyBuffer_Release((&mut left_view as *mut CpythonBuffer).cast()) };
            unsafe { PyBuffer_Release((&mut right_view as *mut CpythonBuffer).cast()) };
            cpython_set_error("out of memory");
            return std::ptr::null_mut();
        }
    };
    let mut payload = Vec::with_capacity(total_len);
    if left_len > 0 {
        // SAFETY: buffer export guarantees `buf` points to at least `len` readable bytes.
        let left_bytes =
            unsafe { std::slice::from_raw_parts(left_view.buf.cast::<u8>(), left_len) };
        payload.extend_from_slice(left_bytes);
    }
    if right_len > 0 {
        // SAFETY: buffer export guarantees `buf` points to at least `len` readable bytes.
        let right_bytes =
            unsafe { std::slice::from_raw_parts(right_view.buf.cast::<u8>(), right_len) };
        payload.extend_from_slice(right_bytes);
    }
    unsafe { PyBuffer_Release((&mut left_view as *mut CpythonBuffer).cast()) };
    unsafe { PyBuffer_Release((&mut right_view as *mut CpythonBuffer).cast()) };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyByteArray_Concat missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        context.alloc_cpython_ptr_for_value(vm.heap.alloc_bytearray(payload))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}
