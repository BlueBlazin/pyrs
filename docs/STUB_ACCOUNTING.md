# Stub and Partial Implementation Accounting (P0)

This document is the P0 ledger for incomplete runtime/stdlib behavior.
Nothing is allowed to stay "half-implemented" without a tracked owner and closure milestone.

## Enforcement
- `NoOp` builtin symbol inventory is tracked in `/Users/$USER/pyrs/docs/NOOP_BUILTIN_INVENTORY.txt`.
- CI test gate: `/Users/$USER/pyrs/tests/noop_inventory.rs`.
- Inventory generator: `cargo run --quiet --bin print_noop_inventory > docs/NOOP_BUILTIN_INVENTORY.txt`.

## Non-NoOp Partial Implementations
These are implemented paths that are intentionally incomplete versus CPython and must be closed before release-complete parity.

| Area | Current partial scope | Exit criteria | Planned closure |
|---|---|---|---|
| `re` | Rust shim for core match/search/fullmatch/escape paths, not full engine parity | Full CPython `re` behavioral parity on harness + focused regression corpus | Milestone 13 |
| `json` | Custom parser/serializer core paths, not full encoder/decoder semantics | Full CPython `json` semantics for encoder options/edge cases and error text contracts | Milestone 13 |
| `math` | Core numeric functions plus many `NoOp` stubs | All CPython `math` public API implemented with parity tests | Milestone 13 |
| `decimal` / `_pylong` | Bootstrap-level stubs for import compatibility | Replace stubs with real semantics needed by stdlib/users | Milestone 13 |
| `os` / `posix` / `pathlib` | Core filesystem/process paths only; several APIs stubbed | Full pure-Python-usable path/process API surface for CPython test coverage in scope | Milestone 13 |
| `inspect` / `types` | Foundational predicates/types only | Full behavior required by stdlib + mainstream pure-Python packages | Milestone 13 |
| `codecs` / `unicodedata` | Core codecs and minimal unicode normalization only | Full codecs registry/error-handler and unicode behavior parity | Milestone 13 |
| `asyncio` / `threading` / `signal` | Foundational runtime paths, not full contract parity | CPython-compatible behavior for supported event loop and thread/signal APIs | Milestone 13/16 |
| `socket` / `_socket` | Module shell exists with many stubs | Real socket semantics for networked stdlib modules | Milestone 13 |
| `subprocess` / `_posixsubprocess` | Minimal bootstrap with stubbed process spawn internals | Production-safe process creation semantics and regression coverage | Milestone 13 |
| `typing` / `dataclasses` / `enum` / `contextvars` | Foundation coverage only | Full semantics required by modern frameworks and CPython suites in scope | Milestone 13 |
| Native extension path | Not implemented in runtime yet | Limited C-API/abi3 and HPy compatibility milestones complete | Milestone 15 |

## Maintenance Rule
- Any newly added `BuiltinFunction::NoOp` usage is blocked until the inventory is updated.
- Any intentionally partial non-`NoOp` behavior must be added to this document in the same PR/commit.
