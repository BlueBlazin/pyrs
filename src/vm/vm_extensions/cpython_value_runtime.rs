use std::ffi::c_void;

use crate::runtime::{BuiltinFunction, DictViewKind, IteratorKind, Object, Value};

use super::{
    ObjRef, PyBaseObject_Type, PyBool_Type, PyByteArray_Type, PyByteArrayIter_Type, PyBytes_Type,
    PyBytesIter_Type, PyCallIter_Type, PyComplex_Type, PyCoro_Type, PyDictItems_Type,
    PyDictIterItem_Type, PyDictIterKey_Type, PyDictIterValue_Type, PyDictKeys_Type,
    PyDictRevIterItem_Type, PyDictRevIterKey_Type, PyDictRevIterValue_Type, PyDict_Type,
    PyDictValues_Type, PyEnum_Type, PyFilter_Type, PyFloat_Type, PyFrozenSet_Type,
    PyFunction_Type, PyGen_Type, PyListRevIter_Type, PyListIter_Type, PyList_Type, PyLong_Type,
    PyMap_Type, PyMemoryView_Type, PyMethod_Type, PyModule_Type, PyNone_Type, PyRange_Type,
    PyRangeIter_Type, PyReversed_Type, PySeqIter_Type, PySet_Type, PySetIter_Type,
    PySlice_Type, PySuper_Type, PyTuple_Type, PyTupleIter_Type, PyType_Type, PyUnicode_Type,
    PyUnicodeIter_Type, PyZip_Type, cpython_exception_ptr_for_name,
};

