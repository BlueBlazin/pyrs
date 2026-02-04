//! Bytecode virtual machine (minimal subset).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::bytecode::{CodeObject, Opcode};
use crate::compiler;
use crate::parser;
use crate::runtime::{BuiltinFunction, FunctionObject, ModuleObject, RuntimeError, Value};

struct Frame {
    code: Rc<CodeObject>,
    ip: usize,
    stack: Vec<Value>,
    locals: HashMap<String, Value>,
    module: Rc<ModuleObject>,
    is_module: bool,
    return_module: bool,
}

impl Frame {
    fn new(code: Rc<CodeObject>, module: Rc<ModuleObject>, is_module: bool, return_module: bool) -> Self {
        Self {
            code,
            ip: 0,
            stack: Vec::new(),
            locals: HashMap::new(),
            module,
            is_module,
            return_module,
        }
    }
}

pub struct Vm {
    frames: Vec<Frame>,
    builtins: HashMap<String, Value>,
    modules: HashMap<String, Rc<ModuleObject>>,
    main_module: Rc<ModuleObject>,
    module_paths: Vec<PathBuf>,
}

impl Vm {
    pub fn new() -> Self {
        let main_module = Rc::new(ModuleObject::new("__main__"));
        main_module.globals.borrow_mut().insert(
            "__name__".to_string(),
            Value::Str("__main__".to_string()),
        );

        let mut modules = HashMap::new();
        modules.insert("__main__".to_string(), main_module.clone());

        let module_paths =
            vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))];

        let mut vm = Self {
            frames: Vec::new(),
            builtins: HashMap::new(),
            modules,
            main_module,
            module_paths,
        };
        vm.install_builtins();
        vm
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: Value) {
        self.main_module
            .globals
            .borrow_mut()
            .insert(name.into(), value);
    }

    pub fn get_global(&self, name: &str) -> Option<Value> {
        let globals = self.main_module.globals.borrow();
        globals.get(name).cloned()
    }

    pub fn add_module_path(&mut self, path: impl Into<PathBuf>) {
        self.module_paths.push(path.into());
    }

    pub fn execute(&mut self, code: &CodeObject) -> Result<Value, RuntimeError> {
        self.frames.clear();
        let code = Rc::new(code.clone());
        self.frames.push(Frame::new(
            code,
            self.main_module.clone(),
            true,
            false,
        ));
        self.run()
    }

    fn load_module(&mut self, name: &str) -> Result<Rc<ModuleObject>, RuntimeError> {
        if let Some(module) = self.modules.get(name).cloned() {
            return Ok(module);
        }

        let path = self
            .find_module_file(name)
            .ok_or_else(|| RuntimeError::new(format!("module '{name}' not found")))?;

        let source = std::fs::read_to_string(&path).map_err(|err| {
            RuntimeError::new(format!("failed to read module '{name}': {err}"))
        })?;

        let module = Rc::new(ModuleObject::new(name));
        {
            let mut globals = module.globals.borrow_mut();
            globals.insert("__name__".to_string(), Value::Str(name.to_string()));
            globals.insert(
                "__file__".to_string(),
                Value::Str(path.to_string_lossy().to_string()),
            );
        }

        self.modules.insert(name.to_string(), module.clone());

        let module_ast = parser::parse_module(&source).map_err(|err| {
            RuntimeError::new(format!(
                "parse error in module '{name}' at {}: {}",
                err.offset, err.message
            ))
        })?;
        let code = compiler::compile_module(&module_ast).map_err(|err| {
            RuntimeError::new(format!("compile error in module '{name}': {}", err.message))
        })?;
        let frame = Frame::new(Rc::new(code), module.clone(), true, true);
        self.frames.push(frame);
        Ok(module)
    }

    fn find_module_file(&self, name: &str) -> Option<PathBuf> {
        let filename = format!("{name}.py");
        for base in &self.module_paths {
            let candidate = base.join(&filename);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }

    fn run(&mut self) -> Result<Value, RuntimeError> {
        loop {
            if self.frames.is_empty() {
                return Ok(Value::None);
            }

            let should_return = {
                let frame = self.frames.last().expect("frame exists");
                frame.ip >= frame.code.instructions.len()
            };

            if should_return {
                let frame = self.frames.pop().expect("frame exists");
                let value = if frame.return_module {
                    Value::Module(frame.module.clone())
                } else {
                    Value::None
                };
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
                Opcode::LoadAttr => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing attribute argument"))?
                        as usize;
                    let attr_name = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .code
                            .names
                            .get(idx)
                            .ok_or_else(|| RuntimeError::new("name index out of range"))?
                            .clone()
                    };
                    let value = self.pop_value()?;
                    match value {
                        Value::Module(module) => {
                            let globals = module.globals.borrow();
                            let attr = globals.get(&attr_name).cloned().ok_or_else(|| {
                                RuntimeError::new(format!(
                                    "module '{}' has no attribute '{}'",
                                    module.name, attr_name
                                ))
                            })?;
                            self.push_value(attr);
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "attribute access unsupported type",
                            ))
                        }
                    }
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
                Opcode::StoreAttr => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing attribute argument"))?
                        as usize;
                    let attr_name = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .code
                            .names
                            .get(idx)
                            .ok_or_else(|| RuntimeError::new("name index out of range"))?
                            .clone()
                    };
                    let value = self.pop_value()?;
                    let target = self.pop_value()?;
                    match target {
                        Value::Module(module) => {
                            module.globals.borrow_mut().insert(attr_name, value);
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "attribute assignment unsupported type",
                            ))
                        }
                    }
                }
                Opcode::StoreGlobal => {
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
                    let value = self.pop_value()?;
                    if let Some(frame) = self.frames.last() {
                        frame.module.globals.borrow_mut().insert(name, value);
                    }
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
                Opcode::BinaryFloorDiv => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    let (left, right) = (value_to_int(left)?, value_to_int(right)?);
                    if right == 0 {
                        return Err(RuntimeError::new("integer division by zero"));
                    }
                    self.push_value(Value::Int(left.div_euclid(right)));
                }
                Opcode::BinaryMod => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    let (left, right) = (value_to_int(left)?, value_to_int(right)?);
                    if right == 0 {
                        return Err(RuntimeError::new("modulo by zero"));
                    }
                    self.push_value(Value::Int(left % right));
                }
                Opcode::CompareEq => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(Value::Bool(left == right));
                }
                Opcode::CompareNe => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(Value::Bool(left != right));
                }
                Opcode::CompareLt => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(compare_lt(left, right)?);
                }
                Opcode::CompareLe => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(compare_le(left, right)?);
                }
                Opcode::CompareGt => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(compare_gt(left, right)?);
                }
                Opcode::CompareGe => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(compare_ge(left, right)?);
                }
                Opcode::CompareIn => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(Value::Bool(compare_in(&left, &right)?));
                }
                Opcode::CompareNotIn => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(Value::Bool(!compare_in(&left, &right)?));
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
                Opcode::BuildSlice => {
                    let step = self.pop_value()?;
                    let upper = self.pop_value()?;
                    let lower = self.pop_value()?;
                    let lower = value_to_optional_index(lower)?;
                    let upper = value_to_optional_index(upper)?;
                    let step = value_to_optional_index(step)?;
                    self.push_value(Value::Slice { lower, upper, step });
                }
                Opcode::Subscript => {
                    let index = self.pop_value()?;
                    let value = self.pop_value()?;
                    match index {
                        Value::Slice { lower, upper, step } => match value {
                            Value::List(values) => {
                                let indices = slice_indices(values.len(), lower, upper, step)?;
                                let mut result = Vec::with_capacity(indices.len());
                                for idx in indices {
                                    result.push(values[idx].clone());
                                }
                                self.push_value(Value::List(result));
                            }
                            Value::Tuple(values) => {
                                let indices = slice_indices(values.len(), lower, upper, step)?;
                                let mut result = Vec::with_capacity(indices.len());
                                for idx in indices {
                                    result.push(values[idx].clone());
                                }
                                self.push_value(Value::Tuple(result));
                            }
                            Value::Str(value) => {
                                let chars: Vec<char> = value.chars().collect();
                                let indices = slice_indices(chars.len(), lower, upper, step)?;
                                let mut result = String::new();
                                for idx in indices {
                                    result.push(chars[idx]);
                                }
                                self.push_value(Value::Str(result));
                            }
                            Value::Dict(_) => {
                                return Err(RuntimeError::new("slicing unsupported for dict"));
                            }
                            _ => return Err(RuntimeError::new("subscript unsupported type")),
                        },
                        index => match value {
                            Value::List(values) => {
                                let mut index_int = value_to_int(index)? as isize;
                                if index_int < 0 {
                                    index_int += values.len() as isize;
                                }
                                if index_int < 0 || index_int as usize >= values.len() {
                                    return Err(RuntimeError::new("list index out of range"));
                                }
                                self.push_value(values[index_int as usize].clone());
                            }
                            Value::Tuple(values) => {
                                let mut index_int = value_to_int(index)? as isize;
                                if index_int < 0 {
                                    index_int += values.len() as isize;
                                }
                                if index_int < 0 || index_int as usize >= values.len() {
                                    return Err(RuntimeError::new("tuple index out of range"));
                                }
                                self.push_value(values[index_int as usize].clone());
                            }
                            Value::Str(value) => {
                                let mut index_int = value_to_int(index)? as isize;
                                let chars: Vec<char> = value.chars().collect();
                                if index_int < 0 {
                                    index_int += chars.len() as isize;
                                }
                                if index_int < 0 || index_int as usize >= chars.len() {
                                    return Err(RuntimeError::new("string index out of range"));
                                }
                                self.push_value(Value::Str(chars[index_int as usize].to_string()));
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
                        },
                    }
                }
                Opcode::StoreSubscript => {
                    let value = self.pop_value()?;
                    let index = self.pop_value()?;
                    let target = self.pop_value()?;
                    match index {
                        Value::Slice { .. } => {
                            return Err(RuntimeError::new("slice assignment not supported"))
                        }
                        index => match target {
                            Value::List(mut values) => {
                                let mut idx = value_to_int(index)? as isize;
                                if idx < 0 {
                                    idx += values.len() as isize;
                                }
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
                        },
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
                    let module = self
                        .frames
                        .last()
                        .expect("frame exists")
                        .module
                        .clone();
                    let func = FunctionObject::new(code, module);
                    self.push_value(Value::Function(Rc::new(func)));
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
                        Value::Function(func) => {
                            if func.code.params.len() != args.len() {
                                return Err(RuntimeError::new("argument count mismatch"));
                            }

                            let params = func.code.params.clone();
                            let mut frame =
                                Frame::new(func.code.clone(), func.module.clone(), false, false);
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
                Opcode::ImportName => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing import argument"))?
                        as usize;
                    let name_value = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .code
                            .constants
                            .get(idx)
                            .cloned()
                            .ok_or_else(|| RuntimeError::new("constant index out of range"))?
                    };
                    let name = match name_value {
                        Value::Str(name) => name,
                        _ => return Err(RuntimeError::new("import expects string name")),
                    };
                    if let Some(module) = self.modules.get(&name).cloned() {
                        self.push_value(Value::Module(module));
                    } else {
                        self.load_module(&name)?;
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
                    let frame = self.frames.pop().expect("frame exists");
                    let value = if frame.return_module {
                        Value::Module(frame.module.clone())
                    } else {
                        value
                    };
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
            if let Some(value) = frame.module.globals.borrow().get(name) {
                return Ok(value.clone());
            }
        }
        self.builtins
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("name '{name}' is not defined")))
    }

    fn store_name(&mut self, name: String, value: Value) {
        if let Some(frame) = self.frames.last_mut() {
            if frame.is_module {
                frame.module.globals.borrow_mut().insert(name, value);
            } else {
                frame.locals.insert(name, value);
            }
        }
    }

    fn install_builtins(&mut self) {
        self.builtins
            .insert("print".to_string(), Value::Builtin(BuiltinFunction::Print));
        self.builtins
            .insert("len".to_string(), Value::Builtin(BuiltinFunction::Len));
        self.builtins
            .insert("range".to_string(), Value::Builtin(BuiltinFunction::Range));
        self.builtins
            .insert("slice".to_string(), Value::Builtin(BuiltinFunction::Slice));
    }
}

