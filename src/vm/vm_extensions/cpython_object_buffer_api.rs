use std::ffi::{c_char, c_void};

use crate::runtime::{BuiltinFunction, Object, Value};

use super::{
    CpythonBuffer, CpythonBufferInternal, ModuleCapiContext, Py_XIncRef, PyBuffer_Release,
    PyErr_BadInternalCall, PyExc_BufferError, PyExc_TypeError, PyExc_ValueError, PyLong_AsLong,
    PyrsObjectHandle, c_name_to_string, cpython_bytes_data_ptr, cpython_call_builtin,
    cpython_new_ptr_for_value, cpython_set_error, cpython_set_typed_error, cpython_value_from_ptr,
    with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_AsFileDescriptor(object: *mut c_void) -> i32 {
    unsafe { PyLong_AsLong(object) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CheckBuffer(object: *mut c_void) -> i32 {
    match cpython_value_from_ptr(object) {
        Ok(Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_)) => 1,
        Ok(_) => 0,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CheckReadBuffer(object: *mut c_void) -> i32 {
    unsafe { PyObject_CheckBuffer(object) }
}

fn cpython_legacy_bytes_buffer_slot(
    context: &mut ModuleCapiContext,
    object: *mut c_void,
    writable: bool,
) -> Result<(PyrsObjectHandle, *mut c_void, usize), String> {
    let handle = context
        .cpython_handle_from_ptr(object)
        .ok_or_else(|| "unknown object pointer".to_string())?;
    let slot = context
        .objects
        .get(&handle)
        .ok_or_else(|| "unknown object handle".to_string())?;
    let len = match &slot.value {
        Value::Bytes(obj) => {
            if writable {
                return Err("expected writable bytes-like object".to_string());
            }
            match &*obj.kind() {
                Object::Bytes(bytes) => bytes.len(),
                _ => return Err("invalid bytes storage".to_string()),
            }
        }
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(bytes) => bytes.len(),
            _ => return Err("invalid bytearray storage".to_string()),
        },
        _ => return Err("expected bytes-like object".to_string()),
    };
    context.sync_cpython_storage_from_value(handle);
    let raw_ptr = context
        .cpython_ptr_by_handle
        .get(&handle)
        .copied()
        .ok_or_else(|| "missing CPython storage pointer".to_string())?;
    Ok((handle, raw_ptr, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_AsReadBuffer(
    object: *mut c_void,
    buffer: *mut *const c_void,
    buffer_len: *mut isize,
) -> i32 {
    if buffer.is_null() || buffer_len.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe {
        *buffer = std::ptr::null();
        *buffer_len = 0;
    }
    with_active_cpython_context_mut(|context| {
        match cpython_legacy_bytes_buffer_slot(context, object, false) {
            Ok((_handle, raw_ptr, len)) => {
                // SAFETY: raw_ptr is owned CPython-compatible bytes/bytearray storage.
                let data = unsafe { cpython_bytes_data_ptr(raw_ptr) };
                unsafe {
                    *buffer = data.cast();
                    *buffer_len = len as isize;
                }
                0
            }
            Err(err) => {
                context.set_error(format!("PyObject_AsReadBuffer {err}"));
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
pub unsafe extern "C" fn PyObject_AsWriteBuffer(
    object: *mut c_void,
    buffer: *mut *mut c_void,
    buffer_len: *mut isize,
) -> i32 {
    if buffer.is_null() || buffer_len.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe {
        *buffer = std::ptr::null_mut();
        *buffer_len = 0;
    }
    with_active_cpython_context_mut(|context| {
        match cpython_legacy_bytes_buffer_slot(context, object, true) {
            Ok((_handle, raw_ptr, len)) => {
                // SAFETY: raw_ptr is owned CPython-compatible bytearray storage.
                let data = unsafe { cpython_bytes_data_ptr(raw_ptr) };
                unsafe {
                    *buffer = data.cast();
                    *buffer_len = len as isize;
                }
                0
            }
            Err(err) => {
                let message = format!("PyObject_AsWriteBuffer {err}");
                let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
                context.set_error_state(
                    unsafe { PyExc_TypeError },
                    pvalue,
                    std::ptr::null_mut(),
                    message,
                );
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
pub unsafe extern "C" fn PyObject_AsCharBuffer(
    object: *mut c_void,
    buffer: *mut *const c_char,
    buffer_len: *mut isize,
) -> i32 {
    if buffer.is_null() || buffer_len.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let mut raw: *const c_void = std::ptr::null();
    let status = unsafe { PyObject_AsReadBuffer(object, &mut raw, buffer_len) };
    if status != 0 {
        return status;
    }
    unsafe { *buffer = raw.cast() };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CopyData(dest: *mut c_void, src: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let (src_handle, src_ptr, src_len) =
            match cpython_legacy_bytes_buffer_slot(context, src, false) {
                Ok(state) => state,
                Err(err) => {
                    context.set_error(format!("PyObject_CopyData source {err}"));
                    return -1;
                }
            };
        let (dest_handle, dest_ptr, dest_len) =
            match cpython_legacy_bytes_buffer_slot(context, dest, true) {
                Ok(state) => state,
                Err(err) => {
                    let message = format!("PyObject_CopyData destination {err}");
                    let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
                    context.set_error_state(
                        unsafe { PyExc_TypeError },
                        pvalue,
                        std::ptr::null_mut(),
                        message,
                    );
                    return -1;
                }
            };
        if src_len != dest_len {
            let message = "source and destination buffers have different lengths".to_string();
            let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
            context.set_error_state(
                unsafe { PyExc_ValueError },
                pvalue,
                std::ptr::null_mut(),
                message,
            );
            return -1;
        }
        if src_len > 0 {
            // SAFETY: pointers are owned bytes storage with at least src_len bytes each.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    cpython_bytes_data_ptr(src_ptr).cast::<u8>(),
                    cpython_bytes_data_ptr(dest_ptr).cast::<u8>(),
                    src_len,
                );
            }
        }
        context.sync_value_from_cpython_storage(src_handle, src_ptr);
        context.sync_value_from_cpython_storage(dest_handle, dest_ptr);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

fn cpython_buffer_layout_from_view(
    view: &CpythonBuffer,
) -> (
    usize,
    Option<String>,
    Option<Vec<isize>>,
    Option<Vec<isize>>,
    bool,
) {
    let itemsize = view.itemsize.max(1) as usize;
    let format = if view.format.is_null() {
        None
    } else {
        unsafe { c_name_to_string(view.format.cast_const()) }.ok()
    };
    let ndim = view.ndim.max(0) as usize;
    let shape = if ndim == 0 || view.shape.is_null() {
        None
    } else {
        Some(
            (0..ndim)
                .map(|idx| {
                    // SAFETY: `shape` is valid for `ndim` entries by C-API contract.
                    unsafe { *view.shape.add(idx) }
                })
                .collect::<Vec<_>>(),
        )
    };
    let strides = if ndim == 0 || view.strides.is_null() {
        None
    } else {
        Some(
            (0..ndim)
                .map(|idx| {
                    // SAFETY: `strides` is valid for `ndim` entries by C-API contract.
                    unsafe { *view.strides.add(idx) }
                })
                .collect::<Vec<_>>(),
        )
    };
    let contiguous =
        unsafe { PyBuffer_IsContiguous(view as *const CpythonBuffer, b'A' as c_char) } != 0;
    (itemsize, format, shape, strides, contiguous)
}

fn cpython_alloc_memoryview_with_layout(
    context: &mut ModuleCapiContext,
    bytes: Vec<u8>,
    writable: bool,
    itemsize: usize,
    format: Option<String>,
    shape: Option<Vec<isize>>,
    strides: Option<Vec<isize>>,
    contiguous: bool,
) -> *mut c_void {
    if context.vm.is_null() {
        context.set_error("memoryview allocation missing VM context");
        return std::ptr::null_mut();
    }
    // SAFETY: VM pointer is valid for context lifetime.
    let vm = unsafe { &mut *context.vm };
    let source = if writable {
        vm.heap.alloc_bytearray(bytes.clone())
    } else {
        vm.heap.alloc_bytes(bytes.clone())
    };
    let source_obj = match &source {
        Value::Bytes(obj) | Value::ByteArray(obj) => obj.clone(),
        _ => {
            context.set_error("memoryview allocation expected bytes-like source");
            return std::ptr::null_mut();
        }
    };
    let value = vm
        .heap
        .alloc_memoryview_with(source_obj, itemsize.max(1), format.clone());
    if let Value::MemoryView(view_obj) = &value
        && let Object::MemoryView(view_data) = &mut *view_obj.kind_mut()
    {
        view_data.shape = shape;
        view_data.strides = strides;
        view_data.contiguous = contiguous;
        view_data.length = Some(bytes.len());
        view_data.start = 0;
        view_data.released = false;
        view_data.format = format;
    }
    context.alloc_cpython_ptr_for_value(value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMemoryView_FromObject(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::MemoryView, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMemoryView_FromMemory(
    mem: *mut c_char,
    size: isize,
    flags: i32,
) -> *mut c_void {
    if mem.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyMemoryView_FromMemory(): mem must not be NULL",
        );
        return std::ptr::null_mut();
    }
    if size < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyMemoryView_FromMemory(): size must be >= 0",
        );
        return std::ptr::null_mut();
    }
    if flags != 0x0100 && flags != 0x0200 {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: caller promises `mem` points to at least `size` bytes.
    let payload = unsafe { std::slice::from_raw_parts(mem.cast::<u8>(), size as usize) }.to_vec();
    let writable = flags == 0x0200;
    with_active_cpython_context_mut(|context| {
        cpython_alloc_memoryview_with_layout(
            context,
            payload,
            writable,
            1,
            Some("B".to_string()),
            None,
            None,
            true,
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMemoryView_FromBuffer(info: *const CpythonBuffer) -> *mut c_void {
    if info.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: caller passed a valid `Py_buffer`.
    let view = unsafe { &*info };
    if view.buf.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyMemoryView_FromBuffer(): info->buf must not be NULL",
        );
        return std::ptr::null_mut();
    }
    if view.len < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyMemoryView_FromBuffer(): info->len must be >= 0",
        );
        return std::ptr::null_mut();
    }
    let len = view.len as usize;
    // SAFETY: caller promises `buf` points to at least `len` bytes.
    let payload = unsafe { std::slice::from_raw_parts(view.buf.cast::<u8>(), len) }.to_vec();
    let writable = view.readonly == 0;
    let (itemsize, format, shape, strides, contiguous) = cpython_buffer_layout_from_view(view);
    with_active_cpython_context_mut(|context| {
        cpython_alloc_memoryview_with_layout(
            context, payload, writable, itemsize, format, shape, strides, contiguous,
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMemoryView_GetContiguous(
    object: *mut c_void,
    buffertype: i32,
    order: c_char,
) -> *mut c_void {
    if buffertype != 0x0100 && buffertype != 0x0200 {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let order_char = order as u8 as char;
    if !matches!(order_char, 'C' | 'F' | 'A') {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }

    let mut view = CpythonBuffer {
        buf: std::ptr::null_mut(),
        obj: std::ptr::null_mut(),
        len: 0,
        itemsize: 0,
        readonly: 1,
        ndim: 0,
        format: std::ptr::null_mut(),
        shape: std::ptr::null_mut(),
        strides: std::ptr::null_mut(),
        suboffsets: std::ptr::null_mut(),
        internal: std::ptr::null_mut(),
    };
    if unsafe { PyObject_GetBuffer(object, &mut view, 0) } != 0 {
        return std::ptr::null_mut();
    }

    if buffertype == 0x0200 && view.readonly != 0 {
        unsafe { PyBuffer_Release((&mut view as *mut CpythonBuffer).cast()) };
        cpython_set_typed_error(
            unsafe { PyExc_BufferError },
            "underlying buffer is not writable",
        );
        return std::ptr::null_mut();
    }

    if unsafe { PyBuffer_IsContiguous(&view, order) } != 0 {
        unsafe { PyBuffer_Release((&mut view as *mut CpythonBuffer).cast()) };
        return unsafe { PyMemoryView_FromObject(object) };
    }

    if buffertype == 0x0200 {
        unsafe { PyBuffer_Release((&mut view as *mut CpythonBuffer).cast()) };
        cpython_set_typed_error(
            unsafe { PyExc_BufferError },
            "writable contiguous buffer requested for a non-contiguous object.",
        );
        return std::ptr::null_mut();
    }

    let len = view.len.max(0) as usize;
    let mut contiguous = vec![0u8; len];
    let copy_status =
        unsafe { PyBuffer_ToContiguous(contiguous.as_mut_ptr().cast(), &view, view.len, order) };
    if copy_status != 0 {
        unsafe { PyBuffer_Release((&mut view as *mut CpythonBuffer).cast()) };
        return std::ptr::null_mut();
    }

    let (itemsize, format, shape, _strides, _contiguous) = cpython_buffer_layout_from_view(&view);
    let mut strides = None;
    if let Some(shape_values) = shape.clone()
        && !shape_values.is_empty()
    {
        let mut computed = vec![0isize; shape_values.len()];
        unsafe {
            PyBuffer_FillContiguousStrides(
                shape_values.len() as i32,
                shape_values.as_ptr(),
                computed.as_mut_ptr(),
                itemsize as i32,
                if order_char == 'F' {
                    b'F' as c_char
                } else {
                    b'C' as c_char
                },
            );
        }
        strides = Some(computed);
    }
    unsafe { PyBuffer_Release((&mut view as *mut CpythonBuffer).cast()) };
    with_active_cpython_context_mut(|context| {
        cpython_alloc_memoryview_with_layout(
            context, contiguous, false, itemsize, format, shape, strides, true,
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetBuffer(
    object: *mut c_void,
    view: *mut CpythonBuffer,
    _flags: i32,
) -> i32 {
    if view.is_null() {
        cpython_set_error("PyObject_GetBuffer received null view");
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(object) else {
            context.set_error("PyObject_GetBuffer received unknown object pointer");
            return -1;
        };
        let info = match context.object_get_buffer_info_v2(handle) {
            Ok(info) => info,
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        };
        let internal = Box::into_raw(Box::new(CpythonBufferInternal { handle }));
        // SAFETY: caller passed a valid writable Py_buffer pointer.
        unsafe {
            *view = CpythonBuffer {
                buf: info.data.cast_mut().cast(),
                obj: object,
                len: info.len as isize,
                itemsize: info.itemsize as isize,
                readonly: info.readonly,
                ndim: info.ndim as i32,
                format: info.format.cast_mut(),
                shape: info.shape.cast_mut(),
                strides: info.strides.cast_mut(),
                suboffsets: std::ptr::null_mut(),
                internal: internal.cast(),
            };
            Py_XIncRef(object);
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

fn cpython_buffer_is_c_contiguous(view: &CpythonBuffer) -> bool {
    if view.len == 0 {
        return true;
    }
    if view.strides.is_null() {
        return true;
    }
    if view.ndim <= 0 || view.shape.is_null() {
        return false;
    }
    let mut stride = view.itemsize;
    for dim in (0..(view.ndim as usize)).rev() {
        // SAFETY: shape/strides are validated as non-null and indexed by ndim.
        let size = unsafe { *view.shape.add(dim) };
        // SAFETY: shape/strides are validated as non-null and indexed by ndim.
        let actual = unsafe { *view.strides.add(dim) };
        if size > 1 && actual != stride {
            return false;
        }
        stride = stride.saturating_mul(size);
    }
    true
}

fn cpython_buffer_is_f_contiguous(view: &CpythonBuffer) -> bool {
    if view.len == 0 {
        return true;
    }
    if view.strides.is_null() {
        if view.ndim <= 1 {
            return true;
        }
        if view.shape.is_null() {
            return false;
        }
        let mut gt_one = 0;
        for dim in 0..(view.ndim as usize) {
            // SAFETY: shape is validated as non-null and indexed by ndim.
            if unsafe { *view.shape.add(dim) } > 1 {
                gt_one += 1;
            }
        }
        return gt_one <= 1;
    }
    if view.ndim <= 0 || view.shape.is_null() {
        return false;
    }
    let mut stride = view.itemsize;
    for dim in 0..(view.ndim as usize) {
        // SAFETY: shape/strides are validated as non-null and indexed by ndim.
        let size = unsafe { *view.shape.add(dim) };
        // SAFETY: shape/strides are validated as non-null and indexed by ndim.
        let actual = unsafe { *view.strides.add(dim) };
        if size > 1 && actual != stride {
            return false;
        }
        stride = stride.saturating_mul(size);
    }
    true
}

fn cpython_buffer_add_one_index_c(index: &mut [isize], shape: &[isize]) {
    for pos in (0..index.len()).rev() {
        if index[pos] < shape[pos].saturating_sub(1) {
            index[pos] += 1;
            break;
        }
        index[pos] = 0;
    }
}

fn cpython_buffer_add_one_index_f(index: &mut [isize], shape: &[isize]) {
    for pos in 0..index.len() {
        if index[pos] < shape[pos].saturating_sub(1) {
            index[pos] += 1;
            break;
        }
        index[pos] = 0;
    }
}

fn cpython_buffer_itemsize_from_format_char(ch: char) -> Option<isize> {
    let size = match ch {
        'x' | 'c' | 'b' | 'B' | '?' => 1,
        'h' | 'H' | 'e' => 2,
        'i' | 'I' | 'l' | 'L' | 'f' | 'n' | 'N' => 4,
        'q' | 'Q' | 'd' | 'P' => 8,
        _ => return None,
    };
    Some(size)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_IsContiguous(view: *const CpythonBuffer, order: c_char) -> i32 {
    if view.is_null() {
        return 0;
    }
    // SAFETY: caller provided a valid Py_buffer pointer.
    let view = unsafe { &*view };
    if !view.suboffsets.is_null() {
        return 0;
    }
    let order = order as u8 as char;
    let contiguous = match order {
        'C' => cpython_buffer_is_c_contiguous(view),
        'F' => cpython_buffer_is_f_contiguous(view),
        'A' => cpython_buffer_is_c_contiguous(view) || cpython_buffer_is_f_contiguous(view),
        _ => false,
    };
    if contiguous { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_GetPointer(
    view: *const CpythonBuffer,
    indices: *const isize,
) -> *mut c_void {
    if view.is_null() || indices.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller provided pointers are valid per C-API contract.
    let view = unsafe { &*view };
    let mut pointer = view.buf.cast::<u8>();
    let ndim = view.ndim.max(0) as usize;
    for dim in 0..ndim {
        let stride = if view.strides.is_null() {
            if view.shape.is_null() {
                view.itemsize
            } else {
                let mut computed = view.itemsize;
                for next in ((dim + 1)..ndim).rev() {
                    // SAFETY: shape is valid for ndim entries.
                    computed = computed.saturating_mul(unsafe { *view.shape.add(next) });
                }
                computed
            }
        } else {
            // SAFETY: strides is valid for ndim entries.
            unsafe { *view.strides.add(dim) }
        };
        // SAFETY: pointers are valid for ndim entries.
        let index = unsafe { *indices.add(dim) };
        // SAFETY: pointer arithmetic follows caller-provided buffer bounds contract.
        pointer = unsafe { pointer.offset(stride.saturating_mul(index)) };
        if !view.suboffsets.is_null() {
            // SAFETY: suboffsets is valid for ndim entries.
            let suboffset = unsafe { *view.suboffsets.add(dim) };
            if suboffset >= 0 {
                // SAFETY: pointer currently addresses a valid pointer-sized slot.
                let indirect = unsafe { *(pointer.cast::<*mut u8>()) };
                // SAFETY: pointer arithmetic follows caller-provided buffer bounds contract.
                pointer = unsafe { indirect.offset(suboffset) };
            }
        }
    }
    pointer.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_SizeFromFormat(format: *const c_char) -> isize {
    if format.is_null() {
        return 1;
    }
    let text = match unsafe { c_name_to_string(format) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let mut chars = text.chars();
    let mut first = chars.next().unwrap_or('B');
    if matches!(first, '@' | '=' | '<' | '>' | '!') {
        first = chars.next().unwrap_or('B');
    }
    match cpython_buffer_itemsize_from_format_char(first) {
        Some(size) => size,
        None => {
            cpython_set_error("PyBuffer_SizeFromFormat unsupported format");
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_FillContiguousStrides(
    ndim: i32,
    shape: *const isize,
    strides: *mut isize,
    itemsize: i32,
    fort: c_char,
) {
    if ndim <= 0 || shape.is_null() || strides.is_null() {
        return;
    }
    let mut stride = itemsize as isize;
    let fort = fort as u8 as char;
    if fort == 'F' {
        for dim in 0..(ndim as usize) {
            // SAFETY: caller provided valid shape/strides arrays.
            unsafe { *strides.add(dim) = stride };
            // SAFETY: caller provided valid shape array.
            stride = stride.saturating_mul(unsafe { *shape.add(dim) });
        }
    } else {
        for dim in (0..(ndim as usize)).rev() {
            // SAFETY: caller provided valid shape/strides arrays.
            unsafe { *strides.add(dim) = stride };
            // SAFETY: caller provided valid shape array.
            stride = stride.saturating_mul(unsafe { *shape.add(dim) });
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_FillInfo(
    view: *mut CpythonBuffer,
    object: *mut c_void,
    buf: *mut c_void,
    len: isize,
    readonly: i32,
    flags: i32,
) -> i32 {
    const PYBUF_SIMPLE: i32 = 0;
    const PYBUF_WRITABLE: i32 = 0x0001;
    const PYBUF_FORMAT: i32 = 0x0004;
    const PYBUF_ND: i32 = 0x0008;
    const PYBUF_STRIDES: i32 = 0x0010 | PYBUF_ND;
    const PYBUF_READ: i32 = 0x0100;
    const PYBUF_WRITE: i32 = 0x0200;

    if view.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_BufferError },
            "PyBuffer_FillInfo: view==NULL argument is obsolete",
        );
        return -1;
    }
    if flags != PYBUF_SIMPLE {
        if flags == PYBUF_READ || flags == PYBUF_WRITE {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        }
        if (flags & PYBUF_WRITABLE) == PYBUF_WRITABLE && readonly == 1 {
            cpython_set_typed_error(unsafe { PyExc_BufferError }, "Object is not writable.");
            return -1;
        }
    }
    // SAFETY: caller passed a valid writable Py_buffer pointer.
    unsafe {
        (*view).obj = object;
        Py_XIncRef(object);
        (*view).buf = buf;
        (*view).len = len;
        (*view).readonly = readonly;
        (*view).itemsize = 1;
        (*view).format = std::ptr::null_mut();
        if (flags & PYBUF_FORMAT) == PYBUF_FORMAT {
            (*view).format = c"B".as_ptr().cast_mut();
        }
        (*view).ndim = 1;
        (*view).shape = std::ptr::null_mut();
        if (flags & PYBUF_ND) == PYBUF_ND {
            (*view).shape = std::ptr::addr_of_mut!((*view).len);
        }
        (*view).strides = std::ptr::null_mut();
        if (flags & PYBUF_STRIDES) == PYBUF_STRIDES {
            (*view).strides = std::ptr::addr_of_mut!((*view).itemsize);
        }
        (*view).suboffsets = std::ptr::null_mut();
        (*view).internal = std::ptr::null_mut();
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_FromContiguous(
    view: *const CpythonBuffer,
    buf: *const c_void,
    mut len: isize,
    fort: c_char,
) -> i32 {
    if view.is_null() || buf.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    // SAFETY: caller provided valid pointers.
    let view = unsafe { &*view };
    if len > view.len {
        len = view.len;
    }
    if len <= 0 {
        return 0;
    }
    let itemsize = view.itemsize.max(1);
    if unsafe { PyBuffer_IsContiguous(view, fort) } != 0 {
        // SAFETY: caller-provided source/destination are valid for `len` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(buf.cast::<u8>(), view.buf.cast::<u8>(), len as usize)
        };
        return 0;
    }
    if view.ndim <= 0 || view.shape.is_null() || view.strides.is_null() {
        cpython_set_error("PyBuffer_FromContiguous requires shape/strides for non-contiguous view");
        return -1;
    }
    let ndim = view.ndim as usize;
    let mut indices = vec![0isize; ndim];
    let shape: Vec<isize> = (0..ndim)
        .map(|idx| {
            // SAFETY: shape pointer is valid for `ndim` entries.
            unsafe { *view.shape.add(idx) }
        })
        .collect();
    let mut src_offset = 0usize;
    let elements = (len / itemsize).max(0) as usize;
    let src = buf.cast::<u8>();
    let use_fortran = (fort as u8 as char) == 'F';
    for _ in 0..elements {
        let dst = unsafe { PyBuffer_GetPointer(view, indices.as_ptr()) };
        if dst.is_null() {
            return -1;
        }
        // SAFETY: source and destination each have `itemsize` bytes for this element.
        unsafe {
            std::ptr::copy_nonoverlapping(src.add(src_offset), dst.cast::<u8>(), itemsize as usize)
        };
        src_offset += itemsize as usize;
        if use_fortran {
            cpython_buffer_add_one_index_f(&mut indices, &shape);
        } else {
            cpython_buffer_add_one_index_c(&mut indices, &shape);
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_ToContiguous(
    buf: *mut c_void,
    src: *const CpythonBuffer,
    len: isize,
    order: c_char,
) -> i32 {
    if buf.is_null() || src.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    // SAFETY: caller provided valid pointers.
    let src = unsafe { &*src };
    if len != src.len {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyBuffer_ToContiguous: len != view->len",
        );
        return -1;
    }
    if len <= 0 {
        return 0;
    }
    let requested_order = order as u8 as char;
    if unsafe { PyBuffer_IsContiguous(src, order) } != 0 {
        // SAFETY: destination and source are valid for `len` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(src.buf.cast::<u8>(), buf.cast::<u8>(), len as usize);
        }
        return 0;
    }
    if src.ndim <= 0 || src.shape.is_null() || src.strides.is_null() {
        cpython_set_error("PyBuffer_ToContiguous requires shape/strides for non-contiguous source");
        return -1;
    }
    let use_fortran = if requested_order == 'F' {
        true
    } else if requested_order == 'A' {
        cpython_buffer_is_f_contiguous(src) && !cpython_buffer_is_c_contiguous(src)
    } else {
        false
    };
    let ndim = src.ndim as usize;
    let shape: Vec<isize> = (0..ndim)
        .map(|idx| {
            // SAFETY: shape pointer is valid for `ndim` entries.
            unsafe { *src.shape.add(idx) }
        })
        .collect();
    let mut indices = vec![0isize; ndim];
    let itemsize = src.itemsize.max(1);
    let elements = (len / itemsize).max(0) as usize;
    let mut dst_offset = 0usize;
    for _ in 0..elements {
        let source_ptr = unsafe { PyBuffer_GetPointer(src, indices.as_ptr()) };
        if source_ptr.is_null() {
            return -1;
        }
        // SAFETY: source and destination each have `itemsize` bytes for this element.
        unsafe {
            std::ptr::copy_nonoverlapping(
                source_ptr.cast::<u8>(),
                buf.cast::<u8>().add(dst_offset),
                itemsize as usize,
            )
        };
        dst_offset += itemsize as usize;
        if use_fortran {
            cpython_buffer_add_one_index_f(&mut indices, &shape);
        } else {
            cpython_buffer_add_one_index_c(&mut indices, &shape);
        }
    }
    0
}
