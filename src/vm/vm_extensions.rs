use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};

use crate::extensions::{
    ExtensionEntrypoint, PYRS_CAPI_ABI_VERSION, PYRS_DYNAMIC_INIT_SYMBOL_V1,
    PYRS_EXTENSION_ABI_TAG, PYRS_EXTENSION_MANIFEST_SUFFIX, PYRS_TYPE_BOOL, PYRS_TYPE_BYTES,
    PYRS_TYPE_DICT, PYRS_TYPE_FLOAT, PYRS_TYPE_INT, PYRS_TYPE_LIST, PYRS_TYPE_NONE, PYRS_TYPE_STR,
    PYRS_TYPE_TUPLE, PyrsApiV1, PyrsBufferViewV1, PyrsCFunctionKwV1, PyrsCFunctionV1,
    PyrsCapsuleDestructorV1, PyrsModuleStateFinalizeV1, PyrsModuleStateFreeV1, PyrsObjectHandle,
    load_dynamic_initializer, parse_extension_manifest, path_is_shared_library,
};
use crate::runtime::{
    BoundMethod, NativeMethodKind, NativeMethodObject, Object, RuntimeError, Value,
};

use super::{
    ExtensionCallableKind, GeneratorResumeOutcome, InternalCallOutcome, NativeCallResult, ObjRef,
    Vm, bytes_like_source_is_readonly, dict_contains_key_checked, dict_get_value,
    dict_remove_value, dict_set_value_checked, memoryview_bounds, value_to_int,
    with_bytes_like_source,
};

struct CapiObjectSlot {
    value: Value,
    refcount: usize,
}

struct CapiCapsuleSlot {
    pointer: usize,
    context: usize,
    name: Option<CString>,
    destructor: Option<PyrsCapsuleDestructorV1>,
    exported_name: Option<String>,
    refcount: usize,
}

struct ModuleCapiContext {
    vm: *mut Vm,
    module: ObjRef,
    next_object_handle: PyrsObjectHandle,
    objects: HashMap<PyrsObjectHandle, CapiObjectSlot>,
    capsules: HashMap<PyrsObjectHandle, CapiCapsuleSlot>,
    last_error: Option<String>,
    scratch_strings: Vec<CString>,
    buffer_pins: HashMap<PyrsObjectHandle, usize>,
}

impl Drop for ModuleCapiContext {
    fn drop(&mut self) {
        let mut capsules = HashMap::new();
        std::mem::swap(&mut capsules, &mut self.capsules);
        for slot in capsules.into_values() {
            if slot.exported_name.is_some() {
                continue;
            }
            if let Some(destructor) = slot.destructor {
                // SAFETY: destructor pointer was provided by extension code.
                unsafe {
                    destructor(slot.pointer as *mut c_void, slot.context as *mut c_void);
                }
            }
        }
    }
}

impl ModuleCapiContext {
    fn new(vm: *mut Vm, module: ObjRef) -> Self {
        Self {
            vm,
            module,
            next_object_handle: 1,
            objects: HashMap::new(),
            capsules: HashMap::new(),
            last_error: None,
            scratch_strings: Vec::new(),
            buffer_pins: HashMap::new(),
        }
    }

    fn set_error(&mut self, message: impl Into<String>) {
        self.last_error = Some(message.into());
    }

    fn clear_error(&mut self) {
        self.last_error = None;
    }

    fn allocate_handle(&mut self) -> PyrsObjectHandle {
        let handle = self.next_object_handle;
        self.next_object_handle = self.next_object_handle.wrapping_add(1);
        if self.next_object_handle == 0 {
            self.next_object_handle = 1;
        }
        handle
    }

    fn alloc_object(&mut self, value: Value) -> PyrsObjectHandle {
        let handle = self.allocate_handle();
        self.objects
            .insert(handle, CapiObjectSlot { value, refcount: 1 });
        handle
    }

    fn alloc_capsule(
        &mut self,
        pointer: *mut c_void,
        name: *const c_char,
    ) -> Result<PyrsObjectHandle, String> {
        if pointer.is_null() {
            return Err("capsule_new requires non-null pointer".to_string());
        }
        let name = if name.is_null() {
            None
        } else {
            // SAFETY: pointer is validated by caller as NUL-terminated C string.
            let raw = unsafe { CStr::from_ptr(name) };
            Some(
                CString::new(
                    raw.to_str()
                        .map_err(|_| "capsule name must be utf-8".to_string())?,
                )
                .map_err(|_| "capsule name contains interior NUL".to_string())?,
            )
        };
        let handle = self.allocate_handle();
        self.capsules.insert(
            handle,
            CapiCapsuleSlot {
                pointer: pointer as usize,
                context: 0,
                name,
                destructor: None,
                exported_name: None,
                refcount: 1,
            },
        );
        Ok(handle)
    }

    fn object_slot(&self, handle: PyrsObjectHandle) -> Option<&CapiObjectSlot> {
        self.objects.get(&handle)
    }

    fn object_value(&self, handle: PyrsObjectHandle) -> Option<Value> {
        self.object_slot(handle).map(|slot| slot.value.clone())
    }

    fn module_get_value(&self, name: &str) -> Result<Value, String> {
        let Object::Module(module_data) = &*self.module.kind() else {
            return Err("module context no longer points to a module".to_string());
        };
        module_data
            .globals
            .get(name)
            .cloned()
            .ok_or_else(|| format!("module attribute '{}' not found", name))
    }

    fn module_get_object(&mut self, name: &str) -> Result<PyrsObjectHandle, String> {
        let value = self.module_get_value(name)?;
        Ok(self.alloc_object(value))
    }

