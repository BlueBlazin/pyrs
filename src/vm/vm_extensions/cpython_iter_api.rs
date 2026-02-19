use std::collections::HashMap;
use std::ffi::c_void;

use crate::runtime::{RuntimeError, Value};
use crate::vm::{GeneratorResumeKind, GeneratorResumeOutcome, InternalCallOutcome, Vm};

use super::{
    PyErr_BadInternalCall, PyExc_TypeError, cpython_exception_value_attr, cpython_set_error,
    with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIter_Check(object: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return 0;
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            return 0;
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match cpython_value_is_iterator_for_capi(vm, &value) {
            Ok(true) => 1,
            Ok(false) | Err(_) => 0,
        }
    })
    .unwrap_or(0)
}

fn cpython_value_is_exception_name(vm: &Vm, value: &Value, expected: &str) -> bool {
    match value {
        Value::Exception(exception) => exception.name == expected,
        Value::ExceptionType(name) => name == expected,
        Value::Instance(instance) => vm
            .exception_class_name_for_instance(instance)
            .is_some_and(|name| name == expected),
        _ => false,
    }
}

pub(in crate::vm::vm_extensions) fn cpython_active_exception_is(vm: &Vm, expected: &str) -> bool {
    vm.frames
        .last()
        .and_then(|frame| frame.active_exception.as_ref())
        .is_some_and(|value| cpython_value_is_exception_name(vm, value, expected))
}

pub(in crate::vm::vm_extensions) fn cpython_clear_active_exception(vm: &mut Vm) {
    if let Some(frame) = vm.frames.last_mut() {
        frame.active_exception = None;
    }
}

fn cpython_value_is_stop_iteration(vm: &Vm, value: &Value) -> bool {
    cpython_value_is_exception_name(vm, value, "StopIteration")
}

fn cpython_stop_iteration_value_from_active_exception(vm: &Vm) -> Option<Value> {
    let active = vm
        .frames
        .last()
        .and_then(|frame| frame.active_exception.clone())?;
    if !cpython_value_is_stop_iteration(vm, &active) {
        return None;
    }
    Some(cpython_exception_value_attr(&active).unwrap_or(Value::None))
}

fn cpython_value_is_iterator_for_capi(vm: &mut Vm, value: &Value) -> Result<bool, RuntimeError> {
    match value {
        Value::Iterator(_) => Ok(true),
        Value::Generator(_) => vm.ensure_sync_iterator_target(value).map(|_| true),
        Value::Instance(_) => Ok(vm.lookup_bound_special_method(value, "__next__")?.is_some()),
        _ => Ok(false),
    }
}

fn cpython_iter_next_for_capi(vm: &mut Vm, iter: &Value) -> Result<Option<Value>, RuntimeError> {
    if !cpython_value_is_iterator_for_capi(vm, iter)? {
        return Err(RuntimeError::new("expected an iterator"));
    }
    let trace_iter_next = std::env::var_os("PYRS_TRACE_CPY_ITERNEXT").is_some();
    if trace_iter_next {
        let tag = match iter {
            Value::Iterator(_) => "iterator",
            Value::Instance(_) => "instance",
            Value::Generator(_) => "generator",
            _ => "other",
        };
        eprintln!("[cpy-iternext] start iter={}", tag);
    }
    match vm.next_from_iterator_value(iter)? {
        GeneratorResumeOutcome::Yield(value) => {
            if trace_iter_next {
                eprintln!("[cpy-iternext] yield {}", crate::vm::format_repr(&value));
            }
            Ok(Some(value))
        }
        GeneratorResumeOutcome::Complete(_) => {
            if trace_iter_next {
                eprintln!("[cpy-iternext] complete");
            }
            Ok(None)
        }
        GeneratorResumeOutcome::PropagatedException => {
            if cpython_stop_iteration_value_from_active_exception(vm).is_some() {
                if trace_iter_next {
                    eprintln!("[cpy-iternext] propagated stop-iteration");
                }
                Ok(None)
            } else {
                if trace_iter_next {
                    eprintln!(
                        "[cpy-iternext] propagated error {}",
                        vm.runtime_error_from_active_exception("iteration failed").message
                    );
                }
                Err(vm.runtime_error_from_active_exception("iteration failed"))
            }
        }
    }
}

