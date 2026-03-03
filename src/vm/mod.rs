//! Bytecode VM runtime and import substrate.
//!
//! This module owns the interpreter process state (`Vm`), per-activation execution
//! state (`Frame`), and the call/import/C-API registries that must remain coherent
//! with CPython 3.14 semantics.

mod builtins_collections;
mod builtins_core;
mod builtins_import;
mod builtins_io;
mod builtins_numeric_time;
mod builtins_os;
mod builtins_system_misc;
mod capi_registry;
mod containers;
mod ops;
mod stdlib;
mod vm_bootstrap_import;
mod vm_builtin_metadata;
mod vm_execution;
mod vm_extensions;
mod vm_native_dispatch;
mod vm_runtime_methods;
#[cfg(target_arch = "wasm32")]
mod wasm_c_float_format;
#[cfg(target_arch = "wasm32")]
mod wasm_libc_shim;

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::ffi::{CString, OsString, c_void};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{IsTerminal, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
use js_sys::Date;

use self::capi_registry::{CapiObjectRegistry, CapiPtrProvenance, CapiRefKind};
use self::containers::{
    dict_contains_key_checked, dict_get_value, dict_remove_value, dict_set_value,
    dict_set_value_checked, ensure_hashable,
};
use self::ops::{
    add_values, and_values, compare_ge, compare_gt, compare_in, compare_le, compare_lt,
    compare_order, div_values, floor_div_values, invert_value, lshift_values, matmul_values,
    mod_values, mul_values, neg_value, or_values, ordering_from_cmp_value, pos_value, pow_values,
    rshift_values, sub_values, xor_values,
};
use self::stdlib::bz2::{Bz2CompressorState, Bz2DecompressorState};
use self::stdlib::expat::ExpatParserState;
use self::stdlib::hashlib::{HashState, HmacState};
use self::stdlib::lzma::{LzmaCompressorState, LzmaDecompressorState};
use self::stdlib::sqlite3::{SqliteBlobState, SqliteConnectionState, SqliteCursorState};
use self::stdlib::zlib::{ZlibCompressObjectState, ZlibDecompressObjectState};
use crate::bytecode::cpython;
use crate::bytecode::metadata::OpcodeMetadata;
use crate::bytecode::{CodeObject, Instruction, Opcode};
use crate::compiler;
use crate::extensions::{
    PyrsCFunctionKwV1, PyrsCFunctionV1, PyrsCapsuleDestructorV1, PyrsModuleStateFinalizeV1,
    PyrsModuleStateFreeV1, SharedLibraryHandle,
};
use crate::host::{HostCapability, NativeHost, VmHost};
use crate::parser;
use crate::runtime::{
    BigInt, BoundMethod, BuiltinFunction, ClassObject, ExceptionObject, FunctionObject,
    GeneratorObject, Heap, InstanceObject, IteratorKind, IteratorObject, MemoryViewObject,
    ModuleObject, NativeMethodKind, NativeMethodObject, Obj, ObjRef, Object, RuntimeError,
    SuperObject, Value, format_repr, format_value,
};

#[derive(Debug, Clone)]
struct Block {
    handler: usize,
    stack_len: usize,
}

unsafe extern "C" {
    fn free(ptr: *mut c_void);
}

#[cfg(target_arch = "wasm32")]
type VmExecutionDeadline = f64;
#[cfg(not(target_arch = "wasm32"))]
type VmExecutionDeadline = Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceFrame {
    frame_id: usize,
    filename: String,
    line: usize,
    column: usize,
    end_line: usize,
    end_column: usize,
    lasti: usize,
    name: String,
    locals: Vec<String>,
    local_values: Vec<(String, Value)>,
    globals: Vec<String>,
    self_local: Option<Value>,
}

#[derive(Clone)]
struct ModuleSourceInfo {
    path: PathBuf,
    is_package: bool,
    package_dirs: Vec<PathBuf>,
    is_namespace: bool,
    is_bytecode: bool,
    is_extension: bool,
}

#[derive(Clone)]
enum ExtensionCallableKind {
    Positional(PyrsCFunctionV1),
    WithKeywords(PyrsCFunctionKwV1),
    CpythonMethod { method_def: usize },
}

#[derive(Clone)]
struct ExtensionCallableEntry {
    module: ObjRef,
    name: String,
    kind: ExtensionCallableKind,
}

#[derive(Clone, Copy)]
struct ExtensionCapsuleRegistryEntry {
    pointer: usize,
    context: usize,
    destructor: Option<PyrsCapsuleDestructorV1>,
}

#[derive(Clone, Copy)]
struct ExtensionModuleStateEntry {
    state: usize,
    free_func: Option<PyrsModuleStateFreeV1>,
    finalize_func: Option<PyrsModuleStateFinalizeV1>,
}

#[derive(Default, Clone, Copy)]
struct ImportPerfCounters {
    fs_source_compiles: u64,
    pyc_load_attempts: u64,
    pyc_load_fallback_to_source: u64,
}

#[derive(Clone)]
struct ImportDirCacheEntry {
    mtime_ns: Option<u128>,
    entries: HashSet<OsString>,
}

#[derive(Clone)]
struct AtexitHandler {
    callable: Value,
    args: Vec<Value>,
    kwargs: HashMap<String, Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompiledCodeMode {
    Exec,
    Eval,
    Single,
}

#[derive(Clone, Debug)]
pub(super) struct CompiledCodeMetadata {
    pub(super) mode: CompiledCodeMode,
    pub(super) source: Option<String>,
}

const DEFAULT_META_PATH_FINDER: &str = "pyrs.PathFinder";
const DEFAULT_PATH_HOOK: &str = "pyrs.FileFinder";
const SOURCE_FILE_LOADER: &str = "pyrs.SourceFileLoader";
const SOURCELESS_FILE_LOADER: &str = "pyrs.SourcelessFileLoader";
const NAMESPACE_LOADER: &str = "pyrs.NamespaceLoader";
const EXTENSION_FILE_LOADER: &str = "pyrs.ExtensionFileLoader";
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
const PURE_STDLIB_COLLECTIONS_MODULES: &[&str] = &["collections", "collections.abc"];
const PURE_STDLIB_FUNCTOOLS_MODULES: &[&str] = &["functools"];
const PURE_STDLIB_FUTURE_MODULES: &[&str] = &["__future__"];
const PURE_STDLIB_ABC_MODULES: &[&str] = &["abc"];
const PURE_STDLIB_OPERATOR_MODULES: &[&str] = &["operator"];
const PURE_STDLIB_SIGNAL_MODULES: &[&str] = &["signal"];
const PURE_STDLIB_INSPECT_MODULES: &[&str] = &["inspect"];
const PURE_STDLIB_IO_MODULES: &[&str] = &["io"];
const PURE_STDLIB_DECIMAL_MODULES: &[&str] = &["decimal"];
const PURE_STDLIB_PATHLIB_MODULES: &[&str] = &["pathlib"];
const PURE_STDLIB_TYPES_MODULES: &[&str] = &["types", "typing"];
const PURE_STDLIB_WEAKREF_MODULES: &[&str] = &["weakref"];
const MT_N: usize = 624;
const MT_M: usize = 397;
const MT_MATRIX_A: u32 = 0x9908_b0df;
const MT_UPPER_MASK: u32 = 0x8000_0000;
const MT_LOWER_MASK: u32 = 0x7fff_ffff;
const SIGNAL_DEFAULT: i64 = 0;
const SIGNAL_IGNORE: i64 = 1;
const SIGNAL_SIGINT: i64 = 2;
const SIGNAL_SIGTERM: i64 = 15;
const SYNTHETIC_THREAD_IDENT_START: i64 = 1_i64 << 60;
const PY_TPFLAGS_DISALLOW_INSTANTIATION: i64 = 1 << 7;
const PY_TPFLAGS_IMMUTABLETYPE: i64 = 1 << 8;
const PY_TPFLAGS_HEAPTYPE: i64 = 1 << 9;
const GC_DEFAULT_THRESHOLD0: usize = 700;
const GC_DEFAULT_THRESHOLD1: usize = 10;
const GC_DEFAULT_THRESHOLD2: usize = 10;
const GC_AUTO_CHECK_INTERVAL: usize = 64;
const LIST_BACKING_STORAGE_ATTR: &str = "__pyrs_list_storage__";
const DEQUE_BACKING_STORAGE_ATTR: &str = "__pyrs_deque_storage__";
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
const MAPPING_PROXY_STORAGE_ATTR: &str = "__pyrs_mappingproxy_storage__";
static MONOTONIC_START: OnceLock<Instant> = OnceLock::new();
static OPCODE_METADATA: OnceLock<OpcodeMetadata> = OnceLock::new();
static SUBMODULE_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
static NEXT_VM_FRAME_ID: AtomicUsize = AtomicUsize::new(1);

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
    fused_direct_code: Option<Rc<CodeObject>>,
    fused_direct_module: Option<ObjRef>,
    fused_direct_owner_class: Option<ObjRef>,
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, allow(dead_code))]
struct FusedDirectOneArgNoCellsMetadata {
    func: ObjRef,
    func_epoch: u64,
    code: Rc<CodeObject>,
    module: ObjRef,
    owner_class: Option<ObjRef>,
}

#[derive(Clone, Copy)]
#[cfg_attr(debug_assertions, allow(dead_code))]
struct LoadFastSiteCacheEntry {
    compare_rhs_int: i64,
    jump_target: usize,
}

#[derive(Clone)]
enum LoadAttrSiteCacheKind {
    InstanceValue { value: Value },
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
    CallFunctionZeroArg,
    CallFunctionOneArg,
    CallFunctionTwoArg,
}

const LOGGING_PERCENT_VALIDATION_PATTERN: &str =
    r"%\(\w+\)[#0+ -]*(\*|\d+)?(\.(\*|\d+))?[diouxefgcrsa%]";
const PKGUTIL_RESOLVE_NAME_PATTERN: &str =
    r"^(?P<pkg>(?!\d)(\w+)(\.(?!\d)(\w+))*)(?P<cln>:(?P<obj>(?!\d)(\w+)(\.(?!\d)(\w+))*)?)?$";
const LOCAL_SHIM_MODULES: &[&str] = &["_ctypes"];
const LOCAL_SHIM_PRECEDENCE_MODULES: &[&str] = &["_ctypes"];
const MONITORING_MAX_USER_TOOL_ID: i64 = 5;
const MONITORING_EVENT_PY_START: i64 = 1 << 0;
const MONITORING_EVENT_PY_RESUME: i64 = 1 << 1;
const MONITORING_EVENT_PY_RETURN: i64 = 1 << 2;
const MONITORING_EVENT_PY_YIELD: i64 = 1 << 3;
const MONITORING_EVENT_CALL: i64 = 1 << 4;
const MONITORING_EVENT_LINE: i64 = 1 << 5;
const MONITORING_EVENT_INSTRUCTION: i64 = 1 << 6;
const MONITORING_EVENT_JUMP: i64 = 1 << 7;
const MONITORING_EVENT_BRANCH_LEFT: i64 = 1 << 8;
const MONITORING_EVENT_BRANCH_RIGHT: i64 = 1 << 9;
const MONITORING_EVENT_STOP_ITERATION: i64 = 1 << 10;
const MONITORING_EVENT_RAISE: i64 = 1 << 11;
const MONITORING_EVENT_EXCEPTION_HANDLED: i64 = 1 << 12;
const MONITORING_EVENT_PY_UNWIND: i64 = 1 << 13;
const MONITORING_EVENT_PY_THROW: i64 = 1 << 14;
const MONITORING_EVENT_RERAISE: i64 = 1 << 15;
const MONITORING_EVENT_C_RETURN: i64 = 1 << 16;
const MONITORING_EVENT_C_RAISE: i64 = 1 << 17;
const MONITORING_EVENT_BRANCH: i64 = 1 << 18;
const MONITORING_EVENT_SET_MAX: i64 = 1 << 19;
const MONITORING_LOCAL_EVENT_SET_MAX: i64 = 1 << 11;

thread_local! {
    static VM_THREAD_IDENT_OVERRIDE: Cell<Option<i64>> = const { Cell::new(None) };
}

pub(super) fn vm_os_thread_ident() -> i64 {
    let mut hasher = DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    (hasher.finish() & i64::MAX as u64) as i64
}

pub(super) fn vm_current_thread_ident() -> i64 {
    VM_THREAD_IDENT_OVERRIDE
        .with(|slot| slot.get())
        .unwrap_or_else(vm_os_thread_ident)
}

const ENV_VAR_PRESENCE_PROBES: &[&str] = &[
    "PYRS_IMPORT_PERF_VERBOSE",
    "PYRS_TRACE_ASSERT_RAISE",
    "PYRS_TRACE_BUILD_CLASS",
    "PYRS_TRACE_CHECK_EXC",
    "PYRS_TRACE_CLASS_BASE",
    "PYRS_TRACE_CPY_COMPARE",
    "PYRS_TRACE_CPY_COMPARE_ERRORS",
    "PYRS_TRACE_CPY_RICH_VALUES",
    "PYRS_TRACE_CPY_STRING_EQ",
    "PYRS_TRACE_CPY_UNKNOWN_PTR",
    "PYRS_TRACE_DELETE_ATTR",
    "PYRS_TRACE_DICT_MERGE",
    "PYRS_TRACE_EXCEPTION_TABLE",
    "PYRS_TRACE_FAST_CELL",
    "PYRS_TRACE_FAST_LOCAL_UNBOUND",
    "PYRS_TRACE_FOR_ITER_FAIL",
    "PYRS_TRACE_IMPORT_PENDING",
    "PYRS_TRACE_INIT_SUBCLASS",
    "PYRS_TRACE_NUMPY_CORE_IMPORTFROM",
    "PYRS_TRACE_NUMPY_DTYPE_RESOLVE",
    "PYRS_TRACE_STARTSWITH_ATTR",
    "PYRS_TRACE_STORE_ATTR",
    "PYRS_TRACE_STORE_SUBSCRIPT",
    "PYRS_TRACE_SUBSCRIPT",
    "PYRS_TRACE_SUBSCRIPT_ERROR",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvPresenceProbeSource {
    Uninitialized = 0,
    Native = 1,
    Unsupported = 2,
}

impl EnvPresenceProbeSource {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Native,
            2 => Self::Unsupported,
            _ => Self::Uninitialized,
        }
    }

    fn from_host(host: &dyn VmHost) -> Self {
        if host.supports(HostCapability::EnvironmentRead) {
            Self::Native
        } else {
            Self::Unsupported
        }
    }
}

static ENV_PRESENCE_PROBE_SOURCE: AtomicU8 =
    AtomicU8::new(EnvPresenceProbeSource::Uninitialized as u8);

#[inline]
fn configure_env_presence_probe_source(host: &dyn VmHost) {
    let source = EnvPresenceProbeSource::from_host(host);
    ENV_PRESENCE_PROBE_SOURCE.store(source as u8, AtomicOrdering::Relaxed);
}

#[inline]
fn env_probe_source() -> EnvPresenceProbeSource {
    EnvPresenceProbeSource::from_raw(ENV_PRESENCE_PROBE_SOURCE.load(AtomicOrdering::Relaxed))
}

#[inline]
fn host_env_var_present(name: &str) -> bool {
    match env_probe_source() {
        EnvPresenceProbeSource::Unsupported => false,
        EnvPresenceProbeSource::Native | EnvPresenceProbeSource::Uninitialized => {
            let host = NativeHost;
            host.env_var_os(name).is_some()
        }
    }
}

#[inline]
fn is_known_env_presence_probe(name: &'static str) -> bool {
    matches!(
        name,
        "PYRS_IMPORT_PERF_VERBOSE"
            | "PYRS_TRACE_ASSERT_RAISE"
            | "PYRS_TRACE_BUILD_CLASS"
            | "PYRS_TRACE_CHECK_EXC"
            | "PYRS_TRACE_CLASS_BASE"
            | "PYRS_TRACE_CPY_COMPARE"
            | "PYRS_TRACE_CPY_COMPARE_ERRORS"
            | "PYRS_TRACE_CPY_RICH_VALUES"
            | "PYRS_TRACE_CPY_STRING_EQ"
            | "PYRS_TRACE_CPY_UNKNOWN_PTR"
            | "PYRS_TRACE_DELETE_ATTR"
            | "PYRS_TRACE_DICT_MERGE"
            | "PYRS_TRACE_EXCEPTION_TABLE"
            | "PYRS_TRACE_FAST_CELL"
            | "PYRS_TRACE_FAST_LOCAL_UNBOUND"
            | "PYRS_TRACE_FOR_ITER_FAIL"
            | "PYRS_TRACE_IMPORT_PENDING"
            | "PYRS_TRACE_INIT_SUBCLASS"
            | "PYRS_TRACE_NUMPY_CORE_IMPORTFROM"
            | "PYRS_TRACE_NUMPY_DTYPE_RESOLVE"
            | "PYRS_TRACE_STARTSWITH_ATTR"
            | "PYRS_TRACE_STORE_ATTR"
            | "PYRS_TRACE_STORE_SUBSCRIPT"
            | "PYRS_TRACE_SUBSCRIPT"
            | "PYRS_TRACE_SUBSCRIPT_ERROR"
    )
}

#[inline]
fn env_var_present_once(name: &'static str, slot: &'static OnceLock<bool>) -> bool {
    *slot.get_or_init(|| host_env_var_present(name))
}

#[derive(Debug, Clone, Copy, Default)]
struct VmTraceFlags {
    import_perf_verbose: bool,
    assert_raise: bool,
    build_class: bool,
    closure_shape: bool,
    check_exc: bool,
    class_base: bool,
    debug_call_builtin_depth: bool,
    delete_attr: bool,
    disable_pending_finalizers: bool,
    dict_merge: bool,
    exception_table: bool,
    fast_cell: bool,
    fast_local_unbound: bool,
    for_iter_fail: bool,
    getitem_entry: bool,
    getitem_index: bool,
    getitem_unsupported: bool,
    import_pending: bool,
    init_subclass: bool,
    load_special: bool,
    native_call_depth: bool,
    native_kw_reject: bool,
    numpy_core_importfrom: bool,
    prepare_call: bool,
    startswith_attr: bool,
    store_attr: bool,
    store_subscript: bool,
    subscript: bool,
    subscript_error: bool,
}

#[derive(Debug, Clone, Default)]
struct VmTraceTextFilters {
    module_return_ip: Option<String>,
    unwind: Option<String>,
}

impl VmTraceTextFilters {
    fn from_host(host: &dyn VmHost) -> Self {
        Self {
            module_return_ip: host.env_var("PYRS_TRACE_MODULE_RETURN_IP"),
            unwind: host.env_var("PYRS_TRACE_UNWIND"),
        }
    }
}

impl VmTraceFlags {
    fn from_host(host: &dyn VmHost) -> Self {
        Self {
            import_perf_verbose: host.env_var_os("PYRS_IMPORT_PERF_VERBOSE").is_some(),
            assert_raise: host.env_var_os("PYRS_TRACE_ASSERT_RAISE").is_some(),
            build_class: host.env_var_os("PYRS_TRACE_BUILD_CLASS").is_some(),
            closure_shape: host.env_var_os("PYRS_TRACE_CLOSURE_SHAPE").is_some(),
            check_exc: host.env_var_os("PYRS_TRACE_CHECK_EXC").is_some(),
            class_base: host.env_var_os("PYRS_TRACE_CLASS_BASE").is_some(),
            debug_call_builtin_depth: host.env_var_os("PYRS_DEBUG_CALL_BUILTIN_DEPTH").is_some(),
            delete_attr: host.env_var_os("PYRS_TRACE_DELETE_ATTR").is_some(),
            disable_pending_finalizers: host
                .env_var_os("PYRS_DISABLE_PENDING_FINALIZERS")
                .is_some(),
            dict_merge: host.env_var_os("PYRS_TRACE_DICT_MERGE").is_some(),
            exception_table: host.env_var_os("PYRS_TRACE_EXCEPTION_TABLE").is_some(),
            fast_cell: host.env_var_os("PYRS_TRACE_FAST_CELL").is_some(),
            fast_local_unbound: host.env_var_os("PYRS_TRACE_FAST_LOCAL_UNBOUND").is_some(),
            for_iter_fail: host.env_var_os("PYRS_TRACE_FOR_ITER_FAIL").is_some(),
            getitem_entry: host.env_var_os("PYRS_TRACE_GETITEM_ENTRY").is_some(),
            getitem_index: host.env_var_os("PYRS_TRACE_GETITEM_INDEX").is_some(),
            getitem_unsupported: host.env_var_os("PYRS_TRACE_GETITEM_UNSUPPORTED").is_some(),
            import_pending: host.env_var_os("PYRS_TRACE_IMPORT_PENDING").is_some(),
            init_subclass: host.env_var_os("PYRS_TRACE_INIT_SUBCLASS").is_some(),
            load_special: host.env_var_os("PYRS_TRACE_LOAD_SPECIAL").is_some(),
            native_call_depth: host.env_var_os("PYRS_TRACE_NATIVE_CALL_DEPTH").is_some(),
            native_kw_reject: host.env_var_os("PYRS_TRACE_NATIVE_KW_REJECT").is_some(),
            numpy_core_importfrom: host
                .env_var_os("PYRS_TRACE_NUMPY_CORE_IMPORTFROM")
                .is_some(),
            prepare_call: host.env_var_os("PYRS_TRACE_PREPARE_CALL").is_some(),
            startswith_attr: host.env_var_os("PYRS_TRACE_STARTSWITH_ATTR").is_some(),
            store_attr: host.env_var_os("PYRS_TRACE_STORE_ATTR").is_some(),
            store_subscript: host.env_var_os("PYRS_TRACE_STORE_SUBSCRIPT").is_some(),
            subscript: host.env_var_os("PYRS_TRACE_SUBSCRIPT").is_some(),
            subscript_error: host.env_var_os("PYRS_TRACE_SUBSCRIPT_ERROR").is_some(),
        }
    }
}

fn env_var_present_cached(name: &'static str) -> bool {
    static ANY_PROBE_ENABLED: OnceLock<bool> = OnceLock::new();
    let any_probe_enabled = *ANY_PROBE_ENABLED.get_or_init(|| {
        ENV_VAR_PRESENCE_PROBES
            .iter()
            .any(|probe| host_env_var_present(probe))
    });
    if !any_probe_enabled && is_known_env_presence_probe(name) {
        return false;
    }
    match name {
        "PYRS_IMPORT_PERF_VERBOSE" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_ASSERT_RAISE" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_BUILD_CLASS" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_CHECK_EXC" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_CLASS_BASE" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_CPY_COMPARE" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_CPY_COMPARE_ERRORS" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_CPY_RICH_VALUES" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_CPY_STRING_EQ" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_CPY_UNKNOWN_PTR" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_DELETE_ATTR" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_DICT_MERGE" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_EXCEPTION_TABLE" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_FAST_CELL" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_FAST_LOCAL_UNBOUND" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_FOR_ITER_FAIL" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_IMPORT_PENDING" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_INIT_SUBCLASS" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_NUMPY_CORE_IMPORTFROM" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_NUMPY_DTYPE_RESOLVE" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_STARTSWITH_ATTR" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_STORE_ATTR" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_STORE_SUBSCRIPT" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_SUBSCRIPT" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        "PYRS_TRACE_SUBSCRIPT_ERROR" => {
            static SLOT: OnceLock<bool> = OnceLock::new();
            env_var_present_once(name, &SLOT)
        }
        _ => host_env_var_present(name),
    }
}

