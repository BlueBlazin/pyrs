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
| `_sqlite3`/`sqlite3` | Not implemented (`sqlite3` import fails) | `_sqlite3` baseline module with connect/execute/fetch/close parity for common DB-API paths | 13 |
| `pickle`/`pickletools`/`copyreg` | `_pickle.Pickler` `__newobj_ex__` C-path parity is landed (including proto>=4) and temporary remap shim is removed; deferred strict lane currently times out under subprocess harness (`test_pickle.py`/`test_pickletools.py`) | Eliminate deferred strict pickle subprocess timeouts, keep allowlist empty, and close deferred strict lane | 13 |
| `_io` | Core mode/newline/validation behavior implemented; strict memoryio lane is green; remaining long-tail behavior exists in full strict stdlib execution | Complete remaining `_io` long-tail behavior required by full strict stdlib execution | 13 |
| `json` | Common `dumps`/`loads` workflows are green in top-stdlib gate; pure-stdlib JSON path is still partial | Full CPython semantic parity (pure + native accelerator paths), malformed-input differential coverage, perf baseline | 13 |
| `_csv`/`csv` | Top-level common workflows are green; long-tail dialect/error parity still partial | Full parser/writer parity (`test_csv` class), malformed-input hardening, perf baseline | 13 |
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
| `dataclasses`/`typing`/`enum`/`contextvars` | Common dataclasses/typing paths are green; enum remains shim-backed and partial | Full in-scope semantics for modern pure-Python apps; retire shim enum for pure `Lib/enum.py` path | 13 |
| `hashlib` extended algorithms (`_sha1`/`_blake2`/`_sha3`/`_hashlib`) | md5/sha2 minimum path is now green; broader algorithm coverage is still partial | Full CPython algorithm surface (or explicit in-scope exclusions) with differential tests and stdlib consumers passing | 13/14 |
| Object-model protocol dispatch (`__contains__`/iterator fallback, slot edge parity) | Truthiness + baseline membership fallback order landed; long-tail slot/error edge parity still partial | Align remaining membership/slot edge semantics with CPython data model/tests | 13 |
| VM/module decomposition | `src/vm/mod.rs` remains large | Move critical paths into focused modules with regression proof | 14 |

## Strict Harness Accounting
- Active strict suite: `tests/cpython_suite_strict_stdlib.txt`
- Deferred strict pickle suite: `tests/cpython_suite_deferred_pickle.txt`
- Active strict allowlist: `tests/cpython_allowlist_strict.txt` (target: empty)
- Deferred strict pickle allowlist: `tests/cpython_allowlist_deferred_pickle.txt` (target: empty)

Policy:
1. Active strict suite should remain green with empty allowlist.
2. Deferred pickle suite remains explicit until re-enabled and closed.
3. Deferred pickle suite is opt-in locally (`PYRS_RUN_DEFERRED_PICKLE=1`) to keep fast loops bounded.

## Remaining Intentional NoOp Scope
- Test-only CPython helper modules (`_testcapi`, `_testinternalcapi` family)
- `sys.monitoring` and `sys._jit` scaffolding

These must remain explicitly listed and must not silently expand.

## Update Rules
1. Any new partial behavior must add/update a row here in the same commit.
2. Any row marked complete must have linked regression tests and (where relevant) perf evidence.
3. Milestone 13 cannot close while any P0 blocker row above is unresolved.
