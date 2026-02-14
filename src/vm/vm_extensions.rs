use std::path::Path;

use crate::extensions::{ExtensionEntrypoint, parse_extension_manifest};
use crate::runtime::{Object, RuntimeError, Value};

use super::ObjRef;
use super::Vm;

impl Vm {
    pub(super) fn exec_extension_module(
        &mut self,
        module: &ObjRef,
        name: &str,
        manifest_path: &Path,
    ) -> Result<(), RuntimeError> {
        let manifest = parse_extension_manifest(manifest_path, name).map_err(RuntimeError::new)?;
        let Object::Module(module_data) = &mut *module.kind_mut() else {
            return Err(RuntimeError::new(format!(
                "module '{name}' extension load target is invalid"
            )));
        };

        module_data
            .globals
            .insert("__pyrs_extension__".to_string(), Value::Bool(true));
        module_data.globals.insert(
            "__pyrs_extension_abi__".to_string(),
            Value::Str(manifest.abi_tag.clone()),
        );
        module_data.globals.insert(
            "__pyrs_extension_entrypoint__".to_string(),
            Value::Str(manifest.entrypoint.as_str().to_string()),
        );

        match manifest.entrypoint {
            ExtensionEntrypoint::HelloExt => {
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
        }
    }
}
