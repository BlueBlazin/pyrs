use std::ffi::{c_char, c_void};

use crate::runtime::{Object, Value};

use super::{
    Py_IncRef, c_name_to_string, cpython_set_error, dict_get_value, dict_set_value_checked,
    with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyContextVar_New(
    name: *const c_char,
    default_value: *mut c_void,
) -> *mut c_void {
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let result = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyContextVar_New missing VM context");
            return std::ptr::null_mut();
        }
        let default = if default_value.is_null() {
            Value::None
        } else {
            context.pin_capsule_allocation_for_vm(default_value);
            match context.cpython_value_from_ptr_or_proxy(default_value) {
                Some(value) => value,
                None => {
                    context.set_error(format!(
                        "PyContextVar_New received unknown default pointer {:p}",
                        default_value
                    ));
                    return std::ptr::null_mut();
                }
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let dict = vm.heap.alloc_dict(vec![
            (Value::Str("name".to_string()), Value::Str(name.clone())),
            (Value::Str("default".to_string()), default),
            (
                Value::Str("__pyrs_contextvar__".to_string()),
                Value::Bool(true),
            ),
        ]);
        let ptr = context.alloc_cpython_ptr_for_value(dict.clone());
        if ptr.is_null() {
            context.set_error("PyContextVar_New failed to materialize context-var object");
            return std::ptr::null_mut();
        }
        // Keep context-var objects process-stable for extension static storage.
        // Context drop will pin (rather than free) allocations with refcount > 1.
        unsafe {
            Py_IncRef(ptr);
        }
        vm.extension_contextvar_registry.insert(ptr as usize, dict);
        ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    });
    if std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some() {
        eprintln!(
            "[numpy-init] PyContextVar_New name={} default_ptr={:p} result={:p}",
            name, default_value, result
        );
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyContextVar_Get(
    var: *mut c_void,
    default_value: *mut c_void,
    out_value: *mut *mut c_void,
) -> i32 {
    if out_value.is_null() {
        cpython_set_error("PyContextVar_Get requires non-null output pointer");
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        let trace_contextvar = std::env::var_os("PYRS_TRACE_CPY_CONTEXTVAR").is_some();
        // Prefer explicit default value if provided.
        let resolved = if !default_value.is_null() {
            match context.cpython_value_from_ptr_or_proxy(default_value) {
                Some(value) => Some(value),
                None => {
                    context.set_error("PyContextVar_Get received unknown default pointer");
                    return -1;
                }
            }
        } else {
            let var_value = context.cpython_value_from_ptr(var).or_else(|| {
                if context.vm.is_null() {
                    None
                } else {
                    // SAFETY: VM pointer is valid for active context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    vm.extension_contextvar_registry
                        .get(&(var as usize))
                        .cloned()
                }
            });
            let Some(var_value) = var_value else {
                context.set_error("PyContextVar_Get received unknown var pointer");
                return -1;
            };
            match var_value {
                Value::Dict(dict_obj) => {
                    dict_get_value(&dict_obj, &Value::Str("default".to_string()))
                }
                _ => None,
            }
        };
        if let Some(value) = resolved {
            let ptr = context.alloc_cpython_ptr_for_value(value);
            if trace_contextvar {
                eprintln!(
                    "[cpy-contextvar] get var={:p} default={:p} -> out={:p}",
                    var, default_value, ptr
                );
            }
            // SAFETY: caller provided writable out pointer.
            unsafe { *out_value = ptr };
        } else {
            if trace_contextvar {
                eprintln!(
                    "[cpy-contextvar] get var={:p} default={:p} -> out=<null>",
                    var, default_value
                );
            }
            // SAFETY: caller provided writable out pointer.
            unsafe { *out_value = std::ptr::null_mut() };
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyContextVar_Set(var: *mut c_void, value: *mut c_void) -> *mut c_void {
    let Some(value) = with_active_cpython_context_mut(|context| {
        let trace_contextvar = std::env::var_os("PYRS_TRACE_CPY_CONTEXTVAR").is_some();
        let var_value = context.cpython_value_from_ptr(var).or_else(|| {
            if context.vm.is_null() {
                None
            } else {
                // SAFETY: VM pointer is valid for active context lifetime.
                let vm = unsafe { &mut *context.vm };
                vm.extension_contextvar_registry
                    .get(&(var as usize))
                    .cloned()
            }
        });
        let Some(var_value) = var_value else {
            context.set_error("PyContextVar_Set received unknown var pointer");
            return None;
        };
        let Some(new_value) = context.cpython_value_from_ptr_or_proxy(value) else {
            context.set_error("PyContextVar_Set received unknown value pointer");
            return None;
        };
        context.pin_capsule_allocation_for_vm(value);
        let Value::Dict(dict_obj) = var_value else {
            context.set_error("PyContextVar_Set expected context-var object");
            return None;
        };
        let Object::Dict(_) = &mut *dict_obj.kind_mut() else {
            context.set_error("PyContextVar_Set context-var storage invalid");
            return None;
        };
        let _ = dict_set_value_checked(&dict_obj, Value::Str("value".to_string()), new_value);
        let token = context.alloc_cpython_ptr_for_value(Value::None);
        if trace_contextvar {
            eprintln!(
                "[cpy-contextvar] set var={:p} value={:p} -> token={:p}",
                var, value, token
            );
        }
        Some(token)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        None
    }) else {
        return std::ptr::null_mut();
    };
    value
}
