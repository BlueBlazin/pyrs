#![cfg(target_arch = "wasm32")]

#[path = "fixtures/wasm_capability_matrix.rs"]
mod wasm_capability_matrix;
#[path = "fixtures/wasm_contract_snippets.rs"]
mod wasm_contract_snippets;
#[path = "fixtures/wasm_module_policy.rs"]
mod wasm_module_policy;
#[path = "fixtures/wasm_worker_contract.rs"]
mod wasm_worker_contract;

use crate::wasm_capability_matrix::WASM_CAPABILITY_FIXTURES;
use crate::wasm_contract_snippets::{
    WASM_CONTRACT_SNIPPET_FIXTURES, WASM_EXECUTION_PHASE_KEYS, WASM_SUPPORT_PHASE_KEYS,
    WasmContractSnippetFixture,
};
use crate::wasm_module_policy::WASM_MODULE_POLICY_FIXTURES;
use crate::wasm_worker_contract::{
    WASM_WORKER_BLOCKER_KEYS, WASM_WORKER_EXECUTE_FIXTURES, WASM_WORKER_EXECUTE_PHASE_KEYS,
    WASM_WORKER_INFO_FIXTURES, WASM_WORKER_LIFECYCLE_FIXTURES, WASM_WORKER_LIFECYCLE_PHASE_KEYS,
    WASM_WORKER_LIFECYCLE_PHASE_KEYS_VM_PROBE_EXTRA, WASM_WORKER_STATE_KEYS,
    WASM_WORKER_TIMEOUT_FIXTURES, WASM_WORKER_TIMEOUT_PHASE_KEYS, WasmWorkerExecuteFixture,
};
use js_sys::Reflect;
use pyrs::wasm::{
    WasmCapabilityReport, WasmSession, WasmWorkerSession, check_compile_result,
    check_syntax_result, execute, wasm_api_version, wasm_capabilities, wasm_capability_error,
    wasm_capability_keys, wasm_execution_blocker_error, wasm_execution_blocker_keys,
    wasm_execution_blockers, wasm_execution_phase_keys, wasm_module_policy_entries,
    wasm_module_support, wasm_runtime_info, wasm_snippet_blockers, wasm_snippet_import_roots,
    wasm_snippet_support, wasm_worker_blocker_error, wasm_worker_blocker_keys,
    wasm_worker_blockers, wasm_worker_execute, wasm_worker_execute_phase_keys,
    wasm_worker_execute_with_operation, wasm_worker_info, wasm_worker_lifecycle_phase_keys,
    wasm_worker_recycle, wasm_worker_set_timeout, wasm_worker_start, wasm_worker_state_keys,
    wasm_worker_terminate, wasm_worker_timeout_phase_keys, wasm_worker_timeout_policy,
};
use std::collections::HashSet;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn capability_supported(report: &WasmCapabilityReport, key: &str) -> bool {
    match key {
        "filesystem_read" => report.filesystem_read(),
        "filesystem_write" => report.filesystem_write(),
        "environment_read" => report.environment_read(),
        "process_args" => report.process_args(),
        "clock_time" => report.clock_time(),
        "thread_sleep" => report.thread_sleep(),
        "process_spawn" => report.process_spawn(),
        "dynamic_library_load" => report.dynamic_library_load(),
        "interactive_terminal" => report.interactive_terminal(),
        "network_sockets" => report.network_sockets(),
        other => panic!("unknown capability key fixture: {other}"),
    }
}

fn vm_probe_enabled() -> bool {
    cfg!(feature = "wasm-vm-probe")
}

fn expected_execution_phase_keys() -> Vec<String> {
    let mut expected: Vec<String> = WASM_EXECUTION_PHASE_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    if vm_probe_enabled() {
        expected.push("ok".to_string());
        expected.push("runtime_error".to_string());
    }
    expected
}

fn expected_execution_blocker_keys(capabilities: &WasmCapabilityReport) -> Vec<String> {
    let mut expected = Vec::new();
    if !vm_probe_enabled() {
        expected.push("execution_backend_unwired".to_string());
        expected.push("vm_runtime_unavailable".to_string());
    }
    for fixture in WASM_CAPABILITY_FIXTURES {
        if !capability_supported(capabilities, fixture.key) {
            expected.push(fixture.key.to_string());
        }
    }
    expected
}

fn expected_execute_phase_for_fixture(fixture: &WasmContractSnippetFixture) -> String {
    if vm_probe_enabled() {
        if let Some(phase) = fixture.expected_vm_probe_execute_phase {
            return phase.to_string();
        }
    }
    fixture.expected_execute_phase.to_string()
}

fn expected_execute_blocker_key_for_fixture(
    fixture: &WasmContractSnippetFixture,
) -> Option<String> {
    if vm_probe_enabled() {
        if let Some(override_blocker) = fixture.expected_vm_probe_execute_blocker_key {
            return override_blocker.map(str::to_string);
        }
    }
    fixture.expected_execute_blocker_key.map(str::to_string)
}