pub(super) fn cpython_type_for_value(value: &Value) -> *mut c_void {
    match value {
        Value::None => std::ptr::addr_of_mut!(PyNone_Type).cast(),
        Value::Bool(_) => std::ptr::addr_of_mut!(PyBool_Type).cast(),
        Value::Int(_) | Value::BigInt(_) => std::ptr::addr_of_mut!(PyLong_Type).cast(),
        Value::Float(_) => std::ptr::addr_of_mut!(PyFloat_Type).cast(),
        Value::Complex { .. } => std::ptr::addr_of_mut!(PyComplex_Type).cast(),
        Value::Str(_) => std::ptr::addr_of_mut!(PyUnicode_Type).cast(),
        Value::List(_) => std::ptr::addr_of_mut!(PyList_Type).cast(),
        Value::Tuple(_) => std::ptr::addr_of_mut!(PyTuple_Type).cast(),
        Value::Dict(_) => std::ptr::addr_of_mut!(PyDict_Type).cast(),
        Value::DictKeys(_) => std::ptr::addr_of_mut!(PyDictKeys_Type).cast(),
        Value::DictValues(_) => std::ptr::addr_of_mut!(PyDictValues_Type).cast(),
        Value::DictItems(_) => std::ptr::addr_of_mut!(PyDictItems_Type).cast(),
        Value::Set(_) => std::ptr::addr_of_mut!(PySet_Type).cast(),
        Value::FrozenSet(_) => std::ptr::addr_of_mut!(PyFrozenSet_Type).cast(),
        Value::Bytes(_) => std::ptr::addr_of_mut!(PyBytes_Type).cast(),
        Value::ByteArray(_) => std::ptr::addr_of_mut!(PyByteArray_Type).cast(),
        Value::MemoryView(_) => std::ptr::addr_of_mut!(PyMemoryView_Type).cast(),
        Value::Iterator(obj) => match &*obj.kind() {
            Object::Iterator(state) => match &state.kind {
                IteratorKind::List(_) => std::ptr::addr_of_mut!(PyListIter_Type).cast(),
                IteratorKind::ListReverse { .. } => {
                    std::ptr::addr_of_mut!(PyListRevIter_Type).cast()
                }
                IteratorKind::Tuple(_) => std::ptr::addr_of_mut!(PyTupleIter_Type).cast(),
                IteratorKind::Str(_) => std::ptr::addr_of_mut!(PyUnicodeIter_Type).cast(),
                IteratorKind::DictView { kind, .. } => match kind {
                    DictViewKind::Keys => std::ptr::addr_of_mut!(PyDictIterKey_Type).cast(),
                    DictViewKind::Values => std::ptr::addr_of_mut!(PyDictIterValue_Type).cast(),
                    DictViewKind::Items => std::ptr::addr_of_mut!(PyDictIterItem_Type).cast(),
                },
                IteratorKind::DictReverse { kind, .. } => match kind {
                    DictViewKind::Keys => std::ptr::addr_of_mut!(PyDictRevIterKey_Type).cast(),
                    DictViewKind::Values => std::ptr::addr_of_mut!(PyDictRevIterValue_Type).cast(),
                    DictViewKind::Items => std::ptr::addr_of_mut!(PyDictRevIterItem_Type).cast(),
                },
                IteratorKind::Set(_) => std::ptr::addr_of_mut!(PySetIter_Type).cast(),
                IteratorKind::Bytes(_) => std::ptr::addr_of_mut!(PyBytesIter_Type).cast(),
                IteratorKind::ByteArray(_) => std::ptr::addr_of_mut!(PyByteArrayIter_Type).cast(),
                IteratorKind::Map { .. } => std::ptr::addr_of_mut!(PyMap_Type).cast(),
                IteratorKind::Zip { .. } => std::ptr::addr_of_mut!(PyZip_Type).cast(),
                IteratorKind::Filter { .. } => std::ptr::addr_of_mut!(PyFilter_Type).cast(),
                IteratorKind::RangeObject { .. } => std::ptr::addr_of_mut!(PyRange_Type).cast(),
                IteratorKind::Range { .. } => std::ptr::addr_of_mut!(PyRangeIter_Type).cast(),
                IteratorKind::Enumerate { .. } => std::ptr::addr_of_mut!(PyEnum_Type).cast(),
                IteratorKind::CallIter { .. } => std::ptr::addr_of_mut!(PyCallIter_Type).cast(),
                IteratorKind::ReversedSequenceGetItem { .. }
                | IteratorKind::ReversedCpythonSequence { .. } => {
                    std::ptr::addr_of_mut!(PyReversed_Type).cast()
                }
                IteratorKind::SequenceGetItem { .. }
                | IteratorKind::CpythonSequence { .. }
                | IteratorKind::MemoryView(_)
                | IteratorKind::Cycle { .. }
                | IteratorKind::Count { .. }
                | IteratorKind::Chain { .. }
                | IteratorKind::ChainFromIterable { .. }
                | IteratorKind::Accumulate { .. }
                | IteratorKind::Combinations { .. }
                | IteratorKind::CombinationsWithReplacement { .. }
                | IteratorKind::Permutations { .. }
                | IteratorKind::Product { .. }
                | IteratorKind::Compress { .. }
                | IteratorKind::DropWhile { .. }
                | IteratorKind::FilterFalse { .. }
                | IteratorKind::Islice { .. }
                | IteratorKind::Pairwise { .. }
                | IteratorKind::StarMap { .. }
                | IteratorKind::TakeWhile { .. }
                | IteratorKind::ZipLongest { .. }
                | IteratorKind::Tee { .. }
                | IteratorKind::Repeat { .. }
                | IteratorKind::Batched { .. }
                | IteratorKind::GroupBy { .. }
                | IteratorKind::GroupByGrouper { .. } => {
                    std::ptr::addr_of_mut!(PySeqIter_Type).cast()
                }
            },
            _ => std::ptr::addr_of_mut!(PyBaseObject_Type).cast(),
        },
        Value::Generator(generator_obj) => match &*generator_obj.kind() {
            Object::Generator(generator_state) if generator_state.is_coroutine => {
                std::ptr::addr_of_mut!(PyCoro_Type).cast()
            }
            _ => std::ptr::addr_of_mut!(PyGen_Type).cast(),
        },
        Value::Module(_) => std::ptr::addr_of_mut!(PyModule_Type).cast(),
        Value::Slice(_) => std::ptr::addr_of_mut!(PySlice_Type).cast(),
        Value::Super(_) => std::ptr::addr_of_mut!(PySuper_Type).cast(),
        Value::Function(_) => std::ptr::addr_of_mut!(PyFunction_Type).cast(),
        Value::BoundMethod(_) => std::ptr::addr_of_mut!(PyMethod_Type).cast(),
        Value::Exception(exception_obj) => cpython_exception_ptr_for_name(&exception_obj.name)
            .unwrap_or_else(|| std::ptr::addr_of_mut!(PyBaseObject_Type).cast()),
        Value::Class(_) => std::ptr::addr_of_mut!(PyType_Type).cast(),
        Value::Builtin(_) => std::ptr::addr_of_mut!(PyBaseObject_Type).cast(),
        _ => std::ptr::addr_of_mut!(PyBaseObject_Type).cast(),
    }
}

