pub struct WasmWorkerLifecycleFixture {
    pub name: &'static str,
    pub action: &'static str,
    pub expected_operation_prefix: &'static str,
    pub expected_phase: &'static str,
    pub expected_state: &'static str,
    pub expected_success: bool,
    pub expected_blocker_key: &'static str,
}

pub struct WasmWorkerExecuteFixture {
    pub name: &'static str,
    pub expected_operation_prefix: &'static str,
    pub source: &'static str,
    pub expected_phase: &'static str,
    pub expected_blocker_key: Option<&'static str>,
    pub expected_vm_probe_phase: Option<&'static str>,
    pub expected_vm_probe_blocker_key: Option<Option<&'static str>>,
    pub expect_error: bool,
    pub expected_vm_probe_expect_error: Option<bool>,
    pub expected_success: bool,
    pub expected_vm_probe_success: Option<bool>,
    pub expect_line_column: bool,
    pub expected_vm_probe_expect_line_column: Option<bool>,
}

pub struct WasmWorkerTimeoutFixture {
    pub name: &'static str,
    pub expected_operation_prefix: &'static str,
    pub timeout_ms: u32,
    pub expected_phase: &'static str,
    pub expected_state: &'static str,
    pub expected_success: bool,
    pub expected_blocker_key: Option<&'static str>,
}

pub struct WasmWorkerInfoFixture {
    pub name: &'static str,
    pub expected_supported: bool,
    pub expected_backend: &'static str,
    pub expected_vm_probe_backend: Option<&'static str>,
    pub expected_state: &'static str,
    pub expected_interruption_model: &'static str,
    pub expected_execution_probe_enabled: bool,
    pub expected_vm_probe_execution_probe_enabled: Option<bool>,
}

pub const WASM_WORKER_INFO_FIXTURES: &[WasmWorkerInfoFixture] = &[WasmWorkerInfoFixture {
    name: "worker_info_runtime_contract",
    expected_supported: false,
    expected_backend: "unwired",
    expected_vm_probe_backend: Some("vm_probe"),
    expected_state: "unwired",
    expected_interruption_model: "worker_recycle",
    expected_execution_probe_enabled: false,
    expected_vm_probe_execution_probe_enabled: Some(true),
},];

pub const WASM_WORKER_LIFECYCLE_FIXTURES: &[WasmWorkerLifecycleFixture] = &[
    WasmWorkerLifecycleFixture {
        name: "worker_start_unwired",
        action: "start",
        expected_operation_prefix: "worker_start_",
        expected_phase: "unsupported_worker_start",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: "worker_runtime_unwired",
    },
    WasmWorkerLifecycleFixture {
        name: "worker_terminate_unwired",
        action: "terminate",
        expected_operation_prefix: "worker_terminate_",
        expected_phase: "unsupported_worker_terminate",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: "worker_runtime_unwired",
    },
    WasmWorkerLifecycleFixture {
        name: "worker_recycle_unwired",
        action: "recycle",
        expected_operation_prefix: "worker_recycle_",
        expected_phase: "unsupported_worker_recycle",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: "worker_runtime_unwired",
    },
];

