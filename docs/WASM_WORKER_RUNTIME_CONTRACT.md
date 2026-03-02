# WASM Worker Runtime Contract (codex/wasm)

Status: branch-local draft.

This document defines the browser worker-runtime contract currently exposed by:

- `wasm_worker_info()`
- `wasm_worker_timeout_policy()`
- `wasm_worker_timeout_phase_keys()`
- `wasm_worker_state_keys()`
- `wasm_worker_lifecycle_phase_keys()`
- `wasm_worker_execute_phase_keys()`
- `wasm_worker_blocker_keys()`
- `wasm_worker_blocker_error(blocker_key)`
- `wasm_worker_blockers()`
- `wasm_worker_start()`
- `wasm_worker_terminate()`
- `wasm_worker_recycle()`
- `wasm_worker_set_timeout(timeout_ms)`
- `wasm_worker_execute(source)`
- `wasm_worker_execute_with_operation(source)`
- `WasmWorkerSession` (stateful wrapper)

## Current Contract (Default + vm-probe Modes)

`wasm_worker_info()` returns:

- `supported = false`
- `backend = "unwired"` in default builds, `"vm_probe"` with `wasm-vm-probe`
- `state = "unwired"` in default builds, `"ready"` with `wasm-vm-probe`
- `interruption_model = "worker_recycle"`
- `lifecycle_supported = false` in default builds, `true` with `wasm-vm-probe`
- `execution_probe_enabled = false` in default builds, `true` with `wasm-vm-probe`
- `execute_supported = false` in default builds, `true` with `wasm-vm-probe`
- `timeout_configuration_supported = false` in default builds, `true` with `wasm-vm-probe`
- `timeout_enforcement_supported = false` in current milestone builds
- `blocker_count = len(wasm_worker_blocker_keys())`

Top-level lifecycle calls now mutate shared worker state, and
`wasm_worker_info().state` reflects that current top-level state.

`wasm_worker_timeout_policy()` currently returns:

- `default_timeout_ms = 5000`
- `min_timeout_ms = 50`
- `max_timeout_ms = 120000`
- `configuration_supported = false` in default builds, `true` with `wasm-vm-probe`
- `recycle_on_timeout = true`
- `enforcement_supported = false`
- `unsupported_phase = "unsupported_worker_timeout_enforcement"`
- `unsupported_reason`:
  - default build: `"wasm worker runtime is not wired yet"`
  - `wasm-vm-probe`:
    `"worker timeout enforcement is not wired yet (wasm-vm-probe currently supports configuration-only updates)"`

`wasm_worker_timeout_phase_keys()` currently includes:

- default build:
  - `unsupported_worker_timeout_enforcement`
  - `invalid_worker_timeout`
- `wasm-vm-probe` build:
  - all default keys, plus:
  - `worker_timeout_configured`

`wasm_worker_state_keys()` currently includes:

- `unwired`
- `starting`
- `ready`
- `busy`
- `terminating`
- `failed`

`wasm_worker_lifecycle_phase_keys()` currently includes:

- default build:
  - `unsupported_worker_start`
  - `unsupported_worker_terminate`
  - `unsupported_worker_recycle`
- `wasm-vm-probe` build:
  - all default keys, plus:
  - `worker_started`
  - `worker_terminated`
  - `worker_recycled`

`wasm_worker_execute_phase_keys()` currently includes:

- default build:
  - `syntax_error`
  - `compile_error`
  - `unsupported_worker_execution`
- `wasm-vm-probe` build:
  - all default keys, plus:
  - `ok`
  - `runtime_error`

`wasm_worker_blocker_keys()` currently returns:

- `worker_runtime_unwired`
- module-policy capability keys currently emitted by worker preflight:
  - `dynamic_library_load`
  - `network_sockets`
  - `process_spawn`
  - `interactive_terminal`

`wasm_worker_blocker_error(key)` currently returns:

- `"wasm worker runtime is not wired yet"` for `worker_runtime_unwired`,
- capability-specific unsupported messages (same message family as
  `wasm_execution_blocker_error`) for known capability keys.

`wasm_worker_blockers()` currently returns structured rows for all worker blocker
keys (`worker_runtime_unwired` + module-policy capability keys).

`wasm_worker_start()` currently returns:

- default build:
  - `success = false`
  - `operation_id = worker_start_<n>`
  - `phase = "unsupported_worker_start"`
  - `state = "unwired"`
  - `blocker_key = "worker_runtime_unwired"`
  - `error = "wasm worker runtime is not wired yet"`
- `wasm-vm-probe` build:
  - `success = true`
  - `operation_id = worker_start_<n>`
  - `phase = "worker_started"`
  - `state = "ready"`
  - `blocker_key = None`
  - `error = None`

`wasm_worker_terminate()` currently returns:

- default build:
  - `success = false`
  - `operation_id = worker_terminate_<n>`
  - `phase = "unsupported_worker_terminate"`
  - `state = "unwired"`
  - `blocker_key = "worker_runtime_unwired"`
  - `error = "wasm worker runtime is not wired yet"`
