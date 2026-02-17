use std::ffi::{c_char, c_void};

use super::{
    ExtensionCallableKind, Object, PyrsApiV1, PyrsBufferInfoV1, PyrsBufferInfoV2, PyrsBufferViewV1,
    PyrsCFunctionKwV1, PyrsCFunctionV1, PyrsCapsuleDestructorV1, PyrsModuleStateFinalizeV1,
    PyrsModuleStateFreeV1, PyrsObjectHandle, PyrsWritableBufferViewV1, Value, Vm, c_name_to_string,
    capi_context_mut, capi_module_insert_value,
};

pub(super) unsafe extern "C" fn capi_api_has_capability(
    module_ctx: *mut c_void,
    name: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    let supported = matches!(
        name.as_str(),
        "module_add_function"
            | "module_add_function_kw"
            | "module_get_object"
            | "module_import"
            | "module_get_attr"
            | "module_set_state"
            | "module_get_state"
            | "module_set_finalize"
            | "module_set_attr"
            | "module_del_attr"
            | "module_has_attr"
            | "object_new_none"
            | "object_new_float"
            | "object_new_bytes"
            | "object_new_bytearray"
            | "object_new_memoryview"
            | "object_new_tuple"
            | "object_new_list"
            | "object_new_dict"
            | "object_len"
            | "object_get_item"
            | "object_set_item"
            | "object_del_item"
            | "object_contains"
            | "object_dict_keys"
            | "object_dict_items"
            | "object_get_buffer"
            | "object_get_writable_buffer"
            | "object_get_buffer_info"
            | "object_get_buffer_info_v2"
            | "object_release_buffer"
            | "capsule_new"
            | "capsule_get_pointer"
            | "capsule_set_pointer"
            | "capsule_get_name"
            | "capsule_set_context"
            | "capsule_get_context"
            | "capsule_set_destructor"
            | "capsule_get_destructor"
            | "capsule_set_name"
            | "capsule_is_valid"
            | "capsule_export"
            | "capsule_import"
            | "object_sequence_len"
            | "object_sequence_get_item"
            | "object_get_iter"
            | "object_iter_next"
            | "object_list_append"
            | "object_list_set_item"
            | "object_dict_len"
            | "object_dict_set_item"
            | "object_dict_get_item"
            | "object_dict_contains"
            | "object_dict_del_item"
            | "object_get_attr"
            | "object_set_attr"
            | "object_del_attr"
            | "object_has_attr"
            | "object_is_instance"
            | "object_is_subclass"
            | "object_call_noargs"
            | "object_call_onearg"
            | "object_call"
            | "error_get_message"
            | "error_state"
            | "extension_symbol_metadata"
    );
    if supported { 1 } else { 0 }
}

pub(super) unsafe extern "C" fn capi_module_set_int(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: i64,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, Value::Int(value)) }
}

pub(super) unsafe extern "C" fn capi_module_set_bool(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: i32,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, Value::Bool(value != 0)) }
}

pub(super) unsafe extern "C" fn capi_module_set_string(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let value = match unsafe { c_name_to_string(value) } {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    unsafe { capi_module_insert_value(context, name, Value::Str(value)) }
}

pub(super) unsafe extern "C" fn capi_module_add_function(
    module_ctx: *mut c_void,
    name: *const c_char,
    callback: Option<PyrsCFunctionV1>,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let Some(callback) = callback else {
        context.set_error("module_add_function requires a non-null callback");
        return -1;
    };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    if context.vm.is_null() {
        context.set_error("module_add_function missing VM context");
        return -1;
    }
    // SAFETY: VM pointer is set by `exec_extension_module` and valid during init callback.
    let vm = unsafe { &mut *context.vm };
    let callable = match vm.register_extension_callable(
        context.module.clone(),
        &name,
        ExtensionCallableKind::Positional(callback),
    ) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err.message);
            return -1;
        }
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        context.set_error("module context no longer points to a module");
        return -1;
    };
    module_data.globals.insert(name, callable);
    0
}

