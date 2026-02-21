use std::collections::HashMap;
use std::ffi::c_void;

use crate::runtime::{Object, Value};
use crate::vm::{NativeCallResult, NativeMethodKind};

use super::{
    CpythonListCompatObject, CpythonMappingMethods, CpythonSequenceMethods, Py_IncRef,
    cpython_debug_compare_value, cpython_set_error, cpython_value_from_ptr,
    with_active_cpython_context_mut,
};

unsafe extern "C" fn cpython_list_sq_length_slot(list: *mut c_void) -> isize {
    unsafe { PyList_Size(list) }
}

unsafe extern "C" fn cpython_list_sq_item_slot(list: *mut c_void, index: isize) -> *mut c_void {
    unsafe { PyList_GetItemRef(list, index) }
}

unsafe extern "C" fn cpython_list_mp_length_slot(list: *mut c_void) -> isize {
    unsafe { PyList_Size(list) }
}

unsafe extern "C" fn cpython_list_mp_subscript_slot(
    list: *mut c_void,
    key: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("list mp_subscript missing VM context");
            return std::ptr::null_mut();
        }
        let Some(list_value) = context.cpython_value_from_ptr_or_proxy(list) else {
            context.set_error("list mp_subscript received unknown list pointer");
            return std::ptr::null_mut();
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("list mp_subscript received unknown key pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.getitem_value(list_value, key_value) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
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

pub(super) static mut PY_LIST_SEQUENCE_METHODS: CpythonSequenceMethods = CpythonSequenceMethods {
    sq_length: cpython_list_sq_length_slot as *mut c_void,
    sq_concat: std::ptr::null_mut(),
    sq_repeat: std::ptr::null_mut(),
    sq_item: cpython_list_sq_item_slot as *mut c_void,
    was_sq_slice: std::ptr::null_mut(),
    sq_ass_item: std::ptr::null_mut(),
    was_sq_ass_slice: std::ptr::null_mut(),
    sq_contains: std::ptr::null_mut(),
    sq_inplace_concat: std::ptr::null_mut(),
    sq_inplace_repeat: std::ptr::null_mut(),
};

pub(super) static mut PY_LIST_MAPPING_METHODS: CpythonMappingMethods = CpythonMappingMethods {
    mp_length: cpython_list_mp_length_slot as *mut c_void,
    mp_subscript: cpython_list_mp_subscript_slot as *mut c_void,
    mp_ass_subscript: std::ptr::null_mut(),
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_New(size: isize) -> *mut c_void {
    let trace_lists = std::env::var_os("PYRS_TRACE_CPY_LIST").is_some();
    if size < 0 {
        cpython_set_error("PyList_New requires non-negative size");
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let list = vm
            .heap
            .alloc(Object::List(vec![Value::None; size as usize]));
        let ptr = context.alloc_cpython_ptr_for_value(Value::List(list));
        if trace_lists {
            eprintln!("[cpy-list-new] size={} ptr={:p}", size, ptr);
        }
        ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Size(list: *mut c_void) -> isize {
    let trace_lists = std::env::var_os("PYRS_TRACE_CPY_LIST").is_some();
    if trace_lists {
        let _ = with_active_cpython_context_mut(|context| {
            if context.owns_cpython_allocation_ptr(list) {
                // SAFETY: owned list pointers use `CpythonListCompatObject` layout.
                let raw = unsafe { list.cast::<CpythonListCompatObject>().as_ref() };
                if let Some(raw) = raw {
                    eprintln!(
                        "[cpy-list-size-raw] ptr={:p} ob_size={} ob_item={:p} allocated={}",
                        list, raw.ob_base.ob_size, raw.ob_item, raw.allocated
                    );
                }
            }
        });
    }
    match cpython_value_from_ptr(list) {
        Ok(Value::List(list_obj)) => match &*list_obj.kind() {
            Object::List(values) => {
                if trace_lists {
                    eprintln!("[cpy-list-size] ptr={:p} len={}", list, values.len());
                }
                values.len() as isize
            }
            _ => {
                if trace_lists {
                    eprintln!("[cpy-list-size] ptr={:p} invalid list storage", list);
                }
                cpython_set_error("PyList_Size encountered invalid list storage");
                -1
            }
        },
        Ok(_) => {
            if trace_lists {
                eprintln!("[cpy-list-size] ptr={:p} non-list object", list);
            }
            cpython_set_error("PyList_Size expected list object");
            -1
        }
        Err(err) => {
            if trace_lists {
                eprintln!("[cpy-list-size] ptr={:p} lookup error: {}", list, err);
            }
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Append(list: *mut c_void, item: *mut c_void) -> i32 {
    let trace_lists = std::env::var_os("PYRS_TRACE_CPY_LIST").is_some();
    with_active_cpython_context_mut(|context| {
        let item_value = match context.cpython_value_from_ptr_or_proxy(item) {
            Some(value) => value,
            None => {
                if trace_lists {
                    eprintln!(
                        "[cpy-list-append] list={:p} item={:p} unknown item pointer",
                        list, item
                    );
                }
                context.set_error("PyList_Append received unknown item pointer");
                return -1;
            }
        };
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            if trace_lists {
                eprintln!("[cpy-list-append] list={:p} unknown list pointer", list);
            }
            context.set_error("PyList_Append received unknown list pointer");
            return -1;
        };
        {
            let Some(slot) = context.objects.get_mut(&handle) else {
                if trace_lists {
                    eprintln!(
                        "[cpy-list-append] list={:p} handle={} missing slot",
                        list, handle
                    );
                }
                context.set_error("PyList_Append list handle is not available");
                return -1;
            };
            let Value::List(list_obj) = &mut slot.value else {
                if trace_lists {
                    eprintln!(
                        "[cpy-list-append] list={:p} handle={} non-list slot",
                        list, handle
                    );
                }
                context.set_error("PyList_Append expected list object");
                return -1;
            };
            let Object::List(values) = &mut *list_obj.kind_mut() else {
                if trace_lists {
                    eprintln!(
                        "[cpy-list-append] list={:p} handle={} invalid list storage",
                        list, handle
                    );
                }
                context.set_error("PyList_Append encountered invalid list storage");
                return -1;
            };
            if trace_lists {
                eprintln!(
                    "[cpy-list-append] list={:p} handle={} before_len={} item={}",
                    list,
                    handle,
                    values.len(),
                    cpython_debug_compare_value(&item_value)
                );
            }
            values.push(item_value);
            if trace_lists {
                eprintln!(
                    "[cpy-list-append] list={:p} handle={} after_len={}",
                    list,
                    handle,
                    values.len()
                );
            }
        }
        // Keep owned CPython list storage (`ob_size` / `ob_item`) synchronized for native callers
        // that access list internals directly between C-API calls.
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_GetItem(list: *mut c_void, index: isize) -> *mut c_void {
    let trace_lists = std::env::var_os("PYRS_TRACE_CPY_LIST").is_some();
    with_active_cpython_context_mut(|context| {
        if index < 0 {
            context.set_error("PyList_GetItem index out of range");
            return std::ptr::null_mut();
        }
        if context.owns_cpython_allocation_ptr(list) {
            // SAFETY: owned list pointers use `CpythonListCompatObject` layout.
            let raw = unsafe { list.cast::<CpythonListCompatObject>().as_ref() };
            let Some(raw) = raw else {
                context.set_error("PyList_GetItem received invalid list pointer");
                return std::ptr::null_mut();
            };
            let len = raw.ob_base.ob_size.max(0) as usize;
            if (index as usize) >= len || raw.ob_item.is_null() {
                context.set_error("PyList_GetItem index out of range");
                return std::ptr::null_mut();
            }
            // SAFETY: `ob_item` points to at least `len` entries.
            let item = unsafe { *raw.ob_item.add(index as usize) };
            if item.is_null() {
                context.set_error("PyList_GetItem encountered null list slot");
                return std::ptr::null_mut();
            }
            return item;
        }
        let Some(value) = context.cpython_value_from_ptr(list) else {
            if trace_lists {
                eprintln!(
                    "[cpy-list-get-item] ptr={:p} index={} unknown list pointer",
                    list, index
                );
            }
            context.set_error("PyList_GetItem received unknown list pointer");
            return std::ptr::null_mut();
        };
        let Value::List(list_obj) = value else {
            context.set_error("PyList_GetItem expected list object");
            return std::ptr::null_mut();
        };
        let Object::List(values) = &*list_obj.kind() else {
            context.set_error("PyList_GetItem encountered invalid list storage");
            return std::ptr::null_mut();
        };
        let idx = index as usize;
        if idx >= values.len() {
            context.set_error("PyList_GetItem index out of range");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(values[idx].clone())
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_SetItem(list: *mut c_void, index: isize, item: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let item_handle = context.cpython_handle_from_ptr(item);
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            if let Some(item_handle) = item_handle {
                let _ = context.decref(item_handle);
            }
            context.set_error("PyList_SetItem received unknown list pointer");
            return -1;
        };
        if index < 0 {
            if let Some(item_handle) = item_handle {
                let _ = context.decref(item_handle);
            }
            context.set_error("PyList_SetItem index out of range");
            return -1;
        }
        let item_value = match context.cpython_value_from_ptr_or_proxy(item) {
            Some(value) => value,
            None => {
                if let Some(item_handle) = item_handle {
                    let _ = context.decref(item_handle);
                }
                context.set_error("PyList_SetItem received unknown item pointer");
                return -1;
            }
        };
        let mut ok = false;
        {
            let Some(slot) = context.objects.get_mut(&handle) else {
                if let Some(item_handle) = item_handle {
                    let _ = context.decref(item_handle);
                }
                context.set_error("PyList_SetItem list handle is not available");
                return -1;
            };
            let Value::List(list_obj) = &mut slot.value else {
                if let Some(item_handle) = item_handle {
                    let _ = context.decref(item_handle);
                }
                context.set_error("PyList_SetItem expected list object");
                return -1;
            };
            let Object::List(values) = &mut *list_obj.kind_mut() else {
                if let Some(item_handle) = item_handle {
                    let _ = context.decref(item_handle);
                }
                context.set_error("PyList_SetItem encountered invalid list storage");
                return -1;
            };
            let idx = index as usize;
            if idx < values.len() {
                values[idx] = item_value;
                ok = true;
            }
        }
        if let Some(item_handle) = item_handle {
            let _ = context.decref(item_handle);
        }
        if !ok {
            context.set_error("PyList_SetItem index out of range");
            return -1;
        }
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Insert(list: *mut c_void, index: isize, item: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            context.set_error("PyList_Insert received unknown list pointer");
            return -1;
        };
        let item_value = match context.cpython_value_from_ptr_or_proxy(item) {
            Some(value) => value,
            None => {
                context.set_error("PyList_Insert received unknown item pointer");
                return -1;
            }
        };
        {
            let Some(slot) = context.objects.get_mut(&handle) else {
                context.set_error("PyList_Insert list handle is not available");
                return -1;
            };
            let Value::List(list_obj) = &mut slot.value else {
                context.set_error("PyList_Insert expected list object");
                return -1;
            };
            let Object::List(values) = &mut *list_obj.kind_mut() else {
                context.set_error("PyList_Insert encountered invalid list storage");
                return -1;
            };
            let insert_at = if index <= 0 {
                0
            } else {
                (index as usize).min(values.len())
            };
            values.insert(insert_at, item_value);
        }
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_GetSlice(
    list: *mut c_void,
    low: isize,
    high: isize,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_GetSlice missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr(list) else {
            context.set_error("PyList_GetSlice received unknown list pointer");
            return std::ptr::null_mut();
        };
        let Value::List(list_obj) = value else {
            context.set_error("PyList_GetSlice expected list object");
            return std::ptr::null_mut();
        };
        let Object::List(values) = &*list_obj.kind() else {
            context.set_error("PyList_GetSlice encountered invalid list storage");
            return std::ptr::null_mut();
        };
        let len = values.len() as isize;
        let mut start = if low < 0 { low + len } else { low };
        let mut end = if high < 0 { high + len } else { high };
        start = start.clamp(0, len);
        end = end.clamp(0, len);
        let slice = if end >= start {
            values[start as usize..end as usize].to_vec()
        } else {
            Vec::new()
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let result = vm.heap.alloc(Object::List(slice));
        context.alloc_cpython_ptr_for_value(Value::List(result))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_SetSlice(
    list: *mut c_void,
    low: isize,
    high: isize,
    itemlist: *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_SetSlice missing VM context");
            return -1;
        }
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            context.set_error("PyList_SetSlice received unknown list pointer");
            return -1;
        };
        let replacement = if itemlist.is_null() {
            Vec::new()
        } else {
            let replacement_value = match context.cpython_value_from_ptr_or_proxy(itemlist) {
                Some(value) => value,
                None => {
                    context.set_error("PyList_SetSlice received unknown itemlist pointer");
                    return -1;
                }
            };
            // SAFETY: VM pointer is valid for context lifetime.
            let vm = unsafe { &mut *context.vm };
            match replacement_value {
                Value::List(list_obj) => match &*list_obj.kind() {
                    Object::List(values) => values.clone(),
                    _ => {
                        context.set_error("PyList_SetSlice encountered invalid replacement list");
                        return -1;
                    }
                },
                Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                    Object::Tuple(values) => values.clone(),
                    _ => {
                        context.set_error("PyList_SetSlice encountered invalid replacement tuple");
                        return -1;
                    }
                },
                other => match vm.collect_iterable_values(other) {
                    Ok(values) => values,
                    Err(err) => {
                        context.set_error(err.message);
                        return -1;
                    }
                },
            }
        };
        {
            let Some(slot) = context.objects.get_mut(&handle) else {
                context.set_error("PyList_SetSlice list handle is not available");
                return -1;
            };
            let Value::List(list_obj) = &mut slot.value else {
                context.set_error("PyList_SetSlice expected list object");
                return -1;
            };
            let Object::List(values) = &mut *list_obj.kind_mut() else {
                context.set_error("PyList_SetSlice encountered invalid list storage");
                return -1;
            };
            let len = values.len() as isize;
            let mut start = if low < 0 { low + len } else { low };
            let mut end = if high < 0 { high + len } else { high };
            start = start.clamp(0, len);
            end = end.clamp(0, len);
            let start = start as usize;
            let end = end.max(start as isize) as usize;
            values.splice(start..end, replacement);
        }
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Sort(list: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_Sort missing VM context");
            return -1;
        }
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            context.set_error("PyList_Sort received unknown list pointer");
            return -1;
        };
        let list_obj = {
            let Some(slot) = context.objects.get(&handle) else {
                context.set_error("PyList_Sort list handle is not available");
                return -1;
            };
            match &slot.value {
                Value::List(list_obj) => list_obj.clone(),
                _ => {
                    context.set_error("PyList_Sort expected list object");
                    return -1;
                }
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::ListSort,
            list_obj,
            Vec::new(),
            HashMap::new(),
        ) {
            Ok(NativeCallResult::Value(_)) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
            Ok(NativeCallResult::PropagatedException) => {
                let err = vm.runtime_error_from_active_exception("list.sort() failed");
                context.set_error(err.message);
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
pub unsafe extern "C" fn PyList_Reverse(list: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_Reverse missing VM context");
            return -1;
        }
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            context.set_error("PyList_Reverse received unknown list pointer");
            return -1;
        };
        let list_obj = {
            let Some(slot) = context.objects.get(&handle) else {
                context.set_error("PyList_Reverse list handle is not available");
                return -1;
            };
            match &slot.value {
                Value::List(list_obj) => list_obj.clone(),
                _ => {
                    context.set_error("PyList_Reverse expected list object");
                    return -1;
                }
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::ListReverse,
            list_obj,
            Vec::new(),
            HashMap::new(),
        ) {
            Ok(NativeCallResult::Value(_)) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
            Ok(NativeCallResult::PropagatedException) => {
                let err = vm.runtime_error_from_active_exception("list.reverse() failed");
                context.set_error(err.message);
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
pub unsafe extern "C" fn PyList_GetItemRef(list: *mut c_void, index: isize) -> *mut c_void {
    let trace_lists = std::env::var_os("PYRS_TRACE_CPY_LIST").is_some();
    with_active_cpython_context_mut(|context| {
        if index < 0 {
            context.set_error("PyList_GetItemRef index out of range");
            return std::ptr::null_mut();
        }
        if context.owns_cpython_allocation_ptr(list) {
            // SAFETY: owned list pointers use `CpythonListCompatObject` layout.
            let raw = unsafe { list.cast::<CpythonListCompatObject>().as_ref() };
            let Some(raw) = raw else {
                context.set_error("PyList_GetItemRef received invalid list pointer");
                return std::ptr::null_mut();
            };
            let len = raw.ob_base.ob_size.max(0) as usize;
            if (index as usize) >= len || raw.ob_item.is_null() {
                context.set_error("PyList_GetItemRef index out of range");
                return std::ptr::null_mut();
            }
            // SAFETY: `ob_item` points to at least `len` entries.
            let item = unsafe { *raw.ob_item.add(index as usize) };
            if item.is_null() {
                context.set_error("PyList_GetItemRef encountered null list slot");
                return std::ptr::null_mut();
            }
            // SAFETY: item is a live PyObject* by list storage contract.
            unsafe { Py_IncRef(item) };
            return item;
        }
        let Some(value) = context.cpython_value_from_ptr(list) else {
            if trace_lists {
                eprintln!(
                    "[cpy-list-get-item-ref] ptr={:p} index={} unknown list pointer",
                    list, index
                );
            }
            context.set_error("PyList_GetItemRef received unknown list pointer");
            return std::ptr::null_mut();
        };
        let Value::List(list_obj) = value else {
            if trace_lists {
                eprintln!(
                    "[cpy-list-get-item-ref] ptr={:p} index={} non-list object",
                    list, index
                );
            }
            context.set_error("PyList_GetItemRef expected list object");
            return std::ptr::null_mut();
        };
        let Object::List(values) = &*list_obj.kind() else {
            if trace_lists {
                eprintln!(
                    "[cpy-list-get-item-ref] ptr={:p} index={} invalid list storage",
                    list, index
                );
            }
            context.set_error("PyList_GetItemRef encountered invalid list storage");
            return std::ptr::null_mut();
        };
        if trace_lists {
            eprintln!(
                "[cpy-list-get-item-ref] ptr={:p} index={} len={}",
                list,
                index,
                values.len()
            );
        }
        let idx = index as usize;
        if idx >= values.len() {
            context.set_error("PyList_GetItemRef index out of range");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(values[idx].clone())
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_AsTuple(list: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_AsTuple missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr(list) else {
            context.set_error("PyList_AsTuple received unknown list pointer");
            return std::ptr::null_mut();
        };
        let Value::List(list_obj) = value else {
            context.set_error("PyList_AsTuple expected list object");
            return std::ptr::null_mut();
        };
        let Object::List(values) = &*list_obj.kind() else {
            context.set_error("PyList_AsTuple encountered invalid list storage");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let tuple = vm.heap.alloc(Object::Tuple(values.clone()));
        context.alloc_cpython_ptr_for_value(Value::Tuple(tuple))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}
