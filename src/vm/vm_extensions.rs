use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};

use crate::extensions::{
    ExtensionEntrypoint, PYRS_CAPI_ABI_VERSION, PYRS_DYNAMIC_INIT_SYMBOL_V1,
    PYRS_EXTENSION_ABI_TAG, PYRS_EXTENSION_MANIFEST_SUFFIX, PyrsApiV1, load_dynamic_initializer,
    parse_extension_manifest, path_is_shared_library,
};
use crate::runtime::{Object, RuntimeError, Value};

use super::ObjRef;
use super::Vm;

struct ModuleCapiContext {
    module: ObjRef,
}

unsafe fn c_name_to_string(name: *const c_char) -> Result<String, ()> {
    if name.is_null() {
        return Err(());
    }
    // SAFETY: caller ensures pointer is a valid NUL-terminated C string.
    let c_name = unsafe { CStr::from_ptr(name) };
    c_name.to_str().map(|text| text.to_string()).map_err(|_| ())
}

unsafe extern "C" fn capi_module_set_int(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: i64,
) -> i32 {
    if module_ctx.is_null() {
        return -1;
    }
    // SAFETY: module_ctx is created in `exec_extension_module` and remains valid for the call.
    let context = unsafe { &mut *(module_ctx as *mut ModuleCapiContext) };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(()) => return -1,
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        return -1;
    };
    module_data.globals.insert(name, Value::Int(value));
    0
}

unsafe extern "C" fn capi_module_set_bool(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: i32,
) -> i32 {
    if module_ctx.is_null() {
        return -1;
    }
    // SAFETY: module_ctx is created in `exec_extension_module` and remains valid for the call.
    let context = unsafe { &mut *(module_ctx as *mut ModuleCapiContext) };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(()) => return -1,
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        return -1;
    };
    module_data.globals.insert(name, Value::Bool(value != 0));
    0
}

unsafe extern "C" fn capi_module_set_string(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: *const c_char,
) -> i32 {
    if module_ctx.is_null() {
        return -1;
    }
    // SAFETY: module_ctx is created in `exec_extension_module` and remains valid for the call.
    let context = unsafe { &mut *(module_ctx as *mut ModuleCapiContext) };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(()) => return -1,
    };
    let value = match unsafe { c_name_to_string(value) } {
        Ok(value) => value,
        Err(()) => return -1,
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        return -1;
    };
    module_data.globals.insert(name, Value::Str(value));
    0
}

enum ExtensionExecutionPlan {
    HelloExt,
    Dynamic {
        library_path: PathBuf,
        symbol: String,
    },
}

impl Vm {
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
        Ok(())
    }

    fn execute_dynamic_extension(
        &mut self,
        module: &ObjRef,
        module_name: &str,
        library_path: &Path,
        symbol: &str,
    ) -> Result<(), RuntimeError> {
        let (handle, initializer) =
            load_dynamic_initializer(library_path, symbol).map_err(RuntimeError::new)?;
        let mut module_ctx = ModuleCapiContext {
            module: module.clone(),
        };
        let api = PyrsApiV1 {
            abi_version: PYRS_CAPI_ABI_VERSION,
            module_set_int: capi_module_set_int,
            module_set_bool: capi_module_set_bool,
            module_set_string: capi_module_set_string,
        };
        // SAFETY: initializer is resolved from the shared object symbol with expected signature;
        // pointers are valid for the duration of the call.
        let status = unsafe {
            initializer(
                &api as *const PyrsApiV1,
                (&mut module_ctx as *mut ModuleCapiContext).cast(),
            )
        };
        if status != 0 {
            return Err(RuntimeError::new(format!(
                "extension '{}' initializer '{}' failed with status {}",
                module_name, symbol, status
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
