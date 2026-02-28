use std::ffi::{c_char, c_void};

use crate::runtime::Value;

use super::{
    CpythonObjectHead, CpythonTypeObject, ModuleCapiContext, Py_DecRef, Py_IncRef,
    c_name_to_string, cpython_set_error, dict_contains_key_checked, dict_get_value,
    dict_remove_value, dict_set_value_checked, with_active_cpython_context_mut,
};

const CONTEXTVAR_MARKER_KEY: &str = "__pyrs_contextvar__";
const CONTEXTVAR_DEFAULT_KEY: &str = "default";
const CONTEXTVAR_HAS_DEFAULT_KEY: &str = "__pyrs_contextvar_has_default__";
const CONTEXTVAR_VALUE_KEY: &str = "value";

const CONTEXTTOKEN_MARKER_KEY: &str = "__pyrs_contexttoken__";
const CONTEXTTOKEN_VAR_KEY: &str = "var";
const CONTEXTTOKEN_HAD_OLD_KEY: &str = "had_old_value";
const CONTEXTTOKEN_OLD_KEY: &str = "old_value";
const CONTEXTTOKEN_USED_KEY: &str = "used";

fn contextvar_value_from_ptr(context: &mut ModuleCapiContext, var: *mut c_void) -> Option<Value> {
    context.cpython_value_from_ptr(var).or_else(|| {
        if context.vm.is_null() {
            None
        } else {
            // SAFETY: VM pointer is valid for active context lifetime.
            let vm = unsafe { &mut *context.vm };
            vm.extension_contextvar_registry
                .get(&(var as usize))
                .cloned()
        }
    })
}

