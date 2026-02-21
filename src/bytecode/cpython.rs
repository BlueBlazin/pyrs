use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use crate::bytecode::metadata::OpcodeMetadata;
use crate::bytecode::pyc::{PycHeader, parse_pyc_header, write_pyc_header};
use crate::bytecode::{CodeObject, ExceptionHandler, Instruction, Opcode};
use crate::runtime::{BigInt, Heap, SliceValue, Value};

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
    StopIteration,
    Ellipsis,
    Bool(bool),
    Int(i64),
    BigInt(BigInt),
    Float(f64),
    Complex {
        real: f64,
        imag: f64,
    },
    Str(String),
    Bytes(Vec<u8>),
    Tuple(Vec<PyObject>),
    List(Vec<PyObject>),
    Dict(Vec<(PyObject, PyObject)>),
    Set(Vec<PyObject>),
    FrozenSet(Vec<PyObject>),
    Code(Rc<CpythonCode>),
    Slice {
        lower: Option<Box<PyObject>>,
        upper: Option<Box<PyObject>>,
        step: Option<Box<PyObject>>,
    },
}

pub fn load_pyc(bytes: &[u8]) -> Result<CpythonCode, CpythonError> {
    let (_header, offset) =
        parse_pyc_header(bytes).map_err(|err| CpythonError::new(err.message))?;
    let mut reader = MarshalReader::new(&bytes[offset..]);
    let obj = reader.read_object(true)?;
    match obj {
        PyObject::Code(code) => Ok((*code).clone()),
        _ => Err(CpythonError::new("pyc did not contain a code object")),
    }
}

pub fn dump_pyc(code: &CpythonCode, header: &PycHeader) -> Result<Vec<u8>, CpythonError> {
    let mut bytes = Vec::new();
    write_pyc_header(header, &mut bytes).map_err(|err| CpythonError::new(err.message))?;
    let mut writer = MarshalWriter::new();
    writer.write_code_object(code)?;
    bytes.extend_from_slice(&writer.into_bytes());
    Ok(bytes)
}

pub fn marshal_dump_object(obj: &PyObject) -> Result<Vec<u8>, CpythonError> {
    let mut writer = MarshalWriter::new();
    writer.write_object(obj)?;
    Ok(writer.into_bytes())
}

pub fn marshal_load_object(bytes: &[u8], allow_code: bool) -> Result<PyObject, CpythonError> {
    let mut reader = MarshalReader::new(bytes);
    reader.read_object(allow_code)
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
    deref_map: Vec<Option<u32>>,
    cellvars: Vec<String>,
    freevars: Vec<String>,
    constants: Vec<Value>,
}

impl<'a> Translator<'a> {
    fn new(code: &'a CpythonCode, heap: &'a mut Heap) -> Result<Self, CpythonError> {
        let metadata =
            OpcodeMetadata::load_default().map_err(|err| CpythonError::new(err.message))?;
        let mut opmap = HashMap::new();
        for info in metadata.opcodes {
            if info.code <= u8::MAX as u16 {
                opmap.insert(info.code as u8, info.name);
            }
        }

        Ok(Self {
            code,
            heap,
            opmap,
            names: Vec::new(),
            name_index: HashMap::new(),
            locals_map: Vec::new(),
            names_map: Vec::new(),
            deref_map: Vec::new(),
            cellvars: Vec::new(),
            freevars: Vec::new(),
            constants: Vec::new(),
        })
    }

    fn translate(&mut self) -> Result<CodeObject, CpythonError> {
        self.build_name_maps()?;
        self.constants = self.convert_constants(&self.code.consts)?;

        let mut result = CodeObject::new(self.code.name.clone(), self.code.filename.clone());
        result.constants = self.constants.clone();
        result.names = self.names.clone();
        result.cellvars = self.cellvars.clone();
        result.freevars = self.freevars.clone();
        result.is_generator = (self.code.flags & 0x20) != 0;
        result.is_coroutine = (self.code.flags & 0x80) != 0 || (self.code.flags & 0x100) != 0;
        result.is_async_generator = (self.code.flags & 0x200) != 0;
        self.populate_params(&mut result)?;

        let instructions = self.translate_instructions()?;
        result.instructions = instructions;
        result.locations = vec![crate::bytecode::Location::unknown(); result.instructions.len()];
        result.exception_handlers =
            parse_exception_table(&self.code.exceptiontable, result.instructions.len())?;
        if std::env::var_os("PYRS_TRACE_EXCEPTION_TABLE").is_some() {
            for entry in &result.exception_handlers {
                eprintln!(
                    "[pyc-exc-entry] start={} end={} target={} depth={} push_lasti={}",
                    entry.start, entry.end, entry.target, entry.depth, entry.push_lasti
                );
            }
        }
        result.constants = self.constants.clone();
        result.rebuild_layout_indexes();
        Ok(result)
    }

