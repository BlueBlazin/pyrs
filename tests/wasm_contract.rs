#![cfg(target_arch = "wasm32")]

use pyrs::wasm::{
    WasmSession, check_compile_result, check_syntax_result, execute, wasm_api_version,
    wasm_capabilities,
    wasm_capability_error, wasm_capability_keys, wasm_execution_blocker_error,
    wasm_execution_blocker_keys, wasm_runtime_info,
};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn wasm_runtime_contract_basics() {
    let runtime = wasm_runtime_info();
    assert_eq!(runtime.api_version(), wasm_api_version());
    assert_eq!(runtime.execution_status(), "syntax_compile_only");
    assert!(!runtime.supports_execution());
    assert!(!runtime.pyrs_version().is_empty());
}

#[wasm_bindgen_test]
fn wasm_capability_contract_is_stable() {
    let keys = wasm_capability_keys();
    let mut listed_keys = Vec::new();
    for index in 0..keys.length() {
        let key = keys
            .get(index)
            .as_string()
            .expect("capability key should be string");
        listed_keys.push(key);
    }
    assert!(listed_keys.contains(&"process_args".to_string()));
    assert!(listed_keys.contains(&"filesystem_read".to_string()));

    let capabilities = wasm_capabilities();
    assert!(capabilities.process_args());
    assert!(!capabilities.filesystem_read());
    assert!(!capabilities.dynamic_library_load());

    assert!(wasm_capability_error("process_args").is_none());
    let fs_error = wasm_capability_error("filesystem_read").expect("filesystem_read unsupported");
    assert!(fs_error.contains("filesystem_read"));
}

#[wasm_bindgen_test]
fn wasm_execution_blocker_contract_is_stable() {
    let keys = wasm_execution_blocker_keys();
    let mut listed_keys = Vec::new();
    for index in 0..keys.length() {
        let key = keys
            .get(index)
            .as_string()
            .expect("blocker key should be string");
        listed_keys.push(key);
    }
    assert!(listed_keys.contains(&"execution_backend_unwired".to_string()));
    let err = wasm_execution_blocker_error("execution_backend_unwired")
        .expect("backend blocker should have error");
    assert!(err.contains("not wired"));
}

#[wasm_bindgen_test]
fn wasm_syntax_and_execute_contract() {
    let valid = check_syntax_result("value = 1\n");
    assert!(valid.ok());
    assert!(valid.error().is_none());

    let invalid = check_syntax_result("def broken(:\n");
    assert!(!invalid.ok());
    assert!(invalid.error().is_some());
    assert!(invalid.line() > 0);
    assert!(invalid.column() > 0);

    let semantic = check_compile_result("return 1\n");
    assert!(!semantic.ok());
    assert_eq!(semantic.phase(), "compile_error");
    let semantic_error = semantic.error().expect("compile error should be populated");
    assert!(semantic_error.contains("outside function"));

    let unsupported = execute("value = 1\n");
    assert!(!unsupported.success());
    assert_eq!(unsupported.phase(), "unsupported_execution");
    assert!(unsupported.error().is_some());
    assert!(unsupported.stderr().contains("not wired"));

    let compile_error = execute("return 1\n");
    assert!(!compile_error.success());
    assert_eq!(compile_error.phase(), "compile_error");
    assert!(compile_error.stderr().contains("outside function"));
}

#[wasm_bindgen_test]
fn wasm_session_tracks_and_resets_state() {
    let mut session = WasmSession::new();
    assert_eq!(session.snippets_checked(), 0);
    assert!(session.last_error().is_none());

    let first = session.check_syntax("x = 1\n");
    assert!(first.ok());
    assert_eq!(session.snippets_checked(), 1);

    let second = session.execute("x = 1\n");
    assert_eq!(second.phase(), "unsupported_execution");
    assert_eq!(session.snippets_checked(), 2);
    assert!(session.last_error().is_some());

    session.reset();
    assert_eq!(session.snippets_checked(), 0);
    assert!(session.last_error().is_none());
}
