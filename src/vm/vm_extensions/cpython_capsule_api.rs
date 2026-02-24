use std::ffi::{CStr, c_char, c_void};

use crate::runtime::{Object, Value};

use super::{
    CPY_PROXY_PTR_ATTR, CpythonCapsuleCompatObject, CpythonObjectHead, CpythonTypeObject,
    ModuleCapiContext, PyCapsule_Type, c_name_to_string, cpython_set_error,
    cpython_value_debug_tag, with_active_cpython_context_mut,
};

unsafe fn cpython_external_capsule_pointer(
    _context: &ModuleCapiContext,
    capsule: *mut c_void,
    requested_name: *const c_char,
) -> Result<Option<*mut c_void>, String> {
    if capsule.is_null() {
        return Ok(None);
    }
    // SAFETY: caller provides an object pointer from extension code; we only inspect the
    // standard object header first and bail out if it's not a capsule.
    let raw = capsule.cast::<CpythonCapsuleCompatObject>();
    let ty = unsafe { (*raw).ob_base.ob_type };
    if ty != std::ptr::addr_of_mut!(PyCapsule_Type).cast() {
        if std::env::var_os("PYRS_TRACE_CPY_CAPSULE").is_some() {
            eprintln!(
                "[cpy-capsule] external type mismatch ptr={:p} type={:p} expected={:p}",
                capsule,
                ty,
                std::ptr::addr_of_mut!(PyCapsule_Type).cast::<c_void>()
            );
        }
        return Ok(None);
    }
    if !requested_name.is_null() {
        // SAFETY: `requested_name` is the C-API input argument.
        let requested = unsafe { CStr::from_ptr(requested_name) };
        // SAFETY: capsule name pointer is part of the external capsule object.
        let actual_ptr = unsafe { (*raw).name };
        if actual_ptr.is_null() {
            if std::env::var_os("PYRS_TRACE_CPY_CAPSULE").is_some() {
                eprintln!(
                    "[cpy-capsule] external name mismatch ptr={:p} requested={} actual=<null> type={:p}",
                    capsule,
                    requested.to_string_lossy(),
                    ty,
                );
            }
            return Err("capsule name mismatch".to_string());
        }
        // SAFETY: `actual_ptr` is validated non-null above.
        let actual = unsafe { CStr::from_ptr(actual_ptr) };
        if requested.to_bytes() != actual.to_bytes() {
            if std::env::var_os("PYRS_TRACE_CPY_CAPSULE").is_some() {
                eprintln!(
                    "[cpy-capsule] external name mismatch ptr={:p} requested={} actual={} type={:p}",
                    capsule,
                    requested.to_string_lossy(),
                    actual.to_string_lossy(),
                    ty,
                );
            }
            return Err(format!(
                "capsule name mismatch (requested='{}', actual='{}')",
                requested.to_string_lossy(),
                actual.to_string_lossy()
            ));
        }
    }
    // SAFETY: `raw` points to a capsule-compatible object.
    let pointer = unsafe { (*raw).pointer };
    if pointer.is_null() {
        return Err("capsule pointer is null".to_string());
    }
    Ok(Some(pointer))
}

unsafe fn cpython_external_capsule_name(capsule: *mut c_void) -> Option<*const c_char> {
    if capsule.is_null() {
        return None;
    }
    // SAFETY: caller provides an object pointer from extension code; we only inspect
    // the capsule-compatible object header and return `None` on type mismatch.
    let raw = capsule.cast::<CpythonCapsuleCompatObject>();
    let ty = unsafe { (*raw).ob_base.ob_type };
    if ty != std::ptr::addr_of_mut!(PyCapsule_Type).cast() {
        return None;
    }
    // SAFETY: `raw` points to a capsule-compatible object.
    Some(unsafe { (*raw).name })
}