fn expected_worker_execute_phase_for_fixture(fixture: &WasmWorkerExecuteFixture) -> String {
    if vm_probe_enabled() {
        if let Some(phase) = fixture.expected_vm_probe_phase {
            return phase.to_string();
        }
    }
    fixture.expected_phase.to_string()
}

fn expected_worker_lifecycle_phase_for_fixture(
    fixture: &wasm_worker_contract::WasmWorkerLifecycleFixture,
) -> String {
    if vm_probe_enabled() {
        if let Some(phase) = fixture.expected_vm_probe_phase {
            return phase.to_string();
        }
    }
    fixture.expected_phase.to_string()
}

fn expected_worker_lifecycle_state_for_fixture(
    fixture: &wasm_worker_contract::WasmWorkerLifecycleFixture,
) -> String {
    if vm_probe_enabled() {
        if let Some(state) = fixture.expected_vm_probe_state {
            return state.to_string();
        }
    }
    fixture.expected_state.to_string()
}

fn expected_worker_lifecycle_success_for_fixture(
    fixture: &wasm_worker_contract::WasmWorkerLifecycleFixture,
) -> bool {
    if vm_probe_enabled() {
        if let Some(success) = fixture.expected_vm_probe_success {
            return success;
        }
    }
    fixture.expected_success
}

fn expected_worker_lifecycle_blocker_key_for_fixture(
    fixture: &wasm_worker_contract::WasmWorkerLifecycleFixture,
) -> Option<String> {
    if vm_probe_enabled() {
        if let Some(override_blocker) = fixture.expected_vm_probe_blocker_key {
            return override_blocker.map(str::to_string);
        }
    }
    fixture.expected_blocker_key.map(str::to_string)
}

fn expected_session_state_after_recycle() -> String {
    expected_worker_lifecycle_state_for_fixture(&WASM_WORKER_LIFECYCLE_FIXTURES[2])
}

fn expected_worker_execute_blocker_key_for_fixture(
    fixture: &WasmWorkerExecuteFixture,
) -> Option<String> {
    if vm_probe_enabled() {
        if let Some(override_blocker) = fixture.expected_vm_probe_blocker_key {
            return override_blocker.map(str::to_string);
        }
    }
    fixture.expected_blocker_key.map(str::to_string)
}

fn expected_worker_execute_expect_error_for_fixture(fixture: &WasmWorkerExecuteFixture) -> bool {
    if vm_probe_enabled() {
        if let Some(expect_error) = fixture.expected_vm_probe_expect_error {
            return expect_error;
        }
    }
    fixture.expect_error
}

fn expected_worker_execute_success_for_fixture(fixture: &WasmWorkerExecuteFixture) -> bool {
    if vm_probe_enabled() {
        if let Some(expected_success) = fixture.expected_vm_probe_success {
            return expected_success;
        }
    }
    fixture.expected_success
}

fn expected_worker_execute_line_column_for_fixture(fixture: &WasmWorkerExecuteFixture) -> bool {
    if vm_probe_enabled() {
        if let Some(expect_line_column) = fixture.expected_vm_probe_expect_line_column {
            return expect_line_column;
        }
    }
    fixture.expect_line_column
}

fn expected_worker_info_backend_for_fixture(
    fixture: &wasm_worker_contract::WasmWorkerInfoFixture,
) -> String {
    if vm_probe_enabled() {
        if let Some(backend) = fixture.expected_vm_probe_backend {
            return backend.to_string();
        }
    }
    fixture.expected_backend.to_string()
}

fn expected_worker_info_execution_probe_enabled_for_fixture(
    fixture: &wasm_worker_contract::WasmWorkerInfoFixture,
) -> bool {
    if vm_probe_enabled() {
        if let Some(enabled) = fixture.expected_vm_probe_execution_probe_enabled {
            return enabled;
        }
    }
    fixture.expected_execution_probe_enabled
}

fn expected_worker_info_execute_supported_for_fixture(
    fixture: &wasm_worker_contract::WasmWorkerInfoFixture,
) -> bool {
    if vm_probe_enabled() {
        if let Some(enabled) = fixture.expected_vm_probe_execute_supported {
            return enabled;
        }
    }
    fixture.expected_execute_supported
}

#[wasm_bindgen_test]
fn wasm_runtime_contract_basics() {
    let runtime = wasm_runtime_info();
    assert_eq!(runtime.api_version(), wasm_api_version());
    assert!(runtime.supports_parse_compile());
    if vm_probe_enabled() {
        assert_eq!(runtime.execution_backend(), "vm_probe".to_string());
        assert_eq!(runtime.execution_status(), "runtime_probe");
        assert!(runtime.supports_execution());
    } else {
        assert_eq!(runtime.execution_backend(), "unwired".to_string());
        assert_eq!(runtime.execution_status(), "syntax_compile_only");
        assert!(!runtime.supports_execution());
    }
    let blocker_keys = wasm_execution_blocker_keys();
    assert_eq!(
        runtime.execution_blocker_count(),
        blocker_keys.length() as usize
    );
    assert!(!runtime.pyrs_version().is_empty());
}

