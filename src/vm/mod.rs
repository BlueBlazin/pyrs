//! Bytecode virtual machine (minimal subset).

use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::bytecode::cpython;
use crate::bytecode::{CodeObject, Instruction, Opcode};
use crate::compiler;
use crate::parser;
use crate::runtime::{
    BoundMethod, BuiltinFunction, ClassObject, ExceptionObject, FunctionObject, GeneratorObject,
    Heap, InstanceObject, IteratorKind, IteratorObject, ModuleObject, NativeMethodKind,
    NativeMethodObject, ObjRef, Object, RuntimeError, SuperObject, Value, format_value,
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
    package_dirs: Vec<PathBuf>,
    is_namespace: bool,
}

const DEFAULT_META_PATH_FINDER: &str = "pyrs.PathFinder";
const DEFAULT_PATH_HOOK: &str = "pyrs.FileFinder";
const SOURCE_FILE_LOADER: &str = "pyrs.SourceFileLoader";
const NAMESPACE_LOADER: &str = "pyrs.NamespaceLoader";
const BUILTIN_MODULE_LOADER: &str = "pyrs.BuiltinLoader";
const MT_N: usize = 624;
const MT_M: usize = 397;
const MT_MATRIX_A: u32 = 0x9908_b0df;
const MT_UPPER_MASK: u32 = 0x8000_0000;
const MT_LOWER_MASK: u32 = 0x7fff_ffff;
const SIGNAL_DEFAULT: i64 = 0;
const SIGNAL_IGNORE: i64 = 1;
const SIGNAL_SIGINT: i64 = 2;
const SIGNAL_SIGTERM: i64 = 15;
static MONOTONIC_START: OnceLock<Instant> = OnceLock::new();

#[derive(Clone)]
struct Mt19937 {
    mt: [u32; MT_N],
    index: usize,
}

impl Mt19937 {
    fn new(seed: u64) -> Self {
        let mut rng = Self {
            mt: [0; MT_N],
            index: MT_N,
        };
        rng.seed(seed);
        rng
    }