    fn build_name_maps(&mut self) -> Result<(), CpythonError> {
        for (idx, name) in self.code.localsplusnames.iter().enumerate() {
            let mapped = self.intern_name(name);
            self.locals_map.push(mapped);
            let kind = self.code.localspluskinds.get(idx).copied().unwrap_or(0);
            let mut deref_index = None;
            if kind & 0x40 != 0 {
                self.cellvars.push(name.clone());
                deref_index = Some((self.cellvars.len() - 1) as u32);
            }
            if kind & 0x80 != 0 {
                self.freevars.push(name.clone());
                let idx = (self.cellvars.len() + self.freevars.len() - 1) as u32;
                deref_index = Some(idx);
            }
            self.deref_map.push(deref_index);
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
        if total_pos < total_posonly {
            return Err(CpythonError::new(
                "argcount is smaller than posonlyargcount",
            ));
        }
        let total_positional = total_pos - total_posonly;
        let total_kwonly = self.code.kwonlyargcount as usize;
        let mut idx = 0;
        if self.code.localsplusnames.len() < total_pos + total_kwonly {
            return Err(CpythonError::new("localsplusnames too short for args"));
        }
        result.posonly_params = self.code.localsplusnames[idx..idx + total_posonly].to_vec();
        idx += total_posonly;
        result.params = self.code.localsplusnames[idx..idx + total_positional].to_vec();
        idx += total_positional;
        result.kwonly_params = self.code.localsplusnames[idx..idx + total_kwonly].to_vec();
        idx += total_kwonly;

        let flags = self.code.flags as u32;
        if flags & 0x0004 != 0
            && let Some(name) = self.code.localsplusnames.get(idx)
        {
            result.vararg = Some(name.clone());
            idx += 1;
        }
        if flags & 0x0008 != 0
            && let Some(name) = self.code.localsplusnames.get(idx)
        {
            result.kwarg = Some(name.clone());
        }
        Ok(())
    }

    fn translate_instructions(&mut self) -> Result<Vec<Instruction>, CpythonError> {
        let cp_instructions = decode_instructions(&self.code.code, &self.opmap)?;
        validate_cpython_control_flow(&cp_instructions)?;
        let trace_this_code = if let Some(filter) =
            std::env::var_os("PYRS_TRACE_PYC_TRANSLATE_FILTER")
            && let Some(filter) = filter.to_str()
        {
            self.code.filename.contains(filter)
        } else {
            std::env::var_os("PYRS_TRACE_PYC_TRANSLATE").is_some_and(|_| {
                self.code.filename.ends_with("/abc.py") && self.code.name == "__new__"
            })
        };
        let mut result = Vec::with_capacity(cp_instructions.len());
        let mut pending_kw_names: Option<u16> = None;
        let mut prev_was_return_generator = false;

        for (idx, instr) in cp_instructions.iter().enumerate() {
            let name = instr.name.as_str();
            let arg = instr.arg;
            if pending_kw_names.is_some() && !kw_names_follower(name) {
                return Err(CpythonError::new(format!(
                    "KW_NAMES at instruction {} is not followed by CALL",
                    idx.saturating_sub(1)
                )));
            }
            let instruction = match name {
                "CACHE"
                | "RESUME"
                | "RESUME_CHECK"
                | "NOP"
                | "NOT_TAKEN"
                | "EXTENDED_ARG"
                | "RETURN_GENERATOR"
                | "COPY_FREE_VARS"
                | "END_SEND"
                | "CLEANUP_THROW"
                | "SETUP_WITH"
                | "SETUP_FINALLY"
                | "SETUP_CLEANUP"
                | "INSTRUMENTED_INSTRUCTION"
                | "INSTRUMENTED_NOT_TAKEN"
                | "INSTRUMENTED_LINE"
                | "ANNOTATIONS_PLACEHOLDER"
                | "INTERPRETER_EXIT"
                | "ENTER_EXECUTOR" => Instruction::new(Opcode::Nop, None),
                "MAKE_CELL" => Instruction::new(Opcode::MakeCell, Some(self.map_local(arg)?)),
                "POP_TOP" if prev_was_return_generator => Instruction::new(Opcode::Nop, None),
                "POP_TOP" => Instruction::new(Opcode::PopTop, None),
                "POP_ITER" => Instruction::new(Opcode::Nop, None),
                "INSTRUMENTED_POP_ITER" => Instruction::new(Opcode::Nop, None),
                "RETURN_VALUE" => Instruction::new(Opcode::ReturnValue, None),
                "INSTRUMENTED_RETURN_VALUE" => Instruction::new(Opcode::ReturnValue, None),
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
                "SETUP_ANNOTATIONS" => Instruction::new(Opcode::SetupAnnotations, None),
                "LOAD_NAME" => Instruction::new(Opcode::LoadName, Some(self.map_name(arg)?)),
                "LOAD_LOCALS" => Instruction::new(Opcode::LoadLocals, None),
                "LOAD_FROM_DICT_OR_GLOBALS" => {
                    Instruction::new(Opcode::LoadFromDictOrGlobals, Some(self.map_name(arg)?))
                }
                "LOAD_FROM_DICT_OR_DEREF" => {
                    Instruction::new(Opcode::LoadFromDictOrDeref, Some(self.map_deref(arg)?))
                }
                "STORE_NAME" => Instruction::new(Opcode::StoreName, Some(self.map_name(arg)?)),
                "DELETE_NAME" => Instruction::new(Opcode::DeleteName, Some(self.map_name(arg)?)),
                "DELETE_FAST" => Instruction::new(Opcode::DeleteFast, Some(self.map_local(arg)?)),
                "LOAD_DEREF" => Instruction::new(Opcode::LoadDeref, Some(self.map_deref(arg)?)),
                "STORE_DEREF" => Instruction::new(Opcode::StoreDeref, Some(self.map_deref(arg)?)),
                "LOAD_CLOSURE" => Instruction::new(Opcode::LoadClosure, Some(self.map_deref(arg)?)),
                "LOAD_GLOBAL"
                | "LOAD_GLOBAL_ADAPTIVE"
                | "LOAD_GLOBAL_BUILTIN"
                | "LOAD_GLOBAL_MODULE" => {
                    let name_idx = arg >> 1;
                    let push_null = arg & 1;
                    let mapped = self.map_name(name_idx)?;
                    let encoded = (mapped << 1) | push_null;
                    Instruction::new(Opcode::LoadGlobal, Some(encoded))
                }
                name if name.starts_with("LOAD_FAST") => match name {
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
                },
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
                    let name_idx = arg >> 1;
                    let push_null = arg & 1;
                    let mapped = self.map_name(name_idx)?;
                    let encoded = (mapped << 1) | push_null;
                    Instruction::new(Opcode::LoadAttr, Some(encoded))
                }
                "LOAD_SUPER_ATTR" => {
                    let name_idx = arg >> 2;
                    let flags = arg & 0b11;
                    let mapped = self.map_name(name_idx)?;
                    let encoded = (mapped << 2) | flags;
                    Instruction::new(Opcode::LoadSuperAttr, Some(encoded))
                }
                "LOAD_SPECIAL" => Instruction::new(Opcode::LoadSpecial, Some(arg)),
                name if name.starts_with("STORE_ATTR") => {
                    Instruction::new(Opcode::StoreAttrCpython, Some(self.map_name(arg)?))
                }
                "DELETE_ATTR" => Instruction::new(Opcode::DeleteAttr, Some(self.map_name(arg)?)),
                "LOAD_BUILD_CLASS" => Instruction::new(Opcode::LoadBuildClass, None),
                "PUSH_NULL" => Instruction::new(Opcode::PushNull, None),
                "GET_AWAITABLE" => Instruction::new(Opcode::GetAwaitable, None),
                "MAKE_FUNCTION" => Instruction::new(Opcode::MakeFunctionStack, None),
                "SET_FUNCTION_ATTRIBUTE" => {
                    Instruction::new(Opcode::SetFunctionAttribute, Some(arg))
                }
                "KW_NAMES" => {
                    pending_kw_names = Some(arg as u16);
                    Instruction::new(Opcode::Nop, None)
                }
                "CALL"
                | "INSTRUMENTED_CALL"
                | "CALL_ALLOC_AND_ENTER_INIT"
                | "CALL_BOUND_METHOD_EXACT_ARGS"
                | "CALL_BOUND_METHOD_GENERAL"
                | "CALL_BUILTIN_CLASS"
                | "CALL_BUILTIN_FAST"
                | "CALL_BUILTIN_FAST_WITH_KEYWORDS"
                | "CALL_BUILTIN_O"
                | "CALL_ISINSTANCE"
                | "CALL_LEN"
                | "CALL_LIST_APPEND"
                | "CALL_METHOD_DESCRIPTOR_FAST"
                | "CALL_METHOD_DESCRIPTOR_FAST_WITH_KEYWORDS"
                | "CALL_METHOD_DESCRIPTOR_NOARGS"
                | "CALL_METHOD_DESCRIPTOR_O"
                | "CALL_NON_PY_GENERAL"
                | "CALL_PY_EXACT_ARGS"
                | "CALL_PY_GENERAL"
                | "CALL_STR_1"
                | "CALL_TUPLE_1"
                | "CALL_TYPE_1" => {
                    let kw_idx = pending_kw_names.take().unwrap_or(u16::MAX);
                    let encoded = ((kw_idx as u32) << 16) | (arg & 0xFFFF);
                    Instruction::new(Opcode::CallCpython, Some(encoded))
                }
                "CALL_KW"
                | "INSTRUMENTED_CALL_KW"
                | "CALL_KW_BOUND_METHOD"
                | "CALL_KW_NON_PY"
                | "CALL_KW_PY" => Instruction::new(Opcode::CallCpythonKwStack, Some(arg)),
                "CALL_FUNCTION_EX" => Instruction::new(Opcode::CallFunctionEx, Some(arg)),
                "CALL_INTRINSIC_1" => Instruction::new(Opcode::CallIntrinsic1, Some(arg)),
                "CALL_INTRINSIC_2" => Instruction::new(Opcode::CallIntrinsic2, Some(arg)),
                "POP_JUMP_IF_FALSE" => Instruction::new(
                    Opcode::JumpIfFalse,
                    Some(relative_forward_target_plus_two(idx, arg)?),
                ),
                "INSTRUMENTED_POP_JUMP_IF_FALSE" => Instruction::new(
                    Opcode::JumpIfFalse,
                    Some(relative_forward_target_plus_two(idx, arg)?),
                ),
                "POP_JUMP_IF_TRUE" => Instruction::new(
                    Opcode::JumpIfTrue,
                    Some(relative_forward_target_plus_two(idx, arg)?),
                ),
                "INSTRUMENTED_POP_JUMP_IF_TRUE" => Instruction::new(
                    Opcode::JumpIfTrue,
                    Some(relative_forward_target_plus_two(idx, arg)?),
                ),
                "POP_JUMP_IF_NONE" | "INSTRUMENTED_POP_JUMP_IF_NONE" => Instruction::new(
                    Opcode::JumpIfNone,
                    Some(relative_forward_target_plus_two(idx, arg)?),
                ),
                "POP_JUMP_IF_NOT_NONE" | "INSTRUMENTED_POP_JUMP_IF_NOT_NONE" => Instruction::new(
                    Opcode::JumpIfNotNone,
                    Some(relative_forward_target_plus_two(idx, arg)?),
                ),
                "JUMP_FORWARD" | "INSTRUMENTED_JUMP_FORWARD" => {
                    Instruction::new(Opcode::Jump, Some(relative_forward_target(idx, arg)?))
                }
                "JUMP_BACKWARD"
                | "JUMP_BACKWARD_NO_JIT"
                | "JUMP_BACKWARD_JIT"
                | "INSTRUMENTED_JUMP_BACKWARD" => {
                    Instruction::new(Opcode::Jump, Some(relative_backward_target(idx, arg)?))
                }
                "JUMP_BACKWARD_NO_INTERRUPT" => Instruction::new(
                    Opcode::Jump,
                    Some(relative_backward_no_interrupt_target(idx, arg)?),
                ),
                "JUMP" | "JUMP_NO_INTERRUPT" => {
                    Instruction::new(Opcode::Jump, Some(relative_forward_target(idx, arg)?))
                }
                "GET_ITER" => Instruction::new(Opcode::GetIter, None),
                "GET_YIELD_FROM_ITER" => Instruction::new(Opcode::GetIter, None),
                "FOR_ITER" => Instruction::new(Opcode::ForIter, Some(for_iter_target(idx, arg)?)),
                "INSTRUMENTED_FOR_ITER"
                | "FOR_ITER_GEN"
                | "FOR_ITER_LIST"
                | "FOR_ITER_RANGE"
                | "FOR_ITER_TUPLE" => {
                    Instruction::new(Opcode::ForIter, Some(for_iter_target(idx, arg)?))
                }
                "SEND" | "SEND_GEN" => Instruction::new(
                    Opcode::Send,
                    Some(relative_forward_target_plus_two(idx, arg)?),
                ),
                "YIELD_VALUE" => Instruction::new(Opcode::YieldValue, None),
                "INSTRUMENTED_YIELD_VALUE" => Instruction::new(Opcode::YieldValue, None),
                "YIELD_FROM" => Instruction::new(Opcode::YieldFrom, None),
                "END_FOR" | "INSTRUMENTED_END_FOR" => Instruction::new(Opcode::EndFor, None),
                "BUILD_LIST" => Instruction::new(Opcode::BuildList, Some(arg)),
                "BUILD_SET" => Instruction::new(Opcode::BuildSet, Some(arg)),
                "BUILD_TUPLE" => Instruction::new(Opcode::BuildTuple, Some(arg)),
                "BUILD_STRING" => Instruction::new(Opcode::BuildString, Some(arg)),
                "LIST_APPEND" => Instruction::new(Opcode::ListAppend, Some(arg)),
                "SET_ADD" => Instruction::new(Opcode::SetAdd, Some(arg)),
                "LIST_EXTEND" => Instruction::new(Opcode::ListExtend, Some(arg)),
                "SET_UPDATE" => Instruction::new(Opcode::SetUpdate, Some(arg)),
                "BUILD_MAP" | "BUILD_DICT" => Instruction::new(Opcode::BuildDict, Some(arg)),
                "MAP_ADD" => Instruction::new(Opcode::MapAdd, Some(arg)),
                "DICT_UPDATE" => Instruction::new(Opcode::DictUpdate, Some(arg)),
                "DICT_MERGE" => Instruction::new(Opcode::DictMerge, Some(arg)),
                "BUILD_SLICE" => Instruction::new(Opcode::BuildSlice, Some(arg)),
                "COPY" => Instruction::new(Opcode::Copy, Some(arg)),
                "SWAP" => Instruction::new(Opcode::Swap, Some(arg)),
                "UNPACK_SEQUENCE"
                | "UNPACK_SEQUENCE_LIST"
                | "UNPACK_SEQUENCE_TUPLE"
                | "UNPACK_SEQUENCE_TWO_TUPLE" => {
                    Instruction::new(Opcode::UnpackSequenceCpython, Some(arg))
                }
                "UNPACK_EX" => Instruction::new(Opcode::UnpackExCpython, Some(arg)),
                "IMPORT_NAME" => {
                    Instruction::new(Opcode::ImportNameCpython, Some(self.map_name(arg)?))
                }
                "IMPORT_FROM" => {
                    Instruction::new(Opcode::ImportFromCpython, Some(self.map_name(arg)?))
                }
                "UNARY_NEGATIVE" => Instruction::new(Opcode::UnaryNeg, None),
                "UNARY_POSITIVE" => Instruction::new(Opcode::UnaryPos, None),
                "UNARY_NOT" => Instruction::new(Opcode::UnaryNot, None),
                "UNARY_INVERT" => Instruction::new(Opcode::UnaryInvert, None),
                "CONVERT_VALUE" => Instruction::new(Opcode::ConvertValue, Some(arg)),
                "FORMAT_SIMPLE" => Instruction::new(Opcode::FormatSimple, None),
                "FORMAT_WITH_SPEC" => Instruction::new(Opcode::FormatWithSpec, None),
                "TO_BOOL"
                | "TO_BOOL_ALWAYS_TRUE"
                | "TO_BOOL_BOOL"
                | "TO_BOOL_INT"
                | "TO_BOOL_LIST"
                | "TO_BOOL_NONE"
                | "TO_BOOL_STR" => Instruction::new(Opcode::ToBool, None),
                "COMPARE_OP" | "COMPARE_OP_FLOAT" | "COMPARE_OP_INT" | "COMPARE_OP_STR" => {
                    self.map_compare_op(idx, arg)?
                }
                "CONTAINS_OP" | "CONTAINS_OP_DICT" | "CONTAINS_OP_SET" => {
                    self.map_contains_op(idx, arg)?
                }
                "IS_OP" => self.map_is_op(idx, arg)?,
                "BINARY_OP" => self.map_binary_op(idx, arg)?,
                name if name.starts_with("BINARY_OP_INPLACE") => {
                    self.map_inplace_binary_op(idx, name)?
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
                name if name.starts_with("BINARY_OP_MATRIX_MULTIPLY")
                    || name.starts_with("BINARY_OP_MATMULT") =>
                {
                    Instruction::new(Opcode::BinaryMatMul, None)
                }
                name if name.starts_with("BINARY_OP_TRUE_DIVIDE") => {
                    Instruction::new(Opcode::BinaryDiv, None)
                }
                name if name.starts_with("BINARY_OP_FLOOR_DIVIDE") => {
                    Instruction::new(Opcode::BinaryFloorDiv, None)
                }
                name if name.starts_with("BINARY_OP_REMAINDER") => {
                    Instruction::new(Opcode::BinaryMod, None)
                }
                name if name.starts_with("BINARY_OP_POWER") => {
                    Instruction::new(Opcode::BinaryPow, None)
                }
                name if name.starts_with("BINARY_OP_LSHIFT") => {
                    Instruction::new(Opcode::BinaryLShift, None)
                }
                name if name.starts_with("BINARY_OP_RSHIFT") => {
                    Instruction::new(Opcode::BinaryRShift, None)
                }
                name if name.starts_with("BINARY_OP_AND") => {
                    Instruction::new(Opcode::BinaryAnd, None)
                }
                name if name.starts_with("BINARY_OP_XOR") => {
                    Instruction::new(Opcode::BinaryXor, None)
                }
                name if name.starts_with("BINARY_OP_OR") => {
                    Instruction::new(Opcode::BinaryOr, None)
                }
                name if name.starts_with("BINARY_OP_SUBSCR") => {
                    Instruction::new(Opcode::Subscript, None)
                }
                "STORE_SUBSCR" | "STORE_SUBSCR_DICT" | "STORE_SUBSCR_LIST_INT" => {
                    Instruction::new(Opcode::StoreSubscript, Some(1))
                }
                "STORE_SLICE" => Instruction::new(Opcode::StoreSlice, None),
                "DELETE_SUBSCR" => Instruction::new(Opcode::DeleteSubscript, None),
                "BINARY_SLICE" => Instruction::new(Opcode::BinarySlice, None),
                "PUSH_EXC_INFO" => Instruction::new(Opcode::PushExcInfo, None),
                "POP_EXCEPT" => Instruction::new(Opcode::PopExcept, None),
                "WITH_EXCEPT_START" => Instruction::new(Opcode::WithExceptStart, None),
                "RERAISE" => Instruction::new(Opcode::Reraise, Some(arg)),
                "RAISE_VARARGS" => Instruction::new(Opcode::Raise, Some(arg)),
                "CHECK_EXC_MATCH" => Instruction::new(Opcode::CheckExcMatch, None),
                // In CPython 3.14 this is a pseudo-op lowered to NOP.
                "POP_BLOCK" => Instruction::new(Opcode::Nop, None),
                _ => {
                    return Err(CpythonError::new(format!(
                        "unsupported CPython opcode '{}' (arg={}) at instruction {}",
                        name, arg, idx
                    )));
                }
            };
            if trace_this_code {
                eprintln!(
                    "[pyc-translate] {}:{} cp={} arg={} -> {:?} {:?}",
                    self.code.filename,
                    self.code.name,
                    name,
                    arg,
                    instruction.opcode,
                    instruction.arg
                );
            }
            prev_was_return_generator = name == "RETURN_GENERATOR";
            result.push(instruction);
        }

        if pending_kw_names.is_some() {
            return Err(CpythonError::new("dangling KW_NAMES without CALL"));
        }

        validate_translated_code(&result)?;
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

    fn map_deref(&self, index: u32) -> Result<u32, CpythonError> {
        let idx = index as usize;
        match self.deref_map.get(idx).copied().flatten() {
            Some(mapped) => Ok(mapped),
            None => Err(CpythonError::new("deref index out of range")),
        }
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
            PyObject::StopIteration => Ok(Value::ExceptionType("StopIteration".to_string())),
            PyObject::Ellipsis => Ok(self.heap.ellipsis_singleton()),
            PyObject::Bool(value) => Ok(Value::Bool(*value)),
            PyObject::Int(value) => Ok(Value::Int(*value)),
            PyObject::BigInt(value) => match value.to_i64() {
                Some(integer) => Ok(Value::Int(integer)),
                None => Ok(Value::BigInt(Box::new(value.clone()))),
            },
            PyObject::Float(value) => Ok(Value::Float(*value)),
            PyObject::Complex { real, imag } => Ok(Value::Complex {
                real: *real,
                imag: *imag,
            }),
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
            PyObject::Set(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.convert_object(item)?);
                }
                Ok(self.heap.alloc_set(values))
            }
            PyObject::FrozenSet(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.convert_object(item)?);
                }
                Ok(self.heap.alloc_frozenset(values))
            }
            PyObject::Code(code) => {
                let code = translate_code(code, self.heap)?;
                Ok(Value::Code(Rc::new(code)))
            }
            PyObject::Slice { lower, upper, step } => {
                let lower = match lower {
                    Some(value) => match self.convert_object(value)? {
                        Value::Int(value) => Some(value),
                        Value::BigInt(value) => value.to_i64(),
                        Value::None => None,
                        _ => return Err(CpythonError::new("slice expects int or None")),
                    },
                    None => None,
                };
                let upper = match upper {
                    Some(value) => match self.convert_object(value)? {
                        Value::Int(value) => Some(value),
                        Value::BigInt(value) => value.to_i64(),
                        Value::None => None,
                        _ => return Err(CpythonError::new("slice expects int or None")),
                    },
                    None => None,
                };
                let step = match step {
                    Some(value) => match self.convert_object(value)? {
                        Value::Int(value) => Some(value),
                        Value::BigInt(value) => value.to_i64(),
                        Value::None => None,
                        _ => return Err(CpythonError::new("slice expects int or None")),
                    },
                    None => None,
                };
                Ok(Value::Slice(Box::new(SliceValue::new(lower, upper, step))))
            }
            PyObject::Bytes(value) => Ok(self.heap.alloc_bytes(value.clone())),
            PyObject::Null => Err(CpythonError::new("unexpected null constant")),
        }
    }

    fn map_compare_op(&self, idx: usize, oparg: u32) -> Result<Instruction, CpythonError> {
        let compare_kind = oparg >> 5;
        let instr = match compare_kind {
            0 => Instruction::new(Opcode::CompareLt, None),
            1 => Instruction::new(Opcode::CompareLe, None),
            2 => Instruction::new(Opcode::CompareEq, None),
            3 => Instruction::new(Opcode::CompareNe, None),
            4 => Instruction::new(Opcode::CompareGt, None),
            5 => Instruction::new(Opcode::CompareGe, None),
            _ => {
                return Err(CpythonError::new(format!(
                    "unsupported COMPARE_OP arg {} at instruction {}",
                    oparg, idx
                )));
            }
        };
        Ok(instr)
    }

    fn map_contains_op(&self, idx: usize, oparg: u32) -> Result<Instruction, CpythonError> {
        let instr = match oparg {
            0 => Instruction::new(Opcode::CompareIn, None),
            1 => Instruction::new(Opcode::CompareNotIn, None),
            _ => {
                return Err(CpythonError::new(format!(
                    "unsupported CONTAINS_OP arg {} at instruction {}",
                    oparg, idx
                )));
            }
        };
        Ok(instr)
    }

    fn map_is_op(&self, idx: usize, oparg: u32) -> Result<Instruction, CpythonError> {
        let instr = match oparg {
            0 => Instruction::new(Opcode::CompareIs, None),
            1 => Instruction::new(Opcode::CompareIsNot, None),
            _ => {
                return Err(CpythonError::new(format!(
                    "unsupported IS_OP arg {} at instruction {}",
                    oparg, idx
                )));
            }
        };
        Ok(instr)
    }

    fn map_inplace_binary_op(&self, idx: usize, name: &str) -> Result<Instruction, CpythonError> {
        let instr = if name.contains("ADD") {
            Instruction::new(Opcode::BinaryAdd, None)
        } else if name.contains("AND") {
            Instruction::new(Opcode::BinaryAnd, None)
        } else if name.contains("SUBTRACT") {
            Instruction::new(Opcode::BinarySub, None)
        } else if name.contains("LSHIFT") {
            Instruction::new(Opcode::BinaryLShift, None)
        } else if name.contains("RSHIFT") {
            Instruction::new(Opcode::BinaryRShift, None)
        } else if name.contains("MATRIX_MULTIPLY") || name.contains("MATMULT") {
            Instruction::new(Opcode::BinaryMatMul, None)
        } else if name.contains("MULTIPLY") {
            Instruction::new(Opcode::BinaryMul, None)
        } else if name.contains("OR") {
            Instruction::new(Opcode::BinaryOr, None)
        } else if name.contains("TRUE_DIVIDE") {
            Instruction::new(Opcode::BinaryDiv, None)
        } else if name.contains("FLOOR_DIVIDE") {
            Instruction::new(Opcode::BinaryFloorDiv, None)
        } else if name.contains("REMAINDER") {
            Instruction::new(Opcode::BinaryMod, None)
        } else if name.contains("POWER") {
            Instruction::new(Opcode::BinaryPow, None)
        } else if name.contains("XOR") {
            Instruction::new(Opcode::BinaryXor, None)
        } else {
            return Err(CpythonError::new(format!(
                "unsupported {} at instruction {}",
                name, idx
            )));
        };
        Ok(instr)
    }

    fn map_binary_op(&self, idx: usize, oparg: u32) -> Result<Instruction, CpythonError> {
        match oparg {
            0 => Ok(Instruction::new(Opcode::BinaryAdd, None)),
            1 => Ok(Instruction::new(Opcode::BinaryAnd, None)),
            2 => Ok(Instruction::new(Opcode::BinaryFloorDiv, None)),
            3 => Ok(Instruction::new(Opcode::BinaryLShift, None)),
            4 => Ok(Instruction::new(Opcode::BinaryMatMul, None)),
            5 => Ok(Instruction::new(Opcode::BinaryMul, None)),
            6 => Ok(Instruction::new(Opcode::BinaryMod, None)),
            7 => Ok(Instruction::new(Opcode::BinaryOr, None)),
            8 => Ok(Instruction::new(Opcode::BinaryPow, None)),
            9 => Ok(Instruction::new(Opcode::BinaryRShift, None)),
            10 => Ok(Instruction::new(Opcode::BinarySub, None)),
            11 => Ok(Instruction::new(Opcode::BinaryDiv, None)),
            12 => Ok(Instruction::new(Opcode::BinaryXor, None)),
            13 => Ok(Instruction::new(Opcode::BinaryAdd, None)),
            14 => Ok(Instruction::new(Opcode::BinaryAnd, None)),
            15 => Ok(Instruction::new(Opcode::BinaryFloorDiv, None)),
            16 => Ok(Instruction::new(Opcode::BinaryLShift, None)),
            17 => Ok(Instruction::new(Opcode::BinaryMatMul, None)),
            18 => Ok(Instruction::new(Opcode::BinaryMul, None)),
            19 => Ok(Instruction::new(Opcode::BinaryMod, None)),
            20 => Ok(Instruction::new(Opcode::BinaryOr, None)),
            21 => Ok(Instruction::new(Opcode::BinaryPow, None)),
            22 => Ok(Instruction::new(Opcode::BinaryRShift, None)),
            23 => Ok(Instruction::new(Opcode::BinarySub, None)),
            24 => Ok(Instruction::new(Opcode::BinaryDiv, None)),
            25 => Ok(Instruction::new(Opcode::BinaryXor, None)),
            26 => Ok(Instruction::new(Opcode::Subscript, None)),
            _ => Err(CpythonError::new(format!(
                "unsupported BINARY_OP arg {} at instruction {}",
                oparg, idx
            ))),
        }
    }
}

