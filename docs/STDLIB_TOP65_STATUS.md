# Top 65 Stdlib Modules Status Tracker

## Purpose
Track the current closure status for the 65 highest-priority stdlib modules we want fully working before beta launch.

This is the canonical ordered execution list and status table.

## Source of Truth
- Baseline artifact: `perf/stdlib_full_probe_latest.json`
- Current snapshot date: `2026-02-25`
- Target runtime: `target/debug/pyrs`
- CPython Lib root: `.local/Python-3.14.3/Lib`

## Ordered Batches
1. Batch 1: `sys`, `types`, `traceback`, `warnings`, `pkgutil`, `typing`
2. Batch 2: `os`, `pathlib`, `io`, `stat`, `shutil`, `tempfile`
3. Batch 3: `re`, `string`, `fnmatch`, `glob`, `textwrap`, `difflib`
4. Batch 4: `json`, `base64`, `hmac`, `hashlib`, `secrets`, `uuid`
5. Batch 5: `datetime`, `time`, `zoneinfo`, `calendar`, `decimal`
6. Batch 6: `math`, `random`, `statistics`, `fractions`, `array`, `bisect`
7. Batch 7: `collections`, `itertools`, `functools`, `dataclasses`, `enum`, `copy`
8. Batch 8: `argparse`, `configparser`, `logging`, `pprint`, `inspect`, `contextlib`
9. Batch 9: `threading`, `queue`, `asyncio`, `subprocess`, `multiprocessing`, `concurrent`
10. Batch 10: `socket`, `ssl`, `urllib`, `http`, `email`, `xml`
11. Batch 11: `sqlite3`, `csv`, `pickle`, `gzip`, `bz2`, `lzma`

## Status Legend
- `Import`: result of module import in pyrs (`PASS`/`FAIL`/`TIMEOUT`)
- `Comprehensive`: mapped CPython stdlib test lane aggregate (`PASS`/`FAIL`/`TIMEOUT`)
- `Mapped tests`: number of mapped `test.test_*` modules
- `Latest mapped test run summary`: most recent local lane capture on record (`run`, `F`, `E`, `S`)
  - latest full-probe rows currently do not include per-test `run`/`F`/`E`/`S` counters, so this column is supplemental and may lag comprehensive status.
  - `0 run, 0F, 0E, 0S` means no completed unittest counts were captured for that row; this is not a pass.

## Current Top-65 Snapshot
- Modules tracked: `65/65`
- Import pass: `65/65`
- Comprehensive pass: `13`
- Comprehensive fail: `40`
- Comprehensive timeout: `12`