pub(super) fn cpython_objref_from_value(value: Value) -> Option<ObjRef> {
    match value {
        Value::List(obj)
        | Value::Tuple(obj)
        | Value::Dict(obj)
        | Value::DictKeys(obj)
        | Value::DictValues(obj)
        | Value::DictItems(obj)
        | Value::Set(obj)
        | Value::FrozenSet(obj)
        | Value::Bytes(obj)
        | Value::ByteArray(obj)
        | Value::MemoryView(obj)
        | Value::Iterator(obj)
        | Value::Generator(obj)
        | Value::Module(obj)
        | Value::Class(obj)
        | Value::Instance(obj)
        | Value::Super(obj)
        | Value::BoundMethod(obj)
        | Value::Function(obj)
        | Value::Cell(obj) => Some(obj),
        _ => None,
    }
}

pub(super) fn cpython_builtin_type_ptr_for_class_name(class_name: &str) -> Option<*mut c_void> {
    Some(match class_name {
        "type" => std::ptr::addr_of_mut!(PyType_Type).cast(),
        "object" => std::ptr::addr_of_mut!(PyBaseObject_Type).cast(),
        "bool" => std::ptr::addr_of_mut!(PyBool_Type).cast(),
        "int" => std::ptr::addr_of_mut!(PyLong_Type).cast(),
        "float" => std::ptr::addr_of_mut!(PyFloat_Type).cast(),
        "complex" => std::ptr::addr_of_mut!(PyComplex_Type).cast(),
        "function" => std::ptr::addr_of_mut!(PyFunction_Type).cast(),
        "coroutine" => std::ptr::addr_of_mut!(PyCoro_Type).cast(),
        "str" => std::ptr::addr_of_mut!(PyUnicode_Type).cast(),
        "bytes" => std::ptr::addr_of_mut!(PyBytes_Type).cast(),
        "bytearray" => std::ptr::addr_of_mut!(PyByteArray_Type).cast(),
        "memoryview" => std::ptr::addr_of_mut!(PyMemoryView_Type).cast(),
        "list" => std::ptr::addr_of_mut!(PyList_Type).cast(),
        "tuple" => std::ptr::addr_of_mut!(PyTuple_Type).cast(),
        "dict" => std::ptr::addr_of_mut!(PyDict_Type).cast(),
        "set" => std::ptr::addr_of_mut!(PySet_Type).cast(),
        "frozenset" => std::ptr::addr_of_mut!(PyFrozenSet_Type).cast(),
        "slice" => std::ptr::addr_of_mut!(PySlice_Type).cast(),
        "range" => std::ptr::addr_of_mut!(PyRange_Type).cast(),
        _ => return None,
    })
}

pub(super) fn cpython_builtin_type_name_for_ptr(ptr: *mut c_void) -> Option<&'static str> {
    if ptr == std::ptr::addr_of_mut!(PyType_Type).cast() {
        Some("type")
    } else if ptr == std::ptr::addr_of_mut!(PyBaseObject_Type).cast() {
        Some("object")
    } else if ptr == std::ptr::addr_of_mut!(PyBool_Type).cast() {
        Some("bool")
    } else if ptr == std::ptr::addr_of_mut!(PyLong_Type).cast() {
        Some("int")
    } else if ptr == std::ptr::addr_of_mut!(PyFloat_Type).cast() {
        Some("float")
    } else if ptr == std::ptr::addr_of_mut!(PyComplex_Type).cast() {
        Some("complex")
    } else if ptr == std::ptr::addr_of_mut!(PyFunction_Type).cast() {
        Some("function")
    } else if ptr == std::ptr::addr_of_mut!(PyCoro_Type).cast() {
        Some("coroutine")
    } else if ptr == std::ptr::addr_of_mut!(PyUnicode_Type).cast() {
        Some("str")
    } else if ptr == std::ptr::addr_of_mut!(PyBytes_Type).cast() {
        Some("bytes")
    } else if ptr == std::ptr::addr_of_mut!(PyByteArray_Type).cast() {
        Some("bytearray")
    } else if ptr == std::ptr::addr_of_mut!(PyMemoryView_Type).cast() {
        Some("memoryview")
    } else if ptr == std::ptr::addr_of_mut!(PyList_Type).cast() {
        Some("list")
    } else if ptr == std::ptr::addr_of_mut!(PyTuple_Type).cast() {
        Some("tuple")
    } else if ptr == std::ptr::addr_of_mut!(PyDict_Type).cast() {
        Some("dict")
    } else if ptr == std::ptr::addr_of_mut!(PySet_Type).cast() {
        Some("set")
    } else if ptr == std::ptr::addr_of_mut!(PyFrozenSet_Type).cast() {
        Some("frozenset")
    } else if ptr == std::ptr::addr_of_mut!(PySlice_Type).cast() {
        Some("slice")
    } else if ptr == std::ptr::addr_of_mut!(PyRange_Type).cast() {
        Some("range")
    } else {
        None
    }
}

