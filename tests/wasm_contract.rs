#![cfg(target_arch = "wasm32")]

use pyrs::wasm::{
    WasmSession, check_compile_result, check_syntax_result, execute, wasm_api_version,
    wasm_capabilities, wasm_execution_blockers, wasm_module_policy_entries, wasm_module_support,
    wasm_capability_error, wasm_capability_keys, wasm_execution_blocker_error,
    wasm_execution_blocker_keys, wasm_runtime_info,
};
use js_sys::Reflect;
use std::collections::HashSet;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn wasm_runtime_contract_basics() {
    let runtime = wasm_runtime_info();
    assert_eq!(runtime.api_version(), wasm_api_version());
    assert!(runtime.supports_parse_compile());
    assert_eq!(runtime.execution_status(), "syntax_compile_only");
    assert!(!runtime.supports_execution());
    let blocker_keys = wasm_execution_blocker_keys();
    assert_eq!(
        runtime.execution_blocker_count(),
        blocker_keys.length() as usize
    );
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
    assert!(listed_keys.contains(&"filesystem_read".to_string()));
    let err = wasm_execution_blocker_error("execution_backend_unwired")
        .expect("backend blocker should have error");
    assert!(err.contains("not wired"));
    let fs_err =
        wasm_execution_blocker_error("filesystem_read").expect("filesystem_read should be blocked");
    assert!(fs_err.contains("filesystem_read"));

    let blockers = wasm_execution_blockers();
    assert!(blockers.length() >= 2);
    let mut saw_backend = false;
    for index in 0..blockers.length() {
        let blocker = blockers.get(index);
        let key = Reflect::get(&blocker, &"key".into())
            .expect("blocker.key")
            .as_string()
            .expect("blocker.key as string");
        let message = Reflect::get(&blocker, &"message".into())
            .expect("blocker.message")
            .as_string()
            .expect("blocker.message as string");
        if key == "execution_backend_unwired" {
            assert!(message.contains("not wired"));
            saw_backend = true;
        }
    }
    assert!(saw_backend);
}

#[wasm_bindgen_test]
fn wasm_execution_blockers_match_capability_matrix() {
    let capabilities = wasm_capabilities();
    let mut expected = HashSet::new();
    expected.insert("execution_backend_unwired".to_string());
    if !capabilities.filesystem_read() {
        expected.insert("filesystem_read".to_string());
    }
    if !capabilities.filesystem_write() {
        expected.insert("filesystem_write".to_string());
    }
    if !capabilities.environment_read() {
        expected.insert("environment_read".to_string());
    }
    if !capabilities.process_args() {
        expected.insert("process_args".to_string());
    }
    if !capabilities.process_spawn() {
        expected.insert("process_spawn".to_string());
    }
    if !capabilities.dynamic_library_load() {
        expected.insert("dynamic_library_load".to_string());
    }
    if !capabilities.interactive_terminal() {
        expected.insert("interactive_terminal".to_string());
    }
    if !capabilities.network_sockets() {
        expected.insert("network_sockets".to_string());
    }

    let keys = wasm_execution_blocker_keys();
    let mut listed_keys = HashSet::new();
    for index in 0..keys.length() {
        let key = keys
            .get(index)
            .as_string()
            .expect("blocker key should be string");
        listed_keys.insert(key);
    }
    assert_eq!(listed_keys, expected);

    for key in listed_keys {
        let message = wasm_execution_blocker_error(&key)
            .unwrap_or_else(|| panic!("missing blocker error for key: {key}"));
        assert!(!message.trim().is_empty());
    }
}

#[wasm_bindgen_test]
fn wasm_module_support_contract_is_stable() {
    let blocked_numpy = wasm_module_support("numpy");
    assert_eq!(blocked_numpy.module(), "numpy");
    assert!(!blocked_numpy.supported());
    assert_eq!(
        blocked_numpy
            .blocker_key()
            .expect("numpy should expose blocker key"),
        "dynamic_library_load".to_string()
    );
    let numpy_message = blocked_numpy
        .message()
        .expect("numpy should expose blocker message");
    assert!(numpy_message.contains("dynamic_library_load"));

    let blocked_socket = wasm_module_support("socket");
    assert!(!blocked_socket.supported());
    assert_eq!(
        blocked_socket
            .blocker_key()
            .expect("socket should expose blocker key"),
        "network_sockets".to_string()
    );

    let neutral_math = wasm_module_support("math");
    assert_eq!(neutral_math.module(), "math");
    assert!(neutral_math.supported());
    assert!(neutral_math.blocker_key().is_none());
    assert!(neutral_math.message().is_none());
}

#[wasm_bindgen_test]
fn wasm_module_policy_entries_are_stable() {
    let entries = wasm_module_policy_entries();
    let mut mappings = HashSet::new();
    for index in 0..entries.length() {
        let entry = entries.get(index);
        let module = Reflect::get(&entry, &"module".into())
            .expect("entry.module")
            .as_string()
            .expect("entry.module string");
        let blocker_key = Reflect::get(&entry, &"blocker_key".into())
            .expect("entry.blocker_key")
            .as_string()
            .expect("entry.blocker_key string");
        mappings.insert((module, blocker_key));
    }

    assert!(mappings.contains(&("numpy".to_string(), "dynamic_library_load".to_string())));
    assert!(mappings.contains(&("socket".to_string(), "network_sockets".to_string())));
    assert!(mappings.contains(&("subprocess".to_string(), "process_spawn".to_string())));
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
    assert_eq!(unsupported.line(), 0);
    assert_eq!(unsupported.column(), 0);

    let compile_error = execute("return 1\n");
    assert!(!compile_error.success());
    assert_eq!(compile_error.phase(), "compile_error");
    assert!(compile_error.stderr().contains("outside function"));
    assert!(compile_error.line() > 0);
    assert!(compile_error.column() > 0);

    let syntax_error = execute("def broken(:\n");
    assert!(!syntax_error.success());
    assert_eq!(syntax_error.phase(), "syntax_error");
    assert!(syntax_error.line() > 0);
    assert!(syntax_error.column() > 0);
}

#[wasm_bindgen_test]
fn wasm_session_tracks_and_resets_state() {
    let mut session = WasmSession::new();
    assert_eq!(session.snippets_checked(), 0);
    assert!(session.last_error().is_none());

    let first = session.check_syntax("x = 1\n");
    assert!(first.ok());
    assert_eq!(session.snippets_checked(), 1);

    let second = session.check_compile("x = 1\n");
    assert!(second.ok());
    assert_eq!(second.phase(), "ok");
    assert_eq!(session.snippets_checked(), 2);

    let third = session.execute("x = 1\n");
    assert_eq!(third.phase(), "unsupported_execution");
    assert_eq!(session.snippets_checked(), 3);
    assert!(session.last_error().is_some());

    let fourth = session.check_compile("return 1\n");
    assert!(!fourth.ok());
    assert_eq!(fourth.phase(), "compile_error");
    assert_eq!(session.snippets_checked(), 4);

    let fifth = session.execute("x = 1\n");
    assert_eq!(fifth.phase(), "unsupported_execution");
    assert_eq!(session.snippets_checked(), 5);
    assert!(session.last_error().is_some());

    session.reset();
    assert_eq!(session.snippets_checked(), 0);
    assert!(session.last_error().is_none());
}

#[wasm_bindgen_test]
fn wasm_session_execute_contract_is_stable() {
    let mut session = WasmSession::new();
    let second = session.execute("x = 1\n");
    assert_eq!(second.phase(), "unsupported_execution");
    assert_eq!(session.snippets_checked(), 1);
    assert!(session.last_error().is_some());
}
