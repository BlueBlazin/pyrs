//! Bytecode representation and metadata (stubbed).

pub mod cpython;
pub mod metadata;
pub mod pyc;

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Nop,
    MakeCell,
    LoadConst,
    LoadName,
    LoadLocals,
    LoadFromDictOrGlobals,
    LoadFromDictOrDeref,
    LoadFast,
    LoadFast2,
    LoadFastAndClear,
    LoadDeref,
    LoadClosure,
    LoadGlobal,
    LoadBuildClass,
    PushNull,
    LoadAttr,
    LoadSuperAttr,
    LoadSpecial,
    StoreName,
    StoreFast,
    StoreFastLoadFast,
    StoreFastStoreFast,
    StoreAttr,
    StoreAttrCpython,
    StoreGlobal,
    StoreDeref,
    DeleteName,
    DeleteFast,
    DeleteAttr,
    DeleteSubscript,
    BinaryAdd,
    BinarySub,
    BinarySubConst,
    BinaryMul,
    BinaryMatMul,
    BinaryDiv,
    BinaryPow,
    BinaryFloorDiv,
    BinaryMod,
    BinaryLShift,
    BinaryRShift,
    BinaryAnd,
    BinaryXor,
    BinaryOr,
    CompareEq,
    CompareNe,
    CompareLt,
    CompareLtConst,
    CompareLe,
    CompareGt,
    CompareGe,
    CompareIn,
    CompareNotIn,
    CompareIs,
    CompareIsNot,
    UnaryNeg,
    UnaryNot,
    UnaryPos,
    UnaryInvert,
    ConvertValue,
    FormatSimple,
    FormatWithSpec,
    ToBool,
    MakeFunction,
    BuildClass,
    CallFunction,
    CallFunction1,
    CallFunctionKw,
    CallFunctionVar,
    CallFunctionEx,
    ImportName,
    ImportNameCpython,
    ImportFromCpython,
    BuildList,
    BuildSet,
    BuildTuple,
    BuildString,
    BuildDict,
    DictMerge,
    BuildSlice,
    BinarySlice,
    StoreSlice,
    Copy,
    Swap,
    UnpackSequence,
    UnpackSequenceCpython,
    UnpackEx,
    UnpackExCpython,
    ListAppend,
    SetAdd,
    ListExtend,
    SetUpdate,
    MapAdd,
    DictSet,
    DictUpdate,
    Subscript,
    StoreSubscript,
    DupTop,
    JumpIfFalse,
    JumpIfTrue,
    JumpIfNone,
    JumpIfNotNone,
    Jump,
    SetupExcept,
    SetupAnnotations,
    PopBlock,
    Raise,
    MatchException,
    CheckExcMatch,
    MatchExceptionStar,
    ClearException,
    PushExcInfo,
    PopExcept,
    WithExceptStart,
    Reraise,
    PopTop,
    EndFor,
    GetIter,
    GetAwaitable,
    ForIter,
    YieldValue,
    YieldFrom,
    Send,
    CallCpython,
    CallCpythonKwStack,
    CallIntrinsic1,
    CallIntrinsic2,
    MakeFunctionStack,
    SetFunctionAttribute,
    ReturnConst,
    ReturnValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExceptionHandler {
    pub start: usize,
    pub end: usize,
    pub target: usize,
    pub depth: usize,
    pub push_lasti: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub opcode: Opcode,
    pub arg: Option<u32>,
}

