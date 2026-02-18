use std::collections::HashMap;
use std::ffi::c_void;

use crate::runtime::Value;

use super::{
    PyMem_Free, PyMem_Malloc, PyMem_Realloc, cpython_set_error, cpython_value_from_ptr,
    with_active_cpython_context_mut,
};

unsafe extern "C" {
    fn calloc(nmemb: usize, size: usize) -> *mut c_void;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Malloc(size: usize) -> *mut c_void {
    unsafe { PyMem_Malloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Calloc(count: usize, size: usize) -> *mut c_void {
    if count.checked_mul(size).is_none() {
        cpython_set_error("PyObject_Calloc size overflow");
        return std::ptr::null_mut();
    }
    // SAFETY: libc calloc contract; returns null on failure.
    unsafe { calloc(count, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe { PyMem_Realloc(ptr, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Free(ptr: *mut c_void) {
    unsafe { PyMem_Free(ptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_Track(_object: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_UnTrack(_object: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_IsTracked(object: *mut c_void) -> i32 {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(_) => return 0,
    };
    let tracked = matches!(
        value,
        Value::List(_)
            | Value::Tuple(_)
            | Value::Dict(_)
            | Value::Set(_)
            | Value::FrozenSet(_)
            | Value::Instance(_)
            | Value::Class(_)
            | Value::Module(_)
            | Value::MemoryView(_)
    );
    i32::from(tracked)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_IsFinalized(_object: *mut c_void) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_Del(object: *mut c_void) {
    unsafe { PyObject_Free(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGC_Collect() -> isize {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyGC_Collect missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_gc_collect(Vec::new(), HashMap::new()) {
            Ok(Value::Int(value)) => value as isize,
            Ok(Value::BigInt(value)) => {
                if let Some(compact) = value.to_i64() {
                    compact.clamp(isize::MIN as i64, isize::MAX as i64) as isize
                } else if value.is_negative() {
                    isize::MIN
                } else {
                    isize::MAX
                }
            }
            Ok(_) => 0,
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
pub unsafe extern "C" fn PyGC_Enable() -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyGC_Enable missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let was_enabled = vm.gc_enabled;
        vm.gc_enabled = true;
        i32::from(was_enabled)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGC_Disable() -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyGC_Disable missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let was_enabled = vm.gc_enabled;
        vm.gc_enabled = false;
        i32::from(was_enabled)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGC_IsEnabled() -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyGC_IsEnabled missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        i32::from(vm.gc_enabled)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}
