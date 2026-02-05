//! Bytecode virtual machine (minimal subset).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::bytecode::{CodeObject, Opcode};
use crate::compiler;
use crate::parser;
use crate::runtime::{
    format_value, BoundMethod, BuiltinFunction, ClassObject, ExceptionObject, FunctionObject,
    InstanceObject, ModuleObject, RuntimeError, Value,
};

#[derive(Debug, Clone)]
struct Block {
    handler: usize,
    stack_len: usize,
}

struct Frame {
    code: Rc<CodeObject>,
    ip: usize,
    stack: Vec<Value>,
    locals: HashMap<String, Value>,
    module: Rc<ModuleObject>,
    function_globals: Rc<ModuleObject>,
    globals_fallback: Option<Rc<ModuleObject>>,
    is_module: bool,
    return_module: bool,
    return_instance: Option<Rc<InstanceObject>>,
    return_class: bool,
    class_bases: Vec<Rc<ClassObject>>,
    blocks: Vec<Block>,
    active_exception: Option<Value>,
}

impl Frame {
    fn new(code: Rc<CodeObject>, module: Rc<ModuleObject>, is_module: bool, return_module: bool) -> Self {
        Self {
            code,
            ip: 0,
            stack: Vec::new(),
            locals: HashMap::new(),
            module: module.clone(),
            function_globals: module,
            globals_fallback: None,
            is_module,
            return_module,
            return_instance: None,
            return_class: false,
            class_bases: Vec::new(),
            blocks: Vec::new(),
            active_exception: None,
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
                let value = if frame.return_class {
                    self.class_value_from_module(&frame.module, frame.class_bases)
                } else if let Some(instance) = frame.return_instance {
                    Value::Instance(instance)
                } else if frame.return_module {
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

            let step_result = (|| -> Result<Option<Value>, RuntimeError> {
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
                        Value::Class(class) => {
                            let attr = class_attr_lookup(&class, &attr_name).ok_or_else(|| {
                                RuntimeError::new(format!(
                                    "class '{}' has no attribute '{}'",
                                    class.name, attr_name
                                ))
                            })?;
                            self.push_value(attr);
                        }
                        Value::Instance(instance) => {
                            if let Some(attr) = instance.attrs.borrow().get(&attr_name).cloned() {
                                self.push_value(attr);
                                return Ok(None);
                            }
                            if let Some(attr) = class_attr_lookup(&instance.class, &attr_name) {
                                if let Value::Function(func) = attr {
                                    let bound = BoundMethod::new(func, instance.clone());
                                    self.push_value(Value::BoundMethod(Rc::new(bound)));
                                } else {
                                    self.push_value(attr);
                                }
                                return Ok(None);
                            }
                            return Err(RuntimeError::new(format!(
                                "'{}' object has no attribute '{}'",
                                instance.class.name, attr_name
                            )));
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
                        Value::Instance(instance) => {
                            instance.attrs.borrow_mut().insert(attr_name, value);
                        }
                        Value::Class(class) => {
                            class.attrs.borrow_mut().insert(attr_name, value);
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
                Opcode::CompareIs => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(Value::Bool(left == right));
                }
                Opcode::CompareIsNot => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(Value::Bool(left != right));
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
                Opcode::UnaryPos => {
                    let value = self.pop_value()?;
                    match value {
                        Value::Int(value) => self.push_value(Value::Int(value)),
                        Value::Bool(value) => self.push_value(Value::Int(if value { 1 } else { 0 })),
                        _ => return Err(RuntimeError::new("unsupported operand type for +")),
                    }
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
                Opcode::ListAppend => {
                    let value = self.pop_value()?;
                    let list = self.pop_value()?;
                    match list {
                        Value::List(mut values) => {
                            values.push(value);
                            self.push_value(Value::List(values));
                        }
                        _ => return Err(RuntimeError::new("list append expects list")),
                    }
                }
                Opcode::ListExtend => {
                    let other = self.pop_value()?;
                    let list = self.pop_value()?;
                    match list {
                        Value::List(mut values) => {
                            match other {
                                Value::List(items) => values.extend(items),
                                Value::Tuple(items) => values.extend(items),
                                Value::Str(text) => {
                                    for ch in text.chars() {
                                        values.push(Value::Str(ch.to_string()));
                                    }
                                }
                                _ => return Err(RuntimeError::new("list extend expects iterable")),
                            }
                            self.push_value(Value::List(values));
                        }
                        _ => return Err(RuntimeError::new("list extend expects list")),
                    }
                }
                Opcode::DictSet => {
                    let value = self.pop_value()?;
                    let key = self.pop_value()?;
                    let dict = self.pop_value()?;
                    match dict {
                        Value::Dict(mut entries) => {
                            let mut found = false;
                            for (stored_key, stored_value) in entries.iter_mut() {
                                if *stored_key == key {
                                    *stored_value = value.clone();
                                    found = true;
                                    break;
                                }
                            }
                            if !found {
                                entries.push((key, value));
                            }
                            self.push_value(Value::Dict(entries));
                        }
                        _ => return Err(RuntimeError::new("dict set expects dict")),
                    }
                }
                Opcode::DictUpdate => {
                    let other = self.pop_value()?;
                    let dict = self.pop_value()?;
                    match (dict, other) {
                        (Value::Dict(mut entries), Value::Dict(other_entries)) => {
                            for (key, value) in other_entries {
                                let mut found = false;
                                for (stored_key, stored_value) in entries.iter_mut() {
                                    if *stored_key == key {
                                        *stored_value = value.clone();
                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    entries.push((key, value));
                                }
                            }
                            self.push_value(Value::Dict(entries));
                        }
                        _ => return Err(RuntimeError::new("dict update expects dict")),
                    }
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
                    let defaults_value = self.pop_value()?;
                    let defaults = match defaults_value {
                        Value::Tuple(values) => values,
                        _ => {
                            return Err(RuntimeError::new(
                                "expected defaults tuple for function",
                            ))
                        }
                    };
                    let module = self
                        .frames
                        .last()
                        .expect("frame exists")
                        .function_globals
                        .clone();
                    let func = FunctionObject::new(code, module, defaults);
                    self.push_value(Value::Function(Rc::new(func)));
                }
                Opcode::BuildClass => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing class code argument"))?
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
                                "expected code object for class body",
                            ))
                        }
                    };
                    let name_value = self.pop_value()?;
                    let bases_value = self.pop_value()?;
                    let class_name = match name_value {
                        Value::Str(name) => name,
                        _ => return Err(RuntimeError::new("class name must be a string")),
                    };
                    let bases = match bases_value {
                        Value::Tuple(values) => values,
                        _ => return Err(RuntimeError::new("class bases must be a tuple")),
                    };
                    let mut base_classes = Vec::new();
                    for base in bases {
                        match base {
                            Value::Class(class) => base_classes.push(class),
                            _ => {
                                return Err(RuntimeError::new(
                                    "class base must be a class object",
                                ))
                            }
                        }
                    }

                    let class_module = Rc::new(ModuleObject::new(class_name.clone()));
                    class_module.globals.borrow_mut().insert(
                        "__name__".to_string(),
                        Value::Str(class_name),
                    );

                    let outer_globals = self
                        .frames
                        .last()
                        .map(|frame| frame.module.clone())
                        .unwrap_or_else(|| self.main_module.clone());
                    let mut frame = Frame::new(code, class_module, true, false);
                    frame.function_globals = outer_globals.clone();
                    frame.globals_fallback = Some(outer_globals);
                    frame.return_class = true;
                    frame.class_bases = base_classes;
                    self.frames.push(frame);
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
                            let bindings = bind_arguments(&func, args, HashMap::new())?;
                            let mut frame =
                                Frame::new(func.code.clone(), func.module.clone(), false, false);
                            apply_bindings(&mut frame, &func.code, bindings);
                            self.frames.push(frame);
                        }
                        Value::BoundMethod(method) => {
                            let mut bound_args = Vec::with_capacity(args.len() + 1);
                            bound_args.push(Value::Instance(method.receiver.clone()));
                            bound_args.extend(args);
                            let bindings = bind_arguments(
                                &method.function,
                                bound_args,
                                HashMap::new(),
                            )?;
                            let mut frame = Frame::new(
                                method.function.code.clone(),
                                method.function.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &method.function.code, bindings);
                            self.frames.push(frame);
                        }
                        Value::Class(class) => {
                            let instance = Rc::new(InstanceObject::new(class.clone()));
                            let init = class_attr_lookup(&class, "__init__");
                            if let Some(Value::Function(init_func)) = init {
                                let mut init_args = Vec::with_capacity(args.len() + 1);
                                init_args.push(Value::Instance(instance.clone()));
                                init_args.extend(args);
                                let bindings =
                                    bind_arguments(&init_func, init_args, HashMap::new())?;
                                let mut frame = Frame::new(
                                    init_func.code.clone(),
                                    init_func.module.clone(),
                                    false,
                                    false,
                                );
                                frame.return_instance = Some(instance);
                                apply_bindings(&mut frame, &init_func.code, bindings);
                                self.frames.push(frame);
                            } else {
                                self.push_value(Value::Instance(instance));
                            }
                        }
                        Value::Builtin(builtin) => {
                            let result = builtin.call(args)?;
                            self.push_value(result);
                        }
                        Value::ExceptionType(name) => {
                            let message = match args.as_slice() {
                                [] => None,
                                [value] => Some(format_value(value)),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "exception constructor expects at most one argument",
                                    ))
                                }
                            };
                            self.push_value(Value::Exception(ExceptionObject { name, message }));
                        }
                        _ => return Err(RuntimeError::new("attempted to call non-function")),
                    }
                }
                Opcode::CallFunctionKw => {
                    let arg = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing call argument"))?;
                    let (pos_count, kw_count) = decode_call_counts(arg);
                    let mut kwargs = HashMap::new();
                    for _ in 0..kw_count {
                        let value = self.pop_value()?;
                        let name = self.pop_value()?;
                        let name = match name {
                            Value::Str(name) => name,
                            _ => return Err(RuntimeError::new("keyword name must be string")),
                        };
                        if kwargs.contains_key(&name) {
                            return Err(RuntimeError::new("duplicate keyword argument"));
                        }
                        kwargs.insert(name, value);
                    }
                    let mut args = Vec::with_capacity(pos_count);
                    for _ in 0..pos_count {
                        args.push(self.pop_value()?);
                    }
                    args.reverse();
                    let func = self.pop_value()?;
                    match func {
                        Value::Function(func) => {
                            let bindings = bind_arguments(&func, args, kwargs)?;
                            let mut frame =
                                Frame::new(func.code.clone(), func.module.clone(), false, false);
                            apply_bindings(&mut frame, &func.code, bindings);
                            self.frames.push(frame);
                        }
                        Value::BoundMethod(method) => {
                            let mut bound_args = Vec::with_capacity(args.len() + 1);
                            bound_args.push(Value::Instance(method.receiver.clone()));
                            bound_args.extend(args);
                            let bindings = bind_arguments(&method.function, bound_args, kwargs)?;
                            let mut frame = Frame::new(
                                method.function.code.clone(),
                                method.function.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &method.function.code, bindings);
                            self.frames.push(frame);
                        }
                        Value::Class(class) => {
                            let instance = Rc::new(InstanceObject::new(class.clone()));
                            let init = class_attr_lookup(&class, "__init__");
                            if let Some(Value::Function(init_func)) = init {
                                let mut init_args = Vec::with_capacity(args.len() + 1);
                                init_args.push(Value::Instance(instance.clone()));
                                init_args.extend(args);
                                let bindings = bind_arguments(&init_func, init_args, kwargs)?;
                                let mut frame = Frame::new(
                                    init_func.code.clone(),
                                    init_func.module.clone(),
                                    false,
                                    false,
                                );
                                frame.return_instance = Some(instance);
                                apply_bindings(&mut frame, &init_func.code, bindings);
                                self.frames.push(frame);
                            } else {
                                if !kwargs.is_empty() {
                                    return Err(RuntimeError::new(
                                        "unexpected keyword arguments",
                                    ));
                                }
                                self.push_value(Value::Instance(instance));
                            }
                        }
                        Value::Builtin(builtin) => {
                            if !kwargs.is_empty() {
                                return Err(RuntimeError::new(
                                    "keyword arguments not supported for builtin",
                                ));
                            }
                            let result = builtin.call(args)?;
                            self.push_value(result);
                        }
                        Value::ExceptionType(name) => {
                            if !kwargs.is_empty() {
                                return Err(RuntimeError::new(
                                    "keyword arguments not supported for exceptions",
                                ));
                            }
                            let message = match args.as_slice() {
                                [] => None,
                                [value] => Some(format_value(value)),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "exception constructor expects at most one argument",
                                    ))
                                }
                            };
                            self.push_value(Value::Exception(ExceptionObject { name, message }));
                        }
                        _ => return Err(RuntimeError::new("attempted to call non-function")),
                    }
                }
                Opcode::CallFunctionVar => {
                    let kwargs_value = self.pop_value()?;
                    let args_value = self.pop_value()?;
                    let func = self.pop_value()?;
                    let kwargs = match kwargs_value {
                        Value::Dict(entries) => {
                            let mut map = HashMap::new();
                            for (key, value) in entries {
                                let key = match key {
                                    Value::Str(name) => name,
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "keyword name must be string",
                                        ))
                                    }
                                };
                                if map.contains_key(&key) {
                                    return Err(RuntimeError::new("duplicate keyword argument"));
                                }
                                map.insert(key, value);
                            }
                            map
                        }
                        _ => return Err(RuntimeError::new("call kwargs must be dict")),
                    };
                    let args = match args_value {
                        Value::List(values) => values,
                        _ => return Err(RuntimeError::new("call args must be list")),
                    };

                    match func {
                        Value::Function(func) => {
                            let bindings = bind_arguments(&func, args, kwargs)?;
                            let mut frame =
                                Frame::new(func.code.clone(), func.module.clone(), false, false);
                            apply_bindings(&mut frame, &func.code, bindings);
                            self.frames.push(frame);
                        }
                        Value::BoundMethod(method) => {
                            let mut bound_args = Vec::with_capacity(args.len() + 1);
                            bound_args.push(Value::Instance(method.receiver.clone()));
                            bound_args.extend(args);
                            let bindings = bind_arguments(&method.function, bound_args, kwargs)?;
                            let mut frame = Frame::new(
                                method.function.code.clone(),
                                method.function.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &method.function.code, bindings);
                            self.frames.push(frame);
                        }
                        Value::Class(class) => {
                            let instance = Rc::new(InstanceObject::new(class.clone()));
                            let init = class_attr_lookup(&class, "__init__");
                            if let Some(Value::Function(init_func)) = init {
                                let mut init_args = Vec::with_capacity(args.len() + 1);
                                init_args.push(Value::Instance(instance.clone()));
                                init_args.extend(args);
                                let bindings = bind_arguments(&init_func, init_args, kwargs)?;
                                let mut frame = Frame::new(
                                    init_func.code.clone(),
                                    init_func.module.clone(),
                                    false,
                                    false,
                                );
                                frame.return_instance = Some(instance);
                                apply_bindings(&mut frame, &init_func.code, bindings);
                                self.frames.push(frame);
                            } else {
                                if !kwargs.is_empty() {
                                    return Err(RuntimeError::new(
                                        "unexpected keyword arguments",
                                    ));
                                }
                                self.push_value(Value::Instance(instance));
                            }
                        }
                        Value::Builtin(builtin) => {
                            if !kwargs.is_empty() {
                                return Err(RuntimeError::new(
                                    "keyword arguments not supported for builtin",
                                ));
                            }
                            let result = builtin.call(args)?;
                            self.push_value(result);
                        }
                        Value::ExceptionType(name) => {
                            if !kwargs.is_empty() {
                                return Err(RuntimeError::new(
                                    "keyword arguments not supported for exceptions",
                                ));
                            }
                            let message = match args.as_slice() {
                                [] => None,
                                [value] => Some(format_value(value)),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "exception constructor expects at most one argument",
                                    ))
                                }
                            };
                            self.push_value(Value::Exception(ExceptionObject { name, message }));
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
                Opcode::SetupExcept => {
                    let handler = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing handler target"))?
                        as usize;
                    let frame = self.frames.last_mut().expect("frame exists");
                    let stack_len = frame.stack.len();
                    frame.blocks.push(Block { handler, stack_len });
                }
                Opcode::PopBlock => {
                    let frame = self.frames.last_mut().expect("frame exists");
                    frame
                        .blocks
                        .pop()
                        .ok_or_else(|| RuntimeError::new("no block to pop"))?;
                }
                Opcode::Raise => {
                    let mode = instr.arg.unwrap_or(1);
                    let value = if mode == 0 {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .active_exception
                            .clone()
                            .ok_or_else(|| RuntimeError::new("no active exception to reraise"))?
                    } else {
                        self.pop_value()?
                    };
                    self.raise_exception(value)?;
                }
                Opcode::MatchException => {
                    let handler_type = self.pop_value()?;
                    let exception = self.pop_value()?;
                    let matches = exception_matches(&exception, &handler_type)?;
                    self.push_value(Value::Bool(matches));
                }
                Opcode::ClearException => {
                    if let Some(frame) = self.frames.last_mut() {
                        frame.active_exception = None;
                    }
                }
                Opcode::PopTop => {
                    let _ = self.pop_value()?;
                }
                Opcode::ReturnValue => {
                    let value = self.pop_value().unwrap_or(Value::None);
                    let frame = self.frames.pop().expect("frame exists");
                    let value = if frame.return_class {
                        self.class_value_from_module(&frame.module, frame.class_bases)
                    } else if let Some(instance) = frame.return_instance {
                        Value::Instance(instance)
                    } else if frame.return_module {
                        Value::Module(frame.module.clone())
                    } else {
                        value
                    };
                    if let Some(caller) = self.frames.last_mut() {
                        caller.stack.push(value);
                        return Ok(None);
                    }
                    return Ok(Some(value));
                }
                }
                Ok(None)
            })();

            match step_result {
                Ok(Some(value)) => return Ok(value),
                Ok(None) => {}
                Err(err) => {
                    if err.message.starts_with("unhandled exception:") {
                        return Err(err);
                    }
                    self.handle_runtime_error(err)?;
                }
            }
        }
    }

    fn raise_exception(&mut self, value: Value) -> Result<(), RuntimeError> {
        let exc = normalize_exception(value)?;
        loop {
            let Some(frame) = self.frames.last_mut() else {
                return Err(RuntimeError::new(format!(
                    "unhandled exception: {}",
                    format_value(&exc)
                )));
            };

            if let Some(block) = frame.blocks.pop() {
                frame.stack.truncate(block.stack_len);
                frame.stack.push(exc.clone());
                frame.ip = block.handler;
                frame.active_exception = Some(exc);
                return Ok(());
            }

            self.frames.pop();
        }
    }

    fn handle_runtime_error(&mut self, err: RuntimeError) -> Result<(), RuntimeError> {
        let exception_type = classify_runtime_error(&err.message);
        let exception = Value::Exception(ExceptionObject {
            name: exception_type.to_string(),
            message: Some(err.message),
        });
        self.raise_exception(exception)
    }

    fn class_value_from_module(
        &self,
        module: &ModuleObject,
        bases: Vec<Rc<ClassObject>>,
    ) -> Value {
        let class = Rc::new(ClassObject::new(module.name.clone(), bases));
        let attrs = module.globals.borrow().clone();
        class.attrs.borrow_mut().extend(attrs);
        Value::Class(class)
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
            if let Some(fallback) = &frame.globals_fallback {
                if let Some(value) = fallback.globals.borrow().get(name) {
                    return Ok(value.clone());
                }
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
        self.builtins
            .insert("bool".to_string(), Value::Builtin(BuiltinFunction::Bool));
        self.builtins
            .insert("int".to_string(), Value::Builtin(BuiltinFunction::Int));
        self.builtins
            .insert("str".to_string(), Value::Builtin(BuiltinFunction::Str));
        self.builtins
            .insert("abs".to_string(), Value::Builtin(BuiltinFunction::Abs));
        self.builtins
            .insert("sum".to_string(), Value::Builtin(BuiltinFunction::Sum));
        self.builtins
            .insert("min".to_string(), Value::Builtin(BuiltinFunction::Min));
        self.builtins
            .insert("max".to_string(), Value::Builtin(BuiltinFunction::Max));
        self.builtins
            .insert("all".to_string(), Value::Builtin(BuiltinFunction::All));
        self.builtins
            .insert("any".to_string(), Value::Builtin(BuiltinFunction::Any));
        self.builtins
            .insert("pow".to_string(), Value::Builtin(BuiltinFunction::Pow));
        self.builtins
            .insert("list".to_string(), Value::Builtin(BuiltinFunction::List));
        self.builtins
            .insert("tuple".to_string(), Value::Builtin(BuiltinFunction::Tuple));
        self.builtins
            .insert("divmod".to_string(), Value::Builtin(BuiltinFunction::DivMod));
        self.builtins
            .insert("sorted".to_string(), Value::Builtin(BuiltinFunction::Sorted));
        self.builtins
            .insert("Exception".to_string(), Value::ExceptionType("Exception".to_string()));
        self.builtins
            .insert("ValueError".to_string(), Value::ExceptionType("ValueError".to_string()));
        self.builtins
            .insert("TypeError".to_string(), Value::ExceptionType("TypeError".to_string()));
        self.builtins
            .insert("IndexError".to_string(), Value::ExceptionType("IndexError".to_string()));
        self.builtins
            .insert("KeyError".to_string(), Value::ExceptionType("KeyError".to_string()));
        self.builtins.insert(
            "AssertionError".to_string(),
            Value::ExceptionType("AssertionError".to_string()),
        );
        self.builtins
            .insert("NameError".to_string(), Value::ExceptionType("NameError".to_string()));
        self.builtins.insert(
            "AttributeError".to_string(),
            Value::ExceptionType("AttributeError".to_string()),
        );
        self.builtins.insert(
            "ZeroDivisionError".to_string(),
            Value::ExceptionType("ZeroDivisionError".to_string()),
        );
        self.builtins
            .insert("RuntimeError".to_string(), Value::ExceptionType("RuntimeError".to_string()));
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
        Value::Module(_)
        | Value::Class(_)
        | Value::Instance(_)
        | Value::BoundMethod(_)
        | Value::Exception(_)
        | Value::ExceptionType(_)
        | Value::Code(_)
        | Value::Function(_)
        | Value::Builtin(_) => true,
    }
}

