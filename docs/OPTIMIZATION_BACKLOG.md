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
- Canonical reference (non-JIT):
  - `time python3.10 -c "fib = lambda n: n if n < 2 else fib(n-1) + fib(n-2); [fib(29) for _ in range(5)]"`
- Target:
  - `< 0.15s` user-time
- Current:
  - ~`0.61-0.63s` user-time (`~0.61-0.64s` wall) for the `fib(29)x5` gate
  - `python3.10` baseline for the same gate: ~`0.50s` user-time
  - ~`0.12s` user-time for `print(fib(29))` single-run reference
  - `python3.10` baseline for `print(fib(29))`: ~`0.11s` user-time
  - release `fib(29)x5` list-comprehension run is currently blocked by a known regression (`OPT-028`) and requires closure before we can treat this gate as authoritative again.

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
| `OPT-023` | P0 | dispatch | Add `LOAD_ATTR`/method-call inline cache specialization path (type/version guarded) | `ceval.c`, `generated_cases.c.h` | `[ ]` |
| `OPT-024` | P1 | calls | Extend call specialization beyond `CALL_FUNCTION` (`CALL_KW`, bound-method calls, builtin/vectorcall analog path) | `call.c`, `ceval.c` | `[ ]` |
| `OPT-025` | P1 | containers | Dict/set probe/load-factor/resizing tuning against CPython behavior (not just correctness) | `dictobject.c`, `setobject.c` | `[ ]` |
| `OPT-026` | P1 | allocations | Add allocator/freelist strategy for hot temporary objects and call argument buffers | `frame.c`, `dictobject.c`, `call.c` | `[ ]` |
| `OPT-027` | P0 | value model | Shrink `Value` payload by boxing heavyweight inline variants used in hot VM transport paths | `ceval.c` value-pointer transport model | `[~]` |
| `OPT-028` | P0 | dispatch correctness | Restore release-path list comprehension/iterator correctness (`FOR_ITER` and list-comp call lanes) so `fib(29)x5` gate is runnable and trustworthy | `ceval.c`, `generated_cases.c.h` | `[~]` |

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
  - borrowed simple-frame acquisition + strict simple-frame `ReturnValue` fast-return path landed for clean one-arg no-cells frames,
  - this wave reduced `fib(29)x5` from about `1.34s` user-time to about `1.00s` user-time, but frame push/recycle and eval-loop dispatch still dominate profiles.
- Latest dispatch checkpoint:
  - `LoadFast` now classifies quickening sites once (`LoadFastPlain` vs `LoadFastCompareLtConstJump`) and stores fused compare-jump metadata in per-site frame cache, removing repeated pattern-scan overhead at hot load sites,
  - conservative `LoadFast -> ReturnValue` fast-return fusion landed for strict clean one-arg no-cells frames,
  - `RETURN_VALUE` path now uses direct frame-local stack pops (removing `pop_value().unwrap_or(...)` overhead),
  - benchmark improved modestly to about `0.95s` user; dominant bottleneck remains frame/call churn + eval-loop dispatch.
- Latest value-model checkpoint:
  - `Value::Exception` now stores boxed exception payloads, removing large inline exception object copies from hot stack/value transport paths,
  - `Value::Slice` now stores boxed slice payloads, reducing enum max-size pressure and value move cost in generic VM operations,
  - benchmark improved to about `0.63-0.64s` user for `fib(29)x5` on release builds; dominant bottleneck remains frame/call setup and eval-loop dispatch.
- Latest call/dispatch checkpoint:
  - `LOAD_GLOBAL` fused direct-call path now executes through cached code/module metadata (no per-hit function-object metadata lookup in that lane),
  - `LOAD_FAST`/`LOAD_FAST2` now use primitive-value clone bypasses (`int`/`float`/`bool`/`None`) for stack push hot paths,
  - removed per-op quickening bookkeeping in `BinaryAdd`/`BinarySub` and `CallFunction1` hot lanes where dedicated opcode fast paths already exist,
  - benchmark currently stabilizes around `0.60-0.61s` user for `fib(29)x5`; remaining top cost is eval-loop dispatch + frame push/pop churn.
- Latest call/dispatch checkpoint:
  - fixed `LOAD_GLOBAL` fused-direct path to avoid borrow-check workarounds and route cached direct no-cells calls through borrowed function metadata paths,
  - one-arg no-cells inline-cache hot path now avoids per-call `code/module/owner_class` cloning and dispatches through `push_simple_positional_function_frame_one_arg_no_cells_from_func`,
  - added slot-0/no-cells fast acquire path for same-module/no-owner frames; profiling still shows simple-frame acquisition and eval-loop dispatch as top remaining costs.
- Latest call/dispatch checkpoint:
  - added a dedicated slot-0 simple-frame recycle fast path and wired strict fast-return sites to use it with owner-aware fallback,
  - split release `LOAD_FAST` quickened handling into explicit hot-site branches (already-quickened compare-jump/plain vs first-time probe) to reduce repeated probe overhead,
  - current warm benchmark remains around `0.61-0.63s` user for `fib(29)x5`; remaining gap is still dominated by eval-loop dispatch and recursive frame/call churn.
- Latest dispatch checkpoint:
  - moved opcode execution body out of `Vm::run`'s per-iteration inline closure into `Vm::execute_instruction`,
  - fast-loop benchmark for `print(fib(29))` now measures around `0.12s` user-time on warm release runs,
  - release-path list-comprehension regression remains open and is now tracked explicitly as `OPT-028` before treating `fib(29)x5` as the primary pass/fail gate again.
