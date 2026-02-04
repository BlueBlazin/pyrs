//! Bytecode virtual machine (minimal subset).

use std::collections::HashMap;
use std::rc::Rc;

use crate::bytecode::{CodeObject, Opcode};
use crate::runtime::{BuiltinFunction, RuntimeError, Value};

struct Frame {
    code: Rc<CodeObject>,
    ip: usize,
    stack: Vec<Value>,
    locals: HashMap<String, Value>,
    is_module: bool,
}

impl Frame {
    fn new(code: Rc<CodeObject>, is_module: bool) -> Self {
        Self {
            code,
            ip: 0,
            stack: Vec::new(),
            locals: HashMap::new(),
            is_module,
        }
    }
}

#[derive(Default)]
pub struct Vm {
    frames: Vec<Frame>,
    globals: HashMap<String, Value>,
}

impl Vm {
    pub fn new() -> Self {
        let mut vm = Self {
            frames: Vec::new(),
            globals: HashMap::new(),
        };
        vm.install_builtins();
        vm
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: Value) {
        self.globals.insert(name.into(), value);
    }

    pub fn get_global(&self, name: &str) -> Option<&Value> {
        self.globals.get(name)
    }

    pub fn execute(&mut self, code: &CodeObject) -> Result<Value, RuntimeError> {
        self.frames.clear();
        let code = Rc::new(code.clone());
        self.frames.push(Frame::new(code, true));
        self.run()
    }

