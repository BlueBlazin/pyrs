# WASM API Contract (codex/wasm)

Status: branch-local draft, API version `1`.

This document defines the JS-facing contract currently exported by
`src/wasm/mod.rs`.

## Execution Mode Matrix

| Surface | Mode | Parse/compile invalid | Parse+compile valid with blocked capability import | Parse+compile valid without blocked imports |
| --- | --- | --- | --- | --- |
| `execute(source)` | default | `syntax_error` / `compile_error` (`blocker_key = None`) | `unsupported_execution` (`blocker_key = <capability_key>`) | `unsupported_execution` (`blocker_key = execution_backend_unwired`) |
| `execute(source)` | `wasm-vm-probe` | `syntax_error` / `compile_error` (`blocker_key = None`) | `unsupported_execution` (`blocker_key = <capability_key>`) | `ok` (success) or `runtime_error` (`blocker_key = None`) |
| `wasm_worker_execute(source)` | default | `syntax_error` / `compile_error` (`blocker_key = None`) | `unsupported_worker_execution` (`blocker_key = <capability_key>`) | `unsupported_worker_execution` (`blocker_key = worker_runtime_unwired`) |
| `wasm_worker_execute(source)` | `wasm-vm-probe` | `syntax_error` / `compile_error` (`blocker_key = None`) | `unsupported_worker_execution` (`blocker_key = <capability_key>`) | `ok` (success) or `runtime_error` (`blocker_key = None`) |

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
  - `lifecycle_supported` is `true` only in `wasm-vm-probe` builds.
- `wasm_worker_timeout_policy() -> WasmWorkerTimeoutPolicy`
  - Returns timeout/recycle contract metadata for worker execution.
  - `configuration_supported` is `true` only in `wasm-vm-probe` builds
    (in-range timeout configuration acceptance).
  - `enforcement_supported` remains `false` in current milestone builds.
- `wasm_worker_timeout_phase_keys() -> Array`
  - Returns canonical timeout phase keys.
  - default keys:
    - `unsupported_worker_timeout_enforcement`
    - `invalid_worker_timeout`
  - `wasm-vm-probe` adds:
    - `worker_timeout_configured`
- `wasm_worker_state_keys() -> Array`
  - Returns canonical worker runtime state keys.
- `wasm_worker_lifecycle_phase_keys() -> Array`
  - Returns canonical worker lifecycle phase keys.
  - default keys:
    - `unsupported_worker_start`
    - `unsupported_worker_terminate`
    - `unsupported_worker_recycle`
  - `wasm-vm-probe` adds:
    - `worker_started`
    - `worker_terminated`
    - `worker_recycled`
- `wasm_worker_execute_phase_keys() -> Array`
  - Returns canonical worker execute phase keys.
  - Includes `ok` + `runtime_error` only when built with `wasm-vm-probe`.
- `wasm_worker_start() -> WasmWorkerLifecycleResult`
  - default build: unsupported unwired lifecycle result
    (`phase = "unsupported_worker_start"`).
  - `wasm-vm-probe`: lifecycle probe success
    (`phase = "worker_started"`, `state = "ready"`).
- `wasm_worker_terminate() -> WasmWorkerLifecycleResult`
  - default build: unsupported unwired lifecycle result
    (`phase = "unsupported_worker_terminate"`).
  - `wasm-vm-probe`: lifecycle probe success
    (`phase = "worker_terminated"`, `state = "unwired"`).
- `wasm_worker_recycle() -> WasmWorkerLifecycleResult`
  - default build: unsupported unwired lifecycle result
    (`phase = "unsupported_worker_recycle"`).
  - `wasm-vm-probe`: lifecycle probe success
    (`phase = "worker_recycled"`, `state = "ready"`).
- `wasm_worker_set_timeout(timeout_ms: u32) -> WasmWorkerTimeoutResult`
  - Worker timeout update contract with deterministic phases:
    - `invalid_worker_timeout` (out-of-range value)
    - default in-range: `unsupported_worker_timeout_enforcement`
    - `wasm-vm-probe` in-range: `worker_timeout_configured`