/// Per-activation interpreter state for one executing `CodeObject`.
///
/// Invariants:
/// - `fast_locals` is authoritative for names mapped by `code.name_to_index`.
/// - `locals` is sparse fallback storage for names without fast slots.
/// - transient control-flow state (`stack`, `blocks`, class build fields,
///   active exception slots, generator resume fields) must be empty before
///   a frame is returned to frame pools.
struct Frame {
    frame_id: usize,
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
    class_namespace: Option<Value>,
    class_bases: Vec<ObjRef>,
    class_orig_bases: Option<Value>,
    class_metaclass: Option<Value>,
    class_keywords: HashMap<String, Value>,
    blocks: Vec<Block>,
    active_exception: Option<Value>,
    except_star_match_lasti: Option<usize>,
    reraise_lasti_override: Option<usize>,
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
            frame_id: NEXT_VM_FRAME_ID.fetch_add(1, AtomicOrdering::Relaxed),
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
            class_namespace: None,
            class_bases: Vec::new(),
            class_orig_bases: None,
            class_metaclass: None,
            class_keywords: HashMap::new(),
            blocks: Vec::with_capacity(2),
            active_exception: None,
            except_star_match_lasti: None,
            reraise_lasti_override: None,
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
        debug_assert!(self.class_orig_bases.is_none());
        debug_assert!(self.class_metaclass.is_none());
        debug_assert!(self.class_namespace.is_none());
        debug_assert!(self.active_exception.is_none());
        debug_assert!(self.except_star_match_lasti.is_none());
        debug_assert!(self.reraise_lasti_override.is_none());
        debug_assert!(self.generator_owner.is_none());
        debug_assert!(self.generator_resume_value.is_none());
        debug_assert!(self.generator_pending_throw.is_none());
        debug_assert!(self.generator_resume_kind.is_none());
        debug_assert!(self.yield_from_iter.is_none());
        self.code = code;
        self.ip = 0;
        self.last_ip = 0;
        self.except_star_match_lasti = None;
        self.reraise_lasti_override = None;
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
            self.except_star_match_lasti = None;
            self.reraise_lasti_override = None;
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
        debug_assert!(self.class_orig_bases.is_none());
        debug_assert!(self.class_metaclass.is_none());
        debug_assert!(self.class_namespace.is_none());
        debug_assert!(self.generator_owner.is_none());
        debug_assert!(self.generator_resume_value.is_none());
        debug_assert!(self.generator_pending_throw.is_none());
        debug_assert!(self.generator_resume_kind.is_none());
        debug_assert!(self.yield_from_iter.is_none());
        debug_assert!(self.active_exception.is_none());
        debug_assert!(self.except_star_match_lasti.is_none());
        debug_assert!(self.reraise_lasti_override.is_none());
        self.code = code.clone();
        self.ip = 0;
        self.last_ip = 0;
        self.except_star_match_lasti = None;
        self.reraise_lasti_override = None;
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

/// Root interpreter state for a single pyrs process.
///
/// `Vm` owns heap/runtime objects, active and pooled frames, import/module state,
/// extension/C-API registries, and subsystem-specific native state used by stdlib
/// shims. Changes to caches or registries here must preserve CPython-visible
/// behavior first; performance fast paths are secondary.
pub struct Vm {
    host: Arc<dyn VmHost>,
    frames: Vec<Box<Frame>>,
    frame_pool: Vec<Box<Frame>>,
    simple_frame_pool: Vec<Box<Frame>>,
    simple_slot0_pool: Vec<Box<Frame>>,
    simple_slot0_pool_key: Option<(usize, u64)>,
    frame_proxy_cache: Vec<ObjRef>,
    frame_proxy_cache_key: Option<Vec<usize>>,
    builtins: HashMap<String, Value>,
    modules: HashMap<String, ObjRef>,
    main_module: ObjRef,
    module_paths: Vec<PathBuf>,
    module_source_positive_cache: HashMap<(PathBuf, String), ModuleSourceInfo>,
    source_text_cache: HashMap<String, Vec<String>>,
    compiled_code_metadata: HashMap<usize, CompiledCodeMetadata>,
    import_dir_cache: HashMap<PathBuf, ImportDirCacheEntry>,
    preferred_filesystem_module_cache: HashMap<String, bool>,
    import_sys_path_signature: u64,
    import_meta_path_signature: u64,
    import_path_hooks_signature: u64,
    import_meta_path_has_default_finder: bool,
    import_path_hooks_has_default_hook: bool,
    import_perf_enabled: bool,
    trace_flags: VmTraceFlags,
    trace_text_filters: VmTraceTextFilters,
    import_perf_counters: ImportPerfCounters,
    heap: Heap,
    random: Mt19937,
    generator_states: HashMap<u64, Box<Frame>>,
    generator_returns: HashMap<u64, Value>,
    pending_generator_exception: Option<Value>,
    active_generator_resume: Option<u64>,
    active_generator_resume_boundary: Option<usize>,
    generator_resume_outcome: Option<GeneratorResumeOutcome>,
    run_stop_depth: Option<usize>,
    suppress_metaclass_dispatch_depth: usize,
    pending_import_drain_depth: usize,
    signal_handlers: HashMap<i64, Value>,
    audit_hooks: Vec<Value>,
    monitoring_tool_names: HashMap<i64, String>,
    monitoring_event_sets: HashMap<i64, i64>,
    monitoring_local_event_sets: HashMap<(i64, usize), i64>,
    monitoring_callbacks: HashMap<(i64, i64), Value>,
    socket_default_timeout: Option<f64>,
    open_files: HashMap<i64, fs::File>,
    fd_inheritable: HashMap<i64, bool>,
    next_fd: i64,
    child_processes: HashMap<i64, Child>,
    child_exit_status: HashMap<i64, i64>,
    csv_dialects: HashMap<String, Value>,
    csv_field_size_limit: i64,
    hash_states: HashMap<u64, HashState>,
    hmac_states: HashMap<u64, HmacState>,
    zlib_compress_objects: HashMap<u64, ZlibCompressObjectState>,
    zlib_decompress_objects: HashMap<u64, ZlibDecompressObjectState>,
    bz2_compressors: HashMap<u64, Bz2CompressorState>,
    bz2_decompressors: HashMap<u64, Bz2DecompressorState>,
    lzma_compressors: HashMap<u64, LzmaCompressorState>,
    lzma_decompressors: HashMap<u64, LzmaDecompressorState>,
    expat_parsers: HashMap<u64, ExpatParserState>,
    sqlite_connections: HashMap<u64, SqliteConnectionState>,
    sqlite_cursors: HashMap<u64, SqliteCursorState>,
    sqlite_blobs: HashMap<u64, SqliteBlobState>,
    sqlite_callback_tracebacks_enabled: bool,
    pickle_copyreg_cache: HashMap<String, Value>,
    pickle_symbol_cache: HashMap<String, Value>,
    defaultdict_factories: HashMap<u64, Value>,
    ordered_dict_instances: HashSet<u64>,
    synthetic_exception_classes: HashMap<String, ObjRef>,
    synthetic_builtin_classes: HashMap<String, ObjRef>,
    mappingproxy_type_class: Option<ObjRef>,
    exception_parents: HashMap<String, String>,
    finalized_del_objects: HashSet<u64>,
    cleared_weakref_objects: HashSet<u64>,
    pending_del_instances: HashMap<u64, ObjRef>,
    weakref_finalizers: HashMap<u64, (Weak<Obj>, Vec<ObjRef>)>,
    typing_overload_registry: HashMap<(String, String), Vec<Value>>,
    thread_info_objects: HashMap<i64, ObjRef>,
    atexit_handlers: Vec<AtexitHandler>,
    extension_libraries: Vec<SharedLibraryHandle>,
    extension_callable_registry: HashMap<u64, ExtensionCallableEntry>,
    callable_attr_overrides: HashMap<u64, HashMap<String, Value>>,
    builtin_attr_overrides: HashMap<BuiltinFunction, HashMap<String, Value>>,
    extension_capsule_registry: HashMap<String, ExtensionCapsuleRegistryEntry>,
    extension_contextvar_registry: HashMap<usize, Value>,
    extension_contextvar_allocations: Vec<*mut u8>,
    extension_pinned_cpython_allocations: Vec<*mut c_void>,
    extension_pinned_cpython_allocation_set: HashSet<usize>,
    extension_pinned_capsule_names: HashMap<usize, CString>,
    extension_cpython_ptr_values: HashMap<usize, Value>,
    extension_cpython_ptr_by_object_id: HashMap<u64, usize>,
    capi_object_registry: CapiObjectRegistry,
    extension_module_def_registry: HashMap<u64, usize>,
    extension_module_state_registry: HashMap<u64, ExtensionModuleStateEntry>,
    extension_init_in_progress: HashSet<String>,
    extension_initialized_names: HashSet<String>,
    extension_init_failures: HashMap<String, String>,
    next_extension_callable_id: u64,
    local_shim_fallback_enabled: bool,
    prefer_pure_json_when_available: bool,
    prefer_pure_pickle_when_available: bool,
    prefer_pure_re_when_available: bool,
    prefer_pyc_when_source_available: bool,
    list_eq_in_progress: Vec<(u64, u64)>,
    repr_in_progress: Vec<u64>,
    hash_cache: HashMap<u64, u64>,
    is_finalizing: bool,
    recursion_limit: i64,
    switch_interval: f64,
    gc_enabled: bool,
    gc_thresholds: [usize; 3],
    gc_counts: [usize; 3],
    gc_last_allocation_count: usize,
    gc_auto_check_budget: usize,
    gc_auto_collect_enabled: bool,
    tracemalloc_enabled: bool,
    tracemalloc_traceback_limit: usize,
    tracemalloc_object_traces: HashMap<u64, Vec<(String, usize)>>,
    next_synthetic_thread_ident: i64,
    builtins_version: u64,
    class_attr_versions: HashMap<u64, u64>,
    type_cache_version_tag: u32,
    abc_invalidation_counter: usize,
    abc_registry: HashMap<u64, Vec<Value>>,
    abc_cache: HashMap<u64, Vec<Value>>,
    abc_negative_cache: HashMap<u64, Vec<Value>>,
    abc_negative_cache_version: HashMap<u64, usize>,
    warnings_bless_my_loader_depth: usize,
    fast_local_unbound_marker: Value,
    traceback_caret_enabled: bool,
    debug_exception_unwind_depth_enabled: bool,
    debug_exception_unwind_depth_limit: usize,
    instruction_step_limit: Option<u64>,
    instruction_steps: u64,
    execution_deadline: Option<VmExecutionDeadline>,
    capture_sys_stream_output: bool,
    captured_sys_stdout: String,
    captured_sys_stderr: String,
}

impl Drop for Vm {
    fn drop(&mut self) {
        // Reclaim Python-level cycles before extension/native teardown mutates
        // pointer-backed proxy state.
        self.heap.collect_cycles(&[]);
        for state in self.extension_module_state_registry.values() {
            if state.state != 0 {
                if let Some(finalize_func) = state.finalize_func {
                    // SAFETY: finalize function pointers come from loaded extension modules and
                    // are invoked before extension libraries are dropped.
                    unsafe {
                        finalize_func(state.state as *mut c_void);
                    }
                }
                if let Some(free_func) = state.free_func {
                    // SAFETY: free function pointers come from loaded extension modules and
                    // are invoked before extension libraries are dropped.
                    unsafe {
                        free_func(state.state as *mut c_void);
                    }
                }
            }
        }
        self.extension_module_state_registry.clear();
        for capsule in self.extension_capsule_registry.values() {
            if let Some(destructor) = capsule.destructor {
                // SAFETY: destructor pointers come from loaded extension modules and
                // are invoked before extension libraries are dropped.
                unsafe {
                    destructor(
                        capsule.pointer as *mut c_void,
                        capsule.context as *mut c_void,
                    );
                }
            }
        }
        self.extension_capsule_registry.clear();
        self.extension_module_def_registry.clear();
        self.extension_contextvar_registry.clear();
        for raw in self.extension_contextvar_allocations.drain(..) {
            if !raw.is_null() {
                // SAFETY: pointers were allocated with Box::into_raw in PyContextVar_New.
                unsafe {
                    drop(Box::from_raw(raw));
                }
            }
        }
        let drained_external_refs = self.capi_registry_drain_external_pins();
        for (ptr, pin_count) in drained_external_refs {
            if ptr == 0 {
                continue;
            }
            self.capi_registry_mark_pending_free(ptr);
            // SAFETY: pointers in this set were incref'd when external proxies were
            // materialized and must be decref'd exactly once at VM teardown.
            for _ in 0..pin_count {
                unsafe {
                    vm_extensions::Py_DecRef(ptr as *mut c_void);
                }
            }
            self.capi_registry_mark_freed(ptr);
        }
        let drained_pinned_allocations: Vec<*mut c_void> = self
            .extension_pinned_cpython_allocations
            .drain(..)
            .collect();
        let mut freed_pinned_allocations: HashSet<usize> = HashSet::new();
        for raw in drained_pinned_allocations {
            if !raw.is_null() {
                let addr = raw as usize;
                self.capi_registry_mark_pending_free(addr);
                if !self.capi_owned_ptr_is_pinned(addr) {
                    if self.host.env_var_os("PYRS_TRACE_PIN_FREE").is_some() {
                        eprintln!("[pin-free] vm-skip ptr={:p} reason=not-in-set", raw);
                    }
                    self.capi_registry_mark_freed(addr);
                    continue;
                }
                if self.capi_registry_is_freed(addr) {
                    if self.host.env_var_os("PYRS_TRACE_PIN_FREE").is_some() {
                        eprintln!("[pin-free] vm-skip ptr={:p} reason=already-freed", raw);
                    }
                    self.capi_registry_mark_freed(addr);
                    continue;
                }
                if !freed_pinned_allocations.insert(addr) {
                    if self.host.env_var_os("PYRS_TRACE_PIN_FREE").is_some() {
                        eprintln!("[pin-free] vm-skip ptr={:p} reason=duplicate", raw);
                    }
                    continue;
                }
                if self.host.env_var_os("PYRS_TRACE_PIN_FREE").is_some() {
                    eprintln!("[pin-free] vm-free ptr={:p}", raw);
                }
                // SAFETY: pointers were allocated via libc malloc in C-API compat paths.
                unsafe {
                    free(raw);
                }
                self.capi_unpin_owned_ptr(addr);
                self.capi_registry_mark_freed(addr);
            }
        }
        self.extension_pinned_cpython_allocation_set.clear();
        self.extension_pinned_capsule_names.clear();
        self.extension_cpython_ptr_values.clear();
        self.extension_cpython_ptr_by_object_id.clear();
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        Self::new_with_host(Arc::new(NativeHost))
    }

