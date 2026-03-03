pub struct WasmModulePolicyFixture {
    pub module: &'static str,
    pub blocker_key: &'static str,
}

pub const WASM_MODULE_POLICY_FIXTURES: &[WasmModulePolicyFixture] = &[
    WasmModulePolicyFixture {
        module: "_ctypes",
        blocker_key: "dynamic_library_load",
    },
    WasmModulePolicyFixture {
        module: "ctypes",
        blocker_key: "dynamic_library_load",
    },
    WasmModulePolicyFixture {
        module: "numpy",
        blocker_key: "dynamic_library_load",
    },
    WasmModulePolicyFixture {
        module: "scipy",
        blocker_key: "dynamic_library_load",
    },
    WasmModulePolicyFixture {
        module: "_socket",
        blocker_key: "network_sockets",
    },
    WasmModulePolicyFixture {
        module: "socket",
        blocker_key: "network_sockets",
    },
    WasmModulePolicyFixture {
        module: "_posixsubprocess",
        blocker_key: "process_spawn",
    },
    WasmModulePolicyFixture {
        module: "subprocess",
        blocker_key: "process_spawn",
    },
    WasmModulePolicyFixture {
        module: "multiprocessing",
        blocker_key: "process_spawn",
    },
    WasmModulePolicyFixture {
        module: "readline",
        blocker_key: "interactive_terminal",
    },
];