    fn seed(&mut self, seed: u64) {
        self.mt[0] = seed as u32;
        for idx in 1..MT_N {
            self.mt[idx] = 1812433253u32
                .wrapping_mul(self.mt[idx - 1] ^ (self.mt[idx - 1] >> 30))
                .wrapping_add(idx as u32);
        }
        self.index = MT_N;
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= MT_N {
            self.twist();
        }
        let mut value = self.mt[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^= value >> 18;
        value
    }

    fn random_f64(&mut self) -> f64 {
        let top = (self.next_u32() >> 5) as u64;
        let bottom = (self.next_u32() >> 6) as u64;
        ((top << 26) | bottom) as f64 / 9007199254740992.0
    }

    fn twist(&mut self) {
        for idx in 0..MT_N {
            let x = (self.mt[idx] & MT_UPPER_MASK) | (self.mt[(idx + 1) % MT_N] & MT_LOWER_MASK);
            let mut x_a = x >> 1;
            if x & 1 != 0 {
                x_a ^= MT_MATRIX_A;
            }
            self.mt[idx] = self.mt[(idx + MT_M) % MT_N] ^ x_a;
        }
        self.index = 0;
    }
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

enum InternalCallOutcome {
    Value(Value),
    CallerExceptionHandled,
}

enum AttrAccessOutcome {
    Value(Value),
    ExceptionHandled,
}

enum AttrMutationOutcome {
    Done,
    ExceptionHandled,
}

enum ClassBuildOutcome {
    Value(Value),
    ExceptionHandled,
}

#[derive(Clone, Copy)]
enum ReMode {
    Search,
    Match,
    FullMatch,
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
    class_metaclass: Option<Value>,
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
            class_metaclass: None,
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
    random: Mt19937,
    generator_states: HashMap<u64, Frame>,
    generator_returns: HashMap<u64, Value>,
    pending_generator_exception: Option<Value>,
    active_generator_resume: Option<u64>,
    active_generator_resume_boundary: Option<usize>,
    generator_resume_outcome: Option<GeneratorResumeOutcome>,
    run_stop_depth: Option<usize>,
    signal_handlers: HashMap<i64, Value>,
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
            random: Mt19937::new(5489),
            generator_states: HashMap::new(),
            generator_returns: HashMap::new(),
            pending_generator_exception: None,
            active_generator_resume: None,
            active_generator_resume_boundary: None,
            generator_resume_outcome: None,
            run_stop_depth: None,
            signal_handlers: HashMap::new(),
        };
        let main = vm.main_module.clone();
        vm.set_module_metadata(&main, "__main__", None, None, false, Vec::new(), false);
        vm.install_sys_module();
        vm.install_importlib_modules();
        vm.install_random_module();
        vm.install_stdlib_modules();
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
            if let Some(meta) = &frame.class_metaclass {
                roots.push(meta.clone());
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
            if let Some(meta) = &frame.class_metaclass {
                roots.push(meta.clone());
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
        self.run_stop_depth = None;
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
        self.set_module_metadata(&sys_module, "sys", None, None, false, Vec::new(), false);
        if let Object::Module(module_data) = &mut *sys_module.kind_mut() {
            module_data
                .globals
                .insert("path".to_string(), self.heap.alloc_list(Vec::new()));
            module_data.globals.insert(
                "meta_path".to_string(),
                self.heap
                    .alloc_list(vec![Value::Str(DEFAULT_META_PATH_FINDER.to_string())]),
            );
            module_data.globals.insert(
                "path_hooks".to_string(),
                self.heap
                    .alloc_list(vec![Value::Str(DEFAULT_PATH_HOOK.to_string())]),
            );
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

    fn install_importlib_modules(&mut self) {
        let importlib = match self.heap.alloc_module(ModuleObject::new("importlib")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &importlib,
            "importlib",
            None,
            Some(BUILTIN_MODULE_LOADER),
            true,
            Vec::new(),
            false,
        );
        if let Object::Module(module_data) = &mut *importlib.kind_mut() {
            module_data.globals.insert(
                "import_module".to_string(),
                Value::Builtin(BuiltinFunction::ImportModule),
            );
            module_data.globals.insert(
                "find_spec".to_string(),
                Value::Builtin(BuiltinFunction::FindSpec),
            );
        }
        self.register_module("importlib", importlib.clone());

        let util = match self.heap.alloc_module(ModuleObject::new("importlib.util")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &util,
            "importlib.util",
            None,
            Some(BUILTIN_MODULE_LOADER),
            false,
            Vec::new(),
            false,
        );
        if let Object::Module(module_data) = &mut *util.kind_mut() {
            module_data.globals.insert(
                "find_spec".to_string(),
                Value::Builtin(BuiltinFunction::FindSpec),
            );
        }
        self.register_module("importlib.util", util.clone());
        self.link_module_chain("importlib.util", util);
    }

    fn install_random_module(&mut self) {
        let random_module = match self.heap.alloc_module(ModuleObject::new("random")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &random_module,
            "random",
            None,
            Some(BUILTIN_MODULE_LOADER),
            false,
            Vec::new(),
            false,
        );
        if let Object::Module(module_data) = &mut *random_module.kind_mut() {
            module_data.globals.insert(
                "seed".to_string(),
                Value::Builtin(BuiltinFunction::RandomSeed),
            );
            module_data.globals.insert(
                "random".to_string(),
                Value::Builtin(BuiltinFunction::RandomRandom),
            );
            module_data.globals.insert(
                "randrange".to_string(),
                Value::Builtin(BuiltinFunction::RandomRandRange),
            );
            module_data.globals.insert(
                "randint".to_string(),
                Value::Builtin(BuiltinFunction::RandomRandInt),
            );
            module_data.globals.insert(
                "getrandbits".to_string(),
                Value::Builtin(BuiltinFunction::RandomGetRandBits),
            );
            module_data.globals.insert(
                "choice".to_string(),
                Value::Builtin(BuiltinFunction::RandomChoice),
            );
            module_data.globals.insert(
                "shuffle".to_string(),
                Value::Builtin(BuiltinFunction::RandomShuffle),
            );
        }
        self.register_module("random", random_module);
    }

    fn install_builtin_module(
        &mut self,
        name: &str,
        functions: &[(&str, BuiltinFunction)],
        constants: Vec<(&str, Value)>,
    ) {
        let module = match self.heap.alloc_module(ModuleObject::new(name)) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &module,
            name,
            None,
            Some(BUILTIN_MODULE_LOADER),
            false,
            Vec::new(),
            false,
        );
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            for (entry_name, builtin) in functions {
                module_data
                    .globals
                    .insert((*entry_name).to_string(), Value::Builtin(*builtin));
            }
            for (entry_name, value) in constants {
                module_data.globals.insert(entry_name.to_string(), value);
            }
        }
        self.register_module(name, module);
    }

    fn install_stdlib_modules(&mut self) {
        self.install_builtin_module(
            "math",
            &[
                ("sqrt", BuiltinFunction::MathSqrt),
                ("floor", BuiltinFunction::MathFloor),
                ("ceil", BuiltinFunction::MathCeil),
                ("isfinite", BuiltinFunction::MathIsFinite),
                ("isinf", BuiltinFunction::MathIsInf),
                ("isnan", BuiltinFunction::MathIsNaN),
            ],
            vec![
                ("pi", Value::Float(std::f64::consts::PI)),
                ("e", Value::Float(std::f64::consts::E)),
                ("tau", Value::Float(std::f64::consts::TAU)),
                ("inf", Value::Float(f64::INFINITY)),
                ("nan", Value::Float(f64::NAN)),
            ],
        );
        self.install_builtin_module(
            "time",
            &[
                ("time", BuiltinFunction::TimeTime),
                ("monotonic", BuiltinFunction::TimeMonotonic),
                ("sleep", BuiltinFunction::TimeSleep),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "os",
            &[
                ("getcwd", BuiltinFunction::OsGetCwd),
                ("listdir", BuiltinFunction::OsListDir),
                ("path_exists", BuiltinFunction::OsPathExists),
                ("path_join", BuiltinFunction::OsPathJoin),
            ],
            vec![
                ("sep", Value::Str(std::path::MAIN_SEPARATOR.to_string())),
                ("pathsep", Value::Str(if cfg!(windows) { ";" } else { ":" }.to_string())),
            ],
        );
        self.install_builtin_module(
            "pathlib",
            &[
                ("Path", BuiltinFunction::OsPathJoin),
                ("joinpath", BuiltinFunction::OsPathJoin),
                ("exists", BuiltinFunction::OsPathExists),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "json",
            &[
                ("dumps", BuiltinFunction::JsonDumps),
                ("loads", BuiltinFunction::JsonLoads),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "codecs",
            &[
                ("encode", BuiltinFunction::CodecsEncode),
                ("decode", BuiltinFunction::CodecsDecode),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "re",
            &[
                ("search", BuiltinFunction::ReSearch),
                ("match", BuiltinFunction::ReMatch),
                ("fullmatch", BuiltinFunction::ReFullMatch),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "operator",
            &[
                ("add", BuiltinFunction::OperatorAdd),
                ("sub", BuiltinFunction::OperatorSub),
                ("mul", BuiltinFunction::OperatorMul),
                ("truediv", BuiltinFunction::OperatorTrueDiv),
                ("eq", BuiltinFunction::OperatorEq),
                ("contains", BuiltinFunction::OperatorContains),
                ("getitem", BuiltinFunction::OperatorGetItem),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "itertools",
            &[
                ("chain", BuiltinFunction::ItertoolsChain),
                ("repeat", BuiltinFunction::ItertoolsRepeat),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "functools",
            &[("reduce", BuiltinFunction::FunctoolsReduce)],
            Vec::new(),
        );
        self.install_builtin_module(
            "collections",
            &[
                ("Counter", BuiltinFunction::CollectionsCounter),
                ("deque", BuiltinFunction::CollectionsDeque),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "types",
            &[("ModuleType", BuiltinFunction::TypesModuleType)],
            Vec::new(),
        );
        self.install_builtin_module(
            "inspect",
            &[
                ("isfunction", BuiltinFunction::InspectIsFunction),
                ("isclass", BuiltinFunction::InspectIsClass),
                ("ismodule", BuiltinFunction::InspectIsModule),
                ("isgenerator", BuiltinFunction::InspectIsGenerator),
                ("iscoroutine", BuiltinFunction::InspectIsCoroutine),
                ("isawaitable", BuiltinFunction::InspectIsAwaitable),
                ("isasyncgen", BuiltinFunction::InspectIsAsyncGen),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "io",
            &[
                ("open", BuiltinFunction::IoOpen),
                ("read_text", BuiltinFunction::IoReadText),
                ("write_text", BuiltinFunction::IoWriteText),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "datetime",
            &[
                ("now", BuiltinFunction::DateTimeNow),
                ("today", BuiltinFunction::DateToday),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "asyncio",
            &[
                ("run", BuiltinFunction::AsyncioRun),
                ("sleep", BuiltinFunction::AsyncioSleep),
                ("create_task", BuiltinFunction::AsyncioCreateTask),
                ("gather", BuiltinFunction::AsyncioGather),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "threading",
            &[
                ("get_ident", BuiltinFunction::ThreadingGetIdent),
                ("current_thread", BuiltinFunction::ThreadingCurrentThread),
                ("main_thread", BuiltinFunction::ThreadingMainThread),
                ("active_count", BuiltinFunction::ThreadingActiveCount),
            ],
            vec![("TIMEOUT_MAX", Value::Float(f64::MAX))],
        );
        self.install_builtin_module(
            "signal",
            &[
                ("signal", BuiltinFunction::SignalSignal),
                ("getsignal", BuiltinFunction::SignalGetSignal),
                ("raise_signal", BuiltinFunction::SignalRaiseSignal),
            ],
            vec![
                ("SIG_DFL", Value::Int(SIGNAL_DEFAULT)),
                ("SIG_IGN", Value::Int(SIGNAL_IGNORE)),
                ("SIGINT", Value::Int(SIGNAL_SIGINT)),
                ("SIGTERM", Value::Int(SIGNAL_SIGTERM)),
            ],
        );
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
        let loader_name = if source_info.is_namespace {
            NAMESPACE_LOADER
        } else {
            SOURCE_FILE_LOADER
        };

        let module = self.create_module_for_loader(name, loader_name)?;
        let origin = if source_info.is_namespace {
            None
        } else {
            Some(&source_info.path)
        };
        self.set_module_metadata(
            &module,
            name,
            origin,
            Some(loader_name),
            source_info.is_package,
            source_info.package_dirs.clone(),
            source_info.is_namespace,
        );

        self.register_module(name, module.clone());
        self.link_module_chain(name, module.clone());
        self.exec_module_for_loader(&module, name, loader_name, &source_info)?;
        Ok(module)
    }

    fn create_module_for_loader(
        &mut self,
        name: &str,
        loader_name: &str,
    ) -> Result<ObjRef, RuntimeError> {
        match loader_name {
            SOURCE_FILE_LOADER | NAMESPACE_LOADER => {
                match self.heap.alloc_module(ModuleObject::new(name)) {
                    Value::Module(obj) => Ok(obj),
                    _ => unreachable!(),
                }
            }
            _ => Err(RuntimeError::new(format!(
                "unsupported loader for module creation: {loader_name}"
            ))),
        }
    }

    fn exec_module_for_loader(
        &mut self,
        module: &ObjRef,
        name: &str,
        loader_name: &str,
        source_info: &ModuleSourceInfo,
    ) -> Result<(), RuntimeError> {
        match loader_name {
            NAMESPACE_LOADER => Ok(()),
            SOURCE_FILE_LOADER => {
                let source = std::fs::read_to_string(&source_info.path).map_err(|err| {
                    RuntimeError::new(format!("failed to read module '{name}': {err}"))
                })?;

                let module_ast = parser::parse_module(&source).map_err(|err| {
                    RuntimeError::new(format!(
                        "parse error in module '{name}' at {}: {}",
                        err.offset, err.message
                    ))
                })?;
                let code = compiler::compile_module_with_filename(
                    &module_ast,
                    &source_info.path.to_string_lossy(),
                )
                .map_err(|err| {
                    RuntimeError::new(format!("compile error in module '{name}': {}", err.message))
                })?;
                let code = Rc::new(code);
                let cells = self.build_cells(&code, Vec::new());
                let mut frame = Frame::new(code, module.clone(), true, false, cells);
                frame.discard_result = true;
                self.frames.push(frame);
                Ok(())
            }
            _ => Err(RuntimeError::new(format!(
                "unsupported loader for module execution: {loader_name}"
            ))),
        }
    }

    fn find_module_source(&mut self, name: &str) -> Option<ModuleSourceInfo> {
        self.sync_module_paths_from_sys();
        let meta_path = self.sys_list_values("meta_path").unwrap_or_default();
        for finder in &meta_path {
            if let Some(source) = self.find_module_source_with_meta_finder(name, finder) {
                return Some(source);
            }
        }
        None
    }

    fn find_module_source_with_meta_finder(
        &mut self,
        name: &str,
        finder: &Value,
    ) -> Option<ModuleSourceInfo> {
        if matches_finder_kind(finder, DEFAULT_META_PATH_FINDER) {
            return self.path_finder_find_spec(name);
        }
        None
    }

    fn path_finder_find_spec(&mut self, name: &str) -> Option<ModuleSourceInfo> {
        if let Some((parent_name, child_name)) = name.rsplit_once('.') {
            if let Some(parent_paths) = self.package_search_paths(parent_name) {
                if let Some(source) = self.find_module_source_in_roots(child_name, &parent_paths) {
                    return Some(source);
                }
            }
        }
        let roots = self.module_paths.clone();
        self.find_module_source_in_roots(name, &roots)
    }

    fn package_search_paths(&self, package_name: &str) -> Option<Vec<PathBuf>> {
        let package = self.modules.get(package_name)?.clone();
        let package_kind = package.kind();
        let module_data = match &*package_kind {
            Object::Module(module) => module,
            _ => return None,
        };
        let path_value = module_data.globals.get("__path__")?;
        let path_list = match path_value {
            Value::List(list) => list.clone(),
            _ => return None,
        };
        let list_kind = path_list.kind();
        let values = match &*list_kind {
            Object::List(values) => values,
            _ => return None,
        };
        let mut roots = Vec::new();
        for value in values {
            if let Value::Str(path) = value {
                roots.push(PathBuf::from(path));
            }
        }
        if roots.is_empty() { None } else { Some(roots) }
    }

    fn find_module_source_in_roots(
        &mut self,
        module_name: &str,
        roots: &[PathBuf],
    ) -> Option<ModuleSourceInfo> {
        let mut namespace_dirs = Vec::new();
        for root in roots {
            let importer = match self.path_importer_for_root(root) {
                Some(importer) => importer,
                None => continue,
            };
            if let Some(spec) = self.find_module_source_with_importer(&importer, module_name) {
                if spec.is_namespace {
                    namespace_dirs.extend(spec.package_dirs);
                    continue;
                }
                return Some(spec);
            }
        }
        if !namespace_dirs.is_empty() {
            return Some(ModuleSourceInfo {
                path: namespace_dirs[0].clone(),
                is_package: true,
                package_dirs: namespace_dirs,
                is_namespace: true,
            });
        }
        None
    }

    fn path_importer_for_root(&mut self, root: &PathBuf) -> Option<Value> {
        let key = Value::Str(root.to_string_lossy().to_string());
        if let Some(cache_dict) = self.sys_dict_obj("path_importer_cache") {
            if let Some(cached) = dict_get_value(&cache_dict, &key) {
                return if matches!(cached, Value::None) {
                    None
                } else {
                    Some(cached)
                };
            }

            let importer = self.run_path_hooks_for_root(root);
            let cached_value = importer.clone().unwrap_or(Value::None);
            dict_set_value(&cache_dict, key, cached_value.clone());
            return if matches!(cached_value, Value::None) {
                None
            } else {
                Some(cached_value)
            };
        }
        self.run_path_hooks_for_root(root)
    }

    fn run_path_hooks_for_root(&mut self, root: &PathBuf) -> Option<Value> {
        let hooks = self.sys_list_values("path_hooks").unwrap_or_default();
        for hook in hooks {
            if matches_finder_kind(&hook, DEFAULT_PATH_HOOK) {
                return Some(self.make_file_finder_importer(root));
            }
        }
        None
    }

    fn make_file_finder_importer(&self, root: &PathBuf) -> Value {
        self.heap.alloc_dict(vec![
            (
                Value::Str("kind".to_string()),
                Value::Str(DEFAULT_PATH_HOOK.to_string()),
            ),
            (
                Value::Str("path".to_string()),
                Value::Str(root.to_string_lossy().to_string()),
            ),
        ])
    }

    fn find_module_source_with_importer(
        &self,
        importer: &Value,
        module_name: &str,
    ) -> Option<ModuleSourceInfo> {
        let importer_dict = match importer {
            Value::Dict(dict) => dict.clone(),
            _ => return None,
        };
        let kind = match dict_get_value(&importer_dict, &Value::Str("kind".to_string())) {
            Some(Value::Str(kind)) => kind,
            _ => return None,
        };
        if kind != DEFAULT_PATH_HOOK {
            None
        } else {
            let root = match dict_get_value(&importer_dict, &Value::Str("path".to_string())) {
                Some(Value::Str(path)) => PathBuf::from(path),
                _ => return None,
            };
            self.find_module_source_in_single_root(module_name, &root)
        }
    }

    fn find_module_source_in_single_root(
        &self,
        module_name: &str,
        root: &PathBuf,
    ) -> Option<ModuleSourceInfo> {
        let rel_name = module_name.replace('.', "/");
        let candidate = root.join(format!("{rel_name}.py"));
        if candidate.exists() {
            return Some(ModuleSourceInfo {
                path: candidate,
                is_package: false,
                package_dirs: Vec::new(),
                is_namespace: false,
            });
        }
        let package_dir = root.join(&rel_name);
        let package_init = package_dir.join("__init__.py");
        if package_init.exists() {
            return Some(ModuleSourceInfo {
                path: package_init,
                is_package: true,
                package_dirs: vec![package_dir],
                is_namespace: false,
            });
        }
        if package_dir.is_dir() {
            return Some(ModuleSourceInfo {
                path: package_dir.clone(),
                is_package: true,
                package_dirs: vec![package_dir],
                is_namespace: true,
            });
        }
        None
    }

    fn sys_list_values(&self, name: &str) -> Option<Vec<Value>> {
        let sys_module = self.modules.get("sys")?.clone();
        let module_kind = sys_module.kind();
        let module_data = match &*module_kind {
            Object::Module(module_data) => module_data,
            _ => return None,
        };
        let list_obj = match module_data.globals.get(name) {
            Some(Value::List(list)) => list.clone(),
            _ => return None,
        };
        match &*list_obj.kind() {
            Object::List(values) => Some(values.clone()),
            _ => None,
        }
    }

    fn sys_dict_obj(&self, name: &str) -> Option<ObjRef> {
        let sys_module = self.modules.get("sys")?.clone();
        let module_kind = sys_module.kind();
        let module_data = match &*module_kind {
            Object::Module(module_data) => module_data,
            _ => return None,
        };
        match module_data.globals.get(name) {
            Some(Value::Dict(dict)) => Some(dict.clone()),
            _ => None,
        }
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
        self.set_module_metadata(&module, name, None, None, false, Vec::new(), false);
        self.register_module(name, module.clone());
        module
    }

    fn set_module_metadata(
        &mut self,
        module: &ObjRef,
        name: &str,
        origin: Option<&PathBuf>,
        loader_name: Option<&str>,
        is_package: bool,
        package_dirs: Vec<PathBuf>,
        is_namespace: bool,
    ) {
        let package_name = if is_package {
            name.to_string()
        } else {
            name.rsplit_once('.')
                .map(|(parent, _)| parent.to_string())
                .unwrap_or_default()
        };
        let loader_value = loader_name
            .map(|loader| Value::Str(loader.to_string()))
            .unwrap_or(Value::None);
        let origin_value = origin
            .map(|path| Value::Str(path.to_string_lossy().to_string()))
            .unwrap_or(Value::None);
        let submodule_locations = if is_package {
            let mut entries = Vec::new();
            for dir in package_dirs.iter() {
                entries.push(Value::Str(dir.to_string_lossy().to_string()));
            }
            self.heap.alloc_list(entries)
        } else {
            Value::None
        };
        let spec_value = self.build_module_spec_value(
            name,
            origin,
            loader_name,
            is_package,
            package_dirs.as_slice(),
            is_namespace,
        );

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
                module_data
                    .globals
                    .insert("__file__".to_string(), origin_value);
            }
            if is_package {
                module_data
                    .globals
                    .insert("__path__".to_string(), submodule_locations);
            }
        }
    }

    fn build_module_spec_value(
        &mut self,
        name: &str,
        origin: Option<&PathBuf>,
        loader_name: Option<&str>,
        is_package: bool,
        package_dirs: &[PathBuf],
        is_namespace: bool,
    ) -> Value {
        let parent = name
            .rsplit_once('.')
            .map(|(parent, _)| parent.to_string())
            .unwrap_or_default();
        let loader_value = loader_name
            .map(|loader| Value::Str(loader.to_string()))
            .unwrap_or(Value::None);
        let origin_value = origin
            .map(|path| Value::Str(path.to_string_lossy().to_string()))
            .unwrap_or(Value::None);
        let submodule_locations = if is_package {
            let mut entries = Vec::new();
            for dir in package_dirs {
                entries.push(Value::Str(dir.to_string_lossy().to_string()));
            }
            self.heap.alloc_list(entries)
        } else {
            Value::None
        };

        self.heap.alloc_dict(vec![
            (Value::Str("name".to_string()), Value::Str(name.to_string())),
            (Value::Str("origin".to_string()), origin_value),
            (Value::Str("loader".to_string()), loader_value),
            (Value::Str("parent".to_string()), Value::Str(parent)),
            (
                Value::Str("submodule_search_locations".to_string()),
                submodule_locations,
            ),
            (
                Value::Str("is_package".to_string()),
                Value::Bool(is_package),
            ),
            (
                Value::Str("is_namespace".to_string()),
                Value::Bool(is_namespace),
            ),
            (
                Value::Str("has_location".to_string()),
                Value::Bool(origin.is_some()),
            ),
            (Value::Str("cached".to_string()), Value::None),
        ])
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

        self.resolve_import_name_from_package(&package, requested, level)
    }

    fn resolve_import_name_from_package(
        &self,
        package: &str,
        requested: &str,
        level: usize,
    ) -> Result<String, RuntimeError> {
        if level == 0 {
            return Ok(requested.to_string());
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
            if let Some(stop_depth) = self.run_stop_depth {
                if self.frames.len() <= stop_depth {
                    return Ok(Value::None);
                }
            }
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
                    match self.class_value_from_module(
                        &frame.module,
                        frame.class_bases,
                        frame.class_metaclass,
                    )? {
                        ClassBuildOutcome::Value(value) => value,
                        ClassBuildOutcome::ExceptionHandled => continue,
                    }
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
                    Opcode::GetAwaitable => {
                        let value = self.pop_value()?;
                        let awaitable = self.awaitable_from_value(value)?;
                        self.push_value(awaitable);
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
                        let caller_idx = self.frames.len().saturating_sub(1);
                        let value = self.pop_value()?;
                        let attr = match value {
                            Value::Module(module) => self.load_attr_module(&module, &attr_name)?,
                            Value::Class(class) => {
                                match self.load_attr_class(&class, &attr_name)? {
                                    AttrAccessOutcome::Value(attr) => attr,
                                    AttrAccessOutcome::ExceptionHandled => return Ok(None),
                                }
                            }
                            Value::Instance(instance) => {
                                match self.load_attr_instance(&instance, &attr_name)? {
                                    AttrAccessOutcome::Value(attr) => attr,
                                    AttrAccessOutcome::ExceptionHandled => return Ok(None),
                                }
                            }
                            Value::Super(super_obj) => {
                                match self.load_attr_super(&super_obj, &attr_name)? {
                                    AttrAccessOutcome::Value(attr) => attr,
                                    AttrAccessOutcome::ExceptionHandled => return Ok(None),
                                }
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
                                    Value::Dict(annotations)
                                } else {
                                    return Err(RuntimeError::new(format!(
                                        "function has no attribute '{}'",
                                        attr_name
                                    )));
                                }
                            }
                            Value::Generator(generator) => {
                                let kind = match &*generator.kind() {
                                    Object::Generator(state) if state.is_async_generator => {
                                        match attr_name.as_str() {
                                            "__aiter__" => NativeMethodKind::GeneratorIter,
                                            "__anext__" => NativeMethodKind::GeneratorANext,
                                            "asend" => NativeMethodKind::GeneratorANext,
                                            "athrow" => NativeMethodKind::GeneratorThrow,
                                            "aclose" => NativeMethodKind::GeneratorClose,
                                            "throw" => NativeMethodKind::GeneratorThrow,
                                            "close" => NativeMethodKind::GeneratorClose,
                                            _ => {
                                                return Err(RuntimeError::new(format!(
                                                    "async_generator has no attribute '{}'",
                                                    attr_name
                                                )));
                                            }
                                        }
                                    }
                                    Object::Generator(state) if state.is_coroutine => {
                                        match attr_name.as_str() {
                                            "__await__" => NativeMethodKind::GeneratorAwait,
                                            "send" => NativeMethodKind::GeneratorSend,
                                            "throw" => NativeMethodKind::GeneratorThrow,
                                            "close" => NativeMethodKind::GeneratorClose,
                                            _ => {
                                                return Err(RuntimeError::new(format!(
                                                    "coroutine has no attribute '{}'",
                                                    attr_name
                                                )));
                                            }
                                        }
                                    }
                                    Object::Generator(_) => match attr_name.as_str() {
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
                                    },
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attribute access unsupported type",
                                        ));
                                    }
                                };
                                let native =
                                    self.heap.alloc_native_method(NativeMethodObject::new(kind));
                                let bound = BoundMethod::new(native, generator);
                                self.heap.alloc_bound_method(bound)
                            }
                            Value::Exception(exception) => match attr_name.as_str() {
                                "__cause__" => exception
                                    .cause
                                    .as_ref()
                                    .map(|cause| Value::Exception((**cause).clone()))
                                    .unwrap_or(Value::None),
                                "__context__" => exception
                                    .context
                                    .as_ref()
                                    .map(|context| Value::Exception((**context).clone()))
                                    .unwrap_or(Value::None),
                                "__suppress_context__" => Value::Bool(exception.suppress_context),
                                _ => {
                                    return Err(RuntimeError::new(format!(
                                        "exception has no attribute '{}'",
                                        attr_name
                                    )));
                                }
                            },
                            _ => {
                                return Err(RuntimeError::new("attribute access unsupported type"));
                            }
                        };
                        if push_null {
                            let frame = self.frames.get_mut(caller_idx).ok_or_else(|| {
                                RuntimeError::new("attribute caller frame missing")
                            })?;
                            frame.stack.push(Value::None);
                        }
                        let frame = self
                            .frames
                            .get_mut(caller_idx)
                            .ok_or_else(|| RuntimeError::new("attribute caller frame missing"))?;
                        frame.stack.push(attr);
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
                    Opcode::DeleteName => {
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
                        let mut removed = false;
                        if let Some(frame) = self.frames.last_mut() {
                            if !frame.is_module {
                                removed = frame.locals.remove(&name).is_some();
                            }
                            if !removed {
                                if let Object::Module(module_data) = &mut *frame.module.kind_mut() {
                                    removed = module_data.globals.remove(&name).is_some();
                                }
                            }
                        }
                        if !removed {
                            return Err(RuntimeError::new(format!(
                                "name '{}' is not defined",
                                name
                            )));
                        }
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
                                match self.store_attr_instance(&instance, &attr_name, value)? {
                                    AttrMutationOutcome::Done => {}
                                    AttrMutationOutcome::ExceptionHandled => return Ok(None),
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
                                match self.store_attr_instance(&instance, &attr_name, value)? {
                                    AttrMutationOutcome::Done => {}
                                    AttrMutationOutcome::ExceptionHandled => return Ok(None),
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
                    Opcode::DeleteAttr => {
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
                        match target {
                            Value::Module(module) => {
                                if let Object::Module(module_data) = &mut *module.kind_mut() {
                                    if module_data.globals.remove(&attr_name).is_none() {
                                        return Err(RuntimeError::new(format!(
                                            "module attribute '{}' does not exist",
                                            attr_name
                                        )));
                                    }
                                }
                            }
                            Value::Class(class) => {
                                if let Object::Class(class_data) = &mut *class.kind_mut() {
                                    if class_data.attrs.remove(&attr_name).is_none() {
                                        return Err(RuntimeError::new(format!(
                                            "class attribute '{}' does not exist",
                                            attr_name
                                        )));
                                    }
                                }
                            }
                            Value::Instance(instance) => {
                                match self.delete_attr_instance(&instance, &attr_name)? {
                                    AttrMutationOutcome::Done => {}
                                    AttrMutationOutcome::ExceptionHandled => return Ok(None),
                                }
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "attribute deletion unsupported type",
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
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(sub_values(left, right)?);
                    }
                    Opcode::BinaryMul => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(mul_values(left, right, &self.heap)?);
                    }
                    Opcode::BinaryMatMul => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(matmul_values(left, right)?);
                    }
                    Opcode::BinaryDiv => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(div_values(left, right)?);
                    }
                    Opcode::BinaryPow => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(pow_values(left, right)?);
                    }
                    Opcode::BinaryFloorDiv => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(floor_div_values(left, right)?);
                    }
                    Opcode::BinaryMod => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(mod_values(left, right)?);
                    }
                    Opcode::BinaryLShift => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(lshift_values(left, right)?);
                    }
                    Opcode::BinaryRShift => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(rshift_values(left, right)?);
                    }
                    Opcode::BinaryAnd => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(and_values(left, right)?);
                    }
                    Opcode::BinaryXor => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(xor_values(left, right)?);
                    }
                    Opcode::BinaryOr => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(or_values(left, right)?);
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
                        self.push_value(neg_value(value)?);
                    }
                    Opcode::UnaryNot => {
                        let value = self.pop_value()?;
                        self.push_value(Value::Bool(!is_truthy(&value)));
                    }
                    Opcode::UnaryPos => {
                        let value = self.pop_value()?;
                        self.push_value(pos_value(value)?);
                    }
                    Opcode::UnaryInvert => {
                        let value = self.pop_value()?;
                        self.push_value(invert_value(value)?);
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
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
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
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
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
                                Value::Bytes(obj) => match &*obj.kind() {
                                    Object::Bytes(values) => {
                                        let indices =
                                            slice_indices(values.len(), lower, upper, step)?;
                                        let mut result = Vec::with_capacity(indices.len());
                                        for idx in indices {
                                            result.push(values[idx]);
                                        }
                                        self.push_value(self.heap.alloc_bytes(result));
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
                                    }
                                },
                                Value::ByteArray(obj) => match &*obj.kind() {
                                    Object::ByteArray(values) => {
                                        let indices =
                                            slice_indices(values.len(), lower, upper, step)?;
                                        let mut result = Vec::with_capacity(indices.len());
                                        for idx in indices {
                                            result.push(values[idx]);
                                        }
                                        self.push_value(self.heap.alloc_bytearray(result));
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
                                    }
                                },
                                Value::MemoryView(obj) => match &*obj.kind() {
                                    Object::MemoryView(view) => match &*view.source.kind() {
                                        Object::Bytes(values) | Object::ByteArray(values) => {
                                            let indices =
                                                slice_indices(values.len(), lower, upper, step)?;
                                            let mut result = Vec::with_capacity(indices.len());
                                            for idx in indices {
                                                result.push(values[idx]);
                                            }
                                            self.push_value(self.heap.alloc_bytes(result));
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "subscript unsupported type",
                                            ));
                                        }
                                    },
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
                                    }
                                },
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
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
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
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
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
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
                                    }
                                },
                                Value::Bytes(obj) => match &*obj.kind() {
                                    Object::Bytes(values) => {
                                        let mut index_int = value_to_int(index)? as isize;
                                        if index_int < 0 {
                                            index_int += values.len() as isize;
                                        }
                                        if index_int < 0 || index_int as usize >= values.len() {
                                            return Err(RuntimeError::new("index out of range"));
                                        }
                                        self.push_value(Value::Int(values[index_int as usize] as i64));
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
                                    }
                                },
                                Value::ByteArray(obj) => match &*obj.kind() {
                                    Object::ByteArray(values) => {
                                        let mut index_int = value_to_int(index)? as isize;
                                        if index_int < 0 {
                                            index_int += values.len() as isize;
                                        }
                                        if index_int < 0 || index_int as usize >= values.len() {
                                            return Err(RuntimeError::new("index out of range"));
                                        }
                                        self.push_value(Value::Int(values[index_int as usize] as i64));
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
                                    }
                                },
                                Value::MemoryView(obj) => match &*obj.kind() {
                                    Object::MemoryView(view) => match &*view.source.kind() {
                                        Object::Bytes(values) | Object::ByteArray(values) => {
                                            let mut index_int = value_to_int(index)? as isize;
                                            if index_int < 0 {
                                                index_int += values.len() as isize;
                                            }
                                            if index_int < 0
                                                || index_int as usize >= values.len()
                                            {
                                                return Err(RuntimeError::new(
                                                    "index out of range",
                                                ));
                                            }
                                            self.push_value(Value::Int(
                                                values[index_int as usize] as i64,
                                            ));
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "subscript unsupported type",
                                            ));
                                        }
                                    },
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "subscript unsupported type",
                                        ));
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
                                Value::ByteArray(obj) => {
                                    if let Object::ByteArray(values) = &mut *obj.kind_mut() {
                                        let mut idx = value_to_int(index)? as isize;
                                        if idx < 0 {
                                            idx += values.len() as isize;
                                        }
                                        if idx < 0 || idx as usize >= values.len() {
                                            return Err(RuntimeError::new("index out of range"));
                                        }
                                        let byte = value_to_int(value)?;
                                        if !(0..=255).contains(&byte) {
                                            return Err(RuntimeError::new(
                                                "byte must be in range(0, 256)",
                                            ));
                                        }
                                        values[idx as usize] = byte as u8;
                                    }
                                    self.push_value(Value::ByteArray(obj));
                                }
                                Value::MemoryView(obj) => {
                                    let source = match &*obj.kind() {
                                        Object::MemoryView(view) => view.source.clone(),
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "store subscript unsupported type",
                                            ));
                                        }
                                    };
                                    match &mut *source.kind_mut() {
                                        Object::ByteArray(values) => {
                                            let mut idx = value_to_int(index)? as isize;
                                            if idx < 0 {
                                                idx += values.len() as isize;
                                            }
                                            if idx < 0 || idx as usize >= values.len() {
                                                return Err(RuntimeError::new(
                                                    "index out of range",
                                                ));
                                            }
                                            let byte = value_to_int(value)?;
                                            if !(0..=255).contains(&byte) {
                                                return Err(RuntimeError::new(
                                                    "byte must be in range(0, 256)",
                                                ));
                                            }
                                            values[idx as usize] = byte as u8;
                                        }
                                        Object::Bytes(_) => {
                                            return Err(RuntimeError::new(
                                                "cannot modify read-only memory",
                                            ));
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "store subscript unsupported type",
                                            ));
                                        }
                                    }
                                    self.push_value(Value::MemoryView(obj));
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "store subscript unsupported type",
                                    ));
                                }
                            },
                        }
                    }
                    Opcode::DeleteSubscript => {
                        let index = self.pop_value()?;
                        let target = self.pop_value()?;
                        match index {
                            Value::Slice { .. } => {
                                return Err(RuntimeError::new("slice deletion not supported"));
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
                                        values.remove(idx as usize);
                                    }
                                }
                                Value::Dict(obj) => {
                                    if let Object::Dict(entries) = &mut *obj.kind_mut() {
                                        let before = entries.len();
                                        entries.retain(|(key, _)| *key != index);
                                        if entries.len() == before {
                                            return Err(RuntimeError::new("key not found"));
                                        }
                                    }
                                }
                                Value::ByteArray(obj) => {
                                    if let Object::ByteArray(values) = &mut *obj.kind_mut() {
                                        let mut idx = value_to_int(index)? as isize;
                                        if idx < 0 {
                                            idx += values.len() as isize;
                                        }
                                        if idx < 0 || idx as usize >= values.len() {
                                            return Err(RuntimeError::new("index out of range"));
                                        }
                                        values.remove(idx as usize);
                                    }
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "subscript deletion unsupported type",
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
                        let metaclass_value = self.pop_value()?;
                        let class_metaclass = if matches!(metaclass_value, Value::None) {
                            None
                        } else {
                            Some(metaclass_value)
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
                        frame.class_metaclass = class_metaclass;
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
                                            return Err(RuntimeError::new(
                                                "defaults must be tuple",
                                            ));
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
                                let frame = self.frames.get_mut(caller_idx).ok_or_else(|| {
                                    RuntimeError::new("builtin caller frame missing")
                                })?;
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
                                self.push_value(Value::Exception(ExceptionObject::new(
                                    name, message,
                                )));
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
                                let frame = self.frames.get_mut(caller_idx).ok_or_else(|| {
                                    RuntimeError::new("builtin caller frame missing")
                                })?;
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
                                self.push_value(Value::Exception(ExceptionObject::new(
                                    name, message,
                                )));
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
                                let frame = self.frames.get_mut(caller_idx).ok_or_else(|| {
                                    RuntimeError::new("builtin caller frame missing")
                                })?;
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
                                self.push_value(Value::Exception(ExceptionObject::new(
                                    name, message,
                                )));
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
                                let frame = self.frames.get_mut(caller_idx).ok_or_else(|| {
                                    RuntimeError::new("builtin caller frame missing")
                                })?;
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
                                self.push_value(Value::Exception(ExceptionObject::new(
                                    name, message,
                                )));
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
                                let frame = self.frames.get_mut(caller_idx).ok_or_else(|| {
                                    RuntimeError::new("builtin caller frame missing")
                                })?;
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
                                self.push_value(Value::Exception(ExceptionObject::new(
                                    name, message,
                                )));
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
                                let frame = self.frames.get_mut(caller_idx).ok_or_else(|| {
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
                        self.ensure_sync_iterator_target(&value)?;
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
                            Value::Set(obj) | Value::FrozenSet(obj) => IteratorObject {
                                kind: IteratorKind::Set(obj),
                                index: 0,
                            },
                            Value::Bytes(obj) => IteratorObject {
                                kind: IteratorKind::Bytes(obj),
                                index: 0,
                            },
                            Value::ByteArray(obj) => IteratorObject {
                                kind: IteratorKind::ByteArray(obj),
                                index: 0,
                            },
                            Value::MemoryView(obj) => IteratorObject {
                                kind: IteratorKind::MemoryView(obj),
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
                                let next_value = self.iterator_next_value(&iterator_ref);
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
                        match mode {
                            0 => {
                                let frame = self.frames.last().expect("frame exists");
                                let value = frame.active_exception.clone().ok_or_else(|| {
                                    RuntimeError::new("no active exception to reraise")
                                })?;
                                self.raise_exception(value)?;
                            }
                            1 => {
                                let value = self.pop_value()?;
                                self.raise_exception(value)?;
                            }
                            2 => {
                                let cause = self.pop_value()?;
                                let value = self.pop_value()?;
                                self.raise_exception_with_cause(value, Some(cause))?;
                            }
                            _ => {
                                return Err(RuntimeError::new("invalid raise mode"));
                            }
                        }
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
                            match self.class_value_from_module(
                                &frame.module,
                                frame.class_bases,
                                frame.class_metaclass,
                            )? {
                                ClassBuildOutcome::Value(value) => value,
                                ClassBuildOutcome::ExceptionHandled => return Ok(None),
                            }
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
                            match self.class_value_from_module(
                                &frame.module,
                                frame.class_bases,
                                frame.class_metaclass,
                            )? {
                                ClassBuildOutcome::Value(value) => value,
                                ClassBuildOutcome::ExceptionHandled => return Ok(None),
                            }
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
        self.raise_exception_with_cause(value, None)
    }

    fn raise_exception_with_cause(
        &mut self,
        value: Value,
        explicit_cause: Option<Value>,
    ) -> Result<(), RuntimeError> {
        let mut exc = normalize_exception(value)?;
        if let Value::Exception(exc_data) = &mut exc {
            if let Some(cause_value) = explicit_cause {
                if matches!(cause_value, Value::None) {
                    exc_data.suppress_context = true;
                    exc_data.cause = None;
                } else {
                    let cause = normalize_exception(cause_value)?;
                    if let Value::Exception(cause_data) = cause {
                        exc_data.cause = Some(Box::new(cause_data));
                        exc_data.suppress_context = true;
                    }
                }
            } else if let Some(current) = self
                .frames
                .last()
                .and_then(|frame| frame.active_exception.clone())
            {
                let context = normalize_exception(current)?;
                if let Value::Exception(context_data) = context {
                    exc_data.context = Some(Box::new(context_data));
                }
            }
        }

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
        let exception = Value::Exception(ExceptionObject::new(
            exception_type.to_string(),
            Some(err.message),
        ));
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
        if let Value::Exception(exception) = exc {
            if let Some(cause) = &exception.cause {
                output.push_str(
                    "\nThe above exception was the direct cause of the following exception:\n",
                );
                output.push_str(&self.format_exception_object(cause));
            } else if !exception.suppress_context {
                if let Some(context) = &exception.context {
                    output.push_str(
                        "\nDuring handling of the above exception, another exception occurred:\n",
                    );
                    output.push_str(&self.format_exception_object(context));
                }
            }
        }
        output
    }

    fn format_exception_object(&self, exception: &ExceptionObject) -> String {
        match &exception.message {
            Some(message) if !message.is_empty() => format!("{}: {}", exception.name, message),
            _ => exception.name.clone(),
        }
    }

    fn class_value_from_module(
        &mut self,
        module: &ObjRef,
        bases: Vec<ObjRef>,
        metaclass: Option<Value>,
    ) -> Result<ClassBuildOutcome, RuntimeError> {
        let (name, attrs) = match &*module.kind() {
            Object::Module(module_data) => (module_data.name.clone(), module_data.globals.clone()),
            _ => ("<class>".to_string(), HashMap::new()),
        };

        if let Some(meta) = metaclass {
            if matches!(meta, Value::Builtin(BuiltinFunction::Type)) {
                return Ok(ClassBuildOutcome::Value(
                    self.build_default_class_value(name, attrs, bases),
                ));
            }
            if self.frames.is_empty() {
                return Err(RuntimeError::new("metaclass call requires active frame"));
            }
            let namespace = self.heap.alloc_dict(
                attrs.iter()
                    .map(|(key, value)| (Value::Str(key.clone()), value.clone()))
                    .collect::<Vec<_>>(),
            );
            let bases_tuple = self.heap.alloc_tuple(
                bases.iter()
                    .cloned()
                    .map(Value::Class)
                    .collect::<Vec<_>>(),
            );
            return match self.call_internal(
                meta,
                vec![Value::Str(name), bases_tuple, namespace],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => {
                    if matches!(value, Value::Class(_)) {
                        Ok(ClassBuildOutcome::Value(value))
                    } else {
                        Err(RuntimeError::new("metaclass must return a class object"))
                    }
                }
                InternalCallOutcome::CallerExceptionHandled => {
                    Ok(ClassBuildOutcome::ExceptionHandled)
                }
            };
        }

        Ok(ClassBuildOutcome::Value(
            self.build_default_class_value(name, attrs, bases),
        ))
    }

    fn build_default_class_value(
        &mut self,
        name: String,
        attrs: HashMap<String, Value>,
        bases: Vec<ObjRef>,
    ) -> Value {
        let class = ClassObject::new(name, bases.clone());
        let class_value = self.heap.alloc_class(class);
        if let Value::Class(class_ref) = &class_value {
            if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
                class_data.attrs.extend(attrs);
                if let Some(slot_names) =
                    slot_names_from_value(class_data.attrs.get("__slots__").cloned())
                {
                    class_data.slots = Some(slot_names.clone());
                    class_data.attrs.insert(
                        "__slots__".to_string(),
                        self.heap.alloc_tuple(
                            slot_names
                                .into_iter()
                                .map(Value::Str)
                                .collect::<Vec<_>>(),
                        ),
                    );
                }
                class_data
                    .attrs
                    .insert("__name__".to_string(), Value::Str(class_data.name.clone()));
                class_data.attrs.insert(
                    "__bases__".to_string(),
                    self.heap.alloc_tuple(
                        class_data
                            .bases
                            .iter()
                            .cloned()
                            .map(Value::Class)
                            .collect::<Vec<_>>(),
                    ),
                );
            }
            if let Ok(mro) = self.build_class_mro(class_ref, &bases) {
                if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
                    class_data.mro = mro.clone();
                    let mro_values = mro.into_iter().map(Value::Class).collect::<Vec<_>>();
                    class_data
                        .attrs
                        .insert("__mro__".to_string(), self.heap.alloc_tuple(mro_values));
                }
            }
        }
        class_value
    }

    fn class_mro_entries(&self, class: &ObjRef) -> Vec<ObjRef> {
        match &*class.kind() {
            Object::Class(class_data) if !class_data.mro.is_empty() => class_data.mro.clone(),
            Object::Class(_) => vec![class.clone()],
            _ => Vec::new(),
        }
    }

    fn build_class_mro(
        &self,
        class: &ObjRef,
        bases: &[ObjRef],
    ) -> Result<Vec<ObjRef>, RuntimeError> {
        if bases.is_empty() {
            return Ok(vec![class.clone()]);
        }

        let mut seqs: Vec<Vec<ObjRef>> = Vec::new();
        for base in bases {
            seqs.push(self.class_mro_entries(base));
        }
        seqs.push(bases.to_vec());

        let mut merged = Vec::new();
        loop {
            seqs.retain(|seq| !seq.is_empty());
            if seqs.is_empty() {
                break;
            }

            let mut candidate = None;
            for seq in &seqs {
                let head = seq[0].clone();
                let in_tail = seqs
                    .iter()
                    .any(|other| other.iter().skip(1).any(|entry| entry.id() == head.id()));
                if !in_tail {
                    candidate = Some(head);
                    break;
                }
            }

            let Some(head) = candidate else {
                return Err(RuntimeError::new(
                    "cannot create a consistent method resolution order (MRO)",
                ));
            };
            merged.push(head.clone());
            for seq in &mut seqs {
                if !seq.is_empty() && seq[0].id() == head.id() {
                    seq.remove(0);
                }
            }
        }

        let mut out = vec![class.clone()];
        out.extend(merged);
        Ok(out)
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
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                func_data.code.is_coroutine,
                func_data.code.is_async_generator,
            )) {
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
            Object::Class(_) => Ok(Value::Class(receiver.clone())),
            Object::Generator(_) => Ok(Value::Generator(receiver.clone())),
            _ => Err(RuntimeError::new("unsupported bound method receiver")),
        }
    }

    fn receiver_from_value(&self, value: &Value) -> Result<ObjRef, RuntimeError> {
        match value {
            Value::Instance(obj) | Value::Class(obj) | Value::Generator(obj) => Ok(obj.clone()),
            _ => Err(RuntimeError::new("unsupported bound-method receiver value")),
        }
    }

    fn class_of_value(&self, value: &Value) -> Option<ObjRef> {
        match value {
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => Some(instance_data.class.clone()),
                _ => None,
            },
            Value::Class(class) => Some(class.clone()),
            Value::Super(super_obj) => match &*super_obj.kind() {
                Object::Super(data) => Some(data.object_type.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    fn bind_descriptor_method(
        &mut self,
        method: Value,
        receiver: &Value,
    ) -> Result<Option<Value>, RuntimeError> {
        match method {
            Value::Function(func) => {
                let receiver_ref = self.receiver_from_value(receiver)?;
                Ok(Some(
                    self.heap
                        .alloc_bound_method(BoundMethod::new(func, receiver_ref)),
                ))
            }
            _ => Ok(None),
        }
    }

    fn lookup_bound_special_method(
        &mut self,
        receiver: &Value,
        method_name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some(class_ref) = self.class_of_value(receiver) else {
            return Ok(None);
        };
        let Some(method) = class_attr_lookup(&class_ref, method_name) else {
            return Ok(None);
        };
        self.bind_descriptor_method(method, receiver)
    }

    fn descriptor_hooks(
        &mut self,
        descriptor: &Value,
    ) -> Result<(Option<Value>, Option<Value>, Option<Value>), RuntimeError> {
        if matches!(descriptor, Value::Function(_)) {
            return Ok((None, None, None));
        }

        let Some(class_ref) = self.class_of_value(descriptor) else {
            return Ok((None, None, None));
        };
        let get = class_attr_lookup(&class_ref, "__get__")
            .map(|method| self.bind_descriptor_method(method, descriptor))
            .transpose()?
            .flatten();
        let set = class_attr_lookup(&class_ref, "__set__")
            .map(|method| self.bind_descriptor_method(method, descriptor))
            .transpose()?
            .flatten();
        let delete = class_attr_lookup(&class_ref, "__delete__")
            .map(|method| self.bind_descriptor_method(method, descriptor))
            .transpose()?
            .flatten();
        Ok((get, set, delete))
    }

    fn call_internal(
        &mut self,
        callable: Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<InternalCallOutcome, RuntimeError> {
        let caller_depth = self.frames.len();
        if caller_depth == 0 {
            return Err(RuntimeError::new(
                "internal call requires an active execution frame",
            ));
        }
        let caller_ip = self.frames.last().map(|frame| frame.ip).unwrap_or(0);

        let needs_run = match callable {
            Value::Function(func) => {
                let func_data = match &*func.kind() {
                    Object::Function(data) => data.clone(),
                    _ => return Err(RuntimeError::new("attempted to call non-function")),
                };
                let depth_before = self.frames.len();
                self.push_function_call(&func_data, args, kwargs)?;
                self.frames.len() > depth_before
            }
            Value::BoundMethod(method) => {
                let method_data = match &*method.kind() {
                    Object::BoundMethod(data) => data.clone(),
                    _ => return Err(RuntimeError::new("attempted to call non-function")),
                };
                match &*method_data.function.kind() {
                    Object::Function(data) => {
                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                        bound_args.push(self.receiver_value(&method_data.receiver)?);
                        bound_args.extend(args);
                        let depth_before = self.frames.len();
                        self.push_function_call(data, bound_args, kwargs)?;
                        self.frames.len() > depth_before
                    }
                    Object::NativeMethod(native) => {
                        match self.call_native_method(
                            native.kind,
                            method_data.receiver.clone(),
                            args,
                            kwargs,
                        )? {
                            NativeCallResult::Value(result) => {
                                return Ok(InternalCallOutcome::Value(result));
                            }
                            NativeCallResult::PropagatedException => {
                                self.propagate_pending_generator_exception()?;
                                return Ok(InternalCallOutcome::CallerExceptionHandled);
                            }
                        }
                    }
                    _ => return Err(RuntimeError::new("attempted to call non-function")),
                }
            }
            Value::Builtin(builtin) => {
                let result = self.call_builtin(builtin, args, kwargs)?;
                return Ok(InternalCallOutcome::Value(result));
            }
            _ => {
                return Err(RuntimeError::new("attempted to call non-function"));
            }
        };

        if !needs_run {
            let value = self.pop_value()?;
            return Ok(InternalCallOutcome::Value(value));
        }

        let previous_stop = self.run_stop_depth;
        self.run_stop_depth = Some(caller_depth);
        let run_result = self.run();
        self.run_stop_depth = previous_stop;
        run_result?;

        if self.frames.len() < caller_depth {
            return Err(RuntimeError::new(
                "internal call unexpectedly unwound the caller frame",
            ));
        }

        let caller = self
            .frames
            .get(caller_depth - 1)
            .ok_or_else(|| RuntimeError::new("caller frame missing"))?;
        if caller.active_exception.is_some() || caller.ip != caller_ip {
            return Ok(InternalCallOutcome::CallerExceptionHandled);
        }

        let value = self.pop_value()?;
        Ok(InternalCallOutcome::Value(value))
    }

    fn load_attr_class(
        &mut self,
        class: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let class_name = match &*class.kind() {
            Object::Class(class_data) => class_data.name.clone(),
            _ => "<class>".to_string(),
        };
        let attr = class_attr_lookup(class, attr_name).ok_or_else(|| {
            RuntimeError::new(format!(
                "class '{}' has no attribute '{}'",
                class_name, attr_name
            ))
        })?;

        let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
        if let Some(getter) = getter {
            return Ok(
                match self.call_internal(
                    getter,
                    vec![Value::None, Value::Class(class.clone())],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                    InternalCallOutcome::CallerExceptionHandled => {
                        AttrAccessOutcome::ExceptionHandled
                    }
                },
            );
        }

        Ok(AttrAccessOutcome::Value(attr))
    }

    fn load_attr_instance(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return Err(RuntimeError::new("attribute access unsupported type")),
        };

        let class_attr = class_attr_lookup(&class_ref, attr_name);
        if let Some(attr) = class_attr.clone() {
            let (getter, setter, deleter) = self.descriptor_hooks(&attr)?;
            if setter.is_some() || deleter.is_some() {
                if let Some(getter) = getter {
                    return Ok(
                        match self.call_internal(
                            getter,
                            vec![
                                Value::Instance(instance.clone()),
                                Value::Class(class_ref.clone()),
                            ],
                            HashMap::new(),
                        )? {
                            InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                            InternalCallOutcome::CallerExceptionHandled => {
                                AttrAccessOutcome::ExceptionHandled
                            }
                        },
                    );
                }
                return Ok(AttrAccessOutcome::Value(attr));
            }
        }

        if let Object::Instance(instance_data) = &*instance.kind() {
            if let Some(attr) = instance_data.attrs.get(attr_name).cloned() {
                return Ok(AttrAccessOutcome::Value(attr));
            }
        }

        if let Some(attr) = class_attr {
            if let Value::Function(func) = attr.clone() {
                let bound = BoundMethod::new(func, instance.clone());
                return Ok(AttrAccessOutcome::Value(
                    self.heap.alloc_bound_method(bound),
                ));
            }
            let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
            if let Some(getter) = getter {
                return Ok(
                    match self.call_internal(
                        getter,
                        vec![
                            Value::Instance(instance.clone()),
                            Value::Class(class_ref.clone()),
                        ],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrAccessOutcome::ExceptionHandled
                        }
                    },
                );
            }
            return Ok(AttrAccessOutcome::Value(attr));
        }

        if let Some(getattr_method) =
            self.lookup_bound_special_method(&Value::Instance(instance.clone()), "__getattr__")?
        {
            return Ok(
                match self.call_internal(
                    getattr_method,
                    vec![Value::Str(attr_name.to_string())],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                    InternalCallOutcome::CallerExceptionHandled => {
                        AttrAccessOutcome::ExceptionHandled
                    }
                },
            );
        }

        let class_name = match &*class_ref.kind() {
            Object::Class(class_data) => class_data.name.clone(),
            _ => "<class>".to_string(),
        };
        Err(RuntimeError::new(format!(
            "'{}' object has no attribute '{}'",
            class_name, attr_name
        )))
    }

    fn load_attr_super(
        &mut self,
        super_ref: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let (start_class, receiver, object_type) = match &*super_ref.kind() {
            Object::Super(data) => (
                data.start_class.clone(),
                data.object.clone(),
                data.object_type.clone(),
            ),
            _ => return Err(RuntimeError::new("attribute access unsupported type")),
        };

        let receiver_value = self.receiver_value(&receiver)?;
        let owner_value = Value::Class(object_type.clone());
        let mro = self.class_mro_entries(&object_type);
        let start_idx = mro
            .iter()
            .position(|entry| entry.id() == start_class.id())
            .map(|idx| idx + 1)
            .unwrap_or(0);

        for class in mro.into_iter().skip(start_idx) {
            if let Some(attr) = class_attr_lookup(&class, attr_name) {
                if let Value::Function(func) = attr.clone() {
                    let bound = BoundMethod::new(func, receiver.clone());
                    return Ok(AttrAccessOutcome::Value(
                        self.heap.alloc_bound_method(bound),
                    ));
                }
                let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
                if let Some(getter) = getter {
                    return Ok(
                        match self.call_internal(
                            getter,
                            vec![receiver_value.clone(), owner_value.clone()],
                            HashMap::new(),
                        )? {
                            InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                            InternalCallOutcome::CallerExceptionHandled => {
                                AttrAccessOutcome::ExceptionHandled
                            }
                        },
                    );
                }
                return Ok(AttrAccessOutcome::Value(attr));
            }
        }

        Err(RuntimeError::new(format!(
            "super object has no attribute '{}'",
            attr_name
        )))
    }

    fn load_attr_module(
        &mut self,
        module: &ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let (module_name, attr) = match &*module.kind() {
            Object::Module(module_data) => {
                let attr = module_data.globals.get(attr_name).cloned();
                let module_name = module_data.name.clone();
                (module_name, attr)
            }
            _ => {
                return Err(RuntimeError::new("attribute access unsupported type"));
            }
        };
        if let Some(attr) = attr {
            return Ok(attr);
        }
        if let Some(attr) = module_name.split('.').last().and_then(|suffix| {
            if suffix == attr_name {
                Some(Value::Module(module.clone()))
            } else {
                None
            }
        }) {
            return Ok(attr);
        }
        if let Some(submodule) = self.load_submodule(module, attr_name) {
            return Ok(Value::Module(submodule));
        }
        Err(RuntimeError::new(format!(
            "module '{}' has no attribute '{}'",
            module_name, attr_name
        )))
    }

    fn store_attr_instance(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
        value: Value,
    ) -> Result<AttrMutationOutcome, RuntimeError> {
        if let Some(setattr_method) =
            self.lookup_bound_special_method(&Value::Instance(instance.clone()), "__setattr__")?
        {
            return Ok(
                match self.call_internal(
                    setattr_method,
                    vec![Value::Str(attr_name.to_string()), value],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                    InternalCallOutcome::CallerExceptionHandled => {
                        AttrMutationOutcome::ExceptionHandled
                    }
                },
            );
        }

        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return Err(RuntimeError::new("attribute assignment unsupported type")),
        };

        if let Some(descriptor) = class_attr_lookup(&class_ref, attr_name) {
            let (_getter, setter, _deleter) = self.descriptor_hooks(&descriptor)?;
            if let Some(setter) = setter {
                return Ok(
                    match self.call_internal(
                        setter,
                        vec![Value::Instance(instance.clone()), value],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrMutationOutcome::ExceptionHandled
                        }
                    },
                );
            }
        }

        if let Some(allowed_slots) = collect_slot_names(&class_ref) {
            let allowed = allowed_slots.iter().any(|name| name == attr_name);
            if !allowed {
                return Err(RuntimeError::new(format!(
                    "'{}' object has no attribute '{}'",
                    class_name_for_instance(instance).unwrap_or_else(|| "object".to_string()),
                    attr_name
                )));
            }
        }

        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data.attrs.insert(attr_name.to_string(), value);
        }
        Ok(AttrMutationOutcome::Done)
    }

    fn delete_attr_instance(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrMutationOutcome, RuntimeError> {
        if let Some(delattr_method) =
            self.lookup_bound_special_method(&Value::Instance(instance.clone()), "__delattr__")?
        {
            return Ok(
                match self.call_internal(
                    delattr_method,
                    vec![Value::Str(attr_name.to_string())],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                    InternalCallOutcome::CallerExceptionHandled => {
                        AttrMutationOutcome::ExceptionHandled
                    }
                },
            );
        }

        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return Err(RuntimeError::new("attribute deletion unsupported type")),
        };

        if let Some(descriptor) = class_attr_lookup(&class_ref, attr_name) {
            let (_getter, _setter, deleter) = self.descriptor_hooks(&descriptor)?;
            if let Some(deleter) = deleter {
                return Ok(
                    match self.call_internal(
                        deleter,
                        vec![Value::Instance(instance.clone())],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrMutationOutcome::ExceptionHandled
                        }
                    },
                );
            }
        }

        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            if instance_data.attrs.remove(attr_name).is_some() {
                return Ok(AttrMutationOutcome::Done);
            }
        }

        Err(RuntimeError::new(format!(
            "attribute '{}' does not exist",
            attr_name
        )))
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
            NativeMethodKind::GeneratorAwait => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("__await__() expects no arguments"));
                }
                let is_coroutine = match &*receiver.kind() {
                    Object::Generator(state) => state.is_coroutine,
                    _ => false,
                };
                if is_coroutine {
                    Ok(NativeCallResult::Value(Value::Generator(receiver)))
                } else {
                    Err(RuntimeError::new("object is not awaitable"))
                }
            }
            NativeMethodKind::GeneratorANext => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("__anext__() expects no arguments"));
                }
                match &*receiver.kind() {
                    Object::Generator(state) if state.is_async_generator => {}
                    _ => return Err(RuntimeError::new("object is not an async generator")),
                }
                match self.resume_generator(&receiver, None, None, GeneratorResumeKind::Next)? {
                    GeneratorResumeOutcome::Yield(value) => Ok(NativeCallResult::Value(
                        self.make_immediate_coroutine(value),
                    )),
                    GeneratorResumeOutcome::Complete(_) => {
                        Err(RuntimeError::new("StopAsyncIteration"))
                    }
                    GeneratorResumeOutcome::PropagatedException => {
                        Ok(NativeCallResult::PropagatedException)
                    }
                }
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

    fn make_immediate_coroutine(&mut self, value: Value) -> Value {
        let mut code = CodeObject::new("<awaitable>", "<builtin>");
        let const_idx = code.add_const(value);
        code.instructions
            .push(Instruction::new(Opcode::LoadConst, Some(const_idx as u32)));
        code.instructions
            .push(Instruction::new(Opcode::ReturnValue, None));
        code.is_generator = true;
        code.is_coroutine = true;
        code.is_async_generator = false;
        let code = Rc::new(code);
        let module = self
            .frames
            .last()
            .map(|frame| frame.module.clone())
            .unwrap_or_else(|| self.main_module.clone());
        let mut frame = Frame::new(code, module, false, false, Vec::new());
        let generator = match self.heap.alloc_generator(GeneratorObject::new(true, false)) {
            Value::Generator(obj) => obj,
            _ => unreachable!(),
        };
        frame.generator_owner = Some(generator.clone());
        self.generator_states.insert(generator.id(), frame);
        Value::Generator(generator)
    }

    fn awaitable_from_value(&mut self, value: Value) -> Result<Value, RuntimeError> {
        match value {
            Value::Generator(generator) => {
                let (is_coroutine, is_async_generator) = match &*generator.kind() {
                    Object::Generator(state) => (state.is_coroutine, state.is_async_generator),
                    _ => (false, false),
                };
                if is_coroutine {
                    Ok(Value::Generator(generator))
                } else if is_async_generator {
                    Err(RuntimeError::new("async generator object is not awaitable"))
                } else {
                    Err(RuntimeError::new("object is not awaitable"))
                }
            }
            Value::Iterator(_) => Err(RuntimeError::new("object is not awaitable")),
            other => {
                let method = self
                    .lookup_bound_special_method(&other, "__await__")?
                    .ok_or_else(|| RuntimeError::new("object is not awaitable"))?;
                match self.call_internal(method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(awaitable) => match awaitable {
                        Value::Generator(generator) => {
                            if let Object::Generator(state) = &*generator.kind() {
                                if state.is_async_generator {
                                    return Err(RuntimeError::new(
                                        "__await__() returned an async generator",
                                    ));
                                }
                            }
                            Ok(Value::Generator(generator))
                        }
                        Value::Iterator(iterator) => Ok(Value::Iterator(iterator)),
                        _ => Err(RuntimeError::new("__await__() returned non-iterator")),
                    },
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(RuntimeError::new("__await__() failed"))
                    }
                }
            }
        }
    }

    fn run_awaitable(&mut self, awaitable: Value) -> Result<Value, RuntimeError> {
        match self.awaitable_from_value(awaitable)? {
            Value::Generator(generator) => loop {
                match self.resume_generator(&generator, None, None, GeneratorResumeKind::Next)? {
                    GeneratorResumeOutcome::Yield(_) => {}
                    GeneratorResumeOutcome::Complete(value) => return Ok(value),
                    GeneratorResumeOutcome::PropagatedException => {
                        self.propagate_pending_generator_exception()?;
                        return Err(RuntimeError::new("awaitable execution failed"));
                    }
                }
            },
            Value::Iterator(iterator) => {
                while self.iterator_next_value(&iterator).is_some() {}
                Ok(Value::None)
            }
            _ => Err(RuntimeError::new("object is not awaitable")),
        }
    }

    fn is_awaitable_value(&self, value: &Value) -> bool {
        match value {
            Value::Generator(generator) => match &*generator.kind() {
                Object::Generator(state) => state.is_coroutine,
                _ => false,
            },
            Value::Iterator(_) => false,
            _ => self
                .class_of_value(value)
                .and_then(|class| class_attr_lookup(&class, "__await__"))
                .is_some(),
        }
    }

    fn ensure_sync_iterator_target(&self, value: &Value) -> Result<(), RuntimeError> {
        if let Value::Generator(generator) = value {
            if let Object::Generator(state) = &*generator.kind() {
                if state.is_coroutine || state.is_async_generator {
                    return Err(RuntimeError::new("object is not iterable"));
                }
            }
        }
        Ok(())
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
            Value::Set(obj) | Value::FrozenSet(obj) => Ok(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::Set(obj),
                index: 0,
            })),
            Value::Bytes(obj) => Ok(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::Bytes(obj),
                index: 0,
            })),
            Value::ByteArray(obj) => Ok(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::ByteArray(obj),
                index: 0,
            })),
            Value::MemoryView(obj) => Ok(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::MemoryView(obj),
                index: 0,
            })),
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
                let next_value = self.iterator_next_value(iterator_ref);
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
                    IteratorKind::Set(_) => "set_iterator",
                    IteratorKind::Bytes(_) => "bytes_iterator",
                    IteratorKind::ByteArray(_) => "bytearray_iterator",
                    IteratorKind::MemoryView(_) => "memoryview_iterator",
                },
                _ => "iterator",
            },
            Value::Generator(_) => "generator",
            _ => "object",
        }
    }

    fn iterator_next_value(&self, iterator_ref: &ObjRef) -> Option<Value> {
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
                IteratorKind::Set(set) => match &*set.kind() {
                    Object::Set(values) | Object::FrozenSet(values) => {
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
                IteratorKind::Bytes(bytes) => match &*bytes.kind() {
                    Object::Bytes(values) => {
                        if state.index >= values.len() {
                            None
                        } else {
                            let value = Value::Int(values[state.index] as i64);
                            state.index += 1;
                            Some(value)
                        }
                    }
                    _ => None,
                },
                IteratorKind::ByteArray(bytes) => match &*bytes.kind() {
                    Object::ByteArray(values) => {
                        if state.index >= values.len() {
                            None
                        } else {
                            let value = Value::Int(values[state.index] as i64);
                            state.index += 1;
                            Some(value)
                        }
                    }
                    _ => None,
                },
                IteratorKind::MemoryView(view_ref) => match &*view_ref.kind() {
                    Object::MemoryView(view) => match &*view.source.kind() {
                        Object::Bytes(values) | Object::ByteArray(values) => {
                            if state.index >= values.len() {
                                None
                            } else {
                                let value = Value::Int(values[state.index] as i64);
                                state.index += 1;
                                Some(value)
                            }
                        }
                        _ => None,
                    },
                    _ => None,
                },
            },
            _ => None,
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
            BuiltinFunction::GetAttr => self.builtin_getattr(args, kwargs),
            BuiltinFunction::SetAttr => self.builtin_setattr(args, kwargs),
            BuiltinFunction::DelAttr => self.builtin_delattr(args, kwargs),
            BuiltinFunction::HasAttr => self.builtin_hasattr(args, kwargs),
            BuiltinFunction::Iter => self.builtin_iter(args, kwargs),
            BuiltinFunction::Next => self.builtin_next(args, kwargs),
            BuiltinFunction::AIter => self.builtin_aiter(args, kwargs),
            BuiltinFunction::ANext => self.builtin_anext(args, kwargs),
            BuiltinFunction::Super => self.builtin_super(args, kwargs),
            BuiltinFunction::Import => self.builtin_import(args, kwargs),
            BuiltinFunction::ImportModule => self.builtin_import_module(args, kwargs),
            BuiltinFunction::FindSpec => self.builtin_find_spec(args, kwargs),
            BuiltinFunction::RandomSeed => self.builtin_random_seed(args, kwargs),
            BuiltinFunction::RandomRandom => self.builtin_random_random(args, kwargs),
            BuiltinFunction::RandomRandRange => self.builtin_random_randrange(args, kwargs),
            BuiltinFunction::RandomRandInt => self.builtin_random_randint(args, kwargs),
            BuiltinFunction::RandomGetRandBits => self.builtin_random_getrandbits(args, kwargs),
            BuiltinFunction::RandomChoice => self.builtin_random_choice(args, kwargs),
            BuiltinFunction::RandomShuffle => self.builtin_random_shuffle(args, kwargs),
            BuiltinFunction::MathSqrt => self.builtin_math_sqrt(args, kwargs),
            BuiltinFunction::MathFloor => self.builtin_math_floor(args, kwargs),
            BuiltinFunction::MathCeil => self.builtin_math_ceil(args, kwargs),
            BuiltinFunction::MathIsFinite => self.builtin_math_isfinite(args, kwargs),
            BuiltinFunction::MathIsInf => self.builtin_math_isinf(args, kwargs),
            BuiltinFunction::MathIsNaN => self.builtin_math_isnan(args, kwargs),
            BuiltinFunction::TimeTime => self.builtin_time_time(args, kwargs),
            BuiltinFunction::TimeMonotonic => self.builtin_time_monotonic(args, kwargs),
            BuiltinFunction::TimeSleep => self.builtin_time_sleep(args, kwargs),
            BuiltinFunction::OsGetCwd => self.builtin_os_getcwd(args, kwargs),
            BuiltinFunction::OsListDir => self.builtin_os_listdir(args, kwargs),
            BuiltinFunction::OsPathExists => self.builtin_os_path_exists(args, kwargs),
            BuiltinFunction::OsPathJoin => self.builtin_os_path_join(args, kwargs),
            BuiltinFunction::JsonDumps => self.builtin_json_dumps(args, kwargs),
            BuiltinFunction::JsonLoads => self.builtin_json_loads(args, kwargs),
            BuiltinFunction::CodecsEncode => self.builtin_codecs_encode(args, kwargs),
            BuiltinFunction::CodecsDecode => self.builtin_codecs_decode(args, kwargs),
            BuiltinFunction::ReSearch => self.builtin_re_search(args, kwargs),
            BuiltinFunction::ReMatch => self.builtin_re_match(args, kwargs),
            BuiltinFunction::ReFullMatch => self.builtin_re_fullmatch(args, kwargs),
            BuiltinFunction::OperatorAdd => self.builtin_operator_add(args, kwargs),
            BuiltinFunction::OperatorSub => self.builtin_operator_sub(args, kwargs),
            BuiltinFunction::OperatorMul => self.builtin_operator_mul(args, kwargs),
            BuiltinFunction::OperatorTrueDiv => self.builtin_operator_truediv(args, kwargs),
            BuiltinFunction::OperatorEq => self.builtin_operator_eq(args, kwargs),
            BuiltinFunction::OperatorContains => self.builtin_operator_contains(args, kwargs),
            BuiltinFunction::OperatorGetItem => self.builtin_operator_getitem(args, kwargs),
            BuiltinFunction::ItertoolsChain => self.builtin_itertools_chain(args, kwargs),
            BuiltinFunction::ItertoolsRepeat => self.builtin_itertools_repeat(args, kwargs),
            BuiltinFunction::FunctoolsReduce => self.builtin_functools_reduce(args, kwargs),
            BuiltinFunction::CollectionsCounter => self.builtin_collections_counter(args, kwargs),
            BuiltinFunction::CollectionsDeque => self.builtin_collections_deque(args, kwargs),
            BuiltinFunction::InspectIsFunction => self.builtin_inspect_isfunction(args, kwargs),
            BuiltinFunction::InspectIsClass => self.builtin_inspect_isclass(args, kwargs),
            BuiltinFunction::InspectIsModule => self.builtin_inspect_ismodule(args, kwargs),
            BuiltinFunction::InspectIsGenerator => self.builtin_inspect_isgenerator(args, kwargs),
            BuiltinFunction::InspectIsCoroutine => self.builtin_inspect_iscoroutine(args, kwargs),
            BuiltinFunction::InspectIsAwaitable => self.builtin_inspect_isawaitable(args, kwargs),
            BuiltinFunction::InspectIsAsyncGen => self.builtin_inspect_isasyncgen(args, kwargs),
            BuiltinFunction::TypesModuleType => self.builtin_types_moduletype(args, kwargs),
            BuiltinFunction::IoOpen => self.builtin_io_open(args, kwargs),
            BuiltinFunction::IoReadText => self.builtin_io_read_text(args, kwargs),
            BuiltinFunction::IoWriteText => self.builtin_io_write_text(args, kwargs),
            BuiltinFunction::DateTimeNow => self.builtin_datetime_now(args, kwargs),
            BuiltinFunction::DateToday => self.builtin_datetime_today(args, kwargs),
            BuiltinFunction::AsyncioRun => self.builtin_asyncio_run(args, kwargs),
            BuiltinFunction::AsyncioSleep => self.builtin_asyncio_sleep(args, kwargs),
            BuiltinFunction::AsyncioCreateTask => self.builtin_asyncio_create_task(args, kwargs),
            BuiltinFunction::AsyncioGather => self.builtin_asyncio_gather(args, kwargs),
            BuiltinFunction::ThreadingGetIdent => self.builtin_threading_get_ident(args, kwargs),
            BuiltinFunction::ThreadingCurrentThread => {
                self.builtin_threading_current_thread(args, kwargs)
            }
            BuiltinFunction::ThreadingMainThread => self.builtin_threading_main_thread(args, kwargs),
            BuiltinFunction::ThreadingActiveCount => {
                self.builtin_threading_active_count(args, kwargs)
            }
            BuiltinFunction::SignalSignal => self.builtin_signal_signal(args, kwargs),
            BuiltinFunction::SignalGetSignal => self.builtin_signal_getsignal(args, kwargs),
            BuiltinFunction::SignalRaiseSignal => self.builtin_signal_raise_signal(args, kwargs),
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

    fn builtin_iter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("iter() expects one argument"));
        }
        let source = args.remove(0);
        self.ensure_sync_iterator_target(&source)?;
        self.to_iterator_value(source)
            .map_err(|_| RuntimeError::new("object is not iterable"))
    }

    fn builtin_next(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("next() expects 1-2 arguments"));
        }
        let default = if args.len() == 2 {
            Some(args.pop().expect("checked len"))
        } else {
            None
        };
        let target = args.pop().expect("checked len");
        self.ensure_sync_iterator_target(&target)?;
        let iterator = self
            .to_iterator_value(target)
            .map_err(|_| RuntimeError::new("next() argument is not iterable"))?;
        match iterator {
            Value::Generator(obj) => match self.generator_for_iter_next(&obj)? {
                GeneratorResumeOutcome::Yield(value) => Ok(value),
                GeneratorResumeOutcome::Complete(_) => {
                    if let Some(default) = default {
                        Ok(default)
                    } else {
                        Err(RuntimeError::new("StopIteration"))
                    }
                }
                GeneratorResumeOutcome::PropagatedException => {
                    self.propagate_pending_generator_exception()?;
                    Err(RuntimeError::new("StopIteration"))
                }
            },
            Value::Iterator(iterator_ref) => {
                if let Some(value) = self.iterator_next_value(&iterator_ref) {
                    Ok(value)
                } else if let Some(default) = default {
                    Ok(default)
                } else {
                    Err(RuntimeError::new("StopIteration"))
                }
            }
            _ => Err(RuntimeError::new("next() argument is not iterable")),
        }
    }

    fn builtin_aiter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("aiter() expects one argument"));
        }
        let source = args.remove(0);
        let source_is_async_generator = if let Value::Generator(generator) = &source {
            matches!(&*generator.kind(), Object::Generator(state) if state.is_async_generator)
        } else {
            false
        };
        if source_is_async_generator {
            return Ok(source);
        }
        let method = self
            .lookup_bound_special_method(&source, "__aiter__")?
            .ok_or_else(|| RuntimeError::new("object is not async iterable"))?;
        match self.call_internal(method, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(RuntimeError::new("aiter() failed"))
            }
        }
    }

    fn builtin_anext(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("anext() expects 1-2 arguments"));
        }
        let default = if args.len() == 2 {
            Some(args.pop().expect("checked len"))
        } else {
            None
        };
        let target = args.pop().expect("checked len");

        let target_is_async_generator = if let Value::Generator(generator) = &target {
            matches!(&*generator.kind(), Object::Generator(state) if state.is_async_generator)
        } else {
            false
        };

        if target_is_async_generator {
            let generator = match &target {
                Value::Generator(generator) => generator,
                _ => unreachable!(),
            };
            return match self.resume_generator(generator, None, None, GeneratorResumeKind::Next)? {
                GeneratorResumeOutcome::Yield(value) => Ok(self.make_immediate_coroutine(value)),
                GeneratorResumeOutcome::Complete(_) => {
                    if let Some(default) = default {
                        Ok(self.make_immediate_coroutine(default))
                    } else {
                        Err(RuntimeError::new("StopAsyncIteration"))
                    }
                }
                GeneratorResumeOutcome::PropagatedException => {
                    self.propagate_pending_generator_exception()?;
                    Err(RuntimeError::new("StopAsyncIteration"))
                }
            }
        }

        let method = self
            .lookup_bound_special_method(&target, "__anext__")?
            .ok_or_else(|| RuntimeError::new("object is not an async iterator"))?;
        match self.call_internal(method, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                if default.is_some() && self.active_exception_is("StopAsyncIteration") {
                    self.clear_active_exception();
                    Ok(self.make_immediate_coroutine(default.expect("checked is_some")))
                } else {
                    Err(RuntimeError::new("anext() failed"))
                }
            }
        }
    }

    fn builtin_getattr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "getattr() got an unexpected keyword argument",
            ));
        }
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new("getattr() expects 2-3 arguments"));
        }

        let target = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("attribute name must be string")),
        };
        let default = args.into_iter().next();

        let looked_up = match target {
            Value::Module(module) => self.load_attr_module(&module, &name),
            Value::Class(class) => match self.load_attr_class(&class, &name)? {
                AttrAccessOutcome::Value(value) => Ok(value),
                AttrAccessOutcome::ExceptionHandled => Ok(Value::None),
            },
            Value::Instance(instance) => match self.load_attr_instance(&instance, &name)? {
                AttrAccessOutcome::Value(value) => Ok(value),
                AttrAccessOutcome::ExceptionHandled => Ok(Value::None),
            },
            Value::Super(super_obj) => match self.load_attr_super(&super_obj, &name)? {
                AttrAccessOutcome::Value(value) => Ok(value),
                AttrAccessOutcome::ExceptionHandled => Ok(Value::None),
            },
            Value::Function(func) => {
                if name != "__annotations__" {
                    Err(RuntimeError::new(format!(
                        "function has no attribute '{}'",
                        name
                    )))
                } else {
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
                                return Err(RuntimeError::new("attribute access unsupported type"));
                            }
                        }
                    };
                    Ok(Value::Dict(annotations))
                }
            }
            Value::Generator(generator) => {
                let kind = match &*generator.kind() {
                    Object::Generator(state) if state.is_async_generator => match name.as_str() {
                        "__aiter__" => NativeMethodKind::GeneratorIter,
                        "__anext__" => NativeMethodKind::GeneratorANext,
                        "asend" => NativeMethodKind::GeneratorANext,
                        "athrow" => NativeMethodKind::GeneratorThrow,
                        "aclose" => NativeMethodKind::GeneratorClose,
                        "throw" => NativeMethodKind::GeneratorThrow,
                        "close" => NativeMethodKind::GeneratorClose,
                        _ => {
                            return Err(RuntimeError::new(format!(
                                "async_generator has no attribute '{}'",
                                name
                            )));
                        }
                    },
                    Object::Generator(state) if state.is_coroutine => match name.as_str() {
                        "__await__" => NativeMethodKind::GeneratorAwait,
                        "send" => NativeMethodKind::GeneratorSend,
                        "throw" => NativeMethodKind::GeneratorThrow,
                        "close" => NativeMethodKind::GeneratorClose,
                        _ => {
                            return Err(RuntimeError::new(format!(
                                "coroutine has no attribute '{}'",
                                name
                            )));
                        }
                    },
                    Object::Generator(_) => match name.as_str() {
                        "__iter__" => NativeMethodKind::GeneratorIter,
                        "__next__" => NativeMethodKind::GeneratorNext,
                        "send" => NativeMethodKind::GeneratorSend,
                        "throw" => NativeMethodKind::GeneratorThrow,
                        "close" => NativeMethodKind::GeneratorClose,
                        _ => {
                            return Err(RuntimeError::new(format!(
                                "generator has no attribute '{}'",
                                name
                            )));
                        }
                    },
                    _ => return Err(RuntimeError::new("attribute access unsupported type")),
                };
                let native = self.heap.alloc_native_method(NativeMethodObject::new(kind));
                let bound = BoundMethod::new(native, generator);
                Ok(self.heap.alloc_bound_method(bound))
            }
            Value::Exception(exception) => match name.as_str() {
                "__cause__" => Ok(exception
                    .cause
                    .as_ref()
                    .map(|cause| Value::Exception((**cause).clone()))
                    .unwrap_or(Value::None)),
                "__context__" => Ok(exception
                    .context
                    .as_ref()
                    .map(|context| Value::Exception((**context).clone()))
                    .unwrap_or(Value::None)),
                "__suppress_context__" => Ok(Value::Bool(exception.suppress_context)),
                _ => Err(RuntimeError::new(format!(
                    "exception has no attribute '{}'",
                    name
                ))),
            },
            _ => Err(RuntimeError::new("attribute access unsupported type")),
        };

        match looked_up {
            Ok(value) => Ok(value),
            Err(err) => {
                if let Some(default) = default {
                    if err.message.contains("has no attribute") {
                        Ok(default)
                    } else {
                        Err(err)
                    }
                } else {
                    Err(err)
                }
            }
        }
    }

    fn builtin_setattr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "setattr() got an unexpected keyword argument",
            ));
        }
        if args.len() != 3 {
            return Err(RuntimeError::new("setattr() expects three arguments"));
        }

        let target = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("attribute name must be string")),
        };
        let value = args.remove(0);

        match target {
            Value::Module(module) => {
                if let Object::Module(module_data) = &mut *module.kind_mut() {
                    module_data.globals.insert(name, value);
                }
            }
            Value::Instance(instance) => match self.store_attr_instance(&instance, &name, value)? {
                AttrMutationOutcome::Done => {}
                AttrMutationOutcome::ExceptionHandled => return Ok(Value::None),
            },
            Value::Class(class) => {
                if let Object::Class(class_data) = &mut *class.kind_mut() {
                    class_data.attrs.insert(name, value);
                }
            }
            Value::Function(func) => {
                if name != "__annotations__" {
                    return Err(RuntimeError::new("attribute assignment unsupported type"));
                }
                let annotations = match value {
                    Value::Dict(obj) => obj,
                    _ => return Err(RuntimeError::new("function __annotations__ must be dict")),
                };
                if let Object::Function(func_data) = &mut *func.kind_mut() {
                    func_data.annotations = Some(annotations);
                }
            }
            _ => return Err(RuntimeError::new("attribute assignment unsupported type")),
        }

        Ok(Value::None)
    }

    fn builtin_delattr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "delattr() got an unexpected keyword argument",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::new("delattr() expects two arguments"));
        }

        let target = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("attribute name must be string")),
        };

        match target {
            Value::Module(module) => {
                if let Object::Module(module_data) = &mut *module.kind_mut() {
                    if module_data.globals.remove(&name).is_none() {
                        return Err(RuntimeError::new(format!(
                            "module attribute '{}' does not exist",
                            name
                        )));
                    }
                }
            }
            Value::Class(class) => {
                if let Object::Class(class_data) = &mut *class.kind_mut() {
                    if class_data.attrs.remove(&name).is_none() {
                        return Err(RuntimeError::new(format!(
                            "class attribute '{}' does not exist",
                            name
                        )));
                    }
                }
            }
            Value::Instance(instance) => match self.delete_attr_instance(&instance, &name)? {
                AttrMutationOutcome::Done => {}
                AttrMutationOutcome::ExceptionHandled => return Ok(Value::None),
            },
            _ => return Err(RuntimeError::new("attribute deletion unsupported type")),
        }

        Ok(Value::None)
    }

    fn builtin_hasattr(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 2 {
            return Err(RuntimeError::new("hasattr() expects two arguments"));
        }
        match self.builtin_getattr(args, kwargs) {
            Ok(_) => Ok(Value::Bool(true)),
            Err(err) if err.message.contains("has no attribute") => Ok(Value::Bool(false)),
            Err(err) => Err(err),
        }
    }

    fn builtin_super(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "super() got an unexpected keyword argument",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::new(
                "super() currently requires explicit type and object arguments",
            ));
        }

        let start_class = match args.remove(0) {
            Value::Class(class) => class,
            _ => return Err(RuntimeError::new("super() first argument must be a class")),
        };
        let object_value = args.remove(0);
        let object_ref = self.receiver_from_value(&object_value)?;
        let object_type = self.class_of_value(&object_value).ok_or_else(|| {
            RuntimeError::new("super() second argument must be an instance or subclass")
        })?;

        let mro = self.class_mro_entries(&object_type);
        if !mro.iter().any(|entry| entry.id() == start_class.id()) {
            return Err(RuntimeError::new(
                "super(type, obj): obj must be an instance or subtype of type",
            ));
        }

        Ok(self
            .heap
            .alloc_super(SuperObject::new(start_class, object_ref, object_type)))
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
            return Err(RuntimeError::new(
                "__import__() missing required argument 'name'",
            ));
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

    fn builtin_import_module(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new(
                "import_module() takes at most 2 arguments",
            ));
        }
        let kw_name = kwargs.remove("name");
        let kw_package = kwargs.remove("package");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "import_module() got an unexpected keyword argument",
            ));
        }

        let name_value = if let Some(value) = kw_name {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "import_module() got multiple values for argument 'name'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new(
                "import_module() missing required argument 'name'",
            ));
        };
        let name = match name_value {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("import_module() name must be string")),
        };

        let package_value = if let Some(value) = kw_package {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "import_module() got multiple values for argument 'package'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            Value::None
        };
        let package = match package_value {
            Value::None => None,
            Value::Str(package) => Some(package),
            _ => {
                return Err(RuntimeError::new(
                    "import_module() package must be string or None",
                ));
            }
        };

        let (level, requested) = split_relative_import_name(&name);
        let resolved_name = if level == 0 {
            name
        } else {
            let package = package.ok_or_else(|| {
                RuntimeError::new("import_module() relative import requires package argument")
            })?;
            self.resolve_import_name_from_package(&package, &requested, level)?
        };
        let module = self.import_module_object(&resolved_name)?;
        Ok(Value::Module(module))
    }

    fn builtin_find_spec(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new("find_spec() takes at most 2 arguments"));
        }
        let kw_name = kwargs.remove("name");
        let kw_package = kwargs.remove("package");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "find_spec() got an unexpected keyword argument",
            ));
        }

        let name_value = if let Some(value) = kw_name {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "find_spec() got multiple values for argument 'name'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new(
                "find_spec() missing required argument 'name'",
            ));
        };
        let name = match name_value {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("find_spec() name must be string")),
        };

        let package_value = if let Some(value) = kw_package {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "find_spec() got multiple values for argument 'package'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            Value::None
        };
        let package = match package_value {
            Value::None => None,
            Value::Str(package) => Some(package),
            _ => {
                return Err(RuntimeError::new(
                    "find_spec() package must be string or None",
                ));
            }
        };

        let (level, requested) = split_relative_import_name(&name);
        let resolved_name = if level == 0 {
            name
        } else {
            let package = package.ok_or_else(|| {
                RuntimeError::new("find_spec() relative import requires package argument")
            })?;
            self.resolve_import_name_from_package(&package, &requested, level)?
        };

        if let Some(existing) = self.modules.get(&resolved_name).cloned() {
            if let Object::Module(module_data) = &*existing.kind() {
                if let Some(spec) = module_data.globals.get("__spec__").cloned() {
                    return Ok(spec);
                }
            }
        }

        let Some(source_info) = self.find_module_source(&resolved_name) else {
            return Ok(Value::None);
        };
        let loader_name = if source_info.is_namespace {
            NAMESPACE_LOADER
        } else {
            SOURCE_FILE_LOADER
        };
        let origin = if source_info.is_namespace {
            None
        } else {
            Some(&source_info.path)
        };
        Ok(self.build_module_spec_value(
            &resolved_name,
            origin,
            Some(loader_name),
            source_info.is_package,
            source_info.package_dirs.as_slice(),
            source_info.is_namespace,
        ))
    }

    fn builtin_random_seed(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new("seed() takes at most 1 argument"));
        }
        let kw_value = kwargs.remove("a");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "seed() got an unexpected keyword argument",
            ));
        }
        if kw_value.is_some() && !args.is_empty() {
            return Err(RuntimeError::new(
                "seed() got multiple values for argument 'a'",
            ));
        }
        let value = kw_value.or_else(|| args.pop()).unwrap_or(Value::None);
        let seed = seed_from_value(&value)?;
        self.random.seed(seed);
        Ok(Value::None)
    }

    fn builtin_random_random(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new("random() takes no arguments"));
        }
        Ok(Value::Float(self.random.random_f64()))
    }

    fn builtin_random_randrange(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 3 {
            return Err(RuntimeError::new(
                "randrange() expected at most 3 arguments",
            ));
        }
        let mut start_kw = kwargs.remove("start");
        let mut stop_kw = kwargs.remove("stop");
        let mut step_kw = kwargs.remove("step");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "randrange() got an unexpected keyword argument",
            ));
        }

        match args.len() {
            0 => {}
            1 => {
                if stop_kw.is_some() {
                    return Err(RuntimeError::new("randrange() got multiple values"));
                }
                stop_kw = Some(args.remove(0));
            }
            2 => {
                if start_kw.is_some() || stop_kw.is_some() {
                    return Err(RuntimeError::new("randrange() got multiple values"));
                }
                start_kw = Some(args.remove(0));
                stop_kw = Some(args.remove(0));
            }
            3 => {
                if start_kw.is_some() || stop_kw.is_some() || step_kw.is_some() {
                    return Err(RuntimeError::new("randrange() got multiple values"));
                }
                start_kw = Some(args.remove(0));
                stop_kw = Some(args.remove(0));
                step_kw = Some(args.remove(0));
            }
            _ => unreachable!(),
        }

        let stop = stop_kw.ok_or_else(|| RuntimeError::new("randrange() missing stop"))?;
        let start = start_kw.unwrap_or(Value::Int(0));
        let step = step_kw.unwrap_or(Value::Int(1));

        let start = value_to_int(start)?;
        let stop = value_to_int(stop)?;
        let step = value_to_int(step)?;
        if step == 0 {
            return Err(RuntimeError::new(
                "randrange() step argument must not be zero",
            ));
        }

        let count = random_range_count(start, stop, step)?;
        let offset = self.random_randbelow(count)?;
        let result = (start as i128)
            .checked_add((step as i128) * (offset as i128))
            .ok_or_else(|| RuntimeError::new("integer overflow"))?;
        let result = i64::try_from(result).map_err(|_| RuntimeError::new("integer overflow"))?;
        Ok(Value::Int(result))
    }

    fn builtin_random_randint(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new("randint() expected 2 arguments"));
        }
        let a_kw = kwargs.remove("a");
        let b_kw = kwargs.remove("b");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "randint() got an unexpected keyword argument",
            ));
        }

        let a_value = if let Some(value) = a_kw {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "randint() got multiple values for argument 'a'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new("randint() missing argument 'a'"));
        };
        let b_value = if let Some(value) = b_kw {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "randint() got multiple values for argument 'b'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new("randint() missing argument 'b'"));
        };
        if !args.is_empty() {
            return Err(RuntimeError::new("randint() expected 2 arguments"));
        }

        let a = value_to_int(a_value)?;
        let b = value_to_int(b_value)?;
        let upper = b
            .checked_add(1)
            .ok_or_else(|| RuntimeError::new("empty range for randint()"))?;
        let count = random_range_count(a, upper, 1)?;
        let offset = self.random_randbelow(count)?;
        let result = a
            .checked_add(offset)
            .ok_or_else(|| RuntimeError::new("integer overflow"))?;
        Ok(Value::Int(result))
    }

    fn builtin_random_getrandbits(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new(
                "getrandbits() takes exactly one argument",
            ));
        }
        let kw_k = kwargs.remove("k");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "getrandbits() got an unexpected keyword argument",
            ));
        }
        let k_value = if let Some(value) = kw_k {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "getrandbits() got multiple values for 'k'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new("getrandbits() missing argument 'k'"));
        };

        let bits = value_to_int(k_value)?;
        if bits < 0 {
            return Err(RuntimeError::new("number of bits must be non-negative"));
        }
        if bits == 0 {
            return Ok(Value::Int(0));
        }
        if bits > 63 {
            return Err(RuntimeError::new(
                "getrandbits() supports up to 63 bits in this runtime",
            ));
        }

        let mut produced = 0u64;
        let mut consumed = 0i64;
        while consumed < bits {
            let chunk = self.random.next_u32() as u64;
            let take = std::cmp::min(32, (bits - consumed) as usize);
            let mask = if take == 32 {
                u64::MAX
            } else {
                (1u64 << take) - 1
            };
            produced |= (chunk & mask) << consumed;
            consumed += take as i64;
        }
        Ok(Value::Int(produced as i64))
    }

    fn builtin_random_choice(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("choice() expects one argument"));
        }
        match &args[0] {
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => {
                    if values.is_empty() {
                        return Err(RuntimeError::new("Cannot choose from an empty sequence"));
                    }
                    let idx = self.random_randbelow(values.len() as i64)? as usize;
                    Ok(values[idx].clone())
                }
                _ => Err(RuntimeError::new("choice() expects a sequence")),
            },
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => {
                    if values.is_empty() {
                        return Err(RuntimeError::new("Cannot choose from an empty sequence"));
                    }
                    let idx = self.random_randbelow(values.len() as i64)? as usize;
                    Ok(values[idx].clone())
                }
                _ => Err(RuntimeError::new("choice() expects a sequence")),
            },
            Value::Str(value) => {
                let chars: Vec<char> = value.chars().collect();
                if chars.is_empty() {
                    return Err(RuntimeError::new("Cannot choose from an empty sequence"));
                }
                let idx = self.random_randbelow(chars.len() as i64)? as usize;
                Ok(Value::Str(chars[idx].to_string()))
            }
            _ => Err(RuntimeError::new("choice() expects a sequence")),
        }
    }

    fn builtin_random_shuffle(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("shuffle() expects one argument"));
        }
        match &args[0] {
            Value::List(obj) => {
                let len = match &*obj.kind() {
                    Object::List(values) => values.len(),
                    _ => return Err(RuntimeError::new("shuffle() expects list")),
                };
                if len <= 1 {
                    return Ok(Value::None);
                }
                for idx in (1..len).rev() {
                    let swap = self.random_randbelow((idx + 1) as i64)? as usize;
                    if let Object::List(values) = &mut *obj.kind_mut() {
                        values.swap(idx, swap);
                    }
                }
                Ok(Value::None)
            }
            _ => Err(RuntimeError::new("shuffle() expects list")),
        }
    }

    fn builtin_math_sqrt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("sqrt() expects one argument"));
        }
        let value = value_to_f64(args[0].clone())?;
        if value < 0.0 {
            return Err(RuntimeError::new("math domain error"));
        }
        Ok(Value::Float(value.sqrt()))
    }

    fn builtin_math_floor(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("floor() expects one argument"));
        }
        let value = value_to_f64(args[0].clone())?.floor();
        if value < i64::MIN as f64 || value > i64::MAX as f64 {
            return Err(RuntimeError::new("integer overflow"));
        }
        Ok(Value::Int(value as i64))
    }

    fn builtin_math_ceil(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("ceil() expects one argument"));
        }
        let value = value_to_f64(args[0].clone())?.ceil();
        if value < i64::MIN as f64 || value > i64::MAX as f64 {
            return Err(RuntimeError::new("integer overflow"));
        }
        Ok(Value::Int(value as i64))
    }

    fn builtin_math_isfinite(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isfinite() expects one argument"));
        }
        Ok(Value::Bool(value_to_f64(args[0].clone())?.is_finite()))
    }

    fn builtin_math_isinf(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isinf() expects one argument"));
        }
        Ok(Value::Bool(value_to_f64(args[0].clone())?.is_infinite()))
    }

    fn builtin_math_isnan(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isnan() expects one argument"));
        }
        Ok(Value::Bool(value_to_f64(args[0].clone())?.is_nan()))
    }

    fn builtin_time_time(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("time() expects no arguments"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuntimeError::new("system time before epoch"))?;
        Ok(Value::Float(now.as_secs_f64()))
    }

    fn builtin_time_monotonic(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("monotonic() expects no arguments"));
        }
        let start = MONOTONIC_START.get_or_init(Instant::now);
        Ok(Value::Float(start.elapsed().as_secs_f64()))
    }

    fn builtin_time_sleep(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("sleep() expects one argument"));
        }
        let seconds = value_to_f64(args[0].clone())?;
        if seconds < 0.0 {
            return Err(RuntimeError::new("sleep length must be non-negative"));
        }
        std::thread::sleep(Duration::from_secs_f64(seconds));
        Ok(Value::None)
    }

    fn builtin_os_getcwd(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("getcwd() expects no arguments"));
        }
        let cwd = std::env::current_dir()
            .map_err(|err| RuntimeError::new(format!("getcwd failed: {err}")))?;
        Ok(Value::Str(cwd.to_string_lossy().to_string()))
    }

    fn builtin_os_listdir(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("listdir() expects at most one argument"));
        }
        let path = if args.is_empty() {
            ".".to_string()
        } else {
            value_to_path(&args[0])?
        };
        let mut names = Vec::new();
        let entries =
            fs::read_dir(&path).map_err(|err| RuntimeError::new(format!("listdir failed: {err}")))?;
        for entry in entries {
            let entry = entry.map_err(|err| RuntimeError::new(format!("listdir failed: {err}")))?;
            let name = entry.file_name().to_string_lossy().to_string();
            names.push(Value::Str(name));
        }
        names.sort_by(|a, b| format_value(a).cmp(&format_value(b)));
        Ok(self.heap.alloc_list(names))
    }

    fn builtin_os_path_exists(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("path_exists() expects one argument"));
        }
        let path = value_to_path(&args[0])?;
        Ok(Value::Bool(PathBuf::from(path).exists()))
    }

    fn builtin_os_path_join(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("path_join() does not accept keyword arguments"));
        }
        if args.is_empty() {
            return Ok(Value::Str(".".to_string()));
        }
        let mut out = PathBuf::from(value_to_path(&args[0])?);
        for part in args.iter().skip(1) {
            out.push(value_to_path(part)?);
        }
        Ok(Value::Str(out.to_string_lossy().to_string()))
    }

    fn builtin_json_dumps(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("dumps() expects one argument"));
        }
        let text = json_serialize_value(&args[0])?;
        Ok(Value::Str(text))
    }

    fn builtin_json_loads(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("loads() expects one argument"));
        }
        let text = match &args[0] {
            Value::Str(text) => text.clone(),
            _ => return Err(RuntimeError::new("loads() expects a string")),
        };
        let node = parse_json_node(&text)?;
        Ok(json_node_to_value(node, &self.heap))
    }

    fn builtin_codecs_encode(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new(
                "encode() expects object, optional encoding, optional errors",
            ));
        }
        let mut encoding = if args.len() >= 2 {
            Some(args.remove(1))
        } else {
            None
        };
        let mut errors = if args.len() >= 2 {
            Some(args.remove(1))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("encoding") {
            if encoding.is_some() {
                return Err(RuntimeError::new("encode() got multiple values for encoding"));
            }
            encoding = Some(value);
        }
        if let Some(value) = kwargs.remove("errors") {
            if errors.is_some() {
                return Err(RuntimeError::new("encode() got multiple values for errors"));
            }
            errors = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "encode() got an unexpected keyword argument",
            ));
        }
        let source = args.remove(0);
        let text = match source {
            Value::Str(text) => text,
            _ => return Err(RuntimeError::new("encode() argument must be str")),
        };
        let encoding = normalize_codec_encoding(encoding.unwrap_or(Value::Str("utf-8".to_string())))?;
        let errors = normalize_codec_errors(errors.unwrap_or(Value::Str("strict".to_string())))?;
        let encoded = encode_text_bytes(&text, &encoding, &errors)?;
        Ok(self.heap.alloc_bytes(encoded))
    }

    fn builtin_codecs_decode(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new(
                "decode() expects object, optional encoding, optional errors",
            ));
        }
        let mut encoding = if args.len() >= 2 {
            Some(args.remove(1))
        } else {
            None
        };
        let mut errors = if args.len() >= 2 {
            Some(args.remove(1))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("encoding") {
            if encoding.is_some() {
                return Err(RuntimeError::new("decode() got multiple values for encoding"));
            }
            encoding = Some(value);
        }
        if let Some(value) = kwargs.remove("errors") {
            if errors.is_some() {
                return Err(RuntimeError::new("decode() got multiple values for errors"));
            }
            errors = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "decode() got an unexpected keyword argument",
            ));
        }
        let source = args.remove(0);
        let bytes = bytes_like_from_value(source)?;
        let encoding = normalize_codec_encoding(encoding.unwrap_or(Value::Str("utf-8".to_string())))?;
        let errors = normalize_codec_errors(errors.unwrap_or(Value::Str("strict".to_string())))?;
        let decoded = decode_text_bytes(&bytes, &encoding, &errors)?;
        Ok(Value::Str(decoded))
    }

    fn builtin_re_search(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_re_match_mode(args, kwargs, ReMode::Search)
    }

    fn builtin_re_match(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_re_match_mode(args, kwargs, ReMode::Match)
    }

    fn builtin_re_fullmatch(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_re_match_mode(args, kwargs, ReMode::FullMatch)
    }

    fn builtin_re_match_mode(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        mode: ReMode,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 {
            return Err(RuntimeError::new("re function expects pattern and string"));
        }
        let pattern = match &args[0] {
            Value::Str(value) => value.clone(),
            _ => return Err(RuntimeError::new("pattern must be string")),
        };
        let text = match &args[1] {
            Value::Str(value) => value.clone(),
            _ => return Err(RuntimeError::new("string must be string")),
        };
        let found = match mode {
            ReMode::Search => text.find(&pattern),
            ReMode::Match => text.starts_with(&pattern).then_some(0),
            ReMode::FullMatch => (text == pattern).then_some(0),
        };
        if let Some(start) = found {
            let end = start + pattern.len();
            Ok(self
                .heap
                .alloc_tuple(vec![Value::Int(start as i64), Value::Int(end as i64)]))
        } else {
            Ok(Value::None)
        }
    }

    fn builtin_operator_add(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, |left, right| add_values(left, right, &self.heap))
    }

    fn builtin_operator_sub(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, sub_values)
    }

    fn builtin_operator_mul(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, |left, right| mul_values(left, right, &self.heap))
    }

    fn builtin_operator_truediv(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, div_values)
    }

    fn builtin_operator_eq(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("operator.eq expects two arguments"));
        }
        Ok(Value::Bool(args[0] == args[1]))
    }

    fn builtin_operator_contains(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("operator.contains expects two arguments"));
        }
        Ok(Value::Bool(compare_in(&args[1], &args[0])?))
    }

    fn builtin_operator_getitem(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("operator.getitem expects two arguments"));
        }
        self.getitem_value(args[0].clone(), args[1].clone())
    }

    fn builtin_itertools_chain(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("chain() does not accept keyword arguments"));
        }
        let mut out = Vec::new();
        for source in args {
            out.extend(self.collect_iterable_values(source)?);
        }
        Ok(self.heap.alloc_list(out))
    }

    fn builtin_itertools_repeat(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("repeat() expects value and count"));
        }
        let count = value_to_int(args[1].clone())?;
        if count < 0 {
            return Err(RuntimeError::new("repeat count must be >= 0"));
        }
        let mut out = Vec::with_capacity(count as usize);
        for _ in 0..count {
            out.push(args[0].clone());
        }
        Ok(self.heap.alloc_list(out))
    }

    fn builtin_functools_reduce(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new("reduce() expects 2-3 arguments"));
        }
        let callable = args[0].clone();
        let values = self.collect_iterable_values(args[1].clone())?;
        let mut iter = values.into_iter();
        let mut accumulator = if args.len() == 3 {
            args[2].clone()
        } else {
            iter.next()
                .ok_or_else(|| RuntimeError::new("reduce() of empty iterable with no initial value"))?
        };

        for item in iter {
            match self.call_internal(
                callable.clone(),
                vec![accumulator.clone(), item],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => accumulator = value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("reduce() callback raised"));
                }
            }
        }
        Ok(accumulator)
    }

    fn builtin_collections_counter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("Counter() expects at most one argument"));
        }
        let mut entries: Vec<(Value, Value)> = Vec::new();
        if let Some(source) = args.into_iter().next() {
            for item in self.collect_iterable_values(source)? {
                if let Some((_, count)) = entries.iter_mut().find(|(key, _)| *key == item) {
                    *count = add_values(count.clone(), Value::Int(1), &self.heap)?;
                } else {
                    entries.push((item, Value::Int(1)));
                }
            }
        }
        Ok(self.heap.alloc_dict(entries))
    }

    fn builtin_collections_deque(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("deque() expects at most one argument"));
        }
        if let Some(source) = args.into_iter().next() {
            let values = self.collect_iterable_values(source)?;
            Ok(self.heap.alloc_list(values))
        } else {
            Ok(self.heap.alloc_list(Vec::new()))
        }
    }

    fn builtin_inspect_isfunction(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| {
            matches!(value, Value::Function(_) | Value::BoundMethod(_))
        })
    }

    fn builtin_inspect_isclass(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| matches!(value, Value::Class(_)))
    }

    fn builtin_inspect_ismodule(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| matches!(value, Value::Module(_)))
    }

    fn builtin_inspect_isgenerator(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| {
            if let Value::Generator(generator) = value {
                if let Object::Generator(state) = &*generator.kind() {
                    return !state.is_coroutine && !state.is_async_generator;
                }
            }
            false
        })
    }

    fn builtin_inspect_iscoroutine(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| {
            if let Value::Generator(generator) = value {
                if let Object::Generator(state) = &*generator.kind() {
                    return state.is_coroutine;
                }
            }
            false
        })
    }

    fn builtin_inspect_isawaitable(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("predicate expects one argument"));
        }
        Ok(Value::Bool(self.is_awaitable_value(&args[0])))
    }

    fn builtin_inspect_isasyncgen(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| {
            if let Value::Generator(generator) = value {
                if let Object::Generator(state) = &*generator.kind() {
                    return state.is_async_generator;
                }
            }
            false
        })
    }

    fn builtin_types_moduletype(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("ModuleType() expects one argument"));
        }
        let name = match &args[0] {
            Value::Str(name) => name.clone(),
            _ => return Err(RuntimeError::new("module name must be string")),
        };
        Ok(self.alloc_module(name))
    }

    fn builtin_io_open(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new("open() expects path, optional mode, optional payload"));
        }
        let path = value_to_path(&args[0])?;
        let mode = if args.len() >= 2 {
            match &args[1] {
                Value::Str(mode) => mode.clone(),
                _ => return Err(RuntimeError::new("mode must be string")),
            }
        } else {
            "r".to_string()
        };

        if mode.starts_with('r') {
            if mode.contains('b') {
                let bytes = fs::read(&path)
                    .map_err(|err| RuntimeError::new(format!("open() read failed: {err}")))?;
                return Ok(self.heap.alloc_bytes(bytes));
            }
            let text = fs::read_to_string(&path)
                .map_err(|err| RuntimeError::new(format!("open() read failed: {err}")))?;
            return Ok(Value::Str(text));
        }

        if !(mode.starts_with('w') || mode.starts_with('a')) {
            return Err(RuntimeError::new("unsupported open mode"));
        }
        if args.len() < 3 {
            return Err(RuntimeError::new("write mode requires payload argument"));
        }

        if mode.contains('b') {
            let payload = value_to_bytes_payload(args[2].clone())?;
            if mode.starts_with('a') {
                use std::io::Write;
                let mut file = fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&path)
                    .map_err(|err| RuntimeError::new(format!("open() write failed: {err}")))?;
                file.write_all(&payload)
                    .map_err(|err| RuntimeError::new(format!("open() write failed: {err}")))?;
            } else {
                fs::write(&path, payload)
                    .map_err(|err| RuntimeError::new(format!("open() write failed: {err}")))?;
            }
            return Ok(Value::None);
        }

        let payload = match &args[2] {
            Value::Str(text) => text.clone(),
            other => format_value(other),
        };
        if mode.starts_with('a') {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&path)
                .map_err(|err| RuntimeError::new(format!("open() write failed: {err}")))?;
            file.write_all(payload.as_bytes())
                .map_err(|err| RuntimeError::new(format!("open() write failed: {err}")))?;
        } else {
            fs::write(&path, payload)
                .map_err(|err| RuntimeError::new(format!("open() write failed: {err}")))?;
        }
        Ok(Value::None)
    }

    fn builtin_io_read_text(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("read_text() expects one argument"));
        }
        let path = value_to_path(&args[0])?;
        let text = fs::read_to_string(path)
            .map_err(|err| RuntimeError::new(format!("read_text failed: {err}")))?;
        Ok(Value::Str(text))
    }

    fn builtin_io_write_text(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("write_text() expects path and text"));
        }
        let path = value_to_path(&args[0])?;
        let text = match &args[1] {
            Value::Str(text) => text.clone(),
            other => format_value(other),
        };
        fs::write(path, text).map_err(|err| RuntimeError::new(format!("write_text failed: {err}")))?;
        Ok(Value::None)
    }

    fn builtin_datetime_now(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("now() expects no arguments"));
        }
        Ok(Value::Str(current_utc_iso()))
    }

    fn builtin_datetime_today(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("today() expects no arguments"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuntimeError::new("system time before epoch"))?;
        let days = (now.as_secs() / 86_400) as i64;
        let (year, month, day) = civil_from_days(days);
        Ok(Value::Str(format!("{year:04}-{month:02}-{day:02}")))
    }

    fn builtin_asyncio_run(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("run() expects one awaitable argument"));
        }
        let awaitable = args.remove(0);
        self.run_awaitable(awaitable)
    }

    fn builtin_asyncio_sleep(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("sleep() expects delay and optional result"));
        }
        let seconds = value_to_f64(args.remove(0))?;
        if seconds < 0.0 {
            return Err(RuntimeError::new("sleep length must be non-negative"));
        }
        let result = if args.is_empty() {
            Value::None
        } else {
            args.remove(0)
        };
        Ok(self.make_immediate_coroutine(result))
    }

    fn builtin_asyncio_create_task(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("create_task() expects one awaitable argument"));
        }
        self.awaitable_from_value(args.remove(0))
    }

    fn builtin_asyncio_gather(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "gather() keyword arguments are not supported",
            ));
        }
        let mut results = Vec::with_capacity(args.len());
        for awaitable in args {
            results.push(self.run_awaitable(awaitable)?);
        }
        Ok(self.make_immediate_coroutine(self.heap.alloc_list(results)))
    }

    fn builtin_threading_get_ident(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("get_ident() expects no arguments"));
        }
        let mut hasher = DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        Ok(Value::Int((hasher.finish() & i64::MAX as u64) as i64))
    }

    fn builtin_threading_current_thread(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("current_thread() expects no arguments"));
        }
        self.thread_info_dict("MainThread")
    }

    fn builtin_threading_main_thread(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("main_thread() expects no arguments"));
        }
        self.thread_info_dict("MainThread")
    }

    fn builtin_threading_active_count(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("active_count() expects no arguments"));
        }
        Ok(Value::Int(1))
    }

    fn builtin_signal_signal(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("signal() expects signum and handler"));
        }
        let signum = value_to_int(args.remove(0))?;
        let handler = args.remove(0);
        let previous = self
            .signal_handlers
            .insert(signum, handler)
            .unwrap_or(Value::Int(SIGNAL_DEFAULT));
        Ok(previous)
    }

    fn builtin_signal_getsignal(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("getsignal() expects one signum argument"));
        }
        let signum = value_to_int(args.remove(0))?;
        Ok(self
            .signal_handlers
            .get(&signum)
            .cloned()
            .unwrap_or(Value::Int(SIGNAL_DEFAULT)))
    }

    fn builtin_signal_raise_signal(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("raise_signal() expects one signum argument"));
        }
        let signum = value_to_int(args.remove(0))?;
        let handler = self
            .signal_handlers
            .get(&signum)
            .cloned()
            .unwrap_or(Value::Int(SIGNAL_DEFAULT));
        match handler {
            Value::Int(code) if code == SIGNAL_IGNORE => Ok(Value::None),
            Value::Int(code) if code == SIGNAL_DEFAULT => {
                if signum == SIGNAL_SIGINT {
                    Err(RuntimeError::new("KeyboardInterrupt"))
                } else {
                    Ok(Value::None)
                }
            }
            callable => {
                match self.call_internal(
                    callable,
                    vec![Value::Int(signum), Value::None],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(_) => Ok(Value::None),
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(RuntimeError::new("signal handler raised"))
                    }
                }
            }
        }
    }

    fn thread_info_dict(&mut self, name: &str) -> Result<Value, RuntimeError> {
        let ident = self.builtin_threading_get_ident(Vec::new(), HashMap::new())?;
        Ok(self.heap.alloc_dict(vec![
            (Value::Str("name".to_string()), Value::Str(name.to_string())),
            (Value::Str("ident".to_string()), ident),
            (Value::Str("daemon".to_string()), Value::Bool(false)),
        ]))
    }

    fn getitem_value(&mut self, value: Value, index: Value) -> Result<Value, RuntimeError> {
        match index {
            Value::Slice { lower, upper, step } => match value {
                Value::List(obj) => match &*obj.kind() {
                    Object::List(values) => {
                        let indices = slice_indices(values.len(), lower, upper, step)?;
                        let mut result = Vec::with_capacity(indices.len());
                        for idx in indices {
                            result.push(values[idx].clone());
                        }
                        Ok(self.heap.alloc_list(result))
                    }
                    _ => Err(RuntimeError::new("subscript unsupported type")),
                },
                Value::Tuple(obj) => match &*obj.kind() {
                    Object::Tuple(values) => {
                        let indices = slice_indices(values.len(), lower, upper, step)?;
                        let mut result = Vec::with_capacity(indices.len());
                        for idx in indices {
                            result.push(values[idx].clone());
                        }
                        Ok(self.heap.alloc_tuple(result))
                    }
                    _ => Err(RuntimeError::new("subscript unsupported type")),
                },
                Value::Str(value) => {
                    let chars: Vec<char> = value.chars().collect();
                    let indices = slice_indices(chars.len(), lower, upper, step)?;
                    let mut result = String::new();
                    for idx in indices {
                        result.push(chars[idx]);
                    }
                    Ok(Value::Str(result))
                }
                Value::Bytes(obj) => match &*obj.kind() {
                    Object::Bytes(values) => {
                        let indices = slice_indices(values.len(), lower, upper, step)?;
                        let mut result = Vec::with_capacity(indices.len());
                        for idx in indices {
                            result.push(values[idx]);
                        }
                        Ok(self.heap.alloc_bytes(result))
                    }
                    _ => Err(RuntimeError::new("subscript unsupported type")),
                },
                Value::ByteArray(obj) => match &*obj.kind() {
                    Object::ByteArray(values) => {
                        let indices = slice_indices(values.len(), lower, upper, step)?;
                        let mut result = Vec::with_capacity(indices.len());
                        for idx in indices {
                            result.push(values[idx]);
                        }
                        Ok(self.heap.alloc_bytearray(result))
                    }
                    _ => Err(RuntimeError::new("subscript unsupported type")),
                },
                Value::MemoryView(obj) => match &*obj.kind() {
                    Object::MemoryView(view) => match &*view.source.kind() {
                        Object::Bytes(values) | Object::ByteArray(values) => {
                            let indices = slice_indices(values.len(), lower, upper, step)?;
                            let mut result = Vec::with_capacity(indices.len());
                            for idx in indices {
                                result.push(values[idx]);
                            }
                            Ok(self.heap.alloc_bytes(result))
                        }
                        _ => Err(RuntimeError::new("subscript unsupported type")),
                    },
                    _ => Err(RuntimeError::new("subscript unsupported type")),
                },
                Value::Dict(_) => Err(RuntimeError::new("slicing unsupported for dict")),
                _ => Err(RuntimeError::new("subscript unsupported type")),
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
                        Ok(values[index_int as usize].clone())
                    }
                    _ => Err(RuntimeError::new("subscript unsupported type")),
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
                        Ok(values[index_int as usize].clone())
                    }
                    _ => Err(RuntimeError::new("subscript unsupported type")),
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
                    Ok(Value::Str(chars[index_int as usize].to_string()))
                }
                Value::Dict(obj) => match &*obj.kind() {
                    Object::Dict(entries) => entries
                        .iter()
                        .find(|(key, _)| *key == index)
                        .map(|(_, value)| value.clone())
                        .ok_or_else(|| RuntimeError::new("key not found")),
                    _ => Err(RuntimeError::new("subscript unsupported type")),
                },
                Value::Bytes(obj) => match &*obj.kind() {
                    Object::Bytes(values) => {
                        let mut index_int = value_to_int(index)? as isize;
                        if index_int < 0 {
                            index_int += values.len() as isize;
                        }
                        if index_int < 0 || index_int as usize >= values.len() {
                            return Err(RuntimeError::new("index out of range"));
                        }
                        Ok(Value::Int(values[index_int as usize] as i64))
                    }
                    _ => Err(RuntimeError::new("subscript unsupported type")),
                },
                Value::ByteArray(obj) => match &*obj.kind() {
                    Object::ByteArray(values) => {
                        let mut index_int = value_to_int(index)? as isize;
                        if index_int < 0 {
                            index_int += values.len() as isize;
                        }
                        if index_int < 0 || index_int as usize >= values.len() {
                            return Err(RuntimeError::new("index out of range"));
                        }
                        Ok(Value::Int(values[index_int as usize] as i64))
                    }
                    _ => Err(RuntimeError::new("subscript unsupported type")),
                },
                Value::MemoryView(obj) => match &*obj.kind() {
                    Object::MemoryView(view) => match &*view.source.kind() {
                        Object::Bytes(values) | Object::ByteArray(values) => {
                            let mut index_int = value_to_int(index)? as isize;
                            if index_int < 0 {
                                index_int += values.len() as isize;
                            }
                            if index_int < 0 || index_int as usize >= values.len() {
                                return Err(RuntimeError::new("index out of range"));
                            }
                            Ok(Value::Int(values[index_int as usize] as i64))
                        }
                        _ => Err(RuntimeError::new("subscript unsupported type")),
                    },
                    _ => Err(RuntimeError::new("subscript unsupported type")),
                },
                _ => Err(RuntimeError::new("subscript unsupported type")),
            },
        }
    }

    fn collect_iterable_values(&mut self, source: Value) -> Result<Vec<Value>, RuntimeError> {
        let iter = self
            .to_iterator_value(source)
            .map_err(|_| RuntimeError::new("expected iterable"))?;
        match iter {
            Value::Iterator(iterator_ref) => {
                let mut out = Vec::new();
                while let Some(value) = self.iterator_next_value(&iterator_ref) {
                    out.push(value);
                }
                Ok(out)
            }
            Value::Generator(generator) => {
                let mut out = Vec::new();
                loop {
                    match self.generator_for_iter_next(&generator)? {
                        GeneratorResumeOutcome::Yield(value) => out.push(value),
                        GeneratorResumeOutcome::Complete(_) => break,
                        GeneratorResumeOutcome::PropagatedException => {
                            self.propagate_pending_generator_exception()?;
                            return Err(RuntimeError::new("iteration failed"));
                        }
                    }
                }
                Ok(out)
            }
            _ => Err(RuntimeError::new("expected iterable")),
        }
    }

    fn random_randbelow(&mut self, upper: i64) -> Result<i64, RuntimeError> {
        if upper <= 0 {
            return Err(RuntimeError::new("empty range for randrange()"));
        }
        let upper = upper as u64;
        let zone = u64::MAX - (u64::MAX % upper);
        loop {
            let value = ((self.random.next_u32() as u64) << 32) | self.random.next_u32() as u64;
            if value < zone {
                return Ok((value % upper) as i64);
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
            .insert("float".to_string(), Value::Builtin(BuiltinFunction::Float));
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
            .insert("set".to_string(), Value::Builtin(BuiltinFunction::Set));
        self.builtins.insert(
            "frozenset".to_string(),
            Value::Builtin(BuiltinFunction::FrozenSet),
        );
        self.builtins
            .insert("bytes".to_string(), Value::Builtin(BuiltinFunction::Bytes));
        self.builtins.insert(
            "bytearray".to_string(),
            Value::Builtin(BuiltinFunction::ByteArray),
        );
        self.builtins.insert(
            "memoryview".to_string(),
            Value::Builtin(BuiltinFunction::MemoryView),
        );
        self.builtins.insert(
            "complex".to_string(),
            Value::Builtin(BuiltinFunction::Complex),
        );
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
        self.builtins
            .insert("iter".to_string(), Value::Builtin(BuiltinFunction::Iter));
        self.builtins
            .insert("next".to_string(), Value::Builtin(BuiltinFunction::Next));
        self.builtins
            .insert("aiter".to_string(), Value::Builtin(BuiltinFunction::AIter));
        self.builtins
            .insert("anext".to_string(), Value::Builtin(BuiltinFunction::ANext));
        self.builtins
            .insert("type".to_string(), Value::Builtin(BuiltinFunction::Type));
        self.builtins.insert(
            "locals".to_string(),
            Value::Builtin(BuiltinFunction::Locals),
        );
        self.builtins.insert(
            "globals".to_string(),
            Value::Builtin(BuiltinFunction::Globals),
        );
        self.builtins.insert(
            "getattr".to_string(),
            Value::Builtin(BuiltinFunction::GetAttr),
        );
        self.builtins.insert(
            "setattr".to_string(),
            Value::Builtin(BuiltinFunction::SetAttr),
        );
        self.builtins.insert(
            "delattr".to_string(),
            Value::Builtin(BuiltinFunction::DelAttr),
        );
        self.builtins.insert(
            "hasattr".to_string(),
            Value::Builtin(BuiltinFunction::HasAttr),
        );
        self.builtins
            .insert("super".to_string(), Value::Builtin(BuiltinFunction::Super));
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
            "StopAsyncIteration".to_string(),
            Value::ExceptionType("StopAsyncIteration".to_string()),
        );
        self.builtins.insert(
            "SystemExit".to_string(),
            Value::ExceptionType("SystemExit".to_string()),
        );
        self.builtins.insert(
            "KeyboardInterrupt".to_string(),
            Value::ExceptionType("KeyboardInterrupt".to_string()),
        );
        self.builtins.insert(
            "GeneratorExit".to_string(),
            Value::ExceptionType("GeneratorExit".to_string()),
        );
    }

    fn call_build_class(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::new(
                "__build_class__ expects at least a function and a name",
            ));
        }
        let metaclass = kwargs.remove("metaclass");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "__build_class__ got an unexpected keyword argument",
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
        frame.class_metaclass = metaclass.filter(|value| !matches!(value, Value::None));
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

