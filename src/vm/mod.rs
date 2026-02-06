//! Bytecode virtual machine (minimal subset).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::bytecode::cpython;
use crate::bytecode::{CodeObject, Opcode};
use crate::compiler;
use crate::parser;
use crate::runtime::{
    BoundMethod, BuiltinFunction, ClassObject, ExceptionObject, FunctionObject, GeneratorObject,
    Heap, InstanceObject, IteratorKind, IteratorObject, ModuleObject, NativeMethodKind,
    NativeMethodObject, ObjRef, Object, RuntimeError, Value, format_value,
};

#[derive(Debug, Clone)]
struct Block {
    handler: usize,
    stack_len: usize,
}

#[derive(Debug, Clone)]
struct TraceFrame {
    filename: String,
    line: usize,
    column: usize,
    name: String,
}

struct ModuleSourceInfo {
    path: PathBuf,
    is_package: bool,
    package_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneratorResumeKind {
    Next,
    Throw,
    Close,
}

enum NativeCallResult {
    Value(Value),
    PropagatedException,
}

enum GeneratorResumeOutcome {
    Yield(Value),
    Complete(Value),
    PropagatedException,
}

struct Frame {
    code: Rc<CodeObject>,
    ip: usize,
    last_ip: usize,
    stack: Vec<Value>,
    locals: HashMap<String, Value>,
    cells: Vec<ObjRef>,
    module: ObjRef,
    function_globals: ObjRef,
    globals_fallback: Option<ObjRef>,
    is_module: bool,
    return_module: bool,
    discard_result: bool,
    return_instance: Option<ObjRef>,
    return_class: bool,
    class_bases: Vec<ObjRef>,
    blocks: Vec<Block>,
    active_exception: Option<Value>,
    expect_none_return: bool,
    generator_owner: Option<ObjRef>,
    generator_awaiting_resume_value: bool,
    generator_resume_value: Option<Value>,
    generator_pending_throw: Option<Value>,
    generator_resume_kind: Option<GeneratorResumeKind>,
    yield_from_iter: Option<Value>,
}

impl Frame {
    fn new(
        code: Rc<CodeObject>,
        module: ObjRef,
        is_module: bool,
        return_module: bool,
        cells: Vec<ObjRef>,
    ) -> Self {
        Self {
            code,
            ip: 0,
            last_ip: 0,
            stack: Vec::new(),
            locals: HashMap::new(),
            cells,
            module: module.clone(),
            function_globals: module,
            globals_fallback: None,
            is_module,
            return_module,
            discard_result: false,
            return_instance: None,
            return_class: false,
            class_bases: Vec::new(),
            blocks: Vec::new(),
            active_exception: None,
            expect_none_return: false,
            generator_owner: None,
            generator_awaiting_resume_value: false,
            generator_resume_value: None,
            generator_pending_throw: None,
            generator_resume_kind: None,
            yield_from_iter: None,
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
    generator_states: HashMap<u64, Frame>,
    generator_returns: HashMap<u64, Value>,
    pending_generator_exception: Option<Value>,
    active_generator_resume: Option<u64>,
    active_generator_resume_boundary: Option<usize>,
    generator_resume_outcome: Option<GeneratorResumeOutcome>,
}

impl Vm {
    pub fn new() -> Self {
        let heap = Heap::new();
        let main_module = match heap.alloc_module(ModuleObject::new("__main__")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };

        let mut modules = HashMap::new();
        modules.insert("__main__".to_string(), main_module.clone());

        let module_paths = vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))];

        let mut vm = Self {
            frames: Vec::new(),
            builtins: HashMap::new(),
            modules,
            main_module,
            module_paths,
            heap,
            generator_states: HashMap::new(),
            generator_returns: HashMap::new(),
            pending_generator_exception: None,
            active_generator_resume: None,
            active_generator_resume_boundary: None,
            generator_resume_outcome: None,
        };
        let main = vm.main_module.clone();
        vm.set_module_metadata(&main, "__main__", None, false, None);
        vm.install_sys_module();
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
        self.sync_sys_path_from_module_paths();
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

    fn build_cells(&mut self, code: &CodeObject, closure: Vec<ObjRef>) -> Vec<ObjRef> {
        let mut cells = Vec::with_capacity(code.cellvars.len() + code.freevars.len());
        for _ in &code.cellvars {
            cells.push(self.heap.alloc_cell_obj(None));
        }
        cells.extend(closure);
        cells
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
            for cell in &frame.cells {
                roots.push(Value::Cell(cell.clone()));
            }
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
        for frame in self.generator_states.values() {
            roots.extend(frame.stack.iter().cloned());
            roots.extend(frame.locals.values().cloned());
            for cell in &frame.cells {
                roots.push(Value::Cell(cell.clone()));
            }
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
            if let Some(iter) = &frame.yield_from_iter {
                roots.push(iter.clone());
            }
            if let Some(value) = &frame.generator_resume_value {
                roots.push(value.clone());
            }
            if let Some(exc) = &frame.generator_pending_throw {
                roots.push(exc.clone());
            }
        }
        roots.extend(self.generator_returns.values().cloned());
        self.heap.collect_cycles(&roots);
    }

    pub fn execute(&mut self, code: &CodeObject) -> Result<Value, RuntimeError> {
        self.frames.clear();
        self.generator_states.clear();
        self.generator_returns.clear();
        self.pending_generator_exception = None;
        self.active_generator_resume = None;
        self.active_generator_resume_boundary = None;
        self.generator_resume_outcome = None;
        let code = Rc::new(code.clone());
        let cells = self.build_cells(&code, Vec::new());
        self.frames.push(Frame::new(
            code,
            self.main_module.clone(),
            true,
            false,
            cells,
        ));
        self.run()
    }

    pub fn execute_pyc_bytes(&mut self, bytes: &[u8]) -> Result<Value, RuntimeError> {
        let pyc = cpython::load_pyc(bytes).map_err(|err| RuntimeError::new(err.message))?;
        let code = cpython::translate_code(&pyc, &mut self.heap)
            .map_err(|err| RuntimeError::new(err.message))?;
        self.execute(&code)
    }

    pub fn execute_pyc_file(&mut self, path: &str) -> Result<Value, RuntimeError> {
        let bytes = std::fs::read(path)
            .map_err(|err| RuntimeError::new(format!("failed to read {path}: {err}")))?;
        self.execute_pyc_bytes(&bytes)
    }

    fn install_sys_module(&mut self) {
        let sys_module = match self.heap.alloc_module(ModuleObject::new("sys")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(&sys_module, "sys", None, false, None);
        if let Object::Module(module_data) = &mut *sys_module.kind_mut() {
            module_data
                .globals
                .insert("path".to_string(), self.heap.alloc_list(Vec::new()));
            module_data
                .globals
                .insert("meta_path".to_string(), self.heap.alloc_list(Vec::new()));
            module_data
                .globals
                .insert("path_hooks".to_string(), self.heap.alloc_list(Vec::new()));
            module_data.globals.insert(
                "path_importer_cache".to_string(),
                self.heap.alloc_dict(Vec::new()),
            );
            module_data
                .globals
                .insert("modules".to_string(), self.heap.alloc_dict(Vec::new()));
        }
        self.register_module("sys", sys_module);
        self.sync_sys_path_from_module_paths();
        self.refresh_sys_modules_dict();
    }

    fn sync_sys_path_from_module_paths(&mut self) {
        let sys_module = match self.modules.get("sys").cloned() {
            Some(module) => module,
            None => return,
        };
        let values = self
            .module_paths
            .iter()
            .map(|path| Value::Str(path.to_string_lossy().to_string()))
            .collect::<Vec<_>>();
        if let Object::Module(module_data) = &mut *sys_module.kind_mut() {
            module_data
                .globals
                .insert("path".to_string(), self.heap.alloc_list(values));
        }
    }

    fn sync_module_paths_from_sys(&mut self) {
        let sys_module = match self.modules.get("sys").cloned() {
            Some(module) => module,
            None => return,
        };
        let path_value = match &*sys_module.kind() {
            Object::Module(module_data) => module_data.globals.get("path").cloned(),
            _ => None,
        };
        let Some(Value::List(path_list)) = path_value else {
            return;
        };

        let mut new_paths = Vec::new();
        if let Object::List(values) = &*path_list.kind() {
            for value in values {
                if let Value::Str(path) = value {
                    new_paths.push(PathBuf::from(path));
                }
            }
        }
        self.module_paths = new_paths;
    }

    fn refresh_sys_modules_dict(&mut self) {
        let sys_module = match self.modules.get("sys").cloned() {
            Some(module) => module,
            None => return,
        };
        let mut entries = Vec::with_capacity(self.modules.len());
        for (name, module) in self.modules.iter() {
            entries.push((Value::Str(name.clone()), Value::Module(module.clone())));
        }
        let modules_dict = self.heap.alloc_dict(entries);
        if let Object::Module(module_data) = &mut *sys_module.kind_mut() {
            module_data
                .globals
                .insert("modules".to_string(), modules_dict);
        }
    }

    fn register_module(&mut self, name: &str, module: ObjRef) {
        self.modules.insert(name.to_string(), module);
        self.refresh_sys_modules_dict();
    }

    fn load_module(&mut self, name: &str) -> Result<ObjRef, RuntimeError> {
        if let Some(module) = self.modules.get(name).cloned() {
            return Ok(module);
        }

        if let Some((parent, _)) = name.rsplit_once('.') {
            if !self.modules.contains_key(parent) {
                if let Some(parent_info) = self.find_module_source(parent) {
                    if parent_info.is_package {
                        let _ = self.load_module(parent)?;
                    }
                }
            }
        }

        let source_info = self
            .find_module_source(name)
            .ok_or_else(|| RuntimeError::new(format!("module '{name}' not found")))?;

        let source = std::fs::read_to_string(&source_info.path)
            .map_err(|err| RuntimeError::new(format!("failed to read module '{name}': {err}")))?;

        let module = match self.heap.alloc_module(ModuleObject::new(name)) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &module,
            name,
            Some(&source_info.path),
            source_info.is_package,
            source_info.package_dir.as_ref(),
        );

        self.register_module(name, module.clone());
        self.link_module_chain(name, module.clone());

        let module_ast = parser::parse_module(&source).map_err(|err| {
            RuntimeError::new(format!(
                "parse error in module '{name}' at {}: {}",
                err.offset, err.message
            ))
        })?;
        let code =
            compiler::compile_module_with_filename(&module_ast, &source_info.path.to_string_lossy())
                .map_err(|err| {
                    RuntimeError::new(format!("compile error in module '{name}': {}", err.message))
                })?;
        let code = Rc::new(code);
        let cells = self.build_cells(&code, Vec::new());
        let mut frame = Frame::new(code, module.clone(), true, false, cells);
        frame.discard_result = true;
        self.frames.push(frame);
        Ok(module)
    }