pub(super) unsafe extern "C" fn capi_module_add_function_kw(
    module_ctx: *mut c_void,
    name: *const c_char,
    callback: Option<PyrsCFunctionKwV1>,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let Some(callback) = callback else {
        context.set_error("module_add_function_kw requires a non-null callback");
        return -1;
    };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    if context.vm.is_null() {
        context.set_error("module_add_function_kw missing VM context");
        return -1;
    }
    // SAFETY: VM pointer is set by `exec_extension_module` and valid during init callback.
    let vm = unsafe { &mut *context.vm };
    let callable = match vm.register_extension_callable(
        context.module.clone(),
        &name,
        ExtensionCallableKind::WithKeywords(callback),
    ) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err.message);
            return -1;
        }
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        context.set_error("module context no longer points to a module");
        return -1;
    };
    module_data.globals.insert(name, callable);
    0
}

pub(super) unsafe extern "C" fn capi_object_new_int(
    module_ctx: *mut c_void,
    value: i64,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Int(value))
}

pub(super) unsafe extern "C" fn capi_object_new_none(module_ctx: *mut c_void) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::None)
}

pub(super) unsafe extern "C" fn capi_object_new_bool(
    module_ctx: *mut c_void,
    value: i32,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Bool(value != 0))
}

pub(super) unsafe extern "C" fn capi_object_new_float(
    module_ctx: *mut c_void,
    value: f64,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Float(value))
}

pub(super) unsafe extern "C" fn capi_object_new_bytes(
    module_ctx: *mut c_void,
    data: *const u8,
    len: usize,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if data.is_null() && len != 0 {
        context.set_error("object_new_bytes received null data pointer with non-zero len");
        return 0;
    }
    let bytes = if len == 0 {
        Vec::new()
    } else {
        // SAFETY: caller-provided pointer/len pair is assumed valid for read.
        unsafe { std::slice::from_raw_parts(data, len) }.to_vec()
    };
    if context.vm.is_null() {
        context.set_error("object_new_bytes missing VM context");
        return 0;
    }
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_bytes(bytes))
}

pub(super) unsafe extern "C" fn capi_object_new_bytearray(
    module_ctx: *mut c_void,
    data: *const u8,
    len: usize,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if data.is_null() && len != 0 {
        context.set_error("object_new_bytearray received null data pointer with non-zero len");
        return 0;
    }
    let bytes = if len == 0 {
        Vec::new()
    } else {
        // SAFETY: caller-provided pointer/len pair is assumed valid for read.
        unsafe { std::slice::from_raw_parts(data, len) }.to_vec()
    };
    if context.vm.is_null() {
        context.set_error("object_new_bytearray missing VM context");
        return 0;
    }
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_bytearray(bytes))
}

pub(super) unsafe extern "C" fn capi_object_new_memoryview(
    module_ctx: *mut c_void,
    source_handle: PyrsObjectHandle,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if context.vm.is_null() {
        context.set_error("object_new_memoryview missing VM context");
        return 0;
    }
    let Some(source_value) = context.object_value(source_handle) else {
        context.set_error(format!("invalid object handle {}", source_handle));
        return 0;
    };
    let source = match source_value {
        Value::Bytes(obj) | Value::ByteArray(obj) => obj,
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => view.source.clone(),
            _ => {
                context.set_error(format!(
                    "object handle {} has invalid memoryview storage",
                    source_handle
                ));
                return 0;
            }
        },
        _ => {
            context.set_error(format!(
                "object handle {} does not support memoryview construction",
                source_handle
            ));
            return 0;
        }
    };
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_memoryview(source))
}

