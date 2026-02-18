use std::ffi::c_void;

use super::with_active_cpython_context_mut;

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
    if let Ok(true) =
        with_active_cpython_context_mut(|context| context.owns_cpython_allocation_ptr(ptr))
    {
        if std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some() {
            eprintln!("[cpy-ptr] suppress free for compat ptr={:p}", ptr);
        }
        return;
    }
    // SAFETY: forwarded directly to C allocator.
    unsafe { free(ptr) };
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
