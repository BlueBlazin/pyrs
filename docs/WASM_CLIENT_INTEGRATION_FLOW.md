# WASM Client Integration Flow (codex/wasm)

Status: branch-local draft.

This guide defines the recommended browser call order for current wasm APIs.

## Recommended Call Order

1. `init_wasm_runtime()`
2. `wasm_runtime_info()`
3. `wasm_worker_info()`
4. `wasm_snippet_support(source)`
5. If `phase == "supported"`:
   - call `check_compile_result(source)` (optional if you already used snippet preflight),
   - call `execute(source)` (currently returns `unsupported_execution` by contract).
6. If `phase == "blocked_capability"`:
   - call `wasm_snippet_blockers(source)` for full blocker rows,
   - render module/capability-specific guidance.
7. If `phase == "syntax_error"` or `phase == "compile_error"`:
   - use `line`/`column` + `error` for diagnostics UI.

## Worker Branch (Current)

`wasm_worker_start()` and `wasm_worker_terminate()` are currently explicit stubs.

- `wasm_worker_start()` -> `phase = "unsupported_worker_start"`
- `wasm_worker_terminate()` -> `phase = "unsupported_worker_terminate"`
- both return `blocker_key = "worker_runtime_unwired"`

Use this to keep UI behavior deterministic before worker backend wiring.

You can call lifecycle methods directly or via `WasmWorkerSession` for stateful
UI telemetry (`starts_requested`, `terminates_requested`, `last_phase`, `last_error`).

Prefer `wasm_worker_state_keys()` and `wasm_worker_lifecycle_phase_keys()` for
UI branching enums instead of hardcoding string literals.

## Minimal Browser Pseudocode

```js
import init, * as pyrs from "./pkg/pyrs.js";

await init();
pyrs.init_wasm_runtime();

const runtime = pyrs.wasm_runtime_info();
const worker = pyrs.wasm_worker_info();

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

const result = pyrs.execute(code);
if (result.phase === "unsupported_execution") {
  showInfo(result.error ?? "Execution backend not wired yet");
}
```

## Notes

1. Treat all `phase` values as contract enums, not free text.
2. Prefer structured fields (`line`, `column`, blocker keys) over message parsing.
3. Keep module/capability messaging aligned with `docs/WASM_MODULE_SUPPORT_POLICY.md`.