    pub fn new_with_host(host: Arc<dyn VmHost>) -> Self {
        configure_env_presence_probe_source(host.as_ref());
        let heap = Heap::new();
        let main_module = match heap.alloc_module(ModuleObject::new("__main__")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        let fast_local_unbound_marker = {
            let marker_class = match heap.alloc_class(ClassObject::new(
                "__pyrs_fast_local_unbound__".to_string(),
                Vec::new(),
            )) {
                Value::Class(obj) => obj,
                _ => unreachable!(),
            };
            match heap.alloc_instance(InstanceObject::new(marker_class)) {
                Value::Instance(obj) => Value::Instance(obj),
                _ => unreachable!(),
            }
        };

        let mut modules = HashMap::new();
        modules.insert("__main__".to_string(), main_module.clone());

        let module_paths = vec![host.current_dir().unwrap_or_else(|_| PathBuf::from("."))];
        let trace_flags = VmTraceFlags::from_host(host.as_ref());
        let trace_text_filters = VmTraceTextFilters::from_host(host.as_ref());
        let debug_exception_unwind_depth_enabled = host
            .env_var_os("PYRS_DEBUG_EXCEPTION_UNWIND_DEPTH")
            .is_some();
        let debug_exception_unwind_depth_limit = if debug_exception_unwind_depth_enabled {
            host.env_var("PYRS_DEBUG_EXCEPTION_UNWIND_DEPTH_LIMIT")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(256)
        } else {
            256
        };

        let mut vm = Self {
            host: host.clone(),
            frames: Vec::with_capacity(128),
            frame_pool: Vec::with_capacity(128),
            simple_frame_pool: Vec::with_capacity(128),
            simple_slot0_pool: Vec::with_capacity(128),
            simple_slot0_pool_key: None,
            frame_proxy_cache: Vec::new(),
            frame_proxy_cache_key: None,
            builtins: HashMap::new(),
            modules,
            main_module,
            module_paths,
            module_source_positive_cache: HashMap::new(),
            source_text_cache: HashMap::new(),
            compiled_code_metadata: HashMap::new(),
            import_dir_cache: HashMap::new(),
            preferred_filesystem_module_cache: HashMap::new(),
            import_sys_path_signature: 0,
            import_meta_path_signature: 0,
            import_path_hooks_signature: 0,
            import_meta_path_has_default_finder: true,
            import_path_hooks_has_default_hook: true,
            import_perf_enabled: host.env_flag_enabled("PYRS_IMPORT_PERF"),
            trace_flags,
            trace_text_filters,
            import_perf_counters: ImportPerfCounters::default(),
            heap,
            random: Mt19937::new(5489),
            generator_states: HashMap::new(),
            generator_returns: HashMap::new(),
            pending_generator_exception: None,
            active_generator_resume: None,
            active_generator_resume_boundary: None,
            generator_resume_outcome: None,
            run_stop_depth: None,
            suppress_metaclass_dispatch_depth: 0,
            pending_import_drain_depth: 0,
            signal_handlers: HashMap::new(),
            audit_hooks: Vec::new(),
            monitoring_tool_names: HashMap::new(),
            monitoring_event_sets: HashMap::new(),
            monitoring_local_event_sets: HashMap::new(),
            monitoring_callbacks: HashMap::new(),
            socket_default_timeout: None,
            open_files: HashMap::new(),
            fd_inheritable: HashMap::new(),
            next_fd: 3,
            child_processes: HashMap::new(),
            child_exit_status: HashMap::new(),
            csv_dialects: HashMap::new(),
            csv_field_size_limit: 131_072,
            hash_states: HashMap::new(),
            hmac_states: HashMap::new(),
            zlib_compress_objects: HashMap::new(),
            zlib_decompress_objects: HashMap::new(),
            bz2_compressors: HashMap::new(),
            bz2_decompressors: HashMap::new(),
            lzma_compressors: HashMap::new(),
            lzma_decompressors: HashMap::new(),
            expat_parsers: HashMap::new(),
            sqlite_connections: HashMap::new(),
            sqlite_cursors: HashMap::new(),
            sqlite_blobs: HashMap::new(),
            sqlite_callback_tracebacks_enabled: false,
            pickle_copyreg_cache: HashMap::new(),
            pickle_symbol_cache: HashMap::new(),
            defaultdict_factories: HashMap::new(),
            ordered_dict_instances: HashSet::new(),
            synthetic_exception_classes: HashMap::new(),
            synthetic_builtin_classes: HashMap::new(),
            mappingproxy_type_class: None,
            exception_parents: HashMap::new(),
            finalized_del_objects: HashSet::new(),
            cleared_weakref_objects: HashSet::new(),
            pending_del_instances: HashMap::new(),
            weakref_finalizers: HashMap::new(),
            typing_overload_registry: HashMap::new(),
            thread_info_objects: HashMap::new(),
            atexit_handlers: Vec::new(),
            extension_libraries: Vec::new(),
            extension_callable_registry: HashMap::new(),
            callable_attr_overrides: HashMap::new(),
            builtin_attr_overrides: HashMap::new(),
            extension_capsule_registry: HashMap::new(),
            extension_contextvar_registry: HashMap::new(),
            extension_contextvar_allocations: Vec::new(),
            extension_pinned_cpython_allocations: Vec::new(),
            extension_pinned_cpython_allocation_set: HashSet::new(),
            extension_pinned_capsule_names: HashMap::new(),
            extension_cpython_ptr_values: HashMap::new(),
            extension_cpython_ptr_by_object_id: HashMap::new(),
            capi_object_registry: CapiObjectRegistry::default(),
            extension_module_def_registry: HashMap::new(),
            extension_module_state_registry: HashMap::new(),
            extension_init_in_progress: HashSet::new(),
            extension_initialized_names: HashSet::new(),
            extension_init_failures: HashMap::new(),
            next_extension_callable_id: 1,
            // Shim fallback is restricted by LOCAL_SHIM_MODULES (`_ctypes`) and only used when
            // normal path resolution fails, so keep it enabled by default (allow explicit opt-out).
            local_shim_fallback_enabled: !host.env_flag_enabled("PYRS_DISABLE_LOCAL_SHIMS"),
            prefer_pure_json_when_available: true,
            prefer_pure_pickle_when_available: true,
            prefer_pure_re_when_available: true,
            // CPython-default behavior: prefer validated source-bound pyc when available.
            // `PYRS_IMPORT_PREFER_PYC` can still explicitly override this.
            prefer_pyc_when_source_available: host
                .env_flag_enabled_or_default("PYRS_IMPORT_PREFER_PYC", true),
            list_eq_in_progress: Vec::new(),
            repr_in_progress: Vec::new(),
            hash_cache: HashMap::new(),
            is_finalizing: false,
            recursion_limit: 1000,
            switch_interval: 0.005,
            gc_enabled: true,
            gc_thresholds: [
                GC_DEFAULT_THRESHOLD0,
                GC_DEFAULT_THRESHOLD1,
                GC_DEFAULT_THRESHOLD2,
            ],
            gc_counts: [0, 0, 0],
            gc_last_allocation_count: 0,
            gc_auto_check_budget: GC_AUTO_CHECK_INTERVAL,
            gc_auto_collect_enabled: false,
            tracemalloc_enabled: false,
            tracemalloc_traceback_limit: 1,
            tracemalloc_object_traces: HashMap::new(),
            next_synthetic_thread_ident: SYNTHETIC_THREAD_IDENT_START,
            builtins_version: 1,
            class_attr_versions: HashMap::new(),
            type_cache_version_tag: 1,
            abc_invalidation_counter: 0,
            abc_registry: HashMap::new(),
            abc_cache: HashMap::new(),
            abc_negative_cache: HashMap::new(),
            abc_negative_cache_version: HashMap::new(),
            warnings_bless_my_loader_depth: 0,
            fast_local_unbound_marker,
            traceback_caret_enabled: true,
            debug_exception_unwind_depth_enabled,
            debug_exception_unwind_depth_limit,
            instruction_step_limit: host
                .env_var("PYRS_STEP_LIMIT")
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|limit| *limit > 0),
            instruction_steps: 0,
            execution_deadline: None,
            capture_sys_stream_output: false,
            captured_sys_stdout: String::new(),
            captured_sys_stderr: String::new(),
        };
        let main = vm.main_module.clone();
        vm.set_module_metadata(
            &main,
            "__main__",
            None,
            None,
            None,
            false,
            Vec::new(),
            false,
        );
        vm.install_sys_module();
        vm.install_importlib_modules();
        // Provide CPython core `_random` substrate; stdlib `random.py` layers on top.
        vm.install_random_module();
        vm.install_stdlib_modules();
        vm.install_builtins();
        vm.normalize_bootstrap_module_classes();
        vm.install_builtins_module();
        vm.refresh_warnings_fallback_defaults();
        vm.refresh_import_resolver_state();
        vm.gc_last_allocation_count = vm.heap.total_allocations();
        vm
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn set_sys_stream_capture_enabled(&mut self, enabled: bool) {
        self.capture_sys_stream_output = enabled;
        if !enabled {
            self.captured_sys_stdout.clear();
            self.captured_sys_stderr.clear();
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn take_captured_sys_stream_output(&mut self) -> (String, String) {
        (
            std::mem::take(&mut self.captured_sys_stdout),
            std::mem::take(&mut self.captured_sys_stderr),
        )
    }

    pub(super) fn capture_sys_stream_text(&mut self, stderr: bool, text: &str) -> bool {
        if !self.capture_sys_stream_output {
            return false;
        }
        if stderr {
            self.captured_sys_stderr.push_str(text);
        } else {
            self.captured_sys_stdout.push_str(text);
        }
        true
    }

    pub(super) fn capi_registry_register_ptr(
        &mut self,
        ptr: usize,
        provenance: CapiPtrProvenance,
        object_id: Option<u64>,
    ) {
        self.capi_object_registry
            .register_ptr(ptr, provenance, object_id);
    }

    pub(super) fn capi_registry_record_ref_kind(&mut self, ptr: usize, ref_kind: CapiRefKind) {
        self.capi_object_registry.record_ref_kind(ptr, ref_kind);
    }

    pub(super) fn capi_registry_pin_external_once(&mut self, ptr: usize) -> bool {
        self.capi_object_registry.pin_external_once(ptr)
    }

    pub(super) fn capi_pin_owned_ptr(&mut self, ptr: usize) -> bool {
        if ptr == 0 {
            return false;
        }
        if !self.capi_object_registry.ensure_owned_compat_entry(ptr) {
            return false;
        }
        let _ = self.capi_object_registry.pin_owned_once(ptr);
        let inserted = self.extension_pinned_cpython_allocation_set.insert(ptr);
        if inserted {
            self.extension_pinned_cpython_allocations
                .push(ptr as *mut c_void);
        }
        inserted
    }

    pub(super) fn capi_owned_ptr_is_pinned(&self, ptr: usize) -> bool {
        self.extension_pinned_cpython_allocation_set.contains(&ptr)
            || self.capi_object_registry.is_owned_pinned(ptr)
    }

    pub(super) fn capi_ptr_is_owned_compat(&self, ptr: usize) -> bool {
        self.extension_pinned_cpython_allocation_set.contains(&ptr)
            || self.capi_object_registry.is_owned_compat(ptr)
    }

    pub(super) fn capi_unpin_owned_ptr(&mut self, ptr: usize) -> bool {
        if ptr == 0 {
            return false;
        }
        let was_pinned = self.capi_owned_ptr_is_pinned(ptr);
        self.extension_pinned_cpython_allocation_set.remove(&ptr);
        self.capi_object_registry.unpin_owned(ptr);
        was_pinned
    }

    pub(super) fn capi_registry_drain_external_pins(&mut self) -> Vec<(usize, usize)> {
        self.capi_object_registry.drain_external_pins()
    }

    pub(super) fn capi_registry_mark_pending_free(&mut self, ptr: usize) {
        self.capi_object_registry.mark_pending_free(ptr);
    }

    pub(super) fn capi_registry_mark_alive(&mut self, ptr: usize) {
        self.capi_object_registry.mark_alive(ptr);
    }

    pub(super) fn capi_registry_should_free_now(&self, ptr: usize) -> bool {
        self.capi_object_registry.should_free_now(ptr)
    }

    pub(super) fn capi_registry_is_freed(&self, ptr: usize) -> bool {
        self.capi_object_registry.is_freed(ptr)
    }

    pub(super) fn capi_registry_contains_live_or_pending(&self, ptr: usize) -> bool {
        self.capi_object_registry.contains_live_or_pending(ptr)
    }

    pub(super) fn capi_registry_contains_alive(&self, ptr: usize) -> bool {
        self.capi_object_registry.contains_alive(ptr)
    }

    pub(super) fn capi_registry_mark_freed(&mut self, ptr: usize) {
        self.capi_object_registry.mark_freed(ptr);
    }

    pub(super) fn capi_registry_set_gc_tracked_override(
        &mut self,
        ptr: usize,
        tracked: bool,
    ) -> bool {
        self.capi_object_registry
            .set_gc_tracked_override(ptr, tracked)
    }

    pub(super) fn capi_registry_gc_tracked_override(&self, ptr: usize) -> Option<bool> {
        self.capi_object_registry.gc_tracked_override(ptr)
    }

    pub(super) fn capi_registry_set_gc_finalized(&mut self, ptr: usize, finalized: bool) -> bool {
        self.capi_object_registry.set_gc_finalized(ptr, finalized)
    }

    pub(super) fn capi_registry_is_gc_finalized(&self, ptr: usize) -> bool {
        self.capi_object_registry.is_gc_finalized(ptr)
    }

    pub(super) fn is_object_gc_finalized(&self, object_id: u64) -> bool {
        self.finalized_del_objects.contains(&object_id)
            || self.cleared_weakref_objects.contains(&object_id)
    }

    pub(super) fn mark_object_weakrefs_cleared(&mut self, object_id: u64) {
        self.cleared_weakref_objects.insert(object_id);
    }

    pub(super) fn clear_runtime_weakrefs_for_target_id(&mut self, target_id: u64) {
        self.mark_object_weakrefs_cleared(target_id);
        let mut wrappers: Vec<(Value, Option<Value>)> = Vec::new();
        for object in self.heap.snapshot_objects() {
            match &*object.kind() {
                Object::Module(module_data) => {
                    if !matches!(
                        module_data.globals.get("__pyrs_weakref_ref__"),
                        Some(Value::Bool(true))
                    ) {
                        continue;
                    }
                    let wrapper_target_id = match module_data.globals.get("target_id") {
                        Some(Value::Int(value)) if *value >= 0 => *value as u64,
                        _ => continue,
                    };
                    if wrapper_target_id != target_id {
                        continue;
                    }
                    let callback = module_data
                        .globals
                        .get("callback")
                        .cloned()
                        .and_then(|value| {
                            if matches!(value, Value::None) {
                                None
                            } else {
                                Some(value)
                            }
                        });
                    wrappers.push((Value::Module(object.clone()), callback));
                }
                Object::Instance(instance_data) => {
                    if !matches!(
                        instance_data.attrs.get("__pyrs_weakref_ref__"),
                        Some(Value::Bool(true))
                    ) {
                        continue;
                    }
                    let wrapper_target_id = match instance_data.attrs.get("target_id") {
                        Some(Value::Int(value)) if *value >= 0 => *value as u64,
                        _ => continue,
                    };
                    if wrapper_target_id != target_id {
                        continue;
                    }
                    let callback = instance_data.attrs.get("callback").cloned().and_then(|value| {
                        if matches!(value, Value::None) {
                            None
                        } else {
                            Some(value)
                        }
                    });
                    wrappers.push((Value::Instance(object.clone()), callback));
                }
                _ => continue,
            }
        }

        for (wrapper, _) in &wrappers {
            match wrapper {
                Value::Module(wrapper_module) => {
                    if let Object::Module(module_data) = &mut *wrapper_module.kind_mut() {
                        module_data
                            .globals
                            .insert("callback".to_string(), Value::None);
                        module_data
                            .globals
                            .insert("__pyrs_weakref_cleared__".to_string(), Value::Bool(true));
                    }
                }
                Value::Instance(wrapper_instance) => {
                    if let Object::Instance(instance_data) = &mut *wrapper_instance.kind_mut() {
                        instance_data
                            .attrs
                            .insert("callback".to_string(), Value::None);
                        instance_data
                            .attrs
                            .insert("__pyrs_weakref_cleared__".to_string(), Value::Bool(true));
                    }
                }
                _ => {}
            }
        }

        for (wrapper, callback) in wrappers {
            let Some(callback) = callback else {
                continue;
            };
            let weakref_value = if let Value::Module(wrapper_module) = &wrapper {
                self.alloc_builtin_bound_method(BuiltinFunction::WeakRefRef, wrapper_module.clone())
            } else {
                wrapper.clone()
            };
            match self.call_internal_preserving_caller(
                callback,
                vec![weakref_value],
                HashMap::new(),
            ) {
                Ok(InternalCallOutcome::Value(_))
                | Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    if let Some(frame) = self.frames.last_mut() {
                        frame.active_exception = None;
                        frame.except_star_match_lasti = None;
                    }
                }
                Err(err) => {
                    let exception = self.runtime_error_to_exception_value(err);
                    self.emit_unraisable_exception(
                        exception,
                        Some(wrapper.clone()),
                        Some("Exception ignored while calling weakref callback"),
                    );
                    if let Some(frame) = self.frames.last_mut() {
                        frame.active_exception = None;
                        frame.except_star_match_lasti = None;
                    }
                }
            }
        }
    }

    pub(super) fn current_thread_ident_value(&self) -> i64 {
        vm_current_thread_ident()
    }

    fn allocate_synthetic_thread_ident(&mut self) -> i64 {
        let ident = self.next_synthetic_thread_ident;
        self.next_synthetic_thread_ident = self.next_synthetic_thread_ident.wrapping_add(1);
        if self.next_synthetic_thread_ident <= 0 {
            self.next_synthetic_thread_ident = SYNTHETIC_THREAD_IDENT_START;
        }
        ident
    }

    fn clear_synthetic_thread_localimpl_entries(&mut self, thread_obj: &ObjRef) {
        let thread_key = Value::Int(thread_obj.id() as i64);
        let snapshot = self.heap.snapshot_objects();
        for obj in snapshot {
            let dict_obj = {
                let Object::Instance(instance_data) = &*obj.kind() else {
                    continue;
                };
                let Object::Class(class_data) = &*instance_data.class.kind() else {
                    continue;
                };
                if class_data.name != "_localimpl" {
                    continue;
                }
                let Some(Value::Dict(dict_obj)) = instance_data.attrs.get("dicts") else {
                    continue;
                };
                dict_obj.clone()
            };
            let _ = dict_remove_value(&dict_obj, &thread_key);
        }
    }

    fn call_internal_in_synthetic_thread(
        &mut self,
        callable: Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<(i64, InternalCallOutcome), RuntimeError> {
        let thread_ident = self.allocate_synthetic_thread_ident();
        let previous = VM_THREAD_IDENT_OVERRIDE.with(|slot| {
            let previous = slot.get();
            slot.set(Some(thread_ident));
            previous
        });
        let outcome = self.call_internal(callable, args, kwargs);
        VM_THREAD_IDENT_OVERRIDE.with(|slot| slot.set(previous));
        let thread_obj = self.thread_info_objects.get(&thread_ident).cloned();
        if let Some(thread_obj) = thread_obj.as_ref() {
            self.clear_synthetic_thread_localimpl_entries(thread_obj);
        }
        self.thread_info_objects.remove(&thread_ident);
        outcome.map(|result| (thread_ident, result))
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

    pub(crate) fn remove_global(&mut self, name: &str) -> Option<Value> {
        let mut removed = None;
        let mut touched_version = None;
        if let Object::Module(module) = &mut *self.main_module.kind_mut() {
            removed = module.globals.remove(name);
            if removed.is_some() {
                module.touch_globals_version();
                touched_version = Some(module.globals_version);
            }
        }
        if let Some(version) = touched_version {
            self.propagate_module_globals_version(self.main_module.id(), version);
        }
        removed
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

    fn normalize_class_annotations_after_attr_set(&mut self, class: &ObjRef, attr_name: &str) {
        if !matches!(attr_name, "__annotate__" | "__annotate_func__") {
            return;
        }
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            // PEP 649/749-style class annotations are driven by __annotate__;
            // replacing __annotate__ must invalidate any materialized dict state.
            class_data.attrs.remove("__annotations__");
            class_data.attrs.remove("__annotations_cache__");
        }
    }

    #[inline]
    fn next_type_cache_version_tag(&mut self) -> u32 {
        self.type_cache_version_tag = self.type_cache_version_tag.wrapping_add(1);
        if self.type_cache_version_tag == 0 {
            self.type_cache_version_tag = 1;
        }
        self.type_cache_version_tag
    }

    fn clear_type_related_inline_caches(&mut self) {
        for frame in &mut self.frames {
            for slot in &mut frame.load_attr_inline_cache {
                *slot = [None, None];
            }
            for slot in &mut frame.one_arg_inline_cache {
                *slot = None;
            }
        }
        for frame in self.generator_states.values_mut() {
            for slot in &mut frame.load_attr_inline_cache {
                *slot = [None, None];
            }
            for slot in &mut frame.one_arg_inline_cache {
                *slot = None;
            }
        }
    }

    fn collect_runtime_class_ids_for_type_cache(&self, modified_class_id: Option<u64>) -> Vec<u64> {
        let mut class_ids = Vec::new();
        for object in self.heap.snapshot_objects() {
            let Object::Class(class_data) = &*object.kind() else {
                continue;
            };
            if modified_class_id.is_none_or(|target_id| {
                object.id() == target_id || class_data.mro.iter().any(|base| base.id() == target_id)
            }) {
                class_ids.push(object.id());
            }
        }
        if let Some(class_id) = modified_class_id {
            class_ids.push(class_id);
        }
        class_ids.sort_unstable();
        class_ids.dedup();
        class_ids
    }

    pub(super) fn invalidate_type_cache_for_class_id(&mut self, class_id: u64) -> u32 {
        let class_ids = self.collect_runtime_class_ids_for_type_cache(Some(class_id));
        for id in class_ids {
            self.touch_class_attr_version_by_id(id);
        }
        self.touch_builtins_version();
        self.clear_type_related_inline_caches();
        self.next_type_cache_version_tag()
    }

    pub(super) fn clear_all_type_caches(&mut self) -> u32 {
        let class_ids = self.collect_runtime_class_ids_for_type_cache(None);
        for id in class_ids {
            self.touch_class_attr_version_by_id(id);
        }
        self.touch_builtins_version();
        self.clear_type_related_inline_caches();
        self.next_type_cache_version_tag()
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

    #[inline]
    fn recursion_limit_error(&self) -> RuntimeError {
        RuntimeError::with_exception(
            "RecursionError",
            Some("maximum recursion depth exceeded".to_string()),
        )
    }

    #[inline]
    fn ensure_can_push_python_frame(&self) -> Result<(), RuntimeError> {
        if self.frames.len() as i64 >= self.recursion_limit {
            if self.host.env_var_os("PYRS_TRACE_RECURSION_LIMIT").is_some() {
                let frame_summary = self
                    .frames
                    .iter()
                    .rev()
                    .take(8)
                    .map(|frame| format!("{}@{}", frame.code.name, frame.code.filename))
                    .collect::<Vec<_>>()
                    .join(" <= ");
                eprintln!(
                    "[recursion-limit] frames={} limit={} top={}",
                    self.frames.len(),
                    self.recursion_limit,
                    frame_summary
                );
            }
            return Err(self.recursion_limit_error());
        }
        Ok(())
    }

    #[inline]
    fn push_frame_checked(&mut self, frame: Box<Frame>) -> Result<(), RuntimeError> {
        self.ensure_can_push_python_frame()?;
        self.frames.push(frame);
        Ok(())
    }

    fn clone_exception_for_active_frame(exception: &ExceptionObject) -> Box<ExceptionObject> {
        let mut cloned = ExceptionObject::new(exception.name.clone(), exception.message.clone());
        cloned.object_id = exception.object_id;
        cloned.traceback_frames = exception.traceback_frames.clone();
        cloned.notes = exception.notes.clone();
        cloned.exceptions = exception
            .exceptions
            .iter()
            .map(|member| *Self::clone_exception_for_active_frame(member))
            .collect();
        cloned.suppress_context = exception.suppress_context;
        cloned.attrs = Rc::new(RefCell::new(exception.attrs.borrow().clone()));
        cloned.cause = exception
            .cause
            .as_ref()
            .map(|cause| Self::clone_exception_for_active_frame(cause));
        cloned.context = exception
            .context
            .as_ref()
            .map(|context| Self::clone_exception_for_active_frame(context));
        Box::new(cloned)
    }

    #[inline]
    fn clone_active_exception_for_call(value: &Value) -> Value {
        match value {
            Value::Exception(exception) => {
                Value::Exception(Self::clone_exception_for_active_frame(exception))
            }
            _ => value.clone(),
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
        frame.class_orig_bases = None;
        frame.class_metaclass = None;
        frame.class_namespace = None;
        frame.active_exception = None;
        frame.except_star_match_lasti = None;
        frame.reraise_lasti_override = None;
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
            frame.except_star_match_lasti = None;
            frame.reraise_lasti_override = None;
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
        frame.except_star_match_lasti = None;
        frame.reraise_lasti_override = None;
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
            && frame.class_orig_bases.is_none()
            && frame.class_metaclass.is_none()
            && frame.class_namespace.is_none()
            && frame.generator_owner.is_none()
            && frame.generator_resume_value.is_none()
            && frame.generator_pending_throw.is_none()
            && frame.generator_resume_kind.is_none()
            && frame.yield_from_iter.is_none()
            && frame.active_exception.is_none()
            && frame.except_star_match_lasti.is_none()
            && frame.reraise_lasti_override.is_none()
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
        if frame.class_orig_bases.is_some() {
            frame.class_orig_bases = None;
        }
        if frame.class_metaclass.is_some() {
            frame.class_metaclass = None;
        }
        if frame.class_namespace.is_some() {
            frame.class_namespace = None;
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
        if frame.except_star_match_lasti.is_some() {
            frame.except_star_match_lasti = None;
        }
        if frame.reraise_lasti_override.is_some() {
            frame.reraise_lasti_override = None;
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
        debug_assert!(frame.class_orig_bases.is_none());
        debug_assert!(frame.class_metaclass.is_none());
        debug_assert!(frame.class_namespace.is_none());
        debug_assert!(frame.generator_owner.is_none());
        debug_assert!(frame.generator_resume_value.is_none());
        debug_assert!(frame.generator_pending_throw.is_none());
        debug_assert!(frame.generator_resume_kind.is_none());
        debug_assert!(frame.yield_from_iter.is_none());
        debug_assert!(frame.active_exception.is_none());
        debug_assert!(frame.except_star_match_lasti.is_none());
        debug_assert!(frame.reraise_lasti_override.is_none());
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

    pub fn repl_root_bindings(&self) -> Vec<(String, Value)> {
        let mut bindings = Vec::new();
        if let Object::Module(module) = &*self.main_module.kind() {
            bindings.extend(
                module
                    .globals
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
        if let Some(builtins_obj) = self.modules.get("builtins")
            && let Object::Module(module) = &*builtins_obj.kind()
        {
            bindings.extend(
                module
                    .globals
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
        bindings.sort_by(|left, right| left.0.cmp(&right.0));
        bindings.dedup_by(|left, right| left.0 == right.0);
        bindings
    }

    pub fn run_shutdown_hooks(&mut self) -> Result<(), RuntimeError> {
        let previous_finalizing = self.is_finalizing;
        self.is_finalizing = true;
        let mut traceback_anchor_backup: Option<Option<Value>> = None;
        if let Some(traceback_module) = self.modules.get("traceback").cloned()
            && let Object::Module(module_data) = &mut *traceback_module.kind_mut()
        {
            let key = "_extract_caret_anchors_from_line_segment".to_string();
            traceback_anchor_backup = Some(module_data.globals.get(&key).cloned());
            module_data.globals.insert(key, Value::None);
        }
        let pushed_shutdown_frame = if self.frames.is_empty() {
            let shutdown_code = Rc::new(CodeObject::new("<sys>", "<sys>"));
            let shutdown_frame = Frame::new(
                shutdown_code,
                self.main_module.clone(),
                true,
                false,
                Vec::new(),
                None,
            );
            self.push_frame_checked(Box::new(shutdown_frame))?;
            true
        } else {
            false
        };
        let shutdown_result = self.builtin_atexit_run_exitfuncs(Vec::new(), HashMap::new());
        self.run_weakref_atexit_finalizers();
        self.run_pending_del_finalizers(true);
        if pushed_shutdown_frame {
            let _ = self.frames.pop();
        }
        if let Some(saved_anchor) = traceback_anchor_backup
            && let Some(traceback_module) = self.modules.get("traceback").cloned()
            && let Object::Module(module_data) = &mut *traceback_module.kind_mut()
        {
            let key = "_extract_caret_anchors_from_line_segment".to_string();
            if let Some(value) = saved_anchor {
                module_data.globals.insert(key, value);
            } else {
                module_data.globals.remove(&key);
            }
        }
        self.is_finalizing = previous_finalizing;
        if self.import_perf_enabled {
            eprintln!(
                "[import-perf] source_compiles={} pyc_attempts={} pyc_fallbacks={}",
                self.import_perf_counters.fs_source_compiles,
                self.import_perf_counters.pyc_load_attempts,
                self.import_perf_counters.pyc_load_fallback_to_source,
            );
        }
        if let Some(capi_perf) = self::vm_extensions::capi_perf_snapshot() {
            eprintln!(
                "[capi-perf] richcompare={} richcompare_bool={} richcompare_slot={} dunder_fallback={} dunder_missing={} dunder_calls={} dunder_owned={} dunder_external={} value_from_ptr={} handle_from_ptr={}/{} py_incref={}/{} py_decref={}/{}",
                capi_perf.richcompare_calls,
                capi_perf.richcompare_bool_calls,
                capi_perf.richcompare_slot_attempts,
                capi_perf.richcompare_dunder_fallback_attempts,
                capi_perf.richcompare_dunder_attr_missing,
                capi_perf.richcompare_dunder_callable_invocations,
                capi_perf.richcompare_dunder_calls_owned,
                capi_perf.richcompare_dunder_calls_external,
                capi_perf.value_from_ptr_calls,
                capi_perf.handle_from_ptr_hits,
                capi_perf.handle_from_ptr_calls,
                capi_perf.py_incref_handle_hits,
                capi_perf.py_incref_calls,
                capi_perf.py_decref_handle_hits,
                capi_perf.py_decref_calls,
            );
        }
        shutdown_result.map(|_| ())
    }

    pub fn add_module_path(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        if self.module_paths.iter().any(|existing| existing == &path) {
            return;
        }
        self.module_paths.push(path);
        self.module_source_positive_cache.clear();
        self.import_dir_cache.clear();
        self.preferred_filesystem_module_cache.clear();
        self.import_sys_path_signature = 0;
        self.sync_sys_path_from_module_paths();
        self.maybe_prefer_cpython_pure_stdlib_modules();
    }

    pub fn add_module_path_front(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.module_paths.retain(|existing| existing != &path);
        self.module_paths.insert(0, path);
        self.module_source_positive_cache.clear();
        self.import_dir_cache.clear();
        self.preferred_filesystem_module_cache.clear();
        self.import_sys_path_signature = 0;
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

    pub fn enable_source_bound_pyc_preference(&mut self) {
        self.prefer_pyc_when_source_available = true;
        self.module_source_positive_cache.clear();
        self.import_dir_cache.clear();
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
            if let Some(value) = slot
                && let Some(name) = frame.code.names.get(idx)
            {
                fallback.insert(name.clone(), value.clone());
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

    fn ensure_exception_default_attrs(&mut self, exception: &ExceptionObject) {
        let message_value = exception
            .message
            .as_ref()
            .map(|text| Value::Str(text.clone()))
            .unwrap_or(Value::None);
        if is_import_error_family(exception.name.as_str()) {
            let mut attrs = exception.attrs.borrow_mut();
            if !attrs.contains_key("msg") {
                attrs.insert("msg".to_string(), message_value.clone());
            }
            if !attrs.contains_key("name") {
                attrs.insert("name".to_string(), Value::None);
            }
            if !attrs.contains_key("path") {
                attrs.insert("path".to_string(), Value::None);
            }
        }
        if exception.attrs.borrow().contains_key("args") {
            return;
        }
        let (os_errno, os_strerror) = {
            let attrs = exception.attrs.borrow();
            let errno = attrs.get("errno").and_then(|value| match value {
                Value::Int(errno) => Some(*errno),
                _ => None,
            });
            let strerror = attrs.get("strerror").and_then(|value| match value {
                Value::Str(text) => Some(text.clone()),
                _ => None,
            });
            (errno, strerror)
        };
        let args = if is_os_error_family(exception.name.as_str()) {
            if let Some(errno) = os_errno {
                let mut values = vec![Value::Int(errno)];
                if let Some(strerror) = os_strerror {
                    values.push(Value::Str(strerror));
                }
                self.heap.alloc_tuple(values)
            } else if let Some(strerror) = os_strerror {
                self.heap.alloc_tuple(vec![Value::Str(strerror)])
            } else if let Some(message) = &exception.message {
                self.heap.alloc_tuple(vec![Value::Str(message.clone())])
            } else {
                self.heap.alloc_tuple(Vec::new())
            }
        } else if let Some(message) = &exception.message {
            self.heap.alloc_tuple(vec![Value::Str(message.clone())])
        } else {
            self.heap.alloc_tuple(Vec::new())
        };
        exception
            .attrs
            .borrow_mut()
            .insert("args".to_string(), args);
    }

    fn runtime_error_to_exception_object(&mut self, err: RuntimeError) -> ExceptionObject {
        if let Some(exception) = err.exception {
            let exception = *exception;
            self.ensure_exception_default_attrs(&exception);
            return exception;
        }
        let message = if err.message.is_empty() {
            None
        } else {
            Some(err.message)
        };
        let exception = ExceptionObject::new("RuntimeError", message);
        self.ensure_exception_default_attrs(&exception);
        exception
    }

    fn runtime_error_to_exception_value(&mut self, err: RuntimeError) -> Value {
        Value::Exception(Box::new(self.runtime_error_to_exception_object(err)))
    }

    pub(crate) fn emit_unraisable_exception(
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
            frame.except_star_match_lasti = None;
        }
    }

    fn run_pending_del_finalizers(&mut self, force_all: bool) {
        let mut ready = Vec::new();
        for (id, instance) in &self.pending_del_instances {
            if force_all || instance.strong_count() == 1 {
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
                        .and_then(|frame| {
                            frame.except_star_match_lasti = None;
                            frame.active_exception.take()
                        })
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
                frame.except_star_match_lasti = None;
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
                    frame.except_star_match_lasti = None;
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
                    frame.except_star_match_lasti = None;
                }
            }
        }
    }

    fn collect_gc_roots(&self) -> Vec<Value> {
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
            roots.extend(frame.code.constants.iter().cloned());
            self.collect_frame_cache_roots(frame, &mut roots);
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
            if let Some(module_locals) = &frame.module_locals_dict {
                roots.push(Value::Dict(module_locals.clone()));
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
            if let Some(orig_bases) = &frame.class_orig_bases {
                roots.push(orig_bases.clone());
            }
            if let Some(meta) = &frame.class_metaclass {
                roots.push(meta.clone());
            }
            if let Some(namespace) = &frame.class_namespace {
                roots.push(namespace.clone());
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
            roots.extend(frame.code.constants.iter().cloned());
            self.collect_frame_cache_roots(frame, &mut roots);
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
            if let Some(module_locals) = &frame.module_locals_dict {
                roots.push(Value::Dict(module_locals.clone()));
            }
            if let Some(instance) = &frame.return_instance {
                roots.push(Value::Instance(instance.clone()));
            }
            for base in &frame.class_bases {
                roots.push(Value::Class(base.clone()));
            }
            if let Some(orig_bases) = &frame.class_orig_bases {
                roots.push(orig_bases.clone());
            }
            if let Some(meta) = &frame.class_metaclass {
                roots.push(meta.clone());
            }
            if let Some(namespace) = &frame.class_namespace {
                roots.push(namespace.clone());
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
        roots
    }

    fn collect_frame_cache_roots(&self, frame: &Frame, roots: &mut Vec<Value>) {
        for slot in frame.load_global_inline_cache.iter().flatten() {
            roots.push(slot.value.clone());
            if let Some(func) = &slot.fused_direct_func {
                roots.push(Value::Function(func.clone()));
            }
        }
        for ways in &frame.load_attr_inline_cache {
            for entry in ways.iter().flatten() {
                roots.push(Value::Class(entry.owner_class.clone()));
                match &entry.kind {
                    LoadAttrSiteCacheKind::InstanceValue { value } => roots.push(value.clone()),
                    LoadAttrSiteCacheKind::InstanceFunction { function } => {
                        roots.push(Value::Function(function.clone()));
                    }
                    LoadAttrSiteCacheKind::InstanceBuiltin { .. } => {}
                    LoadAttrSiteCacheKind::InstanceClassMethod { descriptor }
                    | LoadAttrSiteCacheKind::InstanceStaticMethod { descriptor } => {
                        roots.push(Value::Module(descriptor.clone()));
                    }
                }
            }
        }
        for slot in frame.one_arg_inline_cache.iter().flatten() {
            if let Some(module) = &slot.cached_module {
                roots.push(Value::Module(module.clone()));
            }
            if let Some(owner_class) = &slot.cached_owner_class {
                roots.push(Value::Class(owner_class.clone()));
            }
            if let Some(closure) = &slot.cached_closure {
                for cell in closure {
                    roots.push(Value::Cell(cell.clone()));
                }
            }
        }
    }

    pub fn gc_collect(&mut self) -> usize {
        self.run_pending_del_finalizers(false);
        let roots = self.collect_gc_roots();
        let unreachable = self.heap.unreachable_objects(&roots);
        let unreachable_count = unreachable.len();
        for obj in unreachable {
            let obj_id = obj.id();
            self.pending_del_instances.remove(&obj_id);
            self.cleared_weakref_objects.remove(&obj_id);
            self.hash_states.remove(&obj_id);
            self.hmac_states.remove(&obj_id);
            self.zlib_compress_objects.remove(&obj_id);
            self.zlib_decompress_objects.remove(&obj_id);
            self.bz2_compressors.remove(&obj_id);
            self.bz2_decompressors.remove(&obj_id);
            self.lzma_compressors.remove(&obj_id);
            self.lzma_decompressors.remove(&obj_id);
            self.expat_parsers.remove(&obj_id);
            self.sqlite_cursors.remove(&obj_id);
            self.sqlite_blobs.remove(&obj_id);
            if let Some(mut connection_state) = self.sqlite_connections.remove(&obj_id) {
                let _ = connection_state.close();
                self.sqlite_cursors
                    .retain(|_, cursor_state| cursor_state.connection_id != obj_id);
                self.sqlite_blobs
                    .retain(|_, blob_state| blob_state.connection_id != obj_id);
            }
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
                frame.except_star_match_lasti = None;
            }
        }
        self.heap.collect_cycles(&roots);
        self.gc_last_allocation_count = self.heap.total_allocations();
        self.gc_counts[0] = 0;
        self.gc_auto_check_budget = GC_AUTO_CHECK_INTERVAL;
        unreachable_count
    }

    #[inline]
    fn gc_count0(&self) -> usize {
        self.heap
            .total_allocations()
            .saturating_sub(self.gc_last_allocation_count)
    }

    #[inline]
    fn maybe_gc_collect_automatic(&mut self) {
        if self.gc_auto_check_budget > 0 {
            self.gc_auto_check_budget -= 1;
            return;
        }
        self.gc_auto_check_budget = GC_AUTO_CHECK_INTERVAL;

        if !self.gc_enabled {
            return;
        }
        if !self.gc_auto_collect_enabled {
            self.gc_counts[0] = self.gc_count0();
            return;
        }
        // Running cycle collection in deeply nested execution can overlap with
        // active RefCell borrows in descriptor/import/class-build paths.
        // Keep automatic collection at top-level execution boundaries.
        if self.frames.len() > 1 {
            self.gc_counts[0] = self.gc_count0();
            return;
        }
        let threshold = self.gc_thresholds[0];
        if threshold == 0 {
            self.gc_counts[0] = self.gc_count0();
            return;
        }
        let count0 = self.gc_count0();
        self.gc_counts[0] = count0;
        if count0 < threshold {
            return;
        }

        let roots = self.collect_gc_roots();
        self.heap.collect_cycles(&roots);
        self.gc_last_allocation_count = self.heap.total_allocations();
        self.gc_counts[0] = 0;
        self.gc_auto_check_budget = GC_AUTO_CHECK_INTERVAL;
        self.gc_counts[1] = self.gc_counts[1].saturating_add(1);
        let threshold1 = self.gc_thresholds[1].max(1);
        if self.gc_counts[1] >= threshold1 {
            self.gc_counts[1] = 0;
            self.gc_counts[2] = self.gc_counts[2].saturating_add(1);
            let threshold2 = self.gc_thresholds[2].max(1);
            if self.gc_counts[2] >= threshold2 {
                self.gc_counts[2] = 0;
            }
        }
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
        let unreachable = self.gc_collect();
        Ok(Value::Int(unreachable as i64))
    }

    fn builtin_gc_enable(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("gc.enable() expects no arguments"));
        }
        self.gc_enabled = true;
        Ok(Value::None)
    }

    fn builtin_gc_disable(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("gc.disable() expects no arguments"));
        }
        self.gc_enabled = false;
        Ok(Value::None)
    }

    fn builtin_gc_is_enabled(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("gc.isenabled() expects no arguments"));
        }
        Ok(Value::Bool(self.gc_enabled))
    }

    fn builtin_gc_get_threshold(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("gc.get_threshold() expects no arguments"));
        }
        Ok(self.heap.alloc_tuple(vec![
            Value::Int(self.gc_thresholds[0] as i64),
            Value::Int(self.gc_thresholds[1] as i64),
            Value::Int(self.gc_thresholds[2] as i64),
        ]))
    }

    fn builtin_gc_set_threshold(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new(
                "gc.set_threshold() expects 1-3 integer arguments",
            ));
        }
        let mut new_thresholds = self.gc_thresholds;
        for (idx, arg) in args.into_iter().enumerate() {
            let value = value_to_int(arg)?;
            if value < 0 {
                return Err(RuntimeError::new("gc thresholds must be non-negative"));
            }
            new_thresholds[idx] = value as usize;
        }
        self.gc_thresholds = new_thresholds;
        self.gc_auto_collect_enabled = true;
        self.gc_counts[0] = self.gc_count0();
        Ok(Value::None)
    }

    fn builtin_gc_get_count(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("gc.get_count() expects no arguments"));
        }
        self.gc_counts[0] = self.gc_count0();
        Ok(self.heap.alloc_tuple(vec![
            Value::Int(self.gc_counts[0] as i64),
            Value::Int(self.gc_counts[1] as i64),
            Value::Int(self.gc_counts[2] as i64),
        ]))
    }

    pub fn start_tracemalloc(&mut self, traceback_limit: usize) {
        self.tracemalloc_enabled = true;
        self.tracemalloc_traceback_limit = traceback_limit.clamp(1, 65_535);
        self.tracemalloc_object_traces.clear();
    }

    pub fn stop_tracemalloc(&mut self) {
        self.tracemalloc_enabled = false;
        self.tracemalloc_object_traces.clear();
    }

    fn tracemalloc_capture_current_traceback(&self) -> Vec<(String, usize)> {
        let mut frames = Vec::new();
        for frame in self.frames.iter().rev() {
            let trace = Self::frame_trace(frame);
            if trace.line == 0 {
                continue;
            }
            frames.push((trace.filename, trace.line));
            if frames.len() >= self.tracemalloc_traceback_limit {
                break;
            }
        }
        frames
    }

    pub(super) fn tracemalloc_track_object_allocation(&mut self, obj: &ObjRef) {
        if !self.tracemalloc_enabled {
            return;
        }
        let frames = self.tracemalloc_capture_current_traceback();
        if !frames.is_empty() {
            self.tracemalloc_object_traces.insert(obj.id(), frames);
        }
    }

    fn tracemalloc_traceback_for_value(&self, value: &Value) -> Option<Vec<(String, usize)>> {
        let object_id = weakref_target_id(value)?;
        self.tracemalloc_object_traces.get(&object_id).cloned()
    }

    fn builtin_tracemalloc_start(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new(
                "tracemalloc.start() expects at most one argument",
            ));
        }
        let traceback_limit = if let Some(raw_limit) = args.first() {
            let raw_limit = value_to_int(raw_limit.clone())?;
            if !(1..=65_535).contains(&raw_limit) {
                return Err(RuntimeError::value_error(
                    "the number of frames must be in range [1; 65535]",
                ));
            }
            raw_limit as usize
        } else {
            1
        };
        self.start_tracemalloc(traceback_limit);
        Ok(Value::None)
    }

    fn builtin_tracemalloc_stop(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("tracemalloc.stop() expects no arguments"));
        }
        self.stop_tracemalloc();
        Ok(Value::None)
    }

