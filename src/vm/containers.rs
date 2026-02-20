use crate::runtime::{ObjRef, Object, RuntimeError, SetObject, Value, value_lookup_hash};

pub(crate) fn dedup_hashable_values(values: Vec<Value>) -> Result<Vec<Value>, RuntimeError> {
    let mut deduped = SetObject::new(Vec::new());
    for value in values {
        ensure_hashable(&value)?;
        deduped.insert(value);
    }
    Ok(deduped.to_vec())
}

pub(crate) fn dict_get_value(dict: &ObjRef, key: &Value) -> Option<Value> {
    let dict_kind = dict.kind();
    let entries = match &*dict_kind {
        Object::Dict(entries) => entries,
        _ => return None,
    };
    if let Some(hash) = value_lookup_hash(key) {
        entries.find_with_hash(key, hash).cloned()
    } else {
        entries.find(key).cloned()
    }
}

pub(crate) fn dict_set_value(dict: &ObjRef, key: Value, value: Value) {
    let mut dict_kind = dict.kind_mut();
    let entries = match &mut *dict_kind {
        Object::Dict(entries) => entries,
        _ => return,
    };
    entries.insert(key, value);
}

pub(crate) fn dict_remove_value(dict: &ObjRef, key: &Value) -> Option<Value> {
    let mut dict_kind = dict.kind_mut();
    let entries = match &mut *dict_kind {
        Object::Dict(entries) => entries,
        _ => return None,
    };
    if let Some(hash) = value_lookup_hash(key) {
        entries
            .remove_key_with_hash(key, hash)
            .map(|(_, value)| value)
    } else {
        entries.remove_key(key).map(|(_, value)| value)
    }
}

