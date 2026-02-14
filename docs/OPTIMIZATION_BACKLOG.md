# Optimization Backlog and Status

## Purpose

This is the permanent, canonical optimization checklist for `pyrs`.
Every optimization item must be tracked here with explicit status.

Last updated: 2026-02-14

## Status Legend

- `[x]` done
- `[~]` in progress
- `[ ]` planned
- `[!]` blocked (requires prerequisite decision/work)

## Benchmark Suite (Canonical)

- `scripts/bench_fib_gate.sh 5`
- `scripts/bench_dispatch_hotpath.sh 5`
- `scripts/bench_dict_backend.sh 5`
- `scripts/bench_startup_gate.sh 7`

Use benchmark artifacts in `perf/` as current truth for deltas.
Do not rely on stale point-in-time numbers in this document.

## CPython Reference Map

- Eval/dispatch specialization:
  - `/Users/$USER/Downloads/Python-3.14.3/Python/ceval.c`
  - `/Users/$USER/Downloads/Python-3.14.3/Python/generated_cases.c.h`
- Frame model and lifecycle:
  - `/Users/$USER/Downloads/Python-3.14.3/Include/internal/pycore_frame.h`
  - `/Users/$USER/Downloads/Python-3.14.3/Python/frame.c`
- Call/vectorcall:
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/call.c`
  - `/Users/$USER/Downloads/Python-3.14.3/Include/cpython/abstract.h`
- Integer model:
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/longobject.c`
- Unicode/interning model:
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/unicodeobject.c`
  - `/Users/$USER/Downloads/Python-3.14.3/Include/internal/pycore_unicodeobject.h`
- Dict/set internals:
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/dictobject.c`
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/setobject.c`

## Backlog

| ID | Priority | Area | Optimization Item | CPython Reference | Status |
|---|---|---|---|---|---|
| `OPT-001` | P0 | locals/frame | Slot-backed fast locals as authoritative store (`f_localsplus` direction) | `pycore_frame.h`, `frame.c` | `[x]` |
| `OPT-002` | P0 | calls | Remove full `FunctionObject` clone in opcode call path | `call.c` | `[x]` |
| `OPT-003` | P0 | int ops | i64 fast path before bigint allocation for `+ - * // %` and order compare | `longobject.c` | `[x]` |
| `OPT-004` | P0 | vm hot path | Remove eager formatting/allocation from `pop_value()` success path | `ceval.c` error/slow-path style | `[x]` |
| `OPT-005` | P0 | calls | Simple positional function-call fast path | `call.c` | `[x]` |
| `OPT-006` | P0 | calls | Single-arg `CALL_FUNCTION` specialization path | `call.c` | `[x]` |
| `OPT-007` | P0 | binding | Precompute positional param slot/cell indexes on `CodeObject` | `pycore_frame.h` locals indexing | `[x]` |
| `OPT-008` | P0 | frames | Lightweight function-frame type/path (remove class/module baggage from pure function calls) | `pycore_frame.h`, `ceval.c` | `[~]` |
| `OPT-009` | P0 | frames | Frame freelist/pool for non-generator frames | `frame.c` freelist patterns | `[~]` |
| `OPT-010` | P0 | calls | Fast arity call paths (`argc=2`, `argc=3`) without temporary vec/hashmap churn | `call.c` vectorcall fast arities | `[~]` |
| `OPT-011` | P0 | dispatch | Add adaptive specialized opcodes for hot integer compare/add/sub paths | `generated_cases.c.h` | `[~]` |
| `OPT-012` | P0 | lookup | `LOAD_GLOBAL` cached lookup with invalidation on globals/builtins mutation | `ceval.c` inline cache strategy | `[x]` |
| `OPT-013` | P0 | lookup | Reduce local/global hash churn for repeated name access in hot loops | `ceval.c`, name cache patterns | `[~]` |
| `OPT-014` | P1 | dispatch | Reduce per-opcode branch/indirection overhead in main eval loop | `ceval.c` dispatch structure | `[~]` |
| `OPT-015` | P1 | containers | Dict/set hot-path operations benchmark and algorithmic closure | `dictobject.c`, `setobject.c` | `[~]` |
| `OPT-016` | P1 | startup | Reduce startup/import overhead in non-stdlib benchmark mode where safe | CPython startup path | `[ ]` |
| `OPT-017` | P1 | allocation | Audit and eliminate avoidable `clone`/temporary allocations in hot VM paths | N/A (local audit) | `[~]` |
| `OPT-018` | P1 | toolchain | Evaluate local `target-cpu=native` measurement profile | N/A (toolchain) | `[ ]` |
| `OPT-019` | P2 | toolchain | Evaluate PGO/BOLT branch for release artifacts | CPython PGO precedent | `[ ]` |
| `OPT-020` | P0 | validation | Keep benchmark + flamegraph regression gate for each optimization wave | N/A | `[~]` |
| `OPT-021` | P0 | integer model | CPython small-int/immortal integer strategy review and implementation decision (`[-5, 256]` cache equivalent or explicit immediate-int justification with parity/perf proof) | `longobject.c` | `[x]` |
| `OPT-022` | P0 | unicode | Implement explicit string interning strategy for identifiers/attribute names/module globals (and wire compiler/import call sites) | `unicodeobject.c`, `pycore_unicodeobject.h` | `[~]` |
| `OPT-023` | P0 | dispatch | Add `LOAD_ATTR`/method-call inline cache specialization path (type/version guarded) | `ceval.c`, `generated_cases.c.h` | `[~]` |
| `OPT-024` | P1 | calls | Extend call specialization beyond `CALL_FUNCTION` (`CALL_KW`, bound-method calls, builtin/vectorcall analog path) | `call.c`, `ceval.c` | `[~]` |
| `OPT-025` | P1 | containers | Dict/set probe/load-factor/resizing tuning against CPython behavior (not just correctness) | `dictobject.c`, `setobject.c` | `[~]` |
| `OPT-026` | P1 | allocations | Add allocator/freelist strategy for hot temporary objects and call argument buffers | `frame.c`, `dictobject.c`, `call.c` | `[ ]` |
| `OPT-027` | P0 | value model | Shrink `Value` payload by boxing heavyweight inline variants used in hot VM transport paths | `ceval.c` value-pointer transport model | `[~]` |
| `OPT-028` | P0 | dispatch correctness | Restore release-path list comprehension/iterator correctness (`FOR_ITER` and list-comp call lanes) so `fib(29)x5` gate is runnable and trustworthy | `ceval.c`, `generated_cases.c.h` | `[x]` |
| `OPT-029` | P0 | builtins/calls | Heat-classed builtin call optimization closure (HOT/WARM/COLD policy) with parity-gated fast paths for HOT builtins | `bltinmodule.c`, `call.c`, `ceval.c` | `[~]` |
| `OPT-030` | P1 | gc | Threshold-based automatic cycle collection policy with explicit controls (`gc.set_threshold/get_threshold/get_count`) and parity-safe trigger strategy | `gcmodule.c` | `[~]` |