fn cpython_iter_send_for_capi(
    vm: &mut Vm,
    iter: Value,
    arg: Value,
) -> Result<(i32, Value), RuntimeError> {
    if let Value::Generator(generator) = &iter {
        vm.ensure_sync_iterator_target(&iter)?;
        let sent = if arg == Value::None { None } else { Some(arg) };
        return match vm.resume_generator(generator, sent, None, GeneratorResumeKind::Next)? {
            GeneratorResumeOutcome::Yield(value) => Ok((1, value)),
            GeneratorResumeOutcome::Complete(value) => Ok((0, value)),
            GeneratorResumeOutcome::PropagatedException => {
                if let Some(value) = cpython_stop_iteration_value_from_active_exception(vm) {
                    Ok((0, value))
                } else {
                    Err(vm.runtime_error_from_active_exception("PyIter_Send failed"))
                }
            }
        };
    }

    if arg == Value::None && cpython_value_is_iterator_for_capi(vm, &iter)? {
        return match cpython_iter_next_for_capi(vm, &iter)? {
            Some(value) => Ok((1, value)),
            None => Ok((0, Value::None)),
        };
    }

    let send_method =
        vm.builtin_getattr(vec![iter, Value::Str("send".to_string())], HashMap::new())?;
    match vm.call_internal(send_method, vec![arg], HashMap::new()) {
        Ok(InternalCallOutcome::Value(value)) => Ok((1, value)),
        Ok(InternalCallOutcome::CallerExceptionHandled) => {
            if let Some(value) = cpython_stop_iteration_value_from_active_exception(vm) {
                Ok((0, value))
            } else {
                Err(vm.runtime_error_from_active_exception("PyIter_Send failed"))
            }
        }
        Err(err) => {
            if let Some(value) = cpython_stop_iteration_value_from_active_exception(vm) {
                Ok((0, value))
            } else {
                Err(err)
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIter_NextItem(object: *mut c_void, item: *mut *mut c_void) -> i32 {
    if item.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe { *item = std::ptr::null_mut() };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyIter_NextItem missing VM context");
            return -1;
        }
        let Some(iter) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyIter_NextItem unknown iterator pointer");
            return -1;
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        if !cpython_value_is_iterator_for_capi(vm, &iter).unwrap_or(false) {
            let message = format!(
                "expected an iterator, got '{}'",
                vm.value_type_name_for_error(&iter)
            );
            let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
            context.set_error_state(
                unsafe { PyExc_TypeError },
                pvalue,
                std::ptr::null_mut(),
                message,
            );
            return -1;
        }
        match cpython_iter_next_for_capi(vm, &iter) {
            Ok(Some(next)) => {
                unsafe { *item = context.alloc_cpython_ptr_for_value(next) };
                1
            }
            Ok(None) => 0,
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
pub unsafe extern "C" fn PyIter_Send(
    iter: *mut c_void,
    arg: *mut c_void,
    result: *mut *mut c_void,
) -> i32 {
    if arg.is_null() || result.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe { *result = std::ptr::null_mut() };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyIter_Send missing VM context");
            return -1;
        }
        let Some(iter_value) = context.cpython_value_from_ptr_or_proxy(iter) else {
            context.set_error("PyIter_Send unknown iterator pointer");
            return -1;
        };
        let Some(arg_value) = context.cpython_value_from_ptr_or_proxy(arg) else {
            context.set_error("PyIter_Send unknown argument pointer");
            return -1;
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match cpython_iter_send_for_capi(vm, iter_value, arg_value) {
            Ok((status, value)) => {
                unsafe { *result = context.alloc_cpython_ptr_for_value(value) };
                status
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
pub unsafe extern "C" fn PyIter_Next(object: *mut c_void) -> *mut c_void {
    match with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyIter_Next missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyIter_Next unknown iterator pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let next = match cpython_iter_next_for_capi(vm, &value) {
            Ok(Some(next)) => next,
            Ok(None) => return std::ptr::null_mut(),
            Err(err) => {
                context.set_error(err.message);
                return std::ptr::null_mut();
            }
        };
        context.alloc_cpython_ptr_for_value(next)
    }) {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}
