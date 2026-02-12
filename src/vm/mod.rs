//! Bytecode virtual machine (minimal subset).

mod builtins_collections;
mod builtins_core;
mod builtins_import;
mod builtins_io;
mod builtins_numeric_time;
mod builtins_os;
mod builtins_system_misc;
mod containers;
mod ops;
mod stdlib;
mod vm_bootstrap_import;
mod vm_builtin_metadata;
mod vm_execution;
mod vm_native_dispatch;
mod vm_runtime_methods;

use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{IsTerminal, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::{Rc, Weak};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use self::containers::{
    dedup_hashable_values, dict_get_value, dict_remove_value, dict_set_value,
    dict_set_value_checked, ensure_hashable,
};
use self::ops::{
    add_values, and_values, compare_ge, compare_gt, compare_in, compare_le, compare_lt,
    compare_order, div_values, floor_div_values, invert_value, lshift_values, matmul_values,
    mod_values, mul_values, neg_value, or_values, ordering_from_cmp_value, pos_value, pow_values,
    rshift_values, sub_values, xor_values,
};
use crate::bytecode::cpython;
use crate::bytecode::metadata::OpcodeMetadata;
use crate::bytecode::{CodeObject, Instruction, Opcode};
use crate::compiler;
use crate::parser;
use crate::runtime::{
    BigInt, BoundMethod, BuiltinFunction, ClassObject, ExceptionObject, FunctionObject,
    GeneratorObject, Heap, InstanceObject, IteratorKind, IteratorObject, ModuleObject,
    NativeMethodKind, NativeMethodObject, Obj, ObjRef, Object, RuntimeError, SuperObject, Value,
    format_repr, format_value,
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
    is_bytecode: bool,
}

#[derive(Clone)]
struct AtexitHandler {
    callable: Value,
    args: Vec<Value>,
    kwargs: HashMap<String, Value>,
}

const DEFAULT_META_PATH_FINDER: &str = "pyrs.PathFinder";
const DEFAULT_PATH_HOOK: &str = "pyrs.FileFinder";
const SOURCE_FILE_LOADER: &str = "pyrs.SourceFileLoader";
const SOURCELESS_FILE_LOADER: &str = "pyrs.SourcelessFileLoader";
const NAMESPACE_LOADER: &str = "pyrs.NamespaceLoader";
const BUILTIN_MODULE_LOADER: &str = "pyrs.BuiltinLoader";
const PURE_STDLIB_JSON_MODULES: &[&str] = &["json", "json.decoder", "json.scanner"];
const PURE_STDLIB_PICKLE_MODULES: &[&str] = &["pickle", "pickletools", "copyreg"];
const PURE_STDLIB_RE_MODULES: &[&str] = &[
    "re",
    "re._compiler",
    "re._constants",
    "re._parser",
    "re._casefix",
];
const PURE_STDLIB_PATHLIB_MODULES: &[&str] = &["pathlib"];
const MT_N: usize = 624;
const MT_M: usize = 397;
const MT_MATRIX_A: u32 = 0x9908_b0df;
const MT_UPPER_MASK: u32 = 0x8000_0000;
const MT_LOWER_MASK: u32 = 0x7fff_ffff;
const SIGNAL_DEFAULT: i64 = 0;
const SIGNAL_IGNORE: i64 = 1;
const SIGNAL_SIGINT: i64 = 2;
const SIGNAL_SIGTERM: i64 = 15;
const PY_TPFLAGS_HEAPTYPE: i64 = 1 << 9;
const LIST_BACKING_STORAGE_ATTR: &str = "__pyrs_list_storage__";
const TUPLE_BACKING_STORAGE_ATTR: &str = "__pyrs_tuple_storage__";
const STR_BACKING_STORAGE_ATTR: &str = "__pyrs_str_storage__";
const BYTES_BACKING_STORAGE_ATTR: &str = "__pyrs_bytes_storage__";
const INT_BACKING_STORAGE_ATTR: &str = "__pyrs_int_storage__";
const FLOAT_BACKING_STORAGE_ATTR: &str = "__pyrs_float_storage__";
const COMPLEX_BACKING_STORAGE_ATTR: &str = "__pyrs_complex_storage__";
const DICT_BACKING_STORAGE_ATTR: &str = "__pyrs_dict_storage__";
const SET_BACKING_STORAGE_ATTR: &str = "__pyrs_set_storage__";
const FROZENSET_BACKING_STORAGE_ATTR: &str = "__pyrs_frozenset_storage__";
const INSTANCE_DICT_STORAGE_ATTR: &str = "__pyrs_instance_dict_storage__";
static MONOTONIC_START: OnceLock<Instant> = OnceLock::new();
static OPCODE_METADATA: OnceLock<OpcodeMetadata> = OnceLock::new();
static SUBMODULE_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StructEndian {
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StructFieldKind {
    Pad,
    Bytes,
    Char,
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StructFieldSpec {
    kind: StructFieldKind,
    count: usize,
}

#[derive(Debug, Clone)]
struct StructFormatSpec {
    endian: StructEndian,
    fields: Vec<StructFieldSpec>,
    size: usize,
    value_count: usize,
}

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

#[derive(Clone)]
enum RePatternValue {
    Str(String),
    Bytes(Vec<u8>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OneArgCallHotPath {
    Generic,
    SimplePositional,
    SimplePositionalNoCells,
}

#[derive(Clone)]
struct OneArgCallSiteCacheEntry {
    func_id: u64,
    func_epoch: u64,
    hot_path: OneArgCallHotPath,
    cached_code: Option<Rc<CodeObject>>,
    cached_module: Option<ObjRef>,
    cached_owner_class: Option<ObjRef>,
    cached_closure: Option<Vec<ObjRef>>,
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, allow(dead_code))]
struct LoadGlobalSiteCacheEntry {
    globals_module_id: u64,
    globals_version: u64,
    builtins_version: u64,
    value: Value,
    fused_local_idx: Option<u32>,
    fused_const_idx: Option<u32>,
    fused_const_small_int: Option<i64>,
    fused_direct_one_arg_no_cells: bool,
    fused_direct_func: Option<ObjRef>,
    fused_direct_func_epoch: u64,
}

#[derive(Clone, Copy)]
#[cfg_attr(debug_assertions, allow(dead_code))]
struct LoadFastSiteCacheEntry {
    compare_rhs_int: i64,
    jump_target: usize,
}

#[derive(Clone)]
enum LoadAttrSiteCacheKind {
    InstanceFunction { function: ObjRef },
    InstanceBuiltin { builtin: BuiltinFunction },
    InstanceClassMethod { descriptor: ObjRef },
    InstanceStaticMethod { descriptor: ObjRef },
}

#[derive(Clone)]
struct LoadAttrSiteCacheEntry {
    class_id: u64,
    class_version: u64,
    owner_class: ObjRef,
    owner_class_version: u64,
    kind: LoadAttrSiteCacheKind,
}

const LOAD_ATTR_CACHE_WAYS: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(debug_assertions, allow(dead_code))]
enum QuickenedSiteKind {
    None,
    LoadFastPlain,
    LoadFastCompareLtConstJump,
    CompareLtInt,
    CallFunctionOneArg,
}

const LOGGING_PERCENT_VALIDATION_PATTERN: &str =
    r"%\(\w+\)[#0+ -]*(\*|\d+)?(\.(\*|\d+))?[diouxefgcrsa%]";
const PKGUTIL_RESOLVE_NAME_PATTERN: &str =
    r"^(?P<pkg>(?!\d)(\w+)(\.(?!\d)(\w+))*)(?P<cln>:(?P<obj>(?!\d)(\w+)(\.(?!\d)(\w+))*)?)?$";
const LOCAL_SHIM_MODULES: &[&str] = &["enum", "pkgutil", "importlib.resources"];

struct Frame {
    code: Rc<CodeObject>,
    ip: usize,
    last_ip: usize,
    stack: Vec<Value>,
    locals: HashMap<String, Value>,
    fast_locals: Vec<Option<Value>>,
    module_locals_dict: Option<ObjRef>,
    cells: Vec<ObjRef>,
    module: ObjRef,
    function_globals: ObjRef,
    function_globals_version: u64,
    globals_fallback: Option<ObjRef>,
    locals_fallback: Option<HashMap<String, Value>>,
    owner_class: Option<ObjRef>,
    is_module: bool,
    return_module: bool,
    discard_result: bool,
    return_instance: Option<ObjRef>,
    return_class: bool,
    class_bases: Vec<ObjRef>,
    class_metaclass: Option<Value>,
    class_keywords: HashMap<String, Value>,
    blocks: Vec<Block>,
    active_exception: Option<Value>,
    expect_none_return: bool,
    generator_owner: Option<ObjRef>,
    generator_awaiting_resume_value: bool,
    generator_resume_value: Option<Value>,
    generator_pending_throw: Option<Value>,
    generator_resume_kind: Option<GeneratorResumeKind>,
    yield_from_iter: Option<Value>,
    quickened_sites: Vec<QuickenedSiteKind>,
    load_fast_inline_cache: Vec<Option<LoadFastSiteCacheEntry>>,
    load_attr_inline_cache: Vec<[Option<LoadAttrSiteCacheEntry>; LOAD_ATTR_CACHE_WAYS]>,
    one_arg_inline_cache: Vec<Option<OneArgCallSiteCacheEntry>>,
    load_global_inline_cache: Vec<Option<LoadGlobalSiteCacheEntry>>,
    simple_one_arg_no_cells: bool,
}

impl Frame {
    fn new(
        code: Rc<CodeObject>,
        module: ObjRef,
        is_module: bool,
        return_module: bool,
        cells: Vec<ObjRef>,
        owner_class: Option<ObjRef>,
    ) -> Self {
        let fast_locals_len = code.fast_local_count;
        let instruction_len = code.instructions.len();
        Self {
            code,
            ip: 0,
            last_ip: 0,
            stack: Vec::with_capacity(8),
            locals: HashMap::new(),
            fast_locals: vec![None; fast_locals_len],
            module_locals_dict: None,
            cells,
            module: module.clone(),
            function_globals_version: module_globals_version(&module),
            function_globals: module,
            globals_fallback: None,
            locals_fallback: None,
            owner_class,
            is_module,
            return_module,
            discard_result: false,
            return_instance: None,
            return_class: false,
            class_bases: Vec::new(),
            class_metaclass: None,
            class_keywords: HashMap::new(),
            blocks: Vec::with_capacity(2),
            active_exception: None,
            expect_none_return: false,
            generator_owner: None,
            generator_awaiting_resume_value: false,
            generator_resume_value: None,
            generator_pending_throw: None,
            generator_resume_kind: None,
            yield_from_iter: None,
            quickened_sites: vec![QuickenedSiteKind::None; instruction_len],
            load_fast_inline_cache: vec![None; instruction_len],
            load_attr_inline_cache: (0..instruction_len).map(|_| [None, None]).collect(),
            one_arg_inline_cache: vec![None; instruction_len],
            load_global_inline_cache: vec![None; instruction_len],
            simple_one_arg_no_cells: false,
        }
    }

    fn reset_for_reuse(
        &mut self,
        code: Rc<CodeObject>,
        module: ObjRef,
        is_module: bool,
        return_module: bool,
        cells: Vec<ObjRef>,
        owner_class: Option<ObjRef>,
    ) {
        let same_code = Rc::ptr_eq(&self.code, &code);
        let instruction_len = code.instructions.len();
        debug_assert!(self.stack.is_empty());
        debug_assert!(self.locals.is_empty());
        debug_assert!(self.blocks.is_empty());
        debug_assert!(self.class_bases.is_empty());
        debug_assert!(self.class_keywords.is_empty());
        debug_assert!(self.module_locals_dict.is_none());
        debug_assert!(self.globals_fallback.is_none());
        debug_assert!(self.locals_fallback.is_none());
        debug_assert!(self.return_instance.is_none());
        debug_assert!(self.class_metaclass.is_none());
        debug_assert!(self.active_exception.is_none());
        debug_assert!(self.generator_owner.is_none());
        debug_assert!(self.generator_resume_value.is_none());
        debug_assert!(self.generator_pending_throw.is_none());
        debug_assert!(self.generator_resume_kind.is_none());
        debug_assert!(self.yield_from_iter.is_none());
        self.code = code;
        self.ip = 0;
        self.last_ip = 0;
        if cells.is_empty() {
            if !self.cells.is_empty() {
                self.cells.clear();
            }
        } else {
            self.cells = cells;
        }
        self.module = module.clone();
        self.function_globals_version = module_globals_version(&module);
        self.function_globals = module;
        self.owner_class = owner_class;
        self.is_module = is_module;
        self.return_module = return_module;
        self.discard_result = false;
        self.return_class = false;
        self.expect_none_return = false;
        self.generator_awaiting_resume_value = false;
        self.simple_one_arg_no_cells = false;

        if !same_code {
            self.quickened_sites = vec![QuickenedSiteKind::None; instruction_len];
            self.load_fast_inline_cache = vec![None; instruction_len];
            self.load_attr_inline_cache = (0..instruction_len).map(|_| [None, None]).collect();
            self.one_arg_inline_cache = vec![None; instruction_len];
            self.load_global_inline_cache = vec![None; instruction_len];
            let fast_locals_len = self.code.fast_local_count;
            if self.fast_locals.len() < fast_locals_len {
                self.fast_locals.resize(fast_locals_len, None);
            } else {
                self.fast_locals.truncate(fast_locals_len);
            }
        }
        self.fast_locals.fill(None);
    }

    fn prepare_simple_one_arg_no_cells_ref(
        &mut self,
        code: &Rc<CodeObject>,
        module: &ObjRef,
        owner_class: Option<&ObjRef>,
    ) {
        let same_code = Rc::ptr_eq(&self.code, code);
        if same_code
            && self.module.id() == module.id()
            && self.owner_class.is_none()
            && owner_class.is_none()
            && self.fast_locals.len() == 1
            && code.plain_positional_arg0_slot == Some(0)
        {
            self.ip = 0;
            self.last_ip = 0;
            self.function_globals_version = module_globals_version(module);
            self.simple_one_arg_no_cells = true;
            return;
        }
        let instruction_len = code.instructions.len();
        debug_assert!(self.locals.is_empty());
        debug_assert!(self.cells.is_empty());
        debug_assert!(self.blocks.is_empty());
        debug_assert!(self.class_bases.is_empty());
        debug_assert!(self.class_keywords.is_empty());
        debug_assert!(self.stack.is_empty());
        debug_assert!(self.module_locals_dict.is_none());
        debug_assert!(self.globals_fallback.is_none());
        debug_assert!(self.locals_fallback.is_none());
        debug_assert!(self.return_instance.is_none());
        debug_assert!(self.class_metaclass.is_none());
        debug_assert!(self.generator_owner.is_none());
        debug_assert!(self.generator_resume_value.is_none());
        debug_assert!(self.generator_pending_throw.is_none());
        debug_assert!(self.generator_resume_kind.is_none());
        debug_assert!(self.yield_from_iter.is_none());
        debug_assert!(self.active_exception.is_none());
        self.code = code.clone();
        self.ip = 0;
        self.last_ip = 0;
        if self.module.id() != module.id() {
            self.module = module.clone();
            self.function_globals = module.clone();
        }
        self.function_globals_version = module_globals_version(module);
        let owner_changed = match (&self.owner_class, owner_class) {
            (Some(existing), Some(new_owner)) => existing.id() != new_owner.id(),
            (None, None) => false,
            _ => true,
        };
        if owner_changed {
            self.owner_class = owner_class.cloned();
        }
        self.simple_one_arg_no_cells = true;

        if !same_code {
            self.quickened_sites = vec![QuickenedSiteKind::None; instruction_len];
            self.load_fast_inline_cache = vec![None; instruction_len];
            self.load_attr_inline_cache = (0..instruction_len).map(|_| [None, None]).collect();
            self.one_arg_inline_cache = vec![None; instruction_len];
            self.load_global_inline_cache = vec![None; instruction_len];
            let fast_locals_len = self.code.fast_local_count;
            if self.fast_locals.len() < fast_locals_len {
                self.fast_locals.resize(fast_locals_len, None);
            } else {
                self.fast_locals.truncate(fast_locals_len);
            }
        }
        let single_arg_direct_slot =
            self.code.fast_local_count == 1 && self.code.plain_positional_arg0_slot == Some(0);
        if !single_arg_direct_slot {
            self.fast_locals.fill(None);
        }
    }
}

pub struct Vm {
    frames: Vec<Box<Frame>>,
    frame_pool: Vec<Box<Frame>>,
    simple_frame_pool: Vec<Box<Frame>>,
    simple_slot0_pool: Vec<Box<Frame>>,
    simple_slot0_pool_key: Option<(usize, u64)>,
    builtins: HashMap<String, Value>,
    modules: HashMap<String, ObjRef>,
    main_module: ObjRef,
    module_paths: Vec<PathBuf>,
    heap: Heap,
    random: Mt19937,
    generator_states: HashMap<u64, Box<Frame>>,
    generator_returns: HashMap<u64, Value>,
    pending_generator_exception: Option<Value>,
    active_generator_resume: Option<u64>,
    active_generator_resume_boundary: Option<usize>,
    generator_resume_outcome: Option<GeneratorResumeOutcome>,
    run_stop_depth: Option<usize>,
    signal_handlers: HashMap<i64, Value>,
    socket_default_timeout: Option<f64>,
    open_files: HashMap<i64, fs::File>,
    fd_inheritable: HashMap<i64, bool>,
    next_fd: i64,
    child_processes: HashMap<i64, Child>,
    child_exit_status: HashMap<i64, i64>,
    csv_dialects: HashMap<String, Value>,
    csv_field_size_limit: i64,
    pickle_copyreg_cache: HashMap<String, Value>,
    pickle_symbol_cache: HashMap<String, Value>,
    defaultdict_factories: HashMap<u64, Value>,
    exception_parents: HashMap<String, String>,
    finalized_del_objects: HashSet<u64>,
    pending_del_instances: HashMap<u64, ObjRef>,
    weakref_finalizers: HashMap<u64, (Weak<Obj>, Vec<ObjRef>)>,
    atexit_handlers: Vec<AtexitHandler>,
    prefer_pure_json_when_available: bool,
    prefer_pure_pickle_when_available: bool,
    prefer_pure_re_when_available: bool,
    list_eq_in_progress: Vec<(u64, u64)>,
    repr_in_progress: Vec<u64>,
    recursion_limit: i64,
    builtins_version: u64,
    class_attr_versions: HashMap<u64, u64>,
}

impl Drop for Vm {
    fn drop(&mut self) {
        // Break reference cycles before field teardown so per-VM object graphs
        // do not accumulate across harness runs.
        self.heap.collect_cycles(&[]);
    }
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
            frames: Vec::with_capacity(128),
            frame_pool: Vec::with_capacity(128),
            simple_frame_pool: Vec::with_capacity(128),
            simple_slot0_pool: Vec::with_capacity(128),
            simple_slot0_pool_key: None,
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
            socket_default_timeout: None,
            open_files: HashMap::new(),
            fd_inheritable: HashMap::new(),
            next_fd: 3,
            child_processes: HashMap::new(),
            child_exit_status: HashMap::new(),
            csv_dialects: HashMap::new(),
            csv_field_size_limit: 131_072,
            pickle_copyreg_cache: HashMap::new(),
            pickle_symbol_cache: HashMap::new(),
            defaultdict_factories: HashMap::new(),
            exception_parents: HashMap::new(),
            finalized_del_objects: HashSet::new(),
            pending_del_instances: HashMap::new(),
            weakref_finalizers: HashMap::new(),
            atexit_handlers: Vec::new(),
            prefer_pure_json_when_available: true,
            prefer_pure_pickle_when_available: true,
            prefer_pure_re_when_available: true,
            list_eq_in_progress: Vec::new(),
            repr_in_progress: Vec::new(),
            recursion_limit: 1000,
            builtins_version: 1,
            class_attr_versions: HashMap::new(),
        };
        let main = vm.main_module.clone();
        vm.set_module_metadata(&main, "__main__", None, None, false, Vec::new(), false);
        vm.install_sys_module();
        vm.install_importlib_modules();
        vm.install_random_module();
        vm.install_stdlib_modules();
        vm.install_builtins();
        vm.install_builtins_module();
        vm
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: Value) {
        let mut touched_version = None;
        if let Object::Module(module) = &mut *self.main_module.kind_mut() {
            module.globals.insert(name.into(), value);
            module.touch_globals_version();
            touched_version = Some(module.globals_version);
        }
        if let Some(version) = touched_version {
            self.propagate_module_globals_version(self.main_module.id(), version);
        }
    }

    #[inline]
    fn touch_builtins_version(&mut self) {
        self.builtins_version = self.builtins_version.wrapping_add(1);
        if self.builtins_version == 0 {
            self.builtins_version = 1;
        }
    }

    fn propagate_module_globals_version(&mut self, module_id: u64, version: u64) {
        for frame in &mut self.frames {
            if frame.function_globals.id() == module_id {
                frame.function_globals_version = version;
            }
            for slot in &mut frame.load_global_inline_cache {
                if matches!(slot, Some(cached) if cached.globals_module_id == module_id) {
                    *slot = None;
                }
            }
        }
        for frame in self.generator_states.values_mut() {
            if frame.function_globals.id() == module_id {
                frame.function_globals_version = version;
            }
            for slot in &mut frame.load_global_inline_cache {
                if matches!(slot, Some(cached) if cached.globals_module_id == module_id) {
                    *slot = None;
                }
            }
        }
    }

    #[inline]
    fn class_attr_version(&self, class: &ObjRef) -> u64 {
        self.class_attr_versions
            .get(&class.id())
            .copied()
            .unwrap_or(1)
    }

    #[inline]
    fn touch_class_attr_version_by_id(&mut self, class_id: u64) -> u64 {
        let entry = self.class_attr_versions.entry(class_id).or_insert(1);
        *entry = entry.wrapping_add(1);
        if *entry == 0 {
            *entry = 1;
        }
        *entry
    }

    #[inline]
    fn touch_class_attr_version(&mut self, class: &ObjRef) -> u64 {
        self.touch_class_attr_version_by_id(class.id())
    }

    fn acquire_frame(
        &mut self,
        code: Rc<CodeObject>,
        module: ObjRef,
        is_module: bool,
        return_module: bool,
        cells: Vec<ObjRef>,
        owner_class: Option<ObjRef>,
    ) -> Box<Frame> {
        if let Some(mut frame) = self.frame_pool.pop() {
            frame.reset_for_reuse(code, module, is_module, return_module, cells, owner_class);
            frame
        } else {
            Box::new(Frame::new(
                code,
                module,
                is_module,
                return_module,
                cells,
                owner_class,
            ))
        }
    }

    fn recycle_frame(&mut self, mut frame: Box<Frame>) {
        if self.frame_pool.len() >= 256 {
            return;
        }
        if !frame.fast_locals.is_empty() {
            frame.fast_locals.fill(None);
        }
        if !frame.stack.is_empty() {
            frame.stack.clear();
        }
        if !frame.locals.is_empty() {
            frame.locals.clear();
        }
        if !frame.cells.is_empty() {
            frame.cells.clear();
        }
        if !frame.blocks.is_empty() {
            frame.blocks.clear();
        }
        if !frame.class_bases.is_empty() {
            frame.class_bases.clear();
        }
        if !frame.class_keywords.is_empty() {
            frame.class_keywords.clear();
        }
        frame.module_locals_dict = None;
        frame.globals_fallback = None;
        frame.locals_fallback = None;
        frame.return_instance = None;
        frame.class_metaclass = None;
        frame.active_exception = None;
        frame.generator_owner = None;
        frame.generator_resume_value = None;
        frame.generator_pending_throw = None;
        frame.generator_resume_kind = None;
        frame.yield_from_iter = None;
        frame.simple_one_arg_no_cells = false;
        self.frame_pool.push(frame);
    }

    fn acquire_simple_frame_no_cells_ref(
        &mut self,
        code: &Rc<CodeObject>,
        module: &ObjRef,
        owner_class: Option<&ObjRef>,
    ) -> Box<Frame> {
        if let Some(mut frame) = self.simple_frame_pool.pop() {
            frame.prepare_simple_one_arg_no_cells_ref(code, module, owner_class);
            frame
        } else {
            let mut frame = Box::new(Frame::new(
                code.clone(),
                module.clone(),
                false,
                false,
                Vec::new(),
                owner_class.cloned(),
            ));
            frame.simple_one_arg_no_cells = true;
            frame
        }
    }

    #[inline(always)]
    fn slot0_pool_key(code: &Rc<CodeObject>, module: &ObjRef) -> (usize, u64) {
        (Rc::as_ptr(code) as usize, module.id())
    }

    #[inline(always)]
    fn retarget_simple_slot0_pool(&mut self, key: (usize, u64)) {
        if self.simple_slot0_pool_key == Some(key) {
            return;
        }
        while let Some(frame) = self.simple_slot0_pool.pop() {
            self.simple_frame_pool.push(frame);
        }
        self.simple_slot0_pool_key = Some(key);
    }

    #[inline(always)]
    fn acquire_simple_frame_slot0_no_cells_fast_ref(
        &mut self,
        code: &Rc<CodeObject>,
        module: &ObjRef,
        globals_version: u64,
    ) -> Box<Frame> {
        debug_assert!(code.fast_local_count == 1);
        debug_assert!(code.plain_positional_arg0_slot == Some(0));
        let key = Self::slot0_pool_key(code, module);
        self.retarget_simple_slot0_pool(key);
        if let Some(mut frame) = self.simple_slot0_pool.pop() {
            frame.ip = 0;
            frame.last_ip = 0;
            frame.function_globals_version = globals_version;
            frame.simple_one_arg_no_cells = true;
            return frame;
        }
        if let Some(mut frame) = self.simple_frame_pool.pop() {
            frame.prepare_simple_one_arg_no_cells_ref(code, module, None);
            frame.function_globals_version = globals_version;
            return frame;
        }
        let mut frame = Box::new(Frame::new(
            code.clone(),
            module.clone(),
            false,
            false,
            Vec::new(),
            None,
        ));
        frame.function_globals_version = globals_version;
        frame.simple_one_arg_no_cells = true;
        frame
    }

    #[inline(always)]
    fn try_recycle_simple_slot0_frame(&mut self, mut frame: Box<Frame>) -> Result<(), Box<Frame>> {
        if frame.owner_class.is_some()
            || frame.code.fast_local_count != 1
            || frame.code.plain_positional_arg0_slot != Some(0)
        {
            return Err(frame);
        }
        let key = Self::slot0_pool_key(&frame.code, &frame.module);
        self.retarget_simple_slot0_pool(key);
        if self.simple_slot0_pool.len() >= 256 {
            return Err(frame);
        }
        frame.ip = 0;
        frame.last_ip = 0;
        if let Some(slot) = frame.fast_locals.get_mut(0) {
            *slot = None;
        }
        frame.simple_one_arg_no_cells = true;
        self.simple_slot0_pool.push(frame);
        Ok(())
    }

    fn recycle_simple_frame(&mut self, mut frame: Box<Frame>) {
        if self.simple_frame_pool.len() >= 256 {
            return;
        }
        let single_arg_direct_slot =
            frame.code.fast_local_count == 1 && frame.code.plain_positional_arg0_slot == Some(0);
        if single_arg_direct_slot
            && frame.locals.is_empty()
            && frame.cells.is_empty()
            && frame.blocks.is_empty()
            && frame.class_bases.is_empty()
            && frame.class_keywords.is_empty()
            && frame.stack.is_empty()
            && frame.module_locals_dict.is_none()
            && frame.globals_fallback.is_none()
            && frame.locals_fallback.is_none()
            && frame.return_instance.is_none()
            && frame.class_metaclass.is_none()
            && frame.generator_owner.is_none()
            && frame.generator_resume_value.is_none()
            && frame.generator_pending_throw.is_none()
            && frame.generator_resume_kind.is_none()
            && frame.yield_from_iter.is_none()
            && frame.active_exception.is_none()
            && !frame.discard_result
            && !frame.return_class
            && !frame.expect_none_return
            && !frame.generator_awaiting_resume_value
            && !frame.is_module
            && !frame.return_module
        {
            if let Err(mut frame) = self.try_recycle_simple_slot0_frame(frame) {
                if let Some(slot) = frame.fast_locals.get_mut(0) {
                    *slot = None;
                }
                frame.simple_one_arg_no_cells = true;
                self.simple_frame_pool.push(frame);
            }
            return;
        }
        if !frame.locals.is_empty() {
            frame.locals.clear();
        }
        if !frame.cells.is_empty() {
            frame.cells.clear();
        }
        if !frame.blocks.is_empty() {
            frame.blocks.clear();
        }
        if !frame.class_bases.is_empty() {
            frame.class_bases.clear();
        }
        if !frame.class_keywords.is_empty() {
            frame.class_keywords.clear();
        }
        if !frame.stack.is_empty() {
            frame.stack.clear();
        }
        if frame.module_locals_dict.is_some() {
            frame.module_locals_dict = None;
        }
        if frame.globals_fallback.is_some() {
            frame.globals_fallback = None;
        }
        if frame.locals_fallback.is_some() {
            frame.locals_fallback = None;
        }
        if frame.return_instance.is_some() {
            frame.return_instance = None;
        }
        if frame.class_metaclass.is_some() {
            frame.class_metaclass = None;
        }
        if frame.generator_owner.is_some() {
            frame.generator_owner = None;
        }
        if frame.generator_resume_value.is_some() {
            frame.generator_resume_value = None;
        }
        if frame.generator_pending_throw.is_some() {
            frame.generator_pending_throw = None;
        }
        if frame.generator_resume_kind.is_some() {
            frame.generator_resume_kind = None;
        }
        if frame.yield_from_iter.is_some() {
            frame.yield_from_iter = None;
        }
        if frame.active_exception.is_some() {
            frame.active_exception = None;
        }
        if frame.discard_result {
            frame.discard_result = false;
        }
        if frame.return_class {
            frame.return_class = false;
        }
        if frame.expect_none_return {
            frame.expect_none_return = false;
        }
        if frame.generator_awaiting_resume_value {
            frame.generator_awaiting_resume_value = false;
        }
        if frame.is_module {
            frame.is_module = false;
        }
        if frame.return_module {
            frame.return_module = false;
        }
        frame.simple_one_arg_no_cells = true;
        if single_arg_direct_slot {
            if let Err(mut frame) = self.try_recycle_simple_slot0_frame(frame) {
                frame.fast_locals.fill(None);
                self.simple_frame_pool.push(frame);
            }
            return;
        }
        frame.fast_locals.fill(None);
        self.simple_frame_pool.push(frame);
    }

    #[inline(always)]
    fn recycle_simple_frame_clean_slot0_unchecked(&mut self, mut frame: Box<Frame>) {
        debug_assert!(frame.owner_class.is_none());
        debug_assert!(frame.code.fast_local_count == 1);
        debug_assert!(frame.code.plain_positional_arg0_slot == Some(0));
        debug_assert!(frame.locals.is_empty());
        debug_assert!(frame.cells.is_empty());
        debug_assert!(frame.blocks.is_empty());
        debug_assert!(frame.class_bases.is_empty());
        debug_assert!(frame.class_keywords.is_empty());
        debug_assert!(frame.stack.is_empty());
        debug_assert!(frame.module_locals_dict.is_none());
        debug_assert!(frame.globals_fallback.is_none());
        debug_assert!(frame.locals_fallback.is_none());
        debug_assert!(frame.return_instance.is_none());
        debug_assert!(frame.class_metaclass.is_none());
        debug_assert!(frame.generator_owner.is_none());
        debug_assert!(frame.generator_resume_value.is_none());
        debug_assert!(frame.generator_pending_throw.is_none());
        debug_assert!(frame.generator_resume_kind.is_none());
        debug_assert!(frame.yield_from_iter.is_none());
        debug_assert!(frame.active_exception.is_none());
        debug_assert!(!frame.discard_result);
        debug_assert!(!frame.return_class);
        debug_assert!(!frame.expect_none_return);
        debug_assert!(!frame.generator_awaiting_resume_value);
        debug_assert!(!frame.is_module);
        debug_assert!(!frame.return_module);

        if self.simple_slot0_pool.len() >= 256 {
            return;
        }

        let key = Self::slot0_pool_key(&frame.code, &frame.module);
        self.retarget_simple_slot0_pool(key);
        frame.ip = 0;
        frame.last_ip = 0;
        if let Some(slot) = frame.fast_locals.get_mut(0) {
            *slot = None;
        }
        frame.simple_one_arg_no_cells = true;
        self.simple_slot0_pool.push(frame);
    }

    pub fn get_global(&self, name: &str) -> Option<Value> {
        if let Object::Module(module) = &*self.main_module.kind() {
            return module.globals.get(name).cloned();
        }
        None
    }

    pub fn run_shutdown_hooks(&mut self) -> Result<(), RuntimeError> {
        let pushed_shutdown_frame = if self.frames.is_empty() {
            let shutdown_code = Rc::new(CodeObject::new("<shutdown>", "<shutdown>"));
            let shutdown_frame = Frame::new(
                shutdown_code,
                self.main_module.clone(),
                true,
                false,
                Vec::new(),
                None,
            );
            self.frames.push(Box::new(shutdown_frame));
            true
        } else {
            false
        };
        let shutdown_result = self.builtin_atexit_run_exitfuncs(Vec::new(), HashMap::new());
        self.run_weakref_atexit_finalizers();
        if pushed_shutdown_frame {
            let _ = self.frames.pop();
        }
        self.run_pending_del_finalizers();
        shutdown_result.map(|_| ())
    }

    pub fn add_module_path(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        if self.module_paths.iter().any(|existing| existing == &path) {
            return;
        }
        self.module_paths.push(path);
        self.sync_sys_path_from_module_paths();
        self.maybe_prefer_cpython_pure_stdlib_modules();
    }

    pub fn add_module_path_front(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.module_paths.retain(|existing| existing != &path);
        self.module_paths.insert(0, path);
        self.sync_sys_path_from_module_paths();
        self.maybe_prefer_cpython_pure_stdlib_modules();
    }

    pub fn enable_pure_json_preference(&mut self) {
        self.prefer_pure_json_when_available = true;
        self.maybe_prefer_cpython_pure_stdlib_modules();
    }

    pub fn enable_pure_pickle_preference(&mut self) {
        self.prefer_pure_pickle_when_available = true;
        self.maybe_prefer_cpython_pure_stdlib_modules();
    }

    pub fn enable_pure_re_preference(&mut self) {
        self.prefer_pure_re_when_available = true;
        self.maybe_prefer_cpython_pure_stdlib_modules();
    }

    pub fn import_module(&mut self, name: &str) -> Result<(), RuntimeError> {
        let _ = self.import_module_object(name)?;
        Ok(())
    }

    pub fn noop_builtin_inventory(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut visited = HashSet::new();
        let mut module_names: Vec<String> = self.modules.keys().cloned().collect();
        module_names.sort();
        for module_name in module_names {
            let Some(module) = self.modules.get(&module_name) else {
                continue;
            };
            collect_noop_symbols_from_module(&module_name, module, &mut out, &mut visited);
        }
        out.sort();
        out.dedup();
        out
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

    fn capture_closure_cells_for_code(
        &self,
        code: &CodeObject,
    ) -> Result<Vec<ObjRef>, RuntimeError> {
        let Some(frame) = self.frames.last() else {
            return Ok(Vec::new());
        };
        if code.freevars.is_empty() {
            return Ok(Vec::new());
        }
        let mut closure = Vec::with_capacity(code.freevars.len());
        for name in &code.freevars {
            let idx = if let Some(cell_idx) =
                frame.code.cellvars.iter().position(|cell| cell == name)
            {
                cell_idx
            } else if let Some(free_idx) = frame.code.freevars.iter().position(|free| free == name)
            {
                frame.code.cellvars.len() + free_idx
            } else {
                return Err(RuntimeError::new(format!(
                    "free variable '{}' is not available in enclosing scope",
                    name
                )));
            };
            let cell = frame
                .cells
                .get(idx)
                .cloned()
                .ok_or_else(|| RuntimeError::new("cell index out of range"))?;
            closure.push(cell);
        }
        Ok(closure)
    }

    fn class_lookup_fallback_from_frame(frame: &Frame) -> Option<HashMap<String, Value>> {
        if frame.is_module {
            return None;
        }
        let mut fallback = frame.locals.clone();
        for (idx, slot) in frame.fast_locals.iter().enumerate() {
            if let Some(value) = slot {
                if let Some(name) = frame.code.names.get(idx) {
                    fallback.insert(name.clone(), value.clone());
                }
            }
        }
        for name in &frame.code.cellvars {
            if let Some(value) = frame_cell_value(frame, name) {
                fallback.insert(name.clone(), value);
            }
        }
        for name in &frame.code.freevars {
            if let Some(value) = frame_cell_value(frame, name) {
                fallback.insert(name.clone(), value);
            }
        }
        if fallback.is_empty() {
            None
        } else {
            Some(fallback)
        }
    }

    pub fn heap_object_count(&self) -> usize {
        self.heap.live_objects_count()
    }

    fn track_instance_del_candidate(&mut self, class: &ObjRef, instance: &ObjRef) {
        if class_attr_lookup(class, "__del__").is_some() {
            self.pending_del_instances
                .insert(instance.id(), instance.clone());
        }
    }

    fn runtime_error_to_exception_value(&mut self, err: RuntimeError) -> Value {
        let classified = classify_runtime_error(&err.message);
        let exception_type = if classified == "RuntimeError" {
            extract_runtime_error_exception_name(&err.message)
                .unwrap_or_else(|| classified.to_string())
        } else {
            classified.to_string()
        };
        let mut exception_message = Some(err.message.clone());
        if let Some(from_traceback) =
            extract_runtime_error_final_message(&err.message, &exception_type)
        {
            exception_message = from_traceback;
        } else if let Some(from_prefixed) =
            extract_prefixed_exception_message(&err.message, &exception_type)
        {
            exception_message = from_prefixed;
        }
        let exception = ExceptionObject::new(exception_type, exception_message);
        let args = if let Some(message) = &exception.message {
            self.heap.alloc_tuple(vec![Value::Str(message.clone())])
        } else {
            self.heap.alloc_tuple(Vec::new())
        };
        exception
            .attrs
            .borrow_mut()
            .insert("args".to_string(), args);
        Value::Exception(Box::new(exception))
    }

    fn emit_unraisable_exception(
        &mut self,
        exception: Value,
        object: Option<Value>,
        err_msg: Option<&str>,
    ) {
        let exc_type = match &exception {
            Value::Exception(exc) => self
                .builtins
                .get(&exc.name)
                .cloned()
                .unwrap_or_else(|| Value::ExceptionType(exc.name.clone())),
            Value::ExceptionType(name) => self
                .builtins
                .get(name)
                .cloned()
                .unwrap_or_else(|| Value::ExceptionType(name.clone())),
            Value::Class(class) => Value::Class(class.clone()),
            _ => self
                .builtins
                .get("RuntimeError")
                .cloned()
                .unwrap_or_else(|| Value::ExceptionType("RuntimeError".to_string())),
        };
        let record = match self
            .heap
            .alloc_module(ModuleObject::new("__unraisable__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => return,
        };
        if let Object::Module(module_data) = &mut *record.kind_mut() {
            module_data.globals.insert("exc_type".to_string(), exc_type);
            module_data
                .globals
                .insert("exc_value".to_string(), exception.clone());
            module_data
                .globals
                .insert("exc_traceback".to_string(), Value::None);
            module_data.globals.insert(
                "err_msg".to_string(),
                err_msg
                    .map(|value| Value::Str(value.to_string()))
                    .unwrap_or(Value::None),
            );
            module_data
                .globals
                .insert("object".to_string(), object.unwrap_or(Value::None));
        }
        let hook = self.modules.get("sys").and_then(|sys_module| {
            let Object::Module(module_data) = &*sys_module.kind() else {
                return None;
            };
            module_data
                .globals
                .get("unraisablehook")
                .cloned()
                .or_else(|| module_data.globals.get("__unraisablehook__").cloned())
        });
        let Some(hook) = hook else {
            return;
        };
        let _ =
            self.call_internal_preserving_caller(hook, vec![Value::Module(record)], HashMap::new());
        if let Some(frame) = self.frames.last_mut() {
            frame.active_exception = None;
        }
    }

    fn run_pending_del_finalizers(&mut self) {
        let mut ready = Vec::new();
        for (id, instance) in &self.pending_del_instances {
            if instance.strong_count() == 1 {
                ready.push((*id, instance.clone()));
            }
        }

        for (obj_id, instance) in ready {
            self.pending_del_instances.remove(&obj_id);
            if self.finalized_del_objects.contains(&obj_id) {
                continue;
            }
            let receiver = Value::Instance(instance.clone());
            let del_method = match self.lookup_bound_special_method(&receiver, "__del__") {
                Ok(Some(method)) => method,
                _ => continue,
            };
            self.finalized_del_objects.insert(obj_id);
            match self.call_internal_preserving_caller(del_method, Vec::new(), HashMap::new()) {
                Ok(InternalCallOutcome::Value(_)) => {}
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    let active_exception = self
                        .frames
                        .last_mut()
                        .and_then(|frame| frame.active_exception.take())
                        .unwrap_or_else(|| {
                            self.runtime_error_to_exception_value(RuntimeError::new(
                                "RuntimeError: __del__ finalizer failed",
                            ))
                        });
                    self.emit_unraisable_exception(
                        active_exception,
                        Some(receiver.clone()),
                        Some("Exception ignored in __del__"),
                    );
                }
                Err(err) => {
                    let exception = self.runtime_error_to_exception_value(err);
                    self.emit_unraisable_exception(
                        exception,
                        Some(receiver.clone()),
                        Some("Exception ignored in __del__"),
                    );
                }
            }
            if let Some(frame) = self.frames.last_mut() {
                frame.active_exception = None;
            }
        }
        self.run_pending_weakref_finalizers();
    }

    fn register_weakref_finalizer(&mut self, target: &ObjRef, finalizer: ObjRef) {
        let target_id = target.id();
        self.weakref_finalizers
            .entry(target_id)
            .and_modify(|(_, finalizers)| finalizers.push(finalizer.clone()))
            .or_insert_with(|| (target.downgrade(), vec![finalizer]));
    }

    fn unregister_weakref_finalizer(&mut self, target_id: u64, finalizer_id: u64) {
        if let Some((_, finalizers)) = self.weakref_finalizers.get_mut(&target_id) {
            finalizers.retain(|entry| entry.id() != finalizer_id);
            if finalizers.is_empty() {
                self.weakref_finalizers.remove(&target_id);
            }
        }
    }

    fn run_pending_weakref_finalizers(&mut self) {
        let ready_target_ids: Vec<u64> = self
            .weakref_finalizers
            .iter()
            .filter_map(|(target_id, (target, _))| {
                if target.upgrade().is_none() {
                    Some(*target_id)
                } else {
                    None
                }
            })
            .collect();

        for target_id in ready_target_ids {
            let Some((_, finalizers)) = self.weakref_finalizers.remove(&target_id) else {
                continue;
            };
            for finalizer in finalizers {
                let (callable, call_args, call_kwargs, alive) = match &mut *finalizer.kind_mut() {
                    Object::Module(module_data) => {
                        let alive =
                            matches!(module_data.globals.get("alive"), Some(Value::Bool(true)));
                        if !alive {
                            (Value::None, Vec::new(), HashMap::new(), false)
                        } else {
                            module_data
                                .globals
                                .insert("alive".to_string(), Value::Bool(false));
                            let callable = module_data
                                .globals
                                .get("_func")
                                .cloned()
                                .unwrap_or(Value::None);
                            let call_args = match module_data.globals.get("_args").cloned() {
                                Some(Value::Tuple(obj)) => match &*obj.kind() {
                                    Object::Tuple(values) => values.clone(),
                                    _ => Vec::new(),
                                },
                                _ => Vec::new(),
                            };
                            let call_kwargs = match module_data.globals.get("_kwargs").cloned() {
                                Some(Value::Dict(obj)) => match &*obj.kind() {
                                    Object::Dict(entries) => entries
                                        .iter()
                                        .filter_map(|(key, value)| match key {
                                            Value::Str(name) => Some((name.clone(), value.clone())),
                                            _ => None,
                                        })
                                        .collect(),
                                    _ => HashMap::new(),
                                },
                                _ => HashMap::new(),
                            };
                            (callable, call_args, call_kwargs, true)
                        }
                    }
                    _ => (Value::None, Vec::new(), HashMap::new(), false),
                };
                if !alive || matches!(callable, Value::None) {
                    continue;
                }
                let _ = self.call_internal_preserving_caller(callable, call_args, call_kwargs);
                if let Some(frame) = self.frames.last_mut() {
                    frame.active_exception = None;
                }
            }
        }
    }

    fn run_weakref_atexit_finalizers(&mut self) {
        let target_ids: Vec<u64> = self.weakref_finalizers.keys().copied().collect();
        for target_id in target_ids {
            let Some((_, finalizers)) = self.weakref_finalizers.remove(&target_id) else {
                continue;
            };
            for finalizer in finalizers {
                let (callable, call_args, call_kwargs, alive, run_on_atexit) = match &mut *finalizer
                    .kind_mut()
                {
                    Object::Module(module_data) => {
                        let alive =
                            matches!(module_data.globals.get("alive"), Some(Value::Bool(true)));
                        let run_on_atexit =
                            matches!(module_data.globals.get("atexit"), Some(Value::Bool(true)));
                        if !alive || !run_on_atexit {
                            (Value::None, Vec::new(), HashMap::new(), false, false)
                        } else {
                            module_data
                                .globals
                                .insert("alive".to_string(), Value::Bool(false));
                            let callable = module_data
                                .globals
                                .get("_func")
                                .cloned()
                                .unwrap_or(Value::None);
                            let call_args = match module_data.globals.get("_args").cloned() {
                                Some(Value::Tuple(obj)) => match &*obj.kind() {
                                    Object::Tuple(values) => values.clone(),
                                    _ => Vec::new(),
                                },
                                _ => Vec::new(),
                            };
                            let call_kwargs = match module_data.globals.get("_kwargs").cloned() {
                                Some(Value::Dict(obj)) => match &*obj.kind() {
                                    Object::Dict(entries) => entries
                                        .iter()
                                        .filter_map(|(key, value)| match key {
                                            Value::Str(name) => Some((name.clone(), value.clone())),
                                            _ => None,
                                        })
                                        .collect(),
                                    _ => HashMap::new(),
                                },
                                _ => HashMap::new(),
                            };
                            (callable, call_args, call_kwargs, true, true)
                        }
                    }
                    _ => (Value::None, Vec::new(), HashMap::new(), false, false),
                };
                if !alive || !run_on_atexit || matches!(callable, Value::None) {
                    continue;
                }
                let _ = self.call_internal_preserving_caller(callable, call_args, call_kwargs);
                if let Some(frame) = self.frames.last_mut() {
                    frame.active_exception = None;
                }
            }
        }
    }

    pub fn gc_collect(&mut self) {
        self.run_pending_del_finalizers();
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
            roots.extend(frame.fast_locals.iter().flatten().cloned());
            for cell in &frame.cells {
                roots.push(Value::Cell(cell.clone()));
            }
            roots.push(Value::Module(frame.module.clone()));
            roots.push(Value::Module(frame.function_globals.clone()));
            if let Some(fallback) = &frame.globals_fallback {
                roots.push(Value::Module(fallback.clone()));
            }
            if let Some(fallback) = &frame.locals_fallback {
                roots.extend(fallback.values().cloned());
            }
            if let Some(instance) = &frame.return_instance {
                roots.push(Value::Instance(instance.clone()));
            }
            for base in &frame.class_bases {
                roots.push(Value::Class(base.clone()));
            }
            if let Some(owner) = &frame.owner_class {
                roots.push(Value::Class(owner.clone()));
            }
            if let Some(meta) = &frame.class_metaclass {
                roots.push(meta.clone());
            }
            roots.extend(frame.class_keywords.values().cloned());
            if let Some(exc) = &frame.active_exception {
                roots.push(exc.clone());
            }
        }
        for frame in self.generator_states.values() {
            roots.extend(frame.stack.iter().cloned());
            roots.extend(frame.locals.values().cloned());
            roots.extend(frame.fast_locals.iter().flatten().cloned());
            for cell in &frame.cells {
                roots.push(Value::Cell(cell.clone()));
            }
            roots.push(Value::Module(frame.module.clone()));
            roots.push(Value::Module(frame.function_globals.clone()));
            if let Some(fallback) = &frame.globals_fallback {
                roots.push(Value::Module(fallback.clone()));
            }
            if let Some(fallback) = &frame.locals_fallback {
                roots.extend(fallback.values().cloned());
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
            roots.extend(frame.class_keywords.values().cloned());
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
        let unreachable = self.heap.unreachable_objects(&roots);
        for obj in unreachable {
            let obj_id = obj.id();
            self.pending_del_instances.remove(&obj_id);
            if self.finalized_del_objects.contains(&obj_id) {
                continue;
            }
            if !matches!(&*obj.kind(), Object::Instance(_)) {
                continue;
            }
            let receiver = Value::Instance(obj.clone());
            let del_method = match self.lookup_bound_special_method(&receiver, "__del__") {
                Ok(Some(method)) => method,
                _ => continue,
            };
            self.finalized_del_objects.insert(obj_id);
            let _ = self.call_internal_preserving_caller(del_method, Vec::new(), HashMap::new());
            if let Some(frame) = self.frames.last_mut() {
                frame.active_exception = None;
            }
        }
        self.heap.collect_cycles(&roots);
    }

    fn builtin_gc_collect(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new(
                "gc.collect() expects at most one argument",
            ));
        }
        self.gc_collect();
        Ok(Value::Int(0))
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
        self.frames.push(Box::new(Frame::new(
            code,
            self.main_module.clone(),
            true,
            false,
            cells,
            None,
        )));
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
            let flags = match self.heap.alloc_module(ModuleObject::new("sys.flags")) {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(flags_data) = &mut *flags.kind_mut() {
                flags_data
                    .globals
                    .insert("debug".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("inspect".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("interactive".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("no_site".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("no_user_site".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("optimize".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("verbose".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("bytes_warning".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("ignore_environment".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("isolated".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("dont_write_bytecode".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("quiet".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("hash_randomization".to_string(), Value::Int(1));
                flags_data
                    .globals
                    .insert("dev_mode".to_string(), Value::Bool(false));
                flags_data
                    .globals
                    .insert("utf8_mode".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("warn_default_encoding".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("safe_path".to_string(), Value::Bool(false));
                flags_data
                    .globals
                    .insert("int_max_str_digits".to_string(), Value::Int(4300));
                flags_data.globals.insert("gil".to_string(), Value::Int(1));
                flags_data
                    .globals
                    .insert("thread_inherit_context".to_string(), Value::Int(0));
                flags_data
                    .globals
                    .insert("context_aware_warnings".to_string(), Value::Int(0));
            }
            module_data
                .globals
                .insert("flags".to_string(), Value::Module(flags));
            module_data.globals.insert(
                "version_info".to_string(),
                self.heap.alloc_tuple(vec![
                    Value::Int(3),
                    Value::Int(14),
                    Value::Int(0),
                    Value::Str("final".to_string()),
                    Value::Int(0),
                ]),
            );
            module_data.globals.insert(
                "version".to_string(),
                Value::Str("3.14.0 (pyrs)".to_string()),
            );
            module_data.globals.insert(
                "copyright".to_string(),
                Value::Str("Copyright (c) 2001-2026 Python Software Foundation.".to_string()),
            );
            let implementation = match self
                .heap
                .alloc_module(ModuleObject::new("sys.implementation"))
            {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(impl_data) = &mut *implementation.kind_mut() {
                impl_data
                    .globals
                    .insert("name".to_string(), Value::Str("pyrs".to_string()));
                impl_data.globals.insert(
                    "cache_tag".to_string(),
                    Value::Str("cpython-314".to_string()),
                );
                impl_data.globals.insert(
                    "version".to_string(),
                    self.heap.alloc_tuple(vec![
                        Value::Int(3),
                        Value::Int(14),
                        Value::Int(0),
                        Value::Str("final".to_string()),
                        Value::Int(0),
                    ]),
                );
                impl_data
                    .globals
                    .insert("hexversion".to_string(), Value::Int(0x030e00f0));
                impl_data
                    .globals
                    .insert("_multiarch".to_string(), Value::Str(String::new()));
            }
            module_data
                .globals
                .insert("implementation".to_string(), Value::Module(implementation));
            let argv = std::env::args().map(Value::Str).collect::<Vec<_>>();
            module_data
                .globals
                .insert("argv".to_string(), self.heap.alloc_list(argv));
            let executable = std::env::args()
                .next()
                .unwrap_or_else(|| "pyrs".to_string());
            module_data
                .globals
                .insert("executable".to_string(), Value::Str(executable));
            module_data
                .globals
                .insert("prefix".to_string(), Value::Str(String::new()));
            module_data
                .globals
                .insert("base_prefix".to_string(), Value::Str(String::new()));
            module_data
                .globals
                .insert("exec_prefix".to_string(), Value::Str(String::new()));
            module_data
                .globals
                .insert("base_exec_prefix".to_string(), Value::Str(String::new()));
            let platform = match std::env::consts::OS {
                "macos" => "darwin",
                other => other,
            };
            module_data
                .globals
                .insert("platform".to_string(), Value::Str(platform.to_string()));
            module_data.globals.insert(
                "byteorder".to_string(),
                Value::Str(
                    if cfg!(target_endian = "little") {
                        "little"
                    } else {
                        "big"
                    }
                    .to_string(),
                ),
            );
            module_data
                .globals
                .insert("_framework".to_string(), Value::Str(String::new()));
            module_data
                .globals
                .insert("abiflags".to_string(), Value::Str(String::new()));
            module_data
                .globals
                .insert("dont_write_bytecode".to_string(), Value::Bool(false));
            module_data
                .globals
                .insert("platlibdir".to_string(), Value::Str("lib".to_string()));
            module_data.globals.insert(
                "getfilesystemencoding".to_string(),
                Value::Builtin(BuiltinFunction::SysGetFilesystemEncoding),
            );
            module_data.globals.insert(
                "getfilesystemencodeerrors".to_string(),
                Value::Builtin(BuiltinFunction::SysGetFilesystemEncodeErrors),
            );
            module_data.globals.insert(
                "getrefcount".to_string(),
                Value::Builtin(BuiltinFunction::SysGetRefCount),
            );
            module_data.globals.insert(
                "getrecursionlimit".to_string(),
                Value::Builtin(BuiltinFunction::SysGetRecursionLimit),
            );
            module_data.globals.insert(
                "setrecursionlimit".to_string(),
                Value::Builtin(BuiltinFunction::SysSetRecursionLimit),
            );
            module_data
                .globals
                .insert("intern".to_string(), Value::Builtin(BuiltinFunction::Str));
            module_data
                .globals
                .insert("audit".to_string(), Value::Builtin(BuiltinFunction::NoOp));
            module_data.globals.insert(
                "unraisablehook".to_string(),
                Value::Builtin(BuiltinFunction::NoOp),
            );
            module_data.globals.insert(
                "__unraisablehook__".to_string(),
                Value::Builtin(BuiltinFunction::NoOp),
            );
            let build_stream = |name: &str,
                                write_builtin: BuiltinFunction,
                                flush_builtin: BuiltinFunction,
                                heap: &Heap|
             -> ObjRef {
                let stream = match heap.alloc_module(ModuleObject::new(name.to_string())) {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(stream_data) = &mut *stream.kind_mut() {
                    stream_data
                        .globals
                        .insert("write".to_string(), Value::Builtin(write_builtin));
                    stream_data
                        .globals
                        .insert("flush".to_string(), Value::Builtin(flush_builtin));
                    stream_data.globals.insert(
                        "isatty".to_string(),
                        Value::Builtin(BuiltinFunction::SysStreamIsATty),
                    );
                    stream_data
                        .globals
                        .insert("encoding".to_string(), Value::Str("utf-8".to_string()));
                    let buffer_write_builtin = match write_builtin {
                        BuiltinFunction::SysStdoutWrite => BuiltinFunction::SysStdoutBufferWrite,
                        BuiltinFunction::SysStderrWrite => BuiltinFunction::SysStderrBufferWrite,
                        _ => write_builtin,
                    };
                    let buffer =
                        match heap.alloc_module(ModuleObject::new(format!("{name}.buffer"))) {
                            Value::Module(obj) => obj,
                            _ => unreachable!(),
                        };
                    if let Object::Module(buffer_data) = &mut *buffer.kind_mut() {
                        buffer_data
                            .globals
                            .insert("write".to_string(), Value::Builtin(buffer_write_builtin));
                        buffer_data
                            .globals
                            .insert("flush".to_string(), Value::Builtin(flush_builtin));
                    }
                    stream_data
                        .globals
                        .insert("buffer".to_string(), Value::Module(buffer));
                }
                stream
            };
            let stdout = build_stream(
                "sys.stdout",
                BuiltinFunction::SysStdoutWrite,
                BuiltinFunction::SysStdoutFlush,
                &self.heap,
            );
            let stderr = build_stream(
                "sys.stderr",
                BuiltinFunction::SysStderrWrite,
                BuiltinFunction::SysStderrFlush,
                &self.heap,
            );
            let stdin = build_stream(
                "sys.stdin",
                BuiltinFunction::SysStdinWrite,
                BuiltinFunction::SysStdinFlush,
                &self.heap,
            );
            module_data
                .globals
                .insert("stdout".to_string(), Value::Module(stdout.clone()));
            module_data
                .globals
                .insert("__stdout__".to_string(), Value::Module(stdout));
            module_data
                .globals
                .insert("stderr".to_string(), Value::Module(stderr.clone()));
            module_data
                .globals
                .insert("__stderr__".to_string(), Value::Module(stderr));
            module_data
                .globals
                .insert("stdin".to_string(), Value::Module(stdin.clone()));
            module_data
                .globals
                .insert("__stdin__".to_string(), Value::Module(stdin));
            module_data
                .globals
                .insert("hexversion".to_string(), Value::Int(0x030e00f0));
            module_data
                .globals
                .insert("maxsize".to_string(), Value::Int((1_i64 << 62) - 1));
            let float_info = match self.heap.alloc_module(ModuleObject::new("sys.float_info")) {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(float_info_data) = &mut *float_info.kind_mut() {
                float_info_data
                    .globals
                    .insert("max".to_string(), Value::Float(f64::MAX));
                float_info_data
                    .globals
                    .insert("min".to_string(), Value::Float(f64::MIN_POSITIVE));
                float_info_data
                    .globals
                    .insert("epsilon".to_string(), Value::Float(f64::EPSILON));
                float_info_data
                    .globals
                    .insert("dig".to_string(), Value::Int(f64::DIGITS as i64));
                float_info_data.globals.insert(
                    "mant_dig".to_string(),
                    Value::Int(f64::MANTISSA_DIGITS as i64),
                );
                float_info_data
                    .globals
                    .insert("max_exp".to_string(), Value::Int(f64::MAX_EXP as i64));
                float_info_data
                    .globals
                    .insert("max_10_exp".to_string(), Value::Int(f64::MAX_10_EXP as i64));
                float_info_data
                    .globals
                    .insert("min_exp".to_string(), Value::Int(f64::MIN_EXP as i64));
                float_info_data
                    .globals
                    .insert("min_10_exp".to_string(), Value::Int(f64::MIN_10_EXP as i64));
                float_info_data
                    .globals
                    .insert("radix".to_string(), Value::Int(2));
                float_info_data
                    .globals
                    .insert("rounds".to_string(), Value::Int(1));
            }
            module_data
                .globals
                .insert("float_info".to_string(), Value::Module(float_info));
            module_data.globals.insert(
                "float_repr_style".to_string(),
                Value::Str("short".to_string()),
            );
            let hash_info = match self.heap.alloc_module(ModuleObject::new("sys.hash_info")) {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(hash_data) = &mut *hash_info.kind_mut() {
                hash_data
                    .globals
                    .insert("width".to_string(), Value::Int(64));
                hash_data
                    .globals
                    .insert("modulus".to_string(), Value::Int(2_305_843_009_213_693_951));
                hash_data
                    .globals
                    .insert("inf".to_string(), Value::Int(314_159));
                hash_data.globals.insert("nan".to_string(), Value::Int(0));
                hash_data
                    .globals
                    .insert("imag".to_string(), Value::Int(1_000_003));
                hash_data
                    .globals
                    .insert("algorithm".to_string(), Value::Str("siphash13".to_string()));
                hash_data
                    .globals
                    .insert("hash_bits".to_string(), Value::Int(64));
                hash_data
                    .globals
                    .insert("seed_bits".to_string(), Value::Int(128));
                hash_data
                    .globals
                    .insert("cutoff".to_string(), Value::Int(0));
            }
            module_data
                .globals
                .insert("hash_info".to_string(), Value::Module(hash_info));
            module_data
                .globals
                .insert("warnoptions".to_string(), self.heap.alloc_list(Vec::new()));
            let monitoring = match self.heap.alloc_module(ModuleObject::new("sys.monitoring")) {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(monitoring_data) = &mut *monitoring.kind_mut() {
                let events = match self
                    .heap
                    .alloc_module(ModuleObject::new("sys.monitoring.events"))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(events_data) = &mut *events.kind_mut() {
                    events_data
                        .globals
                        .insert("PY_START".to_string(), Value::Int(1 << 0));
                    events_data
                        .globals
                        .insert("PY_RESUME".to_string(), Value::Int(1 << 1));
                    events_data
                        .globals
                        .insert("PY_THROW".to_string(), Value::Int(1 << 2));
                    events_data
                        .globals
                        .insert("LINE".to_string(), Value::Int(1 << 3));
                    events_data
                        .globals
                        .insert("JUMP".to_string(), Value::Int(1 << 4));
                    events_data
                        .globals
                        .insert("PY_RETURN".to_string(), Value::Int(1 << 5));
                    events_data
                        .globals
                        .insert("PY_YIELD".to_string(), Value::Int(1 << 6));
                    events_data
                        .globals
                        .insert("PY_UNWIND".to_string(), Value::Int(1 << 7));
                    events_data
                        .globals
                        .insert("RAISE".to_string(), Value::Int(1 << 8));
                    events_data
                        .globals
                        .insert("STOP_ITERATION".to_string(), Value::Int(1 << 9));
                    events_data
                        .globals
                        .insert("INSTRUCTION".to_string(), Value::Int(1 << 10));
                }
                monitoring_data
                    .globals
                    .insert("events".to_string(), Value::Module(events));
                monitoring_data
                    .globals
                    .insert("DEBUGGER_ID".to_string(), Value::Int(0));
                monitoring_data
                    .globals
                    .insert("DISABLE".to_string(), Value::Int(-1));
                monitoring_data.globals.insert(
                    "get_tool".to_string(),
                    Value::Builtin(BuiltinFunction::NoOp),
                );
                monitoring_data.globals.insert(
                    "use_tool_id".to_string(),
                    Value::Builtin(BuiltinFunction::NoOp),
                );
                monitoring_data.globals.insert(
                    "clear_tool_id".to_string(),
                    Value::Builtin(BuiltinFunction::NoOp),
                );
                monitoring_data.globals.insert(
                    "free_tool_id".to_string(),
                    Value::Builtin(BuiltinFunction::NoOp),
                );
                monitoring_data.globals.insert(
                    "register_callback".to_string(),
                    Value::Builtin(BuiltinFunction::NoOp),
                );
                monitoring_data.globals.insert(
                    "set_events".to_string(),
                    Value::Builtin(BuiltinFunction::NoOp),
                );
                monitoring_data.globals.insert(
                    "set_local_events".to_string(),
                    Value::Builtin(BuiltinFunction::NoOp),
                );
                monitoring_data.globals.insert(
                    "restart_events".to_string(),
                    Value::Builtin(BuiltinFunction::NoOp),
                );
            }
            module_data
                .globals
                .insert("monitoring".to_string(), Value::Module(monitoring));
            let jit_module = match self.heap.alloc_module(ModuleObject::new("sys._jit")) {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(jit_data) = &mut *jit_module.kind_mut() {
                jit_data.globals.insert(
                    "is_enabled".to_string(),
                    Value::Builtin(BuiltinFunction::NoOp),
                );
                jit_data.globals.insert(
                    "is_available".to_string(),
                    Value::Builtin(BuiltinFunction::NoOp),
                );
            }
            module_data
                .globals
                .insert("_jit".to_string(), Value::Module(jit_module));
            module_data.globals.insert(
                "_getframe".to_string(),
                Value::Builtin(BuiltinFunction::SysGetFrame),
            );
            module_data.globals.insert(
                "exception".to_string(),
                Value::Builtin(BuiltinFunction::SysException),
            );
            module_data.globals.insert(
                "exc_info".to_string(),
                Value::Builtin(BuiltinFunction::SysExcInfo),
            );
            module_data
                .globals
                .insert("exit".to_string(), Value::Builtin(BuiltinFunction::SysExit));
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
            module_data.globals.insert(
                "builtin_module_names".to_string(),
                self.heap.alloc_tuple(vec![
                    Value::Str("sys".to_string()),
                    Value::Str("builtins".to_string()),
                    Value::Str("_imp".to_string()),
                    Value::Str("_io".to_string()),
                    Value::Str("marshal".to_string()),
                    Value::Str("posix".to_string()),
                    Value::Str("errno".to_string()),
                ]),
            );
        }
        self.register_module("sys", sys_module);
        self.sync_sys_path_from_module_paths();
        self.refresh_sys_modules_dict();
    }

    pub fn set_sys_no_site_flag(&mut self, no_site: bool) {
        let Some(sys_module) = self.modules.get("sys").cloned() else {
            return;
        };
        let flags_module = match &*sys_module.kind() {
            Object::Module(module_data) => match module_data.globals.get("flags") {
                Some(Value::Module(flags)) => flags.clone(),
                _ => return,
            },
            _ => return,
        };
        if let Object::Module(flags_data) = &mut *flags_module.kind_mut() {
            flags_data.globals.insert(
                "no_site".to_string(),
                Value::Int(if no_site { 1 } else { 0 }),
            );
        }
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
                "__import__".to_string(),
                Value::Builtin(BuiltinFunction::Import),
            );
            module_data.globals.insert(
                "find_spec".to_string(),
                Value::Builtin(BuiltinFunction::FindSpec),
            );
            module_data.globals.insert(
                "invalidate_caches".to_string(),
                Value::Builtin(BuiltinFunction::ImportlibInvalidateCaches),
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
            module_data.globals.insert(
                "source_from_cache".to_string(),
                Value::Builtin(BuiltinFunction::ImportlibSourceFromCache),
            );
            module_data.globals.insert(
                "cache_from_source".to_string(),
                Value::Builtin(BuiltinFunction::ImportlibCacheFromSource),
            );
            module_data.globals.insert(
                "spec_from_file_location".to_string(),
                Value::Builtin(BuiltinFunction::ImportlibSpecFromFileLocation),
            );
        }
        self.register_module("importlib.util", util.clone());
        self.link_module_chain("importlib.util", util);

        let loader_basics_class = self
            .heap
            .alloc_class(ClassObject::new("_LoaderBasics".to_string(), Vec::new()));
        self.install_builtin_module(
            "_frozen_importlib_external",
            &[
                (
                    "_unpack_uint16",
                    BuiltinFunction::FrozenImportlibExternalUnpackUint16,
                ),
                (
                    "_unpack_uint32",
                    BuiltinFunction::FrozenImportlibExternalUnpackUint32,
                ),
                (
                    "_unpack_uint64",
                    BuiltinFunction::FrozenImportlibExternalUnpackUint64,
                ),
                (
                    "_path_stat",
                    BuiltinFunction::FrozenImportlibExternalPathStat,
                ),
                (
                    "_path_split",
                    BuiltinFunction::FrozenImportlibExternalPathSplit,
                ),
                (
                    "_path_join",
                    BuiltinFunction::FrozenImportlibExternalPathJoin,
                ),
            ],
            vec![
                (
                    "path_sep",
                    Value::Str(std::path::MAIN_SEPARATOR.to_string()),
                ),
                (
                    "path_separators",
                    Value::Str(if cfg!(windows) {
                        "\\/".to_string()
                    } else {
                        "/".to_string()
                    }),
                ),
                ("_LoaderBasics", loader_basics_class),
            ],
        );
        self.install_builtin_module(
            "_frozen_importlib",
            &[
                (
                    "spec_from_loader",
                    BuiltinFunction::FrozenImportlibSpecFromLoader,
                ),
                (
                    "_verbose_message",
                    BuiltinFunction::FrozenImportlibVerboseMessage,
                ),
            ],
            vec![(
                "ModuleSpec",
                self.heap
                    .alloc_class(ClassObject::new("ModuleSpec".to_string(), Vec::new())),
            )],
        );
        self.install_builtin_module(
            "_testinternalcapi",
            &[
                ("set_eval_frame_default", BuiltinFunction::NoOp),
                ("has_inline_values", BuiltinFunction::NoOp),
                (
                    "get_recursion_depth",
                    BuiltinFunction::TestInternalCapiGetRecursionDepth,
                ),
            ],
            Vec::new(),
        );
        self.install_builtin_module("_testlimitedcapi", &[], Vec::new());
        let meth_instance_class = match self
            .heap
            .alloc_class(ClassObject::new("MethInstance".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *meth_instance_class.kind_mut() {
            for method in [
                "meth_varargs",
                "meth_varargs_keywords",
                "meth_fastcall",
                "meth_fastcall_keywords",
                "meth_noargs",
                "meth_o",
            ] {
                class_data
                    .attrs
                    .insert(method.to_string(), Value::Builtin(BuiltinFunction::NoOp));
            }
        }
        let meth_class_class = match self
            .heap
            .alloc_class(ClassObject::new("MethClass".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *meth_class_class.kind_mut() {
            for method in [
                "meth_varargs",
                "meth_varargs_keywords",
                "meth_fastcall",
                "meth_fastcall_keywords",
                "meth_noargs",
                "meth_o",
            ] {
                class_data
                    .attrs
                    .insert(method.to_string(), Value::Builtin(BuiltinFunction::NoOp));
            }
        }
        let meth_static_class = match self
            .heap
            .alloc_class(ClassObject::new("MethStatic".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *meth_static_class.kind_mut() {
            for method in [
                "meth_varargs",
                "meth_varargs_keywords",
                "meth_fastcall",
                "meth_fastcall_keywords",
                "meth_noargs",
                "meth_o",
            ] {
                class_data
                    .attrs
                    .insert(method.to_string(), Value::Builtin(BuiltinFunction::NoOp));
            }
        }
        self.install_builtin_module(
            "_testcapi",
            &[
                ("meth_varargs", BuiltinFunction::NoOp),
                ("meth_varargs_keywords", BuiltinFunction::NoOp),
                ("meth_fastcall", BuiltinFunction::NoOp),
                ("meth_fastcall_keywords", BuiltinFunction::NoOp),
                ("meth_noargs", BuiltinFunction::NoOp),
                ("meth_o", BuiltinFunction::NoOp),
            ],
            vec![
                ("INT_MAX", Value::Int(i32::MAX as i64)),
                ("INT_MIN", Value::Int(i32::MIN as i64)),
                ("PY_SSIZE_T_MAX", Value::Int(i64::MAX)),
                ("PY_SSIZE_T_MIN", Value::Int(i64::MIN)),
                ("MethInstance", Value::Class(meth_instance_class)),
                ("MethClass", Value::Class(meth_class_class)),
                ("MethStatic", Value::Class(meth_static_class)),
            ],
        );
    }

    fn install_random_module(&mut self) {
        let random_module = match self.heap.alloc_module(ModuleObject::new("random")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        let random_class = match self
            .heap
            .alloc_class(ClassObject::new("Random".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *random_class.kind_mut() {
            class_data.attrs.insert(
                "seed".to_string(),
                Value::Builtin(BuiltinFunction::RandomSeed),
            );
            class_data.attrs.insert(
                "random".to_string(),
                Value::Builtin(BuiltinFunction::RandomRandom),
            );
            class_data.attrs.insert(
                "randrange".to_string(),
                Value::Builtin(BuiltinFunction::RandomRandRange),
            );
            class_data.attrs.insert(
                "randint".to_string(),
                Value::Builtin(BuiltinFunction::RandomRandInt),
            );
            class_data.attrs.insert(
                "getrandbits".to_string(),
                Value::Builtin(BuiltinFunction::RandomGetRandBits),
            );
            class_data.attrs.insert(
                "choice".to_string(),
                Value::Builtin(BuiltinFunction::RandomChoice),
            );
            class_data.attrs.insert(
                "choices".to_string(),
                Value::Builtin(BuiltinFunction::RandomChoices),
            );
            class_data.attrs.insert(
                "shuffle".to_string(),
                Value::Builtin(BuiltinFunction::RandomShuffle),
            );
        }
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
                "choices".to_string(),
                Value::Builtin(BuiltinFunction::RandomChoices),
            );
            module_data.globals.insert(
                "shuffle".to_string(),
                Value::Builtin(BuiltinFunction::RandomShuffle),
            );
            module_data
                .globals
                .insert("Random".to_string(), Value::Class(random_class.clone()));
            module_data
                .globals
                .insert("SystemRandom".to_string(), Value::Class(random_class));
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

    fn install_builtins_module(&mut self) {
        let module = match self.heap.alloc_module(ModuleObject::new("builtins")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &module,
            "builtins",
            None,
            Some(BUILTIN_MODULE_LOADER),
            false,
            Vec::new(),
            false,
        );
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            for (name, value) in self.builtins.iter() {
                module_data.globals.insert(name.clone(), value.clone());
            }
        }
        self.register_module("builtins", module.clone());
        if let Object::Module(main_module) = &mut *self.main_module.kind_mut() {
            main_module
                .globals
                .insert("__builtins__".to_string(), Value::Module(module));
        }
    }
}

fn frame_cell_value(frame: &Frame, name: &str) -> Option<Value> {
    let cellvar_len = frame.code.cellvars.len();
    for (index, cell_name) in frame.code.cellvars.iter().enumerate() {
        if cell_name == name {
            if let Some(cell) = frame.cells.get(index) {
                if let Object::Cell(cell_data) = &*cell.kind() {
                    return cell_data.value.clone();
                }
            }
            return None;
        }
    }
    for (offset, free_name) in frame.code.freevars.iter().enumerate() {
        if free_name == name {
            if let Some(cell) = frame.cells.get(cellvar_len + offset) {
                if let Object::Cell(cell_data) = &*cell.kind() {
                    return cell_data.value.clone();
                }
            }
            return None;
        }
    }
    None
}

fn weakref_target_id(target: &Value) -> Option<u64> {
    match target {
        Value::List(obj)
        | Value::Tuple(obj)
        | Value::Dict(obj)
        | Value::DictKeys(obj)
        | Value::Set(obj)
        | Value::FrozenSet(obj)
        | Value::Bytes(obj)
        | Value::ByteArray(obj)
        | Value::MemoryView(obj)
        | Value::Iterator(obj)
        | Value::Generator(obj)
        | Value::Module(obj)
        | Value::Class(obj)
        | Value::Instance(obj)
        | Value::Super(obj)
        | Value::BoundMethod(obj)
        | Value::Function(obj)
        | Value::Cell(obj) => Some(obj.id()),
        _ => None,
    }
}

fn weakref_target_object(target: &Value) -> Option<ObjRef> {
    match target {
        Value::List(obj)
        | Value::Tuple(obj)
        | Value::Dict(obj)
        | Value::DictKeys(obj)
        | Value::Set(obj)
        | Value::FrozenSet(obj)
        | Value::Bytes(obj)
        | Value::ByteArray(obj)
        | Value::MemoryView(obj)
        | Value::Iterator(obj)
        | Value::Generator(obj)
        | Value::Module(obj)
        | Value::Class(obj)
        | Value::Instance(obj)
        | Value::Super(obj)
        | Value::BoundMethod(obj)
        | Value::Function(obj)
        | Value::Cell(obj) => Some(obj.clone()),
        _ => None,
    }
}

fn value_from_object_ref(obj: ObjRef) -> Option<Value> {
    match &*obj.kind() {
        Object::List(_) => Some(Value::List(obj.clone())),
        Object::Tuple(_) => Some(Value::Tuple(obj.clone())),
        Object::Dict(_) => Some(Value::Dict(obj.clone())),
        Object::DictKeysView(_) => Some(Value::DictKeys(obj.clone())),
        Object::Set(_) => Some(Value::Set(obj.clone())),
        Object::FrozenSet(_) => Some(Value::FrozenSet(obj.clone())),
        Object::Bytes(_) => Some(Value::Bytes(obj.clone())),
        Object::ByteArray(_) => Some(Value::ByteArray(obj.clone())),
        Object::MemoryView(_) => Some(Value::MemoryView(obj.clone())),
        Object::Iterator(_) => Some(Value::Iterator(obj.clone())),
        Object::Generator(_) => Some(Value::Generator(obj.clone())),
        Object::Module(_) => Some(Value::Module(obj.clone())),
        Object::Class(_) => Some(Value::Class(obj.clone())),
        Object::Instance(_) => Some(Value::Instance(obj.clone())),
        Object::Super(_) => Some(Value::Super(obj.clone())),
        Object::BoundMethod(_) => Some(Value::BoundMethod(obj.clone())),
        Object::Function(_) => Some(Value::Function(obj.clone())),
        Object::Cell(_) => Some(Value::Cell(obj.clone())),
        Object::NativeMethod(_) => None,
    }
}

fn value_to_int(value: Value) -> Result<i64, RuntimeError> {
    match value {
        Value::Int(value) => Ok(value),
        Value::Bool(value) => Ok(if value { 1 } else { 0 }),
        Value::BigInt(value) => value
            .to_i64()
            .ok_or_else(|| RuntimeError::new("integer overflow")),
        _ => Err(RuntimeError::new("unsupported operand type")),
    }
}

fn value_to_bigint(value: Value) -> Result<BigInt, RuntimeError> {
    match value {
        Value::Int(value) => Ok(BigInt::from_i64(value)),
        Value::Bool(value) => Ok(BigInt::from_i64(if value { 1 } else { 0 })),
        Value::BigInt(value) => Ok(*value),
        _ => Err(RuntimeError::new("range() expects integers")),
    }
}

fn value_from_bigint(value: BigInt) -> Value {
    match value.to_i64() {
        Some(number) => Value::Int(number),
        None => Value::BigInt(Box::new(value)),
    }
}

fn round_float_with_ndigits(value: f64, ndigits: i64) -> f64 {
    if !value.is_finite() {
        return value;
    }

    if ndigits >= 0 {
        if ndigits > 308 {
            return value;
        }
        let factor = 10_f64.powi(ndigits as i32);
        if !factor.is_finite() || factor == 0.0 {
            return value;
        }
        let rounded = (value * factor).round_ties_even() / factor;
        if rounded == 0.0 {
            0.0f64.copysign(value)
        } else {
            rounded
        }
    } else {
        let shift = (-ndigits) as i64;
        if shift > 308 {
            return 0.0f64.copysign(value);
        }
        let factor = 10_f64.powi(shift as i32);
        if !factor.is_finite() || factor == 0.0 {
            return 0.0f64.copysign(value);
        }
        let rounded = (value / factor).round_ties_even() * factor;
        if rounded == 0.0 {
            0.0f64.copysign(value)
        } else {
            rounded
        }
    }
}

fn bigint_from_bytes(bytes: &[u8], little_endian: bool, signed: bool) -> BigInt {
    if bytes.is_empty() {
        return BigInt::zero();
    }

    let mut value = BigInt::zero();
    if little_endian {
        for byte in bytes.iter().rev() {
            value = value.mul_small(256);
            value = value.add_small(*byte as u32);
        }
    } else {
        for byte in bytes {
            value = value.mul_small(256);
            value = value.add_small(*byte as u32);
        }
    }

    let sign_bit_set = if little_endian {
        bytes.last().is_some_and(|byte| (byte & 0x80) != 0)
    } else {
        bytes.first().is_some_and(|byte| (byte & 0x80) != 0)
    };
    if signed && sign_bit_set {
        let modulus = BigInt::one().shl_bits(bytes.len() * 8);
        value = value.sub(&modulus);
    }
    value
}

fn bigint_to_unsigned_le_bytes(value: &BigInt) -> Vec<u8> {
    value.to_abs_le_bytes()
}

fn bigint_to_fixed_bytes(
    value: &BigInt,
    length: usize,
    little_endian: bool,
    signed: bool,
) -> Result<Vec<u8>, RuntimeError> {
    if length == 0 {
        if value.is_zero() {
            return Ok(Vec::new());
        }
        return Err(RuntimeError::new("int too big to convert"));
    }

    let bits = length
        .checked_mul(8)
        .ok_or_else(|| RuntimeError::new("int too big to convert"))?;
    let modulus = BigInt::one().shl_bits(bits);
    let unsigned_value = if signed {
        let signed_limit = BigInt::one().shl_bits(bits - 1);
        let signed_min = signed_limit.negated();
        let signed_max = signed_limit.sub(&BigInt::one());
        if value.cmp_total(&signed_min) == Ordering::Less
            || value.cmp_total(&signed_max) == Ordering::Greater
        {
            return Err(RuntimeError::new("int too big to convert"));
        }
        if value.is_negative() {
            modulus.add(value)
        } else {
            value.clone()
        }
    } else {
        if value.is_negative() {
            return Err(RuntimeError::new("can't convert negative int to unsigned"));
        }
        value.clone()
    };

    let mut bytes = bigint_to_unsigned_le_bytes(&unsigned_value);
    if bytes.len() > length {
        return Err(RuntimeError::new("int too big to convert"));
    }
    bytes.resize(length, 0);
    if !little_endian {
        bytes.reverse();
    }
    Ok(bytes)
}

fn parse_decimal_bigint_literal(text: &str) -> Result<BigInt, RuntimeError> {
    let cleaned = text.trim_end();
    if cleaned.is_empty() {
        return Err(RuntimeError::new("invalid literal for int() with base 10"));
    }
    let (negative, digits) = if let Some(rest) = cleaned.strip_prefix('+') {
        (false, rest)
    } else if let Some(rest) = cleaned.strip_prefix('-') {
        (true, rest)
    } else {
        (false, cleaned)
    };
    let normalized = normalize_decimal_int_digits(digits)
        .ok_or_else(|| RuntimeError::new("invalid literal for int() with base 10"))?;
    let mut value = BigInt::from_str_radix(&normalized, 10)
        .ok_or_else(|| RuntimeError::new("invalid literal for int() with base 10"))?;
    if negative {
        value = value.negated();
    }
    Ok(value)
}

fn normalize_decimal_int_digits(digits: &str) -> Option<String> {
    if digits.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(digits.len());
    let mut saw_digit = false;
    let mut prev_underscore = false;
    for ch in digits.chars() {
        if ch == '_' {
            if !saw_digit || prev_underscore {
                return None;
            }
            prev_underscore = true;
            continue;
        }
        if !ch.is_ascii_digit() {
            return None;
        }
        saw_digit = true;
        prev_underscore = false;
        out.push(ch);
    }
    if !saw_digit || prev_underscore {
        return None;
    }
    Some(out)
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
        Value::BigInt(value) => match value.to_i64() {
            Some(number) => Ok(number as u64),
            None => {
                let text = value.to_string();
                let mut hash: u64 = 1469598103934665603;
                for byte in text.as_bytes() {
                    hash ^= *byte as u64;
                    hash = hash.wrapping_mul(1099511628211);
                }
                Ok(hash)
            }
        },
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
        Value::BigInt(value) => match value.to_i64() {
            Some(number) => NumericValue::Int(number),
            None => NumericValue::Float(value.to_f64()),
        },
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
        Value::BigInt(value) => match value.to_i64() {
            Some(number) => NumericValue::Int(number),
            None => NumericValue::Float(value.to_f64()),
        },
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

fn numeric_as_complex(value: &Value) -> Option<(f64, f64)> {
    match value {
        Value::Int(value) => Some((*value as f64, 0.0)),
        Value::Bool(value) => Some((if *value { 1.0 } else { 0.0 }, 0.0)),
        Value::BigInt(value) => Some((value.to_f64(), 0.0)),
        Value::Float(value) => Some((*value, 0.0)),
        Value::Complex { real, imag } => Some((*real, *imag)),
        _ => None,
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

fn erfc_approx(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x == f64::INFINITY {
        return 0.0;
    }
    if x == f64::NEG_INFINITY {
        return 2.0;
    }
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let ans = t * poly.exp();
    if x >= 0.0 { ans } else { 2.0 - ans }
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
        Value::BigInt(value) => Ok(value.to_f64()),
        Value::Bool(value) => Ok(if value { 1.0 } else { 0.0 }),
        Value::Complex { real, imag } if imag == 0.0 => Ok(real),
        Value::Str(value) => value
            .trim()
            .parse::<f64>()
            .map_err(|_| RuntimeError::new("expected numeric value")),
        _ => Err(RuntimeError::new("expected numeric value")),
    }
}

fn parse_hex_float_literal(text: &str) -> Result<f64, RuntimeError> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("inf") || trimmed.eq_ignore_ascii_case("+inf") {
        return Ok(f64::INFINITY);
    }
    if trimmed.eq_ignore_ascii_case("-inf") {
        return Ok(f64::NEG_INFINITY);
    }
    if trimmed.eq_ignore_ascii_case("nan")
        || trimmed.eq_ignore_ascii_case("+nan")
        || trimmed.eq_ignore_ascii_case("-nan")
    {
        return Ok(f64::NAN);
    }

    let (sign, rest) = if let Some(stripped) = trimmed.strip_prefix('-') {
        (-1.0, stripped)
    } else if let Some(stripped) = trimmed.strip_prefix('+') {
        (1.0, stripped)
    } else {
        (1.0, trimmed)
    };

    let Some(rest) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) else {
        return Err(RuntimeError::new(
            "fromhex() argument must be a hexadecimal string",
        ));
    };
    let Some((mantissa_text, exponent_text)) = rest.split_once(['p', 'P']) else {
        return Err(RuntimeError::new(
            "fromhex() argument must be a hexadecimal string",
        ));
    };
    if mantissa_text.is_empty() || exponent_text.is_empty() {
        return Err(RuntimeError::new(
            "fromhex() argument must be a hexadecimal string",
        ));
    }
    let exponent = exponent_text
        .parse::<i32>()
        .map_err(|_| RuntimeError::new("fromhex() argument must be a hexadecimal string"))?;

    let (whole_text, frac_text) = if let Some((left, right)) = mantissa_text.split_once('.') {
        (left, right)
    } else {
        (mantissa_text, "")
    };
    if whole_text.is_empty() && frac_text.is_empty() {
        return Err(RuntimeError::new(
            "fromhex() argument must be a hexadecimal string",
        ));
    }

    let mut value = 0.0;
    for ch in whole_text.chars() {
        let digit = ch
            .to_digit(16)
            .ok_or_else(|| RuntimeError::new("fromhex() argument must be a hexadecimal string"))?;
        value = value * 16.0 + digit as f64;
    }
    let mut factor = 1.0 / 16.0;
    for ch in frac_text.chars() {
        let digit = ch
            .to_digit(16)
            .ok_or_else(|| RuntimeError::new("fromhex() argument must be a hexadecimal string"))?;
        value += (digit as f64) * factor;
        factor /= 16.0;
    }

    Ok(sign * value * 2.0_f64.powi(exponent))
}

fn format_float_hex(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_string()
        } else {
            "inf".to_string()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0x0.0p+0".to_string()
        } else {
            "0x0.0p+0".to_string()
        };
    }

    let sign = if value.is_sign_negative() { "-" } else { "" };
    let mut normalized = value.abs();
    let mut exponent = 0_i32;
    while normalized >= 2.0 {
        normalized /= 2.0;
        exponent += 1;
    }
    while normalized < 1.0 {
        normalized *= 2.0;
        exponent -= 1;
    }

    let mut fraction = normalized - 1.0;
    let mut digits = String::with_capacity(13);
    for _ in 0..13 {
        fraction *= 16.0;
        let digit = fraction.floor() as u32;
        let ch = char::from_digit(digit.min(15), 16).unwrap_or('0');
        digits.push(ch);
        fraction -= digit as f64;
    }
    while digits.ends_with('0') {
        digits.pop();
    }
    if digits.is_empty() {
        digits.push('0');
    }
    format!("{sign}0x1.{digits}p{exponent:+}")
}

fn value_to_path(value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::Str(path) => Ok(path.clone()),
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(bytes) => Ok(String::from_utf8_lossy(bytes).into_owned()),
            _ => Err(RuntimeError::new("path must be string or bytes")),
        },
        _ => Err(RuntimeError::new("path must be string or bytes")),
    }
}

fn value_to_process_text(value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::Str(text) => Ok(text.clone()),
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(bytes) => Ok(String::from_utf8_lossy(bytes).into_owned()),
            _ => Err(RuntimeError::new("process argument must be str or bytes")),
        },
        _ => Err(RuntimeError::new("process argument must be str or bytes")),
    }
}

fn value_to_sequence_items(value: &Value) -> Result<Vec<Value>, RuntimeError> {
    match value {
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => Ok(values.clone()),
            _ => Err(RuntimeError::new("expected tuple")),
        },
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => Ok(values.clone()),
            _ => Err(RuntimeError::new("expected list")),
        },
        _ => Err(RuntimeError::new("expected tuple or list")),
    }
}

fn collect_process_argv(value: &Value) -> Result<Vec<String>, RuntimeError> {
    let items = value_to_sequence_items(value)?;
    let mut argv = Vec::with_capacity(items.len());
    for item in &items {
        argv.push(value_to_process_text(item)?);
    }
    Ok(argv)
}

fn collect_env_entries(value: &Value) -> Result<Vec<(String, String)>, RuntimeError> {
    let items = value_to_sequence_items(value)?;
    let mut out = Vec::with_capacity(items.len());
    for item in &items {
        let text = value_to_process_text(item)?;
        let Some((key, value)) = text.split_once('=') else {
            return Err(RuntimeError::new("invalid env entry"));
        };
        out.push((key.to_string(), value.to_string()));
    }
    Ok(out)
}

fn is_pyrs_executable(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with("pyrs"))
        .unwrap_or(false)
}

fn parse_modules_to_block_literal(code: &str) -> Vec<String> {
    let marker = "modules_to_block = frozenset({";
    let Some(start) = code.find(marker) else {
        return Vec::new();
    };
    let rest = &code[start + marker.len()..];
    let Some(end) = rest.find("})") else {
        return Vec::new();
    };
    rest[..end]
        .split(',')
        .filter_map(|entry| {
            let item = entry.trim().trim_matches('\'').trim_matches('"');
            if item.is_empty() {
                None
            } else {
                Some(item.to_string())
            }
        })
        .collect()
}

fn system_time_to_secs_f64(value: SystemTime) -> Option<f64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs_f64())
}

fn seconds_to_system_time(value: f64) -> Result<SystemTime, RuntimeError> {
    if !value.is_finite() || value.is_sign_negative() {
        return Err(RuntimeError::new(
            "timestamp must be a non-negative finite number",
        ));
    }
    Ok(UNIX_EPOCH + Duration::from_secs_f64(value))
}

fn source_path_from_cache_path(path: &str) -> String {
    if !path.ends_with(".pyc") {
        return path.to_string();
    }
    let cache_path = Path::new(path);
    let Some(file_name) = cache_path.file_name().and_then(|name| name.to_str()) else {
        return path.trim_end_matches('c').to_string();
    };
    let module_stem = file_name
        .trim_end_matches(".pyc")
        .split('.')
        .next()
        .unwrap_or(file_name.trim_end_matches(".pyc"));
    if cache_path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some("__pycache__")
    {
        if let Some(parent) = cache_path.parent().and_then(|parent| parent.parent()) {
            return parent
                .join(format!("{module_stem}.py"))
                .to_string_lossy()
                .to_string();
        }
    }
    path.trim_end_matches('c').to_string()
}

fn cache_path_from_source_path(path: &str) -> String {
    let source_path = Path::new(path);
    let stem = source_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("module");
    let pycache = source_path
        .parent()
        .map(|parent| parent.join("__pycache__"))
        .unwrap_or_else(|| PathBuf::from("__pycache__"));
    pycache
        .join(format!("{stem}.cpython-314.pyc"))
        .to_string_lossy()
        .to_string()
}

fn cached_module_path(root: &Path, rel_name: &str) -> PathBuf {
    let rel_path = Path::new(rel_name);
    let parent = rel_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = rel_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("module");
    root.join(parent)
        .join("__pycache__")
        .join(format!("{stem}.cpython-314.pyc"))
}

fn uuid_random_bytes(rng: &mut Mt19937) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..4].copy_from_slice(&rng.next_u32().to_be_bytes());
    bytes[4..8].copy_from_slice(&rng.next_u32().to_be_bytes());
    bytes[8..12].copy_from_slice(&rng.next_u32().to_be_bytes());
    bytes[12..].copy_from_slice(&rng.next_u32().to_be_bytes());
    bytes
}

fn apply_uuid_variant(bytes: &mut [u8; 16]) {
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
}

fn apply_uuid_version(bytes: &mut [u8; 16], version: u8) {
    bytes[6] = (bytes[6] & 0x0f) | ((version & 0x0f) << 4);
    apply_uuid_variant(bytes);
}

fn format_uuid_hex(bytes: [u8; 16]) -> String {
    bytes.iter().map(|value| format!("{value:02x}")).collect()
}

fn format_uuid_hyphenated(bytes: [u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn parse_uuid_like_string(text: &str) -> Result<[u8; 16], RuntimeError> {
    let trimmed = text.trim();
    let without_urn = trimmed
        .strip_prefix("urn:uuid:")
        .or_else(|| trimmed.strip_prefix("URN:UUID:"))
        .unwrap_or(trimmed);
    let without_braces = without_urn
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(without_urn);
    let compact: String = without_braces
        .chars()
        .filter(|ch| *ch != '-')
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    if compact.len() != 32 || !compact.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(RuntimeError::new("invalid UUID string"));
    }
    let mut out = [0u8; 16];
    for (index, slot) in out.iter_mut().enumerate() {
        let pair = &compact[index * 2..index * 2 + 2];
        *slot = u8::from_str_radix(pair, 16).map_err(|_| RuntimeError::new("invalid UUID"))?;
    }
    Ok(out)
}

fn uuid_hash_mix_bytes(tag: u8, namespace: [u8; 16], name: &[u8]) -> [u8; 16] {
    let mut first = DefaultHasher::new();
    tag.hash(&mut first);
    namespace.hash(&mut first);
    name.hash(&mut first);
    let high = first.finish();

    let mut second = DefaultHasher::new();
    (tag.wrapping_add(0x9d)).hash(&mut second);
    namespace.hash(&mut second);
    high.hash(&mut second);
    name.hash(&mut second);
    let low = second.finish();

    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&high.to_be_bytes());
    bytes[8..].copy_from_slice(&low.to_be_bytes());
    bytes
}

fn uuid_node_from_hostname() -> i64 {
    let mut hasher = DefaultHasher::new();
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
    host.hash(&mut hasher);
    let mut node = hasher.finish() & 0x0000_FFFF_FFFF_FFFF;
    node |= 0x0000_0100_0000_0000;
    node as i64
}

fn uuid_timestamp_100ns_since_gregorian() -> Result<u64, RuntimeError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RuntimeError::new("system time before epoch"))?;
    let ticks = now
        .as_secs()
        .saturating_mul(10_000_000)
        .saturating_add((now.subsec_nanos() / 100) as u64);
    Ok(ticks.saturating_add(0x01B2_1DD2_1381_4000))
}

#[derive(Debug, Clone)]
enum FormatterFieldKey {
    Int(i64),
    Str(String),
}

fn parse_string_formatter(
    text: &str,
) -> Result<Vec<(String, Option<String>, Option<String>, Option<String>)>, RuntimeError> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut literal = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        if ch == '{' {
            if chars.get(index + 1) == Some(&'{') {
                literal.push('{');
                index += 2;
                continue;
            }

            let field_start = index + 1;
            let mut field_end = field_start;
            let mut nested = 0usize;
            let mut closed = false;
            while field_end < chars.len() {
                match chars[field_end] {
                    '{' => nested += 1,
                    '}' => {
                        if nested == 0 {
                            closed = true;
                            break;
                        }
                        nested -= 1;
                    }
                    _ => {}
                }
                field_end += 1;
            }
            if !closed {
                return Err(RuntimeError::new("Single '{' encountered in format string"));
            }

            let field_text: String = chars[field_start..field_end].iter().collect();
            let (field_name, format_spec, conversion) = split_formatter_field(&field_text);
            out.push((literal, Some(field_name), Some(format_spec), conversion));
            literal = String::new();
            index = field_end + 1;
            continue;
        }
        if ch == '}' {
            if chars.get(index + 1) == Some(&'}') {
                literal.push('}');
                index += 2;
                continue;
            }
            return Err(RuntimeError::new("Single '}' encountered in format string"));
        }
        literal.push(ch);
        index += 1;
    }

    out.push((literal, None, None, None));
    Ok(out)
}

fn split_formatter_field(text: &str) -> (String, String, Option<String>) {
    let chars: Vec<char> = text.chars().collect();
    let mut bracket_depth = 0usize;
    let mut conversion_index: Option<usize> = None;
    let mut format_index: Option<usize> = None;

    for (idx, ch) in chars.iter().enumerate() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
            }
            '!' if bracket_depth == 0 && conversion_index.is_none() && format_index.is_none() => {
                conversion_index = Some(idx);
            }
            ':' if bracket_depth == 0 && format_index.is_none() => {
                format_index = Some(idx);
            }
            _ => {}
        }
    }

    let field_name_end = conversion_index.or(format_index).unwrap_or(chars.len());
    let field_name: String = chars[..field_name_end].iter().collect();
    let format_spec = match format_index {
        Some(idx) => chars[idx + 1..].iter().collect::<String>(),
        None => String::new(),
    };
    let conversion = conversion_index.map(|idx| {
        let end = format_index.unwrap_or(chars.len());
        chars[idx + 1..end].iter().collect::<String>()
    });
    (field_name, format_spec, conversion)
}

fn split_formatter_field_name(
    field_name: &str,
) -> Result<(FormatterFieldKey, Vec<(bool, FormatterFieldKey)>), RuntimeError> {
    let chars: Vec<char> = field_name.chars().collect();
    let mut index = 0usize;
    while index < chars.len() && chars[index] != '.' && chars[index] != '[' {
        index += 1;
    }

    let first_text: String = chars[..index].iter().collect();
    let first = parse_formatter_key(&first_text);
    let mut rest = Vec::new();

    while index < chars.len() {
        match chars[index] {
            '.' => {
                index += 1;
                let start = index;
                while index < chars.len() && chars[index] != '.' && chars[index] != '[' {
                    index += 1;
                }
                let attr: String = chars[start..index].iter().collect();
                rest.push((true, FormatterFieldKey::Str(attr)));
            }
            '[' => {
                index += 1;
                let start = index;
                while index < chars.len() && chars[index] != ']' {
                    index += 1;
                }
                if index >= chars.len() {
                    return Err(RuntimeError::new("malformed field name"));
                }
                let key_text: String = chars[start..index].iter().collect();
                rest.push((false, parse_formatter_key(&key_text)));
                index += 1;
            }
            _ => break,
        }
    }

    Ok((first, rest))
}

fn parse_formatter_key(text: &str) -> FormatterFieldKey {
    if !text.is_empty() && text.chars().all(|ch| ch.is_ascii_digit()) {
        if let Ok(value) = text.parse::<i64>() {
            return FormatterFieldKey::Int(value);
        }
    }
    FormatterFieldKey::Str(text.to_string())
}

fn with_bytes_like_source<R>(source: &ObjRef, map: impl FnOnce(&[u8]) -> R) -> Option<R> {
    match &*source.kind() {
        Object::Bytes(values) | Object::ByteArray(values) => Some(map(values)),
        Object::Instance(instance_data) => {
            match instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR) {
                Some(Value::Bytes(storage)) => match &*storage.kind() {
                    Object::Bytes(values) => Some(map(values)),
                    _ => None,
                },
                Some(Value::ByteArray(storage)) => match &*storage.kind() {
                    Object::ByteArray(values) => Some(map(values)),
                    _ => None,
                },
                _ => None,
            }
        }
        Object::Module(module_data) if module_data.name == "__array__" => {
            let values = module_data.globals.get("values")?;
            let Value::List(values_obj) = values else {
                return None;
            };
            let Object::List(items) = &*values_obj.kind() else {
                return None;
            };
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let value = value_to_int(item.clone()).ok()?;
                if !(0..=255).contains(&value) {
                    return None;
                }
                out.push(value as u8);
            }
            Some(map(&out))
        }
        _ => None,
    }
}

fn memoryview_bounds(start: usize, length: Option<usize>, source_len: usize) -> (usize, usize) {
    let start = start.min(source_len);
    let end = match length {
        Some(length) => start.saturating_add(length).min(source_len),
        None => source_len,
    };
    (start, end)
}

fn bytes_like_source_is_readonly(source: &ObjRef) -> Option<bool> {
    match &*source.kind() {
        Object::Bytes(_) => Some(true),
        Object::ByteArray(_) => Some(false),
        Object::Instance(instance_data) => {
            match instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR) {
                Some(Value::Bytes(_)) => Some(true),
                Some(Value::ByteArray(_)) => Some(false),
                _ => None,
            }
        }
        Object::Module(module_data) if module_data.name == "__array__" => Some(false),
        _ => None,
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
            Object::MemoryView(view) => with_bytes_like_source(&view.source, |values| {
                let (start, end) = memoryview_bounds(view.start, view.length, values.len());
                values[start..end].to_vec()
            })
            .ok_or_else(|| RuntimeError::new("expected bytes-like payload")),
            _ => Err(RuntimeError::new("expected bytes-like payload")),
        },
        Value::Module(obj) => match &*obj.kind() {
            Object::Module(module_data) if module_data.name == "__array__" => {
                match module_data.globals.get("values") {
                    Some(values) => value_to_bytes_payload(values.clone()),
                    None => Err(RuntimeError::new("expected bytes-like payload")),
                }
            }
            _ => Err(RuntimeError::new("expected bytes-like payload")),
        },
        Value::Instance(obj) => match &*obj.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR) {
                    Some(Value::Bytes(storage)) => match &*storage.kind() {
                        Object::Bytes(values) => Ok(values.clone()),
                        _ => Err(RuntimeError::new("expected bytes-like payload")),
                    },
                    Some(Value::ByteArray(storage)) => match &*storage.kind() {
                        Object::ByteArray(values) => Ok(values.clone()),
                        _ => Err(RuntimeError::new("expected bytes-like payload")),
                    },
                    _ => Err(RuntimeError::new("expected bytes-like payload")),
                }
            }
            _ => Err(RuntimeError::new("expected bytes-like payload")),
        },
        Value::Iterator(obj) => {
            let mut obj_kind = obj.kind_mut();
            let Object::Iterator(iterator) = &mut *obj_kind else {
                return Err(RuntimeError::new("expected bytes-like payload"));
            };
            let values = match &mut iterator.kind {
                IteratorKind::List(list_obj) => match &*list_obj.kind() {
                    Object::List(items) => {
                        let start = iterator.index.min(items.len());
                        let out = items[start..].to_vec();
                        iterator.index = items.len();
                        out
                    }
                    _ => return Err(RuntimeError::new("expected bytes-like payload")),
                },
                IteratorKind::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                    Object::Tuple(items) => {
                        let start = iterator.index.min(items.len());
                        let out = items[start..].to_vec();
                        iterator.index = items.len();
                        out
                    }
                    _ => return Err(RuntimeError::new("expected bytes-like payload")),
                },
                IteratorKind::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                    Object::Bytes(items) => {
                        let start = iterator.index.min(items.len());
                        let out = items[start..]
                            .iter()
                            .map(|byte| Value::Int(*byte as i64))
                            .collect::<Vec<_>>();
                        iterator.index = items.len();
                        out
                    }
                    _ => return Err(RuntimeError::new("expected bytes-like payload")),
                },
                IteratorKind::ByteArray(bytearray_obj) => match &*bytearray_obj.kind() {
                    Object::ByteArray(items) => {
                        let start = iterator.index.min(items.len());
                        let out = items[start..]
                            .iter()
                            .map(|byte| Value::Int(*byte as i64))
                            .collect::<Vec<_>>();
                        iterator.index = items.len();
                        out
                    }
                    _ => return Err(RuntimeError::new("expected bytes-like payload")),
                },
                IteratorKind::MemoryView(memory_obj) => match &*memory_obj.kind() {
                    Object::MemoryView(view) => with_bytes_like_source(&view.source, |items| {
                        let (view_start, view_end) =
                            memoryview_bounds(view.start, view.length, items.len());
                        let view_len = view_end.saturating_sub(view_start);
                        let start = iterator.index.min(view_len);
                        let out = items[view_start + start..view_end]
                            .iter()
                            .map(|byte| Value::Int(*byte as i64))
                            .collect::<Vec<_>>();
                        iterator.index = view_len;
                        out
                    })
                    .ok_or_else(|| RuntimeError::new("expected bytes-like payload"))?,
                    _ => return Err(RuntimeError::new("expected bytes-like payload")),
                },
                IteratorKind::Map { values, .. } => {
                    let start = iterator.index.min(values.len());
                    let out = values[start..].to_vec();
                    iterator.index = values.len();
                    out
                }
                IteratorKind::Range {
                    current,
                    stop,
                    step,
                } => {
                    if step.is_zero() {
                        return Err(RuntimeError::new("range() arg 3 must not be zero"));
                    }
                    let mut out = Vec::new();
                    let mut cursor = current.clone();
                    if !step.is_negative() {
                        while cursor.cmp_total(stop) == Ordering::Less {
                            out.push(value_from_bigint(cursor.clone()));
                            cursor = cursor.add(step);
                        }
                    } else {
                        while cursor.cmp_total(stop) == Ordering::Greater {
                            out.push(value_from_bigint(cursor.clone()));
                            cursor = cursor.add(step);
                        }
                    }
                    *current = cursor;
                    iterator.index = iterator.index.saturating_add(out.len());
                    out
                }
                IteratorKind::RangeObject { start, stop, step } => {
                    if step.is_zero() {
                        return Err(RuntimeError::new("range() arg 3 must not be zero"));
                    }
                    let mut cursor = start.clone();
                    for _ in 0..iterator.index {
                        cursor = cursor.add(step);
                    }
                    let mut out = Vec::new();
                    if !step.is_negative() {
                        while cursor.cmp_total(stop) == Ordering::Less {
                            out.push(value_from_bigint(cursor.clone()));
                            cursor = cursor.add(step);
                        }
                    } else {
                        while cursor.cmp_total(stop) == Ordering::Greater {
                            out.push(value_from_bigint(cursor.clone()));
                            cursor = cursor.add(step);
                        }
                    }
                    iterator.index = iterator.index.saturating_add(out.len());
                    out
                }
                IteratorKind::Dict(dict_obj) => match &*dict_obj.kind() {
                    Object::Dict(items) => {
                        let start = iterator.index.min(items.len());
                        let out = items
                            .iter()
                            .skip(start)
                            .map(|(key, _)| key.clone())
                            .collect::<Vec<_>>();
                        iterator.index = items.len();
                        out
                    }
                    _ => return Err(RuntimeError::new("expected bytes-like payload")),
                },
                IteratorKind::Set(set_obj) => match &*set_obj.kind() {
                    Object::Set(items) | Object::FrozenSet(items) => {
                        let all = items.to_vec();
                        let start = iterator.index.min(all.len());
                        let out = all.into_iter().skip(start).collect::<Vec<_>>();
                        iterator.index = start.saturating_add(out.len());
                        out
                    }
                    _ => return Err(RuntimeError::new("expected bytes-like payload")),
                },
                IteratorKind::Str(text) => {
                    let chars = text.chars().collect::<Vec<_>>();
                    let start = iterator.index.min(chars.len());
                    let out = chars[start..]
                        .iter()
                        .map(|ch| Value::Str(ch.to_string()))
                        .collect::<Vec<_>>();
                    iterator.index = chars.len();
                    out
                }
                IteratorKind::SequenceGetItem { .. } => {
                    return Err(RuntimeError::new("expected bytes-like payload"));
                }
                IteratorKind::Count { .. } | IteratorKind::Cycle { .. } => {
                    return Err(RuntimeError::new("expected bytes-like payload"));
                }
            };
            let mut out = Vec::with_capacity(values.len());
            for value in values {
                let byte = value_to_int(value)?;
                if !(0..=255).contains(&byte) {
                    return Err(RuntimeError::new("byte must be in range(0, 256)"));
                }
                out.push(byte as u8);
            }
            Ok(out)
        }
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

fn re_pattern_from_value(value: &Value) -> Result<RePatternValue, RuntimeError> {
    match value {
        Value::Str(text) => Ok(RePatternValue::Str(text.clone())),
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => Ok(RePatternValue::Bytes(values.clone())),
            _ => Err(RuntimeError::new("pattern must be string or bytes")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => Ok(RePatternValue::Bytes(values.clone())),
            _ => Err(RuntimeError::new("pattern must be string or bytes")),
        },
        _ => Err(RuntimeError::new("pattern must be string or bytes")),
    }
}

fn re_pattern_from_compiled_module(module: &ObjRef) -> Result<RePatternValue, RuntimeError> {
    match &*module.kind() {
        Object::Module(module_data) if module_data.name == "__re_pattern__" => {
            let Some(pattern) = module_data.globals.get("pattern") else {
                return Err(RuntimeError::new("pattern receiver is invalid"));
            };
            re_pattern_from_value(pattern)
        }
        _ => Err(RuntimeError::new("pattern receiver is invalid")),
    }
}

fn re_pattern_from_argument(value: &Value) -> Result<RePatternValue, RuntimeError> {
    match value {
        Value::Module(module) => {
            if let Ok(pattern) = re_pattern_from_compiled_module(module) {
                Ok(pattern)
            } else {
                Err(RuntimeError::new("pattern must be string or bytes"))
            }
        }
        other => re_pattern_from_value(other),
    }
}

fn find_bytes_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[derive(Clone, Copy)]
enum ReQuantifier {
    One,
    ZeroOrOne,
    ZeroOrMore,
    OneOrMore,
    Exactly(usize),
}

#[derive(Clone)]
struct ReCharClass {
    negated: bool,
    singles: Vec<char>,
    ranges: Vec<(char, char)>,
}

#[derive(Clone)]
enum ReAtom {
    Literal(char),
    Any,
    Class(ReCharClass),
    Group {
        tokens: Vec<ReToken>,
        capture: Option<usize>,
    },
}

#[derive(Clone)]
struct ReToken {
    atom: ReAtom,
    quantifier: ReQuantifier,
}

#[derive(Clone)]
struct ParsedSimpleRegex {
    start_anchor: bool,
    end_anchor: bool,
    tokens: Vec<ReToken>,
    capture_count: usize,
}

#[derive(Clone)]
struct ReMatchState {
    captures: Vec<Option<(usize, usize)>>,
}

#[derive(Clone)]
struct ReMatchDetail {
    start: usize,
    end: usize,
    captures: Vec<Option<(usize, usize)>>,
}

fn digit_class(negated: bool) -> ReCharClass {
    ReCharClass {
        negated,
        singles: Vec::new(),
        ranges: vec![('0', '9')],
    }
}

fn word_class(negated: bool) -> ReCharClass {
    ReCharClass {
        negated,
        singles: vec!['_'],
        ranges: vec![('0', '9'), ('A', 'Z'), ('a', 'z')],
    }
}

fn space_class(negated: bool) -> ReCharClass {
    ReCharClass {
        negated,
        singles: vec![' ', '\t', '\n', '\r', '\u{000b}', '\u{000c}'],
        ranges: Vec::new(),
    }
}

fn parse_simple_escape(ch: char) -> ReAtom {
    match ch {
        'd' => ReAtom::Class(digit_class(false)),
        'D' => ReAtom::Class(digit_class(true)),
        'w' => ReAtom::Class(word_class(false)),
        'W' => ReAtom::Class(word_class(true)),
        's' => ReAtom::Class(space_class(false)),
        'S' => ReAtom::Class(space_class(true)),
        'n' => ReAtom::Literal('\n'),
        'r' => ReAtom::Literal('\r'),
        't' => ReAtom::Literal('\t'),
        other => ReAtom::Literal(other),
    }
}

fn parse_char_class_char(chars: &[char], idx: &mut usize) -> Option<char> {
    if *idx >= chars.len() {
        return None;
    }
    let ch = chars[*idx];
    if ch == '\\' {
        *idx += 1;
        if *idx >= chars.len() {
            return None;
        }
        let escaped = chars[*idx];
        *idx += 1;
        return Some(match escaped {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            other => other,
        });
    }
    *idx += 1;
    Some(ch)
}

fn parse_char_class(chars: &[char], idx: &mut usize) -> Option<ReCharClass> {
    if *idx >= chars.len() || chars[*idx] != '[' {
        return None;
    }
    *idx += 1;
    let mut negated = false;
    if *idx < chars.len() && chars[*idx] == '^' {
        negated = true;
        *idx += 1;
    }

    let mut singles = Vec::new();
    let mut ranges = Vec::new();
    let mut seen_item = false;
    while *idx < chars.len() {
        if chars[*idx] == ']' && seen_item {
            *idx += 1;
            return Some(ReCharClass {
                negated,
                singles,
                ranges,
            });
        }
        let start = parse_char_class_char(chars, idx)?;
        seen_item = true;
        if *idx + 1 < chars.len() && chars[*idx] == '-' && chars[*idx + 1] != ']' {
            *idx += 1;
            let end = parse_char_class_char(chars, idx)?;
            ranges.push((start, end));
        } else {
            singles.push(start);
        }
    }
    None
}

fn parse_braced_quantifier(chars: &[char], idx: &mut usize) -> Option<usize> {
    if *idx >= chars.len() || chars[*idx] != '{' {
        return None;
    }
    let start = *idx;
    *idx += 1;
    let digits_start = *idx;
    while *idx < chars.len() && chars[*idx].is_ascii_digit() {
        *idx += 1;
    }
    if digits_start == *idx || *idx >= chars.len() || chars[*idx] != '}' {
        *idx = start;
        return None;
    }
    let count: usize = chars[digits_start..*idx]
        .iter()
        .collect::<String>()
        .parse()
        .ok()?;
    *idx += 1;
    Some(count)
}

fn parse_simple_quantifier(chars: &[char], idx: &mut usize) -> Option<ReQuantifier> {
    let quantifier = if *idx < chars.len() {
        match chars[*idx] {
            '?' => {
                *idx += 1;
                ReQuantifier::ZeroOrOne
            }
            '*' => {
                *idx += 1;
                ReQuantifier::ZeroOrMore
            }
            '+' => {
                *idx += 1;
                ReQuantifier::OneOrMore
            }
            '{' => ReQuantifier::Exactly(parse_braced_quantifier(chars, idx)?),
            _ => ReQuantifier::One,
        }
    } else {
        ReQuantifier::One
    };
    if !matches!(quantifier, ReQuantifier::One) && *idx < chars.len() && chars[*idx] == '?' {
        // Non-greedy marker accepted but currently ignored.
        *idx += 1;
    }
    Some(quantifier)
}

fn parse_simple_regex_sequence(
    chars: &[char],
    idx: &mut usize,
    capture_count: &mut usize,
    stop_on_group_end: bool,
) -> Option<Vec<ReToken>> {
    let mut tokens = Vec::new();
    while *idx < chars.len() {
        if chars[*idx] == '|' {
            // Alternation requires a full regex engine; fall back.
            return None;
        }
        if !stop_on_group_end && chars[*idx] == '$' && *idx + 1 == chars.len() {
            break;
        }
        if chars[*idx] == ')' {
            if stop_on_group_end {
                break;
            }
            return None;
        }

        let atom = if chars[*idx] == '(' {
            *idx += 1;
            let mut capture = None;
            if *idx < chars.len() && chars[*idx] == '?' {
                *idx += 1;
                if *idx < chars.len() && chars[*idx] == ':' {
                    *idx += 1;
                } else {
                    return None;
                }
            } else {
                *capture_count += 1;
                capture = Some(*capture_count);
            }
            let inner = parse_simple_regex_sequence(chars, idx, capture_count, true)?;
            if *idx >= chars.len() || chars[*idx] != ')' {
                return None;
            }
            *idx += 1;
            ReAtom::Group {
                tokens: inner,
                capture,
            }
        } else {
            match chars[*idx] {
                '.' => {
                    *idx += 1;
                    ReAtom::Any
                }
                '[' => parse_char_class(chars, idx).map(ReAtom::Class)?,
                '\\' => {
                    *idx += 1;
                    if *idx >= chars.len() {
                        return None;
                    }
                    let escaped = chars[*idx];
                    *idx += 1;
                    parse_simple_escape(escaped)
                }
                ch => {
                    *idx += 1;
                    ReAtom::Literal(ch)
                }
            }
        };

        let quantifier = parse_simple_quantifier(chars, idx)?;
        tokens.push(ReToken { atom, quantifier });
    }
    Some(tokens)
}

fn parse_simple_regex(pattern: &str) -> Option<ParsedSimpleRegex> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut idx = 0usize;
    let mut start_anchor = false;
    let mut end_anchor = false;
    if idx < chars.len() && chars[idx] == '^' {
        start_anchor = true;
        idx += 1;
    }

    let mut capture_count = 0usize;
    let tokens = parse_simple_regex_sequence(&chars, &mut idx, &mut capture_count, false)?;
    if idx < chars.len() && chars[idx] == '$' && idx + 1 == chars.len() {
        end_anchor = true;
        idx += 1;
    }
    if idx != chars.len() {
        return None;
    }
    Some(ParsedSimpleRegex {
        start_anchor,
        end_anchor,
        tokens,
        capture_count,
    })
}

