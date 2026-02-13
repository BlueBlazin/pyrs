# Top Stdlib Common-Usecase Checklist (Milestone 13 Pivot)

## Purpose
This document tracks the fastest path to practical stdlib usability for the most-used Python modules, without shortcuts.

Scope list (user-priority):
- `os`, `sys`, `pathlib`, `re`, `json`, `datetime`, `time`, `math`, `random`, `collections`, `itertools`, `functools`, `logging`, `subprocess`, `typing`, `argparse`, `unittest`, `threading`, `multiprocessing`, `asyncio`, `csv`, `sqlite3`, `urllib`, `http`, `hashlib`, `dataclasses`

## Non-Hacky Execution Rules
1. CPython-source-first:
   - Native references: `Modules/*.c`, `Objects/*.c`
   - Pure stdlib references: `Lib/*.py`
2. Native-core-first for modules that require it (`_io`, `_csv`, `_sre`, `_pickle`, `_sqlite3`, hashing/binascii surfaces).
3. No ad-hoc compatibility shims unless explicitly temporary and tracked in `docs/STUB_ACCOUNTING.md`.
4. Every module closure must land with regression tests and at least one CPython differential check.

## Status Legend
- `GREEN`: import + baseline common workflow confirmed.
- `YELLOW`: imports but common workflow still fails (or only trivial path works).
- `RED`: import itself fails (critical missing native/core surface).

## Baseline Snapshot
- Import pass: `26/26`
- Common-workflow smoke pass: `26/26`
- All top-stdlib common-usecase rows in this checklist are currently green; remaining work is long-tail semantic/perf closure.

## Checklist (Common Functionality)

| Module | Priority | Status | Common functionality checklist (must all pass) | Current blocker(s) |
|---|---|---|---|---|
| `os` | P0 | GREEN | path ops, env ops, file descriptor helpers used by stdlib | Keep regression coverage broad |
| `sys` | P0 | GREEN | `sys.path`, flags, implementation metadata, argv/executable surfaces | Continue parity for flags/runtime fields |
| `pathlib` | P0 | GREEN | `Path.resolve`, `Path.exists`, read/write helpers, path normalization | Deep filesystem semantics still tracked separately |
| `re` | P0 | GREEN | `search`/`match` basics, alternation/grouping, replace/split primitives | Long-tail regex engine parity remains tracked in strict suites |
| `json` | P0 | GREEN | `dumps`/`loads`, common options (`sort_keys`, separators), decode errors | Pure-json default path and `_json` scanner integration are now green; long-tail malformed-input/perf closure remains |
| `datetime` | P0 | GREEN | `datetime/date/time` constructors, arithmetic/comparison, ISO formatting | Deep tz/parsing edges tracked in strict harness |
| `time` | P1 | GREEN | wall clock + monotonic + sleep path semantics | Keep platform edge parity |
| `math` | P0 | GREEN | arithmetic/transcendentals + common integer ops (`factorial`, etc.) | Advanced numeric edge parity tracked in harness |
| `random` | P1 | GREEN | `Random()` ctor, `randrange/randint`, seeding determinism | Wider distribution/statistical API coverage pending |
| `collections` | P0 | GREEN | `Counter`, `deque`, `defaultdict`, namedtuple baseline | expand feature-depth tests |
| `itertools` | P0 | GREEN | `cycle/islice/count/repeat/chain` common composition paths | Iterator laziness depth remains tracked separately |
| `functools` | P1 | GREEN | `lru_cache`, `partial`, comparator helpers | expand interaction tests |
| `logging` | P1 | GREEN | logger creation, levels, handler/formatter baseline | keep traceback formatting parity |
| `subprocess` | P0 | GREEN | `run`, `CompletedProcess`, stdio capture basics | `Popen` pipe attrs (`stdin/stdout/stderr`) with `readline`/`write`/`communicate` text-mode paths are now wired; deeper process-edge parity remains strict-suite tracked |
| `typing` | P1 | GREEN | basic aliases/generics (`Optional`, `Union`, parametric forms) | modern edge semantics still pending |
| `argparse` | P1 | GREEN | parser creation, positional/optional args, errors | keep parse/error parity |
| `unittest` | P1 | GREEN | case execution, assertions, suite/runner baseline | keep exception formatting parity |
| `threading` | P1 | GREEN | thread start/join, lock/event basics | broaden contention semantics |
| `multiprocessing` | P1 | GREEN | process start/join/queue baseline viability check (minimum smoke) | baseline smoke is intentionally minimal; deeper semantics remain tracked in strict/differential lanes |
| `asyncio` | P1 | GREEN | `asyncio.run`, task scheduling baseline, coroutine correctness | expand real-world task patterns |
| `csv` | P0 | GREEN | reader/writer basics via `Lib/csv.py` + `_csv` substrate | long-tail dialect/error parity pending |
| `sqlite3` | P0 | GREEN | in-memory connect/execute/fetch/close | `_sqlite3` now includes descriptor-backed connection attrs (`isolation_level`/`autocommit`/`in_transaction`/`total_changes`), SQL-length DataError precheck, row/text-factory plumbing, `_sqlite3.Row` equality + Sequence-ABC parity + hash parity (`dict(Row)` mapping conversion and ordering-TypeError behavior from `test_factory`), real `create_function()` callback registration, callable `factory=`/path-like database handling (including relayed subclass-factory kwargs and `Base Connection.__init__ not called.` parity), raw bytes/bytearray path handoff to sqlite without lossy UTF-8 replacement, native `check_same_thread` affinity enforcement for connection+cursor methods, CPython-like `Connection.cursor(factory=...)` callable/class dispatch with native `Cursor.__init__` substrate, `Connection.backup()` on native SQLite backup APIs, `set_trace_callback()` statement delivery for VM-executed SQL, and `Connection.iterdump()` dispatch through CPython `Lib/sqlite3/dump.py`; strict stdlib suite includes `test/test_sqlite3/test_dbapi.py`, `test/test_sqlite3/test_backup.py`, `test/test_sqlite3/test_factory.py`, `test/test_sqlite3/test_dump.py`, and `test/test_sqlite3/test_transactions.py` (green). Remaining DB-API long-tail frontier is broader type/runtime depth. |
| `urllib` | P0 | GREEN | URL parse/join/quote basics used by apps | Extended URL policy/IDNA edge parity pending |
| `http` | P0 | GREEN | `http.client` import + request object baseline | deeper enum/object-model long-tail semantics remain tracked (CPython `Lib/enum.py` path is default; local enum shim retired) |
| `hashlib` | P0 | GREEN | `sha256/md5` digest baseline + constructor paths | md5/sha2 native backends are active; broader algorithm surface remains tracked separately |
| `dataclasses` | P0 | GREEN | decorator, generated `__init__`, defaults, repr/eq baseline | Advanced slots/frozen/post-init edge behavior tracked separately |

