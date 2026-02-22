use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void};

use crate::runtime::{BuiltinFunction, Object, Value};

use super::{
    CpythonBaseExceptionCompatObject, InternalCallOutcome, ModuleCapiContext, ObjRef, Py_DecRef,
    PyErr_BadInternalCall,
    PyErr_Occurred, PyExc_EOFError, PyExc_SystemError, PyExc_TypeError, PyLong_FromLong,
    PyObject_CallObject, PyObject_CallOneArg, PyObject_GetAttrString, PyObject_Str,
    PyUnicode_AsUTF8, PyUnicode_FromString, Vm, c_name_to_string, cpython_new_bytes_ptr,
    cpython_new_ptr_for_value, cpython_set_error, cpython_set_typed_error, cpython_value_from_ptr,
    with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_FromFd(
    fd: i32,
    _name: *const c_char,
    mode: *const c_char,
    buffering: i32,
    encoding: *const c_char,
    errors: *const c_char,
    newline: *const c_char,
    closefd: i32,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyFile_FromFd missing VM context");
            return std::ptr::null_mut();
        }
        let mode_value = if mode.is_null() {
            Value::Str("r".to_string())
        } else {
            match unsafe { c_name_to_string(mode) } {
                Ok(text) => Value::Str(text),
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
        };
        let encoding_value = if encoding.is_null() {
            None
        } else {
            match unsafe { c_name_to_string(encoding) } {
                Ok(text) => Some(Value::Str(text)),
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
        };
        let errors_value = if errors.is_null() {
            None
        } else {
            match unsafe { c_name_to_string(errors) } {
                Ok(text) => Some(Value::Str(text)),
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
        };
        let newline_value = if newline.is_null() {
            None
        } else {
            match unsafe { c_name_to_string(newline) } {
                Ok(text) => Some(Value::Str(text)),
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
        };
        let mut kwargs = HashMap::new();
        kwargs.insert("mode".to_string(), mode_value);
        if buffering >= 0 {
            kwargs.insert("buffering".to_string(), Value::Int(buffering as i64));
        }
        kwargs.insert("closefd".to_string(), Value::Bool(closefd != 0));
        if let Some(value) = encoding_value {
            kwargs.insert("encoding".to_string(), value);
        }
        if let Some(value) = errors_value {
            kwargs.insert("errors".to_string(), value);
        }
        if let Some(value) = newline_value {
            kwargs.insert("newline".to_string(), value);
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_internal(
            Value::Builtin(BuiltinFunction::IoOpen),
            vec![Value::Int(fd as i64)],
            kwargs,
        ) {
            Ok(InternalCallOutcome::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("PyFile_FromFd failed")
                        .message,
                );
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_GetLine(file: *mut c_void, n: i32) -> *mut c_void {
    if file.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let readline = unsafe { PyObject_GetAttrString(file, c"readline".as_ptr()) };
    if readline.is_null() {
        return std::ptr::null_mut();
    }
    let result = if n <= 0 {
        unsafe { PyObject_CallObject(readline, std::ptr::null_mut()) }
    } else {
        let arg = unsafe { PyLong_FromLong(n as i64) };
        if arg.is_null() {
            unsafe { Py_DecRef(readline) };
            return std::ptr::null_mut();
        }
        let result = unsafe { PyObject_CallOneArg(readline, arg) };
        unsafe { Py_DecRef(arg) };
        result
    };
    unsafe { Py_DecRef(readline) };
    if result.is_null() {
        return std::ptr::null_mut();
    }
    let value = match cpython_value_from_ptr(result) {
        Ok(value) => value,
        Err(err) => {
            unsafe { Py_DecRef(result) };
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match value {
        Value::Bytes(bytes_obj) => {
            let Object::Bytes(payload) = &*bytes_obj.kind() else {
                unsafe { Py_DecRef(result) };
                cpython_set_typed_error(
                    unsafe { PyExc_TypeError },
                    "object.readline() returned non-string",
                );
                return std::ptr::null_mut();
            };
            if n < 0 {
                if payload.is_empty() {
                    unsafe { Py_DecRef(result) };
                    cpython_set_typed_error(unsafe { PyExc_EOFError }, "EOF when reading a line");
                    return std::ptr::null_mut();
                }
                if payload.last().copied() == Some(b'\n') {
                    let mut trimmed = payload.clone();
                    trimmed.pop();
                    unsafe { Py_DecRef(result) };
                    return cpython_new_bytes_ptr(trimmed);
                }
            }
            result
        }
        Value::Str(text) => {
            if n < 0 {
                if text.is_empty() {
                    unsafe { Py_DecRef(result) };
                    cpython_set_typed_error(unsafe { PyExc_EOFError }, "EOF when reading a line");
                    return std::ptr::null_mut();
                }
                if text.ends_with('\n') {
                    let mut trimmed = text;
                    let _ = trimmed.pop();
                    unsafe { Py_DecRef(result) };
                    return cpython_new_ptr_for_value(Value::Str(trimmed));
                }
            }
            result
        }
        _ => {
            unsafe { Py_DecRef(result) };
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "object.readline() returned non-string",
            );
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_WriteObject(
    value: *mut c_void,
    file: *mut c_void,
    flags: i32,
) -> i32 {
    const PY_PRINT_RAW_FLAG: i32 = 1;
    with_active_cpython_context_mut(|context| {
        if file.is_null() {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, "writeobject with NULL file");
            return -1;
        }
        let Some(file_value) = context.cpython_value_from_ptr_or_proxy(file) else {
            context.set_error("PyFile_WriteObject received unknown file pointer");
            return -1;
        };
        let Some(value) = context.cpython_value_from_ptr_or_proxy(value) else {
            context.set_error("PyFile_WriteObject received unknown value pointer");
            return -1;
        };
        if context.vm.is_null() {
            context.set_error("missing VM context for PyFile_WriteObject");
            return -1;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };

        let rendered = match vm.call_internal(
            Value::Builtin(if (flags & PY_PRINT_RAW_FLAG) != 0 {
                BuiltinFunction::Str
            } else {
                BuiltinFunction::Repr
            }),
            vec![value],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(rendered)) => rendered,
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("PyFile_WriteObject render failed")
                        .message,
                );
                return -1;
            }
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        };

        let writer = match vm.call_internal(
            Value::Builtin(BuiltinFunction::GetAttr),
            vec![file_value, Value::Str("write".to_string())],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(writer)) => writer,
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("PyFile_WriteObject missing write")
                        .message,
                );
                return -1;
            }
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        };

        match vm.call_internal(writer, vec![rendered], HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => 0,
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("PyFile_WriteObject write failed")
                        .message,
                );
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
pub unsafe extern "C" fn PyFile_WriteString(text: *const c_char, file: *mut c_void) -> i32 {
    if file.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                "null file for PyFile_WriteString",
            );
        }
        return -1;
    }
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    let value = unsafe { PyUnicode_FromString(text) };
    if value.is_null() {
        return -1;
    }
    let status = unsafe { PyFile_WriteObject(value, file, 1) };
    unsafe { Py_DecRef(value) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceBack_Here(frame: *mut c_void) -> c_int {
    if frame.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceBack_Print(tb: *mut c_void, file: *mut c_void) -> c_int {
    if file.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                "null file for PyTraceBack_Print",
            );
        }
        return -1;
    }
    if tb.is_null() {
        return 0;
    }
    let rendered = unsafe { PyObject_Str(tb) };
    if rendered.is_null() {
        return -1;
    }
    let text = unsafe { PyUnicode_AsUTF8(rendered) };
    if text.is_null() {
        unsafe { Py_DecRef(rendered) };
        return -1;
    }
    let status = unsafe { PyFile_WriteString(text, file) };
    unsafe { Py_DecRef(rendered) };
    status
}

pub(super) fn cpython_is_exception_instance(
    context: &ModuleCapiContext,
    instance: &ObjRef,
) -> bool {
    if context.vm.is_null() {
        return false;
    }
    // SAFETY: VM pointer is valid for active C-API context lifetime.
    unsafe {
        (&*context.vm)
            .exception_class_name_for_instance(instance)
            .is_some()
    }
}

fn cpython_is_exception_instance_for_vm(vm: *mut Vm, instance: &ObjRef) -> bool {
    if vm.is_null() {
        return false;
    }
    // SAFETY: VM pointer is valid for active C-API context lifetime.
    unsafe { (&*vm).exception_class_name_for_instance(instance).is_some() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_GetTraceback(exception: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(exception) else {
            context.set_error("PyException_GetTraceback received unknown exception pointer");
            return std::ptr::null_mut();
        };
        let traceback = match value {
            Value::Exception(exception_obj) => {
                let attrs = exception_obj.attrs.borrow();
                attrs
                    .get("__traceback__")
                    .cloned()
                    .or_else(|| attrs.get("exc_traceback").cloned())
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance(context, &instance) {
                    context.set_error("PyException_GetTraceback expected exception object");
                    return std::ptr::null_mut();
                }
                let Object::Instance(instance_data) = &*instance.kind() else {
                    context.set_error("PyException_GetTraceback encountered invalid instance");
                    return std::ptr::null_mut();
                };
                instance_data
                    .attrs
                    .get("__traceback__")
                    .cloned()
                    .or_else(|| instance_data.attrs.get("exc_traceback").cloned())
            }
            _ => {
                context.set_error("PyException_GetTraceback expected exception object");
                return std::ptr::null_mut();
            }
        };
        match traceback {
            Some(Value::None) | None => std::ptr::null_mut(),
            Some(value) => context.alloc_cpython_ptr_for_value(value),
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_GetCause(exception: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(exception) else {
            context.set_error("PyException_GetCause received unknown exception pointer");
            return std::ptr::null_mut();
        };
        match value {
            Value::Exception(exception_obj) => match exception_obj.cause {
                Some(cause) => context.alloc_cpython_ptr_for_value(Value::Exception(cause)),
                None => std::ptr::null_mut(),
            },
            Value::Instance(instance) => {
                if !cpython_is_exception_instance(context, &instance) {
                    context.set_error("PyException_GetCause expected exception object");
                    return std::ptr::null_mut();
                }
                let Object::Instance(instance_data) = &*instance.kind() else {
                    context.set_error("PyException_GetCause encountered invalid instance");
                    return std::ptr::null_mut();
                };
                match instance_data.attrs.get("__cause__").cloned() {
                    Some(Value::None) | None => std::ptr::null_mut(),
                    Some(value) => context.alloc_cpython_ptr_for_value(value),
                }
            }
            _ => {
                context.set_error("PyException_GetCause expected exception object");
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
pub unsafe extern "C" fn PyException_GetContext(exception: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(exception) else {
            context.set_error("PyException_GetContext received unknown exception pointer");
            return std::ptr::null_mut();
        };
        match value {
            Value::Exception(exception_obj) => match exception_obj.context {
                Some(context_obj) => {
                    context.alloc_cpython_ptr_for_value(Value::Exception(context_obj))
                }
                None => std::ptr::null_mut(),
            },
            Value::Instance(instance) => {
                if !cpython_is_exception_instance(context, &instance) {
                    context.set_error("PyException_GetContext expected exception object");
                    return std::ptr::null_mut();
                }
                let Object::Instance(instance_data) = &*instance.kind() else {
                    context.set_error("PyException_GetContext encountered invalid instance");
                    return std::ptr::null_mut();
                };
                match instance_data.attrs.get("__context__").cloned() {
                    Some(Value::None) | None => std::ptr::null_mut(),
                    Some(value) => context.alloc_cpython_ptr_for_value(value),
                }
            }
            _ => {
                context.set_error("PyException_GetContext expected exception object");
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
pub unsafe extern "C" fn PyException_GetArgs(exception: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(exception) else {
            context.set_error("PyException_GetArgs received unknown exception pointer");
            return std::ptr::null_mut();
        };
        let args_value = match value {
            Value::Exception(exception_obj) => {
                if let Some(args) = exception_obj.attrs.borrow().get("args").cloned() {
                    args
                } else if context.vm.is_null() {
                    Value::None
                } else if let Some(message) = exception_obj.message {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    vm.heap.alloc_tuple(vec![Value::Str(message)])
                } else {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    vm.heap.alloc_tuple(Vec::new())
                }
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance(context, &instance) {
                    context.set_error("PyException_GetArgs expected exception object");
                    return std::ptr::null_mut();
                }
                let Object::Instance(instance_data) = &*instance.kind() else {
                    context.set_error("PyException_GetArgs encountered invalid instance");
                    return std::ptr::null_mut();
                };
                if let Some(args) = instance_data.attrs.get("args").cloned() {
                    args
                } else if context.vm.is_null() {
                    Value::None
                } else {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    vm.heap.alloc_tuple(Vec::new())
                }
            }
            _ => {
                context.set_error("PyException_GetArgs expected exception object");
                return std::ptr::null_mut();
            }
        };
        context.alloc_cpython_ptr_for_value(args_value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_SetArgs(exception: *mut c_void, args: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let vm_ptr = context.vm;
        let Some(handle) = context.cpython_handle_from_ptr(exception) else {
            context.set_error("PyException_SetArgs received unknown exception pointer");
            return;
        };
        let Some(args_value) = context.cpython_value_from_ptr_or_proxy(args) else {
            context.set_error("PyException_SetArgs received unknown args pointer");
            return;
        };
        let Value::Tuple(_) = args_value else {
            context.set_error("PyException_SetArgs expected tuple object");
            return;
        };
        let args_ptr = context.alloc_cpython_ptr_for_value(args_value.clone());
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyException_SetArgs exception handle is not available");
            return;
        };
        match &mut slot.value {
            Value::Exception(exception_obj) => {
                exception_obj
                    .attrs
                    .borrow_mut()
                    .insert("args".to_string(), args_value);
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance_for_vm(vm_ptr, instance) {
                    context.set_error("PyException_SetArgs expected exception object");
                    return;
                }
                let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                    context.set_error("PyException_SetArgs encountered invalid instance");
                    return;
                };
                instance_data.attrs.insert("args".to_string(), args_value);
            }
            _ => {
                context.set_error("PyException_SetArgs expected exception object");
                return;
            }
        }
        if let Some(raw_ptr) = context.cpython_ptr_by_handle.get(&handle).copied()
            && context.owns_cpython_allocation_ptr(raw_ptr)
        {
            // SAFETY: `raw_ptr` is owned base-exception-compatible storage for this handle.
            unsafe {
                let raw_exception = raw_ptr.cast::<CpythonBaseExceptionCompatObject>();
                (*raw_exception).args = args_ptr;
            }
        }
    })
    .map_err(|err| cpython_set_error(err));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_SetCause(exception: *mut c_void, cause: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let vm_ptr = context.vm;
        let Some(handle) = context.cpython_handle_from_ptr(exception) else {
            context.set_error("PyException_SetCause received unknown exception pointer");
            return;
        };
        let cause_value = if cause.is_null() {
            None
        } else {
            let Some(raw_value) = context.cpython_value_from_ptr_or_proxy(cause) else {
                context.set_error("PyException_SetCause received unknown cause pointer");
                return;
            };
            if vm_ptr.is_null() {
                context.set_error("PyException_SetCause missing VM context");
                return;
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *vm_ptr };
            match vm.normalize_exception_value(raw_value) {
                Ok(Value::Exception(exc)) => Some(Value::Exception(exc)),
                Ok(_) => {
                    context.set_error("PyException_SetCause expected exception cause");
                    return;
                }
                Err(err) => {
                    context.set_error(err.message);
                    return;
                }
            }
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyException_SetCause exception handle is not available");
            return;
        };
        match &mut slot.value {
            Value::Exception(exception_obj) => {
                exception_obj.cause = match cause_value.clone() {
                    Some(Value::Exception(cause_obj)) => Some(cause_obj),
                    Some(_) => {
                        context.set_error("PyException_SetCause expected exception cause");
                        return;
                    }
                    None => None,
                };
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance_for_vm(vm_ptr, instance) {
                    context.set_error("PyException_SetCause expected exception object");
                    return;
                }
                let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                    context.set_error("PyException_SetCause encountered invalid instance");
                    return;
                };
                let stored = cause_value.unwrap_or(Value::None);
                instance_data.attrs.insert("__cause__".to_string(), stored);
            }
            _ => {
                context.set_error("PyException_SetCause expected exception object");
                return;
            }
        }
    })
    .map_err(|err| cpython_set_error(err));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_SetContext(
    exception: *mut c_void,
    context_value: *mut c_void,
) {
    let _ = with_active_cpython_context_mut(|context| {
        let vm_ptr = context.vm;
        let Some(handle) = context.cpython_handle_from_ptr(exception) else {
            context.set_error("PyException_SetContext received unknown exception pointer");
            return;
        };
        let context_obj = if context_value.is_null() {
            None
        } else {
            let Some(raw_value) = context.cpython_value_from_ptr_or_proxy(context_value) else {
                context.set_error("PyException_SetContext received unknown context pointer");
                return;
            };
            if vm_ptr.is_null() {
                context.set_error("PyException_SetContext missing VM context");
                return;
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *vm_ptr };
            match vm.normalize_exception_value(raw_value) {
                Ok(Value::Exception(exc)) => Some(Value::Exception(exc)),
                Ok(_) => {
                    context.set_error("PyException_SetContext expected exception context");
                    return;
                }
                Err(err) => {
                    context.set_error(err.message);
                    return;
                }
            }
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyException_SetContext exception handle is not available");
            return;
        };
        match &mut slot.value {
            Value::Exception(exception_obj) => {
                exception_obj.context = match context_obj.clone() {
                    Some(Value::Exception(context_exception)) => Some(context_exception),
                    Some(_) => {
                        context.set_error("PyException_SetContext expected exception context");
                        return;
                    }
                    None => None,
                };
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance_for_vm(vm_ptr, instance) {
                    context.set_error("PyException_SetContext expected exception object");
                    return;
                }
                let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                    context.set_error("PyException_SetContext encountered invalid instance");
                    return;
                };
                let stored = context_obj.unwrap_or(Value::None);
                instance_data
                    .attrs
                    .insert("__context__".to_string(), stored);
            }
            _ => {
                context.set_error("PyException_SetContext expected exception object");
                return;
            }
        }
    })
    .map_err(|err| cpython_set_error(err));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_SetTraceback(exception: *mut c_void, traceback: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let vm_ptr = context.vm;
        let Some(handle) = context.cpython_handle_from_ptr(exception) else {
            context.set_error("PyException_SetTraceback received unknown exception pointer");
            return;
        };
        let traceback_value = if traceback.is_null() {
            Value::None
        } else {
            match context.cpython_value_from_ptr_or_proxy(traceback) {
                Some(value) => value,
                None => {
                    context
                        .set_error("PyException_SetTraceback received unknown traceback pointer");
                    return;
                }
            }
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyException_SetTraceback exception handle is not available");
            return;
        };
        match &mut slot.value {
            Value::Exception(exception_obj) => {
                let mut attrs = exception_obj.attrs.borrow_mut();
                attrs.insert("__traceback__".to_string(), traceback_value.clone());
                attrs.insert("exc_traceback".to_string(), traceback_value);
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance_for_vm(vm_ptr, instance) {
                    context.set_error("PyException_SetTraceback expected exception object");
                    return;
                }
                let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                    context.set_error("PyException_SetTraceback encountered invalid instance");
                    return;
                };
                instance_data
                    .attrs
                    .insert("__traceback__".to_string(), traceback_value.clone());
                instance_data
                    .attrs
                    .insert("exc_traceback".to_string(), traceback_value);
            }
            _ => {
                context.set_error("PyException_SetTraceback expected exception object");
                return;
            }
        }
    })
    .map_err(|err| cpython_set_error(err));
}