    fn builtin_tracemalloc_is_tracing(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "tracemalloc.is_tracing() expects no arguments",
            ));
        }
        Ok(Value::Bool(self.tracemalloc_enabled))
    }

    fn builtin_tracemalloc_get_traceback_limit(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "tracemalloc.get_traceback_limit() expects no arguments",
            ));
        }
        Ok(Value::Int(self.tracemalloc_traceback_limit as i64))
    }

    fn builtin_tracemalloc_get_traced_memory(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "tracemalloc.get_traced_memory() expects no arguments",
            ));
        }
        Ok(self.heap.alloc_tuple(vec![Value::Int(0), Value::Int(0)]))
    }

    fn builtin_tracemalloc_get_tracemalloc_memory(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "tracemalloc.get_tracemalloc_memory() expects no arguments",
            ));
        }
        Ok(Value::Int(0))
    }

    fn builtin_tracemalloc_reset_peak(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "tracemalloc.reset_peak() expects no arguments",
            ));
        }
        Ok(Value::None)
    }

    fn builtin_tracemalloc_clear_traces(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "tracemalloc.clear_traces() expects no arguments",
            ));
        }
        self.tracemalloc_object_traces.clear();
        Ok(Value::None)
    }

    fn builtin_tracemalloc_get_traces(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "tracemalloc._get_traces() expects no arguments",
            ));
        }
        Ok(self.heap.alloc_list(Vec::new()))
    }

    fn builtin_tracemalloc_get_object_traceback(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "tracemalloc._get_object_traceback() expects one argument",
            ));
        }
        if !self.tracemalloc_enabled {
            return Ok(Value::None);
        }
        let Some(traceback) = self.tracemalloc_traceback_for_value(&args[0]) else {
            return Ok(Value::None);
        };
        let frames = traceback
            .into_iter()
            .map(|(filename, lineno)| {
                self.heap
                    .alloc_tuple(vec![Value::Str(filename), Value::Int(lineno as i64)])
            })
            .collect::<Vec<_>>();
        Ok(self.heap.alloc_tuple(frames))
    }

    pub(crate) fn clear_host_error_indicators(&mut self) {
        self.clear_active_exception();
        vm_extensions::cpython_clear_thread_error_indicator();
    }

    pub fn cache_source_text(&mut self, filename: &str, source: &str) {
        if filename.is_empty() {
            return;
        }
        // Preserve command-mode "<string>" source so later internal helper
        // eval/exec snippets cannot clobber traceback line resolution.
        if filename == "<string>" && self.source_text_cache.contains_key(filename) {
            return;
        }
        let mut lines = Vec::new();
        for line in source.lines() {
            lines.push(line.trim_end_matches('\r').to_string());
        }
        self.source_text_cache.insert(filename.to_string(), lines);
    }

    pub fn set_traceback_caret_enabled(&mut self, enabled: bool) {
        self.traceback_caret_enabled = enabled;
    }

    pub fn register_source_in_linecache(
        &mut self,
        code: &CodeObject,
        source: &str,
        filename: &str,
    ) {
        if self.import_module("linecache").is_err() {
            self.clear_active_exception();
            return;
        }
        let Some(linecache_module) = self.modules.get("linecache").cloned() else {
            return;
        };
        let register = match self.builtin_getattr(
            vec![
                Value::Module(linecache_module),
                Value::Str("_register_code".to_string()),
            ],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(_) => {
                self.clear_active_exception();
                return;
            }
        };
        let register_args = vec![
            Value::Code(Rc::new(code.clone())),
            Value::Str(source.to_string()),
            Value::Str(filename.to_string()),
        ];
        match self.call_internal_preserving_caller(register, register_args, HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => {}
            Ok(InternalCallOutcome::CallerExceptionHandled) | Err(_) => {
                self.clear_active_exception();
            }
        }
    }

    pub fn read_python_source_file(&mut self, path: &str) -> Result<String, RuntimeError> {
        let bytes = std::fs::read(path)
            .map_err(|err| RuntimeError::new(format!("failed to read {path}: {err}")))?;
        self.decode_python_source_bytes(&bytes, Some(path))
    }

    pub(super) fn decode_python_source_bytes(
        &mut self,
        bytes: &[u8],
        filename: Option<&str>,
    ) -> Result<String, RuntimeError> {
        let encoding = self.detect_python_source_encoding(bytes, filename)?;
        self.decode_text_bytes_with_codec_fallback(bytes, &encoding, "strict")
    }

    fn detect_python_source_encoding(
        &mut self,
        bytes: &[u8],
        filename: Option<&str>,
    ) -> Result<String, RuntimeError> {
        let mut cursor = 0usize;
        let mut first = python_source_next_line(bytes, &mut cursor);
        let mut bom_found = false;
        let mut default_encoding = "utf-8".to_string();
        if first.starts_with(&[0xEF, 0xBB, 0xBF]) {
            bom_found = true;
            first = &first[3..];
            default_encoding = "utf-8-sig".to_string();
        }
        if first.is_empty() {
            return Ok(default_encoding);
        }

        if let Some(encoding) =
            self.detect_python_source_cookie_encoding(first, bom_found, filename)?
        {
            self.check_python_source_line_decoding(first, &encoding, filename)?;
            return Ok(encoding);
        }

        if !python_source_line_is_blank_or_comment(first) {
            self.check_python_source_line_decoding(first, &default_encoding, filename)?;
            return Ok(default_encoding);
        }

        let second = python_source_next_line(bytes, &mut cursor);
        if second.is_empty() {
            self.check_python_source_line_decoding(first, &default_encoding, filename)?;
            return Ok(default_encoding);
        }

        if let Some(encoding) =
            self.detect_python_source_cookie_encoding(second, bom_found, filename)?
        {
            let mut combined = Vec::with_capacity(first.len() + second.len());
            combined.extend_from_slice(first);
            combined.extend_from_slice(second);
            self.check_python_source_line_decoding(&combined, &encoding, filename)?;
            return Ok(encoding);
        }

        let mut combined = Vec::with_capacity(first.len() + second.len());
        combined.extend_from_slice(first);
        combined.extend_from_slice(second);
        self.check_python_source_line_decoding(&combined, &default_encoding, filename)?;
        Ok(default_encoding)
    }

    fn detect_python_source_cookie_encoding(
        &mut self,
        line: &[u8],
        bom_found: bool,
        filename: Option<&str>,
    ) -> Result<Option<String>, RuntimeError> {
        let Some(encoding) = python_source_extract_cookie_encoding(line) else {
            return Ok(None);
        };
        self.call_builtin(
            BuiltinFunction::CodecsLookup,
            vec![Value::Str(encoding.clone())],
            HashMap::new(),
        )
        .map_err(|_| python_source_unknown_encoding_error(filename, &encoding))?;
        if bom_found {
            if encoding != "utf-8" {
                return Err(python_source_encoding_problem_error(filename));
            }
            return Ok(Some("utf-8-sig".to_string()));
        }
        Ok(Some(encoding))
    }

    fn check_python_source_line_decoding(
        &mut self,
        bytes: &[u8],
        encoding: &str,
        filename: Option<&str>,
    ) -> Result<(), RuntimeError> {
        if bytes.contains(&0) {
            return Err(RuntimeError::new(
                "SyntaxError: source code cannot contain null bytes",
            ));
        }
        self.decode_text_bytes_with_codec_fallback(bytes, encoding, "strict")
            .map(|_| ())
            .map_err(|_| python_source_invalid_or_missing_encoding_error(filename))
    }

    fn decode_text_bytes_with_codec_fallback(
        &mut self,
        bytes: &[u8],
        encoding: &str,
        errors: &str,
    ) -> Result<String, RuntimeError> {
        let normalized = normalize_codec_encoding(Value::Str(encoding.to_string()))
            .unwrap_or_else(|_| encoding.to_ascii_lowercase().replace('_', "-"));
        match decode_text_bytes(bytes, &normalized, errors) {
            Ok(text) => Ok(text),
            Err(err) if err.message.contains("unsupported encoding") => {
                self.decode_text_bytes_via_codec_lookup(bytes, encoding, errors)
            }
            Err(err) => Err(err),
        }
    }

    fn decode_text_bytes_via_codec_lookup(
        &mut self,
        bytes: &[u8],
        encoding: &str,
        errors: &str,
    ) -> Result<String, RuntimeError> {
        let codec_info = self.call_builtin(
            BuiltinFunction::CodecsLookup,
            vec![Value::Str(encoding.to_string())],
            HashMap::new(),
        )?;
        let decode = self.builtin_getattr(
            vec![codec_info, Value::Str("decode".to_string())],
            HashMap::new(),
        )?;
        let decoded = match self.call_internal_preserving_caller(
            decode,
            vec![
                self.heap.alloc_bytes(bytes.to_vec()),
                Value::Str(errors.to_string()),
            ],
            HashMap::new(),
        )? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(self.runtime_error_from_active_exception("decode() failed"));
            }
        };
        let Value::Tuple(tuple_obj) = decoded else {
            return Err(RuntimeError::new(
                "TypeError: decode codec must return a tuple",
            ));
        };
        let Object::Tuple(items) = &*tuple_obj.kind() else {
            return Err(RuntimeError::new(
                "TypeError: decode codec must return a tuple",
            ));
        };
        let Some(first) = items.first() else {
            return Err(RuntimeError::new(
                "TypeError: decode codec must return a non-empty tuple",
            ));
        };
        match first {
            Value::Str(text) => Ok(text.clone()),
            _ => Err(RuntimeError::new(
                "TypeError: decoder should return a string result",
            )),
        }
    }

    pub(super) fn register_compiled_code_metadata(
        &mut self,
        code: &Rc<CodeObject>,
        mode: CompiledCodeMode,
        source: Option<&str>,
    ) {
        let key = Rc::as_ptr(code) as usize;
        self.compiled_code_metadata.insert(
            key,
            CompiledCodeMetadata {
                mode,
                source: source.map(str::to_string),
            },
        );
    }

    pub(super) fn compiled_code_metadata(
        &self,
        code: &Rc<CodeObject>,
    ) -> Option<&CompiledCodeMetadata> {
        let key = Rc::as_ptr(code) as usize;
        self.compiled_code_metadata.get(&key)
    }

    pub(super) fn traceback_source_line(&mut self, filename: &str, line: usize) -> Option<String> {
        if filename.is_empty() || line == 0 {
            return None;
        }
        if !self.source_text_cache.contains_key(filename)
            && !filename.starts_with('<')
            && let Ok(source) = self.read_python_source_file(filename)
        {
            self.cache_source_text(filename, &source);
        }
        self.source_text_cache
            .get(filename)
            .and_then(|lines| lines.get(line.saturating_sub(1)))
            .cloned()
    }

    pub fn execute(&mut self, code: &CodeObject) -> Result<Value, RuntimeError> {
        self.clear_host_error_indicators();
        self.frames.clear();
        self.generator_states.clear();
        self.generator_returns.clear();
        self.pending_generator_exception = None;
        self.active_generator_resume = None;
        self.active_generator_resume_boundary = None;
        self.generator_resume_outcome = None;
        self.run_stop_depth = None;
        self.pending_import_drain_depth = 0;
        let code = Rc::new(code.clone());
        let cells = self.build_cells(&code, Vec::new());
        self.push_frame_checked(Box::new(Frame::new(
            code,
            self.main_module.clone(),
            true,
            false,
            cells,
            None,
        )))?;
        let result = self.run();
        if result.is_err() {
            self.clear_host_error_indicators();
        }
        result
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn deadline_from_timeout_ms(timeout_ms: u32) -> Option<VmExecutionDeadline> {
        Instant::now().checked_add(Duration::from_millis(timeout_ms as u64))
    }

    #[cfg(target_arch = "wasm32")]
    fn deadline_from_timeout_ms(timeout_ms: u32) -> Option<VmExecutionDeadline> {
        Some(Date::now() + timeout_ms as f64)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn deadline_reached(deadline: VmExecutionDeadline) -> bool {
        Instant::now() >= deadline
    }

    #[cfg(target_arch = "wasm32")]
    fn deadline_reached(deadline: VmExecutionDeadline) -> bool {
        Date::now() >= deadline
    }

    fn execution_deadline_reached(&self) -> bool {
        self.execution_deadline
            .map(Self::deadline_reached)
            .unwrap_or(false)
    }

    pub fn execute_with_timeout_ms(
        &mut self,
        code: &CodeObject,
        timeout_ms: u32,
    ) -> Result<Value, RuntimeError> {
        if timeout_ms == 0 {
            return self.execute(code);
        }
        let previous_deadline = self.execution_deadline;
        self.execution_deadline = Self::deadline_from_timeout_ms(timeout_ms);
        let result = self.execute(code);
        self.execution_deadline = previous_deadline;
        result
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

    fn detect_cpython_stdlib_root(&self) -> Option<PathBuf> {
        if let Some(path) = self.host.env_var("PYRS_CPYTHON_LIB") {
            let path = PathBuf::from(path);
            if self.host.path_is_dir(&path) {
                return Some(path);
            }
        }
        let local = PathBuf::from(".local/Python-3.14.3/Lib");
        if self.host.path_is_dir(&local) {
            return Some(local);
        }
        let framework =
            PathBuf::from("/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14");
        if self.host.path_is_dir(&framework) {
            return Some(framework);
        }
        None
    }

    fn build_bootstrap_stdlib_module_names(&self, stdlib_root: Option<&Path>) -> Vec<String> {
        let mut names: HashSet<String> = HashSet::new();
        for base in [
            "sys", "builtins", "_imp", "_io", "marshal", "posix", "errno",
        ] {
            names.insert(base.to_string());
        }
        names.extend(self.modules.keys().cloned());

        let Some(root) = stdlib_root else {
            let mut sorted = names.into_iter().collect::<Vec<_>>();
            sorted.sort();
            return sorted;
        };

        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.join("__init__.py").is_file()
                        && let Some(name) = path.file_name().and_then(|value| value.to_str())
                    {
                        names.insert(name.to_string());
                    }
                    continue;
                }
                if !path.is_file() {
                    continue;
                }
                if path.extension().and_then(|value| value.to_str()) == Some("py")
                    && let Some(stem) = path.file_stem().and_then(|value| value.to_str())
                    && stem != "__init__"
                {
                    names.insert(stem.to_string());
                }
            }
        }

        let dynload = root.join("lib-dynload");
        if let Ok(entries) = fs::read_dir(dynload) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let ext = path.extension().and_then(|value| value.to_str());
                if !matches!(ext, Some("so" | "pyd" | "dylib")) {
                    continue;
                }
                if let Some(filename) = path.file_name().and_then(|value| value.to_str())
                    && let Some(module_name) = filename.split('.').next()
                    && !module_name.is_empty()
                {
                    names.insert(module_name.to_string());
                }
            }
        }

        let mut sorted = names.into_iter().collect::<Vec<_>>();
        sorted.sort();
        sorted
    }

    fn alloc_sys_tuple_struct_class(&mut self, class_name: &str, fields: &[&str]) -> ObjRef {
        let mut bases = Vec::new();
        if let Some(Value::Class(tuple_class)) = self.builtins.get("tuple").cloned() {
            bases.push(tuple_class);
        }
        let class = match self
            .heap
            .alloc_class(ClassObject::new(class_name.to_string(), bases))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("sys".to_string()));
            class_data
                .attrs
                .insert("__pyrs_tuple_backed_type__".to_string(), Value::Bool(true));
            class_data.attrs.insert(
                "__pyrs_disallow_instantiation__".to_string(),
                Value::Bool(true),
            );
            class_data.attrs.insert(
                "_fields".to_string(),
                self.heap.alloc_tuple(
                    fields
                        .iter()
                        .map(|field| Value::Str((*field).to_string()))
                        .collect(),
                ),
            );
            class_data
                .attrs
                .insert("n_fields".to_string(), Value::Int(fields.len() as i64));
            class_data.attrs.insert(
                "n_sequence_fields".to_string(),
                Value::Int(fields.len() as i64),
            );
            class_data
                .attrs
                .insert("n_unnamed_fields".to_string(), Value::Int(0));
        }
        class
    }

    fn make_sys_tuple_struct_instance(
        &mut self,
        class: &ObjRef,
        fields: &[&str],
        values: Vec<Value>,
    ) -> Value {
        let instance = self.alloc_instance_for_class(class);
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            for (field, value) in fields.iter().zip(values.iter()) {
                instance_data
                    .attrs
                    .insert((*field).to_string(), value.clone());
            }
            instance_data.attrs.insert(
                TUPLE_BACKING_STORAGE_ATTR.to_string(),
                self.heap.alloc_tuple(values),
            );
        }
        Value::Instance(instance)
    }

    fn install_sys_module(&mut self) {
        let sys_module = match self.heap.alloc_module(ModuleObject::new("sys")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &sys_module,
            "sys",
            None,
            None,
            None,
            false,
            Vec::new(),
            false,
        );
        if let Object::Module(module_data) = &mut *sys_module.kind_mut() {
            let flags_fields = [
                "debug",
                "inspect",
                "interactive",
                "optimize",
                "dont_write_bytecode",
                "no_user_site",
                "no_site",
                "ignore_environment",
                "verbose",
                "bytes_warning",
                "quiet",
                "hash_randomization",
                "isolated",
                "dev_mode",
                "utf8_mode",
                "warn_default_encoding",
                "safe_path",
                "int_max_str_digits",
            ];
            let flags_class = self.alloc_sys_tuple_struct_class("flags", &flags_fields);
            let flags_value = self.make_sys_tuple_struct_instance(
                &flags_class,
                &flags_fields,
                vec![
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(1),
                    Value::Int(0),
                    Value::Bool(false),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Bool(false),
                    Value::Int(4300),
                ],
            );
            if let Value::Instance(flags_instance) = flags_value.clone()
                && let Object::Instance(instance_data) = &mut *flags_instance.kind_mut()
            {
                instance_data.attrs.insert("gil".to_string(), Value::Int(1));
                instance_data
                    .attrs
                    .insert("thread_inherit_context".to_string(), Value::Int(0));
                instance_data
                    .attrs
                    .insert("context_aware_warnings".to_string(), Value::Int(0));
            }
            module_data.globals.insert("flags".to_string(), flags_value);

            let version_info_fields = ["major", "minor", "micro", "releaselevel", "serial"];
            let version_info_class =
                self.alloc_sys_tuple_struct_class("version_info", &version_info_fields);
            let version_info_value = self.make_sys_tuple_struct_instance(
                &version_info_class,
                &version_info_fields,
                vec![
                    Value::Int(3),
                    Value::Int(14),
                    Value::Int(0),
                    Value::Str("final".to_string()),
                    Value::Int(0),
                ],
            );
            module_data
                .globals
                .insert("version_info".to_string(), version_info_value.clone());
            module_data
                .globals
                .insert("api_version".to_string(), Value::Int(1013));
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
                impl_data
                    .globals
                    .insert("version".to_string(), version_info_value);
                impl_data
                    .globals
                    .insert("hexversion".to_string(), Value::Int(0x030e00f0));
                impl_data.globals.insert(
                    "supports_isolated_interpreters".to_string(),
                    Value::Bool(false),
                );
                impl_data
                    .globals
                    .insert("_multiarch".to_string(), Value::Str(String::new()));
            }
            module_data
                .globals
                .insert("implementation".to_string(), Value::Module(implementation));
            let process_args = self.host.process_args();
            let argv = process_args
                .iter()
                .cloned()
                .map(Value::Str)
                .collect::<Vec<_>>();
            module_data
                .globals
                .insert("argv".to_string(), self.heap.alloc_list(argv.clone()));
            module_data
                .globals
                .insert("orig_argv".to_string(), self.heap.alloc_list(argv));
            let executable_path = self
                .host
                .current_exe()
                .or_else(|| process_args.first().map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("pyrs"));
            let executable = executable_path.to_string_lossy().to_string();
            let mut inferred_prefix = executable_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            if inferred_prefix
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name == "debug" || name == "release")
                .unwrap_or(false)
            {
                if let Some(parent) = inferred_prefix.parent() {
                    inferred_prefix = parent.to_path_buf();
                }
            }
            if inferred_prefix
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name == "target")
                .unwrap_or(false)
            {
                if let Some(parent) = inferred_prefix.parent() {
                    inferred_prefix = parent.to_path_buf();
                }
            }
            let inferred_prefix_str = inferred_prefix.to_string_lossy().to_string();
            let venv_prefix = self
                .host
                .env_var_os("VIRTUAL_ENV")
                .map(PathBuf::from)
                .filter(|path| self.host.path_is_dir(path))
                .map(|path| path.to_string_lossy().to_string());
            let prefix = venv_prefix
                .clone()
                .unwrap_or_else(|| inferred_prefix_str.clone());
            let exec_prefix = prefix.clone();
            let base_prefix = inferred_prefix_str.clone();
            let base_exec_prefix = inferred_prefix_str.clone();
            module_data
                .globals
                .insert("executable".to_string(), Value::Str(executable));
            module_data
                .globals
                .insert("prefix".to_string(), Value::Str(prefix.clone()));
            module_data
                .globals
                .insert("base_prefix".to_string(), Value::Str(base_prefix));
            module_data
                .globals
                .insert("exec_prefix".to_string(), Value::Str(exec_prefix));
            module_data
                .globals
                .insert("base_exec_prefix".to_string(), Value::Str(base_exec_prefix));
            let platform = match self.host.os_name() {
                "macos" => "darwin",
                other => other,
            };
            module_data
                .globals
                .insert("platform".to_string(), Value::Str(platform.to_string()));
            let stdlib_root = self.detect_cpython_stdlib_root();
            let stdlib_dir = stdlib_root
                .as_ref()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default();
            module_data
                .globals
                .insert("_stdlib_dir".to_string(), Value::Str(stdlib_dir));
            let stdlib_module_names =
                self.build_bootstrap_stdlib_module_names(stdlib_root.as_deref());
            module_data.globals.insert(
                "stdlib_module_names".to_string(),
                self.heap.alloc_frozenset(
                    stdlib_module_names
                        .into_iter()
                        .map(Value::Str)
                        .collect::<Vec<_>>(),
                ),
            );
            let thread_info_fields = ["name", "lock", "version"];
            let thread_name = if platform == "wasi" {
                Value::Str("pthread-stubs".to_string())
            } else if platform == "win32" {
                Value::Str("nt".to_string())
            } else {
                Value::Str("pthread".to_string())
            };
            let thread_lock = if platform == "win32" {
                Value::None
            } else {
                Value::Str("mutex+cond".to_string())
            };
            let thread_info_class =
                self.alloc_sys_tuple_struct_class("thread_info", &thread_info_fields);
            let thread_info = self.make_sys_tuple_struct_instance(
                &thread_info_class,
                &thread_info_fields,
                vec![thread_name, thread_lock, Value::None],
            );
            module_data
                .globals
                .insert("thread_info".to_string(), thread_info);
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
                "getdefaultencoding".to_string(),
                Value::Builtin(BuiltinFunction::SysGetDefaultEncoding),
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
                "getsizeof".to_string(),
                Value::Builtin(BuiltinFunction::SysGetSizeOf),
            );
            module_data.globals.insert(
                "getrecursionlimit".to_string(),
                Value::Builtin(BuiltinFunction::SysGetRecursionLimit),
            );
            module_data.globals.insert(
                "setrecursionlimit".to_string(),
                Value::Builtin(BuiltinFunction::SysSetRecursionLimit),
            );
            module_data.globals.insert(
                "getswitchinterval".to_string(),
                Value::Builtin(BuiltinFunction::SysGetSwitchInterval),
            );
            module_data.globals.insert(
                "setswitchinterval".to_string(),
                Value::Builtin(BuiltinFunction::SysSetSwitchInterval),
            );
            module_data.globals.insert(
                "_clear_type_descriptors".to_string(),
                Value::Builtin(BuiltinFunction::SysClearTypeDescriptors),
            );
            module_data.globals.insert(
                "intern".to_string(),
                Value::Builtin(BuiltinFunction::SysIntern),
            );
            module_data.globals.insert(
                "audit".to_string(),
                Value::Builtin(BuiltinFunction::SysAudit),
            );
            module_data.globals.insert(
                "addaudithook".to_string(),
                Value::Builtin(BuiltinFunction::SysAddAuditHook),
            );
            module_data.globals.insert(
                "excepthook".to_string(),
                Value::Builtin(BuiltinFunction::SysExcepthook),
            );
            module_data.globals.insert(
                "__excepthook__".to_string(),
                Value::Builtin(BuiltinFunction::SysExcepthook),
            );
            module_data.globals.insert(
                "displayhook".to_string(),
                Value::Builtin(BuiltinFunction::SysDisplayHook),
            );
            module_data.globals.insert(
                "__displayhook__".to_string(),
                Value::Builtin(BuiltinFunction::SysDisplayHook),
            );
            module_data.globals.insert(
                "unraisablehook".to_string(),
                Value::Builtin(BuiltinFunction::SysUnraisableHook),
            );
            module_data.globals.insert(
                "__unraisablehook__".to_string(),
                Value::Builtin(BuiltinFunction::SysUnraisableHook),
            );
            module_data.globals.insert(
                "breakpointhook".to_string(),
                Value::Builtin(BuiltinFunction::SysBreakpointHook),
            );
            module_data.globals.insert(
                "__breakpointhook__".to_string(),
                Value::Builtin(BuiltinFunction::SysBreakpointHook),
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
                    let errors = if name.ends_with("stderr") {
                        "backslashreplace"
                    } else {
                        "surrogateescape"
                    };
                    stream_data
                        .globals
                        .insert("errors".to_string(), Value::Str(errors.to_string()));
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
            module_data
                .globals
                .insert("maxunicode".to_string(), Value::Int(0x10ffff));
            let float_info_fields = [
                "max",
                "max_exp",
                "max_10_exp",
                "min",
                "min_exp",
                "min_10_exp",
                "dig",
                "mant_dig",
                "epsilon",
                "radix",
                "rounds",
            ];
            let float_info_class =
                self.alloc_sys_tuple_struct_class("float_info", &float_info_fields);
            let float_info = self.make_sys_tuple_struct_instance(
                &float_info_class,
                &float_info_fields,
                vec![
                    Value::Float(f64::MAX),
                    Value::Int(f64::MAX_EXP as i64),
                    Value::Int(f64::MAX_10_EXP as i64),
                    Value::Float(f64::MIN_POSITIVE),
                    Value::Int(f64::MIN_EXP as i64),
                    Value::Int(f64::MIN_10_EXP as i64),
                    Value::Int(f64::DIGITS as i64),
                    Value::Int(f64::MANTISSA_DIGITS as i64),
                    Value::Float(f64::EPSILON),
                    Value::Int(2),
                    Value::Int(1),
                ],
            );
            module_data
                .globals
                .insert("float_info".to_string(), float_info);
            let int_info_fields = [
                "bits_per_digit",
                "sizeof_digit",
                "default_max_str_digits",
                "str_digits_check_threshold",
            ];
            let int_info_class = self.alloc_sys_tuple_struct_class("int_info", &int_info_fields);
            let int_info = self.make_sys_tuple_struct_instance(
                &int_info_class,
                &int_info_fields,
                vec![
                    Value::Int(30),
                    Value::Int(4),
                    Value::Int(4300),
                    Value::Int(640),
                ],
            );
            module_data.globals.insert("int_info".to_string(), int_info);
            module_data.globals.insert(
                "float_repr_style".to_string(),
                Value::Str("short".to_string()),
            );
            let hash_info_fields = [
                "width",
                "modulus",
                "inf",
                "nan",
                "imag",
                "algorithm",
                "hash_bits",
                "seed_bits",
                "cutoff",
            ];
            let hash_info_class = self.alloc_sys_tuple_struct_class("hash_info", &hash_info_fields);
            let hash_info = self.make_sys_tuple_struct_instance(
                &hash_info_class,
                &hash_info_fields,
                vec![
                    Value::Int(64),
                    Value::Int(2_305_843_009_213_693_951),
                    Value::Int(314_159),
                    Value::Int(0),
                    Value::Int(1_000_003),
                    Value::Str("siphash13".to_string()),
                    Value::Int(64),
                    Value::Int(128),
                    Value::Int(0),
                ],
            );
            module_data
                .globals
                .insert("hash_info".to_string(), hash_info);
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
                        .insert("NO_EVENTS".to_string(), Value::Int(0));
                    events_data.globals.insert(
                        "PY_START".to_string(),
                        Value::Int(MONITORING_EVENT_PY_START),
                    );
                    events_data.globals.insert(
                        "PY_RESUME".to_string(),
                        Value::Int(MONITORING_EVENT_PY_RESUME),
                    );
                    events_data.globals.insert(
                        "PY_RETURN".to_string(),
                        Value::Int(MONITORING_EVENT_PY_RETURN),
                    );
                    events_data.globals.insert(
                        "PY_YIELD".to_string(),
                        Value::Int(MONITORING_EVENT_PY_YIELD),
                    );
                    events_data
                        .globals
                        .insert("CALL".to_string(), Value::Int(MONITORING_EVENT_CALL));
                    events_data
                        .globals
                        .insert("LINE".to_string(), Value::Int(MONITORING_EVENT_LINE));
                    events_data.globals.insert(
                        "INSTRUCTION".to_string(),
                        Value::Int(MONITORING_EVENT_INSTRUCTION),
                    );
                    events_data
                        .globals
                        .insert("JUMP".to_string(), Value::Int(MONITORING_EVENT_JUMP));
                    events_data.globals.insert(
                        "BRANCH_LEFT".to_string(),
                        Value::Int(MONITORING_EVENT_BRANCH_LEFT),
                    );
                    events_data.globals.insert(
                        "BRANCH_RIGHT".to_string(),
                        Value::Int(MONITORING_EVENT_BRANCH_RIGHT),
                    );
                    events_data.globals.insert(
                        "STOP_ITERATION".to_string(),
                        Value::Int(MONITORING_EVENT_STOP_ITERATION),
                    );
                    events_data
                        .globals
                        .insert("RAISE".to_string(), Value::Int(MONITORING_EVENT_RAISE));
                    events_data.globals.insert(
                        "EXCEPTION_HANDLED".to_string(),
                        Value::Int(MONITORING_EVENT_EXCEPTION_HANDLED),
                    );
                    events_data.globals.insert(
                        "PY_UNWIND".to_string(),
                        Value::Int(MONITORING_EVENT_PY_UNWIND),
                    );
                    events_data.globals.insert(
                        "PY_THROW".to_string(),
                        Value::Int(MONITORING_EVENT_PY_THROW),
                    );
                    events_data
                        .globals
                        .insert("RERAISE".to_string(), Value::Int(MONITORING_EVENT_RERAISE));
                    events_data.globals.insert(
                        "C_RETURN".to_string(),
                        Value::Int(MONITORING_EVENT_C_RETURN),
                    );
                    events_data
                        .globals
                        .insert("C_RAISE".to_string(), Value::Int(MONITORING_EVENT_C_RAISE));
                    events_data
                        .globals
                        .insert("BRANCH".to_string(), Value::Int(MONITORING_EVENT_BRANCH));
                }
                monitoring_data
                    .globals
                    .insert("events".to_string(), Value::Module(events));
                monitoring_data
                    .globals
                    .insert("DEBUGGER_ID".to_string(), Value::Int(0));
                monitoring_data
                    .globals
                    .insert("COVERAGE_ID".to_string(), Value::Int(1));
                monitoring_data
                    .globals
                    .insert("PROFILER_ID".to_string(), Value::Int(2));
                monitoring_data
                    .globals
                    .insert("OPTIMIZER_ID".to_string(), Value::Int(5));
                monitoring_data
                    .globals
                    .insert("DISABLE".to_string(), Value::Int(-1));
                monitoring_data.globals.insert(
                    "get_tool".to_string(),
                    Value::Builtin(BuiltinFunction::SysMonitoringGetTool),
                );
                monitoring_data.globals.insert(
                    "use_tool_id".to_string(),
                    Value::Builtin(BuiltinFunction::SysMonitoringUseToolId),
                );
                monitoring_data.globals.insert(
                    "clear_tool_id".to_string(),
                    Value::Builtin(BuiltinFunction::SysMonitoringClearToolId),
                );
                monitoring_data.globals.insert(
                    "free_tool_id".to_string(),
                    Value::Builtin(BuiltinFunction::SysMonitoringFreeToolId),
                );
                monitoring_data.globals.insert(
                    "register_callback".to_string(),
                    Value::Builtin(BuiltinFunction::SysMonitoringRegisterCallback),
                );
                monitoring_data.globals.insert(
                    "get_events".to_string(),
                    Value::Builtin(BuiltinFunction::SysMonitoringGetEvents),
                );
                monitoring_data.globals.insert(
                    "set_events".to_string(),
                    Value::Builtin(BuiltinFunction::SysMonitoringSetEvents),
                );
                monitoring_data.globals.insert(
                    "get_local_events".to_string(),
                    Value::Builtin(BuiltinFunction::SysMonitoringGetLocalEvents),
                );
                monitoring_data.globals.insert(
                    "set_local_events".to_string(),
                    Value::Builtin(BuiltinFunction::SysMonitoringSetLocalEvents),
                );
                monitoring_data.globals.insert(
                    "restart_events".to_string(),
                    Value::Builtin(BuiltinFunction::SysMonitoringRestartEvents),
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
                    Value::Builtin(BuiltinFunction::Bool),
                );
                jit_data.globals.insert(
                    "is_available".to_string(),
                    Value::Builtin(BuiltinFunction::Bool),
                );
                jit_data.globals.insert(
                    "is_active".to_string(),
                    Value::Builtin(BuiltinFunction::Bool),
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
                "_getframemodulename".to_string(),
                Value::Builtin(BuiltinFunction::SysGetFrameModuleName),
            );
            module_data.globals.insert(
                "_current_frames".to_string(),
                Value::Builtin(BuiltinFunction::SysCurrentFrames),
            );
            module_data.globals.insert(
                "call_tracing".to_string(),
                Value::Builtin(BuiltinFunction::SysCallTracing),
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
            module_data.globals.insert(
                "is_finalizing".to_string(),
                Value::Builtin(BuiltinFunction::SysIsFinalizing),
            );
            module_data.globals.insert(
                "is_remote_debug_enabled".to_string(),
                Value::Builtin(BuiltinFunction::SysIsRemoteDebugEnabled),
            );
            module_data.globals.insert(
                "_is_gil_enabled".to_string(),
                Value::Builtin(BuiltinFunction::SysIsGilEnabled),
            );
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
                    .alloc_list(vec![Value::Builtin(BuiltinFunction::ImportlibPathHook)]),
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

    fn set_sys_flag_field(&mut self, field: &str, value: Value) {
        let Some(sys_module) = self.modules.get("sys").cloned() else {
            return;
        };
        let flags_value = match &*sys_module.kind() {
            Object::Module(module_data) => module_data.globals.get("flags").cloned(),
            _ => None,
        };
        let Some(flags_value) = flags_value else {
            return;
        };
        match flags_value {
            Value::Instance(flags) => {
                let mut tuple_index = None;
                if let Object::Instance(instance_data) = &*flags.kind()
                    && let Object::Class(class_data) = &*instance_data.class.kind()
                    && let Some(Value::Tuple(fields_tuple)) = class_data.attrs.get("_fields")
                    && let Object::Tuple(fields) = &*fields_tuple.kind()
                {
                    tuple_index = fields.iter().position(
                        |candidate| matches!(candidate, Value::Str(name) if name == field),
                    );
                }
                if let Object::Instance(instance_data) = &mut *flags.kind_mut() {
                    instance_data.attrs.insert(field.to_string(), value.clone());
                    if let Some(index) = tuple_index
                        && let Some(Value::Tuple(tuple)) =
                            instance_data.attrs.get(TUPLE_BACKING_STORAGE_ATTR).cloned()
                        && let Object::Tuple(tuple_values) = &mut *tuple.kind_mut()
                        && index < tuple_values.len()
                    {
                        tuple_values[index] = value;
                    }
                }
            }
            Value::Module(flags_module) => {
                if let Object::Module(flags_data) = &mut *flags_module.kind_mut() {
                    flags_data.globals.insert(field.to_string(), value);
                }
            }
            _ => {}
        }
    }

    pub fn set_sys_no_site_flag(&mut self, no_site: bool) {
        self.set_sys_flag_field("no_site", Value::Int(if no_site { 1 } else { 0 }));
    }

    pub fn set_sys_interactive_flag(&mut self, interactive: bool) {
        self.set_sys_flag_field("interactive", Value::Int(if interactive { 1 } else { 0 }));
    }

    pub fn set_sys_argv(&mut self, argv: Vec<String>) {
        let Some(sys_module) = self.modules.get("sys").cloned() else {
            return;
        };
        let values = argv.into_iter().map(Value::Str).collect::<Vec<_>>();
        if let Object::Module(module_data) = &mut *sys_module.kind_mut() {
            module_data
                .globals
                .insert("argv".to_string(), self.heap.alloc_list(values));
        }
    }

    pub fn set_sys_warnoptions(&mut self, warnoptions: Vec<String>) {
        let Some(sys_module) = self.modules.get("sys").cloned() else {
            return;
        };
        let values = warnoptions.into_iter().map(Value::Str).collect::<Vec<_>>();
        if let Object::Module(module_data) = &mut *sys_module.kind_mut() {
            module_data
                .globals
                .insert("warnoptions".to_string(), self.heap.alloc_list(values));
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
            module_data.globals.insert(
                "spec_from_loader".to_string(),
                Value::Builtin(BuiltinFunction::FrozenImportlibSpecFromLoader),
            );
            module_data.globals.insert(
                "module_from_spec".to_string(),
                Value::Builtin(BuiltinFunction::ImportlibModuleFromSpec),
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
                    "_pack_uint32",
                    BuiltinFunction::FrozenImportlibExternalPackUint32,
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
                ("__import__", BuiltinFunction::Import),
                ("_setup", BuiltinFunction::NoOp),
                ("_install", BuiltinFunction::NoOp),
                ("_install_external_importers", BuiltinFunction::NoOp),
                (
                    "spec_from_loader",
                    BuiltinFunction::FrozenImportlibSpecFromLoader,
                ),
                (
                    "_verbose_message",
                    BuiltinFunction::FrozenImportlibVerboseMessage,
                ),
            ],
            vec![
                (
                    "ModuleSpec",
                    self.heap
                        .alloc_class(ClassObject::new("ModuleSpec".to_string(), Vec::new())),
                ),
                ("BuiltinImporter", {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BuiltinImporter".to_string(), Vec::new()));
                    if let Value::Class(class_obj) = &class
                        && let Object::Class(class_data) = &mut *class_obj.kind_mut()
                    {
                        class_data.attrs.insert(
                            "__module__".to_string(),
                            Value::Str("_frozen_importlib".to_string()),
                        );
                        class_data
                            .attrs
                            .insert("_ORIGIN".to_string(), Value::Str("built-in".to_string()));
                    }
                    class
                }),
                ("FrozenImporter", {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("FrozenImporter".to_string(), Vec::new()));
                    if let Value::Class(class_obj) = &class
                        && let Object::Class(class_data) = &mut *class_obj.kind_mut()
                    {
                        class_data.attrs.insert(
                            "__module__".to_string(),
                            Value::Str("_frozen_importlib".to_string()),
                        );
                        class_data
                            .attrs
                            .insert("_ORIGIN".to_string(), Value::Str("frozen".to_string()));
                    }
                    class
                }),
            ],
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
            vec![("TIER2_THRESHOLD", Value::Int(0))],
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
                ("call_in_temporary_c_thread", BuiltinFunction::NoOp),
                ("join_temporary_c_thread", BuiltinFunction::NoOp),
                ("exception_print", BuiltinFunction::TestCapiExceptionPrint),
                ("config_get", BuiltinFunction::TestCapiConfigGet),
                (
                    "pyobject_vectorcall",
                    BuiltinFunction::TestCapiPyObjectVectorcall,
                ),
            ],
            vec![
                ("INT_MAX", Value::Int(i32::MAX as i64)),
                ("INT_MIN", Value::Int(i32::MIN as i64)),
                ("UINT_MAX", Value::Int(u32::MAX as i64)),
                ("LLONG_MAX", Value::Int(i64::MAX)),
                ("LLONG_MIN", Value::Int(i64::MIN)),
                (
                    "ULLONG_MAX",
                    Value::BigInt(Box::new(BigInt::from_u64(u64::MAX))),
                ),
                ("PY_SSIZE_T_MAX", Value::Int(i64::MAX)),
                ("PY_SSIZE_T_MIN", Value::Int(i64::MIN)),
                ("MethInstance", Value::Class(meth_instance_class)),
                ("MethClass", Value::Class(meth_class_class)),
                ("MethStatic", Value::Class(meth_static_class)),
            ],
        );
    }

    fn install_random_module(&mut self) {
        let random_module = match self.heap.alloc_module(ModuleObject::new("_random")) {
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
        let system_random_class = match self.heap.alloc_class(ClassObject::new(
            "SystemRandom".to_string(),
            vec![random_class.clone()],
        )) {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *random_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::RandomSeed),
            );
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
        if let Object::Class(class_data) = &mut *system_random_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::RandomSeed),
            );
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
            "_random",
            None,
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
            module_data.globals.insert(
                "SystemRandom".to_string(),
                Value::Class(system_random_class.clone()),
            );
        }
        self.register_module("_random", random_module);

        // Keep a fallback `random` module for environments where stdlib `random.py`
        // is not present on `sys.path` (tests and minimal bootstrap).
        let random_fallback = match self.heap.alloc_module(ModuleObject::new("random")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &random_fallback,
            "random",
            None,
            None,
            Some(BUILTIN_MODULE_LOADER),
            false,
            Vec::new(),
            false,
        );
        if let Object::Module(module_data) = &mut *random_fallback.kind_mut() {
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
                .insert("Random".to_string(), Value::Class(random_class));
            module_data.globals.insert(
                "SystemRandom".to_string(),
                Value::Class(system_random_class),
            );
        }
        self.register_module("random", random_fallback);
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

    fn alloc_bootstrap_class_value(&mut self, name: &str, module_name: &str) -> Value {
        let mut bases = Vec::new();
        if name != "object"
            && let Some(Value::Class(object_class)) = self.builtins.get("object")
        {
            bases.push(object_class.clone());
        }
        let class = match self
            .heap
            .alloc_class(ClassObject::new(name.to_string(), bases.clone()))
        {
            Value::Class(class) => class,
            other => return other,
        };
        let mro = self.build_class_mro(&class, &bases).unwrap_or_else(|_| {
            let mut fallback = vec![class.clone()];
            for base in &bases {
                if !fallback.iter().any(|entry| entry.id() == base.id()) {
                    fallback.push(base.clone());
                }
            }
            fallback
        });
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data.mro = mro.clone();
            class_data
                .attrs
                .insert("__name__".to_string(), Value::Str(name.to_string()));
            class_data
                .attrs
                .insert("__qualname__".to_string(), Value::Str(name.to_string()));
            class_data.attrs.insert(
                "__module__".to_string(),
                Value::Str(module_name.to_string()),
            );
            class_data
                .attrs
                .insert("__flags__".to_string(), Value::Int(PY_TPFLAGS_HEAPTYPE));
            class_data.attrs.insert(
                "__bases__".to_string(),
                self.heap
                    .alloc_tuple(bases.iter().cloned().map(Value::Class).collect::<Vec<_>>()),
            );
            class_data.attrs.insert(
                "__mro__".to_string(),
                self.heap
                    .alloc_tuple(mro.into_iter().map(Value::Class).collect::<Vec<_>>()),
            );
        }
        Value::Class(class)
    }

    fn normalize_bootstrap_module_classes(&mut self) {
        let Some(Value::Class(object_class)) = self.builtins.get("object") else {
            return;
        };
        let object_class = object_class.clone();
        let module_refs = self.modules.values().cloned().collect::<Vec<_>>();
        let mut visited = HashSet::new();
        for module in module_refs {
            let (module_name, classes) = match &*module.kind() {
                Object::Module(module_data) => (
                    module_data.name.clone(),
                    module_data
                        .globals
                        .values()
                        .filter_map(|value| match value {
                            Value::Class(class) => Some(class.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => continue,
            };
            for class in classes {
                if !visited.insert(class.id()) {
                    continue;
                }
                let (class_name, mut bases) = match &*class.kind() {
                    Object::Class(class_data) => {
                        (class_data.name.clone(), class_data.bases.clone())
                    }
                    _ => continue,
                };
                if class_name != "object" && class_name != "type" && bases.is_empty() {
                    bases.push(object_class.clone());
                }
                let mro = self.build_class_mro(&class, &bases).unwrap_or_else(|_| {
                    let mut fallback = vec![class.clone()];
                    for base in &bases {
                        if !fallback.iter().any(|entry| entry.id() == base.id()) {
                            fallback.push(base.clone());
                        }
                    }
                    fallback
                });
                let Object::Class(class_data) = &mut *class.kind_mut() else {
                    continue;
                };
                class_data.bases = bases.clone();
                class_data
                    .attrs
                    .insert("__name__".to_string(), Value::Str(class_data.name.clone()));
                class_data
                    .attrs
                    .entry("__qualname__".to_string())
                    .or_insert_with(|| Value::Str(class_data.name.clone()));
                class_data
                    .attrs
                    .entry("__module__".to_string())
                    .or_insert(Value::Str(module_name.clone()));
                class_data.attrs.insert(
                    "__bases__".to_string(),
                    self.heap
                        .alloc_tuple(bases.iter().cloned().map(Value::Class).collect::<Vec<_>>()),
                );
                class_data.mro = mro.clone();
                class_data.attrs.insert(
                    "__mro__".to_string(),
                    self.heap
                        .alloc_tuple(mro.into_iter().map(Value::Class).collect::<Vec<_>>()),
                );
            }
        }
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
            if let Some(cell) = frame.cells.get(index)
                && let Object::Cell(cell_data) = &*cell.kind()
            {
                return cell_data.value.clone();
            }
            return None;
        }
    }
    for (offset, free_name) in frame.code.freevars.iter().enumerate() {
        if free_name == name {
            if let Some(cell) = frame.cells.get(cellvar_len + offset)
                && let Object::Cell(cell_data) = &*cell.kind()
            {
                return cell_data.value.clone();
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
            .ok_or_else(|| RuntimeError::overflow_error("integer overflow")),
        Value::Instance(instance_obj) => {
            let Object::Instance(instance_data) = &*instance_obj.kind() else {
                return Err(RuntimeError::type_error("unsupported operand type"));
            };
            let Some(backing) = instance_data.attrs.get(INT_BACKING_STORAGE_ATTR) else {
                return Err(RuntimeError::type_error("unsupported operand type"));
            };
            value_to_int(backing.clone())
        }
        other => {
            if env_var_present_cached("PYRS_TRACE_VALUE_TO_INT") {
                eprintln!("[value_to_int] unsupported value={}", format_repr(&other));
                eprintln!(
                    "[value_to_int] backtrace:\n{:?}",
                    std::backtrace::Backtrace::force_capture()
                );
            }
            Err(RuntimeError::type_error("unsupported operand type"))
        }
    }
}

fn value_to_bigint(value: Value) -> Result<BigInt, RuntimeError> {
    match value {
        Value::Int(value) => Ok(BigInt::from_i64(value)),
        Value::Bool(value) => Ok(BigInt::from_i64(if value { 1 } else { 0 })),
        Value::BigInt(value) => Ok(*value),
        _ => Err(RuntimeError::type_error("range() expects integers")),
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
        let shift = -ndigits;
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
        return Err(RuntimeError::value_error(
            "invalid literal for int() with base 10",
        ));
    }
    let (negative, digits) = if let Some(rest) = cleaned.strip_prefix('+') {
        (false, rest)
    } else if let Some(rest) = cleaned.strip_prefix('-') {
        (true, rest)
    } else {
        (false, cleaned)
    };
    let normalized = normalize_decimal_int_digits(digits)
        .ok_or_else(|| RuntimeError::value_error("invalid literal for int() with base 10"))?;
    let mut value = BigInt::from_str_radix(&normalized, 10)
        .ok_or_else(|| RuntimeError::value_error("invalid literal for int() with base 10"))?;
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
        return Err(RuntimeError::value_error("empty range for randrange()"));
    }
    if step > 0 {
        if start >= stop {
            return Err(RuntimeError::value_error("empty range for randrange()"));
        }
        let count = ((stop as i128 - start as i128 - 1) / step as i128) + 1;
        return i64::try_from(count).map_err(|_| RuntimeError::overflow_error("integer overflow"));
    }
    if start <= stop {
        return Err(RuntimeError::value_error("empty range for randrange()"));
    }
    let step_mag = -(step as i128);
    let count = ((start as i128 - stop as i128 - 1) / step_mag) + 1;
    i64::try_from(count).map_err(|_| RuntimeError::overflow_error("integer overflow"))
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
        other => match value_to_int(other) {
            Ok(index) => Ok(Some(index)),
            Err(err)
                if err.message.contains("unsupported operand type")
                    || err.message.contains("cannot be interpreted as an integer") =>
            {
                Err(RuntimeError::new(
                    "TypeError: slice indices must be integers or None or have an __index__ method",
                ))
            }
            Err(err) => Err(err),
        },
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
        return Err(RuntimeError::overflow_error("integer overflow"));
    }
    Ok(div as i64)
}

fn python_mod(left: i64, right: i64) -> Result<i64, RuntimeError> {
    if right == 0 {
        return Err(RuntimeError::zero_division_error("modulo by zero"));
    }
    let a = left as i128;
    let b = right as i128;
    let div = python_floor_div(left, right)? as i128;
    let value = a - b * div;
    if value < i64::MIN as i128 || value > i64::MAX as i128 {
        return Err(RuntimeError::overflow_error("integer overflow"));
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
        Value::Complex { real, imag: 0.0 } => Ok(real),
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
            _ => Err(RuntimeError::type_error("path must be string or bytes")),
        },
        _ => Err(RuntimeError::type_error("path must be string or bytes")),
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

#[cfg(unix)]
fn collect_process_argv(value: &Value) -> Result<Vec<String>, RuntimeError> {
    let items = value_to_sequence_items(value)?;
    let mut argv = Vec::with_capacity(items.len());
    for item in &items {
        argv.push(value_to_process_text(item)?);
    }
    Ok(argv)
}

#[cfg(unix)]
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
        && let Some(parent) = cache_path.parent().and_then(|parent| parent.parent())
    {
        return parent
            .join(format!("{module_stem}.py"))
            .to_string_lossy()
            .to_string();
    }
    path.trim_end_matches('c').to_string()
}

fn cache_path_from_source_path(path: &str) -> String {
    cache_path_from_source_path_with_optimization(path, "")
}

fn cache_path_from_source_path_with_optimization(path: &str, optimization: &str) -> String {
    let source_path = Path::new(path);
    let stem = source_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("module");
    let pycache = source_path
        .parent()
        .map(|parent| parent.join("__pycache__"))
        .unwrap_or_else(|| PathBuf::from("__pycache__"));
    let filename = if optimization.is_empty() {
        format!("{stem}.cpython-314.pyc")
    } else {
        format!("{stem}.cpython-314.opt-{optimization}.pyc")
    };
    pycache.join(filename).to_string_lossy().to_string()
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
    if !text.is_empty()
        && text.chars().all(|ch| ch.is_ascii_digit())
        && let Ok(value) = text.parse::<i64>()
    {
        return FormatterFieldKey::Int(value);
    }
    FormatterFieldKey::Str(text.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemoryViewCastFormat {
    UnsignedByte,
    SignedByte,
    Char,
    UnsignedShort,
    SignedShort,
    UnsignedInt,
    SignedInt,
    UnsignedLong,
    SignedLong,
    UnsignedLongLong,
    SignedLongLong,
    Float,
    Double,
}

impl MemoryViewCastFormat {
    fn code(self) -> &'static str {
        match self {
            Self::UnsignedByte => "B",
            Self::SignedByte => "b",
            Self::Char => "c",
            Self::UnsignedShort => "H",
            Self::SignedShort => "h",
            Self::UnsignedInt => "I",
            Self::SignedInt => "i",
            Self::UnsignedLong => "L",
            Self::SignedLong => "l",
            Self::UnsignedLongLong => "Q",
            Self::SignedLongLong => "q",
            Self::Float => "f",
            Self::Double => "d",
        }
    }

    fn itemsize(self) -> usize {
        match self {
            Self::UnsignedByte | Self::SignedByte | Self::Char => 1,
            Self::UnsignedShort | Self::SignedShort => 2,
            Self::UnsignedInt | Self::SignedInt | Self::Float => 4,
            Self::UnsignedLong | Self::SignedLong => std::mem::size_of::<std::os::raw::c_long>(),
            Self::UnsignedLongLong | Self::SignedLongLong | Self::Double => 8,
        }
    }

    fn integer_signedness(self) -> Option<bool> {
        match self {
            Self::UnsignedByte
            | Self::UnsignedShort
            | Self::UnsignedInt
            | Self::UnsignedLong
            | Self::UnsignedLongLong => Some(false),
            Self::SignedByte
            | Self::SignedShort
            | Self::SignedInt
            | Self::SignedLong
            | Self::SignedLongLong => Some(true),
            Self::Char | Self::Float | Self::Double => None,
        }
    }
}

fn parse_memoryview_cast_format(format: &str) -> Option<MemoryViewCastFormat> {
    match format {
        "B" => Some(MemoryViewCastFormat::UnsignedByte),
        "b" => Some(MemoryViewCastFormat::SignedByte),
        "c" => Some(MemoryViewCastFormat::Char),
        "H" => Some(MemoryViewCastFormat::UnsignedShort),
        "h" => Some(MemoryViewCastFormat::SignedShort),
        "I" => Some(MemoryViewCastFormat::UnsignedInt),
        "i" => Some(MemoryViewCastFormat::SignedInt),
        "L" => Some(MemoryViewCastFormat::UnsignedLong),
        "l" => Some(MemoryViewCastFormat::SignedLong),
        "Q" => Some(MemoryViewCastFormat::UnsignedLongLong),
        "q" => Some(MemoryViewCastFormat::SignedLongLong),
        "f" => Some(MemoryViewCastFormat::Float),
        "d" => Some(MemoryViewCastFormat::Double),
        _ => None,
    }
}

fn memoryview_format_for_view(
    itemsize: usize,
    format: Option<&str>,
) -> Result<MemoryViewCastFormat, RuntimeError> {
    let format_spec = format.unwrap_or("B");
    let cast_format = parse_memoryview_cast_format(format_spec).ok_or_else(|| {
        RuntimeError::not_implemented_error(format!(
            "memoryview: format {format_spec} not supported"
        ))
    })?;
    if cast_format.itemsize() != itemsize.max(1) {
        return Err(RuntimeError::not_implemented_error(
            "memoryview: unsupported format",
        ));
    }
    Ok(cast_format)
}

fn memoryview_invalid_type_error(format: MemoryViewCastFormat) -> RuntimeError {
    RuntimeError::new(format!(
        "TypeError: memoryview: invalid type for format '{}'",
        format.code()
    ))
}

fn memoryview_invalid_value_error(format: MemoryViewCastFormat) -> RuntimeError {
    RuntimeError::new(format!(
        "ValueError: memoryview: invalid value for format '{}'",
        format.code()
    ))
}

fn memoryview_integer_bounds(bits: usize, signed: bool) -> (BigInt, BigInt) {
    if signed {
        let limit = BigInt::one().shl_bits(bits.saturating_sub(1));
        let min = limit.negated();
        let max = limit.sub(&BigInt::one());
        (min, max)
    } else {
        let max = BigInt::one().shl_bits(bits).sub(&BigInt::one());
        (BigInt::zero(), max)
    }
}

fn memoryview_decode_element(
    chunk: &[u8],
    format: MemoryViewCastFormat,
    itemsize: usize,
    heap: &Heap,
) -> Result<Value, RuntimeError> {
    let itemsize = itemsize.max(1);
    if chunk.len() != itemsize || format.itemsize() != itemsize {
        return Err(RuntimeError::not_implemented_error(
            "memoryview: unsupported format",
        ));
    }
    match format {
        MemoryViewCastFormat::UnsignedByte
        | MemoryViewCastFormat::SignedByte
        | MemoryViewCastFormat::UnsignedShort
        | MemoryViewCastFormat::SignedShort
        | MemoryViewCastFormat::UnsignedInt
        | MemoryViewCastFormat::SignedInt
        | MemoryViewCastFormat::UnsignedLong
        | MemoryViewCastFormat::SignedLong
        | MemoryViewCastFormat::UnsignedLongLong
        | MemoryViewCastFormat::SignedLongLong => {
            let signed = format.integer_signedness().unwrap_or(false);
            Ok(value_from_bigint(bigint_from_bytes(
                chunk,
                cfg!(target_endian = "little"),
                signed,
            )))
        }
        MemoryViewCastFormat::Char => Ok(heap.alloc_bytes(vec![chunk[0]])),
        MemoryViewCastFormat::Float => {
            let raw = [chunk[0], chunk[1], chunk[2], chunk[3]];
            Ok(Value::Float(f32::from_ne_bytes(raw) as f64))
        }
        MemoryViewCastFormat::Double => {
            let raw = [
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ];
            Ok(Value::Float(f64::from_ne_bytes(raw)))
        }
    }
}

fn memoryview_encode_element(
    value: Value,
    format: MemoryViewCastFormat,
    itemsize: usize,
) -> Result<Vec<u8>, RuntimeError> {
    let itemsize = itemsize.max(1);
    if format.itemsize() != itemsize {
        return Err(RuntimeError::not_implemented_error(
            "memoryview: unsupported format",
        ));
    }
    match format {
        MemoryViewCastFormat::UnsignedByte
        | MemoryViewCastFormat::SignedByte
        | MemoryViewCastFormat::UnsignedShort
        | MemoryViewCastFormat::SignedShort
        | MemoryViewCastFormat::UnsignedInt
        | MemoryViewCastFormat::SignedInt
        | MemoryViewCastFormat::UnsignedLong
        | MemoryViewCastFormat::SignedLong
        | MemoryViewCastFormat::UnsignedLongLong
        | MemoryViewCastFormat::SignedLongLong => {
            let numeric = match value {
                Value::Int(value) => BigInt::from_i64(value),
                Value::Bool(flag) => BigInt::from_i64(if flag { 1 } else { 0 }),
                Value::BigInt(value) => *value,
                _ => return Err(memoryview_invalid_type_error(format)),
            };
            let signed = format.integer_signedness().unwrap_or(false);
            let (min, max) = memoryview_integer_bounds(itemsize.saturating_mul(8), signed);
            if numeric.cmp_total(&min) == Ordering::Less
                || numeric.cmp_total(&max) == Ordering::Greater
            {
                return Err(memoryview_invalid_value_error(format));
            }
            bigint_to_fixed_bytes(&numeric, itemsize, cfg!(target_endian = "little"), signed)
                .map_err(|_| memoryview_invalid_value_error(format))
        }
        MemoryViewCastFormat::Char => match value {
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(values) if values.len() == 1 => Ok(vec![values[0]]),
                Object::Bytes(_) => Err(memoryview_invalid_value_error(format)),
                _ => Err(memoryview_invalid_type_error(format)),
            },
            _ => Err(memoryview_invalid_type_error(format)),
        },
        MemoryViewCastFormat::Float => {
            let numeric = match value {
                Value::Float(value) => value,
                Value::Int(value) => value as f64,
                Value::Bool(flag) => {
                    if flag {
                        1.0
                    } else {
                        0.0
                    }
                }
                Value::BigInt(value) => value.to_f64(),
                _ => return Err(memoryview_invalid_type_error(format)),
            };
            Ok((numeric as f32).to_ne_bytes().to_vec())
        }
        MemoryViewCastFormat::Double => {
            let numeric = match value {
                Value::Float(value) => value,
                Value::Int(value) => value as f64,
                Value::Bool(flag) => {
                    if flag {
                        1.0
                    } else {
                        0.0
                    }
                }
                Value::BigInt(value) => value.to_f64(),
                _ => return Err(memoryview_invalid_type_error(format)),
            };
            Ok(numeric.to_ne_bytes().to_vec())
        }
    }
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

fn memoryview_shape_and_strides_from_parts(
    start: usize,
    length: Option<usize>,
    shape: Option<&Vec<isize>>,
    strides: Option<&Vec<isize>>,
    itemsize: usize,
    source_len: usize,
) -> Option<(Vec<isize>, Vec<isize>)> {
    if let (Some(shape), Some(strides)) = (shape, strides)
        && !shape.is_empty()
        && shape.len() == strides.len()
    {
        return Some((shape.clone(), strides.clone()));
    }
    let itemsize = itemsize.max(1);
    let (start, end) = memoryview_bounds(start, length, source_len);
    let span = end.saturating_sub(start);
    let logical_len = if span % itemsize == 0 {
        span / itemsize
    } else {
        span
    };
    Some((vec![logical_len as isize], vec![itemsize as isize]))
}

fn memoryview_layout_1d(
    view: &MemoryViewObject,
    source_len: usize,
) -> Option<(isize, usize, isize, usize)> {
    memoryview_layout_1d_from_parts(
        view.start,
        view.length,
        view.itemsize,
        view.shape.as_ref(),
        view.strides.as_ref(),
        source_len,
    )
}

fn memoryview_layout_1d_from_parts(
    start: usize,
    length: Option<usize>,
    itemsize: usize,
    shape: Option<&Vec<isize>>,
    strides: Option<&Vec<isize>>,
    source_len: usize,
) -> Option<(isize, usize, isize, usize)> {
    let itemsize = itemsize.max(1);
    let (shape, strides) = memoryview_shape_and_strides_from_parts(
        start, length, shape, strides, itemsize, source_len,
    )?;
    if shape.len() != 1 || strides.len() != 1 {
        return None;
    }
    let logical_len = if shape[0] < 0 {
        return None;
    } else {
        usize::try_from(shape[0]).ok()?
    };
    let stride = strides[0];
    let origin = isize::try_from(start).ok()?;
    if logical_len == 0 {
        return Some((origin, 0, stride, itemsize));
    }
    let tail_delta = stride.checked_mul((logical_len.saturating_sub(1)) as isize)?;
    let first = origin.min(origin.checked_add(tail_delta)?);
    let last = origin.max(origin.checked_add(tail_delta)?);
    let itemsize_isize = isize::try_from(itemsize).ok()?;
    let highest = last.checked_add(itemsize_isize.checked_sub(1)?)?;
    if first < 0 || highest < 0 {
        return None;
    }
    let source_len_isize = isize::try_from(source_len).ok()?;
    if highest >= source_len_isize {
        return None;
    }
    Some((origin, logical_len, stride, itemsize))
}

fn memoryview_normalize_index(logical_len: usize, index: isize) -> Option<usize> {
    let mut normalized = index;
    if normalized < 0 {
        normalized = normalized.checked_add(logical_len as isize)?;
    }
    if normalized < 0 {
        return None;
    }
    let normalized = usize::try_from(normalized).ok()?;
    if normalized >= logical_len {
        return None;
    }
    Some(normalized)
}

fn memoryview_element_offset(
    origin: isize,
    logical_len: usize,
    stride: isize,
    index: isize,
) -> Option<usize> {
    let normalized = memoryview_normalize_index(logical_len, index)?;
    let delta = stride.checked_mul(normalized as isize)?;
    let offset = origin.checked_add(delta)?;
    if offset < 0 {
        return None;
    }
    usize::try_from(offset).ok()
}

fn memoryview_collect_bytes(
    source: &[u8],
    origin: isize,
    logical_len: usize,
    stride: isize,
    itemsize: usize,
) -> Option<Vec<u8>> {
    if logical_len == 0 {
        return Some(Vec::new());
    }
    let mut out = Vec::with_capacity(logical_len.checked_mul(itemsize)?);
    let itemsize_isize = isize::try_from(itemsize).ok()?;
    for index in 0..logical_len {
        let base = origin.checked_add(stride.checked_mul(index as isize)?)?;
        if base < 0 {
            return None;
        }
        let end = base.checked_add(itemsize_isize)?;
        let source_len = isize::try_from(source.len()).ok()?;
        if end > source_len {
            return None;
        }
        let base_usize = usize::try_from(base).ok()?;
        let end_usize = base_usize.checked_add(itemsize)?;
        out.extend_from_slice(source.get(base_usize..end_usize)?);
    }
    Some(out)
}

fn memoryview_logical_nbytes(shape: &[isize], itemsize: usize) -> Option<usize> {
    let mut elements = 1usize;
    for dim in shape {
        let dim_usize = usize::try_from(*dim).ok()?;
        elements = elements.checked_mul(dim_usize)?;
    }
    elements.checked_mul(itemsize.max(1))
}

fn memoryview_decode_tolist_recursive(
    source: &[u8],
    base: isize,
    itemsize: usize,
    format: MemoryViewCastFormat,
    shape: &[isize],
    strides: &[isize],
    heap: &Heap,
) -> Result<Value, RuntimeError> {
    if shape.is_empty() || shape.len() != strides.len() {
        return Err(RuntimeError::not_implemented_error(
            "memoryview: unsupported format",
        ));
    }
    let dim = usize::try_from(shape[0])
        .map_err(|_| RuntimeError::not_implemented_error("memoryview: unsupported format"))?;
    let stride = strides[0];
    if shape.len() == 1 {
        let mut values = Vec::with_capacity(dim);
        let itemsize_isize = isize::try_from(itemsize)
            .map_err(|_| RuntimeError::not_implemented_error("memoryview: unsupported format"))?;
        for index in 0..dim {
            let delta = stride.checked_mul(index as isize).ok_or_else(|| {
                RuntimeError::not_implemented_error("memoryview: unsupported format")
            })?;
            let offset = base.checked_add(delta).ok_or_else(|| {
                RuntimeError::not_implemented_error("memoryview: unsupported format")
            })?;
            if offset < 0 {
                return Err(RuntimeError::not_implemented_error(
                    "memoryview: unsupported format",
                ));
            }
            let end = offset.checked_add(itemsize_isize).ok_or_else(|| {
                RuntimeError::not_implemented_error("memoryview: unsupported format")
            })?;
            let source_len = isize::try_from(source.len()).map_err(|_| {
                RuntimeError::not_implemented_error("memoryview: unsupported format")
            })?;
            if end > source_len {
                return Err(RuntimeError::not_implemented_error(
                    "memoryview: unsupported format",
                ));
            }
            let offset_usize = usize::try_from(offset).map_err(|_| {
                RuntimeError::not_implemented_error("memoryview: unsupported format")
            })?;
            let end_usize = offset_usize.checked_add(itemsize).ok_or_else(|| {
                RuntimeError::not_implemented_error("memoryview: unsupported format")
            })?;
            let chunk = source.get(offset_usize..end_usize).ok_or_else(|| {
                RuntimeError::not_implemented_error("memoryview: unsupported format")
            })?;
            let value = memoryview_decode_element(chunk, format, itemsize, heap).map_err(|_| {
                RuntimeError::not_implemented_error("memoryview: unsupported format")
            })?;
            values.push(value);
        }
        Ok(heap.alloc_list(values))
    } else {
        let mut rows = Vec::with_capacity(dim);
        for index in 0..dim {
            let delta = stride.checked_mul(index as isize).ok_or_else(|| {
                RuntimeError::not_implemented_error("memoryview: unsupported format")
            })?;
            let row_base = base.checked_add(delta).ok_or_else(|| {
                RuntimeError::not_implemented_error("memoryview: unsupported format")
            })?;
            let row = memoryview_decode_tolist_recursive(
                source,
                row_base,
                itemsize,
                format,
                &shape[1..],
                &strides[1..],
                heap,
            )?;
            rows.push(row);
        }
        Ok(heap.alloc_list(rows))
    }
}

fn memoryview_decode_tolist(
    source: &[u8],
    start: usize,
    itemsize: usize,
    format: MemoryViewCastFormat,
    shape: &[isize],
    strides: &[isize],
    heap: &Heap,
) -> Result<Value, RuntimeError> {
    let start = isize::try_from(start)
        .map_err(|_| RuntimeError::not_implemented_error("memoryview: unsupported format"))?;
    memoryview_decode_tolist_recursive(source, start, itemsize.max(1), format, shape, strides, heap)
}

fn memoryview_collect_bytes_recursive(
    source: &[u8],
    base: isize,
    itemsize: usize,
    shape: &[isize],
    strides: &[isize],
    out: &mut Vec<u8>,
) -> Option<()> {
    if shape.is_empty() || shape.len() != strides.len() {
        return None;
    }
    let dim = usize::try_from(shape[0]).ok()?;
    let stride = strides[0];
    if shape.len() == 1 {
        let itemsize_isize = isize::try_from(itemsize).ok()?;
        for index in 0..dim {
            let delta = stride.checked_mul(index as isize)?;
            let offset = base.checked_add(delta)?;
            if offset < 0 {
                return None;
            }
            let end = offset.checked_add(itemsize_isize)?;
            let source_len = isize::try_from(source.len()).ok()?;
            if end > source_len {
                return None;
            }
            let offset_usize = usize::try_from(offset).ok()?;
            let end_usize = offset_usize.checked_add(itemsize)?;
            out.extend_from_slice(source.get(offset_usize..end_usize)?);
        }
    } else {
        for index in 0..dim {
            let delta = stride.checked_mul(index as isize)?;
            let row_base = base.checked_add(delta)?;
            memoryview_collect_bytes_recursive(
                source,
                row_base,
                itemsize,
                &shape[1..],
                &strides[1..],
                out,
            )?;
        }
    }
    Some(())
}

fn memoryview_collect_bytes_for_view(view: &MemoryViewObject, source: &[u8]) -> Option<Vec<u8>> {
    let itemsize = view.itemsize.max(1);
    let (shape, strides) = memoryview_shape_and_strides_from_parts(
        view.start,
        view.length,
        view.shape.as_ref(),
        view.strides.as_ref(),
        itemsize,
        source.len(),
    )?;
    let total = memoryview_logical_nbytes(&shape, itemsize)?;
    let mut out = Vec::with_capacity(total);
    let start = isize::try_from(view.start).ok()?;
    memoryview_collect_bytes_recursive(source, start, itemsize, &shape, &strides, &mut out)?;
    Some(out)
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
            _ => Err(RuntimeError::type_error("expected bytes-like payload")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => Ok(values.clone()),
            _ => Err(RuntimeError::type_error("expected bytes-like payload")),
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => with_bytes_like_source(&view.source, |values| {
                if let Some((origin, logical_len, stride, itemsize)) =
                    memoryview_layout_1d(view, values.len())
                {
                    memoryview_collect_bytes(values, origin, logical_len, stride, itemsize)
                        .ok_or_else(|| RuntimeError::type_error("expected bytes-like payload"))
                } else {
                    memoryview_collect_bytes_for_view(view, values)
                        .ok_or_else(|| RuntimeError::type_error("expected bytes-like payload"))
                }
            })
            .unwrap_or_else(|| Err(RuntimeError::type_error("expected bytes-like payload"))),
            _ => Err(RuntimeError::type_error("expected bytes-like payload")),
        },
        Value::Module(obj) => match &*obj.kind() {
            Object::Module(module_data) if module_data.name == "__array__" => {
                match module_data.globals.get("values") {
                    Some(values) => value_to_bytes_payload(values.clone()),
                    None => Err(RuntimeError::type_error("expected bytes-like payload")),
                }
            }
            _ => Err(RuntimeError::type_error("expected bytes-like payload")),
        },
        Value::Instance(obj) => match &*obj.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR) {
                    Some(Value::Bytes(storage)) => match &*storage.kind() {
                        Object::Bytes(values) => Ok(values.clone()),
                        _ => Err(RuntimeError::type_error("expected bytes-like payload")),
                    },
                    Some(Value::ByteArray(storage)) => match &*storage.kind() {
                        Object::ByteArray(values) => Ok(values.clone()),
                        _ => Err(RuntimeError::type_error("expected bytes-like payload")),
                    },
                    _ => Err(RuntimeError::type_error("expected bytes-like payload")),
                }
            }
            _ => Err(RuntimeError::type_error("expected bytes-like payload")),
        },
        Value::Iterator(obj) => {
            let mut obj_kind = obj.kind_mut();
            let Object::Iterator(iterator) = &mut *obj_kind else {
                return Err(RuntimeError::type_error("expected bytes-like payload"));
            };
            let values = match &mut iterator.kind {
                IteratorKind::List(list_obj) => match &*list_obj.kind() {
                    Object::List(items) => {
                        let start = iterator.index.min(items.len());
                        let out = items[start..].to_vec();
                        iterator.index = items.len();
                        out
                    }
                    _ => return Err(RuntimeError::type_error("expected bytes-like payload")),
                },
                IteratorKind::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                    Object::Tuple(items) => {
                        let start = iterator.index.min(items.len());
                        let out = items[start..].to_vec();
                        iterator.index = items.len();
                        out
                    }
                    _ => return Err(RuntimeError::type_error("expected bytes-like payload")),
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
                    _ => return Err(RuntimeError::type_error("expected bytes-like payload")),
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
                    _ => return Err(RuntimeError::type_error("expected bytes-like payload")),
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
                    .ok_or_else(|| RuntimeError::type_error("expected bytes-like payload"))?,
                    _ => return Err(RuntimeError::type_error("expected bytes-like payload")),
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
                        return Err(RuntimeError::value_error("range() arg 3 must not be zero"));
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
                        return Err(RuntimeError::value_error("range() arg 3 must not be zero"));
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
                    _ => return Err(RuntimeError::type_error("expected bytes-like payload")),
                },
                IteratorKind::Set(set_obj) => match &*set_obj.kind() {
                    Object::Set(items) | Object::FrozenSet(items) => {
                        let all = items.to_vec();
                        let start = iterator.index.min(all.len());
                        let out = all.into_iter().skip(start).collect::<Vec<_>>();
                        iterator.index = start.saturating_add(out.len());
                        out
                    }
                    _ => return Err(RuntimeError::type_error("expected bytes-like payload")),
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
                    return Err(RuntimeError::type_error("expected bytes-like payload"));
                }
                IteratorKind::CpythonSequence { .. } => {
                    return Err(RuntimeError::type_error("expected bytes-like payload"));
                }
                IteratorKind::CallIter { .. }
                | IteratorKind::Count { .. }
                | IteratorKind::Cycle { .. }
                | IteratorKind::Zip { .. }
                | IteratorKind::Chain { .. }
                | IteratorKind::ChainFromIterable { .. }
                | IteratorKind::Accumulate { .. }
                | IteratorKind::Combinations { .. }
                | IteratorKind::CombinationsWithReplacement { .. }
                | IteratorKind::Permutations { .. }
                | IteratorKind::Product { .. }
                | IteratorKind::Compress { .. }
                | IteratorKind::DropWhile { .. }
                | IteratorKind::FilterFalse { .. }
                | IteratorKind::Islice { .. }
                | IteratorKind::Pairwise { .. }
                | IteratorKind::StarMap { .. }
                | IteratorKind::TakeWhile { .. }
                | IteratorKind::ZipLongest { .. }
                | IteratorKind::Tee { .. }
                | IteratorKind::Repeat { .. }
                | IteratorKind::Batched { .. }
                | IteratorKind::GroupBy { .. }
                | IteratorKind::GroupByGrouper { .. } => {
                    return Err(RuntimeError::type_error("expected bytes-like payload"));
                }
            };
            let mut out = Vec::with_capacity(values.len());
            for value in values {
                let byte = value_to_int(value)?;
                if !(0..=255).contains(&byte) {
                    return Err(RuntimeError::value_error("byte must be in range(0, 256)"));
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
                        return Err(RuntimeError::value_error("byte must be in range(0, 256)"));
                    }
                    out.push(byte as u8);
                }
                Ok(out)
            }
            _ => Err(RuntimeError::type_error("expected bytes-like payload")),
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
            _ => Err(RuntimeError::type_error("pattern must be string or bytes")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => Ok(RePatternValue::Bytes(values.clone())),
            _ => Err(RuntimeError::type_error("pattern must be string or bytes")),
        },
        _ => Err(RuntimeError::type_error("pattern must be string or bytes")),
    }
}

