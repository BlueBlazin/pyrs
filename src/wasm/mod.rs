use std::sync::Once;

use crate::host::{HostCapability, VmHost, WasmHost};
use js_sys::Array;
use wasm_bindgen::prelude::*;

pub const WASM_API_VERSION: u32 = 1;
const WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED: &str = "execution_backend_unwired";

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
pub struct WasmExecutionResult {
    success: bool,
    phase: String,
    stdout: String,
    stderr: String,
    error: Option<String>,
}

#[wasm_bindgen(getter_with_clone)]
pub struct WasmRuntimeInfo {
    api_version: u32,
    pyrs_version: String,
    supports_execution: bool,
    execution_status: String,
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
    pub fn execution_status(&self) -> String {
        self.execution_status.clone()
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
    WasmRuntimeInfo {
        api_version: wasm_api_version(),
        pyrs_version: pyrs_version(),
        supports_execution: false,
        execution_status: "syntax_compile_only".to_string(),
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
    }
}

fn format_parse_error(err: &crate::parser::ParseError) -> String {
    format!("{} (line {}, column {})", err.message, err.line, err.column)
}

fn format_compile_error(err: &crate::compiler::CompileError) -> (String, usize, usize) {
    match err.span {
        Some(span) => (
            format!("{} (line {}, column {})", err.message, span.line, span.column),
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
    let module = match crate::parser::parse_module(source) {
        Ok(module) => module,
        Err(err) => {
            return WasmCompileResult {
                ok: false,
                phase: "syntax_error".to_string(),
                error: Some(format_parse_error(&err)),
                line: err.line,
                column: err.column,
            };
        }
    };
    match crate::compiler::compile_module_with_filename(&module, "<wasm>") {
        Ok(_) => WasmCompileResult {
            ok: true,
            phase: "ok".to_string(),
            error: None,
            line: 0,
            column: 0,
        },
        Err(err) => {
            let (message, line, column) = format_compile_error(&err);
            WasmCompileResult {
                ok: false,
                phase: "compile_error".to_string(),
                error: Some(message),
                line,
                column,
            }
        }
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
