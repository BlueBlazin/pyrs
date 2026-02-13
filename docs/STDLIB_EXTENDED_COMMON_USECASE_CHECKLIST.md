# Extended Stdlib Common-Usecase Checklist

This checklist tracks the expanded stdlib smoke matrix beyond the top-stdlib baseline.

Source artifact: `perf/stdlib_compat_extended_latest.json`

## Latest Probe Summary
- Total modules: `50`
- Import pass: `44/50`
- Common-usecase smoke pass: `39/50`
- Runtime: `target/debug/pyrs`
- CPython Lib: `/Users/$USER/Downloads/Python-3.14.3/Lib`
- Note: targeted closures landed after this snapshot (`queue`, `smtplib` import chain, `imaplib` common `Time2Internaldate(0)` path, email `Content-Type` fold/as_string smoke after str-subclass + regex match indexing parity fixes, `statistics.mean([1,2,3,4])` smoke after numeric ABC + binary add + `float(__float__)` parity fixes, `_sre` named-group fast path for CPython `_pydecimal` parser pattern). Refresh artifact pending.

## Checklist

| Module | Priority | Import | Common use-case smoke | Primary blocker note |
|---|---|---|---|---|
| `os` | DONE | PASS | PASS | - |
| `sys` | DONE | PASS | PASS | - |
| `pathlib` | DONE | PASS | PASS | - |
| `re` | DONE | PASS | PASS | - |
| `json` | DONE | PASS | PASS | - |
| `datetime` | DONE | PASS | PASS | - |
| `time` | DONE | PASS | PASS | - |
| `math` | DONE | PASS | PASS | - |
| `random` | DONE | PASS | PASS | - |
| `collections` | DONE | PASS | PASS | - |
| `itertools` | DONE | PASS | PASS | - |
| `functools` | DONE | PASS | PASS | - |
| `logging` | DONE | PASS | PASS | - |
| `subprocess` | DONE | PASS | PASS | - |
| `typing` | DONE | PASS | PASS | - |
| `argparse` | DONE | PASS | PASS | - |
| `unittest` | DONE | PASS | PASS | - |
| `threading` | DONE | PASS | PASS | - |
| `multiprocessing` | DONE | PASS | PASS | - |
| `asyncio` | DONE | PASS | PASS | - |
| `csv` | DONE | PASS | PASS | - |
| `sqlite3` | DONE | PASS | PASS | - |
| `urllib` | DONE | PASS | PASS | - |
| `http` | DONE | PASS | PASS | - |
| `hashlib` | DONE | PASS | PASS | - |
| `dataclasses` | DONE | PASS | PASS | - |
| `statistics` | P0 | PASS | FAIL* | targeted `statistics.mean([1,2,3,4])` smoke now green; full matrix artifact refresh pending before status flip |
| `decimal` | P0 | PASS | FAIL | `decimal` remains a local bootstrap stub; common constructor flow still fails with `RuntimeError: class constructor takes no arguments` |
| `fractions` | DONE | PASS | PASS | - |
| `pprint` | DONE | PASS | PASS | - |
| `copy` | DONE | PASS | PASS | - |
| `enum` | DONE | PASS | PASS | - |
| `abc` | DONE | PASS | PASS | - |
| `inspect` | DONE | PASS | PASS | - |
| `contextlib` | DONE | PASS | PASS | - |
| `weakref` | DONE | PASS | PASS | - |
| `queue` | DONE | PASS | PASS | - |
| `concurrent.futures` | DONE | PASS | PASS | - |
| `socket` | DONE | PASS | PASS | - |
| `ssl` | P0 | FAIL | FAIL | ModuleNotFoundError: module '_ssl' not found |
| `email` | P0 | PASS | FAIL* | `EmailMessage` header/content fold + `as_string()` smoke is now green in targeted probes; full matrix artifact refresh pending before status flip |
| `smtplib` | P0 | PASS | FAIL | import-chain baseline is green; remaining common-flow risk is missing `hashlib` algorithm coverage (`sha1`/`sha3`/`blake*`/`shake*`) |
| `imaplib` | DONE | PASS | PASS | targeted `Time2Internaldate(0)` smoke now green after `datetime.datetime.fromtimestamp` + `%z` baseline |
| `ftplib` | DONE | PASS | PASS | - |
| `xml` | P1 | PASS | FAIL | ImportError: No module named expat; use SimpleXMLTreeBuilder instead |
| `html` | DONE | PASS | PASS | - |
| `pickle` | DONE | PASS | PASS | - |
| `gzip` | P0 | FAIL | FAIL | ModuleNotFoundError: module 'zlib' not found |
| `bz2` | P0 | FAIL | FAIL | ModuleNotFoundError: module '_bz2' not found |
| `lzma` | P0 | FAIL | FAIL | ModuleNotFoundError: module '_lzma' not found |

## Open Blockers (Grouped)

- Native extension/module gaps: `ssl`, `gzip`, `bz2`, `lzma`
- Extended `hashlib` algorithm coverage gap impacting `smtplib` startup/runtime diagnostics
- Numeric core parity gaps: `decimal` (`statistics` targeted smoke green; full matrix refresh pending)
- XML parser backend gap (`pyexpat`): `xml`

## Shim and Probe Notes
- Default runtime behavior now uses CPython `Lib/enum.py`.
- Local `enum` shim is emergency fallback only and must be explicitly enabled with `PYRS_ENABLE_ENUM_SHIM=1`.
- Local shim fallback for `pkgutil` and `importlib.resources` is disabled by default and can be enabled explicitly with `PYRS_ENABLE_LOCAL_SHIMS=1`.

## Refresh Procedure
1. Regenerate `perf/stdlib_compat_extended_latest.json` with the extended stdlib probe command.
2. Update this checklist from that artifact in the same commit.
3. Any temporary workarounds must be tracked in `docs/STUB_ACCOUNTING.md`.