| Order | Batch | Module | Import | Comprehensive | Mapped tests | Latest mapped test run summary |
|---:|---|---|---|---|---:|---|
| 1 | Batch 1 | `sys` | PASS | FAIL | 3 | 477 run, 0F, 409E, 68S |
| 2 | Batch 1 | `types` | PASS | PASS | 1 | 127 run, 0F, 0E, 2S (local lane 2026-02-25) |
| 3 | Batch 1 | `traceback` | PASS | PASS | 1 | 368 run, 0F, 0E, 208S (local lane 2026-02-26) |
| 4 | Batch 1 | `warnings` | PASS | PASS | 1 | 183 run, 0F, 0E, 10S (local lane 2026-02-26) |
| 5 | Batch 1 | `pkgutil` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 6 | Batch 1 | `typing` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 7 | Batch 2 | `os` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 8 | Batch 2 | `pathlib` | PASS | PASS | 1 | 0 run, 0F, 0E, 0S |
| 9 | Batch 2 | `io` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 10 | Batch 2 | `stat` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 11 | Batch 2 | `shutil` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 12 | Batch 2 | `tempfile` | PASS | FAIL | 1 | 118 run, 2F, 35E, 12S |
| 13 | Batch 3 | `re` | PASS | TIMEOUT | 1 | 0 run, 0F, 0E, 0S |
| 14 | Batch 3 | `string` | PASS | FAIL | 2 | 20 run, 6F, 30E, 0S |
| 15 | Batch 3 | `fnmatch` | PASS | FAIL | 1 | 24 run, 18F, 3E, 0S |
| 16 | Batch 3 | `glob` | PASS | FAIL | 1 | 24 run, 0F, 24E, 0S |
| 17 | Batch 3 | `textwrap` | PASS | FAIL | 1 | 68 run, 40F, 1E, 0S |
| 18 | Batch 3 | `difflib` | PASS | TIMEOUT | 1 | 0 run, 0F, 0E, 0S |
| 19 | Batch 4 | `json` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 20 | Batch 4 | `base64` | PASS | FAIL | 1 | 41 run, 7F, 47E, 1S |
| 21 | Batch 4 | `hmac` | PASS | PASS | 1 | 145 run, 0F, 0E, 3S |
| 22 | Batch 4 | `hashlib` | PASS | FAIL | 1 | 82 run, 1F, 0E, 15S |
| 23 | Batch 4 | `secrets` | PASS | PASS | 1 | 11 run, 0F, 0E, 0S |
| 24 | Batch 4 | `uuid` | PASS | TIMEOUT | 1 | 0 run, 0F, 0E, 0S |
| 25 | Batch 5 | `datetime` | PASS | TIMEOUT | 1 | 0 run, 0F, 0E, 0S |
| 26 | Batch 5 | `time` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 27 | Batch 5 | `zoneinfo` | PASS | PASS | 1 | 0 run, 0F, 0E, 0S |
| 28 | Batch 5 | `calendar` | PASS | FAIL | 1 | 78 run, 31F, 8E, 0S |
| 29 | Batch 5 | `decimal` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 30 | Batch 6 | `math` | PASS | TIMEOUT | 2 | 0 run, 0F, 0E, 0S |
| 31 | Batch 6 | `random` | PASS | TIMEOUT | 1 | 0 run, 0F, 0E, 0S |
| 32 | Batch 6 | `statistics` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 33 | Batch 6 | `fractions` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 34 | Batch 6 | `array` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 35 | Batch 6 | `bisect` | PASS | FAIL | 1 | 46 run, 6F, 9E, 0S |
| 36 | Batch 7 | `collections` | PASS | FAIL | 1 | 101 run, 39F, 45E, 4S |
| 37 | Batch 7 | `itertools` | PASS | TIMEOUT | 1 | 0 run, 0F, 0E, 0S |
| 38 | Batch 7 | `functools` | PASS | FAIL | 1 | 321 run, 35F, 57E, 119S |
| 39 | Batch 7 | `dataclasses` | PASS | FAIL | 1 | 274 run, 120F, 78E, 2S |
| 40 | Batch 7 | `enum` | PASS | TIMEOUT | 1 | 0 run, 0F, 0E, 0S |
| 41 | Batch 7 | `copy` | PASS | FAIL | 1 | 81 run, 5F, 10E, 0S |
| 42 | Batch 8 | `argparse` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 43 | Batch 8 | `configparser` | PASS | FAIL | 1 | 341 run, 36F, 244E, 5S |
| 44 | Batch 8 | `logging` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 45 | Batch 8 | `pprint` | PASS | FAIL | 1 | 45 run, 18F, 8E, 1S |
| 46 | Batch 8 | `inspect` | PASS | PASS | 1 | 0 run, 0F, 0E, 0S |
| 47 | Batch 8 | `contextlib` | PASS | FAIL | 2 | 91 run, 10F, 14E, 0S |
| 48 | Batch 9 | `threading` | PASS | TIMEOUT | 2 | 22 run, 0F, 2E, 2S |
| 49 | Batch 9 | `queue` | PASS | TIMEOUT | 1 | 0 run, 0F, 0E, 0S |
| 50 | Batch 9 | `asyncio` | PASS | PASS | 1 | 0 run, 0F, 0E, 0S |
| 51 | Batch 9 | `subprocess` | PASS | FAIL | 1 | 353 run, 0F, 219E, 134S |
| 52 | Batch 9 | `multiprocessing` | PASS | FAIL | 4 | 39 run, 0F, 40E, 0S |
| 53 | Batch 9 | `concurrent` | PASS | PASS | 1 | 0 run, 0F, 0E, 0S |
| 54 | Batch 10 | `socket` | PASS | FAIL | 1 | 745 run, 10F, 344E, 522S |
| 55 | Batch 10 | `ssl` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |
| 56 | Batch 10 | `urllib` | PASS | FAIL | 2 | 105 run, 5F, 61E, 2S |
| 57 | Batch 10 | `http` | PASS | FAIL | 2 | 109 run, 61F, 77E, 1S |
| 58 | Batch 10 | `email` | PASS | PASS | 1 | 0 run, 0F, 0E, 0S |
| 59 | Batch 10 | `xml` | PASS | FAIL | 4 | 15 run, 1F, 2E, 0S |
| 60 | Batch 11 | `sqlite3` | PASS | PASS | 1 | 0 run, 0F, 0E, 0S |
| 61 | Batch 11 | `csv` | PASS | PASS | 1 | 127 run, 0F, 0E, 6S |
| 62 | Batch 11 | `pickle` | PASS | TIMEOUT | 1 | 0 run, 0F, 0E, 0S |
| 63 | Batch 11 | `gzip` | PASS | FAIL | 1 | 75 run, 8F, 59E, 0S |
| 64 | Batch 11 | `bz2` | PASS | TIMEOUT | 1 | 0 run, 0F, 0E, 0S |
| 65 | Batch 11 | `lzma` | PASS | FAIL | 1 | 0 run, 0F, 0E, 0S |

