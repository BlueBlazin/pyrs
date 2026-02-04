//! Bytecode virtual machine (minimal subset).

use std::collections::HashMap;

use crate::bytecode::{CodeObject, Opcode};
use crate::runtime::{RuntimeError, Value};

#[derive(Default)]
pub struct Vm {
    stack: Vec<Value>,
    globals: HashMap<String, Value>,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            globals: HashMap::new(),
        }
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: Value) {
        self.globals.insert(name.into(), value);
    }

    pub fn get_global(&self, name: &str) -> Option<&Value> {
        self.globals.get(name)
    }

    pub fn execute(&mut self, code: &CodeObject) -> Result<Value, RuntimeError> {
        let mut ip = 0usize;
        while ip < code.instructions.len() {
            let instr = &code.instructions[ip];
            match instr.opcode {
                Opcode::Nop => {}
                Opcode::LoadConst => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing const argument"))?
                        as usize;
                    let value = code
                        .constants
                        .get(idx)
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("constant index out of range"))?;
                    self.stack.push(value);
                }
                Opcode::LoadName => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing name argument"))?
                        as usize;
                    let name = code
                        .names
                        .get(idx)
                        .ok_or_else(|| RuntimeError::new("name index out of range"))?;
                    let value = self.globals.get(name).cloned().ok_or_else(|| {
                        RuntimeError::new(format!("name '{name}' is not defined"))
                    })?;
                    self.stack.push(value);
                }
                Opcode::StoreName => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing name argument"))?
                        as usize;
                    let name = code
                        .names
                        .get(idx)
                        .ok_or_else(|| RuntimeError::new("name index out of range"))?
                        .clone();
                    let value = self
                        .stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("stack underflow"))?;
                    self.globals.insert(name, value);
                }
                Opcode::PopTop => {
                    self.stack.pop();
                }
                Opcode::ReturnValue => {
                    let value = self.stack.pop().unwrap_or(Value::None);
                    return Ok(value);
                }
            }
            ip += 1;
        }

        Ok(Value::None)
    }
}
