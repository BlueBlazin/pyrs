# WASM Worker Runtime Contract (codex/wasm)

Status: branch-local draft.

This document defines the browser worker-runtime contract currently exposed by:

- `wasm_worker_info()`
- `wasm_worker_timeout_policy()`
- `wasm_worker_state_keys()`
- `wasm_worker_lifecycle_phase_keys()`
- `wasm_worker_execute_phase_keys()`
- `wasm_worker_blocker_keys()`
- `wasm_worker_blocker_error(blocker_key)`
- `wasm_worker_blockers()`
- `wasm_worker_start()`
- `wasm_worker_terminate()`
- `wasm_worker_recycle()`
- `wasm_worker_execute(source)`
- `WasmWorkerSession` (stateful wrapper)

## Current Contract (Unwired Baseline)

`wasm_worker_info()` returns:

- `supported = false`
- `state = "unwired"`
- `interruption_model = "worker_recycle"`
- `blocker_count = len(wasm_worker_blocker_keys())`

`wasm_worker_timeout_policy()` currently returns:

- `default_timeout_ms = 5000`
- `min_timeout_ms = 50`
- `max_timeout_ms = 120000`
- `recycle_on_timeout = true`
- `enforcement_supported = false`
- `unsupported_phase = "unsupported_worker_timeout_enforcement"`
- `unsupported_reason = "wasm worker runtime is not wired yet"`

`wasm_worker_state_keys()` currently includes:

- `unwired`
- `starting`
- `ready`
- `busy`
- `terminating`
- `failed`

`wasm_worker_lifecycle_phase_keys()` currently includes:

- `unsupported_worker_start`
- `unsupported_worker_terminate`
- `unsupported_worker_recycle`

`wasm_worker_execute_phase_keys()` currently includes:

- `syntax_error`
- `compile_error`
- `unsupported_worker_execution`

`wasm_worker_blocker_keys()` currently returns:

- `worker_runtime_unwired`

`wasm_worker_blocker_error("worker_runtime_unwired")` returns:

- `"wasm worker runtime is not wired yet"`

Unknown blocker keys return `None`.

`wasm_worker_blockers()` currently returns one structured row with:

- `key = "worker_runtime_unwired"`
- `message = "wasm worker runtime is not wired yet"`

`wasm_worker_start()` currently returns:

- `success = false`
- `phase = "unsupported_worker_start"`
- `state = "unwired"`
- `blocker_key = "worker_runtime_unwired"`
- `error = "wasm worker runtime is not wired yet"`

`wasm_worker_terminate()` currently returns:

- `success = false`
- `phase = "unsupported_worker_terminate"`
- `state = "unwired"`
- `blocker_key = "worker_runtime_unwired"`
- `error = "wasm worker runtime is not wired yet"`

`wasm_worker_recycle()` currently returns:

- `success = false`
- `phase = "unsupported_worker_recycle"`
- `state = "unwired"`
- `blocker_key = "worker_runtime_unwired"`
- `error = "wasm worker runtime is not wired yet"`

`wasm_worker_execute(source)` currently returns:

- `phase = "syntax_error"` when parse fails,
- `phase = "compile_error"` when parse succeeds but compile fails,
- `phase = "unsupported_worker_execution"` when parse+compile succeed but worker backend is unwired.

`WasmWorkerSession` currently wraps lifecycle calls and tracks:

- `starts_requested`
- `terminates_requested`
- `recycles_requested`
- `executes_requested`
- `last_phase`
- `last_error`

## State Model (Planned)

Future states should evolve without breaking existing consumers:

1. `unwired`
2. `starting`
3. `ready`
4. `busy`
5. `terminating`
6. `failed`

The transition from `unwired` to later states is a future milestone; current
clients must treat worker execution as unsupported.

## Interruption Model

`interruption_model = "worker_recycle"` is a design commitment:

- hard cancellation is modeled as worker termination + restart, not in-thread
  signal interruption,
- this keeps browser UI responsive and avoids undefined partial interpreter state.

## Compatibility Rules

1. Existing keys/values remain stable until API version bump.
2. New worker blocker keys may be added, but existing key semantics must not change.
3. `wasm_worker_info().blocker_count` must always match blocker-key export length.
4. Worker lifecycle stubs must keep stable `phase` identifiers until API version bump.
5. Worker execute stubs must keep stable `phase` identifiers until API version bump.
6. Clients should branch on exported key lists, not hardcoded literals.

## Contract Fixtures

Worker lifecycle contract snapshots are tracked in:

- `tests/fixtures/wasm_worker_contract.rs`
- `tests/wasm_contract.rs`

Client orchestration guidance:

- `docs/WASM_CLIENT_INTEGRATION_FLOW.md`