fn expand_simple_group_alternation(pattern: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut idx = 0usize;
    let mut escape = false;
    let mut in_class = false;
    while idx < chars.len() {
        let ch = chars[idx];
        if escape {
            escape = false;
            idx += 1;
            continue;
        }
        if ch == '\\' {
            escape = true;
            idx += 1;
            continue;
        }
        if in_class {
            if ch == ']' {
                in_class = false;
            }
            idx += 1;
            continue;
        }
        if ch == '[' {
            in_class = true;
            idx += 1;
            continue;
        }
        if ch != '(' {
            idx += 1;
            continue;
        }

        let mut group_end = idx + 1;
        let mut depth = 1usize;
        let mut group_escape = false;
        let mut group_in_class = false;
        let mut bars = Vec::new();
        while group_end < chars.len() {
            let cur = chars[group_end];
            if group_escape {
                group_escape = false;
                group_end += 1;
                continue;
            }
            if cur == '\\' {
                group_escape = true;
                group_end += 1;
                continue;
            }
            if group_in_class {
                if cur == ']' {
                    group_in_class = false;
                }
                group_end += 1;
                continue;
            }
            if cur == '[' {
                group_in_class = true;
                group_end += 1;
                continue;
            }
            if cur == '(' {
                depth += 1;
                group_end += 1;
                continue;
            }
            if cur == ')' {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
                group_end += 1;
                continue;
            }
            if cur == '|' && depth == 1 {
                bars.push(group_end);
            }
            group_end += 1;
        }
        if depth != 0 || bars.is_empty() {
            idx += 1;
            continue;
        }

        let mut content_start = idx + 1;
        if content_start + 1 < chars.len()
            && chars[content_start] == '?'
            && chars[content_start + 1] == ':'
        {
            content_start += 2;
        } else if content_start < chars.len() && chars[content_start] == '?' {
            // Unsupported group forms (named groups/lookarounds/backrefs).
            idx += 1;
            continue;
        }

        let mut alternatives = Vec::new();
        let mut segment_start = content_start;
        for bar in &bars {
            alternatives.push(chars[segment_start..*bar].iter().collect::<String>());
            segment_start = *bar + 1;
        }
        alternatives.push(chars[segment_start..group_end].iter().collect::<String>());

        let prefix = chars[..idx].iter().collect::<String>();
        let suffix = chars[group_end + 1..].iter().collect::<String>();
        return Some(
            alternatives
                .into_iter()
                .map(|alt| format!("{prefix}(?:{alt}){suffix}"))
                .collect(),
        );
    }
    None
}

