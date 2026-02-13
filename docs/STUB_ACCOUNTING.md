# Stub and Partial Implementation Ledger

This is the canonical ledger for incomplete runtime/stdlib behavior.
No partial surface may remain untracked.

Status values:
- `OPEN`
- `IN_PROGRESS`
- `CLOSED`

## Enforcement
- No-op inventory artifact: `docs/NOOP_BUILTIN_INVENTORY.txt`
- No-op inventory gate: `tests/noop_inventory.rs`
- Refresh command:
  - `cargo run --quiet --bin print_noop_inventory > docs/NOOP_BUILTIN_INVENTORY.txt`
- Process/quality gates:
  - `docs/ENGINEERING_GATES.md`
  - `docs/ALGO_AUDIT_BACKLOG.md`

## Milestone 13 P0 Blockers

| Surface | Gap summary | Closure criteria | Required evidence | Status | Milestone |
|---|---|---|---|---|---|
| `pickle`/`pickletools`/`copyreg` | Deferred strict pickle harness lane still times out. | Deferred strict pickle harness lane is re-enabled, green, and allowlist is empty. | `PYRS_RUN_DEFERRED_PICKLE=1 cargo test -q --test cpython_harness runs_cpython_deferred_pickle_suite` | IN_PROGRESS | 13 |
| `_io` | Strict stdlib still depends on remaining `_io` edge semantics. | Remaining `_io`-dependent strict-harness-lane failures are closed. | strict harness pass + targeted `tests/vm.rs` regressions for each closed edge | IN_PROGRESS | 13 |
| `json` | Common workflows are green; long-tail semantics/hardening/perf closure is open. | `json` pure + accelerator parity closed, malformed-input differential coverage closed, perf baseline recorded. | `test_json` parity + differential probes + benchmark artifact | IN_PROGRESS | 13 |
| `_csv`/`csv` | Common workflows are green; long-tail dialect/error parity is open. | `test_csv` parity closed with malformed-input hardening and perf baseline. | `test_csv` parity + differential probes + benchmark artifact | IN_PROGRESS | 13 |
| `_sre` | Core surface works; long-tail regex behavior blocks full pure-`re` closure. | Pure `Lib/re/*` path passes curated/strict harness lanes in scope. | strict/curated harness green for `re`-dependent suites | IN_PROGRESS | 13 |
| Hash containers (`dict`/`set`/`frozenset`) | Architecture upgrade landed; long-tail semantic/perf closure remains. | CPython parity on edge behavior and performance closure criteria from readiness/audit docs. | targeted parity tests + benchmark/profile artifacts | IN_PROGRESS | 13/14 |
| Builtin symbol surface (`builtins`) | Parity gate currently green. | Keep `145/145`, zero probe mismatches, and empty allowlists. | `./scripts/run_builtin_parity_gate.sh` | CLOSED | 13 |

## Active Non-P0 Partial Surfaces

| Surface | Gap summary | Closure criteria | Status | Milestone |
|---|---|---|---|---|
| Importlib/resources/pkgutil helpers | Long-tail packaging/resource behavior is partial. | In-scope CPython compatibility for packaging/resource paths. | IN_PROGRESS | 13 |
| `inspect`/`types` | Foundational behavior exists; stdlib-required edges remain. | Full stdlib-required behavior parity in scope. | IN_PROGRESS | 13 |
| `threading`/`signal`/`_thread`/`_warnings` | Foundations exist; behavior depth is incomplete. | Full in-scope behavioral parity. | IN_PROGRESS | 13/16 |
| `socket`/`_socket` | Baseline exists; long-tail API/behavior remains. | Full in-scope API and behavior parity. | IN_PROGRESS | 13 |
| `uuid` | Foundation exists; long-tail parity remains. | Full in-scope API parity. | IN_PROGRESS | 13 |
| `_sqlite3`/`sqlite3` | Baseline is broad; DB-API long-tail remains. | Close remaining DB-API long-tail (including URI undecodable-path edge and autocommit/type edges). | IN_PROGRESS | 13 |
| `dataclasses`/`typing`/`enum`/`contextvars` | Common paths are green; enum remains shim-backed. | Full in-scope semantics; retire enum shim for pure `Lib/enum.py` path. | IN_PROGRESS | 13 |
| `hashlib` extended algorithms (`_sha1`/`_blake2`/`_sha3`/`_hashlib`) | md5/sha2 baseline closed; broader algorithm surface open. | Full in-scope algorithm surface (or explicit exclusions) with tests and consumers green. | IN_PROGRESS | 13/14 |
| Object-model protocol dispatch | Truthiness/membership baseline landed; long-tail slot/error semantics remain. | Align remaining protocol edge semantics with CPython data model/tests. | IN_PROGRESS | 13 |
| VM/module decomposition | VM still has large modules. | Continue concern-based extraction with behavior-preserving tests. | IN_PROGRESS | 14 |

## Strict Harness Lane Accounting
- Active strict suite: `tests/cpython_suite_strict_stdlib.txt`
- Deferred strict pickle suite: `tests/cpython_suite_deferred_pickle.txt`
- Active strict allowlist: `tests/cpython_allowlist_strict.txt` (target: empty)
- Deferred strict pickle allowlist: `tests/cpython_allowlist_deferred_pickle.txt` (target: empty)

Policy:
1. Active strict harness lane stays green with empty allowlist.
2. Deferred strict pickle harness lane stays explicit until re-enabled and closed.
3. Deferred strict pickle harness lane remains opt-in locally (`PYRS_RUN_DEFERRED_PICKLE=1`) for bounded fast loops.

## Remaining Intentional NoOp Scope
- Test-only CPython helper modules (`_testcapi`, `_testinternalcapi` family)
- `sys.monitoring` and `sys._jit` scaffolding

These entries must not silently expand.

## Update Rules
1. New partial behavior must add/update a row in this file in the same commit.
2. Rows may be marked `CLOSED` only with linked regression tests and required performance evidence (where applicable).
3. Milestone 13 cannot close while any P0 blocker row here is not `CLOSED`.