pub(super) unsafe extern "C" fn capi_object_new_tuple(
    module_ctx: *mut c_void,
    len: usize,
    items: *const PyrsObjectHandle,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if len != 0 && items.is_null() {
        context.set_error("object_new_tuple received null items pointer with non-zero len");
        return 0;
    }
    if context.vm.is_null() {
        context.set_error("object_new_tuple missing VM context");
        return 0;
    }
    let mut values = Vec::with_capacity(len);
    for idx in 0..len {
        // SAFETY: caller-provided pointer/len pair is assumed valid for read.
        let handle = unsafe { *items.add(idx) };
        let Some(value) = context.object_value(handle) else {
            context.set_error(format!(
                "object_new_tuple received invalid item handle {} at index {}",
                handle, idx
            ));
            return 0;
        };
        values.push(value);
    }
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_tuple(values))
}

pub(super) unsafe extern "C" fn capi_object_new_list(
    module_ctx: *mut c_void,
    len: usize,
    items: *const PyrsObjectHandle,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if len != 0 && items.is_null() {
        context.set_error("object_new_list received null items pointer with non-zero len");
        return 0;
    }
    if context.vm.is_null() {
        context.set_error("object_new_list missing VM context");
        return 0;
    }
    let mut values = Vec::with_capacity(len);
    for idx in 0..len {
        // SAFETY: caller-provided pointer/len pair is assumed valid for read.
        let handle = unsafe { *items.add(idx) };
        let Some(value) = context.object_value(handle) else {
            context.set_error(format!(
                "object_new_list received invalid item handle {} at index {}",
                handle, idx
            ));
            return 0;
        };
        values.push(value);
    }
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_list(values))
}

pub(super) unsafe extern "C" fn capi_object_new_dict(module_ctx: *mut c_void) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if context.vm.is_null() {
        context.set_error("object_new_dict missing VM context");
        return 0;
    }
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_dict(Vec::new()))
}

pub(super) unsafe extern "C" fn capi_object_new_string(
    module_ctx: *mut c_void,
    value: *const c_char,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    let value = match unsafe { c_name_to_string(value) } {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            return 0;
        }
    };
    context.alloc_object(Value::Str(value))
}

