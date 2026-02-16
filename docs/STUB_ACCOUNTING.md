# Stub and Partial Implementation Ledger

This is the canonical ledger for incomplete runtime/stdlib behavior.
No partial surface may remain untracked.

Status values:
- `OPEN`
- `IN_PROGRESS`
- `CLOSED`

## Enforcement
- No-op inventory artifact: `docs/NOOP_BUILTIN_INVENTORY.txt`
- No-op inventory gate: `tests/noop_inventory.rs`
- Refresh command:
  - `cargo run --quiet --bin print_noop_inventory > docs/NOOP_BUILTIN_INVENTORY.txt`
- Process/quality gates:
  - `docs/ENGINEERING_GATES.md`
  - `docs/ALGO_AUDIT_BACKLOG.md`

## Milestone 13 P0 Blockers

| Surface | Gap summary | Closure criteria | Required evidence | Status | Milestone |
|---|---|---|---|---|---|
| `pickle`/`pickletools`/`copyreg` | Deferred strict pickle harness lane still times out. | Deferred strict pickle harness lane is re-enabled, green, and allowlist is empty. | `PYRS_RUN_DEFERRED_PICKLE=1 cargo test -q --test cpython_harness runs_cpython_deferred_pickle_suite` | IN_PROGRESS | 13 |
| `_io` | Strict stdlib still depends on remaining `_io` edge semantics. | Remaining `_io`-dependent strict-harness-lane failures are closed. | strict harness pass + targeted `tests/vm.rs` regressions for each closed edge | IN_PROGRESS | 13 |
| `json` | Common workflows are green; long-tail semantics/hardening/perf closure is open. | `json` pure + accelerator parity closed, malformed-input differential coverage closed, perf baseline recorded. | `test_json` parity + differential probes + benchmark artifact | IN_PROGRESS | 13 |
| `_csv`/`csv` | Common workflows are green; long-tail dialect/error parity is open. | `test_csv` parity closed with malformed-input hardening and perf baseline. | `test_csv` parity + differential probes + benchmark artifact | IN_PROGRESS | 13 |
| `_sre` | Core surface works; long-tail regex behavior blocks full pure-`re` closure. | Pure `Lib/re/*` path passes curated/strict harness lanes in scope. | strict/curated harness green for `re`-dependent suites | IN_PROGRESS | 13 |
| `.pyc` exception-table execution | Exception-table-driven translated-`.pyc` execution is now active (`PUSH_EXC_INFO`/`POP_EXCEPT`/`WITH_EXCEPT_START`/`RERAISE`/`CHECK_EXC_MATCH` + table-driven unwind/dispatch), and startup `import site` no longer needs source fallback for this gap. Remaining work is `.pyc` long-tail opcode/state parity. | Keep exception-table translated-`.pyc` paths green and close remaining `.pyc` long-tail opcode/state gaps so in-scope startup/import paths stay on `.pyc` without fallback. | pyc preference probes (`PYRS_IMPORT_PREFER_PYC=1`) plus targeted translated-`.pyc` regression tests covering `try`/`except`/`with`/`finally` flows | IN_PROGRESS | 13/14 |
| `_ssl`/`ssl` | `_ssl` baseline is landed and a native bootstrap `ssl` module now closes extended matrix import/common smoke. Remaining gap: full CPython `Lib/ssl.py` path is still blocked by namedtuple/super object-model semantics. | Replace temporary bootstrap `ssl` module with CPython `Lib/ssl.py` as default path, while keeping common-usecase smoke green and adding regression coverage for the namedtuple/super closure. | Extended probe (`perf/stdlib_compat_extended_latest.json`) shows `ssl` import/smoke green + targeted `tests/vm.rs` coverage + object-model closure evidence. | IN_PROGRESS | 13 |
| `pyexpat`/`xml` | Native/runtime `pyexpat` baseline is installed (`ParserCreate`, `ExpatError`, parser callbacks) and `shims/pyexpat.py` is removed. | Keep XML common-smoke and parse-error regressions green without shim fallback. | `tests/vm.rs::xml_elementtree_fromstring_smoke_uses_native_pyexpat`, `tests/vm.rs::pyexpat_parse_error_exposes_code_lineno_offset`, strict/extended suites green. | CLOSED | 13 |
| Hash containers (`dict`/`set`/`frozenset`) | Architecture upgrade landed; long-tail semantic/perf closure remains. | CPython parity on edge behavior and performance closure criteria from readiness/audit docs. | targeted parity tests + benchmark/profile artifacts | IN_PROGRESS | 13/14 |
| Builtin symbol surface (`builtins`) | Parity gate currently green. | Keep `145/145`, zero probe mismatches, and empty allowlists. | `./scripts/run_builtin_parity_gate.sh` | CLOSED | 13 |

## Active Non-P0 Partial Surfaces