- `wasm_worker_execute(source: &str) -> WasmExecutionResult`
  - Default worker execute phases:
    - `syntax_error`
    - `compile_error`
    - `unsupported_worker_execution`
  - `wasm-vm-probe` worker execute phases:
    - same parse/compile/capability-preflight phases as default,
    - capability-allowed snippets can return `ok` or `runtime_error`.
  - `blocker_key` is:
    - `None` for parse/compile failures and vm-probe runtime execution results,
    - `Some("<capability_key>")` when parse+compile-valid source imports a known
      wasm-blocked module capability,
    - `Some("worker_runtime_unwired")` for remaining unsupported worker execution in default builds.
- `wasm_worker_execute_with_operation(source: &str) -> WasmWorkerExecutionResult`
  - Worker execute contract with deterministic phases plus `operation_id`.
- `check_syntax(source: &str) -> Result<(), JsValue>`
  - Syntax validation entrypoint; `Err` includes parser message/line/column.
- `check_syntax_result(source: &str) -> WasmSyntaxResult`
  - Structured syntax validation result.
- `check_compile(source: &str) -> Result<(), JsValue>`
  - Parse+compile validation entrypoint.
- `check_compile_result(source: &str) -> WasmCompileResult`
  - Structured parse+compile validation result.
- `execute(source: &str) -> WasmExecutionResult`
  - Default wasm build behavior:
    - `phase = "syntax_error"` when parse fails.
    - `phase = "compile_error"` when parse passes but compilation fails.
    - `phase = "unsupported_execution"` for parse+compile-valid input.
    - `blocker_key` for unsupported execution is:
      - capability-specific (for known blocked imports),
      - otherwise `execution_backend_unwired`.
  - `wasm-vm-probe` feature behavior:
    - parse/compile/capability-preflight behavior is unchanged,
    - capability-allowed snippets execute via VM and return
      `phase = "ok"` (success) or `phase = "runtime_error"` (VM raised runtime error).
  - `stderr` is populated for both current failure phases.
- `wasm_execution_phase_keys() -> Array`
  - Returns canonical top-level execute phase keys.
  - Includes `ok` + `runtime_error` only when built with `wasm-vm-probe`.
- `wasm_capabilities() -> WasmCapabilityReport`
  - Returns explicit browser capability matrix.
- `wasm_capability_error(capability_key: &str) -> Option<String>`
  - Returns unsupported-capability message for known keys.
- `wasm_capability_keys() -> Array`
  - Returns canonical browser capability keys in stable order.
- `wasm_execution_blocker_keys() -> Array`
  - Returns canonical blocker keys for execution in browser mode
    (default build includes `execution_backend_unwired` and `vm_runtime_unavailable`).
  - In `wasm-vm-probe` builds, unwired runtime blocker keys are omitted.
- `wasm_execution_blocker_error(blocker_key: &str) -> Option<String>`
  - Returns stable blocker message for known execution blockers.
- `wasm_execution_blockers() -> Array`
  - Returns structured blocker entries (`key` + `message`).
- `wasm_worker_blocker_keys() -> Array`
  - Returns canonical worker blocker keys (`worker_runtime_unwired` plus
    module-policy capability blocker keys).
- `wasm_worker_blocker_error(blocker_key: &str) -> Option<String>`
  - Returns stable worker blocker message for known keys (runtime-unwired or
    capability-specific unsupported messages).
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
- `wasm_snippet_import_roots(source: &str) -> Array`
  - Canonical root imports detected from parse+compile-valid snippets.

## Exported Types

## `WasmRuntimeInfo`

- `api_version: u32`
- `pyrs_version: String`
- `supports_parse_compile: bool`
- `supports_execution: bool`
- `execution_backend: String` (default `"unwired"`, `wasm-vm-probe` => `"vm_probe"`)
- `execution_status: String` (default `"syntax_compile_only"`, `wasm-vm-probe` => `"runtime_probe"`)
- `execution_blocker_count: usize`

## `WasmWorkerInfo`

- `supported: bool`
- `backend: String` (default `"unwired"`, `wasm-vm-probe` => `"vm_probe"`)
- `state: String` (currently `"unwired"`)
- `interruption_model: String` (currently `"worker_recycle"`)
- `lifecycle_supported: bool` (`true` only in `wasm-vm-probe` builds)
- `execution_probe_enabled: bool` (`true` only in `wasm-vm-probe` builds)
- `execute_supported: bool` (`true` only in `wasm-vm-probe` builds)
- `blocker_count: usize`

## `WasmWorkerTimeoutPolicy`

