#![cfg(target_arch = "wasm32")]

#[path = "fixtures/wasm_contract_snippets.rs"]
mod wasm_contract_snippets;
#[path = "fixtures/wasm_worker_contract.rs"]
mod wasm_worker_contract;

use crate::wasm_contract_snippets::WASM_CONTRACT_SNIPPET_FIXTURES;
use crate::wasm_worker_contract::{
    WASM_WORKER_EXECUTE_FIXTURES, WASM_WORKER_EXECUTE_PHASE_KEYS, WASM_WORKER_LIFECYCLE_FIXTURES,
    WASM_WORKER_LIFECYCLE_PHASE_KEYS, WASM_WORKER_STATE_KEYS, WASM_WORKER_TIMEOUT_FIXTURES,
    WASM_WORKER_TIMEOUT_PHASE_KEYS,
};
use js_sys::Reflect;
use pyrs::wasm::{
    check_compile_result, check_syntax_result, execute, wasm_api_version, wasm_capabilities,
    wasm_capability_error, wasm_capability_keys, wasm_execution_blocker_error,
    wasm_execution_blocker_keys, wasm_execution_blockers, wasm_module_policy_entries,
    wasm_module_support, wasm_runtime_info, wasm_snippet_blockers, wasm_snippet_import_roots,
    wasm_snippet_support, wasm_worker_blocker_error, wasm_worker_blocker_keys,
    wasm_worker_blockers, wasm_worker_execute, wasm_worker_execute_phase_keys,
    wasm_worker_execute_with_operation, wasm_worker_info, wasm_worker_lifecycle_phase_keys,
    wasm_worker_recycle, wasm_worker_set_timeout, wasm_worker_start, wasm_worker_state_keys,
    wasm_worker_terminate, wasm_worker_timeout_phase_keys, wasm_worker_timeout_policy, WasmSession,
    WasmWorkerSession,
};
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
fn wasm_worker_contract_basics() {
    let info = wasm_worker_info();
    assert!(!info.supported());
    assert_eq!(info.state(), "unwired");
    assert_eq!(info.interruption_model(), "worker_recycle");

    let keys = wasm_worker_blocker_keys();
    assert_eq!(keys.length(), info.blocker_count() as u32);
    assert_eq!(
        keys.get(0).as_string().expect("worker blocker key"),
        "worker_runtime_unwired".to_string()
    );

    let message = wasm_worker_blocker_error("worker_runtime_unwired")
        .expect("worker runtime blocker message should exist");
    assert!(message.contains("not wired"));

    let blockers = wasm_worker_blockers();
    assert_eq!(blockers.length(), keys.length());
    let first = blockers.get(0);
    let first_key = Reflect::get(&first, &"key".into())
        .expect("worker blocker.key")
        .as_string()
        .expect("worker blocker.key as string");
    let first_message = Reflect::get(&first, &"message".into())
        .expect("worker blocker.message")
        .as_string()
        .expect("worker blocker.message as string");
    assert_eq!(first_key, "worker_runtime_unwired");
    assert!(first_message.contains("not wired"));
}

#[wasm_bindgen_test]
fn wasm_worker_timeout_policy_contract_is_stable() {
    let policy = wasm_worker_timeout_policy();
    assert_eq!(policy.default_timeout_ms(), 5_000);
    assert_eq!(policy.min_timeout_ms(), 50);
    assert_eq!(policy.max_timeout_ms(), 120_000);
    assert!(policy.recycle_on_timeout());
    assert!(!policy.enforcement_supported());
    assert_eq!(
        policy.unsupported_phase(),
        "unsupported_worker_timeout_enforcement".to_string()
    );
    let reason = policy
        .unsupported_reason()
        .expect("timeout policy should expose unsupported reason");
    assert!(reason.contains("not wired"));
}

