# WASM Worker Runtime Contract (codex/wasm)

Status: branch-local draft.

This document defines the browser worker-runtime contract currently exposed by:

- `wasm_worker_info()`
- `wasm_worker_blocker_keys()`
- `wasm_worker_blocker_error(blocker_key)`

## Current Contract (Unwired Baseline)

`wasm_worker_info()` returns:

- `supported = false`
- `state = "unwired"`
- `interruption_model = "worker_recycle"`
- `blocker_count = len(wasm_worker_blocker_keys())`

`wasm_worker_blocker_keys()` currently returns:

- `worker_runtime_unwired`

`wasm_worker_blocker_error("worker_runtime_unwired")` returns:

- `"wasm worker runtime is not wired yet"`

Unknown blocker keys return `None`.

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