fn re_pattern_from_compiled_module(module: &ObjRef) -> Result<RePatternValue, RuntimeError> {
    match &*module.kind() {
        Object::Module(module_data) if module_data.name == "__re_pattern__" => {
            let pattern = module_data
                .globals
                .get("__pyrs_compiled_pattern__")
                .or_else(|| module_data.globals.get("pattern"));
            let Some(pattern) = pattern else {
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
                Err(RuntimeError::type_error("pattern must be string or bytes"))
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
        branches: Vec<Vec<ReToken>>,
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
    branches: Vec<Vec<ReToken>>,
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
    if !matches!(quantifier, ReQuantifier::One) {
        while *idx < chars.len() && (chars[*idx] == '?' || chars[*idx] == '+') {
            // Non-greedy (`?`) / possessive (`+`) quantifier modifiers are
            // accepted for compatibility, but ignored by this fallback engine.
            *idx += 1;
        }
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
            break;
        }
        if !stop_on_group_end && chars[*idx] == '$' && *idx + 1 == chars.len() {
            break;
        }
        if !stop_on_group_end
            && chars[*idx] == '\\'
            && *idx + 2 == chars.len()
            && matches!(chars[*idx + 1], 'Z' | 'z')
        {
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
                } else if *idx + 2 < chars.len() && chars[*idx] == 'P' && chars[*idx + 1] == '<' {
                    *idx += 2;
                    while *idx < chars.len() && chars[*idx] != '>' {
                        *idx += 1;
                    }
                    if *idx >= chars.len() || chars[*idx] != '>' {
                        return None;
                    }
                    *idx += 1;
                    *capture_count += 1;
                    capture = Some(*capture_count);
                } else {
                    return None;
                }
            } else {
                *capture_count += 1;
                capture = Some(*capture_count);
            }
            let inner = parse_simple_regex_branches(chars, idx, capture_count, true)?;
            if *idx >= chars.len() || chars[*idx] != ')' {
                return None;
            }
            *idx += 1;
            ReAtom::Group {
                branches: inner,
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

fn parse_simple_regex_branches(
    chars: &[char],
    idx: &mut usize,
    capture_count: &mut usize,
    stop_on_group_end: bool,
) -> Option<Vec<Vec<ReToken>>> {
    let mut branches = Vec::new();
    loop {
        let sequence = parse_simple_regex_sequence(chars, idx, capture_count, stop_on_group_end)?;
        branches.push(sequence);
        if *idx < chars.len() && chars[*idx] == '|' {
            *idx += 1;
            continue;
        }
        break;
    }
    Some(branches)
}

fn parse_simple_regex(pattern: &str) -> Option<ParsedSimpleRegex> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut idx = 0usize;
    let mut start_anchor = false;
    let mut end_anchor = false;
    if idx + 1 < chars.len() && chars[idx] == '\\' && chars[idx + 1] == 'A' {
        start_anchor = true;
        idx += 2;
    } else if idx < chars.len() && chars[idx] == '^' {
        start_anchor = true;
        idx += 1;
    }

    let mut capture_count = 0usize;
    let branches = parse_simple_regex_branches(&chars, &mut idx, &mut capture_count, false)?;
    if idx + 1 < chars.len()
        && chars[idx] == '\\'
        && matches!(chars[idx + 1], 'Z' | 'z')
        && idx + 2 == chars.len()
    {
        end_anchor = true;
        idx += 2;
    } else if idx < chars.len() && chars[idx] == '$' && idx + 1 == chars.len() {
        end_anchor = true;
        idx += 1;
    }
    if idx != chars.len() {
        return None;
    }
    Some(ParsedSimpleRegex {
        start_anchor,
        end_anchor,
        branches,
        capture_count,
    })
}

fn split_top_level_regex_alternation(pattern: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut bars = Vec::new();
    let mut depth = 0usize;
    let mut in_class = false;
    let mut escape = false;
    for (idx, ch) in chars.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if *ch == '\\' {
            escape = true;
            continue;
        }
        if in_class {
            if *ch == ']' {
                in_class = false;
            }
            continue;
        }
        if *ch == '[' {
            in_class = true;
            continue;
        }
        if *ch == '(' {
            depth += 1;
            continue;
        }
        if *ch == ')' {
            if depth == 0 {
                return None;
            }
            depth -= 1;
            continue;
        }
        if *ch == '|' && depth == 0 {
            bars.push(idx);
        }
    }
    if depth != 0 || bars.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(bars.len() + 1);
    let mut start = 0usize;
    for bar in bars {
        out.push(chars[start..bar].iter().collect::<String>());
        start = bar + 1;
    }
    out.push(chars[start..].iter().collect::<String>());
    Some(out)
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
        let mut group_prefix = "(".to_string();
        if content_start + 1 < chars.len()
            && chars[content_start] == '?'
            && chars[content_start + 1] == ':'
        {
            content_start += 2;
            group_prefix = "(?:".to_string();
        } else if content_start + 2 < chars.len()
            && chars[content_start] == '?'
            && chars[content_start + 1] == 'P'
            && chars[content_start + 2] == '<'
        {
            let name_start = content_start + 3;
            let mut name_end = name_start;
            while name_end < chars.len() && chars[name_end] != '>' {
                name_end += 1;
            }
            if name_end >= chars.len() || chars[name_end] != '>' {
                idx += 1;
                continue;
            }
            let name: String = chars[name_start..name_end].iter().collect();
            group_prefix = format!("(?P<{name}>");
            content_start = name_end + 1;
        } else if content_start < chars.len() && chars[content_start] == '?' {
            // Unsupported group forms (lookarounds/backrefs/conditionals).
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
                .map(|alt| format!("{prefix}{group_prefix}{alt}){suffix}"))
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
            ReAtom::Group { branches, capture } => {
                for branch in branches {
                    let Some((end, mut next_state)) =
                        match_simple_regex_tokens(branch, chars, 0, char_idx, false, state.clone())
                    else {
                        continue;
                    };
                    if let Some(index) = capture
                        && let Some(slot) = next_state.captures.get_mut(index - 1)
                    {
                        *slot = Some((char_idx, end));
                    }
                    return Some((end, next_state));
                }
                None
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
                && let Some(result) = match_simple_regex_tokens(
                    tokens,
                    chars,
                    token_idx + 1,
                    next_idx,
                    require_end,
                    next_state,
                )
            {
                return Some(result);
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
    } else if let Some(alternatives) = split_top_level_regex_alternation(pattern) {
        let mut best: Option<ReMatchDetail> = None;
        for alternative in alternatives {
            let Some(detail) = simple_regex_match_details(&alternative, text, mode) else {
                continue;
            };
            let replace = best
                .as_ref()
                .map(|current| detail.start < current.start)
                .unwrap_or(true);
            if replace {
                best = Some(detail);
            }
        }
        return best;
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
        for branch in &parsed.branches {
            let state = ReMatchState {
                captures: vec![None; parsed.capture_count],
            };
            if let Some((end, state)) =
                match_simple_regex_tokens(branch, &chars, 0, start, require_end, state)
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

fn decimal_parser_pattern_matches(pattern_text: &str) -> bool {
    pattern_text.contains("(?P<sign>[-+])?")
        && pattern_text.contains("(?P<int>\\d*)")
        && pattern_text.contains("Inf(inity)?")
        && pattern_text.contains("(?P<signal>s)?")
        && pattern_text.contains("NaN")
        && pattern_text.contains("(?P<diag>\\d*)")
}

fn ascii_case_starts_with(haystack: &[u8], start: usize, needle: &[u8]) -> bool {
    if start + needle.len() > haystack.len() {
        return false;
    }
    haystack[start..start + needle.len()]
        .iter()
        .zip(needle.iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn pydecimal_parser_match_detail(text: &str) -> Option<ReMatchDetail> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut base_captures = vec![None; 10];
    let mut cursor = 0usize;
    if cursor < len && (bytes[cursor] == b'+' || bytes[cursor] == b'-') {
        base_captures[0] = Some((cursor, cursor + 1));
        cursor += 1;
    }
    let body_start = cursor;

    // Numeric branch.
    let mut captures = base_captures.clone();
    let mut index = body_start;
    let has_digit_start = index < len && bytes[index].is_ascii_digit();
    let has_frac_start =
        index + 1 < len && bytes[index] == b'.' && bytes[index + 1].is_ascii_digit();
    if has_digit_start || has_frac_start {
        let int_start = index;
        while index < len && bytes[index].is_ascii_digit() {
            index += 1;
        }
        captures[2] = Some((int_start, index));
        if index < len && bytes[index] == b'.' {
            let dot_start = index;
            index += 1;
            let frac_start = index;
            while index < len && bytes[index].is_ascii_digit() {
                index += 1;
            }
            captures[3] = Some((dot_start, index));
            captures[4] = Some((frac_start, index));
        }
        if index < len && (bytes[index] == b'e' || bytes[index] == b'E') {
            let exp_group_start = index;
            index += 1;
            let exp_start = index;
            if index < len && (bytes[index] == b'+' || bytes[index] == b'-') {
                index += 1;
            }
            let digits_start = index;
            while index < len && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if digits_start == index {
                return None;
            }
            captures[5] = Some((exp_group_start, index));
            captures[6] = Some((exp_start, index));
        }
        if index == len {
            captures[1] = Some((body_start, len));
            return Some(ReMatchDetail {
                start: 0,
                end: len,
                captures,
            });
        }
    }

    // Infinity branch.
    captures = base_captures.clone();
    index = body_start;
    if ascii_case_starts_with(bytes, index, b"inf") {
        index += 3;
        if ascii_case_starts_with(bytes, index, b"inity") {
            captures[7] = Some((index, index + 5));
            index += 5;
        }
        if index == len {
            captures[1] = Some((body_start, len));
            return Some(ReMatchDetail {
                start: 0,
                end: len,
                captures,
            });
        }
    }

    // sNaN/NaN branch.
    captures = base_captures;
    index = body_start;
    if index < len && (bytes[index] == b's' || bytes[index] == b'S') {
        captures[8] = Some((index, index + 1));
        index += 1;
    }
    if ascii_case_starts_with(bytes, index, b"nan") {
        index += 3;
        let diag_start = index;
        while index < len && bytes[index].is_ascii_digit() {
            index += 1;
        }
        captures[9] = Some((diag_start, index));
        if index == len {
            captures[1] = Some((body_start, len));
            return Some(ReMatchDetail {
                start: 0,
                end: len,
                captures,
            });
        }
    }

    None
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
            } else if decimal_parser_pattern_matches(pattern_text) {
                pydecimal_parser_match_detail(text)
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
                    _ => return Err(RuntimeError::type_error("string must be bytes-like")),
                },
                Value::ByteArray(obj) => match &*obj.kind() {
                    Object::ByteArray(values) => values.clone(),
                    _ => return Err(RuntimeError::type_error("string must be bytes-like")),
                },
                Value::MemoryView(obj) => match &*obj.kind() {
                    Object::MemoryView(view) => {
                        with_bytes_like_source(&view.source, |values| values.to_vec())
                            .ok_or_else(|| RuntimeError::type_error("string must be bytes-like"))?
                    }
                    _ => return Err(RuntimeError::type_error("string must be bytes-like")),
                },
                Value::Str(_) => {
                    return Err(RuntimeError::new(
                        "cannot use a bytes pattern on a string-like object",
                    ));
                }
                _ => return Err(RuntimeError::type_error("string must be bytes-like")),
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

fn python_source_next_line<'a>(bytes: &'a [u8], cursor: &mut usize) -> &'a [u8] {
    if *cursor >= bytes.len() {
        return &[];
    }
    let start = *cursor;
    while *cursor < bytes.len() {
        let byte = bytes[*cursor];
        *cursor += 1;
        if byte == b'\n' {
            break;
        }
    }
    &bytes[start..*cursor]
}

fn python_source_line_is_blank_or_comment(line: &[u8]) -> bool {
    let mut index = 0usize;
    while index < line.len() && matches!(line[index], b' ' | b'\t' | b'\x0c') {
        index += 1;
    }
    if index >= line.len() {
        return true;
    }
    matches!(line[index], b'#' | b'\r' | b'\n')
}

fn python_source_normalize_cookie_name(name: &str) -> String {
    let lowered = name
        .chars()
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase()
        .replace('_', "-");
    if lowered == "utf-8" || lowered.starts_with("utf-8-") {
        return "utf-8".to_string();
    }
    if matches!(lowered.as_str(), "latin-1" | "iso-8859-1" | "iso-latin-1")
        || lowered.starts_with("latin-1-")
        || lowered.starts_with("iso-8859-1-")
        || lowered.starts_with("iso-latin-1-")
    {
        return "iso-8859-1".to_string();
    }
    name.to_string()
}

fn python_source_extract_cookie_encoding(line: &[u8]) -> Option<String> {
    let mut index = 0usize;
    while index < line.len() && matches!(line[index], b' ' | b'\t' | b'\x0c') {
        index += 1;
    }
    if index >= line.len() || line[index] != b'#' {
        return None;
    }
    let haystack = &line[index..];
    let mut scan = 1usize;
    while scan + 7 <= haystack.len() {
        if &haystack[scan..scan + 6] == b"coding"
            && scan + 6 < haystack.len()
            && matches!(haystack[scan + 6], b':' | b'=')
        {
            let mut start = scan + 7;
            while start < haystack.len() && matches!(haystack[start], b' ' | b'\t') {
                start += 1;
            }
            let mut end = start;
            while end < haystack.len()
                && (haystack[end].is_ascii_alphanumeric()
                    || matches!(haystack[end], b'-' | b'_' | b'.'))
            {
                end += 1;
            }
            if end > start {
                let encoding = std::str::from_utf8(&haystack[start..end]).ok()?.to_string();
                return Some(python_source_normalize_cookie_name(&encoding));
            }
            return None;
        }
        scan += 1;
    }
    None
}

fn python_source_unknown_encoding_error(filename: Option<&str>, encoding: &str) -> RuntimeError {
    match filename {
        Some(path) => RuntimeError::new(format!(
            "SyntaxError: unknown encoding for '{path}': {encoding}"
        )),
        None => RuntimeError::new(format!("SyntaxError: unknown encoding: {encoding}")),
    }
}

fn python_source_encoding_problem_error(filename: Option<&str>) -> RuntimeError {
    match filename {
        Some(path) => {
            RuntimeError::new(format!("SyntaxError: encoding problem for '{path}': utf-8"))
        }
        None => RuntimeError::new("SyntaxError: encoding problem: utf-8"),
    }
}

fn python_source_invalid_or_missing_encoding_error(filename: Option<&str>) -> RuntimeError {
    match filename {
        Some(path) => RuntimeError::new(format!(
            "SyntaxError: invalid or missing encoding declaration for '{path}'"
        )),
        None => RuntimeError::new("SyntaxError: invalid or missing encoding declaration"),
    }
}

fn normalize_codec_encoding(value: Value) -> Result<String, RuntimeError> {
    let name = match value {
        Value::Str(name) => name.to_ascii_lowercase().replace('_', "-"),
        _ => return Err(RuntimeError::new("encoding must be string")),
    };
    match name.as_str() {
        "utf-8" | "utf8" => Ok("utf-8".to_string()),
        "utf-8-sig" | "utf8-sig" | "utf-8sig" | "utf8sig" => Ok("utf-8-sig".to_string()),
        "utf-16" | "utf16" => Ok("utf-16".to_string()),
        "utf-16-le" | "utf16-le" | "utf-16le" | "utf16le" => Ok("utf-16-le".to_string()),
        "utf-16-be" | "utf16-be" | "utf-16be" | "utf16be" => Ok("utf-16-be".to_string()),
        "utf-32" | "utf32" => Ok("utf-32".to_string()),
        "utf-32-le" | "utf32-le" | "utf-32le" | "utf32le" => Ok("utf-32-le".to_string()),
        "utf-32-be" | "utf32-be" | "utf-32be" | "utf32be" => Ok("utf-32-be".to_string()),
        "ascii" | "us-ascii" | "usascii" | "ansi-x3.4-1968" | "ansi-x3.4-1986" | "iso646-us"
        | "cp367" | "646" => Ok("ascii".to_string()),
        "latin-1" | "latin1" | "iso-8859-1" | "iso8859-1" | "cp819" | "l1" => {
            Ok("latin-1".to_string())
        }
        "gbk" | "cp936" | "ms936" | "936" => Ok("gbk".to_string()),
        "raw-unicode-escape" | "raw_unicode_escape" => Ok("raw-unicode-escape".to_string()),
        "unicode-escape" | "unicode_escape" => Ok("unicode-escape".to_string()),
        _ => Err(RuntimeError::lookup_error("unsupported encoding")),
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
        _ => Err(unknown_codec_error_handler(&mode)),
    }
}

fn unknown_codec_error_handler(handler: &str) -> RuntimeError {
    RuntimeError::lookup_error(format!("unknown error handler name '{handler}'"))
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
        _ => Err(unknown_codec_error_handler(errors)),
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
        "utf-8-sig" => {
            let mut out = vec![0xEF, 0xBB, 0xBF];
            out.extend_from_slice(text.as_bytes());
            Ok(out)
        }
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
                    _ => return Err(unknown_codec_error_handler(errors)),
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
                    _ => return Err(unknown_codec_error_handler(errors)),
                }
            }
            Ok(out)
        }
        "gbk" => encode_gbk_bytes(text, errors),
        "raw-unicode-escape" => Ok(encode_raw_unicode_escape(text)),
        "unicode-escape" => Ok(encode_unicode_escape(text)),
        _ => Err(RuntimeError::lookup_error("unsupported encoding")),
    }
}

fn decode_text_bytes(bytes: &[u8], encoding: &str, errors: &str) -> Result<String, RuntimeError> {
    match encoding {
        "utf-8" => decode_utf8_bytes(bytes, errors),
        "utf-8-sig" => {
            let payload = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                &bytes[3..]
            } else {
                bytes
            };
            decode_utf8_bytes(payload, errors)
        }
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
                    _ => return Err(unknown_codec_error_handler(errors)),
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
        "gbk" => decode_gbk_bytes(bytes, errors),
        "raw-unicode-escape" => decode_raw_unicode_escape(bytes, errors),
        "unicode-escape" => decode_unicode_escape(bytes, errors),
        _ => Err(RuntimeError::lookup_error("unsupported encoding")),
    }
}

fn encode_gbk_bytes(text: &str, errors: &str) -> Result<Vec<u8>, RuntimeError> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii() {
            out.push(ch as u8);
            continue;
        }
        match ch {
            '\u{4E02}' => out.extend_from_slice(&[0x81, 0x40]),
            '\u{5100}' => out.extend_from_slice(&[0x83, 0x78]),
            _ => match errors {
                "strict" => return Err(RuntimeError::new("gbk codec can't encode character")),
                "ignore" => {}
                "replace" | "surrogateescape" => out.push(b'?'),
                _ => return Err(unknown_codec_error_handler(errors)),
            },
        }
    }
    Ok(out)
}