#[wasm_bindgen_test]
fn wasm_worker_timeout_set_contract_is_stable() {
    let mut operation_ids = HashSet::new();
    for fixture in WASM_WORKER_TIMEOUT_FIXTURES {
        let result = wasm_worker_set_timeout(fixture.timeout_ms);
        assert_eq!(
            result.phase(),
            fixture.expected_phase,
            "worker timeout phase mismatch: {}",
            fixture.name
        );
        assert_eq!(
            result.state(),
            fixture.expected_state,
            "worker timeout state mismatch: {}",
            fixture.name
        );
        assert_eq!(
            result.success(),
            fixture.expected_success,
            "worker timeout success mismatch: {}",
            fixture.name
        );
        assert_eq!(
            result.timeout_ms(),
            fixture.timeout_ms,
            "worker timeout value mismatch: {}",
            fixture.name
        );
        let expected_blocker_key = fixture.expected_blocker_key.map(str::to_string);
        assert_eq!(
            result.blocker_key(),
            expected_blocker_key,
            "worker timeout blocker key mismatch: {}",
            fixture.name
        );
        assert!(
            result.error().is_some(),
            "worker timeout error should be populated: {}",
            fixture.name
        );
        let message = result
            .error()
            .expect("worker timeout error message should be populated");
        if fixture.expected_phase == "invalid_worker_timeout" {
            assert!(
                message.contains("between"),
                "invalid timeout error should include range details: {}",
                fixture.name
            );
        } else {
            assert!(
                message.contains("not wired"),
                "unsupported timeout error should include unwired details: {}",
                fixture.name
            );
        }
        let operation_id = result.operation_id();
        assert!(
            operation_id.starts_with(fixture.expected_operation_prefix),
            "worker timeout operation id prefix mismatch: {}",
            fixture.name
        );
        assert!(
            operation_ids.insert(operation_id),
            "worker timeout operation ids should be unique: {}",
            fixture.name
        );
    }
}

#[wasm_bindgen_test]
fn wasm_worker_enum_keys_are_stable() {
    let state_keys = wasm_worker_state_keys();
    let mut states = HashSet::new();
    for index in 0..state_keys.length() {
        let state = state_keys
            .get(index)
            .as_string()
            .expect("worker state key should be string");
        states.insert(state);
    }
    let expected_states: HashSet<String> = WASM_WORKER_STATE_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    assert_eq!(states, expected_states);

    let lifecycle_keys = wasm_worker_lifecycle_phase_keys();
    let mut lifecycle_phases = HashSet::new();
    for index in 0..lifecycle_keys.length() {
        let phase = lifecycle_keys
            .get(index)
            .as_string()
            .expect("worker lifecycle phase key should be string");
        lifecycle_phases.insert(phase);
    }
    let expected_phases: HashSet<String> = WASM_WORKER_LIFECYCLE_PHASE_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    assert_eq!(lifecycle_phases, expected_phases);

    let execute_keys = wasm_worker_execute_phase_keys();
    let mut execute_phases = HashSet::new();
    for index in 0..execute_keys.length() {
        let phase = execute_keys
            .get(index)
            .as_string()
            .expect("worker execute phase key should be string");
        execute_phases.insert(phase);
    }
    let expected_execute_phases: HashSet<String> = WASM_WORKER_EXECUTE_PHASE_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    assert_eq!(execute_phases, expected_execute_phases);

    let timeout_keys = wasm_worker_timeout_phase_keys();
    let mut timeout_phases = HashSet::new();
    for index in 0..timeout_keys.length() {
        let phase = timeout_keys
            .get(index)
            .as_string()
            .expect("worker timeout phase key should be string");
        timeout_phases.insert(phase);
    }
    let expected_timeout_phases: HashSet<String> = WASM_WORKER_TIMEOUT_PHASE_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    assert_eq!(timeout_phases, expected_timeout_phases);
}

#[wasm_bindgen_test]
fn wasm_worker_lifecycle_stub_contract_is_stable() {
    let mut operation_ids = HashSet::new();
    for fixture in WASM_WORKER_LIFECYCLE_FIXTURES {
        let result = match fixture.action {
            "start" => wasm_worker_start(),
            "terminate" => wasm_worker_terminate(),
            "recycle" => wasm_worker_recycle(),
            other => panic!("unknown worker fixture action: {other}"),
        };

        assert_eq!(
            result.phase(),
            fixture.expected_phase,
            "worker lifecycle phase mismatch: {}",
            fixture.name
        );
        assert_eq!(
            result.state(),
            fixture.expected_state,
            "worker lifecycle state mismatch: {}",
            fixture.name
        );
        assert_eq!(
            result.success(),
            fixture.expected_success,
            "worker lifecycle success mismatch: {}",
            fixture.name
        );
        assert_eq!(
            result
                .blocker_key()
                .expect("worker lifecycle blocker key should be present"),
            fixture.expected_blocker_key.to_string(),
            "worker lifecycle blocker key mismatch: {}",
            fixture.name
        );
        let error = result
            .error()
            .expect("worker lifecycle error should be present");
        assert!(
            error.contains("not wired"),
            "worker lifecycle error mismatch: {}",
            fixture.name
        );
        let operation_id = result.operation_id();
        assert!(
            operation_id.starts_with(fixture.expected_operation_prefix),
            "worker lifecycle operation id prefix mismatch: {}",
            fixture.name
        );
        assert!(
            operation_ids.insert(operation_id),
            "worker lifecycle operation ids should be unique: {}",
            fixture.name
        );
    }
}

