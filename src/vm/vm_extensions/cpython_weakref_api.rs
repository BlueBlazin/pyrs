use std::collections::HashMap;
use std::ffi::{c_int, c_void};

use crate::runtime::{BuiltinFunction, NativeMethodKind, Object, Value};

use super::{
    _Py_NoneStruct, CpythonObjectHead, ModuleCapiContext, Py_DecRef, PyCallable_Check,
    PyErr_BadInternalCall, PyExc_TypeError, cpython_call_internal_in_context, cpython_set_error,
    cpython_set_typed_error, with_active_cpython_context_mut,
};

fn cpython_is_runtime_weakref_ref(value: &Value) -> bool {
    if let Value::Instance(instance_obj) = value
        && let Object::Instance(instance_data) = &*instance_obj.kind()
    {
        return matches!(
            instance_data.attrs.get("__pyrs_weakref_ref__"),
            Some(Value::Bool(true))
        );
    }
    if let Value::BoundMethod(bound_obj) = value
        && let Object::BoundMethod(bound_method) = &*bound_obj.kind()
        && let Object::NativeMethod(native_method) = &*bound_method.function.kind()
    {
        if matches!(
            native_method.kind,
            NativeMethodKind::Builtin(BuiltinFunction::WeakRefRef)
        ) && let Object::Module(module_data) = &*bound_method.receiver.kind()
        {
            return matches!(
                module_data.globals.get("__pyrs_weakref_ref__"),
                Some(Value::Bool(true))
            );
        }
        if matches!(
            native_method.kind,
            NativeMethodKind::Builtin(BuiltinFunction::WeakRefRefCall)
        ) && let Object::Instance(instance_data) = &*bound_method.receiver.kind()
        {
            return matches!(
                instance_data.attrs.get("__pyrs_weakref_ref__"),
                Some(Value::Bool(true))
            );
        }
    }
    false
}

fn cpython_weakref_target_from_value(
    context: &mut ModuleCapiContext,
    weakref_value: Value,
) -> Result<Option<Value>, String> {
    if !cpython_is_runtime_weakref_ref(&weakref_value) {
        return Err("expected a weakref".to_string());
    }
    match cpython_call_internal_in_context(context, weakref_value, Vec::new(), HashMap::new()) {
        Ok(Value::None) => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(err) => Err(err),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyWeakref_NewRef(ob: *mut c_void, callback: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if ob.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        }
        let Some(target_value) = context.cpython_value_from_ptr_or_proxy(ob) else {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        };

        let callback_value = if callback.is_null() {
            Value::None
        } else {
            let Some(value) = context.cpython_value_from_ptr_or_proxy(callback) else {
                unsafe { PyErr_BadInternalCall() };
                return std::ptr::null_mut();
            };
            value
        };
        if callback.is_null() {
            // use implicit None
        } else if !matches!(callback_value, Value::None) {
            let callback_ptr = context.alloc_cpython_ptr_for_value(callback_value.clone());
            let is_callable = unsafe { PyCallable_Check(callback_ptr) != 0 };
            unsafe { Py_DecRef(callback_ptr) };
            if !is_callable {
                cpython_set_typed_error(
                    unsafe { PyExc_TypeError },
                    "weakref callback must be callable or None",
                );
                return std::ptr::null_mut();
            }
        }

        let created = match cpython_call_internal_in_context(
            context,
            Value::Builtin(BuiltinFunction::WeakRefRef),
            vec![target_value.clone(), callback_value],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        if !cpython_is_runtime_weakref_ref(&created) {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "cannot create weak reference to object",
            );
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(created)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyWeakref_NewProxy(ob: *mut c_void, callback: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if ob.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        }
        let Some(target_value) = context.cpython_value_from_ptr_or_proxy(ob) else {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        };
        let callback_value = if callback.is_null() {
            Value::None
        } else {
            let Some(value) = context.cpython_value_from_ptr_or_proxy(callback) else {
                unsafe { PyErr_BadInternalCall() };
                return std::ptr::null_mut();
            };
            value
        };
        if callback.is_null() {
            // use implicit None
        } else if !matches!(callback_value, Value::None) {
            let callback_ptr = context.alloc_cpython_ptr_for_value(callback_value.clone());
            let is_callable = unsafe { PyCallable_Check(callback_ptr) != 0 };
            unsafe { Py_DecRef(callback_ptr) };
            if !is_callable {
                cpython_set_typed_error(
                    unsafe { PyExc_TypeError },
                    "weakref callback must be callable or None",
                );
                return std::ptr::null_mut();
            }
        }

        let probe = match cpython_call_internal_in_context(
            context,
            Value::Builtin(BuiltinFunction::WeakRefRef),
            vec![target_value.clone(), Value::None],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        if !cpython_is_runtime_weakref_ref(&probe) {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "cannot create weak reference to object",
            );
            return std::ptr::null_mut();
        }

        match cpython_call_internal_in_context(
            context,
            Value::Builtin(BuiltinFunction::WeakRefProxy),
            vec![target_value, callback_value],
            HashMap::new(),
        ) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyWeakref_GetRef(ref_obj: *mut c_void, pobj: *mut *mut c_void) -> c_int {
    if pobj.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    // SAFETY: caller provided output pointer.
    unsafe {
        *pobj = std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if ref_obj.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        }
        let Some(ref_value) = context.cpython_value_from_ptr_or_proxy(ref_obj) else {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        };
        let target = match cpython_weakref_target_from_value(context, ref_value) {
            Ok(value) => value,
            Err(message) => {
                cpython_set_typed_error(unsafe { PyExc_TypeError }, message);
                return -1;
            }
        };
        match target {
            None => 0,
            Some(value) => {
                let target_ptr = context.alloc_cpython_ptr_for_value(value);
                // SAFETY: caller-provided output pointer is writable.
                unsafe {
                    *pobj = target_ptr;
                }
                1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyWeakref_GetObject(ref_obj: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if ref_obj.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        }
        let Some(ref_value) = context.cpython_value_from_ptr_or_proxy(ref_obj) else {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        };
        let target = match cpython_weakref_target_from_value(context, ref_value) {
            Ok(value) => value,
            Err(_) => {
                unsafe { PyErr_BadInternalCall() };
                return std::ptr::null_mut();
            }
        };
        match target {
            None => std::ptr::addr_of_mut!(_Py_NoneStruct).cast(),
            Some(value) => {
                let ptr = context.alloc_cpython_ptr_for_value(value);
                unsafe { Py_DecRef(ptr) };
                ptr
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_ClearWeakRefs(object: *mut c_void) {
    with_active_cpython_context_mut(|context| {
        if object.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return;
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            unsafe { PyErr_BadInternalCall() };
            return;
        };
        let Some(object_id) = cpython_identity_object_id(&value) else {
            unsafe { PyErr_BadInternalCall() };
            return;
        };
        if context.vm.is_null() {
            context.set_error("PyObject_ClearWeakRefs missing VM context");
            return;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        // Match CPython's deallocation-only precondition for compat-owned objects.
        if vm.capi_ptr_is_owned_compat(object as usize) {
            // SAFETY: owned-compat pointers have CpythonObjectHead prefix layout.
            let refcnt = unsafe { (*object.cast::<CpythonObjectHead>()).ob_refcnt };
            if refcnt != 0 {
                unsafe { PyErr_BadInternalCall() };
                return;
            }
        }
        vm.clear_runtime_weakrefs_for_target_id(object_id);
        let _ = vm.capi_registry_set_gc_tracked_override(object as usize, false);
        let _ = vm.capi_registry_set_gc_finalized(object as usize, true);
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
    });
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
