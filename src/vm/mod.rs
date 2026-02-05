//! Bytecode virtual machine (minimal subset).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::bytecode::{CodeObject, Opcode};
use crate::bytecode::cpython;
use crate::compiler;
use crate::parser;
use crate::runtime::{
    format_value, BoundMethod, BuiltinFunction, ClassObject, ExceptionObject, FunctionObject,
    Heap, InstanceObject, IteratorKind, IteratorObject, ModuleObject, Object, ObjRef,
    RuntimeError, Value,
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
    module: ObjRef,
    function_globals: ObjRef,
    globals_fallback: Option<ObjRef>,
    is_module: bool,
    return_module: bool,
    return_instance: Option<ObjRef>,
    return_class: bool,
    class_bases: Vec<ObjRef>,
    blocks: Vec<Block>,
    active_exception: Option<Value>,
    expect_none_return: bool,
}

impl Frame {
    fn new(code: Rc<CodeObject>, module: ObjRef, is_module: bool, return_module: bool) -> Self {
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
            expect_none_return: false,
        }
    }
}

pub struct Vm {
    frames: Vec<Frame>,
    builtins: HashMap<String, Value>,
    modules: HashMap<String, ObjRef>,
    main_module: ObjRef,
    module_paths: Vec<PathBuf>,
    heap: Heap,
}