pub(super) unsafe extern "C" fn capi_object_incref(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.incref(handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_decref(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.decref(handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_module_set_object(
    module_ctx: *mut c_void,
    name: *const c_char,
    handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let Some(value) = context.object_value(handle) else {
        context.set_error(format!("invalid object handle {}", handle));
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, value) }
}

pub(super) unsafe extern "C" fn capi_module_get_object(
    module_ctx: *mut c_void,
    name: *const c_char,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("module_get_object received null output pointer");
        return -1;
    }
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_get_object(&name) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_module_import(
    module_ctx: *mut c_void,
    module_name: *const c_char,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("module_import received null output pointer");
        return -1;
    }
    let module_name = match unsafe { c_name_to_string(module_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_import(&module_name) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_module_get_attr(
    module_ctx: *mut c_void,
    module_handle: PyrsObjectHandle,
    attr_name: *const c_char,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("module_get_attr received null output pointer");
        return -1;
    }
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_get_attr(module_handle, &attr_name) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_module_set_state(
    module_ctx: *mut c_void,
    state: *mut c_void,
    free_func: Option<PyrsModuleStateFreeV1>,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.module_set_state(state, free_func) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_module_get_state(module_ctx: *mut c_void) -> *mut c_void {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null_mut();
    };
    match context.module_get_state() {
        Ok(state) => state,
        Err(err) => {
            context.set_error(err);
            std::ptr::null_mut()
        }
    }
}

pub(super) unsafe extern "C" fn capi_module_set_finalize(
    module_ctx: *mut c_void,
    finalize_func: Option<PyrsModuleStateFinalizeV1>,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.module_set_finalize(finalize_func) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_module_set_attr(
    module_ctx: *mut c_void,
    module_handle: PyrsObjectHandle,
    attr_name: *const c_char,
    value_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_set_attr(module_handle, &attr_name, value_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_module_del_attr(
    module_ctx: *mut c_void,
    module_handle: PyrsObjectHandle,
    attr_name: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_del_attr(module_handle, &attr_name) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_module_has_attr(
    module_ctx: *mut c_void,
    module_handle: PyrsObjectHandle,
    attr_name: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_has_attr(module_handle, &attr_name) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_type(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    match context.object_type(handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            0
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_is_instance(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    classinfo_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_is_instance(object_handle, classinfo_handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_is_subclass(
    module_ctx: *mut c_void,
    class_handle: PyrsObjectHandle,
    classinfo_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_is_subclass(class_handle, classinfo_handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_int(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out: *mut i64,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out.is_null() {
        context.set_error("object_get_int received null out pointer");
        return -1;
    }
    match context.object_get_int(handle) {
        Ok(value) => {
            // SAFETY: caller provided non-null out pointer.
            unsafe {
                *out = value;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_float(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out: *mut f64,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out.is_null() {
        context.set_error("object_get_float received null out pointer");
        return -1;
    }
    match context.object_get_float(handle) {
        Ok(value) => {
            // SAFETY: caller provided non-null out pointer.
            unsafe {
                *out = value;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_bool(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out: *mut i32,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out.is_null() {
        context.set_error("object_get_bool received null out pointer");
        return -1;
    }
    match context.object_get_bool(handle) {
        Ok(value) => {
            // SAFETY: caller provided non-null out pointer.
            unsafe {
                *out = value;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_bytes(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out_data: *mut *const u8,
    out_len: *mut usize,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_data.is_null() || out_len.is_null() {
        context.set_error("object_get_bytes received null output pointer");
        return -1;
    }
    match context.object_get_bytes_parts(handle) {
        Ok((data_ptr, len)) => {
            // SAFETY: caller provided non-null out pointers.
            unsafe {
                *out_data = data_ptr;
                *out_len = len;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_len(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out_len: *mut usize,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_len.is_null() {
        context.set_error("object_len received null output pointer");
        return -1;
    }
    match context.object_len(handle) {
        Ok(len) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_len = len;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_item(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_get_item received null output pointer");
        return -1;
    }
    match context.object_get_item(object_handle, key_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_set_item(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
    value_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_set_item(object_handle, key_handle, value_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_del_item(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_del_item(object_handle, key_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_contains(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    needle_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_contains(object_handle, needle_handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_dict_keys(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_dict_keys received null output pointer");
        return -1;
    }
    match context.object_dict_keys(dict_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_dict_items(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_dict_items received null output pointer");
        return -1;
    }
    match context.object_dict_items(dict_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_buffer(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    out_view: *mut PyrsBufferViewV1,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_view.is_null() {
        context.set_error("object_get_buffer received null output pointer");
        return -1;
    }
    match context.object_get_buffer(object_handle) {
        Ok(view) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_view = view;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_writable_buffer(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    out_view: *mut PyrsWritableBufferViewV1,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_view.is_null() {
        context.set_error("object_get_writable_buffer received null output pointer");
        return -1;
    }
    match context.object_get_writable_buffer(object_handle) {
        Ok(view) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_view = view;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_buffer_info(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    out_info: *mut PyrsBufferInfoV1,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_info.is_null() {
        context.set_error("object_get_buffer_info received null output pointer");
        return -1;
    }
    match context.object_get_buffer_info(object_handle) {
        Ok(info) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_info = info;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_buffer_info_v2(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    out_info: *mut PyrsBufferInfoV2,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_info.is_null() {
        context.set_error("object_get_buffer_info_v2 received null output pointer");
        return -1;
    }
    match context.object_get_buffer_info_v2(object_handle) {
        Ok(info) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_info = info;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_release_buffer(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_release_buffer(object_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_new(
    module_ctx: *mut c_void,
    pointer: *mut c_void,
    name: *const c_char,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    match context.capsule_new(pointer, name, None) {
        Ok(handle) => handle,
        Err(err) => {
            context.set_error(err);
            0
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_get_pointer(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    name: *const c_char,
) -> *mut c_void {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null_mut();
    };
    match context.capsule_get_pointer(capsule_handle, name) {
        Ok(ptr) => ptr,
        Err(err) => {
            context.set_error(err);
            std::ptr::null_mut()
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_set_pointer(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    pointer: *mut c_void,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.capsule_set_pointer(capsule_handle, pointer) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_get_name(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
) -> *const c_char {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null();
    };
    match context.capsule_get_name_ptr(capsule_handle) {
        Ok(name_ptr) => name_ptr,
        Err(err) => {
            context.set_error(err);
            std::ptr::null()
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_set_context(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    context: *mut c_void,
) -> i32 {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context_obj.capsule_set_context(capsule_handle, context) {
        Ok(()) => 0,
        Err(err) => {
            context_obj.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_get_context(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
) -> *mut c_void {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null_mut();
    };
    match context_obj.capsule_get_context(capsule_handle) {
        Ok(ptr) => ptr,
        Err(err) => {
            context_obj.set_error(err);
            std::ptr::null_mut()
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_set_destructor(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    destructor: Option<PyrsCapsuleDestructorV1>,
) -> i32 {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context_obj.capsule_set_destructor(capsule_handle, destructor) {
        Ok(()) => 0,
        Err(err) => {
            context_obj.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_get_destructor(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
) -> Option<PyrsCapsuleDestructorV1> {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return None;
    };
    match context_obj.capsule_get_destructor(capsule_handle) {
        Ok(destructor) => destructor,
        Err(err) => {
            context_obj.set_error(err);
            None
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_set_name(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    name: *const c_char,
) -> i32 {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context_obj.capsule_set_name(capsule_handle, name) {
        Ok(()) => 0,
        Err(err) => {
            context_obj.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_is_valid(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    name: *const c_char,
) -> i32 {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context_obj.capsule_is_valid(capsule_handle, name) {
        Ok(value) => value,
        Err(err) => {
            context_obj.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_export(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context_obj.capsule_export(capsule_handle) {
        Ok(()) => 0,
        Err(err) => {
            context_obj.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_capsule_import(
    module_ctx: *mut c_void,
    name: *const c_char,
    no_block: i32,
) -> *mut c_void {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null_mut();
    };
    match context_obj.capsule_import(name, no_block) {
        Ok(ptr) => ptr,
        Err(err) => {
            context_obj.set_error(err);
            std::ptr::null_mut()
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_sequence_len(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out_len: *mut usize,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_len.is_null() {
        context.set_error("object_sequence_len received null output pointer");
        return -1;
    }
    match context.object_sequence_len(handle) {
        Ok(len) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_len = len;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_sequence_get_item(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    index: usize,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_sequence_get_item received null output pointer");
        return -1;
    }
    match context.object_sequence_get_item(handle, index) {
        Ok(value) => {
            let item_handle = context.alloc_object(value);
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = item_handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_iter(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_get_iter received null output pointer");
        return -1;
    }
    match context.object_get_iter(handle) {
        Ok(iterator_handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = iterator_handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_iter_next(
    module_ctx: *mut c_void,
    iter_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_iter_next received null output pointer");
        return -1;
    }
    match context.object_iter_next(iter_handle) {
        Ok(Some(item_handle)) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = item_handle;
            }
            1
        }
        Ok(None) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_list_append(
    module_ctx: *mut c_void,
    list_handle: PyrsObjectHandle,
    item_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_list_append(list_handle, item_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_list_set_item(
    module_ctx: *mut c_void,
    list_handle: PyrsObjectHandle,
    index: usize,
    item_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_list_set_item(list_handle, index, item_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_dict_len(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out_len: *mut usize,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_len.is_null() {
        context.set_error("object_dict_len received null output pointer");
        return -1;
    }
    match context.object_dict_len(handle) {
        Ok(len) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_len = len;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_dict_set_item(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
    value_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_dict_set_item(dict_handle, key_handle, value_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_dict_get_item(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_dict_get_item received null output pointer");
        return -1;
    }
    match context.object_dict_get_item(dict_handle, key_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_dict_contains(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_dict_contains(dict_handle, key_handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_dict_del_item(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_dict_del_item(dict_handle, key_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_attr(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    attr_name: *const c_char,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_get_attr received null output pointer");
        return -1;
    }
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.object_get_attr(object_handle, &attr_name) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_set_attr(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    attr_name: *const c_char,
    value_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.object_set_attr(object_handle, &attr_name, value_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_del_attr(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    attr_name: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.object_del_attr(object_handle, &attr_name) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_has_attr(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    attr_name: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.object_has_attr(object_handle, &attr_name) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_call_noargs(
    module_ctx: *mut c_void,
    callable_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_call_noargs received null output pointer");
        return -1;
    }
    match context.object_call_noargs(callable_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_call_onearg(
    module_ctx: *mut c_void,
    callable_handle: PyrsObjectHandle,
    arg_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_call_onearg received null output pointer");
        return -1;
    }
    match context.object_call_onearg(callable_handle, arg_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_call(
    module_ctx: *mut c_void,
    callable_handle: PyrsObjectHandle,
    argc: usize,
    argv: *const PyrsObjectHandle,
    kwargc: usize,
    kwarg_names: *const *const c_char,
    kwarg_values: *const PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_call received null output pointer");
        return -1;
    }
    if argc > 0 && argv.is_null() {
        context.set_error("object_call received null argv pointer");
        return -1;
    }
    if kwargc > 0 && (kwarg_names.is_null() || kwarg_values.is_null()) {
        context.set_error("object_call received null keyword payload");
        return -1;
    }
    let arg_handles = if argc == 0 {
        &[][..]
    } else {
        // SAFETY: validated above; caller guarantees array length by `argc`.
        unsafe { std::slice::from_raw_parts(argv, argc) }
    };
    let mut kwarg_handles = Vec::with_capacity(kwargc);
    if kwargc > 0 {
        // SAFETY: validated above; caller guarantees array lengths by `kwargc`.
        let kw_names = unsafe { std::slice::from_raw_parts(kwarg_names, kwargc) };
        // SAFETY: validated above; caller guarantees array lengths by `kwargc`.
        let kw_values = unsafe { std::slice::from_raw_parts(kwarg_values, kwargc) };
        for idx in 0..kwargc {
            let name_ptr = kw_names[idx];
            let name = match unsafe { c_name_to_string(name_ptr) } {
                Ok(name) => name,
                Err(err) => {
                    context.set_error(format!(
                        "object_call invalid keyword name at index {idx}: {err}"
                    ));
                    return -1;
                }
            };
            kwarg_handles.push((name, kw_values[idx]));
        }
    }
    match context.object_call(callable_handle, arg_handles, &kwarg_handles) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_object_get_string(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
) -> *const c_char {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null();
    };
    match context.object_get_string_ptr(handle) {
        Ok(ptr) => ptr,
        Err(err) => {
            context.set_error(err);
            std::ptr::null()
        }
    }
}

pub(super) unsafe extern "C" fn capi_error_set(
    module_ctx: *mut c_void,
    message: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match unsafe { c_name_to_string(message) } {
        Ok(message) => {
            context.set_error(message);
            -1
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

pub(super) unsafe extern "C" fn capi_error_get_message(module_ctx: *mut c_void) -> *const c_char {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null();
    };
    context.error_get_message_ptr()
}

pub(super) unsafe extern "C" fn capi_error_clear(module_ctx: *mut c_void) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    context.clear_error();
    0
}

pub(super) unsafe extern "C" fn capi_error_occurred(module_ctx: *mut c_void) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 1;
    };
    if context.last_error.is_some() { 1 } else { 0 }
}

impl Vm {
    pub(super) fn capi_api_v1(&self) -> PyrsApiV1 {
        PyrsApiV1 {
            abi_version: super::PYRS_CAPI_ABI_VERSION,
            api_has_capability: capi_api_has_capability,
            module_set_int: capi_module_set_int,
            module_set_bool: capi_module_set_bool,
            module_set_string: capi_module_set_string,
            module_add_function: capi_module_add_function,
            module_add_function_kw: capi_module_add_function_kw,
            object_new_int: capi_object_new_int,
            object_new_none: capi_object_new_none,
            object_new_bool: capi_object_new_bool,
            object_new_float: capi_object_new_float,
            object_new_bytes: capi_object_new_bytes,
            object_new_bytearray: capi_object_new_bytearray,
            object_new_memoryview: capi_object_new_memoryview,
            object_new_tuple: capi_object_new_tuple,
            object_new_list: capi_object_new_list,
            object_new_dict: capi_object_new_dict,
            object_new_string: capi_object_new_string,
            object_incref: capi_object_incref,
            object_decref: capi_object_decref,
            module_set_object: capi_module_set_object,
            module_get_object: capi_module_get_object,
            module_import: capi_module_import,
            module_get_attr: capi_module_get_attr,
            module_set_state: capi_module_set_state,
            module_get_state: capi_module_get_state,
            module_set_finalize: capi_module_set_finalize,
            object_type: capi_object_type,
            object_is_instance: capi_object_is_instance,
            object_is_subclass: capi_object_is_subclass,
            object_get_int: capi_object_get_int,
            object_get_float: capi_object_get_float,
            object_get_bool: capi_object_get_bool,
            object_get_bytes: capi_object_get_bytes,
            object_len: capi_object_len,
            object_get_item: capi_object_get_item,
            object_sequence_len: capi_object_sequence_len,
            object_sequence_get_item: capi_object_sequence_get_item,
            object_get_iter: capi_object_get_iter,
            object_iter_next: capi_object_iter_next,
            object_list_append: capi_object_list_append,
            object_list_set_item: capi_object_list_set_item,
            object_dict_len: capi_object_dict_len,
            object_dict_set_item: capi_object_dict_set_item,
            object_dict_get_item: capi_object_dict_get_item,
            object_dict_contains: capi_object_dict_contains,
            object_dict_del_item: capi_object_dict_del_item,
            object_get_attr: capi_object_get_attr,
            object_set_attr: capi_object_set_attr,
            object_del_attr: capi_object_del_attr,
            object_has_attr: capi_object_has_attr,
            object_call_noargs: capi_object_call_noargs,
            object_call_onearg: capi_object_call_onearg,
            object_call: capi_object_call,
            object_get_string: capi_object_get_string,
            error_set: capi_error_set,
            error_get_message: capi_error_get_message,
            error_clear: capi_error_clear,
            error_occurred: capi_error_occurred,
            module_set_attr: capi_module_set_attr,
            module_del_attr: capi_module_del_attr,
            module_has_attr: capi_module_has_attr,
            object_set_item: capi_object_set_item,
            object_del_item: capi_object_del_item,
            object_contains: capi_object_contains,
            object_dict_keys: capi_object_dict_keys,
            object_dict_items: capi_object_dict_items,
            object_get_buffer: capi_object_get_buffer,
            object_get_writable_buffer: capi_object_get_writable_buffer,
            object_release_buffer: capi_object_release_buffer,
            capsule_new: capi_capsule_new,
            capsule_get_pointer: capi_capsule_get_pointer,
            capsule_set_pointer: capi_capsule_set_pointer,
            capsule_get_name: capi_capsule_get_name,
            capsule_set_context: capi_capsule_set_context,
            capsule_get_context: capi_capsule_get_context,
            capsule_set_destructor: capi_capsule_set_destructor,
            capsule_get_destructor: capi_capsule_get_destructor,
            capsule_set_name: capi_capsule_set_name,
            capsule_is_valid: capi_capsule_is_valid,
            capsule_export: capi_capsule_export,
            capsule_import: capi_capsule_import,
            object_get_buffer_info: capi_object_get_buffer_info,
            object_get_buffer_info_v2: capi_object_get_buffer_info_v2,
        }
    }
}
