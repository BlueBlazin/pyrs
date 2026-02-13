# Project Context: Python Interpreter in Rust (`pyrs`)

## Vision
Build a production-grade Python interpreter in Rust with full source + bytecode compatibility for CPython 3.14, minimal third-party dependencies, and an architecture that can support future JIT/extension work.

## Non-Negotiable Engineering Rule
- Do not make "fast changes" or "quick fixes" as a substitute for proper design.
- Favor careful, fundamental fixes over tactical patches, even if they take longer.
- If a temporary workaround is unavoidable, it must be:
  1. explicitly marked temporary in code/docs,
  2. tracked with owner + closure criteria in `docs/STUB_ACCOUNTING.md` or `docs/ALGO_AUDIT_BACKLOG.md`,
  3. scheduled for near-term removal.

## Scope and Constraints
- Target version: CPython 3.14
- Current goals:
  - Run Python source code
  - Execute CPython 3.14 bytecode (`.pyc`)
- Current non-goals:
  - JIT implementation
  - Full CPython C-API / C-extension compatibility
- Architecture constraints:
  - Packrat parser aligned to CPython grammar
  - AST -> bytecode IR pipeline
  - CPython-like runtime object model, refcount + cycle GC, GIL
  - Keep dependencies minimal and justified

## Milestone Status (Canonical Summary)
- Milestones 0-12: complete
- Milestone 13: in progress (active)
- Milestones 14-16: pending (performance/observability, extension ecosystem, release hardening)

Milestone 13 completion is blocked on P0 closure of:
- deferred strict pickle lane timeout closure (`test/test_pickle.py`, `test/test_pickletools.py`)

## Execution Policy
- Follow CPython source-of-truth for behavior:
  - `Modules/*.c`
  - `Objects/*.c`
  - `Lib/*.py`
- Sequence Milestone 13 work as native-core-first:
  1. Native/runtime core surfaces (`_io`, `_csv`, `_sre`, `_pickle`, object protocol)
  2. Then strict pure-stdlib suite expansion and closure
- Performance checkpoint rule:
  - Optimization phase-1 checkpoint is complete; Milestone 13 functional closure is active again.
  - Keep the benchmark suite (`scripts/bench_fib_gate.sh`, `scripts/bench_dispatch_hotpath.sh`, `scripts/bench_dict_backend.sh`) as a regression gate for runtime changes.
- Prefer official CPython pure-Python stdlib implementations where feasible.
- Keep native VM handlers as accelerator/runtime layers, not full high-level reimplementations.
- Commit frequently in small focused checkpoints.
- Do not leave long-lived dirty worktrees.
- After behavior changes, update docs in the same checkpoint.
- End every assistant turn with immediate next `3-6` concrete steps.

## Test Loop Policy
- Fast local loops should run targeted/unit/integration tests first.
- Strict stdlib harness is opt-in for frequent local loops and reserved for deliberate parity passes:
  - `PYRS_RUN_STRICT_STDLIB=1`
  - `PYRS_PARITY_STRICT=1`
  - Deferred pickle strict lane: `PYRS_RUN_DEFERRED_PICKLE=1`
- Deferred pickle strict lane uses a dedicated subprocess timeout control:
  - `PYRS_DEFERRED_PICKLE_TIMEOUT_SECS` (default `max(PYRS_STRICT_HARNESS_TIMEOUT_SECS, 600)`)
- Keep strict harness subprocess timeout protections enabled to avoid runaway hangs.

## Canonical Documents (Do Not Duplicate Their Contents Here)
- Roadmap and milestone definitions: `docs/ROADMAP.md`
- Production checklist and release blockers: `docs/PRODUCTION_READINESS.md`
- Stub/partial implementation ledger: `docs/STUB_ACCOUNTING.md`
- Top stdlib common-usecase closure tracker: `docs/STDLIB_COMMON_USECASE_CHECKLIST.md`
- Object-model parity audit log: `docs/OBJECT_MODEL_AUDIT.md`
- Stdlib pure-Python migration strategy: `docs/STDLIB_MIGRATION_PLAN.md`
- Engineering quality gates: `docs/ENGINEERING_GATES.md`
- Algorithmic/semantic audit backlog: `docs/ALGO_AUDIT_BACKLOG.md`
- VM architecture map: `docs/VM_ARCHITECTURE_MAP.md`
- Compatibility matrix: `docs/COMPATIBILITY.md`
- Unicode-name data provenance/regeneration: `docs/UNICODE_NAME_DATA.md`
- Coverage gate workflow: `scripts/run_coverage_gate.sh`
- Optimization execution plan: `docs/OPTIMIZATION_PLAN.md`
- Optimization backlog and status ledger: `docs/OPTIMIZATION_BACKLOG.md`