fn decode_gbk_bytes(bytes: &[u8], errors: &str) -> Result<String, RuntimeError> {
    let mut out = String::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte <= 0x7F {
            out.push(byte as char);
            index += 1;
            continue;
        }
        let push_decode_error = |out: &mut String| -> Result<(), RuntimeError> {
            match errors {
                "strict" => Err(RuntimeError::new("gbk codec can't decode bytes")),
                "ignore" => Ok(()),
                "replace" | "surrogateescape" => {
                    out.push('\u{FFFD}');
                    Ok(())
                }
                _ => Err(unknown_codec_error_handler(errors)),
            }
        };
        if index + 1 >= bytes.len() {
            push_decode_error(&mut out)?;
            break;
        }
        let pair = (byte, bytes[index + 1]);
        match pair {
            (0x81, 0x40) => out.push('\u{4E02}'),
            (0x83, 0x78) => out.push('\u{5100}'),
            _ => {
                push_decode_error(&mut out)?;
            }
        }
        index += 2;
    }
    Ok(out)
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
                _ => Err(unknown_codec_error_handler(errors)),
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
            _ => return Err(unknown_codec_error_handler(errors)),
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
                _ => return Err(unknown_codec_error_handler(errors)),
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
                _ => return Err(unknown_codec_error_handler(errors)),
            },
        }
        pos += 4;
    }
    if pos < bytes.len() {
        match errors {
            "strict" => return Err(RuntimeError::new("utf-32 codec can't decode bytes")),
            "ignore" => {}
            "replace" | "surrogateescape" => out.push('\u{FFFD}'),
            _ => return Err(unknown_codec_error_handler(errors)),
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
                    _ => return Err(unknown_codec_error_handler(errors)),
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

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i64;
    let day = day as i64;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
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
    utc_offset_seconds: Option<i32>,
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
        utc_offset_seconds: None,
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
    for month_day in month_days.iter().take(month.saturating_sub(1) as usize) {
        total += *month_day;
    }
    total
}

fn time_parts_from_value(value: &Value) -> Result<TimeParts, RuntimeError> {
    fn extract_time_sequence(value: &Value) -> Result<Vec<Value>, RuntimeError> {
        match value {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => Ok(values.clone()),
                _ => Err(RuntimeError::new("invalid time tuple")),
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => Ok(values.clone()),
                _ => Err(RuntimeError::new("invalid time tuple")),
            },
            Value::Instance(instance_obj) => {
                let Object::Instance(instance_data) = &*instance_obj.kind() else {
                    return Err(RuntimeError::new("invalid time tuple"));
                };
                if let Some(backing) = instance_data.attrs.get(TUPLE_BACKING_STORAGE_ATTR) {
                    return extract_time_sequence(backing);
                }
                if let Some(backing) = instance_data.attrs.get(LIST_BACKING_STORAGE_ATTR) {
                    return extract_time_sequence(backing);
                }
                Err(RuntimeError::new("time tuple must be tuple or list"))
            }
            _ => Err(RuntimeError::new("time tuple must be tuple or list")),
        }
    }

    let values = extract_time_sequence(value)?;
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
        utc_offset_seconds: None,
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
            'z' => {
                if let Some(offset_seconds) = parts.utc_offset_seconds {
                    let sign = if offset_seconds >= 0 { '+' } else { '-' };
                    let abs = offset_seconds.unsigned_abs();
                    let hours = abs / 3600;
                    let minutes = (abs % 3600) / 60;
                    out.push(sign);
                    out.push_str(&format!("{hours:02}{minutes:02}"));
                }
            }
            _ => {
                out.push('%');
                out.push(spec);
            }
        }
    }
    out
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
            Object::Cell(cell) => cell.value.as_ref().is_some_and(is_truthy),
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

