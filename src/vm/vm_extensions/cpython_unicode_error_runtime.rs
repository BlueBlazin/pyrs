use std::ffi::{c_char, c_int, c_void};

use crate::runtime::{Object, Value};
use crate::vm::Vm;

use super::{
    BYTES_BACKING_STORAGE_ATTR, ModuleCapiContext, STR_BACKING_STORAGE_ATTR,
    PyErr_BadInternalCall, c_name_to_string, cpython_set_error, exception_type_is_subclass,
    value_to_int, with_active_cpython_context_mut,
};

pub(in crate::vm::vm_extensions) fn cpython_exception_value_attr(value: &Value) -> Option<Value> {
    match value {
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.attrs.get("value").cloned(),
            _ => None,
        },
        Value::Exception(exception) => exception.attrs.borrow().get("value").cloned(),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(in crate::vm::vm_extensions) enum CpythonUnicodeErrorFlavor {
    Encode,
    Decode,
    Translate,
}

impl CpythonUnicodeErrorFlavor {
    fn expected_name(self) -> &'static str {
        match self {
            Self::Encode => "UnicodeEncodeError",
            Self::Decode => "UnicodeDecodeError",
            Self::Translate => "UnicodeTranslateError",
        }
    }

    fn object_attr_is_bytes(self) -> bool {
        matches!(self, Self::Decode)
    }
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_class_name_for_value(vm: &Vm, value: &Value) -> Option<String> {
    match value {
        Value::Exception(exception) => Some(exception.name.clone()),
        Value::ExceptionType(name) => Some(name.clone()),
        Value::Instance(instance) => vm.exception_class_name_for_instance(instance),
        _ => None,
    }
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_check_type(
    context: &mut ModuleCapiContext,
    value: &Value,
    expected: CpythonUnicodeErrorFlavor,
) -> bool {
    if context.vm.is_null() {
        context.set_error("missing VM context for unicode error API");
        return false;
    }
    // SAFETY: VM pointer is valid for active C-API context lifetime.
    let vm = unsafe { &mut *context.vm };
    let Some(class_name) = cpython_unicode_error_class_name_for_value(vm, value) else {
        let got = vm.value_type_name_for_error(value);
        context.set_error(format!(
            "expecting a {} object, got {}",
            expected.expected_name(),
            got
        ));
        return false;
    };
    if exception_type_is_subclass(&class_name, "UnicodeError") {
        return true;
    }
    context.set_error(format!(
        "expecting a {} object, got {}",
        expected.expected_name(),
        class_name
    ));
    false
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_attr_value(value: &Value, attr: &str) -> Option<Value> {
    match value {
        Value::Exception(exception) => exception.attrs.borrow().get(attr).cloned(),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.attrs.get(attr).cloned(),
            _ => None,
        },
        _ => None,
    }
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_check_string_like_attr(
    context: &mut ModuleCapiContext,
    attr_name: &str,
    value: Value,
) -> Option<Value> {
    match value {
        Value::Str(_) => Some(value),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => {
                if matches!(
                    instance_data.attrs.get(STR_BACKING_STORAGE_ATTR),
                    Some(Value::Str(_))
                ) {
                    Some(Value::Instance(instance.clone()))
                } else {
                    context.set_error(format!(
                        "UnicodeError '{}' attribute must be a string",
                        attr_name
                    ));
                    None
                }
            }
            _ => {
                context.set_error(format!(
                    "UnicodeError '{}' attribute must be a string",
                    attr_name
                ));
                None
            }
        },
        _ => {
            context.set_error(format!(
                "UnicodeError '{}' attribute must be a string",
                attr_name
            ));
            None
        }
    }
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_check_bytes_like_attr(
    context: &mut ModuleCapiContext,
    attr_name: &str,
    value: Value,
) -> Option<Value> {
    match value {
        Value::Bytes(_) => Some(value),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => {
                if matches!(
                    instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR),
                    Some(Value::Bytes(_))
                ) {
                    Some(Value::Instance(instance.clone()))
                } else {
                    context.set_error(format!(
                        "UnicodeError '{}' attribute must be a bytes",
                        attr_name
                    ));
                    None
                }
            }
            _ => {
                context.set_error(format!(
                    "UnicodeError '{}' attribute must be a bytes",
                    attr_name
                ));
                None
            }
        },
        _ => {
            context.set_error(format!(
                "UnicodeError '{}' attribute must be a bytes",
                attr_name
            ));
            None
        }
    }
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_required_attr(
    context: &mut ModuleCapiContext,
    value: &Value,
    attr_name: &str,
    as_bytes: bool,
) -> Option<Value> {
    let Some(attr_value) = cpython_unicode_error_attr_value(value, attr_name) else {
        context.set_error(format!("UnicodeError '{}' attribute is not set", attr_name));
        return None;
    };
    if as_bytes {
        cpython_unicode_error_check_bytes_like_attr(context, attr_name, attr_value)
    } else {
        cpython_unicode_error_check_string_like_attr(context, attr_name, attr_value)
    }
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_index_attr(
    context: &mut ModuleCapiContext,
    value: &Value,
    attr_name: &str,
) -> Option<isize> {
    let Some(raw) = cpython_unicode_error_attr_value(value, attr_name) else {
        context.set_error(format!("UnicodeError '{}' attribute is not set", attr_name));
        return None;
    };
    let number = match value_to_int(raw) {
        Ok(number) => number,
        Err(_) => {
            context.set_error(format!(
                "UnicodeError '{}' attribute must be an integer",
                attr_name
            ));
            return None;
        }
    };
    Some(number as isize)
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_string_length(value: &Value) -> Option<usize> {
    match value {
        Value::Str(text) => Some(text.chars().count()),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get(STR_BACKING_STORAGE_ATTR) {
                    Some(Value::Str(text)) => Some(text.chars().count()),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_bytes_length(value: &Value) -> Option<usize> {
    match value {
        Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
            Object::Bytes(values) => Some(values.len()),
            _ => None,
        },
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR) {
                    Some(Value::Bytes(bytes_obj)) => match &*bytes_obj.kind() {
                        Object::Bytes(values) => Some(values.len()),
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_adjust_start(start: isize, objlen: isize) -> isize {
    if objlen <= 0 {
        return 0;
    }
    if start < 0 {
        return 0;
    }
    if start >= objlen {
        return objlen - 1;
    }
    start
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_adjust_end(end: isize, objlen: isize) -> isize {
    if objlen <= 0 {
        return 0;
    }
    if end < 1 {
        return 1;
    }
    if end > objlen {
        return objlen;
    }
    end
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_set_attr(
    target: &mut Value,
    key: &str,
    value: Value,
) -> Result<(), String> {
    match target {
        Value::Exception(exception_obj) => {
            exception_obj
                .attrs
                .borrow_mut()
                .insert(key.to_string(), value);
            Ok(())
        }
        Value::Instance(instance) => match &mut *instance.kind_mut() {
            Object::Instance(instance_data) => {
                instance_data.attrs.insert(key.to_string(), value);
                Ok(())
            }
            _ => Err("encountered invalid unicode error instance".to_string()),
        },
        _ => Err("expected unicode error object".to_string()),
    }
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_get_encoding_common(
    self_obj: *mut c_void,
    expected: CpythonUnicodeErrorFlavor,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(self_obj) else {
            context.set_error("received unknown unicode error pointer");
            return std::ptr::null_mut();
        };
        if !cpython_unicode_error_check_type(context, &value, expected) {
            return std::ptr::null_mut();
        }
        let Some(encoding) =
            cpython_unicode_error_required_attr(context, &value, "encoding", false)
        else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(encoding)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_get_object_common(
    self_obj: *mut c_void,
    expected: CpythonUnicodeErrorFlavor,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(self_obj) else {
            context.set_error("received unknown unicode error pointer");
            return std::ptr::null_mut();
        };
        if !cpython_unicode_error_check_type(context, &value, expected) {
            return std::ptr::null_mut();
        }
        let as_bytes = expected.object_attr_is_bytes();
        let Some(object_value) =
            cpython_unicode_error_required_attr(context, &value, "object", as_bytes)
        else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(object_value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_get_reason_common(
    self_obj: *mut c_void,
    expected: CpythonUnicodeErrorFlavor,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(self_obj) else {
            context.set_error("received unknown unicode error pointer");
            return std::ptr::null_mut();
        };
        if !cpython_unicode_error_check_type(context, &value, expected) {
            return std::ptr::null_mut();
        }
        let Some(reason) = cpython_unicode_error_required_attr(context, &value, "reason", false)
        else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(reason)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_get_start_common(
    self_obj: *mut c_void,
    out_start: *mut isize,
    expected: CpythonUnicodeErrorFlavor,
) -> c_int {
    with_active_cpython_context_mut(|context| {
        if out_start.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        }
        let Some(value) = context.cpython_value_from_ptr(self_obj) else {
            context.set_error("received unknown unicode error pointer");
            return -1;
        };
        if !cpython_unicode_error_check_type(context, &value, expected) {
            return -1;
        }
        let as_bytes = expected.object_attr_is_bytes();
        let Some(object_value) =
            cpython_unicode_error_required_attr(context, &value, "object", as_bytes)
        else {
            return -1;
        };
        let objlen = if as_bytes {
            cpython_unicode_error_bytes_length(&object_value)
        } else {
            cpython_unicode_error_string_length(&object_value)
        };
        let Some(objlen) = objlen else {
            context.set_error(format!(
                "UnicodeError '{}' attribute must be a {}",
                "object",
                if as_bytes { "bytes" } else { "string" }
            ));
            return -1;
        };
        let Some(raw_start) = cpython_unicode_error_index_attr(context, &value, "start") else {
            return -1;
        };
        let adjusted = cpython_unicode_error_adjust_start(raw_start, objlen as isize);
        // SAFETY: caller provided writable output pointer.
        unsafe { *out_start = adjusted };
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_get_end_common(
    self_obj: *mut c_void,
    out_end: *mut isize,
    expected: CpythonUnicodeErrorFlavor,
) -> c_int {
    with_active_cpython_context_mut(|context| {
        if out_end.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        }
        let Some(value) = context.cpython_value_from_ptr(self_obj) else {
            context.set_error("received unknown unicode error pointer");
            return -1;
        };
        if !cpython_unicode_error_check_type(context, &value, expected) {
            return -1;
        }
        let as_bytes = expected.object_attr_is_bytes();
        let Some(object_value) =
            cpython_unicode_error_required_attr(context, &value, "object", as_bytes)
        else {
            return -1;
        };
        let objlen = if as_bytes {
            cpython_unicode_error_bytes_length(&object_value)
        } else {
            cpython_unicode_error_string_length(&object_value)
        };
        let Some(objlen) = objlen else {
            context.set_error(format!(
                "UnicodeError '{}' attribute must be a {}",
                "object",
                if as_bytes { "bytes" } else { "string" }
            ));
            return -1;
        };
        let Some(raw_end) = cpython_unicode_error_index_attr(context, &value, "end") else {
            return -1;
        };
        let adjusted = cpython_unicode_error_adjust_end(raw_end, objlen as isize);
        // SAFETY: caller provided writable output pointer.
        unsafe { *out_end = adjusted };
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_set_index_common(
    self_obj: *mut c_void,
    field_name: &str,
    index: isize,
    expected: CpythonUnicodeErrorFlavor,
) -> c_int {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(self_obj) else {
            context.set_error("received unknown unicode error pointer");
            return -1;
        };
        let Some(slot_value) = context.objects.get(&handle).map(|slot| slot.value.clone()) else {
            context.set_error("unicode error handle is not available");
            return -1;
        };
        if !cpython_unicode_error_check_type(context, &slot_value, expected) {
            return -1;
        }
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("unicode error handle is not available");
            return -1;
        };
        if let Err(message) =
            cpython_unicode_error_set_attr(&mut slot.value, field_name, Value::Int(index as i64))
        {
            context.set_error(message);
            return -1;
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

pub(in crate::vm::vm_extensions) fn cpython_unicode_error_set_reason_common(
    self_obj: *mut c_void,
    reason: *const c_char,
    expected: CpythonUnicodeErrorFlavor,
) -> c_int {
    let reason_text = match unsafe { c_name_to_string(reason) } {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(format!("invalid reason: {err}"));
            return -1;
        }
    };
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(self_obj) else {
            context.set_error("received unknown unicode error pointer");
            return -1;
        };
        let Some(slot_value) = context.objects.get(&handle).map(|slot| slot.value.clone()) else {
            context.set_error("unicode error handle is not available");
            return -1;
        };
        if !cpython_unicode_error_check_type(context, &slot_value, expected) {
            return -1;
        }
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("unicode error handle is not available");
            return -1;
        };
        if let Err(message) = cpython_unicode_error_set_attr(
            &mut slot.value,
            "reason",
            Value::Str(reason_text.clone()),
        ) {
            context.set_error(message);
            return -1;
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}
