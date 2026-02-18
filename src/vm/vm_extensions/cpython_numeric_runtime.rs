use crate::runtime::{Heap, RuntimeError, Value};

use super::{
    cpython_new_ptr_for_value, cpython_set_error, cpython_value_from_ptr,
    with_active_cpython_context_mut,
};

pub(in crate::vm::vm_extensions) fn cpython_unary_numeric_op(
    object: *mut core::ffi::c_void,
    op: impl FnOnce(Value) -> Result<Value, RuntimeError>,
) -> *mut core::ffi::c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match op(value) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err.message);
            std::ptr::null_mut()
        }
    }
}

pub(in crate::vm::vm_extensions) fn cpython_binary_numeric_op(
    left: *mut core::ffi::c_void,
    right: *mut core::ffi::c_void,
    op: impl FnOnce(Value, Value) -> Result<Value, RuntimeError>,
) -> *mut core::ffi::c_void {
    let left = match cpython_value_from_ptr(left) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let right = match cpython_value_from_ptr(right) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match op(left, right) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err.message);
            std::ptr::null_mut()
        }
    }
}

pub(in crate::vm::vm_extensions) fn cpython_binary_numeric_op_with_heap(
    left: *mut core::ffi::c_void,
    right: *mut core::ffi::c_void,
    op: impl FnOnce(Value, Value, &Heap) -> Result<Value, RuntimeError>,
) -> *mut core::ffi::c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("missing VM context for numeric operation");
            return std::ptr::null_mut();
        }
        let Some(left) = context.cpython_value_from_ptr(left) else {
            context.set_error("unknown left operand pointer");
            return std::ptr::null_mut();
        };
        let Some(right) = context.cpython_value_from_ptr(right) else {
            context.set_error("unknown right operand pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match op(left, right, &vm.heap) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
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
