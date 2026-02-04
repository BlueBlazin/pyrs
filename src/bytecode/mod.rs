//! Bytecode representation and metadata (stubbed).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opcode {
    Nop,
    ReturnConst,
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
}

impl CodeObject {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            instructions: Vec::new(),
        }
    }
}