impl Vm {
    pub fn new() -> Self {
        let heap = Heap::new();
        let main_module = match heap.alloc_module(ModuleObject::new("__main__")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module) = &mut *main_module.kind_mut() {
            module
                .globals
                .insert("__name__".to_string(), Value::Str("__main__".to_string()));
        }

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
            heap,
        };
        vm.install_builtins();
        vm
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: Value) {
        if let Object::Module(module) = &mut *self.main_module.kind_mut() {
            module.globals.insert(name.into(), value);
        }
    }

    pub fn get_global(&self, name: &str) -> Option<Value> {
        if let Object::Module(module) = &*self.main_module.kind() {
            return module.globals.get(name).cloned();
        }
        None
    }

    pub fn add_module_path(&mut self, path: impl Into<PathBuf>) {
        self.module_paths.push(path.into());
    }

    pub fn id_of(&self, value: &Value) -> u64 {
        self.heap.id_of(value)
    }

    pub fn alloc_module(&mut self, name: impl Into<String>) -> Value {
        self.heap.alloc_module(ModuleObject::new(name))
    }

    pub fn alloc_list(&mut self, values: Vec<Value>) -> Value {
        self.heap.alloc_list(values)
    }

    pub fn alloc_tuple(&mut self, values: Vec<Value>) -> Value {
        self.heap.alloc_tuple(values)
    }

    pub fn alloc_dict(&mut self, values: Vec<(Value, Value)>) -> Value {
        self.heap.alloc_dict(values)
    }

    pub fn heap_object_count(&self) -> usize {
        self.heap.live_objects_count()
    }

    pub fn gc_collect(&mut self) {
        let mut roots = Vec::new();
        for value in self.builtins.values() {
            roots.push(value.clone());
        }
        for module in self.modules.values() {
            roots.push(Value::Module(module.clone()));
        }
        roots.push(Value::Module(self.main_module.clone()));
        for frame in &self.frames {
            roots.extend(frame.stack.iter().cloned());
            roots.extend(frame.locals.values().cloned());
            roots.push(Value::Module(frame.module.clone()));
            roots.push(Value::Module(frame.function_globals.clone()));
            if let Some(fallback) = &frame.globals_fallback {
                roots.push(Value::Module(fallback.clone()));
            }
            if let Some(instance) = &frame.return_instance {
                roots.push(Value::Instance(instance.clone()));
            }
            for base in &frame.class_bases {
                roots.push(Value::Class(base.clone()));
            }
            if let Some(exc) = &frame.active_exception {
                roots.push(exc.clone());
            }
        }
        self.heap.collect_cycles(&roots);
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

    pub fn execute_pyc_bytes(&mut self, bytes: &[u8]) -> Result<Value, RuntimeError> {
        let pyc = cpython::load_pyc(bytes).map_err(|err| RuntimeError::new(err.message))?;
        let code =
            cpython::translate_code(&pyc, &mut self.heap).map_err(|err| RuntimeError::new(err.message))?;
        self.execute(&code)
    }

    pub fn execute_pyc_file(&mut self, path: &str) -> Result<Value, RuntimeError> {
        let bytes = std::fs::read(path)
            .map_err(|err| RuntimeError::new(format!("failed to read {path}: {err}")))?;
        self.execute_pyc_bytes(&bytes)
    }

    fn load_module(&mut self, name: &str) -> Result<ObjRef, RuntimeError> {
        if let Some(module) = self.modules.get(name).cloned() {
            return Ok(module);
        }

        let path = self
            .find_module_file(name)
            .ok_or_else(|| RuntimeError::new(format!("module '{name}' not found")))?;

        let source = std::fs::read_to_string(&path).map_err(|err| {
            RuntimeError::new(format!("failed to read module '{name}': {err}"))
        })?;

        let module = match self.heap.alloc_module(ModuleObject::new(name)) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data
                .globals
                .insert("__name__".to_string(), Value::Str(name.to_string()));
            module_data.globals.insert(
                "__file__".to_string(),
                Value::Str(path.to_string_lossy().to_string()),
            );
        }

        self.modules.insert(name.to_string(), module.clone());
        self.link_module_chain(name, module.clone());

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
        let rel_name = name.replace('.', "/");
        let filename = format!("{rel_name}.py");
        for base in &self.module_paths {
            let candidate = base.join(&filename);
            if candidate.exists() {
                return Some(candidate);
            }
            let package_init = base.join(&rel_name).join("__init__.py");
            if package_init.exists() {
                return Some(package_init);
            }
        }
        None
    }

    fn load_submodule(&mut self, parent: &ObjRef, attr_name: &str) -> Option<ObjRef> {
        let parent_name = match &*parent.kind() {
            Object::Module(module) => module.name.clone(),
            _ => return None,
        };
        let full_name = format!("{}.{}", parent_name, attr_name);
        if let Some(module) = self.modules.get(&full_name).cloned() {
            return Some(module);
        }
        if self.find_module_file(&full_name).is_some() {
            if let Ok(module) = self.load_module(&full_name) {
                if let Object::Module(module_data) = &mut *parent.kind_mut() {
                    module_data
                        .globals
                        .insert(attr_name.to_string(), Value::Module(module.clone()));
                }
                return Some(module);
            }
        }
        None
    }

    fn ensure_module(&mut self, name: &str) -> ObjRef {
        if let Some(module) = self.modules.get(name).cloned() {
            return module;
        }
        let module = match self.heap.alloc_module(ModuleObject::new(name)) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data
                .globals
                .insert("__name__".to_string(), Value::Str(name.to_string()));
        }
        self.modules.insert(name.to_string(), module.clone());
        module
    }

    fn link_module_chain(&mut self, name: &str, module: ObjRef) {
        let parts: Vec<&str> = name.split('.').collect();
        if parts.len() <= 1 {
            return;
        }

        let mut current_name = parts[0].to_string();
        let mut current_module = self.ensure_module(&current_name);

        for part in parts.iter().skip(1) {
            let child_name = format!("{current_name}.{part}");
            let child_module = if child_name == name {
                module.clone()
            } else {
                self.ensure_module(&child_name)
            };
            if let Object::Module(module_data) = &mut *current_module.kind_mut() {
                module_data
                    .globals
                    .insert(part.to_string(), Value::Module(child_module.clone()));
            }
            current_module = child_module;
            current_name = child_name;
        }
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
                Opcode::LoadFast => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing local argument"))?
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
                    let value = self
                        .frames
                        .last()
                        .and_then(|frame| frame.locals.get(&name).cloned())
                        .ok_or_else(|| RuntimeError::new(format!("local '{name}' not set")))?;
                    self.push_value(value);
                }
                Opcode::LoadFast2 => {
                    let arg = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing locals argument"))?;
                    let first = (arg >> 16) as usize;
                    let second = (arg & 0xFFFF) as usize;
                    let (first_name, second_name) = {
                        let frame = self.frames.last().expect("frame exists");
                        let first = frame
                            .code
                            .names
                            .get(first)
                            .ok_or_else(|| RuntimeError::new("name index out of range"))?
                            .clone();
                        let second = frame
                            .code
                            .names
                            .get(second)
                            .ok_or_else(|| RuntimeError::new("name index out of range"))?
                            .clone();
                        (first, second)
                    };
                    let (first_value, second_value) = {
                        let frame = self.frames.last().expect("frame exists");
                        let first = frame.locals.get(&first_name).cloned();
                        let second = frame.locals.get(&second_name).cloned();
                        (first, second)
                    };
                    let first_value = first_value
                        .ok_or_else(|| RuntimeError::new(format!("local '{first_name}' not set")))?;
                    let second_value = second_value.ok_or_else(|| {
                        RuntimeError::new(format!("local '{second_name}' not set"))
                    })?;
                    self.push_value(first_value);
                    self.push_value(second_value);
                }
                Opcode::LoadFastAndClear => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing local argument"))?
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
                    let value = self
                        .frames
                        .last_mut()
                        .and_then(|frame| frame.locals.remove(&name))
                        .ok_or_else(|| RuntimeError::new(format!("local '{name}' not set")))?;
                    self.push_value(value);
                }
                Opcode::LoadGlobal => {
                    let raw = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing global argument"))?
                        as usize;
                    let push_null = raw & 1 == 1;
                    let idx = raw >> 1;
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
                        let frame = self.frames.last().expect("frame exists");
                        if let Object::Module(module_data) = &*frame.function_globals.kind() {
                            if let Some(value) = module_data.globals.get(&name) {
                                Some(value.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    .or_else(|| self.builtins.get(&name).cloned())
                    .ok_or_else(|| RuntimeError::new(format!("name '{name}' is not defined")))?;
                    if push_null {
                        self.push_value(Value::None);
                    }
                    self.push_value(value);
                }
                Opcode::LoadBuildClass => {
                    self.push_value(Value::Builtin(BuiltinFunction::BuildClass));
                }
                Opcode::PushNull => {
                    self.push_value(Value::None);
                }
                Opcode::LoadAttr => {
                    let raw = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing attribute argument"))?
                        as usize;
                    let push_null = raw & 1 == 1;
                    let idx = raw >> 1;
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
                            let (module_name, attr) = match &*module.kind() {
                                Object::Module(module_data) => {
                                    let attr = module_data.globals.get(&attr_name).cloned();
                                    let module_name = module_data.name.clone();
                                    (module_name, attr)
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attribute access unsupported type",
                                    ))
                                }
                            };
                            let attr = if let Some(attr) = attr {
                                Some(attr)
                            } else {
                                module_name
                                    .split('.')
                                    .last()
                                    .and_then(|suffix| {
                                        if suffix == attr_name {
                                            Some(Value::Module(module.clone()))
                                        } else {
                                            None
                                        }
                                    })
                            }
                            .or_else(|| {
                                self.load_submodule(&module, &attr_name)
                                    .map(Value::Module)
                            })
                            .ok_or_else(|| {
                                RuntimeError::new(format!(
                                    "module '{}' has no attribute '{}'",
                                    module_name, attr_name
                                ))
                            })?;
                            if push_null {
                                self.push_value(Value::None);
                            }
                            self.push_value(attr);
                        }
                        Value::Class(class) => {
                            let class_name = match &*class.kind() {
                                Object::Class(class_data) => class_data.name.clone(),
                                _ => "<class>".to_string(),
                            };
                            let attr = class_attr_lookup(&class, &attr_name).ok_or_else(|| {
                                RuntimeError::new(format!(
                                    "class '{}' has no attribute '{}'",
                                    class_name, attr_name
                                ))
                            })?;
                            if push_null {
                                self.push_value(Value::None);
                            }
                            self.push_value(attr);
                        }
                        Value::Instance(instance) => {
                            if let Object::Instance(instance_data) = &*instance.kind() {
                                if let Some(attr) =
                                    instance_data.attrs.get(&attr_name).cloned()
                                {
                                    if push_null {
                                        self.push_value(Value::None);
                                    }
                                    self.push_value(attr);
                                    return Ok(None);
                                }
                            }
                            let class_ref = match &*instance.kind() {
                                Object::Instance(instance_data) => instance_data.class.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attribute access unsupported type",
                                    ))
                                }
                            };
                            if let Some(attr) = class_attr_lookup(&class_ref, &attr_name) {
                                if let Value::Function(func) = attr {
                                    let bound = BoundMethod::new(func, instance.clone());
                                    let bound_value = self.heap.alloc_bound_method(bound);
                                    if push_null {
                                        self.push_value(Value::None);
                                    }
                                    self.push_value(bound_value);
                                } else {
                                    if push_null {
                                        self.push_value(Value::None);
                                    }
                                    self.push_value(attr);
                                }
                                return Ok(None);
                            }
                            let class_name = match &*class_ref.kind() {
                                Object::Class(class_data) => class_data.name.clone(),
                                _ => "<class>".to_string(),
                            };
                            return Err(RuntimeError::new(format!(
                                "'{}' object has no attribute '{}'",
                                class_name, attr_name
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
                Opcode::StoreFast => {
                    let idx = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing local argument"))?
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
                    if let Some(frame) = self.frames.last_mut() {
                        frame.locals.insert(name, value);
                    }
                }
                Opcode::StoreFastLoadFast => {
                    let arg = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing locals argument"))?;
                    let first = (arg >> 16) as usize;
                    let second = (arg & 0xFFFF) as usize;
                    let (first_name, second_name) = {
                        let frame = self.frames.last().expect("frame exists");
                        let first = frame
                            .code
                            .names
                            .get(first)
                            .ok_or_else(|| RuntimeError::new("name index out of range"))?
                            .clone();
                        let second = frame
                            .code
                            .names
                            .get(second)
                            .ok_or_else(|| RuntimeError::new("name index out of range"))?
                            .clone();
                        (first, second)
                    };
                    let value = self.pop_value()?;
                    if let Some(frame) = self.frames.last_mut() {
                        frame.locals.insert(first_name, value);
                        let value = frame
                            .locals
                            .get(&second_name)
                            .cloned()
                            .ok_or_else(|| {
                                RuntimeError::new(format!(
                                    "local '{second_name}' not set"
                                ))
                            })?;
                        self.push_value(value);
                    }
                }
                Opcode::StoreFastStoreFast => {
                    let arg = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing locals argument"))?;
                    let first = (arg >> 16) as usize;
                    let second = (arg & 0xFFFF) as usize;
                    let (first_name, second_name) = {
                        let frame = self.frames.last().expect("frame exists");
                        let first = frame
                            .code
                            .names
                            .get(first)
                            .ok_or_else(|| RuntimeError::new("name index out of range"))?
                            .clone();
                        let second = frame
                            .code
                            .names
                            .get(second)
                            .ok_or_else(|| RuntimeError::new("name index out of range"))?
                            .clone();
                        (first, second)
                    };
                    let value2 = self.pop_value()?;
                    let value1 = self.pop_value()?;
                    if let Some(frame) = self.frames.last_mut() {
                        frame.locals.insert(first_name, value1);
                        frame.locals.insert(second_name, value2);
                    }
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
                            if let Object::Module(module_data) = &mut *module.kind_mut() {
                                module_data.globals.insert(attr_name, value);
                            }
                        }
                        Value::Instance(instance) => {
                            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                instance_data.attrs.insert(attr_name, value);
                            }
                        }
                        Value::Class(class) => {
                            if let Object::Class(class_data) = &mut *class.kind_mut() {
                                class_data.attrs.insert(attr_name, value);
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "attribute assignment unsupported type",
                            ))
                        }
                    }
                }
                Opcode::StoreAttrCpython => {
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
                    let target = self.pop_value()?;
                    let value = self.pop_value()?;
                    match target {
                        Value::Module(module) => {
                            if let Object::Module(module_data) = &mut *module.kind_mut() {
                                module_data.globals.insert(attr_name, value);
                            }
                        }
                        Value::Instance(instance) => {
                            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                instance_data.attrs.insert(attr_name, value);
                            }
                        }
                        Value::Class(class) => {
                            if let Object::Class(class_data) = &mut *class.kind_mut() {
                                class_data.attrs.insert(attr_name, value);
                            }
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
                        if let Object::Module(module_data) = &mut *frame.function_globals.kind_mut()
                        {
                            module_data.globals.insert(name, value);
                        }
                    }
                }
                Opcode::BinaryAdd => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(add_values(left, right, &self.heap)?);
                }
                Opcode::BinarySub => {
                    let (left, right) = self.pop_int_pair()?;
                    self.push_value(Value::Int(left - right));
                }
                Opcode::BinaryMul => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    self.push_value(mul_values(left, right, &self.heap)?);
                }
                Opcode::BinaryPow => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    let (left, right) = (value_to_int(left)?, value_to_int(right)?);
                    if right < 0 {
                        return Err(RuntimeError::new("negative exponent not supported"));
                    }
                    let value = left
                        .checked_pow(right as u32)
                        .ok_or_else(|| RuntimeError::new("integer overflow"))?;
                    self.push_value(Value::Int(value));
                }
                Opcode::BinaryFloorDiv => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    let (left, right) = (value_to_int(left)?, value_to_int(right)?);
                    let value = python_floor_div(left, right)?;
                    self.push_value(Value::Int(value));
                }
                Opcode::BinaryMod => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    let (left, right) = (value_to_int(left)?, value_to_int(right)?);
                    let value = python_mod(left, right)?;
                    self.push_value(Value::Int(value));
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
                    let same = self.heap.id_of(&left) == self.heap.id_of(&right);
                    self.push_value(Value::Bool(same));
                }
                Opcode::CompareIsNot => {
                    let right = self.pop_value()?;
                    let left = self.pop_value()?;
                    let same = self.heap.id_of(&left) == self.heap.id_of(&right);
                    self.push_value(Value::Bool(!same));
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
                    self.push_value(self.heap.alloc_list(values));
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
                    self.push_value(self.heap.alloc_tuple(values));
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
                    self.push_value(self.heap.alloc_dict(values));
                }
                Opcode::UnpackSequence => {
                    let count = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing unpack size"))?
                        as usize;
                    let value = self.pop_value()?;
                    let items = match value {
                        Value::List(obj) => match &*obj.kind() {
                            Object::List(values) => values.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "unpack expects list or tuple",
                                ))
                            }
                        },
                        Value::Tuple(obj) => match &*obj.kind() {
                            Object::Tuple(values) => values.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "unpack expects list or tuple",
                                ))
                            }
                        },
                        _ => {
                            return Err(RuntimeError::new(
                                "unpack expects list or tuple",
                            ))
                        }
                    };
                    if items.len() != count {
                        return Err(RuntimeError::new("unpack length mismatch"));
                    }
                    for item in items {
                        self.push_value(item);
                    }
                }
                Opcode::ListAppend => {
                    let value = self.pop_value()?;
                    let list = self.pop_value()?;
                    match list {
                        Value::List(obj) => {
                            if let Object::List(values) = &mut *obj.kind_mut() {
                                values.push(value);
                            }
                            self.push_value(Value::List(obj));
                        }
                        _ => return Err(RuntimeError::new("list append expects list")),
                    }
                }
                Opcode::ListExtend => {
                    let other = self.pop_value()?;
                    let list = self.pop_value()?;
                    match list {
                        Value::List(obj) => {
                            if let Object::List(values) = &mut *obj.kind_mut() {
                                match other {
                                    Value::List(items) => match &*items.kind() {
                                        Object::List(items) => values.extend(items.clone()),
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "list extend expects iterable",
                                            ))
                                        }
                                    },
                                    Value::Tuple(items) => match &*items.kind() {
                                        Object::Tuple(items) => values.extend(items.clone()),
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "list extend expects iterable",
                                            ))
                                        }
                                    },
                                    Value::Str(text) => {
                                        for ch in text.chars() {
                                            values.push(Value::Str(ch.to_string()));
                                        }
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "list extend expects iterable",
                                        ))
                                    }
                                }
                            }
                            self.push_value(Value::List(obj));
                        }
                        _ => return Err(RuntimeError::new("list extend expects list")),
                    }
                }
                Opcode::DictSet => {
                    let value = self.pop_value()?;
                    let key = self.pop_value()?;
                    let dict = self.pop_value()?;
                    match dict {
                        Value::Dict(obj) => {
                            if let Object::Dict(entries) = &mut *obj.kind_mut() {
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
                            self.push_value(Value::Dict(obj));
                        }
                        _ => return Err(RuntimeError::new("dict set expects dict")),
                    }
                }
                Opcode::DictUpdate => {
                    let other = self.pop_value()?;
                    let dict = self.pop_value()?;
                    match (dict, other) {
                        (Value::Dict(obj), Value::Dict(other)) => {
                            let other_entries = match &*other.kind() {
                                Object::Dict(entries) => entries.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "dict update expects dict",
                                    ))
                                }
                            };
                            if let Object::Dict(entries) = &mut *obj.kind_mut() {
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
                            }
                            self.push_value(Value::Dict(obj));
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
                            Value::List(obj) => match &*obj.kind() {
                                Object::List(values) => {
                                    let indices =
                                        slice_indices(values.len(), lower, upper, step)?;
                                    let mut result = Vec::with_capacity(indices.len());
                                    for idx in indices {
                                        result.push(values[idx].clone());
                                    }
                                    self.push_value(self.heap.alloc_list(result));
                                }
                                _ => return Err(RuntimeError::new("subscript unsupported type")),
                            },
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(values) => {
                                    let indices =
                                        slice_indices(values.len(), lower, upper, step)?;
                                    let mut result = Vec::with_capacity(indices.len());
                                    for idx in indices {
                                        result.push(values[idx].clone());
                                    }
                                    self.push_value(self.heap.alloc_tuple(result));
                                }
                                _ => return Err(RuntimeError::new("subscript unsupported type")),
                            },
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
                            Value::List(obj) => match &*obj.kind() {
                                Object::List(values) => {
                                    let mut index_int = value_to_int(index)? as isize;
                                    if index_int < 0 {
                                        index_int += values.len() as isize;
                                    }
                                    if index_int < 0 || index_int as usize >= values.len() {
                                        return Err(RuntimeError::new("list index out of range"));
                                    }
                                    self.push_value(values[index_int as usize].clone());
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "subscript unsupported type",
                                    ))
                                }
                            },
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(values) => {
                                    let mut index_int = value_to_int(index)? as isize;
                                    if index_int < 0 {
                                        index_int += values.len() as isize;
                                    }
                                    if index_int < 0 || index_int as usize >= values.len() {
                                        return Err(RuntimeError::new("tuple index out of range"));
                                    }
                                    self.push_value(values[index_int as usize].clone());
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "subscript unsupported type",
                                    ))
                                }
                            },
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
                            Value::Dict(obj) => match &*obj.kind() {
                                Object::Dict(entries) => {
                                    let mut found = None;
                                    for (key, value) in entries {
                                        if *key == index {
                                            found = Some(value.clone());
                                            break;
                                        }
                                    }
                                    if let Some(value) = found {
                                        self.push_value(value);
                                    } else {
                                        return Err(RuntimeError::new("key not found"));
                                    }
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "subscript unsupported type",
                                    ))
                                }
                            },
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
                            Value::List(obj) => {
                                if let Object::List(values) = &mut *obj.kind_mut() {
                                    let mut idx = value_to_int(index)? as isize;
                                    if idx < 0 {
                                        idx += values.len() as isize;
                                    }
                                    if idx < 0 || idx as usize >= values.len() {
                                        return Err(RuntimeError::new("list index out of range"));
                                    }
                                    values[idx as usize] = value;
                                }
                                self.push_value(Value::List(obj));
                            }
                            Value::Dict(obj) => {
                                if let Object::Dict(entries) = &mut *obj.kind_mut() {
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
                                }
                                self.push_value(Value::Dict(obj));
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
                    let kwonly_value = self.pop_value()?;
                    let kwonly_defaults = match kwonly_value {
                        Value::Dict(obj) => match &*obj.kind() {
                            Object::Dict(entries) => {
                                let mut map = HashMap::new();
                                for (key, value) in entries {
                                    let key = match key {
                                        Value::Str(name) => name.clone(),
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "kwonly default name must be string",
                                            ))
                                        }
                                    };
                                    map.insert(key, value.clone());
                                }
                                map
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "expected kwonly defaults dict for function",
                                ))
                            }
                        },
                        _ => {
                            return Err(RuntimeError::new(
                                "expected kwonly defaults dict for function",
                            ))
                        }
                    };
                    let defaults_value = self.pop_value()?;
                    let defaults = match defaults_value {
                        Value::Tuple(obj) => match &*obj.kind() {
                            Object::Tuple(values) => values.clone(),
                            _ => {
                                return Err(RuntimeError::new(
                                    "expected defaults tuple for function",
                                ))
                            }
                        },
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
                    let func = FunctionObject::new(code, module, defaults, kwonly_defaults);
                    self.push_value(self.heap.alloc_function(func));
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
                        Value::Tuple(obj) => match &*obj.kind() {
                            Object::Tuple(values) => values.clone(),
                            _ => return Err(RuntimeError::new("class bases must be a tuple")),
                        },
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

                    let class_module = match self
                        .heap
                        .alloc_module(ModuleObject::new(class_name.clone()))
                    {
                        Value::Module(obj) => obj,
                        _ => unreachable!(),
                    };
                    if let Object::Module(module_data) = &mut *class_module.kind_mut() {
                        module_data
                            .globals
                            .insert("__name__".to_string(), Value::Str(class_name));
                    }

                    let outer_globals = self
                        .frames
                        .last()
                        .map(|frame| frame.module.clone())
                        .unwrap_or_else(|| self.main_module.clone());
                    let mut frame = Frame::new(code, class_module, true, false);
                    frame.function_globals = outer_globals.clone();
                    frame.globals_fallback = Some(outer_globals);
                    frame.locals.insert(
                        "__classdict__".to_string(),
                        self.heap.alloc_dict(Vec::new()),
                    );
                    frame.return_class = true;
                    frame.class_bases = base_classes;
                    self.frames.push(frame);
                }
                Opcode::MakeFunctionStack => {
                    let value = self.pop_value()?;
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
                        .function_globals
                        .clone();
                    let func = FunctionObject::new(code, module, Vec::new(), HashMap::new());
                    self.push_value(self.heap.alloc_function(func));
                }
                Opcode::SetFunctionAttribute => {
                    let func_value = self.pop_value()?;
                    let attr = self.pop_value()?;
                    let func = match func_value {
                        Value::Function(func) => func,
                        _ => return Err(RuntimeError::new("expected function")),
                    };
                    let attr_kind = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing function attribute kind"))?;
                    match attr_kind {
                        0x01 => {
                            let defaults = match attr {
                                Value::Tuple(obj) => match &*obj.kind() {
                                    Object::Tuple(values) => values.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "defaults must be tuple",
                                        ))
                                    }
                                },
                                _ => return Err(RuntimeError::new("defaults must be tuple")),
                            };
                            if let Object::Function(func_data) = &mut *func.kind_mut() {
                                func_data.defaults = defaults;
                            }
                        }
                        0x02 => {
                            let kwonly = match attr {
                                Value::Dict(obj) => match &*obj.kind() {
                                    Object::Dict(entries) => {
                                        let mut map = HashMap::new();
                                        for (key, value) in entries {
                                            let name = match key {
                                                Value::Str(name) => name.clone(),
                                                _ => {
                                                    return Err(RuntimeError::new(
                                                        "kwonly default name must be string",
                                                    ))
                                                }
                                            };
                                            map.insert(name, value.clone());
                                        }
                                        map
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "kwonly defaults must be dict",
                                        ))
                                    }
                                },
                                _ => return Err(RuntimeError::new("kwonly defaults must be dict")),
                            };
                            if let Object::Function(func_data) = &mut *func.kind_mut() {
                                func_data.kwonly_defaults = kwonly;
                            }
                        }
                        _ => {
                            // ignore annotations/closure for now
                        }
                    }
                    self.push_value(Value::Function(func));
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
                            let func_data = match &*func.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let bindings =
                                bind_arguments(&func_data, &self.heap, args, HashMap::new())?;
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            self.frames.push(frame);
                        }
                        Value::BoundMethod(method) => {
                            let method_data = match &*method.kind() {
                                Object::BoundMethod(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let func_data = match &*method_data.function.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let mut bound_args = Vec::with_capacity(args.len() + 1);
                            bound_args.push(Value::Instance(method_data.receiver.clone()));
                            bound_args.extend(args);
                            let bindings = bind_arguments(
                                &func_data,
                                &self.heap,
                                bound_args,
                                HashMap::new(),
                            )?;
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            self.frames.push(frame);
                        }
                        Value::Class(class) => {
                            let instance = match self
                                .heap
                                .alloc_instance(InstanceObject::new(class.clone()))
                            {
                                Value::Instance(obj) => obj,
                                _ => unreachable!(),
                            };
                            let init = class_attr_lookup(&class, "__init__");
                            if let Some(Value::Function(init_func)) = init {
                                let func_data = match &*init_func.kind() {
                                    Object::Function(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ))
                                    }
                                };
                                let mut init_args = Vec::with_capacity(args.len() + 1);
                                init_args.push(Value::Instance(instance.clone()));
                                init_args.extend(args);
                                let bindings = bind_arguments(
                                    &func_data,
                                    &self.heap,
                                    init_args,
                                    HashMap::new(),
                                )?;
                                let mut frame = Frame::new(
                                    func_data.code.clone(),
                                    func_data.module.clone(),
                                    false,
                                    false,
                                );
                                frame.return_instance = Some(instance);
                                frame.expect_none_return = true;
                                apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                                self.frames.push(frame);
                            } else {
                                self.push_value(Value::Instance(instance));
                            }
                        }
                        Value::Builtin(builtin) => {
                            let result = builtin.call(&self.heap, args)?;
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
                Opcode::CallCpython => {
                    let arg = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing call argument"))?;
                    let pos_count = (arg & 0xFFFF) as usize;
                    let kw_idx = (arg >> 16) as u16;
                    let kw_names = if kw_idx == u16::MAX {
                        None
                    } else {
                        let idx = kw_idx as usize;
                        let value = {
                            let frame = self.frames.last().expect("frame exists");
                            frame
                                .code
                                .constants
                                .get(idx)
                                .cloned()
                                .ok_or_else(|| RuntimeError::new("constant index out of range"))?
                        };
                        Some(value)
                    };

                    let kw_names = if let Some(value) = kw_names {
                        match value {
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(values) => {
                                    let mut names = Vec::new();
                                    for value in values {
                                        match value {
                                            Value::Str(name) => names.push(name.clone()),
                                            _ => {
                                                return Err(RuntimeError::new(
                                                    "kw_names must be tuple of strings",
                                                ))
                                            }
                                        }
                                    }
                                    Some(names)
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "kw_names must be tuple of strings",
                                    ))
                                }
                            },
                            Value::None => None,
                            _ => {
                                return Err(RuntimeError::new(
                                    "kw_names must be tuple of strings",
                                ))
                            }
                        }
                    } else {
                        None
                    };

                    let kw_count = kw_names.as_ref().map(|names| names.len()).unwrap_or(0);
                    if pos_count < kw_count {
                        return Err(RuntimeError::new("call arg count mismatch"));
                    }
                    let mut kwargs = HashMap::new();
                    for idx in (0..kw_count).rev() {
                        let value = self.pop_value()?;
                        let name = kw_names
                            .as_ref()
                            .expect("kw names")
                            .get(idx)
                            .cloned()
                            .ok_or_else(|| RuntimeError::new("kw name index out of range"))?;
                        kwargs.insert(name, value);
                    }
                    let mut args = Vec::with_capacity(pos_count - kw_count);
                    for _ in 0..(pos_count - kw_count) {
                        args.push(self.pop_value()?);
                    }
                    args.reverse();
                    let mut func = self.pop_value()?;
                    if matches!(func, Value::None) {
                        func = self.pop_value()?;
                    }
                    if let Some(Value::None) =
                        self.frames.last().and_then(|frame| frame.stack.last()).cloned()
                    {
                        let _ = self.pop_value();
                    }

                    match func {
                        Value::Function(func) => {
                            let func_data = match &*func.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let bindings = bind_arguments(&func_data, &self.heap, args, kwargs)?;
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            self.frames.push(frame);
                        }
                        Value::BoundMethod(method) => {
                            let method_data = match &*method.kind() {
                                Object::BoundMethod(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let func_data = match &*method_data.function.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let mut bound_args = Vec::with_capacity(args.len() + 1);
                            bound_args.push(Value::Instance(method_data.receiver.clone()));
                            bound_args.extend(args);
                            let bindings = bind_arguments(
                                &func_data,
                                &self.heap,
                                bound_args,
                                kwargs,
                            )?;
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            self.frames.push(frame);
                        }
                        Value::Class(class) => {
                            let instance = match self
                                .heap
                                .alloc_instance(InstanceObject::new(class.clone()))
                            {
                                Value::Instance(obj) => obj,
                                _ => unreachable!(),
                            };
                            let init = class_attr_lookup(&class, "__init__");
                            if let Some(Value::Function(init_func)) = init {
                                let func_data = match &*init_func.kind() {
                                    Object::Function(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ))
                                    }
                                };
                                let mut init_args = Vec::with_capacity(args.len() + 1);
                                init_args.push(Value::Instance(instance.clone()));
                                init_args.extend(args);
                                let bindings = bind_arguments(
                                    &func_data,
                                    &self.heap,
                                    init_args,
                                    kwargs,
                                )?;
                                let mut frame = Frame::new(
                                    func_data.code.clone(),
                                    func_data.module.clone(),
                                    false,
                                    false,
                                );
                                frame.return_instance = Some(instance);
                                frame.expect_none_return = true;
                                apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                                self.frames.push(frame);
                            } else {
                                self.push_value(Value::Instance(instance));
                            }
                        }
                        Value::Builtin(BuiltinFunction::BuildClass) => {
                            let class_value = self.call_build_class(args, kwargs)?;
                            if let Some(value) = class_value {
                                self.push_value(value);
                            }
                        }
                        Value::Builtin(builtin) => {
                            let result = builtin.call(&self.heap, args)?;
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
                Opcode::CallCpythonKwStack => {
                    let pos_total = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing call argument"))?
                        as usize;
                    let kw_names_value = self.pop_value()?;
                    let kw_names = match kw_names_value {
                        Value::Tuple(obj) => match &*obj.kind() {
                            Object::Tuple(values) => {
                                let mut names = Vec::new();
                                for value in values {
                                    match value {
                                        Value::Str(name) => names.push(name.clone()),
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "kw names must be strings",
                                            ))
                                        }
                                    }
                                }
                                names
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "kw names must be tuple",
                                ))
                            }
                        },
                        _ => return Err(RuntimeError::new("kw names must be tuple")),
                    };
                    let kw_count = kw_names.len();
                    if pos_total < kw_count {
                        return Err(RuntimeError::new("call arg count mismatch"));
                    }
                    let mut kwargs = HashMap::new();
                    for idx in (0..kw_count).rev() {
                        let value = self.pop_value()?;
                        let name = kw_names
                            .get(idx)
                            .cloned()
                            .ok_or_else(|| RuntimeError::new("kw name index out of range"))?;
                        kwargs.insert(name, value);
                    }
                    let mut args = Vec::with_capacity(pos_total - kw_count);
                    for _ in 0..(pos_total - kw_count) {
                        args.push(self.pop_value()?);
                    }
                    args.reverse();
                    let mut func = self.pop_value()?;
                    if matches!(func, Value::None) {
                        func = self.pop_value()?;
                    }
                    if let Some(Value::None) =
                        self.frames.last().and_then(|frame| frame.stack.last()).cloned()
                    {
                        let _ = self.pop_value();
                    }

                    match func {
                        Value::Function(func) => {
                            let func_data = match &*func.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let bindings = bind_arguments(&func_data, &self.heap, args, kwargs)?;
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            self.frames.push(frame);
                        }
                        Value::BoundMethod(method) => {
                            let method_data = match &*method.kind() {
                                Object::BoundMethod(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let func_data = match &*method_data.function.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let mut bound_args = Vec::with_capacity(args.len() + 1);
                            bound_args.push(Value::Instance(method_data.receiver.clone()));
                            bound_args.extend(args);
                            let bindings = bind_arguments(
                                &func_data,
                                &self.heap,
                                bound_args,
                                kwargs,
                            )?;
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            self.frames.push(frame);
                        }
                        Value::Class(class) => {
                            let instance = match self
                                .heap
                                .alloc_instance(InstanceObject::new(class.clone()))
                            {
                                Value::Instance(obj) => obj,
                                _ => unreachable!(),
                            };
                            let init = class_attr_lookup(&class, "__init__");
                            if let Some(Value::Function(init_func)) = init {
                                let func_data = match &*init_func.kind() {
                                    Object::Function(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ))
                                    }
                                };
                                let mut init_args = Vec::with_capacity(args.len() + 1);
                                init_args.push(Value::Instance(instance.clone()));
                                init_args.extend(args);
                                let bindings = bind_arguments(
                                    &func_data,
                                    &self.heap,
                                    init_args,
                                    kwargs,
                                )?;
                                let mut frame = Frame::new(
                                    func_data.code.clone(),
                                    func_data.module.clone(),
                                    false,
                                    false,
                                );
                                frame.return_instance = Some(instance);
                                frame.expect_none_return = true;
                                apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                                self.frames.push(frame);
                            } else {
                                self.push_value(Value::Instance(instance));
                            }
                        }
                        Value::Builtin(BuiltinFunction::BuildClass) => {
                            let class_value = self.call_build_class(args, kwargs)?;
                            if let Some(value) = class_value {
                                self.push_value(value);
                            }
                        }
                        Value::Builtin(builtin) => {
                            let result = builtin.call(&self.heap, args)?;
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
                            let func_data = match &*func.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let bindings = bind_arguments(&func_data, &self.heap, args, kwargs)?;
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            self.frames.push(frame);
                        }
                        Value::BoundMethod(method) => {
                            let method_data = match &*method.kind() {
                                Object::BoundMethod(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let func_data = match &*method_data.function.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let mut bound_args = Vec::with_capacity(args.len() + 1);
                            bound_args.push(Value::Instance(method_data.receiver.clone()));
                            bound_args.extend(args);
                            let bindings =
                                bind_arguments(&func_data, &self.heap, bound_args, kwargs)?;
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            self.frames.push(frame);
                        }
                        Value::Class(class) => {
                            let instance = match self
                                .heap
                                .alloc_instance(InstanceObject::new(class.clone()))
                            {
                                Value::Instance(obj) => obj,
                                _ => unreachable!(),
                            };
                            let init = class_attr_lookup(&class, "__init__");
                            if let Some(Value::Function(init_func)) = init {
                                let func_data = match &*init_func.kind() {
                                    Object::Function(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ))
                                    }
                                };
                                let mut init_args = Vec::with_capacity(args.len() + 1);
                                init_args.push(Value::Instance(instance.clone()));
                                init_args.extend(args);
                                let bindings =
                                    bind_arguments(&func_data, &self.heap, init_args, kwargs)?;
                                let mut frame = Frame::new(
                                    func_data.code.clone(),
                                    func_data.module.clone(),
                                    false,
                                    false,
                                );
                                frame.return_instance = Some(instance);
                                frame.expect_none_return = true;
                                apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
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
                            let result =
                                call_builtin_with_kwargs(&self.heap, builtin, args, kwargs)?;
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
                        Value::Dict(obj) => match &*obj.kind() {
                            Object::Dict(entries) => {
                                let mut map = HashMap::new();
                                for (key, value) in entries {
                                    let key = match key {
                                        Value::Str(name) => name.clone(),
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "keyword name must be string",
                                            ))
                                        }
                                    };
                                    if map.contains_key(&key) {
                                        return Err(RuntimeError::new(
                                            "duplicate keyword argument",
                                        ));
                                    }
                                    map.insert(key, value.clone());
                                }
                                map
                            }
                            _ => return Err(RuntimeError::new("call kwargs must be dict")),
                        },
                        _ => return Err(RuntimeError::new("call kwargs must be dict")),
                    };
                    let args = match args_value {
                        Value::List(obj) => match &*obj.kind() {
                            Object::List(values) => values.clone(),
                            _ => return Err(RuntimeError::new("call args must be list")),
                        },
                        _ => return Err(RuntimeError::new("call args must be list")),
                    };

                    match func {
                        Value::Function(func) => {
                            let func_data = match &*func.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let bindings = bind_arguments(&func_data, &self.heap, args, kwargs)?;
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            self.frames.push(frame);
                        }
                        Value::BoundMethod(method) => {
                            let method_data = match &*method.kind() {
                                Object::BoundMethod(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let func_data = match &*method_data.function.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ))
                                }
                            };
                            let mut bound_args = Vec::with_capacity(args.len() + 1);
                            bound_args.push(Value::Instance(method_data.receiver.clone()));
                            bound_args.extend(args);
                            let bindings =
                                bind_arguments(&func_data, &self.heap, bound_args, kwargs)?;
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                            );
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            self.frames.push(frame);
                        }
                        Value::Class(class) => {
                            let instance = match self
                                .heap
                                .alloc_instance(InstanceObject::new(class.clone()))
                            {
                                Value::Instance(obj) => obj,
                                _ => unreachable!(),
                            };
                            let init = class_attr_lookup(&class, "__init__");
                            if let Some(Value::Function(init_func)) = init {
                                let func_data = match &*init_func.kind() {
                                    Object::Function(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ))
                                    }
                                };
                                let mut init_args = Vec::with_capacity(args.len() + 1);
                                init_args.push(Value::Instance(instance.clone()));
                                init_args.extend(args);
                                let bindings =
                                    bind_arguments(&func_data, &self.heap, init_args, kwargs)?;
                                let mut frame = Frame::new(
                                    func_data.code.clone(),
                                    func_data.module.clone(),
                                    false,
                                    false,
                                );
                                frame.return_instance = Some(instance);
                                frame.expect_none_return = true;
                                apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
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
                            let result =
                                call_builtin_with_kwargs(&self.heap, builtin, args, kwargs)?;
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
                    let module = if let Some(module) = self.modules.get(&name).cloned() {
                        module
                    } else {
                        self.load_module(&name)?
                    };
                    let result_module = if let Some((root, _)) = name.split_once('.') {
                        self.link_module_chain(&name, module.clone());
                        if let Some(module) = self.modules.get(root).cloned() {
                            let is_root = match &*module.kind() {
                                Object::Module(module_data) => module_data.name == root,
                                _ => false,
                            };
                            if is_root {
                                module
                            } else {
                                self.ensure_module(root)
                            }
                        } else {
                            self.ensure_module(root)
                        }
                    } else {
                        module
                    };
                    self.push_value(Value::Module(result_module));
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
                Opcode::EndFor => {
                    // END_FOR is a sentinel in CPython; no-op for now.
                }
                Opcode::GetIter => {
                    let value = self.pop_value()?;
                    let iterator = match value {
                        Value::List(obj) => IteratorObject {
                            kind: IteratorKind::List(obj),
                            index: 0,
                        },
                        Value::Tuple(obj) => IteratorObject {
                            kind: IteratorKind::Tuple(obj),
                            index: 0,
                        },
                        Value::Str(value) => IteratorObject {
                            kind: IteratorKind::Str(value),
                            index: 0,
                        },
                        Value::Dict(obj) => IteratorObject {
                            kind: IteratorKind::Dict(obj),
                            index: 0,
                        },
                        Value::Iterator(obj) => {
                            self.push_value(Value::Iterator(obj));
                            return Ok(None);
                        }
                        _ => return Err(RuntimeError::new("object is not iterable")),
                    };
                    let value = self.heap.alloc_iterator(iterator);
                    self.push_value(value);
                }
                Opcode::ForIter => {
                    let target = instr
                        .arg
                        .ok_or_else(|| RuntimeError::new("missing jump target"))?
                        as usize;
                    let iterator_value = self.pop_value()?;
                    let iterator_ref = match iterator_value {
                        Value::Iterator(obj) => obj,
                        _ => return Err(RuntimeError::new("FOR_ITER expects iterator")),
                    };
                    let next_value = {
                        let mut iter = iterator_ref.kind_mut();
                        match &mut *iter {
                            Object::Iterator(state) => match &mut state.kind {
                                IteratorKind::List(list) => match &*list.kind() {
                                    Object::List(values) => {
                                        if state.index >= values.len() {
                                            None
                                        } else {
                                            let value = values[state.index].clone();
                                            state.index += 1;
                                            Some(value)
                                        }
                                    }
                                    _ => None,
                                },
                                IteratorKind::Tuple(list) => match &*list.kind() {
                                    Object::Tuple(values) => {
                                        if state.index >= values.len() {
                                            None
                                        } else {
                                            let value = values[state.index].clone();
                                            state.index += 1;
                                            Some(value)
                                        }
                                    }
                                    _ => None,
                                },
                                IteratorKind::Str(text) => {
                                    let chars: Vec<char> = text.chars().collect();
                                    if state.index >= chars.len() {
                                        None
                                    } else {
                                        let ch = chars[state.index];
                                        state.index += 1;
                                        Some(Value::Str(ch.to_string()))
                                    }
                                }
                                IteratorKind::Dict(dict) => match &*dict.kind() {
                                    Object::Dict(entries) => {
                                        if state.index >= entries.len() {
                                            None
                                        } else {
                                            let value = entries[state.index].0.clone();
                                            state.index += 1;
                                            Some(value)
                                        }
                                    }
                                    _ => None,
                                },
                            },
                            _ => None,
                        }
                    };
                    if let Some(value) = next_value {
                        self.push_value(Value::Iterator(iterator_ref));
                        self.push_value(value);
                    } else {
                        let frame = self.frames.last_mut().expect("frame exists");
                        frame.ip = target;
                    }
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
                Opcode::ReturnConst => {
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
                    let frame = self.frames.pop().expect("frame exists");
                    if frame.expect_none_return && value != Value::None {
                        return Err(RuntimeError::new("__init__() should return None"));
                    }
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
                Opcode::ReturnValue => {
                    let value = self.pop_value().unwrap_or(Value::None);
                    let frame = self.frames.pop().expect("frame exists");
                    if frame.expect_none_return && value != Value::None {
                        return Err(RuntimeError::new(
                            "__init__() should return None",
                        ));
                    }
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
        module: &ObjRef,
        bases: Vec<ObjRef>,
    ) -> Value {
        let (name, attrs) = match &*module.kind() {
            Object::Module(module_data) => (module_data.name.clone(), module_data.globals.clone()),
            _ => ("<class>".to_string(), HashMap::new()),
        };
        let class = ClassObject::new(name, bases);
        let class_value = self.heap.alloc_class(class);
        if let Value::Class(class_ref) = &class_value {
            if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
                class_data.attrs.extend(attrs);
            }
        }
        class_value
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
            if let Object::Module(module_data) = &*frame.module.kind() {
                if let Some(value) = module_data.globals.get(name) {
                    return Ok(value.clone());
                }
            }
            if let Some(fallback) = &frame.globals_fallback {
                if let Object::Module(module_data) = &*fallback.kind() {
                    if let Some(value) = module_data.globals.get(name) {
                        return Ok(value.clone());
                    }
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
                if let Object::Module(module_data) = &mut *frame.module.kind_mut() {
                    module_data.globals.insert(name, value);
                }
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
        self.builtins.insert(
            "enumerate".to_string(),
            Value::Builtin(BuiltinFunction::Enumerate),
        );
        self.builtins
            .insert("id".to_string(), Value::Builtin(BuiltinFunction::Id));
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

    fn call_build_class(
        &mut self,
        mut args: Vec<Value>,
        _kwargs: HashMap<String, Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::new(
                "__build_class__ expects at least a function and a name",
            ));
        }
        let name = match args.remove(1) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("class name must be a string")),
        };
        let func = match args.remove(0) {
            Value::Function(func) => func,
            _ => return Err(RuntimeError::new("class body must be a function")),
        };
        let func_data = match &*func.kind() {
            Object::Function(data) => data.clone(),
            _ => return Err(RuntimeError::new("class body must be a function")),
        };
        let mut base_classes = Vec::new();
        for base in args {
            match base {
                Value::Class(class) => base_classes.push(class),
                _ => {
                    return Err(RuntimeError::new(
                        "class base must be a class object",
                    ))
                }
            }
        }

        let class_module = match self
            .heap
            .alloc_module(ModuleObject::new(name.clone()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *class_module.kind_mut() {
            module_data
                .globals
                .insert("__name__".to_string(), Value::Str(name));
        }

        let outer_globals = func_data.module.clone();
        let mut frame = Frame::new(func_data.code.clone(), class_module, true, false);
        frame.function_globals = outer_globals.clone();
        frame.globals_fallback = Some(outer_globals);
        frame.locals.insert(
            "__classdict__".to_string(),
            self.heap.alloc_dict(Vec::new()),
        );
        frame.return_class = true;
        frame.class_bases = base_classes;
        self.frames.push(frame);
        Ok(None)
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

fn python_floor_div(left: i64, right: i64) -> Result<i64, RuntimeError> {
    if right == 0 {
        return Err(RuntimeError::new("integer division by zero"));
    }
    let a = left as i128;
    let b = right as i128;
    let mut div = a / b;
    let rem = a % b;
    if rem != 0 && ((a < 0) ^ (b < 0)) {
        div -= 1;
    }
    if div < i64::MIN as i128 || div > i64::MAX as i128 {
        return Err(RuntimeError::new("integer overflow"));
    }
    Ok(div as i64)
}

fn python_mod(left: i64, right: i64) -> Result<i64, RuntimeError> {
    if right == 0 {
        return Err(RuntimeError::new("modulo by zero"));
    }
    let a = left as i128;
    let b = right as i128;
    let div = python_floor_div(left, right)? as i128;
    let value = a - b * div;
    if value < i64::MIN as i128 || value > i64::MAX as i128 {
        return Err(RuntimeError::new("integer overflow"));
    }
    Ok(value as i64)
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
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => !values.is_empty(),
            _ => true,
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => !values.is_empty(),
            _ => true,
        },
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(values) => !values.is_empty(),
            _ => true,
        },
        Value::Iterator(_) => true,
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
    posonly: Vec<Value>,
    positional: Vec<Value>,
    kwonly: Vec<Value>,
    vararg: Option<Value>,
    kwarg: Option<Value>,
}

fn bind_arguments(
    func: &FunctionObject,
    heap: &Heap,
    mut positional: Vec<Value>,
    mut kwargs: HashMap<String, Value>,
) -> Result<BoundArguments, RuntimeError> {
    let posonly_len = func.code.posonly_params.len();
    let params_len = func.code.params.len();
    let kwonly_len = func.code.kwonly_params.len();
    let defaults_len = func.defaults.len();
    let total_positional = posonly_len + params_len;
    if defaults_len > total_positional {
        return Err(RuntimeError::new("invalid function defaults"));
    }

    let mut extra_positional = Vec::new();
    if positional.len() > total_positional {
        if func.code.vararg.is_none() {
            return Err(RuntimeError::new("argument count mismatch"));
        }
        extra_positional = positional.split_off(total_positional);
    }

    let required = total_positional - defaults_len;
    let mut bound: Vec<Option<Value>> = vec![None; total_positional];

    for (idx, value) in positional.into_iter().enumerate() {
        bound[idx] = Some(value);
    }

    let mut extra_kwargs: HashMap<String, Value> = HashMap::new();
    let mut kwonly_values: HashMap<String, Value> = HashMap::new();
    for (name, value) in kwargs.drain() {
        if func
            .code
            .posonly_params
            .iter()
            .any(|param| param == &name)
        {
            return Err(RuntimeError::new("unexpected keyword argument"));
        }
        if let Some(index) = func.code.params.iter().position(|param| param == &name) {
            if bound[index].is_some() {
                return Err(RuntimeError::new("multiple values for argument"));
            }
            bound[posonly_len + index] = Some(value);
        } else if func.code.kwonly_params.iter().any(|param| param == &name) {
            if kwonly_values.contains_key(&name) {
                return Err(RuntimeError::new("multiple values for argument"));
            }
            kwonly_values.insert(name, value);
        } else if func.code.kwarg.is_some() {
            if extra_kwargs.contains_key(&name) {
                return Err(RuntimeError::new("duplicate keyword argument"));
            }
            extra_kwargs.insert(name, value);
        } else {
            return Err(RuntimeError::new("unexpected keyword argument"));
        }
    }

    for idx in 0..total_positional {
        if bound[idx].is_none() {
            if idx < required {
                return Err(RuntimeError::new("argument count mismatch"));
            }
            let default_index = idx - required;
            bound[idx] = Some(func.defaults[default_index].clone());
        }
    }

    let posonly = bound[..posonly_len]
        .iter()
        .cloned()
        .map(|value| value.unwrap())
        .collect();
    let positional = bound[posonly_len..]
        .iter()
        .cloned()
        .map(|value| value.unwrap())
        .collect();
    let mut kwonly = Vec::with_capacity(kwonly_len);
    for name in &func.code.kwonly_params {
        if let Some(value) = kwonly_values.remove(name) {
            kwonly.push(value);
        } else if let Some(default) = func.kwonly_defaults.get(name) {
            kwonly.push(default.clone());
        } else {
            return Err(RuntimeError::new("missing keyword-only argument"));
        }
    }
    let vararg = func
        .code
        .vararg
        .as_ref()
        .map(|_| heap.alloc_list(extra_positional));
    let kwarg = func.code.kwarg.as_ref().map(|_| {
        let mut entries = Vec::with_capacity(extra_kwargs.len());
        for (key, value) in extra_kwargs {
            entries.push((Value::Str(key), value));
        }
        heap.alloc_dict(entries)
    });

    Ok(BoundArguments {
        posonly,
        positional,
        kwonly,
        vararg,
        kwarg,
    })
}

fn apply_bindings(frame: &mut Frame, code: &CodeObject, bindings: BoundArguments, heap: &Heap) {
    for (name, value) in code
        .posonly_params
        .iter()
        .cloned()
        .zip(bindings.posonly.into_iter())
    {
        frame.locals.insert(name, value);
    }
    for (name, value) in code
        .params
        .iter()
        .cloned()
        .zip(bindings.positional.into_iter())
    {
        frame.locals.insert(name, value);
    }
    for (name, value) in code
        .kwonly_params
        .iter()
        .cloned()
        .zip(bindings.kwonly.into_iter())
    {
        frame.locals.insert(name, value);
    }

    if let Some(name) = code.vararg.as_ref() {
        let value = bindings
            .vararg
            .unwrap_or_else(|| heap.alloc_list(Vec::new()));
        frame.locals.insert(name.clone(), value);
    }

    if let Some(name) = code.kwarg.as_ref() {
        let value = bindings
            .kwarg
            .unwrap_or_else(|| heap.alloc_dict(Vec::new()));
        frame.locals.insert(name.clone(), value);
    }
}

fn call_builtin_with_kwargs(
    heap: &Heap,
    builtin: BuiltinFunction,
    mut args: Vec<Value>,
    mut kwargs: HashMap<String, Value>,
) -> Result<Value, RuntimeError> {
    match builtin {
        BuiltinFunction::Print => {
            let sep = kwargs
                .remove("sep")
                .map(|value| format_value(&value))
                .unwrap_or_else(|| " ".to_string());
            let end = kwargs
                .remove("end")
                .map(|value| format_value(&value))
                .unwrap_or_else(|| "\n".to_string());
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "print() got an unexpected keyword argument",
                ));
            }
            let mut parts = Vec::new();
            for value in args {
                parts.push(format_value(&value));
            }
            print!("{}{}", parts.join(&sep), end);
            Ok(Value::None)
        }
        BuiltinFunction::Len => {
            if let Some(value) = kwargs.remove("obj") {
                if !args.is_empty() {
                    return Err(RuntimeError::new("len() got multiple values"));
                }
                args.push(value);
            }
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "len() got an unexpected keyword argument",
                ));
            }
            builtin.call(heap, args)
        }
        BuiltinFunction::Range => {
            let mut start = kwargs.remove("start");
            let mut stop = kwargs.remove("stop");
            let mut step = kwargs.remove("step");
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "range() got an unexpected keyword argument",
                ));
            }

            match args.len() {
                0 => {}
                1 => {
                    if stop.is_some() {
                        return Err(RuntimeError::new("range() got multiple values"));
                    }
                    stop = Some(args.remove(0));
                }
                2 => {
                    if start.is_some() || stop.is_some() {
                        return Err(RuntimeError::new("range() got multiple values"));
                    }
                    start = Some(args.remove(0));
                    stop = Some(args.remove(0));
                }
                3 => {
                    if start.is_some() || stop.is_some() || step.is_some() {
                        return Err(RuntimeError::new("range() got multiple values"));
                    }
                    start = Some(args.remove(0));
                    stop = Some(args.remove(0));
                    step = Some(args.remove(0));
                }
                _ => return Err(RuntimeError::new("range() expects 1-3 arguments")),
            }

            let stop = stop.ok_or_else(|| RuntimeError::new("range() missing stop"))?;
            let start = start.unwrap_or(Value::Int(0));
            let step = step.unwrap_or(Value::Int(1));

            let start = value_to_int(start)?;
            let stop = value_to_int(stop)?;
            let step = value_to_int(step)?;

            if step == 0 {
                return Err(RuntimeError::new("range() step cannot be zero"));
            }

            let mut values = Vec::new();
            let mut i = start;
            if step > 0 {
                while i < stop {
                    values.push(Value::Int(i));
                    i += step;
                }
            } else {
                while i > stop {
                    values.push(Value::Int(i));
                    i += step;
                }
            }

            Ok(heap.alloc_list(values))
        }
        BuiltinFunction::Sum => {
            let start = kwargs.remove("start");
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "sum() got an unexpected keyword argument",
                ));
            }
            if let Some(value) = start {
                if args.len() != 1 {
                    return Err(RuntimeError::new("sum() got multiple values"));
                }
                args.push(value);
            }
            builtin.call(heap, args)
        }
        BuiltinFunction::Sorted => {
            let reverse = kwargs
                .remove("reverse")
                .map(|value| is_truthy(&value))
                .unwrap_or(false);
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "sorted() got an unexpected keyword argument",
                ));
            }
            let result = builtin.call(heap, args)?;
            if reverse {
                match result {
                    Value::List(obj) => {
                        if let Object::List(values) = &mut *obj.kind_mut() {
                            values.reverse();
                        }
                        Ok(Value::List(obj))
                    }
                    other => Ok(other),
                }
            } else {
                Ok(result)
            }
        }
        BuiltinFunction::Enumerate => {
            let start = kwargs.remove("start");
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "enumerate() got an unexpected keyword argument",
                ));
            }
            if let Some(value) = start {
                if args.len() != 1 {
                    return Err(RuntimeError::new("enumerate() got multiple values"));
                }
                args.push(value);
            }
            builtin.call(heap, args)
        }
        _ => {
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "keyword arguments not supported for builtin",
                ));
            }
            builtin.call(heap, args)
        }
    }
}

