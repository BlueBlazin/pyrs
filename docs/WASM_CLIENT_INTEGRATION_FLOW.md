# WASM Client Integration Flow (codex/wasm)

Status: branch-local draft.

This guide defines the recommended browser call order for current wasm APIs.

## Recommended Call Order

1. `init_wasm_runtime()`
2. `wasm_runtime_info()`
   - use `execution_backend` + `supports_execution` for backend-readiness UI state.
3. `wasm_worker_info()`
   - use `backend` + `supported` + `lifecycle_supported` + `execution_probe_enabled` + `execute_supported` + `timeout_configuration_supported` + `timeout_enforcement_supported` for worker-runtime/readiness UI state.
   - baseline `state`: default build `"unwired"`, `wasm-vm-probe` `"ready"`.
   - top-level lifecycle calls mutate this shared state.
4. `wasm_worker_timeout_policy()`
   - gate timeout controls on `configuration_supported`;
   - treat `enforcement_supported` as the separate hard-enforcement capability.
5. Optional: `wasm_worker_current_timeout_ms()`
   - read active timeout config for timeout-control UI state.
6. Optional: `wasm_worker_set_timeout(timeout_ms)` for UI timeout controls.
7. `wasm_snippet_support(source)`
8. Optional: `wasm_snippet_import_roots(source)` to display dependency roots.
9. If `phase == "supported"`:
   - call `check_compile_result(source)` (optional if you already used snippet preflight),
   - call `execute(source)`.
   - default build: parse+compile-valid calls return `unsupported_execution`.
   - `wasm-vm-probe` build: capability-allowed snippets can return `ok` or `runtime_error`.
   - when phase is unsupported, use `result.blocker_key` for deterministic UI branching.
   - optional: use `wasm_execution_phase_keys()` for execute-phase enum hydration.
10. If `phase == "blocked_capability"`:
   - call `wasm_snippet_blockers(source)` for full blocker rows,
   - render module/capability-specific guidance.
11. If `phase == "syntax_error"` or `phase == "compile_error"`:
   - use `line`/`column` + `error` for diagnostics UI.

## Worker Branch (Current)

`wasm_worker_start()`, `wasm_worker_terminate()`, and `wasm_worker_recycle()`
are explicit contract lifecycle controls.

- default build:
  - `wasm_worker_start()` -> `phase = "unsupported_worker_start"`
  - `wasm_worker_terminate()` -> `phase = "unsupported_worker_terminate"`
  - `wasm_worker_recycle()` -> `phase = "unsupported_worker_recycle"`
  - lifecycle calls return `blocker_key = "worker_runtime_unwired"`
- `wasm-vm-probe` build:
  - `wasm_worker_start()` -> `phase = "worker_started"` (`state = "ready"`)
  - `wasm_worker_terminate()` -> `phase = "worker_terminated"` (`state = "unwired"`)
  - `wasm_worker_recycle()` -> `phase = "worker_recycled"` (`state = "ready"`)
  - lifecycle calls return `success = true`, `blocker_key = None`, `error = None`
  - `start`/`recycle` reset worker runtime state to a fresh VM session
  - `terminate` clears worker runtime state
- `wasm_worker_execute(source)` -> `phase` in:
  - `syntax_error`
  - `compile_error`
  - `unsupported_worker_execution`
  - `worker phase keys` also include vm-probe lifecycle-only keys:
    - `worker_started`
    - `worker_terminated`
    - `worker_recycled`
  - default build: unsupported phase sets
    `blocker_key = "worker_runtime_unwired"` or `"worker_runtime_failed"`
  - `wasm-vm-probe` build:
    - capability-allowed snippets return `ok` or `runtime_error` only when
      worker `state = "ready"`,
    - capability-allowed snippets run on a persistent worker VM while `state = "ready"`,
    - `runtime_error` keeps worker `state = "ready"` for follow-up executes,
    - when worker `state != "ready"`, capability-allowed snippets return
      `unsupported_worker_execution` with
      `blocker_key = "worker_runtime_unwired"` or `"worker_runtime_failed"`.
- `wasm_worker_set_timeout(timeout_ms)` -> `phase` in:
  - `invalid_worker_timeout` for out-of-range values
  - default in-range: `unsupported_worker_timeout_enforcement`
    (`blocker_key = "worker_runtime_unwired"` or `"worker_runtime_failed"`)
  - `wasm-vm-probe` in-range:
    - `worker_timeout_configured` when worker `state = "ready"`,
    - `unsupported_worker_timeout_enforcement` with
      `blocker_key = "worker_runtime_unwired"` or `"worker_runtime_failed"`
      when worker `state != "ready"`,
    - timeout enforcement is enabled in `wasm-vm-probe` worker execute paths,
    - timeout runtime errors recycle worker runtime state and reset timeout to
      the default (`5000`),
    - when configured, `wasm_worker_current_timeout_ms()` reflects the new value.
  - expected policy shape:
    - default build: `configuration_supported = false`, `enforcement_supported = false`
    - `wasm-vm-probe`: `configuration_supported = true`, `enforcement_supported = true`
