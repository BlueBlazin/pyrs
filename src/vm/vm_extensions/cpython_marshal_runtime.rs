use crate::bytecode::cpython::PyObject as CpythonMarshalObject;
use crate::runtime::{Object, SliceValue, Value};

use super::Vm;

pub(super) fn value_to_cpython_marshal_object(
    value: &Value,
) -> Result<CpythonMarshalObject, String> {
    match value {
        Value::None => Ok(CpythonMarshalObject::None),
        Value::Bool(value) => Ok(CpythonMarshalObject::Bool(*value)),
        Value::Int(value) => Ok(CpythonMarshalObject::Int(*value)),
        Value::BigInt(value) => value
            .to_i64()
            .map(CpythonMarshalObject::Int)
            .ok_or_else(|| "cannot marshal bigint values outside i64 range".to_string()),
        Value::Float(value) => Ok(CpythonMarshalObject::Float(*value)),
        Value::Complex { real, imag } => Ok(CpythonMarshalObject::Complex {
            real: *real,
            imag: *imag,
        }),
        Value::Str(value) => Ok(CpythonMarshalObject::Str(value.clone())),
        Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
            Object::Bytes(payload) => Ok(CpythonMarshalObject::Bytes(payload.clone())),
            _ => Err("invalid bytes object storage".to_string()),
        },
        Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
            Object::Tuple(items) => items
                .iter()
                .map(value_to_cpython_marshal_object)
                .collect::<Result<Vec<_>, _>>()
                .map(CpythonMarshalObject::Tuple),
            _ => Err("invalid tuple object storage".to_string()),
        },
        Value::List(list_obj) => match &*list_obj.kind() {
            Object::List(items) => items
                .iter()
                .map(value_to_cpython_marshal_object)
                .collect::<Result<Vec<_>, _>>()
                .map(CpythonMarshalObject::List),
            _ => Err("invalid list object storage".to_string()),
        },
        Value::Dict(dict_obj) => match &*dict_obj.kind() {
            Object::Dict(entries) => entries
                .iter()
                .map(|(key, value)| {
                    Ok((
                        value_to_cpython_marshal_object(key)?,
                        value_to_cpython_marshal_object(value)?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()
                .map(CpythonMarshalObject::Dict),
            _ => Err("invalid dict object storage".to_string()),
        },
        Value::Set(set_obj) => match &*set_obj.kind() {
            Object::Set(entries) => entries
                .iter()
                .map(value_to_cpython_marshal_object)
                .collect::<Result<Vec<_>, _>>()
                .map(CpythonMarshalObject::Set),
            _ => Err("invalid set object storage".to_string()),
        },
        Value::FrozenSet(set_obj) => match &*set_obj.kind() {
            Object::FrozenSet(entries) => entries
                .iter()
                .map(value_to_cpython_marshal_object)
                .collect::<Result<Vec<_>, _>>()
                .map(CpythonMarshalObject::FrozenSet),
            _ => Err("invalid frozenset object storage".to_string()),
        },
        Value::Slice(slice) => Ok(CpythonMarshalObject::Slice {
            lower: slice
                .lower
                .map(|value| Box::new(CpythonMarshalObject::Int(value))),
            upper: slice
                .upper
                .map(|value| Box::new(CpythonMarshalObject::Int(value))),
            step: slice
                .step
                .map(|value| Box::new(CpythonMarshalObject::Int(value))),
        }),
        _ => Err("marshal unsupported value type".to_string()),
    }
}

pub(super) fn cpython_marshal_object_to_value(
    object: &CpythonMarshalObject,
    vm: &mut Vm,
) -> Result<Value, String> {
    match object {
        CpythonMarshalObject::Null => Ok(Value::None),
        CpythonMarshalObject::None => Ok(Value::None),
        CpythonMarshalObject::Bool(value) => Ok(Value::Bool(*value)),
        CpythonMarshalObject::Int(value) => Ok(Value::Int(*value)),
        CpythonMarshalObject::Float(value) => Ok(Value::Float(*value)),
        CpythonMarshalObject::Complex { real, imag } => Ok(Value::Complex {
            real: *real,
            imag: *imag,
        }),
        CpythonMarshalObject::Str(value) => Ok(Value::Str(value.clone())),
        CpythonMarshalObject::Bytes(bytes) => Ok(vm.heap.alloc_bytes(bytes.clone())),
        CpythonMarshalObject::Tuple(items) => items
            .iter()
            .map(|item| cpython_marshal_object_to_value(item, vm))
            .collect::<Result<Vec<_>, _>>()
            .map(|items| vm.heap.alloc_tuple(items)),
        CpythonMarshalObject::List(items) => items
            .iter()
            .map(|item| cpython_marshal_object_to_value(item, vm))
            .collect::<Result<Vec<_>, _>>()
            .map(|items| vm.heap.alloc_list(items)),
        CpythonMarshalObject::Dict(entries) => entries
            .iter()
            .map(|(key, value)| {
                Ok((
                    cpython_marshal_object_to_value(key, vm)?,
                    cpython_marshal_object_to_value(value, vm)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()
            .map(|entries| vm.heap.alloc_dict(entries)),
        CpythonMarshalObject::Set(items) => items
            .iter()
            .map(|item| cpython_marshal_object_to_value(item, vm))
            .collect::<Result<Vec<_>, _>>()
            .map(|items| vm.heap.alloc_set(items)),
        CpythonMarshalObject::FrozenSet(items) => items
            .iter()
            .map(|item| cpython_marshal_object_to_value(item, vm))
            .collect::<Result<Vec<_>, _>>()
            .map(|items| vm.heap.alloc_frozenset(items)),
        CpythonMarshalObject::Slice { lower, upper, step } => {
            let parse_int =
                |value: &Option<Box<CpythonMarshalObject>>| -> Result<Option<i64>, String> {
                    match value {
                        None => Ok(None),
                        Some(value) => match value.as_ref() {
                            CpythonMarshalObject::Int(value) => Ok(Some(*value)),
                            _ => Err("marshal slice bounds must decode to int".to_string()),
                        },
                    }
                };
            Ok(Value::Slice(Box::new(SliceValue {
                lower: parse_int(lower)?,
                upper: parse_int(upper)?,
                step: parse_int(step)?,
            })))
        }
        CpythonMarshalObject::Code(_) => {
            Err("marshal code objects are not supported in C-API decode".to_string())
        }
    }
}
