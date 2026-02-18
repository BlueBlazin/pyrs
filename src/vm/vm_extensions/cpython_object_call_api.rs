use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_void};
use std::rc::Rc;

use crate::bytecode::CodeObject;
use crate::runtime::{BoundMethod, BuiltinFunction, Object, Value};

use super::{
    CPY_PROXY_GET_ITER_ACTIVE, CpythonMappingMethods, CpythonNumberMethods, CpythonObjectHead,
    CpythonSequenceMethods, CpythonTypeObject, InternalCallOutcome, ModuleCapiContext, Py_DecRef,
    Py_XIncRef, PyDict_Clear, PyErr_BadInternalCall, PyErr_Clear, PyObject_GetAttr,
    PyObject_GetAttrString, PyObject_HasAttrStringWithError, PyTuple_New, PyTuple_SetItem,
    PyTuple_Size, PyUnicode_InternFromString, c_name_to_string, cpython_call_builtin,
    cpython_call_object, cpython_keyword_args_from_dict_object, cpython_new_ptr_for_value,
    cpython_objref_from_value, cpython_positional_args_from_tuple_object,
    cpython_resolve_vectorcall, cpython_set_active_context, cpython_set_error,
    cpython_unicode_text_from_value, cpython_value_debug_tag, cpython_value_from_ptr, is_truthy,
    with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsTrue(object: *mut c_void) -> i32 {
    match with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_IsTrue missing VM context");
            return -1;
        }
        if let Some(value) = context.cpython_value_from_ptr(object) {
            return if is_truthy(&value) { 1 } else { 0 };
        }
        if !object.is_null()
            && (object as usize) >= 0x1_0000_0000
            && (object as usize) % std::mem::align_of::<usize>() == 0
        {
            // SAFETY: pointer shape is validated above; slot calls follow CPython slot ABI.
            unsafe {
                let head = object.cast::<CpythonObjectHead>();
                if let Some(head) = head.as_ref() {
                    let type_ptr = head.ob_type.cast::<CpythonTypeObject>();
                    if !type_ptr.is_null() {
                        let as_number = (*type_ptr).tp_as_number.cast::<CpythonNumberMethods>();
                        if !as_number.is_null() {
                            let nb_bool = (*as_number).nb_bool;
                            if !nb_bool.is_null() {
                                let bool_fn: unsafe extern "C" fn(*mut c_void) -> i32 =
                                    std::mem::transmute(nb_bool);
                                let result = bool_fn(object);
                                return if result < 0 {
                                    -1
                                } else if result == 0 {
                                    0
                                } else {
                                    1
                                };
                            }
                        }
                        let as_mapping = (*type_ptr).tp_as_mapping.cast::<CpythonMappingMethods>();
                        if !as_mapping.is_null() {
                            let mp_length = (*as_mapping).mp_length;
                            if !mp_length.is_null() {
                                let len_fn: unsafe extern "C" fn(*mut c_void) -> isize =
                                    std::mem::transmute(mp_length);
                                let result = len_fn(object);
                                return if result < 0 {
                                    -1
                                } else if result == 0 {
                                    0
                                } else {
                                    1
                                };
                            }
                        }
                        let as_sequence =
                            (*type_ptr).tp_as_sequence.cast::<CpythonSequenceMethods>();
                        if !as_sequence.is_null() {
                            let sq_length = (*as_sequence).sq_length;
                            if !sq_length.is_null() {
                                let len_fn: unsafe extern "C" fn(*mut c_void) -> isize =
                                    std::mem::transmute(sq_length);
                                let result = len_fn(object);
                                return if result < 0 {
                                    -1
                                } else if result == 0 {
                                    0
                                } else {
                                    1
                                };
                            }
                        }
                        return 1;
                    }
                }
            }
        }
        if let Some(value) = context.cpython_value_from_ptr_or_proxy(object) {
            return if is_truthy(&value) { 1 } else { 0 };
        }
        context.set_error("PyObject_IsTrue received unknown object pointer");
        -1
    }) {
        Ok(result) => result,
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Not(object: *mut c_void) -> i32 {
    let truthy = unsafe { PyObject_IsTrue(object) };
    if truthy < 0 {
        -1
    } else if truthy == 0 {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Str(object: *mut c_void) -> *mut c_void {
    if !object.is_null() {
        let str_slot_result = with_active_cpython_context_mut(|context| {
            let type_ptr = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if type_ptr.is_null() {
                return None;
            }
            // SAFETY: `type_ptr` is derived from a valid object header and `tp_str` is read-only.
            let slot = unsafe { (*type_ptr).tp_str };
            if slot.is_null() {
                return None;
            }
            let previous_context = cpython_set_active_context(std::ptr::addr_of_mut!(*context));
            // SAFETY: `tp_str` uses unary slot signature (`reprfunc`) for this type.
            let str_fn: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
                unsafe { std::mem::transmute(slot) };
            let rendered = unsafe { str_fn(object) };
            cpython_set_active_context(previous_context);
            Some(rendered)
        });
        match str_slot_result {
            Ok(Some(rendered)) if !rendered.is_null() => return rendered,
            Ok(Some(_)) => return std::ptr::null_mut(),
            Ok(None) => {}
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    }

    let value = match with_active_cpython_context_mut(|context| {
        context.cpython_value_from_ptr_or_proxy(object)
    }) {
        Ok(Some(value)) => value,
        Ok(None) => {
            if std::env::var_os("PYRS_TRACE_UNKNOWN_PTR").is_some() {
                eprintln!("[cpy-unknown-ptr] PyObject_Str object={:p}", object);
            }
            cpython_set_error("unknown PyObject pointer");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::Str, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Repr(object: *mut c_void) -> *mut c_void {
    if !object.is_null() {
        let repr_slot_result = with_active_cpython_context_mut(|context| {
            let type_ptr = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if type_ptr.is_null() {
                return None;
            }
            // SAFETY: `type_ptr` is derived from a valid object header and `tp_repr` is read-only.
            let slot = unsafe { (*type_ptr).tp_repr };
            if slot.is_null() {
                return None;
            }
            let previous_context = cpython_set_active_context(std::ptr::addr_of_mut!(*context));
            // SAFETY: `tp_repr` uses unary slot signature (`reprfunc`) for this type.
            let repr_fn: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
                unsafe { std::mem::transmute(slot) };
            let rendered = unsafe { repr_fn(object) };
            cpython_set_active_context(previous_context);
            Some(rendered)
        });
        match repr_slot_result {
            Ok(Some(rendered)) if !rendered.is_null() => return rendered,
            Ok(Some(_)) => return std::ptr::null_mut(),
            Ok(None) => {}
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    }

    let value = match with_active_cpython_context_mut(|context| {
        context.cpython_value_from_ptr_or_proxy(object)
    }) {
        Ok(Some(value)) => value,
        Ok(None) => {
            if std::env::var_os("PYRS_TRACE_UNKNOWN_PTR").is_some() {
                eprintln!("[cpy-unknown-ptr] PyObject_Repr object={:p}", object);
            }
            cpython_set_error("unknown PyObject pointer");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::Repr, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_ASCII(object: *mut c_void) -> *mut c_void {
    let value = match with_active_cpython_context_mut(|context| {
        context.cpython_value_from_ptr_or_proxy(object)
    }) {
        Ok(Some(value)) => value,
        Ok(None) => {
            if std::env::var_os("PYRS_TRACE_UNKNOWN_PTR").is_some() {
                eprintln!("[cpy-unknown-ptr] PyObject_ASCII object={:p}", object);
            }
            cpython_set_error("unknown PyObject pointer");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::Ascii, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Dir(object: *mut c_void) -> *mut c_void {
    let args = if object.is_null() {
        Vec::new()
    } else {
        let value = match cpython_value_from_ptr(object) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
        vec![value]
    };
    match cpython_call_builtin(BuiltinFunction::Dir, args) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Bytes(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::Bytes, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Format(
    object: *mut c_void,
    format_spec: *mut c_void,
) -> *mut c_void {
    let object = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let spec = if format_spec.is_null() {
        Value::Str(String::new())
    } else {
        match cpython_value_from_ptr(format_spec) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    match cpython_call_builtin(BuiltinFunction::Format, vec![object, spec]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetIter(object: *mut c_void) -> *mut c_void {
    let object_addr = object as usize;
    let recursion_depth = CPY_PROXY_GET_ITER_ACTIVE.with(|stack| {
        let mut stack = stack.borrow_mut();
        let depth = stack.iter().filter(|entry| **entry == object_addr).count() + 1;
        stack.push(object_addr);
        depth
    });
    if recursion_depth > 64 {
        if let Ok(Some(ptr)) = with_active_cpython_context_mut(|context| {
            if context.vm.is_null() {
                return None;
            }
            let value = context.cpython_value_from_ptr_or_proxy(object)?;
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *context.vm };
            match vm.sequence_iterator_via_getitem(value) {
                Ok(Some(iterator)) => {
                    let iter_ptr = context.alloc_cpython_ptr_for_value(iterator);
                    if iter_ptr.is_null() {
                        None
                    } else {
                        Some(iter_ptr)
                    }
                }
                _ => None,
            }
        }) {
            return ptr;
        }
        if std::env::var_os("PYRS_TRACE_CPY_GETITER_RECURSION").is_some() {
            // SAFETY: best-effort diagnostics for recursion path.
            let (type_ptr, type_name, tp_iter) = unsafe {
                let type_ptr = object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut());
                let type_name = if type_ptr.is_null() {
                    "<null>".to_string()
                } else {
                    c_name_to_string((*type_ptr).tp_name)
                        .unwrap_or_else(|_| "<invalid>".to_string())
                };
                let tp_iter = if type_ptr.is_null() {
                    std::ptr::null_mut()
                } else {
                    (*type_ptr).tp_iter
                };
                (type_ptr, type_name, tp_iter)
            };
            let ownership = with_active_cpython_context_mut(|context| {
                let owned = context.owns_cpython_allocation_ptr(object);
                let known_handle = context.cpython_handle_from_ptr(object).is_some();
                let mapped_value = context.cpython_value_from_ptr(object);
                let mapped = mapped_value
                    .as_ref()
                    .map(cpython_value_debug_tag)
                    .unwrap_or_else(|| "<none>".to_string());
                let specials = if context.vm.is_null() {
                    "getitem=<unknown> iter=<unknown>".to_string()
                } else if let Some(value) = mapped_value {
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    let has_getitem = vm
                        .lookup_bound_special_method(&value, "__getitem__")
                        .ok()
                        .flatten()
                        .is_some();
                    let has_iter = vm
                        .lookup_bound_special_method(&value, "__iter__")
                        .ok()
                        .flatten()
                        .is_some();
                    let has_next = vm
                        .lookup_bound_special_method(&value, "__next__")
                        .ok()
                        .flatten()
                        .is_some();
                    format!("getitem={has_getitem} iter={has_iter} next={has_next}")
                } else {
                    "getitem=false iter=false next=false".to_string()
                };
                format!(
                    "owned={} known_handle={} mapped={mapped} {specials}",
                    owned, known_handle
                )
            })
            .unwrap_or_else(|_| {
                "owned=<unknown> known_handle=<unknown> mapped=<unknown>".to_string()
            });
            eprintln!(
                "[cpy-getiter-recur] object={:p} type={:p} type_name={} tp_iter={:p} pyobject_getiter={:p} tp_iter_is_pyobject_getiter={} {}",
                object,
                type_ptr,
                type_name,
                tp_iter,
                PyObject_GetIter as *const () as *mut c_void,
                tp_iter == (PyObject_GetIter as *const () as *mut c_void),
                ownership
            );
        }
        CPY_PROXY_GET_ITER_ACTIVE.with(|stack| {
            let mut stack = stack.borrow_mut();
            if let Some(last) = stack.last().copied() {
                if last == object_addr {
                    stack.pop();
                    return;
                }
            }
            if let Some(pos) = stack.iter().rposition(|entry| *entry == object_addr) {
                stack.remove(pos);
            }
        });
        cpython_set_error("PyObject_GetIter recursion detected");
        return std::ptr::null_mut();
    }
    let result = with_active_cpython_context_mut(|context| {
        if object.is_null() {
            context.set_error("PyObject_GetIter received null object");
            return std::ptr::null_mut();
        }
        let owned_object = context.owns_cpython_allocation_ptr(object);
        let pinned_owned_object = if context.vm.is_null() {
            false
        } else {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            unsafe {
                (&*context.vm)
                    .extension_pinned_cpython_allocation_set
                    .contains(&(object as usize))
            }
        };
        if !owned_object
            && !pinned_owned_object
            && ModuleCapiContext::is_probable_external_cpython_object_ptr(object)
        {
            // SAFETY: `object` passed probability checks for CPython object layout.
            let type_ptr = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if !type_ptr.is_null() {
                // SAFETY: type pointer is non-null and read-only inspected for slot dispatch.
                let tp_iter_raw = unsafe { (*type_ptr).tp_iter };
                if !tp_iter_raw.is_null() {
                    let tp_iter: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
                        // SAFETY: `tp_iter` follows CPython unary slot ABI.
                        unsafe { std::mem::transmute(tp_iter_raw) };
                    // SAFETY: calling external object's iterator slot.
                    let iter_ptr = unsafe { tp_iter(object) };
                    if !iter_ptr.is_null() {
                        return iter_ptr;
                    }
                    if context.current_error.is_some() || context.last_error.is_some() {
                        return std::ptr::null_mut();
                    }
                    context.set_error("object is not iterable");
                    return std::ptr::null_mut();
                }
            }
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_GetIter received unknown object pointer");
            return std::ptr::null_mut();
        };
        match cpython_call_builtin(BuiltinFunction::Iter, vec![value]) {
            Ok(result) => context.alloc_cpython_ptr_for_value(result),
            Err(err) => {
                context.set_error(err);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    });
    CPY_PROXY_GET_ITER_ACTIVE.with(|stack| {
        let mut stack = stack.borrow_mut();
        if let Some(last) = stack.last().copied() {
            if last == object_addr {
                stack.pop();
                return;
            }
        }
        if let Some(pos) = stack.iter().rposition(|entry| *entry == object_addr) {
            stack.remove(pos);
        }
    });
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyAIter_Check(object: *mut c_void) -> i32 {
    if object.is_null() {
        return 0;
    }
    let status = unsafe { PyObject_HasAttrStringWithError(object, c"__anext__".as_ptr()) };
    if status < 0 {
        unsafe { PyErr_Clear() };
        0
    } else {
        status
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAIter(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_GetAIter missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_GetAIter received unknown object pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let aiter = match vm.builtin_getattr(
            vec![value, Value::Str("__aiter__".to_string())],
            HashMap::new(),
        ) {
            Ok(callable) => callable,
            Err(err) => {
                context.set_error(err.message);
                return std::ptr::null_mut();
            }
        };
        match vm.call_internal(aiter, Vec::new(), HashMap::new()) {
            Ok(InternalCallOutcome::Value(result)) => context.alloc_cpython_ptr_for_value(result),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("PyObject_GetAIter failed")
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
pub unsafe extern "C" fn PyObject_SelfIter(object: *mut c_void) -> *mut c_void {
    unsafe { Py_XIncRef(object) };
    object
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallObject(
    callable: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    let args = match cpython_positional_args_from_tuple_object(args) {
        Ok(args) => args,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_call_object(callable, args, HashMap::new())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Call(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    let args = match cpython_positional_args_from_tuple_object(args) {
        Ok(args) => args,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let kwargs = match cpython_keyword_args_from_dict_object(kwargs) {
        Ok(kwargs) => kwargs,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_call_object(callable, args, kwargs)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallOneArg(
    callable: *mut c_void,
    arg: *mut c_void,
) -> *mut c_void {
    let arg = match cpython_value_from_ptr(arg) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_call_object(callable, vec![arg], HashMap::new())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallNoArgs(callable: *mut c_void) -> *mut c_void {
    cpython_call_object(callable, Vec::new(), HashMap::new())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallFinalizer(self_obj: *mut c_void) {
    if self_obj.is_null() {
        return;
    }
    // SAFETY: caller passes a PyObject-compatible pointer.
    let type_ptr = unsafe {
        self_obj
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return;
    }
    // SAFETY: `type_ptr` points to a type object; tp_finalize follows C-API slot ABI.
    let finalize = unsafe { (*type_ptr).tp_finalize };
    if finalize.is_null() {
        return;
    }
    let finalize_fn: unsafe extern "C" fn(*mut c_void) =
        // SAFETY: tp_finalize slot uses unary-function signature.
        unsafe { std::mem::transmute(finalize) };
    // SAFETY: finalize callback follows C-API signature and accepts the object pointer.
    unsafe { finalize_fn(self_obj) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallFinalizerFromDealloc(self_obj: *mut c_void) -> c_int {
    if self_obj.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe { PyObject_CallFinalizer(self_obj) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_ClearManagedDict(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    // SAFETY: static NUL-terminated attribute name.
    let dict_attr = b"__dict__\0";
    // SAFETY: C-API call expects a valid NUL-terminated name.
    let dict_obj = unsafe { PyObject_GetAttrString(object, dict_attr.as_ptr().cast::<c_char>()) };
    if dict_obj.is_null() {
        unsafe { PyErr_Clear() };
        return;
    }
    // SAFETY: best-effort clear of dict-like attribute payload.
    unsafe {
        let _ = PyDict_Clear(dict_obj);
        Py_DecRef(dict_obj);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_VisitManagedDict(
    object: *mut c_void,
    visitor: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    arg: *mut c_void,
) -> c_int {
    if object.is_null() {
        return 0;
    }
    let Some(visitor_fn) = visitor else {
        return 0;
    };
    // SAFETY: static NUL-terminated attribute name.
    let dict_attr = b"__dict__\0";
    // SAFETY: C-API call expects a valid NUL-terminated name.
    let dict_obj = unsafe { PyObject_GetAttrString(object, dict_attr.as_ptr().cast::<c_char>()) };
    if dict_obj.is_null() {
        unsafe { PyErr_Clear() };
        return 0;
    }
    // SAFETY: callback signature matches visitproc ABI.
    let result = unsafe { visitor_fn(dict_obj, arg) };
    unsafe { Py_DecRef(dict_obj) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnstable_Object_EnableDeferredRefcount(_object: *mut c_void) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMethod_New(function: *mut c_void, self_obj: *mut c_void) -> *mut c_void {
    let trace_pymethod_new = std::env::var_os("PYRS_TRACE_PYMETHOD_NEW").is_some();
    with_active_cpython_context_mut(|context| {
        if function.is_null() || self_obj.is_null() {
            context.set_error("PyMethod_New expected non-null function and self");
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            context.set_error("PyMethod_New missing VM context");
            return std::ptr::null_mut();
        }
        let Some(function_value) = context.cpython_value_from_ptr_or_proxy(function) else {
            context.set_error("PyMethod_New received unknown function pointer");
            return std::ptr::null_mut();
        };
        let Some(self_value) = context.cpython_value_from_ptr_or_proxy(self_obj) else {
            context.set_error("PyMethod_New received unknown self pointer");
            return std::ptr::null_mut();
        };
        if trace_pymethod_new {
            eprintln!(
                "[pymethod-new] function_ptr={:p} function={} self_ptr={:p} self={}",
                function,
                cpython_value_debug_tag(&function_value),
                self_obj,
                cpython_value_debug_tag(&self_value)
            );
        }
        let Some(function_obj) = cpython_objref_from_value(function_value) else {
            context.set_error("PyMethod_New expected object-backed function");
            return std::ptr::null_mut();
        };
        let Some(self_ref) = cpython_objref_from_value(self_value) else {
            context.set_error("PyMethod_New expected object-backed self");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let method_value = vm
            .heap
            .alloc_bound_method(BoundMethod::new(function_obj, self_ref));
        context.alloc_cpython_ptr_for_value(method_value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCode_NewEmpty(
    filename: *const c_char,
    funcname: *const c_char,
    _firstlineno: c_int,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let filename_text = if filename.is_null() {
            "<string>".to_string()
        } else {
            // SAFETY: filename is expected to be a NUL-terminated C string by C-API contract.
            unsafe { c_name_to_string(filename) }.unwrap_or_else(|_| "<string>".to_string())
        };
        let funcname_text = if funcname.is_null() {
            "<module>".to_string()
        } else {
            // SAFETY: funcname is expected to be a NUL-terminated C string by C-API contract.
            unsafe { c_name_to_string(funcname) }.unwrap_or_else(|_| "<module>".to_string())
        };
        let code = CodeObject::new(funcname_text, filename_text);
        let value = Value::Code(Rc::new(code));
        context.alloc_cpython_ptr_for_value(value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnstable_Code_NewWithPosOnlyArgs(
    _argcount: c_int,
    _posonlyargcount: c_int,
    _kwonlyargcount: c_int,
    _nlocals: c_int,
    _stacksize: c_int,
    _flags: c_int,
    _code: *mut c_void,
    _consts: *mut c_void,
    _names: *mut c_void,
    _varnames: *mut c_void,
    _freevars: *mut c_void,
    _cellvars: *mut c_void,
    filename: *mut c_void,
    name: *mut c_void,
    _qualname: *mut c_void,
    _firstlineno: c_int,
    _linetable: *mut c_void,
    _exceptiontable: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let filename_text = context
            .cpython_value_from_ptr_or_proxy(filename)
            .and_then(|value| cpython_unicode_text_from_value(&value))
            .unwrap_or_else(|| "<string>".to_string());
        let name_text = context
            .cpython_value_from_ptr_or_proxy(name)
            .and_then(|value| cpython_unicode_text_from_value(&value))
            .unwrap_or_else(|| "<module>".to_string());
        let code = CodeObject::new(name_text, filename_text);
        context.alloc_cpython_ptr_for_value(Value::Code(Rc::new(code)))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnstable_Code_New(
    argcount: c_int,
    kwonlyargcount: c_int,
    nlocals: c_int,
    stacksize: c_int,
    flags: c_int,
    code: *mut c_void,
    consts: *mut c_void,
    names: *mut c_void,
    varnames: *mut c_void,
    freevars: *mut c_void,
    cellvars: *mut c_void,
    filename: *mut c_void,
    name: *mut c_void,
    qualname: *mut c_void,
    firstlineno: c_int,
    linetable: *mut c_void,
    exceptiontable: *mut c_void,
) -> *mut c_void {
    unsafe {
        PyUnstable_Code_NewWithPosOnlyArgs(
            argcount,
            0,
            kwonlyargcount,
            nlocals,
            stacksize,
            flags,
            code,
            consts,
            names,
            varnames,
            freevars,
            cellvars,
            filename,
            name,
            qualname,
            firstlineno,
            linetable,
            exceptiontable,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyArg_UnpackTuple(
    args: *mut c_void,
    _name: *const c_char,
    min: isize,
    max: isize,
) -> i32 {
    let argc = unsafe { PyTuple_Size(args) };
    if argc < 0 {
        return 0;
    }
    if argc < min || argc > max {
        cpython_set_error("PyArg_UnpackTuple argument count mismatch");
        return 0;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyVectorcall_Call(
    callable: *mut c_void,
    tuple: *mut c_void,
    dict: *mut c_void,
) -> *mut c_void {
    let trace_vectorcall_decode = std::env::var_os("PYRS_TRACE_VECTORCALL_DECODE").is_some();
    with_active_cpython_context_mut(|context| {
        let Some(vectorcall) = (unsafe { cpython_resolve_vectorcall(callable) }) else {
            context.set_error("PyVectorcall_Call target has no vectorcall function");
            return std::ptr::null_mut();
        };
        if context.vm.is_null() {
            context.set_error("PyVectorcall_Call missing VM context");
            return std::ptr::null_mut();
        }

        let mut args_ptrs: Vec<*mut c_void> = Vec::new();
        if !tuple.is_null() {
            let Some(tuple_value) = context.cpython_value_from_ptr(tuple) else {
                context.set_error("PyVectorcall_Call received unknown args tuple");
                return std::ptr::null_mut();
            };
            let Value::Tuple(tuple_obj) = tuple_value else {
                context.set_error("PyVectorcall_Call expected tuple args");
                return std::ptr::null_mut();
            };
            let Object::Tuple(values) = &*tuple_obj.kind() else {
                context.set_error("PyVectorcall_Call tuple storage invalid");
                return std::ptr::null_mut();
            };
            for value in values {
                let ptr = context.alloc_cpython_ptr_for_value(value.clone());
                if ptr.is_null() {
                    context.set_error("PyVectorcall_Call failed to materialize positional arg");
                    return std::ptr::null_mut();
                }
                args_ptrs.push(ptr);
            }
        }

        let positional_count = args_ptrs.len();
        let mut kw_name_ptrs: Vec<*mut c_void> = Vec::new();
        if !dict.is_null() {
            let Some(dict_value) = context.cpython_value_from_ptr(dict) else {
                context.set_error("PyVectorcall_Call received unknown kwargs dict");
                return std::ptr::null_mut();
            };
            let Value::Dict(dict_obj) = dict_value else {
                context.set_error("PyVectorcall_Call expected kwargs dict");
                return std::ptr::null_mut();
            };
            let entries = match &*dict_obj.kind() {
                Object::Dict(entries) => entries.clone(),
                _ => {
                    context.set_error("PyVectorcall_Call kwargs storage invalid");
                    return std::ptr::null_mut();
                }
            };
            for (key, value) in entries {
                let Value::Str(name) = key else {
                    context.set_error("PyVectorcall_Call kwargs must use str keys");
                    return std::ptr::null_mut();
                };
                let c_name = match CString::new(name.as_str()) {
                    Ok(c_name) => c_name,
                    Err(_) => {
                        context
                            .set_error("PyVectorcall_Call keyword name contains interior NUL byte");
                        return std::ptr::null_mut();
                    }
                };
                // SAFETY: C string is NUL-terminated and valid for this call.
                let kw_name_ptr = unsafe { PyUnicode_InternFromString(c_name.as_ptr()) };
                if kw_name_ptr.is_null() {
                    context.set_error("PyVectorcall_Call failed to intern keyword name");
                    return std::ptr::null_mut();
                }
                kw_name_ptrs.push(kw_name_ptr);
                let ptr = context.alloc_cpython_ptr_for_value(value);
                if ptr.is_null() {
                    context.set_error("PyVectorcall_Call failed to materialize keyword arg");
                    return std::ptr::null_mut();
                }
                args_ptrs.push(ptr);
            }
        }

        let keyword_count = kw_name_ptrs.len();
        let has_keywords = keyword_count > 0;
        let kwnames_ptr = if !has_keywords {
            std::ptr::null_mut()
        } else {
            // SAFETY: tuple allocation follows CPython tuple ABI.
            let tuple = unsafe { PyTuple_New(kw_name_ptrs.len() as isize) };
            if tuple.is_null() {
                std::ptr::null_mut()
            } else {
                for (index, name_ptr) in kw_name_ptrs.into_iter().enumerate() {
                    // SAFETY: tuple is newly allocated and index is in-bounds.
                    let status = unsafe { PyTuple_SetItem(tuple, index as isize, name_ptr) };
                    if status != 0 {
                        // SAFETY: tuple owns any already-inserted references.
                        unsafe { Py_DecRef(tuple) };
                        context
                            .set_error("PyVectorcall_Call failed to populate keyword names tuple");
                        return std::ptr::null_mut();
                    }
                }
                tuple
            }
        };
        if has_keywords && kwnames_ptr.is_null() {
            context.set_error("PyVectorcall_Call failed to build keyword names tuple");
            return std::ptr::null_mut();
        }
        if trace_vectorcall_decode {
            let callable_desc = context
                .cpython_value_from_ptr_or_proxy(callable)
                .map(|value| cpython_value_debug_tag(&value))
                .unwrap_or_else(|| format!("<callable:{callable:p}>"));
            let kw_name_desc = if kwnames_ptr.is_null() {
                String::new()
            } else {
                context
                    .cpython_value_from_ptr_or_proxy(kwnames_ptr)
                    .map(|value| match value {
                        Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                            Object::Tuple(values) => values
                                .iter()
                                .map(cpython_value_debug_tag)
                                .collect::<Vec<_>>()
                                .join(", "),
                            _ => "<invalid-kwnames-storage>".to_string(),
                        },
                        other => format!("<non-tuple:{}>", cpython_value_debug_tag(&other)),
                    })
                    .unwrap_or_else(|| "<unknown>".to_string())
            };
            eprintln!(
                "[vectorcall-call] callable={} positional_count={} kw_count={} kwnames=[{}]",
                callable_desc, positional_count, keyword_count, kw_name_desc
            );
        }
        let args_ptr = if args_ptrs.is_empty() {
            std::ptr::null()
        } else {
            args_ptrs.as_ptr()
        };
        let result = unsafe { vectorcall(callable, args_ptr, positional_count, kwnames_ptr) };
        if !kwnames_ptr.is_null() {
            // SAFETY: kwnames tuple is call-local materialization and no longer needed after call.
            unsafe { Py_DecRef(kwnames_ptr) };
        }
        result
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Vectorcall(
    callable: *mut c_void,
    args: *const *mut c_void,
    nargsf: usize,
    kwnames: *mut c_void,
) -> *mut c_void {
    let trace_vectorcall_decode = std::env::var_os("PYRS_TRACE_VECTORCALL_DECODE").is_some();
    if trace_vectorcall_decode {
        let positional_count = nargsf & (usize::MAX >> 1);
        let kw_count = if kwnames.is_null() {
            0usize
        } else {
            unsafe { PyTuple_Size(kwnames) }.max(0) as usize
        };
        let callable_desc = cpython_value_from_ptr(callable)
            .map(|value| cpython_value_debug_tag(&value))
            .unwrap_or_else(|_| format!("<callable:{callable:p}>"));
        let kw_name_desc = if kwnames.is_null() {
            String::new()
        } else {
            match cpython_value_from_ptr(kwnames) {
                Ok(Value::Tuple(tuple_obj)) => {
                    if let Object::Tuple(values) = &*tuple_obj.kind() {
                        values
                            .iter()
                            .map(cpython_value_debug_tag)
                            .collect::<Vec<_>>()
                            .join(", ")
                    } else {
                        "<invalid-kwnames-storage>".to_string()
                    }
                }
                Ok(other) => format!("<non-tuple:{}>", cpython_value_debug_tag(&other)),
                Err(err) => format!("<decode-error:{err}>"),
            }
        };
        eprintln!(
            "[vectorcall-entry] callable={} positional_count={} kw_count={} kwnames=[{}]",
            callable_desc, positional_count, kw_count, kw_name_desc
        );
    }
    if let Some(vectorcall) = unsafe { cpython_resolve_vectorcall(callable) } {
        return unsafe { vectorcall(callable, args, nargsf, kwnames) };
    }
    let positional_count = nargsf & (usize::MAX >> 1);
    let kw_count = if kwnames.is_null() {
        0usize
    } else {
        unsafe { PyTuple_Size(kwnames) }.max(0) as usize
    };
    let total_count = positional_count.saturating_add(kw_count);
    let mut values = Vec::with_capacity(total_count);
    if total_count > 0 {
        if args.is_null() {
            cpython_set_error("PyObject_Vectorcall received null args with non-zero nargsf");
            return std::ptr::null_mut();
        }
        for idx in 0..total_count {
            // SAFETY: caller promises args has at least total_count entries.
            let ptr = unsafe { *args.add(idx) };
            let value = match cpython_value_from_ptr(ptr) {
                Ok(value) => value,
                Err(err) => {
                    let proxied = with_active_cpython_context_mut(|context| {
                        if context.owns_cpython_allocation_ptr(ptr) {
                            return None;
                        }
                        if ModuleCapiContext::is_probable_external_cpython_object_ptr(ptr) {
                            // Restrict proxy-materialization in vectorcall argument decoding to
                            // known scientific-stack object families for now; random stale/raw
                            // pointers should fail closed.
                            // SAFETY: probability guard above validated object/type pointer layout.
                            let type_name = unsafe {
                                let type_ptr = ptr
                                    .cast::<CpythonObjectHead>()
                                    .as_ref()
                                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                                    .unwrap_or(std::ptr::null_mut());
                                if type_ptr.is_null() {
                                    String::new()
                                } else {
                                    c_name_to_string((*type_ptr).tp_name).unwrap_or_default()
                                }
                            };
                            if !type_name.starts_with("numpy.") {
                                return None;
                            }
                            return context.cpython_value_from_ptr_or_proxy(ptr);
                        }
                        None
                    })
                    .ok()
                    .flatten();
                    if let Some(value) = proxied {
                        values.push(value);
                        continue;
                    }
                    let detail = with_active_cpython_context_mut(|context| {
                        let owned = context.owns_cpython_allocation_ptr(ptr);
                        let known_handle = context.cpython_handle_from_ptr(ptr).is_some();
                        let mapped_value = if context.vm.is_null() {
                            false
                        } else {
                            // SAFETY: VM pointer is valid for active context lifetime.
                            let vm = unsafe { &*context.vm };
                            vm.extension_cpython_ptr_values.contains_key(&(ptr as usize))
                        };
                        // SAFETY: best-effort pointer diagnostics; guarded for null/invalid headers.
                        let (type_ptr, type_name) = unsafe {
                            let head = ptr.cast::<CpythonObjectHead>();
                            let type_ptr = head
                                .as_ref()
                                .map(|h| h.ob_type.cast::<CpythonTypeObject>())
                                .unwrap_or(std::ptr::null_mut());
                            if type_ptr.is_null() {
                                (std::ptr::null_mut(), "<null>".to_string())
                            } else {
                                (
                                    type_ptr,
                                    c_name_to_string((*type_ptr).tp_name)
                                        .unwrap_or_else(|_| "<invalid>".to_string()),
                                )
                            }
                        };
                        format!(
                            "ptr={:p} owned={} known_handle={} mapped_value={} type={:p} type_name={}",
                            ptr, owned, known_handle, mapped_value, type_ptr, type_name
                        )
                    })
                    .unwrap_or_else(|_| format!("ptr={:p}", ptr));
                    cpython_set_error(format!("{err}; {detail}"));
                    return std::ptr::null_mut();
                }
            };
            values.push(value);
        }
    }
    let mut kwargs = HashMap::new();
    if kw_count > 0 {
        let kw_tuple = match cpython_value_from_ptr(kwnames) {
            Ok(Value::Tuple(tuple_obj)) => tuple_obj,
            Ok(_) => {
                cpython_set_error("PyObject_Vectorcall expected tuple keyword names");
                return std::ptr::null_mut();
            }
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
        let Object::Tuple(names) = &*kw_tuple.kind() else {
            cpython_set_error("PyObject_Vectorcall keyword tuple storage invalid");
            return std::ptr::null_mut();
        };
        if names.len() != kw_count {
            cpython_set_error("PyObject_Vectorcall keyword tuple length mismatch");
            return std::ptr::null_mut();
        }
        for (offset, name_value) in names.iter().enumerate() {
            let Value::Str(name) = name_value else {
                cpython_set_error("PyObject_Vectorcall keyword names must be str");
                return std::ptr::null_mut();
            };
            let value_index = positional_count + offset;
            let Some(value) = values.get(value_index) else {
                cpython_set_error("PyObject_Vectorcall keyword value missing");
                return std::ptr::null_mut();
            };
            kwargs.insert(name.clone(), value.clone());
        }
        values.truncate(positional_count);
    }
    if trace_vectorcall_decode {
        let callable_desc = cpython_value_from_ptr(callable)
            .map(|value| cpython_value_debug_tag(&value))
            .unwrap_or_else(|_| format!("<callable:{callable:p}>"));
        let arg_desc = values
            .iter()
            .map(cpython_value_debug_tag)
            .collect::<Vec<_>>()
            .join(", ");
        let mut kw_desc = kwargs
            .iter()
            .map(|(name, value)| format!("{name}={}", cpython_value_debug_tag(value)))
            .collect::<Vec<_>>();
        kw_desc.sort();
        eprintln!(
            "[vectorcall-decode] callable={} positional={} kwargs=[{}]",
            callable_desc,
            arg_desc,
            kw_desc.join(", ")
        );
    }
    cpython_call_object(callable, values, kwargs)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_VectorcallDict(
    callable: *mut c_void,
    args: *const *mut c_void,
    nargsf: usize,
    kwargs: *mut c_void,
) -> *mut c_void {
    let positional_count = nargsf & (usize::MAX >> 1);
    if positional_count > 0 && args.is_null() {
        cpython_set_error("PyObject_VectorcallDict received null args with non-zero nargsf");
        return std::ptr::null_mut();
    }

    with_active_cpython_context_mut(|context| {
        let mut positional_values = Vec::with_capacity(positional_count);
        for index in 0..positional_count {
            // SAFETY: caller guarantees at least positional_count entries.
            let arg_ptr = unsafe { *args.add(index) };
            let Some(value) = context.cpython_value_from_ptr_or_proxy(arg_ptr) else {
                context.set_error("PyObject_VectorcallDict received unknown positional arg");
                return std::ptr::null_mut();
            };
            positional_values.push(value);
        }

        let kwargs_entries: Vec<(Value, Value)> = if kwargs.is_null() {
            Vec::new()
        } else {
            let Some(kwargs_value) = context.cpython_value_from_ptr_or_proxy(kwargs) else {
                context.set_error("PyObject_VectorcallDict received unknown kwargs dict");
                return std::ptr::null_mut();
            };
            let Value::Dict(kwargs_dict) = kwargs_value else {
                context.set_error("PyObject_VectorcallDict expected kwargs dict");
                return std::ptr::null_mut();
            };
            match &*kwargs_dict.kind() {
                Object::Dict(entries) => entries.to_vec(),
                _ => {
                    context.set_error("PyObject_VectorcallDict kwargs dict storage invalid");
                    return std::ptr::null_mut();
                }
            }
        };

        if let Some(vectorcall) = unsafe { cpython_resolve_vectorcall(callable) } {
            let mut arg_ptrs = Vec::with_capacity(positional_values.len() + kwargs_entries.len());
            for value in &positional_values {
                let ptr = context.alloc_cpython_ptr_for_value(value.clone());
                if ptr.is_null() {
                    context
                        .set_error("PyObject_VectorcallDict failed to materialize positional arg");
                    return std::ptr::null_mut();
                }
                arg_ptrs.push(ptr);
            }

            let mut kw_name_ptrs = Vec::with_capacity(kwargs_entries.len());
            for (key, value) in kwargs_entries {
                let Value::Str(name) = key else {
                    context.set_error("PyObject_VectorcallDict kwargs must use str keys");
                    return std::ptr::null_mut();
                };
                let c_name = match CString::new(name.as_str()) {
                    Ok(c_name) => c_name,
                    Err(_) => {
                        context.set_error(
                            "PyObject_VectorcallDict keyword name contains interior NUL",
                        );
                        return std::ptr::null_mut();
                    }
                };
                // SAFETY: C string is NUL-terminated and valid for this call.
                let kw_name_ptr = unsafe { PyUnicode_InternFromString(c_name.as_ptr()) };
                if kw_name_ptr.is_null() {
                    context.set_error("PyObject_VectorcallDict failed to intern keyword name");
                    return std::ptr::null_mut();
                }
                kw_name_ptrs.push(kw_name_ptr);
                let value_ptr = context.alloc_cpython_ptr_for_value(value);
                if value_ptr.is_null() {
                    context
                        .set_error("PyObject_VectorcallDict failed to materialize keyword value");
                    return std::ptr::null_mut();
                }
                arg_ptrs.push(value_ptr);
            }

            let kwnames_ptr = if kw_name_ptrs.is_empty() {
                std::ptr::null_mut()
            } else {
                // SAFETY: tuple allocation follows CPython tuple ABI.
                let tuple = unsafe { PyTuple_New(kw_name_ptrs.len() as isize) };
                if tuple.is_null() {
                    context.set_error("PyObject_VectorcallDict failed to allocate keyword tuple");
                    return std::ptr::null_mut();
                }
                for (index, name_ptr) in kw_name_ptrs.into_iter().enumerate() {
                    // SAFETY: tuple is newly allocated and index is in-bounds.
                    let status = unsafe { PyTuple_SetItem(tuple, index as isize, name_ptr) };
                    if status != 0 {
                        // SAFETY: tuple owns any already inserted references.
                        unsafe { Py_DecRef(tuple) };
                        context.set_error("PyObject_VectorcallDict failed to build keyword tuple");
                        return std::ptr::null_mut();
                    }
                }
                tuple
            };
            let args_ptr = if arg_ptrs.is_empty() {
                std::ptr::null()
            } else {
                arg_ptrs.as_ptr()
            };
            let flag_bits = nargsf & !(usize::MAX >> 1);
            let result = unsafe {
                vectorcall(
                    callable,
                    args_ptr,
                    positional_values.len() | flag_bits,
                    kwnames_ptr,
                )
            };
            if !kwnames_ptr.is_null() {
                // SAFETY: kwnames tuple is call-local materialization and no longer needed.
                unsafe { Py_DecRef(kwnames_ptr) };
            }
            return result;
        }

        let mut kwargs_map = HashMap::new();
        for (key, value) in kwargs_entries {
            let Value::Str(name) = key else {
                context.set_error("PyObject_VectorcallDict kwargs must use str keys");
                return std::ptr::null_mut();
            };
            kwargs_map.insert(name, value);
        }
        cpython_call_object(callable, positional_values, kwargs_map)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_VectorcallMethod(
    name: *mut c_void,
    args: *const *mut c_void,
    nargsf: usize,
    kwnames: *mut c_void,
) -> *mut c_void {
    if args.is_null() || nargsf == 0 {
        cpython_set_error("PyObject_VectorcallMethod requires self arg");
        return std::ptr::null_mut();
    }
    // SAFETY: caller guarantees at least one arg pointer.
    let self_obj = unsafe { *args };
    let method = unsafe { PyObject_GetAttr(self_obj, name) };
    if method.is_null() {
        return std::ptr::null_mut();
    }
    let remaining = nargsf.saturating_sub(1);
    let result = unsafe { PyObject_Vectorcall(method, args.add(1), remaining, kwnames) };
    unsafe { Py_DecRef(method) };
    result
}