fn seed_from_value(value: &Value) -> Result<u64, RuntimeError> {
    match value {
        Value::None => {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            Ok(nanos as u64)
        }
        Value::Int(value) => Ok(*value as u64),
        Value::Bool(value) => Ok(if *value { 1 } else { 0 }),
        Value::Float(value) => Ok(value.to_bits()),
        Value::Str(value) => {
            let mut hash: u64 = 1469598103934665603;
            for byte in value.as_bytes() {
                hash ^= *byte as u64;
                hash = hash.wrapping_mul(1099511628211);
            }
            Ok(hash)
        }
        _ => Err(RuntimeError::new("seed() unsupported type")),
    }
}

fn random_range_count(start: i64, stop: i64, step: i64) -> Result<i64, RuntimeError> {
    if step == 0 {
        return Err(RuntimeError::new("empty range for randrange()"));
    }
    if step > 0 {
        if start >= stop {
            return Err(RuntimeError::new("empty range for randrange()"));
        }
        let count = ((stop as i128 - start as i128 - 1) / step as i128) + 1;
        return i64::try_from(count).map_err(|_| RuntimeError::new("integer overflow"));
    }
    if start <= stop {
        return Err(RuntimeError::new("empty range for randrange()"));
    }
    let step_mag = -(step as i128);
    let count = ((start as i128 - stop as i128 - 1) / step_mag) + 1;
    i64::try_from(count).map_err(|_| RuntimeError::new("integer overflow"))
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

#[derive(Clone, Copy)]
enum NumericValue {
    Int(i64),
    Float(f64),
}

fn numeric_pair(left: &Value, right: &Value) -> Option<(NumericValue, NumericValue)> {
    let left = match left {
        Value::Int(value) => NumericValue::Int(*value),
        Value::Bool(value) => {
            if *value {
                NumericValue::Int(1)
            } else {
                NumericValue::Int(0)
            }
        }
        Value::Float(value) => NumericValue::Float(*value),
        _ => return None,
    };

    let right = match right {
        Value::Int(value) => NumericValue::Int(*value),
        Value::Bool(value) => {
            if *value {
                NumericValue::Int(1)
            } else {
                NumericValue::Int(0)
            }
        }
        Value::Float(value) => NumericValue::Float(*value),
        _ => return None,
    };

    Some((left, right))
}

fn numeric_as_f64(value: NumericValue) -> f64 {
    match value {
        NumericValue::Int(value) => value as f64,
        NumericValue::Float(value) => value,
    }
}

fn mod_float(left: f64, right: f64) -> Result<f64, RuntimeError> {
    if right == 0.0 {
        return Err(RuntimeError::new("float modulo by zero"));
    }
    let mut value = left - right * (left / right).floor();
    if value == 0.0 {
        value = 0.0f64.copysign(right);
    }
    Ok(value)
}

fn binary_operator<F>(
    args: Vec<Value>,
    kwargs: HashMap<String, Value>,
    op: F,
) -> Result<Value, RuntimeError>
where
    F: FnOnce(Value, Value) -> Result<Value, RuntimeError>,
{
    if !kwargs.is_empty() || args.len() != 2 {
        return Err(RuntimeError::new("operator expects two arguments"));
    }
    op(args[0].clone(), args[1].clone())
}

fn unary_predicate<F>(
    args: Vec<Value>,
    kwargs: HashMap<String, Value>,
    predicate: F,
) -> Result<Value, RuntimeError>
where
    F: FnOnce(&Value) -> bool,
{
    if !kwargs.is_empty() || args.len() != 1 {
        return Err(RuntimeError::new("predicate expects one argument"));
    }
    Ok(Value::Bool(predicate(&args[0])))
}

fn value_to_f64(value: Value) -> Result<f64, RuntimeError> {
    match value {
        Value::Float(value) => Ok(value),
        Value::Int(value) => Ok(value as f64),
        Value::Bool(value) => Ok(if value { 1.0 } else { 0.0 }),
        Value::Complex { real, imag } if imag == 0.0 => Ok(real),
        Value::Str(value) => value
            .trim()
            .parse::<f64>()
            .map_err(|_| RuntimeError::new("expected numeric value")),
        _ => Err(RuntimeError::new("expected numeric value")),
    }
}

fn value_to_path(value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::Str(path) => Ok(path.clone()),
        _ => Err(RuntimeError::new("path must be string")),
    }
}

