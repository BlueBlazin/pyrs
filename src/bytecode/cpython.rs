use std::collections::HashMap;
use std::rc::Rc;

use crate::bytecode::{CodeObject, Instruction, Opcode};
use crate::bytecode::metadata::OpcodeMetadata;
use crate::bytecode::pyc::parse_pyc_header;
use crate::runtime::{Heap, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpythonError {
    pub message: String,
}

impl CpythonError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CpythonCode {
    pub argcount: i32,
    pub posonlyargcount: i32,
    pub kwonlyargcount: i32,
    pub stacksize: i32,
    pub flags: i32,
    pub code: Vec<u8>,
    pub consts: Vec<PyObject>,
    pub names: Vec<String>,
    pub localsplusnames: Vec<String>,
    pub localspluskinds: Vec<u8>,
    pub filename: String,
    pub name: String,
    pub qualname: String,
    pub firstlineno: i32,
    pub linetable: Vec<u8>,
    pub exceptiontable: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum PyObject {
    Null,
    None,
    Bool(bool),
    Int(i64),
    Str(String),
    Bytes(Vec<u8>),
    Tuple(Vec<PyObject>),
    List(Vec<PyObject>),
    Dict(Vec<(PyObject, PyObject)>),
    Code(Rc<CpythonCode>),
    Slice {
        lower: Option<Box<PyObject>>,
        upper: Option<Box<PyObject>>,
        step: Option<Box<PyObject>>,
    },
}

pub fn load_pyc(bytes: &[u8]) -> Result<CpythonCode, CpythonError> {
    let (_header, offset) = parse_pyc_header(bytes)
        .map_err(|err| CpythonError::new(err.message))?;
    let mut reader = MarshalReader::new(&bytes[offset..]);
    let obj = reader.read_object(true)?;
    match obj {
        PyObject::Code(code) => Ok((*code).clone()),
        _ => Err(CpythonError::new("pyc did not contain a code object")),
    }
}

pub fn translate_code(code: &CpythonCode, heap: &mut Heap) -> Result<CodeObject, CpythonError> {
    let mut translator = Translator::new(code, heap)?;
    translator.translate()
}

struct Translator<'a> {
    code: &'a CpythonCode,
    heap: &'a mut Heap,
    opmap: HashMap<u8, String>,
    names: Vec<String>,
    name_index: HashMap<String, u32>,
    locals_map: Vec<u32>,
    names_map: Vec<u32>,
    constants: Vec<Value>,
}

impl<'a> Translator<'a> {
    fn new(code: &'a CpythonCode, heap: &'a mut Heap) -> Result<Self, CpythonError> {
        let metadata = OpcodeMetadata::load_default()
            .map_err(|err| CpythonError::new(err.message))?;
        let mut opmap = HashMap::new();
        for info in metadata.opcodes {
            opmap.insert(info.code as u8, info.name);
        }

        Ok(Self {
            code,
            heap,
            opmap,
            names: Vec::new(),
            name_index: HashMap::new(),
            locals_map: Vec::new(),
            names_map: Vec::new(),
            constants: Vec::new(),
        })
    }

    fn translate(&mut self) -> Result<CodeObject, CpythonError> {
        self.build_name_maps()?;
        self.constants = self.convert_constants(&self.code.consts)?;

        let mut result = CodeObject::new(self.code.name.clone());
        result.constants = self.constants.clone();
        result.names = self.names.clone();
        self.populate_params(&mut result)?;

        let instructions = self.translate_instructions()?;
        result.instructions = instructions;
        result.constants = self.constants.clone();
        Ok(result)
    }

    fn build_name_maps(&mut self) -> Result<(), CpythonError> {
        for name in &self.code.localsplusnames {
            let idx = self.intern_name(name);
            self.locals_map.push(idx);
        }
        for name in &self.code.names {
            let idx = self.intern_name(name);
            self.names_map.push(idx);
        }
        Ok(())
    }

    fn intern_name(&mut self, name: &str) -> u32 {
        if let Some(idx) = self.name_index.get(name) {
            return *idx;
        }
        let idx = self.names.len() as u32;
        self.names.push(name.to_string());
        self.name_index.insert(name.to_string(), idx);
        idx
    }

    fn populate_params(&self, result: &mut CodeObject) -> Result<(), CpythonError> {
        let total_posonly = self.code.posonlyargcount as usize;
        let total_pos = self.code.argcount as usize;
        let total_kwonly = self.code.kwonlyargcount as usize;
        let mut idx = 0;
        if self.code.localsplusnames.len() < total_posonly + total_pos + total_kwonly {
            return Err(CpythonError::new("localsplusnames too short for args"));
        }
        result.posonly_params = self.code.localsplusnames[idx..idx + total_posonly].to_vec();
        idx += total_posonly;
        result.params = self.code.localsplusnames[idx..idx + total_pos].to_vec();
        idx += total_pos;
        result.kwonly_params = self.code.localsplusnames[idx..idx + total_kwonly].to_vec();
        idx += total_kwonly;

        let flags = self.code.flags as u32;
        if flags & 0x0004 != 0 {
            if let Some(name) = self.code.localsplusnames.get(idx) {
                result.vararg = Some(name.clone());
                idx += 1;
            }
        }
        if flags & 0x0008 != 0 {
            if let Some(name) = self.code.localsplusnames.get(idx) {
                result.kwarg = Some(name.clone());
            }
        }
        Ok(())
    }

    fn translate_instructions(&mut self) -> Result<Vec<Instruction>, CpythonError> {
        let cp_instructions = decode_instructions(&self.code.code, &self.opmap)?;
        let mut result = Vec::with_capacity(cp_instructions.len());
        let mut pending_kw_names: Option<u16> = None;

        for (idx, instr) in cp_instructions.iter().enumerate() {
            let name = instr.name.as_str();
            let arg = instr.arg;
            let instruction = match name {
                "CACHE" | "RESUME" | "NOP" | "NOT_TAKEN" | "EXTENDED_ARG" => {
                    Instruction::new(Opcode::Nop, None)
                }
                "POP_TOP" => Instruction::new(Opcode::PopTop, None),
                "POP_ITER" => Instruction::new(Opcode::Nop, None),
                "RETURN_VALUE" => Instruction::new(Opcode::ReturnValue, None),
                "RETURN_CONST" => Instruction::new(Opcode::ReturnConst, Some(arg)),
                "LOAD_CONST" | "LOAD_CONST_MORTAL" | "LOAD_CONST_IMMORTAL" => {
                    Instruction::new(Opcode::LoadConst, Some(arg))
                }
                "LOAD_SMALL_INT" => {
                    let idx = self.add_const(Value::Int(arg as i64));
                    Instruction::new(Opcode::LoadConst, Some(idx))
                }
                "LOAD_COMMON_CONSTANT" => {
                    let value = match arg {
                        0 => Value::ExceptionType("AssertionError".to_string()),
                        1 => Value::ExceptionType("NotImplementedError".to_string()),
                        2 => self.heap.alloc_tuple(Vec::new()),
                        3 => Value::Builtin(crate::runtime::BuiltinFunction::All),
                        4 => Value::Builtin(crate::runtime::BuiltinFunction::Any),
                        _ => Value::None,
                    };
                    let idx = self.add_const(value);
                    Instruction::new(Opcode::LoadConst, Some(idx))
                }
                "LOAD_NAME" => Instruction::new(Opcode::LoadName, Some(self.map_name(arg)?)),
                "STORE_NAME" => Instruction::new(Opcode::StoreName, Some(self.map_name(arg)?)),
                "LOAD_GLOBAL" | "LOAD_GLOBAL_ADAPTIVE" | "LOAD_GLOBAL_BUILTIN"
                | "LOAD_GLOBAL_MODULE" => {
                    let name_idx = (arg >> 1) as u32;
                    let push_null = (arg & 1) as u32;
                    let mapped = self.map_name(name_idx)?;
                    let encoded = (mapped << 1) | push_null;
                    Instruction::new(Opcode::LoadGlobal, Some(encoded))
                }
                name if name.starts_with("LOAD_FAST") => {
                    match name {
                        "LOAD_FAST_LOAD_FAST" | "LOAD_FAST_BORROW_LOAD_FAST_BORROW" => {
                            let first = (arg >> 4) & 0x0F;
                            let second = arg & 0x0F;
                            let first = self.map_local(first)?;
                            let second = self.map_local(second)?;
                            let encoded = (first << 16) | second;
                            Instruction::new(Opcode::LoadFast2, Some(encoded))
                        }
                        "LOAD_FAST_AND_CLEAR" => {
                            Instruction::new(Opcode::LoadFastAndClear, Some(self.map_local(arg)?))
                        }
                        _ => Instruction::new(Opcode::LoadFast, Some(self.map_local(arg)?)),
                    }
                }
                name if name.starts_with("STORE_FAST") => match name {
                    "STORE_FAST_LOAD_FAST" => {
                        let first = (arg >> 4) & 0x0F;
                        let second = arg & 0x0F;
                        let first = self.map_local(first)?;
                        let second = self.map_local(second)?;
                        let encoded = (first << 16) | second;
                        Instruction::new(Opcode::StoreFastLoadFast, Some(encoded))
                    }
                    "STORE_FAST_STORE_FAST" => {
                        let first = (arg >> 4) & 0x0F;
                        let second = arg & 0x0F;
                        let first = self.map_local(first)?;
                        let second = self.map_local(second)?;
                        let encoded = (first << 16) | second;
                        Instruction::new(Opcode::StoreFastStoreFast, Some(encoded))
                    }
                    _ => Instruction::new(Opcode::StoreFast, Some(self.map_local(arg)?)),
                },
                "STORE_GLOBAL" => Instruction::new(Opcode::StoreGlobal, Some(self.map_name(arg)?)),
                name if name.starts_with("LOAD_ATTR") => {
                    let name_idx = (arg >> 1) as u32;
                    let push_null = (arg & 1) as u32;
                    let mapped = self.map_name(name_idx)?;
                    let encoded = (mapped << 1) | push_null;
                    Instruction::new(Opcode::LoadAttr, Some(encoded))
                }
                name if name.starts_with("STORE_ATTR") => {
                    Instruction::new(Opcode::StoreAttrCpython, Some(self.map_name(arg)?))
                }
                "LOAD_BUILD_CLASS" => Instruction::new(Opcode::LoadBuildClass, None),
                "PUSH_NULL" => Instruction::new(Opcode::PushNull, None),
                "MAKE_FUNCTION" => Instruction::new(Opcode::MakeFunctionStack, None),
                "SET_FUNCTION_ATTRIBUTE" => {
                    Instruction::new(Opcode::SetFunctionAttribute, Some(arg))
                }
                "KW_NAMES" => {
                    pending_kw_names = Some(arg as u16);
                    Instruction::new(Opcode::Nop, None)
                }
                "CALL" => {
                    let kw_idx = pending_kw_names.take().unwrap_or(u16::MAX);
                    let encoded = ((kw_idx as u32) << 16) | (arg & 0xFFFF);
                    Instruction::new(Opcode::CallCpython, Some(encoded))
                }
                "CALL_KW" => Instruction::new(Opcode::CallCpythonKwStack, Some(arg)),
                "POP_JUMP_IF_FALSE" => {
                    let target = idx + 1 + arg as usize;
                    Instruction::new(Opcode::JumpIfFalse, Some(target as u32))
                }
                "POP_JUMP_IF_TRUE" => {
                    let target = idx + 1 + arg as usize;
                    Instruction::new(Opcode::JumpIfTrue, Some(target as u32))
                }
                "JUMP_FORWARD" => {
                    let target = idx + 1 + arg as usize;
                    Instruction::new(Opcode::Jump, Some(target as u32))
                }
                "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT" | "JUMP_BACKWARD_NO_JIT" => {
                    let target = idx + 1 - arg as usize;
                    Instruction::new(Opcode::Jump, Some(target as u32))
                }
                "GET_ITER" => Instruction::new(Opcode::GetIter, None),
                "FOR_ITER" => {
                    let target = idx + 2 + arg as usize;
                    Instruction::new(Opcode::ForIter, Some(target as u32))
                }
                "END_FOR" => Instruction::new(Opcode::EndFor, None),
                "BUILD_LIST" => Instruction::new(Opcode::BuildList, Some(arg)),
                "BUILD_TUPLE" => Instruction::new(Opcode::BuildTuple, Some(arg)),
                "BUILD_MAP" | "BUILD_DICT" => Instruction::new(Opcode::BuildDict, Some(arg)),
                "BUILD_SLICE" => Instruction::new(Opcode::BuildSlice, Some(arg)),
                "UNARY_NEGATIVE" => Instruction::new(Opcode::UnaryNeg, None),
                "UNARY_POSITIVE" => Instruction::new(Opcode::UnaryPos, None),
                "UNARY_NOT" => Instruction::new(Opcode::UnaryNot, None),
                "COMPARE_OP" => match arg {
                    0 => Instruction::new(Opcode::CompareLt, None),
                    1 => Instruction::new(Opcode::CompareLe, None),
                    2 => Instruction::new(Opcode::CompareEq, None),
                    3 => Instruction::new(Opcode::CompareNe, None),
                    4 => Instruction::new(Opcode::CompareGt, None),
                    5 => Instruction::new(Opcode::CompareGe, None),
                    _ => Instruction::new(Opcode::Nop, None),
                },
                "CONTAINS_OP" => match arg {
                    0 => Instruction::new(Opcode::CompareIn, None),
                    1 => Instruction::new(Opcode::CompareNotIn, None),
                    _ => Instruction::new(Opcode::Nop, None),
                },
                "IS_OP" => match arg {
                    0 => Instruction::new(Opcode::CompareIs, None),
                    1 => Instruction::new(Opcode::CompareIsNot, None),
                    _ => Instruction::new(Opcode::Nop, None),
                },
                "BINARY_OP" => self.map_binary_op(arg),
                name if name.starts_with("BINARY_OP_INPLACE") => {
                    if name.contains("ADD") {
                        Instruction::new(Opcode::BinaryAdd, None)
                    } else if name.contains("SUBTRACT") {
                        Instruction::new(Opcode::BinarySub, None)
                    } else if name.contains("MULTIPLY") {
                        Instruction::new(Opcode::BinaryMul, None)
                    } else if name.contains("FLOOR_DIVIDE") {
                        Instruction::new(Opcode::BinaryFloorDiv, None)
                    } else if name.contains("REMAINDER") {
                        Instruction::new(Opcode::BinaryMod, None)
                    } else if name.contains("POWER") {
                        Instruction::new(Opcode::BinaryPow, None)
                    } else {
                        Instruction::new(Opcode::Nop, None)
                    }
                }
                name if name.starts_with("BINARY_OP_ADD") => {
                    Instruction::new(Opcode::BinaryAdd, None)
                }
                name if name.starts_with("BINARY_OP_SUBTRACT") => {
                    Instruction::new(Opcode::BinarySub, None)
                }
                name if name.starts_with("BINARY_OP_MULTIPLY") => {
                    Instruction::new(Opcode::BinaryMul, None)
                }
                name if name.starts_with("BINARY_OP_SUBSCR") => {
                    Instruction::new(Opcode::Subscript, None)
                }
                "STORE_SUBSCR" => Instruction::new(Opcode::StoreSubscript, None),
                "BINARY_SLICE" => Instruction::new(Opcode::Subscript, None),
                _ => Instruction::new(Opcode::Nop, None),
            };
            result.push(instruction);
        }

        Ok(result)
    }

    fn add_const(&mut self, value: Value) -> u32 {
        self.constants.push(value);
        (self.constants.len() - 1) as u32
    }

    fn map_name(&self, index: u32) -> Result<u32, CpythonError> {
        let idx = index as usize;
        self.names_map
            .get(idx)
            .cloned()
            .ok_or_else(|| CpythonError::new("name index out of range"))
    }

    fn map_local(&self, index: u32) -> Result<u32, CpythonError> {
        let idx = index as usize;
        self.locals_map
            .get(idx)
            .cloned()
            .ok_or_else(|| CpythonError::new("local index out of range"))
    }

    fn convert_constants(&mut self, consts: &[PyObject]) -> Result<Vec<Value>, CpythonError> {
        let mut values = Vec::with_capacity(consts.len());
        for obj in consts {
            values.push(self.convert_object(obj)?);
        }
        Ok(values)
    }

    fn convert_object(&mut self, obj: &PyObject) -> Result<Value, CpythonError> {
        match obj {
            PyObject::None => Ok(Value::None),
            PyObject::Bool(value) => Ok(Value::Bool(*value)),
            PyObject::Int(value) => Ok(Value::Int(*value)),
            PyObject::Str(value) => Ok(Value::Str(value.clone())),
            PyObject::Tuple(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.convert_object(item)?);
                }
                Ok(self.heap.alloc_tuple(values))
            }
            PyObject::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.convert_object(item)?);
                }
                Ok(self.heap.alloc_list(values))
            }
            PyObject::Dict(entries) => {
                let mut values = Vec::with_capacity(entries.len());
                for (key, value) in entries {
                    let key = self.convert_object(key)?;
                    let value = self.convert_object(value)?;
                    values.push((key, value));
                }
                Ok(self.heap.alloc_dict(values))
            }
            PyObject::Code(code) => {
                let code = translate_code(code, self.heap)?;
                Ok(Value::Code(Rc::new(code)))
            }
            PyObject::Slice { lower, upper, step } => {
                let lower = match lower {
                    Some(value) => match self.convert_object(value)? {
                        Value::Int(value) => Some(value),
                        Value::None => None,
                        _ => return Err(CpythonError::new("slice expects int or None")),
                    },
                    None => None,
                };
                let upper = match upper {
                    Some(value) => match self.convert_object(value)? {
                        Value::Int(value) => Some(value),
                        Value::None => None,
                        _ => return Err(CpythonError::new("slice expects int or None")),
                    },
                    None => None,
                };
                let step = match step {
                    Some(value) => match self.convert_object(value)? {
                        Value::Int(value) => Some(value),
                        Value::None => None,
                        _ => return Err(CpythonError::new("slice expects int or None")),
                    },
                    None => None,
                };
                Ok(Value::Slice { lower, upper, step })
            }
            PyObject::Bytes(_) => Err(CpythonError::new("bytes constants unsupported")),
            PyObject::Null => Err(CpythonError::new("unexpected null constant")),
        }
    }

    fn map_binary_op(&self, oparg: u32) -> Instruction {
        match oparg {
            0 => Instruction::new(Opcode::BinaryAdd, None),
            2 => Instruction::new(Opcode::BinaryFloorDiv, None),
            5 => Instruction::new(Opcode::BinaryMul, None),
            6 => Instruction::new(Opcode::BinaryMod, None),
            8 => Instruction::new(Opcode::BinaryPow, None),
            10 => Instruction::new(Opcode::BinarySub, None),
            13 => Instruction::new(Opcode::BinaryAdd, None),
            15 => Instruction::new(Opcode::BinaryFloorDiv, None),
            18 => Instruction::new(Opcode::BinaryMul, None),
            19 => Instruction::new(Opcode::BinaryMod, None),
            21 => Instruction::new(Opcode::BinaryPow, None),
            23 => Instruction::new(Opcode::BinarySub, None),
            26 => Instruction::new(Opcode::Subscript, None),
            _ => Instruction::new(Opcode::Nop, None),
        }
    }
}

