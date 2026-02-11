# Optimization Plan (CPython-Referenced)

## Scope

This is the active execution plan for the optimization sprint.
During this sprint, broad runtime throughput work takes precedence over Milestone 13 functional closure.
The canonical optimization status ledger is `docs/OPTIMIZATION_BACKLOG.md`.

## Current Benchmark Suite (Not Fib-Only)

Run these in release mode:

1. `scripts/bench_fib_gate.sh 5`
2. `scripts/bench_dispatch_hotpath.sh 5`
3. `scripts/bench_dict_backend.sh 5`

Latest local snapshot (2026-02-11):

- `fib(29)x5`: `pyrs ~0.54-0.55s` user vs `python3.10 ~0.50-0.51s` user (`~1.08-1.10x`)
- Dispatch hotpath: `pyrs ~0.46-0.60s` vs `python3.10 ~0.055-0.058s` (`~8-10x`)
- Dict microbench: `pyrs ~0.25s` vs `python3.10 ~0.02s`
- Pickle hotspot: `pyrs ~5.1-5.2s` vs `python3.10 ~0.42-0.45s` (`~11-12x`)

Interpretation:
- Recursive arithmetic is no longer the dominant performance blocker.
- Remaining P0 performance risk is dispatch/call/container and stdlib-hotpath overhead.

## Ground Rules

1. No tactical micro-fixes without architecture rationale.
2. Every optimization wave must include:
   - targeted correctness tests (`cargo test --lib`, `cargo test --test vm`)
   - benchmark deltas from the suite above
   - profiler evidence (`cargo flamegraph` or `sample` artifacts in `perf/`)
3. Every wave must map to a CPython reference surface.
4. `docs/OPTIMIZATION_BACKLOG.md` must be updated in the same checkpoint as behavior/perf changes.

## CPython Source Reference Map

- Eval loop and specialization:
  - `/Users/$USER/Downloads/Python-3.14.3/Python/ceval.c`
  - `/Users/$USER/Downloads/Python-3.14.3/Python/generated_cases.c.h`
- Frame model and lifecycle:
  - `/Users/$USER/Downloads/Python-3.14.3/Include/internal/pycore_frame.h`
  - `/Users/$USER/Downloads/Python-3.14.3/Python/frame.c`
- Call protocol/vectorcall:
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/call.c`
  - `/Users/$USER/Downloads/Python-3.14.3/Include/cpython/abstract.h`
- Integer/small-int behavior:
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/longobject.c`
- Unicode/interning:
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/unicodeobject.c`
  - `/Users/$USER/Downloads/Python-3.14.3/Include/internal/pycore_unicodeobject.h`
- Dict/set internals:
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/dictobject.c`
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/setobject.c`

## Execution Workstreams

### 1) Dispatch and Attribute/Method Specialization (P0)

- Complete `OPT-023`: guarded `LOAD_ATTR` and method-call inline cache paths.
- Reduce eval-loop indirection and branch overhead in generic opcode dispatch.
- Ensure cache invalidation parity (type/version mutation paths).

### 2) Call Path Throughput (P0)

- Complete `OPT-024`: broaden call specialization coverage (`CALL_KW`, bound methods, builtin-style fast paths).
  - Current checkpoint: no-keyword single-argument builtin `len` call-site fast lane is in place for hot container loops.
  - Current checkpoint: module-scope `LOAD_NAME`/`STORE_NAME` overhead reduced by avoiding per-opcode name cloning and using indexed store path.
  - Current checkpoint: no-keyword builtin `bool` zero/single-arg fast lanes are in place in dispatch.
  - Current checkpoint: `CALL_FUNCTION`/`CALL_FUNCTION1` builtin branches now hit zero/one-arg no-kwargs fast lanes before generic builtin dispatch.
  - Current checkpoint: module-scope `LOAD_NAME` now uses version-guarded site caching for hash-churn reduction in top-level loops.
  - Current checkpoint: module global upserts now synchronize module-frame fast-local slots for correctness under accelerated `LOAD_NAME`.
  - Current checkpoint: `LOAD_NAME` and indexed `STORE_NAME` now resolve fast-local slots by opcode name-index directly (no `name_to_index` hash lookup on the hot path).
- Remove avoidable temporary allocations/clone churn in argument plumbing.
- Align frame setup/teardown with CPython fast-call lifecycle patterns.

### 3) Container/Lookup Throughput (P1 with P0 impact)

- Complete `OPT-025`: dict/set probe/load-factor/resizing tuning against CPython behavior.
  - Current checkpoint: dict entry->slot backreference map landed to remove O(slots) delete scans; continue with probe/load-factor tuning and set parity/perf closure.
- Continue clone/allocation audit for container hot paths.
- Keep semantic parity checks in lockstep with perf changes.

### 4) String Interning and Allocation Strategy (P0/P1)

- Complete `OPT-022`: explicit interning policy for identifiers, attribute names, and module-global keys.
- Complete `OPT-026`: freelist/buffer reuse strategy for hot temporary objects and call buffers.

### 5) Startup and End-to-End Throughput (P1)

- Execute `OPT-016`: import/startup overhead audit and reductions where semantics permit.
- Keep strict stdlib and curated harness lanes green after each perf wave.

## Exit Criteria for Optimization Sprint

1. No single microbenchmark dominates optimization priorities.
2. Dispatch hotpath and dict/pickle hotspot ratios are materially reduced (tracked in backlog).
3. P0 optimization backlog items are `[x]` or have documented closure criteria/owners.
4. No correctness regressions in:
   - `cargo test --lib`
   - `cargo test --test vm`
   - `cargo test --test cpython_harness` (curated non-strict lane)
