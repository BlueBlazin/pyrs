# WASM API Contract (codex/wasm)

Status: branch-local draft, API version `1`.

This document defines the JS-facing contract currently exported by
`src/wasm/mod.rs`.

## Top-Level Functions

- `pyrs_version() -> String`
  - Returns the PYRS package version.
- `wasm_api_version() -> u32`
  - Returns wasm API contract version.
- `init_wasm_runtime()`
  - Installs panic hook once for browser-console diagnostics.
- `wasm_runtime_info() -> WasmRuntimeInfo`
  - Returns bridge/runtime status summary.
- `check_syntax(source: &str) -> Result<(), JsValue>`
  - Syntax validation entrypoint; `Err` includes parser message/line/column.
- `check_syntax_result(source: &str) -> WasmSyntaxResult`
  - Structured syntax validation result.
- `execute(source: &str) -> WasmExecutionResult`
  - Current behavior:
    - `phase = "syntax_error"` when parse fails.
    - `phase = "unsupported_execution"` for syntax-valid input.
  - `stderr` is populated for both current failure phases.
- `wasm_capabilities() -> WasmCapabilityReport`
  - Returns explicit browser capability matrix.
- `wasm_capability_error(capability_key: &str) -> Option<String>`
  - Returns unsupported-capability message for known keys.

## Exported Types

## `WasmRuntimeInfo`

- `api_version: u32`
- `pyrs_version: String`
- `supports_execution: bool`
- `execution_status: String` (currently `"syntax_only"`)

## `WasmSyntaxResult`

- `ok: bool`
- `error: Option<String>`
- `line: usize`
- `column: usize`

## `WasmExecutionResult`

- `success: bool`
- `phase: String`
- `stdout: String`
- `stderr: String`
- `error: Option<String>`

## `WasmCapabilityReport`

- `filesystem_read: bool`
- `filesystem_write: bool`
- `environment_read: bool`
- `process_args: bool`
- `process_spawn: bool`
- `dynamic_library_load: bool`
- `interactive_terminal: bool`
- `network_sockets: bool`

## `WasmSession`

- `new()`
- `check_syntax(source: &str) -> WasmSyntaxResult`
- `execute(source: &str) -> WasmExecutionResult`
- `reset()`
- `snippets_checked: usize`
- `last_error: Option<String>`

## Stability Rules

1. Any breaking contract change must bump `wasm_api_version()`.
2. Unsupported behavior must remain explicit and structured.
3. Capability key set must stay aligned with `docs/WASM_CAPABILITY_MATRIX.md`.
