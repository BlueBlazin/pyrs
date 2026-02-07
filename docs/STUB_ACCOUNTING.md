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
| `math` | Core numeric and transcendental helpers implemented (`sqrt`/`copysign`/`floor`/`ceil`/`isfinite`/`isinf`/`isnan`/`ldexp`/`hypot`/`fabs`/`exp`/`erfc`/`log`/`fsum`/`sumprod`/trig + `isclose`); full CPython edge/text parity and any remaining APIs still pending | All CPython `math` public API implemented with parity tests | Milestone 13 |
| `itertools` | Previously stubbed helpers now execute non-`NoOp` paths (`accumulate`, `combinations*`, `compress`, `dropwhile`, `filterfalse`, `groupby`, `islice`, `pairwise`, `starmap`, `takewhile`, `tee`, `zip_longest`), but iterator/laziness and edge semantics are still partial | Full CPython iterator protocol/laziness and behavior parity with CPython tests | Milestone 13 |
| `operator` / `functools` | Core callable adapters now execute non-`NoOp` paths (`operator.itemgetter`/`attrgetter`/`methodcaller`, `functools.cmp_to_key` with `sorted`/`min`/`max` key ordering support); long-tail API/edge semantics still pending | Full module API and behavior parity with CPython tests | Milestone 13 |
| `importlib` / `importlib.util` / `_frozen_importlib*` | Core helper surface now includes non-`NoOp` `invalidate_caches`, baseline `spec_from_file_location`, `_frozen_importlib.spec_from_loader`/`_verbose_message`, and `_frozen_importlib_external` `_path_*` + `_unpack_uint*`, but full CPython spec/loader object behavior and edge semantics are still partial | Full importlib helper parity required by stdlib and packaging workflows | Milestone 13 |
| `platform` / `binascii` / `atexit` / `collections` | Previously stubbed helper paths now execute non-`NoOp` logic (`platform.win32_is_iot`, `binascii.crc32`, `atexit.register`/`unregister`/`_run_exitfuncs`/`_clear`, `collections._count_elements`), but full module behavioral parity remains partial | Full API/semantic parity across these modules for CPython suites and common package workflows | Milestone 13 |
| `decimal` / `_pylong` | Bootstrap-level stubs for import compatibility | Replace stubs with real semantics needed by stdlib/users | Milestone 13 |
| `os` / `posix` / `pathlib` | Core filesystem/process paths plus non-`NoOp` `open`/`close`/`isatty`/`stat`/`lstat`/`rmdir`/`utime`/`scandir` and wait-status helpers; broader API surface still incomplete | Full pure-Python-usable path/process API surface for CPython test coverage in scope | Milestone 13 |
| `inspect` / `types` | Foundational predicates/types plus a baseline non-`NoOp` `inspect.signature` path (Signature instance with parameter-kind/default metadata); full Signature/Parameter API and broader module parity pending | Full behavior required by stdlib + mainstream pure-Python packages | Milestone 13 |
| `codecs` / `unicodedata` | Core codecs and minimal unicode normalization only | Full codecs registry/error-handler and unicode behavior parity | Milestone 13 |
| `asyncio` / `threading` / `signal` | Foundational runtime paths, not full contract parity | CPython-compatible behavior for supported event loop and thread/signal APIs | Milestone 13/16 |
| `socket` / `_socket` | Module shell exists with many stubs | Real socket semantics for networked stdlib modules | Milestone 13 |
| `subprocess` / `_posixsubprocess` | Minimal bootstrap with stubbed process spawn internals | Production-safe process creation semantics and regression coverage | Milestone 13 |
| `typing` / `dataclasses` / `enum` / `contextvars` | Foundation coverage only | Full semantics required by modern frameworks and CPython suites in scope | Milestone 13 |
| Native extension path | Not implemented in runtime yet | Limited C-API/abi3 and HPy compatibility milestones complete | Milestone 15 |

## Maintenance Rule
- Any newly added `BuiltinFunction::NoOp` usage is blocked until the inventory is updated.
- Any intentionally partial non-`NoOp` behavior must be added to this document in the same PR/commit.