## Reference Artifacts
- Milestone 12 closure report: `docs/MILESTONE_12_BACKLOG.md`
- Dict backend CPython mapping: `docs/DICT_BACKEND_CPYTHON_MAPPING.md`
- Dict backend benchmark snapshot: `docs/DICT_BACKEND_BENCHMARK.md`
- Clone audit baseline/report: `docs/CLONE_BASELINE.txt`, `docs/CLONE_AUDIT.md`
- No-op inventory snapshot: `docs/NOOP_BUILTIN_INVENTORY.txt`

## Current Focus
- Active top priority: Milestone 13 closure via top-stdlib common-usecase coverage first (`docs/STDLIB_COMMON_USECASE_CHECKLIST.md`), with benchmark-guarded performance maintenance.
- Top-stdlib common-usecase gate current snapshot (local debug, 2026-02-13):
  - import pass: `26/26`
  - smoke pass: `26/26`
  - no red module in the top-stdlib checklist baseline
- Performance suite (canonical):
  - `scripts/bench_fib_gate.sh 5`
  - `scripts/bench_dispatch_hotpath.sh 5`
  - `scripts/bench_dict_backend.sh 5`
- Latest baseline snapshot (2026-02-11, local warm release):
  - `fib(29)x5`: `pyrs ~0.56s` user vs `python3.10 ~0.49s` user (`~1.15x`)
  - dispatch hotpath: `pyrs ~0.44-0.50s` vs `python3.10 ~0.054-0.056s` (`~7.9-9.3x`)
  - dict microbench: `pyrs ~0.24s` vs `python3.10 ~0.02s`
  - pickle hotspot: `pyrs ~5.01s` vs `python3.10 ~0.43s` (`~11.7x`)
- Latest container checkpoint:
  - dict entry->slot backreference map landed to remove O(slots) delete scans and tighten post-delete index maintenance to live-entry-directed updates.
- Latest call-path checkpoint:
  - no-keyword single-argument builtin `len` fast lane is active in opcode call dispatch for hot container loops.
  - no-keyword builtin `bool` zero/single-arg fast lanes are active in opcode call dispatch.
  - `CALL_FUNCTION`/`CALL_FUNCTION1` builtin branches now try direct zero/one-arg no-kwargs fast lanes before generic builtin call fallback.
  - module-scope `LOAD_NAME`/`STORE_NAME` paths now avoid per-opcode name-clone churn; `STORE_NAME` uses indexed storage path with direct module/global upsert.
  - module-scope `LOAD_NAME` now has version-guarded site caching against module+builtins versions.
  - module global writes now synchronize module-frame fast-local slots to keep accelerated `LOAD_NAME` lookups semantically correct.
  - `LOAD_NAME`/indexed `STORE_NAME` now use opcode name-index directly for fast-local slot access instead of `name_to_index` hash lookups.
  - `LOAD_NAME` cache checks now use `frame.function_globals_version` directly, avoiding per-op module-kind version lookups.
- Optimization phase-1 closeout is complete; unresolved throughput gaps remain tracked in `docs/OPTIMIZATION_BACKLOG.md` (`OPT-022` through `OPT-026` and related P1 items).
- CI now runs `scripts/bench_dispatch_hotpath.sh` as non-blocking telemetry and uploads the benchmark artifact for regression tracking.
- Deferred pickle strict lane status:
  - `CPicklingErrorTests.test_bad_newobj_ex_args` parity is landed across proto 2-5 for `_pickle.Pickler`.
  - temporary `_pickle.Pickler.dump` error-remap shim has been removed; save-reduce hook now enforces C-path `__newobj_ex__` argument typing directly.
  - fast decode now handles mixed framed/unframed protocol-4/5 streams and memo opcodes (`MEMOIZE`/`BINGET`/`LONG_BINGET`/`BINPUT`/`LONG_BINPUT`), eliminating many `_loads` fallbacks.
  - `Unpickler.load` fast-probe fallback now preserves caller exception state, so unseekable streams (`tell`/`seek` raising `UnsupportedOperation`) correctly fall back instead of surfacing probe errors.
  - deferred strict pickle suite still times out (`test/test_pickle.py` > 600s) and remains open; remaining work is throughput closure for heavy pure-`pickle._Unpickler` paths.