- `default_timeout_ms: u32` (currently `5000`)
- `min_timeout_ms: u32` (currently `50`)
- `max_timeout_ms: u32` (currently `120000`)
- `configuration_supported: bool` (default `false`, `wasm-vm-probe` => `true`)
- `recycle_on_timeout: bool` (currently `true`)
- `enforcement_supported: bool` (currently `false`)
- `unsupported_phase: String` (currently `"unsupported_worker_timeout_enforcement"`)
- `unsupported_reason: Option<String>`
  - default: `"wasm worker runtime is not wired yet"`
  - `wasm-vm-probe`:
    `"worker timeout enforcement is not wired yet (wasm-vm-probe currently supports configuration-only updates)"`

## `WasmWorkerTimeoutResult`

- `success: bool`
- `operation_id: String` (shape: `worker_set_timeout_<n>`)
- `phase: String` (`"unsupported_worker_timeout_enforcement"`, `"invalid_worker_timeout"`)
  - default: `"unsupported_worker_timeout_enforcement"`, `"invalid_worker_timeout"`
  - `wasm-vm-probe`: also `"worker_timeout_configured"` (in-range config acceptance)
- `state: String` (currently `"unwired"`)
- `timeout_ms: u32`
- `error: Option<String>`
- `blocker_key: Option<String>`

## `WasmWorkerExecutionResult`

- `operation_id: String` (shape: `worker_execute_<n>`)
- `success: bool`
- `phase: String`
- `state: String` (currently `"unwired"`)
- `stdout: String`
- `stderr: String`
- `error: Option<String>`
- `blocker_key: Option<String>`
- `line: usize`
- `column: usize`
  - for `runtime_error` in `wasm-vm-probe` builds, populated from the innermost
    available traceback frame when present.

## `WasmWorkerLifecycleResult`

- `success: bool`
- `operation_id: String` (shape: `worker_<action>_<n>`)
- `phase: String`
  - default: `"unsupported_worker_start"`, `"unsupported_worker_terminate"`,
    `"unsupported_worker_recycle"`
  - `wasm-vm-probe`: `"worker_started"`, `"worker_terminated"`,
    `"worker_recycled"`
- `state: String`
  - default: `"unwired"`
  - `wasm-vm-probe`: `"ready"` for start/recycle, `"unwired"` for terminate
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
  - default: `"syntax_error"`, `"compile_error"`, `"unsupported_execution"`
  - `wasm-vm-probe`: also `"ok"`, `"runtime_error"`
- `stdout: String`
- `stderr: String`
- `error: Option<String>`
- `blocker_key: Option<String>` (`None` for parse/compile failures; unsupported
  execution uses either capability keys or backend/worker unwired keys)
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
- `clock_time: bool`
- `thread_sleep: bool`
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
- `recycle() -> WasmWorkerLifecycleResult`
- `set_timeout_ms(timeout_ms: u32) -> WasmWorkerTimeoutResult`
- `execute(source: &str) -> WasmExecutionResult`
- `execute_with_operation(source: &str) -> WasmWorkerExecutionResult`
- `snapshot() -> WasmWorkerSessionSnapshot`
- `reset()`
- `starts_requested: usize`
- `terminates_requested: usize`
- `recycles_requested: usize`
- `executes_requested: usize`
- `timeout_updates_requested: usize`
- `last_timeout_ms_requested: Option<u32>`
- `last_operation_id: Option<String>`
- `last_phase: Option<String>`
- `last_state: Option<String>`
- `last_error: Option<String>`

## `WasmWorkerSessionSnapshot`

- `starts_requested: usize`
- `terminates_requested: usize`
- `recycles_requested: usize`
- `executes_requested: usize`
- `timeout_updates_requested: usize`
- `last_timeout_ms_requested: Option<u32>`
- `last_operation_id: Option<String>`
- `last_phase: Option<String>`
- `last_state: Option<String>`
- `last_error: Option<String>`

## Stability Rules

1. Any breaking contract change must bump `wasm_api_version()`.
2. Unsupported behavior must remain explicit and structured.
3. Capability key set must stay aligned with `docs/WASM_CAPABILITY_MATRIX.md`.
4. `operation_id` fields guarantee prefix shape + per-process uniqueness only;
   clients must not rely on absolute numeric ordering across runs.

## Related Docs

- `docs/WASM_CLIENT_INTEGRATION_FLOW.md`
- `docs/WASM_MODULE_SUPPORT_POLICY.md`
- `docs/WASM_WORKER_RUNTIME_CONTRACT.md`