pub(super) fn builtin_exception_parent(name: &str) -> Option<&'static str> {
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
        "ArithmeticError" => Some("Exception"),
        "FloatingPointError" => Some("ArithmeticError"),
        "OverflowError" => Some("ArithmeticError"),
        "ZeroDivisionError" => Some("ArithmeticError"),
        "AssertionError" => Some("Exception"),
        "AttributeError" => Some("Exception"),
        "LookupError" => Some("Exception"),
        "IndexError" => Some("LookupError"),
        "KeyError" => Some("LookupError"),
        "BufferError" => Some("Exception"),
        "EOFError" => Some("Exception"),
        "MemoryError" => Some("Exception"),
        "ReferenceError" => Some("Exception"),
        "SyntaxError" => Some("Exception"),
        "IndentationError" => Some("SyntaxError"),
        "TabError" => Some("IndentationError"),
        "NameError" => Some("Exception"),
        "UnboundLocalError" => Some("NameError"),
        "ImportError" => Some("Exception"),
        "ModuleNotFoundError" => Some("ImportError"),
        "RuntimeError" => Some("Exception"),
        "RecursionError" => Some("RuntimeError"),
        "PythonFinalizationError" => Some("RuntimeError"),
        "NotImplementedError" => Some("RuntimeError"),
        "SystemError" => Some("Exception"),
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
        "PickleError" => Some("Exception"),
        "PicklingError" => Some("PickleError"),
        "UnpicklingError" => Some("PickleError"),
        "ExpatError" => Some("Exception"),
        "SSLError" => Some("OSError"),
        "SSLZeroReturnError" => Some("SSLError"),
        "SSLWantReadError" => Some("SSLError"),
        "SSLWantWriteError" => Some("SSLError"),
        "SSLSyscallError" => Some("SSLError"),
        "SSLEOFError" => Some("SSLError"),
        "SSLCertVerificationError" => Some("SSLError"),
        "Error" => Some("Exception"),
        "Warning" => Some("Exception"),
        "UserWarning" => Some("Warning"),
        "DeprecationWarning" => Some("Warning"),
        "PendingDeprecationWarning" => Some("Warning"),
        "RuntimeWarning" => Some("Warning"),
        "SyntaxWarning" => Some("Warning"),
        "FutureWarning" => Some("Warning"),
        "ImportWarning" => Some("Warning"),
        "UnicodeWarning" => Some("Warning"),
        "BytesWarning" => Some("Warning"),
        "ResourceWarning" => Some("Warning"),
        "EncodingWarning" => Some("Warning"),
        "InterfaceError" => Some("Error"),
        "DatabaseError" => Some("Error"),
        "DataError" => Some("DatabaseError"),
        "OperationalError" => Some("DatabaseError"),
        "IntegrityError" => Some("DatabaseError"),
        "InternalError" => Some("DatabaseError"),
        "ProgrammingError" => Some("DatabaseError"),
        "NotSupportedError" => Some("DatabaseError"),
        "UnicodeError" => Some("ValueError"),
        "UnicodeEncodeError" => Some("UnicodeError"),
        "UnicodeDecodeError" => Some("UnicodeError"),
        "UnicodeTranslateError" => Some("UnicodeError"),
        _ => None,
    }
}