fn kw_names_follower(name: &str) -> bool {
    matches!(
        name,
        "CALL"
            | "CALL_KW"
            | "INSTRUMENTED_CALL"
            | "INSTRUMENTED_CALL_KW"
            | "CALL_ALLOC_AND_ENTER_INIT"
            | "CALL_BOUND_METHOD_EXACT_ARGS"
            | "CALL_BOUND_METHOD_GENERAL"
            | "CALL_BUILTIN_CLASS"
            | "CALL_BUILTIN_FAST"
            | "CALL_BUILTIN_FAST_WITH_KEYWORDS"
            | "CALL_BUILTIN_O"
            | "CALL_ISINSTANCE"
            | "CALL_KW_BOUND_METHOD"
            | "CALL_KW_NON_PY"
            | "CALL_KW_PY"
            | "CALL_LEN"
            | "CALL_LIST_APPEND"
            | "CALL_METHOD_DESCRIPTOR_FAST"
            | "CALL_METHOD_DESCRIPTOR_FAST_WITH_KEYWORDS"
            | "CALL_METHOD_DESCRIPTOR_NOARGS"
            | "CALL_METHOD_DESCRIPTOR_O"
            | "CALL_NON_PY_GENERAL"
            | "CALL_PY_EXACT_ARGS"
            | "CALL_PY_GENERAL"
            | "CALL_STR_1"
            | "CALL_TUPLE_1"
            | "CALL_TYPE_1"
            | "CACHE"
            | "RESUME"
            | "RESUME_CHECK"
            | "NOP"
            | "NOT_TAKEN"
            | "EXTENDED_ARG"
    )
}

