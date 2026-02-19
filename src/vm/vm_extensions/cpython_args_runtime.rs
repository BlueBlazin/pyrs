use std::collections::HashMap;
use std::ffi::c_void;

use crate::runtime::{Object, Value};

use super::{
    PyDict_Next, PyTuple_GetItem, PyTuple_Size, cpython_value_from_ptr,
    cpython_value_from_ptr_or_proxy,
};

pub(in crate::vm::vm_extensions) fn cpython_positional_args_from_tuple_object(
    args: *mut c_void,
) -> Result<Vec<Value>, String> {
    if args.is_null() {
        return Ok(Vec::new());
    }
    if let Ok(value) = cpython_value_from_ptr(args) {
        match value {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(values) => return Ok(values.clone()),
                _ => return Err("invalid tuple storage".to_string()),
            },
            _ => return Err("expected tuple for positional arguments".to_string()),
        }
    }
    // Fallback for foreign tuple pointers not yet materialized in this context.
    let argc = unsafe { PyTuple_Size(args) };
    if argc < 0 {
        return Err("failed to inspect positional args tuple".to_string());
    }
    let mut out = Vec::with_capacity(argc as usize);
    for index in 0..argc {
        let item_ptr = unsafe { PyTuple_GetItem(args, index) };
        if item_ptr.is_null() {
            return Err("failed to read positional arg item".to_string());
        }
        let item = cpython_value_from_ptr_or_proxy(item_ptr)
            .map_err(|_| "received unknown positional arg pointer".to_string())?;
        out.push(item);
    }
    Ok(out)
}

pub(in crate::vm::vm_extensions) fn cpython_keyword_args_from_dict_object(
    kwargs: *mut c_void,
) -> Result<HashMap<String, Value>, String> {
    if kwargs.is_null() {
        return Ok(HashMap::new());
    }
    if let Ok(value) = cpython_value_from_ptr(kwargs) {
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
        return Ok(out);
    }

    // Fallback for foreign kwargs dictionaries not yet materialized in this context.
    let mut position: isize = 0;
    let mut key_ptr = std::ptr::null_mut();
    let mut value_ptr = std::ptr::null_mut();
    let mut out = HashMap::new();
    loop {
        let has_next = unsafe { PyDict_Next(kwargs, &mut position, &mut key_ptr, &mut value_ptr) };
        if has_next == 0 {
            break;
        }
        if key_ptr.is_null() || value_ptr.is_null() {
            return Err("failed to read keyword argument entry".to_string());
        }
        let key_value = cpython_value_from_ptr_or_proxy(key_ptr)
            .map_err(|_| "received unknown keyword name pointer".to_string())?;
        let Value::Str(name) = key_value else {
            return Err("keyword argument names must be str".to_string());
        };
        let value = cpython_value_from_ptr_or_proxy(value_ptr)
            .map_err(|_| "received unknown keyword value pointer".to_string())?;
        out.insert(name, value);
    }
    Ok(out)
}