struct CpInstr {
    name: String,
    arg: u32,
}

fn decode_instructions(
    bytes: &[u8],
    opmap: &HashMap<u8, String>,
) -> Result<Vec<CpInstr>, CpythonError> {
    if bytes.len() % 2 != 0 {
        return Err(CpythonError::new("bytecode length must be even"));
    }
    let mut instructions = Vec::with_capacity(bytes.len() / 2);
    let mut ext = 0u32;
    let mut i = 0;
    while i < bytes.len() {
        let opcode = bytes[i];
        let arg = bytes[i + 1] as u32;
        let name = opmap
            .get(&opcode)
            .cloned()
            .unwrap_or_else(|| format!("UNKNOWN_{opcode}"));
        if name == "EXTENDED_ARG" {
            ext = (ext << 8) | arg;
            instructions.push(CpInstr { name, arg: ext });
        } else {
            let full_arg = (ext << 8) | arg;
            ext = 0;
            instructions.push(CpInstr { name, arg: full_arg });
        }
        i += 2;
    }
    Ok(instructions)
}

struct MarshalReader<'a> {
    data: &'a [u8],
    offset: usize,
    refs: Vec<PyObject>,
}

impl<'a> MarshalReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            refs: Vec::new(),
        }
    }

    fn read_object(&mut self, allow_code: bool) -> Result<PyObject, CpythonError> {
        let code = self.read_u8()? as u8;
        let flag = (code & 0x80) != 0;
        let obj_type = (code & 0x7f) as u8;
        let ref_index = if flag {
            Some(self.reserve_ref())
        } else {
            None
        };

        let value = match obj_type as char {
            '0' => PyObject::Null,
            'N' => PyObject::None,
            'F' => PyObject::Bool(false),
            'T' => PyObject::Bool(true),
            'i' => PyObject::Int(self.read_i32()? as i64),
            'l' => PyObject::Int(self.read_long()?),
            's' => PyObject::Bytes(self.read_bytes_long()?),
            'a' | 'A' => PyObject::Str(self.read_string_long()?),
            'z' | 'Z' => PyObject::Str(self.read_string_short()?),
            'u' | 't' => PyObject::Str(self.read_string_long()?),
            '(' => {
                let size = self.read_i32()? as usize;
                let mut items = Vec::with_capacity(size);
                for _ in 0..size {
                    items.push(self.read_object(allow_code)?);
                }
                PyObject::Tuple(items)
            }
            ')' => {
                let size = self.read_u8()? as usize;
                let mut items = Vec::with_capacity(size);
                for _ in 0..size {
                    items.push(self.read_object(allow_code)?);
                }
                PyObject::Tuple(items)
            }
            '[' => {
                let size = self.read_i32()? as usize;
                let mut items = Vec::with_capacity(size);
                for _ in 0..size {
                    items.push(self.read_object(allow_code)?);
                }
                PyObject::List(items)
            }
            '{' => {
                let mut items = Vec::new();
                loop {
                    let key = self.read_object(allow_code)?;
                    if matches!(key, PyObject::Null) {
                        break;
                    }
                    let value = self.read_object(allow_code)?;
                    items.push((key, value));
                }
                PyObject::Dict(items)
            }
            'c' => {
                if !allow_code {
                    return Err(CpythonError::new("code objects not allowed"));
                }
                let argcount = self.read_i32()?;
                let posonlyargcount = self.read_i32()?;
                let kwonlyargcount = self.read_i32()?;
                let stacksize = self.read_i32()?;
                let flags = self.read_i32()?;
                let code_obj = self.read_object(allow_code)?;
                let consts = self.read_object(allow_code)?;
                let names = self.read_object(allow_code)?;
                let localsplusnames = self.read_object(allow_code)?;
                let localspluskinds = self.read_object(allow_code)?;
                let filename = self.read_object(allow_code)?;
                let name = self.read_object(allow_code)?;
                let qualname = self.read_object(allow_code)?;
                let firstlineno = self.read_i32()?;
                let linetable = self.read_object(allow_code)?;
                let exceptiontable = self.read_object(allow_code)?;

                let code = match code_obj {
                    PyObject::Bytes(bytes) => bytes,
                    _ => return Err(CpythonError::new("code object missing bytes")),
                };
                let consts = match consts {
                    PyObject::Tuple(items) | PyObject::List(items) => items,
                    _ => return Err(CpythonError::new("code consts must be tuple/list")),
                };
                let names = match names {
                    PyObject::Tuple(items) | PyObject::List(items) => parse_str_list(items)?,
                    _ => return Err(CpythonError::new("code names must be tuple/list")),
                };
                let localsplusnames = match localsplusnames {
                    PyObject::Tuple(items) | PyObject::List(items) => parse_str_list(items)?,
                    _ => return Err(CpythonError::new("localsplusnames must be tuple/list")),
                };
                let localspluskinds = match localspluskinds {
                    PyObject::Bytes(bytes) => bytes,
                    _ => return Err(CpythonError::new("localspluskinds must be bytes")),
                };
                let filename = match filename {
                    PyObject::Str(value) => value,
                    _ => return Err(CpythonError::new("filename must be string")),
                };
                let name = match name {
                    PyObject::Str(value) => value,
                    _ => return Err(CpythonError::new("name must be string")),
                };
                let qualname = match qualname {
                    PyObject::Str(value) => value,
                    _ => return Err(CpythonError::new("qualname must be string")),
                };
                let linetable = match linetable {
                    PyObject::Bytes(bytes) => bytes,
                    _ => return Err(CpythonError::new("linetable must be bytes")),
                };
                let exceptiontable = match exceptiontable {
                    PyObject::Bytes(bytes) => bytes,
                    _ => return Err(CpythonError::new("exceptiontable must be bytes")),
                };

                PyObject::Code(Rc::new(CpythonCode {
                    argcount,
                    posonlyargcount,
                    kwonlyargcount,
                    stacksize,
                    flags,
                    code,
                    consts,
                    names,
                    localsplusnames,
                    localspluskinds,
                    filename,
                    name,
                    qualname,
                    firstlineno,
                    linetable,
                    exceptiontable,
                }))
            }
            'r' => {
                let index = self.read_i32()? as usize;
                let value = self
                    .refs
                    .get(index)
                    .cloned()
                    .ok_or_else(|| CpythonError::new("invalid marshal reference"))?;
                return Ok(value);
            }
            ':' => {
                let lower = self.read_object(allow_code)?;
                let upper = self.read_object(allow_code)?;
                let step = self.read_object(allow_code)?;
                let lower = if matches!(lower, PyObject::Null) {
                    None
                } else {
                    Some(Box::new(lower))
                };
                let upper = if matches!(upper, PyObject::Null) {
                    None
                } else {
                    Some(Box::new(upper))
                };
                let step = if matches!(step, PyObject::Null) {
                    None
                } else {
                    Some(Box::new(step))
                };
                PyObject::Slice { lower, upper, step }
            }
            other => {
                return Err(CpythonError::new(format!(
                    "unsupported marshal type {other:?}"
                )))
            }
        };

        if let Some(index) = ref_index {
            if index >= self.refs.len() {
                return Err(CpythonError::new("invalid marshal reference"));
            }
            self.refs[index] = value.clone();
        }
        if matches!(value, PyObject::Null) {
            Ok(PyObject::Null)
        } else {
            Ok(value)
        }
    }

    fn read_u8(&mut self) -> Result<u8, CpythonError> {
        let byte = self
            .data
            .get(self.offset)
            .ok_or_else(|| CpythonError::new("unexpected end of data"))?;
        self.offset += 1;
        Ok(*byte)
    }

    fn read_i32(&mut self) -> Result<i32, CpythonError> {
        let bytes = self.read_exact(4)?;
        Ok(i32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_long(&mut self) -> Result<i64, CpythonError> {
        let n = self.read_i32()? as i64;
        if n == 0 {
            return Ok(0);
        }
        let sign = if n < 0 { -1 } else { 1 };
        let count = n.abs() as usize;
        let mut value: i128 = 0;
        let mut factor: i128 = 1;
        for _ in 0..count {
            let digit = self.read_u16()? as i128;
            value += digit * factor;
            factor <<= 15;
        }
        value *= sign as i128;
        if value > i64::MAX as i128 || value < i64::MIN as i128 {
            return Err(CpythonError::new("long constant out of range"));
        }
        Ok(value as i64)
    }

    fn read_u16(&mut self) -> Result<u16, CpythonError> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_bytes_long(&mut self) -> Result<Vec<u8>, CpythonError> {
        let size = self.read_i32()? as usize;
        let bytes = self.read_exact(size)?;
        Ok(bytes.to_vec())
    }

    fn read_string_long(&mut self) -> Result<String, CpythonError> {
        let bytes = self.read_bytes_long()?;
        String::from_utf8(bytes).map_err(|_| CpythonError::new("invalid utf8"))
    }

    fn read_string_short(&mut self) -> Result<String, CpythonError> {
        let size = self.read_u8()? as usize;
        let bytes = self.read_exact(size)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| CpythonError::new("invalid utf8"))
    }

    fn read_exact(&mut self, size: usize) -> Result<&[u8], CpythonError> {
        let end = self.offset + size;
        if end > self.data.len() {
            return Err(CpythonError::new("unexpected end of data"));
        }
        let slice = &self.data[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    fn reserve_ref(&mut self) -> usize {
        let index = self.refs.len();
        self.refs.push(PyObject::Null);
        index
    }
}

fn parse_str_list(items: Vec<PyObject>) -> Result<Vec<String>, CpythonError> {
    let mut result = Vec::with_capacity(items.len());
    for item in items {
        match item {
            PyObject::Str(value) => result.push(value),
            _ => return Err(CpythonError::new("expected string in list")),
        }
    }
    Ok(result)
}
