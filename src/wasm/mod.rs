use std::sync::Once;

use crate::host::{HostCapability, VmHost, WasmHost};
use wasm_bindgen::prelude::*;

pub const WASM_API_VERSION: u32 = 1;

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
pub struct WasmSession {
    snippets_checked: usize,
    last_error: Option<String>,
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

fn format_parse_error(err: &crate::parser::ParseError) -> String {
    format!("{} (line {}, column {})", err.message, err.line, err.column)
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