pub const WASM_WORKER_EXECUTE_FIXTURES: &[WasmWorkerExecuteFixture] = &[
    WasmWorkerExecuteFixture {
        name: "worker_execute_syntax_error",
        expected_operation_prefix: "worker_execute_",
        source: "def broken(:\n",
        expected_phase: "syntax_error",
        expected_blocker_key: None,
        expected_vm_probe_phase: None,
        expected_vm_probe_blocker_key: None,
        expect_error: true,
        expected_vm_probe_expect_error: None,
        expected_success: false,
        expected_vm_probe_success: None,
        expect_line_column: true,
        expected_vm_probe_expect_line_column: None,
    },
    WasmWorkerExecuteFixture {
        name: "worker_execute_compile_error",
        expected_operation_prefix: "worker_execute_",
        source: "return 1\n",
        expected_phase: "compile_error",
        expected_blocker_key: None,
        expected_vm_probe_phase: None,
        expected_vm_probe_blocker_key: None,
        expect_error: true,
        expected_vm_probe_expect_error: None,
        expected_success: false,
        expected_vm_probe_success: None,
        expect_line_column: true,
        expected_vm_probe_expect_line_column: None,
    },
    WasmWorkerExecuteFixture {
        name: "worker_execute_unwired",
        expected_operation_prefix: "worker_execute_",
        source: "x = 1\n",
        expected_phase: "unsupported_worker_execution",
        expected_blocker_key: Some("worker_runtime_unwired"),
        expected_vm_probe_phase: Some("ok"),
        expected_vm_probe_blocker_key: Some(None),
        expect_error: true,
        expected_vm_probe_expect_error: Some(false),
        expected_success: false,
        expected_vm_probe_success: Some(true),
        expect_line_column: false,
        expected_vm_probe_expect_line_column: Some(false),
    },
    WasmWorkerExecuteFixture {
        name: "worker_execute_runtime_error_zero_division",
        expected_operation_prefix: "worker_execute_",
        source: "1 / 0\n",
        expected_phase: "unsupported_worker_execution",
        expected_blocker_key: Some("worker_runtime_unwired"),
        expected_vm_probe_phase: Some("runtime_error"),
        expected_vm_probe_blocker_key: Some(None),
        expect_error: true,
        expected_vm_probe_expect_error: Some(true),
        expected_success: false,
        expected_vm_probe_success: Some(false),
        expect_line_column: false,
        expected_vm_probe_expect_line_column: Some(true),
    },
    WasmWorkerExecuteFixture {
        name: "worker_execute_blocked_socket",
        expected_operation_prefix: "worker_execute_",
        source: "import socket\n",
        expected_phase: "unsupported_worker_execution",
        expected_blocker_key: Some("network_sockets"),
        expected_vm_probe_phase: None,
        expected_vm_probe_blocker_key: None,
        expect_error: true,
        expected_vm_probe_expect_error: None,
        expected_success: false,
        expected_vm_probe_success: None,
        expect_line_column: false,
        expected_vm_probe_expect_line_column: None,
    },
];

pub const WASM_WORKER_TIMEOUT_FIXTURES: &[WasmWorkerTimeoutFixture] = &[
    WasmWorkerTimeoutFixture {
        name: "worker_timeout_invalid_low",
        expected_operation_prefix: "worker_set_timeout_",
        timeout_ms: 0,
        expected_phase: "invalid_worker_timeout",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: None,
    },
    WasmWorkerTimeoutFixture {
        name: "worker_timeout_unwired_min",
        expected_operation_prefix: "worker_set_timeout_",
        timeout_ms: 50,
        expected_phase: "unsupported_worker_timeout_enforcement",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: Some("worker_runtime_unwired"),
    },
    WasmWorkerTimeoutFixture {
        name: "worker_timeout_unwired_default",
        expected_operation_prefix: "worker_set_timeout_",
        timeout_ms: 5_000,
        expected_phase: "unsupported_worker_timeout_enforcement",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: Some("worker_runtime_unwired"),
    },
    WasmWorkerTimeoutFixture {
        name: "worker_timeout_unwired_max",
        expected_operation_prefix: "worker_set_timeout_",
        timeout_ms: 120_000,
        expected_phase: "unsupported_worker_timeout_enforcement",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: Some("worker_runtime_unwired"),
    },
    WasmWorkerTimeoutFixture {
        name: "worker_timeout_invalid_high",
        expected_operation_prefix: "worker_set_timeout_",
        timeout_ms: 120_001,
        expected_phase: "invalid_worker_timeout",
        expected_state: "unwired",
        expected_success: false,
        expected_blocker_key: None,
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

pub const WASM_WORKER_TIMEOUT_PHASE_KEYS: &[&str] = &[
    "unsupported_worker_timeout_enforcement",
    "invalid_worker_timeout",
];

pub const WASM_WORKER_BLOCKER_KEYS: &[&str] = &[
    "worker_runtime_unwired",
    "dynamic_library_load",
    "network_sockets",
    "process_spawn",
    "interactive_terminal",
];
