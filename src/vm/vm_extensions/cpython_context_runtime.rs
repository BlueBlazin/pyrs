use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use crate::runtime::{BuiltinFunction, Object, Value};

use super::cpython_error_numeric_api::cpython_make_exception_instance_from_type_and_value;
use super::{
    ACTIVE_CPYTHON_INIT_CONTEXT, InternalCallOutcome, ModuleCapiContext, PyExc_RuntimeError,
    cpython_call_internal_in_context, cpython_exception_class_name_from_ptr,
    cpython_exception_ptr_for_name, cpython_exception_type_ptr,
    cpython_keyword_args_from_dict_object,
    cpython_positional_args_from_tuple_object,
};

pub(in crate::vm::vm_extensions) fn with_active_cpython_context_mut<R>(
    f: impl FnOnce(&mut ModuleCapiContext) -> R,
) -> Result<R, String> {
    ACTIVE_CPYTHON_INIT_CONTEXT.with(|cell| {
        let ptr = cell.get();
        if ptr.is_null() {
            return Err("no active CPython extension init context".to_string());
        }
        // SAFETY: the pointer is set only while the owning `ModuleCapiContext` is alive.
        Ok(f(unsafe { &mut *ptr }))
    })
}

pub(in crate::vm::vm_extensions) fn cpython_set_active_context(
    context: *mut ModuleCapiContext,
) -> *mut ModuleCapiContext {
    ACTIVE_CPYTHON_INIT_CONTEXT.with(|cell| {
        let previous = cell.get();
        cell.set(context);
        previous
    })
}

pub(in crate::vm::vm_extensions) struct ActiveCpythonContextGuard {
    context: *mut ModuleCapiContext,
    previous: *mut ModuleCapiContext,
}

impl ActiveCpythonContextGuard {
    pub(in crate::vm::vm_extensions) fn push(context: *mut ModuleCapiContext) -> Self {
        let previous = cpython_set_active_context(context);
        Self { context, previous }
    }
}

impl Drop for ActiveCpythonContextGuard {
    fn drop(&mut self) {
        let previous_ptr = self.previous;
        if !self.previous.is_null() && !self.context.is_null() {
            // SAFETY: both pointers refer to live contexts for the active call stack.
            unsafe {
                let previous = &mut *self.previous;
                let current = &mut *self.context;
                if previous.current_error.is_none()
                    && let Some(state) = current.current_error
                {
                    let message = current
                        .last_error
                        .clone()
                        .or_else(|| current.first_error.clone())
                        .unwrap_or_else(|| "nested CPython context raised an error".to_string());
                    let mut propagated_ptype = if state.ptype.is_null() {
                        PyExc_RuntimeError
                    } else if let Some(name) = cpython_exception_class_name_from_ptr(state.ptype) {
                        if let Some(symbol_ptr) = cpython_exception_ptr_for_name(&name) {
                            symbol_ptr
                        } else if let Some(value) =
                            current.cpython_value_from_ptr_or_proxy(state.ptype)
                        {
                            let ptr = previous.alloc_cpython_ptr_for_value(value);
                            if ptr.is_null() {
                                PyExc_RuntimeError
                            } else {
                                ptr
                            }
                        } else {
                            PyExc_RuntimeError
                        }
                    } else if let Some(value) = current.cpython_value_from_ptr_or_proxy(state.ptype)
                    {
                        let ptr = previous.alloc_cpython_ptr_for_value(value);
                        if ptr.is_null() {
                            PyExc_RuntimeError
                        } else {
                            ptr
                        }
                    } else {
                        // Nested context-owned pointers must never escape as raw pointers.
                        PyExc_RuntimeError
                    };
                    let propagated_pvalue = if state.pvalue.is_null() {
                        std::ptr::null_mut()
                    } else if let Some(value) =
                        current.cpython_value_from_ptr_or_proxy(state.pvalue)
                    {
                        previous.alloc_cpython_ptr_for_value(value)
                    } else {
                        std::ptr::null_mut()
                    };
                    if propagated_ptype == PyExc_RuntimeError && !propagated_pvalue.is_null() {
                        let derived = cpython_exception_type_ptr(propagated_pvalue);
                        if !derived.is_null() {
                            propagated_ptype = derived;
                        }
                    }
                    let propagated_ptraceback = if state.ptraceback.is_null() {
                        std::ptr::null_mut()
                    } else if let Some(value) =
                        current.cpython_value_from_ptr_or_proxy(state.ptraceback)
                    {
                        previous.alloc_cpython_ptr_for_value(value)
                    } else {
                        std::ptr::null_mut()
                    };
                    previous.set_error_state(
                        propagated_ptype,
                        propagated_pvalue,
                        propagated_ptraceback,
                        message,
                    );
                }
            }
        }
        cpython_set_active_context(self.previous);
        if !previous_ptr.is_null() {
            // SAFETY: previous context pointer is still live on this call stack.
            unsafe {
                (&mut *previous_ptr).sync_thread_state_exception_view_from_current_error();
            }
        } else if !self.context.is_null() {
            // SAFETY: current context is still live for this drop invocation.
            unsafe {
                let current = &mut *self.context;
                current.current_error = None;
                current.sync_thread_state_exception_view_from_current_error();
            }
        }
    }
}