fn normalize_exception(value: Value) -> Result<Value, RuntimeError> {
    match value {
        Value::Exception(_) => Ok(value),
        Value::ExceptionType(name) => Ok(Value::Exception(ExceptionObject { name, message: None })),
        _ => Err(RuntimeError::new("can only raise Exception types")),
    }
}

fn exception_matches(exception: &Value, handler_type: &Value) -> Result<bool, RuntimeError> {
    let exception_name = match exception {
        Value::Exception(exc) => exc.name.as_str(),
        _ => return Err(RuntimeError::new("expected exception instance")),
    };

    let handler_name = match handler_type {
        Value::ExceptionType(name) => name.as_str(),
        Value::Exception(exc) => exc.name.as_str(),
        _ => return Err(RuntimeError::new("except expects exception type")),
    };

    if handler_name == "Exception" {
        return Ok(true);
    }

    Ok(exception_name == handler_name)
}

struct BoundArguments {
    positional: Vec<Value>,
    vararg: Option<Value>,
    kwarg: Option<Value>,
}

fn bind_arguments(
    func: &FunctionObject,
    mut positional: Vec<Value>,
    mut kwargs: HashMap<String, Value>,
) -> Result<BoundArguments, RuntimeError> {
    let params_len = func.code.params.len();
    let defaults_len = func.defaults.len();
    if defaults_len > params_len {
        return Err(RuntimeError::new("invalid function defaults"));
    }

    let mut extra_positional = Vec::new();
    if positional.len() > params_len {
        if func.code.vararg.is_none() {
            return Err(RuntimeError::new("argument count mismatch"));
        }
        extra_positional = positional.split_off(params_len);
    }

    let required = params_len - defaults_len;
    let mut bound: Vec<Option<Value>> = vec![None; params_len];

    for (idx, value) in positional.into_iter().enumerate() {
        bound[idx] = Some(value);
    }

    let mut extra_kwargs: HashMap<String, Value> = HashMap::new();
    for (name, value) in kwargs.drain() {
        if let Some(index) = func.code.params.iter().position(|param| param == &name) {
            if bound[index].is_some() {
                return Err(RuntimeError::new("multiple values for argument"));
            }
            bound[index] = Some(value);
        } else if func.code.kwarg.is_some() {
            if extra_kwargs.contains_key(&name) {
                return Err(RuntimeError::new("duplicate keyword argument"));
            }
            extra_kwargs.insert(name, value);
        } else {
            return Err(RuntimeError::new("unexpected keyword argument"));
        }
    }

    for idx in 0..params_len {
        if bound[idx].is_none() {
            if idx < required {
                return Err(RuntimeError::new("argument count mismatch"));
            }
            let default_index = idx - required;
            bound[idx] = Some(func.defaults[default_index].clone());
        }
    }

    let positional = bound.into_iter().map(|value| value.unwrap()).collect();
    let vararg = func
        .code
        .vararg
        .as_ref()
        .map(|_| Value::List(extra_positional));
    let kwarg = func.code.kwarg.as_ref().map(|_| {
        let mut entries = Vec::with_capacity(extra_kwargs.len());
        for (key, value) in extra_kwargs {
            entries.push((Value::Str(key), value));
        }
        Value::Dict(entries)
    });

    Ok(BoundArguments {
        positional,
        vararg,
        kwarg,
    })
}