fn class_matches(class: &ReCharClass, ch: char) -> bool {
    let mut matched = class.singles.contains(&ch);
    if !matched {
        let code = ch as u32;
        matched = class.ranges.iter().any(|(start, end)| {
            let start_code = *start as u32;
            let end_code = *end as u32;
            code >= start_code && code <= end_code
        });
    }
    if class.negated { !matched } else { matched }
}

fn atom_matches_char(atom: &ReAtom, ch: char) -> bool {
    match atom {
        ReAtom::Literal(expected) => *expected == ch,
        ReAtom::Any => true,
        ReAtom::Class(class) => class_matches(class, ch),
        ReAtom::Group { .. } => false,
    }
}

fn match_simple_regex_tokens(
    tokens: &[ReToken],
    chars: &[char],
    token_idx: usize,
    char_idx: usize,
    require_end: bool,
    state: ReMatchState,
) -> Option<(usize, ReMatchState)> {
    if token_idx == tokens.len() {
        if require_end && char_idx != chars.len() {
            return None;
        }
        return Some((char_idx, state));
    }

    fn match_atom_once(
        atom: &ReAtom,
        chars: &[char],
        char_idx: usize,
        state: &ReMatchState,
    ) -> Option<(usize, ReMatchState)> {
        match atom {
            ReAtom::Literal(_) | ReAtom::Any | ReAtom::Class(_) => {
                if char_idx >= chars.len() || !atom_matches_char(atom, chars[char_idx]) {
                    return None;
                }
                Some((char_idx + 1, state.clone()))
            }
            ReAtom::Group { tokens, capture } => {
                let (end, mut next_state) =
                    match_simple_regex_tokens(tokens, chars, 0, char_idx, false, state.clone())?;
                if let Some(index) = capture {
                    if let Some(slot) = next_state.captures.get_mut(index - 1) {
                        *slot = Some((char_idx, end));
                    }
                }
                Some((end, next_state))
            }
        }
    }

    let token = &tokens[token_idx];
    match token.quantifier {
        ReQuantifier::One => {
            let (next_idx, next_state) = match_atom_once(&token.atom, chars, char_idx, &state)?;
            match_simple_regex_tokens(
                tokens,
                chars,
                token_idx + 1,
                next_idx,
                require_end,
                next_state,
            )
        }
        ReQuantifier::ZeroOrOne => {
            if let Some((next_idx, next_state)) =
                match_atom_once(&token.atom, chars, char_idx, &state)
            {
                if let Some(result) = match_simple_regex_tokens(
                    tokens,
                    chars,
                    token_idx + 1,
                    next_idx,
                    require_end,
                    next_state,
                ) {
                    return Some(result);
                }
            }
            match_simple_regex_tokens(tokens, chars, token_idx + 1, char_idx, require_end, state)
        }
        ReQuantifier::ZeroOrMore | ReQuantifier::OneOrMore | ReQuantifier::Exactly(_) => {
            let mut states = Vec::new();
            states.push((char_idx, state.clone()));
            let mut cursor_idx = char_idx;
            let mut cursor_state = state;
            while let Some((next_idx, next_state)) =
                match_atom_once(&token.atom, chars, cursor_idx, &cursor_state)
            {
                states.push((next_idx, next_state.clone()));
                if next_idx == cursor_idx {
                    break;
                }
                cursor_idx = next_idx;
                cursor_state = next_state;
            }

            let max_reps = states.len().saturating_sub(1);
            let (min_reps, max_reps) = match token.quantifier {
                ReQuantifier::ZeroOrMore => (0usize, max_reps),
                ReQuantifier::OneOrMore => {
                    if max_reps == 0 {
                        return None;
                    }
                    (1usize, max_reps)
                }
                ReQuantifier::Exactly(expected) => {
                    if expected > max_reps {
                        return None;
                    }
                    (expected, expected)
                }
                _ => unreachable!(),
            };

            for reps in (min_reps..=max_reps).rev() {
                let (candidate_idx, candidate_state) = states[reps].clone();
                if let Some(result) = match_simple_regex_tokens(
                    tokens,
                    chars,
                    token_idx + 1,
                    candidate_idx,
                    require_end,
                    candidate_state,
                ) {
                    return Some(result);
                }
            }
            None
        }
    }
}