pub(in crate::vm::vm_extensions) fn cpython_trace_numpy_reduce_enabled() -> bool {
    cpython_trace_flag_enabled("PYRS_TRACE_NUMPY_REDUCE")
}

pub(in crate::vm::vm_extensions) fn cpython_trace_flag_enabled(name: &'static str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock()
        && let Some(value) = guard.get(name)
    {
        return *value;
    }
    let enabled = std::env::var_os(name).is_some();
    if let Ok(mut guard) = cache.lock() {
        guard.insert(name, enabled);
    }
    enabled
}

pub(in crate::vm::vm_extensions) fn cpython_is_reduce_probe_name(name: &str) -> bool {
    matches!(
        name,
        "__reduce__"
            | "__reduce_cython__"
            | "__setstate__"
            | "__setstate_cython__"
            | "__set_name__"
            | "__name__"
            | "__dict__"
    )
}

pub(in crate::vm::vm_extensions) fn cpython_error_message_indicates_missing_attribute() -> bool {
    with_active_cpython_context_mut(|context| {
        context
            .last_error
            .as_deref()
            .or(context.first_error.as_deref())
            .is_some_and(|message| message.contains("has no attribute"))
    })
    .unwrap_or(false)
}

#[track_caller]
pub(in crate::vm::vm_extensions) fn cpython_set_error(message: impl Into<String>) {
    let message = message.into();
    if cpython_trace_flag_enabled("PYRS_TRACE_UNKNOWN_PTR")
        && message.contains("unknown PyObject pointer")
    {
        let caller = std::panic::Location::caller();
        eprintln!(
            "[cpy-unknown-set-error] {} (at {}:{})",
            message,
            caller.file(),
            caller.line()
        );
    }
    if cpython_trace_flag_enabled("PYRS_TRACE_CPY_ERRORS") {
        let caller = std::panic::Location::caller();
        eprintln!(
            "[cpy-err] {} (at {}:{})",
            message,
            caller.file(),
            caller.line()
        );
    }
    let _ = with_active_cpython_context_mut(|context| {
        context.set_error(message);
    });
}

#[track_caller]
pub(in crate::vm::vm_extensions) fn cpython_set_typed_error(
    ptype: *mut c_void,
    message: impl Into<String>,
) {
    let message = message.into();
    if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
        let caller = std::panic::Location::caller();
        eprintln!(
            "[cpy-typed-err-src] {} (at {}:{})",
            message,
            caller.file(),
            caller.line()
        );
    }
    let _ = with_active_cpython_context_mut(|context| {
        let ty = if ptype.is_null() {
            // SAFETY: exception singleton pointer is process-global.
            unsafe { PyExc_RuntimeError }
        } else {
            ptype
        };
        let fallback_value = Value::Str(message.clone());
        // Mirror CPython's normalized error state when possible so consumers
        // that read `tstate->current_exception` get an exception instance.
        let pvalue =
            cpython_make_exception_instance_from_type_and_value(context, ty, Some(fallback_value))
                .unwrap_or_else(|| {
                    context.alloc_cpython_ptr_for_value(Value::Str(message.clone()))
                });
        context.set_error_state(ty, pvalue, std::ptr::null_mut(), message);
    });
}

pub(in crate::vm::vm_extensions) fn cpython_value_from_ptr(
    object: *mut c_void,
) -> Result<Value, String> {
    if object.is_null() {
        return Err("received null PyObject pointer".to_string());
    }
    let resolved =
        with_active_cpython_context_mut(|context| context.cpython_value_from_borrowed_ptr(object))
            .map_err(|err| err.to_string())?;
    if resolved.is_none() && std::env::var_os("PYRS_TRACE_UNKNOWN_PTR").is_some() {
        eprintln!(
            "[cpy-unknown-ptr] cpython_value_from_ptr object={:p}",
            object
        );
    }
    resolved.ok_or_else(|| "unknown PyObject pointer".to_string())
}