unsafe fn cpython_external_capsule_is_valid(capsule: *mut c_void, name: *const c_char) -> bool {
    if capsule.is_null() {
        return false;
    }
    // SAFETY: caller provides an object pointer from extension code; we only inspect
    // the capsule-compatible object header and return false on type mismatch.
    let raw = capsule.cast::<CpythonCapsuleCompatObject>();
    let ty = unsafe { (*raw).ob_base.ob_type };
    if ty != std::ptr::addr_of_mut!(PyCapsule_Type).cast() {
        return false;
    }
    // SAFETY: `raw` points to a capsule-compatible object.
    let pointer = unsafe { (*raw).pointer };
    if pointer.is_null() {
        return false;
    }
    // SAFETY: `raw` points to a capsule-compatible object.
    let capsule_name = unsafe { (*raw).name };
    match (capsule_name.is_null(), name.is_null()) {
        (true, true) => true,
        (false, false) => {
            // SAFETY: both pointers are non-null and expected NUL-terminated by C-API contract.
            let actual = unsafe { CStr::from_ptr(capsule_name) };
            // SAFETY: both pointers are non-null and expected NUL-terminated by C-API contract.
            let requested = unsafe { CStr::from_ptr(name) };
            actual.to_bytes() == requested.to_bytes()
        }
        _ => false,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_New(
    pointer: *mut c_void,
    name: *const c_char,
    destructor: Option<unsafe extern "C" fn(*mut c_void)>,
) -> *mut c_void {
    if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_IMPORT").is_some() {
        let name_text = if name.is_null() {
            "<null>".to_string()
        } else {
            // SAFETY: capsule name pointer is expected to be NUL-terminated.
            unsafe { CStr::from_ptr(name) }
                .to_str()
                .map(|text| text.to_string())
                .unwrap_or_else(|_| "<invalid>".to_string())
        };
        eprintln!(
            "[capsule-new] name={} pointer={:p} destructor={:?}",
            name_text, pointer, destructor
        );
    }
    if std::env::var_os("PYRS_TRACE_PYBIND11_ATTRS").is_some() && !name.is_null() {
        // SAFETY: capsule name pointer is expected to be NUL-terminated.
        let name_text = unsafe { CStr::from_ptr(name) }
            .to_str()
            .unwrap_or("<invalid>");
        if name_text.contains("__pybind11") {
            eprintln!(
                "[pybind11-capsule] new name={} payload={:p} destructor={:?}",
                name_text, pointer, destructor
            );
        }
    }
    if std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some() {
        let name_text = if name.is_null() {
            "<null>".to_string()
        } else {
            // SAFETY: capsule name pointer is expected to be NUL-terminated.
            unsafe { CStr::from_ptr(name) }
                .to_str()
                .map(|text| text.to_string())
                .unwrap_or_else(|_| "<invalid>".to_string())
        };
        eprintln!(
            "[numpy-init] PyCapsule_New name={} pointer={:p}",
            name_text, pointer
        );
    }
    let result = with_active_cpython_context_mut(|context| {
        let ptr = match context.capsule_new(pointer, name, destructor) {
            Ok(handle) => context.alloc_cpython_ptr_for_handle(handle),
            Err(err) => {
                context.set_error(err);
                std::ptr::null_mut()
            }
        };
        if !ptr.is_null() {
            context.pin_capsule_allocation_for_vm(ptr);
        }
        ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    });
    if std::env::var_os("PYRS_TRACE_NUMPY_MEM_HANDLER").is_some() && !name.is_null() {
        // SAFETY: capsule name pointer is expected to be NUL-terminated.
        let name_text = unsafe { CStr::from_ptr(name) }
            .to_str()
            .unwrap_or("<invalid>");
        if name_text == "mem_handler" {
            eprintln!(
                "[numpy-mem-handler] capsule_new ptr={:p} payload={:p}",
                result, pointer
            );
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_GetPointer(
    capsule: *mut c_void,
    name: *const c_char,
) -> *mut c_void {
    let trace_capsule_get = std::env::var_os("PYRS_TRACE_CPY_CAPSULE_GET").is_some();
    let trace_numpy_capsules = std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some();
    let requested_name = if name.is_null() {
        None
    } else {
        // SAFETY: capsule name pointer is expected to be NUL-terminated.
        unsafe { CStr::from_ptr(name) }.to_str().ok()
    };
    let trace_array_api = trace_numpy_capsules
        && requested_name
            .map(|requested| requested.contains("_ARRAY_API") || requested.contains("_UFUNC_API"))
            .unwrap_or(false);
    let trace_pybind_capsule = std::env::var_os("PYRS_TRACE_PYBIND11_ATTRS").is_some()
        && requested_name
            .map(|requested| requested.contains("__pybind11"))
            .unwrap_or(false);
    with_active_cpython_context_mut(|context| {
        if std::env::var_os("PYRS_TRACE_NUMPY_MEM_HANDLER").is_some() && !name.is_null() {
            // SAFETY: capsule name pointer is expected to be NUL-terminated.
            let name_text = unsafe { CStr::from_ptr(name) }
                .to_str()
                .unwrap_or("<invalid>");
            if name_text == "mem_handler" {
                let handle = context.cpython_handle_from_ptr(capsule);
                let tag = handle
                    .and_then(|h| {
                        context
                            .objects
                            .get(&h)
                            .map(|slot| cpython_value_debug_tag(&slot.value))
                    })
                    .unwrap_or_else(|| "<none>".to_string());
                let capsule_name = handle
                    .and_then(|h| context.capsules.get(&h))
                    .and_then(|slot| slot.name.as_ref())
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "<none>".to_string());
                eprintln!(
                    "[numpy-mem-handler] get_pointer capsule={:p} handle={:?} tag={} capsule_name={}",
                    capsule, handle, tag, capsule_name
                );
            }
        }
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            match unsafe { cpython_external_capsule_pointer(context, capsule, name) } {
                Ok(Some(pointer)) => {
                    if trace_pybind_capsule {
                        eprintln!(
                            "[pybind11-capsule] get external requested={} capsule={:p} payload={:p}",
                            requested_name.unwrap_or("<invalid>"),
                            capsule,
                            pointer
                        );
                    }
                    if trace_capsule_get {
                        eprintln!(
                            "[cpy-capsule-get] external capsule={:p} requested={} payload={:p}",
                            capsule,
                            requested_name.unwrap_or("<invalid>"),
                            pointer
                        );
                    }
                    if trace_array_api {
                        // SAFETY: pointer comes from external capsule payload.
                        let first = unsafe {
                            if pointer.is_null() {
                                0usize
                            } else {
                                *pointer.cast::<usize>()
                            }
                        };
                        eprintln!(
                            "[numpy-init] PyCapsule_GetPointer external name={} capsule={:p} payload={:p} first_word=0x{:x}",
                            requested_name.unwrap_or("<invalid>"),
                            capsule,
                            pointer,
                            first
                        );
                    }
                    return pointer;
                }
                Ok(None) => {
                    if std::env::var_os("PYRS_TRACE_CPY_CAPSULE").is_some() {
                        let requested_name = if name.is_null() {
                            "<null>".to_string()
                        } else {
                            unsafe { CStr::from_ptr(name) }
                                .to_str()
                                .map(|value| value.to_string())
                                .unwrap_or_else(|_| "<invalid utf8>".to_string())
                        };
                        eprintln!(
                            "[cpy-capsule] get_pointer unknown ptr={:p} requested_name={}",
                            capsule, requested_name
                        );
                    }
                    context.set_error("PyCapsule_GetPointer received unknown object pointer");
                    return std::ptr::null_mut();
                }
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
        };
        if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some()
            && !context.capsules.contains_key(&handle)
        {
            let tag = context
                .objects
                .get(&handle)
                .map(|slot| cpython_value_debug_tag(&slot.value))
                .unwrap_or_else(|| "<missing>".to_string());
            let requested_name = if name.is_null() {
                "<null>".to_string()
            } else {
                // SAFETY: caller provides a NUL-terminated capsule name.
                unsafe { CStr::from_ptr(name) }
                    .to_str()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|_| "<invalid utf8>".to_string())
            };
            let raw_type = if capsule.is_null() {
                std::ptr::null_mut()
            } else {
                // SAFETY: `capsule` is a candidate PyObject*.
                unsafe { (*capsule.cast::<CpythonObjectHead>()).ob_type }
            };
            let raw_type_name = unsafe {
                raw_type
                    .cast::<CpythonTypeObject>()
                    .as_ref()
                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string())
            };
            eprintln!(
                "[cpy-capsule] get_pointer ptr={:p} handle={} name={} non_capsule_tag={} raw_type={:p} raw_type_name={} expected_capsule_type={:p}",
                capsule,
                handle,
                requested_name,
                tag,
                raw_type,
                raw_type_name,
                std::ptr::addr_of_mut!(PyCapsule_Type).cast::<c_void>()
            );
        }
        if !context.capsules.contains_key(&handle) {
            match unsafe { cpython_external_capsule_pointer(context, capsule, name) } {
                Ok(Some(pointer)) => {
                    if trace_pybind_capsule {
                        eprintln!(
                            "[pybind11-capsule] get proxy-fallback requested={} capsule={:p} payload={:p}",
                            requested_name.unwrap_or("<invalid>"),
                            capsule,
                            pointer
                        );
                    }
                    if trace_capsule_get {
                        eprintln!(
                            "[cpy-capsule-get] proxy-fallback capsule={:p} requested={} payload={:p}",
                            capsule,
                            requested_name.unwrap_or("<invalid>"),
                            pointer
                        );
                    }
                    if trace_array_api {
                        // SAFETY: pointer comes from external capsule payload.
                        let first = unsafe {
                            if pointer.is_null() {
                                0usize
                            } else {
                                *pointer.cast::<usize>()
                            }
                        };
                        eprintln!(
                            "[numpy-init] PyCapsule_GetPointer proxy-fallback name={} capsule={:p} payload={:p} first_word=0x{:x}",
                            requested_name.unwrap_or("<invalid>"),
                            capsule,
                            pointer,
                            first
                        );
                    }
                    return pointer;
                }
                Ok(None) => {}
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
            let proxy_ptr = context.objects.get(&handle).and_then(|slot| {
                let Value::Class(class_obj) = &slot.value else {
                    return None;
                };
                let Object::Class(class_data) = &*class_obj.kind() else {
                    return None;
                };
                match class_data.attrs.get(CPY_PROXY_PTR_ATTR) {
                    Some(Value::Int(raw)) if *raw >= 0 => Some(*raw as usize as *mut c_void),
                    _ => None,
                }
            });
            if let Some(proxy_ptr) = proxy_ptr {
                match unsafe { cpython_external_capsule_pointer(context, proxy_ptr, name) } {
                    Ok(Some(pointer)) => return pointer,
                    Ok(None) => {}
                    Err(err) => {
                        context.set_error(err);
                        return std::ptr::null_mut();
                    }
                }
            }
            context.set_error(format!("invalid capsule handle {}", handle));
            return std::ptr::null_mut();
        }
        match context.capsule_get_pointer(handle, name) {
            Ok(pointer) => {
                if trace_pybind_capsule {
                    eprintln!(
                        "[pybind11-capsule] get managed requested={} capsule={:p} handle={} payload={:p}",
                        requested_name.unwrap_or("<invalid>"),
                        capsule,
                        handle,
                        pointer
                    );
                }
                if trace_capsule_get {
                    eprintln!(
                        "[cpy-capsule-get] managed capsule={:p} requested={} handle={} payload={:p}",
                        capsule,
                        requested_name.unwrap_or("<invalid>"),
                        handle,
                        pointer
                    );
                }
                if trace_array_api {
                    // SAFETY: pointer is a capsule payload from context registry.
                    let first = unsafe {
                        if pointer.is_null() {
                            0usize
                        } else {
                            *pointer.cast::<usize>()
                        }
                    };
                    eprintln!(
                        "[numpy-init] PyCapsule_GetPointer managed name={} capsule={:p} handle={} payload={:p} first_word=0x{:x}",
                        requested_name.unwrap_or("<invalid>"),
                        capsule,
                        handle,
                        pointer,
                        first
                    );
                }
                pointer
            }
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
pub unsafe extern "C" fn PyCapsule_GetName(capsule: *mut c_void) -> *const c_char {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            let external_name = unsafe { cpython_external_capsule_name(capsule) };
            if let Some(name) = external_name {
                return name;
            }
            context.set_error("PyCapsule_GetName received unknown object pointer");
            return std::ptr::null();
        };
        match context.capsule_get_name_ptr(handle) {
            Ok(name) => name,
            Err(err) => {
                context.set_error(err);
                std::ptr::null()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_SetPointer(capsule: *mut c_void, pointer: *mut c_void) -> i32 {
    if pointer.is_null() {
        cpython_set_error("PyCapsule_SetPointer called with null pointer");
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_SetPointer received unknown object pointer");
            return -1;
        };
        match context.capsule_set_pointer(handle, pointer) {
            Ok(()) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyCapsule_GetDestructor(
    capsule: *mut c_void,
) -> Option<unsafe extern "C" fn(*mut c_void)> {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_GetDestructor received unknown object pointer");
            return None;
        };
        match context.capsule_get_cpython_destructor(handle) {
            Ok(destructor) => destructor,
            Err(err) => {
                context.set_error(err);
                None
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        None
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_SetDestructor(
    capsule: *mut c_void,
    destructor: Option<unsafe extern "C" fn(*mut c_void)>,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_SetDestructor received unknown object pointer");
            return -1;
        };
        match context.capsule_set_cpython_destructor(handle, destructor) {
            Ok(()) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyCapsule_SetContext(
    capsule: *mut c_void,
    context_value: *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_SetContext received unknown object pointer");
            return -1;
        };
        match context.capsule_set_context(handle, context_value) {
            Ok(()) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyCapsule_GetContext(capsule: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_GetContext received unknown object pointer");
            return std::ptr::null_mut();
        };
        match context.capsule_get_context(handle) {
            Ok(ctx) => ctx,
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
pub unsafe extern "C" fn PyCapsule_SetName(capsule: *mut c_void, name: *const c_char) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_SetName received unknown object pointer");
            return -1;
        };
        match context.capsule_set_name(handle, name) {
            Ok(()) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyCapsule_IsValid(capsule: *mut c_void, name: *const c_char) -> i32 {
    if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_VALID").is_some() {
        eprintln!(
            "[cpy-capsule-valid] enter ptr={:p} requested_name={}",
            capsule,
            if name.is_null() {
                "<null>".to_string()
            } else {
                unsafe { CStr::from_ptr(name) }
                    .to_str()
                    .map(|text| text.to_string())
                    .unwrap_or_else(|_| "<invalid utf8>".to_string())
            }
        );
    }
    match with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            if unsafe { cpython_external_capsule_is_valid(capsule, name) } {
                return 1;
            }
            if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_VALID").is_some() {
                let raw_type = if capsule.is_null() {
                    std::ptr::null_mut()
                } else {
                    // SAFETY: `capsule` is a candidate PyObject*.
                    unsafe { (*capsule.cast::<CpythonObjectHead>()).ob_type }
                };
                let raw_type_name = unsafe {
                    raw_type
                        .cast::<CpythonTypeObject>()
                        .as_ref()
                        .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                        .unwrap_or_else(|| "<unknown>".to_string())
                };
                eprintln!(
                    "[cpy-capsule-valid] missing-handle ptr={:p} type={:p} type_name={} requested_name={}",
                    capsule,
                    raw_type,
                    raw_type_name,
                    if name.is_null() {
                        "<null>".to_string()
                    } else {
                        unsafe { CStr::from_ptr(name) }
                            .to_str()
                            .map(|text| text.to_string())
                            .unwrap_or_else(|_| "<invalid utf8>".to_string())
                    }
                );
            }
            return 0;
        };
        match context.capsule_is_valid(handle, name) {
            Ok(valid) => {
                if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_VALID").is_some() {
                    eprintln!(
                        "[cpy-capsule-valid] handle={} ptr={:p} valid={} requested_name={}",
                        handle,
                        capsule,
                        valid,
                        if name.is_null() {
                            "<null>".to_string()
                        } else {
                            unsafe { CStr::from_ptr(name) }
                                .to_str()
                                .map(|text| text.to_string())
                                .unwrap_or_else(|_| "<invalid utf8>".to_string())
                        }
                    );
                }
                valid
            }
            Err(err) => {
                if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_VALID").is_some() {
                    eprintln!(
                        "[cpy-capsule-valid] handle={} ptr={:p} error={}",
                        handle, capsule, err
                    );
                }
                0
            }
        }
    }) {
        Ok(value) => value,
        Err(err) => {
            if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_VALID").is_some() {
                eprintln!(
                    "[cpy-capsule-valid] no-active-context ptr={:p} requested_name={} err={}",
                    capsule,
                    if name.is_null() {
                        "<null>".to_string()
                    } else {
                        unsafe { CStr::from_ptr(name) }
                            .to_str()
                            .map(|text| text.to_string())
                            .unwrap_or_else(|_| "<invalid utf8>".to_string())
                    },
                    err
                );
            }
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_Import(name: *const c_char, no_block: i32) -> *mut c_void {
    with_active_cpython_context_mut(|context| match context.capsule_import(name, no_block) {
        Ok(pointer) => pointer,
        Err(err) => {
            context.set_error(err);
            std::ptr::null_mut()
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}