impl Instruction {
    pub fn new(opcode: Opcode, arg: Option<u32>) -> Self {
        Self { opcode, arg }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl Location {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }

    pub fn unknown() -> Self {
        Self { line: 0, column: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeObject {
    pub name: String,
    pub filename: String,
    pub instructions: Vec<Instruction>,
    pub locations: Vec<Location>,
    pub constants: Vec<crate::runtime::Value>,
    pub names: Vec<String>,
    pub cellvars: Vec<String>,
    pub freevars: Vec<String>,
    pub posonly_params: Vec<String>,
    pub params: Vec<String>,
    pub vararg: Option<String>,
    pub kwarg: Option<String>,
    pub kwonly_params: Vec<String>,
    pub name_to_index: HashMap<String, usize>,
    pub cellvar_to_index: HashMap<String, usize>,
    pub fast_local_count: usize,
    pub plain_positional_arity: Option<usize>,
    pub plain_positional_arg0_slot: Option<usize>,
    pub plain_positional_arg0_cell: Option<usize>,
    pub plain_positional_arg1_slot: Option<usize>,
    pub plain_positional_arg1_cell: Option<usize>,
    pub plain_positional_arg2_slot: Option<usize>,
    pub plain_positional_arg2_cell: Option<usize>,
    pub positional_param_slot_indexes: Vec<Option<usize>>,
    pub positional_param_cell_indexes: Vec<Option<usize>>,
    pub is_comprehension: bool,
    pub is_generator: bool,
    pub is_coroutine: bool,
    pub is_async_generator: bool,
    pub exception_handlers: Vec<ExceptionHandler>,
}

impl CodeObject {
    pub fn new(name: impl Into<String>, filename: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            is_comprehension: matches!(name.as_str(), "<listcomp>" | "<dictcomp>" | "<genexpr>"),
            name,
            filename: filename.into(),
            instructions: Vec::new(),
            locations: Vec::new(),
            constants: vec![crate::runtime::Value::None],
            names: Vec::new(),
            cellvars: Vec::new(),
            freevars: Vec::new(),
            posonly_params: Vec::new(),
            params: Vec::new(),
            vararg: None,
            kwarg: None,
            kwonly_params: Vec::new(),
            name_to_index: HashMap::new(),
            cellvar_to_index: HashMap::new(),
            fast_local_count: 0,
            plain_positional_arity: None,
            plain_positional_arg0_slot: None,
            plain_positional_arg0_cell: None,
            plain_positional_arg1_slot: None,
            plain_positional_arg1_cell: None,
            plain_positional_arg2_slot: None,
            plain_positional_arg2_cell: None,
            positional_param_slot_indexes: Vec::new(),
            positional_param_cell_indexes: Vec::new(),
            is_generator: false,
            is_coroutine: false,
            is_async_generator: false,
            exception_handlers: Vec::new(),
        }
    }

    pub fn add_const(&mut self, value: crate::runtime::Value) -> u32 {
        self.constants.push(value);
        (self.constants.len() - 1) as u32
    }

    pub fn add_name(&mut self, name: impl Into<String>) -> u32 {
        let name = name.into();
        if let Some(idx) = self.name_to_index.get(&name) {
            return *idx as u32;
        }
        self.name_to_index.insert(name.clone(), self.names.len());
        self.names.push(name);
        (self.names.len() - 1) as u32
    }

    pub fn rebuild_layout_indexes(&mut self) {
        self.name_to_index.clear();
        for (idx, name) in self.names.iter().enumerate() {
            self.name_to_index.insert(name.clone(), idx);
        }
        self.cellvar_to_index.clear();
        for (idx, name) in self.cellvars.iter().enumerate() {
            self.cellvar_to_index.insert(name.clone(), idx);
        }
        self.positional_param_slot_indexes.clear();
        self.positional_param_cell_indexes.clear();
        self.positional_param_slot_indexes
            .reserve(self.posonly_params.len() + self.params.len());
        self.positional_param_cell_indexes
            .reserve(self.posonly_params.len() + self.params.len());
        for name in self.posonly_params.iter().chain(self.params.iter()) {
            self.positional_param_slot_indexes
                .push(self.name_to_index.get(name).copied());
            self.positional_param_cell_indexes
                .push(self.cellvar_to_index.get(name).copied());
        }

        let mut fast_local_count = 0usize;
        for instr in &self.instructions {
            match instr.opcode {
                Opcode::LoadFast | Opcode::StoreFast | Opcode::LoadFastAndClear => {
                    if let Some(idx) = instr.arg {
                        let idx = idx as usize + 1;
                        if idx > fast_local_count {
                            fast_local_count = idx;
                        }
                    }
                }
                Opcode::LoadFast2 | Opcode::StoreFastLoadFast | Opcode::StoreFastStoreFast => {
                    if let Some(arg) = instr.arg {
                        let first = ((arg >> 16) as usize) + 1;
                        let second = ((arg & 0xFFFF) as usize) + 1;
                        if first > fast_local_count {
                            fast_local_count = first;
                        }
                        if second > fast_local_count {
                            fast_local_count = second;
                        }
                    }
                }
                _ => {}
            }
        }
        for idx in self.positional_param_slot_indexes.iter().flatten().copied() {
            let next = idx + 1;
            if next > fast_local_count {
                fast_local_count = next;
            }
        }
        self.fast_local_count = fast_local_count;
        self.plain_positional_arg0_slot = self
            .positional_param_slot_indexes
            .first()
            .and_then(|idx| *idx);
        self.plain_positional_arg0_cell = self
            .positional_param_cell_indexes
            .first()
            .and_then(|idx| *idx);
        self.plain_positional_arg1_slot = self
            .positional_param_slot_indexes
            .get(1)
            .and_then(|idx| *idx);
        self.plain_positional_arg1_cell = self
            .positional_param_cell_indexes
            .get(1)
            .and_then(|idx| *idx);
        self.plain_positional_arg2_slot = self
            .positional_param_slot_indexes
            .get(2)
            .and_then(|idx| *idx);
        self.plain_positional_arg2_cell = self
            .positional_param_cell_indexes
            .get(2)
            .and_then(|idx| *idx);
        self.is_comprehension = matches!(
            self.name.as_str(),
            "<listcomp>" | "<dictcomp>" | "<genexpr>"
        );
        if self.kwonly_params.is_empty() && self.vararg.is_none() && self.kwarg.is_none() {
            self.plain_positional_arity = Some(self.posonly_params.len() + self.params.len());
        } else {
            self.plain_positional_arity = None;
        }
    }

    pub fn disassemble(&self) -> String {
        let mut output = String::new();
        for (idx, instr) in self.instructions.iter().enumerate() {
            if let Some(arg) = instr.arg {
                output.push_str(&format!("{idx:04} {:?} {arg}\n", instr.opcode));
            } else {
                output.push_str(&format!("{idx:04} {:?}\n", instr.opcode));
            }
        }
        output
    }
}
