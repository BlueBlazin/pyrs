#[cfg(feature = "wasm-vm-probe")]
use std::cell::RefCell;
use std::collections::HashSet;
#[cfg(feature = "wasm-vm-probe")]
use std::sync::Arc;
use std::sync::Once;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use crate::host::{HostCapability, VmHost, WasmHost};
#[cfg(feature = "wasm-vm-probe")]
use crate::vm::Vm;
use js_sys::Array;
use wasm_bindgen::prelude::*;

pub const WASM_API_VERSION: u32 = 1;
const WASM_EXECUTION_BACKEND_UNWIRED: &str = "unwired";
#[cfg(feature = "wasm-vm-probe")]
const WASM_EXECUTION_BACKEND_VM_PROBE: &str = "vm_probe";
const WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED: &str = "execution_backend_unwired";
const WASM_EXECUTION_BLOCKER_VM_RUNTIME_UNAVAILABLE: &str = "vm_runtime_unavailable";
const WASM_EXECUTION_PHASE_OK: &str = "ok";
#[cfg(feature = "wasm-vm-probe")]
const WASM_EXECUTION_PHASE_RUNTIME_ERROR: &str = "runtime_error";
const WASM_WORKER_BLOCKER_RUNTIME_UNWIRED: &str = "worker_runtime_unwired";
const WASM_WORKER_BLOCKER_RUNTIME_FAILED: &str = "worker_runtime_failed";
const WASM_WORKER_INTERRUPT_MODEL_RECYCLE: &str = "worker_recycle";
const WASM_WORKER_BACKEND_UNWIRED: &str = "unwired";
#[cfg(feature = "wasm-vm-probe")]
const WASM_WORKER_BACKEND_VM_PROBE: &str = "vm_probe";
#[cfg(feature = "wasm-vm-probe")]
const WASM_WORKER_LIFECYCLE_PHASE_STARTED: &str = "worker_started";
#[cfg(feature = "wasm-vm-probe")]
const WASM_WORKER_LIFECYCLE_PHASE_TERMINATED: &str = "worker_terminated";
#[cfg(feature = "wasm-vm-probe")]
const WASM_WORKER_LIFECYCLE_PHASE_RECYCLED: &str = "worker_recycled";
const WASM_WORKER_TIMEOUT_DEFAULT_MS: u32 = 5_000;
const WASM_WORKER_TIMEOUT_MIN_MS: u32 = 50;
const WASM_WORKER_TIMEOUT_MAX_MS: u32 = 120_000;
const WASM_REPL_FILENAME: &str = "<wasm-repl>";
const WASM_WORKER_TIMEOUT_UNSUPPORTED_PHASE: &str = "unsupported_worker_timeout_enforcement";
const WASM_WORKER_TIMEOUT_INVALID_PHASE: &str = "invalid_worker_timeout";
#[cfg(feature = "wasm-vm-probe")]
const WASM_WORKER_TIMEOUT_CONFIGURED_PHASE: &str = "worker_timeout_configured";
#[cfg(feature = "wasm-vm-probe")]
const WASM_WORKER_TIMEOUT_EXCEEDED_PREFIX: &str = "execution timeout exceeded";
const WASM_MODULE_BLOCKER_POLICY: [(&str, &str); 10] = [
    ("_ctypes", "dynamic_library_load"),
    ("ctypes", "dynamic_library_load"),
    ("numpy", "dynamic_library_load"),
    ("scipy", "dynamic_library_load"),
    ("_socket", "network_sockets"),
    ("socket", "network_sockets"),
    ("_posixsubprocess", "process_spawn"),
    ("subprocess", "process_spawn"),
    ("multiprocessing", "process_spawn"),
    ("readline", "interactive_terminal"),
];

fn module_blocker_key(module_name: &str) -> Option<&'static str> {
    WASM_MODULE_BLOCKER_POLICY
        .iter()
        .find_map(|(name, blocker)| (*name == module_name).then_some(*blocker))
}

fn module_policy_blocker_keys() -> Vec<&'static str> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    for (_, blocker_key) in WASM_MODULE_BLOCKER_POLICY {
        if seen.insert(blocker_key) {
            keys.push(blocker_key);
        }
    }
    keys
}

fn execution_blocker_keys(host: &dyn VmHost) -> Vec<&'static str> {
    let mut keys = Vec::new();
    if !wasm_vm_runtime_enabled() {
        keys.push(WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED);
        keys.push(WASM_EXECUTION_BLOCKER_VM_RUNTIME_UNAVAILABLE);
    }
    for capability in HostCapability::all() {
        if !host.supports(*capability) {
            keys.push(capability.key());
        }
    }
    keys
}

fn wasm_vm_runtime_enabled() -> bool {
    cfg!(feature = "wasm-vm-probe")
}

fn worker_blocker_keys() -> Vec<&'static str> {
    let mut keys = vec![
        WASM_WORKER_BLOCKER_RUNTIME_UNWIRED,
        WASM_WORKER_BLOCKER_RUNTIME_FAILED,
    ];
    keys.extend(module_policy_blocker_keys());
    keys
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum WasmWorkerState {
    Unwired,
    Starting,
    Ready,
    Busy,
    Terminating,
    Failed,
}

impl WasmWorkerState {
    const ALL: [WasmWorkerState; 6] = [
        WasmWorkerState::Unwired,
        WasmWorkerState::Starting,
        WasmWorkerState::Ready,
        WasmWorkerState::Busy,
        WasmWorkerState::Terminating,
        WasmWorkerState::Failed,
    ];

    fn key(self) -> &'static str {
        match self {
            WasmWorkerState::Unwired => "unwired",
            WasmWorkerState::Starting => "starting",
            WasmWorkerState::Ready => "ready",
            WasmWorkerState::Busy => "busy",
            WasmWorkerState::Terminating => "terminating",
            WasmWorkerState::Failed => "failed",
        }
    }

    fn from_storage(value: u8) -> Self {
        match value {
            x if x == Self::Unwired as u8 => Self::Unwired,
            x if x == Self::Starting as u8 => Self::Starting,
            x if x == Self::Ready as u8 => Self::Ready,
            x if x == Self::Busy as u8 => Self::Busy,
            x if x == Self::Terminating as u8 => Self::Terminating,
            x if x == Self::Failed as u8 => Self::Failed,
            _ => worker_state_baseline(),
        }
    }
}

#[cfg(feature = "wasm-vm-probe")]
const fn worker_state_baseline() -> WasmWorkerState {
    WasmWorkerState::Ready
}

#[cfg(not(feature = "wasm-vm-probe"))]
const fn worker_state_baseline() -> WasmWorkerState {
    WasmWorkerState::Unwired
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WasmWorkerLifecyclePhase {
    UnsupportedStart,
    UnsupportedTerminate,
    UnsupportedRecycle,
}

impl WasmWorkerLifecyclePhase {
    const ALL: [WasmWorkerLifecyclePhase; 3] = [
        WasmWorkerLifecyclePhase::UnsupportedStart,
        WasmWorkerLifecyclePhase::UnsupportedTerminate,
        WasmWorkerLifecyclePhase::UnsupportedRecycle,
    ];

    fn key(self) -> &'static str {
        match self {
            WasmWorkerLifecyclePhase::UnsupportedStart => "unsupported_worker_start",
            WasmWorkerLifecyclePhase::UnsupportedTerminate => "unsupported_worker_terminate",
            WasmWorkerLifecyclePhase::UnsupportedRecycle => "unsupported_worker_recycle",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WasmWorkerExecutePhase {
    SyntaxError,
    CompileError,
    UnsupportedExecution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WasmWorkerTimeoutPhase {
    UnsupportedEnforcement,
    InvalidTimeout,
}

impl WasmWorkerTimeoutPhase {
    const ALL: [WasmWorkerTimeoutPhase; 2] = [
        WasmWorkerTimeoutPhase::UnsupportedEnforcement,
        WasmWorkerTimeoutPhase::InvalidTimeout,
    ];

    fn key(self) -> &'static str {
        match self {
            WasmWorkerTimeoutPhase::UnsupportedEnforcement => WASM_WORKER_TIMEOUT_UNSUPPORTED_PHASE,
            WasmWorkerTimeoutPhase::InvalidTimeout => WASM_WORKER_TIMEOUT_INVALID_PHASE,
        }
    }
}

impl WasmWorkerExecutePhase {
    const ALL: [WasmWorkerExecutePhase; 3] = [
        WasmWorkerExecutePhase::SyntaxError,
        WasmWorkerExecutePhase::CompileError,
        WasmWorkerExecutePhase::UnsupportedExecution,
    ];

    fn key(self) -> &'static str {
        match self {
            WasmWorkerExecutePhase::SyntaxError => "syntax_error",
            WasmWorkerExecutePhase::CompileError => "compile_error",
            WasmWorkerExecutePhase::UnsupportedExecution => "unsupported_worker_execution",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WasmExecutionPhase {
    SyntaxError,
    CompileError,
    UnsupportedExecution,
}

impl WasmExecutionPhase {
    const ALL: [WasmExecutionPhase; 3] = [
        WasmExecutionPhase::SyntaxError,
        WasmExecutionPhase::CompileError,
        WasmExecutionPhase::UnsupportedExecution,
    ];

    fn key(self) -> &'static str {
        match self {
            WasmExecutionPhase::SyntaxError => "syntax_error",
            WasmExecutionPhase::CompileError => "compile_error",
            WasmExecutionPhase::UnsupportedExecution => "unsupported_execution",
        }
    }
}

fn execution_phase_keys() -> Vec<&'static str> {
    #[cfg(feature = "wasm-vm-probe")]
    {
        let mut keys: Vec<&'static str> = WasmExecutionPhase::ALL
            .iter()
            .map(|phase| phase.key())
            .collect();
        keys.push(WASM_EXECUTION_PHASE_OK);
        keys.push(WASM_EXECUTION_PHASE_RUNTIME_ERROR);
        keys
    }
    #[cfg(not(feature = "wasm-vm-probe"))]
    {
        WasmExecutionPhase::ALL
            .iter()
            .map(|phase| phase.key())
            .collect()
    }
}

fn worker_state_keys() -> Vec<&'static str> {
    WasmWorkerState::ALL
        .iter()
        .map(|state| state.key())
        .collect()
}

fn worker_lifecycle_phase_keys() -> Vec<&'static str> {
    #[cfg(feature = "wasm-vm-probe")]
    {
        let mut keys: Vec<&'static str> = WasmWorkerLifecyclePhase::ALL
            .iter()
            .map(|phase| phase.key())
            .collect();
        keys.push(WASM_WORKER_LIFECYCLE_PHASE_STARTED);
        keys.push(WASM_WORKER_LIFECYCLE_PHASE_TERMINATED);
        keys.push(WASM_WORKER_LIFECYCLE_PHASE_RECYCLED);
        keys
    }
    #[cfg(not(feature = "wasm-vm-probe"))]
    {
        WasmWorkerLifecyclePhase::ALL
            .iter()
            .map(|phase| phase.key())
            .collect()
    }
}

fn worker_execute_phase_keys() -> Vec<&'static str> {
    #[cfg(feature = "wasm-vm-probe")]
    {
        let mut keys: Vec<&'static str> = WasmWorkerExecutePhase::ALL
            .iter()
            .map(|phase| phase.key())
            .collect();
        keys.push(WASM_EXECUTION_PHASE_OK);
        keys.push(WASM_EXECUTION_PHASE_RUNTIME_ERROR);
        keys
    }
    #[cfg(not(feature = "wasm-vm-probe"))]
    {
        WasmWorkerExecutePhase::ALL
            .iter()
            .map(|phase| phase.key())
            .collect()
    }
}

fn worker_timeout_phase_keys() -> Vec<&'static str> {
    #[cfg(feature = "wasm-vm-probe")]
    {
        let mut keys: Vec<&'static str> = WasmWorkerTimeoutPhase::ALL
            .iter()
            .map(|phase| phase.key())
            .collect();
        keys.push(WASM_WORKER_TIMEOUT_CONFIGURED_PHASE);
        keys
    }
    #[cfg(not(feature = "wasm-vm-probe"))]
    {
        WasmWorkerTimeoutPhase::ALL
            .iter()
            .map(|phase| phase.key())
            .collect()
    }
}

fn current_worker_state() -> WasmWorkerState {
    WasmWorkerState::from_storage(CURRENT_WASM_WORKER_STATE.load(Ordering::Relaxed))
}

fn set_current_worker_state(state: WasmWorkerState) {
    CURRENT_WASM_WORKER_STATE.store(state as u8, Ordering::Relaxed);
}

fn worker_runtime_ready() -> bool {
    current_worker_state() == WasmWorkerState::Ready
}

fn current_worker_state_key() -> String {
    current_worker_state().key().to_string()
}

fn current_worker_timeout_ms() -> u32 {
    CURRENT_WASM_WORKER_TIMEOUT_MS.load(Ordering::Relaxed)
}

fn set_current_worker_timeout_ms(timeout_ms: u32) {
    CURRENT_WASM_WORKER_TIMEOUT_MS.store(timeout_ms, Ordering::Relaxed);
}

fn reset_worker_timeout_ms() {
    set_current_worker_timeout_ms(WASM_WORKER_TIMEOUT_DEFAULT_MS);
}

fn worker_timeout_policy_unsupported_reason() -> String {
    wasm_worker_blocker_error(WASM_WORKER_BLOCKER_RUNTIME_UNWIRED)
        .unwrap_or_else(|| "wasm worker runtime is not wired yet".to_string())
}

fn worker_unavailable_blocker_key_for_state(state: WasmWorkerState) -> &'static str {
    if state == WasmWorkerState::Failed {
        WASM_WORKER_BLOCKER_RUNTIME_FAILED
    } else {
        WASM_WORKER_BLOCKER_RUNTIME_UNWIRED
    }
}

fn worker_unavailable_error_for_state(state: WasmWorkerState) -> String {
    let blocker_key = worker_unavailable_blocker_key_for_state(state);
    wasm_worker_blocker_error(blocker_key).unwrap_or_else(|| {
        if blocker_key == WASM_WORKER_BLOCKER_RUNTIME_FAILED {
            "wasm worker runtime entered failed state".to_string()
        } else {
            "wasm worker runtime is not wired yet".to_string()
        }
    })
}