fn value_to_bytes_payload(value: Value) -> Result<Vec<u8>, RuntimeError> {
    match value {
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => Ok(values.clone()),
            _ => Err(RuntimeError::new("expected bytes-like payload")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => Ok(values.clone()),
            _ => Err(RuntimeError::new("expected bytes-like payload")),
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => match &*view.source.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => Ok(values.clone()),
                _ => Err(RuntimeError::new("expected bytes-like payload")),
            },
            _ => Err(RuntimeError::new("expected bytes-like payload")),
        },
        Value::Str(text) => Ok(text.into_bytes()),
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => {
                let mut out = Vec::with_capacity(values.len());
                for value in values {
                    let byte = value_to_int(value.clone())?;
                    if !(0..=255).contains(&byte) {
                        return Err(RuntimeError::new("byte must be in range(0, 256)"));
                    }
                    out.push(byte as u8);
                }
                Ok(out)
            }
            _ => Err(RuntimeError::new("expected bytes-like payload")),
        },
        other => Err(RuntimeError::new(format!(
            "unsupported payload type: {}",
            format_value(&other)
        ))),
    }
}

fn bytes_like_from_value(value: Value) -> Result<Vec<u8>, RuntimeError> {
    match value {
        Value::Str(_) => Err(RuntimeError::new("expected bytes-like value")),
        other => value_to_bytes_payload(other),
    }
}