    fn run(&mut self) -> Result<Value, RuntimeError> {
        loop {
            if self.frames.is_empty() {
                return Ok(Value::None);
            }

            let should_return_none = {
                let frame = self.frames.last_mut().expect("frame exists");
                if frame.ip >= frame.code.instructions.len() {
                    true
                } else {
                    false
                }
            };

            if should_return_none {
                let value = Value::None;
                self.frames.pop();
                if let Some(caller) = self.frames.last_mut() {
                    caller.stack.push(value);
                    continue;
                }
                return Ok(value);
            }

            let instr = {
                let frame = self.frames.last_mut().expect("frame exists");
                let instr = frame.code.instructions[frame.ip].clone();
                frame.ip += 1;
                instr
            };

            match instr.opcode {
                Opcode::Nop => {}
                Opcode::LoadConst => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing const argument"))?
                        as usize;
                    let value = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .code
                            .constants
                            .get(idx)
                            .cloned()
                            .ok_or_else(|| RuntimeError::new("constant index out of range"))?
                    };
                    self.frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(value);
                }
                Opcode::LoadName => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing name argument"))?
                        as usize;
                    let name = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .code
                            .names
                            .get(idx)
                            .ok_or_else(|| RuntimeError::new("name index out of range"))?
                            .clone()
                    };
                    let value = self.lookup_name(&name)?;
                    self.frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(value);
                }
                Opcode::StoreName => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing name argument"))?
                        as usize;
                    let name = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .code
                            .names
                            .get(idx)
                            .ok_or_else(|| RuntimeError::new("name index out of range"))?
                            .clone()
                    };
                    let value = {
                        let frame = self.frames.last_mut().expect("frame exists");
                        frame
                            .stack
                            .pop()
                            .ok_or_else(|| RuntimeError::new("stack underflow"))?
                    };
                    self.store_name(name, value);
                }
                Opcode::BinaryAdd => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(add_values(left, right)?);
                }
                Opcode::BinarySub => {
                    let (left, right) = self.pop_int_pair()?;
                    self.push_value(Value::Int(left - right));
                }
                Opcode::BinaryMul => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(mul_values(left, right)?);
                }
                Opcode::CompareEq => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(Value::Bool(left == right));
                }
                Opcode::CompareLt => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(compare_lt(left, right)?);
                }
                Opcode::UnaryNeg => {
                    let value = self.pop_value()?;
                    let value = value_to_int(value)?;
                    self.push_value(Value::Int(-value));
                }
                Opcode::UnaryNot => {
                    let value = self.pop_value()?;
                    self.push_value(Value::Bool(!is_truthy(&value)));
                }
                Opcode::BuildList => {
                    let count = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing list size"))?
                        as usize;
                    let mut values = Vec::with_capacity(count);
                    for _ in 0..count {
                        values.push(self.pop_value()?);
                    }
                    values.reverse();
                    self.push_value(Value::List(values));
                }
                Opcode::BuildTuple => {
                    let count = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing tuple size"))?
                        as usize;
                    let mut values = Vec::with_capacity(count);
                    for _ in 0..count {
                        values.push(self.pop_value()?);
                    }
                    values.reverse();
                    self.push_value(Value::Tuple(values));
                }
                Opcode::BuildDict => {
                    let count = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing dict size"))?
                        as usize;
                    let mut values = Vec::with_capacity(count);
                    for _ in 0..count {
                        let value = self.pop_value()?;
                        let key = self.pop_value()?;
                        values.push((key, value));
                    }
                    values.reverse();
                    self.push_value(Value::Dict(values));
                }
                Opcode::Subscript => {
                    let index = self.pop_value()?;
                    let value = self.pop_value()?;
                    match value {
                        Value::List(values) => {
                            let index_int = value_to_int(index)? as isize;
                            if index_int < 0 || index_int as usize >= values.len() {
                                return Err(RuntimeError::new("list index out of range"));
                            }
                            self.push_value(values[index_int as usize].clone());
                        }
                        Value::Tuple(values) => {
                            let index_int = value_to_int(index)? as isize;
                            if index_int < 0 || index_int as usize >= values.len() {
                                return Err(RuntimeError::new("tuple index out of range"));
                            }
                            self.push_value(values[index_int as usize].clone());
                        }
                        Value::Dict(entries) => {
                            let mut found = None;
                            for (key, value) in entries {
                                if key == index {
                                    found = Some(value);
                                    break;
                                }
                            }
                            if let Some(value) = found {
                                self.push_value(value);
                            } else {
                                return Err(RuntimeError::new("key not found"));
                            }
                        }
                        _ => return Err(RuntimeError::new("subscript unsupported type")),
                    }
                }
                Opcode::StoreSubscript => {
                    let value = self.pop_value()?;
                    let index = self.pop_value()?;
                    let target = self.pop_value()?;
                    match target {
                        Value::List(mut values) => {
                            let idx = value_to_int(index)? as isize;
                            if idx < 0 || idx as usize >= values.len() {
                                return Err(RuntimeError::new("list index out of range"));
                            }
                            values[idx as usize] = value;
                            self.push_value(Value::List(values));
                        }
                        Value::Dict(mut entries) => {
                            let mut found = false;
                            for (key, stored) in entries.iter_mut() {
                                if *key == index {
                                    *stored = value.clone();
                                    found = true;
                                    break;
                                }
                            }
                            if !found {
                                entries.push((index, value));
                            }
                            self.push_value(Value::Dict(entries));
                        }
                        _ => return Err(RuntimeError::new("store subscript unsupported type")),
                    }
                }
                Opcode::MakeFunction => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing function argument"))?
                        as usize;
                    let value = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .code
                            .constants
                            .get(idx)
                            .cloned()
                            .ok_or_else(|| RuntimeError::new("constant index out of range"))?
                    };
                    let code = match value {
                        Value::Code(code) => code,
                        _ => {
                            return Err(RuntimeError::new(
                                "expected code object for function",
                            ))
                        }
                    };
                    self.push_value(Value::Function(code));
                }
                Opcode::CallFunction => {
                    let argc = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing call argument"))?
                        as usize;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(self.pop_value()?);
                    }
                    args.reverse();
                    let func = self.pop_value()?;
                    match func {
                        Value::Function(code) => {
                            if code.params.len() != args.len() {
                                return Err(RuntimeError::new("argument count mismatch"));
                            }

                            let params = code.params.clone();
                            let mut frame = Frame::new(code, false);
                            for (name, value) in params.into_iter().zip(args.into_iter()) {
                                frame.locals.insert(name, value);
                            }
                            self.frames.push(frame);
                        }
                        Value::Builtin(builtin) => {
                            let result = builtin.call(args)?;
                            self.push_value(result);
                        }
                        _ => return Err(RuntimeError::new("attempted to call non-function")),
                    }
                }
                Opcode::JumpIfFalse => {
                    let target = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing jump target"))?
                        as usize;
                    let value = self.pop_value()?;
                    if !is_truthy(&value) {
                        let frame = self.frames.last_mut().expect("frame exists");
                        frame.ip = target;
                    }
                }
                Opcode::JumpIfTrue => {
                    let target = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing jump target"))?
                        as usize;
                    let value = self.pop_value()?;
                    if is_truthy(&value) {
                        let frame = self.frames.last_mut().expect("frame exists");
                        frame.ip = target;
                    }
                }
                Opcode::DupTop => {
                    let value = self
                        .frames
                        .last()
                        .and_then(|frame| frame.stack.last())
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("stack underflow"))?;
                    self.push_value(value);
                }
                Opcode::Jump => {
                    let target = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing jump target"))?
                        as usize;
                    let frame = self.frames.last_mut().expect("frame exists");
                    frame.ip = target;
                }
                Opcode::PopTop => {
                    let _ = self.pop_value()?;
                }
                Opcode::ReturnValue => {
                    let value = self.pop_value().unwrap_or(Value::None);
                    self.frames.pop();
                    if let Some(caller) = self.frames.last_mut() {
                        caller.stack.push(value);
                        continue;
                    }
                    return Ok(value);
                }
            }
        }
    }

    fn pop_value(&mut self) -> Result<Value, RuntimeError> {
        let frame = self.frames.last_mut().expect("frame exists");
        frame
            .stack
            .pop()
            .ok_or_else(|| RuntimeError::new("stack underflow"))
    }

    fn push_value(&mut self, value: Value) {
        let frame = self.frames.last_mut().expect("frame exists");
        frame.stack.push(value);
    }

    fn pop_int_pair(&mut self) -> Result<(i64, i64), RuntimeError> {
        let right = self.pop_value()?;
        let left = self.pop_value()?;
        Ok((value_to_int(left)?, value_to_int(right)?))
    }

    fn lookup_name(&self, name: &str) -> Result<Value, RuntimeError> {
        if let Some(frame) = self.frames.last() {
            if let Some(value) = frame.locals.get(name) {
                return Ok(value.clone());
            }
        }
        self.globals
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("name '{name}' is not defined")))
    }

    fn store_name(&mut self, name: String, value: Value) {
        if let Some(frame) = self.frames.last_mut() {
            if frame.is_module {
                self.globals.insert(name, value);
            } else {
                frame.locals.insert(name, value);
            }
        }
    }

    fn install_builtins(&mut self) {
        self.globals
            .insert("print".to_string(), Value::Builtin(BuiltinFunction::Print));
        self.globals
            .insert("len".to_string(), Value::Builtin(BuiltinFunction::Len));
        self.globals
            .insert("range".to_string(), Value::Builtin(BuiltinFunction::Range));
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
        Value::List(values) => !values.is_empty(),
        Value::Tuple(values) => !values.is_empty(),
        Value::Dict(values) => !values.is_empty(),
        Value::Code(_) | Value::Function(_) | Value::Builtin(_) => true,
    }
}

