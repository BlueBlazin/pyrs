# Project Context: Python Interpreter in Rust (`pyrs`)

## Vision
Build a production-grade Python interpreter in Rust with source + bytecode compatibility for CPython 3.14, minimal third-party dependencies, and architecture that can later support JIT and extension work.

## Non-Negotiable Engineering Rule
- Do not use quick fixes as a substitute for correct design.
- Prioritize root-cause, foundational solutions over tactical patches.
- Any temporary workaround must be explicitly marked and tracked with closure criteria in:
  - `docs/STUB_ACCOUNTING.md`, or
  - `docs/ALGO_AUDIT_BACKLOG.md`.

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
  - CPython-like runtime object model, refcount + cycle GC, and GIL
  - Minimal, justified dependencies only

## Milestone State
- Milestones 0-12: complete
- Milestone 13: in progress (active)
- Milestones 14-16: pending

Milestone 13 closes only when P0 blockers in `docs/PRODUCTION_READINESS.md` and `docs/STUB_ACCOUNTING.md` are fully closed.

## Current Snapshot (2026-02-13)
- Top-stdlib common-usecase gate: `26/26` import, `26/26` smoke.
- Extended stdlib probe: `50/50` import, `47/50` smoke (`perf/stdlib_compat_extended_latest.json`).
- Newly landed parity checkpoints:
  - `math.gcd()` baseline (unblocks `fractions` common path).
  - `threading.Condition.__enter__/__exit__` baseline.
  - `datetime.date/datetime.strftime()` baseline.
  - `_operator._compare_digest` baseline and `_operator` module registration.
  - `collections.deque` class surface (`__init__`, `append*`, `pop*`, `extend*`, `clear`, `__len__`, `__iter__`) wired into module bootstrap.
  - `bytes` / `bytearray` constructor VM paths now accept generator/iterator/iterable payloads and explicit `encoding`/`errors` argument forms.
  - `datetime.date/datetime` gained `toordinal`, `weekday`, `isoweekday`; `datetime.timezone` baseline symbol added for stdlib import-chain compatibility.
  - `datetime.datetime.fromtimestamp` + `datetime.astimezone` fixed-offset baseline landed, including `%z` formatting in `strftime`.
  - synthetic exception-class materialization for `Value::ExceptionType` bases now builds CPython-style exception ancestry and wires `ExceptionTypeInit` to unblock stdlib exception subclasses.
  - `_sre` pattern object gained `split`; class/instance `__doc__` fallback parity tightened for stdlib object-model paths.
  - internal-call exception propagation now treats caller `active_exception` deltas as propagated failures (prevents false-success stack pops/underflow in descriptor/property call paths).
  - `str.join` now accepts `str` subclass instances via backing-string extraction.
  - `super(...).__init__` for synthetic builtin `list`/`dict` bases now resolves to native container initializers (`list.__init__`, `dict.__init__`) instead of falling through to `object.__init__`.
  - codec normalization now accepts core CPython aliases (`us-ascii`, `iso-8859-1`, etc.) in both codec paths and `bytes(..., encoding=...)` construction.
  - regex `Match` now supports subscript group aliasing (`m[0]`, `m[1]`, ...), including module-wrapper dispatch needed by CPython email header folding paths.
  - instance subscription now delegates to tuple/str backing for synthetic builtin subclasses; `str()` now returns backing text for str-backed instances.
  - targeted CPython `email` smoke (`EmailMessage` header/content fold + `as_string`) is green locally; extended matrix artifact refresh pending.
  - numeric parity checkpoint: `int` now exposes `numerator`/`denominator`/`real`/`imag`; `sum()` now uses binary add runtime fallback; `float()` now honors `__float__`; primitive numeric instances satisfy `numbers` ABC checks used by `fractions`/`statistics`.
  - regex parity checkpoint: `_sre` now recognizes CPython `_pydecimal` parser pattern structure with named-group captures (`sign`/`int`/`frac`/`exp`/`signal`/`diag`) and matching `groupindex` mapping.
  - import binding parity: `import` / `__import__` now bind the canonical `sys.modules[name]` module object when module code replaces the entry during execution.
  - pure `decimal` preference: when CPython `Lib/decimal.py` is available on `sys.path`, builtin bootstrap `decimal` is unloaded and pure `decimal`/`_pydecimal` is used.
  - enum shim retirement: `shims/enum.py` is removed; enum import now resolves only through CPython `Lib/enum.py` path.
  - exception hierarchy parity: `LookupError`/`IndexError`/`KeyError`, `ArithmeticError` family, warning family, pickle error family, and several core parents now follow CPython ancestry.
  - exception-match resilience: `except` matching now falls back to active exception state when stack operands are polluted by import-failure edges.
  - heavy CPython-stdlib VM tests now run on dedicated 32MB stack threads for stability (`import_http_client_runs_package_init_first`, `pyio_fileio_del_namedexpr_does_not_leak_bound_method_or_pin_cycle`, `c_pickler_newobj_ex_argument_type_errors_match_cpython_protocols_2_through_5`, `pickle_newobj_generic_matrix_from_pickletester_roundtrips`, `prefers_cpython_pkgutil_and_resources_over_local_shims_when_stdlib_is_available`, `pkgutil_resolve_name_accepts_module_only_target`).
  - additional pickle-heavy VM tests now run on dedicated 32MB stack threads (`pickle_protocol4_dict_chunking_emits_multiple_setitems_for_large_dicts`, `pickle_slot_list_roundtrip_preserves_slots_and_dynamic_dict_attrs`, `with_assert_raises_handles_missing_attr_without_stack_underflow`), and local full `cargo test -q --test vm` is green.
  - CPython harness import suites (`runs_cpython_language_suite`, `runs_cpython_import_suite`) now run on dedicated 32MB stack threads to avoid debug-thread stack overflows during deep import chains.
  - strict stdlib lane remains green after enum-shim retirement (`PYRS_RUN_STRICT_STDLIB=1 cargo test -q --test cpython_harness runs_cpython_strict_stdlib_suite`).
  - `_io.open` now preserves raw bytes paths when dispatching opener callbacks (no lossy bytes->str conversion), with regression coverage in `tests/vm.rs::io_open_passes_bytes_path_to_opener_without_lossy_conversion`.
  - `_sqlite3.connect` / `Connection.__init__` now preserve raw bytes/bytearray database paths when calling `sqlite3_open_v2` (no lossy UTF-8 replacement in native handoff).
  - `_sqlite3.connect` now accepts path-like (`__fspath__`) database arguments, matching CPython DB-API expectations.
  - `_sqlite3.Row` parity advanced: equality now compares description + row payload (not identity), and `issubclass(sqlite3.Row, collections.abc.Sequence)` / `isinstance(row, Sequence)` now pass in the runtime.
  - `_sqlite3` thread-affinity guard is now tracked in native connection state and enforced on connection operations when `check_same_thread=True` (default), matching CPython policy.
  - `_sqlite3` thread-affinity checks now apply to cursor operations (`close`/`fetch*`/`set*size` and other cursor methods), matching CPython `test_dbapi.ThreadTests` expectations.
  - `_sqlite3.Connection` now exposes `set_trace_callback`, `set_authorizer`, `set_progress_handler`, `create_collation`, `create_window_function`, and `iterdump`; `iterdump()` delegates to CPython `Lib/sqlite3/dump.py::_iterdump`.
  - `_sqlite3.Connection.backup()` now uses SQLite backup APIs with CPython-like semantics for target validation, `pages`, `progress`, `name`, and `sleep`.
  - `_sqlite3.autocommit` now supports `True` / `False` / `sqlite3.LEGACY_TRANSACTION_CONTROL` with transition semantics aligned to CPython (`BEGIN`/`COMMIT`/`ROLLBACK` behavior, context-manager mode behavior, and `close()` implicit rollback for disabled mode).
  - `_sqlite3` callback surfaces (`set_trace_callback`, `set_authorizer`, `set_progress_handler`, `create_collation`) now route through native sqlite callback APIs, including callback-traceback/unraisable handling and deprecated-keyword warning behavior expected by CPython 3.14 tests.
  - `sqlite3` DB-API failfast probe (`Lib/test/test_sqlite3/test_dbapi.py`) is now green locally.
  - strict stdlib harness suite now includes `test/test_sqlite3/test_dbapi.py`, `test/test_sqlite3/test_backup.py`, `test/test_sqlite3/test_factory.py`, `test/test_sqlite3/test_dump.py`, `test/test_sqlite3/test_transactions.py`, and `test/test_sqlite3/test_hooks.py` and stays green (`PYRS_RUN_STRICT_STDLIB=1 cargo test -q --test cpython_harness runs_cpython_strict_stdlib_suite`).
  - `_sqlite3` factory parity checkpoint: `connect(factory=ConnectionSubclass)` now relays kwargs and preserves `Base Connection.__init__ not called.` behavior for defective subclasses; `Connection.cursor(factory=...)` now follows CPython callable/class dispatch semantics, including positional/keyword `factory` handling, callable return-type validation, and native `Cursor.__init__` substrate wiring used by cursor subclasses.
  - `_sqlite3.Row` parity checkpoint: `dict(Row)` mapping conversion now follows mapping-protocol behavior, ordering comparisons now raise `TypeError` (no false ordering via equality fallback), and row hash parity (`hash(description) ^ hash(data)`) is wired via native `Row.__hash__`.
  - runtime threading identity emulation now assigns per-start synthetic ids for `_thread.start_new_thread` and `threading.Thread.start` target execution; `threading.get_ident()` reports those ids inside spawned targets.
  - object-model parity checkpoint: `object.__format__` now follows CPython semantics (empty spec -> `str(self)`, non-empty spec -> `TypeError`), unblocking unittest subtest rendering paths that rely on `str.format`.
  - builtin `threading` module now exposes `_dangling` registry baseline required by CPython test/support threading helpers.
