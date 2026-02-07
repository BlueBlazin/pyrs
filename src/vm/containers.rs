use crate::runtime::{ObjRef, Object, RuntimeError, Value};

pub(crate) fn dedup_hashable_values(values: Vec<Value>) -> Result<Vec<Value>, RuntimeError> {
    let mut out = Vec::new();
    for value in values {
        ensure_hashable(&value)?;
        if !out.iter().any(|existing| *existing == value) {
            out.push(value);
        }
    }
    Ok(out)
}

pub(crate) fn dict_get_value(dict: &ObjRef, key: &Value) -> Option<Value> {
    let dict_kind = dict.kind();
    let entries = match &*dict_kind {
        Object::Dict(entries) => entries,
        _ => return None,
    };
    entries.find(key).cloned()
}

pub(crate) fn dict_set_value(dict: &ObjRef, key: Value, value: Value) {
    let mut dict_kind = dict.kind_mut();
    let entries = match &mut *dict_kind {
        Object::Dict(entries) => entries,
        _ => return,
    };
    entries.insert(key, value);
}

pub(crate) fn dict_set_value_checked(
    dict: &ObjRef,
    key: Value,
    value: Value,
) -> Result<(), RuntimeError> {
    ensure_hashable(&key)?;
    dict_set_value(dict, key, value);
    Ok(())
}

pub(crate) fn ensure_hashable(value: &Value) -> Result<(), RuntimeError> {
    if is_hashable(value) {
        Ok(())
    } else {
        Err(RuntimeError::new(format!(
            "unhashable type: '{}'",
            value_type_name(value)
        )))
    }
}

fn is_hashable(value: &Value) -> bool {
    match value {
        Value::List(_)
        | Value::Dict(_)
        | Value::Set(_)
        | Value::ByteArray(_)
        | Value::Slice { .. } => false,
        Value::MemoryView(_) => false,
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => values.iter().all(is_hashable),
            _ => false,
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => values.iter().all(is_hashable),
            _ => false,
        },
        _ => true,
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::None => "NoneType",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::BigInt(_) => "int",
        Value::Float(_) => "float",
        Value::Complex { .. } => "complex",
        Value::Str(_) => "str",
        Value::List(_) => "list",
        Value::Tuple(_) => "tuple",
        Value::Dict(_) => "dict",
        Value::Set(_) => "set",
        Value::FrozenSet(_) => "frozenset",
        Value::Bytes(_) => "bytes",
        Value::ByteArray(_) => "bytearray",
        Value::MemoryView(_) => "memoryview",
        Value::Iterator(_) => "iterator",
        Value::Generator(_) => "generator",
        Value::Module(_) => "module",
        Value::Class(_) => "type",
        Value::Instance(_) => "object",
        Value::Super(_) => "super",
        Value::Function(_) => "function",
        Value::BoundMethod(_) => "method",
        Value::Exception(_) => "exception",
        Value::ExceptionType(_) => "exceptiontype",
        Value::Slice { .. } => "slice",
        Value::Code(_) => "code",
        Value::Builtin(_) => "builtin_function_or_method",
        Value::Cell(_) => "cell",
    }
}
