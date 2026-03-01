pub struct WasmContractSnippetFixture {
    pub name: &'static str,
    pub source: &'static str,
    pub expected_compile_phase: &'static str,
    pub expected_execute_phase: &'static str,
    pub expected_execute_blocker_key: Option<&'static str>,
    pub expected_vm_probe_execute_phase: Option<&'static str>,
    pub expected_vm_probe_execute_blocker_key: Option<Option<&'static str>>,
    pub expected_support_phase: &'static str,
    pub expected_first_blocker_key: Option<&'static str>,
}

pub const WASM_EXECUTION_PHASE_KEYS: &[&str] = &[
    "syntax_error",
    "compile_error",
    "unsupported_execution",
];

pub const WASM_SUPPORT_PHASE_KEYS: &[&str] = &[
    "supported",
    "blocked_capability",
    "syntax_error",
    "compile_error",
];

pub const WASM_CONTRACT_SNIPPET_FIXTURES: &[WasmContractSnippetFixture] = &[
    WasmContractSnippetFixture {
        name: "supported_math_import",
        source: "import math\nx = 1\n",
        expected_compile_phase: "ok",
        expected_execute_phase: "unsupported_execution",
        expected_execute_blocker_key: Some("execution_backend_unwired"),
        expected_vm_probe_execute_phase: Some("ok"),
        expected_vm_probe_execute_blocker_key: Some(None),
        expected_support_phase: "supported",
        expected_first_blocker_key: None,
    },
    WasmContractSnippetFixture {
        name: "runtime_error_zero_division",
        source: "1 / 0\n",
        expected_compile_phase: "ok",
        expected_execute_phase: "unsupported_execution",
        expected_execute_blocker_key: Some("execution_backend_unwired"),
        expected_vm_probe_execute_phase: Some("runtime_error"),
        expected_vm_probe_execute_blocker_key: Some(None),
        expected_support_phase: "supported",
        expected_first_blocker_key: None,
    },
    WasmContractSnippetFixture {
        name: "blocked_socket_import",
        source: "import socket\n",
        expected_compile_phase: "ok",
        expected_execute_phase: "unsupported_execution",
        expected_execute_blocker_key: Some("network_sockets"),
        expected_vm_probe_execute_phase: None,
        expected_vm_probe_execute_blocker_key: None,
        expected_support_phase: "blocked_capability",
        expected_first_blocker_key: Some("network_sockets"),
    },
    WasmContractSnippetFixture {
        name: "compile_error_return_outside_function",
        source: "return 1\n",
        expected_compile_phase: "compile_error",
        expected_execute_phase: "compile_error",
        expected_execute_blocker_key: None,
        expected_vm_probe_execute_phase: None,
        expected_vm_probe_execute_blocker_key: None,
        expected_support_phase: "compile_error",
        expected_first_blocker_key: None,
    },
    WasmContractSnippetFixture {
        name: "syntax_error_broken_def",
        source: "def broken(:\n",
        expected_compile_phase: "syntax_error",
        expected_execute_phase: "syntax_error",
        expected_execute_blocker_key: None,
        expected_vm_probe_execute_phase: None,
        expected_vm_probe_execute_blocker_key: None,
        expected_support_phase: "syntax_error",
        expected_first_blocker_key: None,
    },
];