pub(super) fn cpython_builtin_type_ptr_for_builtin(
    builtin: &BuiltinFunction,
) -> Option<*mut c_void> {
    Some(match builtin {
        BuiltinFunction::Type => std::ptr::addr_of_mut!(PyType_Type).cast(),
        BuiltinFunction::Slice => std::ptr::addr_of_mut!(PySlice_Type).cast(),
        BuiltinFunction::Bool => std::ptr::addr_of_mut!(PyBool_Type).cast(),
        BuiltinFunction::Int => std::ptr::addr_of_mut!(PyLong_Type).cast(),
        BuiltinFunction::Float => std::ptr::addr_of_mut!(PyFloat_Type).cast(),
        BuiltinFunction::Complex => std::ptr::addr_of_mut!(PyComplex_Type).cast(),
        BuiltinFunction::CoroutineType => std::ptr::addr_of_mut!(PyCoro_Type).cast(),
        BuiltinFunction::Str => std::ptr::addr_of_mut!(PyUnicode_Type).cast(),
        BuiltinFunction::List => std::ptr::addr_of_mut!(PyList_Type).cast(),
        BuiltinFunction::Tuple => std::ptr::addr_of_mut!(PyTuple_Type).cast(),
        BuiltinFunction::Dict => std::ptr::addr_of_mut!(PyDict_Type).cast(),
        BuiltinFunction::Set => std::ptr::addr_of_mut!(PySet_Type).cast(),
        BuiltinFunction::FrozenSet => std::ptr::addr_of_mut!(PyFrozenSet_Type).cast(),
        BuiltinFunction::Bytes => std::ptr::addr_of_mut!(PyBytes_Type).cast(),
        BuiltinFunction::ByteArray => std::ptr::addr_of_mut!(PyByteArray_Type).cast(),
        BuiltinFunction::MemoryView => std::ptr::addr_of_mut!(PyMemoryView_Type).cast(),
        BuiltinFunction::Range => std::ptr::addr_of_mut!(PyRange_Type).cast(),
        _ => return None,
    })
}

