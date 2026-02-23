# Extended Stdlib Common-Usecase Checklist

This checklist tracks the expanded stdlib smoke matrix beyond the top-stdlib baseline.

Source artifact: `perf/stdlib_compat_extended_latest.json`

## Latest Probe Summary
- Total modules: `50`
- Import pass: `50/50`
- Common-usecase smoke pass: `50/50`
- Runtime: `target/debug/pyrs`
- CPython Lib: `.local/Python-3.14.3/Lib`
- Probe command: `python3 scripts/probe_stdlib_extended.py --pyrs target/debug/pyrs --cpython-lib .local/Python-3.14.3/Lib --out perf/stdlib_compat_extended_latest.json --timeout 20`
- Current failing modules: none.

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
| `statistics` | DONE | PASS | PASS* | targeted `statistics.mean([1,2,3,4])` smoke is green (artifact refresh pending) |
| `decimal` | DONE | PASS | PASS* | targeted pure-stdlib smoke is green after import binding parity + pure `decimal` preference (artifact refresh pending) |
| `fractions` | DONE | PASS | PASS | - |
| `pprint` | DONE | PASS | PASS | - |
| `copy` | DONE | PASS | PASS | - |
| `enum` | DONE | PASS | PASS | - |
| `abc` | DONE | PASS | PASS | - |
| `inspect` | DONE | PASS | PASS | - |
| `contextlib` | DONE | PASS | PASS | - |
| `weakref` | DONE | PASS | PASS | - |
| `queue` | DONE | PASS | PASS | - |
| `concurrent.futures` | DONE | PASS | PASS | fixed iterator protocol parity (`itertools.count.__iter__`/`__next__`) and semaphore bound semantics in threadpool shutdown path |
| `socket` | DONE | PASS | PASS | - |
| `ssl` | DONE | PASS | PASS | `_ssl` baseline landed and a native `ssl` bootstrap module now provides import/common context surfaces while namedtuple+super enum bootstrap gap remains tracked in object-model backlog |
| `email` | DONE | PASS | PASS* | `EmailMessage` header/content fold + `as_string()` smoke is green (artifact refresh pending) |
| `smtplib` | DONE | PASS* | PASS* | targeted import + common constructor smoke is green; runtime still logs missing `hashlib` algorithms (`sha1`/`sha3`/`blake*`/`shake*`) |
| `imaplib` | DONE | PASS | PASS | targeted `Time2Internaldate(0)` smoke now green after `datetime.datetime.fromtimestamp` + `%z` baseline |
| `ftplib` | DONE | PASS | PASS | - |
| `xml` | DONE | PASS | PASS | native runtime `pyexpat` baseline is active; shim removed |
| `html` | DONE | PASS | PASS | - |
| `pickle` | DONE | PASS | PASS | - |
| `gzip` | DONE | PASS | PASS | `bytes.lstrip`/`bytes.strip` parity landed, unblocking `gzip.decompress` smoke |
| `bz2` | DONE | PASS | PASS | native `_bz2` baseline landed (`BZ2Compressor`/`BZ2Decompressor` one-shot workflows) |
| `lzma` | DONE | PASS | PASS | native `_lzma` baseline landed (`LZMACompressor`/`LZMADecompressor` + constants/is_check_supported) |

## Open Blockers (Grouped)

- No module-level blockers remain for this 50-module extended smoke matrix (`50/50` import + `50/50` smoke).
- Long-tail work remains tracked in `docs/STUB_ACCOUNTING.md` (notably deferred pickle throughput and broader hashlib algorithm surface).

## Shim and Probe Notes
- Default runtime behavior now uses CPython `Lib/enum.py`.
- Local `enum` shim has been retired (`shims/enum.py` removed).
- Local shim fallback is now `_ctypes`-only and is enabled by default; set `PYRS_DISABLE_LOCAL_SHIMS=1` to force-disable fallback.

## Refresh Procedure
1. Regenerate `perf/stdlib_compat_extended_latest.json` with the extended stdlib probe command.
2. Update this checklist from that artifact in the same commit.
3. Any temporary workarounds must be tracked in `docs/STUB_ACCOUNTING.md`.
