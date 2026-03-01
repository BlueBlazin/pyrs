pub struct WasmWorkerLifecycleFixture {
    pub name: &'static str,
    pub action: &'static str,
    pub expected_phase: &'static str,
    pub expected_state: &'static str,
    pub expected_success: bool,
    pub expected_blocker_key: &'static str,
}

pub const WASM_WORKER_LIFECYCLE_FIXTURES: &[WasmWorkerLifecycleFixture] = &[
    WasmWorkerLifecycleFixture {
        name: "worker_start_unwired",
        action: "start",
        expected_phase: "unsupported_worker_start",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: "worker_runtime_unwired",
    },
    WasmWorkerLifecycleFixture {
        name: "worker_terminate_unwired",
        action: "terminate",
        expected_phase: "unsupported_worker_terminate",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: "worker_runtime_unwired",
    },
];

pub const WASM_WORKER_STATE_KEYS: &[&str] = &[
    "unwired",
    "starting",
    "ready",
    "busy",
    "terminating",
    "failed",
];

pub const WASM_WORKER_LIFECYCLE_PHASE_KEYS: &[&str] =
    &["unsupported_worker_start", "unsupported_worker_terminate"];