fn value_to_int(value: Value) -> Result<i64, RuntimeError> {
    match value {
        Value::Int(value) => Ok(value),
        Value::Bool(value) => Ok(if value { 1 } else { 0 }),
        _ => Err(RuntimeError::new("unsupported operand type")),
    }
}

fn value_to_optional_index(value: Value) -> Result<Option<i64>, RuntimeError> {
    match value {
        Value::None => Ok(None),
        other => Ok(Some(value_to_int(other)?)),
    }
}

fn numeric_pair(left: &Value, right: &Value) -> Option<(i64, i64)> {
    let left = match left {
        Value::Int(value) => *value,
        Value::Bool(value) => {
            if *value {
                1
            } else {
                0
            }
        }
        _ => return None,
    };

    let right = match right {
        Value::Int(value) => *value,
        Value::Bool(value) => {
            if *value {
                1
            } else {
                0
            }
        }
        _ => return None,
    };

    Some((left, right))
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
        Value::Slice { .. } => true,
        Value::Module(_) | Value::Code(_) | Value::Function(_) | Value::Builtin(_) => true,
    }
}

fn add_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return Ok(Value::Int(left + right));
    }

    match (left, right) {
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

fn compare_order(left: Value, right: Value) -> Result<Ordering, RuntimeError> {
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return Ok(left.cmp(&right));
    }

    match (left, right) {
        (Value::Str(a), Value::Str(b)) => Ok(a.cmp(&b)),
        _ => Err(RuntimeError::new("unsupported operand type for comparison")),
    }
}

