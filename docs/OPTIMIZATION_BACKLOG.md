# Optimization Backlog and Status

## Purpose

This is the permanent, canonical optimization checklist for `pyrs`.
Every optimization item must be tracked here with explicit status.

Last updated: 2026-02-11

## Status Legend

- `[x]` done
- `[~]` in progress
- `[ ]` planned
- `[!]` blocked (requires prerequisite decision/work)

## Primary Performance Gate

- Command:
  - `time target/release/pyrs -c "fib = lambda n: n if n < 2 else fib(n-1) + fib(n-2); [fib(29) for _ in range(5)]"`
- Target:
  - `< 0.15s` user-time
- Current:
  - ~`1.11s` user-time (`~1.13s` wall, warm) for the `fib(29)x5` gate
  - ~`0.24s` user-time for `print(fib(29))` single-run reference

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
| `OPT-014` | P1 | dispatch | Reduce per-opcode branch/indirection overhead in main eval loop | `ceval.c` dispatch structure | `[ ]` |
| `OPT-015` | P1 | containers | Dict/set hot-path operations benchmark and algorithmic closure | `dictobject.c`, `setobject.c` | `[~]` |
| `OPT-016` | P1 | startup | Reduce startup/import overhead in non-stdlib benchmark mode where safe | CPython startup path | `[ ]` |
| `OPT-017` | P1 | allocation | Audit and eliminate avoidable `clone`/temporary allocations in hot VM paths | N/A (local audit) | `[~]` |
| `OPT-018` | P1 | toolchain | Evaluate local `target-cpu=native` measurement profile | N/A (toolchain) | `[ ]` |
| `OPT-019` | P2 | toolchain | Evaluate PGO/BOLT branch for release artifacts | CPython PGO precedent | `[ ]` |
| `OPT-020` | P0 | validation | Keep benchmark + flamegraph regression gate for each optimization wave | N/A | `[~]` |
| `OPT-021` | P0 | integer model | CPython small-int/immortal integer strategy review and implementation decision (`[-5, 256]` cache equivalent or explicit immediate-int justification with parity/perf proof) | `longobject.c` | `[x]` |
| `OPT-022` | P0 | unicode | Implement explicit string interning strategy for identifiers/attribute names/module globals (and wire compiler/import call sites) | `unicodeobject.c`, `pycore_unicodeobject.h` | `[~]` |
| `OPT-023` | P0 | dispatch | Add `LOAD_ATTR`/method-call inline cache specialization path (type/version guarded) | `ceval.c`, `generated_cases.c.h` | `[ ]` |
| `OPT-024` | P1 | calls | Extend call specialization beyond `CALL_FUNCTION` (`CALL_KW`, bound-method calls, builtin/vectorcall analog path) | `call.c`, `ceval.c` | `[ ]` |
| `OPT-025` | P1 | containers | Dict/set probe/load-factor/resizing tuning against CPython behavior (not just correctness) | `dictobject.c`, `setobject.c` | `[ ]` |
| `OPT-026` | P1 | allocations | Add allocator/freelist strategy for hot temporary objects and call argument buffers | `frame.c`, `dictobject.c`, `call.c` | `[ ]` |

## Rules For This Backlog

1. Every optimization commit must update relevant item status here.
2. New optimization ideas must be added as new `OPT-*` rows before implementation.
3. Do not mark sprint complete until all P0 items required for target gate are `[x]`.

## Current Notes

- Latest landed checkpoint:
  - per-site frame `LOAD_GLOBAL` inline cache slots with VM epoch invalidation (replacing hot global hash-map cache lookup),
  - one-arg call-site cache hot-path clone removal,
  - regression tests for global cache invalidation on `StoreGlobal` and module-attribute mutation.
- New landed checkpoint:
  - `LOAD_GLOBAL` cache now guarded by namespace versions (`function_globals` + `builtins`) with frame-version propagation on module writes,
  - direct `CALL_FUNCTION` arity-2/3 specialization paths added for plain positional calls,
  - small-int `id()` fast cache for CPython range `[-5, 256]` added in `Heap`,
  - initial `OPT-022` wiring started by reducing repeat module-global key allocation (`get_mut`/upsert path instead of unconditional key reallocation).
- Latest call-path checkpoint:
  - one-arg plain-function fast path now bypasses generic cell/binding setup for no-closure, non-generator call shapes;
  - compiler now lowers one-positional-arg calls to dedicated `CallFunction1` opcode;
  - runtime fuses `CompareLt* + JumpIfFalse` and `LoadGlobal + LoadFast + BinarySubConst + CallFunction1` on release builds;
  - benchmark currently stabilizes around `0.26-0.27s` user-time for `fib(29)` (still above target).
- `OPT-009` remains in progress: boxed-frame pool and reuse path are active, but profiling still shows frame setup/reset as a visible hotspot in recursive call workloads.
- Latest dict-path checkpoint:
  - fixed `Vm::getitem_value` for `Value::Dict` to route through `DictObject` backend lookup APIs (`find_with_hash`) instead of linear `entries.iter().find(...)` scans with generic `==`,
  - dict microbench regression is resolved: `200k` insert+getitem loop now runs around `0.33s` user-time (previously observed in multi-second to extreme outlier ranges when subscripting bypassed backend probing).
- Latest call-path checkpoint:
  - simple-frame pool preparation now avoids duplicate full scrub on acquire (frames are scrubbed on recycle and minimally prepared on acquire),
  - fused `LOAD_FAST - CONST` one-arg call path now uses by-reference int/bool arithmetic fast path before generic `Value` cloning fallback,
  - recursive benchmark is currently stable around `1.11s` user-time for `fib(29)x5` across repeated warm runs.
- Latest dispatch/call-path checkpoint:
  - release-path `LoadFast + CompareLtConst + JumpIfFalse` fusion now bypasses intermediate stack bool work for int/bool locals,
  - `LOAD_GLOBAL` fused one-arg call path now caches small-int RHS constants and routes through a direct subtract helper before fallback,
  - this wave reduced `fib(29)x5` from about `1.34s` user-time to about `1.11s` user-time, but frame push/recycle and eval-loop dispatch still dominate profiles.
