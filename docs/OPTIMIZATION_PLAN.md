# Optimization Plan (CPython-Referenced)

## Scope
This document defines optimization execution policy and workstreams.
Functional closure remains Milestone 13 priority; optimization changes are merged continuously behind correctness gates.
Canonical optimization status lives in `docs/OPTIMIZATION_BACKLOG.md`.

## Benchmark Suite (Canonical)
Run in release mode:
1. `scripts/bench_fib_gate.sh 5`
2. `scripts/bench_dispatch_hotpath.sh 5`
3. `scripts/bench_dict_backend.sh 5`

Use benchmark outputs in `perf/` as the current source of truth.
Do not treat point-in-time numbers in docs as authoritative.

## Ground Rules
1. No optimization without semantic parity coverage.
2. No benchmark-only shortcuts that weaken architecture.
3. Every optimization wave must include:
   - targeted correctness tests (`cargo test --lib`, `cargo test --test vm`)
   - benchmark deltas (suite above)
   - profiler evidence (`cargo flamegraph` or `sample` artifacts in `perf/`)
4. Every wave must map to CPython references and update `docs/OPTIMIZATION_BACKLOG.md` in the same commit.

## CPython Reference Map
- Eval loop and specialization:
  - `.local/Python-3.14.3/Python/ceval.c`
  - `.local/Python-3.14.3/Python/generated_cases.c.h`
- Frame model/lifecycle:
  - `.local/Python-3.14.3/Include/internal/pycore_frame.h`
  - `.local/Python-3.14.3/Python/frame.c`
- Call protocol/vectorcall:
  - `.local/Python-3.14.3/Objects/call.c`
  - `.local/Python-3.14.3/Include/cpython/abstract.h`
- Integer model:
  - `.local/Python-3.14.3/Objects/longobject.c`
- Unicode/interning model:
  - `.local/Python-3.14.3/Objects/unicodeobject.c`
  - `.local/Python-3.14.3/Include/internal/pycore_unicodeobject.h`
- Dict/set internals:
  - `.local/Python-3.14.3/Objects/dictobject.c`
  - `.local/Python-3.14.3/Objects/setobject.c`

## Active Workstreams
1. Dispatch and attribute/method specialization (`OPT-023`, `OPT-014`)
2. Call-path throughput and specialization breadth (`OPT-024`, `OPT-010`)
3. Container throughput tuning with parity guarantees (`OPT-025`, `OPT-015`)
4. String interning + allocation strategy closure (`OPT-022`, `OPT-026`)
5. Startup and end-to-end import overhead closure (`OPT-016`)
6. Builtin hot-path optimization under parity gate (`OPT-029`, policy in `docs/BUILTIN_OPTIMIZATION_POLICY.md`)
7. Automatic GC trigger strategy closure (`OPT-030`) with parity-safe defaults

## Exit Criteria (Optimization Milestone)
1. P0 optimization items in `docs/OPTIMIZATION_BACKLOG.md` are closed or explicitly downgraded with rationale.
2. Benchmark deltas show sustained throughput improvements on dispatch/container-heavy workloads.
3. No regressions in:
   - `cargo test --lib`
   - `cargo test --test vm`
   - `cargo test --test cpython_harness` (curated lane)
4. Builtin parity gate remains green.
