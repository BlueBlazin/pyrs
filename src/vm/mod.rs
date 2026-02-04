//! Bytecode virtual machine (stubbed).

use crate::bytecode::CodeObject;
use crate::runtime::{RuntimeError, Value};

#[derive(Default)]
pub struct Vm;

impl Vm {
    pub fn new() -> Self {
        Self
    }

    pub fn execute(&mut self, _code: &CodeObject) -> Result<Value, RuntimeError> {
        Ok(Value::None)
    }
}