## Rules For This Backlog

1. Every optimization commit must update relevant item status here.
2. New optimization ideas must be added as new `OPT-*` rows before implementation.
3. Do not mark the sprint complete until P0 items and the benchmark-suite gaps are closed.

## Current Notes

- Phase-1 optimization checkpoint is complete; this backlog now tracks remaining throughput closure.
- Dispatch and call-path specialization is active (`OPT-023`, `OPT-024`) with parity-first guardrails.
- Container performance closure remains open (`OPT-025`) after dict backend architecture improvements.
- String interning and allocation strategy closure remain active (`OPT-022`, `OPT-026`).
- Benchmarks run in CI as telemetry; regressions must be investigated before item closure.
- Builtin parity gate is a mandatory safety rail for builtin call-path optimization work.
- 2026-02-14 wave:
  - landed `LOAD_ATTR` cache coverage for plain instance/class values (in addition to function/descriptor variants).
  - landed `CALL_FUNCTION` site quickening metadata for zero-arg and two-arg direct function lanes.
  - landed `gc` control surface (`enable/disable/isenabled/get_threshold/set_threshold/get_count`); automatic cycle GC is enabled after explicit threshold configuration and guarded to avoid semantic regressions.
  - reduced clone churn in iterator conversion and dict update native paths (`vm_native_dispatch`), including alias-safe `dict.update(self)` handling.
  - reduced clone churn in opcode execution paths (`UNPACK_SEQUENCE` / `UNPACK_EX` / `DICT_UPDATE`) by removing whole-container vector cloning on list/tuple/dict fast paths.
  - reduced `LOAD_GLOBAL` fused-call overhead by caching direct one-arg/no-cells frame metadata (`code/module/owner`) and reusing ref-based frame entry paths after epoch validation.
  - added memoization for filesystem stdlib-preference probes (`has_preferred_filesystem_module`) with invalidation on `sys.path` mutations to reduce startup/import stat churn.
  - removed full tuple payload clones from native tuple methods (`tuple.count`, `tuple.index`) and validated bound/subclass semantics with regression coverage.
  - fused terminal arithmetic return for simple no-cells frames: `BinaryAdd` / `BinarySub` / `BinaryMul` / `BinaryDiv` / `BinaryFloorDiv` / `BinaryMod` now fast-return directly to caller when next opcode is `ReturnValue`, avoiding extra dispatch and stack roundtrips.
  - tightened `CompareLt`/`CompareLtConst` jump path truthiness handling to avoid temporary `Value::Bool` materialization when the result is used only for branch control.
  - release profile now uses `lto = "fat"` (from thin) to improve cross-function optimization in hot VM call/dispatch paths.
  - dispatch hotpath benchmark remains non-regressing after terminal-op fusion extension (`scripts/bench_dispatch_hotpath.sh`: `0.8493s` at `bfeba79` vs `0.8470s` current in local runs; lower is better).