fn simple_regex_match_details(pattern: &str, text: &str, mode: ReMode) -> Option<ReMatchDetail> {
    let parsed = if let Some(parsed) = parse_simple_regex(pattern) {
        parsed
    } else if let Some(expanded_patterns) = expand_simple_group_alternation(pattern) {
        for expanded in expanded_patterns {
            if let Some(detail) = simple_regex_match_details(&expanded, text, mode) {
                return Some(detail);
            }
        }
        return None;
    } else {
        return None;
    };
    let chars: Vec<char> = text.chars().collect();
    let starts: Vec<usize> = match mode {
        ReMode::Search if !parsed.start_anchor => (0..=chars.len()).collect(),
        _ => vec![0],
    };
    let require_end = matches!(mode, ReMode::FullMatch) || parsed.end_anchor;
    let mut byte_offsets: Vec<usize> = text.char_indices().map(|(idx, _)| idx).collect();
    byte_offsets.push(text.len());

    for start in starts {
        if parsed.start_anchor && start != 0 {
            continue;
        }
        let state = ReMatchState {
            captures: vec![None; parsed.capture_count],
        };
        if let Some((end, state)) =
            match_simple_regex_tokens(&parsed.tokens, &chars, 0, start, require_end, state)
        {
            let captures = state
                .captures
                .into_iter()
                .map(|capture| {
                    capture.map(|(capture_start, capture_end)| {
                        (byte_offsets[capture_start], byte_offsets[capture_end])
                    })
                })
                .collect();
            return Some(ReMatchDetail {
                start: byte_offsets[start],
                end: byte_offsets[end],
                captures,
            });
        }
    }
    None
}