fn relative_forward_target(idx: usize, arg: u32) -> Result<u32, CpythonError> {
    let delta = arg as usize;
    let target = idx
        .checked_add(1)
        .and_then(|value| value.checked_add(delta))
        .ok_or_else(|| CpythonError::new("jump target overflow"))?;
    u32::try_from(target).map_err(|_| CpythonError::new("jump target overflow"))
}

fn relative_forward_target_plus_two(idx: usize, arg: u32) -> Result<u32, CpythonError> {
    let delta = arg as usize;
    let target = idx
        .checked_add(2)
        .and_then(|value| value.checked_add(delta))
        .ok_or_else(|| CpythonError::new("jump target overflow"))?;
    u32::try_from(target).map_err(|_| CpythonError::new("jump target overflow"))
}

fn relative_backward_target(idx: usize, arg: u32) -> Result<u32, CpythonError> {
    let delta = arg as usize;
    let base = idx
        .checked_add(2)
        .ok_or_else(|| CpythonError::new("jump target overflow"))?;
    let target = base
        .checked_sub(delta)
        .ok_or_else(|| CpythonError::new("backward jump before start"))?;
    u32::try_from(target).map_err(|_| CpythonError::new("jump target overflow"))
}

fn relative_backward_no_interrupt_target(idx: usize, arg: u32) -> Result<u32, CpythonError> {
    let delta = arg as usize;
    let base = idx
        .checked_add(1)
        .ok_or_else(|| CpythonError::new("jump target overflow"))?;
    let target = base
        .checked_sub(delta)
        .ok_or_else(|| CpythonError::new("backward jump before start"))?;
    u32::try_from(target).map_err(|_| CpythonError::new("jump target overflow"))
}

