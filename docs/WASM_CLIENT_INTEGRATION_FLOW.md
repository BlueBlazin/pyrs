# WASM Client Integration Flow (codex/wasm)

Status: branch-local draft.

This guide defines the recommended browser call order for current wasm APIs.

## Recommended Call Order

1. `init_wasm_runtime()`
2. `wasm_runtime_info()`
3. `wasm_worker_info()`
4. `wasm_worker_timeout_policy()`
5. Optional: `wasm_worker_set_timeout(timeout_ms)` for UI timeout controls.
6. `wasm_snippet_support(source)`
7. Optional: `wasm_snippet_import_roots(source)` to display dependency roots.
8. If `phase == "supported"`:
   - call `check_compile_result(source)` (optional if you already used snippet preflight),
   - call `execute(source)` (currently returns `unsupported_execution` by contract).
   - when phase is unsupported, use `result.blocker_key` for deterministic UI branching.
   - optional: use `wasm_execution_phase_keys()` for execute-phase enum hydration.
9. If `phase == "blocked_capability"`:
   - call `wasm_snippet_blockers(source)` for full blocker rows,
   - render module/capability-specific guidance.
10. If `phase == "syntax_error"` or `phase == "compile_error"`:
   - use `line`/`column` + `error` for diagnostics UI.

## Worker Branch (Current)

`wasm_worker_start()`, `wasm_worker_terminate()`, and `wasm_worker_recycle()`
are currently explicit stubs.

- `wasm_worker_start()` -> `phase = "unsupported_worker_start"`
- `wasm_worker_terminate()` -> `phase = "unsupported_worker_terminate"`
- `wasm_worker_recycle()` -> `phase = "unsupported_worker_recycle"`
- both return `blocker_key = "worker_runtime_unwired"`
- `wasm_worker_execute(source)` -> `phase` in:
  - `syntax_error`
  - `compile_error`
  - `unsupported_worker_execution`
  - unsupported phase sets `blocker_key = "worker_runtime_unwired"`
- `wasm_worker_execute_with_operation(source)` -> same phases plus
  `operation_id = worker_execute_<n>`

Use this to keep UI behavior deterministic before worker backend wiring.

For worker-specific diagnostics UI, call `wasm_worker_blockers()` to get stable
structured key/message rows without hardcoding blocker text.
Use `wasm_worker_timeout_policy()` to keep timeout controls aligned with the
current worker recycle model and unsupported timeout-enforcement phase.
Use `wasm_worker_timeout_phase_keys()` to branch timeout UI on canonical phase
enums instead of string literals.
Use `operation_id` fields from lifecycle/timeout results (and from
`wasm_worker_execute_with_operation`) for request correlation in UI logs and diagnostics.

You can call lifecycle methods directly or via `WasmWorkerSession` for stateful
UI telemetry (`starts_requested`, `terminates_requested`, `recycles_requested`,
`executes_requested`, `timeout_updates_requested`, `last_timeout_ms_requested`,
`last_operation_id`, `last_phase`, `last_error`).

Prefer `wasm_worker_state_keys()`, `wasm_worker_lifecycle_phase_keys()`, and
`wasm_worker_execute_phase_keys()`, and `wasm_worker_timeout_phase_keys()` for
UI branching enums instead of hardcoding
string literals.

## Minimal Browser Pseudocode

```js
import init, * as pyrs from "./pkg/pyrs.js";

await init();
pyrs.init_wasm_runtime();

const runtime = pyrs.wasm_runtime_info();
const worker = pyrs.wasm_worker_info();
const timeoutPolicy = pyrs.wasm_worker_timeout_policy();

const timeoutResult = pyrs.wasm_worker_set_timeout(timeoutPolicy.default_timeout_ms);
if (timeoutResult.phase === "invalid_worker_timeout") {
  showError(timeoutResult.error);
  return;
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
if (result.phase === "unsupported_worker_execution") {
  showInfo(
    `${result.operation_id}: ${result.blocker_key ?? "worker_runtime_unwired"}: ${
      result.error ?? "Execution backend not wired yet"
    }`
  );
}
```

## Notes

1. Treat all `phase` values as contract enums, not free text.
2. Prefer structured fields (`line`, `column`, blocker keys) over message parsing.
3. Keep module/capability messaging aligned with `docs/WASM_MODULE_SUPPORT_POLICY.md`.
