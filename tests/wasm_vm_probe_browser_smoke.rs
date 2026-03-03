#![cfg(all(target_arch = "wasm32", feature = "wasm-vm-probe"))]

use pyrs::wasm::{
    WasmReplSession, WasmWorkerSession, execute, wasm_runtime_info, wasm_worker_current_timeout_ms,
    wasm_worker_execute_with_operation, wasm_worker_force_failed_state_for_tests, wasm_worker_info,
    wasm_worker_recycle, wasm_worker_set_timeout, wasm_worker_start, wasm_worker_terminate,
};
use wasm_bindgen_test::*;

#[wasm_bindgen_test]
fn vm_probe_runtime_executes_basic_snippet() {
    let info = wasm_runtime_info();
    assert_eq!(info.execution_backend(), "vm_probe".to_string());
    assert!(info.supports_execution());

    let result = execute("1 + 1");
    assert_eq!(result.phase(), "ok".to_string());
    assert!(result.success());
    assert!(result.stderr().is_empty());
}

#[wasm_bindgen_test]
fn vm_probe_repl_session_continuation_executes_class_on_blank_line() {
    let mut repl = WasmReplSession::new();

    let header = repl.execute_input("class Counter:");
    assert_eq!(header.phase(), "ok".to_string());
    assert!(header.success());
    assert!(header.stdout().is_empty());
    assert!(header.stderr().is_empty());

    let body = repl.execute_input("    value = 3");
    assert_eq!(body.phase(), "ok".to_string());
    assert!(body.success());
    assert!(body.stdout().is_empty());
    assert!(body.stderr().is_empty());

    let finalize = repl.execute_input("");
    assert_eq!(finalize.phase(), "ok".to_string());
    assert!(finalize.success());
    assert!(finalize.stdout().is_empty());
    assert!(finalize.stderr().is_empty());

    let expression = repl.execute_input("Counter.value");
    assert_eq!(expression.phase(), "ok".to_string());
    assert!(expression.success());
    assert_eq!(expression.stdout(), "3".to_string());
    assert!(expression.stderr().is_empty());
}

#[wasm_bindgen_test]
fn vm_probe_worker_state_gate_roundtrip() {
    let baseline = wasm_worker_info();
    assert_eq!(baseline.backend(), "vm_probe".to_string());
    assert_eq!(baseline.state(), "ready".to_string());
    assert!(baseline.execute_supported());

    let start = wasm_worker_start();
    assert_eq!(start.phase(), "worker_started".to_string());
    assert_eq!(start.state(), "ready".to_string());
    assert!(start.success());

    let configured = wasm_worker_set_timeout(5_000);
    assert_eq!(configured.phase(), "worker_timeout_configured".to_string());
    assert_eq!(configured.state(), "ready".to_string());
    assert!(configured.success());

    let terminate = wasm_worker_terminate();
    assert_eq!(terminate.phase(), "worker_terminated".to_string());
    assert_eq!(terminate.state(), "unwired".to_string());
    assert!(terminate.success());

    let blocked_timeout = wasm_worker_set_timeout(5_000);
    assert_eq!(
        blocked_timeout.phase(),
        "unsupported_worker_timeout_enforcement".to_string()
    );
    assert_eq!(blocked_timeout.state(), "unwired".to_string());
    assert!(!blocked_timeout.success());
    assert_eq!(
        blocked_timeout.blocker_key(),
        Some("worker_runtime_unwired".to_string())
    );
    assert_eq!(wasm_worker_info().state(), "unwired".to_string());

    let recycle = wasm_worker_recycle();
    assert_eq!(recycle.phase(), "worker_recycled".to_string());
    assert_eq!(recycle.state(), "ready".to_string());
    assert!(recycle.success());

    let resumed_timeout = wasm_worker_set_timeout(5_000);
    assert_eq!(
        resumed_timeout.phase(),
        "worker_timeout_configured".to_string()
    );
    assert_eq!(resumed_timeout.state(), "ready".to_string());
    assert!(resumed_timeout.success());
    assert_eq!(wasm_worker_info().state(), "ready".to_string());
}