fn for_iter_target(idx: usize, arg: u32) -> Result<u32, CpythonError> {
    let delta = arg as usize;
    let target = idx
        .checked_add(2)
        .and_then(|value| value.checked_add(delta))
        .ok_or_else(|| CpythonError::new("FOR_ITER target overflow"))?;
    u32::try_from(target).map_err(|_| CpythonError::new("FOR_ITER target overflow"))
}

fn validate_cpython_control_flow(instructions: &[CpInstr]) -> Result<(), CpythonError> {
    let len = instructions.len();
    for (idx, instr) in instructions.iter().enumerate() {
        let name = instr.name.as_str();
        let target = match name {
            "POP_JUMP_IF_FALSE"
            | "POP_JUMP_IF_TRUE"
            | "POP_JUMP_IF_NONE"
            | "POP_JUMP_IF_NOT_NONE"
            | "INSTRUMENTED_POP_JUMP_IF_FALSE"
            | "INSTRUMENTED_POP_JUMP_IF_TRUE"
            | "INSTRUMENTED_POP_JUMP_IF_NONE"
            | "INSTRUMENTED_POP_JUMP_IF_NOT_NONE" => {
                Some(relative_forward_target_plus_two(idx, instr.arg)? as usize)
            }
            "JUMP_FORWARD" | "INSTRUMENTED_JUMP_FORWARD" | "JUMP" | "JUMP_NO_INTERRUPT" => {
                Some(relative_forward_target(idx, instr.arg)? as usize)
            }
            "SEND" | "SEND_GEN" => Some(relative_forward_target_plus_two(idx, instr.arg)? as usize),
            "JUMP_BACKWARD"
            | "JUMP_BACKWARD_NO_JIT"
            | "JUMP_BACKWARD_JIT"
            | "INSTRUMENTED_JUMP_BACKWARD" => {
                Some(relative_backward_target(idx, instr.arg)? as usize)
            }
            "JUMP_BACKWARD_NO_INTERRUPT" => {
                Some(relative_backward_no_interrupt_target(idx, instr.arg)? as usize)
            }
            "FOR_ITER"
            | "INSTRUMENTED_FOR_ITER"
            | "FOR_ITER_GEN"
            | "FOR_ITER_LIST"
            | "FOR_ITER_RANGE"
            | "FOR_ITER_TUPLE" => Some(for_iter_target(idx, instr.arg)? as usize),
            _ => None,
        };
        if let Some(target) = target
            && target > len
        {
            return Err(CpythonError::new(format!(
                "jump target {} out of range at instruction {}",
                target, idx
            )));
        }
    }
    Ok(())
}

