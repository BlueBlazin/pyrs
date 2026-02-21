use std::ffi::c_void;

use crate::extensions::{
    PyrsCapsuleDestructorV1, PyrsModuleStateFinalizeV1, PyrsModuleStateFreeV1, PyrsObjectHandle,
};
use crate::runtime::{Object, Value};
use crate::vm::{ExtensionCapsuleRegistryEntry, ExtensionModuleStateEntry};

use super::{ModuleCapiContext, ObjRef};

impl ModuleCapiContext {
    fn module_obj(&self, module_handle: PyrsObjectHandle) -> Result<ObjRef, String> {
        let module = self
            .object_value(module_handle)
            .ok_or_else(|| format!("invalid module handle {}", module_handle))?;
        match module {
            Value::Module(module_obj) => Ok(module_obj),
            _ => Err(format!("object handle {} is not a module", module_handle)),
        }
    }

    pub(in crate::vm::vm_extensions) fn module_set_attr(
        &mut self,
        module_handle: PyrsObjectHandle,
        attr_name: &str,
        value_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let module_obj = self.module_obj(module_handle)?;
        let value = self
            .object_value(value_handle)
            .ok_or_else(|| format!("invalid value handle {}", value_handle))?;
        let mut module_kind = module_obj.kind_mut();
        let Object::Module(module_data) = &mut *module_kind else {
            return Err(format!(
                "object handle {} has invalid module storage",
                module_handle
            ));
        };
        module_data.globals.insert(attr_name.to_string(), value);
        Ok(())
    }

    pub(in crate::vm::vm_extensions) fn module_del_attr(
        &mut self,
        module_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<(), String> {
        let module_obj = self.module_obj(module_handle)?;
        let mut module_kind = module_obj.kind_mut();
        let Object::Module(module_data) = &mut *module_kind else {
            return Err(format!(
                "object handle {} has invalid module storage",
                module_handle
            ));
        };
        if module_data.globals.remove(attr_name).is_none() {
            return Err(format!("module attribute '{}' not found", attr_name));
        }
        Ok(())
    }

    pub(in crate::vm::vm_extensions) fn module_has_attr(
        &mut self,
        module_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<i32, String> {
        let module_obj = self.module_obj(module_handle)?;
        let module_kind = module_obj.kind();
        let Object::Module(module_data) = &*module_kind else {
            return Err(format!(
                "object handle {} has invalid module storage",
                module_handle
            ));
        };
        Ok(if module_data.globals.contains_key(attr_name) {
            1
        } else {
            0
        })
    }

    pub(in crate::vm::vm_extensions) fn module_set_state(
        &mut self,
        state: *mut c_void,
        free_func: Option<PyrsModuleStateFreeV1>,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("module_set_state missing VM context".to_string());
        }
        let module_id = self.module.id();
        // SAFETY: VM pointer is valid for the context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.prune_extension_module_state_registry();
        if state.is_null() {
            if let Some(previous) = vm.extension_module_state_registry.remove(&module_id) {
                if previous.state != 0 {
                    if let Some(previous_finalize) = previous.finalize_func {
                        // SAFETY: finalize function pointer was provided by extension code.
                        unsafe {
                            previous_finalize(previous.state as *mut c_void);
                        }
                    }
                    if let Some(previous_free) = previous.free_func {
                        // SAFETY: free function pointer was provided by extension code.
                        unsafe {
                            previous_free(previous.state as *mut c_void);
                        }
                    }
                }
                if let Some(previous_finalize) = previous.finalize_func {
                    vm.extension_module_state_registry.insert(
                        module_id,
                        ExtensionModuleStateEntry {
                            state: 0,
                            free_func: None,
                            finalize_func: Some(previous_finalize),
                        },
                    );
                }
            }
            if let Some(handle) = self.cpython_object_handles_by_id.get(&module_id).copied() {
                self.sync_cpython_storage_from_value(handle);
            }
            return Ok(());
        }
        let finalize_func = vm
            .extension_module_state_registry
            .get(&module_id)
            .and_then(|entry| entry.finalize_func);
        let entry = ExtensionModuleStateEntry {
            state: state as usize,
            free_func,
            finalize_func,
        };
        let previous = vm.extension_module_state_registry.insert(module_id, entry);
        if let Some(previous) = previous {
            let replaced_state = previous.state != state as usize;
            let replaced_free =
                previous.free_func.map(|func| func as usize) != free_func.map(|func| func as usize);
            if (replaced_state || replaced_free) && previous.state != 0 {
                if let Some(previous_finalize) = previous.finalize_func {
                    // SAFETY: finalize function pointer was provided by extension code.
                    unsafe {
                        previous_finalize(previous.state as *mut c_void);
                    }
                }
                if let Some(previous_free) = previous.free_func {
                    // SAFETY: free function pointer was provided by extension code.
                    unsafe {
                        previous_free(previous.state as *mut c_void);
                    }
                }
            }
        }
        if let Some(handle) = self.cpython_object_handles_by_id.get(&module_id).copied() {
            self.sync_cpython_storage_from_value(handle);
        }
        Ok(())
    }

    pub(in crate::vm::vm_extensions) fn module_set_finalize(
        &mut self,
        finalize_func: Option<PyrsModuleStateFinalizeV1>,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("module_set_finalize missing VM context".to_string());
        }
        let module_id = self.module.id();
        // SAFETY: VM pointer is valid for the context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.prune_extension_module_state_registry();
        if let Some(entry) = vm.extension_module_state_registry.get_mut(&module_id) {
            entry.finalize_func = finalize_func;
            if entry.state == 0 && entry.free_func.is_none() && entry.finalize_func.is_none() {
                vm.extension_module_state_registry.remove(&module_id);
            }
            return Ok(());
        }
        if let Some(finalize_func) = finalize_func {
            vm.extension_module_state_registry.insert(
                module_id,
                ExtensionModuleStateEntry {
                    state: 0,
                    free_func: None,
                    finalize_func: Some(finalize_func),
                },
            );
        }
        Ok(())
    }