    fn find_module_source(&mut self, name: &str) -> Option<ModuleSourceInfo> {
        self.sync_module_paths_from_sys();
        let rel_name = name.replace('.', "/");
        let filename = format!("{rel_name}.py");
        for base in &self.module_paths {
            let candidate = base.join(&filename);
            if candidate.exists() {
                return Some(ModuleSourceInfo {
                    path: candidate,
                    is_package: false,
                    package_dir: None,
                });
            }
            let package_dir = base.join(&rel_name);
            let package_init = package_dir.join("__init__.py");
            if package_init.exists() {
                return Some(ModuleSourceInfo {
                    path: package_init,
                    is_package: true,
                    package_dir: Some(package_dir),
                });
            }
        }
        None
    }

    fn find_module_file(&mut self, name: &str) -> Option<PathBuf> {
        self.find_module_source(name).map(|info| info.path)
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
        self.set_module_metadata(&module, name, None, false, None);
        self.register_module(name, module.clone());
        module
    }

    fn set_module_metadata(
        &mut self,
        module: &ObjRef,
        name: &str,
        origin: Option<&PathBuf>,
        is_package: bool,
        package_dir: Option<&PathBuf>,
    ) {
        let package_name = if is_package {
            name.to_string()
        } else {
            name.rsplit_once('.')
                .map(|(parent, _)| parent.to_string())
                .unwrap_or_default()
        };
        let parent = name
            .rsplit_once('.')
            .map(|(parent, _)| parent.to_string())
            .unwrap_or_default();
        let loader_value = if origin.is_some() {
            Value::Str("pyrs.SourceFileLoader".to_string())
        } else {
            Value::None
        };
        let origin_value = origin
            .map(|path| Value::Str(path.to_string_lossy().to_string()))
            .unwrap_or(Value::None);
        let submodule_locations = if is_package {
            let mut entries = Vec::new();
            if let Some(dir) = package_dir {
                entries.push(Value::Str(dir.to_string_lossy().to_string()));
            }
            self.heap.alloc_list(entries)
        } else {
            Value::None
        };
        let spec_value = self.heap.alloc_dict(vec![
            (Value::Str("name".to_string()), Value::Str(name.to_string())),
            (Value::Str("origin".to_string()), origin_value.clone()),
            (Value::Str("loader".to_string()), loader_value.clone()),
            (Value::Str("parent".to_string()), Value::Str(parent)),
            (
                Value::Str("submodule_search_locations".to_string()),
                submodule_locations.clone(),
            ),
            (
                Value::Str("is_package".to_string()),
                Value::Bool(is_package),
            ),
        ]);

        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data
                .globals
                .insert("__name__".to_string(), Value::Str(name.to_string()));
            module_data
                .globals
                .insert("__package__".to_string(), Value::Str(package_name));
            module_data
                .globals
                .insert("__loader__".to_string(), loader_value);
            module_data
                .globals
                .insert("__spec__".to_string(), spec_value);
            if origin.is_some() {
                module_data.globals.insert("__file__".to_string(), origin_value);
            }
            if is_package {
                module_data
                    .globals
                    .insert("__path__".to_string(), submodule_locations);
            }
        }
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

    fn import_module_object(&mut self, name: &str) -> Result<ObjRef, RuntimeError> {
        if let Some(module) = self.modules.get(name).cloned() {
            Ok(module)
        } else {
            self.load_module(name)
        }
    }

    fn module_for_plain_import(&mut self, name: &str, module: ObjRef) -> ObjRef {
        if let Some((root, _)) = name.split_once('.') {
            self.link_module_chain(name, module);
            self.ensure_module(root)
        } else {
            module
        }
    }

    fn fromlist_requested(&self, fromlist: &Value) -> bool {
        match fromlist {
            Value::None => false,
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => !values.is_empty(),
                _ => true,
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => !values.is_empty(),
                _ => true,
            },
            _ => true,
        }
    }

    fn import_package_context(&self) -> Option<String> {
        let frame = self.frames.last()?;
        let module_ref = frame.module.kind();
        let module = match &*module_ref {
            Object::Module(module) => module,
            _ => return None,
        };
        if let Some(Value::Str(package)) = module.globals.get("__package__") {
            return Some(package.clone());
        }
        if module.globals.contains_key("__path__") {
            return Some(module.name.clone());
        }
        Some(
            module
                .name
                .rsplit_once('.')
                .map(|(parent, _)| parent.to_string())
                .unwrap_or_default(),
        )
    }

    fn resolve_import_name(&self, requested: &str, level: usize) -> Result<String, RuntimeError> {
        if level == 0 {
            return Ok(requested.to_string());
        }

        let package = self
            .import_package_context()
            .ok_or_else(|| RuntimeError::new("relative import outside module context"))?;
        if package.is_empty() {
            return Err(RuntimeError::new(
                "attempted relative import with no known parent package",
            ));
        }

        let mut parts: Vec<&str> = package.split('.').collect();
        let trim = level.saturating_sub(1);
        if trim > parts.len() {
            return Err(RuntimeError::new(
                "attempted relative import beyond top-level package",
            ));
        }
        parts.truncate(parts.len() - trim);

        let mut resolved = parts.join(".");
        if !requested.is_empty() {
            if !resolved.is_empty() {
                resolved.push('.');
            }
            resolved.push_str(requested);
        }
        Ok(resolved)
    }

