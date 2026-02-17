use std::ffi::{c_char, c_void};

use crate::runtime::Value;

use super::{ModuleCapiContext, c_name_to_string};

pub(in crate::vm::vm_extensions) fn cpython_module_add_type_name(
    tp_name: *const c_char,
) -> Result<String, String> {
    // SAFETY: C-API caller passes NUL-terminated `tp_name`.
    let full_name = unsafe { c_name_to_string(tp_name) }?;
    let short_name = full_name
        .rsplit('.')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(full_name.as_str());
    Ok(short_name.to_string())
}

pub(in crate::vm::vm_extensions) fn cpython_optional_value_from_ptr(
    context: &mut ModuleCapiContext,
    object: *mut c_void,
    label: &str,
) -> Result<Value, String> {
    if object.is_null() {
        return Ok(Value::None);
    }
    context
        .cpython_value_from_ptr_or_proxy(object)
        .ok_or_else(|| format!("unknown {label} object pointer"))
}

pub(in crate::vm::vm_extensions) fn cpython_module_name_from_object(
    context: &mut ModuleCapiContext,
    name: *mut c_void,
    api_name: &str,
) -> Result<String, String> {
    if name.is_null() {
        return Err(format!("{api_name} expected module name"));
    }
    match context
        .cpython_value_from_ptr_or_proxy(name)
        .ok_or_else(|| format!("{api_name} received unknown module name pointer"))?
    {
        Value::Str(name) => Ok(name),
        _ => Err(format!("{api_name} expected module name string")),
    }
}