- Extended probe remaining red modules:
  - `xml`, `gzip`, `bz2`, `lzma`.
  - `smtplib` targeted smoke is green but still logs unsupported `hashlib` algorithms (`sha1`/`sha3`/`blake*`/`shake*`).

## Execution Policy
- CPython behavior is the source of truth:
  - `Modules/*.c`
  - `Objects/*.c`
  - `Lib/*.py`
- Sequence Milestone 13 work as native-core-first:
  1. Native/runtime substrate closure (`_io`, `_csv`, `_sre`, `_pickle`, object protocol)
  2. Pure-stdlib strict-lane expansion/closure
- Prefer official CPython pure-Python stdlib implementations where feasible.
- Keep native handlers as substrate/accelerator layers, not replacement semantics.
- Local shim policy:
  - CPython `Lib/enum.py` path is now the default.
  - local `enum` shim has been retired (`shims/enum.py` removed); enum behavior now always follows CPython `Lib/enum.py` when stdlib is present.
  - `pkgutil`/`importlib.resources` local shims are fallback-only and require `PYRS_ENABLE_LOCAL_SHIMS=1`.
  - CPython enum probe regression: `tests/vm.rs::cpython_enum_path_supports_member_value_and_name`.
- Keep docs updated in the same checkpoint as behavior changes.
- Keep worktrees clean; commit small focused checkpoints.
- End every assistant turn with immediate next `3-6` concrete steps.