fn validate_translated_code(instructions: &[Instruction]) -> Result<(), CpythonError> {
    let mut queue = VecDeque::new();
    let mut seen: HashSet<usize> = HashSet::new();
    queue.push_back((0usize, 0i32));

    while let Some((ip, stack_depth)) = queue.pop_front() {
        if ip >= instructions.len() {
            continue;
        }
        if !seen.insert(ip) {
            continue;
        }
        let instr = &instructions[ip];
        let successors = translated_successors(ip, stack_depth, instr, instructions.len())?;
        for (next_ip, next_depth) in successors {
            if next_depth < 0 {
                return Err(CpythonError::new(format!(
                    "stack underflow at instruction {} ({:?})",
                    ip, instr.opcode
                )));
            }
            if next_ip > instructions.len() {
                return Err(CpythonError::new(format!(
                    "translated jump target {} out of range at instruction {}",
                    next_ip, ip
                )));
            }
            queue.push_back((next_ip, next_depth));
        }
    }
    Ok(())
}

fn parse_varint(bytes: &[u8], index: &mut usize) -> Result<u32, CpythonError> {
    if *index >= bytes.len() {
        return Err(CpythonError::new(
            "truncated exception table while parsing varint",
        ));
    }
    let mut value = (bytes[*index] & 0x3F) as u32;
    while bytes[*index] & 0x40 != 0 {
        *index += 1;
        if *index >= bytes.len() {
            return Err(CpythonError::new(
                "truncated exception table while parsing varint",
            ));
        }
        value = (value << 6) | ((bytes[*index] & 0x3F) as u32);
    }
    *index += 1;
    Ok(value)
}

fn parse_exception_table(
    bytes: &[u8],
    instruction_count: usize,
) -> Result<Vec<ExceptionHandler>, CpythonError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let mut handlers = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let start = parse_varint(bytes, &mut index)? as usize;
        let size = parse_varint(bytes, &mut index)? as usize;
        let target = parse_varint(bytes, &mut index)? as usize;
        let depth_and_lasti = parse_varint(bytes, &mut index)? as usize;
        let end = start.saturating_add(size);
        let depth = depth_and_lasti >> 1;
        let push_lasti = (depth_and_lasti & 1) == 1;

        if start > instruction_count || end > instruction_count || target > instruction_count {
            return Err(CpythonError::new(format!(
                "exception table entry out of range (start={start}, end={end}, target={target}, instruction_count={instruction_count})",
            )));
        }
        if start >= end {
            return Err(CpythonError::new(format!(
                "invalid exception table entry range [{start}, {end})"
            )));
        }
        handlers.push(ExceptionHandler {
            start,
            end,
            target,
            depth,
            push_lasti,
        });
    }
    Ok(handlers)
}

