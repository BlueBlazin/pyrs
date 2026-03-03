use std::alloc::{Layout, alloc, alloc_zeroed, dealloc, realloc as rust_realloc};
use std::ffi::c_void;
use std::mem::{align_of, size_of};

#[repr(C, align(16))]
struct WasmAllocHeader {
    size: usize,
}

const WASM_ALLOC_ALIGN: usize = align_of::<WasmAllocHeader>();
const WASM_ALLOC_HEADER_SIZE: usize = size_of::<WasmAllocHeader>();

fn wasm_layout_for_size(payload_size: usize) -> Option<Layout> {
    let alloc_size = payload_size.max(1);
    let total = WASM_ALLOC_HEADER_SIZE.checked_add(alloc_size)?;
    Layout::from_size_align(total, WASM_ALLOC_ALIGN).ok()
}

unsafe fn wasm_alloc_header_from_payload(payload: *mut c_void) -> *mut WasmAllocHeader {
    // SAFETY: caller ensures payload points to memory returned by wasm_malloc-like shims.
    unsafe {
        payload
            .cast::<u8>()
            .sub(WASM_ALLOC_HEADER_SIZE)
            .cast::<WasmAllocHeader>()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    let Some(layout) = wasm_layout_for_size(size) else {
        return std::ptr::null_mut();
    };
    // SAFETY: layout comes from validated size+alignment.
    let raw = unsafe { alloc(layout) };
    if raw.is_null() {
        return std::ptr::null_mut();
    }
    let header = raw.cast::<WasmAllocHeader>();
    // SAFETY: header lies in allocated region and is properly aligned.
    unsafe {
        (*header).size = size.max(1);
    }
    // SAFETY: payload lies immediately after fixed-size header.
    unsafe { raw.add(WASM_ALLOC_HEADER_SIZE).cast::<c_void>() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    let Some(payload_size) = count.checked_mul(size) else {
        return std::ptr::null_mut();
    };
    let Some(layout) = wasm_layout_for_size(payload_size) else {
        return std::ptr::null_mut();
    };
    // SAFETY: layout comes from validated size+alignment.
    let raw = unsafe { alloc_zeroed(layout) };
    if raw.is_null() {
        return std::ptr::null_mut();
    }
    let header = raw.cast::<WasmAllocHeader>();
    // SAFETY: header lies in allocated region and is properly aligned.
    unsafe {
        (*header).size = payload_size.max(1);
    }
    // SAFETY: payload lies immediately after fixed-size header.
    unsafe { raw.add(WASM_ALLOC_HEADER_SIZE).cast::<c_void>() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: pointer originates from this shim family.
    let header = unsafe { wasm_alloc_header_from_payload(ptr) };
    // SAFETY: header points to initialized allocation metadata.
    let payload_size = unsafe { (*header).size };
    let Some(layout) = wasm_layout_for_size(payload_size) else {
        return;
    };
    // SAFETY: header/layout match original allocation contract.
    unsafe { dealloc(header.cast::<u8>(), layout) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if ptr.is_null() {
        // SAFETY: `malloc` shim handles allocation semantics for null input.
        return unsafe { malloc(size) };
    }
    if size == 0 {
        // SAFETY: pointer came from this shim family and can be freed.
        unsafe { free(ptr) };
        return std::ptr::null_mut();
    }

    // SAFETY: pointer originates from this shim family.
    let header = unsafe { wasm_alloc_header_from_payload(ptr) };
    // SAFETY: header points to initialized allocation metadata.
    let old_payload_size = unsafe { (*header).size };
    let Some(old_layout) = wasm_layout_for_size(old_payload_size) else {
        return std::ptr::null_mut();
    };
    let new_payload_size = size.max(1);
    let Some(new_layout) = wasm_layout_for_size(new_payload_size) else {
        return std::ptr::null_mut();
    };

    // SAFETY: old pointer/layout are valid; new size derived from validated layout.
    let new_raw = unsafe { rust_realloc(header.cast::<u8>(), old_layout, new_layout.size()) };
    if new_raw.is_null() {
        return std::ptr::null_mut();
    }
    let new_header = new_raw.cast::<WasmAllocHeader>();
    // SAFETY: header points to newly allocated block metadata.
    unsafe {
        (*new_header).size = new_payload_size;
    }
    // SAFETY: payload lies immediately after fixed-size header.
    unsafe { new_raw.add(WASM_ALLOC_HEADER_SIZE).cast::<c_void>() }
}
