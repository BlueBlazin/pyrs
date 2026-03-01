use std::sync::Once;

use wasm_bindgen::prelude::*;

/// Minimal WASM bridge surface used during compile-isolation bring-up.
#[wasm_bindgen]
pub fn pyrs_version() -> String {
    crate::VERSION.to_string()
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
