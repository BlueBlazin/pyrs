use std::collections::HashMap;
use std::ffi::{c_char, c_void};

use crate::runtime::{Object, Value};

use super::{cpython_set_error, with_active_cpython_context_mut};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_GetBuiltins() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyEval_GetBuiltins missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(module) = vm.modules.get("builtins") else {
            context.set_error("PyEval_GetBuiltins missing builtins module");
            return std::ptr::null_mut();
        };
        let globals = match &*module.kind() {
            Object::Module(data) => data.globals.clone(),
            _ => {
                context.set_error("PyEval_GetBuiltins invalid builtins module object");
                return std::ptr::null_mut();
            }
        };
        let entries: Vec<(Value, Value)> = globals
            .into_iter()
            .map(|(name, value)| (Value::Str(name), value))
            .collect();
        let dict = vm.heap.alloc_dict(entries);
        context.alloc_cpython_ptr_for_value(dict)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_GetFrame() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(active) = vm.frames.last() else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(Value::Code(active.code.clone()))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_GetGlobals() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_globals(Vec::new(), HashMap::new()) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) if err.message == "no frame" => std::ptr::null_mut(),
            Err(err) => {
                context.set_error(format!("PyEval_GetGlobals failed: {}", err.message));
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
pub unsafe extern "C" fn PyEval_GetLocals() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_locals(Vec::new(), HashMap::new()) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) if err.message == "no frame" => std::ptr::null_mut(),
            Err(err) => {
                context.set_error(format!("PyEval_GetLocals failed: {}", err.message));
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
pub unsafe extern "C" fn PyEval_GetFrameBuiltins() -> *mut c_void {
    unsafe { PyEval_GetBuiltins() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_GetFrameGlobals() -> *mut c_void {
    unsafe { PyEval_GetGlobals() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_GetFrameLocals() -> *mut c_void {
    unsafe { PyEval_GetLocals() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_GetFuncName(func: *mut c_void) -> *const c_char {
    with_active_cpython_context_mut(|context| {
        if func.is_null() {
            return c"<unknown>".as_ptr();
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(func) else {
            return c"<unknown>".as_ptr();
        };
        let name = match value {
            Value::Function(function_obj) => match &*function_obj.kind() {
                Object::Function(function_data) => function_data.code.name.clone(),
                _ => "<function>".to_string(),
            },
            Value::BoundMethod(bound_obj) => match &*bound_obj.kind() {
                Object::BoundMethod(bound_data) => match &*bound_data.function.kind() {
                    Object::Function(function_data) => function_data.code.name.clone(),
                    Object::NativeMethod(_) => "<built-in>".to_string(),
                    _ => "<bound method>".to_string(),
                },
                _ => "<bound method>".to_string(),
            },
            Value::Builtin(_) => "<built-in>".to_string(),
            Value::Class(class_obj) => match &*class_obj.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "type".to_string(),
            },
            other => {
                if context.vm.is_null() {
                    "<object>".to_string()
                } else {
                    // SAFETY: VM pointer is valid for active context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    vm.value_type_name_for_error(&other)
                }
            }
        };
        context
            .scratch_c_string_ptr(&name)
            .unwrap_or(c"<unknown>".as_ptr())
    })
    .unwrap_or(c"<unknown>".as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_GetFuncDesc(func: *mut c_void) -> *const c_char {
    with_active_cpython_context_mut(|context| {
        if func.is_null() {
            return c" object".as_ptr();
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(func) else {
            return c" object".as_ptr();
        };
        match value {
            Value::Class(_) => c" constructor".as_ptr(),
            Value::Function(_) | Value::BoundMethod(_) | Value::Builtin(_) => c"()".as_ptr(),
            other => {
                if context.vm.is_null() {
                    c" object".as_ptr()
                } else {
                    // SAFETY: VM pointer is valid for active context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    if vm.is_callable_value(&other) {
                        c"()".as_ptr()
                    } else {
                        c" object".as_ptr()
                    }
                }
            }
        }
    })
    .unwrap_or(c" object".as_ptr())
}