fn apply_bindings(frame: &mut Frame, code: &CodeObject, bindings: BoundArguments) {
    for (name, value) in code
        .params
        .iter()
        .cloned()
        .zip(bindings.positional.into_iter())
    {
        frame.locals.insert(name, value);
    }

    if let Some(name) = code.vararg.as_ref() {
        let value = bindings.vararg.unwrap_or_else(|| Value::List(Vec::new()));
        frame.locals.insert(name.clone(), value);
    }

    if let Some(name) = code.kwarg.as_ref() {
        let value = bindings.kwarg.unwrap_or_else(|| Value::Dict(Vec::new()));
        frame.locals.insert(name.clone(), value);
    }
}

fn decode_call_counts(arg: u32) -> (usize, usize) {
    let pos = (arg & 0xFFFF) as usize;
    let kw = (arg >> 16) as usize;
    (pos, kw)
}

fn class_attr_lookup(class: &Rc<ClassObject>, name: &str) -> Option<Value> {
    if let Some(value) = class.attrs.borrow().get(name).cloned() {
        return Some(value);
    }
    for base in &class.bases {
        if let Some(value) = class_attr_lookup(base, name) {
            return Some(value);
        }
    }
    None
}

fn classify_runtime_error(message: &str) -> &'static str {
    if message.contains("index out of range") {
        return "IndexError";
    }
    if message.contains("key not found") {
        return "KeyError";
    }
    if message.contains("division by zero") || message.contains("modulo by zero") {
        return "ZeroDivisionError";
    }
    if message.starts_with("name '") && message.ends_with("is not defined") {
        return "NameError";
    }
    if message.contains("has no attribute") {
        return "AttributeError";
    }
    if message.contains("unsupported operand type") || message.contains("expects") {
        return "TypeError";
    }
    "RuntimeError"
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