- `wasm-vm-probe` build:
  - `success = true`
  - `operation_id = worker_terminate_<n>`
  - `phase = "worker_terminated"`
  - `state = "unwired"`
  - `blocker_key = None`
  - `error = None`

`wasm_worker_recycle()` currently returns:

- default build:
  - `success = false`
  - `operation_id = worker_recycle_<n>`
  - `phase = "unsupported_worker_recycle"`
  - `state = "unwired"`
  - `blocker_key = "worker_runtime_unwired"`
  - `error = "wasm worker runtime is not wired yet"`
- `wasm-vm-probe` build:
  - `success = true`
  - `operation_id = worker_recycle_<n>`
  - `phase = "worker_recycled"`
  - `state = "ready"`
  - `blocker_key = None`
  - `error = None`

`wasm_worker_set_timeout(timeout_ms)` currently returns:

- `operation_id = worker_set_timeout_<n>`
- `state` reflects current shared top-level worker state.
- `phase = "invalid_worker_timeout"` for out-of-range values.
- for in-range values (`50..=120000` ms):
  - default build:
    - `phase = "unsupported_worker_timeout_enforcement"`
    - `success = false`
    - `blocker_key = "worker_runtime_unwired"`
    - `error = "wasm worker runtime is not wired yet"`
  - `wasm-vm-probe` build:
    - when `state = "ready"`:
      - `phase = "worker_timeout_configured"`
      - `success = true`
      - `blocker_key = None`
      - `error = None`
    - when `state != "ready"`:
      - `phase = "unsupported_worker_timeout_enforcement"`
      - `success = false`
      - `blocker_key = "worker_runtime_unwired"`
      - `error = "wasm worker runtime is not wired yet"`

`worker_timeout_configured` is configuration-only in vm-probe mode; timeout
enforcement still remains unwired (`enforcement_supported = false`).
Use `configuration_supported` from `wasm_worker_timeout_policy()` for timeout-UI
controls, and `enforcement_supported` for hard runtime-enforcement guarantees.

`wasm_worker_execute(source)` currently returns:

- `phase = "syntax_error"` when parse fails,
- `phase = "compile_error"` when parse succeeds but compile fails,
- parse+compile-valid snippets with known blocked imports:
  - `phase = "unsupported_worker_execution"`
  - `blocker_key = "<capability_key>"` (for example `network_sockets`)
- parse+compile-valid snippets without blocked imports:
  - default build:
    - `phase = "unsupported_worker_execution"`
    - `blocker_key = "worker_runtime_unwired"`
  - `wasm-vm-probe` build:
    - when `state = "ready"`:
      - `phase = "ok"` on VM success
      - `phase = "runtime_error"` on VM runtime failure
      - `blocker_key = None`
    - when `state != "ready"`:
      - `phase = "unsupported_worker_execution"`
      - `blocker_key = "worker_runtime_unwired"`

`wasm_worker_execute_with_operation(source)` mirrors
`wasm_worker_execute(source)` and also returns:

- `operation_id = worker_execute_<n>`
- `state` reflects current shared top-level worker state.

`WasmWorkerSession` currently wraps lifecycle calls and tracks:

- `starts_requested`
- `terminates_requested`
- `recycles_requested`
- `executes_requested`
- `timeout_updates_requested`
- `last_timeout_ms_requested`
- `last_operation_id`
- `last_phase`
- `last_state`
- `last_error`

Session state behavior:

- `last_state` is lifecycle-derived (`start`/`terminate`/`recycle` results).
- `info().state` is session-local: once lifecycle calls run, `info()` reflects
  the session’s most recent lifecycle state (instead of always top-level
  unwired state).
- `execute_with_operation` and `set_timeout_ms` now follow the operation-reported
  shared worker state (so external top-level lifecycle changes are reflected in
  returned/session telemetry state).
- in `wasm-vm-probe`, calling `recycle()` before execute/timeout yields
  `last_state = "ready"` for those follow-up operations.

## State Model (Planned)

Future states should evolve without breaking existing consumers:

1. `unwired`
2. `starting`
3. `ready`
4. `busy`
5. `terminating`
6. `failed`

The transition from true worker lifecycle orchestration is still a future
milestone. Current behavior is:

- default builds stay unwired with explicit unsupported lifecycle phases,
- `wasm-vm-probe` builds expose deterministic lifecycle probe phases
  (`worker_started` / `worker_terminated` / `worker_recycled`) so UI paths can
  validate control flow without a real worker thread.

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
6. `operation_id` fields guarantee prefix shape + per-process uniqueness only;
   absolute numeric ordering across runs is not a contract guarantee.
7. Clients should branch on exported key lists, not hardcoded literals.

## Contract Fixtures

Worker lifecycle contract snapshots are tracked in:

- `tests/fixtures/wasm_worker_contract.rs`
- `tests/wasm_contract.rs`

Client orchestration guidance:

- `docs/WASM_CLIENT_INTEGRATION_FLOW.md`