- sqlite/json checkpoint:
  - `_sqlite3` baseline is landed: module import surface, `connect`, `Connection.cursor/execute/close`, `Cursor.execute/fetchone/fetchall/close`, adapter/converter registries, and core exception/type exports.
  - `Connection.blobopen` and `_sqlite3.Blob` baseline native methods are wired (`close`, `read`, `write`, `seek`, `tell`, context-manager hooks, `__len__`, `__getitem__`, `__setitem__`) with regression coverage.
  - `_sqlite3` constant export surface now includes CPython-style authorizer/limit/dbconfig constants (`SQLITE_LIMIT_*`, `SQLITE_DBCONFIG_*`, etc.).
  - `_sqlite3` connection/cursor surface now also includes `Connection.__del__`, descriptor-backed `Connection` attributes (`isolation_level`, `in_transaction`, `total_changes`), SQL-length/DataError parity guard, cursor `description`, row/text-factory plumbing, and `_sqlite3.Row` baseline methods (`keys`, `__len__`, `__getitem__`, `__iter__`, `__eq__`).
  - inspect signature closure landed for sqlite callables: `inspect.signature(obj)` now consumes `__text_signature__`, and inspect `Signature.__str__/__repr__` render CPython-style signature text.
  - Current `test.test_sqlite3.test_dbapi` failfast frontier is `test_in_transaction` (transaction-state parity after DML + commit/rollback flows).
  - pure-stdlib JSON remains the default when CPython `Lib/` is available, and `_json` scanner integration now handles `json.loads` decode flow with correct regex `pos/endpos` handling.
- Hashlib checkpoint:
  - native `_md5` and `_sha2` backends are wired using Rust crypto crates with constructor/update/digest/hexdigest/copy parity tests.
  - common `hashlib.md5` and `hashlib.sha256` stdlib paths are now green.
- Enum shim policy:
  - local `shims/enum.py` remains active and currently precedes CPython `Lib/enum.py` on `sys.path` to avoid known pure-enum bootstrap incompatibilities.
  - `http.client` import-chain baseline is now green; full enum-member parity closure is still required before this shim can be retired.
- Runtime implementation identity:
  - `sys.implementation.name` is `pyrs` (not `cpython`) so CPython-only stdlib tests skip correctly.
  - `sys.implementation.cache_tag` remains `cpython-314` for bytecode cache compatibility.
- Active strict stdlib suite now includes `test/test_memoryio.py` (green, with CPython-only tests skipped).
- `_io` parity checkpoints landed this round:
  - `IOBase` close/flush/finalizer default semantics for `_io`/`io` base classes.
  - `RawIOBase` default `read`/`readall` via `readinto`.
  - `BufferedIOBase` default `readinto`/`readinto1`.
- Compiler correctness checkpoint landed this round:
  - named-expression (`:=`) targets now participate in local-scope collection for statement/default-expression analysis, closing `_pyio.__del__` walrus leakage into module globals.
  - temporary assignment carrier names (`__pyrs_assign_*`) are deleted after attribute/subscript stores, and module-scope `DELETE_NAME` now clears fast-local-only names; this removed a hidden ref-retention path that affected GC-sensitive flows.
- Runtime parity checkpoint landed this round:
  - weakrefs now stay dead once object finalization starts, matching CPython behavior during `__del__` side-effect windows (e.g. warning payloads).
  - `len()` unsupported-type surfaces now raise `TypeError` classification with object type context.
  - OS path helpers now raise `ValueError` for embedded NUL bytes before syscall dispatch.
  - runtime-exception metadata parity advanced: `OSError` now derives `errno`/`strerror`/`args` more consistently, and class/instance normalization ensures `args` is populated.
  - `sys.flags` now exposes CPython 3.14 fields used by stdlib, including `warn_default_encoding`, `gil`, `thread_inherit_context`, and `context_aware_warnings`.
  - `_io.BufferedReader.readline()` now wraps bad `readinto()` type/value returns as `OSError`, with `TypeError`-cause linkage for the non-int return path.
  - buffered-read caching for `_io.BufferedReader` now honors buffer-size prefetch patterns and seek/tell invalidation semantics (strict `test_buffering` path now green).
- `test.test_io` failfast probe now clears CIOTest + PyIOTest and deep CBufferedReader coverage (close ordering/context, `detach`, `read1`/`peek`/`readinto1`, readonly attr and recursive repr behavior, char-device seek/tell sanity, threaded reads, and readonly `truncate` semantics); core `bytes.count`/`bytearray.count` support has landed, and the current first blocker is outside `_io`: `CBufferedReaderTest.test_uninitialized`, caused by `_sre` regex alternation mismatch in `assertRaisesRegex`.
- Optimization work must reference CPython internals directly (`Python/ceval.c`, `Python/generated_cases.c.h`, `Include/internal/pycore_frame.h`, `Objects/call.c`, `Objects/longobject.c`) and track decisions in `docs/OPTIMIZATION_PLAN.md`.
- Optimization item status must be updated in `docs/OPTIMIZATION_BACKLOG.md` in the same checkpoint as performance changes.
- If optimization work is resumed as primary focus, it must explicitly close foundational missing surfaces tracked in backlog (`OPT-022` string interning strategy and remaining `OPT-023+` dispatch/call/container items).