## Refresh Procedure
1. Re-run full probe and regenerate `perf/stdlib_full_probe_latest.json`.
2. Update this tracker in the same commit.
3. Continue fixes strictly in batch order above.

## In-Progress Checkpoints
### 2026-02-25: Batch 1 `types` lane
- Full lane status is now green: `test.test_types` -> `127` run, `0` failures, `0` errors, `2` skipped.
- Final closure delta in this round:
  - descriptor parity for unbound native method `__get__` wrappers (`_io._TextIOBase.read`, `_queue.SimpleQueue.put`, `str.capitalize`),
  - function annotation normalization now resolves forward references through captured defining-scope locals (`__annotate__` path),
  - exception-instance `isinstance` classification now correctly recognizes `SkipTest` in class setup paths,
  - numeric formatting substrate closure for `int/float.__format__` and `%` formatting (`c`, `n`, `e/E/f/F/g/G/%`, grouping/alignment/alternate/sign semantics used by `types` tests).

### 2026-02-25: Batch 1 `traceback` lane (active)
- Lane remains non-green (`Comprehensive=FAIL`), but core CPython parity blockers in this pass were closed:
  - `re.Pattern.split` capture-group output parity fixed (both text and bytes paths), unblocking traceback boundary splitting for cause/context chains.
  - Exception base attrs now default on creation (`__traceback__`, `__cause__`, `__context__`, `__suppress_context__`) for user-defined exception classes.
  - Truthiness parity fixed for builtin-backed instance subclasses (for example empty `traceback.StackSummary()` no longer evaluates truthy).
  - `ExceptionGroup` display text parity aligned (`"{message} ({n} sub-exception[s])"`), and exception matching now treats `ExceptionGroup` as inheriting `Exception` in current runtime model.
  - Function metadata setattr parity closed for `__defaults__` / `__kwdefaults__`.
- Newly added regressions:
  - `tests/vm.rs::re_split_includes_capturing_groups`
  - `tests/vm.rs::user_exception_instances_expose_default_traceback_chain_attrs`
  - `tests/vm.rs::traceback_format_exception_without_tb_omits_traceback_header`
- Immediate next target: continue failfast closure for `test.test_traceback` until lane is fully green, then proceed to `warnings`.

### 2026-02-26: Batch 1 `traceback` lane (active, refreshed)
- Additional closures landed in this lane:
  - `_testcapi.exception_print` parity surface implemented and wired through native dispatch, unblocking traceback colorized-default assertions.
  - Import-spec loader coherence fixed: module `__spec__.loader` is now refreshed to concrete importlib machinery loader objects where available, restoring `linecache.lazycache` source lookup paths.
  - `_io.open_code` baseline added and exported on both `io` and `_io`, closing `traceback.FrameSummary` file-source open path failures.
- Confirmed targeted traceback tests now green:
  - `test.test_traceback.TestColorizedTraceback.test_colorized_traceback_is_the_default`
  - `test.test_traceback.TestFrame.test_basics`

### 2026-02-26: Batch 1 `traceback` lane (closed)
- Full lane status is now green: `test.test_traceback` -> `368` run, `0` failures, `0` errors, `208` skipped.
- Final closure delta in this round:
  - signature default rendering for `inspect.signature(traceback.print_exception)` now uses runtime `repr()` semantics, restoring `<implicit>` defaults for traceback sentinels.
- Hand-off target after this checkpoint was Batch 1 `warnings`.

### 2026-02-26: Batch 1 `warnings` lane (closed)
- Full lane status is now green: `test.test_warnings` -> `183` run, `0` failures, `0` errors, `10` skipped.
- Final closure deltas in this round:
  - CLI script execution now sets CPython-style absolute `__main__.__file__` for file mode and uses it for source/traceback filename metadata.
  - startup `-X tracemalloc[=N]` support added and wired into VM startup state.
  - `_tracemalloc` native substrate implemented for lane-critical paths:
    - start/stop/is_tracing/get_traceback_limit/get_traced_memory/get_tracemalloc_memory/reset_peak/clear_traces/_get_traces/_get_object_traceback.
    - file-object allocation trace capture hooked into `io` object construction for ResourceWarning traceback parity.
  - `_warnings` dispatch now resolves/uses the active warnings module companion implementation robustly across fresh-module swaps.
  - warnings resilience paths now tolerate deleted module attrs with CPython-compatible behavior:
    - `defaultaction`,
    - `filters`,
    - `onceregistry`,
    - `_showwarnmsg`.
  - sys.modules write-path sync now rebinds warnings module context (`warnings._set_module(warnings)`) when `sys.modules['warnings']` is reassigned.
- Immediate next target: proceed to Batch 1 `pkgutil` lane failfast closure.
