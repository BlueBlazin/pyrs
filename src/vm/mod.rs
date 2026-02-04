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
                Opcode::BinaryAdd => {
                    let (left, right) = self.pop_int_pair()?;
                    self.stack.push(Value::Int(left + right));
                }
                Opcode::BinarySub => {
                    let (left, right) = self.pop_int_pair()?;
                    self.stack.push(Value::Int(left - right));
                }
                Opcode::BinaryMul => {
                    let (left, right) = self.pop_int_pair()?;
                    self.stack.push(Value::Int(left * right));
                }
                Opcode::CompareEq => {
                    let right = self
                        .stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("stack underflow"))?;
                    let left = self
                        .stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("stack underflow"))?;
                    self.stack.push(Value::Bool(left == right));
                }
                Opcode::CompareLt => {
                    let (left, right) = self.pop_int_pair()?;
                    self.stack.push(Value::Bool(left < right));
                }
                Opcode::UnaryNeg => {
                    let value = self
                        .stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("stack underflow"))?;
                    let value = value_to_int(value)?;
                    self.stack.push(Value::Int(-value));
                }
                Opcode::JumpIfFalse => {
                    let target = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing jump target"))? as usize;
                    let value = self
                        .stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("stack underflow"))?;
                    if !is_truthy(&value) {
                        ip = target;
                        continue;
                    }
                }
                Opcode::Jump => {
                    let target = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing jump target"))? as usize;
                    ip = target;
                    continue;
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

    fn pop_int_pair(&mut self) -> Result<(i64, i64), RuntimeError> {
        let right = self
            .stack
            .pop()
            .ok_or_else(|| RuntimeError::new("stack underflow"))?;
        let left = self
            .stack
            .pop()
            .ok_or_else(|| RuntimeError::new("stack underflow"))?;
        Ok((value_to_int(left)?, value_to_int(right)?))
    }
}

fn value_to_int(value: Value) -> Result<i64, RuntimeError> {
    match value {
        Value::Int(value) => Ok(value),
        Value::Bool(value) => Ok(if value { 1 } else { 0 }),
        _ => Err(RuntimeError::new("unsupported operand type")),
    }
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::None => false,
        Value::Bool(value) => *value,
        Value::Int(value) => *value != 0,
        Value::Str(value) => !value.is_empty(),
    }
}