- `wasm_worker_execute_with_operation(source)` -> same phases plus
  `operation_id = worker_execute_<n>`

Use this to keep UI behavior deterministic before worker backend wiring.

For worker-specific diagnostics UI, call `wasm_worker_blockers()` to get stable
structured key/message rows without hardcoding blocker text.
Use `wasm_worker_timeout_policy()` to keep timeout controls aligned with the
current worker recycle model and timeout phase semantics.
Use `wasm_worker_timeout_phase_keys()` to branch timeout UI on canonical phase
enums instead of string literals.
Use `operation_id` fields from lifecycle/timeout results (and from
`wasm_worker_execute_with_operation`) plus execute-result `state` for request
correlation and worker-status breadcrumbs in UI logs/diagnostics.

You can call lifecycle methods directly or via `WasmWorkerSession` for stateful
UI telemetry (`starts_requested`, `terminates_requested`, `recycles_requested`,
`executes_requested`, `timeout_updates_requested`, `last_timeout_ms_requested`,
`last_operation_id`, `last_phase`, `last_state`, `last_error`).
With `WasmWorkerSession`, `info().state` reflects the shared top-level worker
state; use `last_state` for session-local telemetry history.
For atomic telemetry reads, use `WasmWorkerSession.snapshot()`.

Prefer `wasm_worker_state_keys()`, `wasm_worker_lifecycle_phase_keys()`, and
`wasm_worker_execute_phase_keys()`, and `wasm_worker_timeout_phase_keys()` for
UI branching enums instead of hardcoding
string literals.

## `/playground` Worker Transport (Website)

Current website `/playground` integration uses a dedicated module worker:

- worker module:
  - `website/public/workers/playground-runtime-worker.js`
- worker startup:
  - `new Worker(workerEntrypoint, { type: "module" })`
- request envelope:
  - `{ requestId, action, ...payload }`
- response envelope:
  - `{ requestId, ok, ...payload }`

Supported request actions:

1. `load`
   - payload: `{ wasmEntrypoint }`
   - response:
     - success: `{ ok: true, runtimeInfo, prompt_continuation }`
     - failure: `{ ok: false, error }`
2. `execute`
   - payload: `{ source }`
   - response:
     - success: `{ ok: true, result, prompt_continuation }` where `result`
       mirrors `WasmExecutionResult` shape.
     - failure: `{ ok: false, error }`
3. `reset`
   - payload: none
   - response:
     - success: `{ ok: true, prompt_continuation }`
     - failure: `{ ok: false, error }`

Integration guardrails:

- treat worker `error`/`messageerror` events as fatal for in-flight requests;
- reject all pending request promises on worker failure;
- keep transcript rendering on main thread, and keep wasm execution in worker only.

## Minimal Browser Pseudocode

```js
import init, * as pyrs from "./pkg/pyrs.js";

await init();
pyrs.init_wasm_runtime();

const runtime = pyrs.wasm_runtime_info();
const worker = pyrs.wasm_worker_info();
const timeoutPolicy = pyrs.wasm_worker_timeout_policy();
const currentTimeoutMs = pyrs.wasm_worker_current_timeout_ms();

if (timeoutPolicy.configuration_supported) {
  const timeoutResult = pyrs.wasm_worker_set_timeout(
    currentTimeoutMs || timeoutPolicy.default_timeout_ms,
  );
  if (timeoutResult.phase === "invalid_worker_timeout") {
    showError(timeoutResult.error);
    return;
  }
}

const support = pyrs.wasm_snippet_support(code);
if (support.phase === "syntax_error" || support.phase === "compile_error") {
  showDiagnostic(support.error, support.line, support.column);
  return;
}
if (support.phase === "blocked_capability") {
  const blockers = pyrs.wasm_snippet_blockers(code);
  showBlockers(blockers);
  return;
}
const importRoots = pyrs.wasm_snippet_import_roots(code);
showImports(importRoots);

const result = pyrs.wasm_worker_execute_with_operation(code);
if (result.phase === "syntax_error" || result.phase === "compile_error") {
  showDiagnostic(result.error, result.line, result.column);
} else if (result.phase === "runtime_error") {
  showRuntimeError(result.error, result.line, result.column);
} else if (result.phase === "unsupported_worker_execution") {
  showInfo(
    `${result.operation_id}: ${result.blocker_key ?? "worker_runtime_unwired"}: ${
      result.error ?? "Execution backend not wired yet"
    }`
  );
} else if (result.phase === "ok") {
  showResult(result.stdout);
}
```

## Notes

1. Treat all `phase` values as contract enums, not free text.
2. Prefer structured fields (`line`, `column`, blocker keys) over message parsing.
3. Keep module/capability messaging aligned with `docs/WASM_MODULE_SUPPORT_POLICY.md`.