fn normalize_codec_encoding(value: Value) -> Result<String, RuntimeError> {
    let name = match value {
        Value::Str(name) => name.to_ascii_lowercase().replace('_', "-"),
        _ => return Err(RuntimeError::new("encoding must be string")),
    };
    match name.as_str() {
        "utf-8" | "utf8" => Ok("utf-8".to_string()),
        "ascii" => Ok("ascii".to_string()),
        "latin-1" | "latin1" => Ok("latin-1".to_string()),
        _ => Err(RuntimeError::new("unsupported encoding")),
    }
}

fn normalize_codec_errors(value: Value) -> Result<String, RuntimeError> {
    let mode = match value {
        Value::Str(mode) => mode.to_ascii_lowercase(),
        _ => return Err(RuntimeError::new("errors must be string")),
    };
    match mode.as_str() {
        "strict" | "ignore" | "replace" => Ok(mode),
        _ => Err(RuntimeError::new("unsupported error handler")),
    }
}

fn encode_text_bytes(text: &str, encoding: &str, errors: &str) -> Result<Vec<u8>, RuntimeError> {
    match encoding {
        "utf-8" => Ok(text.as_bytes().to_vec()),
        "ascii" => {
            let mut out = Vec::new();
            for ch in text.chars() {
                let code = ch as u32;
                if code <= 0x7F {
                    out.push(code as u8);
                    continue;
                }
                match errors {
                    "strict" => return Err(RuntimeError::new("ascii codec can't encode character")),
                    "ignore" => {}
                    "replace" => out.push(b'?'),
                    _ => return Err(RuntimeError::new("unsupported error handler")),
                }
            }
            Ok(out)
        }
        "latin-1" => {
            let mut out = Vec::new();
            for ch in text.chars() {
                let code = ch as u32;
                if code <= 0xFF {
                    out.push(code as u8);
                    continue;
                }
                match errors {
                    "strict" => {
                        return Err(RuntimeError::new("latin-1 codec can't encode character"));
                    }
                    "ignore" => {}
                    "replace" => out.push(b'?'),
                    _ => return Err(RuntimeError::new("unsupported error handler")),
                }
            }
            Ok(out)
        }
        _ => Err(RuntimeError::new("unsupported encoding")),
    }
}

