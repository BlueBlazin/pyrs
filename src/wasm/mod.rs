use wasm_bindgen::prelude::*;

/// Minimal WASM bridge surface used during compile-isolation bring-up.
#[wasm_bindgen]
pub fn pyrs_version() -> String {
    crate::VERSION.to_string()
}