fn translated_successors(
    ip: usize,
    stack_depth: i32,
    instr: &Instruction,
    code_len: usize,
) -> Result<Vec<(usize, i32)>, CpythonError> {
    let next_ip = ip + 1;
    let arg = instr.arg;
    let pop = |count: i32| -> Result<i32, CpythonError> {
        if stack_depth < count {
            return Err(CpythonError::new(format!(
                "stack underflow at instruction {} ({:?})",
                ip, instr.opcode
            )));
        }
        Ok(stack_depth - count)
    };

    let successors = match instr.opcode {
        Opcode::Nop
        | Opcode::MakeCell
        | Opcode::DeleteFast
        | Opcode::SetupExcept
        | Opcode::PopBlock
        | Opcode::ClearException
        | Opcode::EndFor
        | Opcode::SetupAnnotations => vec![(next_ip, stack_depth)],
        Opcode::LoadConst
        | Opcode::LoadName
        | Opcode::LoadLocals
        | Opcode::LoadFast
        | Opcode::LoadDeref
        | Opcode::LoadClosure
        | Opcode::LoadBuildClass
        | Opcode::PushNull => vec![(next_ip, stack_depth + 1)],
        Opcode::LoadFromDictOrGlobals | Opcode::LoadFromDictOrDeref => {
            vec![(next_ip, pop(1)? + 1)]
        }
        Opcode::LoadFast2 => vec![(next_ip, stack_depth + 2)],
        Opcode::LoadFastAndClear => vec![(next_ip, stack_depth + 1)],
        Opcode::LoadGlobal => {
            let push_null = arg.unwrap_or(0) & 1;
            let pushes = if push_null == 1 { 2 } else { 1 };
            vec![(next_ip, stack_depth + pushes)]
        }
        Opcode::LoadAttr => {
            let raw = arg.ok_or_else(|| CpythonError::new("missing LOAD_ATTR arg"))?;
            let push_null = (raw & 1) as i32;
            vec![(next_ip, pop(1)? + 1 + push_null)]
        }
        Opcode::LoadSuperAttr => {
            let raw = arg.ok_or_else(|| CpythonError::new("missing LOAD_SUPER_ATTR arg"))?;
            let push_null = (raw & 1) as i32;
            vec![(next_ip, pop(3)? + 1 + push_null)]
        }
        Opcode::LoadSpecial => vec![(next_ip, pop(1)? + 2)],
        Opcode::StoreName
        | Opcode::StoreFast
        | Opcode::StoreGlobal
        | Opcode::StoreDeref
        | Opcode::DeleteAttr
        | Opcode::PopTop
        | Opcode::PopExcept => vec![(next_ip, pop(1)?)],
        Opcode::PushExcInfo => vec![(next_ip, pop(1)? + 2)],
        Opcode::WithExceptStart => vec![(next_ip, pop(5)? + 6)],
        Opcode::UnaryNeg
        | Opcode::UnaryNot
        | Opcode::UnaryPos
        | Opcode::UnaryInvert
        | Opcode::ConvertValue
        | Opcode::FormatSimple
        | Opcode::CallIntrinsic1
        | Opcode::ToBool => vec![(next_ip, pop(1)? + 1)],
        Opcode::CallIntrinsic2 => vec![(next_ip, pop(2)? + 1)],
        Opcode::StoreFastLoadFast => {
            let depth = pop(1)? + 1;
            vec![(next_ip, depth)]
        }
        Opcode::StoreFastStoreFast => vec![(next_ip, pop(2)?)],
        Opcode::StoreAttr | Opcode::StoreAttrCpython => vec![(next_ip, pop(2)?)],
        Opcode::Copy => {
            let depth = arg.ok_or_else(|| CpythonError::new("missing COPY depth"))? as i32;
            if depth <= 0 {
                return Err(CpythonError::new(format!(
                    "invalid COPY arg {} at instruction {}",
                    depth, ip
                )));
            }
            vec![(next_ip, pop(depth)? + depth + 1)]
        }
        Opcode::Swap => {
            let depth = arg.ok_or_else(|| CpythonError::new("missing SWAP depth"))? as i32;
            if depth < 2 {
                return Err(CpythonError::new(format!(
                    "invalid SWAP arg {} at instruction {}",
                    depth, ip
                )));
            }
            vec![(next_ip, pop(depth)? + depth)]
        }
        Opcode::BinaryAdd
        | Opcode::BinarySub
        | Opcode::BinaryMul
        | Opcode::BinaryDiv
        | Opcode::BinaryPow
        | Opcode::BinaryFloorDiv
        | Opcode::BinaryMod
        | Opcode::CompareEq
        | Opcode::CompareNe
        | Opcode::CompareLt
        | Opcode::CompareLe
        | Opcode::CompareGt
        | Opcode::CompareGe
        | Opcode::CompareIn
        | Opcode::CompareNotIn
        | Opcode::CompareIs
        | Opcode::CompareIsNot
        | Opcode::Subscript
        | Opcode::MatchException => vec![(next_ip, pop(2)? + 1)],
        Opcode::CheckExcMatch => vec![(next_ip, pop(2)? + 2)],
        Opcode::MatchExceptionStar => vec![(next_ip, pop(2)? + 2)],
        Opcode::BuildList | Opcode::BuildSet | Opcode::BuildTuple | Opcode::BuildString => {
            let count = arg.ok_or_else(|| CpythonError::new("missing build count"))? as i32;
            vec![(next_ip, pop(count)? + 1)]
        }
        Opcode::FormatWithSpec => vec![(next_ip, pop(2)? + 1)],
        Opcode::BuildDict => {
            let count = arg.ok_or_else(|| CpythonError::new("missing dict count"))? as i32;
            vec![(next_ip, pop(count * 2)? + 1)]
        }
        Opcode::ListAppend | Opcode::SetAdd | Opcode::ListExtend | Opcode::SetUpdate => {
            let oparg = arg.ok_or_else(|| CpythonError::new("missing LIST_* arg"))? as i32;
            if oparg <= 0 {
                return Err(CpythonError::new(format!(
                    "invalid LIST_* arg {} at instruction {}",
                    oparg, ip
                )));
            }
            vec![(next_ip, pop(oparg + 1)? + oparg)]
        }
        Opcode::MapAdd => {
            let oparg = arg.ok_or_else(|| CpythonError::new("missing MAP_ADD arg"))? as i32;
            if oparg <= 0 {
                return Err(CpythonError::new(format!(
                    "invalid MAP_ADD arg {} at instruction {}",
                    oparg, ip
                )));
            }
            vec![(next_ip, pop(oparg + 2)? + oparg)]
        }
        Opcode::DictMerge => {
            let oparg = arg.ok_or_else(|| CpythonError::new("missing DICT_MERGE arg"))? as i32;
            if oparg <= 0 {
                return Err(CpythonError::new(format!(
                    "invalid DICT_MERGE arg {} at instruction {}",
                    oparg, ip
                )));
            }
            vec![(next_ip, pop(oparg + 1)? + oparg)]
        }
        Opcode::BuildSlice => {
            let count = arg.unwrap_or(3) as i32;
            if count != 2 && count != 3 {
                return Err(CpythonError::new(format!(
                    "invalid BUILD_SLICE arg {} at instruction {}",
                    count, ip
                )));
            }
            vec![(next_ip, pop(count)? + 1)]
        }
        Opcode::BinarySlice => vec![(next_ip, pop(3)? + 1)],
        Opcode::UnpackSequence | Opcode::UnpackSequenceCpython => {
            let count = arg.ok_or_else(|| CpythonError::new("missing unpack count"))? as i32;
            vec![(next_ip, pop(1)? + count)]
        }
        Opcode::UnpackEx | Opcode::UnpackExCpython => {
            let packed = arg.ok_or_else(|| CpythonError::new("missing unpack sizes"))?;
            let before = (packed & 0xFFFF) as i32;
            let after = (packed >> 16) as i32;
            vec![(next_ip, pop(1)? + before + after + 1)]
        }
        Opcode::DictSet => vec![(next_ip, pop(3)? + 1)],
        Opcode::DictUpdate => {
            let oparg = arg.unwrap_or(1) as i32;
            if oparg <= 0 {
                return Err(CpythonError::new(format!(
                    "invalid DICT_UPDATE arg {} at instruction {}",
                    oparg, ip
                )));
            }
            vec![(next_ip, pop(oparg + 1)? + oparg)]
        }
        Opcode::StoreSubscript => {
            if arg == Some(1) {
                vec![(next_ip, pop(3)?)]
            } else {
                vec![(next_ip, pop(3)? + 1)]
            }
        }
        Opcode::StoreSlice => vec![(next_ip, pop(4)?)],
        Opcode::DeleteSubscript => vec![(next_ip, pop(2)?)],
        Opcode::DupTop => vec![(next_ip, pop(1)? + 2)],
        Opcode::JumpIfFalse | Opcode::JumpIfTrue | Opcode::JumpIfNone | Opcode::JumpIfNotNone => {
            let target = arg.ok_or_else(|| CpythonError::new("missing jump target"))? as usize;
            let depth = pop(1)?;
            vec![(next_ip, depth), (target, depth)]
        }
        Opcode::Jump => {
            let target = arg.ok_or_else(|| CpythonError::new("missing jump target"))? as usize;
            vec![(target, stack_depth)]
        }
        Opcode::GetIter => vec![(next_ip, pop(1)? + 1)],
        Opcode::GetAwaitable => vec![(next_ip, pop(1)? + 1)],
        Opcode::ForIter => {
            let target = arg.ok_or_else(|| CpythonError::new("missing for-iter target"))? as usize;
            vec![(next_ip, pop(1)? + 2), (target, pop(1)?)]
        }
        Opcode::YieldValue => {
            let depth = pop(1)? + 1;
            vec![(next_ip, depth)]
        }
        Opcode::YieldFrom => vec![(next_ip, stack_depth)],
        Opcode::Send => {
            let target = arg.ok_or_else(|| CpythonError::new("missing send target"))? as usize;
            let depth = pop(2)?;
            vec![(next_ip, depth + 2), (target, depth + 1)]
        }
        Opcode::MakeFunction => vec![(next_ip, pop(2)? + 1)],
        Opcode::MakeFunctionStack => vec![(next_ip, pop(1)? + 1)],
        Opcode::SetFunctionAttribute => vec![(next_ip, pop(2)? + 1)],
        Opcode::BuildClass => vec![(next_ip, pop(3)? + 1)],
        Opcode::CallFunction => {
            let argc = arg.ok_or_else(|| CpythonError::new("missing call argc"))? as i32;
            vec![(next_ip, pop(argc + 1)? + 1)]
        }
        Opcode::CallFunctionKw => vec![(next_ip, pop(1)? + 1)],
        Opcode::CallFunctionVar => vec![(next_ip, pop(3)? + 1)],
        Opcode::CallFunctionEx => vec![(next_ip, pop(4)? + 1)],
        Opcode::CallCpython => {
            let argc = (arg.ok_or_else(|| CpythonError::new("missing call argc"))? & 0xFFFF) as i32;
            vec![(next_ip, pop(argc + 1)? + 1)]
        }
        Opcode::CallCpythonKwStack => {
            let argc = arg.ok_or_else(|| CpythonError::new("missing call argc"))? as i32;
            vec![(next_ip, pop(argc + 2)? + 1)]
        }
        Opcode::ImportName => vec![(next_ip, stack_depth + 1)],
        Opcode::ImportNameCpython => vec![(next_ip, pop(2)? + 1)],
        Opcode::ImportFromCpython => vec![(next_ip, pop(1)? + 2)],
        Opcode::Raise | Opcode::Reraise | Opcode::ReturnConst | Opcode::ReturnValue => Vec::new(),
        _ => vec![(next_ip, stack_depth)],
    };

    if successors.iter().any(|(next, _)| *next > code_len) {
        return Err(CpythonError::new(format!(
            "translated jump target out of range at instruction {}",
            ip
        )));
    }

    Ok(successors)
}

struct CpInstr {
    name: String,
    arg: u32,
}

fn decode_instructions(
    bytes: &[u8],
    opmap: &HashMap<u8, String>,
) -> Result<Vec<CpInstr>, CpythonError> {
    if !bytes.len().is_multiple_of(2) {
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
            instructions.push(CpInstr {
                name,
                arg: full_arg,
            });
        }
        i += 2;
    }
    Ok(instructions)
}

struct MarshalWriter {
    data: Vec<u8>,
}