pub(super) fn cpython_value_debug_tag(value: &Value) -> String {
    match value {
        Value::None => "None".to_string(),
        Value::Bool(flag) => format!("Bool({flag})"),
        Value::Int(_) => "Int".to_string(),
        Value::BigInt(_) => "BigInt".to_string(),
        Value::Float(_) => "Float".to_string(),
        Value::Complex { .. } => "Complex".to_string(),
        Value::Str(_) => "Str".to_string(),
        Value::List(_) => "List".to_string(),
        Value::Tuple(_) => "Tuple".to_string(),
        Value::Dict(_) => "Dict".to_string(),
        Value::DictKeys(_) => "DictKeys".to_string(),
        Value::DictValues(_) => "DictValues".to_string(),
        Value::DictItems(_) => "DictItems".to_string(),
        Value::Set(_) => "Set".to_string(),
        Value::FrozenSet(_) => "FrozenSet".to_string(),
        Value::Bytes(_) => "Bytes".to_string(),
        Value::ByteArray(_) => "ByteArray".to_string(),
        Value::MemoryView(_) => "MemoryView".to_string(),
        Value::Iterator(_) => "Iterator".to_string(),
        Value::Generator(_) => "Generator".to_string(),
        Value::Module(module) => {
            if let Object::Module(data) = &*module.kind() {
                format!("Module({})", data.name)
            } else {
                "Module(<invalid>)".to_string()
            }
        }
        Value::Class(class) => {
            if let Object::Class(data) = &*class.kind() {
                format!("Class({})", data.name)
            } else {
                "Class(<invalid>)".to_string()
            }
        }
        Value::Instance(_) => "Instance".to_string(),
        Value::Super(_) => "Super".to_string(),
        Value::BoundMethod(bound_obj) => {
            if let Object::BoundMethod(bound_data) = &*bound_obj.kind()
                && let Object::Function(func_data) = &*bound_data.function.kind()
            {
                format!("BoundMethod({})", func_data.code.name)
            } else {
                "BoundMethod".to_string()
            }
        }
        Value::Function(func_obj) => {
            if let Object::Function(func_data) = &*func_obj.kind() {
                format!(
                    "Function({}@{})",
                    func_data.code.name, func_data.code.filename
                )
            } else {
                "Function".to_string()
            }
        }
        Value::Cell(_) => "Cell".to_string(),
        Value::Exception(err) => format!("Exception({})", err.name),
        Value::ExceptionType(name) => format!("ExceptionType({name})"),
        Value::Slice(_) => "Slice".to_string(),
        Value::Code(_) => "Code".to_string(),
        Value::Builtin(builtin) => format!("Builtin({builtin:?})"),
    }
}

pub(super) fn cpython_debug_ufunc_attr_summary(value: &Value, depth: usize) -> String {
    if depth == 0 {
        return cpython_value_debug_tag(value);
    }
    match value {
        Value::None => "None".to_string(),
        Value::Bool(flag) => format!("Bool({flag})"),
        Value::Int(number) => format!("Int({number})"),
        Value::Float(number) => format!("Float({number})"),
        Value::Str(text) => format!("Str({text})"),
        Value::Class(class_obj) => {
            if let Object::Class(class_data) = &*class_obj.kind() {
                format!("Class({})", class_data.name)
            } else {
                "Class(<invalid>)".to_string()
            }
        }
        Value::Instance(instance_obj) => {
            if let Object::Instance(instance_data) = &*instance_obj.kind() {
                if let Object::Class(class_data) = &*instance_data.class.kind() {
                    return format!("Instance({})", class_data.name);
                }
            }
            "Instance".to_string()
        }
        Value::Tuple(tuple_obj) => {
            if let Object::Tuple(items) = &*tuple_obj.kind() {
                let rendered = items
                    .iter()
                    .take(6)
                    .map(|item| cpython_debug_ufunc_attr_summary(item, depth - 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                if items.len() > 6 {
                    format!("Tuple(len={}, [{} ...])", items.len(), rendered)
                } else {
                    format!("Tuple([{}])", rendered)
                }
            } else {
                "Tuple(<invalid>)".to_string()
            }
        }
        Value::List(list_obj) => {
            if let Object::List(items) = &*list_obj.kind() {
                let rendered = items
                    .iter()
                    .take(6)
                    .map(|item| cpython_debug_ufunc_attr_summary(item, depth - 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                if items.len() > 6 {
                    format!("List(len={}, [{} ...])", items.len(), rendered)
                } else {
                    format!("List([{}])", rendered)
                }
            } else {
                "List(<invalid>)".to_string()
            }
        }
        _ => cpython_value_debug_tag(value),
    }
}

pub(super) fn cpython_debug_ufunc_exception_summary(value: &Value) -> String {
    match value {
        Value::Exception(exception_obj) => {
            let attrs = exception_obj.attrs.borrow();
            let mut parts = Vec::new();
            for key in ["ufunc", "dtypes", "casting", "signature"] {
                if let Some(attr_value) = attrs.get(key) {
                    parts.push(format!(
                        "{}={}",
                        key,
                        cpython_debug_ufunc_attr_summary(attr_value, 3)
                    ));
                }
            }
            if parts.is_empty() {
                format!("Exception({})", exception_obj.name)
            } else {
                format!("Exception({}; {})", exception_obj.name, parts.join(", "))
            }
        }
        _ => cpython_debug_ufunc_attr_summary(value, 3),
    }
}
