use std::backtrace::Backtrace;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};

use crate::runtime::{BuiltinFunction, Object, Value};

use super::{
    ACTIVE_CPYTHON_INIT_CONTEXT, CPY_EXCEPTION_TYPE_PTR_ATTR, CpythonComplexValue,
    CpythonErrorState, CpythonObjectHead, CpythonStructSeqTypeInfo, CpythonStructSequenceDesc,
    CpythonTypeObject, CpythonVarObjectHead, InternalCallOutcome, ModuleCapiContext,
    PY_TPFLAGS_BASETYPE, PY_TPFLAGS_READY, Py_DecRef, Py_IncRef, Py_XDecRef, Py_XIncRef,
    PyDict_Contains, PyDict_New, PyDict_SetItem, PyDict_SetItemString, PyErr_BadInternalCall,
    PyExc_Exception, PyExc_ImportError, PyExc_MemoryError, PyExc_OSError, PyExc_OverflowError,
    PyExc_ResourceWarning, PyExc_RuntimeError, PyExc_RuntimeWarning, PyExc_SystemError,
    PyExc_TypeError, PyExc_ValueError, PyObject_CallObject, PyObject_GetAttrString,
    PyObject_IsSubclass, PyObject_SetAttrString, PyTuple_GetItem, PyTuple_New, PyTuple_SetItem,
    PyTuple_Type, PyType_IsSubtype, PyType_Ready, PyType_Type, PyUnicode_FromString,
    PyUnicode_FromStringAndSize, c_name_to_string, cpython_bigint_low_u64, cpython_bigint_to_u64,
    cpython_call_builtin, cpython_exception_name_parts, cpython_exception_value_from_ptr,
    cpython_foreign_long_to_i64, cpython_foreign_long_to_u64, cpython_is_exception_instance,
    cpython_is_type_object_ptr, cpython_new_ptr_for_value, cpython_set_error,
    cpython_set_typed_error, cpython_structseq_count_fields, cpython_structseq_registry,
    cpython_tuple_items_ptr, cpython_type_name_for_object_ptr, cpython_value_debug_tag,
    cpython_value_from_ptr, cpython_value_from_ptr_or_proxy, value_to_int,
    with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_AsDouble(object: *mut c_void) -> f64 {
    match cpython_value_from_ptr_or_proxy(object) {
        Ok(Value::Float(value)) => value,
        Ok(Value::Int(value)) => value as f64,
        Ok(Value::Bool(value)) => {
            if value {
                1.0
            } else {
                0.0
            }
        }
        Ok(Value::BigInt(value)) => value.to_f64(),
        Ok(value) => match cpython_call_builtin(BuiltinFunction::Float, vec![value]) {
            Ok(Value::Float(value)) => value,
            Ok(Value::Int(value)) => value as f64,
            Ok(Value::Bool(value)) => {
                if value {
                    1.0
                } else {
                    0.0
                }
            }
            Ok(Value::BigInt(value)) => value.to_f64(),
            Ok(_) => {
                cpython_set_error("__float__ returned non-float-compatible result");
                -1.0
            }
            Err(err) => {
                cpython_set_error(err);
                -1.0
            }
        },
        Err(err) => {
            cpython_set_error(err);
            -1.0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_GetMax() -> f64 {
    f64::MAX
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_GetMin() -> f64 {
    f64::MIN_POSITIVE
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_GetInfo() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyFloat_GetInfo missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(sys_module) = vm.modules.get("sys").cloned() else {
            context.set_error("PyFloat_GetInfo missing sys module");
            return std::ptr::null_mut();
        };
        let float_info = match &*sys_module.kind() {
            Object::Module(module_data) => module_data.globals.get("float_info").cloned(),
            _ => None,
        };
        let Some(float_info) = float_info else {
            context.set_error("PyFloat_GetInfo missing sys.float_info");
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(float_info)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsLong(object: *mut c_void) -> i64 {
    match cpython_value_from_ptr(object) {
        Ok(value) => match value_to_int(value) {
            Ok(value) => {
                if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
                    eprintln!(
                        "[cpy-long] mapped value object={:p} value={}",
                        object, value
                    );
                }
                value
            }
            Err(err) => {
                if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
                    eprintln!(
                        "[cpy-long] mapped conversion failed object={:p} err={}",
                        object, err.message
                    );
                }
                cpython_set_error(err.message);
                -1
            }
        },
        Err(err) => {
            if let Some(value) = unsafe { cpython_foreign_long_to_i64(object) } {
                if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
                    eprintln!(
                        "[cpy-long] foreign fallback object={:p} value={}",
                        object, value
                    );
                }
                return value;
            }
            if std::env::var_os("PYRS_TRACE_CPY_LONG").is_some() {
                eprintln!("[cpy-long] foreign fallback failed object={:p}", object);
            }
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsLongLong(object: *mut c_void) -> i64 {
    unsafe { PyLong_AsLong(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsSsize_t(object: *mut c_void) -> isize {
    unsafe { PyLong_AsLong(object) as isize }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLong(object: *mut c_void) -> u64 {
    match cpython_value_from_ptr(object) {
        Ok(Value::Bool(value)) => {
            if value {
                1
            } else {
                0
            }
        }
        Ok(Value::Int(value)) => {
            if value < 0 {
                cpython_set_typed_error(
                    unsafe { PyExc_OverflowError },
                    "can't convert negative value to unsigned int",
                );
                return u64::MAX;
            }
            value as u64
        }
        Ok(Value::BigInt(value)) => {
            if value.is_negative() {
                cpython_set_typed_error(
                    unsafe { PyExc_OverflowError },
                    "can't convert negative value to unsigned int",
                );
                return u64::MAX;
            }
            match cpython_bigint_to_u64(&value) {
                Some(compact) => compact,
                None => {
                    cpython_set_typed_error(
                        unsafe { PyExc_OverflowError },
                        "Python int too large to convert to C unsigned long",
                    );
                    u64::MAX
                }
            }
        }
        Ok(value) => match value_to_int(value) {
            Ok(compact) => {
                if compact < 0 {
                    cpython_set_typed_error(
                        unsafe { PyExc_OverflowError },
                        "can't convert negative value to unsigned int",
                    );
                    return u64::MAX;
                }
                compact as u64
            }
            Err(err) => {
                cpython_set_error(err.message);
                u64::MAX
            }
        },
        Err(err) => {
            if let Some(compact) = unsafe { cpython_foreign_long_to_u64(object) } {
                return compact;
            }
            cpython_set_error(err);
            u64::MAX
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLongLong(object: *mut c_void) -> u64 {
    unsafe { PyLong_AsUnsignedLong(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLongMask(object: *mut c_void) -> u64 {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            if let Some(compact) = unsafe { cpython_foreign_long_to_i64(object) } {
                return compact as u64;
            }
            cpython_set_error(err);
            return u64::MAX;
        }
    };
    let normalized = match value {
        Value::Int(_) | Value::Bool(_) | Value::BigInt(_) => value,
        other => match cpython_call_builtin(BuiltinFunction::Int, vec![other]) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return u64::MAX;
            }
        },
    };
    match normalized {
        Value::Bool(flag) => {
            if flag {
                1
            } else {
                0
            }
        }
        Value::Int(compact) => compact as u64,
        Value::BigInt(bigint) => {
            let lower = cpython_bigint_low_u64(&bigint);
            if bigint.is_negative() {
                (0u64).wrapping_sub(lower)
            } else {
                lower
            }
        }
        _ => u64::MAX,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLongLongMask(object: *mut c_void) -> u64 {
    unsafe { PyLong_AsUnsignedLongMask(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsSize_t(object: *mut c_void) -> usize {
    let value = unsafe { PyLong_AsUnsignedLongLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return usize::MAX;
    }
    if value > usize::MAX as u64 {
        cpython_set_typed_error(
            unsafe { PyExc_OverflowError },
            "Python int too large to convert to C size_t",
        );
        return usize::MAX;
    }
    value as usize
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsInt(object: *mut c_void) -> i32 {
    let value = unsafe { PyLong_AsLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    if value < i32::MIN as i64 || value > i32::MAX as i64 {
        cpython_set_typed_error(
            unsafe { PyExc_OverflowError },
            "Python int too large to convert to C int",
        );
        return -1;
    }
    value as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsInt32(object: *mut c_void, out: *mut i32) -> i32 {
    if out.is_null() {
        cpython_set_error("PyLong_AsInt32 requires non-null output pointer");
        return -1;
    }
    let value = unsafe { PyLong_AsLongLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    if value < i32::MIN as i64 || value > i32::MAX as i64 {
        cpython_set_typed_error(
            unsafe { PyExc_OverflowError },
            "Python int too large to convert to C int32_t",
        );
        return -1;
    }
    // SAFETY: caller provided writable output pointer.
    unsafe { *out = value as i32 };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsInt64(object: *mut c_void, out: *mut i64) -> i32 {
    if out.is_null() {
        cpython_set_error("PyLong_AsInt64 requires non-null output pointer");
        return -1;
    }
    let value = unsafe { PyLong_AsLongLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    // SAFETY: caller provided writable output pointer.
    unsafe { *out = value };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUInt32(object: *mut c_void, out: *mut u32) -> i32 {
    if out.is_null() {
        cpython_set_error("PyLong_AsUInt32 requires non-null output pointer");
        return -1;
    }
    let value = unsafe { PyLong_AsUnsignedLongLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    if value > u32::MAX as u64 {
        cpython_set_typed_error(
            unsafe { PyExc_OverflowError },
            "Python int too large to convert to C uint32_t",
        );
        return -1;
    }
    // SAFETY: caller provided writable output pointer.
    unsafe { *out = value as u32 };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUInt64(object: *mut c_void, out: *mut u64) -> i32 {
    if out.is_null() {
        cpython_set_error("PyLong_AsUInt64 requires non-null output pointer");
        return -1;
    }
    let value = unsafe { PyLong_AsUnsignedLongLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    // SAFETY: caller provided writable output pointer.
    unsafe { *out = value };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsVoidPtr(object: *mut c_void) -> *mut c_void {
    unsafe { PyLong_AsUnsignedLongLong(object) as usize as *mut c_void }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsLongAndOverflow(object: *mut c_void, overflow: *mut i32) -> i64 {
    if !overflow.is_null() {
        // SAFETY: caller provided pointer is writable.
        unsafe { *overflow = 0 };
    }
    match cpython_value_from_ptr(object) {
        Ok(Value::BigInt(value)) => {
            if let Some(compact) = value.to_i64() {
                compact
            } else {
                if !overflow.is_null() {
                    // SAFETY: caller provided pointer is writable.
                    unsafe { *overflow = if value.is_negative() { -1 } else { 1 } };
                }
                -1
            }
        }
        Ok(value) => match value_to_int(value) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err.message);
                -1
            }
        },
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsLongLongAndOverflow(
    object: *mut c_void,
    overflow: *mut i32,
) -> i64 {
    unsafe { PyLong_AsLongAndOverflow(object, overflow) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsDouble(object: *mut c_void) -> f64 {
    match cpython_value_from_ptr(object) {
        Ok(Value::Bool(value)) => {
            if value {
                1.0
            } else {
                0.0
            }
        }
        Ok(Value::Int(value)) => value as f64,
        Ok(Value::BigInt(value)) => {
            let as_double = value.to_f64();
            if !as_double.is_finite() {
                cpython_set_typed_error(
                    unsafe { PyExc_OverflowError },
                    "int too large to convert to float",
                );
                return -1.0;
            }
            as_double
        }
        Ok(value) => match value_to_int(value) {
            Ok(compact) => compact as f64,
            Err(err) => {
                cpython_set_error(err.message);
                -1.0
            }
        },
        Err(err) => {
            if let Some(compact) = unsafe { cpython_foreign_long_to_i64(object) } {
                return compact as f64;
            }
            cpython_set_error(err);
            -1.0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromDouble(value: f64) -> *mut c_void {
    if !value.is_finite() {
        cpython_set_error("PyLong_FromDouble cannot convert inf/nan");
        return std::ptr::null_mut();
    }
    let truncated = value.trunc();
    if truncated < i64::MIN as f64 || truncated > i64::MAX as f64 {
        cpython_set_error("PyLong_FromDouble overflow");
        return std::ptr::null_mut();
    }
    cpython_new_ptr_for_value(Value::Int(truncated as i64))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_FromDoubles(real: f64, imag: f64) -> *mut c_void {
    cpython_new_ptr_for_value(Value::Complex { real, imag })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_FromCComplex(value: CpythonComplexValue) -> *mut c_void {
    unsafe { PyComplex_FromDoubles(value.real, value.imag) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_AsCComplex(object: *mut c_void) -> CpythonComplexValue {
    let err_value = CpythonComplexValue {
        real: -1.0,
        imag: 0.0,
    };
    if object.is_null() {
        cpython_set_error("PyComplex_AsCComplex received null object");
        return err_value;
    }
    match cpython_value_from_ptr(object) {
        Ok(Value::Complex { real, imag }) => CpythonComplexValue { real, imag },
        Ok(Value::Float(real)) => CpythonComplexValue { real, imag: 0.0 },
        Ok(Value::Int(real)) => CpythonComplexValue {
            real: real as f64,
            imag: 0.0,
        },
        Ok(Value::Bool(flag)) => CpythonComplexValue {
            real: if flag { 1.0 } else { 0.0 },
            imag: 0.0,
        },
        Ok(Value::BigInt(real)) => CpythonComplexValue {
            real: real.to_f64(),
            imag: 0.0,
        },
        Ok(_) => {
            // CPython behavior:
            // 1) If __complex__ exists, call it and require a complex result.
            // 2) Otherwise, fall back to PyFloat_AsDouble(op) + 0j.
            let method_name = b"__complex__\0";
            let method = unsafe { PyObject_GetAttrString(object, method_name.as_ptr().cast()) };
            if !method.is_null() {
                let result = unsafe { PyObject_CallObject(method, std::ptr::null_mut()) };
                unsafe { Py_DecRef(method) };
                if result.is_null() {
                    return err_value;
                }
                let complex_value = match cpython_value_from_ptr(result) {
                    Ok(Value::Complex { real, imag }) => CpythonComplexValue { real, imag },
                    Ok(_) => {
                        cpython_set_error("__complex__ returned non-complex object");
                        err_value
                    }
                    Err(err) => {
                        cpython_set_error(err);
                        err_value
                    }
                };
                unsafe { Py_DecRef(result) };
                return complex_value;
            }
            let attribute_missing = with_active_cpython_context_mut(|context| {
                context
                    .last_error
                    .as_deref()
                    .is_some_and(|message| message.contains("has no attribute"))
            })
            .unwrap_or(false);
            if attribute_missing {
                unsafe { PyErr_Clear() };
            } else if !unsafe { PyErr_Occurred() }.is_null() {
                return err_value;
            }
            let real = unsafe { PyFloat_AsDouble(object) };
            CpythonComplexValue { real, imag: 0.0 }
        }
        Err(err) => {
            cpython_set_error(err);
            err_value
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_RealAsDouble(object: *mut c_void) -> f64 {
    unsafe { PyComplex_AsCComplex(object) }.real
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_ImagAsDouble(object: *mut c_void) -> f64 {
    unsafe { PyComplex_AsCComplex(object) }.imag
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyStructSequence_NewType(desc: *mut c_void) -> *mut c_void {
    if desc.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyStructSequence_NewType expected non-null descriptor",
        );
        return std::ptr::null_mut();
    }
    // SAFETY: descriptor pointer is validated non-null.
    let desc_ref = unsafe { &*desc.cast::<CpythonStructSequenceDesc>() };
    if desc_ref.name.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyStructSequence_NewType expected descriptor name",
        );
        return std::ptr::null_mut();
    }
    let type_name = match unsafe { c_name_to_string(desc_ref.name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(format!("PyStructSequence_NewType invalid name: {err}"));
            return std::ptr::null_mut();
        }
    };
    let owned_name = match CString::new(type_name.clone()) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(format!("PyStructSequence_NewType invalid name: {err}"));
            return std::ptr::null_mut();
        }
    };
    let field_count = match cpython_structseq_count_fields(desc_ref.fields) {
        Ok(count) => count,
        Err(err) => {
            cpython_set_typed_error(unsafe { PyExc_SystemError }, err);
            return std::ptr::null_mut();
        }
    };
    let visible_count = if desc_ref.n_in_sequence < 0 {
        field_count
    } else {
        (desc_ref.n_in_sequence as usize).min(field_count)
    };

    // SAFETY: static tuple type can be copied by value to seed a heap-like type shell.
    let mut type_value = unsafe { std::ptr::read(std::ptr::addr_of!(PyTuple_Type)) };
    type_value.ob_refcnt = 1;
    type_value.ob_type = std::ptr::addr_of_mut!(PyType_Type).cast();
    type_value.ob_size = 0;
    type_value.tp_name = owned_name.as_ptr();
    type_value.tp_doc = desc_ref.doc;
    type_value.tp_base = std::ptr::addr_of_mut!(PyTuple_Type);
    type_value.tp_members = std::ptr::null_mut();
    type_value.tp_dict = std::ptr::null_mut();
    type_value.tp_flags |= PY_TPFLAGS_BASETYPE;
    type_value.tp_flags &= !PY_TPFLAGS_READY;

    let type_ptr = Box::into_raw(Box::new(type_value));
    if unsafe { PyType_Ready(type_ptr.cast()) } != 0 {
        // SAFETY: type_ptr allocated above and not published on failure path.
        unsafe {
            let _ = Box::from_raw(type_ptr);
        }
        return std::ptr::null_mut();
    }
    if let Ok(mut registry) = cpython_structseq_registry().lock() {
        registry.insert(
            type_ptr as usize,
            CpythonStructSeqTypeInfo {
                field_count,
                _visible_count: visible_count,
                _name: owned_name,
            },
        );
    }
    type_ptr.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyStructSequence_New(type_obj: *mut c_void) -> *mut c_void {
    if type_obj.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyStructSequence_New expected non-null type",
        );
        return std::ptr::null_mut();
    }
    let field_count = cpython_structseq_registry()
        .lock()
        .ok()
        .and_then(|registry| {
            registry
                .get(&(type_obj as usize))
                .map(|entry| entry.field_count)
        })
        .unwrap_or(0);
    unsafe { PyTuple_New(field_count as isize) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyStructSequence_SetItem(
    object: *mut c_void,
    index: isize,
    value: *mut c_void,
) {
    if object.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyStructSequence_SetItem expected non-null object",
        );
        return;
    }
    let _ = unsafe { PyTuple_SetItem(object, index, value) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyStructSequence_GetItem(
    object: *mut c_void,
    index: isize,
) -> *mut c_void {
    if object.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyStructSequence_GetItem expected non-null object",
        );
        return std::ptr::null_mut();
    }
    unsafe { PyTuple_GetItem(object, index) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_set_error_message(message: *const c_char) {
    if message.is_null() {
        cpython_set_error("received null error message from C shim");
        return;
    }
    // SAFETY: caller provides a valid NUL-terminated error string.
    let text = unsafe { CStr::from_ptr(message) };
    match text.to_str() {
        Ok(message) => cpython_set_error(message.to_string()),
        Err(_) => cpython_set_error("received invalid UTF-8 error message from C shim"),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_tuple_pack_from_array(
    size: isize,
    items: *const *mut c_void,
) -> *mut c_void {
    if size < 0 {
        cpython_set_error("PyTuple_Pack requires non-negative size");
        return std::ptr::null_mut();
    }
    let tuple = unsafe { PyTuple_New(size) };
    if tuple.is_null() {
        return std::ptr::null_mut();
    }
    if size == 0 {
        return tuple;
    }
    if items.is_null() {
        cpython_set_error("PyTuple_Pack received null items array");
        unsafe { Py_DecRef(tuple) };
        return std::ptr::null_mut();
    }
    for idx in 0..(size as usize) {
        // SAFETY: `items` has at least `size` entries supplied by the C shim.
        let item = unsafe { *items.add(idx) };
        if item.is_null() {
            cpython_set_error("PyTuple_Pack received null item pointer");
            unsafe { Py_DecRef(tuple) };
            return std::ptr::null_mut();
        }
        // PyTuple_Pack consumes borrowed inputs, so incref before handing off to
        // PyTuple_SetItem (which steals one reference by CPython contract).
        unsafe { Py_XIncRef(item) };
        if unsafe { PyTuple_SetItem(tuple, idx as isize, item) } != 0 {
            unsafe { Py_DecRef(tuple) };
            return std::ptr::null_mut();
        }
    }
    tuple
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetString(_exception: *mut c_void, message: *const c_char) {
    match unsafe { c_name_to_string(message) } {
        Ok(message) => {
            if std::env::var_os("PYRS_TRACE_NUMPY_DTYPE").is_some() && message.contains("data type")
            {
                eprintln!(
                    "[cpy-dtype] PyErr_SetString exc={:p} msg={} bt={:?}",
                    _exception,
                    message,
                    Backtrace::force_capture()
                );
            }
            if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some()
                && message.starts_with("cannot add indexed loop to ufunc")
            {
                let _ = with_active_cpython_context_mut(|context| {
                    if let Some(previous) = context.last_error.as_ref() {
                        eprintln!("[cpy-err-prev] {previous}");
                    } else {
                        eprintln!("[cpy-err-prev] <none>");
                    }
                });
            }
            if std::env::var_os("PYRS_TRACE_DOCSTRING_ERRORS").is_some()
                && message == "Cannot set a docstring for that object"
            {
                eprintln!(
                    "[cpy-doc-error] exc={:p} message={} bt={:?}",
                    _exception,
                    message,
                    Backtrace::force_capture()
                );
            }
            if std::env::var_os("PYRS_TRACE_NUMPY_PICKLE_FAIL").is_some()
                && message.starts_with("Unable to initialize pickling for ")
            {
                eprintln!(
                    "[numpy-pickle-fail] from-PyErr_SetString message={} bt={:?}",
                    message,
                    Backtrace::force_capture()
                );
            }
            let _ = with_active_cpython_context_mut(|context| {
                let ptype = if _exception.is_null() {
                    unsafe { PyExc_RuntimeError }
                } else {
                    _exception
                };
                context.set_error_state(ptype, std::ptr::null_mut(), std::ptr::null_mut(), message);
            })
            .map_err(|err| {
                cpython_set_error(err);
            });
        }
        Err(err) => cpython_set_error(format!("PyErr_SetString invalid message: {err}")),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_NewException(
    name: *const c_char,
    mut base: *mut c_void,
    mut dict: *mut c_void,
) -> *mut c_void {
    if name.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let name_text = match unsafe { c_name_to_string(name) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(format!("PyErr_NewException invalid name: {err}"));
            return std::ptr::null_mut();
        }
    };
    let Some((module_name, class_name)) = cpython_exception_name_parts(&name_text) else {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyErr_NewException: name must be module.class",
        );
        return std::ptr::null_mut();
    };
    if base.is_null() {
        base = unsafe { PyExc_Exception };
    }

    let mut mydict: *mut c_void = std::ptr::null_mut();
    let mut modulename_obj: *mut c_void = std::ptr::null_mut();
    let module_key = unsafe { PyUnicode_FromString(c"__module__".as_ptr()) };
    if module_key.is_null() {
        return std::ptr::null_mut();
    }

    if dict.is_null() {
        dict = unsafe { PyDict_New() };
        if dict.is_null() {
            unsafe { Py_DecRef(module_key) };
            return std::ptr::null_mut();
        }
        mydict = dict;
    }

    let mut contains_module = unsafe { PyDict_Contains(dict, module_key) };
    if contains_module < 0 {
        unsafe {
            Py_DecRef(module_key);
            Py_XDecRef(mydict);
        }
        return std::ptr::null_mut();
    }
    if contains_module == 0 {
        modulename_obj = unsafe {
            PyUnicode_FromStringAndSize(module_name.as_ptr().cast(), module_name.len() as isize)
        };
        if modulename_obj.is_null() {
            unsafe {
                Py_DecRef(module_key);
                Py_XDecRef(mydict);
            }
            return std::ptr::null_mut();
        }
        if unsafe { PyDict_SetItem(dict, module_key, modulename_obj) } != 0 {
            unsafe {
                Py_DecRef(module_key);
                Py_XDecRef(modulename_obj);
                Py_XDecRef(mydict);
            }
            return std::ptr::null_mut();
        }
        contains_module = 1;
    }
    debug_assert!(contains_module == 1);

    let result = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return Err("missing VM context for PyErr_NewException".to_string());
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let base_value = context
            .cpython_value_from_ptr_or_proxy(base)
            .ok_or_else(|| "PyErr_NewException received invalid base object".to_string())?;
        let bases = match base_value {
            Value::Tuple(_) | Value::List(_) => base_value,
            other => vm.heap.alloc_tuple(vec![other]),
        };
        let namespace = context
            .cpython_value_from_ptr_or_proxy(dict)
            .ok_or_else(|| "PyErr_NewException received invalid dict object".to_string())?;
        let class_value = match vm.call_internal(
            Value::Builtin(BuiltinFunction::Type),
            vec![Value::Str(class_name.to_string()), bases, namespace],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(value)) => value,
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                return Err(vm
                    .runtime_error_from_active_exception("PyErr_NewException failed")
                    .message);
            }
            Err(err) => return Err(err.message),
        };
        Ok(context.alloc_cpython_ptr_for_value(class_value))
    })
    .unwrap_or_else(|err| Err(err.to_string()));

    let result = match result {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    };
    unsafe {
        Py_DecRef(module_key);
        Py_XDecRef(modulename_obj);
        Py_XDecRef(mydict);
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_NewExceptionWithDoc(
    name: *const c_char,
    doc: *const c_char,
    base: *mut c_void,
    mut dict: *mut c_void,
) -> *mut c_void {
    let mut mydict: *mut c_void = std::ptr::null_mut();
    if dict.is_null() {
        dict = unsafe { PyDict_New() };
        if dict.is_null() {
            return std::ptr::null_mut();
        }
        mydict = dict;
    }

    if !doc.is_null() {
        let doc_obj = unsafe { PyUnicode_FromString(doc) };
        if doc_obj.is_null() {
            unsafe { Py_XDecRef(mydict) };
            return std::ptr::null_mut();
        }
        let status = unsafe { PyDict_SetItemString(dict, c"__doc__".as_ptr(), doc_obj) };
        unsafe { Py_DecRef(doc_obj) };
        if status != 0 {
            unsafe { Py_XDecRef(mydict) };
            return std::ptr::null_mut();
        }
    }

    let result = unsafe { PyErr_NewException(name, base, dict) };
    unsafe { Py_XDecRef(mydict) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyExceptionClass_Name(exception_class: *mut c_void) -> *const c_char {
    match with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr_or_proxy(exception_class) else {
            context.set_error("PyExceptionClass_Name received unknown object pointer");
            return std::ptr::null();
        };
        let name = match value {
            Value::ExceptionType(name) => name,
            Value::Class(class_obj) => match &*class_obj.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => {
                    context.set_error("PyExceptionClass_Name expected exception class object");
                    return std::ptr::null();
                }
            },
            _ => {
                context.set_error("PyExceptionClass_Name expected exception class");
                return std::ptr::null();
            }
        };
        context
            .scratch_c_string_ptr(&name)
            .unwrap_or(std::ptr::null())
    }) {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Occurred() -> *mut c_void {
    match with_active_cpython_context_mut(|context| {
        let ptr = context
            .current_error
            .as_ref()
            .map_or(std::ptr::null_mut(), |state| state.ptype);
        if std::env::var_os("PYRS_TRACE_PYERR_OCCURRED").is_some() && !ptr.is_null() {
            let active = ACTIVE_CPYTHON_INIT_CONTEXT.with(|cell| cell.get());
            eprintln!(
                "[cpy-err-occurred] active_ctx={:p} ctx={:p} ptype={:p} last_error={:?}",
                active, context as *mut ModuleCapiContext, ptr, context.last_error
            );
        }
        ptr
    }) {
        Ok(ptr) => ptr,
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Clear() {
    let _ = with_active_cpython_context_mut(|context| {
        if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() && context.last_error.is_some() {
            if let Some(previous) = context.last_error.as_ref() {
                eprintln!("[cpy-err-clear] clearing: {previous}");
            }
        }
        context.clear_error();
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_ExceptionMatches(_exception: *mut c_void) -> i32 {
    let occurred = unsafe { PyErr_Occurred() };
    if occurred.is_null() {
        return 0;
    }
    unsafe { PyErr_GivenExceptionMatches(occurred, _exception) }
}

pub(in crate::vm::vm_extensions) fn cpython_ptr_is_type_object(ptr: *mut c_void) -> bool {
    cpython_is_type_object_ptr(ptr)
}

fn cpython_probable_c_string_ptr(ptr: *const c_char) -> bool {
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if ptr.is_null() {
        return false;
    }
    let addr = ptr as usize;
    addr >= MIN_VALID_PTR && addr % std::mem::align_of::<usize>() == 0
}

fn cpython_safe_type_name(type_ptr: *mut CpythonTypeObject) -> Option<String> {
    if type_ptr.is_null() {
        return None;
    }
    // SAFETY: caller provides a candidate type pointer; this function performs
    // conservative pointer checks before touching foreign string memory.
    unsafe {
        let ty = type_ptr.as_ref()?;
        if !cpython_probable_c_string_ptr(ty.tp_name) {
            return None;
        }
        c_name_to_string(ty.tp_name).ok()
    }
}

pub(in crate::vm::vm_extensions) fn cpython_safe_object_type_name(
    object: *mut c_void,
) -> Option<String> {
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if object.is_null() {
        return None;
    }
    let addr = object as usize;
    if addr < MIN_VALID_PTR || addr % std::mem::align_of::<usize>() != 0 {
        return None;
    }
    // SAFETY: pointer passes conservative shape checks; read-only access to object head.
    unsafe {
        let head = object.cast::<CpythonObjectHead>().as_ref()?;
        cpython_safe_type_name(head.ob_type.cast::<CpythonTypeObject>())
    }
}

pub(in crate::vm::vm_extensions) fn cpython_exception_type_ptr(ptr: *mut c_void) -> *mut c_void {
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    if cpython_ptr_is_type_object(ptr) {
        return ptr;
    }
    // SAFETY: pointer is inspected as a CPython object header.
    unsafe {
        ptr.cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type)
            .unwrap_or(std::ptr::null_mut())
    }
}

pub(in crate::vm::vm_extensions) fn cpython_exception_class_name_from_ptr(
    ptr: *mut c_void,
) -> Option<String> {
    let type_ptr = cpython_exception_type_ptr(ptr);
    if type_ptr.is_null() || !cpython_ptr_is_type_object(type_ptr) {
        return None;
    }
    let name = cpython_safe_type_name(type_ptr.cast::<CpythonTypeObject>())?;
    if name.is_empty() || name == "type" {
        None
    } else {
        Some(name)
    }
}

fn cpython_exception_expected_name_from_ptr(ptr: *mut c_void) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    if let Some(Value::ExceptionType(name)) = cpython_exception_value_from_ptr(ptr as usize) {
        return Some(name);
    }
    cpython_exception_class_name_from_ptr(ptr)
}

fn cpython_type_inherits_exception_name(type_ptr: *mut c_void, expected_name: &str) -> bool {
    if type_ptr.is_null() || expected_name.is_empty() {
        return false;
    }
    let mut depth = 0usize;
    let mut current = type_ptr.cast::<CpythonTypeObject>();
    while !current.is_null() && depth < 128 {
        if !cpython_ptr_is_type_object(current.cast()) {
            return false;
        }
        let current_name = cpython_safe_type_name(current).unwrap_or_default();
        if current_name == expected_name {
            return true;
        }
        // SAFETY: `current` is non-null; reading `tp_base` is valid for CPython type layouts.
        current = unsafe {
            current
                .as_ref()
                .map(|ty| ty.tp_base)
                .unwrap_or(std::ptr::null_mut())
        };
        depth += 1;
    }
    false
}

fn cpython_tuple_items_for_match(tuple: *mut c_void) -> Option<Vec<*mut c_void>> {
    if tuple.is_null() {
        return None;
    }
    let tuple_type = std::ptr::addr_of_mut!(PyTuple_Type).cast::<c_void>();
    // SAFETY: pointer is inspected as CPython object header for tuple type checks.
    let ty = unsafe {
        tuple
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type)
            .unwrap_or(std::ptr::null_mut())
    };
    if ty.is_null() {
        return None;
    }
    let is_tuple = ty == tuple_type
        // SAFETY: both pointers are valid type objects for subtype checks.
        || unsafe { PyType_IsSubtype(ty, tuple_type) != 0 };
    if !is_tuple {
        return None;
    }
    // SAFETY: tuple pointer has CPython tuple layout with contiguous items.
    let len = unsafe {
        tuple
            .cast::<CpythonVarObjectHead>()
            .as_ref()
            .map(|head| head.ob_size.max(0) as usize)
            .unwrap_or(0)
    };
    // SAFETY: tuple pointer has CPython tuple layout.
    let item_ptr = unsafe { cpython_tuple_items_ptr(tuple) };
    if item_ptr.is_null() {
        return Some(Vec::new());
    }
    let mut items = Vec::with_capacity(len);
    // SAFETY: tuple stores at least `len` item pointers.
    unsafe {
        for idx in 0..len {
            items.push(*item_ptr.add(idx));
        }
    }
    Some(items)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_GivenExceptionMatches(
    given: *mut c_void,
    expected: *mut c_void,
) -> i32 {
    if given.is_null() || expected.is_null() {
        return 0;
    }
    if given == expected {
        return 1;
    }
    if let Some(items) = cpython_tuple_items_for_match(expected) {
        for item in items {
            if unsafe { PyErr_GivenExceptionMatches(given, item) } != 0 {
                return 1;
            }
        }
        return 0;
    }
    let given_type = cpython_exception_type_ptr(given);
    let expected_type = cpython_exception_type_ptr(expected);
    if given_type.is_null() || expected_type.is_null() {
        return 0;
    }
    if given_type == expected_type {
        return 1;
    }
    // SAFETY: both pointers refer to CPython-compatible type objects.
    if unsafe { PyType_IsSubtype(given_type, expected_type) } != 0 {
        return 1;
    }
    if let Some(expected_name) = cpython_exception_expected_name_from_ptr(expected)
        && cpython_type_inherits_exception_name(given_type, &expected_name)
    {
        return 1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Fetch(
    ptype: *mut *mut c_void,
    pvalue: *mut *mut c_void,
    ptraceback: *mut *mut c_void,
) {
    let state = with_active_cpython_context_mut(|context| context.fetch_error_state()).unwrap_or(
        CpythonErrorState {
            ptype: std::ptr::null_mut(),
            pvalue: std::ptr::null_mut(),
            ptraceback: std::ptr::null_mut(),
        },
    );
    if !ptype.is_null() {
        // SAFETY: caller provided writable error-type output pointer.
        unsafe { *ptype = state.ptype };
    }
    if !pvalue.is_null() {
        // SAFETY: caller provided writable error-value output pointer.
        unsafe { *pvalue = state.pvalue };
    }
    if !ptraceback.is_null() {
        // SAFETY: caller provided writable traceback output pointer.
        unsafe { *ptraceback = state.ptraceback };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Restore(
    ptype: *mut c_void,
    pvalue: *mut c_void,
    _ptraceback: *mut c_void,
) {
    let _ = with_active_cpython_context_mut(|context| {
        context.restore_error_state(CpythonErrorState {
            ptype,
            pvalue,
            ptraceback: _ptraceback,
        });
    })
    .map_err(|err| {
        cpython_set_error(err);
    });
}

pub(in crate::vm::vm_extensions) fn cpython_exception_type_ptr_for_value(
    context: &mut ModuleCapiContext,
    value: &Value,
) -> Option<*mut c_void> {
    match value {
        Value::Exception(exception_obj) => {
            let attr_hint = exception_obj
                .attrs
                .borrow()
                .get(CPY_EXCEPTION_TYPE_PTR_ATTR)
                .cloned();
            if std::env::var_os("PYRS_TRACE_CPY_EXC_TYPE_HINT").is_some() {
                let map_hit = context
                    .exception_type_ptr_by_name
                    .get(&exception_obj.name)
                    .copied();
                eprintln!(
                    "[cpy-exc-type] name={} attr_hint={attr_hint:?} map_hit={map_hit:?}",
                    exception_obj.name
                );
            }
            if let Some(Value::Int(raw)) = attr_hint
                && raw > 0
            {
                return Some(raw as usize as *mut c_void);
            }
            if let Some(raw_ptr) = context.exception_type_ptr_by_name.get(&exception_obj.name) {
                return Some(*raw_ptr as *mut c_void);
            }
            if context.vm.is_null() {
                return None;
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *context.vm };
            let class = vm.alloc_synthetic_exception_class(&exception_obj.name);
            Some(context.alloc_cpython_ptr_for_value(Value::Class(class)))
        }
        Value::Instance(instance) => {
            let hint = {
                let Object::Instance(instance_data) = &*instance.kind() else {
                    return None;
                };
                instance_data
                    .attrs
                    .get(CPY_EXCEPTION_TYPE_PTR_ATTR)
                    .cloned()
            };
            if let Some(Value::Int(raw)) = hint
                && raw > 0
            {
                return Some(raw as usize as *mut c_void);
            }
            if !cpython_is_exception_instance(context, instance) {
                let instance_name = {
                    let Object::Instance(instance_data) = &*instance.kind() else {
                        return None;
                    };
                    let Object::Class(class_data) = &*instance_data.class.kind() else {
                        return None;
                    };
                    class_data.name.clone()
                };
                if let Some(raw_ptr) = context.exception_type_ptr_by_name.get(&instance_name) {
                    return Some(*raw_ptr as *mut c_void);
                }
                return None;
            }
            let Object::Instance(instance_data) = &*instance.kind() else {
                return None;
            };
            let class = instance_data.class.clone();
            Some(context.alloc_cpython_ptr_for_value(Value::Class(class)))
        }
        Value::ExceptionType(name) => {
            if context.vm.is_null() {
                return None;
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *context.vm };
            let class = vm.alloc_synthetic_exception_class(name);
            Some(context.alloc_cpython_ptr_for_value(Value::Class(class)))
        }
        Value::Class(class) => {
            Some(context.alloc_cpython_ptr_for_value(Value::Class(class.clone())))
        }
        _ => None,
    }
}

pub(in crate::vm::vm_extensions) fn cpython_exception_traceback_ptr_for_value(
    context: &mut ModuleCapiContext,
    value: &Value,
) -> Option<*mut c_void> {
    match value {
        Value::Exception(exception_obj) => exception_obj
            .attrs
            .borrow()
            .get("__traceback__")
            .cloned()
            .or_else(|| exception_obj.attrs.borrow().get("exc_traceback").cloned())
            .filter(|tb| !matches!(tb, Value::None))
            .map(|tb| context.alloc_cpython_ptr_for_value(tb)),
        Value::Instance(instance) => {
            if !cpython_is_exception_instance(context, instance) {
                return None;
            }
            let Object::Instance(instance_data) = &*instance.kind() else {
                return None;
            };
            instance_data
                .attrs
                .get("__traceback__")
                .cloned()
                .or_else(|| instance_data.attrs.get("exc_traceback").cloned())
                .filter(|tb| !matches!(tb, Value::None))
                .map(|tb| context.alloc_cpython_ptr_for_value(tb))
        }
        _ => None,
    }
}

pub(in crate::vm::vm_extensions) fn cpython_make_exception_instance_from_type_and_value(
    context: &mut ModuleCapiContext,
    ptype: *mut c_void,
    pvalue: Option<Value>,
) -> Option<*mut c_void> {
    if context.vm.is_null() || ptype.is_null() {
        return None;
    }
    // SAFETY: VM pointer is valid for active C-API context lifetime.
    let vm = unsafe { &mut *context.vm };
    let callable = if let Some(Value::ExceptionType(name)) =
        cpython_exception_value_from_ptr(ptype as usize)
    {
        Value::Class(vm.alloc_synthetic_exception_class(&name))
    } else {
        match context.cpython_value_from_ptr_or_proxy(ptype)? {
            Value::Class(class) => Value::Class(class),
            Value::ExceptionType(name) => Value::Class(vm.alloc_synthetic_exception_class(&name)),
            _ => return None,
        }
    };
    let args = match pvalue {
        Some(Value::Tuple(tuple_obj)) => match &*tuple_obj.kind() {
            Object::Tuple(values) => values.clone(),
            _ => Vec::new(),
        },
        Some(Value::None) | None => Vec::new(),
        Some(value) => vec![value],
    };
    match vm.call_internal(callable, args, HashMap::new()) {
        Ok(InternalCallOutcome::Value(value)) => match vm.normalize_exception_value(value) {
            Ok(value) => Some(context.alloc_cpython_ptr_for_value(value)),
            Err(_) => None,
        },
        _ => None,
    }
}

fn cpython_raised_exception_ptr_from_state(
    context: &mut ModuleCapiContext,
    state: CpythonErrorState,
) -> *mut c_void {
    if state.ptype.is_null() && state.pvalue.is_null() && state.ptraceback.is_null() {
        return std::ptr::null_mut();
    }
    let value = if !state.pvalue.is_null() {
        context.cpython_value_from_ptr_or_proxy(state.pvalue)
    } else {
        None
    };
    if let Some(value) = value.as_ref() {
        if cpython_is_exception_value(context, value) {
            return context.alloc_cpython_ptr_for_value(value.clone());
        }
    }
    if let Some(ptr) =
        cpython_make_exception_instance_from_type_and_value(context, state.ptype, value.clone())
    {
        return ptr;
    }
    if let Some(value) = value {
        return context.alloc_cpython_ptr_for_value(value);
    }
    if !state.ptype.is_null() {
        return state.ptype;
    }
    std::ptr::null_mut()
}

fn cpython_is_exception_value(context: &ModuleCapiContext, value: &Value) -> bool {
    match value {
        Value::Exception(_) | Value::ExceptionType(_) => true,
        Value::Instance(instance) => cpython_is_exception_instance(context, instance),
        _ => false,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_GetRaisedException() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let state = context.fetch_error_state();
        cpython_raised_exception_ptr_from_state(context, state)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetRaisedException(exc: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        if exc.is_null() {
            context.clear_error();
            return;
        }
        let message = context.error_message_from_ptr(exc);
        let ptype = cpython_exception_type_ptr(exc);
        if ptype.is_null() {
            context.set_error("PyErr_SetRaisedException expected exception object");
            return;
        }
        let ptraceback = context
            .cpython_value_from_ptr_or_proxy(exc)
            .as_ref()
            .and_then(|value| cpython_exception_traceback_ptr_for_value(context, value))
            .unwrap_or(std::ptr::null_mut());
        context.set_error_state(ptype, exc, ptraceback, message);
    })
    .map_err(cpython_set_error);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_GetHandledException() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        context
            .handled_exception_get()
            .map(|value| context.alloc_cpython_ptr_for_value(value))
            .unwrap_or(std::ptr::null_mut())
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetHandledException(exc: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        if exc.is_null() {
            context.handled_exception_set(None);
            return;
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(exc) else {
            context.set_error("PyErr_SetHandledException received unknown exception pointer");
            return;
        };
        if matches!(value, Value::None) {
            context.handled_exception_set(None);
            return;
        }
        let normalized = if context.vm.is_null() {
            value
        } else {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *context.vm };
            match vm.normalize_exception_value(value) {
                Ok(value) => value,
                Err(err) => {
                    context.set_error(err.message);
                    return;
                }
            }
        };
        if !cpython_is_exception_value(context, &normalized) {
            context.set_error("PyErr_SetHandledException expected exception object");
            return;
        }
        context.handled_exception_set(Some(normalized));
    })
    .map_err(cpython_set_error);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_GetExcInfo(
    p_type: *mut *mut c_void,
    p_value: *mut *mut c_void,
    p_traceback: *mut *mut c_void,
) {
    let _ = with_active_cpython_context_mut(|context| {
        let handled = context.handled_exception_get();
        if !p_type.is_null() {
            let value = handled
                .as_ref()
                .and_then(|value| cpython_exception_type_ptr_for_value(context, value))
                .unwrap_or_else(|| context.alloc_cpython_ptr_for_value(Value::None));
            // SAFETY: caller provided writable output pointer.
            unsafe { *p_type = value };
        }
        if !p_value.is_null() {
            let value = handled
                .clone()
                .map(|value| context.alloc_cpython_ptr_for_value(value))
                .unwrap_or(std::ptr::null_mut());
            // SAFETY: caller provided writable output pointer.
            unsafe { *p_value = value };
        }
        if !p_traceback.is_null() {
            let value = handled
                .as_ref()
                .and_then(|value| cpython_exception_traceback_ptr_for_value(context, value))
                .unwrap_or_else(|| context.alloc_cpython_ptr_for_value(Value::None));
            // SAFETY: caller provided writable output pointer.
            unsafe { *p_traceback = value };
        }
    })
    .map_err(cpython_set_error);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcInfo(
    p_type: *mut c_void,
    p_value: *mut c_void,
    p_traceback: *mut c_void,
) {
    unsafe { PyErr_SetHandledException(p_value) };
    // Keep CPython ownership semantics: arguments are stolen.
    unsafe {
        Py_XDecRef(p_value);
        Py_XDecRef(p_type);
        Py_XDecRef(p_traceback);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_pyerr_format_fallback(
    exception: *mut c_void,
    format: *const c_char,
) -> *mut c_void {
    let message = if format.is_null() {
        "error".to_string()
    } else {
        unsafe { CStr::from_ptr(format) }
            .to_str()
            .unwrap_or("error")
            .to_string()
    };
    if std::env::var_os("PYRS_TRACE_NUMPY_DTYPE").is_some() && message.contains("data type") {
        eprintln!(
            "[cpy-dtype] PyErr_Format exception={:p} msg={} bt={:?}",
            exception,
            message,
            Backtrace::force_capture()
        );
    }
    if std::env::var_os("PYRS_TRACE_NUMPY_PICKLE_FAIL").is_some()
        && message.starts_with("Unable to initialize pickling for ")
    {
        eprintln!(
            "[numpy-pickle-fail] from-PyErr_Format message={} bt={:?}",
            message,
            Backtrace::force_capture()
        );
    }
    if exception.is_null() {
        cpython_set_error(message);
    } else {
        cpython_set_typed_error(exception, message);
    }
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_pyerr_formatv_fallback(
    exception: *mut c_void,
    format: *const c_char,
    vargs: *mut c_void,
) -> *mut c_void {
    let _ = vargs;
    unsafe { pyrs_capi_pyerr_format_fallback(exception, format) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_NormalizeException(
    _ptype: *mut *mut c_void,
    _pvalue: *mut *mut c_void,
    _ptraceback: *mut *mut c_void,
) {
}

fn cpython_optional_filename_from_c(name: *const c_char) -> Option<String> {
    if name.is_null() {
        return None;
    }
    unsafe { c_name_to_string(name) }.ok()
}

fn cpython_optional_filename_from_object(name: *mut c_void) -> Option<String> {
    if name.is_null() {
        return None;
    }
    with_active_cpython_context_mut(|context| {
        let value = context.cpython_value_from_ptr_or_proxy(name)?;
        match value {
            Value::Str(text) => Some(text),
            Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
                _ => None,
            },
            Value::ByteArray(bytes_obj) => match &*bytes_obj.kind() {
                Object::ByteArray(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
                _ => None,
            },
            _ => None,
        }
    })
    .ok()
    .flatten()
}

fn cpython_set_os_error_message(
    exception: *mut c_void,
    code: Option<i32>,
    filename: Option<String>,
    filename2: Option<String>,
) {
    let mut message = match code {
        Some(code) => format!("system error {code}"),
        None => "system error".to_string(),
    };
    if let Some(filename) = filename {
        message.push_str(&format!(": {filename}"));
    }
    if let Some(filename2) = filename2 {
        message.push_str(&format!(" -> {filename2}"));
    }
    let exception = if exception.is_null() {
        unsafe { PyExc_OSError }
    } else {
        exception
    };
    cpython_set_typed_error(exception, message);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrno(exception: *mut c_void) -> *mut c_void {
    cpython_set_os_error_message(exception, None, None, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrnoWithFilename(
    exception: *mut c_void,
    filename: *const c_char,
) -> *mut c_void {
    let filename = cpython_optional_filename_from_c(filename);
    cpython_set_os_error_message(exception, None, filename, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrnoWithFilenameObject(
    exception: *mut c_void,
    filename: *mut c_void,
) -> *mut c_void {
    let filename = cpython_optional_filename_from_object(filename);
    cpython_set_os_error_message(exception, None, filename, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrnoWithFilenameObjects(
    exception: *mut c_void,
    filename1: *mut c_void,
    filename2: *mut c_void,
) -> *mut c_void {
    let filename1 = cpython_optional_filename_from_object(filename1);
    let filename2 = cpython_optional_filename_from_object(filename2);
    cpython_set_os_error_message(exception, None, filename1, filename2);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErr(
    exception: *mut c_void,
    ierr: i32,
) -> *mut c_void {
    cpython_set_os_error_message(exception, Some(ierr), None, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErrWithFilename(
    exception: *mut c_void,
    ierr: i32,
    filename: *const c_char,
) -> *mut c_void {
    let filename = cpython_optional_filename_from_c(filename);
    cpython_set_os_error_message(exception, Some(ierr), filename, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErrWithFilenameObject(
    exception: *mut c_void,
    ierr: i32,
    filename: *mut c_void,
) -> *mut c_void {
    let filename = cpython_optional_filename_from_object(filename);
    cpython_set_os_error_message(exception, Some(ierr), filename, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErrWithFilenameObjects(
    exception: *mut c_void,
    ierr: i32,
    filename1: *mut c_void,
    filename2: *mut c_void,
) -> *mut c_void {
    let filename1 = cpython_optional_filename_from_object(filename1);
    let filename2 = cpython_optional_filename_from_object(filename2);
    cpython_set_os_error_message(exception, Some(ierr), filename1, filename2);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromWindowsErr(ierr: i32) -> *mut c_void {
    unsafe { PyErr_SetExcFromWindowsErr(std::ptr::null_mut(), ierr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromWindowsErrWithFilename(
    ierr: i32,
    filename: *const c_char,
) -> *mut c_void {
    unsafe { PyErr_SetExcFromWindowsErrWithFilename(std::ptr::null_mut(), ierr, filename) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetInterrupt() {
    cpython_set_typed_error(std::ptr::null_mut(), "KeyboardInterrupt");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetInterruptEx(signum: i32) -> i32 {
    if signum <= 0 {
        return -1;
    }
    unsafe { PyErr_SetInterrupt() };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SyntaxLocation(filename: *const c_char, lineno: i32) {
    unsafe { PyErr_SyntaxLocationEx(filename, lineno, 0) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SyntaxLocationEx(
    filename: *const c_char,
    lineno: i32,
    col_offset: i32,
) {
    let filename =
        cpython_optional_filename_from_c(filename).unwrap_or_else(|| "<unknown>".to_string());
    let message = format!("invalid syntax ({filename}, line {lineno}, column {col_offset})");
    cpython_set_typed_error(std::ptr::null_mut(), message);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_ProgramText(filename: *const c_char, lineno: i32) -> *mut c_void {
    if lineno <= 0 {
        return std::ptr::null_mut();
    }
    let Some(filename) = cpython_optional_filename_from_c(filename) else {
        return std::ptr::null_mut();
    };
    let Ok(contents) = std::fs::read_to_string(&filename) else {
        return std::ptr::null_mut();
    };
    let index = (lineno - 1) as usize;
    let line = if let Some(line) = contents.split_inclusive('\n').nth(index) {
        line.to_string()
    } else if let Some(line) = contents.lines().nth(index) {
        line.to_string()
    } else {
        return std::ptr::null_mut();
    };
    cpython_new_ptr_for_value(Value::Str(line))
}

fn cpython_import_error_arg_or_none(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        cpython_new_ptr_for_value(Value::None)
    } else {
        object
    }
}

fn cpython_set_import_error_subclass_with_name_from(
    exception: *mut c_void,
    msg: *mut c_void,
    name: *mut c_void,
    path: *mut c_void,
    from_name: *mut c_void,
) -> *mut c_void {
    if exception.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "expected a subclass of ImportError",
        );
        return std::ptr::null_mut();
    }
    let is_subclass = unsafe { PyObject_IsSubclass(exception, PyExc_ImportError) };
    if is_subclass < 0 {
        return std::ptr::null_mut();
    }
    if is_subclass == 0 {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "expected a subclass of ImportError",
        );
        return std::ptr::null_mut();
    }
    if msg.is_null() {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "expected a message argument");
        return std::ptr::null_mut();
    }

    let name_obj = cpython_import_error_arg_or_none(name);
    let path_obj = cpython_import_error_arg_or_none(path);
    let from_name_obj = cpython_import_error_arg_or_none(from_name);
    if name_obj.is_null() || path_obj.is_null() || from_name_obj.is_null() {
        return std::ptr::null_mut();
    }

    let args = unsafe { PyTuple_New(1) };
    if args.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { Py_IncRef(msg) };
    if unsafe { PyTuple_SetItem(args, 0, msg) } != 0 {
        unsafe { Py_DecRef(args) };
        return std::ptr::null_mut();
    }

    let error_instance = unsafe { PyObject_CallObject(exception, args) };
    unsafe { Py_DecRef(args) };
    if error_instance.is_null() {
        return std::ptr::null_mut();
    }
    if unsafe { PyObject_SetAttrString(error_instance, c"name".as_ptr(), name_obj) } != 0
        || unsafe { PyObject_SetAttrString(error_instance, c"path".as_ptr(), path_obj) } != 0
        || unsafe { PyObject_SetAttrString(error_instance, c"name_from".as_ptr(), from_name_obj) }
            != 0
    {
        return std::ptr::null_mut();
    }

    unsafe { PyErr_SetObject(exception, error_instance) };
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetImportError(
    msg: *mut c_void,
    name: *mut c_void,
    path: *mut c_void,
) -> *mut c_void {
    cpython_set_import_error_subclass_with_name_from(
        unsafe { PyExc_ImportError },
        msg,
        name,
        path,
        std::ptr::null_mut(),
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetImportErrorSubclass(
    exception: *mut c_void,
    msg: *mut c_void,
    name: *mut c_void,
    path: *mut c_void,
) -> *mut c_void {
    cpython_set_import_error_subclass_with_name_from(
        exception,
        msg,
        name,
        path,
        std::ptr::null_mut(),
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_WarnEx(
    _category: *mut c_void,
    message: *const c_char,
    _stacklevel: isize,
) -> i32 {
    if !message.is_null()
        && let Ok(text) = unsafe { c_name_to_string(message) }
    {
        eprintln!("warning: {text}");
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_WarnExplicit(
    category: *mut c_void,
    text: *const c_char,
    filename: *const c_char,
    lineno: i32,
    module: *const c_char,
    _registry: *mut c_void,
) -> i32 {
    let text = match unsafe { c_name_to_string(text) } {
        Ok(value) => value,
        Err(_) => {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "PyErr_WarnExplicit requires non-null message",
            );
            return -1;
        }
    };
    let filename = if filename.is_null() {
        "<string>".to_string()
    } else {
        match unsafe { c_name_to_string(filename) } {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return -1;
            }
        }
    };
    let module = if module.is_null() {
        None
    } else {
        match unsafe { c_name_to_string(module) } {
            Ok(value) => Some(value),
            Err(err) => {
                cpython_set_error(err);
                return -1;
            }
        }
    };
    let mut rendered = format!("{filename}:{lineno}: {text}");
    if let Some(module) = module {
        rendered = format!("{module}: {rendered}");
    }
    let Ok(rendered) = CString::new(rendered) else {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "warning message contains interior NUL byte",
        );
        return -1;
    };
    unsafe { PyErr_WarnEx(category, rendered.as_ptr(), 1) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_WarnFormat(
    category: *mut c_void,
    stacklevel: isize,
    format: *const c_char,
) -> i32 {
    unsafe { PyErr_WarnEx(category, format, stacklevel) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_ResourceWarning(
    _source: *mut c_void,
    stack_level: isize,
    format: *const c_char,
) -> i32 {
    if format.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "PyErr_ResourceWarning requires non-null format string",
        );
        return -1;
    }
    let category = unsafe {
        if PyExc_ResourceWarning.is_null() {
            PyExc_RuntimeWarning
        } else {
            PyExc_ResourceWarning
        }
    };
    unsafe { PyErr_WarnEx(category, format, stack_level) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_WriteUnraisable(_object: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Print() {
    if let Ok(Some(message)) = with_active_cpython_context_mut(|context| context.last_error.clone())
    {
        eprintln!("error: {message}");
    }
    unsafe { PyErr_Clear() };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_PrintEx(_set_sys_last_vars: i32) {
    unsafe { PyErr_Print() };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Display(
    _exception: *mut c_void,
    value: *mut c_void,
    _traceback: *mut c_void,
) {
    let _ = with_active_cpython_context_mut(|context| {
        let message = if value.is_null() {
            "unhandled exception".to_string()
        } else {
            context.error_message_from_ptr(value)
        };
        eprintln!("error: {message}");
        context.clear_error();
    })
    .map_err(|err| cpython_set_error(err));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_DisplayException(exc: *mut c_void) {
    unsafe { PyErr_Display(std::ptr::null_mut(), exc, std::ptr::null_mut()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnstable_Object_IsUniquelyReferenced(_object: *mut c_void) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnstable_Object_IsUniqueReferencedTemporary(
    _object: *mut c_void,
) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GenericAlias(origin: *mut c_void, _args: *mut c_void) -> *mut c_void {
    unsafe { Py_XIncRef(origin) };
    origin
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetObject(_exception: *mut c_void, value: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let ptype = if _exception.is_null() {
            unsafe { PyExc_RuntimeError }
        } else {
            _exception
        };
        let value_obj = context.cpython_value_from_ptr_or_proxy(value);
        if std::env::var_os("PYRS_TRACE_CPY_UFUNC_ERRORS").is_some() {
            let exception_name = cpython_exception_class_name_from_ptr(ptype)
                .unwrap_or_else(|| cpython_type_name_for_object_ptr(ptype));
            if exception_name.contains("UFunc") || exception_name.contains("ufunc") {
                eprintln!(
                    "[cpy-ufunc-error] ptype={:p} name={} value_ptr={:p} value={} bt={:?}",
                    ptype,
                    exception_name,
                    value,
                    value_obj
                        .as_ref()
                        .map(cpython_value_debug_tag)
                        .unwrap_or_else(|| "<unknown>".to_string()),
                    Backtrace::force_capture()
                );
            }
        }
        if let Some(normalized) =
            cpython_make_exception_instance_from_type_and_value(context, ptype, value_obj.clone())
        {
            let message = context.error_message_from_ptr(normalized);
            context.set_error_state(ptype, normalized, std::ptr::null_mut(), message);
            return;
        }
        let message = context.error_message_from_ptr(value);
        context.set_error_state(ptype, value, std::ptr::null_mut(), message);
    })
    .map_err(|err| {
        cpython_set_error(err);
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetNone(exception: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let ptype = if exception.is_null() {
            unsafe { PyExc_RuntimeError }
        } else {
            exception
        };
        context.set_error_state(
            ptype,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            "error".to_string(),
        );
    })
    .map_err(|err| {
        cpython_set_error(err);
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_NoMemory() -> *mut c_void {
    let _ = with_active_cpython_context_mut(|context| {
        let message = "out of memory".to_string();
        let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
        context.set_error_state(
            unsafe { PyExc_MemoryError },
            pvalue,
            std::ptr::null_mut(),
            message,
        );
    })
    .map_err(|err| {
        cpython_set_error(err);
    });
    std::ptr::null_mut()
}
