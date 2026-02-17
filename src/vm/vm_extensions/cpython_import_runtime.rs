use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use crate::bytecode::CodeObject;
use crate::runtime::{Object, Value};

use super::{
    ModuleCapiContext, ObjRef, dict_remove_value, dict_set_value_checked,
};

pub(in crate::vm::vm_extensions) fn cpython_import_add_module_by_name(
    context: &mut ModuleCapiContext,
    module_name: &str,
) -> Result<ObjRef, String> {
    if context.vm.is_null() {
        return Err("missing VM context for import API".to_string());
    }
    // SAFETY: VM pointer is valid for the active context lifetime.
    let vm = unsafe { &mut *context.vm };
    let module = vm.ensure_module(module_name);
    if let Some(modules_dict) = vm.sys_dict_obj("modules") {
        dict_set_value_checked(
            &modules_dict,
            Value::Str(module_name.to_string()),
            Value::Module(module.clone()),
        )
        .map_err(|err| err.message)?;
    } else {
        vm.refresh_sys_modules_dict();
    }
    Ok(module)
}

pub(in crate::vm::vm_extensions) type CpythonInittabInitFunc = unsafe extern "C" fn() -> *mut c_void;

pub(in crate::vm::vm_extensions) fn cpython_inittab_registry(
) -> &'static Mutex<HashMap<String, CpythonInittabInitFunc>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, CpythonInittabInitFunc>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(in crate::vm::vm_extensions) fn cpython_import_from_inittab(
    context: &mut ModuleCapiContext,
    module_name: &str,
) -> Result<Option<Value>, String> {
    let init_func = {
        let guard = cpython_inittab_registry()
            .lock()
            .map_err(|_| "PyImport inittab registry lock poisoned".to_string())?;
        guard.get(module_name).copied()
    };
    let Some(init_func) = init_func else {
        return Ok(None);
    };
    let module_ptr = unsafe { init_func() };
    if module_ptr.is_null() {
        if let Some(message) = context.last_error.clone() {
            return Err(message);
        }
        return Err(format!(
            "PyImport inittab initializer for '{}' returned null",
            module_name
        ));
    }
    let Some(module_value) = context.cpython_value_from_ptr_or_proxy(module_ptr) else {
        return Err(format!(
            "PyImport inittab initializer for '{}' returned unknown module pointer",
            module_name
        ));
    };
    let Value::Module(module_obj) = module_value else {
        return Err(format!(
            "PyImport inittab initializer for '{}' did not return a module object",
            module_name
        ));
    };
    if context.vm.is_null() {
        return Err("missing VM context for inittab import".to_string());
    }
    // SAFETY: VM pointer is valid for active context lifetime.
    let vm = unsafe { &mut *context.vm };
    vm.modules
        .insert(module_name.to_string(), module_obj.clone());
    if let Some(modules_dict) = vm.sys_dict_obj("modules") {
        dict_set_value_checked(
            &modules_dict,
            Value::Str(module_name.to_string()),
            Value::Module(module_obj.clone()),
        )
        .map_err(|err| err.message)?;
    } else {
        vm.refresh_sys_modules_dict();
    }
    Ok(Some(Value::Module(module_obj)))
}

pub(in crate::vm::vm_extensions) fn cpython_import_exec_code_in_module(
    context: &mut ModuleCapiContext,
    module_name: &str,
    code: Rc<CodeObject>,
    pathname: Option<Value>,
) -> Result<Value, String> {
    let module = cpython_import_add_module_by_name(context, module_name)?;
    if let Some(path_value) = pathname
        && path_value != Value::None
        && let Object::Module(module_data) = &mut *module.kind_mut()
    {
        module_data
            .globals
            .insert("__file__".to_string(), path_value);
    }
    if context.vm.is_null() {
        return Err("PyImport_ExecCodeModule* missing VM context".to_string());
    }
    // SAFETY: VM pointer is valid for active context lifetime.
    let vm = unsafe { &mut *context.vm };
    match vm.builtin_exec(
        vec![
            Value::Code(code),
            Value::Module(module.clone()),
            Value::Module(module.clone()),
        ],
        HashMap::new(),
    ) {
        Ok(_) => Ok(Value::Module(module)),
        Err(err) => {
            vm.modules.remove(module_name);
            if let Some(modules_dict) = vm.sys_dict_obj("modules") {
                let _ = dict_remove_value(&modules_dict, &Value::Str(module_name.to_string()));
            }
            Err(err.message)
        }
    }
}
