# Production Readiness Accounting (CPython 3.14)

This is the canonical release-readiness checklist.
Use this file for "what must be true before production release".

For planning/order of execution, see `docs/ROADMAP.md`.
For partial/stub module details, see `docs/STUB_ACCOUNTING.md`.

Status:
- `[ ]` not started
- `[~]` in progress
- `[x]` complete

## Current Priority Override

Performance sprint is the active top priority before further Milestone 13 closure work.

Required benchmark suite for this sprint:
1. `scripts/bench_fib_gate.sh 5`
2. `scripts/bench_dispatch_hotpath.sh 5`
3. `scripts/bench_dict_backend.sh 5`

Latest local snapshot (2026-02-11):
- `fib(29)x5`: `pyrs ~0.54s` vs `python3.10 ~0.50s` (`~1.08x`)
- dispatch hotpath: `pyrs ~0.54-0.65s` vs `python3.10 ~0.058-0.061s` (`~9-11x`)
- dict microbench: `pyrs ~0.28s` vs `python3.10 ~0.01-0.02s`
- pickle hotspot: `pyrs ~5.1-5.2s` vs `python3.10 ~0.42-0.45s` (`~11-12x`)

Implementation strategy is tracked in `docs/OPTIMIZATION_PLAN.md` and is explicitly CPython-referenced.
Canonical optimization status is tracked in `docs/OPTIMIZATION_BACKLOG.md`.

## P0 Release Blockers

| Area | Status | Notes |
|---|---|---|
| `json` parity and hardening | `[~]` | Full semantic, malformed-input, and perf closure still required |
| `_csv`/`csv` parity and hardening | `[~]` | Full parser/writer semantic and perf closure still required |
| `pickle`/`pickletools`/`copyreg` parity and hardening | `[~]` | Still open; deferred strict pickle lane remains open |
| `_io` behavioral parity needed by stdlib | `[~]` | Core mode/newline/validation landed; full stream parity pending |
| `_sre` parity needed for pure `re` default | `[~]` | Core surface exists; long-tail behavior still pending |
| Hash-container parity and performance closure (`dict`/`set`/`frozenset`) | `[~]` | Backend upgraded; long-tail semantic/perf closure pending |
| VM throughput/perf closure vs CPython for production workloads | `[~]` | Fib recursion gate is near baseline on this machine, but major throughput gaps remain in dispatch and container/stdlib hotpaths; closure requires `OPT-022` through `OPT-026` completion (`docs/OPTIMIZATION_BACKLOG.md`) |
| P0 engineering gate backlog (`docs/ALGO_AUDIT_BACKLOG.md`) | `[~]` | Must be fully closed before Milestone 13 completion |

## Core Interpreter Readiness

### Language/Compiler/VM
- `[~]` Full tokenizer/grammar parity for Python 3.14
- `[~]` Full opcode execution parity for Python 3.14
- `[~]` Long-tail runtime semantic parity (attribute/data-model/pattern/exception edges)
- `[x]` Core parser/compiler/VM foundations through Milestone 12

### Runtime and Object Model
- `[x]` Object identity (`id`, `is`) and refcount/cycle-GC foundations
- `[x]` Core truth-value protocol semantics (`__bool__`/`__len__`) for VM control flow and key coercion sites
- `[~]` Data-model parity closure (descriptors, attribute hooks, metaclass/super edges, slots edges)
- `[~]` Numeric long-tail parity (big-int conversion/format/error-edge behavior)

### Import and Module System
- `[x]` Curated import-system foundations (meta path, hooks, namespace packages, module metadata)
- `[~]` Full importlib/pkgutil/resources parity in broader stdlib/package scenarios

## Stdlib Readiness

### Native-Core-First Requirement
Milestone 13 stdlib closure proceeds in this order:
1. Native core parity (`_io`, `_csv`, `_sre`, `_pickle`, object protocol hooks)
2. Pure CPython stdlib module execution as primary behavior
3. Strict stdlib lane expansion and closure

### Active Stdlib Readiness
- `[~]` `json`: pure-module-first direction in place; full parity/perf closure pending
- `[~]` `csv`: native `_csv` substrate in place; full parity/perf closure pending
- `[~]` `pickle`: native substrate partially in place; strict deferred lane still open
- `[~]` `re`: `_sre` substrate partially in place; pure `Lib/re/*` default closure pending

## Test and Quality Gates
- `[x]` Curated language/import CPython harness suites green with empty allowlist
- `[~]` Strict stdlib lane active and green for non-pickle scope
- `[~]` Deferred pickle strict lane tracked and still open
- `[x]` Differential and fuzz foundations in place
- `[x]` Coverage gate script/CI wiring in place
- `[~]` Full P0 closure of algorithmic/semantic audit backlog still pending

## Milestone Completion Criteria

### Milestone 13 is complete only when:
1. All P0 release blockers in this file are `[x]`.
2. `docs/STUB_ACCOUNTING.md` has no remaining Milestone-13 P0 unresolved rows.
3. Active strict stdlib lane is green with empty allowlist.
4. Deferred pickle strict lane is re-enabled and green.
5. Engineering gates in `docs/ENGINEERING_GATES.md` are satisfied for scope.

## Companion Sources
- Roadmap and milestone order: `docs/ROADMAP.md`
- Stub/partial details: `docs/STUB_ACCOUNTING.md`
- Pure-stdlib migration policy: `docs/STDLIB_MIGRATION_PLAN.md`
- Engineering gates: `docs/ENGINEERING_GATES.md`
- Audit backlog: `docs/ALGO_AUDIT_BACKLOG.md`
- Optimization backlog and status: `docs/OPTIMIZATION_BACKLOG.md`
