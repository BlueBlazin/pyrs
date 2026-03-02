#![cfg(all(target_arch = "wasm32", feature = "wasm-vm-probe"))]

use pyrs::wasm::{
    execute, wasm_runtime_info, wasm_worker_info, wasm_worker_recycle, wasm_worker_set_timeout,
    wasm_worker_start, wasm_worker_terminate,
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
    assert_eq!(resumed_timeout.phase(), "worker_timeout_configured".to_string());
    assert_eq!(resumed_timeout.state(), "ready".to_string());
    assert!(resumed_timeout.success());
    assert_eq!(wasm_worker_info().state(), "ready".to_string());
}
