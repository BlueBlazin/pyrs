#![cfg(all(target_arch = "wasm32", feature = "wasm-vm-probe"))]

use pyrs::wasm::{
    execute, wasm_runtime_info, wasm_worker_execute, wasm_worker_info, wasm_worker_recycle,
    wasm_worker_start, wasm_worker_terminate,
};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn vm_probe_runtime_executes_basic_snippet() {
    let info = wasm_runtime_info();
    assert_eq!(info.execution_backend(), "vm_probe".to_string());
    assert!(info.supports_execution());

    let result = execute("print(1 + 1)");
    assert_eq!(result.phase(), "ok".to_string());
    assert!(result.success());
    assert!(result.stderr().is_empty());
    assert!(
        result.stdout().contains("2"),
        "expected execute stdout to contain result value, got: {}",
        result.stdout()
    );
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

    let assign = wasm_worker_execute("value = 41 + 1\nprint(value)");
    assert_eq!(assign.phase(), "ok".to_string());
    assert!(assign.success());
    assert!(
        assign.stdout().contains("42"),
        "expected worker stdout to contain assigned value, got: {}",
        assign.stdout()
    );
    assert_eq!(wasm_worker_info().state(), "ready".to_string());

    let terminate = wasm_worker_terminate();
    assert_eq!(terminate.phase(), "worker_terminated".to_string());
    assert_eq!(terminate.state(), "unwired".to_string());
    assert!(terminate.success());

    let blocked = wasm_worker_execute("print(value)");
    assert_eq!(blocked.phase(), "unsupported_worker_execution".to_string());
    assert!(!blocked.success());
    assert_eq!(
        blocked.blocker_key(),
        Some("worker_runtime_unwired".to_string())
    );
    assert_eq!(wasm_worker_info().state(), "unwired".to_string());

    let recycle = wasm_worker_recycle();
    assert_eq!(recycle.phase(), "worker_recycled".to_string());
    assert_eq!(recycle.state(), "ready".to_string());
    assert!(recycle.success());

    let resumed = wasm_worker_execute("print('ready')");
    assert_eq!(resumed.phase(), "ok".to_string());
    assert!(resumed.success());
    assert!(
        resumed.stdout().contains("ready"),
        "expected recycled worker execute stdout, got: {}",
        resumed.stdout()
    );
    assert_eq!(wasm_worker_info().state(), "ready".to_string());
}
