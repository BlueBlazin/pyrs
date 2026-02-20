use std::ffi::c_void;

use crate::runtime::{Object, Value};

use super::{
    CpythonObjectHead, CpythonTypeObject, CpythonVarObjectHead, ModuleCapiContext,
    c_name_to_string, cpython_debug_compare_value, cpython_set_error, cpython_tuple_items_ptr,
    cpython_value_from_ptr, with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_New(size: isize) -> *mut c_void {
    if size < 0 {
        cpython_set_error("PyTuple_New requires non-negative size");
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyTuple_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let tuple = vm
            .heap
            .alloc(Object::Tuple(vec![Value::None; size as usize]));
        let ptr = context.alloc_cpython_ptr_for_value(Value::Tuple(tuple));
        if std::env::var_os("PYRS_TRACE_CPY_TUPLE").is_some() {
            eprintln!("[cpy-tuple] new size={} ptr={:p}", size, ptr);
        }
        ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_Size(tuple: *mut c_void) -> isize {
    if tuple.is_null() {
        cpython_set_error("PyTuple_Size expected tuple object");
        return -1;
    }
    if let Ok(Some(size)) = with_active_cpython_context_mut(|context| {
        if context.owns_cpython_allocation_ptr(tuple) {
            // SAFETY: owned tuple pointers use CPython-compatible varobject header.
            let size = unsafe { (*tuple.cast::<CpythonVarObjectHead>()).ob_size };
            return Some(size.max(0));
        }
        None
    }) {
        return size;
    }
    match cpython_value_from_ptr(tuple) {
        Ok(Value::Tuple(tuple_obj)) => match &*tuple_obj.kind() {
            Object::Tuple(values) => values.len() as isize,
            _ => {
                cpython_set_error("PyTuple_Size encountered invalid tuple storage");
                -1
            }
        },
        Ok(_) => {
            cpython_set_error("PyTuple_Size expected tuple object");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_GetItem(tuple: *mut c_void, index: isize) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.owns_cpython_allocation_ptr(tuple) {
            // SAFETY: owned tuple pointers use CPython-compatible varobject header
            // followed by contiguous `PyObject*` item slots.
            unsafe {
                let head = tuple.cast::<CpythonVarObjectHead>();
                let len = (*head).ob_size.max(0) as usize;
                let idx = if index < 0 {
                    len as isize + index
                } else {
                    index
                };
                if idx < 0 || idx as usize >= len {
                    context.set_error("PyTuple_GetItem index out of range");
                    return std::ptr::null_mut();
                }
                let items_ptr = cpython_tuple_items_ptr(tuple);
                return *items_ptr.add(idx as usize);
            }
        }
        let Some(value) = context.cpython_value_from_ptr(tuple) else {
            context.set_error("PyTuple_GetItem received unknown tuple pointer");
            return std::ptr::null_mut();
        };
        let Value::Tuple(tuple_obj) = value else {
            context.set_error("PyTuple_GetItem expected tuple object");
            return std::ptr::null_mut();
        };
        let Object::Tuple(values) = &*tuple_obj.kind() else {
            context.set_error("PyTuple_GetItem encountered invalid tuple storage");
            return std::ptr::null_mut();
        };
        let idx = if index < 0 {
            values.len() as isize + index
        } else {
            index
        };
        if idx < 0 || idx as usize >= values.len() {
            context.set_error("PyTuple_GetItem index out of range");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(values[idx as usize].clone())
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_SetItem(
    tuple: *mut c_void,
    index: isize,
    item: *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        let item_handle = context.cpython_handle_from_ptr(item);
        let Some(handle) = context.cpython_handle_from_ptr(tuple) else {
            context.set_error("PyTuple_SetItem received unknown tuple pointer");
            return -1;
        };
        let tuple_owned = context.owns_cpython_allocation_ptr(tuple);
        let item_value = match context.cpython_value_from_stolen_ptr(item) {
            Some(value) => value,
            None => {
                // SAFETY: best-effort diagnostics for unknown tuple item pointers.
                let (
                    type_ptr,
                    type_name,
                    item_refcnt,
                    type_refcnt,
                    type_flags,
                    type_metatype_ptr,
                    type_metatype_refcnt,
                    type_metatype_flags,
                ) = unsafe {
                    let item_head = item.cast::<CpythonObjectHead>().as_ref();
                    let item_refcnt = item_head.map(|head| head.ob_refcnt).unwrap_or(0);
                    let type_ptr = item_head
                        .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                        .unwrap_or(std::ptr::null_mut());
                    if type_ptr.is_null() {
                        (
                            std::ptr::null_mut(),
                            "<null>".to_string(),
                            item_refcnt,
                            0,
                            0,
                            std::ptr::null_mut(),
                            0,
                            0,
                        )
                    } else {
                        let type_name = c_name_to_string((*type_ptr).tp_name)
                            .unwrap_or_else(|_| "<invalid>".to_string());
                        let type_refcnt = type_ptr
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_refcnt)
                            .unwrap_or(0);
                        let type_flags = (*type_ptr).tp_flags;
                        let type_metatype_ptr = type_ptr
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                            .unwrap_or(std::ptr::null_mut());
                        let (type_metatype_refcnt, type_metatype_flags) = if type_metatype_ptr
                            .is_null()
                        {
                            (0, 0)
                        } else {
                            (
                                type_metatype_ptr
                                    .cast::<CpythonObjectHead>()
                                    .as_ref()
                                    .map(|head| head.ob_refcnt)
                                    .unwrap_or(0),
                                (*type_metatype_ptr).tp_flags,
                            )
                        };
                        (
                            type_ptr,
                            type_name,
                            item_refcnt,
                            type_refcnt,
                            type_flags,
                            type_metatype_ptr,
                            type_metatype_refcnt,
                            type_metatype_flags,
                        )
                    }
                };
                let probable = ModuleCapiContext::is_probable_external_cpython_object_ptr(item);
                context.set_error(format!(
                    "PyTuple_SetItem received unknown item pointer ptr={:p} type={:p} type_name={} probable_external={} item_refcnt={} type_refcnt={} type_flags=0x{:x} type_metatype={:p} type_metatype_refcnt={} type_metatype_flags=0x{:x}",
                    item,
                    type_ptr,
                    type_name,
                    probable,
                    item_refcnt,
                    type_refcnt,
                    type_flags,
                    type_metatype_ptr,
                    type_metatype_refcnt,
                    type_metatype_flags
                ));
                return -1;
            }
        };
        if std::env::var_os("PYRS_TRACE_CPY_TUPLE_SET").is_some() {
            eprintln!(
                "[cpy-tuple-set] tuple={:p} idx={} item_ptr={:p} item={}",
                tuple,
                index,
                item,
                cpython_debug_compare_value(&item_value)
            );
        }
        let mut status = 0;
        {
            let Some(slot) = context.objects.get_mut(&handle) else {
                context.set_error("PyTuple_SetItem tuple handle is not available");
                return -1;
            };
            let Value::Tuple(tuple_obj) = &mut slot.value else {
                context.set_error("PyTuple_SetItem expected tuple object");
                return -1;
            };
            let Object::Tuple(values) = &mut *tuple_obj.kind_mut() else {
                context.set_error("PyTuple_SetItem encountered invalid tuple storage");
                return -1;
            };
            let idx = if index < 0 {
                values.len() as isize + index
            } else {
                index
            };
            if idx < 0 || idx as usize >= values.len() {
                status = -1;
            } else {
                values[idx as usize] = item_value;
            }
        }
        if status != 0 {
            context.set_error("PyTuple_SetItem index out of range");
            return -1;
        }
        if tuple_owned {
            // SAFETY: owned tuple pointers use CPython-compatible varobject header
            // followed by contiguous `PyObject*` item slots.
            unsafe {
                let head = tuple.cast::<CpythonVarObjectHead>();
                let capacity = (*head).ob_size.max(0) as usize;
                let idx = if index < 0 {
                    capacity as isize + index
                } else {
                    index
                };
                if idx >= 0 && (idx as usize) < capacity {
                    let items_ptr = cpython_tuple_items_ptr(tuple);
                    *items_ptr.add(idx as usize) = item;
                }
            }
        }
        if let Some(item_handle) = item_handle {
            let _ = context.decref(item_handle);
        }
        if std::env::var_os("PYRS_TRACE_CPY_TUPLE").is_some() {
            eprintln!(
                "[cpy-tuple] set ptr={:p} index={} item={:p}",
                tuple, index, item
            );
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_GetSlice(
    tuple: *mut c_void,
    low: isize,
    high: isize,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyTuple_GetSlice missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr(tuple) else {
            context.set_error("PyTuple_GetSlice received unknown tuple pointer");
            return std::ptr::null_mut();
        };
        let Value::Tuple(tuple_obj) = value else {
            context.set_error("PyTuple_GetSlice expected tuple object");
            return std::ptr::null_mut();
        };
        let Object::Tuple(values) = &*tuple_obj.kind() else {
            context.set_error("PyTuple_GetSlice encountered invalid tuple storage");
            return std::ptr::null_mut();
        };
        let len = values.len() as isize;
        let start = low.clamp(0, len) as usize;
        let end = high.clamp(0, len) as usize;
        let slice = if end >= start {
            values[start..end].to_vec()
        } else {
            Vec::new()
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let result = vm.heap.alloc(Object::Tuple(slice));
        context.alloc_cpython_ptr_for_value(Value::Tuple(result))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}