## Closure Criteria (Module-Level Definition of Done)
For each module above:
1. Import passes with CPython 3.14 `Lib/`.
2. Common-workflow checklist (table row) passes in automated tests.
3. At least one negative-path/error semantic check matches CPython.
4. No untracked shim/partial behavior remains for that module:
   - tracked in `docs/STUB_ACCOUNTING.md` until closed.

## Delivery Plan (Non-Hacky, Foundational First)

### Wave 1 (P0 foundation unlockers)
1. `hashlib` native crypto substrates (`md5`, `sha2`) are landed with parity tests.
2. `_sqlite3` baseline import/connect/execute/fetch path is landed with regression tests.
3. Keep enum/http behavior closure tracked on CPython `Lib/enum.py` path with no local enum shim fallback.
4. Keep constructor/object-model and iterator fixes covered with targeted regressions (no regressions to resolved rows).

### Wave 2 (P0 module closure pass)
1. Close `pathlib`, `json`, `math`, `itertools`, `urllib` common paths.
2. Add module-specific common-usecase tests for each P0 row.
3. Re-run curated and strict stdlib harness lanes; keep allowlists empty where owned.

### Wave 3 (P1 module depth + stabilization)
1. Expand breadth for `threading`, `multiprocessing`, `asyncio`, `typing`, `logging`, `argparse`, `unittest`.
2. Add performance sanity checks for heavy-use modules (`json`, `csv`, `re`, `pathlib`).
3. Keep benchmark suite green (`bench_fib_gate`, `bench_dispatch_hotpath`, `bench_dict_backend`).

## Test/Gate Plan
- Add deterministic module smoke/regression tests under `tests/` for each row (import + common path + one negative path).
- Keep strict harness opt-in locally for fast loops; run full strict lanes during closure checkpoints.
- For each foundational fix, run:
  - targeted unit/integration tests,
  - module-specific smoke tests,
  - relevant CPython harness entries.

## Ownership and Tracking
- This checklist is the canonical tracker for the "common functionality first" pivot.
- Extended-module coverage tracker: `docs/STDLIB_EXTENDED_COMMON_USECASE_CHECKLIST.md`.
- `docs/PRODUCTION_READINESS.md` tracks release blocker status.
- `docs/STUB_ACCOUNTING.md` tracks any temporary/partial behavior that remains.