#[wasm_bindgen_test]
fn wasm_worker_operation_id_shape_is_stable() {
    let mut ids = HashSet::new();
    for fixture in WASM_WORKER_LIFECYCLE_FIXTURES {
        let result = match fixture.action {
            "start" => wasm_worker_start(),
            "terminate" => wasm_worker_terminate(),
            "recycle" => wasm_worker_recycle(),
            other => panic!("unknown worker fixture action: {other}"),
        };
        let operation_id = result.operation_id();
        assert!(operation_id.starts_with(fixture.expected_operation_prefix));
        assert!(ids.insert(operation_id));
    }

    for fixture in WASM_WORKER_TIMEOUT_FIXTURES {
        let result = wasm_worker_set_timeout(fixture.timeout_ms);
        let operation_id = result.operation_id();
        assert!(operation_id.starts_with(fixture.expected_operation_prefix));
        assert!(ids.insert(operation_id));
    }

    let execute = wasm_worker_execute_with_operation("x = 1\n");
    let execute_id = execute.operation_id();
    assert!(execute_id.starts_with(WASM_WORKER_EXECUTE_FIXTURES[0].expected_operation_prefix));
    assert!(ids.insert(execute_id));
}

#[wasm_bindgen_test]
fn wasm_worker_execute_stub_contract_is_stable() {
    for fixture in WASM_WORKER_EXECUTE_FIXTURES {
        let result = wasm_worker_execute(fixture.source);
        assert_eq!(
            result.phase(),
            fixture.expected_phase,
            "worker execute phase mismatch: {}",
            fixture.name
        );
        assert!(
            !result.success(),
            "worker execute success mismatch: {}",
            fixture.name
        );
        assert_eq!(
            result.error().is_some(),
            fixture.expect_error,
            "worker execute error mismatch: {}",
            fixture.name
        );
        if fixture.expect_line_column {
            assert!(
                result.line() > 0 && result.column() > 0,
                "worker execute line/column mismatch: {}",
                fixture.name
            );
        } else {
            assert_eq!(
                result.line(),
                0,
                "worker execute line mismatch: {}",
                fixture.name
            );
            assert_eq!(
                result.column(),
                0,
                "worker execute column mismatch: {}",
                fixture.name
            );
        }
    }
}

#[wasm_bindgen_test]
fn wasm_worker_execute_with_operation_contract_is_stable() {
    let mut operation_ids = HashSet::new();
    for fixture in WASM_WORKER_EXECUTE_FIXTURES {
        let result = wasm_worker_execute_with_operation(fixture.source);
        assert_eq!(
            result.phase(),
            fixture.expected_phase,
            "worker execute-with-operation phase mismatch: {}",
            fixture.name
        );
        let operation_id = result.operation_id();
        assert!(
            operation_id.starts_with(fixture.expected_operation_prefix),
            "worker execute operation id prefix mismatch: {}",
            fixture.name
        );
        assert!(
            operation_ids.insert(operation_id),
            "worker execute operation ids should be unique: {}",
            fixture.name
        );
    }
}