fn decode_text_bytes(bytes: &[u8], encoding: &str, errors: &str) -> Result<String, RuntimeError> {
    match encoding {
        "utf-8" => decode_utf8_bytes(bytes, errors),
        "ascii" => {
            let mut out = String::new();
            for byte in bytes {
                if *byte <= 0x7F {
                    out.push(*byte as char);
                    continue;
                }
                match errors {
                    "strict" => return Err(RuntimeError::new("ascii codec can't decode byte")),
                    "ignore" => {}
                    "replace" => out.push('\u{FFFD}'),
                    _ => return Err(RuntimeError::new("unsupported error handler")),
                }
            }
            Ok(out)
        }
        "latin-1" => {
            let mut out = String::with_capacity(bytes.len());
            for byte in bytes {
                out.push(*byte as char);
            }
            Ok(out)
        }
        _ => Err(RuntimeError::new("unsupported encoding")),
    }
}

fn decode_utf8_bytes(bytes: &[u8], errors: &str) -> Result<String, RuntimeError> {
    if errors == "strict" {
        return std::str::from_utf8(bytes)
            .map(|value| value.to_string())
            .map_err(|_| RuntimeError::new("utf-8 codec can't decode bytes"));
    }
    let mut out = String::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        match std::str::from_utf8(&bytes[pos..]) {
            Ok(rest) => {
                out.push_str(rest);
                break;
            }
            Err(err) => {
                let valid = err.valid_up_to();
                if valid > 0 {
                    let fragment = std::str::from_utf8(&bytes[pos..pos + valid]).map_err(|_| {
                        RuntimeError::new("utf-8 codec can't decode bytes")
                    })?;
                    out.push_str(fragment);
                    pos += valid;
                }

                let invalid_len = err.error_len();
                match errors {
                    "ignore" => {
                        if let Some(len) = invalid_len {
                            pos += len;
                        } else {
                            break;
                        }
                    }
                    "replace" => {
                        out.push('\u{FFFD}');
                        if let Some(len) = invalid_len {
                            pos += len;
                        } else {
                            break;
                        }
                    }
                    _ => return Err(RuntimeError::new("unsupported error handler")),
                }
            }
        }
    }
    Ok(out)
}