fn csv_sniffer_doublequote_quote(pattern: &str) -> Option<char> {
    if !pattern.starts_with("((") || !pattern.contains(")|^)") || !pattern.ends_with(")|$)") {
        return None;
    }
    let marker = if pattern.contains(")|^)\\W*") {
        ")|^)\\W*"
    } else if pattern.contains(")|^)W*") {
        ")|^)W*"
    } else {
        return None;
    };
    let marker_pos = pattern.find(marker)?;
    let mut rest = &pattern[marker_pos + marker.len()..];
    if let Some(stripped) = rest.strip_prefix('\\') {
        rest = stripped;
    }
    let mut chars = rest.chars();
    let first = chars.next()?;
    let quote = if first == '\\' { chars.next()? } else { first };
    if quote == '\'' || quote == '"' {
        Some(quote)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
struct PkgutilDottedWords {
    span: (usize, usize),
    first_word: (usize, usize),
    last_dotted_segment: Option<(usize, usize)>,
    last_word: Option<(usize, usize)>,
}

fn pkgutil_word_char(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

fn parse_pkgutil_word(text: &str, mut index: usize) -> Option<(usize, usize)> {
    if index >= text.len() {
        return None;
    }
    let start = index;
    let first = text[index..].chars().next()?;
    if first.is_ascii_digit() || !pkgutil_word_char(first) {
        return None;
    }
    index += first.len_utf8();
    while index < text.len() {
        let ch = text[index..].chars().next()?;
        if !pkgutil_word_char(ch) {
            break;
        }
        index += ch.len_utf8();
    }
    Some((start, index))
}

fn parse_pkgutil_dotted_words(text: &str, start: usize) -> Option<PkgutilDottedWords> {
    let (first_start, first_end) = parse_pkgutil_word(text, start)?;
    let mut cursor = first_end;
    let mut last_dotted_segment = None;
    let mut last_word = None;
    while cursor < text.len() && text[cursor..].starts_with('.') {
        let dot_start = cursor;
        cursor += 1;
        let (word_start, word_end) = parse_pkgutil_word(text, cursor)?;
        cursor = word_end;
        last_dotted_segment = Some((dot_start, cursor));
        last_word = Some((word_start, word_end));
    }
    Some(PkgutilDottedWords {
        span: (start, cursor),
        first_word: (first_start, first_end),
        last_dotted_segment,
        last_word,
    })
}

fn pkgutil_resolve_name_match_detail(text: &str) -> Option<ReMatchDetail> {
    let pkg = parse_pkgutil_dotted_words(text, 0)?;
    let mut cursor = pkg.span.1;
    let mut cln_span = None;
    let mut obj = None;
    if cursor < text.len() {
        if !text[cursor..].starts_with(':') {
            return None;
        }
        let colon_start = cursor;
        cursor += 1;
        if cursor < text.len() {
            let parsed_obj = parse_pkgutil_dotted_words(text, cursor)?;
            cursor = parsed_obj.span.1;
            cln_span = Some((colon_start, cursor));
            obj = Some(parsed_obj);
        } else {
            cln_span = Some((colon_start, cursor));
        }
    }
    if cursor != text.len() {
        return None;
    }

    // Captures follow CPython's group numbering for pkgutil._NAME_PATTERN.
    let mut captures = vec![None; 9];
    captures[0] = Some(pkg.span);
    captures[1] = Some(pkg.first_word);
    captures[2] = pkg.last_dotted_segment;
    captures[3] = pkg.last_word;
    captures[4] = cln_span;
    if let Some(obj) = obj {
        captures[5] = Some(obj.span);
        captures[6] = Some(obj.first_word);
        captures[7] = obj.last_dotted_segment;
        captures[8] = obj.last_word;
    }

    Some(ReMatchDetail {
        start: 0,
        end: text.len(),
        captures,
    })
}

fn re_match_details(
    pattern: &RePatternValue,
    text: &Value,
    mode: ReMode,
) -> Result<Option<ReMatchDetail>, RuntimeError> {
    match pattern {
        RePatternValue::Str(pattern_text) => {
            let text = match text {
                Value::Str(value) => value,
                Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                    return Err(RuntimeError::new(
                        "cannot use a string pattern on a bytes-like object",
                    ));
                }
                _ => return Err(RuntimeError::new("string must be string")),
            };
            let found = if pattern_text == PKGUTIL_RESOLVE_NAME_PATTERN {
                pkgutil_resolve_name_match_detail(text)
            } else if matches!(mode, ReMode::Search) {
                if let Some(quote) = csv_sniffer_doublequote_quote(pattern_text) {
                    let needle = format!("{quote}{quote}");
                    text.find(&needle).map(|start| ReMatchDetail {
                        start,
                        end: start + needle.len(),
                        captures: Vec::new(),
                    })
                } else if pattern_text == LOGGING_PERCENT_VALIDATION_PATTERN {
                    find_logging_percent_style_match(text).map(|(start, end)| ReMatchDetail {
                        start,
                        end,
                        captures: Vec::new(),
                    })
                } else if let Some(detail) = simple_regex_match_details(pattern_text, text, mode) {
                    Some(detail)
                } else {
                    text.find(pattern_text).map(|start| ReMatchDetail {
                        start,
                        end: start + pattern_text.len(),
                        captures: Vec::new(),
                    })
                }
            } else if pattern_text == LOGGING_PERCENT_VALIDATION_PATTERN {
                find_logging_percent_style_match(text).map(|(start, end)| ReMatchDetail {
                    start,
                    end,
                    captures: Vec::new(),
                })
            } else if let Some(detail) = simple_regex_match_details(pattern_text, text, mode) {
                Some(detail)
            } else {
                match mode {
                    ReMode::Search => text.find(pattern_text).map(|start| ReMatchDetail {
                        start,
                        end: start + pattern_text.len(),
                        captures: Vec::new(),
                    }),
                    ReMode::Match => text.starts_with(pattern_text).then_some(ReMatchDetail {
                        start: 0,
                        end: pattern_text.len(),
                        captures: Vec::new(),
                    }),
                    ReMode::FullMatch => (text == pattern_text).then_some(ReMatchDetail {
                        start: 0,
                        end: pattern_text.len(),
                        captures: Vec::new(),
                    }),
                }
            };
            let found = match mode {
                ReMode::Search => found,
                ReMode::Match => found.filter(|detail| detail.start == 0),
                ReMode::FullMatch => {
                    found.filter(|detail| detail.start == 0 && detail.end == text.len())
                }
            };
            Ok(found)
        }
        RePatternValue::Bytes(pattern_bytes) => {
            let text = match text {
                Value::Bytes(obj) => match &*obj.kind() {
                    Object::Bytes(values) => values.clone(),
                    _ => return Err(RuntimeError::new("string must be bytes-like")),
                },
                Value::ByteArray(obj) => match &*obj.kind() {
                    Object::ByteArray(values) => values.clone(),
                    _ => return Err(RuntimeError::new("string must be bytes-like")),
                },
                Value::MemoryView(obj) => match &*obj.kind() {
                    Object::MemoryView(view) => {
                        with_bytes_like_source(&view.source, |values| values.to_vec())
                            .ok_or_else(|| RuntimeError::new("string must be bytes-like"))?
                    }
                    _ => return Err(RuntimeError::new("string must be bytes-like")),
                },
                Value::Str(_) => {
                    return Err(RuntimeError::new(
                        "cannot use a bytes pattern on a string-like object",
                    ));
                }
                _ => return Err(RuntimeError::new("string must be bytes-like")),
            };
            let regex_found = match (
                std::str::from_utf8(pattern_bytes.as_slice()),
                std::str::from_utf8(text.as_slice()),
            ) {
                (Ok(pattern_text), Ok(text_text)) => {
                    if pattern_text == LOGGING_PERCENT_VALIDATION_PATTERN {
                        find_logging_percent_style_match(text_text).map(|(start, end)| {
                            ReMatchDetail {
                                start,
                                end,
                                captures: Vec::new(),
                            }
                        })
                    } else {
                        simple_regex_match_details(pattern_text, text_text, mode)
                    }
                }
                _ => None,
            };
            let found = match mode {
                ReMode::Search => regex_found.or_else(|| {
                    find_bytes_subslice(&text, pattern_bytes).map(|start| ReMatchDetail {
                        start,
                        end: start + pattern_bytes.len(),
                        captures: Vec::new(),
                    })
                }),
                ReMode::Match => regex_found.or_else(|| {
                    text.starts_with(pattern_bytes).then_some(ReMatchDetail {
                        start: 0,
                        end: pattern_bytes.len(),
                        captures: Vec::new(),
                    })
                }),
                ReMode::FullMatch => regex_found.or_else(|| {
                    (text == *pattern_bytes).then_some(ReMatchDetail {
                        start: 0,
                        end: pattern_bytes.len(),
                        captures: Vec::new(),
                    })
                }),
            };
            let found = match mode {
                ReMode::Search => found,
                ReMode::Match => found.filter(|detail| detail.start == 0),
                ReMode::FullMatch => {
                    found.filter(|detail| detail.start == 0 && detail.end == text.len())
                }
            };
            Ok(found)
        }
    }
}

fn re_match_bounds(
    pattern: &RePatternValue,
    text: &Value,
    mode: ReMode,
) -> Result<Option<(usize, usize)>, RuntimeError> {
    Ok(re_match_details(pattern, text, mode)?.map(|detail| (detail.start, detail.end)))
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
        "utf-16" | "utf16" => Ok("utf-16".to_string()),
        "utf-16-le" | "utf16-le" | "utf-16le" | "utf16le" => Ok("utf-16-le".to_string()),
        "utf-16-be" | "utf16-be" | "utf-16be" | "utf16be" => Ok("utf-16-be".to_string()),
        "utf-32" | "utf32" => Ok("utf-32".to_string()),
        "utf-32-le" | "utf32-le" | "utf-32le" | "utf32le" => Ok("utf-32-le".to_string()),
        "utf-32-be" | "utf32-be" | "utf-32be" | "utf32be" => Ok("utf-32-be".to_string()),
        "ascii" => Ok("ascii".to_string()),
        "latin-1" | "latin1" => Ok("latin-1".to_string()),
        "raw-unicode-escape" | "raw_unicode_escape" => Ok("raw-unicode-escape".to_string()),
        "unicode-escape" | "unicode_escape" => Ok("unicode-escape".to_string()),
        _ => Err(RuntimeError::new("unsupported encoding")),
    }
}

fn normalize_codec_errors(value: Value) -> Result<String, RuntimeError> {
    let mode = match value {
        Value::Str(mode) => mode.to_ascii_lowercase(),
        _ => return Err(RuntimeError::new("errors must be string")),
    };
    match mode.as_str() {
        "strict" | "ignore" | "replace" | "surrogateescape" => Ok(mode),
        "surrogatepass" => Ok("strict".to_string()),
        "backslashreplace" | "namereplace" | "xmlcharrefreplace" => Ok("replace".to_string()),
        _ => Err(RuntimeError::new("unsupported error handler")),
    }
}

fn push_escape_decode_error(
    errors: &str,
    out: &mut Vec<u8>,
    message: &str,
) -> Result<(), RuntimeError> {
    match errors {
        "strict" => Err(RuntimeError::new(message)),
        "ignore" => Ok(()),
        "replace" | "surrogateescape" => {
            out.push(b'?');
            Ok(())
        }
        _ => Err(RuntimeError::new("unsupported error handler")),
    }
}

fn decode_escape_bytes(input: &[u8], errors: &str) -> Result<Vec<u8>, RuntimeError> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0usize;
    while i < input.len() {
        let byte = input[i];
        if byte != b'\\' {
            out.push(byte);
            i += 1;
            continue;
        }
        if i + 1 >= input.len() {
            push_escape_decode_error(errors, &mut out, "invalid trailing escape in bytes literal")?;
            break;
        }
        let esc = input[i + 1];
        match esc {
            b'\\' | b'\'' | b'"' => {
                out.push(esc);
                i += 2;
            }
            b'a' => {
                out.push(0x07);
                i += 2;
            }
            b'b' => {
                out.push(0x08);
                i += 2;
            }
            b'f' => {
                out.push(0x0C);
                i += 2;
            }
            b'n' => {
                out.push(b'\n');
                i += 2;
            }
            b'r' => {
                out.push(b'\r');
                i += 2;
            }
            b't' => {
                out.push(b'\t');
                i += 2;
            }
            b'v' => {
                out.push(0x0B);
                i += 2;
            }
            b'x' => {
                if i + 3 < input.len() {
                    let hi = (input[i + 2] as char).to_digit(16);
                    let lo = (input[i + 3] as char).to_digit(16);
                    if let (Some(hi), Some(lo)) = (hi, lo) {
                        out.push(((hi << 4) as u8) | (lo as u8));
                        i += 4;
                        continue;
                    }
                }
                push_escape_decode_error(errors, &mut out, "invalid \\x escape in bytes literal")?;
                i += 2;
            }
            b'0'..=b'7' => {
                let mut value = (esc - b'0') as u32;
                let mut consumed = 1usize;
                while consumed < 3 && i + 1 + consumed < input.len() {
                    let next = input[i + 1 + consumed];
                    if !(b'0'..=b'7').contains(&next) {
                        break;
                    }
                    value = (value << 3) | ((next - b'0') as u32);
                    consumed += 1;
                }
                out.push((value & 0xFF) as u8);
                i += 1 + consumed;
            }
            _ => {
                // CPython keeps unknown escapes by discarding the backslash.
                out.push(esc);
                i += 2;
            }
        }
    }
    Ok(out)
}