#[wasm_bindgen_test]
fn wasm_worker_session_contract_is_stable() {
    let mut session = WasmWorkerSession::new();
    assert_eq!(session.starts_requested(), 0);
    assert_eq!(session.terminates_requested(), 0);
    assert_eq!(session.recycles_requested(), 0);
    assert_eq!(session.executes_requested(), 0);
    assert_eq!(session.timeout_updates_requested(), 0);
    assert!(session.last_timeout_ms_requested().is_none());
    assert!(session.last_operation_id().is_none());
    assert!(session.last_phase().is_none());
    assert!(session.last_error().is_none());

    let info = session.info();
    assert!(!info.supported());
    assert_eq!(info.state(), "unwired");

    let start = session.start();
    assert_eq!(start.phase(), "unsupported_worker_start");
    assert_eq!(session.starts_requested(), 1);
    assert_eq!(session.last_operation_id(), Some(start.operation_id()));
    assert_eq!(
        session
            .last_phase()
            .expect("last phase after worker start should exist"),
        "unsupported_worker_start".to_string()
    );

    let terminate = session.terminate();
    assert_eq!(terminate.phase(), "unsupported_worker_terminate");
    assert_eq!(session.terminates_requested(), 1);
    assert_eq!(session.last_operation_id(), Some(terminate.operation_id()));
    assert_eq!(
        session
            .last_phase()
            .expect("last phase after worker terminate should exist"),
        "unsupported_worker_terminate".to_string()
    );
    assert!(session.last_error().is_some());

    let recycle = session.recycle();
    assert_eq!(recycle.phase(), "unsupported_worker_recycle");
    assert_eq!(session.recycles_requested(), 1);
    assert_eq!(session.last_operation_id(), Some(recycle.operation_id()));
    assert_eq!(
        session
            .last_phase()
            .expect("last phase after worker recycle should exist"),
        "unsupported_worker_recycle".to_string()
    );
    assert!(session.last_error().is_some());

    let execute = session.execute("x = 1\n");
    assert_eq!(execute.phase(), "unsupported_worker_execution");
    assert_eq!(session.executes_requested(), 1);
    let execute_operation_id = session
        .last_operation_id()
        .expect("last operation id after worker execute should exist");
    assert!(execute_operation_id.starts_with("worker_execute_"));
    assert_eq!(
        session
            .last_phase()
            .expect("last phase after worker execute should exist"),
        "unsupported_worker_execution".to_string()
    );
    assert!(session.last_error().is_some());

    let invalid_timeout = session.set_timeout_ms(0);
    assert_eq!(
        invalid_timeout.phase(),
        "invalid_worker_timeout".to_string()
    );
    assert_eq!(session.timeout_updates_requested(), 1);
    assert_eq!(session.last_timeout_ms_requested(), Some(0));
    assert_eq!(
        session.last_operation_id(),
        Some(invalid_timeout.operation_id())
    );
    assert_eq!(
        session
            .last_phase()
            .expect("last phase after worker timeout update should exist"),
        "invalid_worker_timeout".to_string()
    );
    assert!(session.last_error().is_some());

    let timeout = session.set_timeout_ms(5_000);
    assert_eq!(
        timeout.phase(),
        "unsupported_worker_timeout_enforcement".to_string()
    );
    assert_eq!(session.timeout_updates_requested(), 2);
    assert_eq!(session.last_timeout_ms_requested(), Some(5_000));
    assert_eq!(session.last_operation_id(), Some(timeout.operation_id()));
    assert_eq!(
        session
            .last_phase()
            .expect("last phase after worker timeout update should exist"),
        "unsupported_worker_timeout_enforcement".to_string()
    );
    assert!(session.last_error().is_some());

    session.reset();
    assert_eq!(session.starts_requested(), 0);
    assert_eq!(session.terminates_requested(), 0);
    assert_eq!(session.recycles_requested(), 0);
    assert_eq!(session.executes_requested(), 0);
    assert_eq!(session.timeout_updates_requested(), 0);
    assert!(session.last_timeout_ms_requested().is_none());
    assert!(session.last_operation_id().is_none());
    assert!(session.last_phase().is_none());
    assert!(session.last_error().is_none());
}

#[wasm_bindgen_test]
fn wasm_worker_session_execute_with_operation_contract_is_stable() {
    let mut session = WasmWorkerSession::new();
    let first = session.execute_with_operation("x = 1\n");
    assert_eq!(
        first.phase(),
        "unsupported_worker_execution",
        "first worker execute-with-operation phase mismatch"
    );
    let first_id = first.operation_id();
    assert!(first_id.starts_with("worker_execute_"));
    assert_eq!(session.executes_requested(), 1);
    assert_eq!(session.last_operation_id(), Some(first_id.clone()));
    assert_eq!(
        session.last_phase(),
        Some("unsupported_worker_execution".to_string())
    );

    let second = session.execute_with_operation("def broken(:\n");
    assert_eq!(
        second.phase(),
        "syntax_error",
        "second worker execute-with-operation phase mismatch"
    );
    let second_id = second.operation_id();
    assert!(second_id.starts_with("worker_execute_"));
    assert_ne!(first_id, second_id);
    assert_eq!(session.executes_requested(), 2);
    assert_eq!(session.last_operation_id(), Some(second_id));
    assert_eq!(session.last_phase(), Some("syntax_error".to_string()));
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
    assert!(listed_keys.contains(&"vm_runtime_unavailable".to_string()));
    assert!(listed_keys.contains(&"filesystem_read".to_string()));
    let err = wasm_execution_blocker_error("execution_backend_unwired")
        .expect("backend blocker should have error");
    assert!(err.contains("not wired"));
    let vm_err = wasm_execution_blocker_error("vm_runtime_unavailable")
        .expect("vm runtime blocker should have error");
    assert!(vm_err.contains("not available"));
    let fs_err =
        wasm_execution_blocker_error("filesystem_read").expect("filesystem_read should be blocked");
    assert!(fs_err.contains("filesystem_read"));

    let blockers = wasm_execution_blockers();
    assert!(blockers.length() >= 3);
    let mut saw_backend = false;
    let mut saw_vm_runtime = false;
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
        } else if key == "vm_runtime_unavailable" {
            assert!(message.contains("not available"));
            saw_vm_runtime = true;
        }
    }
    assert!(saw_backend);
    assert!(saw_vm_runtime);
}

