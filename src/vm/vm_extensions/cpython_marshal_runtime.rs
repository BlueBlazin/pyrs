use std::rc::Rc;

use crate::bytecode::cpython::{PyObject as CpythonMarshalObject, translate_code};
use crate::bytecode::cpython::CpythonCode;
use crate::{compiler, parser};
use crate::runtime::{Object, SliceValue, Value};

use super::Vm;

const PYRS_MARSHALLED_SOURCE_CODE_MAGIC: &[u8] = b"PYRS_SRC_CODE_V1";

fn source_marshaled_code_object(
    code: &crate::bytecode::CodeObject,
    source: String,
) -> CpythonCode {
    CpythonCode {
        argcount: 0,
        posonlyargcount: 0,
        kwonlyargcount: 0,
        stacksize: 0,
        flags: 0,
        code: PYRS_MARSHALLED_SOURCE_CODE_MAGIC.to_vec(),
        consts: vec![CpythonMarshalObject::Str(source)],
        names: Vec::new(),
        localsplusnames: Vec::new(),
        localspluskinds: Vec::new(),
        filename: code.filename.clone(),
        name: code.name.clone(),
        qualname: code.name.clone(),
        firstlineno: code.first_line as i32,
        linetable: Vec::new(),
        exceptiontable: Vec::new(),
    }
}

fn source_marshaled_code_to_value(code: &CpythonCode, _vm: &mut Vm) -> Result<Value, String> {
    if code.code != PYRS_MARSHALLED_SOURCE_CODE_MAGIC {
        return Err("unsupported source-backed marshalled code header".to_string());
    }
    let Some(CpythonMarshalObject::Str(source_text)) = code.consts.first() else {
        return Err("source-backed marshalled code missing source text".to_string());
    };
    let module_ast = parser::parse_module(source_text)
        .map_err(|err| format!("marshal source parse error: {}", err.message))?;
    let compiled = compiler::compile_module_with_filename(&module_ast, &code.filename)
        .map_err(|err| err.message)?;
    Ok(Value::Code(Rc::new(compiled)))
}

pub(super) fn value_to_cpython_marshal_object(
    value: &Value,
) -> Result<CpythonMarshalObject, String> {
    match value {
        Value::None => Ok(CpythonMarshalObject::None),
        Value::Bool(value) => Ok(CpythonMarshalObject::Bool(*value)),
        Value::Int(value) => Ok(CpythonMarshalObject::Int(*value)),
        Value::ExceptionType(name) if name == "StopIteration" => {
            Ok(CpythonMarshalObject::StopIteration)
        }
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
        CpythonMarshalObject::StopIteration => {
            Ok(Value::ExceptionType("StopIteration".to_string()))
        }
        CpythonMarshalObject::Ellipsis => Ok(vm.heap.ellipsis_singleton()),
        CpythonMarshalObject::Bool(value) => Ok(Value::Bool(*value)),
        CpythonMarshalObject::Int(value) => Ok(Value::Int(*value)),
        CpythonMarshalObject::BigInt(value) => match value.to_i64() {
            Some(integer) => Ok(Value::Int(integer)),
            None => Ok(Value::BigInt(Box::new(value.clone()))),
        },
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
        CpythonMarshalObject::Code(code)
            if code.code == PYRS_MARSHALLED_SOURCE_CODE_MAGIC =>
        {
            source_marshaled_code_to_value(code, vm)
        }
        CpythonMarshalObject::Code(code) => translate_code(code, &mut vm.heap)
            .map(|translated| Value::Code(Rc::new(translated)))
            .map_err(|err| err.message),
    }
}

impl Vm {
    pub(crate) fn marshal_object_to_value(
        &mut self,
        object: &CpythonMarshalObject,
    ) -> Result<Value, String> {
        cpython_marshal_object_to_value(object, self)
    }

    pub(crate) fn value_to_marshal_object(
        &self,
        value: &Value,
    ) -> Result<CpythonMarshalObject, String> {
        if let Value::Code(code) = value {
            if code.name != "<module>" {
                return Err("marshal only supports source-backed module code objects".to_string());
            }
            let source = self
                .compiled_code_metadata(code)
                .and_then(|metadata| metadata.source.clone())
                .or_else(|| self.source_text_cache.get(&code.filename).map(|lines| lines.join("\n")))
                .or_else(|| {
                    if code.filename.starts_with('<') {
                        None
                    } else {
                        std::fs::read_to_string(&code.filename).ok()
                    }
                })
                .ok_or_else(|| "marshal code objects require source text".to_string())?;
            return Ok(CpythonMarshalObject::Code(Rc::new(
                source_marshaled_code_object(code, source),
            )));
        }
        value_to_cpython_marshal_object(value)
    }
}
