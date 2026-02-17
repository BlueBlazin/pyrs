use std::collections::HashMap;
use std::ffi::c_void;

use crate::runtime::{Object, Value};

use super::cpython_value_from_ptr;

pub(in crate::vm::vm_extensions) fn cpython_positional_args_from_tuple_object(
    args: *mut c_void,
) -> Result<Vec<Value>, String> {
    if args.is_null() {
        return Ok(Vec::new());
    }
    let value = cpython_value_from_ptr(args)?;
    match value {
        Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
            Object::Tuple(values) => Ok(values.clone()),
            _ => Err("invalid tuple storage".to_string()),
        },
        _ => Err("expected tuple for positional arguments".to_string()),
    }
}

pub(in crate::vm::vm_extensions) fn cpython_keyword_args_from_dict_object(
    kwargs: *mut c_void,
) -> Result<HashMap<String, Value>, String> {
    if kwargs.is_null() {
        return Ok(HashMap::new());
    }
    let value = cpython_value_from_ptr(kwargs)?;
    let Value::Dict(dict_obj) = value else {
        return Err("expected dict for keyword arguments".to_string());
    };
    let Object::Dict(entries) = &*dict_obj.kind() else {
        return Err("invalid kwargs dict storage".to_string());
    };
    let mut out = HashMap::new();
    for (key, value) in entries.iter() {
        let Value::Str(name) = key else {
            return Err("keyword argument names must be str".to_string());
        };
        out.insert(name.clone(), value.clone());
    }
    Ok(out)
}
