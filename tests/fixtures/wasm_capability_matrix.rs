pub struct WasmCapabilityFixture {
    pub key: &'static str,
    pub native_supported: bool,
    pub wasm_supported: bool,
}

pub const WASM_CAPABILITY_FIXTURES: &[WasmCapabilityFixture] = &[
    WasmCapabilityFixture {
        key: "filesystem_read",
        native_supported: true,
        wasm_supported: false,
    },
    WasmCapabilityFixture {
        key: "filesystem_write",
        native_supported: true,
        wasm_supported: false,
    },
    WasmCapabilityFixture {
        key: "environment_read",
        native_supported: true,
        wasm_supported: false,
    },
    WasmCapabilityFixture {
        key: "process_args",
        native_supported: true,
        wasm_supported: true,
    },
    WasmCapabilityFixture {
        key: "clock_time",
        native_supported: true,
        wasm_supported: true,
    },
    WasmCapabilityFixture {
        key: "thread_sleep",
        native_supported: true,
        wasm_supported: false,
    },
    WasmCapabilityFixture {
        key: "process_spawn",
        native_supported: true,
        wasm_supported: false,
    },
    WasmCapabilityFixture {
        key: "dynamic_library_load",
        native_supported: true,
        wasm_supported: false,
    },
    WasmCapabilityFixture {
        key: "interactive_terminal",
        native_supported: true,
        wasm_supported: false,
    },
    WasmCapabilityFixture {
        key: "network_sockets",
        native_supported: true,
        wasm_supported: false,
    },
];
