# Production Readiness Accounting (CPython 3.14)

This is the canonical release-readiness checklist.
Use this file for release blockers; use `docs/ROADMAP.md` for sequencing.

Status:
- `[ ]` not started
- `[~]` in progress
- `[x]` complete

## Release Gate Policy
- Functional closure is Milestone 13.
- Native extension ecosystem closure is Milestone 15.
- Performance/architecture hardening is Milestone 14.
- Final release certification is Milestone 16.
- No blocker is considered closed without tests and documented evidence.

## P0 Release Blockers

| Area | Status | Closure criteria |
|---|---|---|
| Top stdlib common-usecase baseline (`docs/STDLIB_COMMON_USECASE_CHECKLIST.md`) | `[x]` | Keep `26/26` import + smoke baseline green in `tests/stdlib_common_usecases.rs` |
| `json` parity + hardening | `[~]` | Full semantic parity for pure + accelerator paths, malformed-input differential coverage, and perf baseline |
| `_csv`/`csv` parity + hardening | `[~]` | Full parser/writer parity, malformed-input differential coverage, and perf baseline |
| `pickle`/`pickletools`/`copyreg` parity + hardening | `[~]` | Re-enable deferred strict pickle harness lane and close it without timeouts |
| `_io` stdlib-required behavioral parity | `[~]` | Close remaining strict-harness-lane `_io` dependencies and edge semantics |
| `_sre` parity needed for pure `re` closure | `[~]` | Close long-tail regex semantics needed by strict stdlib and differential tests |
| CPython `.pyc` exception-table execution parity | `[~]` | Keep landed exception-table-driven handler/unwind semantics green for translated `.pyc` and close remaining `.pyc` long-tail opcode/state parity without reintroducing source fallback for covered startup/import paths |
| Hash-container semantic/perf closure (`dict`/`set`/`frozenset`) | `[~]` | CPython parity on edge semantics and expected throughput characteristics |
| VM throughput closure for production workloads | `[~]` | Close P0/P1 performance items tracked in `docs/OPTIMIZATION_BACKLOG.md` |
| P0 engineering-gate backlog (`docs/ALGO_AUDIT_BACKLOG.md`) | `[~]` | Close all P0 audit rows with test + benchmark evidence |
| Builtin surface parity gate (`docs/BUILTIN_PARITY.md`) | `[x]` | Keep gate green with empty allowlists |

## Core Interpreter Readiness

### Language/Compiler/VM
- `[~]` Full tokenizer/grammar parity
- `[~]` Full opcode parity
- `[~]` Long-tail runtime semantic parity
- `[x]` Core parser/compiler/VM foundations

### Runtime/Object Model
- `[x]` Object identity + refcount/cycle-GC foundations
- `[x]` Core truthiness protocol behavior (`__bool__` then `__len__`)
- `[~]` Descriptor/metaclass/slots long-tail parity
- `[~]` Big-int conversion/formatting/error-edge long-tail parity

### Import/Module System
- `[x]` Curated import-system foundations
- `[~]` Full importlib/pkgutil/resources long-tail parity

## Stdlib Readiness
- `[~]` Native-core-first closure for `_io`, `_csv`, `_sre`, `_pickle`
- `[~]` Pure-stdlib default-path closure with strict tests
- `[x]` `hashlib` md5/sha2 baseline path (`_md5`, `_sha2`) with parity tests, including `_blake2` full constructor parameter-block vectors

## Test and Quality Gates
- `[x]` Curated CPython harness lanes green
- `[~]` Active strict harness lane green; deferred strict pickle harness lane open
- `[x]` Differential + fuzz foundations in place
- `[x]` Coverage gate workflow in place
- `[x]` Builtin parity gate active and green
- `[~]` Algorithmic/semantic P0 audit closure pending

## Milestone 13 Completion Rule
Milestone 13 closes only when:
1. all P0 blockers above are `[x]`
2. `docs/STUB_ACCOUNTING.md` has no open Milestone-13 P0 rows
3. strict harness lanes and allowlists are closed as defined in `docs/STUB_ACCOUNTING.md`

## Companion Docs
- `docs/README.md`
- `docs/ROADMAP.md`
- `docs/STUB_ACCOUNTING.md`
- `docs/STDLIB_COMMON_USECASE_CHECKLIST.md`
- `docs/STDLIB_MIGRATION_PLAN.md`
- `docs/ENGINEERING_GATES.md`
- `docs/ALGO_AUDIT_BACKLOG.md`
- `docs/OPTIMIZATION_PLAN.md`
- `docs/OPTIMIZATION_BACKLOG.md`
- `docs/BUILTIN_PARITY.md`