    fn run(&mut self) -> Result<Value, RuntimeError> {
        loop {
            if self.frames.is_empty() {
                return Ok(Value::None);
            }
            if let Some(target) = self.active_generator_resume {
                if self.generator_resume_outcome.is_some() {
                    return Ok(Value::None);
                }
                let target_active = self.frames.iter().any(|frame| {
                    frame
                        .generator_owner
                        .as_ref()
                        .map(|owner| owner.id() == target)
                        .unwrap_or(false)
                });
                if !target_active {
                    self.generator_resume_outcome =
                        Some(GeneratorResumeOutcome::PropagatedException);
                    return Ok(Value::None);
                }
            }

            let pending_resume = {
                let frame = self.frames.last_mut().expect("frame exists");
                if frame.generator_owner.is_some() && frame.generator_awaiting_resume_value {
                    frame.generator_awaiting_resume_value = false;
                    let thrown = frame.generator_pending_throw.take();
                    let sent = frame.generator_resume_value.take().unwrap_or(Value::None);
                    Some((thrown, sent))
                } else {
                    None
                }
            };
            if let Some((thrown, sent)) = pending_resume {
                if let Some(exc) = thrown {
                    self.raise_exception(exc)?;
                    continue;
                }
                self.push_value(sent);
            }

            let should_return = {
                let frame = self.frames.last().expect("frame exists");
                frame.ip >= frame.code.instructions.len()
            };

            if should_return {
                let frame = self.frames.pop().expect("frame exists");
                if let Some(owner) = frame.generator_owner {
                    self.finish_generator_resume(owner, Value::None);
                    continue;
                }
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
                    if !frame.discard_result {
                        caller.stack.push(value);
                    }
                    continue;
                }
                return Ok(value);
            }

            let instr = {
                let frame = self.frames.last_mut().expect("frame exists");
                frame.last_ip = frame.ip;
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
                        let value =
                            {
                                let frame = self.frames.last().expect("frame exists");
                                frame.code.constants.get(idx).cloned().ok_or_else(|| {
                                    RuntimeError::new("constant index out of range")
                                })?
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
                    Opcode::LoadLocals => {
                        let value = self.builtin_locals(Vec::new(), HashMap::new())?;
                        self.push_value(value);
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
                    Opcode::LoadDeref => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing deref argument"))?
                            as usize;
                        let value = self.load_deref(idx)?;
                        self.push_value(value);
                    }
                    Opcode::LoadClosure => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing closure argument"))?
                            as usize;
                        let cell = self.get_cell(idx)?;
                        self.push_value(Value::Cell(cell));
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
                        let first_value = first_value.ok_or_else(|| {
                            RuntimeError::new(format!("local '{first_name}' not set"))
                        })?;
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
                        .ok_or_else(|| {
                            RuntimeError::new(format!("name '{name}' is not defined"))
                        })?;
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
                                let caller_idx = self.frames.len().saturating_sub(1);
                                let (module_name, attr) = match &*module.kind() {
                                    Object::Module(module_data) => {
                                        let attr = module_data.globals.get(&attr_name).cloned();
                                        let module_name = module_data.name.clone();
                                        (module_name, attr)
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attribute access unsupported type",
                                        ));
                                    }
                                };
                                let attr = if let Some(attr) = attr {
                                    Some(attr)
                                } else {
                                    module_name.split('.').last().and_then(|suffix| {
                                        if suffix == attr_name {
                                            Some(Value::Module(module.clone()))
                                        } else {
                                            None
                                        }
                                    })
                                }
                                .or_else(|| {
                                    self.load_submodule(&module, &attr_name).map(Value::Module)
                                })
                                .ok_or_else(|| {
                                    RuntimeError::new(format!(
                                    "module '{}' has no attribute '{}'",
                                    module_name, attr_name
                                ))
                                })?;
                                let frame = self
                                    .frames
                                    .get_mut(caller_idx)
                                    .ok_or_else(|| RuntimeError::new("attribute caller frame missing"))?;
                                if push_null {
                                    frame.stack.push(Value::None);
                                }
                                frame.stack.push(attr);
                            }
                            Value::Class(class) => {
                                let class_name = match &*class.kind() {
                                    Object::Class(class_data) => class_data.name.clone(),
                                    _ => "<class>".to_string(),
                                };
                                let attr =
                                    class_attr_lookup(&class, &attr_name).ok_or_else(|| {
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
                                    if let Some(attr) = instance_data.attrs.get(&attr_name).cloned()
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
                                        ));
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
                            Value::Function(func) => {
                                if attr_name == "__annotations__" {
                                    let annotations = {
                                        let mut func_ref = func.kind_mut();
                                        match &mut *func_ref {
                                            Object::Function(func_data) => {
                                                if let Some(obj) = &func_data.annotations {
                                                    obj.clone()
                                                } else {
                                                    let dict = self.heap.alloc_dict(Vec::new());
                                                    let obj = match dict {
                                                        Value::Dict(obj) => obj,
                                                        _ => unreachable!(),
                                                    };
                                                    func_data.annotations = Some(obj.clone());
                                                    obj
                                                }
                                            }
                                            _ => {
                                                return Err(RuntimeError::new(
                                                    "attribute access unsupported type",
                                                ));
                                            }
                                        }
                                    };
                                    if push_null {
                                        self.push_value(Value::None);
                                    }
                                    self.push_value(Value::Dict(annotations));
                                } else {
                                    return Err(RuntimeError::new(format!(
                                        "function has no attribute '{}'",
                                        attr_name
                                    )));
                                }
                            }
                            Value::Generator(generator) => {
                                let kind = match attr_name.as_str() {
                                    "__iter__" => NativeMethodKind::GeneratorIter,
                                    "__next__" => NativeMethodKind::GeneratorNext,
                                    "send" => NativeMethodKind::GeneratorSend,
                                    "throw" => NativeMethodKind::GeneratorThrow,
                                    "close" => NativeMethodKind::GeneratorClose,
                                    _ => {
                                        return Err(RuntimeError::new(format!(
                                            "generator has no attribute '{}'",
                                            attr_name
                                        )));
                                    }
                                };
                                let native =
                                    self.heap.alloc_native_method(NativeMethodObject::new(kind));
                                let bound = BoundMethod::new(native, generator);
                                let bound_value = self.heap.alloc_bound_method(bound);
                                if push_null {
                                    self.push_value(Value::None);
                                }
                                self.push_value(bound_value);
                            }
                            _ => {
                                return Err(RuntimeError::new("attribute access unsupported type"));
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
                    Opcode::StoreDeref => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing deref argument"))?
                            as usize;
                        let value = self.pop_value()?;
                        self.store_deref(idx, value)?;
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
                            let value =
                                frame.locals.get(&second_name).cloned().ok_or_else(|| {
                                    RuntimeError::new(format!("local '{second_name}' not set"))
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
                            Value::Function(func) => {
                                if attr_name != "__annotations__" {
                                    return Err(RuntimeError::new(
                                        "attribute assignment unsupported type",
                                    ));
                                }
                                let annotations = match value {
                                    Value::Dict(obj) => obj,
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "function __annotations__ must be dict",
                                        ));
                                    }
                                };
                                if let Object::Function(func_data) = &mut *func.kind_mut() {
                                    func_data.annotations = Some(annotations);
                                }
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "attribute assignment unsupported type",
                                ));
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
                            Value::Function(func) => {
                                if attr_name != "__annotations__" {
                                    return Err(RuntimeError::new(
                                        "attribute assignment unsupported type",
                                    ));
                                }
                                let annotations = match value {
                                    Value::Dict(obj) => obj,
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "function __annotations__ must be dict",
                                        ));
                                    }
                                };
                                if let Object::Function(func_data) = &mut *func.kind_mut() {
                                    func_data.annotations = Some(annotations);
                                }
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "attribute assignment unsupported type",
                                ));
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
                            if let Object::Module(module_data) =
                                &mut *frame.function_globals.kind_mut()
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
                            Value::Bool(value) => {
                                self.push_value(Value::Int(if value { 1 } else { 0 }))
                            }
                            _ => return Err(RuntimeError::new("unsupported operand type for +")),
                        }
                    }
                    Opcode::ToBool => {
                        let value = self.pop_value()?;
                        self.push_value(Value::Bool(is_truthy(&value)));
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
                                _ => return Err(RuntimeError::new("unpack expects list or tuple")),
                            },
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(values) => values.clone(),
                                _ => return Err(RuntimeError::new("unpack expects list or tuple")),
                            },
                            _ => return Err(RuntimeError::new("unpack expects list or tuple")),
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
                                                ));
                                            }
                                        },
                                        Value::Tuple(items) => match &*items.kind() {
                                            Object::Tuple(items) => values.extend(items.clone()),
                                            _ => {
                                                return Err(RuntimeError::new(
                                                    "list extend expects iterable",
                                                ));
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
                                            ));
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
                                    _ => return Err(RuntimeError::new("dict update expects dict")),
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
                                    _ => {
                                        return Err(RuntimeError::new("subscript unsupported type"));
                                    }
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
                                    _ => {
                                        return Err(RuntimeError::new("subscript unsupported type"));
                                    }
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
                                            return Err(RuntimeError::new(
                                                "list index out of range",
                                            ));
                                        }
                                        self.push_value(values[index_int as usize].clone());
                                    }
                                    _ => {
                                        return Err(RuntimeError::new("subscript unsupported type"));
                                    }
                                },
                                Value::Tuple(obj) => match &*obj.kind() {
                                    Object::Tuple(values) => {
                                        let mut index_int = value_to_int(index)? as isize;
                                        if index_int < 0 {
                                            index_int += values.len() as isize;
                                        }
                                        if index_int < 0 || index_int as usize >= values.len() {
                                            return Err(RuntimeError::new(
                                                "tuple index out of range",
                                            ));
                                        }
                                        self.push_value(values[index_int as usize].clone());
                                    }
                                    _ => {
                                        return Err(RuntimeError::new("subscript unsupported type"));
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
                                    self.push_value(Value::Str(
                                        chars[index_int as usize].to_string(),
                                    ));
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
                                        return Err(RuntimeError::new("subscript unsupported type"));
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
                                return Err(RuntimeError::new("slice assignment not supported"));
                            }
                            index => match target {
                                Value::List(obj) => {
                                    if let Object::List(values) = &mut *obj.kind_mut() {
                                        let mut idx = value_to_int(index)? as isize;
                                        if idx < 0 {
                                            idx += values.len() as isize;
                                        }
                                        if idx < 0 || idx as usize >= values.len() {
                                            return Err(RuntimeError::new(
                                                "list index out of range",
                                            ));
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
                                _ => {
                                    return Err(RuntimeError::new(
                                        "store subscript unsupported type",
                                    ));
                                }
                            },
                        }
                    }
                    Opcode::MakeFunction => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing function argument"))?
                            as usize;
                        let value =
                            {
                                let frame = self.frames.last().expect("frame exists");
                                frame.code.constants.get(idx).cloned().ok_or_else(|| {
                                    RuntimeError::new("constant index out of range")
                                })?
                            };
                        let code = match value {
                            Value::Code(code) => code,
                            _ => {
                                return Err(RuntimeError::new("expected code object for function"));
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
                                                ));
                                            }
                                        };
                                        map.insert(key, value.clone());
                                    }
                                    map
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "expected kwonly defaults dict for function",
                                    ));
                                }
                            },
                            _ => {
                                return Err(RuntimeError::new(
                                    "expected kwonly defaults dict for function",
                                ));
                            }
                        };
                        let defaults_value = self.pop_value()?;
                        let defaults = match defaults_value {
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(values) => values.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "expected defaults tuple for function",
                                    ));
                                }
                            },
                            _ => {
                                return Err(RuntimeError::new(
                                    "expected defaults tuple for function",
                                ));
                            }
                        };
                        let module = self
                            .frames
                            .last()
                            .expect("frame exists")
                            .function_globals
                            .clone();
                        let func = FunctionObject::new(
                            code,
                            module,
                            defaults,
                            kwonly_defaults,
                            Vec::new(),
                            None,
                        );
                        self.push_value(self.heap.alloc_function(func));
                    }
                    Opcode::BuildClass => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing class code argument"))?
                            as usize;
                        let value =
                            {
                                let frame = self.frames.last().expect("frame exists");
                                frame.code.constants.get(idx).cloned().ok_or_else(|| {
                                    RuntimeError::new("constant index out of range")
                                })?
                            };
                        let code = match value {
                            Value::Code(code) => code,
                            _ => {
                                return Err(RuntimeError::new(
                                    "expected code object for class body",
                                ));
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
                                    ));
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
                        let cells = self.build_cells(&code, Vec::new());
                        let mut frame = Frame::new(code, class_module, true, false, cells);
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
                                return Err(RuntimeError::new("expected code object for function"));
                            }
                        };
                        let module = self
                            .frames
                            .last()
                            .expect("frame exists")
                            .function_globals
                            .clone();
                        let func = FunctionObject::new(
                            code,
                            module,
                            Vec::new(),
                            HashMap::new(),
                            Vec::new(),
                            None,
                        );
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
                                            return Err(RuntimeError::new("defaults must be tuple"));
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
                                                        ));
                                                    }
                                                };
                                                map.insert(name, value.clone());
                                            }
                                            map
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "kwonly defaults must be dict",
                                            ));
                                        }
                                    },
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "kwonly defaults must be dict",
                                        ));
                                    }
                                };
                                if let Object::Function(func_data) = &mut *func.kind_mut() {
                                    func_data.kwonly_defaults = kwonly;
                                }
                            }
                            0x04 => {
                                let annotations = match attr {
                                    Value::Dict(obj) => obj,
                                    _ => return Err(RuntimeError::new("annotations must be dict")),
                                };
                                if let Object::Function(func_data) = &mut *func.kind_mut() {
                                    func_data.annotations = Some(annotations);
                                }
                            }
                            0x08 => {
                                let closure = match attr {
                                    Value::Tuple(obj) => match &*obj.kind() {
                                        Object::Tuple(values) => {
                                            let mut cells = Vec::with_capacity(values.len());
                                            for value in values {
                                                match value {
                                                    Value::Cell(cell) => cells.push(cell.clone()),
                                                    _ => {
                                                        return Err(RuntimeError::new(
                                                            "closure entries must be cells",
                                                        ));
                                                    }
                                                }
                                            }
                                            cells
                                        }
                                        _ => {
                                            return Err(RuntimeError::new("closure must be tuple"));
                                        }
                                    },
                                    _ => return Err(RuntimeError::new("closure must be tuple")),
                                };
                                if let Object::Function(func_data) = &mut *func.kind_mut() {
                                    func_data.closure = closure;
                                }
                            }
                            _ => {
                                // ignore annotations for now
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
                                        ));
                                    }
                                };
                                self.push_function_call(&func_data, args, HashMap::new())?;
                            }
                            Value::BoundMethod(method) => {
                                let method_data = match &*method.kind() {
                                    Object::BoundMethod(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                };
                                match &*method_data.function.kind() {
                                    Object::Function(data) => {
                                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                                        bound_args
                                            .push(self.receiver_value(&method_data.receiver)?);
                                        bound_args.extend(args);
                                        self.push_function_call(data, bound_args, HashMap::new())?;
                                    }
                                    Object::NativeMethod(native) => {
                                        match self.call_native_method(
                                            native.kind,
                                            method_data.receiver.clone(),
                                            args,
                                            HashMap::new(),
                                        )? {
                                            NativeCallResult::Value(result) => {
                                                self.push_value(result)
                                            }
                                            NativeCallResult::PropagatedException => {
                                                self.propagate_pending_generator_exception()?;
                                            }
                                        }
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                }
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
                                            ));
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
                                    let cells = self
                                        .build_cells(&func_data.code, func_data.closure.clone());
                                    let mut frame = Frame::new(
                                        func_data.code.clone(),
                                        func_data.module.clone(),
                                        false,
                                        false,
                                        cells,
                                    );
                                    frame.return_instance = Some(instance);
                                    frame.expect_none_return = true;
                                    apply_bindings(
                                        &mut frame,
                                        &func_data.code,
                                        bindings,
                                        &self.heap,
                                    );
                                    self.frames.push(frame);
                                } else {
                                    self.push_value(Value::Instance(instance));
                                }
                            }
                            Value::Builtin(builtin) => {
                                let caller_idx = self.frames.len().saturating_sub(1);
                                let result = self.call_builtin(builtin, args, HashMap::new())?;
                                let frame = self
                                    .frames
                                    .get_mut(caller_idx)
                                    .ok_or_else(|| RuntimeError::new("builtin caller frame missing"))?;
                                frame.stack.push(result);
                            }
                            Value::ExceptionType(name) => {
                                let message = match args.as_slice() {
                                    [] => None,
                                    [value] => Some(format_value(value)),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "exception constructor expects at most one argument",
                                        ));
                                    }
                                };
                                self.push_value(Value::Exception(ExceptionObject {
                                    name,
                                    message,
                                }));
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
                                frame.code.constants.get(idx).cloned().ok_or_else(|| {
                                    RuntimeError::new("constant index out of range")
                                })?
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
                                                    ));
                                                }
                                            }
                                        }
                                        Some(names)
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "kw_names must be tuple of strings",
                                        ));
                                    }
                                },
                                Value::None => None,
                                _ => {
                                    return Err(RuntimeError::new(
                                        "kw_names must be tuple of strings",
                                    ));
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
                        if let Some(Value::None) = self
                            .frames
                            .last()
                            .and_then(|frame| frame.stack.last())
                            .cloned()
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
                                        ));
                                    }
                                };
                                self.push_function_call(&func_data, args, kwargs)?;
                            }
                            Value::BoundMethod(method) => {
                                let method_data = match &*method.kind() {
                                    Object::BoundMethod(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                };
                                match &*method_data.function.kind() {
                                    Object::Function(data) => {
                                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                                        bound_args
                                            .push(self.receiver_value(&method_data.receiver)?);
                                        bound_args.extend(args);
                                        self.push_function_call(data, bound_args, kwargs)?;
                                    }
                                    Object::NativeMethod(native) => {
                                        match self.call_native_method(
                                            native.kind,
                                            method_data.receiver.clone(),
                                            args,
                                            kwargs,
                                        )? {
                                            NativeCallResult::Value(result) => {
                                                self.push_value(result)
                                            }
                                            NativeCallResult::PropagatedException => {
                                                self.propagate_pending_generator_exception()?;
                                            }
                                        }
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                }
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
                                            ));
                                        }
                                    };
                                    let mut init_args = Vec::with_capacity(args.len() + 1);
                                    init_args.push(Value::Instance(instance.clone()));
                                    init_args.extend(args);
                                    let bindings =
                                        bind_arguments(&func_data, &self.heap, init_args, kwargs)?;
                                    let cells = self
                                        .build_cells(&func_data.code, func_data.closure.clone());
                                    let mut frame = Frame::new(
                                        func_data.code.clone(),
                                        func_data.module.clone(),
                                        false,
                                        false,
                                        cells,
                                    );
                                    frame.return_instance = Some(instance);
                                    frame.expect_none_return = true;
                                    apply_bindings(
                                        &mut frame,
                                        &func_data.code,
                                        bindings,
                                        &self.heap,
                                    );
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
                                let caller_idx = self.frames.len().saturating_sub(1);
                                let result = self.call_builtin(builtin, args, kwargs)?;
                                let frame = self
                                    .frames
                                    .get_mut(caller_idx)
                                    .ok_or_else(|| RuntimeError::new("builtin caller frame missing"))?;
                                frame.stack.push(result);
                            }
                            Value::ExceptionType(name) => {
                                let message = match args.as_slice() {
                                    [] => None,
                                    [value] => Some(format_value(value)),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "exception constructor expects at most one argument",
                                        ));
                                    }
                                };
                                self.push_value(Value::Exception(ExceptionObject {
                                    name,
                                    message,
                                }));
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
                                                ));
                                            }
                                        }
                                    }
                                    names
                                }
                                _ => return Err(RuntimeError::new("kw names must be tuple")),
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
                        if let Some(Value::None) = self
                            .frames
                            .last()
                            .and_then(|frame| frame.stack.last())
                            .cloned()
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
                                        ));
                                    }
                                };
                                self.push_function_call(&func_data, args, kwargs)?;
                            }
                            Value::BoundMethod(method) => {
                                let method_data = match &*method.kind() {
                                    Object::BoundMethod(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                };
                                match &*method_data.function.kind() {
                                    Object::Function(data) => {
                                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                                        bound_args
                                            .push(self.receiver_value(&method_data.receiver)?);
                                        bound_args.extend(args);
                                        self.push_function_call(data, bound_args, kwargs)?;
                                    }
                                    Object::NativeMethod(native) => {
                                        match self.call_native_method(
                                            native.kind,
                                            method_data.receiver.clone(),
                                            args,
                                            kwargs,
                                        )? {
                                            NativeCallResult::Value(result) => {
                                                self.push_value(result)
                                            }
                                            NativeCallResult::PropagatedException => {
                                                self.propagate_pending_generator_exception()?;
                                            }
                                        }
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                }
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
                                            ));
                                        }
                                    };
                                    let mut init_args = Vec::with_capacity(args.len() + 1);
                                    init_args.push(Value::Instance(instance.clone()));
                                    init_args.extend(args);
                                    let bindings =
                                        bind_arguments(&func_data, &self.heap, init_args, kwargs)?;
                                    let cells = self
                                        .build_cells(&func_data.code, func_data.closure.clone());
                                    let mut frame = Frame::new(
                                        func_data.code.clone(),
                                        func_data.module.clone(),
                                        false,
                                        false,
                                        cells,
                                    );
                                    frame.return_instance = Some(instance);
                                    frame.expect_none_return = true;
                                    apply_bindings(
                                        &mut frame,
                                        &func_data.code,
                                        bindings,
                                        &self.heap,
                                    );
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
                                let caller_idx = self.frames.len().saturating_sub(1);
                                let result = self.call_builtin(builtin, args, kwargs)?;
                                let frame = self
                                    .frames
                                    .get_mut(caller_idx)
                                    .ok_or_else(|| RuntimeError::new("builtin caller frame missing"))?;
                                frame.stack.push(result);
                            }
                            Value::ExceptionType(name) => {
                                let message = match args.as_slice() {
                                    [] => None,
                                    [value] => Some(format_value(value)),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "exception constructor expects at most one argument",
                                        ));
                                    }
                                };
                                self.push_value(Value::Exception(ExceptionObject {
                                    name,
                                    message,
                                }));
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
                                        ));
                                    }
                                };
                                self.push_function_call(&func_data, args, kwargs)?;
                            }
                            Value::BoundMethod(method) => {
                                let method_data = match &*method.kind() {
                                    Object::BoundMethod(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                };
                                match &*method_data.function.kind() {
                                    Object::Function(data) => {
                                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                                        bound_args
                                            .push(self.receiver_value(&method_data.receiver)?);
                                        bound_args.extend(args);
                                        self.push_function_call(data, bound_args, kwargs)?;
                                    }
                                    Object::NativeMethod(native) => {
                                        match self.call_native_method(
                                            native.kind,
                                            method_data.receiver.clone(),
                                            args,
                                            kwargs,
                                        )? {
                                            NativeCallResult::Value(result) => {
                                                self.push_value(result)
                                            }
                                            NativeCallResult::PropagatedException => {
                                                self.propagate_pending_generator_exception()?;
                                            }
                                        }
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                }
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
                                            ));
                                        }
                                    };
                                    let mut init_args = Vec::with_capacity(args.len() + 1);
                                    init_args.push(Value::Instance(instance.clone()));
                                    init_args.extend(args);
                                    let bindings =
                                        bind_arguments(&func_data, &self.heap, init_args, kwargs)?;
                                    let cells = self
                                        .build_cells(&func_data.code, func_data.closure.clone());
                                    let mut frame = Frame::new(
                                        func_data.code.clone(),
                                        func_data.module.clone(),
                                        false,
                                        false,
                                        cells,
                                    );
                                    frame.return_instance = Some(instance);
                                    frame.expect_none_return = true;
                                    apply_bindings(
                                        &mut frame,
                                        &func_data.code,
                                        bindings,
                                        &self.heap,
                                    );
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
                                let caller_idx = self.frames.len().saturating_sub(1);
                                let result = self.call_builtin(builtin, args, kwargs)?;
                                let frame = self
                                    .frames
                                    .get_mut(caller_idx)
                                    .ok_or_else(|| RuntimeError::new("builtin caller frame missing"))?;
                                frame.stack.push(result);
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
                                        ));
                                    }
                                };
                                self.push_value(Value::Exception(ExceptionObject {
                                    name,
                                    message,
                                }));
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
                                                ));
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
                                        ));
                                    }
                                };
                                self.push_function_call(&func_data, args, kwargs)?;
                            }
                            Value::BoundMethod(method) => {
                                let method_data = match &*method.kind() {
                                    Object::BoundMethod(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                };
                                match &*method_data.function.kind() {
                                    Object::Function(data) => {
                                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                                        bound_args
                                            .push(self.receiver_value(&method_data.receiver)?);
                                        bound_args.extend(args);
                                        self.push_function_call(data, bound_args, kwargs)?;
                                    }
                                    Object::NativeMethod(native) => {
                                        match self.call_native_method(
                                            native.kind,
                                            method_data.receiver.clone(),
                                            args,
                                            kwargs,
                                        )? {
                                            NativeCallResult::Value(result) => {
                                                self.push_value(result)
                                            }
                                            NativeCallResult::PropagatedException => {
                                                self.propagate_pending_generator_exception()?;
                                            }
                                        }
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                }
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
                                            ));
                                        }
                                    };
                                    let mut init_args = Vec::with_capacity(args.len() + 1);
                                    init_args.push(Value::Instance(instance.clone()));
                                    init_args.extend(args);
                                    let bindings =
                                        bind_arguments(&func_data, &self.heap, init_args, kwargs)?;
                                    let cells = self
                                        .build_cells(&func_data.code, func_data.closure.clone());
                                    let mut frame = Frame::new(
                                        func_data.code.clone(),
                                        func_data.module.clone(),
                                        false,
                                        false,
                                        cells,
                                    );
                                    frame.return_instance = Some(instance);
                                    frame.expect_none_return = true;
                                    apply_bindings(
                                        &mut frame,
                                        &func_data.code,
                                        bindings,
                                        &self.heap,
                                    );
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
                                let caller_idx = self.frames.len().saturating_sub(1);
                                let result = self.call_builtin(builtin, args, kwargs)?;
                                let frame = self
                                    .frames
                                    .get_mut(caller_idx)
                                    .ok_or_else(|| RuntimeError::new("builtin caller frame missing"))?;
                                frame.stack.push(result);
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
                                        ));
                                    }
                                };
                                self.push_value(Value::Exception(ExceptionObject {
                                    name,
                                    message,
                                }));
                            }
                            _ => return Err(RuntimeError::new("attempted to call non-function")),
                        }
                    }
                    Opcode::ImportName => {
                        let caller_idx = self.frames.len().saturating_sub(1);
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing import argument"))?
                            as usize;
                        let name = {
                            let frame = self.frames.last().expect("frame exists");
                            if let Some(Value::Str(name)) = frame.code.constants.get(idx) {
                                name.clone()
                            } else if let Some(name) = frame.code.names.get(idx) {
                                name.clone()
                            } else {
                                return Err(RuntimeError::new("import name index out of range"));
                            }
                        };
                        let module = self.import_module_object(&name)?;
                        let result_module = self.module_for_plain_import(&name, module);
                        let frame = self
                            .frames
                            .get_mut(caller_idx)
                            .ok_or_else(|| RuntimeError::new("import caller frame missing"))?;
                        frame.stack.push(Value::Module(result_module));
                    }
                    Opcode::ImportNameCpython => {
                        let caller_idx = self.frames.len().saturating_sub(1);
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing import argument"))?
                            as usize;
                        let name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame.code.names.get(idx).cloned().ok_or_else(|| {
                                RuntimeError::new("import name index out of range")
                            })?
                        };
                        let fromlist = self.pop_value()?;
                        let level_value = self.pop_value()?;
                        let level = value_to_int(level_value)?;
                        if level < 0 {
                            return Err(RuntimeError::new("negative import level"));
                        }
                        let resolved_name = self.resolve_import_name(&name, level as usize)?;
                        let module = self.import_module_object(&resolved_name)?;
                        let result_module = if self.fromlist_requested(&fromlist) {
                            module
                        } else {
                            self.module_for_plain_import(&resolved_name, module)
                        };
                        let frame = self
                            .frames
                            .get_mut(caller_idx)
                            .ok_or_else(|| RuntimeError::new("import caller frame missing"))?;
                        frame.stack.push(Value::Module(result_module));
                    }
                    Opcode::ImportFromCpython => {
                        let caller_idx = self.frames.len().saturating_sub(1);
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing import argument"))?
                            as usize;
                        let attr_name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame.code.names.get(idx).cloned().ok_or_else(|| {
                                RuntimeError::new("import name index out of range")
                            })?
                        };
                        let module = self
                            .frames
                            .last()
                            .and_then(|frame| frame.stack.last())
                            .cloned()
                            .ok_or_else(|| RuntimeError::new("stack underflow"))?;
                        match module {
                            Value::Module(module_obj) => {
                                let (module_name, attr) = match &*module_obj.kind() {
                                    Object::Module(module_data) => {
                                        let attr = module_data.globals.get(&attr_name).cloned();
                                        let module_name = module_data.name.clone();
                                        (module_name, attr)
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "import from expects module object",
                                        ));
                                    }
                                };
                                let attr = if let Some(attr) = attr {
                                    attr
                                } else if let Some(module) =
                                    self.load_submodule(&module_obj, &attr_name)
                                {
                                    Value::Module(module)
                                } else {
                                    return Err(RuntimeError::new(format!(
                                        "cannot import name '{}' from '{}'",
                                        attr_name, module_name
                                    )));
                                };
                                let frame =
                                    self.frames.get_mut(caller_idx).ok_or_else(|| {
                                        RuntimeError::new("import caller frame missing")
                                    })?;
                                frame.stack.push(attr);
                            }
                            _ => {
                                return Err(RuntimeError::new("import from expects module object"));
                            }
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
                    Opcode::JumpIfNone => {
                        let target = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing jump target"))?
                            as usize;
                        let value = self.pop_value()?;
                        if matches!(value, Value::None) {
                            let frame = self.frames.last_mut().expect("frame exists");
                            frame.ip = target;
                        }
                    }
                    Opcode::JumpIfNotNone => {
                        let target = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing jump target"))?
                            as usize;
                        let value = self.pop_value()?;
                        if !matches!(value, Value::None) {
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
                            Value::Generator(obj) => {
                                self.push_value(Value::Generator(obj));
                                return Ok(None);
                            }
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
                        match iterator_value {
                            Value::Generator(obj) => match self.generator_for_iter_next(&obj)? {
                                GeneratorResumeOutcome::Yield(value) => {
                                    self.push_value(Value::Generator(obj));
                                    self.push_value(value);
                                }
                                GeneratorResumeOutcome::Complete(_) => {
                                    let frame = self.frames.last_mut().expect("frame exists");
                                    frame.ip = target;
                                }
                                GeneratorResumeOutcome::PropagatedException => {
                                    self.propagate_pending_generator_exception()?;
                                }
                            },
                            Value::Iterator(iterator_ref) => {
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
                            _ => return Err(RuntimeError::new("FOR_ITER expects iterator")),
                        }
                    }
                    Opcode::YieldValue => {
                        let owner = self
                            .frames
                            .last()
                            .and_then(|frame| frame.generator_owner.clone())
                            .ok_or_else(|| RuntimeError::new("yield outside generator"))?;
                        let yielded = self.pop_value()?;
                        let mut frame = self.frames.pop().expect("frame exists");
                        let resume_kind = frame
                            .generator_resume_kind
                            .take()
                            .unwrap_or(GeneratorResumeKind::Next);
                        frame.generator_awaiting_resume_value = true;
                        frame.generator_pending_throw = None;
                        frame.generator_resume_value = None;
                        let owner_id = owner.id();
                        self.set_generator_running(&owner, false)?;
                        self.set_generator_started(&owner, true)?;
                        self.generator_states.insert(owner_id, frame);
                        if resume_kind == GeneratorResumeKind::Close {
                            return Err(RuntimeError::new("generator ignored GeneratorExit"));
                        }
                        if self.active_generator_resume == Some(owner_id) {
                            self.generator_resume_outcome =
                                Some(GeneratorResumeOutcome::Yield(yielded));
                        } else if let Some(caller) = self.frames.last_mut() {
                            caller.stack.push(yielded);
                        } else {
                            return Ok(Some(Value::None));
                        }
                    }
                    Opcode::Send => {
                        let target = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing send target"))?
                            as usize;
                        let sent = self.pop_value()?;
                        let iter = self.pop_value()?;
                        match self.delegate_yield_from(
                            &iter,
                            sent,
                            None,
                            GeneratorResumeKind::Next,
                        )? {
                            GeneratorResumeOutcome::Yield(value) => {
                                self.push_value(iter);
                                self.push_value(value);
                            }
                            GeneratorResumeOutcome::Complete(value) => {
                                self.push_value(value);
                                let frame = self.frames.last_mut().expect("frame exists");
                                frame.ip = target;
                            }
                            GeneratorResumeOutcome::PropagatedException => {
                                self.propagate_pending_generator_exception()?;
                                return Ok(None);
                            }
                        }
                    }
                    Opcode::YieldFrom => {
                        let owner = self
                            .frames
                            .last()
                            .and_then(|frame| frame.generator_owner.clone())
                            .ok_or_else(|| RuntimeError::new("yield from outside generator"))?;
                        let owner_id = owner.id();
                        let (iter_opt, source_opt, sent, thrown, resume_kind) = {
                            let frame = self.frames.last_mut().expect("frame exists");
                            let source = if frame.yield_from_iter.is_some() {
                                None
                            } else {
                                Some(
                                    frame
                                        .stack
                                        .pop()
                                        .ok_or_else(|| RuntimeError::new("stack underflow"))?,
                                )
                            };
                            let iter = frame.yield_from_iter.take();
                            let sent = frame.generator_resume_value.take().unwrap_or(Value::None);
                            let thrown = frame.generator_pending_throw.take();
                            let resume_kind = frame
                                .generator_resume_kind
                                .take()
                                .unwrap_or(GeneratorResumeKind::Next);
                            (iter, source, sent, thrown, resume_kind)
                        };
                        let iter = if let Some(iter) = iter_opt {
                            iter
                        } else {
                            self.to_iterator_value(source_opt.expect("source present"))?
                        };
                        match self.delegate_yield_from(&iter, sent, thrown, resume_kind)? {
                            GeneratorResumeOutcome::Yield(value) => {
                                let mut frame = self.frames.pop().expect("frame exists");
                                frame.ip = frame.ip.saturating_sub(1);
                                frame.yield_from_iter = Some(iter);
                                frame.generator_awaiting_resume_value = false;
                                frame.generator_resume_value = None;
                                frame.generator_pending_throw = None;
                                self.set_generator_running(&owner, false)?;
                                self.set_generator_started(&owner, true)?;
                                self.generator_states.insert(owner_id, frame);
                                if resume_kind == GeneratorResumeKind::Close {
                                    return Err(RuntimeError::new(
                                        "generator ignored GeneratorExit",
                                    ));
                                }
                                if self.active_generator_resume == Some(owner_id) {
                                    self.generator_resume_outcome =
                                        Some(GeneratorResumeOutcome::Yield(value));
                                } else if let Some(caller) = self.frames.last_mut() {
                                    caller.stack.push(value);
                                } else {
                                    return Ok(Some(Value::None));
                                }
                            }
                            GeneratorResumeOutcome::Complete(value) => {
                                let frame = self.frames.last_mut().expect("frame exists");
                                frame.yield_from_iter = None;
                                frame.generator_resume_value = None;
                                frame.generator_pending_throw = None;
                                frame.generator_awaiting_resume_value = false;
                                frame.stack.push(value);
                            }
                            GeneratorResumeOutcome::PropagatedException => {
                                self.propagate_pending_generator_exception()?;
                                return Ok(None);
                            }
                        }
                    }
                    Opcode::SetupAnnotations => {
                        let dict = self.heap.alloc_dict(Vec::new());
                        self.store_name("__annotations__".to_string(), dict);
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
                            frame.active_exception.clone().ok_or_else(|| {
                                RuntimeError::new("no active exception to reraise")
                            })?
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
                        let value =
                            {
                                let frame = self.frames.last().expect("frame exists");
                                frame.code.constants.get(idx).cloned().ok_or_else(|| {
                                    RuntimeError::new("constant index out of range")
                                })?
                            };
                        let frame = self.frames.pop().expect("frame exists");
                        if frame.expect_none_return && value != Value::None {
                            return Err(RuntimeError::new("__init__() should return None"));
                        }
                        if let Some(owner) = frame.generator_owner {
                            self.finish_generator_resume(owner, value);
                            return Ok(None);
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
                        if !frame.discard_result {
                            caller.stack.push(value);
                        }
                        return Ok(None);
                    }
                    return Ok(Some(value));
                }
                Opcode::ReturnValue => {
                        let value = self.pop_value().unwrap_or(Value::None);
                        let frame = self.frames.pop().expect("frame exists");
                        if frame.expect_none_return && value != Value::None {
                            return Err(RuntimeError::new("__init__() should return None"));
                        }
                        if let Some(owner) = frame.generator_owner {
                            self.finish_generator_resume(owner, value);
                            return Ok(None);
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
                        if !frame.discard_result {
                            caller.stack.push(value);
                        }
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
                Err(err) => match self.handle_runtime_error(err) {
                    Ok(()) => {}
                    Err(err) => return Err(err),
                },
            }
        }
    }

    fn raise_exception(&mut self, value: Value) -> Result<(), RuntimeError> {
        let exc = normalize_exception(value)?;
        let mut traceback = Vec::new();
        loop {
            let Some(frame) = self.frames.last_mut() else {
                let message = self.format_traceback(&traceback, &exc);
                return Err(RuntimeError::new(message));
            };

            traceback.push(Self::frame_trace(frame));

            if let Some(block) = frame.blocks.pop() {
                frame.stack.truncate(block.stack_len);
                frame.stack.push(exc.clone());
                frame.ip = block.handler;
                frame.active_exception = Some(exc);
                return Ok(());
            }

            if let Some(boundary) = self.active_generator_resume_boundary {
                if self.frames.len() <= boundary {
                    self.pending_generator_exception = Some(exc);
                    self.generator_resume_outcome =
                        Some(GeneratorResumeOutcome::PropagatedException);
                    return Ok(());
                }
            }

            let frame = self.frames.pop().expect("frame exists");
            if let Some(owner) = frame.generator_owner {
                self.generator_states.remove(&owner.id());
                let _ = self.set_generator_running(&owner, false);
                let _ = self.set_generator_started(&owner, true);
                let _ = self.set_generator_closed(&owner, true);
                if self.active_generator_resume == Some(owner.id()) {
                    self.generator_resume_outcome =
                        Some(GeneratorResumeOutcome::PropagatedException);
                }
            }
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

    fn frame_trace(frame: &Frame) -> TraceFrame {
        let location = frame.code.locations.get(frame.last_ip);
        let line = location.map(|loc| loc.line).unwrap_or(0);
        let column = location.map(|loc| loc.column).unwrap_or(0);
        TraceFrame {
            filename: frame.code.filename.clone(),
            line,
            column,
            name: frame.code.name.clone(),
        }
    }

    fn format_traceback(&self, frames: &[TraceFrame], exc: &Value) -> String {
        let mut output = String::from("Traceback (most recent call last):\n");
        for frame in frames.iter().rev() {
            output.push_str(&format!(
                "  File \"{}\", line {}, column {}, in {}\n",
                frame.filename, frame.line, frame.column, frame.name
            ));
        }
        output.push_str(&format_value(exc));
        output
    }

    fn class_value_from_module(&self, module: &ObjRef, bases: Vec<ObjRef>) -> Value {
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

    fn get_cell(&self, idx: usize) -> Result<ObjRef, RuntimeError> {
        let frame = self.frames.last().expect("frame exists");
        frame
            .cells
            .get(idx)
            .cloned()
            .ok_or_else(|| RuntimeError::new("cell index out of range"))
    }

    fn load_deref(&self, idx: usize) -> Result<Value, RuntimeError> {
        let frame = self.frames.last().expect("frame exists");
        let cell = frame
            .cells
            .get(idx)
            .ok_or_else(|| RuntimeError::new("cell index out of range"))?;
        match &*cell.kind() {
            Object::Cell(cell_data) => cell_data.value.clone().ok_or_else(|| {
                let name = deref_name(&frame.code, idx).unwrap_or("<cell>");
                RuntimeError::new(format!(
                    "free variable '{}' referenced before assignment",
                    name
                ))
            }),
            _ => Err(RuntimeError::new("invalid cell object")),
        }
    }

    fn store_deref(&mut self, idx: usize, value: Value) -> Result<(), RuntimeError> {
        let frame = self.frames.last_mut().expect("frame exists");
        let cell = frame
            .cells
            .get(idx)
            .cloned()
            .ok_or_else(|| RuntimeError::new("cell index out of range"))?;
        match &mut *cell.kind_mut() {
            Object::Cell(cell_data) => {
                cell_data.value = Some(value);
                Ok(())
            }
            _ => Err(RuntimeError::new("invalid cell object")),
        }
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

    fn push_function_call(
        &mut self,
        func_data: &FunctionObject,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<(), RuntimeError> {
        let bindings = bind_arguments(func_data, &self.heap, args, kwargs)?;
        let cells = self.build_cells(&func_data.code, func_data.closure.clone());
        let mut frame = Frame::new(
            func_data.code.clone(),
            func_data.module.clone(),
            false,
            false,
            cells,
        );
        apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
        if func_data.code.is_generator {
            let generator = match self.heap.alloc_generator(GeneratorObject::new()) {
                Value::Generator(obj) => obj,
                _ => unreachable!(),
            };
            frame.generator_owner = Some(generator.clone());
            self.generator_states.insert(generator.id(), frame);
            self.push_value(Value::Generator(generator));
            return Ok(());
        }
        self.frames.push(frame);
        Ok(())
    }

    fn receiver_value(&self, receiver: &ObjRef) -> Result<Value, RuntimeError> {
        match &*receiver.kind() {
            Object::Instance(_) => Ok(Value::Instance(receiver.clone())),
            Object::Generator(_) => Ok(Value::Generator(receiver.clone())),
            _ => Err(RuntimeError::new("unsupported bound method receiver")),
        }
    }

    fn call_native_method(
        &mut self,
        kind: NativeMethodKind,
        receiver: ObjRef,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<NativeCallResult, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("native methods do not accept keywords"));
        }
        match kind {
            NativeMethodKind::GeneratorIter => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("__iter__() expects no arguments"));
                }
                Ok(NativeCallResult::Value(Value::Generator(receiver)))
            }
            NativeMethodKind::GeneratorNext => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("__next__() expects no arguments"));
                }
                match self.resume_generator(&receiver, None, None, GeneratorResumeKind::Next)? {
                    GeneratorResumeOutcome::Yield(value) => Ok(NativeCallResult::Value(value)),
                    GeneratorResumeOutcome::Complete(_) => Err(RuntimeError::new("StopIteration")),
                    GeneratorResumeOutcome::PropagatedException => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::GeneratorSend => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("send() expects one argument"));
                }
                let sent = args.into_iter().next();
                match self.resume_generator(&receiver, sent, None, GeneratorResumeKind::Next)? {
                    GeneratorResumeOutcome::Yield(value) => Ok(NativeCallResult::Value(value)),
                    GeneratorResumeOutcome::Complete(_) => Err(RuntimeError::new("StopIteration")),
                    GeneratorResumeOutcome::PropagatedException => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::GeneratorThrow => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("throw() expects 1-2 arguments"));
                }
                let exc = args.into_iter().next().expect("checked len");
                let exc = match exc {
                    Value::Exception(_) | Value::ExceptionType(_) => exc,
                    _ => return Err(RuntimeError::new("throw() expects an exception type/value")),
                };
                match self.resume_generator(
                    &receiver,
                    None,
                    Some(exc),
                    GeneratorResumeKind::Throw,
                )? {
                    GeneratorResumeOutcome::Yield(value) => Ok(NativeCallResult::Value(value)),
                    GeneratorResumeOutcome::Complete(_) => Err(RuntimeError::new("StopIteration")),
                    GeneratorResumeOutcome::PropagatedException => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
            }
            NativeMethodKind::GeneratorClose => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("close() expects no arguments"));
                }
                match &*receiver.kind() {
                    Object::Generator(state) if state.closed => {
                        return Ok(NativeCallResult::Value(Value::None));
                    }
                    Object::Generator(_) => {}
                    _ => return Err(RuntimeError::new("object is not a generator")),
                }
                let close_exc = Value::ExceptionType("GeneratorExit".to_string());
                match self.resume_generator(
                    &receiver,
                    None,
                    Some(close_exc),
                    GeneratorResumeKind::Close,
                ) {
                    Ok(GeneratorResumeOutcome::Yield(_)) => {
                        Err(RuntimeError::new("generator ignored GeneratorExit"))
                    }
                    Ok(GeneratorResumeOutcome::Complete(_)) => {
                        self.set_generator_closed(&receiver, true)?;
                        Ok(NativeCallResult::Value(Value::None))
                    }
                    Ok(GeneratorResumeOutcome::PropagatedException) => {
                        if self
                            .pending_generator_exception
                            .as_ref()
                            .map(|exc| exception_is_named(exc, "GeneratorExit"))
                            .unwrap_or(false)
                        {
                            self.pending_generator_exception = None;
                            self.set_generator_closed(&receiver, true)?;
                            Ok(NativeCallResult::Value(Value::None))
                        } else if self.active_exception_is("GeneratorExit") {
                            self.clear_active_exception();
                            self.set_generator_closed(&receiver, true)?;
                            Ok(NativeCallResult::Value(Value::None))
                        } else {
                            Ok(NativeCallResult::PropagatedException)
                        }
                    }
                    Err(err) => {
                        if err.message.contains("GeneratorExit") {
                            self.set_generator_closed(&receiver, true)?;
                            Ok(NativeCallResult::Value(Value::None))
                        } else {
                            Err(err)
                        }
                    }
                }
            }
        }
    }

    fn generator_for_iter_next(
        &mut self,
        generator: &ObjRef,
    ) -> Result<GeneratorResumeOutcome, RuntimeError> {
        self.resume_generator(generator, None, None, GeneratorResumeKind::Next)
    }

    fn to_iterator_value(&mut self, source: Value) -> Result<Value, RuntimeError> {
        match source {
            Value::Iterator(_) | Value::Generator(_) => Ok(source),
            Value::List(obj) => match &*obj.kind() {
                Object::List(_) => Ok(self.heap.alloc_iterator(IteratorObject {
                    kind: IteratorKind::List(obj.clone()),
                    index: 0,
                })),
                _ => Err(RuntimeError::new("yield from expects iterable")),
            },
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(_) => Ok(self.heap.alloc_iterator(IteratorObject {
                    kind: IteratorKind::Tuple(obj.clone()),
                    index: 0,
                })),
                _ => Err(RuntimeError::new("yield from expects iterable")),
            },
            Value::Str(value) => Ok(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::Str(value),
                index: 0,
            })),
            Value::Dict(obj) => match &*obj.kind() {
                Object::Dict(_) => Ok(self.heap.alloc_iterator(IteratorObject {
                    kind: IteratorKind::Dict(obj.clone()),
                    index: 0,
                })),
                _ => Err(RuntimeError::new("yield from expects iterable")),
            },
            _ => Err(RuntimeError::new("yield from expects iterable")),
        }
    }

    fn next_from_iterator_value(
        &mut self,
        iterator: &Value,
    ) -> Result<GeneratorResumeOutcome, RuntimeError> {
        match iterator {
            Value::Generator(obj) => self.generator_for_iter_next(obj),
            Value::Iterator(iterator_ref) => {
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
                    Ok(GeneratorResumeOutcome::Yield(value))
                } else {
                    Ok(GeneratorResumeOutcome::Complete(Value::None))
                }
            }
            _ => Err(RuntimeError::new("yield from expects iterable")),
        }
    }

    fn delegate_yield_from(
        &mut self,
        iterator: &Value,
        sent: Value,
        thrown: Option<Value>,
        resume_kind: GeneratorResumeKind,
    ) -> Result<GeneratorResumeOutcome, RuntimeError> {
        if let Some(exc) = thrown {
            return match iterator {
                Value::Generator(obj) => {
                    let delegated_kind = if resume_kind == GeneratorResumeKind::Close
                        && exception_is_named(&exc, "GeneratorExit")
                    {
                        GeneratorResumeKind::Close
                    } else {
                        GeneratorResumeKind::Throw
                    };
                    let outcome =
                        self.resume_generator(obj, None, Some(exc.clone()), delegated_kind)?;
                    if resume_kind == GeneratorResumeKind::Close
                        && exception_is_named(&exc, "GeneratorExit")
                    {
                        match outcome {
                            GeneratorResumeOutcome::Yield(_) => {
                                Err(RuntimeError::new("generator ignored GeneratorExit"))
                            }
                            GeneratorResumeOutcome::Complete(_) => {
                                self.raise_exception(exc)?;
                                Ok(GeneratorResumeOutcome::PropagatedException)
                            }
                            GeneratorResumeOutcome::PropagatedException => {
                                if self.active_exception_is("GeneratorExit") {
                                    self.clear_active_exception();
                                    self.raise_exception(exc)?;
                                }
                                Ok(GeneratorResumeOutcome::PropagatedException)
                            }
                        }
                    } else {
                        Ok(outcome)
                    }
                }
                Value::Iterator(_) => {
                    self.raise_exception(exc)?;
                    Ok(GeneratorResumeOutcome::PropagatedException)
                }
                _ => Err(RuntimeError::new("yield from expects iterable")),
            };
        }

        if sent != Value::None {
            return match iterator {
                Value::Generator(obj) => {
                    self.resume_generator(obj, Some(sent), None, GeneratorResumeKind::Next)
                }
                Value::Iterator(_) => Err(RuntimeError::new(format!(
                    "'{}' object has no attribute 'send'",
                    self.iterator_type_name(iterator)
                ))),
                _ => Err(RuntimeError::new("yield from expects iterable")),
            };
        }

        self.next_from_iterator_value(iterator)
    }

    fn iterator_type_name(&self, iterator: &Value) -> &'static str {
        match iterator {
            Value::Iterator(obj) => match &*obj.kind() {
                Object::Iterator(state) => match state.kind {
                    IteratorKind::List(_) => "list_iterator",
                    IteratorKind::Tuple(_) => "tuple_iterator",
                    IteratorKind::Str(_) => "str_iterator",
                    IteratorKind::Dict(_) => "dict_keyiterator",
                },
                _ => "iterator",
            },
            Value::Generator(_) => "generator",
            _ => "object",
        }
    }

    fn resume_generator(
        &mut self,
        generator: &ObjRef,
        sent: Option<Value>,
        thrown: Option<Value>,
        kind: GeneratorResumeKind,
    ) -> Result<GeneratorResumeOutcome, RuntimeError> {
        let (started, running, closed) = match &*generator.kind() {
            Object::Generator(state) => (state.started, state.running, state.closed),
            _ => return Err(RuntimeError::new("object is not a generator")),
        };
        if running {
            return Err(RuntimeError::new("generator already executing"));
        }
        if closed {
            let value = self
                .generator_returns
                .get(&generator.id())
                .cloned()
                .unwrap_or(Value::None);
            return Ok(GeneratorResumeOutcome::Complete(value));
        }
        if thrown.is_none() && !started {
            if let Some(value) = &sent {
                if *value != Value::None {
                    return Err(RuntimeError::new(
                        "can't send non-None value to a just-started generator",
                    ));
                }
            }
        }

        let mut frame = self
            .generator_states
            .remove(&generator.id())
            .ok_or_else(|| RuntimeError::new("generator has no suspended frame"))?;
        frame.generator_resume_value = sent;
        frame.generator_pending_throw = thrown;
        frame.generator_resume_kind = Some(kind);
        self.set_generator_running(generator, true)?;
        self.set_generator_started(generator, true)?;

        let previous_active = self.active_generator_resume;
        let previous_boundary = self.active_generator_resume_boundary;
        let previous_outcome = self.generator_resume_outcome.take();
        let previous_pending = self.pending_generator_exception.take();

        self.active_generator_resume = Some(generator.id());
        self.active_generator_resume_boundary = Some(self.frames.len());
        self.generator_resume_outcome = None;
        self.pending_generator_exception = None;
        self.frames.push(frame);
        let run_result = self.run();
        let outcome = self.generator_resume_outcome.take();
        let pending = self.pending_generator_exception.take();
        self.active_generator_resume = previous_active;
        self.active_generator_resume_boundary = previous_boundary;
        self.generator_resume_outcome = previous_outcome;
        self.pending_generator_exception = pending.or(previous_pending);

        match run_result {
            Ok(_) => {
                if let Some(outcome) = outcome {
                    Ok(outcome)
                } else {
                    let value = self
                        .generator_returns
                        .get(&generator.id())
                        .cloned()
                        .unwrap_or(Value::None);
                    Ok(GeneratorResumeOutcome::Complete(value))
                }
            }
            Err(err) => {
                let _ = self.set_generator_running(generator, false);
                Err(err)
            }
        }
    }

    fn finish_generator_resume(&mut self, owner: ObjRef, value: Value) {
        self.generator_states.remove(&owner.id());
        self.generator_returns.insert(owner.id(), value.clone());
        let _ = self.set_generator_running(&owner, false);
        let _ = self.set_generator_started(&owner, true);
        let _ = self.set_generator_closed(&owner, true);
        if self.active_generator_resume == Some(owner.id()) {
            self.generator_resume_outcome = Some(GeneratorResumeOutcome::Complete(value));
        }
    }

    fn set_generator_started(&self, generator: &ObjRef, started: bool) -> Result<(), RuntimeError> {
        match &mut *generator.kind_mut() {
            Object::Generator(state) => {
                state.started = started;
                Ok(())
            }
            _ => Err(RuntimeError::new("object is not a generator")),
        }
    }

    fn set_generator_running(&self, generator: &ObjRef, running: bool) -> Result<(), RuntimeError> {
        match &mut *generator.kind_mut() {
            Object::Generator(state) => {
                state.running = running;
                Ok(())
            }
            _ => Err(RuntimeError::new("object is not a generator")),
        }
    }

    fn set_generator_closed(&self, generator: &ObjRef, closed: bool) -> Result<(), RuntimeError> {
        match &mut *generator.kind_mut() {
            Object::Generator(state) => {
                state.closed = closed;
                if closed {
                    state.running = false;
                }
                Ok(())
            }
            _ => Err(RuntimeError::new("object is not a generator")),
        }
    }

    fn active_exception_is(&self, name: &str) -> bool {
        self.frames
            .last()
            .and_then(|frame| frame.active_exception.as_ref())
            .and_then(|value| match value {
                Value::Exception(exc) => Some(exc.name.as_str()),
                _ => None,
            })
            .map(|exc_name| exc_name == name)
            .unwrap_or(false)
    }

    fn clear_active_exception(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.active_exception = None;
        }
    }

    fn propagate_pending_generator_exception(&mut self) -> Result<(), RuntimeError> {
        if let Some(exc) = self.pending_generator_exception.take() {
            self.raise_exception(exc)?;
        }
        Ok(())
    }

    fn call_builtin(
        &mut self,
        builtin: BuiltinFunction,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        match builtin {
            BuiltinFunction::Locals => self.builtin_locals(args, kwargs),
            BuiltinFunction::Globals => self.builtin_globals(args, kwargs),
            BuiltinFunction::Import => self.builtin_import(args, kwargs),
            _ => {
                if kwargs.is_empty() {
                    builtin.call(&self.heap, args)
                } else {
                    call_builtin_with_kwargs(&self.heap, builtin, args, kwargs)
                }
            }
        }
    }

    fn builtin_locals(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("locals() expects no arguments"));
        }
        let frame = self
            .frames
            .last()
            .ok_or_else(|| RuntimeError::new("no frame"))?;
        if frame.is_module {
            if let Object::Module(module_data) = &*frame.module.kind() {
                let mut entries = Vec::with_capacity(module_data.globals.len());
                for (name, value) in module_data.globals.iter() {
                    entries.push((Value::Str(name.clone()), value.clone()));
                }
                return Ok(self.heap.alloc_dict(entries));
            }
        }
        let mut map = frame.locals.clone();
        for (idx, name) in frame.code.cellvars.iter().enumerate() {
            if !map.contains_key(name) {
                if let Some(cell) = frame.cells.get(idx) {
                    if let Object::Cell(cell_data) = &*cell.kind() {
                        map.insert(name.clone(), cell_data.value.clone().unwrap_or(Value::None));
                    }
                }
            }
        }
        let cell_offset = frame.code.cellvars.len();
        for (idx, name) in frame.code.freevars.iter().enumerate() {
            if !map.contains_key(name) {
                if let Some(cell) = frame.cells.get(cell_offset + idx) {
                    if let Object::Cell(cell_data) = &*cell.kind() {
                        map.insert(name.clone(), cell_data.value.clone().unwrap_or(Value::None));
                    }
                }
            }
        }
        let mut entries = Vec::with_capacity(map.len());
        for (name, value) in map {
            entries.push((Value::Str(name), value));
        }
        Ok(self.heap.alloc_dict(entries))
    }

    fn builtin_globals(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("globals() expects no arguments"));
        }
        let frame = self
            .frames
            .last()
            .ok_or_else(|| RuntimeError::new("no frame"))?;
        if let Object::Module(module_data) = &*frame.function_globals.kind() {
            let mut entries = Vec::with_capacity(module_data.globals.len());
            for (name, value) in module_data.globals.iter() {
                entries.push((Value::Str(name.clone()), value.clone()));
            }
            Ok(self.heap.alloc_dict(entries))
        } else {
            Ok(self.heap.alloc_dict(Vec::new()))
        }
    }

    fn builtin_import(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 5 {
            return Err(RuntimeError::new("__import__() takes at most 5 arguments"));
        }

        let kw_name = kwargs.remove("name");
        let kw_globals = kwargs.remove("globals");
        let kw_locals = kwargs.remove("locals");
        let kw_fromlist = kwargs.remove("fromlist");
        let kw_level = kwargs.remove("level");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "__import__() got an unexpected keyword argument",
            ));
        }

        let name_value = if let Some(value) = kw_name {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "__import__() got multiple values for argument 'name'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new("__import__() missing required argument 'name'"));
        };
        let name = match name_value {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("__import__() name must be string")),
        };

        if kw_globals.is_some() && !args.is_empty() {
            return Err(RuntimeError::new(
                "__import__() got multiple values for argument 'globals'",
            ));
        }
        if kw_locals.is_some() && args.len() > 1 {
            return Err(RuntimeError::new(
                "__import__() got multiple values for argument 'locals'",
            ));
        }
        let fromlist = if let Some(value) = kw_fromlist {
            if args.len() > 2 {
                return Err(RuntimeError::new(
                    "__import__() got multiple values for argument 'fromlist'",
                ));
            }
            value
        } else if args.len() > 2 {
            args[2].clone()
        } else {
            Value::None
        };
        let level = if let Some(value) = kw_level {
            if args.len() > 3 {
                return Err(RuntimeError::new(
                    "__import__() got multiple values for argument 'level'",
                ));
            }
            value_to_int(value)?
        } else if args.len() > 3 {
            value_to_int(args[3].clone())?
        } else {
            0
        };
        if level < 0 {
            return Err(RuntimeError::new("level must be >= 0"));
        }

        let resolved_name = self.resolve_import_name(&name, level as usize)?;
        let module = self.import_module_object(&resolved_name)?;
        let result = if self.fromlist_requested(&fromlist) {
            module
        } else {
            self.module_for_plain_import(&resolved_name, module)
        };
        Ok(Value::Module(result))
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
        self.builtins.insert(
            "divmod".to_string(),
            Value::Builtin(BuiltinFunction::DivMod),
        );
        self.builtins.insert(
            "sorted".to_string(),
            Value::Builtin(BuiltinFunction::Sorted),
        );
        self.builtins.insert(
            "enumerate".to_string(),
            Value::Builtin(BuiltinFunction::Enumerate),
        );
        self.builtins
            .insert("id".to_string(), Value::Builtin(BuiltinFunction::Id));
        self.builtins.insert(
            "locals".to_string(),
            Value::Builtin(BuiltinFunction::Locals),
        );
        self.builtins.insert(
            "globals".to_string(),
            Value::Builtin(BuiltinFunction::Globals),
        );
        self.builtins.insert(
            "__import__".to_string(),
            Value::Builtin(BuiltinFunction::Import),
        );
        self.builtins.insert(
            "BaseException".to_string(),
            Value::ExceptionType("BaseException".to_string()),
        );
        self.builtins.insert(
            "Exception".to_string(),
            Value::ExceptionType("Exception".to_string()),
        );
        self.builtins.insert(
            "ValueError".to_string(),
            Value::ExceptionType("ValueError".to_string()),
        );
        self.builtins.insert(
            "TypeError".to_string(),
            Value::ExceptionType("TypeError".to_string()),
        );
        self.builtins.insert(
            "IndexError".to_string(),
            Value::ExceptionType("IndexError".to_string()),
        );
        self.builtins.insert(
            "KeyError".to_string(),
            Value::ExceptionType("KeyError".to_string()),
        );
        self.builtins.insert(
            "AssertionError".to_string(),
            Value::ExceptionType("AssertionError".to_string()),
        );
        self.builtins.insert(
            "NameError".to_string(),
            Value::ExceptionType("NameError".to_string()),
        );
        self.builtins.insert(
            "AttributeError".to_string(),
            Value::ExceptionType("AttributeError".to_string()),
        );
        self.builtins.insert(
            "ZeroDivisionError".to_string(),
            Value::ExceptionType("ZeroDivisionError".to_string()),
        );
        self.builtins.insert(
            "RuntimeError".to_string(),
            Value::ExceptionType("RuntimeError".to_string()),
        );
        self.builtins.insert(
            "StopIteration".to_string(),
            Value::ExceptionType("StopIteration".to_string()),
        );
        self.builtins.insert(
            "GeneratorExit".to_string(),
            Value::ExceptionType("GeneratorExit".to_string()),
        );
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
                _ => return Err(RuntimeError::new("class base must be a class object")),
            }
        }

        let class_module = match self.heap.alloc_module(ModuleObject::new(name.clone())) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *class_module.kind_mut() {
            module_data
                .globals
                .insert("__name__".to_string(), Value::Str(name));
        }

        let outer_globals = func_data.module.clone();
        let cells = self.build_cells(&func_data.code, func_data.closure.clone());
        let mut frame = Frame::new(func_data.code.clone(), class_module, true, false, cells);
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