fn json_escape_string(text: &str) -> String {
    let mut out = String::new();
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_serialize_value(value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::None => Ok("null".to_string()),
        Value::Bool(value) => Ok(if *value {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        Value::Int(value) => Ok(value.to_string()),
        Value::Float(value) => {
            if !value.is_finite() {
                return Err(RuntimeError::new("json cannot encode NaN or Infinity"));
            }
            Ok(value.to_string())
        }
        Value::Str(value) => Ok(json_escape_string(value)),
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => {
                let mut parts = Vec::with_capacity(values.len());
                for value in values {
                    parts.push(json_serialize_value(value)?);
                }
                Ok(format!("[{}]", parts.join(",")))
            }
            _ => Err(RuntimeError::new("json unsupported type")),
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => {
                let mut parts = Vec::with_capacity(values.len());
                for value in values {
                    parts.push(json_serialize_value(value)?);
                }
                Ok(format!("[{}]", parts.join(",")))
            }
            _ => Err(RuntimeError::new("json unsupported type")),
        },
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => {
                let mut parts = Vec::with_capacity(values.len());
                for value in values {
                    parts.push(json_serialize_value(value)?);
                }
                Ok(format!("[{}]", parts.join(",")))
            }
            _ => Err(RuntimeError::new("json unsupported type")),
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => {
                let mut parts = Vec::with_capacity(values.len());
                for value in values {
                    parts.push(json_serialize_value(value)?);
                }
                Ok(format!("[{}]", parts.join(",")))
            }
            _ => Err(RuntimeError::new("json unsupported type")),
        },
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(entries) => {
                let mut parts = Vec::with_capacity(entries.len());
                for (key, value) in entries {
                    let key = match key {
                        Value::Str(text) => text,
                        _ => return Err(RuntimeError::new("json object keys must be strings")),
                    };
                    parts.push(format!(
                        "{}:{}",
                        json_escape_string(key),
                        json_serialize_value(value)?
                    ));
                }
                Ok(format!("{{{}}}", parts.join(",")))
            }
            _ => Err(RuntimeError::new("json unsupported type")),
        },
        _ => Err(RuntimeError::new("json unsupported type")),
    }
}

