//! Bytecode representation and metadata (stubbed).

pub mod cpython;
pub mod metadata;
pub mod pyc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opcode {
    Nop,
    LoadConst,
    LoadName,
    LoadLocals,
    LoadFast,
    LoadFast2,
    LoadFastAndClear,
    LoadDeref,
    LoadClosure,
    LoadGlobal,
    LoadBuildClass,
    PushNull,
    LoadAttr,
    StoreName,
    StoreFast,
    StoreFastLoadFast,
    StoreFastStoreFast,
    StoreAttr,
    StoreAttrCpython,
    StoreGlobal,
    StoreDeref,
    BinaryAdd,
    BinarySub,
    BinaryMul,
    BinaryPow,
    BinaryFloorDiv,
    BinaryMod,
    CompareEq,
    CompareNe,
    CompareLt,
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
    ToBool,
    MakeFunction,
    BuildClass,
    CallFunction,
    CallFunctionKw,
    CallFunctionVar,
    ImportName,
    ImportNameCpython,
    ImportFromCpython,
    BuildList,
    BuildTuple,
    BuildDict,
    BuildSlice,
    UnpackSequence,
    ListAppend,
    ListExtend,
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
    ClearException,
    PopTop,
    EndFor,
    GetIter,
    ForIter,
    YieldValue,
    YieldFrom,
    Send,
    CallCpython,
    CallCpythonKwStack,
    MakeFunctionStack,
    SetFunctionAttribute,
    ReturnConst,
    ReturnValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub is_generator: bool,
}

impl CodeObject {
    pub fn new(name: impl Into<String>, filename: impl Into<String>) -> Self {
        Self {
            name: name.into(),
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
            is_generator: false,
        }
    }

    pub fn add_const(&mut self, value: crate::runtime::Value) -> u32 {
        self.constants.push(value);
        (self.constants.len() - 1) as u32
    }

    pub fn add_name(&mut self, name: impl Into<String>) -> u32 {
        let name = name.into();
        if let Some((idx, _)) = self
            .names
            .iter()
            .enumerate()
            .find(|(_, existing)| *existing == &name)
        {
            return idx as u32;
        }
        self.names.push(name);
        (self.names.len() - 1) as u32
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