fn compare_lt(left: Value, right: Value) -> Result<Value, RuntimeError> {
    Ok(Value::Bool(
        compare_order(left, right)? == Ordering::Less,
    ))
}

fn compare_le(left: Value, right: Value) -> Result<Value, RuntimeError> {
    Ok(Value::Bool(
        compare_order(left, right)? != Ordering::Greater,
    ))
}

fn compare_gt(left: Value, right: Value) -> Result<Value, RuntimeError> {
    Ok(Value::Bool(
        compare_order(left, right)? == Ordering::Greater,
    ))
}

fn compare_ge(left: Value, right: Value) -> Result<Value, RuntimeError> {
    Ok(Value::Bool(
        compare_order(left, right)? != Ordering::Less,
    ))
}

fn compare_in(left: &Value, right: &Value) -> Result<bool, RuntimeError> {
    match right {
        Value::List(values) => Ok(values.iter().any(|value| value == left)),
        Value::Tuple(values) => Ok(values.iter().any(|value| value == left)),
        Value::Dict(entries) => Ok(entries.iter().any(|(key, _)| key == left)),
        Value::Str(haystack) => match left {
            Value::Str(needle) => Ok(haystack.contains(needle)),
            _ => Err(RuntimeError::new("in expects string on left")),
        },
        _ => Err(RuntimeError::new("unsupported operand type for in")),
    }
}