impl MarshalWriter {
    fn new() -> Self {
        Self { data: Vec::new() }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    fn write_code_object(&mut self, code: &CpythonCode) -> Result<(), CpythonError> {
        self.write_u8(b'c');
        self.write_i32(code.argcount);
        self.write_i32(code.posonlyargcount);
        self.write_i32(code.kwonlyargcount);
        self.write_i32(code.stacksize);
        self.write_i32(code.flags);
        self.write_object(&PyObject::Bytes(code.code.clone()))?;
        self.write_object(&PyObject::Tuple(code.consts.clone()))?;
        self.write_object(&PyObject::Tuple(
            code.names.iter().cloned().map(PyObject::Str).collect(),
        ))?;
        self.write_object(&PyObject::Tuple(
            code.localsplusnames
                .iter()
                .cloned()
                .map(PyObject::Str)
                .collect(),
        ))?;
        self.write_object(&PyObject::Bytes(code.localspluskinds.clone()))?;
        self.write_object(&PyObject::Str(code.filename.clone()))?;
        self.write_object(&PyObject::Str(code.name.clone()))?;
        self.write_object(&PyObject::Str(code.qualname.clone()))?;
        self.write_i32(code.firstlineno);
        self.write_object(&PyObject::Bytes(code.linetable.clone()))?;
        self.write_object(&PyObject::Bytes(code.exceptiontable.clone()))?;
        Ok(())
    }

    fn write_object(&mut self, obj: &PyObject) -> Result<(), CpythonError> {
        match obj {
            PyObject::Null => self.write_u8(b'0'),
            PyObject::None => self.write_u8(b'N'),
            PyObject::StopIteration => self.write_u8(b'S'),
            PyObject::Ellipsis => self.write_u8(b'.'),
            PyObject::Bool(false) => self.write_u8(b'F'),
            PyObject::Bool(true) => self.write_u8(b'T'),
            PyObject::Int(value) => {
                if *value >= i32::MIN as i64 && *value <= i32::MAX as i64 {
                    self.write_u8(b'i');
                    self.write_i32(*value as i32);
                } else {
                    self.write_u8(b'l');
                    self.write_long(*value)?;
                }
            }
            PyObject::BigInt(_) => {
                return Err(CpythonError::new(
                    "cannot marshal bigint constants outside i64 range",
                ));
            }
            PyObject::Float(value) => {
                self.write_u8(b'g');
                self.data.extend_from_slice(&value.to_le_bytes());
            }
            PyObject::Complex { real, imag } => {
                self.write_u8(b'y');
                self.data.extend_from_slice(&real.to_le_bytes());
                self.data.extend_from_slice(&imag.to_le_bytes());
            }
            PyObject::Str(value) => {
                self.write_u8(b'u');
                self.write_bytes_long(value.as_bytes())?;
            }
            PyObject::Bytes(value) => {
                self.write_u8(b's');
                self.write_bytes_long(value)?;
            }
            PyObject::Tuple(items) => {
                self.write_u8(b'(');
                self.write_i32(
                    i32::try_from(items.len())
                        .map_err(|_| CpythonError::new("tuple constant too large"))?,
                );
                for item in items {
                    self.write_object(item)?;
                }
            }
            PyObject::List(items) => {
                self.write_u8(b'[');
                self.write_i32(
                    i32::try_from(items.len())
                        .map_err(|_| CpythonError::new("list constant too large"))?,
                );
                for item in items {
                    self.write_object(item)?;
                }
            }
            PyObject::Dict(entries) => {
                self.write_u8(b'{');
                for (key, value) in entries {
                    self.write_object(key)?;
                    self.write_object(value)?;
                }
                self.write_u8(b'0');
            }
            PyObject::Set(items) => {
                self.write_u8(b'<');
                self.write_i32(
                    i32::try_from(items.len())
                        .map_err(|_| CpythonError::new("set constant too large"))?,
                );
                for item in items {
                    self.write_object(item)?;
                }
            }
            PyObject::FrozenSet(items) => {
                self.write_u8(b'>');
                self.write_i32(
                    i32::try_from(items.len())
                        .map_err(|_| CpythonError::new("frozenset constant too large"))?,
                );
                for item in items {
                    self.write_object(item)?;
                }
            }
            PyObject::Code(code) => {
                self.write_code_object(code)?;
            }
            PyObject::Slice { lower, upper, step } => {
                self.write_u8(b':');
                if let Some(value) = lower {
                    self.write_object(value)?;
                } else {
                    self.write_object(&PyObject::Null)?;
                }
                if let Some(value) = upper {
                    self.write_object(value)?;
                } else {
                    self.write_object(&PyObject::Null)?;
                }
                if let Some(value) = step {
                    self.write_object(value)?;
                } else {
                    self.write_object(&PyObject::Null)?;
                }
            }
        }
        Ok(())
    }

    fn write_u8(&mut self, value: u8) {
        self.data.push(value);
    }

    fn write_i32(&mut self, value: i32) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u16(&mut self, value: u16) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }

    fn write_bytes_long(&mut self, bytes: &[u8]) -> Result<(), CpythonError> {
        self.write_i32(
            i32::try_from(bytes.len())
                .map_err(|_| CpythonError::new("byte sequence too large for marshal"))?,
        );
        self.data.extend_from_slice(bytes);
        Ok(())
    }

    fn write_long(&mut self, value: i64) -> Result<(), CpythonError> {
        if value == 0 {
            self.write_i32(0);
            return Ok(());
        }
        let sign = if value < 0 { -1 } else { 1 };
        let mut abs = (value as i128).abs();
        let mut digits = Vec::new();
        while abs > 0 {
            digits.push((abs & 0x7fff) as u16);
            abs >>= 15;
        }
        let count = i32::try_from(digits.len())
            .map_err(|_| CpythonError::new("long constant too large"))?;
        self.write_i32(if sign < 0 { -count } else { count });
        for digit in digits {
            self.write_u16(digit);
        }
        Ok(())
    }
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
        let code = self.read_u8()?;
        let flag = (code & 0x80) != 0;
        let obj_type = code & 0x7f;
        let ref_index = if flag { Some(self.reserve_ref()) } else { None };

        let value = match obj_type as char {
            '0' => PyObject::Null,
            'N' => PyObject::None,
            'S' => PyObject::StopIteration,
            '.' => PyObject::Ellipsis,
            'F' => PyObject::Bool(false),
            'T' => PyObject::Bool(true),
            'i' => PyObject::Int(self.read_i32()? as i64),
            'l' => self.read_long_object()?,
            'g' => PyObject::Float(f64::from_le_bytes(self.read_exact(8)?.try_into().unwrap())),
            'y' => {
                let real = f64::from_le_bytes(self.read_exact(8)?.try_into().unwrap());
                let imag = f64::from_le_bytes(self.read_exact(8)?.try_into().unwrap());
                PyObject::Complex { real, imag }
            }
            'f' => {
                let len = self.read_u8()? as usize;
                let bytes = self.read_exact(len)?;
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| CpythonError::new("invalid marshal float string"))?;
                let value = text
                    .parse::<f64>()
                    .map_err(|_| CpythonError::new("invalid marshal float literal"))?;
                PyObject::Float(value)
            }
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
            '<' => {
                let size = self.read_i32()? as usize;
                let mut items = Vec::with_capacity(size);
                for _ in 0..size {
                    items.push(self.read_object(allow_code)?);
                }
                PyObject::Set(items)
            }
            '>' => {
                let size = self.read_i32()? as usize;
                let mut items = Vec::with_capacity(size);
                for _ in 0..size {
                    items.push(self.read_object(allow_code)?);
                }
                PyObject::FrozenSet(items)
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
                )));
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

    fn read_long_object(&mut self) -> Result<PyObject, CpythonError> {
        let n = self.read_i32()?;
        if n == 0 {
            return Ok(PyObject::Int(0));
        }
        let sign = if n < 0 { -1 } else { 1 };
        let count = n.unsigned_abs() as usize;
        let mut digits = Vec::with_capacity(count);
        for _ in 0..count {
            digits.push(self.read_u16()? as u64);
        }
        let mut value = BigInt::zero();
        for digit in digits.into_iter().rev() {
            value = value.shl_bits(15);
            if digit != 0 {
                value = value.add(&BigInt::from_u64(digit));
            }
        }
        if sign < 0 {
            value = value.negated();
        }
        match value.to_i64() {
            Some(integer) => Ok(PyObject::Int(integer)),
            None => Ok(PyObject::BigInt(value)),
        }
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