fn encode_text_bytes(text: &str, encoding: &str, errors: &str) -> Result<Vec<u8>, RuntimeError> {
    match encoding {
        "utf-8" => Ok(text.as_bytes().to_vec()),
        "utf-16" => {
            let mut out = Vec::new();
            out.extend_from_slice(&[0xFF, 0xFE]);
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            Ok(out)
        }
        "utf-16-le" => {
            let mut out = Vec::new();
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            Ok(out)
        }
        "utf-16-be" => {
            let mut out = Vec::new();
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_be_bytes());
            }
            Ok(out)
        }
        "utf-32" => {
            let mut out = Vec::new();
            out.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x00]);
            for ch in text.chars() {
                out.extend_from_slice(&(ch as u32).to_le_bytes());
            }
            Ok(out)
        }
        "utf-32-le" => {
            let mut out = Vec::new();
            for ch in text.chars() {
                out.extend_from_slice(&(ch as u32).to_le_bytes());
            }
            Ok(out)
        }
        "utf-32-be" => {
            let mut out = Vec::new();
            for ch in text.chars() {
                out.extend_from_slice(&(ch as u32).to_be_bytes());
            }
            Ok(out)
        }
        "ascii" => {
            let mut out = Vec::new();
            for ch in text.chars() {
                let code = ch as u32;
                if code <= 0x7F {
                    out.push(code as u8);
                    continue;
                }
                match errors {
                    "strict" => {
                        return Err(RuntimeError::new("ascii codec can't encode character"));
                    }
                    "ignore" => {}
                    "replace" | "surrogateescape" => out.push(b'?'),
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
                    "replace" | "surrogateescape" => out.push(b'?'),
                    _ => return Err(RuntimeError::new("unsupported error handler")),
                }
            }
            Ok(out)
        }
        "raw-unicode-escape" => Ok(encode_raw_unicode_escape(text)),
        "unicode-escape" => Ok(encode_unicode_escape(text)),
        _ => Err(RuntimeError::new("unsupported encoding")),
    }
}

fn decode_text_bytes(bytes: &[u8], encoding: &str, errors: &str) -> Result<String, RuntimeError> {
    match encoding {
        "utf-8" => decode_utf8_bytes(bytes, errors),
        "utf-16" => {
            if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
                decode_utf16_bytes(&bytes[2..], errors, true)
            } else if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
                decode_utf16_bytes(&bytes[2..], errors, false)
            } else {
                decode_utf16_bytes(bytes, errors, false)
            }
        }
        "utf-16-le" => decode_utf16_bytes(bytes, errors, false),
        "utf-16-be" => decode_utf16_bytes(bytes, errors, true),
        "utf-32" => {
            if bytes.len() >= 4 && bytes[0..4] == [0x00, 0x00, 0xFE, 0xFF] {
                decode_utf32_bytes(&bytes[4..], errors, true)
            } else if bytes.len() >= 4 && bytes[0..4] == [0xFF, 0xFE, 0x00, 0x00] {
                decode_utf32_bytes(&bytes[4..], errors, false)
            } else {
                decode_utf32_bytes(bytes, errors, false)
            }
        }
        "utf-32-le" => decode_utf32_bytes(bytes, errors, false),
        "utf-32-be" => decode_utf32_bytes(bytes, errors, true),
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
                    "replace" | "surrogateescape" => out.push('\u{FFFD}'),
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
        "raw-unicode-escape" => decode_raw_unicode_escape(bytes, errors),
        "unicode-escape" => decode_unicode_escape(bytes, errors),
        _ => Err(RuntimeError::new("unsupported encoding")),
    }
}

fn decode_raw_unicode_escape(bytes: &[u8], errors: &str) -> Result<String, RuntimeError> {
    let mut out = String::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' && index + 1 < bytes.len() {
            let kind = bytes[index + 1];
            if kind == b'u' && index + 5 < bytes.len() {
                let hex = &bytes[index + 2..index + 6];
                let parsed = std::str::from_utf8(hex)
                    .ok()
                    .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                    .and_then(char::from_u32);
                if let Some(ch) = parsed {
                    out.push(ch);
                    index += 6;
                    continue;
                }
                if errors == "strict" {
                    return Err(RuntimeError::new("unicode escape decode failed"));
                }
            } else if kind == b'U' && index + 9 < bytes.len() {
                let hex = &bytes[index + 2..index + 10];
                let parsed = std::str::from_utf8(hex)
                    .ok()
                    .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                    .and_then(char::from_u32);
                if let Some(ch) = parsed {
                    out.push(ch);
                    index += 10;
                    continue;
                }
                if errors == "strict" {
                    return Err(RuntimeError::new("unicode escape decode failed"));
                }
            }
        }
        out.push(byte as char);
        index += 1;
    }
    Ok(out)
}