#[wasm_bindgen_test]
fn vm_probe_worker_forced_failed_state_roundtrip() {
    let recycle = wasm_worker_recycle();
    assert_eq!(recycle.phase(), "worker_recycled".to_string());
    assert_eq!(recycle.state(), "ready".to_string());

    let assign = wasm_worker_execute_with_operation("x = 8\n");
    assert_eq!(assign.phase(), "ok".to_string());
    assert_eq!(assign.state(), "ready".to_string());
    assert!(assign.success());

    let forced = wasm_worker_force_failed_state_for_tests();
    assert_eq!(forced.phase(), "worker_failed_forced".to_string());
    assert_eq!(forced.state(), "failed".to_string());
    assert!(forced.success());
    assert_eq!(
        forced.blocker_key(),
        Some("worker_runtime_failed".to_string())
    );

    let blocked_execute = wasm_worker_execute_with_operation("x\n");
    assert_eq!(
        blocked_execute.phase(),
        "unsupported_worker_execution".to_string()
    );
    assert_eq!(blocked_execute.state(), "failed".to_string());
    assert!(!blocked_execute.success());
    assert_eq!(
        blocked_execute.blocker_key(),
        Some("worker_runtime_failed".to_string())
    );

    let restart = wasm_worker_start();
    assert_eq!(restart.phase(), "worker_started".to_string());
    assert_eq!(restart.state(), "ready".to_string());
    assert!(restart.success());

    let missing_after_start = wasm_worker_execute_with_operation("x\n");
    assert_eq!(missing_after_start.phase(), "runtime_error".to_string());
    assert_eq!(missing_after_start.state(), "ready".to_string());
    assert!(!missing_after_start.success());
    let err = missing_after_start
        .error()
        .expect("post-start execution should report NameError");
    assert!(err.contains("NameError"));
}

#[wasm_bindgen_test]
fn vm_probe_worker_timeout_path_uses_wasm_clock() {
    let recycle = wasm_worker_recycle();
    assert_eq!(recycle.phase(), "worker_recycled".to_string());
    assert_eq!(recycle.state(), "ready".to_string());

    let configured = wasm_worker_set_timeout(50);
    assert_eq!(configured.phase(), "worker_timeout_configured".to_string());
    assert_eq!(configured.state(), "ready".to_string());
    assert!(configured.success());
    assert_eq!(wasm_worker_current_timeout_ms(), 50);

    let timed_out = wasm_worker_execute_with_operation("while True:\n    pass\n");
    assert_eq!(timed_out.phase(), "runtime_error".to_string());
    assert_eq!(timed_out.state(), "ready".to_string());
    assert!(!timed_out.success());
    let timeout_error = timed_out
        .error()
        .expect("timeout execution should report runtime error");
    assert!(
        timeout_error.contains("execution timeout exceeded"),
        "expected timeout marker in runtime error, got: {}",
        timeout_error
    );
    assert_eq!(wasm_worker_current_timeout_ms(), 5_000);
}

#[wasm_bindgen_test]
fn vm_probe_top_level_execute_survives_worker_terminated_state() {
    let recycle = wasm_worker_recycle();
    assert_eq!(recycle.phase(), "worker_recycled".to_string());
    assert_eq!(recycle.state(), "ready".to_string());

    let terminate = wasm_worker_terminate();
    assert_eq!(terminate.phase(), "worker_terminated".to_string());
    assert_eq!(terminate.state(), "unwired".to_string());
    assert!(terminate.success());

    let blocked_worker = wasm_worker_execute_with_operation("1 + 1");
    assert_eq!(
        blocked_worker.phase(),
        "unsupported_worker_execution".to_string()
    );
    assert_eq!(blocked_worker.state(), "unwired".to_string());
    assert!(!blocked_worker.success());
    assert_eq!(
        blocked_worker.blocker_key(),
        Some("worker_runtime_unwired".to_string())
    );

    let top_level = execute("40 + 2");
    assert_eq!(top_level.phase(), "ok".to_string());
    assert!(top_level.success());
    assert!(top_level.error().is_none());

    let restore = wasm_worker_recycle();
    assert_eq!(restore.phase(), "worker_recycled".to_string());
    assert_eq!(restore.state(), "ready".to_string());
    assert!(restore.success());
}

#[wasm_bindgen_test]
fn vm_probe_worker_session_tracks_shared_lifecycle_state() {
    let recycle = wasm_worker_recycle();
    assert_eq!(recycle.phase(), "worker_recycled".to_string());
    assert_eq!(recycle.state(), "ready".to_string());

    let mut session = WasmWorkerSession::new();
    let start = session.start();
    assert_eq!(start.phase(), "worker_started".to_string());
    assert_eq!(start.state(), "ready".to_string());
    assert!(start.success());

    let terminate = wasm_worker_terminate();
    assert_eq!(terminate.phase(), "worker_terminated".to_string());
    assert_eq!(terminate.state(), "unwired".to_string());
    assert!(terminate.success());

    let info = session.info();
    assert_eq!(info.state(), "unwired".to_string());
    assert_eq!(info.backend(), "vm_probe".to_string());

    let blocked = session.execute_with_operation("x = 1\n");
    assert_eq!(blocked.phase(), "unsupported_worker_execution".to_string());
    assert_eq!(blocked.state(), "unwired".to_string());
    assert_eq!(
        blocked.blocker_key(),
        Some("worker_runtime_unwired".to_string())
    );

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.last_phase(),
        Some("unsupported_worker_execution".to_string())
    );
    assert_eq!(snapshot.last_state(), Some("unwired".to_string()));
    assert_eq!(snapshot.executes_requested(), 1);

    let resumed = session.recycle();
    assert_eq!(resumed.phase(), "worker_recycled".to_string());
    assert_eq!(resumed.state(), "ready".to_string());
    assert!(resumed.success());
}