    fn module_import(&mut self, module_name: &str) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("module_import missing VM context".to_string());
        }
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .builtin_import_module(vec![Value::Str(module_name.to_string())], HashMap::new())
            .map_err(|err| err.message)?;
        Ok(self.alloc_object(value))
    }

    fn module_get_attr(
        &mut self,
        module_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("module_get_attr missing VM context".to_string());
        }
        let module = self
            .object_value(module_handle)
            .ok_or_else(|| format!("invalid module handle {}", module_handle))?;
        let module_obj = match module {
            Value::Module(module_obj) => module_obj,
            _ => {
                return Err(format!("object handle {} is not a module", module_handle));
            }
        };
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .load_attr_module(&module_obj, attr_name)
            .map_err(|err| err.message)?;
        Ok(self.alloc_object(value))
    }

    fn module_obj(&self, module_handle: PyrsObjectHandle) -> Result<ObjRef, String> {
        let module = self
            .object_value(module_handle)
            .ok_or_else(|| format!("invalid module handle {}", module_handle))?;
        match module {
            Value::Module(module_obj) => Ok(module_obj),
            _ => Err(format!("object handle {} is not a module", module_handle)),
        }
    }

    fn module_set_attr(
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

    fn module_del_attr(
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

    fn module_has_attr(
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

    fn module_set_state(
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
                        super::ExtensionModuleStateEntry {
                            state: 0,
                            free_func: None,
                            finalize_func: Some(previous_finalize),
                        },
                    );
                }
            }
            return Ok(());
        }
        let finalize_func = vm
            .extension_module_state_registry
            .get(&module_id)
            .and_then(|entry| entry.finalize_func);
        let entry = super::ExtensionModuleStateEntry {
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
        Ok(())
    }

    fn module_set_finalize(
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
                super::ExtensionModuleStateEntry {
                    state: 0,
                    free_func: None,
                    finalize_func: Some(finalize_func),
                },
            );
        }
        Ok(())
    }

    fn module_get_state(&mut self) -> Result<*mut c_void, String> {
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

    fn sync_exported_capsule(
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
            super::ExtensionCapsuleRegistryEntry {
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

    fn incref(&mut self, handle: PyrsObjectHandle) -> Result<(), String> {
        if let Some(slot) = self.objects.get_mut(&handle) {
            slot.refcount = slot.refcount.saturating_add(1);
            return Ok(());
        }
        if let Some(slot) = self.capsules.get_mut(&handle) {
            slot.refcount = slot.refcount.saturating_add(1);
            return Ok(());
        }
        Err(format!("invalid object handle {}", handle))
    }

    fn decref(&mut self, handle: PyrsObjectHandle) -> Result<(), String> {
        if let Some(slot) = self.objects.get_mut(&handle) {
            if slot.refcount == 0 {
                self.objects.remove(&handle);
                return Ok(());
            }
            slot.refcount -= 1;
            if slot.refcount == 0 {
                self.objects.remove(&handle);
            }
            return Ok(());
        }
        if let Some(slot) = self.capsules.get_mut(&handle) {
            if slot.refcount == 0 {
                self.capsules.remove(&handle);
                return Ok(());
            }
            slot.refcount -= 1;
            if slot.refcount == 0 {
                let slot = self
                    .capsules
                    .remove(&handle)
                    .ok_or_else(|| format!("invalid object handle {}", handle))?;
                if slot.exported_name.is_none() {
                    if let Some(destructor) = slot.destructor {
                        // SAFETY: destructor pointer was provided by extension code.
                        unsafe {
                            destructor(slot.pointer as *mut c_void, slot.context as *mut c_void);
                        }
                    }
                }
            }
            return Ok(());
        }
        Err(format!("invalid object handle {}", handle))
    }

    fn capsule_export(&mut self, capsule_handle: PyrsObjectHandle) -> Result<(), String> {
        let (name, pointer, context, destructor) = {
            let Some(slot) = self.capsules.get(&capsule_handle) else {
                return Err(format!("invalid capsule handle {}", capsule_handle));
            };
            let Some(name) = slot.name.as_ref() else {
                return Err("capsule_export requires named capsule".to_string());
            };
            let name = name
                .to_str()
                .map_err(|_| "capsule name must be utf-8".to_string())?
                .to_string();
            (name, slot.pointer, slot.context, slot.destructor)
        };
        self.sync_exported_capsule(Some(name.as_str()), pointer, context, destructor, true)?;
        let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        slot.exported_name = Some(name);
        Ok(())
    }

    fn capsule_import(
        &mut self,
        name: *const c_char,
        _no_block: i32,
    ) -> Result<*mut c_void, String> {
        if name.is_null() {
            return Err("capsule_import requires non-null name".to_string());
        }
        // SAFETY: caller provides valid NUL-terminated string pointer.
        let raw = unsafe { CStr::from_ptr(name) };
        let requested_name = raw
            .to_str()
            .map_err(|_| "capsule name must be utf-8".to_string())?;
        if self.vm.is_null() {
            return Err("capsule_import missing VM context".to_string());
        }
        // SAFETY: VM pointer is valid for the context lifetime.
        let vm = unsafe { &mut *self.vm };
        if let Some(entry) = vm.extension_capsule_registry.get(requested_name) {
            return Ok(entry.pointer as *mut c_void);
        }
        let mut parts = requested_name.split('.');
        let Some(module_name) = parts.next() else {
            return Err(format!(
                "PyCapsule_Import \"{}\" is not valid",
                requested_name
            ));
        };
        if module_name.is_empty() {
            return Err(format!(
                "PyCapsule_Import \"{}\" is not valid",
                requested_name
            ));
        }
        let mut object = vm
            .builtin_import_module(vec![Value::Str(module_name.to_string())], HashMap::new())
            .map_err(|_| {
                format!(
                    "PyCapsule_Import could not import module \"{}\"",
                    module_name
                )
            })?;
        for part in parts {
            object = vm
                .builtin_getattr(vec![object, Value::Str(part.to_string())], HashMap::new())
                .map_err(|_| format!("PyCapsule_Import \"{}\" is not valid", requested_name))?;
        }
        let _ = object;
        Err(format!(
            "PyCapsule_Import \"{}\" is not valid",
            requested_name
        ))
    }

    fn capsule_new(
        &mut self,
        pointer: *mut c_void,
        name: *const c_char,
    ) -> Result<PyrsObjectHandle, String> {
        self.alloc_capsule(pointer, name)
    }

    fn capsule_get_pointer(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        name: *const c_char,
    ) -> Result<*mut c_void, String> {
        let Some(slot) = self.capsules.get(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        if !self.capsule_name_matches(slot, name)? {
            return Err("capsule name mismatch".to_string());
        }
        Ok(slot.pointer as *mut c_void)
    }

    fn capsule_set_pointer(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        pointer: *mut c_void,
    ) -> Result<(), String> {
        if pointer.is_null() {
            return Err("capsule_set_pointer requires non-null pointer".to_string());
        }
        let (exported_name, context, destructor) = {
            let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
                return Err(format!("invalid capsule handle {}", capsule_handle));
            };
            slot.pointer = pointer as usize;
            (slot.exported_name.clone(), slot.context, slot.destructor)
        };
        self.sync_exported_capsule(
            exported_name.as_deref(),
            pointer as usize,
            context,
            destructor,
            false,
        )?;
        Ok(())
    }

    fn capsule_name_matches(
        &self,
        slot: &CapiCapsuleSlot,
        name: *const c_char,
    ) -> Result<bool, String> {
        let requested_name = if name.is_null() {
            None
        } else {
            // SAFETY: caller provides a valid NUL-terminated C string.
            let raw = unsafe { CStr::from_ptr(name) };
            Some(
                raw.to_str()
                    .map_err(|_| "capsule name must be utf-8".to_string())?,
            )
        };
        let expected_name = slot.name.as_ref().map(|value| value.to_string_lossy());
        Ok(match (expected_name.as_ref(), requested_name) {
            (None, None) => true,
            (Some(expected), Some(requested)) => expected.as_ref() == requested,
            _ => false,
        })
    }

    fn capsule_get_name_ptr(
        &mut self,
        capsule_handle: PyrsObjectHandle,
    ) -> Result<*const c_char, String> {
        let Some(slot) = self.capsules.get(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        Ok(slot
            .name
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr()))
    }

    fn capsule_set_context(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        context: *mut c_void,
    ) -> Result<(), String> {
        let (exported_name, pointer, destructor) = {
            let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
                return Err(format!("invalid capsule handle {}", capsule_handle));
            };
            slot.context = context as usize;
            (slot.exported_name.clone(), slot.pointer, slot.destructor)
        };
        self.sync_exported_capsule(
            exported_name.as_deref(),
            pointer,
            context as usize,
            destructor,
            false,
        )?;
        Ok(())
    }

    fn capsule_get_context(
        &mut self,
        capsule_handle: PyrsObjectHandle,
    ) -> Result<*mut c_void, String> {
        let Some(slot) = self.capsules.get(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        Ok(slot.context as *mut c_void)
    }

    fn capsule_set_destructor(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        destructor: Option<PyrsCapsuleDestructorV1>,
    ) -> Result<(), String> {
        let (exported_name, pointer, context) = {
            let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
                return Err(format!("invalid capsule handle {}", capsule_handle));
            };
            slot.destructor = destructor;
            (slot.exported_name.clone(), slot.pointer, slot.context)
        };
        self.sync_exported_capsule(
            exported_name.as_deref(),
            pointer,
            context,
            destructor,
            false,
        )?;
        Ok(())
    }

    fn capsule_get_destructor(
        &mut self,
        capsule_handle: PyrsObjectHandle,
    ) -> Result<Option<PyrsCapsuleDestructorV1>, String> {
        let Some(slot) = self.capsules.get(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        Ok(slot.destructor)
    }

    fn capsule_set_name(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        name: *const c_char,
    ) -> Result<(), String> {
        let (old_exported_name, new_name, pointer, context, destructor) = {
            let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
                return Err(format!("invalid capsule handle {}", capsule_handle));
            };
            let old_exported_name = slot.exported_name.clone();
            let new_name = if name.is_null() {
                slot.name = None;
                None
            } else {
                // SAFETY: caller provides valid NUL-terminated string pointer.
                let raw = unsafe { CStr::from_ptr(name) };
                let text = raw
                    .to_str()
                    .map_err(|_| "capsule name must be utf-8".to_string())?
                    .to_string();
                let value = CString::new(text.as_str())
                    .map_err(|_| "capsule name contains interior NUL".to_string())?;
                slot.name = Some(value);
                Some(text)
            };
            if old_exported_name.is_some() {
                slot.exported_name = new_name.clone();
            }
            (
                old_exported_name,
                new_name,
                slot.pointer,
                slot.context,
                slot.destructor,
            )
        };
        if let Some(old) = old_exported_name.as_deref() {
            if new_name.as_deref() != Some(old) {
                if self.vm.is_null() {
                    return Err("capsule_set_name missing VM context".to_string());
                }
                // SAFETY: VM pointer is valid for the context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.extension_capsule_registry.remove(old);
            }
        }
        self.sync_exported_capsule(new_name.as_deref(), pointer, context, destructor, false)?;
        Ok(())
    }

    fn capsule_is_valid(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        name: *const c_char,
    ) -> Result<i32, String> {
        let Some(slot) = self.capsules.get(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        if self.capsule_name_matches(slot, name)? {
            Ok(1)
        } else {
            Ok(0)
        }
    }

    fn object_type(&self, handle: PyrsObjectHandle) -> Result<i32, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        let ty = match slot.value {
            Value::None => PYRS_TYPE_NONE,
            Value::Bool(_) => PYRS_TYPE_BOOL,
            Value::Int(_) => PYRS_TYPE_INT,
            Value::Str(_) => PYRS_TYPE_STR,
            Value::Float(_) => PYRS_TYPE_FLOAT,
            Value::Bytes(_) | Value::ByteArray(_) => PYRS_TYPE_BYTES,
            Value::Tuple(_) => PYRS_TYPE_TUPLE,
            Value::List(_) => PYRS_TYPE_LIST,
            Value::Dict(_) => PYRS_TYPE_DICT,
            _ => 0,
        };
        Ok(ty)
    }

    fn object_is_instance(
        &mut self,
        object_handle: PyrsObjectHandle,
        classinfo_handle: PyrsObjectHandle,
    ) -> Result<i32, String> {
        if self.vm.is_null() {
            return Err("object_is_instance missing VM context".to_string());
        }
        let object = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let classinfo = self
            .object_value(classinfo_handle)
            .ok_or_else(|| format!("invalid classinfo handle {}", classinfo_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .builtin_isinstance(vec![object, classinfo], HashMap::new())
            .map_err(|err| err.message)?;
        match value {
            Value::Bool(true) => Ok(1),
            Value::Bool(false) => Ok(0),
            other => Err(format!("isinstance returned non-bool value: {other:?}")),
        }
    }

    fn object_is_subclass(
        &mut self,
        class_handle: PyrsObjectHandle,
        classinfo_handle: PyrsObjectHandle,
    ) -> Result<i32, String> {
        if self.vm.is_null() {
            return Err("object_is_subclass missing VM context".to_string());
        }
        let class = self
            .object_value(class_handle)
            .ok_or_else(|| format!("invalid class handle {}", class_handle))?;
        let classinfo = self
            .object_value(classinfo_handle)
            .ok_or_else(|| format!("invalid classinfo handle {}", classinfo_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .builtin_issubclass(vec![class, classinfo], HashMap::new())
            .map_err(|err| err.message)?;
        match value {
            Value::Bool(true) => Ok(1),
            Value::Bool(false) => Ok(0),
            other => Err(format!("issubclass returned non-bool value: {other:?}")),
        }
    }

    fn object_get_int(&self, handle: PyrsObjectHandle) -> Result<i64, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match slot.value {
            Value::Int(value) => Ok(value),
            _ => Err(format!("object handle {} is not an int", handle)),
        }
    }

    fn object_get_bool(&self, handle: PyrsObjectHandle) -> Result<i32, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match slot.value {
            Value::Bool(value) => Ok(if value { 1 } else { 0 }),
            _ => Err(format!("object handle {} is not a bool", handle)),
        }
    }

    fn object_get_float(&self, handle: PyrsObjectHandle) -> Result<f64, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match slot.value {
            Value::Float(value) => Ok(value),
            _ => Err(format!("object handle {} is not a float", handle)),
        }
    }

    fn object_get_bytes_parts(
        &self,
        handle: PyrsObjectHandle,
    ) -> Result<(*const u8, usize), String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match &slot.value {
            Value::Bytes(bytes_obj) | Value::ByteArray(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => {
                    Ok((values.as_ptr(), values.len()))
                }
                _ => Err(format!(
                    "object handle {} has invalid bytes storage",
                    handle
                )),
            },
            _ => Err(format!("object handle {} is not bytes-like", handle)),
        }
    }

    fn object_len(&mut self, handle: PyrsObjectHandle) -> Result<usize, String> {
        if self.vm.is_null() {
            return Err("object_len missing VM context".to_string());
        }
        let value = self
            .object_value(handle)
            .ok_or_else(|| format!("invalid object handle {}", handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let length_value = vm
            .builtin_len(vec![value], HashMap::new())
            .map_err(|err| err.message)?;
        match length_value {
            Value::Int(length) => usize::try_from(length)
                .map_err(|_| format!("length {} is out of range for usize", length)),
            Value::BigInt(bigint) => {
                let text = bigint.to_string();
                let parsed = text
                    .parse::<usize>()
                    .map_err(|_| format!("length {} is out of range for usize", text))?;
                Ok(parsed)
            }
            other => Err(format!("len() returned non-int value: {other:?}")),
        }
    }

    fn object_get_item(
        &mut self,
        object_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_get_item missing VM context".to_string());
        }
        let object = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm.getitem_value(object, key).map_err(|err| err.message)?;
        Ok(self.alloc_object(value))
    }

    fn object_set_item(
        &mut self,
        object_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
        value_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("object_set_item missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        let value = self
            .object_value(value_handle)
            .ok_or_else(|| format!("invalid value handle {}", value_handle))?;
        match &target {
            Value::Dict(dict_obj) => {
                return dict_set_value_checked(dict_obj, key, value).map_err(|err| err.message);
            }
            Value::List(list_obj) => {
                let mut list_kind = list_obj.kind_mut();
                let Object::List(values) = &mut *list_kind else {
                    return Err(format!(
                        "object handle {} has invalid list storage",
                        object_handle
                    ));
                };
                let mut idx = value_to_int(key).map_err(|err| err.message)? as isize;
                if idx < 0 {
                    idx += values.len() as isize;
                }
                if idx < 0 || idx as usize >= values.len() {
                    return Err("index out of range".to_string());
                }
                values[idx as usize] = value;
                return Ok(());
            }
            Value::ByteArray(bytearray_obj) => {
                let mut bytes_kind = bytearray_obj.kind_mut();
                let Object::ByteArray(values) = &mut *bytes_kind else {
                    return Err(format!(
                        "object handle {} has invalid bytearray storage",
                        object_handle
                    ));
                };
                let mut idx = value_to_int(key).map_err(|err| err.message)? as isize;
                if idx < 0 {
                    idx += values.len() as isize;
                }
                if idx < 0 || idx as usize >= values.len() {
                    return Err("index out of range".to_string());
                }
                let byte = value_to_int(value).map_err(|err| err.message)?;
                if !(0..=255).contains(&byte) {
                    return Err("byte must be in range(0, 256)".to_string());
                }
                values[idx as usize] = byte as u8;
                return Ok(());
            }
            _ => {}
        }
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let Some(setitem) = vm
            .lookup_bound_special_method(&target, "__setitem__")
            .map_err(|err| err.message)?
        else {
            return Err("object does not support item assignment".to_string());
        };
        match vm
            .call_internal(setitem, vec![key, value], HashMap::new())
            .map_err(|err| err.message)?
        {
            InternalCallOutcome::Value(_) => Ok(()),
            InternalCallOutcome::CallerExceptionHandled => Err(vm
                .runtime_error_from_active_exception("object_set_item() failed")
                .message),
        }
    }

    fn object_del_item(
        &mut self,
        object_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("object_del_item missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        match &target {
            Value::Dict(dict_obj) => {
                if dict_remove_value(dict_obj, &key).is_none() {
                    return Err("dict key not found".to_string());
                }
                return Ok(());
            }
            Value::List(list_obj) => {
                let mut list_kind = list_obj.kind_mut();
                let Object::List(values) = &mut *list_kind else {
                    return Err(format!(
                        "object handle {} has invalid list storage",
                        object_handle
                    ));
                };
                let mut idx = value_to_int(key).map_err(|err| err.message)? as isize;
                if idx < 0 {
                    idx += values.len() as isize;
                }
                if idx < 0 || idx as usize >= values.len() {
                    return Err("index out of range".to_string());
                }
                values.remove(idx as usize);
                return Ok(());
            }
            Value::ByteArray(bytearray_obj) => {
                let mut bytes_kind = bytearray_obj.kind_mut();
                let Object::ByteArray(values) = &mut *bytes_kind else {
                    return Err(format!(
                        "object handle {} has invalid bytearray storage",
                        object_handle
                    ));
                };
                let mut idx = value_to_int(key).map_err(|err| err.message)? as isize;
                if idx < 0 {
                    idx += values.len() as isize;
                }
                if idx < 0 || idx as usize >= values.len() {
                    return Err("index out of range".to_string());
                }
                values.remove(idx as usize);
                return Ok(());
            }
            _ => {}
        }
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let Some(delitem) = vm
            .lookup_bound_special_method(&target, "__delitem__")
            .map_err(|err| err.message)?
        else {
            return Err("object does not support item deletion".to_string());
        };
        match vm
            .call_internal(delitem, vec![key], HashMap::new())
            .map_err(|err| err.message)?
        {
            InternalCallOutcome::Value(_) => Ok(()),
            InternalCallOutcome::CallerExceptionHandled => Err(vm
                .runtime_error_from_active_exception("object_del_item() failed")
                .message),
        }
    }

    fn object_contains(
        &mut self,
        object_handle: PyrsObjectHandle,
        needle_handle: PyrsObjectHandle,
    ) -> Result<i32, String> {
        if self.vm.is_null() {
            return Err("object_contains missing VM context".to_string());
        }
        let container = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let needle = self
            .object_value(needle_handle)
            .ok_or_else(|| format!("invalid needle handle {}", needle_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let contains = vm
            .compare_in_runtime(needle, container)
            .map_err(|err| err.message)?;
        Ok(if contains { 1 } else { 0 })
    }

    fn object_dict_keys(
        &mut self,
        dict_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_dict_keys missing VM context".to_string());
        }
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let entries = match &*dict_obj.kind() {
            Object::Dict(entries) => entries.to_vec(),
            _ => {
                return Err(format!(
                    "object handle {} has invalid dict storage",
                    dict_handle
                ));
            }
        };
        let mut keys = Vec::with_capacity(entries.len());
        for (key, _) in entries {
            keys.push(key);
        }
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        Ok(self.alloc_object(vm.heap.alloc_list(keys)))
    }

    fn object_dict_items(
        &mut self,
        dict_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_dict_items missing VM context".to_string());
        }
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let entries = match &*dict_obj.kind() {
            Object::Dict(entries) => entries.to_vec(),
            _ => {
                return Err(format!(
                    "object handle {} has invalid dict storage",
                    dict_handle
                ));
            }
        };
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let mut items = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            items.push(vm.heap.alloc_tuple(vec![key, value]));
        }
        Ok(self.alloc_object(vm.heap.alloc_list(items)))
    }

    fn object_get_buffer(
        &mut self,
        object_handle: PyrsObjectHandle,
    ) -> Result<PyrsBufferViewV1, String> {
        let value = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let (data, len, readonly) = match &value {
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(values) => (values.as_ptr(), values.len(), true),
                _ => {
                    return Err(format!(
                        "object handle {} has invalid bytes storage",
                        object_handle
                    ));
                }
            },
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(values) => (values.as_ptr(), values.len(), false),
                _ => {
                    return Err(format!(
                        "object handle {} has invalid bytearray storage",
                        object_handle
                    ));
                }
            },
            Value::MemoryView(obj) => match &*obj.kind() {
                Object::MemoryView(view) => {
                    if view.released {
                        return Err("memoryview is released".to_string());
                    }
                    let readonly = bytes_like_source_is_readonly(&view.source).unwrap_or(true);
                    let Some((ptr, len)) = with_bytes_like_source(&view.source, |values| {
                        let (start, end) = memoryview_bounds(view.start, view.length, values.len());
                        (
                            values.as_ptr().wrapping_add(start),
                            end.saturating_sub(start),
                        )
                    }) else {
                        return Err("memoryview source is not bytes-like".to_string());
                    };
                    (ptr, len, readonly)
                }
                _ => {
                    return Err(format!(
                        "object handle {} has invalid memoryview storage",
                        object_handle
                    ));
                }
            },
            _ => {
                return Err(format!(
                    "object handle {} does not support buffer access",
                    object_handle
                ));
            }
        };
        self.incref(object_handle)?;
        *self.buffer_pins.entry(object_handle).or_insert(0) += 1;
        Ok(PyrsBufferViewV1 {
            data,
            len,
            readonly: if readonly { 1 } else { 0 },
        })
    }

    fn object_release_buffer(&mut self, object_handle: PyrsObjectHandle) -> Result<(), String> {
        let Some(pins) = self.buffer_pins.get_mut(&object_handle) else {
            return Err("buffer was not acquired for this handle".to_string());
        };
        if *pins == 0 {
            self.buffer_pins.remove(&object_handle);
            return Err("buffer was not acquired for this handle".to_string());
        }
        *pins -= 1;
        if *pins == 0 {
            self.buffer_pins.remove(&object_handle);
        }
        self.decref(object_handle)
    }

    fn object_sequence_len(&self, handle: PyrsObjectHandle) -> Result<usize, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match &slot.value {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => Ok(values.len()),
                _ => Err(format!(
                    "object handle {} has invalid tuple storage",
                    handle
                )),
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => Ok(values.len()),
                _ => Err(format!("object handle {} has invalid list storage", handle)),
            },
            _ => Err(format!("object handle {} is not tuple/list", handle)),
        }
    }

    fn object_sequence_get_item(
        &self,
        handle: PyrsObjectHandle,
        index: usize,
    ) -> Result<Value, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match &slot.value {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("sequence index {} out of range", index)),
                _ => Err(format!(
                    "object handle {} has invalid tuple storage",
                    handle
                )),
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => values
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("sequence index {} out of range", index)),
                _ => Err(format!("object handle {} has invalid list storage", handle)),
            },
            _ => Err(format!("object handle {} is not tuple/list", handle)),
        }
    }

    fn object_get_iter(&mut self, handle: PyrsObjectHandle) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_get_iter missing VM context".to_string());
        }
        let source = self
            .object_value(handle)
            .ok_or_else(|| format!("invalid object handle {}", handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let iterator = vm
            .builtin_iter(vec![source], HashMap::new())
            .map_err(|err| err.message)?;
        Ok(self.alloc_object(iterator))
    }

    fn object_iter_next(
        &mut self,
        iter_handle: PyrsObjectHandle,
    ) -> Result<Option<PyrsObjectHandle>, String> {
        if self.vm.is_null() {
            return Err("object_iter_next missing VM context".to_string());
        }
        let iterator = self
            .object_value(iter_handle)
            .ok_or_else(|| format!("invalid iterator handle {}", iter_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        match vm
            .next_from_iterator_value(&iterator)
            .map_err(|err| err.message)?
        {
            GeneratorResumeOutcome::Yield(value) => Ok(Some(self.alloc_object(value))),
            GeneratorResumeOutcome::Complete(_) => Ok(None),
            GeneratorResumeOutcome::PropagatedException => Err(vm
                .runtime_error_from_active_exception("object_iter_next() failed")
                .message),
        }
    }

    fn object_list_obj(&self, handle: PyrsObjectHandle) -> Result<ObjRef, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match &slot.value {
            Value::List(obj) => Ok(obj.clone()),
            _ => Err(format!("object handle {} is not list", handle)),
        }
    }

    fn object_list_append(
        &mut self,
        list_handle: PyrsObjectHandle,
        item_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let list_obj = self.object_list_obj(list_handle)?;
        let item = self
            .object_value(item_handle)
            .ok_or_else(|| format!("invalid item handle {}", item_handle))?;
        let mut list_kind = list_obj.kind_mut();
        let Object::List(values) = &mut *list_kind else {
            return Err(format!(
                "object handle {} has invalid list storage",
                list_handle
            ));
        };
        values.push(item);
        Ok(())
    }

    fn object_list_set_item(
        &mut self,
        list_handle: PyrsObjectHandle,
        index: usize,
        item_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let list_obj = self.object_list_obj(list_handle)?;
        let item = self
            .object_value(item_handle)
            .ok_or_else(|| format!("invalid item handle {}", item_handle))?;
        let mut list_kind = list_obj.kind_mut();
        let Object::List(values) = &mut *list_kind else {
            return Err(format!(
                "object handle {} has invalid list storage",
                list_handle
            ));
        };
        let Some(slot) = values.get_mut(index) else {
            return Err(format!("list index {} out of range", index));
        };
        *slot = item;
        Ok(())
    }

    fn object_dict_obj(&self, handle: PyrsObjectHandle) -> Result<ObjRef, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match &slot.value {
            Value::Dict(obj) => Ok(obj.clone()),
            _ => Err(format!("object handle {} is not dict", handle)),
        }
    }

    fn object_dict_len(&self, handle: PyrsObjectHandle) -> Result<usize, String> {
        let dict_obj = self.object_dict_obj(handle)?;
        match &*dict_obj.kind() {
            Object::Dict(entries) => Ok(entries.len()),
            _ => Err(format!("object handle {} has invalid dict storage", handle)),
        }
    }

    fn object_dict_set_item(
        &mut self,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
        value_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        let value = self
            .object_value(value_handle)
            .ok_or_else(|| format!("invalid value handle {}", value_handle))?;
        dict_set_value_checked(&dict_obj, key, value).map_err(|err| err.message)
    }

    fn object_dict_get_item(
        &mut self,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        let value =
            dict_get_value(&dict_obj, &key).ok_or_else(|| "dict key not found".to_string())?;
        Ok(self.alloc_object(value))
    }

    fn object_dict_contains(
        &mut self,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> Result<i32, String> {
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        let contains = dict_contains_key_checked(&dict_obj, &key).map_err(|err| err.message)?;
        Ok(if contains { 1 } else { 0 })
    }

    fn object_dict_del_item(
        &mut self,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        let removed = dict_remove_value(&dict_obj, &key);
        if removed.is_none() {
            return Err("dict key not found".to_string());
        }
        Ok(())
    }

    fn object_get_attr(
        &mut self,
        object_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_get_attr missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .builtin_getattr(
                vec![target, Value::Str(attr_name.to_string())],
                HashMap::new(),
            )
            .map_err(|err| err.message)?;
        Ok(self.alloc_object(value))
    }

    fn object_set_attr(
        &mut self,
        object_handle: PyrsObjectHandle,
        attr_name: &str,
        value_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("object_set_attr missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let value = self
            .object_value(value_handle)
            .ok_or_else(|| format!("invalid object handle {}", value_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.builtin_setattr(
            vec![target, Value::Str(attr_name.to_string()), value],
            HashMap::new(),
        )
        .map_err(|err| err.message)?;
        Ok(())
    }

    fn object_del_attr(
        &mut self,
        object_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("object_del_attr missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.builtin_delattr(
            vec![target, Value::Str(attr_name.to_string())],
            HashMap::new(),
        )
        .map_err(|err| err.message)?;
        Ok(())
    }

    fn object_has_attr(
        &mut self,
        object_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<i32, String> {
        if self.vm.is_null() {
            return Err("object_has_attr missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .builtin_hasattr(
                vec![target, Value::Str(attr_name.to_string())],
                HashMap::new(),
            )
            .map_err(|err| err.message)?;
        match value {
            Value::Bool(true) => Ok(1),
            Value::Bool(false) => Ok(0),
            other => Err(format!("hasattr returned non-bool value: {other:?}")),
        }
    }

    fn object_call_noargs(
        &mut self,
        callable_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        self.object_call(callable_handle, &[], &[])
    }

    fn object_call_onearg(
        &mut self,
        callable_handle: PyrsObjectHandle,
        arg_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        self.object_call(callable_handle, &[arg_handle], &[])
    }

    fn object_call(
        &mut self,
        callable_handle: PyrsObjectHandle,
        arg_handles: &[PyrsObjectHandle],
        kwarg_handles: &[(String, PyrsObjectHandle)],
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_call missing VM context".to_string());
        }
        let callable = self
            .object_value(callable_handle)
            .ok_or_else(|| format!("invalid callable handle {}", callable_handle))?;
        let mut args = Vec::with_capacity(arg_handles.len());
        for handle in arg_handles {
            let value = self
                .object_value(*handle)
                .ok_or_else(|| format!("invalid argument handle {}", handle))?;
            args.push(value);
        }
        let mut kwargs = HashMap::with_capacity(kwarg_handles.len());
        for (name, handle) in kwarg_handles {
            let value = self
                .object_value(*handle)
                .ok_or_else(|| format!("invalid keyword argument handle {}", handle))?;
            if kwargs.insert(name.clone(), value).is_some() {
                return Err(format!("duplicate keyword argument '{}'", name));
            }
        }
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        if !vm.is_callable_value(&callable) {
            return Err("object_call target is not callable".to_string());
        }
        let result = match vm
            .call_internal(callable, args, kwargs)
            .map_err(|err| err.message)?
        {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(vm
                    .runtime_error_from_active_exception("object_call() failed")
                    .message);
            }
        };
        Ok(self.alloc_object(result))
    }

    fn error_get_message_ptr(&mut self) -> *const c_char {
        let Some(message) = self.last_error.as_deref() else {
            return std::ptr::null();
        };
        let cstring = match CString::new(message) {
            Ok(value) => value,
            Err(_) => CString::new("error message contains interior NUL")
                .expect("fallback error message has no interior NUL"),
        };
        self.scratch_strings.push(cstring);
        self.scratch_strings
            .last()
            .map(|value| value.as_ptr())
            .unwrap_or(std::ptr::null())
    }

    fn object_get_string_ptr(&mut self, handle: PyrsObjectHandle) -> Result<*const c_char, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        let Value::Str(text) = &slot.value else {
            return Err(format!("object handle {} is not a str", handle));
        };
        let cstring = CString::new(text.as_str())
            .map_err(|_| "string contains interior NUL byte".to_string())?;
        self.scratch_strings.push(cstring);
        let ptr = self
            .scratch_strings
            .last()
            .map(|value| value.as_ptr())
            .unwrap_or(std::ptr::null());
        if ptr.is_null() {
            Err("failed to materialize string pointer".to_string())
        } else {
            Ok(ptr)
        }
    }
}

unsafe fn capi_context_mut<'a>(module_ctx: *mut c_void) -> Option<&'a mut ModuleCapiContext> {
    if module_ctx.is_null() {
        return None;
    }
    // SAFETY: caller guarantees `module_ctx` points to a valid `ModuleCapiContext`.
    Some(unsafe { &mut *(module_ctx as *mut ModuleCapiContext) })
}

unsafe fn c_name_to_string(name: *const c_char) -> Result<String, String> {
    if name.is_null() {
        return Err("received null C string pointer".to_string());
    }
    // SAFETY: caller ensures pointer is a valid NUL-terminated C string.
    let c_name = unsafe { CStr::from_ptr(name) };
    c_name
        .to_str()
        .map(|text| text.to_string())
        .map_err(|_| "received non-utf8 C string".to_string())
}

unsafe fn capi_module_insert_value(
    context: &mut ModuleCapiContext,
    name: *const c_char,
    value: Value,
) -> i32 {
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        context.set_error("module context no longer points to a module");
        return -1;
    };
    module_data.globals.insert(name, value);
    0
}

unsafe extern "C" fn capi_api_has_capability(module_ctx: *mut c_void, name: *const c_char) -> i32 {
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

unsafe extern "C" fn capi_module_set_int(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: i64,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, Value::Int(value)) }
}

unsafe extern "C" fn capi_module_set_bool(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: i32,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, Value::Bool(value != 0)) }
}

