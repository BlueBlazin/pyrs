use std::ffi::c_void;

use super::{ModuleCapiContext, with_active_cpython_context_mut};

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(nmemb: usize, size: usize) -> *mut c_void;
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawMalloc(size: usize) -> *mut c_void {
    // SAFETY: forwarded directly to C allocator.
    unsafe { malloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawCalloc(count: usize, size: usize) -> *mut c_void {
    // SAFETY: forwarded directly to C allocator.
    unsafe { calloc(count, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawRealloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    // SAFETY: forwarded directly to C allocator.
    unsafe { realloc(ptr, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawFree(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    let mut handled = false;
    let mut suppress_free = false;
    let mut deregistered_vm_pin = false;
    if let Ok(()) = with_active_cpython_context_mut(|context: &mut ModuleCapiContext| {
        if context.owns_cpython_allocation_ptr(ptr) {
            suppress_free = true;
            handled = true;
            return;
        }
        if !context.vm.is_null() {
            // SAFETY: VM pointer is valid for the active context lifetime.
            let vm = unsafe { &mut *context.vm };
            if vm
                .extension_pinned_cpython_allocation_set
                .remove(&(ptr as usize))
            {
                context.capi_registry_mark_pending_free_ptr(ptr);
                vm.extension_pinned_capsule_names.remove(&(ptr as usize));
                vm.capi_registry_mark_freed(ptr as usize);
                deregistered_vm_pin = true;
                handled = true;
            }
        }
    }) {
        if suppress_free {
            if std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some() {
                eprintln!("[cpy-ptr] suppress free for compat ptr={:p}", ptr);
            }
            return;
        }
        if deregistered_vm_pin && std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some() {
            eprintln!("[cpy-ptr] free deregistered pinned ptr={:p}", ptr);
        }
    }
    if handled {
        // SAFETY: caller explicitly requested deallocation; pointer was either non-owned
        // by the active context or removed from VM pinned ownership before this free.
        unsafe {
            free(ptr);
        }
        let _ = with_active_cpython_context_mut(|context: &mut ModuleCapiContext| {
            context.capi_registry_mark_freed_ptr(ptr);
        });
        return;
    }
    let _ = with_active_cpython_context_mut(|context: &mut ModuleCapiContext| {
        context.capi_registry_mark_pending_free_ptr(ptr);
    });
    // SAFETY: forwarded directly to C allocator.
    unsafe { free(ptr) };
    let _ = with_active_cpython_context_mut(|context: &mut ModuleCapiContext| {
        context.capi_registry_mark_freed_ptr(ptr);
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Malloc(size: usize) -> *mut c_void {
    unsafe { PyMem_RawMalloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Calloc(count: usize, size: usize) -> *mut c_void {
    unsafe { PyMem_RawCalloc(count, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe { PyMem_RawRealloc(ptr, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Free(ptr: *mut c_void) {
    unsafe { PyMem_RawFree(ptr) };
}