/// Canonical argument binding output before locals/cell assignment.
///
/// Values are split by CPython-style parameter kinds so call binding and frame
/// assignment can remain decoupled.
struct BoundArguments {
    posonly: Vec<Value>,
    positional: Vec<Value>,
    kwonly: Vec<Value>,
    vararg: Option<Value>,
    kwarg: Option<Value>,
}

fn format_missing_positional_arguments_error(function_name: &str, missing: &[String]) -> String {
    match missing.len() {
        0 => format!("{function_name}() missing required positional argument"),
        1 => format!(
            "{function_name}() missing 1 required positional argument: '{}'",
            missing[0]
        ),
        2 => format!(
            "{function_name}() missing 2 required positional arguments: '{}' and '{}'",
            missing[0], missing[1]
        ),
        count => {
            let mut quoted = missing
                .iter()
                .map(|name| format!("'{name}'"))
                .collect::<Vec<_>>();
            let tail = quoted.pop().unwrap_or_default();
            format!(
                "{function_name}() missing {count} required positional arguments: {}, and {}",
                quoted.join(", "),
                tail
            )
        }
    }
}

fn format_too_many_positional_arguments_error(
    function_name: &str,
    min_expected: usize,
    max_expected: usize,
    given: usize,
) -> String {
    let given_verb = if given == 1 { "was" } else { "were" };
    if min_expected == max_expected {
        let arg_word = if max_expected == 1 {
            "argument"
        } else {
            "arguments"
        };
        return format!(
            "{function_name}() takes {max_expected} positional {arg_word} but {given} {given_verb} given"
        );
    }
    format!(
        "{function_name}() takes from {min_expected} to {max_expected} positional arguments but {given} {given_verb} given"
    )
}

fn function_name_for_argument_errors(func: &FunctionObject) -> String {
    if let Some(dict) = &func.dict
        && let Object::Dict(entries) = &*dict.kind()
    {
        for (key, value) in entries.iter() {
            if matches!(key, Value::Str(name) if name == "__qualname__")
                && let Value::Str(qualname) = value
            {
                return qualname.clone();
            }
        }
    }
    if let Some(owner_class) = &func.owner_class
        && let Object::Class(class_data) = &*owner_class.kind()
    {
        let owner_qualname = match class_data.attrs.get("__qualname__") {
            Some(Value::Str(qualname)) => qualname.clone(),
            _ => class_data.name.clone(),
        };
        return format!("{owner_qualname}.{}", func.code.name);
    }
    func.code.name.clone()
}

/// Bind positional/keyword call inputs to a function signature.
///
/// Semantics intentionally follow CPython 3.14 for positional-only handling,
/// default filling, duplicate detection, and keyword insertion order preservation
/// (`kwargs_order`) for `**kwargs`.
fn bind_arguments(
    func: &FunctionObject,
    heap: &Heap,
    mut positional: Vec<Value>,
    mut kwargs: HashMap<String, Value>,
    kwargs_order: Option<Vec<String>>,
) -> Result<BoundArguments, RuntimeError> {
    let function_name = function_name_for_argument_errors(func);
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
            if env_var_present_cached("PYRS_TRACE_BIND_ARGS") {
                eprintln!(
                    "[bind-args] fn={} file={} count-mismatch positional={} expected={} posonly={:?} params={:?}",
                    func.code.name,
                    func.code.filename,
                    positional.len(),
                    total_positional,
                    func.code.posonly_params,
                    func.code.params
                );
            }
            if positional.len() < total_positional {
                let missing = (positional.len()..total_positional)
                    .map(|idx| {
                        if idx < posonly_len {
                            func.code.posonly_params[idx].clone()
                        } else {
                            func.code.params[idx - posonly_len].clone()
                        }
                    })
                    .collect::<Vec<_>>();
                return Err(RuntimeError::type_error(
                    format_missing_positional_arguments_error(&function_name, &missing),
                ));
            }
            return Err(RuntimeError::type_error(
                format_too_many_positional_arguments_error(
                    &function_name,
                    total_positional,
                    total_positional,
                    positional.len(),
                ),
            ));
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
            if env_var_present_cached("PYRS_TRACE_BIND_ARGS") {
                eprintln!(
                    "[bind-args] fn={} file={} extra-positional={} max={} (no vararg)",
                    func.code.name,
                    func.code.filename,
                    positional.len(),
                    total_positional
                );
            }
            let min_positional = total_positional.saturating_sub(defaults_len);
            return Err(RuntimeError::type_error(
                format_too_many_positional_arguments_error(
                    &function_name,
                    min_positional,
                    total_positional,
                    positional.len(),
                ),
            ));
        }
        extra_positional = positional.split_off(total_positional);
    }

    let required = total_positional - defaults_len;
    let mut bound: Vec<Option<Value>> = vec![None; total_positional];

    for (idx, value) in positional.into_iter().enumerate() {
        bound[idx] = Some(value);
    }

    let mut extra_kwargs: Vec<(String, Value)> = Vec::new();
    let mut extra_kwargs_seen: HashSet<String> = HashSet::new();
    let mut kwonly_values: HashMap<String, Value> = HashMap::new();
    let mut ordered_kwargs: Vec<(String, Value)> = Vec::new();
    if let Some(order) = kwargs_order {
        for name in order {
            if let Some(value) = kwargs.remove(&name) {
                ordered_kwargs.push((name, value));
            }
        }
    }
    for (name, value) in kwargs.drain() {
        ordered_kwargs.push((name, value));
    }

    for (name, value) in ordered_kwargs {
        if func.code.posonly_params.iter().any(|param| param == &name) {
            if func.code.kwarg.is_some() {
                if !extra_kwargs_seen.insert(name.clone()) {
                    return Err(RuntimeError::type_error(format!(
                        "{}() got multiple values for argument '{}'",
                        function_name, name
                    )));
                }
                extra_kwargs.push((name, value));
                continue;
            }
            if env_var_present_cached("PYRS_TRACE_BIND_ARGS") {
                eprintln!(
                    "[bind-args] fn={} unexpected-posonly-keyword={}",
                    func.code.name, name
                );
                eprintln!(
                    "[bind-args] fn={} signature posonly={:?} params={:?} kwonly={:?} vararg={:?} kwarg={:?}",
                    func.code.name,
                    func.code.posonly_params,
                    func.code.params,
                    func.code.kwonly_params,
                    func.code.vararg,
                    func.code.kwarg
                );
            }
            return Err(RuntimeError::type_error(format!(
                "{}() got an unexpected keyword argument '{}'",
                function_name, name
            )));
        }
        if let Some(index) = func.code.params.iter().position(|param| param == &name) {
            if bound[posonly_len + index].is_some() {
                return Err(RuntimeError::type_error(format!(
                    "{}() got multiple values for argument '{}'",
                    function_name, name
                )));
            }
            bound[posonly_len + index] = Some(value);
        } else if func.code.kwonly_params.iter().any(|param| param == &name) {
            if kwonly_values.contains_key(&name) {
                return Err(RuntimeError::type_error(format!(
                    "{}() got multiple values for argument '{}'",
                    function_name, name
                )));
            }
            kwonly_values.insert(name, value);
        } else if func.code.kwarg.is_some() {
            if !extra_kwargs_seen.insert(name.clone()) {
                return Err(RuntimeError::type_error(format!(
                    "{}() got multiple values for argument '{}'",
                    function_name, name
                )));
            }
            extra_kwargs.push((name, value));
        } else {
            if env_var_present_cached("PYRS_TRACE_BIND_ARGS") {
                eprintln!(
                    "[bind-args] fn={} unexpected-keyword={}",
                    func.code.name, name
                );
                eprintln!(
                    "[bind-args] fn={} signature posonly={:?} params={:?} kwonly={:?} vararg={:?} kwarg={:?}",
                    func.code.name,
                    func.code.posonly_params,
                    func.code.params,
                    func.code.kwonly_params,
                    func.code.vararg,
                    func.code.kwarg
                );
            }
            return Err(RuntimeError::type_error(format!(
                "{}() got an unexpected keyword argument '{}'",
                function_name, name
            )));
        }
    }

    let mut missing_required = Vec::new();
    for idx in 0..required {
        if bound[idx].is_none() {
            let name = if idx < posonly_len {
                func.code.posonly_params[idx].clone()
            } else {
                func.code.params[idx - posonly_len].clone()
            };
            missing_required.push(name);
        }
    }
    if !missing_required.is_empty() {
        if env_var_present_cached("PYRS_TRACE_BIND_ARGS") {
            eprintln!(
                "[bind-args] fn={} file={} missing-required positional={:?} signature posonly={:?} params={:?}",
                func.code.name,
                func.code.filename,
                missing_required,
                func.code.posonly_params,
                func.code.params
            );
        }
        return Err(RuntimeError::type_error(
            format_missing_positional_arguments_error(&function_name, &missing_required),
        ));
    }

    for (idx, slot) in bound
        .iter_mut()
        .enumerate()
        .take(total_positional)
        .skip(required)
    {
        if slot.is_none() {
            let default_index = idx - required;
            *slot = Some(func.defaults[default_index].clone());
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
            return Err(RuntimeError::type_error("missing keyword-only argument"));
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
    if let Some(idx) = code.cellvar_to_index.get(name).copied()
        && let Some(cell) = frame.cells.get(idx)
        && let Object::Cell(cell_data) = &mut *cell.kind_mut()
    {
        cell_data.value = Some(value.clone());
        if let Some(slot_idx) = code.name_to_index.get(name).copied()
            && let Some(slot) = frame.fast_locals.get_mut(slot_idx)
        {
            *slot = Some(value.clone());
        }
        if let Some(existing) = frame.locals.get_mut(name) {
            *existing = value;
        }
        return;
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

/// Materialize `BoundArguments` into frame locals/cells using `CodeObject` layout.
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
            if !kwargs.is_empty() {
                return Err(RuntimeError::type_error("len() takes no keyword arguments"));
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
                let keyword = kwargs
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".to_string());
                return Err(RuntimeError::type_error(format!(
                    "range() got an unexpected keyword argument '{}'",
                    keyword
                )));
            }

            match args.len() {
                0 => {}
                1 => {
                    if stop.is_some() {
                        return Err(RuntimeError::type_error("range() got multiple values"));
                    }
                    stop = Some(args.remove(0));
                }
                2 => {
                    if start.is_some() || stop.is_some() {
                        return Err(RuntimeError::type_error("range() got multiple values"));
                    }
                    start = Some(args.remove(0));
                    stop = Some(args.remove(0));
                }
                3 => {
                    if start.is_some() || stop.is_some() || step.is_some() {
                        return Err(RuntimeError::type_error("range() got multiple values"));
                    }
                    start = Some(args.remove(0));
                    stop = Some(args.remove(0));
                    step = Some(args.remove(0));
                }
                _ => return Err(RuntimeError::type_error("range() expects 1-3 arguments")),
            }

            let stop = stop.ok_or_else(|| {
                RuntimeError::type_error("range expected at least 1 argument, got 0")
            })?;
            let start = start.unwrap_or(Value::Int(0));
            let step = step.unwrap_or(Value::Int(1));

            let start_big = value_to_bigint(start)?;
            let stop_big = value_to_bigint(stop)?;
            let step_big = value_to_bigint(step)?;
            if step_big.is_zero() {
                return Err(RuntimeError::value_error("range() step cannot be zero"));
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
        BuiltinFunction::ContextVar => {
            if let Some(name) = kwargs.remove("name") {
                if !args.is_empty() {
                    return Err(RuntimeError::new(
                        "ContextVar() got multiple values for argument 'name'",
                    ));
                }
                args.push(name);
            }
            if let Some(default) = kwargs.remove("default") {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "ContextVar() got multiple values for argument 'default'",
                    ));
                }
                args.push(default);
            }
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "ContextVar() got an unexpected keyword argument",
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
        BuiltinFunction::NoOp => {
            kwargs.clear();
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
        Value::Builtin(BuiltinFunction::ImportlibPathHook) => expected == DEFAULT_PATH_HOOK,
        Value::Instance(instance) => matches!(
            &*instance.kind(),
            Object::Instance(instance_data)
                if matches!(
                    instance_data.attrs.get("kind"),
                    Some(Value::Str(name)) if name == expected
                )
        ),
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
    fn proxy_seen_key(class: &ObjRef) -> Option<u64> {
        let class_kind = class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return None;
        };
        match class_data.attrs.get("__pyrs_cpython_proxy_ptr__") {
            Some(Value::Int(raw_ptr)) if *raw_ptr >= 0 => Some((*raw_ptr as u64) | (1u64 << 63)),
            _ => None,
        }
    }

    fn seen_contains_class(class: &ObjRef, seen: &HashSet<u64>) -> bool {
        if seen.contains(&class.id()) {
            return true;
        }
        proxy_seen_key(class).is_some_and(|key| seen.contains(&key))
    }

    fn seen_insert_class(class: &ObjRef, seen: &mut HashSet<u64>) -> bool {
        let mut inserted = seen.insert(class.id());
        if let Some(proxy_key) = proxy_seen_key(class) {
            inserted = seen.insert(proxy_key) || inserted;
        }
        inserted
    }

    fn walk_recursive(
        class: &ObjRef,
        out: &mut Vec<ObjRef>,
        seen: &mut HashSet<u64>,
        depth: usize,
    ) {
        if env_var_present_cached("PYRS_DEBUG_CLASS_ATTR_WALK_DEPTH") && depth > 256 {
            let class_name = match &*class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "<non-class>".to_string(),
            };
            panic!(
                "class_attr_walk recursion depth exceeded: depth={depth} class={class_name} id={}",
                class.id()
            );
        }
        let class_kind = class.kind();
        let class_data = match &*class_kind {
            Object::Class(class_data) => class_data,
            _ => return,
        };
        if !class_data.mro.is_empty() {
            let mut mro = class_data.mro.clone();
            if let Some(object_idx) = mro.iter().position(|entry| {
                matches!(&*entry.kind(), Object::Class(candidate) if candidate.name == "object")
            })
                && object_idx + 1 != mro.len()
            {
                let object_entry = mro.remove(object_idx);
                mro.push(object_entry);
            }
            for entry in mro {
                if !seen_contains_class(&entry, seen) && seen_insert_class(&entry, seen) {
                    out.push(entry);
                }
            }
            return;
        }
        if !seen_insert_class(class, seen) {
            return;
        }
        out.push(class.clone());
        for base in &class_data.bases {
            walk_recursive(base, out, seen, depth.saturating_add(1));
        }
    }

    let mut out = Vec::new();
    let mut seen: HashSet<u64> = HashSet::new();
    walk_recursive(class, &mut out, &mut seen, 0);
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
        Value::Dict(obj) => {
            if let Object::Dict(entries) = &*obj.kind() {
                for (key, _) in entries {
                    if let Value::Str(name) = key {
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
    input_class_data.slots.as_ref()?;

    let mut names = Vec::new();
    for candidate in class_attr_walk(class) {
        if let Object::Class(class_data) = &*candidate.kind()
            && let Some(slots) = &class_data.slots
        {
            for slot in slots {
                if !names.iter().any(|existing| existing == slot) {
                    names.push(slot.clone());
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
                let user_class = matches!(
                    class_data.attrs.get("__pyrs_user_class__"),
                    Some(Value::Bool(true))
                );
                let cpython_proxy = matches!(
                    class_data.attrs.get("__pyrs_cpython_proxy_marker__"),
                    Some(Value::Bool(true))
                );
                if user_class || cpython_proxy {
                    return true;
                }
            }
        }
    }
    false
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
            if let Some((next_idx, next_ch)) = chars.peek().copied()
                && next_ch == '\n'
            {
                let _ = chars.next();
                end = next_idx + next_ch.len_utf8();
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
    runtime_error_matches_exception(err, "AttributeError")
}

fn exception_type_is_subclass(candidate: &str, expected: &str) -> bool {
    if candidate == expected || expected == "BaseException" {
        return true;
    }
    if expected == "Warning" && candidate.ends_with("Warning") {
        return true;
    }
    let mut current = Some(candidate);
    let mut depth = 0usize;
    while let Some(name) = current {
        if name == expected {
            return true;
        }
        current = builtin_exception_parent(name);
        depth += 1;
        if depth > 64 {
            break;
        }
    }
    false
}

#[inline]
fn is_os_error_family(name: &str) -> bool {
    matches!(
        name,
        "OSError"
            | "BlockingIOError"
            | "ChildProcessError"
            | "ConnectionError"
            | "BrokenPipeError"
            | "ConnectionAbortedError"
            | "ConnectionRefusedError"
            | "ConnectionResetError"
            | "FileNotFoundError"
            | "FileExistsError"
            | "InterruptedError"
            | "ProcessLookupError"
            | "TimeoutError"
            | "PermissionError"
            | "NotADirectoryError"
            | "IsADirectoryError"
    )
}

#[inline]
fn is_import_error_family(name: &str) -> bool {
    matches!(name, "ImportError" | "ModuleNotFoundError")
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

fn runtime_error_matches_exception(err: &RuntimeError, expected: &str) -> bool {
    err.exception_name()
        .is_some_and(|exception_name| exception_type_is_subclass(exception_name, expected))
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
        return Err(RuntimeError::value_error("slice step cannot be zero"));
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