/// Minimal WASM bridge surface used during compile-isolation bring-up.
#[wasm_bindgen]
pub fn pyrs_version() -> String {
    crate::VERSION.to_string()
}

/// Version of the wasm JS-facing API contract.
#[wasm_bindgen]
pub fn wasm_api_version() -> u32 {
    WASM_API_VERSION
}

static PANIC_HOOK_ONCE: Once = Once::new();
static NEXT_WASM_WORKER_OPERATION_ID: AtomicU64 = AtomicU64::new(1);
static CURRENT_WASM_WORKER_STATE: AtomicU8 = AtomicU8::new(worker_state_baseline() as u8);
static CURRENT_WASM_WORKER_TIMEOUT_MS: AtomicU32 = AtomicU32::new(WASM_WORKER_TIMEOUT_DEFAULT_MS);
#[cfg(feature = "wasm-vm-probe")]
thread_local! {
    static WASM_WORKER_VM: RefCell<Option<Vm>> = const { RefCell::new(None) };
}

fn next_worker_operation_id(action: &str) -> String {
    let id = NEXT_WASM_WORKER_OPERATION_ID.fetch_add(1, Ordering::Relaxed);
    format!("worker_{action}_{id}")
}

/// Installs panic hook once so Rust panic payloads surface in browser console.
#[wasm_bindgen]
pub fn init_wasm_runtime() {
    PANIC_HOOK_ONCE.call_once(console_error_panic_hook::set_once);
}

#[cfg(feature = "wasm-vm-probe")]
fn new_wasm_repl_vm() -> Vm {
    Vm::new_with_host(Arc::new(WasmHost))
}