fn contextvar_dict_from_var_ptr(
    context: &mut ModuleCapiContext,
    var: *mut c_void,
    api_name: &str,
) -> Result<crate::vm::ObjRef, i32> {
    let Some(var_value) = contextvar_value_from_ptr(context, var) else {
        context.set_error(format!("{api_name} received unknown var pointer"));
        return Err(-1);
    };
    let Value::Dict(var_dict) = var_value else {
        context.set_error(format!("{api_name} expected context-var object"));
        return Err(-1);
    };
    let marker = dict_get_value(&var_dict, &Value::Str(CONTEXTVAR_MARKER_KEY.to_string()));
    if !matches!(marker, Some(Value::Bool(true))) {
        context.set_error(format!("{api_name} expected context-var object"));
        return Err(-1);
    }
    Ok(var_dict)
}

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
        let has_default = !default_value.is_null();
        let default = if has_default {
            context.pin_capsule_allocation_for_vm(default_value);
            context.pin_owned_cpython_allocation_for_vm(default_value);
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
        } else {
            Value::None
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let dict = vm.heap.alloc_dict(vec![
            (Value::Str("name".to_string()), Value::Str(name.clone())),
            (Value::Str(CONTEXTVAR_DEFAULT_KEY.to_string()), default),
            (
                Value::Str(CONTEXTVAR_HAS_DEFAULT_KEY.to_string()),
                Value::Bool(has_default),
            ),
            (
                Value::Str(CONTEXTVAR_MARKER_KEY.to_string()),
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
        unsafe { Py_IncRef(ptr) };
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
        let var_dict = match contextvar_dict_from_var_ptr(context, var, "PyContextVar_Get") {
            Ok(dict) => dict,
            Err(status) => return status,
        };
        let has_value = match dict_contains_key_checked(
            &var_dict,
            &Value::Str(CONTEXTVAR_VALUE_KEY.to_string()),
        ) {
            Ok(flag) => flag,
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        };
        let resolved = if has_value {
            dict_get_value(&var_dict, &Value::Str(CONTEXTVAR_VALUE_KEY.to_string()))
        } else if !default_value.is_null() {
            match context.cpython_value_from_ptr_or_proxy(default_value) {
                Some(value) => Some(value),
                None => {
                    context.set_error("PyContextVar_Get received unknown default pointer");
                    return -1;
                }
            }
        } else {
            let has_default = match dict_get_value(
                &var_dict,
                &Value::Str(CONTEXTVAR_HAS_DEFAULT_KEY.to_string()),
            ) {
                Some(Value::Bool(flag)) => flag,
                // Compatibility: Python-level `_contextvars.ContextVar` objects created by
                // older bootstrap paths may omit the explicit has-default marker.
                _ => dict_contains_key_checked(
                    &var_dict,
                    &Value::Str(CONTEXTVAR_DEFAULT_KEY.to_string()),
                )
                .unwrap_or(false),
            };
            if has_default {
                dict_get_value(&var_dict, &Value::Str(CONTEXTVAR_DEFAULT_KEY.to_string()))
                    .or(Some(Value::None))
            } else {
                None
            }
        };
        let ptr = resolved
            .map(|value| context.alloc_cpython_ptr_for_value(value))
            .unwrap_or(std::ptr::null_mut());
        if !ptr.is_null() {
            // PyContextVar_Get returns a new reference.
            unsafe { Py_IncRef(ptr) };
        }
        if trace_contextvar {
            if ptr.is_null() {
                eprintln!(
                    "[cpy-contextvar] get var={:p} default={:p} -> out=<null>",
                    var, default_value
                );
            } else {
                // SAFETY: debug-only best-effort type/refcount probe.
                let (type_name, refcnt) = unsafe {
                    let head = ptr.cast::<CpythonObjectHead>().as_ref();
                    let refcnt = head.map(|h| h.ob_refcnt).unwrap_or(-1);
                    let type_name = head
                        .and_then(|h| h.ob_type.cast::<CpythonTypeObject>().as_ref())
                        .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    (type_name, refcnt)
                };
                eprintln!(
                    "[cpy-contextvar] get var={:p} default={:p} -> out={:p} type={} rc={}",
                    var, default_value, ptr, type_name, refcnt
                );
                // Decimal's Context object stores `modstate` at offset 0x60.
                let modstate = unsafe { *((ptr.cast::<u8>().add(0x60)).cast::<*mut c_void>()) };
                eprintln!(
                    "[cpy-contextvar] get out={:p} modstate@0x60={:p}",
                    ptr, modstate
                );
            }
        }
        // SAFETY: caller provided writable out pointer.
        unsafe { *out_value = ptr };
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyContextVar_Set(var: *mut c_void, value: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let trace_contextvar = std::env::var_os("PYRS_TRACE_CPY_CONTEXTVAR").is_some();
        let var_dict = match contextvar_dict_from_var_ptr(context, var, "PyContextVar_Set") {
            Ok(dict) => dict,
            Err(_) => return std::ptr::null_mut(),
        };
        let Some(new_value) = context.cpython_value_from_ptr_or_proxy(value) else {
            context.set_error("PyContextVar_Set received unknown value pointer");
            return std::ptr::null_mut();
        };
        context.pin_capsule_allocation_for_vm(value);
        context.pin_owned_cpython_allocation_for_vm(value);
        let had_old = match dict_contains_key_checked(
            &var_dict,
            &Value::Str(CONTEXTVAR_VALUE_KEY.to_string()),
        ) {
            Ok(flag) => flag,
            Err(err) => {
                context.set_error(err.message);
                return std::ptr::null_mut();
            }
        };
        let old_value_ptr = if had_old {
            dict_get_value(&var_dict, &Value::Str(CONTEXTVAR_VALUE_KEY.to_string()))
                .map(|old| context.alloc_cpython_ptr_for_value(old))
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        };
        let old_value = dict_get_value(&var_dict, &Value::Str(CONTEXTVAR_VALUE_KEY.to_string()))
            .unwrap_or(Value::None);
        // ContextVar keeps a strong reference to the stored value.
        unsafe { Py_IncRef(value) };
        if let Err(err) = dict_set_value_checked(
            &var_dict,
            Value::Str(CONTEXTVAR_VALUE_KEY.to_string()),
            new_value,
        ) {
            context.set_error(err.message);
            return std::ptr::null_mut();
        }
        if !old_value_ptr.is_null() {
            unsafe { Py_DecRef(old_value_ptr) };
        }
        if context.vm.is_null() {
            context.set_error("PyContextVar_Set missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let token = vm.heap.alloc_dict(vec![
            (
                Value::Str(CONTEXTTOKEN_MARKER_KEY.to_string()),
                Value::Bool(true),
            ),
            (
                Value::Str(CONTEXTTOKEN_VAR_KEY.to_string()),
                Value::Dict(var_dict),
            ),
            (
                Value::Str(CONTEXTTOKEN_HAD_OLD_KEY.to_string()),
                Value::Bool(had_old),
            ),
            (Value::Str(CONTEXTTOKEN_OLD_KEY.to_string()), old_value),
            (
                Value::Str(CONTEXTTOKEN_USED_KEY.to_string()),
                Value::Bool(false),
            ),
        ]);
        let token_ptr = context.alloc_cpython_ptr_for_value(token);
        if trace_contextvar {
            // SAFETY: debug-only best-effort type/refcount probe.
            let (type_name, refcnt) = unsafe {
                let head = value.cast::<CpythonObjectHead>().as_ref();
                let refcnt = head.map(|h| h.ob_refcnt).unwrap_or(-1);
                let type_name = head
                    .and_then(|h| h.ob_type.cast::<CpythonTypeObject>().as_ref())
                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string());
                (type_name, refcnt)
            };
            eprintln!(
                "[cpy-contextvar] set var={:p} value={:p} type={} rc={} -> token={:p}",
                var, value, type_name, refcnt, token_ptr
            );
            if !value.is_null() {
                // Decimal's Context object stores `modstate` at offset 0x60.
                let modstate = unsafe { *((value.cast::<u8>().add(0x60)).cast::<*mut c_void>()) };
                eprintln!(
                    "[cpy-contextvar] set value={:p} modstate@0x60={:p}",
                    value, modstate
                );
            }
        }
        token_ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyContextVar_Reset(var: *mut c_void, token: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let var_dict = match contextvar_dict_from_var_ptr(context, var, "PyContextVar_Reset") {
            Ok(dict) => dict,
            Err(status) => return status,
        };
        let Some(token_value) = context.cpython_value_from_ptr_or_proxy(token) else {
            context.set_error("PyContextVar_Reset received unknown token pointer");
            return -1;
        };
        let Value::Dict(token_dict) = token_value else {
            context.set_error("PyContextVar_Reset expected token object");
            return -1;
        };
        if !matches!(
            dict_get_value(
                &token_dict,
                &Value::Str(CONTEXTTOKEN_MARKER_KEY.to_string())
            ),
            Some(Value::Bool(true))
        ) {
            context.set_error("PyContextVar_Reset expected token object");
            return -1;
        }
        if matches!(
            dict_get_value(&token_dict, &Value::Str(CONTEXTTOKEN_USED_KEY.to_string())),
            Some(Value::Bool(true))
        ) {
            context.set_error("PyContextVar_Reset token already used");
            return -1;
        }
        let token_var = dict_get_value(&token_dict, &Value::Str(CONTEXTTOKEN_VAR_KEY.to_string()));
        let same_var = match token_var {
            Some(Value::Dict(token_var_dict)) => token_var_dict.id() == var_dict.id(),
            _ => false,
        };
        if !same_var {
            context.set_error("PyContextVar_Reset token belongs to a different ContextVar");
            return -1;
        }

        let has_current = match dict_contains_key_checked(
            &var_dict,
            &Value::Str(CONTEXTVAR_VALUE_KEY.to_string()),
        ) {
            Ok(flag) => flag,
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        };
        let current_value_ptr = if has_current {
            dict_get_value(&var_dict, &Value::Str(CONTEXTVAR_VALUE_KEY.to_string()))
                .map(|value| context.alloc_cpython_ptr_for_value(value))
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        };

        let had_old = matches!(
            dict_get_value(
                &token_dict,
                &Value::Str(CONTEXTTOKEN_HAD_OLD_KEY.to_string())
            ),
            Some(Value::Bool(true))
        );
        if had_old {
            let old_value =
                dict_get_value(&token_dict, &Value::Str(CONTEXTTOKEN_OLD_KEY.to_string()))
                    .unwrap_or(Value::None);
            let old_value_ptr = context.alloc_cpython_ptr_for_value(old_value.clone());
            if !old_value_ptr.is_null() {
                unsafe { Py_IncRef(old_value_ptr) };
            }
            if let Err(err) = dict_set_value_checked(
                &var_dict,
                Value::Str(CONTEXTVAR_VALUE_KEY.to_string()),
                old_value,
            ) {
                context.set_error(err.message);
                return -1;
            }
        } else {
            let _ = dict_remove_value(&var_dict, &Value::Str(CONTEXTVAR_VALUE_KEY.to_string()));
        }
        if !current_value_ptr.is_null() {
            unsafe { Py_DecRef(current_value_ptr) };
        }

        if let Err(err) = dict_set_value_checked(
            &token_dict,
            Value::Str(CONTEXTTOKEN_USED_KEY.to_string()),
            Value::Bool(true),
        ) {
            context.set_error(err.message);
            return -1;
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}
