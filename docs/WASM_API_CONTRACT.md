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
- `wasm_worker_info() -> WasmWorkerInfo`
  - Returns worker-runtime contract status summary.
- `wasm_worker_timeout_policy() -> WasmWorkerTimeoutPolicy`
  - Returns timeout/recycle contract metadata for worker execution.
- `wasm_worker_state_keys() -> Array`
  - Returns canonical worker runtime state keys.
- `wasm_worker_lifecycle_phase_keys() -> Array`
  - Returns canonical worker lifecycle phase keys.
- `wasm_worker_execute_phase_keys() -> Array`
  - Returns canonical worker execute phase keys.
- `wasm_worker_start() -> WasmWorkerLifecycleResult`
  - Worker lifecycle start contract (currently unsupported/unwired).
- `wasm_worker_terminate() -> WasmWorkerLifecycleResult`
  - Worker lifecycle terminate contract (currently unsupported/unwired).
- `wasm_worker_execute(source: &str) -> WasmExecutionResult`
  - Worker execute contract with deterministic phases:
    - `syntax_error`
    - `compile_error`
    - `unsupported_worker_execution`
- `check_syntax(source: &str) -> Result<(), JsValue>`
  - Syntax validation entrypoint; `Err` includes parser message/line/column.
- `check_syntax_result(source: &str) -> WasmSyntaxResult`
  - Structured syntax validation result.
- `check_compile(source: &str) -> Result<(), JsValue>`
  - Parse+compile validation entrypoint.
- `check_compile_result(source: &str) -> WasmCompileResult`
  - Structured parse+compile validation result.
- `execute(source: &str) -> WasmExecutionResult`
  - Current behavior:
    - `phase = "syntax_error"` when parse fails.
    - `phase = "compile_error"` when parse passes but compilation fails.
    - `phase = "unsupported_execution"` for parse+compile-valid input.
  - `stderr` is populated for both current failure phases.
- `wasm_capabilities() -> WasmCapabilityReport`
  - Returns explicit browser capability matrix.
- `wasm_capability_error(capability_key: &str) -> Option<String>`
  - Returns unsupported-capability message for known keys.
- `wasm_execution_blocker_keys() -> Array`
  - Returns canonical blocker keys for execution in browser mode.
- `wasm_execution_blocker_error(blocker_key: &str) -> Option<String>`
  - Returns stable blocker message for known execution blockers.
- `wasm_execution_blockers() -> Array`
  - Returns structured blocker entries (`key` + `message`).
- `wasm_worker_blocker_keys() -> Array`
  - Returns canonical worker blocker keys.
- `wasm_worker_blocker_error(blocker_key: &str) -> Option<String>`
  - Returns stable worker blocker message for known keys.
- `wasm_worker_blockers() -> Array`
  - Returns structured worker blocker entries (`key` + `message`).
- `wasm_module_support(module_name: &str) -> WasmModuleSupport`
  - Returns module-level support/preflight status (`supported`, blocker key/message).
- `wasm_module_policy_entries() -> Array`
  - Returns module->blocker policy rows used for browser preflight UX.
- `wasm_snippet_support(source: &str) -> WasmSnippetSupport`
  - Parse+compile + import-capability preflight summary for snippet viability.
- `wasm_snippet_blockers(source: &str) -> Array`
  - Structured module blocker rows for parse+compile-valid snippets.

## Exported Types

## `WasmRuntimeInfo`

- `api_version: u32`
- `pyrs_version: String`
- `supports_parse_compile: bool`
- `supports_execution: bool`
- `execution_status: String` (currently `"syntax_compile_only"`)
- `execution_blocker_count: usize`

## `WasmWorkerInfo`

- `supported: bool`
- `state: String` (currently `"unwired"`)
- `interruption_model: String` (currently `"worker_recycle"`)
- `blocker_count: usize`

## `WasmWorkerTimeoutPolicy`

- `default_timeout_ms: u32` (currently `5000`)
- `min_timeout_ms: u32` (currently `50`)
- `max_timeout_ms: u32` (currently `120000`)
- `recycle_on_timeout: bool` (currently `true`)
- `enforcement_supported: bool` (currently `false`)
- `unsupported_phase: String` (currently `"unsupported_worker_timeout_enforcement"`)
- `unsupported_reason: Option<String>`

## `WasmWorkerLifecycleResult`

- `success: bool`
- `phase: String` (`"unsupported_worker_start"`, `"unsupported_worker_terminate"`)
- `state: String` (currently `"unwired"`)
- `error: Option<String>`
- `blocker_key: Option<String>`

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
- `line: usize`
- `column: usize`

## `WasmCompileResult`

- `ok: bool`
- `phase: String` (`"ok"`, `"syntax_error"`, `"compile_error"`)
- `error: Option<String>`
- `line: usize`
- `column: usize`

## `WasmCapabilityReport`

- `filesystem_read: bool`
- `filesystem_write: bool`
- `environment_read: bool`
- `process_args: bool`
- `process_spawn: bool`
- `dynamic_library_load: bool`
- `interactive_terminal: bool`
- `network_sockets: bool`

## `WasmExecutionBlocker`

- `key: String`
- `message: String`

## `WasmWorkerBlocker`

- `key: String`
- `message: String`

## `WasmModuleSupport`

- `module: String`
- `supported: bool`
- `blocker_key: Option<String>`
- `message: Option<String>`

## `WasmModulePolicyEntry`

- `module: String`
- `blocker_key: String`

## `WasmSnippetSupport`

- `supported: bool`
- `phase: String` (`"supported"`, `"blocked_capability"`, `"syntax_error"`, `"compile_error"`)
- `error: Option<String>`
- `line: usize`
- `column: usize`
- `imported_module_count: usize`
- `blocker_count: usize`
- `first_blocker_module: Option<String>`
- `first_blocker_key: Option<String>`
- `first_blocker_message: Option<String>`

## `WasmSnippetBlocker`

- `module: String`
- `blocker_key: String`
- `message: String`

## `WasmSession`

- `new()`
- `check_syntax(source: &str) -> WasmSyntaxResult`
- `check_compile(source: &str) -> WasmCompileResult`
- `execute(source: &str) -> WasmExecutionResult`
- `reset()`
- `snippets_checked: usize`
- `last_error: Option<String>`

## `WasmWorkerSession`

- `new()`
- `info() -> WasmWorkerInfo`
- `start() -> WasmWorkerLifecycleResult`
- `terminate() -> WasmWorkerLifecycleResult`
- `execute(source: &str) -> WasmExecutionResult`
- `reset()`
- `starts_requested: usize`
- `terminates_requested: usize`
- `executes_requested: usize`
- `last_phase: Option<String>`
- `last_error: Option<String>`

## Stability Rules

1. Any breaking contract change must bump `wasm_api_version()`.
2. Unsupported behavior must remain explicit and structured.
3. Capability key set must stay aligned with `docs/WASM_CAPABILITY_MATRIX.md`.

## Related Docs

- `docs/WASM_CLIENT_INTEGRATION_FLOW.md`
- `docs/WASM_MODULE_SUPPORT_POLICY.md`
- `docs/WASM_WORKER_RUNTIME_CONTRACT.md`
