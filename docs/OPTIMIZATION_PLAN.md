# Optimization Plan (CPython-Referenced)

## Scope

This document is the active execution plan for the performance sprint.
During this sprint, performance work takes precedence over Milestone 13 functional closure.
The permanent optimization status ledger is `docs/OPTIMIZATION_BACKLOG.md`.

## Gap Audit (2026-02-10)

The previous optimization plan under-specified several foundational CPython performance surfaces.
These are now tracked in `docs/OPTIMIZATION_BACKLOG.md` as:
- `OPT-021` small-int/immortal integer strategy
- `OPT-022` explicit string interning strategy
- `OPT-023` `LOAD_ATTR`/method-call inline cache specialization
- `OPT-024` broader call-path specialization (`CALL_KW`, bound-method, builtin/vectorcall analogs)
- `OPT-025` dict/set probing/resizing performance tuning
- `OPT-026` allocator/freelist strategy for hot temporaries

Primary benchmark gate:
- Command: `time target/release/pyrs -c "fib = lambda n: n if n < 2 else fib(n-1) + fib(n-2); [fib(29) for _ in range(5)]"`
- Canonical reference (non-JIT): `time python3.10 -c "fib = lambda n: n if n < 2 else fib(n-1) + fib(n-2); [fib(29) for _ in range(5)]"`
- Target: `< 0.15s` user-time
- Current baseline (latest run): about `0.60-0.61s` user-time (`~0.62-0.64s` wall)
- `python3.10` baseline for same gate: about `0.50s` user-time
- Latest checkpoint before this wave: about `0.95s` user-time (`~0.96s` wall after warm-up)

## Ground Rules

1. No patchy micro-fixes without architecture rationale.
2. Every optimization must be validated with:
   - targeted tests (`cargo test --lib`, `cargo test --test vm`)
   - benchmark deltas
   - profiler evidence (`cargo flamegraph` when available, otherwise `sample` captures in `perf/`)
3. Every optimization wave should map explicitly to CPython internals.
4. Item status must be updated in `docs/OPTIMIZATION_BACKLOG.md` in the same checkpoint.

Canonical profiler command for this sprint:
- `mkdir -p perf && CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --bin pyrs --output perf/fib35_after_single_slot_fill.svg -- -S -c "fib = lambda n: n if n < 2 else fib(n-1) + fib(n-2); print(fib(35))"`

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
6. Added `LOAD_GLOBAL` cache path keyed by `(code, name index)` with invalidation on global mutation paths.
7. Added hot-opcode fast paths for `LoadFast`, `LoadFast2`, `BinaryAdd`, `BinarySub`, `CompareLt`, and `CallFunction(argc=1)` stack pop.
8. Reduced per-opcode finalizer polling overhead by gating on pending-finalizer state.
9. Replaced global hash-map `LOAD_GLOBAL` cache lookups with per-site frame inline cache slots guarded by VM cache epoch invalidation.
10. Removed eager one-arg call-site cache cloning on hot path and retained cache only as guarded call metadata.
11. Reworked `LOAD_GLOBAL` cache guards to CPython-style namespace version checks (`function_globals` version + builtins version), removing VM-wide cache epoch invalidation.
12. Added direct `CALL_FUNCTION` arity-2/arity-3 fast paths for plain positional functions.
13. Extended positional binding precompute data for args 1/2 and routed hot simple-call frame setup through shared frame-prep/store helpers.
14. Landed CPython-range small-int fast-ID path (`[-5, 256]`) to avoid hash-map growth on immediate-id hot/value paths.
15. Added a dedicated one-arg plain-function fast path for no-closure/non-generator calls (direct fast-local bind) to reduce generic call/setup overhead on recursive workloads.
16. Added dedicated `CallFunction1` opcode lowering for one-positional-arg calls (compiler -> VM).
17. Added bool fast path in `JumpIfTrue/JumpIfFalse` to skip generic truthiness conversion for common compare-result branches.
18. Added release-path fused branch evaluation for `CompareLt/CompareLtConst` followed by `JumpIfFalse` (no intermediate stack bool).
19. Added release-path fused recursive-call sequence for `LoadGlobal + LoadFast + BinarySubConst + CallFunction1`.
20. Removed duplicate simple-frame scrub work by scrubbing on recycle and doing minimal state prep on acquire for one-arg no-cells pooled frames.
21. Added by-reference int/bool fast path in fused `LOAD_FAST - CONST` call preparation to avoid hot-path `Value` clone churn before fallback.
22. Added fused `LoadFast + CompareLtConst + JumpIfFalse` release-path branch evaluation to skip stack push/pop + compare dispatch for int/bool locals.
23. Added `LOAD_GLOBAL` fused-call small-int RHS cache (`fused_const_small_int`) and direct fast subtract path to avoid repeated constant decoding + clone fallback churn.
24. Added borrowed simple-frame acquisition path for fused direct calls plus a strict `ReturnValue` fast-return path for clean simple one-arg no-cells frames.
25. Added `LoadFast` per-site quickening classification (`LoadFastPlain` / `LoadFastCompareLtConstJump`) so compare-jump fusion probing is done once per site instead of repeated pattern scans.
26. Added conservative `LoadFast -> ReturnValue` fast-return fusion for strict clean one-arg no-cells frames, with full fallback to generic return behavior.
27. Removed `pop_value().unwrap_or(...)` overhead from hot `RETURN_VALUE` paths by direct stack-pop in frame-local return handling.
28. Reduced `Value` payload footprint by boxing heavyweight variants used in VM transport (`ExceptionObject` and slice payload), shrinking stack/clone/move churn in hot loops.
29. Fixed `LOAD_GLOBAL` fused-direct borrow contention by splitting cache metadata extraction from mutable VM operations.
30. Routed one-arg no-cells direct-call execution through borrowed function metadata paths (avoiding per-call `code/module/owner_class` clone in that lane).
31. Added a slot-0/no-cells simple-frame fast acquire path for same-module/no-owner calls.

## Current Hotspots (Post-Change)

1. Function-call setup overhead (`push_function_call_one_arg_from_obj`) remains dominant.
2. Generic opcode dispatch overhead in the eval loop (`run::_closure`) remains dominant.
3. Frame construction/reset overhead (`acquire_frame`) is improved but still visible in recursion-heavy code.
4. Stack movement/copy work (`_platform_memmove`) remains significant in tight recursive loops.
5. Attribute/method lookup and interning gaps remain for broader workloads (`OPT-022`, `OPT-023`).
6. Recursive-call workloads are still dominated by frame/call setup and stack churn; current `fib(29)x5` remains around `0.60-0.61s` user-time (target `<0.15s`).
7. Dict subscripting now routes through hash-probing backend lookup in `getitem` paths (linear scan bypass removed); remaining primary gap is recursive call/dispatch overhead, not dict key lookup.

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
4. Add `LOAD_ATTR` + method-call inline cache specialization with guarded invalidation.

### Phase 3: Data/Lookup Fast Paths

1. Reduce hash-map churn in local/global lookups on hot code paths.
2. Introduce compact per-frame lookup caches where semantics permit.
3. Validate against CPython behavior for invalidation and shadowing rules.
4. Add explicit string interning policy (identifier names, attribute names, module global keys).
5. Land small-int/immortal integer strategy decision and implementation with parity/perf proof.

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