    pub(in crate::vm::vm_extensions) fn module_get_state(&mut self) -> Result<*mut c_void, String> {
        if self.vm.is_null() {
            return Err("module_get_state missing VM context".to_string());
        }
        let module_id = self.module.id();
        // SAFETY: VM pointer is valid for the context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.prune_extension_module_state_registry();
        let state = vm
            .extension_module_state_registry
            .get(&module_id)
            .map_or(std::ptr::null_mut(), |entry| entry.state as *mut c_void);
        Ok(state)
    }

    pub(in crate::vm::vm_extensions) fn sync_exported_capsule(
        &mut self,
        exported_name: Option<&str>,
        pointer: usize,
        context: usize,
        destructor: Option<PyrsCapsuleDestructorV1>,
        release_previous: bool,
    ) -> Result<(), String> {
        let Some(name) = exported_name else {
            return Ok(());
        };
        if self.vm.is_null() {
            return Err("capsule export missing VM context".to_string());
        }
        // SAFETY: VM pointer is valid for the context lifetime.
        let vm = unsafe { &mut *self.vm };
        let previous = vm.extension_capsule_registry.insert(
            name.to_string(),
            ExtensionCapsuleRegistryEntry {
                pointer,
                context,
                destructor,
            },
        );
        if release_previous && let Some(previous) = previous {
            let replaced_pointer = previous.pointer != pointer || previous.context != context;
            let replaced_destructor = previous.destructor.map(|func| func as usize)
                != destructor.map(|func| func as usize);
            if (replaced_pointer || replaced_destructor)
                && let Some(previous_destructor) = previous.destructor
            {
                // SAFETY: destructor pointer came from a previously registered capsule.
                unsafe {
                    previous_destructor(
                        previous.pointer as *mut c_void,
                        previous.context as *mut c_void,
                    );
                }
            }
        }
        Ok(())
    }
}
