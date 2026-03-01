use std::collections::HashMap;
use std::ffi::c_void;

use crate::runtime::{BuiltinFunction, Object, Value};
use crate::vm::ops::compare_order;
use crate::vm::{
    InternalCallOutcome, dict_remove_value, dict_set_value_checked, env_var_present_cached,
};

use super::cpython_object_call_api::PyObject_IsTrue;
use super::{
    _Py_NotImplementedStruct, CpythonObjectHead, CpythonTypeObject, CpythonVarObjectHead,
    ModuleCapiContext, PY_TPFLAGS_TYPE_SUBCLASS, Py_DecRef, PyErr_BadInternalCall, PyErr_Clear,
    PyErr_ExceptionMatches, PyErr_Occurred, PyExc_AttributeError, PyExc_IndexError, PyExc_KeyError,
    PyExc_TypeError, PyLong_AsSsize_t, PyObject_GetAttr, PyObject_GetAttrString, PyType_IsSubtype,
    PyType_Type, PyUnicode_Type, c_name_to_string, capi_perf_inc_richcompare_bool_calls,
    capi_perf_inc_richcompare_calls, capi_perf_inc_richcompare_dunder_attr_missing,
    capi_perf_inc_richcompare_dunder_callable_invocations,
    capi_perf_inc_richcompare_dunder_calls_external, capi_perf_inc_richcompare_dunder_calls_owned,
    capi_perf_inc_richcompare_dunder_fallback_attempts, capi_perf_inc_richcompare_slot_attempts,
    cpython_call_builtin, cpython_call_object, cpython_error_message_indicates_missing_attribute,
    cpython_is_interned_unicode_ptr, cpython_lookup_interned_unicode_text,
    cpython_mapping_ass_subscript_slot, cpython_mapping_subscript_slot, cpython_new_ptr_for_value,
    cpython_sequence_item_slot, cpython_set_error, cpython_set_typed_error,
    cpython_slice_bounds_step_one, cpython_slice_indices_for_len,
    cpython_trace_numpy_reduce_enabled, cpython_try_richcompare_slot, cpython_tuple_items_ptr,
    cpython_unicode_text_from_value, cpython_value_debug_tag, cpython_value_from_ptr, is_truthy,
    value_to_int, with_active_cpython_context_mut,
};

