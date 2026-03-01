use std::collections::HashSet;
use std::sync::Once;

use crate::host::{HostCapability, VmHost, WasmHost};
use js_sys::Array;
use wasm_bindgen::prelude::*;

pub const WASM_API_VERSION: u32 = 1;
const WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED: &str = "execution_backend_unwired";
const WASM_WORKER_BLOCKER_RUNTIME_UNWIRED: &str = "worker_runtime_unwired";
const WASM_WORKER_PHASE_UNSUPPORTED_START: &str = "unsupported_worker_start";
const WASM_WORKER_PHASE_UNSUPPORTED_TERMINATE: &str = "unsupported_worker_terminate";
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

fn execution_blocker_keys(host: &dyn VmHost) -> Vec<&'static str> {
    let mut keys = vec![WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED];
    for capability in HostCapability::all() {
        if !host.supports(*capability) {
            keys.push(capability.key());
        }
    }
    keys
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

/// Installs panic hook once so Rust panic payloads surface in browser console.
#[wasm_bindgen]
pub fn init_wasm_runtime() {
    PANIC_HOOK_ONCE.call_once(console_error_panic_hook::set_once);
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

#[wasm_bindgen(getter_with_clone)]
pub struct WasmWorkerSession {
    starts_requested: usize,
    terminates_requested: usize,
    last_phase: Option<String>,
    last_error: Option<String>,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmExecutionResult {
    success: bool,
    phase: String,
    stdout: String,
    stderr: String,
    error: Option<String>,
    line: usize,
    column: usize,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmRuntimeInfo {
    api_version: u32,
    pyrs_version: String,
    supports_parse_compile: bool,
    supports_execution: bool,
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
    state: String,
    interruption_model: String,
    blocker_count: usize,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmWorkerLifecycleResult {
    success: bool,
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
    pub fn state(&self) -> String {
        self.state.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn interruption_model(&self) -> String {
        self.interruption_model.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn blocker_count(&self) -> usize {
        self.blocker_count
    }
}

#[wasm_bindgen]
impl WasmWorkerLifecycleResult {
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
impl WasmWorkerSession {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        init_wasm_runtime();
        Self {
            starts_requested: 0,
            terminates_requested: 0,
            last_phase: None,
            last_error: None,
        }
    }

    pub fn info(&self) -> WasmWorkerInfo {
        wasm_worker_info()
    }

    pub fn start(&mut self) -> WasmWorkerLifecycleResult {
        let result = wasm_worker_start();
        self.starts_requested += 1;
        self.last_phase = Some(result.phase.clone());
        self.last_error = result.error.clone();
        result
    }

    pub fn terminate(&mut self) -> WasmWorkerLifecycleResult {
        let result = wasm_worker_terminate();
        self.terminates_requested += 1;
        self.last_phase = Some(result.phase.clone());
        self.last_error = result.error.clone();
        result
    }

    pub fn reset(&mut self) {
        self.starts_requested = 0;
        self.terminates_requested = 0;
        self.last_phase = None;
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
    pub fn last_phase(&self) -> Option<String> {
        self.last_phase.clone()
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

/// Reports runtime contract status for browser clients.
#[wasm_bindgen]
pub fn wasm_runtime_info() -> WasmRuntimeInfo {
    let host = WasmHost;
    let blocker_count = execution_blocker_keys(&host).len();
    WasmRuntimeInfo {
        api_version: wasm_api_version(),
        pyrs_version: pyrs_version(),
        supports_parse_compile: true,
        supports_execution: false,
        execution_status: "syntax_compile_only".to_string(),
        execution_blocker_count: blocker_count,
    }
}

/// Returns canonical blocker keys for worker-mode execution.
#[wasm_bindgen]
pub fn wasm_worker_blocker_keys() -> Array {
    let keys = Array::new();
    keys.push(&JsValue::from_str(WASM_WORKER_BLOCKER_RUNTIME_UNWIRED));
    keys
}

/// Returns a stable blocker message for wasm worker blockers.
#[wasm_bindgen]
pub fn wasm_worker_blocker_error(blocker_key: &str) -> Option<String> {
    if blocker_key == WASM_WORKER_BLOCKER_RUNTIME_UNWIRED {
        return Some("wasm worker runtime is not wired yet".to_string());
    }
    None
}

/// Reports worker-runtime contract state for browser clients.
#[wasm_bindgen]
pub fn wasm_worker_info() -> WasmWorkerInfo {
    let blockers = wasm_worker_blocker_keys();
    WasmWorkerInfo {
        supported: false,
        state: "unwired".to_string(),
        interruption_model: "worker_recycle".to_string(),
        blocker_count: blockers.length() as usize,
    }
}

fn worker_unwired_result(phase: &'static str) -> WasmWorkerLifecycleResult {
    let blocker_key = WASM_WORKER_BLOCKER_RUNTIME_UNWIRED.to_string();
    let message = wasm_worker_blocker_error(WASM_WORKER_BLOCKER_RUNTIME_UNWIRED)
        .unwrap_or_else(|| "wasm worker runtime is not wired yet".to_string());
    WasmWorkerLifecycleResult {
        success: false,
        phase: phase.to_string(),
        state: "unwired".to_string(),
        error: Some(message),
        blocker_key: Some(blocker_key),
    }
}

/// Starts worker runtime execution.
///
/// Current milestone behavior:
/// - returns `phase = "unsupported_worker_start"` until worker backend is wired.
#[wasm_bindgen]
pub fn wasm_worker_start() -> WasmWorkerLifecycleResult {
    worker_unwired_result(WASM_WORKER_PHASE_UNSUPPORTED_START)
}

/// Terminates worker runtime execution.
///
/// Current milestone behavior:
/// - returns `phase = "unsupported_worker_terminate"` until worker backend is wired.
#[wasm_bindgen]
pub fn wasm_worker_terminate() -> WasmWorkerLifecycleResult {
    worker_unwired_result(WASM_WORKER_PHASE_UNSUPPORTED_TERMINATE)
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
    if root.is_empty() {
        None
    } else {
        Some(root)
    }
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

fn parse_and_compile_module(source: &str) -> Result<crate::ast::Module, WasmCompileResult> {
    let module = match crate::parser::parse_module(source) {
        Ok(module) => module,
        Err(err) => {
            return Err(WasmCompileResult {
                ok: false,
                phase: "syntax_error".to_string(),
                error: Some(format_parse_error(&err)),
                line: err.line,
                column: err.column,
            });
        }
    };

    match crate::compiler::compile_module_with_filename(&module, "<wasm>") {
        Ok(_) => Ok(module),
        Err(err) => {
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
    let module = match parse_and_compile_module(source) {
        Ok(module) => module,
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
    let import_roots = collect_import_roots(&module);
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
    let Ok(module) = parse_and_compile_module(source) else {
        return Array::new();
    };
    let import_roots = collect_import_roots(&module);
    let blockers = snippet_blockers_from_import_roots(&import_roots, &host);
    let result = Array::new();
    for blocker in blockers {
        result.push(&JsValue::from(blocker));
    }
    result
}

/// Returns a stable blocker message for wasm execution blockers.
#[wasm_bindgen]
pub fn wasm_execution_blocker_error(blocker_key: &str) -> Option<String> {
    if blocker_key == WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED {
        return Some("wasm execution backend is not wired yet".to_string());
    }
    wasm_capability_error(blocker_key)
}

/// Executes a snippet using the current wasm bridge contract.
///
/// Current milestone behavior:
/// - parse-invalid input returns `phase = "syntax_error"`
/// - parse-valid but compile-invalid input returns `phase = "compile_error"`
/// - parse+compile-valid input returns `phase = "unsupported_execution"`
///   until runtime execution is wired for wasm.
#[wasm_bindgen]
pub fn execute(source: &str) -> WasmExecutionResult {
    let compile = check_compile_result(source);
    if !compile.ok {
        let error = compile.error;
        let stderr = error
            .clone()
            .unwrap_or_else(|| "parse/compile check failed".to_string());
        return WasmExecutionResult {
            success: false,
            phase: compile.phase,
            stdout: String::new(),
            stderr,
            error,
            line: compile.line,
            column: compile.column,
        };
    }

    let message = wasm_execution_blocker_error(WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED)
        .unwrap_or_else(|| "wasm execution backend is not wired yet".to_string());
    WasmExecutionResult {
        success: false,
        phase: "unsupported_execution".to_string(),
        stdout: String::new(),
        stderr: message.clone(),
        error: Some(message),
        line: 0,
        column: 0,
    }
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
    match parse_and_compile_module(source) {
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
    use super::{check_syntax, pyrs_version};

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
}
