use std::collections::HashMap;
use std::ffi::c_void;

use crate::runtime::{Object, Value};
use crate::vm::{NativeCallResult, NativeMethodKind};

use super::{cpython_set_error, cpython_value_from_ptr, with_active_cpython_context_mut};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_New(iterable: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySet_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let value = if iterable.is_null() {
            vm.heap.alloc_set(Vec::new())
        } else {
            let source = match context.cpython_value_from_ptr_or_proxy(iterable) {
                Some(value) => value,
                None => {
                    context.set_error("PySet_New received unknown iterable pointer");
                    return std::ptr::null_mut();
                }
            };
            match vm.builtin_set(vec![source], HashMap::new()) {
                Ok(value) => value,
                Err(err) => {
                    context.set_error(err.message);
                    return std::ptr::null_mut();
                }
            }
        };
        context.alloc_cpython_ptr_for_value(value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrozenSet_New(iterable: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyFrozenSet_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let value = if iterable.is_null() {
            vm.heap.alloc_frozenset(Vec::new())
        } else {
            let source = match context.cpython_value_from_ptr_or_proxy(iterable) {
                Some(value) => value,
                None => {
                    context.set_error("PyFrozenSet_New received unknown iterable pointer");
                    return std::ptr::null_mut();
                }
            };
            match vm.builtin_frozenset(vec![source], HashMap::new()) {
                Ok(value) => value,
                Err(err) => {
                    context.set_error(err.message);
                    return std::ptr::null_mut();
                }
            }
        };
        context.alloc_cpython_ptr_for_value(value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Size(anyset: *mut c_void) -> isize {
    match cpython_value_from_ptr(anyset) {
        Ok(Value::Set(set_obj)) => match &*set_obj.kind() {
            Object::Set(values) => values.len() as isize,
            _ => {
                cpython_set_error("PySet_Size encountered invalid set storage");
                -1
            }
        },
        Ok(Value::FrozenSet(set_obj)) => match &*set_obj.kind() {
            Object::FrozenSet(values) => values.len() as isize,
            _ => {
                cpython_set_error("PySet_Size encountered invalid frozenset storage");
                -1
            }
        },
        Ok(_) => {
            cpython_set_error("PySet_Size expected set object");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Contains(anyset: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySet_Contains missing VM context");
            return -1;
        }
        let receiver = match context.cpython_value_from_ptr(anyset) {
            Some(Value::Set(set_obj)) | Some(Value::FrozenSet(set_obj)) => set_obj,
            Some(_) => {
                context.set_error("PySet_Contains expected set object");
                return -1;
            }
            None => {
                context.set_error("PySet_Contains received unknown set pointer");
                return -1;
            }
        };
        let key_value = match context.cpython_value_from_ptr_or_proxy(key) {
            Some(value) => value,
            None => {
                context.set_error("PySet_Contains received unknown key pointer");
                return -1;
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::SetContains,
            receiver,
            vec![key_value],
            HashMap::new(),
        ) {
            Ok(NativeCallResult::Value(Value::Bool(true))) => 1,
            Ok(NativeCallResult::Value(Value::Bool(false))) => 0,
            Ok(_) => {
                context.set_error("PySet_Contains returned non-boolean result");
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
pub unsafe extern "C" fn PySet_Add(set: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySet_Add missing VM context");
            return -1;
        }
        let receiver = match context.cpython_value_from_ptr(set) {
            Some(Value::Set(set_obj)) => set_obj,
            Some(_) => {
                context.set_error("PySet_Add expected set object");
                return -1;
            }
            None => {
                context.set_error("PySet_Add received unknown set pointer");
                return -1;
            }
        };
        let key_value = match context.cpython_value_from_ptr_or_proxy(key) {
            Some(value) => value,
            None => {
                context.set_error("PySet_Add received unknown key pointer");
                return -1;
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::SetAdd,
            receiver,
            vec![key_value],
            HashMap::new(),
        ) {
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
pub unsafe extern "C" fn PySet_Discard(set: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySet_Discard missing VM context");
            return -1;
        }
        let receiver = match context.cpython_value_from_ptr(set) {
            Some(Value::Set(set_obj)) => set_obj,
            Some(_) => {
                context.set_error("PySet_Discard expected set object");
                return -1;
            }
            None => {
                context.set_error("PySet_Discard received unknown set pointer");
                return -1;
            }
        };
        let key_value = match context.cpython_value_from_ptr_or_proxy(key) {
            Some(value) => value,
            None => {
                context.set_error("PySet_Discard received unknown key pointer");
                return -1;
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::SetDiscard,
            receiver,
            vec![key_value],
            HashMap::new(),
        ) {
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
pub unsafe extern "C" fn PySet_Clear(set: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(set) else {
            context.set_error("PySet_Clear received unknown set pointer");
            return -1;
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PySet_Clear set handle is not available");
            return -1;
        };
        let Value::Set(set_obj) = &mut slot.value else {
            context.set_error("PySet_Clear expected set object");
            return -1;
        };
        let Object::Set(values) = &mut *set_obj.kind_mut() else {
            context.set_error("PySet_Clear encountered invalid set storage");
            return -1;
        };
        values.clear();
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Pop(set: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySet_Pop missing VM context");
            return std::ptr::null_mut();
        }
        let receiver = match context.cpython_value_from_ptr(set) {
            Some(Value::Set(set_obj)) => set_obj,
            Some(_) => {
                context.set_error("PySet_Pop expected set object");
                return std::ptr::null_mut();
            }
            None => {
                context.set_error("PySet_Pop received unknown set pointer");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::SetPop,
            receiver,
            Vec::new(),
            HashMap::new(),
        ) {
            Ok(NativeCallResult::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(_) => {
                context.set_error("PySet_Pop returned invalid result");
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