fn add_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
        (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
        (Value::List(mut a), Value::List(b)) => {
            a.extend(b);
            Ok(Value::List(a))
        }
        (Value::Tuple(mut a), Value::Tuple(b)) => {
            a.extend(b);
            Ok(Value::Tuple(a))
        }
        _ => Err(RuntimeError::new("unsupported operand type for +")),
    }
}

fn compare_lt(left: Value, right: Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
        (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a < b)),
        (Value::Str(a), Value::Str(b)) => Ok(Value::Bool(a < b)),
        _ => Err(RuntimeError::new("unsupported operand type for <")),
    }
}

fn mul_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
        (Value::Bool(a), Value::Bool(b)) => Ok(Value::Int((a as i64) * (b as i64))),
        (Value::Str(s), other) | (other, Value::Str(s)) => {
            let count = value_to_int(other)?;
            if count <= 0 {
                return Ok(Value::Str(String::new()));
            }
            Ok(Value::Str(s.repeat(count as usize)))
        }
        (Value::List(values), other) | (other, Value::List(values)) => {
            let count = value_to_int(other)?;
            if count <= 0 {
                return Ok(Value::List(Vec::new()));
            }
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(Value::List(result))
        }
        (Value::Tuple(values), other) | (other, Value::Tuple(values)) => {
            let count = value_to_int(other)?;
            if count <= 0 {
                return Ok(Value::Tuple(Vec::new()));
            }
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(Value::Tuple(result))
        }
        _ => Err(RuntimeError::new("unsupported operand type for *")),
    }
}
