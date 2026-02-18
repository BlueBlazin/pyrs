use std::ffi::{CStr, c_char, c_void};

use crate::runtime::{BigInt, BuiltinFunction, ModuleObject, Object, Value};

use super::{
    Py_DecRef, PyErr_BadInternalCall, PyExc_ValueError, PyNumber_Index,
    cpython_asnativebytes_resolve_endian, cpython_bigint_from_twos_complement_le,
    cpython_bigint_from_value, cpython_bigint_to_twos_complement_le, cpython_call_builtin,
    cpython_new_ptr_for_value, cpython_required_signed_bytes_for_bigint,
    cpython_required_unsigned_bytes_for_bigint, cpython_set_error, cpython_set_typed_error,
    cpython_value_from_ptr, with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromLong(value: i64) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        context.alloc_cpython_ptr_for_value(Value::Int(value))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromLongLong(value: i64) -> *mut c_void {
    unsafe { PyLong_FromLong(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromSsize_t(value: isize) -> *mut c_void {
    unsafe { PyLong_FromLongLong(value as i64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromSize_t(value: usize) -> *mut c_void {
    if i64::try_from(value).is_ok() {
        return cpython_new_ptr_for_value(Value::Int(value as i64));
    }
    cpython_new_ptr_for_value(Value::BigInt(Box::new(BigInt::from_u64(value as u64))))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromInt32(value: i32) -> *mut c_void {
    unsafe { PyLong_FromLong(value as i64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUInt32(value: u32) -> *mut c_void {
    unsafe { PyLong_FromUnsignedLong(value as u64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromInt64(value: i64) -> *mut c_void {
    unsafe { PyLong_FromLongLong(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUInt64(value: u64) -> *mut c_void {
    unsafe { PyLong_FromUnsignedLongLong(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUnsignedLong(value: u64) -> *mut c_void {
    if i64::try_from(value).is_ok() {
        return cpython_new_ptr_for_value(Value::Int(value as i64));
    }
    cpython_new_ptr_for_value(Value::BigInt(Box::new(BigInt::from_u64(value))))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUnsignedLongLong(value: u64) -> *mut c_void {
    unsafe { PyLong_FromUnsignedLong(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromVoidPtr(value: *mut c_void) -> *mut c_void {
    unsafe { PyLong_FromUnsignedLongLong(value as usize as u64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUnicodeObject(object: *mut c_void, base: i32) -> *mut c_void {
    let Value::Str(text) = (match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    }) else {
        cpython_set_error("PyLong_FromUnicodeObject expects str input");
        return std::ptr::null_mut();
    };
    let parsed_base = if base == 0 {
        10
    } else if (2..=36).contains(&base) {
        base as u32
    } else {
        cpython_set_error("PyLong_FromUnicodeObject received invalid base");
        return std::ptr::null_mut();
    };
    let trimmed = text.trim();
    match BigInt::from_str_radix(trimmed, parsed_base) {
        Some(bigint) => {
            if let Some(i) = bigint.to_i64() {
                cpython_new_ptr_for_value(Value::Int(i))
            } else {
                cpython_new_ptr_for_value(Value::BigInt(Box::new(bigint)))
            }
        }
        None => {
            cpython_set_error("PyLong_FromUnicodeObject failed to parse integer");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromString(
    value: *const c_char,
    pend: *mut *mut c_char,
    base: i32,
) -> *mut c_void {
    if value.is_null() {
        unsafe { PyErr_BadInternalCall() };
        if !pend.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe { *pend = std::ptr::null_mut() };
        }
        return std::ptr::null_mut();
    }
    if !(base == 0 || (2..=36).contains(&base)) {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "int() base must be >= 2 and <= 36, or 0",
        );
        if !pend.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe { *pend = value as *mut c_char };
        }
        return std::ptr::null_mut();
    }
    // SAFETY: `value` points to a NUL-terminated C string per API contract.
    let source = unsafe { CStr::from_ptr(value) };
    let source_text = source.to_string_lossy().into_owned();
    let mut args = vec![Value::Str(source_text)];
    if base != 10 {
        args.push(Value::Int(base as i64));
    }
    let parsed = match cpython_call_builtin(BuiltinFunction::Int, args) {
        Ok(parsed) => parsed,
        Err(err) => {
            cpython_set_typed_error(unsafe { PyExc_ValueError }, &err);
            if !pend.is_null() {
                // SAFETY: caller provided writable output pointer.
                unsafe { *pend = value as *mut c_char };
            }
            return std::ptr::null_mut();
        }
    };
    if !pend.is_null() {
        // SAFETY: `value` points to a NUL-terminated string; advancing by `to_bytes().len()`
        // lands on the trailing NUL, matching CPython's full-consume success path.
        unsafe { *pend = value.add(source.to_bytes().len()) as *mut c_char };
    }
    cpython_new_ptr_for_value(parsed)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_GetInfo() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyLong_GetInfo missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(sys_module) = vm.modules.get("sys").cloned() else {
            context.set_error("PyLong_GetInfo missing sys module");
            return std::ptr::null_mut();
        };
        let int_info = match &*sys_module.kind() {
            Object::Module(module_data) => module_data.globals.get("int_info").cloned(),
            _ => None,
        };
        let int_info = int_info.unwrap_or_else(|| {
            let mut synthetic = ModuleObject::new("<int_info>");
            synthetic
                .globals
                .insert("bits_per_digit".to_string(), Value::Int(30));
            synthetic
                .globals
                .insert("sizeof_digit".to_string(), Value::Int(4));
            synthetic
                .globals
                .insert("default_max_str_digits".to_string(), Value::Int(0));
            synthetic
                .globals
                .insert("str_digits_check_threshold".to_string(), Value::Int(0));
            vm.heap.alloc_module(synthetic)
        });
        context.alloc_cpython_ptr_for_value(int_info)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsNativeBytes(
    object: *mut c_void,
    buffer: *mut c_void,
    n_bytes: isize,
    flags: i32,
) -> isize {
    const PY_ASNATIVEBYTES_UNSIGNED_BUFFER: i32 = 0x4;
    const PY_ASNATIVEBYTES_REJECT_NEGATIVE: i32 = 0x8;
    const PY_ASNATIVEBYTES_ALLOW_INDEX: i32 = 0x10;

    if object.is_null() || n_bytes < 0 {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if n_bytes > 0 && buffer.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let little_endian = cpython_asnativebytes_resolve_endian(flags);
    with_active_cpython_context_mut(|context| {
        let mut value = match context.cpython_value_from_ptr_or_proxy(object) {
            Some(value) => value,
            None => {
                context.set_error("PyLong_AsNativeBytes received unknown object pointer");
                return -1;
            }
        };
        if !matches!(value, Value::Int(_) | Value::Bool(_) | Value::BigInt(_)) {
            if flags != -1 && (flags & PY_ASNATIVEBYTES_ALLOW_INDEX) != 0 {
                let indexed = unsafe { PyNumber_Index(object) };
                if indexed.is_null() {
                    return -1;
                }
                let next = match context.cpython_value_from_ptr_or_proxy(indexed) {
                    Some(value) => value,
                    None => {
                        unsafe { Py_DecRef(indexed) };
                        context.set_error("PyLong_AsNativeBytes index conversion failed");
                        return -1;
                    }
                };
                unsafe { Py_DecRef(indexed) };
                value = next;
            } else {
                context.set_error("expect int");
                return -1;
            }
        }
        let bigint = match cpython_bigint_from_value(value) {
            Ok(bigint) => bigint,
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        };
        if flags != -1 && (flags & PY_ASNATIVEBYTES_REJECT_NEGATIVE) != 0 && bigint.is_negative() {
            cpython_set_typed_error(unsafe { PyExc_ValueError }, "Cannot convert negative int");
            return -1;
        }
        let required = if !bigint.is_negative()
            && (flags == -1 || (flags & PY_ASNATIVEBYTES_UNSIGNED_BUFFER) != 0)
        {
            cpython_required_unsigned_bytes_for_bigint(&bigint)
        } else {
            cpython_required_signed_bytes_for_bigint(&bigint)
        };
        if n_bytes == 0 {
            return required as isize;
        }
        let n = n_bytes as usize;
        let encoded_le = cpython_bigint_to_twos_complement_le(&bigint, n);
        if little_endian != 0 {
            // SAFETY: caller provided writable output buffer of `n` bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(encoded_le.as_ptr(), buffer.cast::<u8>(), n);
            }
        } else {
            for (idx, byte) in encoded_le.iter().enumerate() {
                // SAFETY: caller provided writable output buffer of `n` bytes.
                unsafe {
                    *buffer.cast::<u8>().add(n - idx - 1) = *byte;
                }
            }
        }
        required as isize
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromNativeBytes(
    buffer: *const c_void,
    n_bytes: usize,
    flags: i32,
) -> *mut c_void {
    const PY_ASNATIVEBYTES_UNSIGNED_BUFFER: i32 = 0x4;
    if buffer.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let little_endian = cpython_asnativebytes_resolve_endian(flags);
    let signed = flags == -1 || (flags & PY_ASNATIVEBYTES_UNSIGNED_BUFFER) == 0;
    let raw = if n_bytes == 0 {
        &[][..]
    } else {
        // SAFETY: caller guarantees readable `n_bytes` bytes at `buffer`.
        unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n_bytes) }
    };
    let mut le = raw.to_vec();
    if little_endian == 0 {
        le.reverse();
    }
    let bigint = cpython_bigint_from_twos_complement_le(&le, signed);
    match bigint.to_i64() {
        Some(value) => cpython_new_ptr_for_value(Value::Int(value)),
        None => cpython_new_ptr_for_value(Value::BigInt(Box::new(bigint))),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUnsignedNativeBytes(
    buffer: *const c_void,
    n_bytes: usize,
    flags: i32,
) -> *mut c_void {
    if buffer.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let little_endian = cpython_asnativebytes_resolve_endian(flags);
    let raw = if n_bytes == 0 {
        &[][..]
    } else {
        // SAFETY: caller guarantees readable `n_bytes` bytes at `buffer`.
        unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n_bytes) }
    };
    let mut le = raw.to_vec();
    if little_endian == 0 {
        le.reverse();
    }
    let bigint = cpython_bigint_from_twos_complement_le(&le, false);
    match bigint.to_i64() {
        Some(value) => cpython_new_ptr_for_value(Value::Int(value)),
        None => cpython_new_ptr_for_value(Value::BigInt(Box::new(bigint))),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBool_FromLong(value: i64) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        context.alloc_cpython_ptr_for_value(Value::Bool(value != 0))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_FromDouble(value: f64) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        context.alloc_cpython_ptr_for_value(Value::Float(value))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_FromString(
    object: *mut c_void,
    _endptr: *mut *mut c_char,
) -> *mut c_void {
    match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => match text.parse::<f64>() {
            Ok(value) => cpython_new_ptr_for_value(Value::Float(value)),
            Err(_) => {
                cpython_set_error("PyFloat_FromString failed to parse float");
                std::ptr::null_mut()
            }
        },
        Ok(_) => {
            cpython_set_error("PyFloat_FromString expects str object");
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}
