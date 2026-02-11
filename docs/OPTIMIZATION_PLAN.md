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

- `fib(29)x5`: `pyrs ~0.54s` user vs `python3.10 ~0.50s` user (`~1.07x`)
- Dispatch hotpath: `pyrs ~0.955s` vs `python3.10 ~0.057s` (`~16.7x`)
- Dict microbench: `pyrs ~0.28s` vs `python3.10 ~0.01s`
- Pickle hotspot: `pyrs ~6.39s` vs `python3.10 ~0.46s` (`~13.9x`)

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
- Remove avoidable temporary allocations/clone churn in argument plumbing.
- Align frame setup/teardown with CPython fast-call lifecycle patterns.

### 3) Container/Lookup Throughput (P1 with P0 impact)

- Complete `OPT-025`: dict/set probe/load-factor/resizing tuning against CPython behavior.
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
