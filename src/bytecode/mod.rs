//! Bytecode representation and metadata (stubbed).

pub mod metadata;
pub mod pyc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opcode {
    Nop,
    LoadConst,
    LoadName,
    LoadAttr,
    StoreName,
    BinaryAdd,
    BinarySub,
    BinaryMul,
    BinaryMod,
    CompareEq,
    CompareNe,
    CompareLt,
    CompareLe,
    CompareGt,
    CompareGe,
    CompareIn,
    CompareNotIn,
    UnaryNeg,
    UnaryNot,
    MakeFunction,
    CallFunction,
    ImportName,
    BuildList,
    BuildTuple,
    BuildDict,
    BuildSlice,
    Subscript,
    StoreSubscript,
    DupTop,
    JumpIfFalse,
    JumpIfTrue,
    Jump,
    PopTop,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeObject {
    pub name: String,
    pub instructions: Vec<Instruction>,
    pub constants: Vec<crate::runtime::Value>,
    pub names: Vec<String>,
    pub params: Vec<String>,
}

impl CodeObject {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            instructions: Vec::new(),
            constants: vec![crate::runtime::Value::None],
            names: Vec::new(),
            params: Vec::new(),
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
