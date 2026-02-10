# Stub and Partial Implementation Ledger

This file is the canonical ledger for incomplete runtime/stdlib behavior.
No partially implemented surface is allowed to remain untracked.

## Enforcement
- `NoOp` inventory snapshot: `docs/NOOP_BUILTIN_INVENTORY.txt`
- Inventory gate test: `tests/noop_inventory.rs`
- Refresh command:
  - `cargo run --quiet --bin print_noop_inventory > docs/NOOP_BUILTIN_INVENTORY.txt`
- Engineering gates:
  - `docs/ENGINEERING_GATES.md`
  - `docs/ALGO_AUDIT_BACKLOG.md`

## Milestone 13 P0 Blockers

| Surface | Current state | Closure criteria | Milestone |
|---|---|---|---|
| `json` | Pure-module-first path exists; native fallback still partial | Full CPython semantic parity, malformed-input differential coverage, and perf baseline | 13 |
| `_csv`/`csv` | Native substrate exists; behavior still partial in long-tail cases | Full parser/writer parity (`test_csv` class), malformed-input hardening, perf baseline | 13 |
| `pickle`/`pickletools`/`copyreg` | Partial parity; deferred strict pickle lane still open | Strict deferred lane closure + protocol/runtime parity + perf baseline | 13 |
| `_io` | Core mode/newline/validation behavior implemented; stream parity incomplete | Complete `_io` behavior required by strict stdlib and pure-stdlib execution | 13 |
| `_sre` | Core accelerator surface bootstrapped; long-tail behavior pending | Pure `Lib/re/*` default path passes strict/curated gates | 13 |
| Hash containers | Dict backend upgraded; set/frozenset mostly hash-indexed | Long-tail semantic + performance parity closure for dict/set/frozenset | 13/14 |

## Active Non-P0 Partial Surfaces

| Surface | Current state | Closure criteria | Milestone |
|---|---|---|---|
| Importlib/resources/pkgutil helpers | Foundations implemented; long-tail behavior partial | CPython compatibility for packaging/resource call paths in scope | 13 |
| `inspect`/`types` | Foundational behavior implemented | Full stdlib-required behavior parity | 13 |
| `threading`/`signal`/`_thread`/`_warnings` | Foundations implemented | Full in-scope behavioral parity | 13/16 |
| `socket`/`_socket` | Baseline methods/helpers implemented | Full in-scope API and behavior parity | 13 |
| `uuid` | Foundations implemented | Full in-scope API parity | 13 |
| `dataclasses`/`typing`/`enum`/`contextvars` | Partial stdlib compatibility | Full in-scope semantics for modern pure-Python apps | 13 |
| Object-model protocol dispatch (`__contains__`/iterator fallback, slot edge parity) | Truthiness + baseline membership fallback order landed; long-tail slot/error edge parity still partial | Align remaining membership/slot edge semantics with CPython data model/tests | 13 |
| VM/module decomposition | `src/vm/mod.rs` remains large | Move critical paths into focused modules with regression proof | 14 |

## Strict Harness Accounting
- Active strict suite: `tests/cpython_suite_strict_stdlib.txt`
- Deferred strict pickle suite: `tests/cpython_suite_deferred_pickle.txt`
- Active strict allowlist: `tests/cpython_allowlist_strict.txt` (target: empty)

Policy:
1. Active strict suite should remain green with empty allowlist.
2. Deferred pickle suite remains explicit until re-enabled and closed.

## Remaining Intentional NoOp Scope
- Test-only CPython helper modules (`_testcapi`, `_testinternalcapi` family)
- `sys.monitoring` and `sys._jit` scaffolding

These must remain explicitly listed and must not silently expand.

## Update Rules
1. Any new partial behavior must add/update a row here in the same commit.
2. Any row marked complete must have linked regression tests and (where relevant) perf evidence.
3. Milestone 13 cannot close while any P0 blocker row above is unresolved.
