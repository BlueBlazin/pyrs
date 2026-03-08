use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};

use crate::runtime::{BuiltinFunction, IteratorKind, IteratorObject, Object, RuntimeError, Value};
use crate::vm::{InternalCallOutcome, add_values, is_truthy, mul_values};

use super::{
    CpythonMappingMethods, CpythonObjectHead, CpythonSequenceMethods, CpythonTypeObject,
    ModuleCapiContext, Py_DecRef, Py_XDecRef, Py_XIncRef, PyCallable_Check, PyDict_Items,
    PyDict_Keys, PyDict_Values, PyErr_BadInternalCall, PyErr_Clear, PyExc_TypeError,
    PyLong_FromSsize_t, PyObject_CallNoArgs, PyObject_DelItem, PyObject_GetAttrString,
    PyObject_GetItem, PyObject_HasAttrString, PyObject_HasAttrStringWithError, PyObject_SetItem,
    PyObject_Size, PySlice_New, PyUnicode_FromString, c_name_to_string,
    cpython_active_exception_is, cpython_binary_numeric_op_with_heap,
    cpython_clear_active_exception, cpython_set_error, cpython_set_typed_error,
    cpython_value_from_ptr, cpython_value_type_name_from_ptr, with_active_cpython_context_mut,
};

fn set_context_error_from_runtime_error(context: &mut ModuleCapiContext, err: RuntimeError) {
    context.set_error_from_runtime_error(err);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Check(object: *mut c_void) -> i32 {
    if object.is_null() {
        return 0;
    }
    if let Ok(result) = with_active_cpython_context_mut(|context| {
        if let Some(value) = context.cpython_value_from_ptr_or_proxy(object) {
            if matches!(
                value,
                Value::Tuple(_)
                    | Value::List(_)
                    | Value::Str(_)
                    | Value::Bytes(_)
                    | Value::ByteArray(_)
                    | Value::MemoryView(_)
            ) {
                return 1;
            }
            if matches!(value, Value::Dict(_)) {
                return 0;
            }
        }
        const MIN_VALID_PTR: usize = super::MIN_VALID_PTR_THRESHOLD;
        if (object as usize) < MIN_VALID_PTR
            || (object as usize) % std::mem::align_of::<CpythonObjectHead>() != 0
        {
            return 0;
        }
        // SAFETY: best-effort slot lookup for CPython-compatible object pointers.
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
            return 0;
        }
        // SAFETY: `type_ptr` is validated non-null/aligned.
        let as_sequence = unsafe {
            (*type_ptr)
                .tp_as_sequence
                .cast::<CpythonSequenceMethods>()
                .as_ref()
        };
        if let Some(methods) = as_sequence
            && (!methods.sq_item.is_null() || !methods.sq_length.is_null())
        {
            return 1;
        }
        0
    }) {
        if result != 0 {
            return result;
        }
    }
    let getitem = unsafe { PyObject_HasAttrString(object, c"__getitem__".as_ptr()) };
    if getitem <= 0 {
        return 0;
    }
    let len = unsafe { PyObject_HasAttrString(object, c"__len__".as_ptr()) };
    if len <= 0 { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Size(object: *mut c_void) -> isize {
    unsafe { PyObject_Size(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Length(object: *mut c_void) -> isize {
    unsafe { PySequence_Size(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_GetItem(object: *mut c_void, index: isize) -> *mut c_void {
    let index = unsafe { PyLong_FromSsize_t(index) };
    if index.is_null() {
        return std::ptr::null_mut();
    }
    let value = unsafe { PyObject_GetItem(object, index) };
    unsafe { Py_DecRef(index) };
    value
}

unsafe fn cpython_sequence_build_slice(low: isize, high: isize) -> *mut c_void {
    let start = unsafe { PyLong_FromSsize_t(low) };
    if start.is_null() {
        return std::ptr::null_mut();
    }
    let stop = unsafe { PyLong_FromSsize_t(high) };
    if stop.is_null() {
        unsafe { Py_DecRef(start) };
        return std::ptr::null_mut();
    }
    let slice = unsafe { PySlice_New(start, stop, std::ptr::null_mut()) };
    unsafe {
        Py_DecRef(start);
        Py_DecRef(stop);
    }
    slice
}

pub(in crate::vm::vm_extensions) fn cpython_slice_bounds_step_one(
    len: usize,
    lower: Option<i64>,
    upper: Option<i64>,
) -> (usize, usize) {
    let len_isize = len as isize;
    let mut start = lower.unwrap_or(0) as isize;
    if start < 0 {
        start += len_isize;
    }
    if start < 0 {
        start = 0;
    } else if start > len_isize {
        start = len_isize;
    }

    let mut stop = upper.unwrap_or(len as i64) as isize;
    if stop < 0 {
        stop += len_isize;
    }
    if stop < 0 {
        stop = 0;
    } else if stop > len_isize {
        stop = len_isize;
    }

    let start = start as usize;
    let stop = (if stop < start as isize {
        start as isize
    } else {
        stop
    }) as usize;
    (start, stop)
}

pub(in crate::vm::vm_extensions) fn cpython_slice_indices_for_len(
    len: usize,
    lower: Option<i64>,
    upper: Option<i64>,
    step: Option<i64>,
) -> Result<Vec<usize>, String> {
    let len_isize = len as isize;
    let step = step.unwrap_or(1);
    if step == 0 {
        return Err("slice step cannot be zero".to_string());
    }
    let step = step as isize;

    let (start, stop) = if step > 0 {
        let mut start = lower.unwrap_or(0) as isize;
        if start < 0 {
            start += len_isize;
        }
        if start < 0 {
            start = 0;
        } else if start > len_isize {
            start = len_isize;
        }

        let mut stop = upper.unwrap_or(len as i64) as isize;
        if stop < 0 {
            stop += len_isize;
        }
        if stop < 0 {
            stop = 0;
        } else if stop > len_isize {
            stop = len_isize;
        }
        (start, stop)
    } else {
        let mut start = lower.unwrap_or(len as i64 - 1) as isize;
        if start < 0 {
            start += len_isize;
        }
        if start < -1 {
            start = -1;
        } else if start >= len_isize {
            start = len_isize - 1;
        }

        let mut stop = upper.unwrap_or(-1) as isize;
        if upper.is_some() && stop < 0 {
            stop += len_isize;
        }
        if stop < -1 {
            stop = -1;
        } else if stop >= len_isize {
            stop = len_isize - 1;
        }
        (start, stop)
    };

    let mut out = Vec::new();
    if step > 0 {
        let mut idx = start;
        while idx < stop {
            out.push(idx as usize);
            idx += step;
        }
    } else {
        let mut idx = start;
        while idx > stop {
            out.push(idx as usize);
            idx += step;
        }
    }
    Ok(out)
}

unsafe fn cpython_sequence_del_item_with_key(object: *mut c_void, key: *mut c_void) -> i32 {
    unsafe { PyObject_DelItem(object, key) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_GetSlice(
    object: *mut c_void,
    low: isize,
    high: isize,
) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let slice = unsafe { cpython_sequence_build_slice(low, high) };
    if slice.is_null() {
        return std::ptr::null_mut();
    }
    let result = unsafe { PyObject_GetItem(object, slice) };
    unsafe { Py_DecRef(slice) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_SetItem(
    object: *mut c_void,
    index: isize,
    value: *mut c_void,
) -> i32 {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if value.is_null() {
        return unsafe { PySequence_DelItem(object, index) };
    }
    let key = unsafe { PyLong_FromSsize_t(index) };
    if key.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_SetItem(object, key, value) };
    unsafe { Py_DecRef(key) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_DelItem(object: *mut c_void, index: isize) -> i32 {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let key = unsafe { PyLong_FromSsize_t(index) };
    if key.is_null() {
        return -1;
    }
    let status = unsafe { cpython_sequence_del_item_with_key(object, key) };
    unsafe { Py_DecRef(key) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_SetSlice(
    object: *mut c_void,
    low: isize,
    high: isize,
    value: *mut c_void,
) -> i32 {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if value.is_null() {
        return unsafe { PySequence_DelSlice(object, low, high) };
    }
    let slice = unsafe { cpython_sequence_build_slice(low, high) };
    if slice.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_SetItem(object, slice, value) };
    unsafe { Py_DecRef(slice) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_DelSlice(object: *mut c_void, low: isize, high: isize) -> i32 {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let slice = unsafe { cpython_sequence_build_slice(low, high) };
    if slice.is_null() {
        return -1;
    }
    let status = unsafe { cpython_sequence_del_item_with_key(object, slice) };
    unsafe { Py_DecRef(slice) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Contains(container: *mut c_void, value: *mut c_void) -> i32 {
    let container = match cpython_value_from_ptr(container) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let value = match cpython_value_from_ptr(value) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySequence_Contains missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_operator_contains(vec![container, value], HashMap::new()) {
            Ok(Value::Bool(flag)) => i32::from(flag),
            Ok(other) => {
                context.set_error(format!(
                    "PySequence_Contains expected bool result, got {other:?}"
                ));
                -1
            }
            Err(err) => {
                context.set_error(err.message);
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
pub unsafe extern "C" fn PySequence_In(container: *mut c_void, value: *mut c_void) -> i32 {
    unsafe { PySequence_Contains(container, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Tuple(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PySequence_Tuple received unknown object pointer");
            return std::ptr::null_mut();
        };
        if context.vm.is_null() {
            context.set_error("PySequence_Tuple missing VM context");
            return std::ptr::null_mut();
        }
        let vm = unsafe { &mut *context.vm };
        match vm.call_internal(
            Value::Builtin(BuiltinFunction::Tuple),
            vec![value],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                set_context_error_from_runtime_error(
                    context,
                    vm.runtime_error_from_active_exception("tuple() failed"),
                );
                std::ptr::null_mut()
            }
            Err(err) => {
                set_context_error_from_runtime_error(context, err);
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
pub unsafe extern "C" fn PySequence_List(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PySequence_List received unknown object pointer");
            return std::ptr::null_mut();
        };
        if context.vm.is_null() {
            context.set_error("PySequence_List missing VM context");
            return std::ptr::null_mut();
        }
        let vm = unsafe { &mut *context.vm };
        match vm.call_internal(
            Value::Builtin(BuiltinFunction::List),
            vec![value],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                set_context_error_from_runtime_error(
                    context,
                    vm.runtime_error_from_active_exception("list() failed"),
                );
                std::ptr::null_mut()
            }
            Err(err) => {
                set_context_error_from_runtime_error(context, err);
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
pub unsafe extern "C" fn PySequence_Fast(object: *mut c_void, msg: *const c_char) -> *mut c_void {
    match cpython_value_from_ptr(object) {
        Ok(Value::Tuple(_) | Value::List(_)) => {
            unsafe { Py_XIncRef(object) };
            object
        }
        Ok(value) => with_active_cpython_context_mut(|context| {
            if context.vm.is_null() {
                context.set_error("PySequence_Fast missing VM context");
                return std::ptr::null_mut();
            }
            let vm = unsafe { &mut *context.vm };
            match vm.call_internal(
                Value::Builtin(BuiltinFunction::Tuple),
                vec![value],
                HashMap::new(),
            ) {
                Ok(InternalCallOutcome::Value(value)) => context.alloc_cpython_ptr_for_value(value),
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    let err = vm.runtime_error_from_active_exception("tuple() failed");
                    if err.exception_name() == Some("TypeError")
                        && !msg.is_null()
                        && let Ok(text) = unsafe { c_name_to_string(msg) }
                    {
                        cpython_set_typed_error(unsafe { PyExc_TypeError }, text);
                    } else {
                        set_context_error_from_runtime_error(context, err);
                    }
                    std::ptr::null_mut()
                }
                Err(err) => {
                    if err.exception_name() == Some("TypeError")
                        && !msg.is_null()
                        && let Ok(text) = unsafe { c_name_to_string(msg) }
                    {
                        cpython_set_typed_error(unsafe { PyExc_TypeError }, text);
                    } else {
                        set_context_error_from_runtime_error(context, err);
                    }
                    std::ptr::null_mut()
                }
            }
        })
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            std::ptr::null_mut()
        }),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Concat(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op_with_heap(left, right, add_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_InPlaceConcat(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    unsafe { PySequence_Concat(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Repeat(object: *mut c_void, count: isize) -> *mut c_void {
    let count = unsafe { PyLong_FromSsize_t(count) };
    if count.is_null() {
        return std::ptr::null_mut();
    }
    let result = cpython_binary_numeric_op_with_heap(object, count, mul_values);
    unsafe { Py_DecRef(count) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_InPlaceRepeat(
    object: *mut c_void,
    count: isize,
) -> *mut c_void {
    unsafe { PySequence_Repeat(object, count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Count(object: *mut c_void, value: *mut c_void) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(sequence_handle) = context.cpython_handle_from_ptr(object) else {
            context.set_error("PySequence_Count received unknown sequence pointer");
            return -1;
        };
        let Some(value_handle) = context.cpython_handle_from_ptr(value) else {
            context.set_error("PySequence_Count received unknown value pointer");
            return -1;
        };
        let Some(needle) = context.object_value(value_handle) else {
            context.set_error("PySequence_Count value handle is not available");
            return -1;
        };
        let iterator_handle = match context.object_get_iter(sequence_handle) {
            Ok(handle) => handle,
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        };
        let mut count: isize = 0;
        loop {
            let next_handle = match context.object_iter_next(iterator_handle) {
                Ok(next) => next,
                Err(err) => {
                    context.set_error(err);
                    let _ = context.decref(iterator_handle);
                    return -1;
                }
            };
            let Some(item_handle) = next_handle else {
                break;
            };
            let Some(item_value) = context.object_value(item_handle) else {
                context.set_error("PySequence_Count iterator item handle is not available");
                let _ = context.decref(item_handle);
                let _ = context.decref(iterator_handle);
                return -1;
            };
            let is_match = {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *context.vm };
                match vm.compare_eq_runtime(item_value, needle.clone()) {
                    Ok(Value::Bool(flag)) => flag,
                    Ok(other) => is_truthy(&other),
                    Err(err) => {
                        context.set_error(err.message);
                        let _ = context.decref(item_handle);
                        let _ = context.decref(iterator_handle);
                        return -1;
                    }
                }
            };
            let _ = context.decref(item_handle);
            if is_match {
                count = match count.checked_add(1) {
                    Some(next) => next,
                    None => {
                        context.set_error("count exceeds C integer size");
                        let _ = context.decref(iterator_handle);
                        return -1;
                    }
                };
            }
        }
        let _ = context.decref(iterator_handle);
        count
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Index(object: *mut c_void, value: *mut c_void) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(sequence_handle) = context.cpython_handle_from_ptr(object) else {
            context.set_error("PySequence_Index received unknown sequence pointer");
            return -1;
        };
        let Some(value_handle) = context.cpython_handle_from_ptr(value) else {
            context.set_error("PySequence_Index received unknown value pointer");
            return -1;
        };
        let Some(needle) = context.object_value(value_handle) else {
            context.set_error("PySequence_Index value handle is not available");
            return -1;
        };
        let iterator_handle = match context.object_get_iter(sequence_handle) {
            Ok(handle) => handle,
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        };
        let mut index: isize = 0;
        loop {
            let next_handle = match context.object_iter_next(iterator_handle) {
                Ok(next) => next,
                Err(err) => {
                    context.set_error(err);
                    let _ = context.decref(iterator_handle);
                    return -1;
                }
            };
            let Some(item_handle) = next_handle else {
                break;
            };
            let Some(item_value) = context.object_value(item_handle) else {
                context.set_error("PySequence_Index iterator item handle is not available");
                let _ = context.decref(item_handle);
                let _ = context.decref(iterator_handle);
                return -1;
            };
            let is_match = {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *context.vm };
                match vm.compare_eq_runtime(item_value, needle.clone()) {
                    Ok(Value::Bool(flag)) => flag,
                    Ok(other) => is_truthy(&other),
                    Err(err) => {
                        context.set_error(err.message);
                        let _ = context.decref(item_handle);
                        let _ = context.decref(iterator_handle);
                        return -1;
                    }
                }
            };
            let _ = context.decref(item_handle);
            if is_match {
                let _ = context.decref(iterator_handle);
                return index;
            }
            index = match index.checked_add(1) {
                Some(next) => next,
                None => {
                    context.set_error("index exceeds C integer size");
                    let _ = context.decref(iterator_handle);
                    return -1;
                }
            };
        }
        let _ = context.decref(iterator_handle);
        context.set_error("sequence.index(x): x not in sequence");
        -1
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_GetItemString(
    mapping: *mut c_void,
    key: *const c_char,
) -> *mut c_void {
    if mapping.is_null() || key.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let key = unsafe { PyUnicode_FromString(key) };
    if key.is_null() {
        return std::ptr::null_mut();
    }
    let result = unsafe { PyObject_GetItem(mapping, key) };
    unsafe { Py_DecRef(key) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Check(object: *mut c_void) -> i32 {
    let trace_mapping_check = super::super::env_var_present_cached("PYRS_TRACE_PYMAPPING_CHECK");
    if object.is_null() {
        return 0;
    }
    if let Ok(result) = with_active_cpython_context_mut(|context| {
        if let Some(value) = context.cpython_value_from_ptr_or_proxy(object) {
            if matches!(
                value,
                Value::Dict(_)
                    | Value::List(_)
                    | Value::Tuple(_)
                    | Value::Str(_)
                    | Value::Bytes(_)
                    | Value::ByteArray(_)
                    | Value::MemoryView(_)
            ) {
                return 1;
            }
        }
        const MIN_VALID_PTR: usize = super::MIN_VALID_PTR_THRESHOLD;
        if (object as usize) < MIN_VALID_PTR
            || (object as usize) % std::mem::align_of::<CpythonObjectHead>() != 0
        {
            return 0;
        }
        // SAFETY: best-effort slot lookup for CPython-compatible object pointers.
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
            return 0;
        }
        // SAFETY: `type_ptr` is validated non-null/aligned.
        let as_mapping = unsafe {
            (*type_ptr)
                .tp_as_mapping
                .cast::<CpythonMappingMethods>()
                .as_ref()
        };
        if as_mapping.is_some_and(|methods| !methods.mp_subscript.is_null()) {
            return 1;
        }
        // SAFETY: `type_ptr` is validated non-null/aligned.
        let as_sequence = unsafe {
            (*type_ptr)
                .tp_as_sequence
                .cast::<CpythonSequenceMethods>()
                .as_ref()
        };
        if as_sequence.is_some_and(|methods| !methods.sq_item.is_null()) {
            return 1;
        }
        0
    }) {
        if result != 0 {
            return 1;
        }
    }
    match cpython_value_from_ptr(object) {
        Ok(_) => {
            let status =
                unsafe { PyObject_HasAttrStringWithError(object, c"__getitem__".as_ptr()) };
            if status < 0 {
                unsafe { PyErr_Clear() };
                if trace_mapping_check {
                    eprintln!(
                        "[cpy-mapping-check] object={:p} status=0 reason=hasattr-error",
                        object
                    );
                }
                0
            } else {
                if trace_mapping_check {
                    let _ = with_active_cpython_context_mut(|context| {
                        let object_tag = context
                            .cpython_value_from_ptr_or_proxy(object)
                            .map(|value| super::cpython_value_debug_tag(&value))
                            .unwrap_or_else(|| "<unknown>".to_string());
                        let type_name = super::cpython_safe_object_type_name(object)
                            .unwrap_or_else(|| "<unknown>".to_string());
                        eprintln!(
                            "[cpy-mapping-check] object={:p} status={} object_tag={} type_name={} reason=hasattr",
                            object, status, object_tag, type_name
                        );
                    });
                }
                status
            }
        }
        Err(_) => {
            if trace_mapping_check {
                let _ = with_active_cpython_context_mut(|context| {
                    let object_tag = context
                        .cpython_value_from_ptr_or_proxy(object)
                        .map(|value| super::cpython_value_debug_tag(&value))
                        .unwrap_or_else(|| "<unknown>".to_string());
                    let type_name = super::cpython_safe_object_type_name(object)
                        .unwrap_or_else(|| "<unknown>".to_string());
                    eprintln!(
                        "[cpy-mapping-check] object={:p} status=0 object_tag={} type_name={} reason=unknown-ptr",
                        object, object_tag, type_name
                    );
                });
            }
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Size(object: *mut c_void) -> isize {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if unsafe { PyMapping_Check(object) } == 0 {
        let type_name = cpython_value_type_name_from_ptr(object);
        let has_len = unsafe { PyObject_HasAttrStringWithError(object, c"__len__".as_ptr()) };
        if has_len < 0 {
            return -1;
        }
        if has_len == 1 {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                format!("{type_name} is not a mapping"),
            );
        } else {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                format!("object of type '{type_name}' has no len()"),
            );
        }
        return -1;
    }
    unsafe { PyObject_Size(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Length(object: *mut c_void) -> isize {
    unsafe { PyMapping_Size(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_GetOptionalItem(
    object: *mut c_void,
    key: *mut c_void,
    result: *mut *mut c_void,
) -> i32 {
    if result.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe { *result = std::ptr::null_mut() };
    if object.is_null() || key.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyMapping_GetOptionalItem missing VM context");
            return -1;
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyMapping_GetOptionalItem received unknown object pointer");
            return -1;
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyMapping_GetOptionalItem received unknown key pointer");
            return -1;
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.getitem_value(object_value, key_value) {
            Ok(value) => {
                let ptr = context.alloc_cpython_ptr_for_value(value);
                if ptr.is_null() {
                    -1
                } else {
                    unsafe { *result = ptr };
                    1
                }
            }
            Err(err) => {
                if cpython_active_exception_is(vm, "KeyError")
                    || err.message.contains("key not found")
                    || err.message.contains("KeyError")
                {
                    cpython_clear_active_exception(vm);
                    0
                } else {
                    context.set_error(err.message);
                    -1
                }
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_GetOptionalItemString(
    object: *mut c_void,
    key: *const c_char,
    result: *mut *mut c_void,
) -> i32 {
    if key.is_null() || result.is_null() {
        if !result.is_null() {
            unsafe { *result = std::ptr::null_mut() };
        }
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        unsafe { *result = std::ptr::null_mut() };
        return -1;
    }
    let status = unsafe { PyMapping_GetOptionalItem(object, key_obj, result) };
    unsafe { Py_DecRef(key_obj) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_SetItemString(
    object: *mut c_void,
    key: *const c_char,
    value: *mut c_void,
) -> i32 {
    if key.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_SetItem(object, key_obj, value) };
    unsafe { Py_DecRef(key_obj) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKeyWithError(object: *mut c_void, key: *mut c_void) -> i32 {
    let mut value: *mut c_void = std::ptr::null_mut();
    let status = unsafe { PyMapping_GetOptionalItem(object, key, &mut value) };
    unsafe { Py_XDecRef(value) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKeyStringWithError(
    object: *mut c_void,
    key: *const c_char,
) -> i32 {
    let mut value: *mut c_void = std::ptr::null_mut();
    let status = unsafe { PyMapping_GetOptionalItemString(object, key, &mut value) };
    unsafe { Py_XDecRef(value) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKey(object: *mut c_void, key: *mut c_void) -> i32 {
    let status = unsafe { PyMapping_HasKeyWithError(object, key) };
    if status < 0 {
        unsafe { PyErr_Clear() };
        0
    } else {
        status
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKeyString(object: *mut c_void, key: *const c_char) -> i32 {
    let status = unsafe { PyMapping_HasKeyStringWithError(object, key) };
    if status < 0 {
        unsafe { PyErr_Clear() };
        0
    } else {
        status
    }
}

fn cpython_mapping_method_output_as_list(object: *mut c_void, method_name: &str) -> *mut c_void {
    let method = match CString::new(method_name) {
        Ok(name) => unsafe { PyObject_GetAttrString(object, name.as_ptr()) },
        Err(err) => {
            cpython_set_error(err.to_string());
            return std::ptr::null_mut();
        }
    };
    if method.is_null() {
        return std::ptr::null_mut();
    }
    let output = unsafe { PyObject_CallNoArgs(method) };
    unsafe { Py_DecRef(method) };
    if output.is_null() {
        return std::ptr::null_mut();
    }
    let list = match cpython_value_from_ptr(output) {
        Ok(Value::List(_)) => output,
        Ok(_) => {
            let list = unsafe { PySequence_List(output) };
            unsafe { Py_DecRef(output) };
            list
        }
        Err(err) => {
            cpython_set_error(err);
            unsafe { Py_DecRef(output) };
            std::ptr::null_mut()
        }
    };
    list
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Keys(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if let Ok(Value::Dict(_)) = cpython_value_from_ptr(object) {
        return unsafe { PyDict_Keys(object) };
    }
    cpython_mapping_method_output_as_list(object, "keys")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Items(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if let Ok(Value::Dict(_)) = cpython_value_from_ptr(object) {
        return unsafe { PyDict_Items(object) };
    }
    cpython_mapping_method_output_as_list(object, "items")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Values(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if let Ok(Value::Dict(_)) = cpython_value_from_ptr(object) {
        return unsafe { PyDict_Values(object) };
    }
    cpython_mapping_method_output_as_list(object, "values")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySeqIter_New(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if object.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            context.set_error("PySeqIter_New missing VM context");
            return std::ptr::null_mut();
        }
        if unsafe { PySequence_Check(object) } == 0 {
            context.set_error("PySeqIter_New() argument must be a sequence");
            return std::ptr::null_mut();
        }
        let Some(target_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PySeqIter_New received unknown object pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let iterator = vm.heap.alloc(Object::Iterator(IteratorObject {
            kind: IteratorKind::CpythonSequence {
                target: target_value,
            },
            index: 0,
        }));
        context.alloc_cpython_ptr_for_value(Value::Iterator(iterator))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCallIter_New(
    callable: *mut c_void,
    sentinel: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if callable.is_null() || sentinel.is_null() {
            context.set_error("PyCallIter_New received null callable/sentinel");
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            context.set_error("PyCallIter_New missing VM context");
            return std::ptr::null_mut();
        }
        let Some(callable_value) = context.cpython_value_from_ptr_or_proxy(callable) else {
            context.set_error("PyCallIter_New received unknown callable pointer");
            return std::ptr::null_mut();
        };
        let Some(sentinel_value) = context.cpython_value_from_ptr_or_proxy(sentinel) else {
            context.set_error("PyCallIter_New received unknown sentinel pointer");
            return std::ptr::null_mut();
        };
        let callable_check = unsafe { PyCallable_Check(callable) };
        if callable_check < 0 {
            return std::ptr::null_mut();
        }
        if callable_check == 0 {
            context.set_error("TypeError: iter(v, w): v must be callable");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let iterator = vm.heap.alloc(Object::Iterator(IteratorObject {
            kind: IteratorKind::CallIter {
                callable: callable_value,
                sentinel: sentinel_value,
            },
            index: 0,
        }));
        context.alloc_cpython_ptr_for_value(Value::Iterator(iterator))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}