| Surface | Gap summary | Closure criteria | Status | Milestone |
|---|---|---|---|---|
| Extension ecosystem substrate (`.pyrs-ext` + shared-library loading) | Manifest-backed loader, direct shared-object loading (`.so/.dylib/.pyd`), compiled extension smoke coverage, tagged-filename resolution, C-API v1 slice (module setters/getters/import/attr-load + positional/keyword callable registration + init-scoped object handles/type getters/generic len-getitem helpers/sequence+dict access + iterator helpers + list/dict mutation helpers + object attribute get/set/del/has + type relation checks (`isinstance`/`issubclass`) + handle-based callable invocation (`object_call`, `object_call_noargs`, `object_call_onearg`) + capability probe + import-time error state + error message inspection), and `_sysconfigdata__*` extension-build var baseline (now with compile+import smoke) are landed. Direct CPython-style `PyInit_<module>` fallback now executes through an initial single-phase compatibility slice (`PyModule_Create2`, `PyModule_AddObjectRef`, `PyModule_AddIntConstant`, `PyModule_AddStringConstant`, `PyLong/PyFloat/PyUnicode/PyBytes/PyBool` constructors, `PyErr_*`, `Py_[X]IncRef/Py_[X]DecRef`). Current partial compatibility notes: `PyEval_EvalCodeEx` currently supports only the simple no-args/no-closure call form (extended args/defs/closure semantics remain open), `PyEval_EvalFrame`/`PyEval_EvalFrameEx` currently provide null/current-frame guard behavior but not full CPython frame-evaluation semantics, `PySys_Audit`/`PySys_AuditTuple` currently return success without hook dispatch semantics, codec handler APIs (`PyCodec_*Errors`, `PyCodec_LookupError`) currently provide strict type-guard + baseline tuple behavior but not full CPython Unicode-error transformation semantics, and `PyMember_SetOne` currently omits CPython truncation/runtime-warning emission for narrowing or negative-to-unsigned writes (value-write baseline is implemented). Remaining work: broad CPython C-ABI surface closure, type-object/exception-object exports, PEP 489 multi-phase lifecycle, and scientific-stack execution without shims/bridge. | `docs/EXTENSION_CAPABILITY_MATRIX.md` tracks required C runtime + lifecycle surfaces, `tests/extension_smoke.rs::imports_direct_cpython_style_single_phase_extension` is green, and `perf/numpy_gate_direct_latest.json` tracks remaining direct-mode failures. | IN_PROGRESS | 15 |
| Importlib/resources/pkgutil helpers | `pkgutil` fallback is now native (`get_data`, `resolve_name`, `iter_modules`, `walk_packages`) and local `pkgutil` shim is removed; long-tail `importlib.resources` behavior remains partial. | In-scope CPython compatibility for packaging/resource paths. | IN_PROGRESS | 13 |
| `inspect`/`types` | Foundational behavior exists; stdlib-required edges remain. | Full stdlib-required behavior parity in scope. | IN_PROGRESS | 13 |
| `threading`/`signal`/`_thread`/`_warnings` | Foundations exist; behavior depth is incomplete. `concurrent.futures` common smoke is now green after iterator protocol + semaphore bound fixes. | Full in-scope behavioral parity in strict/curated suites beyond common smoke paths. | IN_PROGRESS | 13/16 |
| `socket`/`_socket` | Baseline exists; long-tail API/behavior remains. | Full in-scope API and behavior parity. | IN_PROGRESS | 13 |
| `uuid` | Foundation exists; long-tail parity remains. | Full in-scope API parity. | IN_PROGRESS | 13 |
| `_sqlite3`/`sqlite3` | Baseline is broad; DB-API long-tail remains. Bytes/bytearray database arguments now flow to sqlite using raw bytes (no lossy UTF-8 replacement), path-like database arguments are accepted, connection+cursor thread-affinity checks are enforced when `check_same_thread=True`, `Connection.iterdump()` is wired through CPython `Lib/sqlite3/dump.py::_iterdump`, `Connection.backup()` is now implemented on top of SQLite backup APIs with CPython-like target/progress/name/pages/sleep semantics, `autocommit` attribute transitions are implemented (`True`/`False`/`LEGACY_TRANSACTION_CONTROL`) with context-manager/commit/rollback semantics, `connect(factory=...)` now relays kwargs and preserves `Base Connection.__init__ not called.` parity for defective subclasses, `Connection.cursor(factory=...)` now follows CPython callable/class dispatch (`factory` positional/keyword semantics, callable return-type validation, and native `Cursor.__init__` substrate), `set_authorizer()` / `set_progress_handler()` / `set_trace_callback()` / `create_collation()` are now backed by native sqlite callback APIs (including callback-traceback and keyword-deprecation behavior used by CPython tests), and `Row` mapping/order/hash behavior now matches `test_factory` expectations (`dict(Row)`, ordering-TypeError paths, stable hash parity for equal rows). Strict stdlib suite runs `test/test_sqlite3/test_dbapi.py`, `test/test_sqlite3/test_backup.py`, `test/test_sqlite3/test_factory.py`, `test/test_sqlite3/test_dump.py`, `test/test_sqlite3/test_transactions.py`, and `test/test_sqlite3/test_hooks.py` green. | Close remaining DB-API long-tail (broader type/threading-runtime parity depth and remaining strict-scope module surfaces such as types/userfunctions/regression). | IN_PROGRESS | 13 |
| `decimal`/`_pydecimal` | Bootstrap `decimal` remains as no-stdlib fallback, but CPython `Lib/decimal.py` is now preferred when available on `sys.path`; targeted pure decimal constructor/context/addition smoke is green. | Refresh extended matrix artifact and close remaining decimal long-tail parity in curated/strict suites. | IN_PROGRESS | 13 |
| `dataclasses`/`typing`/`enum`/`contextvars` | Common paths are green; CPython `Lib/enum.py` path is baseline-green/default and local `enum` shim is retired. Remaining work is long-tail enum semantics only. | Full in-scope semantics for modern pure-Python apps. | IN_PROGRESS | 13 |
| `hashlib` extended algorithms (`_sha1`/`_blake2`/`_sha3`/`_hashlib`) | md5/sha2 baseline closed; broader algorithm surface open. | Full in-scope algorithm surface (or explicit exclusions) with tests and consumers green. | IN_PROGRESS | 13/14 |
| Compression extensions (`zlib`/`_bz2`/`_lzma`) | Native baselines now close common import/one-shot flows for `gzip`/`bz2`/`lzma`; streaming/filter-property long-tail semantics remain partial. | Strict/curated suites and extended probe remain green while placeholder filter APIs are retired. | IN_PROGRESS | 13 |
| Object-model protocol dispatch | Truthiness/membership baseline landed; long-tail slot/error semantics remain. `bytes.lstrip`/`bytes.strip` parity now covers `gzip.decompress` common path, memoryview typed scalar index/store format semantics are landed, first-axis multidim memoryview slice/tolist shape+stride parity is landed, and memoryview byte-export/iteration now honor strided layout + typed decode; remaining memoryview multi-dimensional indexing/slice-assignment long-tail behavior is still open. | Align remaining protocol edge semantics with CPython data model/tests. | IN_PROGRESS | 13 |
| VM/module decomposition | VM still has large modules. | Continue concern-based extraction with behavior-preserving tests. | IN_PROGRESS | 14 |

