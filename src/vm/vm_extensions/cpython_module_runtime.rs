use std::ffi::c_void;

use crate::runtime::{ModuleObject, Value};
use crate::vm::ExtensionModuleStateEntry;

use super::{CpythonModuleDef, ModuleCapiContext, ObjRef, calloc, free};

pub(in crate::vm::vm_extensions) unsafe extern "C" fn cpython_module_state_free(
    state: *mut c_void,
) {
    if state.is_null() {
        return;
    }
    // SAFETY: module state pointers are allocated with `calloc` in this module.
    unsafe {
        free(state);
    }
}

pub(in crate::vm::vm_extensions) fn cpython_bind_module_def(
    context: &mut ModuleCapiContext,
    module_obj: &ObjRef,
    module_def: *mut CpythonModuleDef,
) -> Result<(), String> {
    if module_def.is_null() {
        return Err("null module definition".to_string());
    }
    if context.vm.is_null() {
        return Err("missing VM context".to_string());
    }
    // SAFETY: VM pointer is valid while C-API context is active.
    let vm = unsafe { &mut *context.vm };
    vm.extension_module_def_registry
        .insert(module_obj.id(), module_def as usize);
    // SAFETY: module_def points to extension-provided PyModuleDef storage.
    let module_state_size = unsafe { (*module_def).m_size };
    if module_state_size <= 0 {
        return Ok(());
    }
    if let Some(existing) = vm.extension_module_state_registry.get(&module_obj.id())
        && existing.state != 0
    {
        return Ok(());
    }
    let finalize_func = vm
        .extension_module_state_registry
        .get(&module_obj.id())
        .and_then(|entry| entry.finalize_func);
    let state_ptr = unsafe { calloc(1, module_state_size as usize) };
    if state_ptr.is_null() {
        return Err(format!(
            "failed to allocate module state ({} bytes)",
            module_state_size
        ));
    }
    let previous = vm.extension_module_state_registry.insert(
        module_obj.id(),
        ExtensionModuleStateEntry {
            state: state_ptr as usize,
            free_func: Some(cpython_module_state_free),
            finalize_func,
        },
    );
    if let Some(previous) = previous
        && previous.state != state_ptr as usize
        && previous.state != 0
    {
        if let Some(finalize) = previous.finalize_func {
            // SAFETY: callback pointer was provided by extension code.
            unsafe {
                finalize(previous.state as *mut c_void);
            }
        }
        if let Some(free_func) = previous.free_func {
            // SAFETY: callback pointer was provided by extension code.
            unsafe {
                free_func(previous.state as *mut c_void);
            }
        }
    }
    Ok(())
}

pub(in crate::vm::vm_extensions) fn cpython_new_module_data(name: String) -> ModuleObject {
    let mut module = ModuleObject::new(name.clone());
    module
        .globals
        .insert("__name__".to_string(), Value::Str(name));
    module.globals.insert("__doc__".to_string(), Value::None);
    module
        .globals
        .insert("__package__".to_string(), Value::None);
    module.globals.insert("__loader__".to_string(), Value::None);
    module.globals.insert("__spec__".to_string(), Value::None);
    module.touch_globals_version();
    module
}