pub(crate) fn dict_contains_key_checked(dict: &ObjRef, key: &Value) -> Result<bool, RuntimeError> {
    let hash = value_lookup_hash(key)
        .ok_or_else(|| RuntimeError::new(format!("unhashable type: '{}'", value_type_name(key))))?;
    let dict_kind = dict.kind();
    let entries = match &*dict_kind {
        Object::Dict(entries) => entries,
        _ => return Err(RuntimeError::type_error("unsupported operand type for in")),
    };
    Ok(entries.contains_key_with_hash(key, hash))
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
        | Value::DictKeys(_)
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
        Value::DictKeys(_) => "dict_keys",
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

#[cfg(test)]
mod tests {
    use super::{
        ObjRef, dedup_hashable_values, dict_get_value, dict_remove_value, dict_set_value,
        dict_set_value_checked, ensure_hashable,
    };
    use crate::runtime::{Heap, Object, SetObject, Value};

    fn empty_dict_obj(heap: &Heap) -> ObjRef {
        let Value::Dict(dict) = heap.alloc_dict(Vec::new()) else {
            unreachable!("heap.alloc_dict must return Value::Dict");
        };
        dict
    }

    #[test]
    fn ensure_hashable_rejects_mutable_container_values() {
        let heap = Heap::new();
        let list = heap.alloc_list(vec![Value::Int(1)]);
        let dict = heap.alloc_dict(vec![(Value::Str("k".to_string()), Value::Int(1))]);
        let set = heap.alloc_set(vec![Value::Int(1)]);

        let list_err = ensure_hashable(&list).expect_err("list should be unhashable");
        assert!(list_err.message.contains("unhashable type: 'list'"));

        let dict_err = ensure_hashable(&dict).expect_err("dict should be unhashable");
        assert!(dict_err.message.contains("unhashable type: 'dict'"));

        let set_err = ensure_hashable(&set).expect_err("set should be unhashable");
        assert!(set_err.message.contains("unhashable type: 'set'"));
    }

    #[test]
    fn ensure_hashable_accepts_nested_hashable_values() {
        let heap = Heap::new();
        let tuple = heap.alloc_tuple(vec![
            Value::Int(1),
            Value::Str("x".to_string()),
            heap.alloc_tuple(vec![Value::Int(2), Value::Str("y".to_string())]),
        ]);
        let frozen = heap.alloc_frozenset(vec![Value::Int(1), Value::Str("ok".to_string())]);

        assert!(ensure_hashable(&tuple).is_ok());
        assert!(ensure_hashable(&frozen).is_ok());
    }

    #[test]
    fn dict_helpers_support_get_update_and_remove() {
        let heap = Heap::new();
        let dict = empty_dict_obj(&heap);
        let key = Value::Str("answer".to_string());

        assert_eq!(dict_get_value(&dict, &key), None);
        dict_set_value(&dict, key.clone(), Value::Int(41));
        assert_eq!(dict_get_value(&dict, &key), Some(Value::Int(41)));

        dict_set_value(&dict, key.clone(), Value::Int(42));
        assert_eq!(dict_get_value(&dict, &key), Some(Value::Int(42)));

        assert_eq!(dict_remove_value(&dict, &key), Some(Value::Int(42)));
        assert_eq!(dict_get_value(&dict, &key), None);
    }

    #[test]
    fn dict_set_value_checked_rejects_unhashable_key_without_mutation() {
        let heap = Heap::new();
        let dict = empty_dict_obj(&heap);
        let bad_key = heap.alloc_list(vec![Value::Int(1)]);

        let err =
            dict_set_value_checked(&dict, bad_key, Value::Int(5)).expect_err("key should fail");
        assert!(err.message.contains("unhashable type: 'list'"));

        match &*dict.kind() {
            Object::Dict(entries) => assert_eq!(entries.len(), 0),
            other => panic!("unexpected object kind: {other:?}"),
        }
    }

    #[test]
    fn dedup_hashable_values_removes_duplicates() {
        let values = vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(1),
            Value::Str("x".to_string()),
            Value::Str("x".to_string()),
        ];
        let deduped = dedup_hashable_values(values).expect("hashable values should dedup");

        assert_eq!(deduped.len(), 3);
        assert!(deduped.contains(&Value::Int(1)));
        assert!(deduped.contains(&Value::Int(2)));
        assert!(deduped.contains(&Value::Str("x".to_string())));
    }

    #[test]
    fn dict_bulk_insert_remove_keeps_hash_index_consistent() {
        let heap = Heap::new();
        let dict = empty_dict_obj(&heap);
        const COUNT: i64 = 3000;

        for value in 0..COUNT {
            dict_set_value_checked(&dict, Value::Int(value), Value::Int(value * 10))
                .expect("bulk insert should succeed");
        }
        for value in 0..COUNT {
            assert_eq!(
                dict_get_value(&dict, &Value::Int(value)),
                Some(Value::Int(value * 10))
            );
        }

        for value in (0..COUNT).step_by(3) {
            assert_eq!(
                dict_remove_value(&dict, &Value::Int(value)),
                Some(Value::Int(value * 10))
            );
        }

        for value in 0..COUNT {
            let expected = if value % 3 == 0 {
                None
            } else {
                Some(Value::Int(value * 10))
            };
            assert_eq!(dict_get_value(&dict, &Value::Int(value)), expected);
        }
    }

    #[test]
    fn set_bulk_insert_remove_keeps_membership_consistent() {
        const COUNT: i64 = 4000;
        let mut set = SetObject::new(Vec::new());

        for value in 0..COUNT {
            assert!(set.insert(Value::Int(value)));
        }
        for value in 0..COUNT {
            assert!(set.contains(&Value::Int(value)));
        }

        for value in (0..COUNT).step_by(4) {
            assert!(set.remove_value(&Value::Int(value)));
        }

        for value in 0..COUNT {
            let present = value % 4 != 0;
            assert_eq!(set.contains(&Value::Int(value)), present);
        }
    }

    #[test]
    fn dict_numeric_equivalent_keys_share_single_entry() {
        let heap = Heap::new();
        let dict = empty_dict_obj(&heap);
        dict_set_value_checked(&dict, Value::Int(1), Value::Str("int".to_string()))
            .expect("int key should insert");
        dict_set_value_checked(&dict, Value::Bool(true), Value::Str("bool".to_string()))
            .expect("bool key should update");
        dict_set_value_checked(&dict, Value::Float(1.0), Value::Str("float".to_string()))
            .expect("float key should update");

        let kind = dict.kind();
        let Object::Dict(entries) = &*kind else {
            panic!("expected dict object");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries.find(&Value::Int(1)),
            Some(&Value::Str("float".to_string()))
        );
    }
}