fn decode_unicode_escape(bytes: &[u8], errors: &str) -> Result<String, RuntimeError> {
    let mut out = String::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte != b'\\' || index + 1 >= bytes.len() {
            out.push(byte as char);
            index += 1;
            continue;
        }

        let esc = bytes[index + 1];
        let mut push_error = |errors: &str| -> Result<(), RuntimeError> {
            match errors {
                "strict" => Err(RuntimeError::new("unicode escape decode failed")),
                "ignore" => Ok(()),
                "replace" | "surrogateescape" => {
                    out.push('\u{FFFD}');
                    Ok(())
                }
                _ => Err(RuntimeError::new("unsupported error handler")),
            }
        };
        match esc {
            b'\\' => {
                out.push('\\');
                index += 2;
            }
            b'\'' => {
                out.push('\'');
                index += 2;
            }
            b'"' => {
                out.push('"');
                index += 2;
            }
            b'a' => {
                out.push('\u{0007}');
                index += 2;
            }
            b'b' => {
                out.push('\u{0008}');
                index += 2;
            }
            b'f' => {
                out.push('\u{000c}');
                index += 2;
            }
            b'n' => {
                out.push('\n');
                index += 2;
            }
            b'r' => {
                out.push('\r');
                index += 2;
            }
            b't' => {
                out.push('\t');
                index += 2;
            }
            b'v' => {
                out.push('\u{000b}');
                index += 2;
            }
            b'x' => {
                if index + 3 < bytes.len() {
                    let hi = (bytes[index + 2] as char).to_digit(16);
                    let lo = (bytes[index + 3] as char).to_digit(16);
                    if let (Some(hi), Some(lo)) = (hi, lo) {
                        out.push(((hi << 4) | lo) as u8 as char);
                        index += 4;
                        continue;
                    }
                }
                push_error(errors)?;
                index += 2;
            }
            b'u' => {
                if index + 5 < bytes.len() {
                    let hex = &bytes[index + 2..index + 6];
                    let parsed = std::str::from_utf8(hex)
                        .ok()
                        .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                        .and_then(char::from_u32);
                    if let Some(ch) = parsed {
                        out.push(ch);
                        index += 6;
                        continue;
                    }
                }
                push_error(errors)?;
                index += 2;
            }
            b'U' => {
                if index + 9 < bytes.len() {
                    let hex = &bytes[index + 2..index + 10];
                    let parsed = std::str::from_utf8(hex)
                        .ok()
                        .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                        .and_then(char::from_u32);
                    if let Some(ch) = parsed {
                        out.push(ch);
                        index += 10;
                        continue;
                    }
                }
                push_error(errors)?;
                index += 2;
            }
            b'0'..=b'7' => {
                let mut value = (esc - b'0') as u32;
                let mut consumed = 1usize;
                while consumed < 3 && index + 1 + consumed < bytes.len() {
                    let next = bytes[index + 1 + consumed];
                    if !(b'0'..=b'7').contains(&next) {
                        break;
                    }
                    value = (value << 3) | ((next - b'0') as u32);
                    consumed += 1;
                }
                out.push((value & 0xFF) as u8 as char);
                index += 1 + consumed;
            }
            _ => {
                out.push(esc as char);
                index += 2;
            }
        }
    }
    Ok(out)
}

fn encode_raw_unicode_escape(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let code = ch as u32;
        if code <= 0x7f {
            out.push(code as u8);
        } else if code <= 0xffff {
            out.extend_from_slice(format!("\\u{code:04x}").as_bytes());
        } else {
            out.extend_from_slice(format!("\\U{code:08x}").as_bytes());
        }
    }
    out
}

fn encode_unicode_escape(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{0007}' => out.extend_from_slice(b"\\a"),
            '\u{0008}' => out.extend_from_slice(b"\\b"),
            '\u{0009}' => out.extend_from_slice(b"\\t"),
            '\u{000a}' => out.extend_from_slice(b"\\n"),
            '\u{000b}' => out.extend_from_slice(b"\\v"),
            '\u{000c}' => out.extend_from_slice(b"\\f"),
            '\u{000d}' => out.extend_from_slice(b"\\r"),
            _ => {
                let code = ch as u32;
                if (0x20..=0x7e).contains(&code) {
                    out.push(code as u8);
                } else if code <= 0xffff {
                    out.extend_from_slice(format!("\\u{code:04x}").as_bytes());
                } else {
                    out.extend_from_slice(format!("\\U{code:08x}").as_bytes());
                }
            }
        }
    }
    out
}

fn decode_utf16_bytes(
    bytes: &[u8],
    errors: &str,
    big_endian: bool,
) -> Result<String, RuntimeError> {
    let mut units = Vec::new();
    let mut pos = 0usize;
    while pos + 1 < bytes.len() {
        let pair = [bytes[pos], bytes[pos + 1]];
        let unit = if big_endian {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        };
        units.push(unit);
        pos += 2;
    }
    if pos < bytes.len() {
        match errors {
            "strict" => return Err(RuntimeError::new("utf-16 codec can't decode bytes")),
            "ignore" => {}
            "replace" | "surrogateescape" => units.push(0xFFFD),
            _ => return Err(RuntimeError::new("unsupported error handler")),
        }
    }

    let mut out = String::new();
    for decoded in std::char::decode_utf16(units.into_iter()) {
        match decoded {
            Ok(ch) => out.push(ch),
            Err(_) => match errors {
                "strict" => return Err(RuntimeError::new("utf-16 codec can't decode bytes")),
                "ignore" => {}
                "replace" | "surrogateescape" => out.push('\u{FFFD}'),
                _ => return Err(RuntimeError::new("unsupported error handler")),
            },
        }
    }
    Ok(out)
}

fn decode_utf32_bytes(
    bytes: &[u8],
    errors: &str,
    big_endian: bool,
) -> Result<String, RuntimeError> {
    let mut out = String::new();
    let mut pos = 0usize;
    while pos + 4 <= bytes.len() {
        let chunk = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
        let code = if big_endian {
            u32::from_be_bytes(chunk)
        } else {
            u32::from_le_bytes(chunk)
        };
        match char::from_u32(code) {
            Some(ch) => out.push(ch),
            None => match errors {
                "strict" => return Err(RuntimeError::new("utf-32 codec can't decode bytes")),
                "ignore" => {}
                "replace" | "surrogateescape" => out.push('\u{FFFD}'),
                _ => return Err(RuntimeError::new("unsupported error handler")),
            },
        }
        pos += 4;
    }
    if pos < bytes.len() {
        match errors {
            "strict" => return Err(RuntimeError::new("utf-32 codec can't decode bytes")),
            "ignore" => {}
            "replace" | "surrogateescape" => out.push('\u{FFFD}'),
            _ => return Err(RuntimeError::new("unsupported error handler")),
        }
    }
    Ok(out)
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
                    let fragment = std::str::from_utf8(&bytes[pos..pos + valid])
                        .map_err(|_| RuntimeError::new("utf-8 codec can't decode bytes"))?;
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
                    "replace" | "surrogateescape" => {
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

fn is_ascii_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn find_logging_percent_style_match(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'%' || i + 2 >= bytes.len() || bytes[i + 1] != b'(' {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        if !is_ascii_word(bytes[j]) {
            i += 1;
            continue;
        }
        while j < bytes.len() && is_ascii_word(bytes[j]) {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b')' {
            i += 1;
            continue;
        }
        j += 1;
        while j < bytes.len() && matches!(bytes[j], b'#' | b'0' | b'+' | b' ' | b'-') {
            j += 1;
        }
        if j < bytes.len() {
            if bytes[j] == b'*' {
                j += 1;
            } else {
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
            }
        }
        if j < bytes.len() && bytes[j] == b'.' {
            j += 1;
            if j >= bytes.len() {
                i += 1;
                continue;
            }
            if bytes[j] == b'*' {
                j += 1;
            } else if bytes[j].is_ascii_digit() {
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
            } else {
                i += 1;
                continue;
            }
        }
        if j >= bytes.len() {
            i += 1;
            continue;
        }
        let spec = bytes[j].to_ascii_lowercase();
        if matches!(
            spec,
            b'd' | b'i'
                | b'o'
                | b'u'
                | b'x'
                | b'e'
                | b'f'
                | b'g'
                | b'c'
                | b'r'
                | b's'
                | b'a'
                | b'%'
        ) {
            return Some((i, j + 1));
        }
        i += 1;
    }
    None
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

#[derive(Clone, Copy)]
struct TimeParts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    weekday: u32,
    yearday: u32,
    isdst: i32,
}

fn unix_seconds_now() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    now.as_secs().min(i64::MAX as u64) as i64
}

fn split_unix_timestamp(total_secs: i64) -> TimeParts {
    let days = total_secs.div_euclid(86_400);
    let sec_of_day = total_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = (sec_of_day / 3600) as u32;
    let minute = ((sec_of_day % 3600) / 60) as u32;
    let second = (sec_of_day % 60) as u32;
    let weekday = (days + 3).rem_euclid(7) as u32; // Monday=0
    let yearday = day_of_year(year, month, day);
    TimeParts {
        year,
        month,
        day,
        hour,
        minute,
        second,
        weekday,
        yearday,
        isdst: -1,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    let month_days = [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut total = day;
    for idx in 0..month.saturating_sub(1) as usize {
        total += month_days[idx];
    }
    total
}

fn time_parts_from_value(value: &Value) -> Result<TimeParts, RuntimeError> {
    let values = match value {
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => values.clone(),
            _ => return Err(RuntimeError::new("invalid time tuple")),
        },
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => values.clone(),
            _ => return Err(RuntimeError::new("invalid time tuple")),
        },
        _ => return Err(RuntimeError::new("time tuple must be tuple or list")),
    };
    if values.len() < 9 {
        return Err(RuntimeError::new("time tuple must have at least 9 items"));
    }
    Ok(TimeParts {
        year: value_to_int(values[0].clone())? as i32,
        month: value_to_int(values[1].clone())? as u32,
        day: value_to_int(values[2].clone())? as u32,
        hour: value_to_int(values[3].clone())? as u32,
        minute: value_to_int(values[4].clone())? as u32,
        second: value_to_int(values[5].clone())? as u32,
        weekday: value_to_int(values[6].clone())? as u32,
        yearday: value_to_int(values[7].clone())? as u32,
        isdst: value_to_int(values[8].clone())? as i32,
    })
}

fn format_strftime(format: &str, parts: TimeParts) -> String {
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        let Some(spec) = chars.next() else {
            out.push('%');
            break;
        };
        match spec {
            '%' => out.push('%'),
            'Y' => out.push_str(&format!("{:04}", parts.year)),
            'm' => out.push_str(&format!("{:02}", parts.month)),
            'd' => out.push_str(&format!("{:02}", parts.day)),
            'H' => out.push_str(&format!("{:02}", parts.hour)),
            'M' => out.push_str(&format!("{:02}", parts.minute)),
            'S' => out.push_str(&format!("{:02}", parts.second)),
            'y' => out.push_str(&format!("{:02}", parts.year.rem_euclid(100))),
            'j' => out.push_str(&format!("{:03}", parts.yearday)),
            'w' => out.push_str(&format!("{}", (parts.weekday + 1) % 7)),
            _ => {
                out.push('%');
                out.push(spec);
            }
        }
    }
    out
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
        Value::BigInt(value) => !value.is_zero(),
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
        Value::DictKeys(obj) => match &*obj.kind() {
            Object::DictKeysView(view) => match &*view.dict.kind() {
                Object::Dict(values) => !values.is_empty(),
                _ => true,
            },
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
            Object::MemoryView(view) => {
                with_bytes_like_source(&view.source, |values| !values.is_empty()).unwrap_or(true)
            }
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

fn exception_is_named(exception: &Value, name: &str) -> bool {
    match exception {
        Value::Exception(exc) => exc.name == name,
        Value::ExceptionType(exc_name) => exc_name == name,
        _ => false,
    }
}

fn builtin_exception_parent(name: &str) -> Option<&'static str> {
    match name {
        "BaseException" => None,
        "Exception" => Some("BaseException"),
        "GeneratorExit" => Some("BaseException"),
        "SystemExit" => Some("BaseException"),
        "KeyboardInterrupt" => Some("BaseException"),
        "BaseExceptionGroup" => Some("BaseException"),
        "ExceptionGroup" => Some("BaseExceptionGroup"),
        "StopIteration" => Some("Exception"),
        "StopAsyncIteration" => Some("Exception"),
        "AssertionError" => Some("Exception"),
        "AttributeError" => Some("Exception"),
        "IndexError" => Some("Exception"),
        "KeyError" => Some("Exception"),
        "NameError" => Some("Exception"),
        "ImportError" => Some("Exception"),
        "ModuleNotFoundError" => Some("ImportError"),
        "RuntimeError" => Some("Exception"),
        "PythonFinalizationError" => Some("RuntimeError"),
        "_IncompleteInputError" => Some("SyntaxError"),
        "OSError" => Some("Exception"),
        "FileNotFoundError" => Some("OSError"),
        "FileExistsError" => Some("OSError"),
        "IsADirectoryError" => Some("OSError"),
        "BlockingIOError" => Some("OSError"),
        "InterruptedError" => Some("OSError"),
        "ProcessLookupError" => Some("OSError"),
        "ChildProcessError" => Some("OSError"),
        "ConnectionError" => Some("OSError"),
        "BrokenPipeError" => Some("ConnectionError"),
        "ConnectionAbortedError" => Some("ConnectionError"),
        "ConnectionRefusedError" => Some("ConnectionError"),
        "ConnectionResetError" => Some("ConnectionError"),
        "TimeoutError" => Some("OSError"),
        "NotADirectoryError" => Some("OSError"),
        "PermissionError" => Some("OSError"),
        "UnsupportedOperation" => Some("OSError"),
        "TypeError" => Some("Exception"),
        "ValueError" => Some("Exception"),
        "Error" => Some("Exception"),
        "UnicodeError" => Some("ValueError"),
        "UnicodeEncodeError" => Some("UnicodeError"),
        "UnicodeDecodeError" => Some("UnicodeError"),
        "UnicodeTranslateError" => Some("UnicodeError"),
        "EncodingWarning" => Some("Warning"),
        "ZeroDivisionError" => Some("Exception"),
        _ => None,
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

    // Common fast path for plain positional calls (no defaults/kwargs/varargs).
    if kwargs.is_empty()
        && defaults_len == 0
        && kwonly_len == 0
        && func.code.vararg.is_none()
        && func.code.kwarg.is_none()
    {
        if positional.len() != total_positional {
            return Err(RuntimeError::new("argument count mismatch"));
        }
        if posonly_len == 0 {
            return Ok(BoundArguments {
                posonly: Vec::new(),
                positional,
                kwonly: Vec::new(),
                vararg: None,
                kwarg: None,
            });
        }
        let positional_values = positional.split_off(posonly_len);
        return Ok(BoundArguments {
            posonly: positional,
            positional: positional_values,
            kwonly: Vec::new(),
            vararg: None,
            kwarg: None,
        });
    }

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
            if bound[posonly_len + index].is_some() {
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
        .map(|_| heap.alloc_tuple(extra_positional));
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

fn assign_binding(frame: &mut Frame, code: &CodeObject, name: &str, value: Value) {
    if let Some(idx) = code.cellvar_to_index.get(name).copied() {
        if let Some(cell) = frame.cells.get(idx) {
            if let Object::Cell(cell_data) = &mut *cell.kind_mut() {
                cell_data.value = Some(value);
                return;
            }
        }
    }
    if let Some(slot_idx) = code.name_to_index.get(name).copied() {
        if let Some(slot) = frame.fast_locals.get_mut(slot_idx) {
            *slot = Some(value.clone());
        }
        // Fast locals are authoritative; keep dict-style locals sparse.
        if let Some(existing) = frame.locals.get_mut(name) {
            *existing = value;
        }
        return;
    }
    frame.locals.insert(name.to_string(), value);
}

fn apply_bindings(frame: &mut Frame, code: &CodeObject, bindings: BoundArguments, heap: &Heap) {
    for (name, value) in code.posonly_params.iter().zip(bindings.posonly.into_iter()) {
        assign_binding(frame, code, name, value);
    }
    for (name, value) in code.params.iter().zip(bindings.positional.into_iter()) {
        assign_binding(frame, code, name, value);
    }
    for (name, value) in code.kwonly_params.iter().zip(bindings.kwonly.into_iter()) {
        assign_binding(frame, code, name, value);
    }

    if let Some(name) = code.vararg.as_ref() {
        let value = bindings
            .vararg
            .unwrap_or_else(|| heap.alloc_tuple(Vec::new()));
        assign_binding(frame, code, name, value);
    }

    if let Some(name) = code.kwarg.as_ref() {
        let value = bindings
            .kwarg
            .unwrap_or_else(|| heap.alloc_dict(Vec::new()));
        assign_binding(frame, code, name, value);
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
            let _ = kwargs.remove("file");
            let _ = kwargs.remove("flush");
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
        BuiltinFunction::Int => {
            if let Some(base) = kwargs.remove("base") {
                if args.len() > 1 {
                    return Err(RuntimeError::new("int() got multiple values for base"));
                }
                args.push(base);
            }
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "int() got an unexpected keyword argument",
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

            let start_big = value_to_bigint(start)?;
            let stop_big = value_to_bigint(stop)?;
            let step_big = value_to_bigint(step)?;
            if step_big.is_zero() {
                return Err(RuntimeError::new("range() step cannot be zero"));
            }

            Ok(Value::Iterator(heap.alloc(Object::Iterator(
                IteratorObject {
                    kind: IteratorKind::RangeObject {
                        start: start_big,
                        stop: stop_big,
                        step: step_big,
                    },
                    index: 0,
                },
            ))))
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
        BuiltinFunction::Dict => {
            let base = builtin.call(heap, args)?;
            let dict_obj = match base {
                Value::Dict(obj) => obj,
                _ => return Err(RuntimeError::new("dict() internal error")),
            };
            for (name, value) in kwargs {
                dict_set_value_checked(&dict_obj, Value::Str(name), value)?;
            }
            Ok(Value::Dict(dict_obj))
        }
        BuiltinFunction::CollectionsNamedTuple => {
            let _module = kwargs.remove("module");
            let _rename = kwargs.remove("rename");
            let _defaults = kwargs.remove("defaults");
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "namedtuple() got an unexpected keyword argument",
                ));
            }
            builtin.call(heap, args)
        }
        BuiltinFunction::FunctoolsLruCache => {
            let _ = kwargs.remove("maxsize");
            let _ = kwargs.remove("typed");
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "lru_cache() got an unexpected keyword argument",
                ));
            }
            builtin.call(heap, args)
        }
        BuiltinFunction::TypingTypeVar
        | BuiltinFunction::TypingParamSpec
        | BuiltinFunction::TypingTypeVarTuple
        | BuiltinFunction::TypingTypeAliasType => {
            // Keep keyword arguments permissive for typing bootstrap stubs.
            kwargs.clear();
            builtin.call(heap, args)
        }
        BuiltinFunction::TypingIdFunc => {
            if args.is_empty() && !kwargs.is_empty() {
                kwargs.clear();
                Ok(Value::Builtin(BuiltinFunction::TypingIdFunc))
            } else {
                kwargs.clear();
                builtin.call(heap, args)
            }
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

fn collect_noop_symbols_from_module(
    module_name: &str,
    module: &ObjRef,
    out: &mut Vec<String>,
    visited: &mut HashSet<u64>,
) {
    if !visited.insert(module.id()) {
        return;
    }
    let module_kind = module.kind();
    let module_data = match &*module_kind {
        Object::Module(module_data) => module_data,
        _ => return,
    };
    let mut names: Vec<String> = module_data.globals.keys().cloned().collect();
    names.sort();
    for name in names {
        let Some(value) = module_data.globals.get(&name) else {
            continue;
        };
        collect_noop_symbols_from_value(&format!("{module_name}.{name}"), value, out, visited);
    }
}

fn collect_noop_symbols_from_value(
    path: &str,
    value: &Value,
    out: &mut Vec<String>,
    visited: &mut HashSet<u64>,
) {
    match value {
        Value::Builtin(BuiltinFunction::NoOp) => out.push(path.to_string()),
        Value::Module(module) => collect_noop_symbols_from_module(path, module, out, visited),
        Value::Class(class) => {
            if !visited.insert(class.id()) {
                return;
            }
            let class_kind = class.kind();
            let class_data = match &*class_kind {
                Object::Class(class_data) => class_data,
                _ => return,
            };
            let mut attrs: Vec<String> = class_data.attrs.keys().cloned().collect();
            attrs.sort();
            for attr in attrs {
                let Some(attr_value) = class_data.attrs.get(&attr) else {
                    continue;
                };
                collect_noop_symbols_from_value(
                    &format!("{path}.{attr}"),
                    attr_value,
                    out,
                    visited,
                );
            }
        }
        Value::Instance(instance) => {
            if !visited.insert(instance.id()) {
                return;
            }
            let instance_kind = instance.kind();
            let instance_data = match &*instance_kind {
                Object::Instance(instance_data) => instance_data,
                _ => return,
            };
            let mut attrs: Vec<String> = instance_data.attrs.keys().cloned().collect();
            attrs.sort();
            for attr in attrs {
                let Some(attr_value) = instance_data.attrs.get(&attr) else {
                    continue;
                };
                collect_noop_symbols_from_value(
                    &format!("{path}.{attr}"),
                    attr_value,
                    out,
                    visited,
                );
            }
        }
        Value::List(obj) | Value::Tuple(obj) | Value::Set(obj) | Value::FrozenSet(obj) => {
            if !visited.insert(obj.id()) {
                return;
            }
            let kind = obj.kind();
            match &*kind {
                Object::List(values) | Object::Tuple(values) => {
                    for (idx, item) in values.iter().enumerate() {
                        collect_noop_symbols_from_value(
                            &format!("{path}[{idx}]"),
                            item,
                            out,
                            visited,
                        );
                    }
                }
                Object::Set(values) | Object::FrozenSet(values) => {
                    for (idx, item) in values.iter().enumerate() {
                        collect_noop_symbols_from_value(
                            &format!("{path}[{idx}]"),
                            item,
                            out,
                            visited,
                        );
                    }
                }
                _ => {}
            }
        }
        Value::Dict(obj) => {
            if !visited.insert(obj.id()) {
                return;
            }
            let kind = obj.kind();
            let entries = match &*kind {
                Object::Dict(entries) => entries,
                _ => return,
            };
            for (idx, (key, value)) in entries.iter().enumerate() {
                collect_noop_symbols_from_value(&format!("{path}{{key:{idx}}}"), key, out, visited);
                collect_noop_symbols_from_value(
                    &format!("{path}{{value:{idx}}}"),
                    value,
                    out,
                    visited,
                );
            }
        }
        Value::Cell(cell) => {
            if !visited.insert(cell.id()) {
                return;
            }
            let cell_value = match &*cell.kind() {
                Object::Cell(cell_data) => cell_data.value.clone(),
                _ => None,
            };
            if let Some(cell_value) = cell_value {
                collect_noop_symbols_from_value(path, &cell_value, out, visited);
            }
        }
        _ => {}
    }
}

fn decode_call_counts(arg: u32) -> (usize, usize) {
    let pos = (arg & 0xFFFF) as usize;
    let kw = (arg >> 16) as usize;
    (pos, kw)
}

fn module_globals_version(module: &ObjRef) -> u64 {
    match &*module.kind() {
        Object::Module(module_data) => module_data.globals_version,
        _ => 0,
    }
}

fn is_comprehension_code(code: &CodeObject) -> bool {
    code.is_comprehension
}

fn exception_message_from_call_args(args: &[Value]) -> Option<String> {
    if args.is_empty() {
        return None;
    }
    if args.len() == 1 {
        return Some(format_value(&args[0]));
    }
    let parts = args.iter().map(format_value).collect::<Vec<_>>();
    Some(format!("({})", parts.join(", ")))
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

fn class_attr_lookup(class: &ObjRef, name: &str) -> Option<Value> {
    for candidate in class_attr_walk(class) {
        if let Some(value) = class_attr_lookup_direct(&candidate, name) {
            return Some(value);
        }
    }
    None
}

fn class_attr_lookup_direct(class: &ObjRef, name: &str) -> Option<Value> {
    let class_kind = class.kind();
    let Object::Class(class_data) = &*class_kind else {
        return None;
    };
    class_data.attrs.get(name).cloned()
}

fn class_attr_walk(class: &ObjRef) -> Vec<ObjRef> {
    let class_kind = class.kind();
    let class_data = match &*class_kind {
        Object::Class(class_data) => class_data,
        _ => return Vec::new(),
    };

    if !class_data.mro.is_empty() {
        let mut mro = class_data.mro.clone();
        if let Some(object_idx) = mro.iter().position(|entry| {
            matches!(&*entry.kind(), Object::Class(candidate) if candidate.name == "object")
        }) {
            if object_idx + 1 != mro.len() {
                let object_entry = mro.remove(object_idx);
                mro.push(object_entry);
            }
        }
        return mro;
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
    let Object::Class(input_class_data) = &*class.kind() else {
        return None;
    };
    if input_class_data.slots.is_none() {
        return None;
    }

    let mut names = Vec::new();
    for candidate in class_attr_walk(class) {
        if let Object::Class(class_data) = &*candidate.kind() {
            if let Some(slots) = &class_data.slots {
                for slot in slots {
                    if !names.iter().any(|existing| existing == slot) {
                        names.push(slot.clone());
                    }
                }
            }
        }
    }
    Some(names)
}

fn class_inherits_dynamic_instance_dict(class: &ObjRef) -> bool {
    let mro = class_attr_walk(class);
    for candidate in mro.into_iter().skip(1) {
        let Object::Class(class_data) = &*candidate.kind() else {
            continue;
        };
        match &class_data.slots {
            Some(slots) => {
                if slots.iter().any(|name| name == "__dict__") {
                    return true;
                }
            }
            None => {
                if class_data.name != "object" {
                    return true;
                }
            }
        }
    }
    false
}

fn class_of_class(class: &ObjRef) -> Option<ObjRef> {
    match &*class.kind() {
        Object::Class(class_data) => class_data.metaclass.clone(),
        _ => None,
    }
}

fn first_whitespace_run(text: &str) -> Option<(usize, usize)> {
    let mut run_start: Option<usize> = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if run_start.is_none() {
                run_start = Some(idx);
            }
        } else if let Some(start) = run_start {
            return Some((start, idx));
        }
    }
    run_start.map(|start| (start, text.len()))
}

fn last_whitespace_run(text: &str) -> Option<(usize, usize)> {
    let mut run_start: Option<usize> = None;
    let mut last_run: Option<(usize, usize)> = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if run_start.is_none() {
                run_start = Some(idx);
            }
        } else if let Some(start) = run_start.take() {
            last_run = Some((start, idx));
        }
    }
    if let Some(start) = run_start {
        Some((start, text.len()))
    } else {
        last_run
    }
}

fn py_split_whitespace(text: &str, maxsplit: i64) -> Vec<String> {
    if maxsplit < 0 {
        return text
            .split_whitespace()
            .map(|part| part.to_string())
            .collect();
    }

    if maxsplit == 0 {
        let trimmed = text.trim_start_matches(|ch: char| ch.is_whitespace());
        if trimmed.is_empty() {
            return Vec::new();
        }
        return vec![trimmed.to_string()];
    }

    let mut remainder = text.trim_start_matches(|ch: char| ch.is_whitespace());
    if remainder.is_empty() {
        return Vec::new();
    }

    let mut parts = Vec::new();
    let mut splits = 0;
    while splits < maxsplit {
        let Some((run_start, run_end)) = first_whitespace_run(remainder) else {
            break;
        };
        parts.push(remainder[..run_start].to_string());
        remainder = remainder[run_end..].trim_start_matches(|ch: char| ch.is_whitespace());
        splits += 1;
        if remainder.is_empty() {
            break;
        }
    }

    if !remainder.is_empty() {
        parts.push(remainder.to_string());
    }
    parts
}

fn py_rsplit_whitespace(text: &str, maxsplit: i64) -> Vec<String> {
    if maxsplit < 0 {
        return text
            .split_whitespace()
            .map(|part| part.to_string())
            .collect();
    }

    if maxsplit == 0 {
        let trimmed = text.trim_end_matches(|ch: char| ch.is_whitespace());
        if trimmed.is_empty() {
            return Vec::new();
        }
        return vec![trimmed.to_string()];
    }

    let mut remainder = text.trim_end_matches(|ch: char| ch.is_whitespace());
    if remainder.is_empty() {
        return Vec::new();
    }

    let mut parts = Vec::new();
    let mut splits = 0;
    while splits < maxsplit {
        let Some((run_start, run_end)) = last_whitespace_run(remainder) else {
            break;
        };
        parts.push(remainder[run_end..].to_string());
        remainder = remainder[..run_start].trim_end_matches(|ch: char| ch.is_whitespace());
        splits += 1;
        if remainder.is_empty() {
            break;
        }
    }

    if !remainder.is_empty() {
        parts.push(remainder.to_string());
    }
    parts.reverse();
    parts
}

fn py_splitlines(text: &str, keepends: bool) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut chars = text.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        let mut linebreak_end = None;
        if ch == '\r' {
            let mut end = idx + ch.len_utf8();
            if let Some((next_idx, next_ch)) = chars.peek().copied() {
                if next_ch == '\n' {
                    let _ = chars.next();
                    end = next_idx + next_ch.len_utf8();
                }
            }
            linebreak_end = Some(end);
        } else if is_py_splitline_break(ch) {
            linebreak_end = Some(idx + ch.len_utf8());
        }

        if let Some(end) = linebreak_end {
            let line_end = if keepends { end } else { idx };
            parts.push(text[start..line_end].to_string());
            start = end;
        }
    }

    if start < text.len() {
        parts.push(text[start..].to_string());
    }

    parts
}