fn decode_call_counts(arg: u32) -> (usize, usize) {
    let pos = (arg & 0xFFFF) as usize;
    let kw = (arg >> 16) as usize;
    (pos, kw)
}

fn class_attr_lookup(class: &ObjRef, name: &str) -> Option<Value> {
    let class_kind = class.kind();
    let (attrs, bases) = match &*class_kind {
        Object::Class(class_data) => (&class_data.attrs, &class_data.bases),
        _ => return None,
    };
    if let Some(value) = attrs.get(name).cloned() {
        return Some(value);
    }
    for base in bases {
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
    if message.contains("__init__() should return None") {
        return "TypeError";
    }
    if message.contains("unsupported operand type") || message.contains("expects") {
        return "TypeError";
    }
    "RuntimeError"
}

fn add_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return Ok(Value::Int(left + right));
    }

    match (left, right) {
        (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
        (Value::List(a), Value::List(b)) => match (&*a.kind(), &*b.kind()) {
            (Object::List(left), Object::List(right)) => {
                let mut result = left.clone();
                result.extend(right.clone());
                Ok(heap.alloc_list(result))
            }
            _ => Err(RuntimeError::new("unsupported operand type for +")),
        },
        (Value::Tuple(a), Value::Tuple(b)) => match (&*a.kind(), &*b.kind()) {
            (Object::Tuple(left), Object::Tuple(right)) => {
                let mut result = left.clone();
                result.extend(right.clone());
                Ok(heap.alloc_tuple(result))
            }
            _ => Err(RuntimeError::new("unsupported operand type for +")),
        },
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
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => Ok(values.iter().any(|value| value == left)),
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => Ok(values.iter().any(|value| value == left)),
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(entries) => Ok(entries.iter().any(|(key, _)| key == left)),
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::Str(haystack) => match left {
            Value::Str(needle) => Ok(haystack.contains(needle)),
            _ => Err(RuntimeError::new("in expects string on left")),
        },
        _ => Err(RuntimeError::new("unsupported operand type for in")),
    }
}

fn mul_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
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
        (Value::List(obj), other) | (other, Value::List(obj)) => {
            let count = value_to_int(other)?;
            if count <= 0 {
                return Ok(heap.alloc_list(Vec::new()));
            }
            let values = match &*obj.kind() {
                Object::List(values) => values.clone(),
                _ => return Err(RuntimeError::new("unsupported operand type for *")),
            };
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(heap.alloc_list(result))
        }
        (Value::Tuple(obj), other) | (other, Value::Tuple(obj)) => {
            let count = value_to_int(other)?;
            if count <= 0 {
                return Ok(heap.alloc_tuple(Vec::new()));
            }
            let values = match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::new("unsupported operand type for *")),
            };
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(heap.alloc_tuple(result))
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