fn mul_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return Ok(Value::Int(left * right));
    }

    match (left, right) {
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

fn slice_indices(
    len: usize,
    lower: Option<i64>,
    upper: Option<i64>,
    step: Option<i64>,
) -> Result<Vec<usize>, RuntimeError> {
    let len_isize = len as isize;
    let step = step.unwrap_or(1);
    if step == 0 {
        return Err(RuntimeError::new("slice step cannot be zero"));
    }
    let step = step as isize;

    let (start, stop) = if step > 0 {
        let mut start = lower.unwrap_or(0) as isize;
        if start < 0 {
            start += len_isize;
        }
        if start < 0 {
            start = 0;
        } else if start > len_isize {
            start = len_isize;
        }

        let mut stop = upper.unwrap_or(len as i64) as isize;
        if stop < 0 {
            stop += len_isize;
        }
        if stop < 0 {
            stop = 0;
        } else if stop > len_isize {
            stop = len_isize;
        }
        (start, stop)
    } else {
        let mut start = lower.unwrap_or(len as i64 - 1) as isize;
        if start < 0 {
            start += len_isize;
        }
        if start < -1 {
            start = -1;
        } else if start >= len_isize {
            start = len_isize - 1;
        }

        let mut stop = upper.unwrap_or(-1) as isize;
        if upper.is_some() && stop < 0 {
            stop += len_isize;
        }
        if stop < -1 {
            stop = -1;
        } else if stop >= len_isize {
            stop = len_isize - 1;
        }
        (start, stop)
    };

    let mut indices = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < stop {
            if i >= 0 && i < len_isize {
                indices.push(i as usize);
            }
            i += step;
        }
    } else {
        while i > stop {
            if i >= 0 && i < len_isize {
                indices.push(i as usize);
            }
            i += step;
        }
    }
    Ok(indices)
}