## Test Loop Policy
- Fast local loops: targeted unit/integration tests first.
- Strict stdlib harness is opt-in for frequent local loops:
  - `PYRS_RUN_STRICT_STDLIB=1`
  - `PYRS_PARITY_STRICT=1`
- Deferred strict pickle lane is opt-in until closure:
  - `PYRS_RUN_DEFERRED_PICKLE=1`
  - `PYRS_DEFERRED_PICKLE_TIMEOUT_SECS` (default `max(PYRS_STRICT_HARNESS_TIMEOUT_SECS, 600)`)

## Performance Policy
- Optimization phase-1 checkpoint is complete.
- Functional Milestone 13 closure is active with benchmark regression protection.
- Canonical benchmark gates:
  - `scripts/bench_fib_gate.sh 5`
  - `scripts/bench_dispatch_hotpath.sh 5`
  - `scripts/bench_dict_backend.sh 5`
- All optimization work must update `docs/OPTIMIZATION_BACKLOG.md` in the same checkpoint.

## Canonical Documents
- Docs index and ownership map: `docs/README.md`
- Milestones and sequencing: `docs/ROADMAP.md`
- Production blockers and release criteria: `docs/PRODUCTION_READINESS.md`
- Partial/stub ledger: `docs/STUB_ACCOUNTING.md`
- Top stdlib common-usecase tracker: `docs/STDLIB_COMMON_USECASE_CHECKLIST.md`
- Object-model parity audit: `docs/OBJECT_MODEL_AUDIT.md`
- Pure-stdlib migration policy: `docs/STDLIB_MIGRATION_PLAN.md`
- Engineering gates: `docs/ENGINEERING_GATES.md`
- Algorithmic/semantic audit backlog: `docs/ALGO_AUDIT_BACKLOG.md`
- Compatibility matrix: `docs/COMPATIBILITY.md`
- VM architecture map: `docs/VM_ARCHITECTURE_MAP.md`
- Optimization execution plan: `docs/OPTIMIZATION_PLAN.md`
- Optimization backlog/status: `docs/OPTIMIZATION_BACKLOG.md`
- Builtin parity gate and policy: `docs/BUILTIN_PARITY.md`, `docs/BUILTIN_OPTIMIZATION_POLICY.md`
- Unicode-name table provenance: `docs/UNICODE_NAME_DATA.md`

## Reference Artifacts
- Milestone 12 closure report: `docs/MILESTONE_12_BACKLOG.md`
- Dict backend CPython mapping: `docs/DICT_BACKEND_CPYTHON_MAPPING.md`
- Dict backend benchmark snapshot: `docs/DICT_BACKEND_BENCHMARK.md`
- Clone audit artifacts: `docs/CLONE_BASELINE.txt`, `docs/CLONE_AUDIT.md`
- No-op inventory snapshot: `docs/NOOP_BUILTIN_INVENTORY.txt`