## Strict Harness Lane Accounting
- Active strict suite: `tests/cpython_suite_strict_stdlib.txt`
- Deferred strict pickle suite: `tests/cpython_suite_deferred_pickle.txt`
- Active strict allowlist: `tests/cpython_allowlist_strict.txt` (target: empty)
- Deferred strict pickle allowlist: `tests/cpython_allowlist_deferred_pickle.txt` (target: empty)

Policy:
1. Active strict harness lane stays green with empty allowlist.
2. Deferred strict pickle harness lane stays explicit until re-enabled and closed.
3. Deferred strict pickle harness lane remains opt-in locally (`PYRS_RUN_DEFERRED_PICKLE=1`) for bounded fast loops.

## Local Shim Retirement Checklist

Shims are temporary bootstrap fallbacks and are not allowed to shadow CPython `Lib/` when it is available.

| Shim surface | Current state | Closure criteria | Status |
|---|---|---|---|
| `enum` | Local `shims/enum.py` is removed; enum resolution now follows CPython `Lib/enum.py` only. | Keep `tests/vm.rs::cpython_enum_path_supports_member_value_and_name` and strict stdlib suites green without enum-specific fallback toggles. | CLOSED |
| `pkgutil` | Local shim is removed (`shims/pkgutil.py` deleted); stdlib-less fallback is now native runtime (`pkgutil` builtin module surface). | Keep fallback resource and resolve-name regressions green without filesystem shim fallback. | CLOSED |
| `importlib.resources` | Local shim is fallback-only (allowlist-restricted) with fallback enabled by default. | Remove shim after stdlib-less bootstrap requirement is removed or replaced by native/runtime capability. | IN_PROGRESS |
| `pyexpat` | Local shim is removed (`shims/pyexpat.py` deleted); runtime-native parser baseline is active. | Keep XML parser regressions green on native path. | CLOSED |

## Remaining Intentional NoOp Scope
- Test-only CPython helper modules (`_testcapi`, `_testinternalcapi` family)
- `sys.monitoring` and `sys._jit` scaffolding

These entries must not silently expand.

## Update Rules
1. New partial behavior must add/update a row in this file in the same commit.
2. Rows may be marked `CLOSED` only with linked regression tests and required performance evidence (where applicable).
3. Milestone 13 cannot close while any P0 blocker row here is not `CLOSED`.
