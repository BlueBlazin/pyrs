pub struct WasmWorkerLifecycleFixture {
    pub name: &'static str,
    pub action: &'static str,
    pub expected_phase: &'static str,
    pub expected_state: &'static str,
    pub expected_success: bool,
    pub expected_blocker_key: &'static str,
}

pub struct WasmWorkerExecuteFixture {
    pub name: &'static str,
    pub source: &'static str,
    pub expected_phase: &'static str,
    pub expect_error: bool,
    pub expect_line_column: bool,
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
    WasmWorkerLifecycleFixture {
        name: "worker_recycle_unwired",
        action: "recycle",
        expected_phase: "unsupported_worker_recycle",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: "worker_runtime_unwired",
    },
];

pub const WASM_WORKER_EXECUTE_FIXTURES: &[WasmWorkerExecuteFixture] = &[
    WasmWorkerExecuteFixture {
        name: "worker_execute_syntax_error",
        source: "def broken(:\n",
        expected_phase: "syntax_error",
        expect_error: true,
        expect_line_column: true,
    },
    WasmWorkerExecuteFixture {
        name: "worker_execute_compile_error",
        source: "return 1\n",
        expected_phase: "compile_error",
        expect_error: true,
        expect_line_column: true,
    },
    WasmWorkerExecuteFixture {
        name: "worker_execute_unwired",
        source: "x = 1\n",
        expected_phase: "unsupported_worker_execution",
        expect_error: true,
        expect_line_column: false,
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

pub const WASM_WORKER_LIFECYCLE_PHASE_KEYS: &[&str] = &[
    "unsupported_worker_start",
    "unsupported_worker_terminate",
    "unsupported_worker_recycle",
];

pub const WASM_WORKER_EXECUTE_PHASE_KEYS: &[&str] = &[
    "syntax_error",
    "compile_error",
    "unsupported_worker_execution",
];
