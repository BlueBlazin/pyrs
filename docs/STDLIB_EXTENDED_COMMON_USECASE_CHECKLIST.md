# Extended Stdlib Common-Usecase Checklist

This checklist tracks the expanded stdlib smoke matrix beyond the top-stdlib baseline.

Source artifact: `perf/stdlib_compat_extended_latest.json`

## Latest Probe Summary
- Total modules: `50`
- Import pass: `44/50`
- Common-usecase smoke pass: `36/50`
- Runtime: `target/debug/pyrs`
- CPython Lib: `/Users/$USER/Downloads/Python-3.14.3/Lib`

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
| `statistics` | P0 | PASS | FAIL | TypeError: can't convert type 'int' to numerator/denominator |
| `decimal` | P0 | PASS | FAIL | RuntimeError: class constructor takes no arguments |
| `fractions` | P0 | PASS | FAIL | AttributeError: module 'math' has no attribute 'gcd' |
| `pprint` | DONE | PASS | PASS | - |
| `copy` | DONE | PASS | PASS | - |
| `enum` | P0 | PASS | FAIL | AttributeError: int has no attribute 'value' |
| `abc` | DONE | PASS | PASS | - |
| `inspect` | DONE | PASS | PASS | - |
| `contextlib` | DONE | PASS | PASS | - |
| `weakref` | DONE | PASS | PASS | - |
| `queue` | P0 | PASS | FAIL | AttributeError: 'Condition' object has no attribute '__enter__' |
| `concurrent.futures` | P0 | PASS | FAIL | AttributeError: 'Condition' object has no attribute '__enter__' |
| `socket` | DONE | PASS | PASS | - |
| `ssl` | P0 | FAIL | FAIL | ModuleNotFoundError: module '_ssl' not found |
| `email` | P0 | PASS | FAIL | AttributeError: module '__re_pattern__' has no attribute 'split' |
| `smtplib` | P0 | FAIL | FAIL | ModuleNotFoundError: module '_operator' not found |
| `imaplib` | P0 | FAIL | FAIL | AttributeError: 'date' object has no attribute 'strftime' |
| `ftplib` | DONE | PASS | PASS | - |
| `xml` | P1 | PASS | FAIL | ImportError: No module named expat; use SimpleXMLTreeBuilder instead |
| `html` | DONE | PASS | PASS | - |
| `pickle` | DONE | PASS | PASS | - |
| `gzip` | P0 | FAIL | FAIL | ModuleNotFoundError: module 'zlib' not found |
| `bz2` | P0 | FAIL | FAIL | ModuleNotFoundError: module '_bz2' not found |
| `lzma` | P0 | FAIL | FAIL | ModuleNotFoundError: module '_lzma' not found |

## Open Blockers (Grouped)

- Native extension/module gaps: `ssl`, `smtplib`, `gzip`, `bz2`, `lzma`
- Concurrency/context-manager protocol gaps: `queue`, `concurrent.futures`
- Numeric core parity gaps: `statistics`, `decimal`, `fractions`
- Regex accelerator parity gaps: `email`
- Object-model/enum/date method parity gaps: `enum`, `imaplib`
- XML parser backend gap (`pyexpat`): `xml`

## Refresh Procedure
1. Regenerate `perf/stdlib_compat_extended_latest.json` with the extended stdlib probe command.
2. Update this checklist from that artifact in the same commit.
3. Any temporary workarounds must be tracked in `docs/STUB_ACCOUNTING.md`.