#[cfg(feature = "wasm-vm-probe")]
fn clear_worker_vm() {
    WASM_WORKER_VM.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

#[cfg(feature = "wasm-vm-probe")]
fn reset_worker_vm() {
    WASM_WORKER_VM.with(|slot| {
        *slot.borrow_mut() = Some(new_wasm_repl_vm());
    });
}

#[cfg(feature = "wasm-vm-probe")]
fn ensure_worker_vm_initialized() {
    WASM_WORKER_VM.with(|slot| {
        if slot.borrow().is_none() {
            *slot.borrow_mut() = Some(new_wasm_repl_vm());
        }
    });
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmSyntaxResult {
    ok: bool,
    error: Option<String>,
    line: usize,
    column: usize,
}

#[wasm_bindgen]
impl WasmSyntaxResult {
    #[wasm_bindgen(getter)]
    pub fn ok(&self) -> bool {
        self.ok
    }

    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn line(&self) -> usize {
        self.line
    }

    #[wasm_bindgen(getter)]
    pub fn column(&self) -> usize {
        self.column
    }
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmCompileResult {
    ok: bool,
    phase: String,
    error: Option<String>,
    line: usize,
    column: usize,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmSession {
    snippets_checked: usize,
    last_error: Option<String>,
}

#[wasm_bindgen]
pub struct WasmReplSession {
    inputs_executed: usize,
    last_error: Option<String>,
    repl_state: crate::repl_core::ReplCoreState,
    #[cfg(feature = "wasm-vm-probe")]
    vm: Vm,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmWorkerSession {
    starts_requested: usize,
    terminates_requested: usize,
    recycles_requested: usize,
    executes_requested: usize,
    timeout_updates_requested: usize,
    last_timeout_ms_requested: Option<u32>,
    last_operation_id: Option<String>,
    last_phase: Option<String>,
    last_state: Option<String>,
    last_error: Option<String>,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmWorkerSessionSnapshot {
    starts_requested: usize,
    terminates_requested: usize,
    recycles_requested: usize,
    executes_requested: usize,
    timeout_updates_requested: usize,
    last_timeout_ms_requested: Option<u32>,
    last_operation_id: Option<String>,
    last_phase: Option<String>,
    last_state: Option<String>,
    last_error: Option<String>,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmExecutionResult {
    success: bool,
    phase: String,
    stdout: String,
    stderr: String,
    error: Option<String>,
    blocker_key: Option<String>,
    line: usize,
    column: usize,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmWorkerExecutionResult {
    operation_id: String,
    success: bool,
    phase: String,
    state: String,
    stdout: String,
    stderr: String,
    error: Option<String>,
    blocker_key: Option<String>,
    line: usize,
    column: usize,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmRuntimeInfo {
    api_version: u32,
    pyrs_version: String,
    supports_parse_compile: bool,
    supports_execution: bool,
    execution_backend: String,
    execution_status: String,
    execution_blocker_count: usize,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmSnippetSupport {
    supported: bool,
    phase: String,
    error: Option<String>,
    line: usize,
    column: usize,
    imported_module_count: usize,
    blocker_count: usize,
    first_blocker_module: Option<String>,
    first_blocker_key: Option<String>,
    first_blocker_message: Option<String>,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmWorkerInfo {
    supported: bool,
    backend: String,
    state: String,
    interruption_model: String,
    lifecycle_supported: bool,
    execution_probe_enabled: bool,
    execute_supported: bool,
    timeout_configuration_supported: bool,
    timeout_enforcement_supported: bool,
    blocker_count: usize,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmWorkerTimeoutPolicy {
    default_timeout_ms: u32,
    min_timeout_ms: u32,
    max_timeout_ms: u32,
    configuration_supported: bool,
    recycle_on_timeout: bool,
    enforcement_supported: bool,
    unsupported_phase: String,
    unsupported_reason: Option<String>,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmWorkerTimeoutResult {
    success: bool,
    operation_id: String,
    phase: String,
    state: String,
    timeout_ms: u32,
    error: Option<String>,
    blocker_key: Option<String>,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmWorkerLifecycleResult {
    success: bool,
    operation_id: String,
    phase: String,
    state: String,
    error: Option<String>,
    blocker_key: Option<String>,
}

#[wasm_bindgen]
impl WasmExecutionResult {
    #[wasm_bindgen(getter)]
    pub fn success(&self) -> bool {
        self.success
    }

    #[wasm_bindgen(getter)]
    pub fn phase(&self) -> String {
        self.phase.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn stdout(&self) -> String {
        self.stdout.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn stderr(&self) -> String {
        self.stderr.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn blocker_key(&self) -> Option<String> {
        self.blocker_key.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn line(&self) -> usize {
        self.line
    }

    #[wasm_bindgen(getter)]
    pub fn column(&self) -> usize {
        self.column
    }
}

#[wasm_bindgen]
impl WasmWorkerExecutionResult {
    #[wasm_bindgen(getter)]
    pub fn operation_id(&self) -> String {
        self.operation_id.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn success(&self) -> bool {
        self.success
    }

    #[wasm_bindgen(getter)]
    pub fn phase(&self) -> String {
        self.phase.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn state(&self) -> String {
        self.state.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn stdout(&self) -> String {
        self.stdout.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn stderr(&self) -> String {
        self.stderr.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn blocker_key(&self) -> Option<String> {
        self.blocker_key.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn line(&self) -> usize {
        self.line
    }

    #[wasm_bindgen(getter)]
    pub fn column(&self) -> usize {
        self.column
    }
}

#[wasm_bindgen]
impl WasmCompileResult {
    #[wasm_bindgen(getter)]
    pub fn ok(&self) -> bool {
        self.ok
    }

    #[wasm_bindgen(getter)]
    pub fn phase(&self) -> String {
        self.phase.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn line(&self) -> usize {
        self.line
    }

    #[wasm_bindgen(getter)]
    pub fn column(&self) -> usize {
        self.column
    }
}

#[wasm_bindgen]
impl WasmRuntimeInfo {
    #[wasm_bindgen(getter)]
    pub fn api_version(&self) -> u32 {
        self.api_version
    }

    #[wasm_bindgen(getter)]
    pub fn pyrs_version(&self) -> String {
        self.pyrs_version.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn supports_execution(&self) -> bool {
        self.supports_execution
    }

    #[wasm_bindgen(getter)]
    pub fn supports_parse_compile(&self) -> bool {
        self.supports_parse_compile
    }

    #[wasm_bindgen(getter)]
    pub fn execution_backend(&self) -> String {
        self.execution_backend.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn execution_status(&self) -> String {
        self.execution_status.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn execution_blocker_count(&self) -> usize {
        self.execution_blocker_count
    }
}

#[wasm_bindgen]
impl WasmSnippetSupport {
    #[wasm_bindgen(getter)]
    pub fn supported(&self) -> bool {
        self.supported
    }

    #[wasm_bindgen(getter)]
    pub fn phase(&self) -> String {
        self.phase.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn line(&self) -> usize {
        self.line
    }

    #[wasm_bindgen(getter)]
    pub fn column(&self) -> usize {
        self.column
    }

    #[wasm_bindgen(getter)]
    pub fn imported_module_count(&self) -> usize {
        self.imported_module_count
    }

    #[wasm_bindgen(getter)]
    pub fn blocker_count(&self) -> usize {
        self.blocker_count
    }

    #[wasm_bindgen(getter)]
    pub fn first_blocker_module(&self) -> Option<String> {
        self.first_blocker_module.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn first_blocker_key(&self) -> Option<String> {
        self.first_blocker_key.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn first_blocker_message(&self) -> Option<String> {
        self.first_blocker_message.clone()
    }
}

#[wasm_bindgen]
impl WasmWorkerInfo {
    #[wasm_bindgen(getter)]
    pub fn supported(&self) -> bool {
        self.supported
    }

    #[wasm_bindgen(getter)]
    pub fn backend(&self) -> String {
        self.backend.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn state(&self) -> String {
        self.state.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn interruption_model(&self) -> String {
        self.interruption_model.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn lifecycle_supported(&self) -> bool {
        self.lifecycle_supported
    }

    #[wasm_bindgen(getter)]
    pub fn execution_probe_enabled(&self) -> bool {
        self.execution_probe_enabled
    }

    #[wasm_bindgen(getter)]
    pub fn execute_supported(&self) -> bool {
        self.execute_supported
    }

    #[wasm_bindgen(getter)]
    pub fn timeout_configuration_supported(&self) -> bool {
        self.timeout_configuration_supported
    }

    #[wasm_bindgen(getter)]
    pub fn timeout_enforcement_supported(&self) -> bool {
        self.timeout_enforcement_supported
    }

    #[wasm_bindgen(getter)]
    pub fn blocker_count(&self) -> usize {
        self.blocker_count
    }
}

#[wasm_bindgen]
impl WasmWorkerTimeoutPolicy {
    #[wasm_bindgen(getter)]
    pub fn default_timeout_ms(&self) -> u32 {
        self.default_timeout_ms
    }

    #[wasm_bindgen(getter)]
    pub fn min_timeout_ms(&self) -> u32 {
        self.min_timeout_ms
    }

    #[wasm_bindgen(getter)]
    pub fn max_timeout_ms(&self) -> u32 {
        self.max_timeout_ms
    }

    #[wasm_bindgen(getter)]
    pub fn configuration_supported(&self) -> bool {
        self.configuration_supported
    }

    #[wasm_bindgen(getter)]
    pub fn recycle_on_timeout(&self) -> bool {
        self.recycle_on_timeout
    }

    #[wasm_bindgen(getter)]
    pub fn enforcement_supported(&self) -> bool {
        self.enforcement_supported
    }

    #[wasm_bindgen(getter)]
    pub fn unsupported_phase(&self) -> String {
        self.unsupported_phase.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn unsupported_reason(&self) -> Option<String> {
        self.unsupported_reason.clone()
    }
}

#[wasm_bindgen]
impl WasmWorkerTimeoutResult {
    #[wasm_bindgen(getter)]
    pub fn success(&self) -> bool {
        self.success
    }

    #[wasm_bindgen(getter)]
    pub fn operation_id(&self) -> String {
        self.operation_id.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn phase(&self) -> String {
        self.phase.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn state(&self) -> String {
        self.state.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn blocker_key(&self) -> Option<String> {
        self.blocker_key.clone()
    }
}

#[wasm_bindgen]
impl WasmWorkerLifecycleResult {
    #[wasm_bindgen(getter)]
    pub fn success(&self) -> bool {
        self.success
    }

    #[wasm_bindgen(getter)]
    pub fn operation_id(&self) -> String {
        self.operation_id.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn phase(&self) -> String {
        self.phase.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn state(&self) -> String {
        self.state.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn blocker_key(&self) -> Option<String> {
        self.blocker_key.clone()
    }
}

#[wasm_bindgen]
impl WasmSession {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        init_wasm_runtime();
        Self {
            snippets_checked: 0,
            last_error: None,
        }
    }

    pub fn check_syntax(&mut self, source: &str) -> WasmSyntaxResult {
        let result = check_syntax_result(source);
        self.snippets_checked += 1;
        self.last_error = result.error.clone();
        result
    }

    pub fn check_compile(&mut self, source: &str) -> WasmCompileResult {
        let result = check_compile_result(source);
        self.snippets_checked += 1;
        self.last_error = result.error.clone();
        result
    }

    pub fn execute(&mut self, source: &str) -> WasmExecutionResult {
        let result = execute(source);
        self.snippets_checked += 1;
        self.last_error = result.error.clone();
        result
    }

    pub fn reset(&mut self) {
        self.snippets_checked = 0;
        self.last_error = None;
    }

    #[wasm_bindgen(getter)]
    pub fn snippets_checked(&self) -> usize {
        self.snippets_checked
    }

    #[wasm_bindgen(getter)]
    pub fn last_error(&self) -> Option<String> {
        self.last_error.clone()
    }

}

#[wasm_bindgen]
impl WasmReplSession {
    fn finish_execution_result(&mut self, result: WasmExecutionResult) -> WasmExecutionResult {
        self.last_error = result.error.clone();
        result
    }

    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        init_wasm_runtime();
        Self {
            inputs_executed: 0,
            last_error: None,
            repl_state: crate::repl_core::ReplCoreState::new(crate::repl_core::ReplProfile::WasmLean),
            #[cfg(feature = "wasm-vm-probe")]
            vm: new_wasm_repl_vm(),
        }
    }

    pub fn execute_input(&mut self, source: &str) -> WasmExecutionResult {
        self.inputs_executed += 1;
        debug_assert_eq!(
            self.repl_state.profile(),
            crate::repl_core::ReplProfile::WasmLean
        );

        let prepared = match self
            .repl_state
            .submit_line_prepare_module(source, WASM_REPL_FILENAME)
        {
            crate::repl_core::ReplLinePrepareResult::NeedMoreInput => {
                return self.finish_execution_result(execution_ok_result(String::new()));
            }
            crate::repl_core::ReplLinePrepareResult::ParseError { error, .. } => {
                let message = format_parse_error(&error);
                return self.finish_execution_result(execution_error_with_message(
                    WasmExecutionPhase::SyntaxError.key(),
                    message,
                    None,
                    error.line,
                    error.column,
                ));
            }
            crate::repl_core::ReplLinePrepareResult::CompileError { error, .. } => {
                let (message, line, column) = format_compile_error(&error);
                return self.finish_execution_result(execution_error_with_message(
                    WasmExecutionPhase::CompileError.key(),
                    message,
                    None,
                    line,
                    column,
                ));
            }
            crate::repl_core::ReplLinePrepareResult::Ready {
                source,
                module,
                code,
            } => (source, module, code),
        };
        let (ready_source, ready_module, compile_code) = prepared;
        #[cfg(not(feature = "wasm-vm-probe"))]
        let _ = (&ready_source, &compile_code);

        let host = WasmHost;
        let import_roots = collect_import_roots(&ready_module);
        if let Some(blocker) = snippet_blockers_from_import_roots(&import_roots, &host)
            .into_iter()
            .next()
        {
            return self.finish_execution_result(unsupported_execution_result(
                WasmExecutionPhase::UnsupportedExecution.key(),
                blocker.message,
                Some(blocker.blocker_key),
            ));
        }

        #[cfg(feature = "wasm-vm-probe")]
        {
            if !wasm_vm_runtime_enabled() {
                let message = wasm_execution_blocker_error(WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED)
                    .unwrap_or_else(|| "wasm execution backend is not wired yet".to_string());
                return self.finish_execution_result(unsupported_execution_result(
                    WasmExecutionPhase::UnsupportedExecution.key(),
                    message,
                    Some(WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED.to_string()),
                ));
            }
            let result = match crate::repl_core::run_ready_module(
                &mut self.vm,
                &ready_source,
                &ready_module,
                WASM_REPL_FILENAME,
                Some(&compile_code),
            ) {
                Ok(stdout) => execution_ok_result(stdout.unwrap_or_default()),
                Err(crate::repl_core::ReplExecutionError::Compile(err)) => {
                    let (message, line, column) = format_compile_error(&err);
                    execution_error_with_message(
                        WasmExecutionPhase::CompileError.key(),
                        message,
                        None,
                        line,
                        column,
                    )
                }
                Err(crate::repl_core::ReplExecutionError::Runtime(err)) => {
                    runtime_error_to_execution_result(err)
                }
            };
            return self.finish_execution_result(result);
        }

        #[cfg(not(feature = "wasm-vm-probe"))]
        {
            let message = wasm_execution_blocker_error(WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED)
                .unwrap_or_else(|| "wasm execution backend is not wired yet".to_string());
            return self.finish_execution_result(unsupported_execution_result(
                WasmExecutionPhase::UnsupportedExecution.key(),
                message,
                Some(WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED.to_string()),
            ));
        }
    }

    pub fn reset(&mut self) {
        self.inputs_executed = 0;
        self.last_error = None;
        self.repl_state.reset();
        #[cfg(feature = "wasm-vm-probe")]
        {
            self.vm = new_wasm_repl_vm();
        }
    }

    #[wasm_bindgen(getter)]
    pub fn inputs_executed(&self) -> usize {
        self.inputs_executed
    }

    #[wasm_bindgen(getter)]
    pub fn last_error(&self) -> Option<String> {
        self.last_error.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn continuation_prompt(&self) -> bool {
        matches!(
            self.repl_state.prompt_kind(),
            crate::repl_core::ReplPromptKind::Continuation
        )
    }
}

#[wasm_bindgen]
impl WasmWorkerSession {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        init_wasm_runtime();
        Self {
            starts_requested: 0,
            terminates_requested: 0,
            recycles_requested: 0,
            executes_requested: 0,
            timeout_updates_requested: 0,
            last_timeout_ms_requested: None,
            last_operation_id: None,
            last_phase: None,
            last_state: None,
            last_error: None,
        }
    }

    pub fn info(&self) -> WasmWorkerInfo {
        wasm_worker_info()
    }

    pub fn start(&mut self) -> WasmWorkerLifecycleResult {
        let result = wasm_worker_start();
        self.starts_requested += 1;
        self.last_operation_id = Some(result.operation_id.clone());
        self.last_phase = Some(result.phase.clone());
        self.last_state = Some(result.state.clone());
        self.last_error = result.error.clone();
        result
    }

    pub fn terminate(&mut self) -> WasmWorkerLifecycleResult {
        let result = wasm_worker_terminate();
        self.terminates_requested += 1;
        self.last_operation_id = Some(result.operation_id.clone());
        self.last_phase = Some(result.phase.clone());
        self.last_state = Some(result.state.clone());
        self.last_error = result.error.clone();
        result
    }

    pub fn recycle(&mut self) -> WasmWorkerLifecycleResult {
        let result = wasm_worker_recycle();
        self.recycles_requested += 1;
        self.last_operation_id = Some(result.operation_id.clone());
        self.last_phase = Some(result.phase.clone());
        self.last_state = Some(result.state.clone());
        self.last_error = result.error.clone();
        result
    }

    pub fn execute(&mut self, source: &str) -> WasmExecutionResult {
        let result = self.execute_with_operation(source);
        WasmExecutionResult {
            success: result.success,
            phase: result.phase,
            stdout: result.stdout,
            stderr: result.stderr,
            error: result.error,
            blocker_key: result.blocker_key,
            line: result.line,
            column: result.column,
        }
    }

    pub fn execute_with_operation(&mut self, source: &str) -> WasmWorkerExecutionResult {
        let result = wasm_worker_execute_with_operation(source);
        self.executes_requested += 1;
        self.last_operation_id = Some(result.operation_id.clone());
        self.last_phase = Some(result.phase.clone());
        self.last_state = Some(result.state.clone());
        self.last_error = result.error.clone();
        result
    }

    pub fn set_timeout_ms(&mut self, timeout_ms: u32) -> WasmWorkerTimeoutResult {
        let result = wasm_worker_set_timeout(timeout_ms);
        self.timeout_updates_requested += 1;
        self.last_timeout_ms_requested = Some(timeout_ms);
        self.last_operation_id = Some(result.operation_id.clone());
        self.last_phase = Some(result.phase.clone());
        self.last_state = Some(result.state.clone());
        self.last_error = result.error.clone();
        result
    }

    pub fn snapshot(&self) -> WasmWorkerSessionSnapshot {
        WasmWorkerSessionSnapshot {
            starts_requested: self.starts_requested,
            terminates_requested: self.terminates_requested,
            recycles_requested: self.recycles_requested,
            executes_requested: self.executes_requested,
            timeout_updates_requested: self.timeout_updates_requested,
            last_timeout_ms_requested: self.last_timeout_ms_requested,
            last_operation_id: self.last_operation_id.clone(),
            last_phase: self.last_phase.clone(),
            last_state: self.last_state.clone(),
            last_error: self.last_error.clone(),
        }
    }

    pub fn reset(&mut self) {
        self.starts_requested = 0;
        self.terminates_requested = 0;
        self.recycles_requested = 0;
        self.executes_requested = 0;
        self.timeout_updates_requested = 0;
        self.last_timeout_ms_requested = None;
        self.last_operation_id = None;
        self.last_phase = None;
        self.last_state = None;
        self.last_error = None;
    }

    #[wasm_bindgen(getter)]
    pub fn starts_requested(&self) -> usize {
        self.starts_requested
    }

    #[wasm_bindgen(getter)]
    pub fn terminates_requested(&self) -> usize {
        self.terminates_requested
    }

    #[wasm_bindgen(getter)]
    pub fn recycles_requested(&self) -> usize {
        self.recycles_requested
    }

    #[wasm_bindgen(getter)]
    pub fn executes_requested(&self) -> usize {
        self.executes_requested
    }

    #[wasm_bindgen(getter)]
    pub fn timeout_updates_requested(&self) -> usize {
        self.timeout_updates_requested
    }

    #[wasm_bindgen(getter)]
    pub fn last_timeout_ms_requested(&self) -> Option<u32> {
        self.last_timeout_ms_requested
    }

    #[wasm_bindgen(getter)]
    pub fn last_operation_id(&self) -> Option<String> {
        self.last_operation_id.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn last_phase(&self) -> Option<String> {
        self.last_phase.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn last_state(&self) -> Option<String> {
        self.last_state.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn last_error(&self) -> Option<String> {
        self.last_error.clone()
    }
}

#[wasm_bindgen]
impl WasmWorkerSessionSnapshot {
    #[wasm_bindgen(getter)]
    pub fn starts_requested(&self) -> usize {
        self.starts_requested
    }

    #[wasm_bindgen(getter)]
    pub fn terminates_requested(&self) -> usize {
        self.terminates_requested
    }

    #[wasm_bindgen(getter)]
    pub fn recycles_requested(&self) -> usize {
        self.recycles_requested
    }

    #[wasm_bindgen(getter)]
    pub fn executes_requested(&self) -> usize {
        self.executes_requested
    }

    #[wasm_bindgen(getter)]
    pub fn timeout_updates_requested(&self) -> usize {
        self.timeout_updates_requested
    }

    #[wasm_bindgen(getter)]
    pub fn last_timeout_ms_requested(&self) -> Option<u32> {
        self.last_timeout_ms_requested
    }

    #[wasm_bindgen(getter)]
    pub fn last_operation_id(&self) -> Option<String> {
        self.last_operation_id.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn last_phase(&self) -> Option<String> {
        self.last_phase.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn last_state(&self) -> Option<String> {
        self.last_state.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn last_error(&self) -> Option<String> {
        self.last_error.clone()
    }
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmCapabilityReport {
    filesystem_read: bool,
    filesystem_write: bool,
    environment_read: bool,
    process_args: bool,
    clock_time: bool,
    thread_sleep: bool,
    process_spawn: bool,
    dynamic_library_load: bool,
    interactive_terminal: bool,
    network_sockets: bool,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmExecutionBlocker {
    key: String,
    message: String,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmWorkerBlocker {
    key: String,
    message: String,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmModuleSupport {
    module: String,
    supported: bool,
    blocker_key: Option<String>,
    message: Option<String>,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmModulePolicyEntry {
    module: String,
    blocker_key: String,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmSnippetBlocker {
    module: String,
    blocker_key: String,
    message: String,
}

#[wasm_bindgen]
impl WasmCapabilityReport {
    #[wasm_bindgen(getter)]
    pub fn filesystem_read(&self) -> bool {
        self.filesystem_read
    }

    #[wasm_bindgen(getter)]
    pub fn filesystem_write(&self) -> bool {
        self.filesystem_write
    }

    #[wasm_bindgen(getter)]
    pub fn environment_read(&self) -> bool {
        self.environment_read
    }

    #[wasm_bindgen(getter)]
    pub fn process_args(&self) -> bool {
        self.process_args
    }

    #[wasm_bindgen(getter)]
    pub fn clock_time(&self) -> bool {
        self.clock_time
    }

    #[wasm_bindgen(getter)]
    pub fn thread_sleep(&self) -> bool {
        self.thread_sleep
    }

    #[wasm_bindgen(getter)]
    pub fn process_spawn(&self) -> bool {
        self.process_spawn
    }

    #[wasm_bindgen(getter)]
    pub fn dynamic_library_load(&self) -> bool {
        self.dynamic_library_load
    }

    #[wasm_bindgen(getter)]
    pub fn interactive_terminal(&self) -> bool {
        self.interactive_terminal
    }

    #[wasm_bindgen(getter)]
    pub fn network_sockets(&self) -> bool {
        self.network_sockets
    }
}

#[wasm_bindgen]
impl WasmExecutionBlocker {
    #[wasm_bindgen(getter)]
    pub fn key(&self) -> String {
        self.key.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }
}

#[wasm_bindgen]
impl WasmWorkerBlocker {
    #[wasm_bindgen(getter)]
    pub fn key(&self) -> String {
        self.key.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }
}

#[wasm_bindgen]
impl WasmModuleSupport {
    #[wasm_bindgen(getter)]
    pub fn module(&self) -> String {
        self.module.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn supported(&self) -> bool {
        self.supported
    }

    #[wasm_bindgen(getter)]
    pub fn blocker_key(&self) -> Option<String> {
        self.blocker_key.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn message(&self) -> Option<String> {
        self.message.clone()
    }
}

#[wasm_bindgen]
impl WasmModulePolicyEntry {
    #[wasm_bindgen(getter)]
    pub fn module(&self) -> String {
        self.module.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn blocker_key(&self) -> String {
        self.blocker_key.clone()
    }
}

#[wasm_bindgen]
impl WasmSnippetBlocker {
    #[wasm_bindgen(getter)]
    pub fn module(&self) -> String {
        self.module.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn blocker_key(&self) -> String {
        self.blocker_key.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }
}

/// Exposes explicit capability support for browser mode.
#[wasm_bindgen]
pub fn wasm_capabilities() -> WasmCapabilityReport {
    let host = WasmHost;
    WasmCapabilityReport {
        filesystem_read: host.supports(HostCapability::FilesystemRead),
        filesystem_write: host.supports(HostCapability::FilesystemWrite),
        environment_read: host.supports(HostCapability::EnvironmentRead),
        process_args: host.supports(HostCapability::ProcessArgs),
        clock_time: host.supports(HostCapability::ClockTime),
        thread_sleep: host.supports(HostCapability::ThreadSleep),
        process_spawn: host.supports(HostCapability::ProcessSpawn),
        dynamic_library_load: host.supports(HostCapability::DynamicLibraryLoad),
        interactive_terminal: host.supports(HostCapability::InteractiveTerminal),
        network_sockets: host.supports(HostCapability::NetworkSockets),
    }
}

/// Returns a stable unsupported-capability message for browser mode.
#[wasm_bindgen]
pub fn wasm_capability_error(capability_key: &str) -> Option<String> {
    let host = WasmHost;
    let capability = HostCapability::from_key(capability_key)?;
    host.unsupported_message(capability)
}

/// Returns the canonical capability keys exported by the wasm bridge.
#[wasm_bindgen]
pub fn wasm_capability_keys() -> Array {
    let keys = Array::new();
    for capability in HostCapability::all() {
        keys.push(&JsValue::from_str(capability.key()));
    }
    keys
}

/// Returns canonical phase keys for top-level execute() contract responses.
#[wasm_bindgen]
pub fn wasm_execution_phase_keys() -> Array {
    let keys = Array::new();
    for key in execution_phase_keys() {
        keys.push(&JsValue::from_str(key));
    }
    keys
}

/// Reports runtime contract status for browser clients.
#[wasm_bindgen]
pub fn wasm_runtime_info() -> WasmRuntimeInfo {
    let host = WasmHost;
    let blocker_count = execution_blocker_keys(&host).len();
    let supports_execution = wasm_vm_runtime_enabled();
    let execution_backend = if supports_execution {
        #[cfg(feature = "wasm-vm-probe")]
        {
            WASM_EXECUTION_BACKEND_VM_PROBE.to_string()
        }
        #[cfg(not(feature = "wasm-vm-probe"))]
        {
            WASM_EXECUTION_BACKEND_UNWIRED.to_string()
        }
    } else {
        WASM_EXECUTION_BACKEND_UNWIRED.to_string()
    };
    let execution_status = if supports_execution {
        "runtime_probe".to_string()
    } else {
        "syntax_compile_only".to_string()
    };
    WasmRuntimeInfo {
        api_version: wasm_api_version(),
        pyrs_version: pyrs_version(),
        supports_parse_compile: true,
        supports_execution,
        execution_backend,
        execution_status,
        execution_blocker_count: blocker_count,
    }
}

/// Returns canonical blocker keys for worker-mode execution.
#[wasm_bindgen]
pub fn wasm_worker_blocker_keys() -> Array {
    let keys = Array::new();
    for key in worker_blocker_keys() {
        keys.push(&JsValue::from_str(key));
    }
    keys
}

/// Returns a stable blocker message for wasm worker blockers.
#[wasm_bindgen]
pub fn wasm_worker_blocker_error(blocker_key: &str) -> Option<String> {
    if blocker_key == WASM_WORKER_BLOCKER_RUNTIME_UNWIRED {
        return Some("wasm worker runtime is not wired yet".to_string());
    }
    if blocker_key == WASM_WORKER_BLOCKER_RUNTIME_FAILED {
        return Some(
            "wasm worker runtime entered failed state; call wasm_worker_start() or wasm_worker_recycle()"
                .to_string(),
        );
    }
    wasm_execution_blocker_error(blocker_key)
}

/// Reports worker-runtime contract state for browser clients.
#[wasm_bindgen]
pub fn wasm_worker_info() -> WasmWorkerInfo {
    let blockers = worker_blocker_keys();
    let backend = if wasm_vm_runtime_enabled() {
        #[cfg(feature = "wasm-vm-probe")]
        {
            WASM_WORKER_BACKEND_VM_PROBE.to_string()
        }
        #[cfg(not(feature = "wasm-vm-probe"))]
        {
            WASM_WORKER_BACKEND_UNWIRED.to_string()
        }
    } else {
        WASM_WORKER_BACKEND_UNWIRED.to_string()
    };
    WasmWorkerInfo {
        supported: wasm_vm_runtime_enabled(),
        backend,
        state: current_worker_state_key(),
        interruption_model: WASM_WORKER_INTERRUPT_MODEL_RECYCLE.to_string(),
        lifecycle_supported: wasm_vm_runtime_enabled(),
        execution_probe_enabled: wasm_vm_runtime_enabled(),
        execute_supported: wasm_vm_runtime_enabled() && worker_runtime_ready(),
        timeout_configuration_supported: wasm_vm_runtime_enabled() && worker_runtime_ready(),
        timeout_enforcement_supported: wasm_vm_runtime_enabled() && worker_runtime_ready(),
        blocker_count: blockers.len(),
    }
}

/// Returns timeout/recycle policy contract for wasm worker execution.
#[wasm_bindgen]
pub fn wasm_worker_timeout_policy() -> WasmWorkerTimeoutPolicy {
    WasmWorkerTimeoutPolicy {
        default_timeout_ms: WASM_WORKER_TIMEOUT_DEFAULT_MS,
        min_timeout_ms: WASM_WORKER_TIMEOUT_MIN_MS,
        max_timeout_ms: WASM_WORKER_TIMEOUT_MAX_MS,
        configuration_supported: wasm_vm_runtime_enabled(),
        recycle_on_timeout: true,
        enforcement_supported: wasm_vm_runtime_enabled(),
        unsupported_phase: WasmWorkerTimeoutPhase::UnsupportedEnforcement
            .key()
            .to_string(),
        unsupported_reason: if wasm_vm_runtime_enabled() {
            None
        } else {
            Some(worker_timeout_policy_unsupported_reason())
        },
    }
}

/// Returns the currently configured worker timeout value in milliseconds.
#[wasm_bindgen]
pub fn wasm_worker_current_timeout_ms() -> u32 {
    current_worker_timeout_ms()
}

fn worker_timeout_result(
    success: bool,
    phase: &str,
    timeout_ms: u32,
    error: Option<String>,
    blocker_key: Option<String>,
) -> WasmWorkerTimeoutResult {
    WasmWorkerTimeoutResult {
        success,
        operation_id: next_worker_operation_id("set_timeout"),
        phase: phase.to_string(),
        state: current_worker_state_key(),
        timeout_ms,
        error,
        blocker_key,
    }
}

/// Applies a requested timeout policy update for worker execution.
///
/// Current milestone behavior:
/// - out-of-range values report `invalid_worker_timeout`,
/// - default builds report `unsupported_worker_timeout_enforcement` for in-range values,
/// - `wasm-vm-probe` builds report `worker_timeout_configured` for in-range values.
#[wasm_bindgen]
pub fn wasm_worker_set_timeout(timeout_ms: u32) -> WasmWorkerTimeoutResult {
    if !(WASM_WORKER_TIMEOUT_MIN_MS..=WASM_WORKER_TIMEOUT_MAX_MS).contains(&timeout_ms) {
        return worker_timeout_result(
            false,
            WasmWorkerTimeoutPhase::InvalidTimeout.key(),
            timeout_ms,
            Some(format!(
                "worker timeout must be between {} and {} ms",
                WASM_WORKER_TIMEOUT_MIN_MS, WASM_WORKER_TIMEOUT_MAX_MS
            )),
            None,
        );
    }

    if !worker_runtime_ready() {
        let state = current_worker_state();
        let blocker_key = worker_unavailable_blocker_key_for_state(state).to_string();
        let message = worker_unavailable_error_for_state(state);
        return worker_timeout_result(
            false,
            WasmWorkerTimeoutPhase::UnsupportedEnforcement.key(),
            timeout_ms,
            Some(message),
            Some(blocker_key),
        );
    }

    if wasm_vm_runtime_enabled() {
        #[cfg(feature = "wasm-vm-probe")]
        {
            set_current_worker_timeout_ms(timeout_ms);
            return worker_timeout_result(
                true,
                WASM_WORKER_TIMEOUT_CONFIGURED_PHASE,
                timeout_ms,
                None,
                None,
            );
        }
    }

    let blocker_key = WASM_WORKER_BLOCKER_RUNTIME_UNWIRED.to_string();
    let message = wasm_worker_blocker_error(WASM_WORKER_BLOCKER_RUNTIME_UNWIRED)
        .unwrap_or_else(|| "wasm worker runtime is not wired yet".to_string());
    worker_timeout_result(
        false,
        WasmWorkerTimeoutPhase::UnsupportedEnforcement.key(),
        timeout_ms,
        Some(message),
        Some(blocker_key),
    )
}

/// Returns key+message entries for known worker blockers.
#[wasm_bindgen]
pub fn wasm_worker_blockers() -> Array {
    let blockers = Array::new();
    for key in worker_blocker_keys() {
        let message = wasm_worker_blocker_error(key)
            .unwrap_or_else(|| "unknown wasm worker blocker".to_string());
        blockers.push(&JsValue::from(WasmWorkerBlocker {
            key: key.to_string(),
            message,
        }));
    }
    blockers
}

/// Returns canonical worker state keys for wasm worker runtime contracts.
#[wasm_bindgen]
pub fn wasm_worker_state_keys() -> Array {
    let keys = Array::new();
    for key in worker_state_keys() {
        keys.push(&JsValue::from_str(key));
    }
    keys
}

/// Returns canonical lifecycle phase keys for wasm worker runtime contracts.
#[wasm_bindgen]
pub fn wasm_worker_lifecycle_phase_keys() -> Array {
    let keys = Array::new();
    for key in worker_lifecycle_phase_keys() {
        keys.push(&JsValue::from_str(key));
    }
    keys
}

/// Returns canonical execute phase keys for wasm worker runtime contracts.
#[wasm_bindgen]
pub fn wasm_worker_execute_phase_keys() -> Array {
    let keys = Array::new();
    for key in worker_execute_phase_keys() {
        keys.push(&JsValue::from_str(key));
    }
    keys
}

/// Returns canonical timeout phase keys for wasm worker timeout contracts.
#[wasm_bindgen]
pub fn wasm_worker_timeout_phase_keys() -> Array {
    let keys = Array::new();
    for key in worker_timeout_phase_keys() {
        keys.push(&JsValue::from_str(key));
    }
    keys
}

fn worker_lifecycle_result(
    action: &str,
    success: bool,
    phase: &str,
    state: String,
    error: Option<String>,
    blocker_key: Option<String>,
) -> WasmWorkerLifecycleResult {
    WasmWorkerLifecycleResult {
        success,
        operation_id: next_worker_operation_id(action),
        phase: phase.to_string(),
        state,
        error,
        blocker_key,
    }
}

#[cfg(not(feature = "wasm-vm-probe"))]
fn worker_unwired_result(phase: WasmWorkerLifecyclePhase) -> WasmWorkerLifecycleResult {
    let action = match phase {
        WasmWorkerLifecyclePhase::UnsupportedStart => "start",
        WasmWorkerLifecyclePhase::UnsupportedTerminate => "terminate",
        WasmWorkerLifecyclePhase::UnsupportedRecycle => "recycle",
    };
    let blocker_key = WASM_WORKER_BLOCKER_RUNTIME_UNWIRED.to_string();
    let message = wasm_worker_blocker_error(WASM_WORKER_BLOCKER_RUNTIME_UNWIRED)
        .unwrap_or_else(|| "wasm worker runtime is not wired yet".to_string());
    reset_worker_timeout_ms();
    set_current_worker_state(WasmWorkerState::Unwired);
    worker_lifecycle_result(
        action,
        false,
        phase.key(),
        current_worker_state_key(),
        Some(message),
        Some(blocker_key),
    )
}

#[cfg(feature = "wasm-vm-probe")]
fn worker_vm_probe_lifecycle_result(
    action: &str,
    phase: &'static str,
    state: WasmWorkerState,
) -> WasmWorkerLifecycleResult {
    set_current_worker_state(state);
    worker_lifecycle_result(action, true, phase, state.key().to_string(), None, None)
}

/// Starts worker runtime execution.
///
/// Current milestone behavior:
/// - `wasm-vm-probe`: returns `phase = "worker_started"` and `state = "ready"`,
/// - default builds: unsupported lifecycle result with unwired blocker.
#[cfg(feature = "wasm-vm-probe")]
#[wasm_bindgen]
pub fn wasm_worker_start() -> WasmWorkerLifecycleResult {
    set_current_worker_state(WasmWorkerState::Starting);
    reset_worker_timeout_ms();
    reset_worker_vm();
    worker_vm_probe_lifecycle_result(
        "start",
        WASM_WORKER_LIFECYCLE_PHASE_STARTED,
        WasmWorkerState::Ready,
    )
}

/// Starts worker runtime execution.
///
/// Current milestone behavior:
/// - returns unsupported lifecycle result with unwired blocker.
#[cfg(not(feature = "wasm-vm-probe"))]
#[wasm_bindgen]
pub fn wasm_worker_start() -> WasmWorkerLifecycleResult {
    worker_unwired_result(WasmWorkerLifecyclePhase::UnsupportedStart)
}

/// Terminates worker runtime execution.
///
/// Current milestone behavior:
/// - `wasm-vm-probe`: returns `phase = "worker_terminated"` and `state = "unwired"`,
/// - default builds: unsupported lifecycle result with unwired blocker.
#[cfg(feature = "wasm-vm-probe")]
#[wasm_bindgen]
pub fn wasm_worker_terminate() -> WasmWorkerLifecycleResult {
    set_current_worker_state(WasmWorkerState::Terminating);
    reset_worker_timeout_ms();
    clear_worker_vm();
    worker_vm_probe_lifecycle_result(
        "terminate",
        WASM_WORKER_LIFECYCLE_PHASE_TERMINATED,
        WasmWorkerState::Unwired,
    )
}

/// Terminates worker runtime execution.
///
/// Current milestone behavior:
/// - returns unsupported lifecycle result with unwired blocker.
#[cfg(not(feature = "wasm-vm-probe"))]
#[wasm_bindgen]
pub fn wasm_worker_terminate() -> WasmWorkerLifecycleResult {
    worker_unwired_result(WasmWorkerLifecyclePhase::UnsupportedTerminate)
}

/// Recycles worker runtime execution state.
///
/// Current milestone behavior:
/// - `wasm-vm-probe`: returns `phase = "worker_recycled"` and `state = "ready"`,
/// - default builds: unsupported lifecycle result with unwired blocker.
#[cfg(feature = "wasm-vm-probe")]
#[wasm_bindgen]
pub fn wasm_worker_recycle() -> WasmWorkerLifecycleResult {
    set_current_worker_state(WasmWorkerState::Starting);
    reset_worker_timeout_ms();
    reset_worker_vm();
    worker_vm_probe_lifecycle_result(
        "recycle",
        WASM_WORKER_LIFECYCLE_PHASE_RECYCLED,
        WasmWorkerState::Ready,
    )
}

/// Recycles worker runtime execution state.
///
/// Current milestone behavior:
/// - returns unsupported lifecycle result with unwired blocker.
#[cfg(not(feature = "wasm-vm-probe"))]
#[wasm_bindgen]
pub fn wasm_worker_recycle() -> WasmWorkerLifecycleResult {
    worker_unwired_result(WasmWorkerLifecyclePhase::UnsupportedRecycle)
}

/// Forces wasm worker state into `failed` for vm-probe contract tests.
///
/// This hook is intentionally vm-probe-only so default wasm builds do not
/// expose native-runtime lifecycle simulation controls.
#[cfg(feature = "wasm-vm-probe")]
pub fn wasm_worker_force_failed_state_for_tests() -> WasmWorkerLifecycleResult {
    clear_worker_vm();
    set_current_worker_state(WasmWorkerState::Failed);
    let blocker_key = WASM_WORKER_BLOCKER_RUNTIME_FAILED.to_string();
    let message = wasm_worker_blocker_error(WASM_WORKER_BLOCKER_RUNTIME_FAILED)
        .unwrap_or_else(|| "wasm worker runtime entered failed state".to_string());
    WasmWorkerLifecycleResult {
        success: true,
        operation_id: next_worker_operation_id("force_failed"),
        phase: "worker_failed_forced".to_string(),
        state: current_worker_state_key(),
        error: Some(message),
        blocker_key: Some(blocker_key),
    }
}

/// Executes a snippet through the wasm worker contract.
///
/// Current milestone behavior:
/// - parse-invalid input returns `phase = "syntax_error"`,
/// - parse-valid but compile-invalid input returns `phase = "compile_error"`,
/// - parse+compile-valid snippets that import known blocked modules return
///   `phase = "unsupported_worker_execution"` with capability-specific blocker keys,
/// - default builds return `phase = "unsupported_worker_execution"` for remaining
///   parse+compile-valid snippets,
/// - `wasm-vm-probe` builds execute remaining snippets through VM and return
///   `phase = "ok"` or `phase = "runtime_error"`.
#[wasm_bindgen]
pub fn wasm_worker_execute(source: &str) -> WasmExecutionResult {
    execute_snippet_with_contract(source, WasmExecutionContractMode::Worker)
}

/// Executes a snippet through the worker contract with an operation correlation id.
#[wasm_bindgen]
pub fn wasm_worker_execute_with_operation(source: &str) -> WasmWorkerExecutionResult {
    let result = wasm_worker_execute(source);
    WasmWorkerExecutionResult {
        operation_id: next_worker_operation_id("execute"),
        success: result.success,
        phase: result.phase,
        state: current_worker_state_key(),
        stdout: result.stdout,
        stderr: result.stderr,
        error: result.error,
        blocker_key: result.blocker_key,
        line: result.line,
        column: result.column,
    }
}

/// Returns canonical blocker keys that currently prevent wasm execution.
#[wasm_bindgen]
pub fn wasm_execution_blocker_keys() -> Array {
    let host = WasmHost;
    let keys = Array::new();
    for key in execution_blocker_keys(&host) {
        keys.push(&JsValue::from_str(key));
    }
    keys
}

/// Returns key+message entries for known execution blockers.
#[wasm_bindgen]
pub fn wasm_execution_blockers() -> Array {
    let host = WasmHost;
    let blockers = Array::new();
    for key in execution_blocker_keys(&host) {
        let message = wasm_execution_blocker_error(key)
            .unwrap_or_else(|| "unknown wasm execution blocker".to_string());
        blockers.push(&JsValue::from(WasmExecutionBlocker {
            key: key.to_string(),
            message,
        }));
    }
    blockers
}

/// Reports whether a module is known to require an unsupported wasm capability.
#[wasm_bindgen]
pub fn wasm_module_support(module_name: &str) -> WasmModuleSupport {
    let host = WasmHost;
    let normalized = module_name.trim();
    let blocker_key =
        module_blocker_key(normalized).and_then(|key| match HostCapability::from_key(key) {
            Some(capability) if host.supports(capability) => None,
            _ => Some(key),
        });
    let message = blocker_key.and_then(wasm_execution_blocker_error);
    WasmModuleSupport {
        module: normalized.to_string(),
        supported: blocker_key.is_none(),
        blocker_key: blocker_key.map(str::to_string),
        message,
    }
}

/// Returns module-level blocker policy entries for browser-mode preflight UX.
#[wasm_bindgen]
pub fn wasm_module_policy_entries() -> Array {
    let entries = Array::new();
    for (module, blocker_key) in WASM_MODULE_BLOCKER_POLICY {
        entries.push(&JsValue::from(WasmModulePolicyEntry {
            module: module.to_string(),
            blocker_key: blocker_key.to_string(),
        }));
    }
    entries
}

fn root_module_name(raw: &str) -> Option<&str> {
    let root = raw.split('.').next()?.trim();
    if root.is_empty() { None } else { Some(root) }
}

fn push_import_root(raw: &str, seen: &mut HashSet<String>, roots: &mut Vec<String>) {
    let Some(root) = root_module_name(raw) else {
        return;
    };
    if seen.insert(root.to_string()) {
        roots.push(root.to_string());
    }
}

fn collect_import_roots_from_stmts(
    stmts: &[crate::ast::Stmt],
    seen: &mut HashSet<String>,
    roots: &mut Vec<String>,
) {
    for stmt in stmts {
        use crate::ast::StmtKind;
        match &stmt.node {
            StmtKind::Import { names } => {
                for alias in names {
                    push_import_root(&alias.name, seen, roots);
                }
            }
            StmtKind::ImportFrom { module, .. } => {
                if let Some(module) = module {
                    push_import_root(module, seen, roots);
                }
            }
            StmtKind::If { body, orelse, .. }
            | StmtKind::While { body, orelse, .. }
            | StmtKind::For { body, orelse, .. } => {
                collect_import_roots_from_stmts(body, seen, roots);
                collect_import_roots_from_stmts(orelse, seen, roots);
            }
            StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                collect_import_roots_from_stmts(body, seen, roots);
                for handler in handlers {
                    collect_import_roots_from_stmts(&handler.body, seen, roots);
                }
                collect_import_roots_from_stmts(orelse, seen, roots);
                collect_import_roots_from_stmts(finalbody, seen, roots);
            }
            StmtKind::With { body, .. }
            | StmtKind::FunctionDef { body, .. }
            | StmtKind::ClassDef { body, .. } => {
                collect_import_roots_from_stmts(body, seen, roots);
            }
            StmtKind::Match { cases, .. } => {
                for case in cases {
                    collect_import_roots_from_stmts(&case.body, seen, roots);
                }
            }
            StmtKind::Decorated { stmt, .. } => {
                collect_import_roots_from_stmts(std::slice::from_ref(stmt.as_ref()), seen, roots);
            }
            _ => {}
        }
    }
}

fn collect_import_roots(module: &crate::ast::Module) -> Vec<String> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    collect_import_roots_from_stmts(&module.body, &mut seen, &mut roots);
    roots
}

struct ParsedCompiledSnippet {
    module: crate::ast::Module,
    #[cfg(feature = "wasm-vm-probe")]
    code: crate::bytecode::CodeObject,
}

fn parse_and_compile_snippet(source: &str) -> Result<ParsedCompiledSnippet, WasmCompileResult> {
    match crate::repl_core::parse_and_compile_module_source(source, "<wasm>") {
        Ok((module, code)) => {
            #[cfg(feature = "wasm-vm-probe")]
            {
                Ok(ParsedCompiledSnippet { module, code })
            }
            #[cfg(not(feature = "wasm-vm-probe"))]
            {
                let _ = code;
                Ok(ParsedCompiledSnippet { module })
            }
        }
        Err(crate::repl_core::ReplSourcePrepareError::Parse(err)) => Err(WasmCompileResult {
            ok: false,
            phase: "syntax_error".to_string(),
            error: Some(format_parse_error(&err)),
            line: err.line,
            column: err.column,
        }),
        Err(crate::repl_core::ReplSourcePrepareError::Compile(err)) => {
            let (message, line, column) = format_compile_error(&err);
            Err(WasmCompileResult {
                ok: false,
                phase: "compile_error".to_string(),
                error: Some(message),
                line,
                column,
            })
        }
    }
}

fn compile_failure_to_execution_result(
    compile: WasmCompileResult,
    syntax_phase_key: &str,
    compile_phase_key: &str,
    fallback_error: &str,
) -> WasmExecutionResult {
    let error = compile.error;
    let stderr = error.clone().unwrap_or_else(|| fallback_error.to_string());
    let phase = if compile.phase == "syntax_error" {
        syntax_phase_key.to_string()
    } else {
        compile_phase_key.to_string()
    };
    execution_failure_result(
        &phase,
        stderr,
        error,
        None,
        compile.line,
        compile.column,
    )
}

fn execution_failure_result(
    phase_key: &str,
    stderr: String,
    error: Option<String>,
    blocker_key: Option<String>,
    line: usize,
    column: usize,
) -> WasmExecutionResult {
    WasmExecutionResult {
        success: false,
        phase: phase_key.to_string(),
        stdout: String::new(),
        stderr,
        error,
        blocker_key,
        line,
        column,
    }
}

fn execution_ok_result(stdout: String) -> WasmExecutionResult {
    WasmExecutionResult {
        success: true,
        phase: WASM_EXECUTION_PHASE_OK.to_string(),
        stdout,
        stderr: String::new(),
        error: None,
        blocker_key: None,
        line: 0,
        column: 0,
    }
}

fn execution_error_with_message(
    phase_key: &str,
    message: String,
    blocker_key: Option<String>,
    line: usize,
    column: usize,
) -> WasmExecutionResult {
    execution_failure_result(
        phase_key,
        message.clone(),
        Some(message),
        blocker_key,
        line,
        column,
    )
}

fn unsupported_execution_result(
    phase_key: &str,
    message: String,
    blocker_key: Option<String>,
) -> WasmExecutionResult {
    execution_error_with_message(phase_key, message, blocker_key, 0, 0)
}

#[derive(Clone, Copy)]
enum WasmExecutionContractMode {
    TopLevel,
    Worker,
}

impl WasmExecutionContractMode {
    fn syntax_phase_key(self) -> &'static str {
        match self {
            WasmExecutionContractMode::TopLevel => WasmExecutionPhase::SyntaxError.key(),
            WasmExecutionContractMode::Worker => WasmWorkerExecutePhase::SyntaxError.key(),
        }
    }

    fn compile_phase_key(self) -> &'static str {
        match self {
            WasmExecutionContractMode::TopLevel => WasmExecutionPhase::CompileError.key(),
            WasmExecutionContractMode::Worker => WasmWorkerExecutePhase::CompileError.key(),
        }
    }

    fn unsupported_phase_key(self) -> &'static str {
        match self {
            WasmExecutionContractMode::TopLevel => WasmExecutionPhase::UnsupportedExecution.key(),
            WasmExecutionContractMode::Worker => WasmWorkerExecutePhase::UnsupportedExecution.key(),
        }
    }

    fn compile_fallback_error(self) -> &'static str {
        match self {
            WasmExecutionContractMode::TopLevel => "parse/compile check failed",
            WasmExecutionContractMode::Worker => "worker parse/compile check failed",
        }
    }

    fn unwired_blocker_key(self) -> &'static str {
        match self {
            WasmExecutionContractMode::TopLevel => WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED,
            WasmExecutionContractMode::Worker => WASM_WORKER_BLOCKER_RUNTIME_UNWIRED,
        }
    }

    fn unwired_error_message(self) -> String {
        match self {
            WasmExecutionContractMode::TopLevel => {
                wasm_execution_blocker_error(self.unwired_blocker_key())
                    .unwrap_or_else(|| "wasm execution backend is not wired yet".to_string())
            }
            WasmExecutionContractMode::Worker => {
                wasm_worker_blocker_error(self.unwired_blocker_key())
                    .unwrap_or_else(|| "wasm worker runtime is not wired yet".to_string())
            }
        }
    }

    fn requires_worker_ready_state(self) -> bool {
        matches!(self, WasmExecutionContractMode::Worker)
    }
}

fn execute_snippet_with_contract(
    source: &str,
    contract: WasmExecutionContractMode,
) -> WasmExecutionResult {
    let host = WasmHost;
    let parsed = match parse_and_compile_snippet(source) {
        Ok(parsed) => parsed,
        Err(compile) => {
            return compile_failure_to_execution_result(
                compile,
                contract.syntax_phase_key(),
                contract.compile_phase_key(),
                contract.compile_fallback_error(),
            );
        }
    };

    let import_roots = collect_import_roots(&parsed.module);
    let first_blocker = snippet_blockers_from_import_roots(&import_roots, &host)
        .into_iter()
        .next();
    if let Some(blocker) = first_blocker {
        return unsupported_execution_result(
            contract.unsupported_phase_key(),
            blocker.message,
            Some(blocker.blocker_key),
        );
    }

    if contract.requires_worker_ready_state() && !worker_runtime_ready() {
        let state = current_worker_state();
        let blocker_key = worker_unavailable_blocker_key_for_state(state).to_string();
        let message = worker_unavailable_error_for_state(state);
        return unsupported_execution_result(
            contract.unsupported_phase_key(),
            message,
            Some(blocker_key),
        );
    }

    if wasm_vm_runtime_enabled() {
        #[cfg(feature = "wasm-vm-probe")]
        {
            return if contract.requires_worker_ready_state() {
                execute_compiled_snippet_with_worker_vm(&parsed.code)
            } else {
                execute_compiled_snippet_with_fresh_vm(&parsed.code)
            };
        }
    }

    let message = contract.unwired_error_message();
    unsupported_execution_result(
        contract.unsupported_phase_key(),
        message,
        Some(contract.unwired_blocker_key().to_string()),
    )
}

fn snippet_blockers_from_import_roots(
    import_roots: &[String],
    host: &dyn VmHost,
) -> Vec<WasmSnippetBlocker> {
    let mut blockers = Vec::new();
    for module in import_roots {
        let Some(blocker_key) = module_blocker_key(module) else {
            continue;
        };
        let blocked = match HostCapability::from_key(blocker_key) {
            Some(capability) => !host.supports(capability),
            None => true,
        };
        if !blocked {
            continue;
        }
        let message = wasm_execution_blocker_error(blocker_key).unwrap_or_else(|| {
            format!(
                "unsupported blocker '{}' for module '{}'",
                blocker_key, module
            )
        });
        blockers.push(WasmSnippetBlocker {
            module: module.clone(),
            blocker_key: blocker_key.to_string(),
            message,
        });
    }
    blockers
}

/// Preflight analysis for snippet viability in wasm mode.
#[wasm_bindgen]
pub fn wasm_snippet_support(source: &str) -> WasmSnippetSupport {
    init_wasm_runtime();
    let host = WasmHost;
    let parsed = match parse_and_compile_snippet(source) {
        Ok(parsed) => parsed,
        Err(result) => {
            return WasmSnippetSupport {
                supported: false,
                phase: result.phase,
                error: result.error,
                line: result.line,
                column: result.column,
                imported_module_count: 0,
                blocker_count: 0,
                first_blocker_module: None,
                first_blocker_key: None,
                first_blocker_message: None,
            };
        }
    };
    let import_roots = collect_import_roots(&parsed.module);
    let blockers = snippet_blockers_from_import_roots(&import_roots, &host);
    let first = blockers.first();
    if let Some(first) = first {
        return WasmSnippetSupport {
            supported: false,
            phase: "blocked_capability".to_string(),
            error: Some(format!(
                "snippet requires unsupported capability '{}' via module '{}'",
                first.blocker_key, first.module
            )),
            line: 0,
            column: 0,
            imported_module_count: import_roots.len(),
            blocker_count: blockers.len(),
            first_blocker_module: Some(first.module.clone()),
            first_blocker_key: Some(first.blocker_key.clone()),
            first_blocker_message: Some(first.message.clone()),
        };
    }
    WasmSnippetSupport {
        supported: true,
        phase: "supported".to_string(),
        error: None,
        line: 0,
        column: 0,
        imported_module_count: import_roots.len(),
        blocker_count: 0,
        first_blocker_module: None,
        first_blocker_key: None,
        first_blocker_message: None,
    }
}

/// Returns snippet blockers detected from import preflight analysis.
#[wasm_bindgen]
pub fn wasm_snippet_blockers(source: &str) -> Array {
    init_wasm_runtime();
    let host = WasmHost;
    let Ok(parsed) = parse_and_compile_snippet(source) else {
        return Array::new();
    };
    let import_roots = collect_import_roots(&parsed.module);
    let blockers = snippet_blockers_from_import_roots(&import_roots, &host);
    let result = Array::new();
    for blocker in blockers {
        result.push(&JsValue::from(blocker));
    }
    result
}

/// Returns canonical root imports detected from parse+compile-valid snippet source.
#[wasm_bindgen]
pub fn wasm_snippet_import_roots(source: &str) -> Array {
    init_wasm_runtime();
    let Ok(parsed) = parse_and_compile_snippet(source) else {
        return Array::new();
    };
    let roots = collect_import_roots(&parsed.module);
    let result = Array::new();
    for root in roots {
        result.push(&JsValue::from_str(&root));
    }
    result
}

/// Returns a stable blocker message for wasm execution blockers.
#[wasm_bindgen]
pub fn wasm_execution_blocker_error(blocker_key: &str) -> Option<String> {
    if blocker_key == WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED {
        return Some("wasm execution backend is not wired yet".to_string());
    }
    if blocker_key == WASM_EXECUTION_BLOCKER_VM_RUNTIME_UNAVAILABLE {
        return Some("vm runtime is not available on wasm target yet".to_string());
    }
    wasm_capability_error(blocker_key)
}

#[cfg(feature = "wasm-vm-probe")]
fn execute_compiled_snippet_with_vm(
    vm: &mut Vm,
    code: &crate::bytecode::CodeObject,
    timeout_ms: Option<u32>,
) -> (WasmExecutionResult, bool, bool) {
    let execution = if let Some(timeout_ms) = timeout_ms {
        vm.execute_with_timeout_ms(code, timeout_ms)
    } else {
        vm.execute(code)
    };
    match execution {
        Ok(_) => (execution_ok_result(String::new()), false, false),
        Err(err) => {
            let timeout_exceeded = runtime_error_is_execution_timeout(&err);
            let internal_failure = err.exception.is_none() && !timeout_exceeded;
            (
                runtime_error_to_execution_result(err),
                internal_failure,
                timeout_exceeded,
            )
        }
    }
}

#[cfg(feature = "wasm-vm-probe")]
fn execute_compiled_snippet_with_fresh_vm(
    code: &crate::bytecode::CodeObject,
) -> WasmExecutionResult {
    let mut vm = Vm::new_with_host(Arc::new(WasmHost));
    let (result, _internal_failure, _timeout_exceeded) =
        execute_compiled_snippet_with_vm(&mut vm, code, None);
    result
}

#[cfg(feature = "wasm-vm-probe")]
fn execute_compiled_snippet_with_worker_vm(
    code: &crate::bytecode::CodeObject,
) -> WasmExecutionResult {
    ensure_worker_vm_initialized();
    set_current_worker_state(WasmWorkerState::Busy);
    let timeout_ms = current_worker_timeout_ms();
    let (result, internal_failure, timeout_exceeded) = WASM_WORKER_VM.with(|slot| {
        let mut worker_vm = slot.borrow_mut();
        let Some(vm) = worker_vm.as_mut() else {
            let error = worker_unavailable_error_for_state(WasmWorkerState::Failed);
            return (
                execution_error_with_message(
                    WasmWorkerExecutePhase::UnsupportedExecution.key(),
                    error,
                    Some(WASM_WORKER_BLOCKER_RUNTIME_FAILED.to_string()),
                    0,
                    0,
                ),
                true,
                false,
            );
        };
        execute_compiled_snippet_with_vm(vm, code, Some(timeout_ms))
    });
    if internal_failure {
        clear_worker_vm();
        set_current_worker_state(WasmWorkerState::Failed);
    } else if timeout_exceeded {
        reset_worker_timeout_ms();
        reset_worker_vm();
        set_current_worker_state(WasmWorkerState::Ready);
    } else if current_worker_state() == WasmWorkerState::Busy {
        set_current_worker_state(WasmWorkerState::Ready);
    }
    result
}

#[cfg(feature = "wasm-vm-probe")]
fn runtime_error_to_execution_result(err: crate::runtime::RuntimeError) -> WasmExecutionResult {
    let (line, column) = runtime_error_line_column(&err);
    let message = err.message.clone();
    execution_error_with_message(
        WASM_EXECUTION_PHASE_RUNTIME_ERROR,
        message,
        None,
        line,
        column,
    )
}

#[cfg(feature = "wasm-vm-probe")]
fn runtime_error_is_execution_timeout(err: &crate::runtime::RuntimeError) -> bool {
    err.message.starts_with(WASM_WORKER_TIMEOUT_EXCEEDED_PREFIX)
}

#[cfg(feature = "wasm-vm-probe")]
fn runtime_error_line_column(err: &crate::runtime::RuntimeError) -> (usize, usize) {
    if let Some(exception) = err.exception.as_ref() {
        for frame in &exception.traceback_frames {
            if frame.line > 0 {
                return (frame.line, frame.column);
            }
        }
    }
    (0, 0)
}

/// Executes a snippet using the current wasm bridge contract.
///
/// Current milestone behavior:
/// - parse-invalid input returns `phase = "syntax_error"`
/// - parse-valid but compile-invalid input returns `phase = "compile_error"`
/// - parse+compile-valid snippets that import known blocked modules return
///   `phase = "unsupported_execution"` with capability-specific blocker keys,
/// - default wasm builds return `phase = "unsupported_execution"` for remaining
///   parse+compile-valid snippets,
/// - `wasm-vm-probe` builds execute remaining snippets through VM and return
///   `phase = "ok"` or `phase = "runtime_error"`.
#[wasm_bindgen]
pub fn execute(source: &str) -> WasmExecutionResult {
    execute_snippet_with_contract(source, WasmExecutionContractMode::TopLevel)
}

fn format_parse_error(err: &crate::parser::ParseError) -> String {
    format!("{} (line {}, column {})", err.message, err.line, err.column)
}

fn format_compile_error(err: &crate::compiler::CompileError) -> (String, usize, usize) {
    match err.span {
        Some(span) => (
            format!(
                "{} (line {}, column {})",
                err.message, span.line, span.column
            ),
            span.line,
            span.column,
        ),
        None => (err.message.clone(), 0, 0),
    }
}

/// Parser-backed syntax check with structured diagnostics for web clients.
#[wasm_bindgen]
pub fn check_syntax_result(source: &str) -> WasmSyntaxResult {
    init_wasm_runtime();
    match crate::parser::parse_module(source) {
        Ok(_) => WasmSyntaxResult {
            ok: true,
            error: None,
            line: 0,
            column: 0,
        },
        Err(err) => WasmSyntaxResult {
            ok: false,
            error: Some(format_parse_error(&err)),
            line: err.line,
            column: err.column,
        },
    }
}

/// Parses module source and reports syntax diagnostics with parser-native text.
#[wasm_bindgen]
pub fn check_syntax(source: &str) -> Result<(), JsValue> {
    let result = check_syntax_result(source);
    if result.ok {
        Ok(())
    } else {
        Err(JsValue::from_str(
            &result
                .error
                .unwrap_or_else(|| "syntax check failed".to_string()),
        ))
    }
}

/// Parse+compile validation with structured diagnostics for web clients.
#[wasm_bindgen]
pub fn check_compile_result(source: &str) -> WasmCompileResult {
    init_wasm_runtime();
    match parse_and_compile_snippet(source) {
        Ok(_) => WasmCompileResult {
            ok: true,
            phase: "ok".to_string(),
            error: None,
            line: 0,
            column: 0,
        },
        Err(result) => result,
    }
}

/// Parse+compile gate with JS error for web clients.
#[wasm_bindgen]
pub fn check_compile(source: &str) -> Result<(), JsValue> {
    let result = check_compile_result(source);
    if result.ok {
        Ok(())
    } else {
        Err(JsValue::from_str(
            &result
                .error
                .unwrap_or_else(|| "compile check failed".to_string()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WasmExecutionPhase, WasmReplSession, check_syntax, execute, execution_phase_keys,
        pyrs_version, wasm_worker_current_timeout_ms, wasm_worker_execute, wasm_worker_info,
        wasm_worker_recycle, wasm_worker_set_timeout, wasm_worker_start, wasm_worker_terminate,
    };
    #[cfg(feature = "wasm-vm-probe")]
    use super::{
        WasmWorkerState, clear_worker_vm, set_current_worker_state,
        wasm_worker_execute_with_operation,
    };
    #[cfg(feature = "wasm-vm-probe")]
    use std::collections::HashSet;

    fn vm_probe_enabled() -> bool {
        cfg!(feature = "wasm-vm-probe")
    }

    #[test]
    fn wasm_exports_version() {
        assert!(!pyrs_version().is_empty());
    }

    #[test]
    fn wasm_syntax_check_accepts_valid_module() {
        assert!(check_syntax("x = 1\n").is_ok());
    }

    #[test]
    fn wasm_syntax_check_rejects_invalid_module() {
        assert!(check_syntax("def broken(:\n").is_err());
    }

    #[test]
    fn wasm_execution_phase_keys_are_stable_native() {
        let mut expected = vec![
            WasmExecutionPhase::SyntaxError.key(),
            WasmExecutionPhase::CompileError.key(),
            WasmExecutionPhase::UnsupportedExecution.key(),
        ];
        if vm_probe_enabled() {
            expected.push("ok");
            expected.push("runtime_error");
        }
        assert_eq!(execution_phase_keys(), expected);
    }

    #[test]
    fn wasm_execute_unwired_sets_backend_blocker_key() {
        let result = execute("x = 1\n");
        if vm_probe_enabled() {
            assert_eq!(result.phase(), "ok".to_string());
            assert!(result.blocker_key().is_none());
        } else {
            assert_eq!(result.phase(), "unsupported_execution".to_string());
            assert_eq!(
                result.blocker_key(),
                Some("execution_backend_unwired".to_string())
            );
        }
    }

    #[test]
    fn wasm_execute_blocked_import_sets_capability_blocker_key() {
        let result = execute("import socket\n");
        assert_eq!(result.phase(), "unsupported_execution".to_string());
        assert_eq!(result.blocker_key(), Some("network_sockets".to_string()));
    }

    #[test]
    fn wasm_execute_parse_compile_failures_have_no_blocker_key() {
        let compile_error = execute("return 1\n");
        assert_eq!(compile_error.phase(), "compile_error".to_string());
        assert!(compile_error.blocker_key().is_none());
        assert!(compile_error.line() > 0);
        assert!(compile_error.column() > 0);

        let syntax_error = execute("def broken(:\n");
        assert_eq!(syntax_error.phase(), "syntax_error".to_string());
        assert!(syntax_error.blocker_key().is_none());
        assert!(syntax_error.line() > 0);
        assert!(syntax_error.column() > 0);
    }

    #[test]
    fn wasm_repl_session_mode_contract_is_stable() {
        let mut session = WasmReplSession::new();
        assert!(!session.continuation_prompt());
        let result = session.execute_input("x = 1\n");
        if vm_probe_enabled() {
            assert_eq!(result.phase(), "ok".to_string());
            assert!(result.blocker_key().is_none());
        } else {
            assert_eq!(result.phase(), "unsupported_execution".to_string());
            assert_eq!(
                result.blocker_key(),
                Some("execution_backend_unwired".to_string())
            );
        }
        assert_eq!(session.inputs_executed(), 1);
        assert!(!session.continuation_prompt());
    }

    #[test]
    fn wasm_repl_session_incomplete_input_returns_ok_without_errors() {
        let mut session = WasmReplSession::new();
        assert!(!session.continuation_prompt());
        let header = session.execute_input("if True:");
        assert!(header.success());
        assert_eq!(header.phase(), "ok".to_string());
        assert!(header.stdout().is_empty());
        assert!(header.stderr().is_empty());
        assert!(header.error().is_none());
        assert!(header.blocker_key().is_none());
        assert!(session.continuation_prompt());

        let body = session.execute_input("    x = 1");
        assert!(body.success());
        assert_eq!(body.phase(), "ok".to_string());
        assert!(body.stdout().is_empty());
        assert!(body.stderr().is_empty());
        assert!(body.error().is_none());
        assert!(body.blocker_key().is_none());
        assert!(session.continuation_prompt());

        let finalize = session.execute_input("");
        assert!(finalize.success());
        assert_eq!(finalize.phase(), "ok".to_string());
        assert!(!session.continuation_prompt());
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_repl_session_vm_probe_persists_assignments_and_echoes_expressions() {
        let mut session = WasmReplSession::new();

        let assign = session.execute_input("x = 41\n");
        assert_eq!(assign.phase(), "ok".to_string());
        assert!(assign.success());
        assert_eq!(assign.stdout(), String::new());

        let expression = session.execute_input("x + 1\n");
        assert_eq!(expression.phase(), "ok".to_string());
        assert!(expression.success());
        assert_eq!(expression.stdout(), "42".to_string());
        assert!(expression.stderr().is_empty());
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_repl_session_vm_probe_executes_multiline_class_block_on_blank_line() {
        let mut session = WasmReplSession::new();

        let header = session.execute_input("class Counter:");
        assert!(header.success());
        assert_eq!(header.phase(), "ok".to_string());
        assert!(header.stdout().is_empty());
        assert!(header.stderr().is_empty());

        let body = session.execute_input("    value = 3");
        assert!(body.success());
        assert_eq!(body.phase(), "ok".to_string());
        assert!(body.stdout().is_empty());
        assert!(body.stderr().is_empty());

        let finalize = session.execute_input("");
        assert!(finalize.success());
        assert_eq!(finalize.phase(), "ok".to_string());
        assert!(finalize.stdout().is_empty());
        assert!(finalize.stderr().is_empty());

        let expression = session.execute_input("Counter.value");
        assert!(expression.success());
        assert_eq!(expression.phase(), "ok".to_string());
        assert_eq!(expression.stdout(), "3".to_string());
        assert!(expression.stderr().is_empty());
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_repl_session_reset_reinitializes_runtime_state() {
        let mut session = WasmReplSession::new();
        let assign = session.execute_input("x = 7\n");
        assert!(assign.success());
        assert_eq!(session.inputs_executed(), 1);
        assert!(!session.continuation_prompt());

        session.reset();
        assert_eq!(session.inputs_executed(), 0);
        assert!(!session.continuation_prompt());

        let missing = session.execute_input("x\n");
        assert_eq!(missing.phase(), "runtime_error".to_string());
        assert!(!missing.success());
        assert!(missing.stderr().contains("NameError"));
        assert_eq!(session.inputs_executed(), 1);
        assert!(!session.continuation_prompt());
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_execute_vm_probe_runtime_error_contract_is_stable() {
        let runtime_error = execute("1 / 0\n");
        assert_eq!(runtime_error.phase(), "runtime_error".to_string());
        assert!(runtime_error.blocker_key().is_none());
        assert!(runtime_error.error().is_some());
        assert!(runtime_error.line() > 0);
        assert!(runtime_error.column() > 0);
    }

    #[test]
    fn wasm_worker_execute_unwired_sets_worker_blocker_key() {
        let unsupported = wasm_worker_execute("x = 1\n");
        if vm_probe_enabled() {
            assert_eq!(unsupported.phase(), "ok".to_string());
            assert!(unsupported.blocker_key().is_none());
        } else {
            assert_eq!(
                unsupported.phase(),
                "unsupported_worker_execution".to_string()
            );
            assert_eq!(
                unsupported.blocker_key(),
                Some("worker_runtime_unwired".to_string())
            );
        }

        let compile_error = wasm_worker_execute("return 1\n");
        assert_eq!(compile_error.phase(), "compile_error".to_string());
        assert!(compile_error.blocker_key().is_none());
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_worker_execute_vm_probe_runtime_error_contract_is_stable() {
        let runtime_error = wasm_worker_execute("1 / 0\n");
        assert_eq!(runtime_error.phase(), "runtime_error".to_string());
        assert!(runtime_error.blocker_key().is_none());
        assert!(runtime_error.error().is_some());
        assert!(runtime_error.line() > 0);
        assert!(runtime_error.column() > 0);
    }

    #[test]
    fn wasm_worker_execute_blocked_import_sets_capability_blocker_key() {
        let blocked = wasm_worker_execute("import socket\n");
        assert_eq!(blocked.phase(), "unsupported_worker_execution".to_string());
        assert_eq!(blocked.blocker_key(), Some("network_sockets".to_string()));
    }

    #[test]
    fn wasm_worker_timeout_configuration_value_is_deterministic() {
        let baseline = wasm_worker_recycle();
        if vm_probe_enabled() {
            assert_eq!(baseline.phase(), "worker_recycled".to_string());
            assert_eq!(baseline.state(), "ready".to_string());
        } else {
            assert_eq!(baseline.phase(), "unsupported_worker_recycle".to_string());
            assert_eq!(baseline.state(), "unwired".to_string());
        }
        assert_eq!(wasm_worker_current_timeout_ms(), 5_000);

        let configured = wasm_worker_set_timeout(7_500);
        if vm_probe_enabled() {
            assert_eq!(configured.phase(), "worker_timeout_configured".to_string());
            assert!(configured.success());
            assert_eq!(configured.timeout_ms(), 7_500);
            assert_eq!(wasm_worker_current_timeout_ms(), 7_500);
        } else {
            assert_eq!(
                configured.phase(),
                "unsupported_worker_timeout_enforcement".to_string()
            );
            assert!(!configured.success());
            assert_eq!(wasm_worker_current_timeout_ms(), 5_000);
        }

        let out_of_range = wasm_worker_set_timeout(1);
        assert_eq!(out_of_range.phase(), "invalid_worker_timeout".to_string());
        if vm_probe_enabled() {
            assert_eq!(wasm_worker_current_timeout_ms(), 7_500);
        } else {
            assert_eq!(wasm_worker_current_timeout_ms(), 5_000);
        }

        let terminated = wasm_worker_terminate();
        if vm_probe_enabled() {
            assert_eq!(terminated.phase(), "worker_terminated".to_string());
            assert_eq!(terminated.state(), "unwired".to_string());
        } else {
            assert_eq!(
                terminated.phase(),
                "unsupported_worker_terminate".to_string()
            );
            assert_eq!(terminated.state(), "unwired".to_string());
        }
        assert_eq!(wasm_worker_current_timeout_ms(), 5_000);

        let started = wasm_worker_start();
        if vm_probe_enabled() {
            assert_eq!(started.phase(), "worker_started".to_string());
            assert_eq!(started.state(), "ready".to_string());
        } else {
            assert_eq!(started.phase(), "unsupported_worker_start".to_string());
            assert_eq!(started.state(), "unwired".to_string());
        }
        assert_eq!(wasm_worker_current_timeout_ms(), 5_000);
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_worker_vm_probe_state_gates_follow_lifecycle_transitions() {
        let start = wasm_worker_start();
        assert_eq!(start.phase(), "worker_started".to_string());
        assert_eq!(start.state(), "ready".to_string());
        assert!(start.success());
        assert!(start.blocker_key().is_none());

        let info_after_start = wasm_worker_info();
        assert_eq!(info_after_start.state(), "ready".to_string());
        assert!(info_after_start.execute_supported());
        assert!(info_after_start.timeout_configuration_supported());
        assert!(info_after_start.timeout_enforcement_supported());

        let terminate = wasm_worker_terminate();
        assert_eq!(terminate.phase(), "worker_terminated".to_string());
        assert_eq!(terminate.state(), "unwired".to_string());
        assert!(terminate.success());
        let info_after_terminate = wasm_worker_info();
        assert_eq!(info_after_terminate.state(), "unwired".to_string());
        assert!(!info_after_terminate.execute_supported());
        assert!(!info_after_terminate.timeout_configuration_supported());
        assert!(!info_after_terminate.timeout_enforcement_supported());

        let blocked_execute = wasm_worker_execute_with_operation("x = 1\n");
        assert_eq!(
            blocked_execute.phase(),
            "unsupported_worker_execution".to_string()
        );
        assert_eq!(blocked_execute.state(), "unwired".to_string());
        assert_eq!(
            blocked_execute.blocker_key(),
            Some("worker_runtime_unwired".to_string())
        );

        let blocked_timeout = wasm_worker_set_timeout(5_000);
        assert_eq!(
            blocked_timeout.phase(),
            "unsupported_worker_timeout_enforcement".to_string()
        );
        assert_eq!(blocked_timeout.state(), "unwired".to_string());
        assert_eq!(
            blocked_timeout.blocker_key(),
            Some("worker_runtime_unwired".to_string())
        );

        let recycle = wasm_worker_recycle();
        assert_eq!(recycle.phase(), "worker_recycled".to_string());
        assert_eq!(recycle.state(), "ready".to_string());
        assert!(recycle.success());
        let info_after_recycle = wasm_worker_info();
        assert_eq!(info_after_recycle.state(), "ready".to_string());
        assert!(info_after_recycle.execute_supported());
        assert!(info_after_recycle.timeout_configuration_supported());
        assert!(info_after_recycle.timeout_enforcement_supported());

        let resumed_execute = wasm_worker_execute_with_operation("x = 1\n");
        assert_eq!(resumed_execute.phase(), "ok".to_string());
        assert_eq!(resumed_execute.state(), "ready".to_string());
        assert!(resumed_execute.success());
        assert!(resumed_execute.blocker_key().is_none());

        let resumed_timeout = wasm_worker_set_timeout(5_000);
        assert_eq!(
            resumed_timeout.phase(),
            "worker_timeout_configured".to_string()
        );
        assert_eq!(resumed_timeout.state(), "ready".to_string());
        assert!(resumed_timeout.success());
        assert!(resumed_timeout.blocker_key().is_none());
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_worker_vm_probe_runtime_state_persists_until_recycle_or_start() {
        let recycle = wasm_worker_recycle();
        assert_eq!(recycle.phase(), "worker_recycled".to_string());
        assert_eq!(recycle.state(), "ready".to_string());

        let assign = wasm_worker_execute("x = 41\n");
        assert_eq!(assign.phase(), "ok".to_string());
        assert!(assign.success());

        let assert_present = wasm_worker_execute("assert x == 41\n");
        assert_eq!(assert_present.phase(), "ok".to_string());
        assert!(assert_present.success());

        let recycled = wasm_worker_recycle();
        assert_eq!(recycled.phase(), "worker_recycled".to_string());
        assert_eq!(recycled.state(), "ready".to_string());

        let missing_after_recycle = wasm_worker_execute("x\n");
        assert_eq!(missing_after_recycle.phase(), "runtime_error".to_string());
        assert!(!missing_after_recycle.success());
        assert!(
            missing_after_recycle.stderr().contains("NameError"),
            "expected NameError after recycle reset, got: {}",
            missing_after_recycle.stderr()
        );

        let reassign = wasm_worker_execute("x = 9\n");
        assert_eq!(reassign.phase(), "ok".to_string());
        assert!(reassign.success());

        let terminate = wasm_worker_terminate();
        assert_eq!(terminate.phase(), "worker_terminated".to_string());
        assert_eq!(terminate.state(), "unwired".to_string());

        let blocked_while_terminated = wasm_worker_execute("x\n");
        assert_eq!(
            blocked_while_terminated.phase(),
            "unsupported_worker_execution".to_string()
        );
        assert_eq!(
            blocked_while_terminated.blocker_key(),
            Some("worker_runtime_unwired".to_string())
        );

        let start = wasm_worker_start();
        assert_eq!(start.phase(), "worker_started".to_string());
        assert_eq!(start.state(), "ready".to_string());

        let missing_after_start = wasm_worker_execute("x\n");
        assert_eq!(missing_after_start.phase(), "runtime_error".to_string());
        assert!(!missing_after_start.success());
        assert!(
            missing_after_start.stderr().contains("NameError"),
            "expected NameError after terminate/start reset, got: {}",
            missing_after_start.stderr()
        );
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_worker_vm_probe_runtime_error_keeps_worker_ready_and_stateful() {
        let recycle = wasm_worker_recycle();
        assert_eq!(recycle.phase(), "worker_recycled".to_string());
        assert_eq!(recycle.state(), "ready".to_string());

        let assign = wasm_worker_execute_with_operation("x = 11\n");
        assert_eq!(assign.phase(), "ok".to_string());
        assert_eq!(assign.state(), "ready".to_string());
        assert!(assign.success());

        let runtime_error = wasm_worker_execute_with_operation("1 / 0\n");
        assert_eq!(runtime_error.phase(), "runtime_error".to_string());
        assert_eq!(runtime_error.state(), "ready".to_string());
        assert!(!runtime_error.success());
        assert!(runtime_error.blocker_key().is_none());
        assert!(runtime_error.error().is_some());

        let resumed = wasm_worker_execute_with_operation("assert x == 11\n");
        assert_eq!(resumed.phase(), "ok".to_string());
        assert_eq!(resumed.state(), "ready".to_string());
        assert!(resumed.success());
        assert!(resumed.blocker_key().is_none());

        let info = wasm_worker_info();
        assert_eq!(info.state(), "ready".to_string());
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_worker_vm_probe_timeout_recycles_worker_runtime_state() {
        let recycle = wasm_worker_recycle();
        assert_eq!(recycle.phase(), "worker_recycled".to_string());
        assert_eq!(recycle.state(), "ready".to_string());

        let configured = wasm_worker_set_timeout(50);
        assert_eq!(configured.phase(), "worker_timeout_configured".to_string());
        assert!(configured.success());
        assert_eq!(wasm_worker_current_timeout_ms(), 50);

        let assigned = wasm_worker_execute_with_operation("x = 123\n");
        assert_eq!(assigned.phase(), "ok".to_string());
        assert_eq!(assigned.state(), "ready".to_string());
        assert!(assigned.success());

        let timed_out = wasm_worker_execute_with_operation("while True:\n    pass\n");
        assert_eq!(timed_out.phase(), "runtime_error".to_string());
        assert_eq!(timed_out.state(), "ready".to_string());
        assert!(!timed_out.success());
        let timeout_error = timed_out
            .error()
            .expect("timeout execution should report runtime error");
        assert!(timeout_error.contains("execution timeout exceeded"));

        assert_eq!(wasm_worker_current_timeout_ms(), 5_000);

        let missing_after_timeout = wasm_worker_execute_with_operation("x\n");
        assert_eq!(missing_after_timeout.phase(), "runtime_error".to_string());
        assert_eq!(missing_after_timeout.state(), "ready".to_string());
        let missing_error = missing_after_timeout
            .error()
            .expect("post-timeout execution should report NameError");
        assert!(missing_error.contains("NameError"));
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_worker_vm_probe_failed_state_blocks_until_recovered() {
        clear_worker_vm();
        set_current_worker_state(WasmWorkerState::Failed);
        let mut operation_ids = HashSet::new();
        let failed_info = wasm_worker_info();
        assert_eq!(failed_info.state(), "failed".to_string());
        assert!(!failed_info.execute_supported());
        assert!(!failed_info.timeout_configuration_supported());
        assert!(!failed_info.timeout_enforcement_supported());

        let blocked_execute = wasm_worker_execute_with_operation("x = 1\n");
        assert_eq!(
            blocked_execute.phase(),
            "unsupported_worker_execution".to_string()
        );
        assert_eq!(blocked_execute.state(), "failed".to_string());
        assert_eq!(
            blocked_execute.blocker_key(),
            Some("worker_runtime_failed".to_string())
        );
        let blocked_execute_id = blocked_execute.operation_id();
        assert!(blocked_execute_id.starts_with("worker_execute_"));
        assert!(operation_ids.insert(blocked_execute_id));
        assert!(
            blocked_execute
                .error()
                .expect("failed-state execute should have error")
                .contains("failed state")
        );

        let blocked_timeout = wasm_worker_set_timeout(5_000);
        assert_eq!(
            blocked_timeout.phase(),
            "unsupported_worker_timeout_enforcement".to_string()
        );
        assert_eq!(blocked_timeout.state(), "failed".to_string());
        assert_eq!(
            blocked_timeout.blocker_key(),
            Some("worker_runtime_failed".to_string())
        );
        let blocked_timeout_id = blocked_timeout.operation_id();
        assert!(blocked_timeout_id.starts_with("worker_set_timeout_"));
        assert!(operation_ids.insert(blocked_timeout_id));

        let invalid_timeout = wasm_worker_set_timeout(0);
        assert_eq!(
            invalid_timeout.phase(),
            "invalid_worker_timeout".to_string()
        );
        assert_eq!(invalid_timeout.state(), "failed".to_string());
        assert!(!invalid_timeout.success());
        assert!(invalid_timeout.blocker_key().is_none());
        assert!(invalid_timeout.error().is_some());
        let invalid_timeout_id = invalid_timeout.operation_id();
        assert!(invalid_timeout_id.starts_with("worker_set_timeout_"));
        assert!(operation_ids.insert(invalid_timeout_id));

        let recycle = wasm_worker_recycle();
        assert_eq!(recycle.phase(), "worker_recycled".to_string());
        assert_eq!(recycle.state(), "ready".to_string());
        assert!(recycle.success());
        let recycle_id = recycle.operation_id();
        assert!(recycle_id.starts_with("worker_recycle_"));
        assert!(operation_ids.insert(recycle_id));
        let recovered_info = wasm_worker_info();
        assert_eq!(recovered_info.state(), "ready".to_string());
        assert!(recovered_info.execute_supported());
        assert!(recovered_info.timeout_configuration_supported());
        assert!(recovered_info.timeout_enforcement_supported());

        let resumed_execute = wasm_worker_execute_with_operation("x = 1\n");
        assert_eq!(resumed_execute.phase(), "ok".to_string());
        assert_eq!(resumed_execute.state(), "ready".to_string());
        assert!(resumed_execute.success());
        assert!(resumed_execute.blocker_key().is_none());
        let resumed_execute_id = resumed_execute.operation_id();
        assert!(resumed_execute_id.starts_with("worker_execute_"));
        assert!(operation_ids.insert(resumed_execute_id));
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_worker_vm_probe_failed_state_keeps_top_level_execute_available() {
        clear_worker_vm();
        set_current_worker_state(WasmWorkerState::Failed);

        let blocked_worker = wasm_worker_execute_with_operation("x = 1\n");
        assert_eq!(
            blocked_worker.phase(),
            "unsupported_worker_execution".to_string()
        );
        assert_eq!(
            blocked_worker.blocker_key(),
            Some("worker_runtime_failed".to_string())
        );

        let top_level = execute("x = 1\n");
        assert_eq!(top_level.phase(), "ok".to_string());
        assert!(top_level.success());
        assert!(top_level.blocker_key().is_none());

        let started = wasm_worker_start();
        assert_eq!(started.phase(), "worker_started".to_string());
        assert_eq!(started.state(), "ready".to_string());

        let resumed_worker = wasm_worker_execute_with_operation("assert x == 1\n");
        assert_eq!(resumed_worker.phase(), "ok".to_string());
        assert_eq!(resumed_worker.state(), "ready".to_string());
        assert!(resumed_worker.success());
        assert!(resumed_worker.blocker_key().is_none());
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_worker_vm_probe_failed_state_start_restores_timeout_configuration() {
        clear_worker_vm();
        set_current_worker_state(WasmWorkerState::Failed);

        let blocked_timeout = wasm_worker_set_timeout(5_000);
        assert_eq!(
            blocked_timeout.phase(),
            "unsupported_worker_timeout_enforcement".to_string()
        );
        assert_eq!(blocked_timeout.state(), "failed".to_string());
        assert_eq!(
            blocked_timeout.blocker_key(),
            Some("worker_runtime_failed".to_string())
        );

        let started = wasm_worker_start();
        assert_eq!(started.phase(), "worker_started".to_string());
        assert_eq!(started.state(), "ready".to_string());
        assert!(started.success());
        assert_eq!(wasm_worker_current_timeout_ms(), 5_000);

        let configured = wasm_worker_set_timeout(250);
        assert_eq!(configured.phase(), "worker_timeout_configured".to_string());
        assert_eq!(configured.state(), "ready".to_string());
        assert!(configured.success());
        assert!(configured.blocker_key().is_none());
        assert_eq!(wasm_worker_current_timeout_ms(), 250);

        let execute_ok = wasm_worker_execute_with_operation("x = 7\n");
        assert_eq!(execute_ok.phase(), "ok".to_string());
        assert_eq!(execute_ok.state(), "ready".to_string());
        assert!(execute_ok.success());
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_worker_vm_probe_failed_state_terminate_then_start_restores_worker_execute() {
        let baseline = wasm_worker_recycle();
        assert_eq!(baseline.phase(), "worker_recycled".to_string());
        assert_eq!(baseline.state(), "ready".to_string());
        let preconfigured = wasm_worker_set_timeout(250);
        assert_eq!(
            preconfigured.phase(),
            "worker_timeout_configured".to_string()
        );
        assert!(preconfigured.success());
        assert_eq!(wasm_worker_current_timeout_ms(), 250);
        let pre_assigned = wasm_worker_execute_with_operation("x = 1\n");
        assert_eq!(pre_assigned.phase(), "ok".to_string());
        assert!(pre_assigned.success());

        set_current_worker_state(WasmWorkerState::Failed);

        let terminated = wasm_worker_terminate();
        assert_eq!(terminated.phase(), "worker_terminated".to_string());
        assert_eq!(terminated.state(), "unwired".to_string());
        assert!(terminated.success());
        let info_after_terminate = wasm_worker_info();
        assert_eq!(info_after_terminate.state(), "unwired".to_string());
        assert!(!info_after_terminate.execute_supported());
        assert!(!info_after_terminate.timeout_configuration_supported());
        assert!(!info_after_terminate.timeout_enforcement_supported());

        let blocked_while_unwired = wasm_worker_execute_with_operation("x = 1\n");
        assert_eq!(
            blocked_while_unwired.phase(),
            "unsupported_worker_execution".to_string()
        );
        assert_eq!(blocked_while_unwired.state(), "unwired".to_string());
        assert_eq!(
            blocked_while_unwired.blocker_key(),
            Some("worker_runtime_unwired".to_string())
        );
        let blocked_timeout = wasm_worker_set_timeout(5_000);
        assert_eq!(
            blocked_timeout.phase(),
            "unsupported_worker_timeout_enforcement".to_string()
        );
        assert_eq!(blocked_timeout.state(), "unwired".to_string());
        assert_eq!(
            blocked_timeout.blocker_key(),
            Some("worker_runtime_unwired".to_string())
        );

        let started = wasm_worker_start();
        assert_eq!(started.phase(), "worker_started".to_string());
        assert_eq!(started.state(), "ready".to_string());
        assert!(started.success());
        assert_eq!(wasm_worker_current_timeout_ms(), 5_000);
        let info_after_start = wasm_worker_info();
        assert_eq!(info_after_start.state(), "ready".to_string());
        assert!(info_after_start.execute_supported());
        assert!(info_after_start.timeout_configuration_supported());
        assert!(info_after_start.timeout_enforcement_supported());
        let missing_after_start = wasm_worker_execute_with_operation("x\n");
        assert_eq!(missing_after_start.phase(), "runtime_error".to_string());
        assert_eq!(missing_after_start.state(), "ready".to_string());
        assert!(!missing_after_start.success());
        let missing_error = missing_after_start
            .error()
            .expect("post-terminate/start execute should report NameError");
        assert!(missing_error.contains("NameError"));
        let configured_timeout = wasm_worker_set_timeout(200);
        assert_eq!(
            configured_timeout.phase(),
            "worker_timeout_configured".to_string()
        );
        assert_eq!(configured_timeout.state(), "ready".to_string());
        assert!(configured_timeout.success());
        assert!(configured_timeout.blocker_key().is_none());

        let resumed = wasm_worker_execute_with_operation("x = 1\n");
        assert_eq!(resumed.phase(), "ok".to_string());
        assert_eq!(resumed.state(), "ready".to_string());
        assert!(resumed.success());
        assert!(resumed.blocker_key().is_none());
    }

    #[cfg(feature = "wasm-vm-probe")]
    #[test]
    fn wasm_worker_vm_probe_failed_state_recycle_resets_timeout_and_vm_state() {
        let baseline = wasm_worker_recycle();
        assert_eq!(baseline.phase(), "worker_recycled".to_string());
        assert_eq!(baseline.state(), "ready".to_string());

        let configured = wasm_worker_set_timeout(250);
        assert_eq!(configured.phase(), "worker_timeout_configured".to_string());
        assert_eq!(configured.state(), "ready".to_string());
        assert!(configured.success());
        assert_eq!(wasm_worker_current_timeout_ms(), 250);

        let assigned = wasm_worker_execute_with_operation("x = 42\n");
        assert_eq!(assigned.phase(), "ok".to_string());
        assert_eq!(assigned.state(), "ready".to_string());
        assert!(assigned.success());

        set_current_worker_state(WasmWorkerState::Failed);
        let recycled = wasm_worker_recycle();
        assert_eq!(recycled.phase(), "worker_recycled".to_string());
        assert_eq!(recycled.state(), "ready".to_string());
        assert!(recycled.success());
        assert_eq!(wasm_worker_current_timeout_ms(), 5_000);

        let missing_after_recycle = wasm_worker_execute_with_operation("x\n");
        assert_eq!(missing_after_recycle.phase(), "runtime_error".to_string());
        assert_eq!(missing_after_recycle.state(), "ready".to_string());
        assert!(!missing_after_recycle.success());
        let missing_error = missing_after_recycle
            .error()
            .expect("post-failed recycle execute should report NameError");
        assert!(missing_error.contains("NameError"));
    }

    #[cfg(not(feature = "wasm-vm-probe"))]
    #[test]
    fn wasm_worker_default_mode_lifecycle_remains_unwired() {
        let start = wasm_worker_start();
        assert_eq!(start.phase(), "unsupported_worker_start".to_string());
        assert_eq!(start.state(), "unwired".to_string());
        assert!(!start.success());
        assert_eq!(
            start.blocker_key(),
            Some("worker_runtime_unwired".to_string())
        );

        let terminate = wasm_worker_terminate();
        assert_eq!(
            terminate.phase(),
            "unsupported_worker_terminate".to_string()
        );
        assert_eq!(terminate.state(), "unwired".to_string());
        assert!(!terminate.success());
        assert_eq!(
            terminate.blocker_key(),
            Some("worker_runtime_unwired".to_string())
        );

        let recycle = wasm_worker_recycle();
        assert_eq!(recycle.phase(), "unsupported_worker_recycle".to_string());
        assert_eq!(recycle.state(), "unwired".to_string());
        assert!(!recycle.success());
        assert_eq!(
            recycle.blocker_key(),
            Some("worker_runtime_unwired".to_string())
        );

        let info = wasm_worker_info();
        assert_eq!(info.state(), "unwired".to_string());
        assert!(!info.execute_supported());
        assert!(!info.timeout_configuration_supported());
        assert!(!info.timeout_enforcement_supported());
    }
}
