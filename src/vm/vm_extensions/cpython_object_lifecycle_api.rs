use std::ffi::c_void;

use super::{
    CpythonObjectHead, CpythonTypeObject, CpythonVarObjectHead, PyErr_NoMemory, PyObject_Malloc,
    c_name_to_string, cpython_set_error, with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Init(object: *mut c_void, ty: *mut c_void) -> *mut c_void {
    if object.is_null() || ty.is_null() {
        cpython_set_error("PyObject_Init received null object/type");
        return std::ptr::null_mut();
    }
    // SAFETY: caller guarantees object points to writable PyObject-compatible memory.
    unsafe {
        let head = object.cast::<CpythonObjectHead>();
        (*head).ob_refcnt = 1;
        (*head).ob_type = ty;
    }
    object
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_InitVar(
    object: *mut c_void,
    ty: *mut c_void,
    size: isize,
) -> *mut c_void {
    if object.is_null() || ty.is_null() {
        cpython_set_error("PyObject_InitVar received null object/type");
        return std::ptr::null_mut();
    }
    // SAFETY: caller guarantees object points to writable PyVarObject-compatible memory.
    unsafe {
        let head = object.cast::<CpythonVarObjectHead>();
        (*head).ob_base.ob_refcnt = 1;
        (*head).ob_base.ob_type = ty;
        (*head).ob_size = size;
    }
    object
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_New(ty: *mut CpythonTypeObject) -> *mut c_void {
    let basicsize = if ty.is_null() {
        std::mem::size_of::<CpythonObjectHead>()
    } else {
        // SAFETY: caller provided a type pointer.
        let size = unsafe { (*ty).tp_basicsize };
        if size <= 0 {
            std::mem::size_of::<CpythonObjectHead>()
        } else {
            size as usize
        }
    };
    let raw = unsafe { PyObject_Malloc(basicsize) }.cast::<u8>();
    if raw.is_null() {
        unsafe { PyErr_NoMemory() };
        return std::ptr::null_mut();
    }
    // SAFETY: newly allocated buffer has at least basicsize bytes.
    unsafe {
        std::ptr::write_bytes(raw, 0, basicsize);
        let head = raw.cast::<CpythonObjectHead>();
        (*head).ob_refcnt = 1;
        (*head).ob_type = ty.cast();
    }
    raw.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_NewVar(
    ty: *mut CpythonTypeObject,
    nitems: isize,
) -> *mut c_void {
    let base = if ty.is_null() {
        std::mem::size_of::<CpythonVarObjectHead>()
    } else {
        let size = unsafe { (*ty).tp_basicsize };
        if size <= 0 {
            std::mem::size_of::<CpythonVarObjectHead>()
        } else {
            size as usize
        }
    };
    let item_size = if ty.is_null() {
        0usize
    } else {
        unsafe { (*ty).tp_itemsize.max(0) as usize }
    };
    let extra = if nitems <= 0 {
        0usize
    } else {
        item_size.saturating_mul(nitems as usize)
    };
    let total = base.saturating_add(extra);
    let raw = unsafe { PyObject_Malloc(total) }.cast::<u8>();
    if raw.is_null() {
        unsafe { PyErr_NoMemory() };
        return std::ptr::null_mut();
    }
    // SAFETY: newly allocated buffer has at least total bytes.
    unsafe {
        std::ptr::write_bytes(raw, 0, total);
        let head = raw.cast::<CpythonVarObjectHead>();
        (*head).ob_base.ob_refcnt = 1;
        (*head).ob_base.ob_type = ty.cast();
        (*head).ob_size = nitems;
    }
    raw.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_GC_New(ty: *mut CpythonTypeObject) -> *mut c_void {
    unsafe { _PyObject_New(ty) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_Dealloc(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    let trace_dealloc = super::super::env_var_present_cached("PYRS_TRACE_CPY_DEALLOC");
    let object_type_name = if trace_dealloc {
        // SAFETY: best-effort debug read for candidate PyObject*.
        let ty_ptr =
            unsafe { (*object.cast::<CpythonObjectHead>()).ob_type }.cast::<CpythonTypeObject>();
        if ty_ptr.is_null() {
            "<null>".to_string()
        } else {
            // SAFETY: best-effort type name read for tracing.
            unsafe { c_name_to_string((*ty_ptr).tp_name) }
                .unwrap_or_else(|_| "<invalid>".to_string())
        }
    } else {
        String::new()
    };
    if trace_dealloc {
        eprintln!(
            "[cpy-dealloc] object={:p} type={}",
            object, object_type_name
        );
    }
    enum DeallocAction {
        Handled,
        NoContextMatch,
    }
    let action = with_active_cpython_context_mut(|context| {
        let handle = match context.cpython_handle_from_ptr(object) {
            Some(handle) => handle,
            None => {
                let _ = context.cpython_value_from_ptr_or_proxy(object);
                let Some(handle) = context.cpython_handle_from_ptr(object) else {
                    return DeallocAction::NoContextMatch;
                };
                handle
            }
        };
        context.set_object_refcount(handle, 0);
        let _ = context.release_object_handle_after_zero_ref(handle);
        DeallocAction::Handled
    })
    .unwrap_or(DeallocAction::NoContextMatch);
    if trace_dealloc {
        let action_label = match action {
            DeallocAction::Handled => "handled",
            DeallocAction::NoContextMatch => "none",
        };
        eprintln!("[cpy-dealloc] action={action_label} object={:p}", object);
    }
    if matches!(
        action,
        DeallocAction::Handled | DeallocAction::NoContextMatch
    ) {
        return;
    }
}