unsafe extern "C" fn capi_module_set_string(
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

unsafe extern "C" fn capi_module_add_function(
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

unsafe extern "C" fn capi_module_add_function_kw(
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

unsafe extern "C" fn capi_object_new_int(module_ctx: *mut c_void, value: i64) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Int(value))
}

unsafe extern "C" fn capi_object_new_none(module_ctx: *mut c_void) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::None)
}

unsafe extern "C" fn capi_object_new_bool(module_ctx: *mut c_void, value: i32) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Bool(value != 0))
}

unsafe extern "C" fn capi_object_new_float(
    module_ctx: *mut c_void,
    value: f64,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Float(value))
}

unsafe extern "C" fn capi_object_new_bytes(
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

unsafe extern "C" fn capi_object_new_bytearray(
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

unsafe extern "C" fn capi_object_new_tuple(
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

unsafe extern "C" fn capi_object_new_list(
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

unsafe extern "C" fn capi_object_new_dict(module_ctx: *mut c_void) -> PyrsObjectHandle {
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

unsafe extern "C" fn capi_object_new_string(
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

unsafe extern "C" fn capi_object_incref(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32 {
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

unsafe extern "C" fn capi_object_decref(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32 {
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

unsafe extern "C" fn capi_module_set_object(
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

unsafe extern "C" fn capi_module_get_object(
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

unsafe extern "C" fn capi_module_import(
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

unsafe extern "C" fn capi_module_get_attr(
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

unsafe extern "C" fn capi_module_set_state(
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

unsafe extern "C" fn capi_module_get_state(module_ctx: *mut c_void) -> *mut c_void {
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

unsafe extern "C" fn capi_module_set_finalize(
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

unsafe extern "C" fn capi_module_set_attr(
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

unsafe extern "C" fn capi_module_del_attr(
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

unsafe extern "C" fn capi_module_has_attr(
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

unsafe extern "C" fn capi_object_type(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32 {
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

unsafe extern "C" fn capi_object_is_instance(
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

unsafe extern "C" fn capi_object_is_subclass(
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

unsafe extern "C" fn capi_object_get_int(
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

unsafe extern "C" fn capi_object_get_float(
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

unsafe extern "C" fn capi_object_get_bool(
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

unsafe extern "C" fn capi_object_get_bytes(
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

unsafe extern "C" fn capi_object_len(
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

unsafe extern "C" fn capi_object_get_item(
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

unsafe extern "C" fn capi_object_set_item(
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

unsafe extern "C" fn capi_object_del_item(
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

unsafe extern "C" fn capi_object_contains(
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

unsafe extern "C" fn capi_object_dict_keys(
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

unsafe extern "C" fn capi_object_dict_items(
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

unsafe extern "C" fn capi_object_get_buffer(
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

unsafe extern "C" fn capi_object_release_buffer(
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

unsafe extern "C" fn capi_capsule_new(
    module_ctx: *mut c_void,
    pointer: *mut c_void,
    name: *const c_char,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    match context.capsule_new(pointer, name) {
        Ok(handle) => handle,
        Err(err) => {
            context.set_error(err);
            0
        }
    }
}

unsafe extern "C" fn capi_capsule_get_pointer(
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

unsafe extern "C" fn capi_capsule_set_pointer(
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

unsafe extern "C" fn capi_capsule_get_name(
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

unsafe extern "C" fn capi_capsule_set_context(
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

unsafe extern "C" fn capi_capsule_get_context(
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

unsafe extern "C" fn capi_capsule_set_destructor(
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

unsafe extern "C" fn capi_capsule_get_destructor(
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

unsafe extern "C" fn capi_capsule_set_name(
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

unsafe extern "C" fn capi_capsule_is_valid(
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

unsafe extern "C" fn capi_capsule_export(
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

unsafe extern "C" fn capi_capsule_import(
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

unsafe extern "C" fn capi_object_sequence_len(
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

unsafe extern "C" fn capi_object_sequence_get_item(
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

unsafe extern "C" fn capi_object_get_iter(
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

unsafe extern "C" fn capi_object_iter_next(
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

unsafe extern "C" fn capi_object_list_append(
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

unsafe extern "C" fn capi_object_list_set_item(
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

unsafe extern "C" fn capi_object_dict_len(
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

unsafe extern "C" fn capi_object_dict_set_item(
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

unsafe extern "C" fn capi_object_dict_get_item(
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

unsafe extern "C" fn capi_object_dict_contains(
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

unsafe extern "C" fn capi_object_dict_del_item(
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

unsafe extern "C" fn capi_object_get_attr(
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

unsafe extern "C" fn capi_object_set_attr(
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

unsafe extern "C" fn capi_object_del_attr(
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

unsafe extern "C" fn capi_object_has_attr(
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

unsafe extern "C" fn capi_object_call_noargs(
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

unsafe extern "C" fn capi_object_call_onearg(
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

unsafe extern "C" fn capi_object_call(
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

unsafe extern "C" fn capi_object_get_string(
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

unsafe extern "C" fn capi_error_set(module_ctx: *mut c_void, message: *const c_char) -> i32 {
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

unsafe extern "C" fn capi_error_get_message(module_ctx: *mut c_void) -> *const c_char {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null();
    };
    context.error_get_message_ptr()
}

unsafe extern "C" fn capi_error_clear(module_ctx: *mut c_void) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    context.clear_error();
    0
}

unsafe extern "C" fn capi_error_occurred(module_ctx: *mut c_void) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 1;
    };
    if context.last_error.is_some() { 1 } else { 0 }
}

enum ExtensionExecutionPlan {
    HelloExt,
    Dynamic {
        library_path: PathBuf,
        symbol: String,
    },
}

impl Vm {
    fn prune_extension_module_state_registry(&mut self) {
        let live_module_ids: std::collections::HashSet<u64> =
            self.modules.values().map(|module| module.id()).collect();
        let stale_ids: Vec<u64> = self
            .extension_module_state_registry
            .keys()
            .copied()
            .filter(|id| !live_module_ids.contains(id))
            .collect();
        for stale_id in stale_ids {
            if let Some(entry) = self.extension_module_state_registry.remove(&stale_id) {
                if entry.state != 0 {
                    if let Some(finalize_func) = entry.finalize_func {
                        // SAFETY: finalize function pointer was provided by extension code.
                        unsafe {
                            finalize_func(entry.state as *mut c_void);
                        }
                    }
                    if let Some(free_func) = entry.free_func {
                        // SAFETY: free function pointer was provided by extension code.
                        unsafe {
                            free_func(entry.state as *mut c_void);
                        }
                    }
                }
            }
        }
    }

    fn cpython_init_symbol_for_module(module_name: &str) -> String {
        let leaf = module_name
            .rsplit('.')
            .next()
            .unwrap_or(module_name)
            .replace('-', "_");
        format!("PyInit_{leaf}")
    }

    fn capi_api_v1(&self) -> PyrsApiV1 {
        PyrsApiV1 {
            abi_version: PYRS_CAPI_ABI_VERSION,
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
        }
    }

    pub(super) fn register_extension_callable(
        &mut self,
        module: ObjRef,
        name: &str,
        kind: ExtensionCallableKind,
    ) -> Result<Value, RuntimeError> {
        let id = self.next_extension_callable_id;
        self.next_extension_callable_id = self.next_extension_callable_id.wrapping_add(1);
        if self.next_extension_callable_id == 0 {
            self.next_extension_callable_id = 1;
        }
        self.extension_callable_registry.insert(
            id,
            super::ExtensionCallableEntry {
                module: module.clone(),
                name: name.to_string(),
                kind,
            },
        );

        let native = self.heap.alloc_native_method(NativeMethodObject::new(
            NativeMethodKind::ExtensionFunctionCall(id),
        ));
        let bound = match self
            .heap
            .alloc_bound_method(BoundMethod::new(native, module))
        {
            Value::BoundMethod(obj) => obj,
            _ => unreachable!(),
        };
        Ok(Value::BoundMethod(bound))
    }

    pub(super) fn call_extension_callable(
        &mut self,
        function_id: u64,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<NativeCallResult, RuntimeError> {
        let Some(entry) = self.extension_callable_registry.get(&function_id).cloned() else {
            return Err(RuntimeError::new(format!(
                "unknown extension callable id {}",
                function_id
            )));
        };
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, entry.module.clone());
        let mut arg_handles = Vec::with_capacity(args.len());
        for arg in args {
            arg_handles.push(call_ctx.alloc_object(arg));
        }
        let api = self.capi_api_v1();
        let mut result_handle: PyrsObjectHandle = 0;
        let status = match entry.kind {
            ExtensionCallableKind::Positional(callback) => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(format!(
                        "extension function '{}.{}' does not accept keyword arguments",
                        match &*entry.module.kind() {
                            Object::Module(module_data) => module_data.name.clone(),
                            _ => "<extension>".to_string(),
                        },
                        entry.name
                    )));
                }
                // SAFETY: callback pointer comes from extension registration and the API/context
                // pointers remain valid for the duration of this call.
                unsafe {
                    callback(
                        &api as *const PyrsApiV1,
                        (&mut call_ctx as *mut ModuleCapiContext).cast(),
                        arg_handles.len(),
                        arg_handles.as_ptr(),
                        &mut result_handle as *mut PyrsObjectHandle,
                    )
                }
            }
            ExtensionCallableKind::WithKeywords(callback) => {
                let mut kw_name_storage = Vec::with_capacity(kwargs.len());
                let mut kw_name_ptrs = Vec::with_capacity(kwargs.len());
                let mut kw_value_handles = Vec::with_capacity(kwargs.len());
                for (name, value) in kwargs {
                    let c_name = CString::new(name.as_str()).map_err(|_| {
                        RuntimeError::new("extension call keyword name contains interior NUL byte")
                    })?;
                    kw_name_storage.push(c_name);
                    let ptr = kw_name_storage
                        .last()
                        .map(|name| name.as_ptr())
                        .unwrap_or(std::ptr::null());
                    kw_name_ptrs.push(ptr);
                    kw_value_handles.push(call_ctx.alloc_object(value));
                }
                // SAFETY: callback pointer comes from extension registration and the API/context
                // pointers remain valid for the duration of this call. Keyword C strings and
                // value handles remain alive for the callback duration.
                unsafe {
                    callback(
                        &api as *const PyrsApiV1,
                        (&mut call_ctx as *mut ModuleCapiContext).cast(),
                        arg_handles.len(),
                        arg_handles.as_ptr(),
                        kw_name_ptrs.len(),
                        kw_name_ptrs.as_ptr(),
                        kw_value_handles.as_ptr(),
                        &mut result_handle as *mut PyrsObjectHandle,
                    )
                }
            }
        };
        if status != 0 {
            let detail = call_ctx
                .last_error
                .as_deref()
                .map(|text| format!(": {text}"))
                .unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "extension function '{}.{}' failed with status {}{}",
                match &*entry.module.kind() {
                    Object::Module(module_data) => module_data.name.clone(),
                    _ => "<extension>".to_string(),
                },
                entry.name,
                status,
                detail
            )));
        }
        if result_handle == 0 {
            return Err(RuntimeError::new(format!(
                "extension function '{}.{}' returned null handle",
                match &*entry.module.kind() {
                    Object::Module(module_data) => module_data.name.clone(),
                    _ => "<extension>".to_string(),
                },
                entry.name
            )));
        }
        let Some(result) = call_ctx.object_value(result_handle) else {
            return Err(RuntimeError::new(format!(
                "extension function '{}.{}' returned unknown handle {}",
                match &*entry.module.kind() {
                    Object::Module(module_data) => module_data.name.clone(),
                    _ => "<extension>".to_string(),
                },
                entry.name,
                result_handle
            )));
        };
        Ok(NativeCallResult::Value(result))
    }

    fn set_extension_metadata(
        &mut self,
        module: &ObjRef,
        abi_tag: &str,
        entrypoint: &str,
        origin: &Path,
    ) -> Result<(), RuntimeError> {
        let Object::Module(module_data) = &mut *module.kind_mut() else {
            return Err(RuntimeError::new("extension load target is not a module"));
        };
        module_data
            .globals
            .insert("__pyrs_extension__".to_string(), Value::Bool(true));
        module_data.globals.insert(
            "__pyrs_extension_abi__".to_string(),
            Value::Str(abi_tag.to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_entrypoint__".to_string(),
            Value::Str(entrypoint.to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_origin__".to_string(),
            Value::Str(origin.to_string_lossy().to_string()),
        );
        module_data.globals.insert(
            "__pyrs_capi_abi_version__".to_string(),
            Value::Int(PYRS_CAPI_ABI_VERSION as i64),
        );
        Ok(())
    }

    fn execute_dynamic_extension(
        &mut self,
        module: &ObjRef,
        module_name: &str,
        library_path: &Path,
        symbol: &str,
    ) -> Result<(), RuntimeError> {
        let (handle, initializer) = load_dynamic_initializer(library_path, symbol).map_err(|err| {
            if symbol == PYRS_DYNAMIC_INIT_SYMBOL_V1 {
                let cpython_symbol = Self::cpython_init_symbol_for_module(module_name);
                RuntimeError::new(format!(
                    "{err}; expected '{}'. CPython-style extension symbols such as '{}' are not supported yet",
                    PYRS_DYNAMIC_INIT_SYMBOL_V1, cpython_symbol
                ))
            } else {
                RuntimeError::new(err)
            }
        })?;
        let mut module_ctx = ModuleCapiContext::new(self as *mut Vm, module.clone());
        let api = self.capi_api_v1();
        // SAFETY: initializer is resolved from the shared object symbol with expected signature;
        // pointers are valid for the duration of the call.
        let status = unsafe {
            initializer(
                &api as *const PyrsApiV1,
                (&mut module_ctx as *mut ModuleCapiContext).cast(),
            )
        };
        if status != 0 {
            let message = module_ctx
                .last_error
                .as_deref()
                .map(|text| format!(": {text}"))
                .unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "extension '{}' initializer '{}' failed with status {}{}",
                module_name, symbol, status, message
            )));
        }
        if let Some(message) = module_ctx.last_error.as_deref() {
            return Err(RuntimeError::new(format!(
                "extension '{}' initializer '{}' reported error despite success: {}",
                module_name, symbol, message
            )));
        }

        let Object::Module(module_data) = &mut *module.kind_mut() else {
            return Err(RuntimeError::new(format!(
                "module '{}' invalid after extension init",
                module_name
            )));
        };
        module_data.globals.insert(
            "__pyrs_extension_library__".to_string(),
            Value::Str(library_path.to_string_lossy().to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_symbol__".to_string(),
            Value::Str(symbol.to_string()),
        );
        let symbol_family = if symbol == PYRS_DYNAMIC_INIT_SYMBOL_V1 {
            "pyrs-v1"
        } else if symbol.starts_with("PyInit_") {
            "cpython"
        } else {
            "custom"
        };
        module_data.globals.insert(
            "__pyrs_extension_expected_symbol__".to_string(),
            Value::Str(symbol.to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_symbol_family__".to_string(),
            Value::Str(symbol_family.to_string()),
        );
        self.extension_libraries.push(handle);
        Ok(())
    }

    pub(super) fn exec_extension_module(
        &mut self,
        module: &ObjRef,
        name: &str,
        source_path: &Path,
    ) -> Result<(), RuntimeError> {
        let (abi_tag, entrypoint_name, plan) = if source_path
            .to_string_lossy()
            .ends_with(PYRS_EXTENSION_MANIFEST_SUFFIX)
        {
            let manifest =
                parse_extension_manifest(source_path, name).map_err(RuntimeError::new)?;
            let entrypoint_name = manifest.entrypoint.as_str();
            let plan = match manifest.entrypoint {
                ExtensionEntrypoint::HelloExt => ExtensionExecutionPlan::HelloExt,
                ExtensionEntrypoint::DynamicSymbol(ref symbol) => {
                    let library_path =
                        manifest.resolve_library_path(source_path).ok_or_else(|| {
                            RuntimeError::new(format!(
                                "extension manifest '{}' missing dynamic library path",
                                source_path.display()
                            ))
                        })?;
                    ExtensionExecutionPlan::Dynamic {
                        library_path,
                        symbol: symbol.clone(),
                    }
                }
            };
            (manifest.abi_tag, entrypoint_name, plan)
        } else if path_is_shared_library(source_path) {
            (
                PYRS_EXTENSION_ABI_TAG.to_string(),
                format!("dynamic:{PYRS_DYNAMIC_INIT_SYMBOL_V1}"),
                ExtensionExecutionPlan::Dynamic {
                    library_path: source_path.to_path_buf(),
                    symbol: PYRS_DYNAMIC_INIT_SYMBOL_V1.to_string(),
                },
            )
        } else {
            return Err(RuntimeError::new(format!(
                "unsupported extension module source '{}'",
                source_path.display()
            )));
        };

        self.set_extension_metadata(module, &abi_tag, &entrypoint_name, source_path)?;

        match plan {
            ExtensionExecutionPlan::HelloExt => {
                let Object::Module(module_data) = &mut *module.kind_mut() else {
                    return Err(RuntimeError::new(format!(
                        "module '{}' extension load target is invalid",
                        name
                    )));
                };
                module_data
                    .globals
                    .insert("EXTENSION_LOADED".to_string(), Value::Bool(true));
                module_data.globals.insert(
                    "ENTRYPOINT".to_string(),
                    Value::Str("hello_ext".to_string()),
                );
                module_data.globals.insert(
                    "MESSAGE".to_string(),
                    Value::Str("hello from hello_ext".to_string()),
                );
                Ok(())
            }
            ExtensionExecutionPlan::Dynamic {
                library_path,
                symbol,
            } => self.execute_dynamic_extension(module, name, &library_path, &symbol),
        }
    }
}
