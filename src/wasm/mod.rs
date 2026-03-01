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

/// Parses module source and reports syntax diagnostics with parser-native text.
#[wasm_bindgen]
pub fn check_syntax(source: &str) -> Result<(), JsValue> {
    init_wasm_runtime();
    crate::parser::parse_module(source).map(|_| ()).map_err(|err| {
        let message = format!(
            "{} (line {}, column {})",
            err.message, err.line, err.column
        );
        JsValue::from_str(&message)
    })
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