fn deref_name(code: &CodeObject, idx: usize) -> Option<&str> {
    if idx < code.cellvars.len() {
        return code.cellvars.get(idx).map(|name| name.as_str());
    }
    let free_idx = idx - code.cellvars.len();
    code.freevars.get(free_idx).map(|name| name.as_str())
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
        Value::Cell(obj) => match &*obj.kind() {
            Object::Cell(cell) => cell.value.as_ref().map_or(false, is_truthy),
            _ => true,
        },
        Value::Iterator(_) => true,
        Value::Generator(_) => true,
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
        Value::ExceptionType(name) => Ok(Value::Exception(ExceptionObject {
            name,
            message: None,
        })),
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

    Ok(exception_inherits(exception_name, handler_name))
}

fn exception_is_named(exception: &Value, name: &str) -> bool {
    match exception {
        Value::Exception(exc) => exc.name == name,
        Value::ExceptionType(exc_name) => exc_name == name,
        _ => false,
    }
}

fn exception_inherits(exception_name: &str, handler_name: &str) -> bool {
    if exception_name == handler_name {
        return true;
    }
    let mut current = exception_parent(exception_name);
    while let Some(name) = current {
        if name == handler_name {
            return true;
        }
        current = exception_parent(name);
    }
    false
}

fn exception_parent(name: &str) -> Option<&'static str> {
    match name {
        "BaseException" => None,
        "Exception" => Some("BaseException"),
        "GeneratorExit" => Some("BaseException"),
        "SystemExit" => Some("BaseException"),
        "KeyboardInterrupt" => Some("BaseException"),
        "StopIteration" => Some("Exception"),
        "AssertionError" => Some("Exception"),
        "AttributeError" => Some("Exception"),
        "IndexError" => Some("Exception"),
        "KeyError" => Some("Exception"),
        "NameError" => Some("Exception"),
        "RuntimeError" => Some("Exception"),
        "TypeError" => Some("Exception"),
        "ValueError" => Some("Exception"),
        "ZeroDivisionError" => Some("Exception"),
        _ => Some("Exception"),
    }
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
        if func.code.posonly_params.iter().any(|param| param == &name) {
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
    let mut assign = |name: String, value: Value| {
        if let Some(idx) = code.cellvars.iter().position(|cell| cell == &name) {
            if let Some(cell) = frame.cells.get(idx) {
                if let Object::Cell(cell_data) = &mut *cell.kind_mut() {
                    cell_data.value = Some(value);
                    return;
                }
            }
        }
        frame.locals.insert(name, value);
    };
    for (name, value) in code
        .posonly_params
        .iter()
        .cloned()
        .zip(bindings.posonly.into_iter())
    {
        assign(name, value);
    }
    for (name, value) in code
        .params
        .iter()
        .cloned()
        .zip(bindings.positional.into_iter())
    {
        assign(name, value);
    }
    for (name, value) in code
        .kwonly_params
        .iter()
        .cloned()
        .zip(bindings.kwonly.into_iter())
    {
        assign(name, value);
    }

    if let Some(name) = code.vararg.as_ref() {
        let value = bindings
            .vararg
            .unwrap_or_else(|| heap.alloc_list(Vec::new()));
        assign(name.clone(), value);
    }

    if let Some(name) = code.kwarg.as_ref() {
        let value = bindings
            .kwarg
            .unwrap_or_else(|| heap.alloc_dict(Vec::new()));
        assign(name.clone(), value);
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
    if message.trim() == "StopIteration" {
        return "StopIteration";
    }
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
    Ok(Value::Bool(compare_order(left, right)? == Ordering::Less))
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
    Ok(Value::Bool(compare_order(left, right)? != Ordering::Less))
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
