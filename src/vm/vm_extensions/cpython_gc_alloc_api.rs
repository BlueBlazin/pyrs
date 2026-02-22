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
pub unsafe extern "C" fn PyObject_GC_Track(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    let _ = with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            return;
        };
        if !cpython_value_supports_gc_tracking(&value) || context.vm.is_null() {
            return;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let _ = vm.capi_registry_set_gc_tracked_override(object as usize, true);
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_UnTrack(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    let _ = with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            return;
        };
        if !cpython_value_supports_gc_tracking(&value) || context.vm.is_null() {
            return;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let _ = vm.capi_registry_set_gc_tracked_override(object as usize, false);
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_IsTracked(object: *mut c_void) -> i32 {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(_) => return 0,
    };
    let mut tracked = cpython_value_supports_gc_tracking(&value);
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        if let Some(override_value) = vm.capi_registry_gc_tracked_override(object as usize) {
            tracked = override_value;
        }
    });
    i32::from(tracked)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_IsFinalized(object: *mut c_void) -> i32 {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(_) => return 0,
    };
    if !cpython_value_supports_gc_tracking(&value) {
        return 0;
    }
    let object_id = cpython_identity_object_id(&value);
    let mut finalized = false;
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        finalized = vm.capi_registry_is_gc_finalized(object as usize)
            || object_id.is_some_and(|id| vm.is_object_gc_finalized(id));
    });
    i32::from(finalized)
}

fn cpython_value_supports_gc_tracking(value: &Value) -> bool {
    matches!(
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
            | Value::Function(_)
            | Value::BoundMethod(_)
            | Value::Cell(_)
    )
}

fn cpython_identity_object_id(value: &Value) -> Option<u64> {
    match value {
        Value::List(obj)
        | Value::Tuple(obj)
        | Value::Dict(obj)
        | Value::DictKeys(obj)
        | Value::Set(obj)
        | Value::FrozenSet(obj)
        | Value::Bytes(obj)
        | Value::ByteArray(obj)
        | Value::MemoryView(obj)
        | Value::Iterator(obj)
        | Value::Generator(obj)
        | Value::Module(obj)
        | Value::Class(obj)
        | Value::Instance(obj)
        | Value::Super(obj)
        | Value::Function(obj)
        | Value::BoundMethod(obj)
        | Value::Cell(obj) => Some(obj.id()),
        _ => None,
    }
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