pub(in crate::vm::vm_extensions) fn cpython_value_from_ptr_or_proxy(
    object: *mut c_void,
) -> Result<Value, String> {
    if object.is_null() {
        return Err("received null PyObject pointer".to_string());
    }
    let resolved =
        with_active_cpython_context_mut(|context| context.cpython_value_from_borrowed_ptr(object))
            .map_err(|err| err.to_string())?;
    if resolved.is_none() && std::env::var_os("PYRS_TRACE_UNKNOWN_PTR").is_some() {
        eprintln!(
            "[cpy-unknown-ptr] cpython_value_from_ptr_or_proxy object={:p}",
            object
        );
    }
    resolved.ok_or_else(|| "unknown PyObject pointer".to_string())
}

pub(in crate::vm::vm_extensions) fn cpython_new_ptr_for_value(value: Value) -> *mut c_void {
    with_active_cpython_context_mut(|context| context.alloc_cpython_ptr_for_value(value))
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            std::ptr::null_mut()
        })
}

pub(in crate::vm::vm_extensions) fn cpython_new_bytes_ptr(bytes: Vec<u8>) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("missing VM context for bytes allocation");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for the active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let bytes_obj = vm.heap.alloc(Object::Bytes(bytes));
        context.alloc_cpython_ptr_for_value(Value::Bytes(bytes_obj))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

pub(in crate::vm::vm_extensions) fn cpython_call_builtin(
    function: BuiltinFunction,
    args: Vec<Value>,
) -> Result<Value, String> {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return Err("missing VM context for builtin call".to_string());
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_internal(Value::Builtin(function), args, HashMap::new()) {
            Ok(InternalCallOutcome::Value(value)) => Ok(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => Err(vm
                .runtime_error_from_active_exception("builtin call failed")
                .message),
            Err(err) => Err(err.message),
        }
    })?
}

pub(in crate::vm::vm_extensions) unsafe extern "C" fn cpython_builtin_cfunction_varargs_kwargs(
    self_obj: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if self_obj.is_null() {
            context.set_error("builtin cfunction shim missing method definition");
            return std::ptr::null_mut();
        }
        let Some(builtin) = context
            .cpython_builtin_by_method_def
            .get(&(self_obj as usize))
            .copied()
        else {
            context.set_error("builtin cfunction shim received unknown method definition");
            return std::ptr::null_mut();
        };
        let positional = match cpython_positional_args_from_tuple_object(args) {
            Ok(values) => values,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let keyword_args = match cpython_keyword_args_from_dict_object(kwargs) {
            Ok(values) => values,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let result = match cpython_call_internal_in_context(
            context,
            Value::Builtin(builtin),
            positional,
            keyword_args,
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[cfg(test)]
mod tests {
    use crate::vm::Vm;

    use super::{
        ACTIVE_CPYTHON_INIT_CONTEXT, ActiveCpythonContextGuard, ModuleCapiContext,
        cpython_set_active_context,
    };

    #[test]
    fn active_context_guard_restores_previous_context() {
        let mut vm = Vm::new();
        let mut outer = ModuleCapiContext::new(std::ptr::addr_of_mut!(vm), vm.main_module.clone());
        let mut inner = ModuleCapiContext::new(std::ptr::addr_of_mut!(vm), vm.main_module.clone());
        let outer_ptr = std::ptr::addr_of_mut!(outer);
        let inner_ptr = std::ptr::addr_of_mut!(inner);

        let prior = cpython_set_active_context(outer_ptr);
        assert!(prior.is_null());
        {
            let _guard = ActiveCpythonContextGuard::push(inner_ptr);
            let active = ACTIVE_CPYTHON_INIT_CONTEXT.with(|cell| cell.get());
            assert_eq!(active, inner_ptr);
        }
        let active = ACTIVE_CPYTHON_INIT_CONTEXT.with(|cell| cell.get());
        assert_eq!(active, outer_ptr);
        cpython_set_active_context(std::ptr::null_mut());
    }

    #[test]
    fn active_context_guard_propagates_nested_error_message() {
        let mut vm = Vm::new();
        let mut outer = ModuleCapiContext::new(std::ptr::addr_of_mut!(vm), vm.main_module.clone());
        let mut inner = ModuleCapiContext::new(std::ptr::addr_of_mut!(vm), vm.main_module.clone());
        let outer_ptr = std::ptr::addr_of_mut!(outer);
        let inner_ptr = std::ptr::addr_of_mut!(inner);

        cpython_set_active_context(outer_ptr);
        {
            let _guard = ActiveCpythonContextGuard::push(inner_ptr);
            inner.set_error("nested error");
        }
        assert_eq!(outer.last_error.as_deref(), Some("nested error"));
        assert!(outer.current_error.is_some());
        cpython_set_active_context(std::ptr::null_mut());
    }
}
