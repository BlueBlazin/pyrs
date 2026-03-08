use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};

use crate::runtime::{Object, Value};
use crate::unicode::{canonical_codepoint_for_internal_char, internal_char_from_codepoint};
use crate::vm::{BuiltinFunction, dict_set_value_checked, mod_values, value_to_int};

use super::{
    _Py_NotImplementedStruct, CpythonBuffer, CpythonBufferProcs, CpythonObjectHead,
    CpythonTypeObject, Cwchar, ModuleCapiContext, Py_DecRef, Py_IncRef, Py_XDecRef,
    PyBytes_AsString, PyErr_BadArgument, PyErr_BadInternalCall, PyErr_Clear, PyErr_NoMemory,
    PyErr_Occurred, PyExc_IndexError, PyExc_RuntimeError, PyExc_SystemError, PyExc_TypeError,
    PyExc_UnicodeEncodeError, PyExc_ValueError, PyMem_Malloc, PyOS_FSPath, c_name_to_string,
    cpython_call_internal_in_context, cpython_call_method_for_capi,
    cpython_codec_error_name_optional, cpython_codec_module_in_context,
    cpython_codec_name_or_default, cpython_getattr_in_context, cpython_lookup_interned_unicode_ptr,
    cpython_new_bytes_ptr, cpython_new_ptr_for_value, cpython_register_interned_unicode,
    cpython_resolve_vectorcall, cpython_set_error, cpython_set_typed_error,
    cpython_stable_utf8_ptr, cpython_string_to_wide_units,
    cpython_unicode_decode_with_codec_in_context, cpython_unicode_encode_with_codec_in_context,
    cpython_unicode_text_from_value, cpython_value_debug_tag, cpython_value_from_ptr,
    cpython_wide_ptr_to_string, with_active_cpython_context_mut,
};