fn cpython_set_item_runtime_error(message: String) {
    if message.starts_with("IndexError:")
        || message.contains("index out of range")
        || message.contains("out of bounds for axis")
    {
        let detail = message
            .strip_prefix("IndexError:")
            .map(str::trim)
            .unwrap_or(message.as_str());
        cpython_set_typed_error(unsafe { PyExc_IndexError }, detail);
        return;
    }
    if message.starts_with("KeyError:") || message.contains("key not found") {
        let detail = message
            .strip_prefix("KeyError:")
            .map(str::trim)
            .unwrap_or(message.as_str());
        cpython_set_typed_error(unsafe { PyExc_KeyError }, detail);
        return;
    }
    cpython_set_error(message);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetItem(object: *mut c_void, key: *mut c_void) -> *mut c_void {
    let trace_getitem = env_var_present_cached("PYRS_TRACE_CPY_GETITEM");
    if trace_getitem {
        let (object_tag, key_desc) = with_active_cpython_context_mut(|context| {
            let object_tag = context
                .cpython_value_from_ptr_or_proxy(object)
                .map(|value| cpython_value_debug_tag(&value))
                .unwrap_or_else(|| "<unknown>".to_string());
            context
                .cpython_value_from_ptr_or_proxy(key)
                .map(|value| cpython_debug_compare_value(&value))
                .map(|key| (object_tag.clone(), key))
                .unwrap_or_else(|| (object_tag, "<unknown>".to_string()))
        })
        .unwrap_or_else(|_| ("<no-context>".to_string(), "<no-context>".to_string()));
        eprintln!(
            "[cpy-getitem] object_ptr={:p} key_ptr={:p} object={} key={}",
            object, key, object_tag, key_desc
        );
    }
    let result = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_GetItem missing VM context");
            return std::ptr::null_mut();
        }
        if !object.is_null()
            // SAFETY: pointer shape checks + slot reads guard this fast-path.
            && let Some(result_ptr) = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .and_then(|head| {
                        let type_ptr = head.ob_type.cast::<CpythonTypeObject>();
                        if let Some(subscript_slot) = cpython_mapping_subscript_slot(type_ptr) {
                            Some(subscript_slot(object, key))
                        } else if let Some(item_slot) = cpython_sequence_item_slot(type_ptr) {
                            let idx = PyLong_AsSsize_t(key);
                            if idx == -1 && !PyErr_Occurred().is_null() {
                                Some(std::ptr::null_mut())
                            } else {
                                Some(item_slot(object, idx))
                            }
                        } else {
                            None
                        }
                    })
            }
        {
            if !result_ptr.is_null() {
                return result_ptr;
            }
            if context.current_error.is_some()
                || context.last_error.is_some()
                || !unsafe { PyErr_Occurred() }.is_null()
            {
                return std::ptr::null_mut();
            }
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_GetItem received unknown object pointer");
            return std::ptr::null_mut();
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyObject_GetItem received unknown key pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.getitem_value(object_value, key_value) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                cpython_set_item_runtime_error(err.message);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    });
    if result.is_null() && unsafe { PyErr_Occurred() }.is_null() {
        let _ = with_active_cpython_context_mut(|context| {
            let existing = context
                .last_error
                .clone()
                .or_else(|| context.first_error.clone())
                .unwrap_or_default();
            if existing.is_empty() {
                context.set_error("PyObject_GetItem returned NULL without setting an exception");
            }
        });
    }
    if trace_getitem {
        if result.is_null() {
            let detail = with_active_cpython_context_mut(|context| {
                context
                    .last_error
                    .clone()
                    .or_else(|| context.first_error.clone())
                    .unwrap_or_else(|| "<none>".to_string())
            })
            .unwrap_or_else(|_| "<no-context>".to_string());
            eprintln!("[cpy-getitem] result=<null> error={}", detail);
        } else {
            let result_tag = with_active_cpython_context_mut(|context| {
                context
                    .cpython_value_from_ptr_or_proxy(result)
                    .map(|value| cpython_value_debug_tag(&value))
                    .unwrap_or_else(|| "<unknown>".to_string())
            })
            .unwrap_or_else(|_| "<no-context>".to_string());
            eprintln!("[cpy-getitem] result={:p} tag={}", result, result_tag);
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetItem(
    object: *mut c_void,
    key: *mut c_void,
    value: *mut c_void,
) -> i32 {
    let trace_setitem = env_var_present_cached("PYRS_TRACE_CPY_SETITEM");
    if trace_setitem {
        let (obj_desc, key_desc, val_desc) = with_active_cpython_context_mut(|context| {
            let obj = context
                .cpython_value_from_ptr_or_proxy(object)
                .map(|v| cpython_value_debug_tag(&v))
                .unwrap_or_else(|| "<unknown>".to_string());
            let key = context
                .cpython_value_from_ptr_or_proxy(key)
                .map(|v| cpython_value_debug_tag(&v))
                .unwrap_or_else(|| "<unknown>".to_string());
            let val = context
                .cpython_value_from_ptr_or_proxy(value)
                .map(|v| cpython_value_debug_tag(&v))
                .unwrap_or_else(|| "<unknown>".to_string());
            (obj, key, val)
        })
        .unwrap_or_else(|_| {
            (
                "<no-context>".to_string(),
                "<no-context>".to_string(),
                "<no-context>".to_string(),
            )
        });
        eprintln!(
            "[cpy-setitem] object={:p} key={:p} value={:p} obj={} key_desc={} value_desc={}",
            object, key, value, obj_desc, key_desc, val_desc
        );
    }
    let status = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_SetItem missing VM context");
            return -1;
        }
        let module_target = context.module_dict_module_for_ptr(object);
        // Prefer native mapping assign slot for external/proxy objects.
        if !object.is_null()
            // SAFETY: pointer shape checks + slot reads guard this fast-path.
            && let Some(status) = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .and_then(|head| {
                        cpython_mapping_ass_subscript_slot(head.ob_type.cast::<CpythonTypeObject>())
                    })
                    .map(|assign_slot| assign_slot(object, key, value))
            }
        {
            if status == 0 {
                return 0;
            }
            if context.current_error.is_none() {
                context.set_error("object does not support item assignment");
            }
            return -1;
        }
        let object_handle = context.cpython_handle_from_ptr(object);
        let Some(target) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_SetItem received unknown object pointer");
            return -1;
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyObject_SetItem received unknown key pointer");
            return -1;
        };
        let Some(item_value) = context.cpython_value_from_ptr_or_proxy(value) else {
            context.set_error("PyObject_SetItem received unknown value pointer");
            return -1;
        };
        let trace_pyx_capi_enabled = env_var_present_cached("PYRS_TRACE_PYX_CAPI");
        let trace_pyx_capi_item =
            trace_pyx_capi_enabled && matches!(&key_value, Value::Str(name) if name == "__pyx_capi__");
        let trace_module_dict_set = trace_pyx_capi_enabled && module_target.is_some();
        if trace_module_dict_set {
            eprintln!(
                "[pyx-capi] module-dict-set object={:p} key={} value_tag={}",
                object,
                cpython_value_debug_tag(&key_value),
                cpython_value_debug_tag(&item_value)
            );
        }
        if trace_pyx_capi_item {
            eprintln!(
                "[pyx-capi] PyObject_SetItem object={:p} target={} module_dict={} value_tag={}",
                object,
                cpython_value_debug_tag(&target),
                module_target.is_some(),
                cpython_value_debug_tag(&item_value)
            );
        }
        match &target {
            Value::Dict(dict_obj) => {
                let key_for_module = key_value.clone();
                let item_for_module = item_value.clone();
                return match dict_set_value_checked(dict_obj, key_value, item_value) {
                    Ok(()) => {
                        if let Some(module_obj) = module_target
                            && let Value::Str(name) = key_for_module
                            && let Object::Module(module_data) = &mut *module_obj.kind_mut()
                        {
                            module_data.globals.insert(name, item_for_module);
                        }
                        if trace_pyx_capi_item {
                            eprintln!(
                                "[pyx-capi] PyObject_SetItem dict-path object={:p} status=0",
                                object
                            );
                        }
                        0
                    }
                    Err(err) => {
                        if trace_pyx_capi_item {
                            eprintln!(
                                "[pyx-capi] PyObject_SetItem dict-path object={:p} status=-1 err={}",
                                object, err.message
                            );
                        }
                        context.set_error(err.message);
                        -1
                    }
                }
                ;
            }
            Value::List(list_obj) => {
                if let Ok(raw_idx) = value_to_int(key_value.clone()) {
                    {
                        let mut list_kind = list_obj.kind_mut();
                        let Object::List(values) = &mut *list_kind else {
                            context.set_error("PyObject_SetItem encountered invalid list storage");
                            return -1;
                        };
                        let mut idx = raw_idx as isize;
                        if idx < 0 {
                            idx += values.len() as isize;
                        }
                        if idx < 0 || idx as usize >= values.len() {
                            context.set_error("index out of range");
                            return -1;
                        }
                        values[idx as usize] = item_value;
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
                if let Value::Slice(slice_value) = &key_value {
                    let replacement_values = {
                        // SAFETY: VM pointer is valid for context lifetime.
                        let vm = unsafe { &mut *context.vm };
                        match vm.collect_iterable_values(item_value.clone()) {
                            Ok(values) => values,
                            Err(err) => {
                                context.set_error(err.message);
                                return -1;
                            }
                        }
                    };
                    {
                        let mut list_kind = list_obj.kind_mut();
                        let Object::List(values) = &mut *list_kind else {
                            context.set_error("PyObject_SetItem encountered invalid list storage");
                            return -1;
                        };
                        let step = slice_value.step.unwrap_or(1);
                        if step == 1 {
                            let (start, stop) = cpython_slice_bounds_step_one(
                                values.len(),
                                slice_value.lower,
                                slice_value.upper,
                            );
                            values.splice(start..stop, replacement_values);
                        } else {
                            let indices = match cpython_slice_indices_for_len(
                                values.len(),
                                slice_value.lower,
                                slice_value.upper,
                                slice_value.step,
                            ) {
                                Ok(indices) => indices,
                                Err(err) => {
                                    context.set_error(err);
                                    return -1;
                                }
                            };
                            if indices.len() != replacement_values.len() {
                                context.set_error(
                                    "attempt to assign sequence of size to extended slice of different size",
                                );
                                return -1;
                            }
                            for (idx, item) in indices.into_iter().zip(replacement_values.into_iter()) {
                                values[idx] = item;
                            }
                        }
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
            }
            Value::ByteArray(bytearray_obj) => {
                if let Ok(raw_idx) = value_to_int(key_value.clone()) {
                    {
                        let mut bytes_kind = bytearray_obj.kind_mut();
                        let Object::ByteArray(values) = &mut *bytes_kind else {
                            context.set_error(
                                "PyObject_SetItem encountered invalid bytearray storage",
                            );
                            return -1;
                        };
                        let mut idx = raw_idx as isize;
                        if idx < 0 {
                            idx += values.len() as isize;
                        }
                        if idx < 0 || idx as usize >= values.len() {
                            context.set_error("index out of range");
                            return -1;
                        }
                        let byte = match value_to_int(item_value.clone()) {
                            Ok(value) => value,
                            Err(err) => {
                                context.set_error(err.message);
                                return -1;
                            }
                        };
                        if !(0..=255).contains(&byte) {
                            context.set_error("byte must be in range(0, 256)");
                            return -1;
                        }
                        values[idx as usize] = byte as u8;
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
            }
            _ => {}
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(setitem) = (match vm.lookup_bound_special_method(&target, "__setitem__") {
            Ok(method) => method,
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        }) else {
            context.set_error("object does not support item assignment");
            return -1;
        };
        match vm.call_internal(setitem, vec![key_value, item_value], HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => {
                if let Some(handle) = object_handle {
                    context.sync_cpython_storage_from_value(handle);
                }
                0
            }
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(vm.runtime_error_from_active_exception("object_set_item() failed").message);
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
    });
    if status < 0 && unsafe { PyErr_Occurred() }.is_null() {
        let _ = with_active_cpython_context_mut(|context| {
            context.set_error("PyObject_SetItem returned -1 without setting an exception");
        });
    }
    if trace_setitem {
        let occurred = unsafe { PyErr_Occurred() };
        let detail = with_active_cpython_context_mut(|context| {
            context
                .last_error
                .clone()
                .or_else(|| context.first_error.clone())
                .unwrap_or_else(|| "<none>".to_string())
        })
        .unwrap_or_else(|_| "<no-context>".to_string());
        eprintln!(
            "[cpy-setitem] status={} occurred={:p} detail={}",
            status, occurred, detail
        );
    }
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelItem(object: *mut c_void, key: *mut c_void) -> i32 {
    let status = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_DelItem missing VM context");
            return -1;
        }
        let module_target = context.module_dict_module_for_ptr(object);
        let object_handle = context.cpython_handle_from_ptr(object);
        let Some(target) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_DelItem received unknown object pointer");
            return -1;
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyObject_DelItem received unknown key pointer");
            return -1;
        };
        match &target {
            Value::Dict(dict_obj) => {
                if dict_remove_value(dict_obj, &key_value).is_some() {
                    if let Some(module_obj) = module_target
                        && let Value::Str(name) = &key_value
                        && let Object::Module(module_data) = &mut *module_obj.kind_mut()
                    {
                        module_data.globals.remove(name);
                    }
                    return 0;
                }
                context.set_error("dict key not found");
                return -1;
            }
            Value::List(list_obj) => {
                if let Ok(raw_idx) = value_to_int(key_value.clone()) {
                    {
                        let mut list_kind = list_obj.kind_mut();
                        let Object::List(values) = &mut *list_kind else {
                            context.set_error("PyObject_DelItem encountered invalid list storage");
                            return -1;
                        };
                        let mut idx = raw_idx as isize;
                        if idx < 0 {
                            idx += values.len() as isize;
                        }
                        if idx < 0 || idx as usize >= values.len() {
                            context.set_error("index out of range");
                            return -1;
                        }
                        values.remove(idx as usize);
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
                if let Value::Slice(slice_value) = &key_value {
                    {
                        let mut list_kind = list_obj.kind_mut();
                        let Object::List(values) = &mut *list_kind else {
                            context.set_error("PyObject_DelItem encountered invalid list storage");
                            return -1;
                        };
                        let step = slice_value.step.unwrap_or(1);
                        if step == 1 {
                            let (start, stop) = cpython_slice_bounds_step_one(
                                values.len(),
                                slice_value.lower,
                                slice_value.upper,
                            );
                            values.drain(start..stop);
                        } else {
                            let mut indices = match cpython_slice_indices_for_len(
                                values.len(),
                                slice_value.lower,
                                slice_value.upper,
                                slice_value.step,
                            ) {
                                Ok(indices) => indices,
                                Err(err) => {
                                    context.set_error(err);
                                    return -1;
                                }
                            };
                            indices.sort_unstable();
                            indices.dedup();
                            for idx in indices.into_iter().rev() {
                                values.remove(idx);
                            }
                        }
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
            }
            Value::ByteArray(bytearray_obj) => {
                if let Ok(raw_idx) = value_to_int(key_value.clone()) {
                    {
                        let mut bytes_kind = bytearray_obj.kind_mut();
                        let Object::ByteArray(values) = &mut *bytes_kind else {
                            context.set_error(
                                "PyObject_DelItem encountered invalid bytearray storage",
                            );
                            return -1;
                        };
                        let mut idx = raw_idx as isize;
                        if idx < 0 {
                            idx += values.len() as isize;
                        }
                        if idx < 0 || idx as usize >= values.len() {
                            context.set_error("index out of range");
                            return -1;
                        }
                        values.remove(idx as usize);
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
            }
            _ => {}
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(delitem) = (match vm.lookup_bound_special_method(&target, "__delitem__") {
            Ok(method) => method,
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        }) else {
            context.set_error("object does not support item deletion");
            return -1;
        };
        match vm.call_internal(delitem, vec![key_value], HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => {
                if let Some(handle) = object_handle {
                    context.sync_cpython_storage_from_value(handle);
                }
                0
            }
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("object_del_item() failed")
                        .message,
                );
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
    });
    if status < 0 && unsafe { PyErr_Occurred() }.is_null() {
        let _ = with_active_cpython_context_mut(|context| {
            context.set_error("PyObject_DelItem returned -1 without setting an exception");
        });
    }
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Size(object: *mut c_void) -> isize {
    let size = with_active_cpython_context_mut(|context| {
        if !object.is_null()
            && (object as usize) >= super::MIN_VALID_PTR_THRESHOLD
            && (object as usize) % std::mem::align_of::<usize>() == 0
        {
            // SAFETY: pointer shape validated above; slot calls follow CPython slot ABI.
            unsafe {
                if let Some(head) = object.cast::<CpythonObjectHead>().as_ref() {
                    let type_ptr = head.ob_type.cast::<CpythonTypeObject>();
                    if !type_ptr.is_null() {
                        let as_mapping = (*type_ptr)
                            .tp_as_mapping
                            .cast::<super::CpythonMappingMethods>();
                        if !as_mapping.is_null() {
                            let mp_length = (*as_mapping).mp_length;
                            if !mp_length.is_null() {
                                let len_fn: unsafe extern "C" fn(*mut c_void) -> isize =
                                    std::mem::transmute(mp_length);
                                return len_fn(object);
                            }
                        }
                        let as_sequence = (*type_ptr)
                            .tp_as_sequence
                            .cast::<super::CpythonSequenceMethods>();
                        if !as_sequence.is_null() {
                            let sq_length = (*as_sequence).sq_length;
                            if !sq_length.is_null() {
                                let len_fn: unsafe extern "C" fn(*mut c_void) -> isize =
                                    std::mem::transmute(sq_length);
                                return len_fn(object);
                            }
                        }
                    }
                }
            }
        }
        let value = match cpython_value_from_ptr(object) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        };
        match cpython_call_builtin(BuiltinFunction::Len, vec![value]) {
            Ok(Value::Int(size)) => size as isize,
            Ok(Value::BigInt(big)) => big.to_i64().unwrap_or(-1) as isize,
            Ok(_) => {
                context.set_error("PyObject_Size expected integer len() result");
                -1
            }
            Err(err) => {
                context.set_error(err);
                -1
            }
        }
    });
    match size {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Length(object: *mut c_void) -> isize {
    unsafe { PyObject_Size(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_LengthHint(object: *mut c_void, default: isize) -> isize {
    let size = unsafe { PyObject_Size(object) };
    if size < 0 {
        unsafe { PyErr_Clear() };
        default
    } else {
        size
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Hash(object: *mut c_void) -> isize {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    match cpython_call_builtin(BuiltinFunction::Hash, vec![value]) {
        Ok(Value::Int(hash)) => hash as isize,
        Ok(Value::BigInt(hash)) => hash.to_i64().unwrap_or(-1) as isize,
        Ok(_) => {
            cpython_set_error("PyObject_Hash expected integer hash() result");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericHash(object: *mut c_void) -> isize {
    unsafe { PyObject_Hash(object) }
}

pub(super) fn cpython_value_type_name_from_ptr(object: *mut c_void) -> String {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return "object".to_string();
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            return "object".to_string();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        vm.value_type_name_for_error(&value)
    })
    .unwrap_or_else(|_| "object".to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_HashNotImplemented(object: *mut c_void) -> isize {
    let type_name = cpython_value_type_name_from_ptr(object);
    cpython_set_typed_error(
        unsafe { PyExc_TypeError },
        format!("unhashable type: '{type_name}'"),
    );
    -1
}

pub(super) unsafe extern "C" fn cpython_tuple_richcompare_slot(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> *mut c_void {
    if left.is_null() || right.is_null() {
        cpython_set_error("tuple richcompare received null operand");
        return std::ptr::null_mut();
    }
    if op < 0 || op > 5 {
        cpython_set_error("tuple richcompare received invalid compare op");
        return std::ptr::null_mut();
    }
    if left == right {
        let result = match op {
            0 | 4 => false,
            1 | 2 | 5 => true,
            3 => false,
            _ => unreachable!(),
        };
        return cpython_new_ptr_for_value(Value::Bool(result));
    }
    // SAFETY: slot is only installed on tuple type; pointer layout is validated by caller.
    let (left_len, right_len, left_items, right_items) = unsafe {
        let left_var = left.cast::<CpythonVarObjectHead>();
        let right_var = right.cast::<CpythonVarObjectHead>();
        (
            (*left_var).ob_size.max(0) as usize,
            (*right_var).ob_size.max(0) as usize,
            cpython_tuple_items_ptr(left),
            cpython_tuple_items_ptr(right),
        )
    };

    let mut differing_index = None;
    let min_len = left_len.min(right_len);
    for idx in 0..min_len {
        // SAFETY: tuple items are contiguous with length ob_size.
        let (left_item, right_item) = unsafe { (*left_items.add(idx), *right_items.add(idx)) };
        if left_item == right_item {
            continue;
        }
        // Fast-path type-object equality by identity; this avoids expensive recursive
        // rich-compare traffic during NumPy operator table initialization.
        if unsafe { cpython_is_type_object_ptr(left_item) }
            && unsafe { cpython_is_type_object_ptr(right_item) }
        {
            differing_index = Some(idx);
            break;
        }
        let eq = unsafe { PyObject_RichCompareBool(left_item, right_item, 2) };
        if eq < 0 {
            return std::ptr::null_mut();
        }
        if eq == 0 {
            differing_index = Some(idx);
            break;
        }
    }

    if let Some(idx) = differing_index {
        if op == 2 {
            return cpython_new_ptr_for_value(Value::Bool(false));
        }
        if op == 3 {
            return cpython_new_ptr_for_value(Value::Bool(true));
        }
        // SAFETY: idx is < min_len <= left/right lengths.
        let (left_item, right_item) = unsafe { (*left_items.add(idx), *right_items.add(idx)) };
        return unsafe { PyObject_RichCompare(left_item, right_item, op) };
    }

    let len_cmp = left_len.cmp(&right_len);
    let result = match op {
        0 => len_cmp == std::cmp::Ordering::Less,
        1 => len_cmp != std::cmp::Ordering::Greater,
        2 => len_cmp == std::cmp::Ordering::Equal,
        3 => len_cmp != std::cmp::Ordering::Equal,
        4 => len_cmp == std::cmp::Ordering::Greater,
        5 => len_cmp != std::cmp::Ordering::Less,
        _ => unreachable!(),
    };
    cpython_new_ptr_for_value(Value::Bool(result))
}

unsafe fn cpython_is_type_object_ptr(object: *mut c_void) -> bool {
    if object.is_null() {
        return false;
    }
    // SAFETY: read-only header probe and type metadata inspection.
    unsafe {
        let Some(head) = object.cast::<CpythonObjectHead>().as_ref() else {
            return false;
        };
        let metatype = head.ob_type.cast::<CpythonTypeObject>();
        if metatype.is_null() {
            return false;
        }
        let type_ptr = std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>();
        if metatype.cast::<c_void>() == type_ptr {
            return true;
        }
        ((*metatype).tp_flags & PY_TPFLAGS_TYPE_SUBCLASS) != 0
            || PyType_IsSubtype(metatype.cast::<c_void>(), type_ptr) != 0
    }
}

fn cpython_rich_compare_slot_name(op: i32) -> Option<&'static std::ffi::CStr> {
    match op {
        0 => Some(c"__lt__"),
        1 => Some(c"__le__"),
        2 => Some(c"__eq__"),
        3 => Some(c"__ne__"),
        4 => Some(c"__gt__"),
        5 => Some(c"__ge__"),
        _ => None,
    }
}

fn cpython_swapped_compare_op(op: i32) -> Option<i32> {
    match op {
        0 => Some(4),
        1 => Some(5),
        2 => Some(2),
        3 => Some(3),
        4 => Some(0),
        5 => Some(1),
        _ => None,
    }
}

fn cpython_compare_op_symbol(op: i32) -> &'static str {
    match op {
        0 => "<",
        1 => "<=",
        2 => "==",
        3 => "!=",
        4 => ">",
        5 => ">=",
        _ => "?",
    }
}

fn cpython_direct_rich_compare(left: &Value, right: &Value, op: i32) -> Option<bool> {
    match (left, right) {
        (
            Value::Bool(_) | Value::Int(_) | Value::BigInt(_),
            Value::Bool(_) | Value::Int(_) | Value::BigInt(_),
        ) => {
            let ordering = compare_order(left.clone(), right.clone()).ok()?;
            Some(match op {
                0 => ordering == std::cmp::Ordering::Less,
                1 => ordering != std::cmp::Ordering::Greater,
                2 => ordering == std::cmp::Ordering::Equal,
                3 => ordering != std::cmp::Ordering::Equal,
                4 => ordering == std::cmp::Ordering::Greater,
                5 => ordering != std::cmp::Ordering::Less,
                _ => return None,
            })
        }
        (Value::Str(lhs), Value::Str(rhs)) => {
            if op == 2
                && env_var_present_cached("PYRS_TRACE_CPY_STRING_EQ")
                && (lhs.contains("device") || rhs.contains("device"))
            {
                eprintln!(
                    "[cpy-str-eq] lhs={:?} rhs={:?} lhs_len={} rhs_len={}",
                    lhs,
                    rhs,
                    lhs.len(),
                    rhs.len()
                );
            }
            Some(match op {
                0 => lhs < rhs,
                1 => lhs <= rhs,
                2 => lhs == rhs,
                3 => lhs != rhs,
                4 => lhs > rhs,
                5 => lhs >= rhs,
                _ => return None,
            })
        }
        _ => None,
    }
}

pub(super) fn cpython_type_name_for_object_ptr(object: *mut c_void) -> String {
    if object.is_null() {
        return "<null>".to_string();
    }
    // SAFETY: caller provides a potential PyObject pointer and we guard all nulls.
    unsafe {
        let Some(head) = object.cast::<CpythonObjectHead>().as_ref() else {
            return "<unknown>".to_string();
        };
        let ty = head.ob_type.cast::<CpythonTypeObject>();
        if ty.is_null() {
            return "<unknown>".to_string();
        }
        c_name_to_string((*ty).tp_name).unwrap_or_else(|_| "<unnamed>".to_string())
    }
}

fn cpython_is_not_implemented_ptr(value: *mut c_void) -> bool {
    if value.is_null() {
        return false;
    }
    if value == std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast::<c_void>() {
        return true;
    }
    if ModuleCapiContext::is_probable_external_cpython_object_ptr(value) {
        // SAFETY: external pointer passed probability checks above.
        let is_external_not_implemented = unsafe {
            let head = value.cast::<CpythonObjectHead>();
            let Some(head) = head.as_ref() else {
                return false;
            };
            let type_ptr = head.ob_type.cast::<CpythonTypeObject>();
            if type_ptr.is_null() {
                return false;
            }
            c_name_to_string((*type_ptr).tp_name)
                .map(|name| name == "NotImplementedType")
                .unwrap_or(false)
        };
        if is_external_not_implemented {
            return true;
        }
    }
    with_active_cpython_context_mut(|context| {
        let Some(mapped) = context.cpython_value_from_ptr(value) else {
            return false;
        };
        if context.vm.is_null() {
            return false;
        }
        // SAFETY: VM pointer is valid for the C-API context lifetime.
        let vm = unsafe { &*context.vm };
        vm.builtins
            .get("NotImplemented")
            .is_some_and(|not_implemented| *not_implemented == mapped)
    })
    .unwrap_or(false)
}

pub(super) fn cpython_debug_compare_value(value: &Value) -> String {
    match value {
        Value::Tuple(tuple_obj) => {
            if let Object::Tuple(values) = &*tuple_obj.kind() {
                let mut rendered = Vec::with_capacity(values.len());
                for item in values {
                    rendered.push(match item {
                        Value::Class(obj) => format!("Class#{}", obj.id()),
                        Value::Tuple(obj) => format!("Tuple#{}", obj.id()),
                        Value::Int(v) => format!("Int({v})"),
                        Value::Str(text) => format!("Str({text})"),
                        other => format!("{other:?}"),
                    });
                }
                format!("Tuple#{}({})", tuple_obj.id(), rendered.join(","))
            } else {
                format!("Tuple#{}(<invalid>)", tuple_obj.id())
            }
        }
        Value::Class(obj) => format!("Class#{}", obj.id()),
        Value::List(obj) => format!("List#{}", obj.id()),
        Value::Int(v) => format!("Int({v})"),
        Value::Str(text) => format!("Str({text})"),
        other => format!("{other:?}"),
    }
}

fn cpython_debug_tuple_raw_ptrs(
    context: &ModuleCapiContext,
    object: *mut c_void,
) -> Option<String> {
    if object.is_null() || !context.owns_cpython_allocation_ptr(object) {
        return None;
    }
    // SAFETY: owned tuple pointers use CPython-compatible varobject header
    // followed by contiguous `PyObject*` item slots.
    unsafe {
        let head = object.cast::<CpythonVarObjectHead>();
        let len = (*head).ob_size.max(0) as usize;
        if len == 0 {
            return Some("[]".to_string());
        }
        let items_ptr = cpython_tuple_items_ptr(object);
        let mut rendered = Vec::with_capacity(len);
        for idx in 0..len {
            let item = *items_ptr.add(idx);
            rendered.push(format!("{:p}", item));
        }
        Some(format!("[{}]", rendered.join(",")))
    }
}

fn cpython_unicode_text_from_ptr_for_compare(object: *mut c_void) -> Option<String> {
    if cpython_is_interned_unicode_ptr(object) {
        return cpython_lookup_interned_unicode_text(object);
    }
    with_active_cpython_context_mut(|context| {
        context
            .cpython_value_from_ptr(object)
            .and_then(|value| cpython_unicode_text_from_value(&value))
    })
    .ok()
    .flatten()
}

fn cpython_try_unicode_pointer_compare(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> Option<*mut c_void> {
    let result = cpython_try_unicode_pointer_compare_bool(left, right, op)?;
    Some(cpython_new_ptr_for_value(Value::Bool(result)))
}

fn cpython_try_unicode_pointer_compare_bool(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> Option<bool> {
    if op != 2 && op != 3 {
        return None;
    }
    if !unsafe { cpython_is_exact_unicode_object_ptr(left) }
        || !unsafe { cpython_is_exact_unicode_object_ptr(right) }
    {
        return None;
    }
    let left_text = cpython_unicode_text_from_ptr_for_compare(left)?;
    let right_text = cpython_unicode_text_from_ptr_for_compare(right)?;
    let equal = left_text == right_text;
    Some(if op == 2 { equal } else { !equal })
}

unsafe fn cpython_is_exact_unicode_object_ptr(object: *mut c_void) -> bool {
    if object.is_null() {
        return false;
    }
    // SAFETY: read-only type-header probe for exact built-in type match.
    let Some(head) = (unsafe { object.cast::<CpythonObjectHead>().as_ref() }) else {
        return false;
    };
    head.ob_type.cast::<c_void>() == std::ptr::addr_of_mut!(PyUnicode_Type).cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_RichCompare(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> *mut c_void {
    capi_perf_inc_richcompare_calls();
    if left.is_null() || right.is_null() {
        cpython_set_error("PyObject_RichCompare received null operand");
        return std::ptr::null_mut();
    }
    let Some(slot_name) = cpython_rich_compare_slot_name(op) else {
        cpython_set_error("PyObject_RichCompare received invalid compare op");
        return std::ptr::null_mut();
    };
    if (op == 2 || op == 3) && left == right {
        return cpython_new_ptr_for_value(Value::Bool(op == 2));
    }
    if (op == 2 || op == 3)
        && unsafe { cpython_is_type_object_ptr(left) }
        && unsafe { cpython_is_type_object_ptr(right) }
    {
        return cpython_new_ptr_for_value(Value::Bool(if op == 2 {
            left == right
        } else {
            left != right
        }));
    }
    if let Some(result) = cpython_try_unicode_pointer_compare(left, right, op) {
        return result;
    }
    capi_perf_inc_richcompare_slot_attempts();
    if let Some(result) = cpython_try_richcompare_slot(left, right, op) {
        return result;
    }
    let (allow_direct_compare, allow_dunder_fallback) =
        with_active_cpython_context_mut(|context| {
            (
                context.owns_cpython_allocation_ptr(left)
                    && context.owns_cpython_allocation_ptr(right),
                context.owns_cpython_allocation_ptr(left)
                    || context.owns_cpython_allocation_ptr(right),
            )
        })
        .unwrap_or((false, false));
    if !allow_dunder_fallback {
        if let Some(result) = cpython_try_unicode_pointer_compare(left, right, op) {
            return result;
        }
        return match op {
            2 => cpython_new_ptr_for_value(Value::Bool(left == right)),
            3 => cpython_new_ptr_for_value(Value::Bool(left != right)),
            _ => {
                cpython_set_error(format!(
                    "TypeError: '{}' not supported between instances of '{}' and '{}'",
                    cpython_compare_op_symbol(op),
                    cpython_type_name_for_object_ptr(left),
                    cpython_type_name_for_object_ptr(right)
                ));
                std::ptr::null_mut()
            }
        };
    }
    let pre_mapped_values = match with_active_cpython_context_mut(|context| {
        (
            context.cpython_value_from_ptr_or_proxy(left),
            context.cpython_value_from_ptr_or_proxy(right),
        )
    }) {
        Ok(values) => values,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    if allow_direct_compare
        && let (Some(left_value), Some(right_value)) = (&pre_mapped_values.0, &pre_mapped_values.1)
        && let Some(result) = cpython_direct_rich_compare(left_value, right_value, op)
    {
        return cpython_new_ptr_for_value(Value::Bool(result));
    }
    let (left_value, right_value) = match pre_mapped_values {
        (Some(left_value), Some(right_value)) => (left_value, right_value),
        (_left_value, right_value) => {
            if let Some(result) = cpython_try_unicode_pointer_compare(left, right, op) {
                return result;
            }
            if op == 2 || op == 3 {
                let equal = std::ptr::eq(left, right);
                let result = if op == 2 { equal } else { !equal };
                return cpython_new_ptr_for_value(Value::Bool(result));
            }
            if env_var_present_cached("PYRS_TRACE_CPY_UNKNOWN_PTR") {
                if right_value.is_none() {
                    eprintln!("[cpy-rich-unknown] right_ptr={right:p} left_ptr={left:p} op={op}");
                } else {
                    let _ = with_active_cpython_context_mut(|context| {
                        eprintln!(
                            "[cpy-rich-unknown] left_ptr={left:p} right_ptr={right:p} op={op} owns_left={} probable_left={} left_handle={:?}",
                            context.owns_cpython_allocation_ptr(left),
                            ModuleCapiContext::is_probable_external_cpython_object_ptr(left),
                            context.cpython_handle_from_ptr(left),
                        );
                    });
                }
            }
            cpython_set_error("unknown PyObject pointer");
            return std::ptr::null_mut();
        }
    };
    if op == 2 && env_var_present_cached("PYRS_TRACE_CPY_RICH_VALUES") {
        eprintln!(
            "[cpy-rich] left={:?} right={:?}",
            cpython_debug_compare_value(&left_value),
            cpython_debug_compare_value(&right_value)
        );
    }
    if let Some(result) = cpython_direct_rich_compare(&left_value, &right_value, op) {
        if op == 2 && env_var_present_cached("PYRS_TRACE_CPY_RICH_VALUES") {
            eprintln!("[cpy-rich] direct={result}");
        }
        return cpython_new_ptr_for_value(Value::Bool(result));
    }

    enum RichCompareAttempt {
        Missing,
        Value(*mut c_void),
        Error,
    }

    capi_perf_inc_richcompare_dunder_fallback_attempts();
    let try_call = |receiver_ptr: *mut c_void,
                    method_name: &std::ffi::CStr,
                    arg: Value|
     -> RichCompareAttempt {
        let callable = unsafe { PyObject_GetAttrString(receiver_ptr, method_name.as_ptr()) };
        if callable.is_null() {
            capi_perf_inc_richcompare_dunder_attr_missing();
            unsafe { PyErr_Clear() };
            return RichCompareAttempt::Missing;
        }
        let _ = with_active_cpython_context_mut(|context| {
            if context.owns_cpython_allocation_ptr(receiver_ptr) {
                capi_perf_inc_richcompare_dunder_calls_owned();
            } else {
                capi_perf_inc_richcompare_dunder_calls_external();
            }
        });
        capi_perf_inc_richcompare_dunder_callable_invocations();
        let result = cpython_call_object(callable, vec![arg], HashMap::new());
        unsafe { Py_DecRef(callable) };
        if result.is_null() {
            RichCompareAttempt::Error
        } else {
            RichCompareAttempt::Value(result)
        }
    };

    match try_call(left, slot_name, right_value.clone()) {
        RichCompareAttempt::Value(result) => {
            if !cpython_is_not_implemented_ptr(result) {
                return result;
            }
            unsafe { Py_DecRef(result) };
        }
        RichCompareAttempt::Error => return std::ptr::null_mut(),
        RichCompareAttempt::Missing => {}
    }

    let swapped_op = cpython_swapped_compare_op(op).expect("valid compare op has swapped mapping");
    let swapped_slot_name =
        cpython_rich_compare_slot_name(swapped_op).expect("valid compare op has slot");
    match try_call(right, swapped_slot_name, left_value.clone()) {
        RichCompareAttempt::Value(result) => {
            if !cpython_is_not_implemented_ptr(result) {
                return result;
            }
            unsafe { Py_DecRef(result) };
        }
        RichCompareAttempt::Error => return std::ptr::null_mut(),
        RichCompareAttempt::Missing => {}
    }

    match op {
        2 => cpython_new_ptr_for_value(Value::Bool(left == right)),
        3 => cpython_new_ptr_for_value(Value::Bool(left != right)),
        _ => {
            cpython_set_error(format!(
                "TypeError: '{}' not supported between instances of '{}' and '{}'",
                cpython_compare_op_symbol(op),
                cpython_type_name_for_object_ptr(left),
                cpython_type_name_for_object_ptr(right)
            ));
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_RichCompareBool(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> i32 {
    capi_perf_inc_richcompare_bool_calls();
    let trace_compare_errors = env_var_present_cached("PYRS_TRACE_CPY_COMPARE_ERRORS");
    if left == right {
        if op == 2 {
            return 1;
        }
        if op == 3 {
            return 0;
        }
    }
    if (op == 2 || op == 3)
        && unsafe { cpython_is_type_object_ptr(left) }
        && unsafe { cpython_is_type_object_ptr(right) }
    {
        return i32::from(if op == 2 {
            left == right
        } else {
            left != right
        });
    }
    if let Some(result) = cpython_try_unicode_pointer_compare_bool(left, right, op) {
        return i32::from(result);
    }
    capi_perf_inc_richcompare_slot_attempts();
    if let Some(result) = cpython_try_richcompare_slot(left, right, op) {
        if result.is_null() {
            return -1;
        }
        if cpython_is_not_implemented_ptr(result) {
            unsafe { Py_DecRef(result) };
        } else {
            let truth = unsafe { PyObject_IsTrue(result) };
            unsafe { Py_DecRef(result) };
            return truth;
        }
    }
    let trace_compare = env_var_present_cached("PYRS_TRACE_CPY_COMPARE");
    let allow_direct_compare = with_active_cpython_context_mut(|context| {
        context.owns_cpython_allocation_ptr(left) && context.owns_cpython_allocation_ptr(right)
    })
    .unwrap_or(false);
    if allow_direct_compare {
        let direct_compare = with_active_cpython_context_mut(|context| {
            let left_value = context.cpython_value_from_ptr_or_proxy(left);
            let right_value = context.cpython_value_from_ptr_or_proxy(right);
            match (left_value, right_value) {
                (Some(left_value), Some(right_value)) => {
                    cpython_direct_rich_compare(&left_value, &right_value, op)
                }
                _ => None,
            }
        })
        .ok()
        .flatten();
        if let Some(result) = direct_compare {
            return i32::from(result);
        }
    }
    if trace_compare && op == 2 {
        let mut left_raw = String::new();
        let mut right_raw = String::new();
        let left_desc = with_active_cpython_context_mut(|context| {
            left_raw = cpython_debug_tuple_raw_ptrs(context, left).unwrap_or_default();
            match context.cpython_value_from_ptr(left) {
                Some(value) => cpython_debug_compare_value(&value),
                None => "ERR(unknown)".to_string(),
            }
        })
        .unwrap_or_else(|err| format!("ERR({err})"));
        let right_desc = with_active_cpython_context_mut(|context| {
            right_raw = cpython_debug_tuple_raw_ptrs(context, right).unwrap_or_default();
            match context.cpython_value_from_ptr(right) {
                Some(value) => cpython_debug_compare_value(&value),
                None => "ERR(unknown)".to_string(),
            }
        })
        .unwrap_or_else(|err| format!("ERR({err})"));
        eprintln!(
            "[cpy-cmp] eq left_ptr={:p} right_ptr={:p} left={} right={} left_raw={} right_raw={}",
            left, right, left_desc, right_desc, left_raw, right_raw
        );
    }
    let value = unsafe { PyObject_RichCompare(left, right, op) };
    if value.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            cpython_set_error("PyObject_RichCompare returned null without setting an exception");
        }
        if trace_compare_errors {
            eprintln!(
                "[cpy-cmp-err] PyObject_RichCompare returned null op={} left={:p} right={:p}",
                op, left, right
            );
        }
        if trace_compare && op == 2 {
            eprintln!("[cpy-cmp] eq result=<null>");
        }
        return -1;
    }
    if cpython_is_not_implemented_ptr(value) {
        unsafe { Py_DecRef(value) };
        return match op {
            2 => i32::from(left == right),
            3 => i32::from(left != right),
            _ => {
                cpython_set_error(format!(
                    "TypeError: '{}' not supported between instances of '{}' and '{}'",
                    cpython_compare_op_symbol(op),
                    cpython_type_name_for_object_ptr(left),
                    cpython_type_name_for_object_ptr(right)
                ));
                -1
            }
        };
    }
    let truth = unsafe { PyObject_IsTrue(value) };
    unsafe { Py_DecRef(value) };
    if truth < 0 && unsafe { PyErr_Occurred() }.is_null() {
        cpython_set_error("PyObject_IsTrue failed without setting an exception");
    }
    if trace_compare_errors && truth < 0 {
        eprintln!(
            "[cpy-cmp-err] PyObject_IsTrue failed op={} left={:p} right={:p}",
            op, left, right
        );
    }
    if trace_compare && op == 2 {
        eprintln!("[cpy-cmp] eq truth={truth}");
    }
    truth
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsInstance(object: *mut c_void, class: *mut c_void) -> i32 {
    if env_var_present_cached("PYRS_TRACE_ISINSTANCE") {
        eprintln!(
            "[cpy-isinstance] raw-enter object={:p} class={:p}",
            object, class
        );
    }
    if object.is_null() || class.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let trace_isinstance = env_var_present_cached("PYRS_TRACE_ISINSTANCE");
    let class_type_name = if trace_isinstance {
        cpython_type_name_for_object_ptr(class)
    } else {
        String::new()
    };
    let should_trace = trace_isinstance;
    if should_trace {
        eprintln!(
            "[cpy-isinstance] enter object={:p} object_type={} class={:p} class_type={}",
            object,
            cpython_type_name_for_object_ptr(object),
            class,
            class_type_name
        );
    }
    with_active_cpython_context_mut(|context| {
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_IsInstance received unknown object pointer");
            return -1;
        };
        let Some(class_value) = context.cpython_value_from_ptr_or_proxy(class) else {
            context.set_error("PyObject_IsInstance received unknown class pointer");
            return -1;
        };
        if context.vm.is_null() {
            context.set_error("PyObject_IsInstance missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_isinstance(vec![object_value, class_value], HashMap::new()) {
            Ok(Value::Bool(flag)) => {
                if should_trace {
                    eprintln!(
                        "[cpy-isinstance] result={} object={:p} class={:p}",
                        flag, object, class
                    );
                }
                i32::from(flag)
            }
            Ok(_) => {
                context.set_error("PyObject_IsInstance returned non-bool result");
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
pub unsafe extern "C" fn PyObject_IsSubclass(subclass: *mut c_void, class: *mut c_void) -> i32 {
    let subclass = match cpython_value_from_ptr(subclass) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let class = match cpython_value_from_ptr(class) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    match cpython_call_builtin(BuiltinFunction::IsSubclass, vec![subclass, class]) {
        Ok(value) => {
            if is_truthy(&value) {
                1
            } else {
                0
            }
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetOptionalAttr(
    object: *mut c_void,
    name: *mut c_void,
    result: *mut *mut c_void,
) -> i32 {
    if result.is_null() {
        cpython_set_error("PyObject_GetOptionalAttr requires non-null result pointer");
        return -1;
    }
    let trace_enabled = cpython_trace_numpy_reduce_enabled();
    let mut trace_name: Option<String> = None;
    if trace_enabled
        && let Ok(value) = cpython_value_from_ptr(name)
        && let Value::Str(text) = value
    {
        trace_name = Some(text);
    }
    if trace_enabled {
        eprintln!(
            "[numpy-reduce] PyObject_GetOptionalAttr object={:p} name_ptr={:p} attr={}",
            object,
            name,
            trace_name.as_deref().unwrap_or("<unmapped>")
        );
    }
    let value = unsafe { PyObject_GetAttr(object, name) };
    if !value.is_null() {
        unsafe {
            *result = value;
        }
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_GetOptionalAttr hit object={:p} attr={} result={:p}",
                object,
                trace_name.as_deref().unwrap_or("<unmapped>"),
                value
            );
        }
        return 1;
    }
    if unsafe { PyErr_Occurred() }.is_null() {
        unsafe {
            *result = std::ptr::null_mut();
        }
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_GetOptionalAttr miss-noerr object={:p} attr={}",
                object,
                trace_name.as_deref().unwrap_or("<unmapped>")
            );
        }
        return 0;
    }
    if unsafe { PyErr_ExceptionMatches(PyExc_AttributeError) } != 0
        || cpython_error_message_indicates_missing_attribute()
    {
        unsafe {
            *result = std::ptr::null_mut();
            PyErr_Clear();
        }
        if trace_enabled {
            eprintln!(
                "[numpy-reduce] PyObject_GetOptionalAttr miss object={:p} attr={}",
                object,
                trace_name.as_deref().unwrap_or("<unmapped>")
            );
        }
        return 0;
    }
    if trace_enabled {
        eprintln!(
            "[numpy-reduce] PyObject_GetOptionalAttr error object={:p} attr={}",
            object,
            trace_name.as_deref().unwrap_or("<unmapped>")
        );
    }
    -1
}
