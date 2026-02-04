//! Bytecode representation and metadata (stubbed).

pub mod metadata;
pub mod pyc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opcode {
    Nop,
    LoadConst,
    LoadName,
    StoreName,
    BinaryAdd,
    BinarySub,
    BinaryMul,
    CompareEq,
    CompareLt,
    UnaryNeg,
    JumpIfFalse,
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
}

impl CodeObject {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            instructions: Vec::new(),
            constants: vec![crate::runtime::Value::None],
            names: Vec::new(),
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
}