fn cpython_codec_name_normalized(name: &str) -> String {
    name.chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' {
                Some(ch.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect()
}

fn cpython_codec_is_rot13(name: &str) -> bool {
    matches!(
        cpython_codec_name_normalized(name).as_str(),
        "rot13" | "rot.13"
    )
}

fn cpython_rot13_text(text: &str) -> String {
    fn rotate(ch: char, base: char) -> char {
        let offset = ch as u32 - base as u32;
        let rotated = (offset + 13) % 26 + base as u32;
        char::from_u32(rotated).unwrap_or(ch)
    }

    text.chars()
        .map(|ch| {
            if ch.is_ascii_lowercase() {
                rotate(ch, 'a')
            } else if ch.is_ascii_uppercase() {
                rotate(ch, 'A')
            } else {
                ch
            }
        })
        .collect()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromString(value: *const c_char) -> *mut c_void {
    match unsafe { c_name_to_string(value) } {
        Ok(text) => with_active_cpython_context_mut(|context| {
            context.alloc_cpython_ptr_for_value(Value::Str(text))
        })
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            std::ptr::null_mut()
        }),
        Err(err) => {
            cpython_set_error(format!(
                "PyUnicode_FromString received invalid string: {err}"
            ));
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromStringAndSize(
    value: *const c_char,
    len: isize,
) -> *mut c_void {
    if len < 0 {
        cpython_set_error("PyUnicode_FromStringAndSize received negative length");
        return std::ptr::null_mut();
    }
    if value.is_null() && len != 0 {
        cpython_set_error("PyUnicode_FromStringAndSize received null pointer with non-zero length");
        return std::ptr::null_mut();
    }
    let bytes = if len == 0 {
        Vec::new()
    } else {
        // SAFETY: caller guarantees `value` points to at least len bytes.
        unsafe { std::slice::from_raw_parts(value.cast::<u8>(), len as usize).to_vec() }
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    cpython_new_ptr_for_value(Value::Str(text))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromWideChar(value: *const Cwchar, len: isize) -> *mut c_void {
    let text = match unsafe { cpython_wide_ptr_to_string(value, len, "PyUnicode_FromWideChar") } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_new_ptr_for_value(Value::Str(text))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsWideCharString(
    unicode: *mut c_void,
    size: *mut isize,
) -> *mut Cwchar {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            context.set_error("PyUnicode_AsWideCharString received unknown object pointer");
            return std::ptr::null_mut();
        };
        let Some(text) = cpython_unicode_text_from_value(&value) else {
            context.set_error("PyUnicode_AsWideCharString expected str object");
            return std::ptr::null_mut();
        };
        if size.is_null() && text.chars().any(|ch| ch == '\0') {
            cpython_set_typed_error(unsafe { PyExc_ValueError }, "embedded null character");
            return std::ptr::null_mut();
        }
        let units = cpython_string_to_wide_units(&text);
        if !size.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe {
                *size = units.len() as isize;
            }
        }
        let Some(byte_len) = units
            .len()
            .checked_add(1)
            .and_then(|count| count.checked_mul(std::mem::size_of::<Cwchar>()))
        else {
            context.set_error("PyUnicode_AsWideCharString size overflow");
            return std::ptr::null_mut();
        };
        // SAFETY: allocated by CPython-compatible allocator and owned by caller.
        let raw = unsafe { PyMem_Malloc(byte_len) }.cast::<Cwchar>();
        if raw.is_null() {
            unsafe { PyErr_NoMemory() };
            return std::ptr::null_mut();
        }
        if !units.is_empty() {
            // SAFETY: destination has capacity for `units.len()` elements.
            unsafe {
                std::ptr::copy_nonoverlapping(units.as_ptr(), raw, units.len());
            }
        }
        // SAFETY: destination has at least one trailing slot for NUL terminator.
        unsafe {
            *raw.add(units.len()) = 0;
        }
        raw
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsWideChar(
    unicode: *mut c_void,
    value: *mut Cwchar,
    size: isize,
) -> isize {
    if size < 0 {
        cpython_set_error("PyUnicode_AsWideChar received negative size");
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        let Some(unicode_value) = context.cpython_value_from_ptr(unicode) else {
            context.set_error("PyUnicode_AsWideChar received unknown object pointer");
            return -1;
        };
        let Some(text) = cpython_unicode_text_from_value(&unicode_value) else {
            context.set_error("PyUnicode_AsWideChar expected str object");
            return -1;
        };
        let units = cpython_string_to_wide_units(&text);
        if value.is_null() {
            if size == 0 {
                return units.len() as isize;
            }
            context.set_error("PyUnicode_AsWideChar requires non-null output buffer");
            return -1;
        }
        let write_cap = size as usize;
        let write_len = write_cap.min(units.len());
        if write_len > 0 {
            // SAFETY: caller-provided output buffer has `write_cap` writable elements.
            unsafe {
                std::ptr::copy_nonoverlapping(units.as_ptr(), value, write_len);
            }
        }
        if write_len < write_cap {
            // SAFETY: `write_len < write_cap` guarantees valid trailing slot.
            unsafe {
                *value.add(write_len) = 0;
            }
        }
        write_len as isize
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromEncodedObject(
    object: *mut c_void,
    _encoding: *const c_char,
    _errors: *const c_char,
) -> *mut c_void {
    match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => cpython_new_ptr_for_value(Value::Str(text)),
        Ok(Value::Bytes(bytes_obj)) | Ok(Value::ByteArray(bytes_obj)) => match &*bytes_obj.kind() {
            Object::Bytes(values) | Object::ByteArray(values) => {
                let text = String::from_utf8_lossy(values).into_owned();
                cpython_new_ptr_for_value(Value::Str(text))
            }
            _ => {
                cpython_set_error("PyUnicode_FromEncodedObject encountered invalid bytes storage");
                std::ptr::null_mut()
            }
        },
        Ok(_) => {
            cpython_set_error("PyUnicode_FromEncodedObject expects str/bytes-like object");
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromKindAndData(
    kind: i32,
    buffer: *const c_void,
    size: isize,
) -> *mut c_void {
    if size < 0 {
        cpython_set_error("PyUnicode_FromKindAndData received negative size");
        return std::ptr::null_mut();
    }
    if buffer.is_null() && size != 0 {
        cpython_set_error("PyUnicode_FromKindAndData received null buffer with non-zero size");
        return std::ptr::null_mut();
    }
    let text = match kind {
        1 => {
            let bytes = if size == 0 {
                &[][..]
            } else {
                // SAFETY: caller guarantees buffer points to `size` bytes.
                unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), size as usize) }
            };
            String::from_utf8_lossy(bytes).into_owned()
        }
        2 => {
            let units = if size == 0 {
                &[][..]
            } else {
                // SAFETY: caller guarantees buffer points to `size` u16 values.
                unsafe { std::slice::from_raw_parts(buffer.cast::<u16>(), size as usize) }
            };
            String::from_utf16_lossy(units)
        }
        4 => {
            let units = if size == 0 {
                &[][..]
            } else {
                // SAFETY: caller guarantees buffer points to `size` u32 values.
                unsafe { std::slice::from_raw_parts(buffer.cast::<u32>(), size as usize) }
            };
            units
                .iter()
                .filter_map(|codepoint| char::from_u32(*codepoint))
                .collect()
        }
        _ => {
            cpython_set_error("PyUnicode_FromKindAndData received unsupported kind");
            return std::ptr::null_mut();
        }
    };
    cpython_new_ptr_for_value(Value::Str(text))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_New(size: isize, maxchar: c_uint) -> *mut c_void {
    if size == 0 {
        return cpython_new_ptr_for_value(Value::Str(String::new()));
    }
    if size < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "Negative size passed to PyUnicode_New",
        );
        return std::ptr::null_mut();
    }
    if maxchar > 0x10ffff {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "invalid maximum character passed to PyUnicode_New",
        );
        return std::ptr::null_mut();
    }
    cpython_new_ptr_for_value(Value::Str("\0".repeat(size as usize)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF8(object: *mut c_void) -> *const c_char {
    match with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyUnicode_AsUTF8 received unknown object pointer");
            return std::ptr::null();
        };
        let Value::Str(text) = value else {
            context.set_error("PyUnicode_AsUTF8 expected str object");
            return std::ptr::null();
        };
        match cpython_stable_utf8_ptr(&text) {
            Ok(ptr) => ptr,
            Err(err) => {
                context.set_error(err);
                std::ptr::null()
            }
        }
    }) {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF8AndSize(
    object: *mut c_void,
    out_len: *mut isize,
) -> *const c_char {
    match with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyUnicode_AsUTF8AndSize received unknown object pointer");
            return std::ptr::null();
        };
        let Value::Str(text) = value else {
            context.set_error("PyUnicode_AsUTF8AndSize expected str object");
            return std::ptr::null();
        };
        let ptr = match cpython_stable_utf8_ptr(&text) {
            Ok(ptr) => ptr,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null();
            }
        };
        if !out_len.is_null() {
            // SAFETY: caller provided writable out pointer.
            unsafe { *out_len = text.len() as isize };
        }
        ptr
    }) {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF8String(object: *mut c_void) -> *mut c_void {
    let ptr = unsafe { PyUnicode_AsUTF8(object) };
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: returned pointer is NUL-terminated scratch string.
    let bytes = unsafe { CStr::from_ptr(ptr).to_bytes().to_vec() };
    cpython_new_bytes_ptr(bytes)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsASCIIString(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsUTF8String(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsLatin1String(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsUTF8String(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsMBCSString(object: *mut c_void) -> *mut c_void {
    cpython_unicode_encode_with_encoding_name(
        object,
        cpython_codepage_encoding_name(0),
        std::ptr::null(),
        "PyUnicode_AsMBCSString",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsCharmapString(
    object: *mut c_void,
    mapping: *mut c_void,
) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if mapping.is_null() {
        let value = match cpython_value_from_ptr(object) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
        let Some(text) = cpython_unicode_text_from_value(&value) else {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "PyUnicode_AsCharmapString expected str object",
            );
            return std::ptr::null_mut();
        };
        let mut out = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let code = ch as u32;
            if code > 0xFF {
                cpython_set_typed_error(
                    unsafe { PyExc_UnicodeEncodeError },
                    "character maps outside latin-1 range",
                );
                return std::ptr::null_mut();
            }
            out.push(code as u8);
        }
        return cpython_new_bytes_ptr(out);
    }
    with_active_cpython_context_mut(|context| {
        let Some(unicode_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyUnicode_AsCharmapString received unknown object");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&unicode_value).is_none() {
            context.set_error("PyUnicode_AsCharmapString expected str object");
            return std::ptr::null_mut();
        }
        let mapping_value = if mapping.is_null() {
            Value::None
        } else if let Some(value) = context.cpython_value_from_ptr_or_proxy(mapping) {
            value
        } else {
            context.set_error("PyUnicode_AsCharmapString received unknown mapping object");
            return std::ptr::null_mut();
        };
        let codec_module = match cpython_codec_module_in_context(context) {
            Ok(module) => module,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let charmap_encode = {
            let Object::Module(module_data) = &*codec_module.kind() else {
                context.set_error("codecs module is invalid");
                return std::ptr::null_mut();
            };
            match module_data.globals.get("charmap_encode").cloned() {
                Some(value) => value,
                None => {
                    context.set_error("codecs.charmap_encode unavailable");
                    return std::ptr::null_mut();
                }
            }
        };
        let encoded_tuple = match cpython_call_internal_in_context(
            context,
            charmap_encode,
            vec![
                unicode_value,
                Value::Str("strict".to_string()),
                mapping_value,
            ],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let Value::Tuple(parts) = encoded_tuple else {
            context.set_error("codecs.charmap_encode() returned non-tuple");
            return std::ptr::null_mut();
        };
        let Object::Tuple(items) = &*parts.kind() else {
            context.set_error("codecs.charmap_encode() returned invalid tuple");
            return std::ptr::null_mut();
        };
        let Some(encoded) = items.first().cloned() else {
            context.set_error("codecs.charmap_encode() returned empty tuple");
            return std::ptr::null_mut();
        };
        match &encoded {
            Value::Bytes(_) => context.alloc_cpython_ptr_for_value(encoded),
            _ => {
                context.set_error("codecs.charmap_encode() did not return bytes");
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
pub unsafe extern "C" fn PyUnicode_AsRawUnicodeEscapeString(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedObject(object, c"raw_unicode_escape".as_ptr(), std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUnicodeEscapeString(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedObject(object, c"unicode_escape".as_ptr(), std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF16String(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedObject(object, c"utf-16".as_ptr(), std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF32String(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedObject(object, c"utf-32".as_ptr(), std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsEncodedString(
    object: *mut c_void,
    _encoding: *const c_char,
    _errors: *const c_char,
) -> *mut c_void {
    unsafe { PyUnicode_AsUTF8String(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Compare(left: *mut c_void, right: *mut c_void) -> i32 {
    let left = match cpython_value_from_ptr(left) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Compare expected str left operand");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let right = match cpython_value_from_ptr(right) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Compare expected str right operand");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    use std::cmp::Ordering;
    match left.cmp(&right) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_CompareWithASCIIString(
    left: *mut c_void,
    right: *const c_char,
) -> i32 {
    let right = unsafe { PyUnicode_FromString(right) };
    if right.is_null() {
        return -1;
    }
    let result = unsafe { PyUnicode_Compare(left, right) };
    unsafe { Py_DecRef(right) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Concat(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    let left = match cpython_value_from_ptr(left) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Concat expected str left operand");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let right = match cpython_value_from_ptr(right) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Concat expected str right operand");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_new_ptr_for_value(Value::Str(format!("{left}{right}")))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Contains(container: *mut c_void, element: *mut c_void) -> i32 {
    let haystack = match cpython_value_from_ptr(container) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Contains expected str container");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let needle = match cpython_value_from_ptr(element) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Contains expected str element");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    i32::from(haystack.contains(&needle))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Format(format: *mut c_void, arg: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if format.is_null() {
            context.set_error("PyUnicode_Format received null format");
            return std::ptr::null_mut();
        }
        let format_value = match context.cpython_value_from_ptr_or_proxy(format) {
            Some(value @ Value::Str(_)) => value,
            Some(other) => {
                let got = if context.vm.is_null() {
                    cpython_value_debug_tag(&other)
                } else {
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    unsafe { (&mut *context.vm).value_type_name_for_error(&other) }
                };
                context.set_error(format!("PyUnicode_Format expected str format, got {got}"));
                return std::ptr::null_mut();
            }
            None => {
                context.set_error("PyUnicode_Format received unknown format pointer");
                return std::ptr::null_mut();
            }
        };
        let arg_value = if arg.is_null() {
            Value::None
        } else {
            match context.cpython_value_from_ptr_or_proxy(arg) {
                Some(value) => value,
                None => {
                    context.set_error("PyUnicode_Format received unknown argument pointer");
                    return std::ptr::null_mut();
                }
            }
        };
        if context.vm.is_null() {
            context.set_error("missing VM context for PyUnicode_Format");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let rendered = match mod_values(format_value, arg_value, &vm.heap) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err.message);
                return std::ptr::null_mut();
            }
        };
        match rendered {
            Value::Str(_) => context.alloc_cpython_ptr_for_value(rendered),
            other => {
                let got = if context.vm.is_null() {
                    cpython_value_debug_tag(&other)
                } else {
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    unsafe { (&mut *context.vm).value_type_name_for_error(&other) }
                };
                context.set_error(format!("PyUnicode_Format expected str result, got {got}"));
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
pub unsafe extern "C" fn PyUnicode_GetLength(object: *mut c_void) -> isize {
    match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text.chars().count() as isize,
        Ok(_) => {
            cpython_set_error("PyUnicode_GetLength expected str object");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_InternFromString(value: *const c_char) -> *mut c_void {
    if value.is_null() {
        cpython_set_error("PyUnicode_InternFromString received null string");
        return std::ptr::null_mut();
    }
    let text = match unsafe { c_name_to_string(value) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(format!(
                "PyUnicode_InternFromString received invalid string: {err}"
            ));
            return std::ptr::null_mut();
        }
    };
    if let Some(existing) = cpython_lookup_interned_unicode_ptr(&text) {
        unsafe { Py_IncRef(existing) };
        return existing;
    }
    let created = unsafe { PyUnicode_FromString(value) };
    if created.is_null() {
        return std::ptr::null_mut();
    }
    cpython_register_interned_unicode(&text, created);
    created
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromObject(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if object.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyUnicode_FromObject received unknown object pointer");
            return std::ptr::null_mut();
        };
        match value {
            Value::Str(_) => context.alloc_cpython_ptr_for_value(value),
            other => match cpython_unicode_text_from_value(&other) {
                Some(text) => context.alloc_cpython_ptr_for_value(Value::Str(text)),
                None => {
                    let got = if context.vm.is_null() {
                        "object".to_string()
                    } else {
                        // SAFETY: VM pointer is valid for active C-API context lifetime.
                        unsafe { (&mut *context.vm).value_type_name_for_error(&other) }
                    };
                    context.set_error(format!("Can't convert '{got}' object to str implicitly"));
                    std::ptr::null_mut()
                }
            },
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromOrdinal(ordinal: c_int) -> *mut c_void {
    if !(0..=0x10FFFF).contains(&ordinal) {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "chr() arg not in range(0x110000)",
        );
        return std::ptr::null_mut();
    }
    let ch = if let Some(ch) = internal_char_from_codepoint(ordinal as u32) {
        ch
    } else {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "chr() arg not in range(0x110000)",
        );
        return std::ptr::null_mut();
    };
    cpython_new_ptr_for_value(Value::Str(ch.to_string()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_GetDefaultEncoding() -> *const c_char {
    static UTF8: &[u8] = b"utf-8\0";
    UTF8.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Equal(str1: *mut c_void, str2: *mut c_void) -> c_int {
    with_active_cpython_context_mut(|context| {
        let Some(value1) = context.cpython_value_from_ptr(str1) else {
            context.set_error("PyUnicode_Equal received unknown first pointer");
            return -1;
        };
        let Some(value2) = context.cpython_value_from_ptr(str2) else {
            context.set_error("PyUnicode_Equal received unknown second pointer");
            return -1;
        };
        let Some(text1) = cpython_unicode_text_from_value(&value1) else {
            let got = if context.vm.is_null() {
                "object".to_string()
            } else {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                unsafe { (&mut *context.vm).value_type_name_for_error(&value1) }
            };
            context.set_error(format!("first argument must be str, not {got}"));
            return -1;
        };
        let Some(text2) = cpython_unicode_text_from_value(&value2) else {
            let got = if context.vm.is_null() {
                "object".to_string()
            } else {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                unsafe { (&mut *context.vm).value_type_name_for_error(&value2) }
            };
            context.set_error(format!("second argument must be str, not {got}"));
            return -1;
        };
        if text1 == text2 { 1 } else { 0 }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EqualToUTF8(unicode: *mut c_void, text: *const c_char) -> c_int {
    if text.is_null() {
        return 0;
    }
    let utf8 = match unsafe { CStr::from_ptr(text).to_str() } {
        Ok(value) => value,
        Err(_) => return 0,
    };
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            return 0;
        };
        let Some(unicode_text) = cpython_unicode_text_from_value(&value) else {
            return 0;
        };
        if unicode_text == utf8 { 1 } else { 0 }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_EqualToASCIIString(
    unicode: *mut c_void,
    text: *const c_char,
) -> c_int {
    if text.is_null() {
        return 0;
    }
    let ascii = match unsafe { CStr::from_ptr(text).to_bytes() } {
        bytes if bytes.iter().all(|byte| *byte < 0x80) => bytes,
        _ => return 0,
    };
    let ascii_text = match std::str::from_utf8(ascii) {
        Ok(value) => value,
        Err(_) => return 0,
    };
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            return 0;
        };
        let Some(unicode_text) = cpython_unicode_text_from_value(&value) else {
            return 0;
        };
        if unicode_text.is_ascii() && unicode_text == ascii_text {
            1
        } else {
            0
        }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EqualToUTF8AndSize(
    unicode: *mut c_void,
    text: *const c_char,
    size: isize,
) -> c_int {
    if text.is_null() || size < 0 {
        return 0;
    }
    let bytes = if size == 0 {
        &[][..]
    } else {
        // SAFETY: caller guarantees readable `size` bytes at `text`.
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), size as usize) }
    };
    let utf8 = match std::str::from_utf8(bytes) {
        Ok(value) => value,
        Err(_) => return 0,
    };
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            return 0;
        };
        let Some(unicode_text) = cpython_unicode_text_from_value(&value) else {
            return 0;
        };
        if unicode_text == utf8 { 1 } else { 0 }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_ReadChar(unicode: *mut c_void, index: isize) -> u32 {
    let text = match cpython_value_from_ptr(unicode) {
        Ok(value) => match cpython_unicode_text_from_value(&value) {
            Some(text) => text,
            None => {
                unsafe { PyErr_BadArgument() };
                return u32::MAX;
            }
        },
        Err(err) => {
            cpython_set_error(err);
            return u32::MAX;
        }
    };
    if index < 0 || index >= text.chars().count() as isize {
        cpython_set_typed_error(unsafe { PyExc_IndexError }, "string index out of range");
        return u32::MAX;
    }
    text.chars()
        .nth(index as usize)
        .map(canonical_codepoint_for_internal_char)
        .unwrap_or(u32::MAX)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Find(
    str_obj: *mut c_void,
    substr: *mut c_void,
    start: isize,
    end: isize,
    direction: c_int,
) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(str_obj) else {
            context.set_error("PyUnicode_Find received unknown string pointer");
            return -2;
        };
        let Some(needle) = context.cpython_value_from_ptr(substr) else {
            context.set_error("PyUnicode_Find received unknown substring pointer");
            return -2;
        };
        if cpython_unicode_text_from_value(&receiver).is_none()
            || cpython_unicode_text_from_value(&needle).is_none()
        {
            context.set_error("PyUnicode_Find expects str arguments");
            return -2;
        }
        let method = if direction >= 0 { "find" } else { "rfind" };
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            method,
            vec![needle, Value::Int(start as i64), Value::Int(end as i64)],
            "PyUnicode_Find",
        ) else {
            return -2;
        };
        match value_to_int(result) {
            Ok(index) => index as isize,
            Err(_) => {
                context.set_error("PyUnicode_Find expected integer return value");
                -2
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -2
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FindChar(
    str_obj: *mut c_void,
    ch: u32,
    start: isize,
    end: isize,
    direction: c_int,
) -> isize {
    let Some(ch) = char::from_u32(ch) else {
        return -1;
    };
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(str_obj) else {
            context.set_error("PyUnicode_FindChar received unknown string pointer");
            return -1;
        };
        if cpython_unicode_text_from_value(&receiver).is_none() {
            context.set_error("PyUnicode_FindChar expects str object");
            return -1;
        }
        let method = if direction >= 0 { "find" } else { "rfind" };
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            method,
            vec![
                Value::Str(ch.to_string()),
                Value::Int(start as i64),
                Value::Int(end as i64),
            ],
            "PyUnicode_FindChar",
        ) else {
            return -1;
        };
        match value_to_int(result) {
            Ok(index) => index as isize,
            Err(_) => {
                context.set_error("PyUnicode_FindChar expected integer return value");
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Count(
    str_obj: *mut c_void,
    substr: *mut c_void,
    start: isize,
    end: isize,
) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(str_obj) else {
            context.set_error("PyUnicode_Count received unknown string pointer");
            return -1;
        };
        let Some(needle) = context.cpython_value_from_ptr(substr) else {
            context.set_error("PyUnicode_Count received unknown substring pointer");
            return -1;
        };
        if cpython_unicode_text_from_value(&receiver).is_none()
            || cpython_unicode_text_from_value(&needle).is_none()
        {
            context.set_error("PyUnicode_Count expects str arguments");
            return -1;
        }
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            "count",
            vec![needle, Value::Int(start as i64), Value::Int(end as i64)],
            "PyUnicode_Count",
        ) else {
            return -1;
        };
        match value_to_int(result) {
            Ok(count) => count as isize,
            Err(_) => {
                context.set_error("PyUnicode_Count expected integer return value");
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Join(separator: *mut c_void, seq: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(separator_value) = context.cpython_value_from_ptr(separator) else {
            context.set_error("PyUnicode_Join received unknown separator pointer");
            return std::ptr::null_mut();
        };
        let Some(seq_value) = context.cpython_value_from_ptr_or_proxy(seq) else {
            context.set_error("PyUnicode_Join received unknown sequence pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&separator_value).is_none() {
            context.set_error("PyUnicode_Join expects str separator");
            return std::ptr::null_mut();
        }
        let Some(result) = cpython_call_method_for_capi(
            context,
            separator_value,
            "join",
            vec![seq_value],
            "PyUnicode_Join",
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Split(
    string: *mut c_void,
    sep: *mut c_void,
    maxsplit: isize,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(string) else {
            context.set_error("PyUnicode_Split received unknown string pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&receiver).is_none() {
            context.set_error("PyUnicode_Split expects str receiver");
            return std::ptr::null_mut();
        }
        let args = if sep.is_null() {
            if maxsplit < 0 {
                Vec::new()
            } else {
                vec![Value::None, Value::Int(maxsplit as i64)]
            }
        } else {
            let Some(sep_value) = context.cpython_value_from_ptr(sep) else {
                context.set_error("PyUnicode_Split received unknown separator pointer");
                return std::ptr::null_mut();
            };
            vec![sep_value, Value::Int(maxsplit as i64)]
        };
        let Some(result) =
            cpython_call_method_for_capi(context, receiver, "split", args, "PyUnicode_Split")
        else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_RSplit(
    string: *mut c_void,
    sep: *mut c_void,
    maxsplit: isize,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(string) else {
            context.set_error("PyUnicode_RSplit received unknown string pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&receiver).is_none() {
            context.set_error("PyUnicode_RSplit expects str receiver");
            return std::ptr::null_mut();
        }
        let args = if sep.is_null() {
            if maxsplit < 0 {
                Vec::new()
            } else {
                vec![Value::None, Value::Int(maxsplit as i64)]
            }
        } else {
            let Some(sep_value) = context.cpython_value_from_ptr(sep) else {
                context.set_error("PyUnicode_RSplit received unknown separator pointer");
                return std::ptr::null_mut();
            };
            vec![sep_value, Value::Int(maxsplit as i64)]
        };
        let Some(result) =
            cpython_call_method_for_capi(context, receiver, "rsplit", args, "PyUnicode_RSplit")
        else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Splitlines(string: *mut c_void, keepends: c_int) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(string) else {
            context.set_error("PyUnicode_Splitlines received unknown string pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&receiver).is_none() {
            context.set_error("PyUnicode_Splitlines expects str receiver");
            return std::ptr::null_mut();
        }
        let args = if keepends == 0 {
            Vec::new()
        } else {
            vec![Value::Bool(true)]
        };
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            "splitlines",
            args,
            "PyUnicode_Splitlines",
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Partition(string: *mut c_void, sep: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(string) else {
            context.set_error("PyUnicode_Partition received unknown string pointer");
            return std::ptr::null_mut();
        };
        let Some(sep_value) = context.cpython_value_from_ptr(sep) else {
            context.set_error("PyUnicode_Partition received unknown separator pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&receiver).is_none()
            || cpython_unicode_text_from_value(&sep_value).is_none()
        {
            context.set_error("PyUnicode_Partition expects str arguments");
            return std::ptr::null_mut();
        }
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            "partition",
            vec![sep_value],
            "PyUnicode_Partition",
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_RPartition(
    string: *mut c_void,
    sep: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(string) else {
            context.set_error("PyUnicode_RPartition received unknown string pointer");
            return std::ptr::null_mut();
        };
        let Some(sep_value) = context.cpython_value_from_ptr(sep) else {
            context.set_error("PyUnicode_RPartition received unknown separator pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&receiver).is_none()
            || cpython_unicode_text_from_value(&sep_value).is_none()
        {
            context.set_error("PyUnicode_RPartition expects str arguments");
            return std::ptr::null_mut();
        }
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            "rpartition",
            vec![sep_value],
            "PyUnicode_RPartition",
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_IsIdentifier(object: *mut c_void) -> c_int {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyUnicode_IsIdentifier received unknown string pointer");
            return -1;
        };
        if cpython_unicode_text_from_value(&receiver).is_none() {
            context.set_error("PyUnicode_IsIdentifier expects str object");
            return -1;
        }
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            "isidentifier",
            Vec::new(),
            "PyUnicode_IsIdentifier",
        ) else {
            return -1;
        };
        match result {
            Value::Bool(value) => i32::from(value),
            other => {
                let got = if context.vm.is_null() {
                    "object".to_string()
                } else {
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    unsafe { (&mut *context.vm).value_type_name_for_error(&other) }
                };
                context.set_error(format!(
                    "PyUnicode_IsIdentifier expected bool return value, got {got}"
                ));
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_GetSize(unicode: *mut c_void) -> isize {
    let _ = unicode;
    cpython_set_typed_error(
        unsafe { PyExc_RuntimeError },
        "PyUnicode_GetSize has been removed.",
    );
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_InternInPlace(unicode: *mut *mut c_void) {
    if unicode.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            unsafe { PyErr_BadInternalCall() };
        }
        return;
    }
    // SAFETY: caller provides writable pointer slot.
    let object = unsafe { *unicode };
    if object.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            unsafe { PyErr_BadInternalCall() };
        }
        return;
    }
    let _ = with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(object) else {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            return;
        };
        if cpython_unicode_text_from_value(&value).is_none()
            && unsafe { PyErr_Occurred() }.is_null()
        {
            unsafe { PyErr_BadInternalCall() };
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_InternImmortal(unicode: *mut *mut c_void) {
    unsafe { PyUnicode_InternInPlace(unicode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Append(left: *mut *mut c_void, right: *mut c_void) {
    if left.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            unsafe { PyErr_BadInternalCall() };
        }
        return;
    }
    // SAFETY: caller provides writable pointer slot.
    let left_ptr = unsafe { *left };
    let mut should_clear_left = false;
    let mut replacement: Option<*mut c_void> = None;
    let status = with_active_cpython_context_mut(|context| {
        if left_ptr.is_null() || right.is_null() {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            should_clear_left = true;
            return;
        }
        let Some(left_value) = context.cpython_value_from_ptr(left_ptr) else {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            should_clear_left = true;
            return;
        };
        let Some(right_value) = context.cpython_value_from_ptr(right) else {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            should_clear_left = true;
            return;
        };
        let Some(left_text) = cpython_unicode_text_from_value(&left_value) else {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            should_clear_left = true;
            return;
        };
        let Some(right_text) = cpython_unicode_text_from_value(&right_value) else {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            should_clear_left = true;
            return;
        };
        if left_text.is_empty() {
            unsafe { Py_IncRef(right) };
            replacement = Some(right);
            return;
        }
        if right_text.is_empty() {
            return;
        }
        let combined = format!("{left_text}{right_text}");
        replacement = Some(context.alloc_cpython_ptr_for_value(Value::Str(combined)));
    });
    if status.is_err() {
        if unsafe { PyErr_Occurred() }.is_null() {
            cpython_set_error("PyUnicode_Append failed due to missing active C-API context");
        }
        should_clear_left = true;
    }
    if should_clear_left {
        // SAFETY: left points to writable slot; Py_CLEAR semantics.
        unsafe {
            if !(*left).is_null() {
                Py_DecRef(*left);
            }
            *left = std::ptr::null_mut();
        }
        return;
    }
    if let Some(new_left) = replacement {
        // SAFETY: left points to writable slot.
        unsafe {
            Py_DecRef(*left);
            *left = new_left;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AppendAndDel(left: *mut *mut c_void, right: *mut c_void) {
    unsafe { PyUnicode_Append(left, right) };
    unsafe { Py_XDecRef(right) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_RichCompare(
    left: *mut c_void,
    right: *mut c_void,
    op: c_int,
) -> *mut c_void {
    const PY_LT: c_int = 0;
    const PY_LE: c_int = 1;
    const PY_EQ: c_int = 2;
    const PY_NE: c_int = 3;
    const PY_GT: c_int = 4;
    const PY_GE: c_int = 5;

    with_active_cpython_context_mut(|context| {
        let Some(left_value) = context.cpython_value_from_ptr(left) else {
            context.set_error("PyUnicode_RichCompare received unknown left pointer");
            return std::ptr::null_mut();
        };
        let Some(right_value) = context.cpython_value_from_ptr(right) else {
            context.set_error("PyUnicode_RichCompare received unknown right pointer");
            return std::ptr::null_mut();
        };
        let left_text = cpython_unicode_text_from_value(&left_value);
        let right_text = cpython_unicode_text_from_value(&right_value);
        if left_text.is_none() || right_text.is_none() {
            let not_impl = std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast::<c_void>();
            unsafe { Py_IncRef(not_impl) };
            return not_impl;
        }
        let left_text = left_text.unwrap_or_default();
        let right_text = right_text.unwrap_or_default();
        let result = if left == right {
            match op {
                PY_EQ | PY_LE | PY_GE => Some(true),
                PY_NE | PY_LT | PY_GT => Some(false),
                _ => None,
            }
        } else {
            match op {
                PY_EQ => Some(left_text == right_text),
                PY_NE => Some(left_text != right_text),
                PY_LT => Some(left_text < right_text),
                PY_LE => Some(left_text <= right_text),
                PY_GT => Some(left_text > right_text),
                PY_GE => Some(left_text >= right_text),
                _ => None,
            }
        };
        let Some(result) = result else {
            unsafe { PyErr_BadArgument() };
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(Value::Bool(result))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_WriteChar(
    unicode: *mut c_void,
    index: isize,
    character: c_uint,
) -> c_int {
    if index < 0 {
        cpython_set_typed_error(unsafe { PyExc_IndexError }, "string index out of range");
        return -1;
    }
    let Some(ch) = char::from_u32(character) else {
        cpython_set_typed_error(unsafe { PyExc_ValueError }, "character out of range");
        return -1;
    };
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(unicode) else {
            context.set_error("PyUnicode_WriteChar received unknown object pointer");
            return -1;
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyUnicode_WriteChar received unknown object handle");
            return -1;
        };
        let Value::Str(text) = &slot.value else {
            context.set_error("PyUnicode_WriteChar expected str object");
            return -1;
        };
        let mut chars: Vec<char> = text.chars().collect();
        let idx = index as usize;
        if idx >= chars.len() {
            cpython_set_typed_error(unsafe { PyExc_IndexError }, "string index out of range");
            return -1;
        }
        chars[idx] = ch;
        slot.value = Value::Str(chars.into_iter().collect());
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_CopyCharacters(
    to: *mut c_void,
    to_start: isize,
    from: *mut c_void,
    from_start: isize,
    how_many: isize,
) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(to_handle) = context.cpython_handle_from_ptr(to) else {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        };
        let to_text = {
            let Some(to_slot) = context.objects.get(&to_handle) else {
                unsafe { PyErr_BadInternalCall() };
                return -1;
            };
            let Value::Str(to_text) = &to_slot.value else {
                unsafe { PyErr_BadInternalCall() };
                return -1;
            };
            to_text.clone()
        };
        let Some(from_value) = context.cpython_value_from_ptr_or_proxy(from) else {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        };
        let Value::Str(from_text) = from_value else {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        };

        let from_chars: Vec<char> = from_text.chars().collect();
        let mut to_chars: Vec<char> = to_text.chars().collect();

        if from_start < 0 || (from_start as usize) > from_chars.len() {
            cpython_set_typed_error(unsafe { PyExc_IndexError }, "string index out of range");
            return -1;
        }
        if to_start < 0 || (to_start as usize) > to_chars.len() {
            cpython_set_typed_error(unsafe { PyExc_IndexError }, "string index out of range");
            return -1;
        }
        if how_many < 0 {
            cpython_set_typed_error(unsafe { PyExc_SystemError }, "how_many cannot be negative");
            return -1;
        }

        let from_index = from_start as usize;
        let to_index = to_start as usize;
        let requested = how_many as usize;
        let available = from_chars.len().saturating_sub(from_index);
        let copy_len = available.min(requested);
        if to_index + copy_len > to_chars.len() {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                format!(
                    "Cannot write {} characters at {} in a string of {} characters",
                    copy_len,
                    to_start,
                    to_chars.len()
                ),
            );
            return -1;
        }
        if copy_len == 0 {
            return 0;
        }

        for offset in 0..copy_len {
            to_chars[to_index + offset] = from_chars[from_index + offset];
        }

        if let Some(slot) = context.objects.get_mut(&to_handle) {
            slot.value = Value::Str(to_chars.into_iter().collect());
            context.sync_cpython_storage_from_value(to_handle);
            copy_len as isize
        } else {
            unsafe { PyErr_BadInternalCall() };
            -1
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

fn cpython_unicode_decode_common(
    bytes_ptr: *const c_char,
    size: isize,
    encoding: *const c_char,
    errors: *const c_char,
    default_encoding: &str,
    api_name: &str,
) -> *mut c_void {
    if size < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            format!("{api_name} received negative size"),
        );
        return std::ptr::null_mut();
    }
    if bytes_ptr.is_null() && size != 0 {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let encoding_name = match cpython_codec_name_or_default(encoding, default_encoding, api_name) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let errors_name = match cpython_codec_error_name_optional(errors, api_name) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error(format!("{api_name} missing VM context"));
            return std::ptr::null_mut();
        }
        let raw = if size == 0 {
            Vec::new()
        } else {
            // SAFETY: caller guarantees readable `size` bytes from `bytes_ptr`.
            unsafe { std::slice::from_raw_parts(bytes_ptr.cast::<u8>(), size as usize).to_vec() }
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let source = vm.heap.alloc_bytes(raw);
        let Some(decoded) = cpython_unicode_decode_with_codec_in_context(
            context,
            source,
            encoding_name,
            errors_name,
            api_name,
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(decoded)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_unicode_decode_with_encoding_name(
    bytes_ptr: *const c_char,
    size: isize,
    encoding_name: String,
    errors: *const c_char,
    api_name: &str,
) -> *mut c_void {
    if size < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            format!("{api_name} received negative size"),
        );
        return std::ptr::null_mut();
    }
    if bytes_ptr.is_null() && size != 0 {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let errors_name = match cpython_codec_error_name_optional(errors, api_name) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error(format!("{api_name} missing VM context"));
            return std::ptr::null_mut();
        }
        let raw = if size == 0 {
            Vec::new()
        } else {
            // SAFETY: caller guarantees readable `size` bytes from `bytes_ptr`.
            unsafe { std::slice::from_raw_parts(bytes_ptr.cast::<u8>(), size as usize).to_vec() }
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let source = vm.heap.alloc_bytes(raw);
        let Some(decoded) = cpython_unicode_decode_with_codec_in_context(
            context,
            source,
            encoding_name,
            errors_name,
            api_name,
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(decoded)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_unicode_encode_with_encoding_name(
    unicode: *mut c_void,
    encoding_name: String,
    errors: *const c_char,
    api_name: &str,
) -> *mut c_void {
    if unicode.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let errors_name = match cpython_codec_error_name_optional(errors, api_name) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let Some(unicode_value) = context.cpython_value_from_ptr_or_proxy(unicode) else {
            context.set_error(format!("{api_name} received unknown unicode pointer"));
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&unicode_value).is_none() {
            context.set_error(format!("{api_name} expected str object"));
            return std::ptr::null_mut();
        }
        let Some(encoded) = cpython_unicode_encode_with_codec_in_context(
            context,
            unicode_value,
            encoding_name,
            errors_name,
            api_name,
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(encoded)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_codepage_encoding_name(code_page: c_int) -> String {
    if code_page <= 0 || code_page == 65001 {
        "utf-8".to_string()
    } else {
        format!("cp{code_page}")
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Decode(
    bytes_ptr: *const c_char,
    size: isize,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        encoding,
        errors,
        "utf-8",
        "PyUnicode_Decode",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeASCII(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"ascii".as_ptr(),
        errors,
        "ascii",
        "PyUnicode_DecodeASCII",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeLatin1(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"latin-1".as_ptr(),
        errors,
        "latin-1",
        "PyUnicode_DecodeLatin1",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF8(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-8".as_ptr(),
        errors,
        "utf-8",
        "PyUnicode_DecodeUTF8",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF8Stateful(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    consumed: *mut isize,
) -> *mut c_void {
    let result = cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-8".as_ptr(),
        errors,
        "utf-8",
        "PyUnicode_DecodeUTF8Stateful",
    );
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable pointer for consumed output.
        unsafe { *consumed = size };
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF7(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    let result = cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-7".as_ptr(),
        errors,
        "utf-7",
        "PyUnicode_DecodeUTF7",
    );
    if !result.is_null() {
        return result;
    }
    if !unsafe { PyErr_Occurred() }.is_null() {
        unsafe { PyErr_Clear() };
    }
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-8".as_ptr(),
        errors,
        "utf-8",
        "PyUnicode_DecodeUTF7",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF7Stateful(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    consumed: *mut isize,
) -> *mut c_void {
    let result = unsafe { PyUnicode_DecodeUTF7(bytes_ptr, size, errors) };
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable consumed output pointer.
        unsafe {
            *consumed = size;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeMBCS(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_with_encoding_name(
        bytes_ptr,
        size,
        cpython_codepage_encoding_name(0),
        errors,
        "PyUnicode_DecodeMBCS",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeMBCSStateful(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    consumed: *mut isize,
) -> *mut c_void {
    let result = unsafe { PyUnicode_DecodeMBCS(bytes_ptr, size, errors) };
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable consumed output pointer.
        unsafe {
            *consumed = size;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeCodePageStateful(
    code_page: c_int,
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    consumed: *mut isize,
) -> *mut c_void {
    let result = cpython_unicode_decode_with_encoding_name(
        bytes_ptr,
        size,
        cpython_codepage_encoding_name(code_page),
        errors,
        "PyUnicode_DecodeCodePageStateful",
    );
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable consumed output pointer.
        unsafe {
            *consumed = size;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeCharmap(
    bytes_ptr: *const c_char,
    size: isize,
    mapping: *mut c_void,
    errors: *const c_char,
) -> *mut c_void {
    if size < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyUnicode_DecodeCharmap received negative size",
        );
        return std::ptr::null_mut();
    }
    if bytes_ptr.is_null() && size != 0 {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if mapping.is_null() {
        let raw = if size == 0 {
            Vec::new()
        } else {
            // SAFETY: caller guarantees readable `size` bytes from `bytes_ptr`.
            unsafe { std::slice::from_raw_parts(bytes_ptr.cast::<u8>(), size as usize).to_vec() }
        };
        let decoded: String = raw.into_iter().map(char::from).collect();
        return cpython_new_ptr_for_value(Value::Str(decoded));
    }
    let errors_name = match cpython_codec_error_name_optional(errors, "PyUnicode_DecodeCharmap") {
        Ok(name) => name.unwrap_or_else(|| "strict".to_string()),
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let mapping_value = if mapping.is_null() {
            Value::None
        } else if let Some(value) = context.cpython_value_from_ptr_or_proxy(mapping) {
            value
        } else {
            context.set_error("PyUnicode_DecodeCharmap received unknown mapping object");
            return std::ptr::null_mut();
        };
        let codec_module = match cpython_codec_module_in_context(context) {
            Ok(module) => module,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let charmap_decode = {
            let Object::Module(module_data) = &*codec_module.kind() else {
                context.set_error("codecs module is invalid");
                return std::ptr::null_mut();
            };
            match module_data.globals.get("charmap_decode").cloned() {
                Some(value) => value,
                None => {
                    context.set_error("codecs.charmap_decode unavailable");
                    return std::ptr::null_mut();
                }
            }
        };
        if context.vm.is_null() {
            context.set_error("PyUnicode_DecodeCharmap missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let raw = if size == 0 {
            Vec::new()
        } else {
            // SAFETY: caller guarantees readable `size` bytes from `bytes_ptr`.
            unsafe { std::slice::from_raw_parts(bytes_ptr.cast::<u8>(), size as usize).to_vec() }
        };
        let bytes_value = vm.heap.alloc_bytes(raw);
        let decoded_tuple = match cpython_call_internal_in_context(
            context,
            charmap_decode,
            vec![bytes_value, Value::Str(errors_name), mapping_value],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let Value::Tuple(parts) = decoded_tuple else {
            context.set_error("codecs.charmap_decode() returned non-tuple");
            return std::ptr::null_mut();
        };
        let Object::Tuple(items) = &*parts.kind() else {
            context.set_error("codecs.charmap_decode() returned invalid tuple");
            return std::ptr::null_mut();
        };
        let Some(decoded) = items.first().cloned() else {
            context.set_error("codecs.charmap_decode() returned empty tuple");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&decoded).is_none() {
            context.set_error("codecs.charmap_decode() did not return unicode text");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(decoded)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_BuildEncodingMap(string: *mut c_void) -> *mut c_void {
    if string.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let unicode_value = match cpython_value_from_ptr(string) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let Some(text) = cpython_unicode_text_from_value(&unicode_value) else {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "PyUnicode_BuildEncodingMap expected str object",
        );
        return std::ptr::null_mut();
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyUnicode_BuildEncodingMap missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let dict = vm.heap.alloc_dict(Vec::new());
        let Value::Dict(dict_obj) = &dict else {
            context.set_error("PyUnicode_BuildEncodingMap internal dict allocation failed");
            return std::ptr::null_mut();
        };
        for (index, ch) in text.chars().enumerate() {
            let key = Value::Int(ch as i64);
            let value = Value::Int(index as i64);
            if let Err(err) = dict_set_value_checked(dict_obj, key, value) {
                context.set_error(err.message);
                return std::ptr::null_mut();
            }
        }
        context.alloc_cpython_ptr_for_value(dict)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeRawUnicodeEscape(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"raw_unicode_escape".as_ptr(),
        errors,
        "raw_unicode_escape",
        "PyUnicode_DecodeRawUnicodeEscape",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUnicodeEscape(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"unicode_escape".as_ptr(),
        errors,
        "unicode_escape",
        "PyUnicode_DecodeUnicodeEscape",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF16(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    byteorder: *mut c_int,
) -> *mut c_void {
    let result = cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-16".as_ptr(),
        errors,
        "utf-16",
        "PyUnicode_DecodeUTF16",
    );
    if !result.is_null() && !byteorder.is_null() {
        // SAFETY: caller provided writable byteorder output pointer.
        unsafe {
            *byteorder = 0;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF16Stateful(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    byteorder: *mut c_int,
    consumed: *mut isize,
) -> *mut c_void {
    let result = unsafe { PyUnicode_DecodeUTF16(bytes_ptr, size, errors, byteorder) };
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable consumed output pointer.
        unsafe {
            *consumed = size;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF32(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    byteorder: *mut c_int,
) -> *mut c_void {
    let result = cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-32".as_ptr(),
        errors,
        "utf-32",
        "PyUnicode_DecodeUTF32",
    );
    if !result.is_null() && !byteorder.is_null() {
        // SAFETY: caller provided writable byteorder output pointer.
        unsafe {
            *byteorder = 0;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF32Stateful(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    byteorder: *mut c_int,
    consumed: *mut isize,
) -> *mut c_void {
    let result = unsafe { PyUnicode_DecodeUTF32(bytes_ptr, size, errors, byteorder) };
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable consumed output pointer.
        unsafe {
            *consumed = size;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeFSDefaultAndSize(
    bytes_ptr: *const c_char,
    size: isize,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-8".as_ptr(),
        std::ptr::null(),
        "utf-8",
        "PyUnicode_DecodeFSDefaultAndSize",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeFSDefault(bytes_ptr: *const c_char) -> *mut c_void {
    if bytes_ptr.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: bytes_ptr is expected to be NUL-terminated.
    let size = unsafe { CStr::from_ptr(bytes_ptr).to_bytes().len() as isize };
    unsafe { PyUnicode_DecodeFSDefaultAndSize(bytes_ptr, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeLocaleAndSize(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-8".as_ptr(),
        errors,
        "utf-8",
        "PyUnicode_DecodeLocaleAndSize",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeLocale(
    bytes_ptr: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    if bytes_ptr.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: bytes_ptr is expected to be NUL-terminated.
    let size = unsafe { CStr::from_ptr(bytes_ptr).to_bytes().len() as isize };
    unsafe { PyUnicode_DecodeLocaleAndSize(bytes_ptr, size, errors) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EncodeFSDefault(unicode: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedString(unicode, c"utf-8".as_ptr(), std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EncodeLocale(
    unicode: *mut c_void,
    errors: *const c_char,
) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedString(unicode, c"utf-8".as_ptr(), errors) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EncodeCodePage(
    code_page: c_int,
    unicode: *mut c_void,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_encode_with_encoding_name(
        unicode,
        cpython_codepage_encoding_name(code_page),
        errors,
        "PyUnicode_EncodeCodePage",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsDecodedObject(
    unicode: *mut c_void,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    if unicode.is_null() {
        unsafe { PyErr_BadArgument() };
        return std::ptr::null_mut();
    }
    let encoding_name =
        match cpython_codec_name_or_default(encoding, "utf-8", "PyUnicode_AsDecodedObject") {
            Ok(name) => name,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
    let errors_name = match cpython_codec_error_name_optional(errors, "PyUnicode_AsDecodedObject") {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            context.set_error("PyUnicode_AsDecodedObject received unknown object pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&value).is_none() {
            unsafe { PyErr_BadArgument() };
            return std::ptr::null_mut();
        }
        if let Some(text) = cpython_unicode_text_from_value(&value)
            && cpython_codec_is_rot13(&encoding_name)
        {
            return context.alloc_cpython_ptr_for_value(Value::Str(cpython_rot13_text(&text)));
        }
        let mut args = vec![value, Value::Str(encoding_name)];
        if let Some(errors_name) = errors_name {
            args.push(Value::Str(errors_name));
        }
        match cpython_call_internal_in_context(
            context,
            Value::Builtin(BuiltinFunction::CodecsDecode),
            args,
            HashMap::new(),
        ) {
            Ok(decoded) => context.alloc_cpython_ptr_for_value(decoded),
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
pub unsafe extern "C" fn PyUnicode_AsDecodedUnicode(
    unicode: *mut c_void,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    let result = unsafe { PyUnicode_AsDecodedObject(unicode, encoding, errors) };
    if result.is_null() {
        return std::ptr::null_mut();
    }
    match cpython_value_from_ptr(result) {
        Ok(value) if cpython_unicode_text_from_value(&value).is_some() => result,
        Ok(value) => {
            let got = with_active_cpython_context_mut(|context| {
                if context.vm.is_null() {
                    "object".to_string()
                } else {
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    unsafe { (&mut *context.vm).value_type_name_for_error(&value) }
                }
            })
            .unwrap_or_else(|_| "object".to_string());
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                format!("decoder returned '{got}' instead of 'str'"),
            );
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsEncodedObject(
    unicode: *mut c_void,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    if unicode.is_null() {
        unsafe { PyErr_BadArgument() };
        return std::ptr::null_mut();
    }
    let encoding_name =
        match cpython_codec_name_or_default(encoding, "utf-8", "PyUnicode_AsEncodedObject") {
            Ok(name) => name,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
    let errors_name = match cpython_codec_error_name_optional(errors, "PyUnicode_AsEncodedObject") {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            context.set_error("PyUnicode_AsEncodedObject received unknown object pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&value).is_none() {
            unsafe { PyErr_BadArgument() };
            return std::ptr::null_mut();
        }
        let Some(encoded) = cpython_unicode_encode_with_codec_in_context(
            context,
            value,
            encoding_name,
            errors_name,
            "PyUnicode_AsEncodedObject",
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(encoded)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsEncodedUnicode(
    unicode: *mut c_void,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    let result = unsafe { PyUnicode_AsEncodedObject(unicode, encoding, errors) };
    if result.is_null() {
        return std::ptr::null_mut();
    }
    match cpython_value_from_ptr(result) {
        Ok(value) if cpython_unicode_text_from_value(&value).is_some() => result,
        Ok(value) => {
            let got = with_active_cpython_context_mut(|context| {
                if context.vm.is_null() {
                    "object".to_string()
                } else {
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    unsafe { (&mut *context.vm).value_type_name_for_error(&value) }
                }
            })
            .unwrap_or_else(|_| "object".to_string());
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                format!("encoder returned '{got}' instead of 'str'"),
            );
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FSConverter(arg: *mut c_void, addr: *mut c_void) -> c_int {
    if addr.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return 0;
    }
    if arg.is_null() {
        // SAFETY: caller passes output slot pointer when requesting cleanup.
        unsafe {
            let slot = addr.cast::<*mut c_void>();
            if !(*slot).is_null() {
                Py_DecRef(*slot);
            }
            *slot = std::ptr::null_mut();
        }
        return 1;
    }
    let path = unsafe { PyOS_FSPath(arg) };
    if path.is_null() {
        return 0;
    }
    let output = match cpython_value_from_ptr(path) {
        Ok(Value::Bytes(_)) => path,
        Ok(value) if cpython_unicode_text_from_value(&value).is_some() => {
            let encoded = unsafe { PyUnicode_EncodeFSDefault(path) };
            unsafe { Py_DecRef(path) };
            if encoded.is_null() {
                return 0;
            }
            encoded
        }
        _ => {
            unsafe { Py_DecRef(path) };
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "path should be string, bytes, or os.PathLike",
            );
            return 0;
        }
    };
    // SAFETY: caller provides writable PyObject** slot in addr.
    unsafe {
        let slot = addr.cast::<*mut c_void>();
        *slot = output;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FSDecoder(arg: *mut c_void, addr: *mut c_void) -> c_int {
    if addr.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return 0;
    }
    if arg.is_null() {
        // SAFETY: caller passes output slot pointer when requesting cleanup.
        unsafe {
            let slot = addr.cast::<*mut c_void>();
            if !(*slot).is_null() {
                Py_DecRef(*slot);
            }
            *slot = std::ptr::null_mut();
        }
        return 1;
    }
    let path = unsafe { PyOS_FSPath(arg) };
    if path.is_null() {
        return 0;
    }
    let output = match cpython_value_from_ptr(path) {
        Ok(value) if cpython_unicode_text_from_value(&value).is_some() => path,
        Ok(Value::Bytes(bytes_obj)) => {
            let size = match &*bytes_obj.kind() {
                Object::Bytes(values) => values.len() as isize,
                _ => {
                    unsafe { Py_DecRef(path) };
                    cpython_set_error("PyUnicode_FSDecoder encountered invalid bytes storage");
                    return 0;
                }
            };
            let data = unsafe { PyBytes_AsString(path) };
            let decoded = unsafe { PyUnicode_DecodeFSDefaultAndSize(data, size) };
            unsafe { Py_DecRef(path) };
            if decoded.is_null() {
                return 0;
            }
            decoded
        }
        _ => {
            unsafe { Py_DecRef(path) };
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "path should be string, bytes, or os.PathLike",
            );
            return 0;
        }
    };
    // SAFETY: caller provides writable PyObject** slot in addr.
    unsafe {
        let slot = addr.cast::<*mut c_void>();
        *slot = output;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Translate(
    object: *mut c_void,
    table: *mut c_void,
    errors: *const c_char,
) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if let Err(err) = cpython_codec_error_name_optional(errors, "PyUnicode_Translate") {
        cpython_set_error(err);
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyUnicode_Translate missing VM context");
            return std::ptr::null_mut();
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyUnicode_Translate received unknown object");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&object_value).is_none() {
            context.set_error("PyUnicode_Translate expected str receiver");
            return std::ptr::null_mut();
        }
        let table_value = if table.is_null() {
            Value::None
        } else if let Some(value) = context.cpython_value_from_ptr_or_proxy(table) {
            value
        } else {
            context.set_error("PyUnicode_Translate received unknown mapping table");
            return std::ptr::null_mut();
        };
        let translate = match cpython_getattr_in_context(context, object_value, "translate") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let translated = match cpython_call_internal_in_context(
            context,
            translate,
            vec![table_value],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        if cpython_unicode_text_from_value(&translated).is_none() {
            context.set_error("str.translate() returned non-str result");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(translated)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Resize(unicode: *mut *mut c_void, length: isize) -> c_int {
    if unicode.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if length < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyUnicode_Resize received negative size",
        );
        return -1;
    }
    // SAFETY: caller provides writable unicode pointer.
    let current = unsafe { *unicode };
    if current.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let value = match cpython_value_from_ptr(current) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let Some(text) = cpython_unicode_text_from_value(&value) else {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "PyUnicode_Resize expected str object",
        );
        return -1;
    };
    let mut chars: Vec<char> = text.chars().collect();
    if (length as usize) < chars.len() {
        chars.truncate(length as usize);
    } else if (length as usize) > chars.len() {
        chars.resize(length as usize, '\0');
    }
    let resized = chars.into_iter().collect::<String>();
    let resized_ptr = cpython_new_ptr_for_value(Value::Str(resized));
    if resized_ptr.is_null() {
        return -1;
    }
    // SAFETY: output slot and old object pointer are valid for update/decref.
    unsafe {
        *unicode = resized_ptr;
        Py_DecRef(current);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Replace(
    object: *mut c_void,
    substr: *mut c_void,
    repl: *mut c_void,
    count: isize,
) -> *mut c_void {
    let object = match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Replace expected str receiver");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let substr = match cpython_value_from_ptr(substr) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Replace expected str search value");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let repl = match cpython_value_from_ptr(repl) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Replace expected str replacement");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let replaced = if count < 0 {
        object.replace(&substr, &repl)
    } else {
        object.replacen(&substr, &repl, count as usize)
    };
    cpython_new_ptr_for_value(Value::Str(replaced))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Substring(
    object: *mut c_void,
    start: isize,
    end: isize,
) -> *mut c_void {
    let text = match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Substring expected str receiver");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as isize;
    let lo = start.clamp(0, len) as usize;
    let hi = end.clamp(0, len) as usize;
    let slice = if hi >= lo {
        chars[lo..hi].iter().collect::<String>()
    } else {
        String::new()
    };
    cpython_new_ptr_for_value(Value::Str(slice))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Tailmatch(
    object: *mut c_void,
    substr: *mut c_void,
    start: isize,
    end: isize,
    direction: i32,
) -> isize {
    let text = match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Tailmatch expected str receiver");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let suffix = match cpython_value_from_ptr(substr) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Tailmatch expected str suffix");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as isize;
    let lo = start.clamp(0, len) as usize;
    let hi = end.clamp(0, len) as usize;
    let section = if hi >= lo {
        chars[lo..hi].iter().collect::<String>()
    } else {
        String::new()
    };
    let matched = if direction >= 0 {
        section.ends_with(&suffix)
    } else {
        section.starts_with(&suffix)
    };
    if matched { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUCS4(
    object: *mut c_void,
    buffer: *mut u32,
    buflen: isize,
    copy_null: i32,
) -> *mut u32 {
    if buffer.is_null() || buflen < 0 {
        cpython_set_error("PyUnicode_AsUCS4 received invalid output buffer");
        return std::ptr::null_mut();
    }
    let text = match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_AsUCS4 expected str object");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let mut units: Vec<u32> = text.chars().map(|ch| ch as u32).collect();
    if copy_null != 0 {
        units.push(0);
    }
    if units.len() > buflen as usize {
        cpython_set_error("PyUnicode_AsUCS4 output buffer too small");
        return std::ptr::null_mut();
    }
    // SAFETY: caller provided writable buffer with buflen entries.
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), buffer, units.len());
    }
    buffer
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUCS4Copy(object: *mut c_void) -> *mut u32 {
    let text = match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_AsUCS4Copy expected str object");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let mut units: Vec<u32> = text.chars().map(|ch| ch as u32).collect();
    units.push(0);
    let bytes = units
        .len()
        .checked_mul(std::mem::size_of::<u32>())
        .unwrap_or(0);
    if bytes == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: allocate and copy raw u32 buffer for caller-owned lifetime.
    let raw = unsafe { PyMem_Malloc(bytes) }.cast::<u32>();
    if raw.is_null() {
        cpython_set_error("PyUnicode_AsUCS4Copy allocation failed");
        return std::ptr::null_mut();
    }
    // SAFETY: raw buffer has at least `units.len()` u32 slots.
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), raw, units.len());
    }
    raw
}

fn cpython_external_releasebuffer_slot(
    object: *mut c_void,
) -> Option<unsafe extern "C" fn(*mut c_void, *mut c_void)> {
    if object.is_null() {
        return None;
    }
    // SAFETY: caller provided a candidate CPython object pointer.
    let object_type = unsafe {
        object
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if object_type.is_null() {
        return None;
    }
    // SAFETY: `object_type` is non-null and inspected read-only for slot pointers.
    let buffer_procs = unsafe { (*object_type).tp_as_buffer.cast::<CpythonBufferProcs>() };
    if buffer_procs.is_null() {
        return None;
    }
    // SAFETY: buffer-procs table is non-null in this branch.
    unsafe { (*buffer_procs).bf_releasebuffer }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_Release(view: *mut c_void) {
    if view.is_null() {
        return;
    }
    // SAFETY: caller provided a valid Py_buffer-compatible pointer.
    let view_ref = unsafe { &mut *view.cast::<CpythonBuffer>() };
    let object = view_ref.obj;
    let internal = view_ref.internal;
    let mut released_owned_internal = false;
    if !internal.is_null() {
        let _ = with_active_cpython_context_mut(|context| {
            if let Some(handle) = context.take_owned_buffer_internal_handle(internal) {
                let _ = context.object_release_buffer(handle);
                released_owned_internal = true;
            }
        });
    }
    if !released_owned_internal
        && !object.is_null()
        && let Some(releasebuffer) = cpython_external_releasebuffer_slot(object)
    {
        // SAFETY: release slot originates from the object's `tp_as_buffer` table.
        unsafe { releasebuffer(object, view) };
    }
    if !object.is_null() {
        unsafe { Py_XDecRef(object) };
    }
    view_ref.buf = std::ptr::null_mut();
    view_ref.obj = std::ptr::null_mut();
    view_ref.len = 0;
    view_ref.itemsize = 0;
    view_ref.readonly = 1;
    view_ref.ndim = 0;
    view_ref.format = std::ptr::null_mut();
    view_ref.shape = std::ptr::null_mut();
    view_ref.strides = std::ptr::null_mut();
    view_ref.suboffsets = std::ptr::null_mut();
    view_ref.internal = std::ptr::null_mut();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCallable_Check(object: *mut c_void) -> i32 {
    match with_active_cpython_context_mut(|context| {
        let raw_ptr_is_callable = |ptr: *mut c_void| -> bool {
            if ptr.is_null() {
                return false;
            }
            // SAFETY: pointer is inspected as a CPython object header.
            let type_ptr = unsafe {
                ptr.cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if type_ptr.is_null() {
                return false;
            }
            // SAFETY: type pointer is valid for call-slot metadata inspection.
            let has_tp_call = unsafe { !(*type_ptr).tp_call.is_null() };
            if has_tp_call {
                return true;
            }
            // SAFETY: vectorcall resolver only inspects callable layout metadata.
            unsafe { cpython_resolve_vectorcall(ptr).is_some() }
        };
        let value = context.cpython_value_from_ptr(object);
        if context.vm.is_null() {
            context.set_error("PyCallable_Check missing VM context");
            return -1;
        }
        let result = if let Some(value) = value.as_ref() {
            // SAFETY: VM pointer is valid for context lifetime.
            let vm = unsafe { &mut *context.vm };
            if vm.is_callable_value(&value) {
                1
            } else if let Some(raw_proxy) =
                ModuleCapiContext::cpython_proxy_raw_ptr_from_value(&value)
            {
                if raw_ptr_is_callable(raw_proxy) { 1 } else { 0 }
            } else if raw_ptr_is_callable(object) {
                1
            } else {
                0
            }
        } else if raw_ptr_is_callable(object) {
            1
        } else {
            0
        };
        if super::super::env_var_present_cached("PYRS_TRACE_NUMPY_INIT") {
            let value_tag = value
                .as_ref()
                .map(cpython_value_debug_tag)
                .unwrap_or_else(|| "<raw-foreign>".to_string());
            eprintln!(
                "[numpy-init] PyCallable_Check object={:p} value_tag={} result={}",
                object, value_tag, result
            );
        }
        result
    }) {
        Ok(result) => result,
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIndex_Check(object: *mut c_void) -> i32 {
    match cpython_value_from_ptr(object) {
        Ok(Value::Bool(_) | Value::Int(_) | Value::BigInt(_)) => 1,
        Ok(_) => 0,
        Err(_) => 0,
    }
}
