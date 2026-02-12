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

## Baseline Snapshot (2026-02-12 local probe, debug build)
- Import pass: `23/26`
- Common-workflow smoke pass: `13/26`
- Major blockers are foundational (`_sre`, constructors/object model, missing native modules/surfaces), not isolated single-module bugs.

## Checklist (Common Functionality)

| Module | Priority | Status | Common functionality checklist (must all pass) | Current blocker(s) |
|---|---|---|---|---|
| `os` | P0 | GREEN | path ops, env ops, file descriptor helpers used by stdlib | Keep regression coverage broad |
| `sys` | P0 | GREEN | `sys.path`, flags, implementation metadata, argv/executable surfaces | Continue parity for flags/runtime fields |
| `pathlib` | P0 | YELLOW | `Path.resolve`, `Path.exists`, read/write helpers, path normalization | `abspath()` call path mismatch |
| `re` | P0 | YELLOW | `search`/`match` basics, alternation/grouping, replace/split primitives | `_sre` alternation mismatch |
| `json` | P0 | YELLOW | `dumps`/`loads`, common options (`sort_keys`, separators), decode errors | exception-init/object-model path mismatch |
| `datetime` | P0 | YELLOW | `datetime/date/time` constructors, arithmetic/comparison, ISO formatting | builtin constructor dispatch mismatch |
| `time` | P1 | GREEN | wall clock + monotonic + sleep path semantics | Keep platform edge parity |
| `math` | P0 | YELLOW | arithmetic/transcendentals + common integer ops (`factorial`, etc.) | missing `math.factorial` |
| `random` | P1 | YELLOW | `Random()` ctor, `randrange/randint`, seeding determinism | class constructor dispatch mismatch |
| `collections` | P0 | GREEN | `Counter`, `deque`, `defaultdict`, namedtuple baseline | expand feature-depth tests |
| `itertools` | P0 | YELLOW | `cycle/islice/count/repeat/chain` common composition paths | hang/timeout in common composition path |
| `functools` | P1 | GREEN | `lru_cache`, `partial`, comparator helpers | expand interaction tests |
| `logging` | P1 | GREEN | logger creation, levels, handler/formatter baseline | keep traceback formatting parity |
| `subprocess` | P0 | YELLOW | `run`, `CompletedProcess`, stdio capture basics | `CompletedProcess` missing |
| `typing` | P1 | GREEN | basic aliases/generics (`Optional`, `Union`, parametric forms) | modern edge semantics still pending |
| `argparse` | P1 | GREEN | parser creation, positional/optional args, errors | keep parse/error parity |
| `unittest` | P1 | GREEN | case execution, assertions, suite/runner baseline | keep exception formatting parity |
| `threading` | P1 | GREEN | thread start/join, lock/event basics | broaden contention semantics |
| `multiprocessing` | P1 | YELLOW | process start/join/queue basics (or explicit, documented limitation) | only minimal probe currently verified |
| `asyncio` | P1 | GREEN | `asyncio.run`, task scheduling baseline, coroutine correctness | expand real-world task patterns |
| `csv` | P0 | GREEN | reader/writer basics via `Lib/csv.py` + `_csv` substrate | long-tail dialect/error parity pending |
| `sqlite3` | P0 | RED | in-memory connect/execute/fetch/close | `_sqlite3` missing |
| `urllib` | P0 | YELLOW | URL parse/join/quote basics used by apps | missing string method semantics (`isalpha`) |
| `http` | P0 | RED | `http.client` import + request object baseline | `binascii.b2a_base64` missing (import chain) |
| `hashlib` | P0 | RED | `sha256/md5` digest baseline + constructor paths | unsupported hash backends/surfaces |
| `dataclasses` | P0 | YELLOW | decorator, generated `__init__`, defaults, repr/eq baseline | constructor/object-init semantics mismatch |

## Closure Criteria (Module-Level Definition of Done)
For each module above:
1. Import passes with CPython 3.14 `Lib/`.
2. Common-workflow checklist (table row) passes in automated tests.
3. At least one negative-path/error semantic check matches CPython.
4. No untracked shim/partial behavior remains for that module:
   - tracked in `docs/STUB_ACCOUNTING.md` until closed.

## Delivery Plan (Non-Hacky, Foundational First)

### Wave 1 (P0 foundation unlockers)
1. `_sre` alternation/grouping correctness (`re`, `assertRaisesRegex`, downstream test infra).
2. Constructor/object-model parity for builtin/native-backed classes (`datetime`, `random`, `dataclasses`, `json` error paths).
3. Missing core stdlib-native surfaces: `_sqlite3`, `hashlib`/`binascii` essentials, `subprocess.CompletedProcess`.
4. Missing high-value builtin methods used transitively (`str.isalpha` and related predicates).

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
- `docs/PRODUCTION_READINESS.md` tracks release blocker status.
- `docs/STUB_ACCOUNTING.md` tracks any temporary/partial behavior that remains.
