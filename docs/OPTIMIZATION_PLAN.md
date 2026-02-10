# Optimization Plan (CPython-Referenced)

## Scope

This document is the active execution plan for the performance sprint.
During this sprint, performance work takes precedence over Milestone 13 functional closure.

Primary benchmark gate:
- Command: `time target/release/pyrs -c "fib = lambda n: n if n < 2 else fib(n-1) + fib(n-2); print(fib(29))"`
- Target: `< 0.10s` user-time
- Current baseline (latest run): about `1.00s` user-time

## Ground Rules

1. No patchy micro-fixes without architecture rationale.
2. Every optimization must be validated with:
   - targeted tests (`cargo test --lib`, `cargo test --test vm`)
   - benchmark deltas
   - profiler evidence (`cargo flamegraph`)
3. Every optimization wave should map explicitly to CPython internals.

## CPython Source References

- Eval loop and adaptive dispatch:
  - `/Users/$USER/Downloads/Python-3.14.3/Python/ceval.c`
  - `/Users/$USER/Downloads/Python-3.14.3/Python/generated_cases.c.h`
- Frame and localsplus lifecycle:
  - `/Users/$USER/Downloads/Python-3.14.3/Include/internal/pycore_frame.h`
  - `/Users/$USER/Downloads/Python-3.14.3/Python/frame.c`
- Call/vectorcall paths:
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/call.c`
  - `/Users/$USER/Downloads/Python-3.14.3/Include/cpython/abstract.h`
- Integer and small-int behavior:
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/longobject.c`

## Completed in This Sprint

1. Removed bigint conversion from hot int arithmetic/comparison fast paths.
2. Added single-arg call path specialization for `CALL_FUNCTION`.
3. Added simple positional function-call fast path for common Python function calls.
4. Removed eager error-formatting overhead in `pop_value()` success path.
5. Added precomputed positional parameter binding indexes on `CodeObject`.

## Current Hotspots (Post-Change)

1. Function frame setup overhead (`push_simple_positional_function_frame`, `Frame::new`).
2. Name hashing during binding/global lookup (`hashbrown::map::make_hash`).
3. Generic opcode dispatch overhead in the eval loop.

## Execution Plan

### Phase 1: Frame/Call Path Closure

1. Introduce a lightweight function-frame path that avoids class/module frame baggage.
2. Add frame-object pooling/freelist for non-generator function frames.
3. Remove per-call temporary allocations in argument plumbing for hot arities (`argc=1`, `argc=2`).

### Phase 2: CPython-Style Fast Dispatch

1. Add quickened specialized opcode paths for hot integer operations:
   - compare-int
   - add-int
   - sub-int
2. Add cached global/builtin lookup path for repeated `LOAD_GLOBAL` names.
3. Use profiler-driven opcode frequency data to choose first specialization set.

### Phase 3: Data/Lookup Fast Paths

1. Reduce hash-map churn in local/global lookups on hot code paths.
2. Introduce compact per-frame lookup caches where semantics permit.
3. Validate against CPython behavior for invalidation and shadowing rules.

### Phase 4: Toolchain and Build Optimizations

1. Evaluate `target-cpu=native` release profile for local perf measurement.
2. Add PGO/BOLT exploration branch for release artifacts.
3. Keep default CI/release behavior deterministic unless explicitly switched.

## Stop Conditions for Sprint

1. `fib(29)` reaches `< 0.10s` user-time.
2. No regressions in:
   - `cargo test --lib`
   - `cargo test --test vm`
   - `cargo test --test cpython_harness` (non-strict lane)
3. Profiling shows no single avoidable hotspot above ~5% in the core benchmark.