fn is_py_splitline_break(ch: char) -> bool {
    matches!(
        ch,
        '\n' | '\u{000B}'
            | '\u{000C}'
            | '\u{001C}'
            | '\u{001D}'
            | '\u{001E}'
            | '\u{0085}'
            | '\u{2028}'
            | '\u{2029}'
    )
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

fn is_missing_attribute_error(err: &RuntimeError) -> bool {
    err.message.contains("has no attribute")
        || err.message.contains("AttributeError:")
        || err.message == "attribute access unsupported type"
}

fn exception_type_is_subclass(candidate: &str, expected: &str) -> bool {
    if candidate == expected || expected == "BaseException" {
        return true;
    }
    if expected == "Exception" {
        return candidate != "BaseException";
    }
    if expected == "Warning" && candidate.ends_with("Warning") {
        return true;
    }
    if expected == "UnicodeError"
        && matches!(
            candidate,
            "UnicodeEncodeError" | "UnicodeDecodeError" | "UnicodeTranslateError"
        )
    {
        return true;
    }
    if expected == "ImportError" && candidate == "ModuleNotFoundError" {
        return true;
    }
    if expected == "SyntaxError"
        && matches!(
            candidate,
            "IndentationError" | "TabError" | "_IncompleteInputError"
        )
    {
        return true;
    }
    if expected == "RuntimeError" && candidate == "PythonFinalizationError" {
        return true;
    }
    if expected == "PickleError" && matches!(candidate, "PicklingError" | "UnpicklingError") {
        return true;
    }
    if expected == "OSError"
        && matches!(
            candidate,
            "FileNotFoundError"
                | "BlockingIOError"
                | "TimeoutError"
                | "NotADirectoryError"
                | "PermissionError"
                | "FileExistsError"
                | "IsADirectoryError"
                | "InterruptedError"
                | "ProcessLookupError"
                | "ChildProcessError"
                | "ConnectionError"
                | "BrokenPipeError"
                | "ConnectionAbortedError"
                | "ConnectionRefusedError"
                | "ConnectionResetError"
                | "UnsupportedOperation"
        )
    {
        return true;
    }
    if expected == "ConnectionError"
        && matches!(
            candidate,
            "BrokenPipeError"
                | "ConnectionAbortedError"
                | "ConnectionRefusedError"
                | "ConnectionResetError"
        )
    {
        return true;
    }
    false
}

#[inline]
fn runtime_error_line_matches_exception(line: &str, exception: &str) -> bool {
    if line == exception {
        return true;
    }
    match line.strip_prefix(exception) {
        Some(rest) => rest.starts_with(':'),
        None => false,
    }
}

fn extract_runtime_error_exception_name(message: &str) -> Option<String> {
    let candidate_line = if message.starts_with("Traceback (most recent call last):") {
        message
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
            .unwrap_or("")
    } else {
        message.trim()
    };
    let candidate = candidate_line
        .split_once(':')
        .map(|(name, _)| name.trim())
        .unwrap_or(candidate_line);
    let mut chars = candidate.chars();
    let Some(first) = chars.next() else {
        return None;
    };
    if !(first.is_ascii_uppercase() || first == '_') {
        return None;
    }
    if !candidate
        .chars()
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(candidate.to_string())
}

fn extract_runtime_error_final_message(message: &str, exception: &str) -> Option<Option<String>> {
    if !message.starts_with("Traceback (most recent call last):") {
        return None;
    }
    let line = message
        .lines()
        .rev()
        .find(|entry| !entry.trim().is_empty())
        .map(str::trim)?;
    if !runtime_error_line_matches_exception(line, exception) {
        return None;
    }
    let message = line
        .split_once(':')
        .map(|(_, rest)| rest.trim_start().to_string())
        .filter(|rest| !rest.is_empty());
    Some(message)
}

fn extract_prefixed_exception_message(message: &str, exception: &str) -> Option<Option<String>> {
    let line = message.trim();
    if !runtime_error_line_matches_exception(line, exception) {
        return None;
    }
    let message = line
        .split_once(':')
        .map(|(_, rest)| rest.trim_start().to_string())
        .filter(|rest| !rest.is_empty());
    Some(message)
}

#[inline]
fn is_os_error_family(name: &str) -> bool {
    matches!(
        name,
        "OSError"
            | "FileNotFoundError"
            | "FileExistsError"
            | "PermissionError"
            | "NotADirectoryError"
            | "IsADirectoryError"
    )
}

fn extract_os_error_errno(message: &str) -> Option<i64> {
    let marker = "os error ";
    if let Some(idx) = message.rfind(marker) {
        let tail = &message[idx + marker.len()..];
        let digits: String = tail.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !digits.is_empty() {
            return digits.parse::<i64>().ok();
        }
    }
    let marker = "[Errno ";
    if let Some(idx) = message.find(marker) {
        let tail = &message[idx + marker.len()..];
        let digits: String = tail.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !digits.is_empty() {
            return digits.parse::<i64>().ok();
        }
    }
    None
}

fn infer_os_error_errno(message: &str) -> Option<i64> {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("bad file descriptor") {
        return Some(9);
    }
    if normalized.contains("no such file or directory") {
        return Some(2);
    }
    if normalized.contains("permission denied") {
        return Some(13);
    }
    if normalized.contains("file exists") {
        return Some(17);
    }
    if normalized.contains("not a directory") {
        return Some(20);
    }
    if normalized.contains("is a directory") {
        return Some(21);
    }
    if normalized.contains("invalid argument") {
        return Some(22);
    }
    None
}

fn extract_os_error_strerror(message: &str) -> Option<String> {
    let mut line = message
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(message)
        .trim()
        .to_string();
    if let Some((_, tail)) = line.split_once(": ") {
        line = tail.to_string();
    }
    if let Some(rest) = line.strip_prefix("[Errno ") {
        if let Some((_, tail)) = rest.split_once("] ") {
            line = tail.to_string();
        }
    }
    if let Some(idx) = line.rfind(" (os error ") {
        if line[idx..].ends_with(')') {
            line = line[..idx].to_string();
        }
    }
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    Some(line.to_string())
}

fn extract_import_error_name(message: &str) -> Option<String> {
    let trimmed = message.trim();
    if let Some(start) = trimmed.find("No module named '") {
        let rest = &trimmed[start + "No module named '".len()..];
        if let Some(end) = rest.find('\'') {
            return Some(rest[..end].to_string());
        }
    }
    if let Some(start) = trimmed.find("module '") {
        let rest = &trimmed[start + "module '".len()..];
        if let Some(end) = rest.find('\'') {
            return Some(rest[..end].to_string());
        }
    }
    if let Some(start) = trimmed.find("cannot import name '") {
        let rest = &trimmed[start + "cannot import name '".len()..];
        if let Some(end) = rest.find('\'') {
            return Some(rest[..end].to_string());
        }
    }
    None
}

#[inline]
fn should_refine_os_error(message: &str) -> bool {
    message.contains("open failed:")
        || message.contains("open() failed:")
        || message.contains("mkdir failed:")
        || message.contains("ftruncate failed:")
        || message.contains("chmod failed:")
        || message.contains("access failed:")
        || message.contains("remove failed:")
        || message.contains("rmdir failed:")
        || message.contains("stat failed:")
        || message.contains("lstat failed:")
        || message.contains("scandir failed:")
}

fn classify_runtime_error(message: &str) -> &'static str {
    const DIRECT_PREFIX_EXCEPTIONS: [&str; 24] = [
        "TypeError",
        "ValueError",
        "RuntimeError",
        "PythonFinalizationError",
        "AttributeError",
        "IndexError",
        "KeyError",
        "NameError",
        "ImportError",
        "ModuleNotFoundError",
        "OSError",
        "Error",
        "AssertionError",
        "ZeroDivisionError",
        "StopIteration",
        "StopAsyncIteration",
        "SystemExit",
        "KeyboardInterrupt",
        "LookupError",
        "CalledProcessError",
        "PickleError",
        "PicklingError",
        "UnpicklingError",
        "BufferError",
    ];
    let trimmed = message.trim();
    if message.starts_with("Traceback (most recent call last):") {
        if let Some(last_non_empty_line) = message
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
        {
            for exception in DIRECT_PREFIX_EXCEPTIONS {
                if runtime_error_line_matches_exception(last_non_empty_line, exception) {
                    if exception == "OSError" && should_refine_os_error(last_non_empty_line) {
                        continue;
                    }
                    return exception;
                }
            }
        }
    }
    for exception in DIRECT_PREFIX_EXCEPTIONS {
        if runtime_error_line_matches_exception(message, exception) {
            if exception == "OSError" && should_refine_os_error(message) {
                continue;
            }
            return exception;
        }
    }

    if runtime_error_line_matches_exception(trimmed, "StopIteration") {
        return "StopIteration";
    }
    if runtime_error_line_matches_exception(trimmed, "StopAsyncIteration") {
        return "StopAsyncIteration";
    }
    if runtime_error_line_matches_exception(trimmed, "CalledProcessError") {
        return "CalledProcessError";
    }
    if message.contains("unknown dialect")
        || message.contains("new-line character seen in unquoted field")
        || message.contains("field larger than field limit")
        || message.contains("need to escape")
        || message.contains("unexpected end of data")
        || message.contains("',' expected after '\"'")
        || message.contains("single empty field record must be quoted")
        || message.contains(
            "empty field must be quoted if delimiter is space and skipinitialspace is true",
        )
        || message.starts_with("iterable expected, not ")
        || message.contains("iterator should return strings, not ")
    {
        return "Error";
    }
    if message.contains("csv dialect attributes are read-only") {
        return "AttributeError";
    }
    if message.contains("cannot be a newline")
        || message.contains("cannot be the same")
        || message.contains("cannot be a space when skipinitialspace is true")
        || message.contains("lineterminator cannot contain delimiter, quotechar, or escapechar")
        || message.contains("lineterminator must not be empty")
        || message.contains("bad delimiter value")
        || message.contains("bad quotechar value")
        || message.contains("bad escapechar value")
        || message.contains("bad delimiter or quotechar value")
        || message.contains("bad delimiter or escapechar value")
        || message.contains("bad escapechar or quotechar value")
        || message.contains("bad delimiter or lineterminator value")
        || message.contains("bad quotechar or lineterminator value")
        || message.contains("bad escapechar or lineterminator value")
        || message.contains("not enough values to unpack")
        || message.contains("too many values to unpack")
    {
        return "ValueError";
    }
    if message.contains("must be a unicode character")
        || message.contains("must be a string")
        || message.contains("must be bool")
        || message.contains("missing required argument")
        || message.contains("required positional argument")
        || message.contains("received unexpected arguments")
        || message.contains("unexpected keyword argument")
        || message.contains("expected iterable")
        || message.contains("must have a write method")
        || message.contains("name must be str")
        || message.contains("quotechar must be set if quoting enabled")
        || message.contains("bad \"quoting\" value")
        || message.contains("argument count mismatch")
        || message.contains("decoding str is not supported")
        || message.contains("cannot pickle 'Dialect' instances")
        || message.contains("write() argument must be str")
        || message.contains("attempted to call non-function")
        || message.contains("is not a type object")
    {
        return "TypeError";
    }
    if runtime_error_line_matches_exception(trimmed, "KeyboardInterrupt") {
        return "KeyboardInterrupt";
    }
    if runtime_error_line_matches_exception(trimmed, "UnsupportedOperation") {
        return "UnsupportedOperation";
    }
    if runtime_error_line_matches_exception(trimmed, "SystemExit") {
        return "SystemExit";
    }
    if message.contains("index out of range")
        || message.contains("pop index out of range")
        || message.contains("pop from empty list")
    {
        return "IndexError";
    }
    if message.contains("key not found") {
        return "KeyError";
    }
    if message.contains("division by zero") || message.contains("modulo by zero") {
        return "ZeroDivisionError";
    }
    if message.contains("can't decode bytes")
        || message.contains("codec can't decode byte")
        || message.contains("codec can't decode bytes")
    {
        return "UnicodeDecodeError";
    }
    if message.contains("unknown encoding") {
        return "LookupError";
    }
    if message.contains("Pickler.__init__() was not called by Pickler.__init__") {
        return "PicklingError";
    }
    if message.contains("Unpickler.__init__() was not called by Unpickler.__init__") {
        return "UnpicklingError";
    }
    if message.contains("can't encode character")
        || message.contains("can't encode")
        || message.contains("ordinal not in range")
    {
        return "UnicodeEncodeError";
    }
    if message.starts_with("name '") && message.ends_with("is not defined") {
        return "NameError";
    }
    if message.contains("has no attribute") || message == "attribute access unsupported type" {
        return "AttributeError";
    }
    if message.contains("__init__() should return None") {
        return "TypeError";
    }
    if message.starts_with("module '") && message.ends_with("' not found") {
        return "ModuleNotFoundError";
    }
    if message.starts_with("No module named '") {
        return "ModuleNotFoundError";
    }
    if message.starts_with("cannot import name '") && message.contains("' from '") {
        return "ImportError";
    }
    if message.contains("attempted relative import with no known parent package") {
        return "ImportError";
    }
    if message.contains("metaclass conflict") {
        return "TypeError";
    }
    if message.starts_with("cannot create '") && message.ends_with("' instances") {
        return "TypeError";
    }
    if message.contains("object is not iterable")
        || message.contains("argument is not iterable")
        || message.contains("__iter__() returned non-iterator")
    {
        return "TypeError";
    }
    if message.contains("math domain error")
        || message.contains("tolerances must be non-negative")
        || message.contains("inputs are not the same length")
        || message.contains("not in list")
        || message.contains("substring not found")
        || message.contains("invalid literal for int")
        || message.contains("int() invalid literal")
        || message.contains("could not convert string to float")
        || message.contains("complex() invalid literal")
        || message.contains("list modified during sort")
        || message.starts_with("invalid mode:")
        || message.contains("must have exactly one of create/read/write/append mode")
        || message.contains("can't have text and binary mode at once")
        || message.contains("can't have unbuffered text I/O")
        || message.contains("invalid buffering size")
        || message.contains("binary mode doesn't take an encoding argument")
        || message.contains("binary mode doesn't take an errors argument")
        || message.contains("binary mode doesn't take a newline argument")
        || message.contains("I/O operation on closed file")
        || message.contains("Cannot use closefd=False with file name")
        || message.starts_with("opener returned ")
    {
        return "ValueError";
    }
    if message.contains("open failed:") {
        if message.contains("File exists") || message.contains("os error 17") {
            return "FileExistsError";
        }
        if message.contains("No such file or directory") || message.contains("os error 2") {
            return "FileNotFoundError";
        }
        if message.contains("Permission denied") || message.contains("os error 13") {
            return "PermissionError";
        }
        if message.contains("Is a directory") || message.contains("os error 21") {
            return "IsADirectoryError";
        }
        if message.contains("Not a directory") || message.contains("os error 20") {
            return "NotADirectoryError";
        }
    }
    if message.contains("open() failed:") {
        if message.contains("File exists") || message.contains("os error 17") {
            return "FileExistsError";
        }
        if message.contains("No such file or directory") || message.contains("os error 2") {
            return "FileNotFoundError";
        }
        if message.contains("Permission denied") || message.contains("os error 13") {
            return "PermissionError";
        }
        if message.contains("Is a directory") || message.contains("os error 21") {
            return "IsADirectoryError";
        }
        if message.contains("Not a directory") || message.contains("os error 20") {
            return "NotADirectoryError";
        }
    }
    if message.contains("mkdir failed:") {
        if message.contains("File exists") || message.contains("os error 17") {
            return "FileExistsError";
        }
        if message.contains("No such file or directory") || message.contains("os error 2") {
            return "FileNotFoundError";
        }
        if message.contains("Permission denied") || message.contains("os error 13") {
            return "PermissionError";
        }
        if message.contains("Not a directory") || message.contains("os error 20") {
            return "NotADirectoryError";
        }
    }
    if message.contains("access failed:") {
        if message.contains("No such file or directory") || message.contains("os error 2") {
            return "FileNotFoundError";
        }
        if message.contains("Permission denied") || message.contains("os error 13") {
            return "PermissionError";
        }
    }
    if message.contains("chmod failed:") {
        if message.contains("No such file or directory") || message.contains("os error 2") {
            return "FileNotFoundError";
        }
        if message.contains("Permission denied") || message.contains("os error 13") {
            return "PermissionError";
        }
        if message.contains("Not a directory") || message.contains("os error 20") {
            return "NotADirectoryError";
        }
    }
    if message.contains("rmdir failed:") {
        if message.contains("No such file or directory") || message.contains("os error 2") {
            return "FileNotFoundError";
        }
        if message.contains("Permission denied") || message.contains("os error 13") {
            return "PermissionError";
        }
        if message.contains("Not a directory") || message.contains("os error 20") {
            return "NotADirectoryError";
        }
    }
    if message.contains("remove failed:") {
        if message.contains("No such file or directory") || message.contains("os error 2") {
            return "FileNotFoundError";
        }
        if message.contains("Is a directory") || message.contains("os error 21") {
            return "IsADirectoryError";
        }
        if message.contains("Not a directory") || message.contains("os error 20") {
            return "NotADirectoryError";
        }
        if message.contains("Permission denied")
            || message.contains("os error 13")
            || message.contains("Operation not permitted")
            || message.contains("os error 1")
        {
            return "PermissionError";
        }
    }
    if message.contains("bad file descriptor")
        || message.contains("open failed:")
        || message.contains("open() failed:")
        || message.contains("mkdir failed:")
        || message.contains("chmod failed:")
        || message.contains("access failed:")
        || message.contains("remove failed:")
        || message.contains("ftruncate failed:")
        || message.contains("close failed:")
        || message.contains("stat failed:")
        || message.contains("lstat failed:")
        || message.contains("rmdir failed:")
        || message.contains("utime failed:")
        || message.contains("scandir failed:")
    {
        return "OSError";
    }
    if message.contains("__len__() should return >= 0") {
        return "ValueError";
    }
    if message.contains("__bool__ should return bool")
        || message.contains("object cannot be interpreted as an integer")
    {
        return "TypeError";
    }
    if message.contains("unsupported operand type") || message.contains("expects") {
        return "TypeError";
    }
    "RuntimeError"
}

fn is_runtime_type_name_marker(name: &str) -> bool {
    matches!(
        name,
        "NoneType"
            | "dict_keys"
            | "iterator"
            | "generator"
            | "async_generator"
            | "coroutine"
            | "module"
            | "method"
            | "function"
            | "cell"
            | "code"
            | "super"
            | "object"
    )
}

fn runtime_error_matches_exception(message: &str, expected: &str) -> bool {
    let classified = classify_runtime_error(message);
    if classified == expected || exception_type_is_subclass(classified, expected) {
        return true;
    }
    if let Some(exception_name) = extract_runtime_error_exception_name(message) {
        if exception_name == expected || exception_type_is_subclass(&exception_name, expected) {
            return true;
        }
    }
    let Some(last_non_empty_line) = message
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
    else {
        return false;
    };
    last_non_empty_line == expected || last_non_empty_line.starts_with(&format!("{expected}:"))
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

fn slice_bounds_for_step_one(len: usize, lower: Option<i64>, upper: Option<i64>) -> (usize, usize) {
    let len_isize = len as isize;
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

    (start as usize, stop as usize)
}

fn opcode_flags_contains(flags: &str, target: &str) -> bool {
    flags.split('|').any(|part| part.trim() == target)
}
