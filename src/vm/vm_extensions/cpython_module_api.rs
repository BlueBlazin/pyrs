use std::ffi::{CString, c_char, c_void};

use crate::runtime::{Object, Value};

use super::cpython_module_name_runtime::cpython_module_add_type_name;
use super::cpython_module_runtime::{cpython_bind_module_def, cpython_new_module_data};
use super::{
    CpythonMethodDef, CpythonModuleDef, CpythonModuleDefSlot, CpythonObjectHead, CpythonTypeObject,
    ExtensionCallableKind, Py_DecRef, Py_XDecRef, PyErr_BadArgument, PyErr_BadInternalCall,
    PyExc_SystemError, PyExc_TypeError, PyLong_FromLongLong, PyObject_SetAttrString, PyType_Ready,
    PyUnicode_AsUTF8, PyUnicode_FromString, c_name_to_string, cpython_new_ptr_for_value,
    cpython_set_error, cpython_set_typed_error, cpython_value_debug_tag,
    with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModuleDef_Init(module: *mut c_void) -> *mut c_void {
    module
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_Create2(module: *mut c_void, _apiver: i32) -> *mut c_void {
    if module.is_null() {
        cpython_set_error("PyModule_Create2 received null module definition");
        return std::ptr::null_mut();
    }
    let module = module.cast::<CpythonModuleDef>();
    let result = with_active_cpython_context_mut(|context| {
        if !unsafe { (*module).m_name.is_null() } {
            let name_result = unsafe { c_name_to_string((*module).m_name) };
            if let Err(err) = name_result {
                context.set_error(format!("PyModule_Create2 invalid module name: {err}"));
                return std::ptr::null_mut();
            }
        }
        let module_obj = context.module.clone();
        if let Err(err) = cpython_bind_module_def(context, &module_obj, module) {
            context.set_error(format!(
                "PyModule_Create2 failed to bind module definition: {err}"
            ));
            return std::ptr::null_mut();
        }
        if !unsafe { (*module).m_doc.is_null() } {
            // SAFETY: m_doc is either null or points to a valid NUL-terminated C string.
            let doc = unsafe { c_name_to_string((*module).m_doc) };
            if let Ok(doc) = doc
                && let Object::Module(module_data) = &mut *module_obj.kind_mut()
            {
                module_data
                    .globals
                    .insert("__doc__".to_string(), Value::Str(doc));
            }
        }
        context.alloc_cpython_ptr_for_value(Value::Module(module_obj))
    });
    match result {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_FromDefAndSpec2(
    module_def: *mut c_void,
    spec: *mut c_void,
    _module_api_version: i32,
) -> *mut c_void {
    if module_def.is_null() {
        cpython_set_error("PyModule_FromDefAndSpec2 received null module definition");
        return std::ptr::null_mut();
    }
    let module_def = module_def.cast::<CpythonModuleDef>();
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyModule_FromDefAndSpec2 missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: module_def points to extension-provided PyModuleDef layout.
        let module_name = if unsafe { (*module_def).m_name.is_null() } {
            "module".to_string()
        } else {
            match unsafe { c_name_to_string((*module_def).m_name) } {
                Ok(name) => name,
                Err(err) => {
                    context.set_error(format!(
                        "PyModule_FromDefAndSpec2 invalid module definition name: {err}"
                    ));
                    return std::ptr::null_mut();
                }
            }
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let mut module_data = cpython_new_module_data(module_name);
        if let Some(spec_value) = context.cpython_value_from_ptr_or_proxy(spec) {
            module_data
                .globals
                .insert("__spec__".to_string(), spec_value);
        }
        if !unsafe { (*module_def).m_doc.is_null() }
            && let Ok(doc) = unsafe { c_name_to_string((*module_def).m_doc) }
        {
            module_data
                .globals
                .insert("__doc__".to_string(), Value::Str(doc));
        }
        let module_obj = vm.heap.alloc_module(module_data);
        let Value::Module(module_obj_ref) = module_obj else {
            context.set_error("PyModule_FromDefAndSpec2 failed to allocate module object");
            return std::ptr::null_mut();
        };
        if let Err(err) = vm.register_cpython_module_methods_from_def(&module_obj_ref, module_def) {
            context.set_error(format!(
                "PyModule_FromDefAndSpec2 failed to register methods: {}",
                err.message
            ));
            return std::ptr::null_mut();
        }
        if let Err(err) = cpython_bind_module_def(context, &module_obj_ref, module_def) {
            context.set_error(format!(
                "PyModule_FromDefAndSpec2 failed to bind module definition: {err}"
            ));
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(Value::Module(module_obj_ref))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_ExecDef(module: *mut c_void, module_def: *mut c_void) -> i32 {
    if module_def.is_null() {
        cpython_set_error("PyModule_ExecDef received null module definition");
        return -1;
    }
    let module_def = module_def.cast::<CpythonModuleDef>();
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyModule_ExecDef missing VM context");
            return -1;
        }
        let module_obj = match context.cpython_module_obj_from_ptr(module) {
            Ok(module_obj) => module_obj,
            Err(err) => {
                context.set_error(format!("PyModule_ExecDef invalid module object: {err}"));
                return -1;
            }
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        if let Err(err) = vm.register_cpython_module_methods_from_def(&module_obj, module_def) {
            context.set_error(format!(
                "PyModule_ExecDef failed to register methods: {}",
                err.message
            ));
            return -1;
        }
        if let Err(err) = cpython_bind_module_def(context, &module_obj, module_def) {
            context.set_error(format!(
                "PyModule_ExecDef failed to bind module definition: {err}"
            ));
            return -1;
        }
        // SAFETY: module_def points to extension-provided PyModuleDef layout.
        let slots_ptr = unsafe { (*module_def).m_slots };
        if slots_ptr.is_null() {
            return 0;
        }
        let module_ptr = context.alloc_cpython_ptr_for_value(Value::Module(module_obj));
        let mut cursor = slots_ptr.cast::<CpythonModuleDefSlot>();
        loop {
            // SAFETY: slot table is terminated by {0, NULL}.
            let slot = unsafe { (*cursor).slot };
            // SAFETY: slot table is terminated by {0, NULL}.
            let value = unsafe { (*cursor).value };
            if slot == 0 {
                break;
            }
            if slot == 2 && !value.is_null() {
                // Py_mod_exec(module) -> int
                let exec: unsafe extern "C" fn(*mut c_void) -> i32 =
                    unsafe { std::mem::transmute(value) };
                let status = unsafe { exec(module_ptr) };
                if status != 0 {
                    if context.last_error.is_none() {
                        context.set_error("Py_mod_exec failed");
                    }
                    return -1;
                }
            }
            // SAFETY: slot array entries are contiguous.
            cursor = unsafe { cursor.add(1) };
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetDef(module: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyModule_GetDef missing VM context");
            return std::ptr::null_mut();
        }
        let module_obj = match context.cpython_module_obj_from_ptr(module) {
            Ok(module_obj) => module_obj,
            Err(_) => {
                cpython_set_typed_error(unsafe { PyExc_SystemError }, "module object expected");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        vm.extension_module_def_registry
            .get(&module_obj.id())
            .copied()
            .map_or(std::ptr::null_mut(), |ptr| ptr as *mut c_void)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetState(module: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyModule_GetState missing VM context");
            return std::ptr::null_mut();
        }
        let module_obj = match context.cpython_module_obj_from_ptr(module) {
            Ok(module_obj) => module_obj,
            Err(_) => {
                cpython_set_typed_error(unsafe { PyExc_SystemError }, "module object expected");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        vm.extension_module_state_registry
            .get(&module_obj.id())
            .map_or(std::ptr::null_mut(), |entry| entry.state as *mut c_void)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_NewObject(name: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyModule_NewObject missing VM context");
            return std::ptr::null_mut();
        }
        let Some(name_value) = context.cpython_value_from_ptr_or_proxy(name) else {
            let _ = unsafe { PyErr_BadArgument() };
            return std::ptr::null_mut();
        };
        let Value::Str(module_name) = name_value else {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, "module name must be str");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let module_value = vm.heap.alloc_module(cpython_new_module_data(module_name));
        context.alloc_cpython_ptr_for_value(module_value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_New(name: *const c_char) -> *mut c_void {
    if name.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let name_obj = unsafe { PyUnicode_FromString(name) };
    if name_obj.is_null() {
        return std::ptr::null_mut();
    }
    let module = unsafe { PyModule_NewObject(name_obj) };
    unsafe { Py_DecRef(name_obj) };
    module
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddObjectRef(
    module: *mut c_void,
    name: *const c_char,
    value: *mut c_void,
) -> i32 {
    match with_active_cpython_context_mut(|context| {
        let value_ptr = value;
        let attr_name = match unsafe { c_name_to_string(name) } {
            Ok(name) => name,
            Err(err) => {
                context.set_error(format!("PyModule_AddObjectRef invalid name: {err}"));
                return -1;
            }
        };
        let module_obj = match context.cpython_module_obj_from_ptr(module) {
            Ok(module_obj) => module_obj,
            Err(err) => {
                context.set_error(format!("PyModule_AddObjectRef invalid module: {err}"));
                return -1;
            }
        };
        let value = match context.cpython_value_from_ptr_or_proxy(value) {
            Some(value) => value,
            None => {
                // SAFETY: best-effort diagnostics for invalid value pointers.
                let (type_ptr, type_name) = unsafe {
                    let type_ptr = value_ptr
                        .cast::<CpythonObjectHead>()
                        .as_ref()
                        .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                        .unwrap_or(std::ptr::null_mut());
                    if type_ptr.is_null() {
                        (std::ptr::null_mut(), "<null>".to_string())
                    } else {
                        (
                            type_ptr,
                            c_name_to_string((*type_ptr).tp_name)
                                .unwrap_or_else(|_| "<invalid>".to_string()),
                        )
                    }
                };
                let owned = context.owns_cpython_allocation_ptr(value_ptr);
                let known_handle = context.cpython_handle_from_ptr(value_ptr).is_some();
                context.set_error(format!(
                    "PyModule_AddObjectRef invalid value object name={} ptr={:p} type={:p} type_name={} owned={} known_handle={}",
                    attr_name, value_ptr, type_ptr, type_name, owned, known_handle
                ));
                return -1;
            }
        };
        let Object::Module(module_data) = &mut *module_obj.kind_mut() else {
            context.set_error("PyModule_AddObjectRef module no longer valid");
            return -1;
        };
        if std::env::var_os("PYRS_TRACE_CPY_MODULE_ADD").is_some() {
            eprintln!(
                "[cpy-module-add] module={} attr={} value_tag={} value_ptr={:p}",
                module_data.name,
                attr_name,
                cpython_value_debug_tag(&value),
                value_ptr
            );
        }
        module_data.globals.insert(attr_name.clone(), value.clone());
        if let Err(err) = context.sync_module_dict_set(&module_obj, &attr_name, &value) {
            context.set_error(format!(
                "PyModule_AddObjectRef failed syncing module dict entry '{}': {}",
                attr_name, err
            ));
            return -1;
        }
        0
    }) {
        Ok(status) => status,
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddObject(
    module: *mut c_void,
    name: *const c_char,
    value: *mut c_void,
) -> i32 {
    let status = unsafe { PyModule_AddObjectRef(module, name, value) };
    if status != 0 || value.is_null() {
        return status;
    }
    let _ = with_active_cpython_context_mut(|context| {
        if let Some(handle) = context.cpython_handle_from_ptr(value) {
            let _ = context.decref(handle);
        }
    });
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_Add(
    module: *mut c_void,
    name: *const c_char,
    value: *mut c_void,
) -> i32 {
    let status = unsafe { PyModule_AddObjectRef(module, name, value) };
    unsafe { Py_XDecRef(value) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddFunctions(module: *mut c_void, functions: *mut c_void) -> i32 {
    if functions.is_null() {
        return 0;
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyModule_AddFunctions missing VM context");
            return -1;
        }
        let module_obj = match context.cpython_module_obj_from_ptr(module) {
            Ok(module_obj) => module_obj,
            Err(_) => {
                let _ = unsafe { PyErr_BadArgument() };
                return -1;
            }
        };
        let mut method = functions.cast::<CpythonMethodDef>();
        loop {
            // SAFETY: method table is terminated by a null ml_name.
            let method_name_ptr = unsafe { (*method).ml_name };
            if method_name_ptr.is_null() {
                break;
            }
            let method_name = match unsafe { c_name_to_string(method_name_ptr) } {
                Ok(name) => name,
                Err(err) => {
                    context.set_error(format!("PyModule_AddFunctions invalid method name: {err}"));
                    return -1;
                }
            };
            // SAFETY: VM pointer is valid for context lifetime.
            let vm = unsafe { &mut *context.vm };
            let callable = match vm.register_extension_callable(
                module_obj.clone(),
                &method_name,
                ExtensionCallableKind::CpythonMethod {
                    method_def: method as usize,
                },
            ) {
                Ok(callable) => callable,
                Err(err) => {
                    context.set_error(err.message);
                    return -1;
                }
            };
            let Object::Module(module_data) = &mut *module_obj.kind_mut() else {
                context.set_error("PyModule_AddFunctions target is not a module");
                return -1;
            };
            module_data
                .globals
                .insert(method_name.clone(), callable.clone());
            if let Err(err) = context.sync_module_dict_set(&module_obj, &method_name, &callable) {
                context.set_error(format!(
                    "PyModule_AddFunctions failed syncing module dict entry '{}': {}",
                    method_name, err
                ));
                return -1;
            }
            // SAFETY: method table entries are contiguous.
            method = unsafe { method.add(1) };
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddType(module: *mut c_void, type_ptr: *mut c_void) -> i32 {
    if type_ptr.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if unsafe { PyType_Ready(type_ptr) } != 0 {
        return -1;
    }
    // SAFETY: caller provides a valid type object pointer.
    let name_ptr = unsafe {
        (type_ptr as *mut CpythonTypeObject)
            .as_ref()
            .map(|tp| tp.tp_name)
    }
    .unwrap_or(std::ptr::null());
    if name_ptr.is_null() {
        cpython_set_typed_error(unsafe { PyExc_SystemError }, "type has no tp_name");
        return -1;
    }
    let short_name = match cpython_module_add_type_name(name_ptr) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(format!("PyModule_AddType invalid type name: {err}"));
            return -1;
        }
    };
    let c_name = match CString::new(short_name) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(format!("PyModule_AddType invalid type name: {err}"));
            return -1;
        }
    };
    unsafe { PyModule_AddObjectRef(module, c_name.as_ptr(), type_ptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddIntConstant(
    module: *mut c_void,
    name: *const c_char,
    value: i64,
) -> i32 {
    let object = unsafe { PyLong_FromLongLong(value) };
    if object.is_null() {
        return -1;
    }
    unsafe { PyModule_AddObject(module, name, object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddStringConstant(
    module: *mut c_void,
    name: *const c_char,
    value: *const c_char,
) -> i32 {
    let object = unsafe { PyUnicode_FromString(value) };
    if object.is_null() {
        return -1;
    }
    unsafe { PyModule_AddObject(module, name, object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetDict(module: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyModule_GetDict missing VM context");
            return std::ptr::null_mut();
        }
        let module_obj = match context.cpython_module_obj_from_ptr(module) {
            Ok(module_obj) => module_obj,
            Err(err) => {
                context.set_error(format!("PyModule_GetDict invalid module: {err}"));
                return std::ptr::null_mut();
            }
        };
        if let Some(existing_handle) = context.module_dict_handle_for_module(&module_obj) {
            return context.alloc_cpython_ptr_for_handle(existing_handle);
        }
        let globals = match &*module_obj.kind() {
            Object::Module(data) => data.globals.clone(),
            _ => {
                context.set_error("PyModule_GetDict module pointer is not a module");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let dict = vm.heap.alloc_dict(
            globals
                .into_iter()
                .map(|(name, value)| (Value::Str(name), value))
                .collect(),
        );
        let dict_ptr = context.alloc_cpython_ptr_for_value(dict);
        let Some(dict_handle) = context.cpython_handle_from_ptr(dict_ptr) else {
            context.set_error("PyModule_GetDict failed to materialize dict handle");
            return std::ptr::null_mut();
        };
        context
            .module_dict_handles
            .insert(dict_handle, module_obj.clone());
        context
            .module_dict_handle_by_module_id
            .insert(module_obj.id(), dict_handle);
        dict_ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_SetDocString(module: *mut c_void, doc: *const c_char) -> i32 {
    let value = if doc.is_null() {
        cpython_new_ptr_for_value(Value::None)
    } else {
        unsafe { PyUnicode_FromString(doc) }
    };
    if value.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_SetAttrString(module, c"__doc__".as_ptr(), value) };
    unsafe { Py_DecRef(value) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetNameObject(module: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let module_obj = match context.cpython_module_obj_from_ptr(module) {
            Ok(module_obj) => module_obj,
            Err(_) => {
                let _ = unsafe { PyErr_BadArgument() };
                return std::ptr::null_mut();
            }
        };
        let module_name = match &*module_obj.kind() {
            Object::Module(module_data) => module_data.globals.get("__name__").cloned(),
            _ => None,
        };
        match module_name {
            Some(Value::Str(name)) => context.alloc_cpython_ptr_for_value(Value::Str(name)),
            _ => {
                cpython_set_typed_error(unsafe { PyExc_SystemError }, "nameless module");
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
pub unsafe extern "C" fn PyModule_GetName(module: *mut c_void) -> *const c_char {
    let name_obj = unsafe { PyModule_GetNameObject(module) };
    if name_obj.is_null() {
        return std::ptr::null();
    }
    let utf8 = unsafe { PyUnicode_AsUTF8(name_obj) };
    unsafe { Py_DecRef(name_obj) };
    utf8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetFilenameObject(module: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let module_obj = match context.cpython_module_obj_from_ptr(module) {
            Ok(module_obj) => module_obj,
            Err(_) => {
                let _ = unsafe { PyErr_BadArgument() };
                return std::ptr::null_mut();
            }
        };
        let file_value = match &*module_obj.kind() {
            Object::Module(module_data) => module_data.globals.get("__file__").cloned(),
            _ => None,
        };
        match file_value {
            Some(Value::Str(path)) => context.alloc_cpython_ptr_for_value(Value::Str(path)),
            _ => {
                cpython_set_typed_error(unsafe { PyExc_SystemError }, "module filename missing");
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
pub unsafe extern "C" fn PyModule_GetFilename(module: *mut c_void) -> *const c_char {
    let filename_obj = unsafe { PyModule_GetFilenameObject(module) };
    if filename_obj.is_null() {
        return std::ptr::null();
    }
    let utf8 = unsafe { PyUnicode_AsUTF8(filename_obj) };
    unsafe { Py_DecRef(filename_obj) };
    utf8
}