#[wasm_bindgen_test]
fn wasm_execution_phase_keys_are_stable() {
    let keys = wasm_execution_phase_keys();
    let mut phases = Vec::new();
    for index in 0..keys.length() {
        let phase = keys
            .get(index)
            .as_string()
            .expect("execution phase key should be string");
        phases.push(phase);
    }
    let expected = expected_execution_phase_keys();
    assert_eq!(phases, expected);
    let phase_set: HashSet<String> = phases.iter().cloned().collect();

    let unsupported = execute("x = 1\n");
    assert!(phase_set.contains(&unsupported.phase()));

    let syntax = execute("def broken(:\n");
    assert!(phase_set.contains(&syntax.phase()));
}

#[wasm_bindgen_test]
fn wasm_worker_contract_basics() {
    let info = wasm_worker_info();
    let fixture = &WASM_WORKER_INFO_FIXTURES[0];
    assert_eq!(
        info.supported(),
        fixture.expected_supported,
        "worker info supported mismatch: {}",
        fixture.name
    );
    assert_eq!(
        info.backend(),
        expected_worker_info_backend_for_fixture(fixture),
        "worker info backend mismatch: {}",
        fixture.name
    );
    assert_eq!(
        info.state(),
        fixture.expected_state.to_string(),
        "worker info state mismatch: {}",
        fixture.name
    );
    assert_eq!(
        info.interruption_model(),
        fixture.expected_interruption_model.to_string(),
        "worker info interruption model mismatch: {}",
        fixture.name
    );
    assert_eq!(
        info.execution_probe_enabled(),
        expected_worker_info_execution_probe_enabled_for_fixture(fixture),
        "worker info execution_probe_enabled mismatch: {}",
        fixture.name
    );
    assert_eq!(
        info.execute_supported(),
        expected_worker_info_execute_supported_for_fixture(fixture),
        "worker info execute_supported mismatch: {}",
        fixture.name
    );

    let keys = wasm_worker_blocker_keys();
    assert_eq!(keys.length(), info.blocker_count() as u32);
    for (index, expected_key) in WASM_WORKER_BLOCKER_KEYS.iter().enumerate() {
        let key = keys
            .get(index as u32)
            .as_string()
            .expect("worker blocker key should be string");
        assert_eq!(
            key, *expected_key,
            "worker blocker key order mismatch at index {index}"
        );
    }
    let mut key_set = HashSet::new();
    for index in 0..keys.length() {
        let key = keys
            .get(index)
            .as_string()
            .expect("worker blocker key should be string");
        key_set.insert(key);
    }
    let expected_key_set: HashSet<String> = WASM_WORKER_BLOCKER_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    assert_eq!(key_set, expected_key_set);
    assert_eq!(
        keys.get(0).as_string().expect("worker blocker key"),
        "worker_runtime_unwired".to_string()
    );

    let message = wasm_worker_blocker_error("worker_runtime_unwired")
        .expect("worker runtime blocker message should exist");
    assert!(message.contains("not wired"));
    let network_message = wasm_worker_blocker_error("network_sockets")
        .expect("network socket blocker message should exist");
    assert!(network_message.contains("network_sockets"));

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
    let mut states = Vec::new();
    for index in 0..state_keys.length() {
        let state = state_keys
            .get(index)
            .as_string()
            .expect("worker state key should be string");
        states.push(state);
    }
    let expected_states: Vec<String> = WASM_WORKER_STATE_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    assert_eq!(states, expected_states);
    let state_set: HashSet<String> = states.iter().cloned().collect();
    assert_eq!(state_set.len(), expected_states.len());

    let lifecycle_keys = wasm_worker_lifecycle_phase_keys();
    let mut lifecycle_phases = Vec::new();
    for index in 0..lifecycle_keys.length() {
        let phase = lifecycle_keys
            .get(index)
            .as_string()
            .expect("worker lifecycle phase key should be string");
        lifecycle_phases.push(phase);
    }
    let expected_phases: Vec<String> = WASM_WORKER_LIFECYCLE_PHASE_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    let mut expected_phases = expected_phases;
    if vm_probe_enabled() {
        expected_phases.extend(
            WASM_WORKER_LIFECYCLE_PHASE_KEYS_VM_PROBE_EXTRA
                .iter()
                .map(|value| (*value).to_string()),
        );
    }
    assert_eq!(lifecycle_phases, expected_phases);
    let lifecycle_phase_set: HashSet<String> = lifecycle_phases.iter().cloned().collect();
    assert_eq!(lifecycle_phase_set.len(), expected_phases.len());

    let execute_keys = wasm_worker_execute_phase_keys();
    let mut execute_phases = Vec::new();
    for index in 0..execute_keys.length() {
        let phase = execute_keys
            .get(index)
            .as_string()
            .expect("worker execute phase key should be string");
        execute_phases.push(phase);
    }
    let mut expected_execute_phases: Vec<String> = WASM_WORKER_EXECUTE_PHASE_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    if vm_probe_enabled() {
        expected_execute_phases.push("ok".to_string());
        expected_execute_phases.push("runtime_error".to_string());
    }
    assert_eq!(execute_phases, expected_execute_phases);
    let execute_phase_set: HashSet<String> = execute_phases.iter().cloned().collect();
    assert_eq!(execute_phase_set.len(), expected_execute_phases.len());

    let timeout_keys = wasm_worker_timeout_phase_keys();
    let mut timeout_phases = Vec::new();
    for index in 0..timeout_keys.length() {
        let phase = timeout_keys
            .get(index)
            .as_string()
            .expect("worker timeout phase key should be string");
        timeout_phases.push(phase);
    }
    let expected_timeout_phases: Vec<String> = WASM_WORKER_TIMEOUT_PHASE_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    assert_eq!(timeout_phases, expected_timeout_phases);
    let timeout_phase_set: HashSet<String> = timeout_phases.iter().cloned().collect();
    assert_eq!(timeout_phase_set.len(), expected_timeout_phases.len());
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
            expected_worker_lifecycle_phase_for_fixture(fixture),
            "worker lifecycle phase mismatch: {}",
            fixture.name
        );
        assert_eq!(
            result.state(),
            expected_worker_lifecycle_state_for_fixture(fixture),
            "worker lifecycle state mismatch: {}",
            fixture.name
        );
        assert_eq!(
            result.success(),
            expected_worker_lifecycle_success_for_fixture(fixture),
            "worker lifecycle success mismatch: {}",
            fixture.name
        );
        let expected_blocker = expected_worker_lifecycle_blocker_key_for_fixture(fixture);
        assert_eq!(
            result.blocker_key(),
            expected_blocker,
            "worker lifecycle blocker key mismatch: {}",
            fixture.name
        );
        if vm_probe_enabled() {
            assert!(
                result.error().is_none(),
                "worker lifecycle vm-probe result should not have error: {}",
                fixture.name
            );
        } else {
            let error = result
                .error()
                .expect("worker lifecycle error should be present");
            assert!(
                error.contains("not wired"),
                "worker lifecycle error mismatch: {}",
                fixture.name
            );
        }
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
        let expected_phase = expected_worker_execute_phase_for_fixture(fixture);
        assert_eq!(
            result.phase(),
            expected_phase,
            "worker execute phase mismatch: {}",
            fixture.name
        );
        let expected_success = expected_worker_execute_success_for_fixture(fixture);
        assert_eq!(
            result.success(),
            expected_success,
            "worker execute success mismatch: {}",
            fixture.name
        );
        let expected_error = expected_worker_execute_expect_error_for_fixture(fixture);
        assert_eq!(
            result.error().is_some(),
            expected_error,
            "worker execute error mismatch: {}",
            fixture.name
        );
        let expected_blocker_key = expected_worker_execute_blocker_key_for_fixture(fixture);
        assert_eq!(
            result.blocker_key(),
            expected_blocker_key,
            "worker execute blocker key mismatch: {}",
            fixture.name
        );
        if expected_worker_execute_line_column_for_fixture(fixture) {
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
        let expected_phase = expected_worker_execute_phase_for_fixture(fixture);
        assert_eq!(
            result.phase(),
            expected_phase,
            "worker execute-with-operation phase mismatch: {}",
            fixture.name
        );
        let operation_id = result.operation_id();
        assert!(
            operation_id.starts_with(fixture.expected_operation_prefix),
            "worker execute operation id prefix mismatch: {}",
            fixture.name
        );
        assert_eq!(
            result.state(),
            "unwired".to_string(),
            "worker execute-with-operation state mismatch: {}",
            fixture.name
        );
        assert!(
            operation_ids.insert(operation_id),
            "worker execute operation ids should be unique: {}",
            fixture.name
        );
        let expected_blocker_key = expected_worker_execute_blocker_key_for_fixture(fixture);
        assert_eq!(
            result.blocker_key(),
            expected_blocker_key,
            "worker execute-with-operation blocker key mismatch: {}",
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
    assert!(session.last_state().is_none());
    assert!(session.last_error().is_none());
    let initial_snapshot = session.snapshot();
    assert_eq!(initial_snapshot.starts_requested(), 0);
    assert_eq!(initial_snapshot.terminates_requested(), 0);
    assert_eq!(initial_snapshot.recycles_requested(), 0);
    assert_eq!(initial_snapshot.executes_requested(), 0);
    assert_eq!(initial_snapshot.timeout_updates_requested(), 0);
    assert!(initial_snapshot.last_timeout_ms_requested().is_none());
    assert!(initial_snapshot.last_operation_id().is_none());
    assert!(initial_snapshot.last_phase().is_none());
    assert!(initial_snapshot.last_state().is_none());
    assert!(initial_snapshot.last_error().is_none());

    let info = session.info();
    let fixture = &WASM_WORKER_INFO_FIXTURES[0];
    assert_eq!(
        info.supported(),
        fixture.expected_supported,
        "worker session info supported mismatch: {}",
        fixture.name
    );
    assert_eq!(
        info.backend(),
        expected_worker_info_backend_for_fixture(fixture),
        "worker session info backend mismatch: {}",
        fixture.name
    );
    assert_eq!(
        info.state(),
        fixture.expected_state.to_string(),
        "worker session info state mismatch: {}",
        fixture.name
    );
    assert_eq!(
        info.execution_probe_enabled(),
        expected_worker_info_execution_probe_enabled_for_fixture(fixture),
        "worker session info execution_probe_enabled mismatch: {}",
        fixture.name
    );
    assert_eq!(
        info.execute_supported(),
        expected_worker_info_execute_supported_for_fixture(fixture),
        "worker session info execute_supported mismatch: {}",
        fixture.name
    );

    let start = session.start();
    assert_eq!(
        start.phase(),
        expected_worker_lifecycle_phase_for_fixture(&WASM_WORKER_LIFECYCLE_FIXTURES[0])
    );
    assert_eq!(session.starts_requested(), 1);
    assert_eq!(session.last_operation_id(), Some(start.operation_id()));
    assert_eq!(
        session
            .last_phase()
            .expect("last phase after worker start should exist"),
        expected_worker_lifecycle_phase_for_fixture(&WASM_WORKER_LIFECYCLE_FIXTURES[0])
    );
    assert_eq!(
        session.last_state(),
        Some(expected_worker_lifecycle_state_for_fixture(
            &WASM_WORKER_LIFECYCLE_FIXTURES[0]
        ))
    );
    if vm_probe_enabled() {
        assert!(session.last_error().is_none());
    } else {
        assert!(session.last_error().is_some());
    }

    let terminate = session.terminate();
    assert_eq!(
        terminate.phase(),
        expected_worker_lifecycle_phase_for_fixture(&WASM_WORKER_LIFECYCLE_FIXTURES[1])
    );
    assert_eq!(session.terminates_requested(), 1);
    assert_eq!(session.last_operation_id(), Some(terminate.operation_id()));
    assert_eq!(
        session
            .last_phase()
            .expect("last phase after worker terminate should exist"),
        expected_worker_lifecycle_phase_for_fixture(&WASM_WORKER_LIFECYCLE_FIXTURES[1])
    );
    assert_eq!(
        session.last_state(),
        Some(expected_worker_lifecycle_state_for_fixture(
            &WASM_WORKER_LIFECYCLE_FIXTURES[1]
        ))
    );
    if vm_probe_enabled() {
        assert!(session.last_error().is_none());
    } else {
        assert!(session.last_error().is_some());
    }

    let recycle = session.recycle();
    assert_eq!(
        recycle.phase(),
        expected_worker_lifecycle_phase_for_fixture(&WASM_WORKER_LIFECYCLE_FIXTURES[2])
    );
    assert_eq!(session.recycles_requested(), 1);
    assert_eq!(session.last_operation_id(), Some(recycle.operation_id()));
    assert_eq!(
        session
            .last_phase()
            .expect("last phase after worker recycle should exist"),
        expected_worker_lifecycle_phase_for_fixture(&WASM_WORKER_LIFECYCLE_FIXTURES[2])
    );
    assert_eq!(
        session.last_state(),
        Some(expected_worker_lifecycle_state_for_fixture(
            &WASM_WORKER_LIFECYCLE_FIXTURES[2]
        ))
    );
    if vm_probe_enabled() {
        assert!(session.last_error().is_none());
    } else {
        assert!(session.last_error().is_some());
    }

    let execute = session.execute("x = 1\n");
    if vm_probe_enabled() {
        assert_eq!(execute.phase(), "ok");
        assert!(execute.blocker_key().is_none());
    } else {
        assert_eq!(execute.phase(), "unsupported_worker_execution");
        assert_eq!(
            execute.blocker_key(),
            Some("worker_runtime_unwired".to_string())
        );
    }
    assert_eq!(session.executes_requested(), 1);
    let execute_operation_id = session
        .last_operation_id()
        .expect("last operation id after worker execute should exist");
    assert!(execute_operation_id.starts_with("worker_execute_"));
    assert_eq!(session.last_state(), Some(expected_session_state_after_recycle()));
    if vm_probe_enabled() {
        assert_eq!(
            session
                .last_phase()
                .expect("last phase after worker execute should exist"),
            "ok".to_string()
        );
        assert!(session.last_error().is_none());
    } else {
        assert_eq!(
            session
                .last_phase()
                .expect("last phase after worker execute should exist"),
            "unsupported_worker_execution".to_string()
        );
        assert!(session.last_error().is_some());
    }

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
    assert_eq!(session.last_state(), Some(expected_session_state_after_recycle()));
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
    assert_eq!(session.last_state(), Some(expected_session_state_after_recycle()));
    assert!(session.last_error().is_some());
    let pre_reset_snapshot = session.snapshot();
    assert_eq!(pre_reset_snapshot.starts_requested(), 1);
    assert_eq!(pre_reset_snapshot.terminates_requested(), 1);
    assert_eq!(pre_reset_snapshot.recycles_requested(), 1);
    assert_eq!(pre_reset_snapshot.executes_requested(), 1);
    assert_eq!(pre_reset_snapshot.timeout_updates_requested(), 2);
    assert_eq!(pre_reset_snapshot.last_timeout_ms_requested(), Some(5_000));
    assert_eq!(
        pre_reset_snapshot.last_phase(),
        Some("unsupported_worker_timeout_enforcement".to_string())
    );
    assert_eq!(
        pre_reset_snapshot.last_state(),
        Some(expected_session_state_after_recycle())
    );
    assert!(pre_reset_snapshot.last_error().is_some());

    session.reset();
    assert_eq!(session.starts_requested(), 0);
    assert_eq!(session.terminates_requested(), 0);
    assert_eq!(session.recycles_requested(), 0);
    assert_eq!(session.executes_requested(), 0);
    assert_eq!(session.timeout_updates_requested(), 0);
    assert!(session.last_timeout_ms_requested().is_none());
    assert!(session.last_operation_id().is_none());
    assert!(session.last_phase().is_none());
    assert!(session.last_state().is_none());
    assert!(session.last_error().is_none());
    let reset_snapshot = session.snapshot();
    assert_eq!(reset_snapshot.starts_requested(), 0);
    assert_eq!(reset_snapshot.terminates_requested(), 0);
    assert_eq!(reset_snapshot.recycles_requested(), 0);
    assert_eq!(reset_snapshot.executes_requested(), 0);
    assert_eq!(reset_snapshot.timeout_updates_requested(), 0);
    assert!(reset_snapshot.last_timeout_ms_requested().is_none());
    assert!(reset_snapshot.last_operation_id().is_none());
    assert!(reset_snapshot.last_phase().is_none());
    assert!(reset_snapshot.last_state().is_none());
    assert!(reset_snapshot.last_error().is_none());
}

#[wasm_bindgen_test]
fn wasm_worker_session_execute_with_operation_contract_is_stable() {
    let mut session = WasmWorkerSession::new();
    let first = session.execute_with_operation("x = 1\n");
    if vm_probe_enabled() {
        assert_eq!(
            first.phase(),
            "ok",
            "first worker execute-with-operation phase mismatch"
        );
    } else {
        assert_eq!(
            first.phase(),
            "unsupported_worker_execution",
            "first worker execute-with-operation phase mismatch"
        );
    }
    let first_id = first.operation_id();
    assert!(first_id.starts_with("worker_execute_"));
    assert_eq!(first.state(), "unwired".to_string());
    assert_eq!(session.executes_requested(), 1);
    assert_eq!(session.last_operation_id(), Some(first_id.clone()));
    assert_eq!(session.last_state(), Some("unwired".to_string()));
    let first_snapshot = session.snapshot();
    assert_eq!(first_snapshot.executes_requested(), 1);
    assert_eq!(first_snapshot.last_operation_id(), Some(first_id.clone()));
    assert_eq!(first_snapshot.last_state(), Some("unwired".to_string()));
    if vm_probe_enabled() {
        assert_eq!(session.last_phase(), Some("ok".to_string()));
        assert!(first.blocker_key().is_none());
    } else {
        assert_eq!(
            session.last_phase(),
            Some("unsupported_worker_execution".to_string())
        );
        assert_eq!(
            first.blocker_key(),
            Some("worker_runtime_unwired".to_string())
        );
    }

    let second = session.execute_with_operation("def broken(:\n");
    assert_eq!(
        second.phase(),
        "syntax_error",
        "second worker execute-with-operation phase mismatch"
    );
    let second_id = second.operation_id();
    assert!(second_id.starts_with("worker_execute_"));
    assert_eq!(second.state(), "unwired".to_string());
    assert_ne!(first_id, second_id);
    assert_eq!(session.executes_requested(), 2);
    assert_eq!(session.last_operation_id(), Some(second_id));
    assert_eq!(session.last_phase(), Some("syntax_error".to_string()));
    assert_eq!(session.last_state(), Some("unwired".to_string()));
    assert!(second.blocker_key().is_none());
    let second_snapshot = session.snapshot();
    assert_eq!(second_snapshot.executes_requested(), 2);
    assert_eq!(
        second_snapshot.last_phase(),
        Some("syntax_error".to_string())
    );
    assert_eq!(second_snapshot.last_state(), Some("unwired".to_string()));
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
    let expected_keys: Vec<String> = WASM_CAPABILITY_FIXTURES
        .iter()
        .map(|fixture| fixture.key.to_string())
        .collect();
    assert_eq!(listed_keys, expected_keys);
    let key_set: HashSet<String> = listed_keys.iter().cloned().collect();
    assert_eq!(key_set.len(), expected_keys.len());

    let capabilities = wasm_capabilities();
    for fixture in WASM_CAPABILITY_FIXTURES {
        assert!(
            fixture.native_supported,
            "native fixture baseline should remain supported: {}",
            fixture.key
        );
        let supported = capability_supported(&capabilities, fixture.key);
        assert_eq!(
            supported, fixture.wasm_supported,
            "wasm capability mismatch for {}",
            fixture.key
        );
        let capability_error = wasm_capability_error(fixture.key);
        if fixture.wasm_supported {
            assert!(
                capability_error.is_none(),
                "supported capability should not have error: {}",
                fixture.key
            );
        } else {
            let message = capability_error
                .unwrap_or_else(|| panic!("unsupported capability missing error: {}", fixture.key));
            assert!(
                message.contains(fixture.key),
                "unsupported capability error should include key: {}",
                fixture.key
            );
        }
    }
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
    let capabilities = wasm_capabilities();
    let expected_keys = expected_execution_blocker_keys(&capabilities);
    assert_eq!(listed_keys, expected_keys);
    if !vm_probe_enabled() {
        let err = wasm_execution_blocker_error("execution_backend_unwired")
            .expect("backend blocker should have error");
        assert!(err.contains("not wired"));
        let vm_err = wasm_execution_blocker_error("vm_runtime_unavailable")
            .expect("vm runtime blocker should have error");
        assert!(vm_err.contains("not available"));
    }
    let fs_err =
        wasm_execution_blocker_error("filesystem_read").expect("filesystem_read should be blocked");
    assert!(fs_err.contains("filesystem_read"));

    let blockers = wasm_execution_blockers();
    assert_eq!(blockers.length() as usize, expected_keys.len());
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
    if vm_probe_enabled() {
        assert!(!saw_backend);
        assert!(!saw_vm_runtime);
    } else {
        assert!(saw_backend);
        assert!(saw_vm_runtime);
    }
}

#[wasm_bindgen_test]
fn wasm_execution_blockers_match_capability_matrix() {
    let capabilities = wasm_capabilities();
    let expected = expected_execution_blocker_keys(&capabilities);
    for fixture in WASM_CAPABILITY_FIXTURES {
        let supported = capability_supported(&capabilities, fixture.key);
        assert_eq!(
            supported, fixture.wasm_supported,
            "capability fixture/report mismatch for {}",
            fixture.key
        );
    }

    let keys = wasm_execution_blocker_keys();
    let mut listed_keys = Vec::new();
    for index in 0..keys.length() {
        let key = keys
            .get(index)
            .as_string()
            .expect("blocker key should be string");
        listed_keys.push(key);
    }
    assert_eq!(listed_keys, expected);
    let listed_key_set: HashSet<String> = listed_keys.iter().cloned().collect();
    assert_eq!(listed_key_set.len(), expected.len());

    for key in &listed_keys {
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
    let mut mappings = Vec::new();
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
        mappings.push((module, blocker_key));
    }

    let expected: Vec<(String, String)> = WASM_MODULE_POLICY_FIXTURES
        .iter()
        .map(|fixture| (fixture.module.to_string(), fixture.blocker_key.to_string()))
        .collect();
    assert_eq!(mappings, expected);
    let mapping_set: HashSet<(String, String)> = mappings.iter().cloned().collect();
    assert_eq!(mapping_set.len(), expected.len());
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
    if vm_probe_enabled() {
        assert!(unsupported.success());
        assert_eq!(unsupported.phase(), "ok");
        assert!(unsupported.error().is_none());
        assert!(unsupported.blocker_key().is_none());
    } else {
        assert!(!unsupported.success());
        assert_eq!(unsupported.phase(), "unsupported_execution");
        assert!(unsupported.error().is_some());
        assert_eq!(
            unsupported.blocker_key(),
            Some("execution_backend_unwired".to_string())
        );
        assert!(unsupported.stderr().contains("not wired"));
        assert_eq!(unsupported.line(), 0);
        assert_eq!(unsupported.column(), 0);
    }

    let blocked = execute("import socket\n");
    assert!(!blocked.success());
    assert_eq!(blocked.phase(), "unsupported_execution");
    assert_eq!(blocked.blocker_key(), Some("network_sockets".to_string()));
    assert!(blocked.error().is_some());
    assert!(blocked.stderr().contains("network_sockets"));

    let compile_error = execute("return 1\n");
    assert!(!compile_error.success());
    assert_eq!(compile_error.phase(), "compile_error");
    assert!(compile_error.blocker_key().is_none());
    assert!(compile_error.stderr().contains("outside function"));
    assert!(compile_error.line() > 0);
    assert!(compile_error.column() > 0);

    let syntax_error = execute("def broken(:\n");
    assert!(!syntax_error.success());
    assert_eq!(syntax_error.phase(), "syntax_error");
    assert!(syntax_error.blocker_key().is_none());
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
    if vm_probe_enabled() {
        assert_eq!(third.phase(), "ok");
        assert!(third.blocker_key().is_none());
    } else {
        assert_eq!(third.phase(), "unsupported_execution");
        assert_eq!(
            third.blocker_key(),
            Some("execution_backend_unwired".to_string())
        );
    }
    assert_eq!(session.snippets_checked(), 3);
    if vm_probe_enabled() {
        assert!(session.last_error().is_none());
    } else {
        assert!(session.last_error().is_some());
    }

    let fourth = session.check_compile("return 1\n");
    assert!(!fourth.ok());
    assert_eq!(fourth.phase(), "compile_error");
    assert_eq!(session.snippets_checked(), 4);

    let fifth = session.execute("x = 1\n");
    if vm_probe_enabled() {
        assert_eq!(fifth.phase(), "ok");
        assert!(fifth.blocker_key().is_none());
    } else {
        assert_eq!(fifth.phase(), "unsupported_execution");
        assert_eq!(
            fifth.blocker_key(),
            Some("execution_backend_unwired".to_string())
        );
    }
    assert_eq!(session.snippets_checked(), 5);
    if vm_probe_enabled() {
        assert!(session.last_error().is_none());
    } else {
        assert!(session.last_error().is_some());
    }

    session.reset();
    assert_eq!(session.snippets_checked(), 0);
    assert!(session.last_error().is_none());
}

#[wasm_bindgen_test]
fn wasm_session_execute_contract_is_stable() {
    let mut session = WasmSession::new();
    let second = session.execute("x = 1\n");
    if vm_probe_enabled() {
        assert_eq!(second.phase(), "ok");
        assert!(second.blocker_key().is_none());
    } else {
        assert_eq!(second.phase(), "unsupported_execution");
        assert_eq!(
            second.blocker_key(),
            Some("execution_backend_unwired".to_string())
        );
    }
    assert_eq!(session.snippets_checked(), 1);
    if vm_probe_enabled() {
        assert!(session.last_error().is_none());
    } else {
        assert!(session.last_error().is_some());
    }
}

#[wasm_bindgen_test]
fn wasm_contract_snippet_fixtures_are_current() {
    let support_phase_keys: HashSet<String> = WASM_SUPPORT_PHASE_KEYS
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    for fixture in WASM_CONTRACT_SNIPPET_FIXTURES {
        assert!(
            support_phase_keys.contains(fixture.expected_support_phase),
            "fixture support phase must be canonical: {}",
            fixture.name
        );
        let compile = check_compile_result(fixture.source);
        assert_eq!(
            compile.phase(),
            fixture.expected_compile_phase,
            "fixture compile phase mismatch: {}",
            fixture.name
        );

        let execution = execute(fixture.source);
        let expected_execute_phase = expected_execute_phase_for_fixture(fixture);
        assert_eq!(
            execution.phase(),
            expected_execute_phase,
            "fixture execute phase mismatch: {}",
            fixture.name
        );
        let expected_execute_blocker_key = expected_execute_blocker_key_for_fixture(fixture);
        assert_eq!(
            execution.blocker_key(),
            expected_execute_blocker_key,
            "fixture execute blocker key mismatch: {}",
            fixture.name
        );

        let support = wasm_snippet_support(fixture.source);
        assert_eq!(
            support.phase(),
            fixture.expected_support_phase,
            "fixture support phase mismatch: {}",
            fixture.name
        );
        assert!(
            support_phase_keys.contains(&support.phase()),
            "runtime support phase must be canonical: {}",
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

#[cfg(feature = "wasm-vm-probe")]
#[wasm_bindgen_test]
fn wasm_vm_probe_runtime_error_phase_is_reported() {
    let runtime_error = execute("1 / 0\n");
    assert!(!runtime_error.success());
    assert_eq!(runtime_error.phase(), "runtime_error");
    assert!(runtime_error.error().is_some());
    assert!(runtime_error.blocker_key().is_none());
    assert!(runtime_error.line() > 0);
    assert!(runtime_error.column() > 0);

    let worker_runtime_error = wasm_worker_execute("1 / 0\n");
    assert!(!worker_runtime_error.success());
    assert_eq!(worker_runtime_error.phase(), "runtime_error");
    assert!(worker_runtime_error.error().is_some());
    assert!(worker_runtime_error.blocker_key().is_none());
    assert!(worker_runtime_error.line() > 0);
    assert!(worker_runtime_error.column() > 0);
}
