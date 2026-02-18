use std::ffi::{c_char, c_int, c_void};

use crate::runtime::{ExceptionObject, Value};

use super::cpython_unicode_error_runtime::{
    CpythonUnicodeErrorFlavor, cpython_unicode_error_get_encoding_common,
    cpython_unicode_error_get_end_common, cpython_unicode_error_get_object_common,
    cpython_unicode_error_get_reason_common, cpython_unicode_error_get_start_common,
    cpython_unicode_error_set_index_common, cpython_unicode_error_set_reason_common,
};
use super::{
    PyExc_SystemError, c_name_to_string, cpython_set_error, cpython_set_typed_error,
    with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_Create(
    encoding: *const c_char,
    object: *const c_char,
    length: isize,
    start: isize,
    end: isize,
    reason: *const c_char,
) -> *mut c_void {
    if length < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyUnicodeDecodeError_Create received negative length",
        );
        return std::ptr::null_mut();
    }
    if encoding.is_null() || reason.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyUnicodeDecodeError_Create received null encoding or reason",
        );
        return std::ptr::null_mut();
    }
    if object.is_null() && length > 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyUnicodeDecodeError_Create received null object with non-zero length",
        );
        return std::ptr::null_mut();
    }
    let encoding_text = match unsafe { c_name_to_string(encoding) } {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(format!(
                "PyUnicodeDecodeError_Create invalid encoding: {err}"
            ));
            return std::ptr::null_mut();
        }
    };
    let reason_text = match unsafe { c_name_to_string(reason) } {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(format!("PyUnicodeDecodeError_Create invalid reason: {err}"));
            return std::ptr::null_mut();
        }
    };
    let payload = if length == 0 {
        Vec::new()
    } else {
        // SAFETY: `object` is validated non-null above for non-zero `length`.
        unsafe { std::slice::from_raw_parts(object.cast::<u8>(), length as usize).to_vec() }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyUnicodeDecodeError_Create missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid while C-API context is active.
        let vm = unsafe { &mut *context.vm };
        let object_value = vm.heap.alloc_bytes(payload);
        let start_value = Value::Int(start as i64);
        let end_value = Value::Int(end as i64);
        let exception = ExceptionObject::new("UnicodeDecodeError".to_string(), None);
        {
            let mut attrs = exception.attrs.borrow_mut();
            attrs.insert("encoding".to_string(), Value::Str(encoding_text.clone()));
            attrs.insert("object".to_string(), object_value.clone());
            attrs.insert("start".to_string(), start_value.clone());
            attrs.insert("end".to_string(), end_value.clone());
            attrs.insert("reason".to_string(), Value::Str(reason_text.clone()));
            attrs.insert(
                "args".to_string(),
                vm.heap.alloc_tuple(vec![
                    Value::Str(encoding_text),
                    object_value,
                    start_value,
                    end_value,
                    Value::Str(reason_text),
                ]),
            );
        }
        context.alloc_cpython_ptr_for_value(Value::Exception(Box::new(exception)))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_GetEncoding(self_obj: *mut c_void) -> *mut c_void {
    cpython_unicode_error_get_encoding_common(self_obj, CpythonUnicodeErrorFlavor::Encode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_GetEncoding(self_obj: *mut c_void) -> *mut c_void {
    cpython_unicode_error_get_encoding_common(self_obj, CpythonUnicodeErrorFlavor::Decode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_GetObject(self_obj: *mut c_void) -> *mut c_void {
    cpython_unicode_error_get_object_common(self_obj, CpythonUnicodeErrorFlavor::Encode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_GetObject(self_obj: *mut c_void) -> *mut c_void {
    cpython_unicode_error_get_object_common(self_obj, CpythonUnicodeErrorFlavor::Decode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_GetObject(self_obj: *mut c_void) -> *mut c_void {
    cpython_unicode_error_get_object_common(self_obj, CpythonUnicodeErrorFlavor::Translate)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_GetStart(
    self_obj: *mut c_void,
    start: *mut isize,
) -> c_int {
    cpython_unicode_error_get_start_common(self_obj, start, CpythonUnicodeErrorFlavor::Encode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_GetStart(
    self_obj: *mut c_void,
    start: *mut isize,
) -> c_int {
    cpython_unicode_error_get_start_common(self_obj, start, CpythonUnicodeErrorFlavor::Decode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_GetStart(
    self_obj: *mut c_void,
    start: *mut isize,
) -> c_int {
    cpython_unicode_error_get_start_common(self_obj, start, CpythonUnicodeErrorFlavor::Translate)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_SetStart(
    self_obj: *mut c_void,
    start: isize,
) -> c_int {
    cpython_unicode_error_set_index_common(
        self_obj,
        "start",
        start,
        CpythonUnicodeErrorFlavor::Encode,
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_SetStart(
    self_obj: *mut c_void,
    start: isize,
) -> c_int {
    cpython_unicode_error_set_index_common(
        self_obj,
        "start",
        start,
        CpythonUnicodeErrorFlavor::Decode,
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_SetStart(
    self_obj: *mut c_void,
    start: isize,
) -> c_int {
    cpython_unicode_error_set_index_common(
        self_obj,
        "start",
        start,
        CpythonUnicodeErrorFlavor::Translate,
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_GetEnd(
    self_obj: *mut c_void,
    end: *mut isize,
) -> c_int {
    cpython_unicode_error_get_end_common(self_obj, end, CpythonUnicodeErrorFlavor::Encode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_GetEnd(
    self_obj: *mut c_void,
    end: *mut isize,
) -> c_int {
    cpython_unicode_error_get_end_common(self_obj, end, CpythonUnicodeErrorFlavor::Decode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_GetEnd(
    self_obj: *mut c_void,
    end: *mut isize,
) -> c_int {
    cpython_unicode_error_get_end_common(self_obj, end, CpythonUnicodeErrorFlavor::Translate)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_SetEnd(self_obj: *mut c_void, end: isize) -> c_int {
    cpython_unicode_error_set_index_common(self_obj, "end", end, CpythonUnicodeErrorFlavor::Encode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_SetEnd(self_obj: *mut c_void, end: isize) -> c_int {
    cpython_unicode_error_set_index_common(self_obj, "end", end, CpythonUnicodeErrorFlavor::Decode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_SetEnd(
    self_obj: *mut c_void,
    end: isize,
) -> c_int {
    cpython_unicode_error_set_index_common(
        self_obj,
        "end",
        end,
        CpythonUnicodeErrorFlavor::Translate,
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_GetReason(self_obj: *mut c_void) -> *mut c_void {
    cpython_unicode_error_get_reason_common(self_obj, CpythonUnicodeErrorFlavor::Encode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_GetReason(self_obj: *mut c_void) -> *mut c_void {
    cpython_unicode_error_get_reason_common(self_obj, CpythonUnicodeErrorFlavor::Decode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_GetReason(self_obj: *mut c_void) -> *mut c_void {
    cpython_unicode_error_get_reason_common(self_obj, CpythonUnicodeErrorFlavor::Translate)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_SetReason(
    self_obj: *mut c_void,
    reason: *const c_char,
) -> c_int {
    cpython_unicode_error_set_reason_common(self_obj, reason, CpythonUnicodeErrorFlavor::Encode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_SetReason(
    self_obj: *mut c_void,
    reason: *const c_char,
) -> c_int {
    cpython_unicode_error_set_reason_common(self_obj, reason, CpythonUnicodeErrorFlavor::Decode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_SetReason(
    self_obj: *mut c_void,
    reason: *const c_char,
) -> c_int {
    cpython_unicode_error_set_reason_common(self_obj, reason, CpythonUnicodeErrorFlavor::Translate)
}
