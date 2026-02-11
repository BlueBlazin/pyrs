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

## Benchmark Suite (Canonical)

- `scripts/bench_fib_gate.sh 5`
- `scripts/bench_dispatch_hotpath.sh 5`
- `scripts/bench_dict_backend.sh 5`

Latest local snapshot (2026-02-11):
- `fib(29)x5`: `pyrs ~0.54s` user vs `python3.10 ~0.50s` user (`~1.08x`)
- dispatch hotpath: `pyrs ~0.54-0.65s` vs `python3.10 ~0.058-0.061s` (`~9-11x`)
- dict microbench: `pyrs ~0.28s` vs `python3.10 ~0.01-0.02s`
- pickle hotspot: `pyrs ~5.1-5.2s` vs `python3.10 ~0.42-0.45s` (`~11-12x`)

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
| `OPT-025` | P1 | containers | Dict/set probe/load-factor/resizing tuning against CPython behavior (not just correctness) | `dictobject.c`, `setobject.c` | `[ ]` |
| `OPT-026` | P1 | allocations | Add allocator/freelist strategy for hot temporary objects and call argument buffers | `frame.c`, `dictobject.c`, `call.c` | `[ ]` |
| `OPT-027` | P0 | value model | Shrink `Value` payload by boxing heavyweight inline variants used in hot VM transport paths | `ceval.c` value-pointer transport model | `[~]` |
| `OPT-028` | P0 | dispatch correctness | Restore release-path list comprehension/iterator correctness (`FOR_ITER` and list-comp call lanes) so `fib(29)x5` gate is runnable and trustworthy | `ceval.c`, `generated_cases.c.h` | `[x]` |

## Rules For This Backlog

1. Every optimization commit must update relevant item status here.
2. New optimization ideas must be added as new `OPT-*` rows before implementation.
3. Do not mark the sprint complete until P0 items and the benchmark-suite gaps are closed.

## Current Notes

- Latest optimization checkpoint:
  - `load_attr_instance` now bypasses generic bound-method invocation when `__getattribute__` resolves to builtin `object.__getattribute__`, routing directly to default slot-style attribute resolution.
  - Added guarded per-site `LOAD_ATTR` instance cache for function/builtin/classmethod/staticmethod descriptors with class/version invalidation (receiver + owner class versions).
  - Upgraded load-attr inline cache to two-way polymorphic slots per site.
  - Added class attribute version tracking and mutation bump points (`STORE_ATTR` / `DELETE_ATTR` / `setattr` / `delattr` class targets).
- Additional checkpoint:
  - `CALL_FUNCTION` now has one/two/three-argument bound-method fast paths that inject the receiver directly into function fast-call lanes instead of routing through generic call dispatch.
  - Extended no-keyword small-arity fast dispatch into `CallCpython`, `CallCpythonKwStack`, and `CallFunctionKw` lanes (including arity-0).
  - Added no-keyword small-arity internal-call fast paths in `call_internal` to reduce call/arg churn in stdlib-heavy paths (notably pickle stack).
  - Dispatch benchmark now sits around `~0.54-0.65s` in current local runs while preserving vm + curated harness parity.
- CI checkpoint:
  - parity workflow now runs `scripts/bench_dispatch_hotpath.sh` in non-blocking mode and uploads the perf artifact for regression visibility.
- Fib recursion gate is near `python3.10` on this machine and now serves as a regression smoke, not the sole optimization target.
- Largest remaining throughput gaps are dispatch hotpath and pickle/container-heavy workloads.
- Active foundational items for closure: `OPT-022`, `OPT-023`, `OPT-024`, `OPT-025`, `OPT-026`.
- Detailed historical optimization deltas are tracked in git history; keep this section to current-state notes only.