#[wasm_bindgen_test]
fn wasm_execution_blockers_match_capability_matrix() {
    let capabilities = wasm_capabilities();
    let mut expected = HashSet::new();
    expected.insert("execution_backend_unwired".to_string());
    expected.insert("vm_runtime_unavailable".to_string());
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
fn wasm_snippet_support_preflight_contract_is_stable() {
    let supported = wasm_snippet_support("import math\nx = 1\n");
    assert!(supported.supported());
    assert_eq!(supported.phase(), "supported");
    assert!(supported.error().is_none());
    assert_eq!(supported.blocker_count(), 0);
    assert!(supported.imported_module_count() >= 1);

    let blocked = wasm_snippet_support("import socket\nimport math\n");
    assert!(!blocked.supported());
    assert_eq!(blocked.phase(), "blocked_capability");
    assert_eq!(blocked.blocker_count(), 1);
    assert_eq!(
        blocked
            .first_blocker_module()
            .expect("socket blocker module"),
        "socket".to_string()
    );
    assert_eq!(
        blocked.first_blocker_key().expect("socket blocker key"),
        "network_sockets".to_string()
    );
    assert!(blocked.first_blocker_message().is_some());

    let compile_error = wasm_snippet_support("return 1\n");
    assert!(!compile_error.supported());
    assert_eq!(compile_error.phase(), "compile_error");
    assert!(compile_error.error().is_some());
    assert!(compile_error.line() > 0);
    assert!(compile_error.column() > 0);

    let syntax_error = wasm_snippet_support("def broken(:\n");
    assert!(!syntax_error.supported());
    assert_eq!(syntax_error.phase(), "syntax_error");
    assert!(syntax_error.error().is_some());
    assert!(syntax_error.line() > 0);
    assert!(syntax_error.column() > 0);
}

#[wasm_bindgen_test]
fn wasm_snippet_blockers_contract_is_stable() {
    let blockers = wasm_snippet_blockers("import socket\nimport ctypes\n");
    assert_eq!(blockers.length(), 2);
    let first = blockers.get(0);
    let first_module = Reflect::get(&first, &"module".into())
        .expect("first.module")
        .as_string()
        .expect("first.module string");
    let first_key = Reflect::get(&first, &"blocker_key".into())
        .expect("first.blocker_key")
        .as_string()
        .expect("first.blocker_key string");
    assert_eq!(first_module, "socket");
    assert_eq!(first_key, "network_sockets");

    let none = wasm_snippet_blockers("def broken(:\n");
    assert_eq!(none.length(), 0);
}

#[wasm_bindgen_test]
fn wasm_snippet_import_roots_contract_is_stable() {
    let roots = wasm_snippet_import_roots(
        "import socket\nfrom math import sin\nimport socket\nfrom numpy.linalg import norm\n",
    );
    let mut values = Vec::new();
    for index in 0..roots.length() {
        let root = roots
            .get(index)
            .as_string()
            .expect("snippet import root should be string");
        values.push(root);
    }
    assert_eq!(
        values,
        vec![
            "socket".to_string(),
            "math".to_string(),
            "numpy".to_string()
        ]
    );

    let invalid = wasm_snippet_import_roots("def broken(:\n");
    assert_eq!(invalid.length(), 0);
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

#[wasm_bindgen_test]
fn wasm_contract_snippet_fixtures_are_current() {
    for fixture in WASM_CONTRACT_SNIPPET_FIXTURES {
        let compile = check_compile_result(fixture.source);
        assert_eq!(
            compile.phase(),
            fixture.expected_compile_phase,
            "fixture compile phase mismatch: {}",
            fixture.name
        );

        let execution = execute(fixture.source);
        assert_eq!(
            execution.phase(),
            fixture.expected_execute_phase,
            "fixture execute phase mismatch: {}",
            fixture.name
        );

        let support = wasm_snippet_support(fixture.source);
        assert_eq!(
            support.phase(),
            fixture.expected_support_phase,
            "fixture support phase mismatch: {}",
            fixture.name
        );

        let blocker_key = support.first_blocker_key();
        let expected_key = fixture.expected_first_blocker_key.map(str::to_string);
        assert_eq!(
            blocker_key, expected_key,
            "fixture blocker key mismatch: {}",
            fixture.name
        );
    }
}