#[derive(Debug)]
enum JsonNode {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<JsonNode>),
    Object(Vec<(String, JsonNode)>),
}

struct JsonParser<'a> {
    source: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source: source.as_bytes(),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<JsonNode, RuntimeError> {
        self.skip_ws();
        let value = self.parse_value()?;
        self.skip_ws();
        if self.pos != self.source.len() {
            return Err(RuntimeError::new("invalid JSON trailing data"));
        }
        Ok(value)
    }

    fn parse_value(&mut self) -> Result<JsonNode, RuntimeError> {
        self.skip_ws();
        let byte = self
            .peek()
            .ok_or_else(|| RuntimeError::new("unexpected end of JSON"))?;
        match byte {
            b'n' => self.parse_literal(b"null", JsonNode::Null),
            b't' => self.parse_literal(b"true", JsonNode::Bool(true)),
            b'f' => self.parse_literal(b"false", JsonNode::Bool(false)),
            b'"' => self.parse_string().map(JsonNode::String),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(RuntimeError::new("invalid JSON value")),
        }
    }

    fn parse_literal(&mut self, text: &[u8], node: JsonNode) -> Result<JsonNode, RuntimeError> {
        if self.source.get(self.pos..self.pos + text.len()) == Some(text) {
            self.pos += text.len();
            Ok(node)
        } else {
            Err(RuntimeError::new("invalid JSON literal"))
        }
    }

    fn parse_string(&mut self) -> Result<String, RuntimeError> {
        self.expect(b'"')?;
        let mut out = String::new();
        while let Some(byte) = self.next() {
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self
                        .next()
                        .ok_or_else(|| RuntimeError::new("invalid JSON escape"))?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex_u16()?;
                            let ch = char::from_u32(code as u32)
                                .ok_or_else(|| RuntimeError::new("invalid unicode escape"))?;
                            out.push(ch);
                        }
                        _ => return Err(RuntimeError::new("invalid JSON escape")),
                    }
                }
                b => out.push(b as char),
            }
        }
        Err(RuntimeError::new("unterminated JSON string"))
    }

    fn parse_hex_u16(&mut self) -> Result<u16, RuntimeError> {
        let mut value: u16 = 0;
        for _ in 0..4 {
            let byte = self
                .next()
                .ok_or_else(|| RuntimeError::new("invalid unicode escape"))?;
            value <<= 4;
            value |= match byte {
                b'0'..=b'9' => (byte - b'0') as u16,
                b'a'..=b'f' => (byte - b'a' + 10) as u16,
                b'A'..=b'F' => (byte - b'A' + 10) as u16,
                _ => return Err(RuntimeError::new("invalid unicode escape")),
            };
        }
        Ok(value)
    }

    fn parse_array(&mut self) -> Result<JsonNode, RuntimeError> {
        self.expect(b'[')?;
        self.skip_ws();
        let mut values = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonNode::Array(values));
        }
        loop {
            values.push(self.parse_value()?);
            self.skip_ws();
            match self.next() {
                Some(b',') => {
                    self.skip_ws();
                }
                Some(b']') => break,
                _ => return Err(RuntimeError::new("invalid JSON array")),
            }
        }
        Ok(JsonNode::Array(values))
    }

    fn parse_object(&mut self) -> Result<JsonNode, RuntimeError> {
        self.expect(b'{')?;
        self.skip_ws();
        let mut values = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonNode::Object(values));
        }
        loop {
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.parse_value()?;
            values.push((key, value));
            self.skip_ws();
            match self.next() {
                Some(b',') => {
                    self.skip_ws();
                }
                Some(b'}') => break,
                _ => return Err(RuntimeError::new("invalid JSON object")),
            }
        }
        Ok(JsonNode::Object(values))
    }

    fn parse_number(&mut self) -> Result<JsonNode, RuntimeError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.source[start..self.pos])
            .map_err(|_| RuntimeError::new("invalid JSON number"))?;
        if text.contains('.') || text.contains('e') || text.contains('E') {
            let value = text
                .parse::<f64>()
                .map_err(|_| RuntimeError::new("invalid JSON number"))?;
            Ok(JsonNode::Float(value))
        } else {
            let value = text
                .parse::<i64>()
                .map_err(|_| RuntimeError::new("invalid JSON number"))?;
            Ok(JsonNode::Int(value))
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), RuntimeError> {
        match self.next() {
            Some(found) if found == byte => Ok(()),
            _ => Err(RuntimeError::new("invalid JSON syntax")),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.pos += 1;
        Some(value)
    }
}

fn parse_json_node(source: &str) -> Result<JsonNode, RuntimeError> {
    JsonParser::new(source).parse()
}

fn json_node_to_value(node: JsonNode, heap: &Heap) -> Value {
    match node {
        JsonNode::Null => Value::None,
        JsonNode::Bool(value) => Value::Bool(value),
        JsonNode::Int(value) => Value::Int(value),
        JsonNode::Float(value) => Value::Float(value),
        JsonNode::String(value) => Value::Str(value),
        JsonNode::Array(values) => heap.alloc_list(
            values
                .into_iter()
                .map(|value| json_node_to_value(value, heap))
                .collect(),
        ),
        JsonNode::Object(entries) => heap.alloc_dict(
            entries
                .into_iter()
                .map(|(key, value)| (Value::Str(key), json_node_to_value(value, heap)))
                .collect(),
        ),
    }
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
}

fn current_utc_iso() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    let total_secs = now.as_secs() as i64;
    let days = total_secs.div_euclid(86_400);
    let sec_of_day = total_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = sec_of_day / 3600;
    let minute = (sec_of_day % 3600) / 60;
    let second = sec_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::None => false,
        Value::Bool(value) => *value,
        Value::Int(value) => *value != 0,
        Value::Float(value) => *value != 0.0,
        Value::Complex { real, imag } => *real != 0.0 || *imag != 0.0,
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
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => !values.is_empty(),
            _ => true,
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => !values.is_empty(),
            _ => true,
        },
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => !values.is_empty(),
            _ => true,
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => !values.is_empty(),
            _ => true,
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => match &*view.source.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => !values.is_empty(),
                _ => true,
            },
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
        | Value::Super(_)
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
        Value::ExceptionType(name) => Ok(Value::Exception(ExceptionObject::new(name, None))),
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
        "StopAsyncIteration" => Some("Exception"),
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

fn matches_finder_kind(value: &Value, expected: &str) -> bool {
    match value {
        Value::Str(name) => name == expected,
        Value::Dict(dict) => matches!(
            dict_get_value(dict, &Value::Str("kind".to_string())),
            Some(Value::Str(name)) if name == expected
        ),
        _ => false,
    }
}

fn split_relative_import_name(name: &str) -> (usize, String) {
    let mut level = 0usize;
    for ch in name.chars() {
        if ch == '.' {
            level += 1;
        } else {
            break;
        }
    }
    (level, name.chars().skip(level).collect())
}

fn dict_get_value(dict: &ObjRef, key: &Value) -> Option<Value> {
    let dict_kind = dict.kind();
    let entries = match &*dict_kind {
        Object::Dict(entries) => entries,
        _ => return None,
    };
    entries
        .iter()
        .find(|(entry_key, _)| entry_key == key)
        .map(|(_, value)| value.clone())
}

fn dict_set_value(dict: &ObjRef, key: Value, value: Value) {
    let mut dict_kind = dict.kind_mut();
    let entries = match &mut *dict_kind {
        Object::Dict(entries) => entries,
        _ => return,
    };
    if let Some((_, entry_value)) = entries.iter_mut().find(|(entry_key, _)| *entry_key == key) {
        *entry_value = value;
        return;
    }
    entries.push((key, value));
}

fn class_attr_lookup(class: &ObjRef, name: &str) -> Option<Value> {
    for candidate in class_attr_walk(class) {
        let class_kind = candidate.kind();
        if let Object::Class(class_data) = &*class_kind {
            if let Some(value) = class_data.attrs.get(name).cloned() {
                return Some(value);
            }
        }
    }
    None
}

fn class_attr_walk(class: &ObjRef) -> Vec<ObjRef> {
    let class_kind = class.kind();
    let class_data = match &*class_kind {
        Object::Class(class_data) => class_data,
        _ => return Vec::new(),
    };

    if !class_data.mro.is_empty() {
        return class_data.mro.clone();
    }

    let mut out = vec![class.clone()];
    for base in &class_data.bases {
        out.extend(class_attr_walk(base));
    }
    out
}

fn slot_names_from_value(value: Option<Value>) -> Option<Vec<String>> {
    let value = value?;
    let mut slots = Vec::new();
    match value {
        Value::Str(name) => slots.push(name),
        Value::Tuple(obj) => {
            if let Object::Tuple(values) = &*obj.kind() {
                for value in values {
                    if let Value::Str(name) = value {
                        slots.push(name.clone());
                    } else {
                        return None;
                    }
                }
            } else {
                return None;
            }
        }
        Value::List(obj) => {
            if let Object::List(values) = &*obj.kind() {
                for value in values {
                    if let Value::Str(name) = value {
                        slots.push(name.clone());
                    } else {
                        return None;
                    }
                }
            } else {
                return None;
            }
        }
        _ => return None,
    }
    Some(slots)
}

fn collect_slot_names(class: &ObjRef) -> Option<Vec<String>> {
    let mut has_slots = false;
    let mut names = Vec::new();
    for candidate in class_attr_walk(class) {
        if let Object::Class(class_data) = &*candidate.kind() {
            if let Some(slots) = &class_data.slots {
                has_slots = true;
                for slot in slots {
                    if !names.iter().any(|existing| existing == slot) {
                        names.push(slot.clone());
                    }
                }
            }
        }
    }
    if has_slots { Some(names) } else { None }
}

fn class_name_for_instance(instance: &ObjRef) -> Option<String> {
    let kind = instance.kind();
    let class = match &*kind {
        Object::Instance(instance_data) => instance_data.class.clone(),
        _ => return None,
    };
    match &*class.kind() {
        Object::Class(class_data) => Some(class_data.name.clone()),
        _ => None,
    }
}

fn classify_runtime_error(message: &str) -> &'static str {
    if message.trim() == "StopIteration" {
        return "StopIteration";
    }
    if message.trim() == "StopAsyncIteration" {
        return "StopAsyncIteration";
    }
    if message.trim() == "KeyboardInterrupt" {
        return "KeyboardInterrupt";
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
        return match (left, right) {
            (NumericValue::Int(left), NumericValue::Int(right)) => {
                let value = left
                    .checked_add(right)
                    .ok_or_else(|| RuntimeError::new("integer overflow"))?;
                Ok(Value::Int(value))
            }
            (left, right) => Ok(Value::Float(numeric_as_f64(left) + numeric_as_f64(right))),
        };
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

fn sub_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    match numeric_pair(&left, &right) {
        Some((NumericValue::Int(left), NumericValue::Int(right))) => {
            let value = left
                .checked_sub(right)
                .ok_or_else(|| RuntimeError::new("integer overflow"))?;
            Ok(Value::Int(value))
        }
        Some((left, right)) => Ok(Value::Float(numeric_as_f64(left) - numeric_as_f64(right))),
        None => Err(RuntimeError::new("unsupported operand type for -")),
    }
}

fn div_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let (left, right) = numeric_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for /"))?;
    let right_value = numeric_as_f64(right);
    if right_value == 0.0 {
        return Err(RuntimeError::new("division by zero"));
    }
    Ok(Value::Float(numeric_as_f64(left) / right_value))
}

fn floor_div_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let (left, right) = numeric_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for //"))?;
    match (left, right) {
        (NumericValue::Int(left), NumericValue::Int(right)) => {
            Ok(Value::Int(python_floor_div(left, right)?))
        }
        (left, right) => {
            let right_value = numeric_as_f64(right);
            if right_value == 0.0 {
                return Err(RuntimeError::new("division by zero"));
            }
            Ok(Value::Float((numeric_as_f64(left) / right_value).floor()))
        }
    }
}

fn mod_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let (left, right) = numeric_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for %"))?;
    match (left, right) {
        (NumericValue::Int(left), NumericValue::Int(right)) => {
            Ok(Value::Int(python_mod(left, right)?))
        }
        (left, right) => Ok(Value::Float(mod_float(
            numeric_as_f64(left),
            numeric_as_f64(right),
        )?)),
    }
}

fn pow_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let (left, right) = numeric_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for **"))?;
    match (left, right) {
        (NumericValue::Int(left), NumericValue::Int(right)) if right >= 0 => {
            let value = left
                .checked_pow(right as u32)
                .ok_or_else(|| RuntimeError::new("integer overflow"))?;
            Ok(Value::Int(value))
        }
        (left, right) => {
            let left = numeric_as_f64(left);
            let right = numeric_as_f64(right);
            if left == 0.0 && right < 0.0 {
                return Err(RuntimeError::new("division by zero"));
            }
            Ok(Value::Float(left.powf(right)))
        }
    }
}

fn neg_value(value: Value) -> Result<Value, RuntimeError> {
    match value {
        Value::Int(value) => {
            let value = value
                .checked_neg()
                .ok_or_else(|| RuntimeError::new("integer overflow"))?;
            Ok(Value::Int(value))
        }
        Value::Bool(value) => Ok(Value::Int(if value { -1 } else { 0 })),
        Value::Float(value) => Ok(Value::Float(-value)),
        _ => Err(RuntimeError::new("unsupported operand type for -")),
    }
}

fn pos_value(value: Value) -> Result<Value, RuntimeError> {
    match value {
        Value::Int(value) => Ok(Value::Int(value)),
        Value::Bool(value) => Ok(Value::Int(if value { 1 } else { 0 })),
        Value::Float(value) => Ok(Value::Float(value)),
        _ => Err(RuntimeError::new("unsupported operand type for +")),
    }
}

fn invert_value(value: Value) -> Result<Value, RuntimeError> {
    let value = value_to_int(value).map_err(|_| RuntimeError::new("unsupported operand type for ~"))?;
    Ok(Value::Int(!value))
}

fn and_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
        return Ok(Value::Bool(*left & *right));
    }
    let left =
        value_to_int(left).map_err(|_| RuntimeError::new("unsupported operand type for &"))?;
    let right =
        value_to_int(right).map_err(|_| RuntimeError::new("unsupported operand type for &"))?;
    Ok(Value::Int(left & right))
}

fn xor_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
        return Ok(Value::Bool(*left ^ *right));
    }
    let left =
        value_to_int(left).map_err(|_| RuntimeError::new("unsupported operand type for ^"))?;
    let right =
        value_to_int(right).map_err(|_| RuntimeError::new("unsupported operand type for ^"))?;
    Ok(Value::Int(left ^ right))
}

fn or_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
        return Ok(Value::Bool(*left | *right));
    }
    let left =
        value_to_int(left).map_err(|_| RuntimeError::new("unsupported operand type for |"))?;
    let right =
        value_to_int(right).map_err(|_| RuntimeError::new("unsupported operand type for |"))?;
    Ok(Value::Int(left | right))
}

fn lshift_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let left =
        value_to_int(left).map_err(|_| RuntimeError::new("unsupported operand type for <<"))?;
    let right =
        value_to_int(right).map_err(|_| RuntimeError::new("unsupported operand type for <<"))?;
    if right < 0 {
        return Err(RuntimeError::new("negative shift count"));
    }
    let shift = right as u32;
    if shift >= i64::BITS {
        return Ok(Value::Int(0));
    }
    let value = left
        .checked_shl(shift)
        .ok_or_else(|| RuntimeError::new("integer overflow"))?;
    Ok(Value::Int(value))
}

fn rshift_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let left =
        value_to_int(left).map_err(|_| RuntimeError::new("unsupported operand type for >>"))?;
    let right =
        value_to_int(right).map_err(|_| RuntimeError::new("unsupported operand type for >>"))?;
    if right < 0 {
        return Err(RuntimeError::new("negative shift count"));
    }
    let shift = right as u32;
    if shift >= i64::BITS {
        return Ok(Value::Int(if left < 0 { -1 } else { 0 }));
    }
    Ok(Value::Int(left >> shift))
}

fn matmul_values(_left: Value, _right: Value) -> Result<Value, RuntimeError> {
    Err(RuntimeError::new("unsupported operand type for @"))
}

fn compare_order(left: Value, right: Value) -> Result<Ordering, RuntimeError> {
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return Ok(match (left, right) {
            (NumericValue::Int(left), NumericValue::Int(right)) => left.cmp(&right),
            (left, right) => numeric_as_f64(left).total_cmp(&numeric_as_f64(right)),
        });
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
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => Ok(values.iter().any(|value| value == left)),
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => Ok(values.iter().any(|value| value == left)),
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::Str(haystack) => match left {
            Value::Str(needle) => Ok(haystack.contains(needle)),
            _ => Err(RuntimeError::new("in expects string on left")),
        },
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => {
                let needle = value_to_int(left.clone())?;
                if !(0..=255).contains(&needle) {
                    return Ok(false);
                }
                Ok(values.iter().any(|value| *value as i64 == needle))
            }
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => {
                let needle = value_to_int(left.clone())?;
                if !(0..=255).contains(&needle) {
                    return Ok(false);
                }
                Ok(values.iter().any(|value| *value as i64 == needle))
            }
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => match &*view.source.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => {
                    let needle = value_to_int(left.clone())?;
                    if !(0..=255).contains(&needle) {
                        return Ok(false);
                    }
                    Ok(values.iter().any(|value| *value as i64 == needle))
                }
                _ => Err(RuntimeError::new("unsupported operand type for in")),
            },
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        _ => Err(RuntimeError::new("unsupported operand type for in")),
    }
}

fn mul_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return match (left, right) {
            (NumericValue::Int(left), NumericValue::Int(right)) => {
                let value = left
                    .checked_mul(right)
                    .ok_or_else(|| RuntimeError::new("integer overflow"))?;
                Ok(Value::Int(value))
            }
            (left, right) => Ok(Value::Float(numeric_as_f64(left) * numeric_as_f64(right))),
        };
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
