use std::ffi::{CStr, c_char, c_int, c_void};

use super::{
    _PyObject_NewVar, CpythonObjectHead, CpythonTypeObject, CpythonVarObjectHead,
    cpython_is_interned_unicode_ptr, cpython_set_error, with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IncRef(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    // SAFETY: caller provides a PyObject-compatible pointer.
    unsafe {
        if let Some(head) = object.cast::<CpythonObjectHead>().as_mut()
            && head.ob_refcnt >= 0
        {
            head.ob_refcnt = head.ob_refcnt.saturating_add(1);
        }
    }
    let _ = with_active_cpython_context_mut(|context| {
        if let Some(handle) = context.cpython_handle_from_ptr(object) {
            let _ = context.incref(handle);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_DecRef(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    let interned = cpython_is_interned_unicode_ptr(object);
    // SAFETY: caller provides a PyObject-compatible pointer.
    unsafe {
        if let Some(head) = object.cast::<CpythonObjectHead>().as_mut()
            && head.ob_refcnt > 0
        {
            if !interned || head.ob_refcnt > 1 {
                head.ob_refcnt = head.ob_refcnt.saturating_sub(1);
            }
        }
    }
    let _ = with_active_cpython_context_mut(|context| {
        if interned {
            return;
        }
        if let Some(handle) = context.cpython_handle_from_ptr(object) {
            let _ = context.decref(handle);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_XIncRef(object: *mut c_void) {
    if !object.is_null() {
        unsafe { Py_IncRef(object) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_XDecRef(object: *mut c_void) {
    if !object.is_null() {
        unsafe { Py_DecRef(object) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_IncRef(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    // SAFETY: caller provides a PyObject-compatible pointer.
    unsafe {
        if let Some(head) = object.cast::<CpythonObjectHead>().as_mut()
            && head.ob_refcnt >= 0
        {
            head.ob_refcnt = head.ob_refcnt.saturating_add(1);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_DecRef(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    let interned = cpython_is_interned_unicode_ptr(object);
    // SAFETY: caller provides a PyObject-compatible pointer.
    unsafe {
        if let Some(head) = object.cast::<CpythonObjectHead>().as_mut()
            && head.ob_refcnt > 0
        {
            if !interned || head.ob_refcnt > 1 {
                head.ob_refcnt = head.ob_refcnt.saturating_sub(1);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_SetRefcnt(object: *mut c_void, refcnt: isize) {
    if object.is_null() {
        return;
    }
    // SAFETY: caller provides a PyObject-compatible pointer.
    unsafe {
        (*object.cast::<CpythonObjectHead>()).ob_refcnt = refcnt.max(0);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_NegativeRefcount(
    filename: *const c_char,
    lineno: c_int,
    _object: *mut c_void,
) {
    let file = if filename.is_null() {
        "<unknown>".to_string()
    } else {
        // SAFETY: caller provides a valid C string when non-null.
        unsafe { CStr::from_ptr(filename) }
            .to_string_lossy()
            .to_string()
    };
    cpython_set_error(format!("negative refcount at {file}:{lineno}"));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_CheckRecursiveCall(_where: *const c_char) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_GC_NewVar(
    ty: *mut CpythonTypeObject,
    nitems: isize,
) -> *mut c_void {
    unsafe { _PyObject_NewVar(ty, nitems) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_GC_Resize(object: *mut c_void, nitems: isize) -> *mut c_void {
    if object.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller provides a valid PyVarObject-compatible pointer.
    unsafe {
        (*object.cast::<CpythonVarObjectHead>()).ob_size = nitems;
    }
    object
}
