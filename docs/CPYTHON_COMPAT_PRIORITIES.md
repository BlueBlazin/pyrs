# CPython Compatibility Priorities

## Purpose

This document turns the checked-in CPython compatibility benchmark into a
practical execution order.

It is intentionally narrower than the old planning docs. The evidence for this
file comes from:

- `perf/cpython_compat_benchmark_latest/summary.json`
- `perf/cpython_compat_benchmark_latest/derived_summary.json`
- `scripts/run_cpython_compat_focus.py`
- targeted runtime and harness tests under `tests/`

## Current Snapshot

Artifact root: `perf/cpython_compat_benchmark_latest`

Snapshot metadata:

- generated at: `2026-03-09T05:36:36Z`
- git head: `775b50502268024d67fee75758348f1fdbff8a69`
- host: macOS `arm64`

Headline counts:

- discoverable benchmark entries: `492`
- runnable entries after inventory: `454`
- clean-pass modules: `41`
- modules that execute but still fail cases: `265`
- blocked modules (`load_error` + `process_error` + `process_timeout`): `118`
- discoverable test cases: `47,040`
- executed case outcomes: `23,303`
- passed case outcomes: `11,332`
- executed subtest outcomes: `41,421`
- passed subtest outcomes: `38,344`

The main leverage signal is still blocked execution coverage:

- discoverable cases currently trapped behind blocked modules: `23,051`

Movement from the prior checked-in snapshot:

- clean-pass modules moved from `38` to `41`
- blocked modules moved from `138` to `118`
- discoverable cases hidden behind blocked modules moved from `26,439` to
  `23,051`
- executed case outcomes moved from `19,793` to `23,303`
- major drivers were the recent import/bootstrap fixes for socket protocol
  constants, `datetime.timetuple()`, `array` reconstruction/type-object
  surface, `_interpreters` import substrate, threading native-id exposure,
  SyntaxError attribute normalization, container `super().__init__`
  dispatch, and the GC/finalizer fix that restored CPython-style collection of
  deferred `__del__` self-cycles during `gc.collect()`

## Prioritization Rules

1. Fix root causes that convert blocked modules into runnable modules before
   chasing isolated assertion deltas.
2. Prefer changes that move shared runtime substrate used by many stdlib
   modules.
3. Prefer user-visible runtime and stdlib surfaces over CPython-internal
   test-helper-only rows.
4. Promote areas that already have dedicated source modules and targeted tests
   even if the raw benchmark count is smaller.

## Current Execution Order

| Order | Lane | Why now | Primary focused suite(s) |
|---|---|---|---|
| 1 | Process errors and timeouts | Largest blocked bucket now hides `12,719` discoverable cases | `timeouts-crashes`, `high-leverage` |
| 2 | Import/bootstrap load errors | Remaining load blockers still hide `10,332` discoverable cases | `high-leverage`, `import-bootstrap` |
| 3 | Asyncio / OS / socket transport parity | Largest user-visible runnable cluster now spans asyncio task coverage plus socket and OS gaps | `high-leverage`, `os-fs-socket` |
| 4 | Object model / container / call / format parity | Broadest cross-cutting runtime lane after concurrency/transport work | `object-model-call` |
| 5 | Text/codecs/XML substrate | Concentrated cluster with good focused payoff after broader runtime lanes move | `text-codecs-xml` |

## Lane Details

### 1. Process errors and timeouts

Why first:

- `35` modules currently end in `process_error`
- `29` modules currently end in `process_timeout`
- together they hide `12,719` discoverable cases

Largest blocked rows:

- timeouts:
  - `test.test_email`
  - `test.test_datetime`
  - `test.test_pickle`
  - `test.test_decimal`
  - `test.test_set`
  - `test.test_sqlite3`
  - `test.test_sys_settrace`
- process errors:
  - `test.test_tarfile`
  - `test.test_io`
  - `test.test_statistics`
  - `test.test___all__`

Observed patterns:

- hard crashes and negative return codes (`-10`, `-11`, `-6`)
- `_io` destructor/finalizer churn precedes at least one of the aborting runs
- non-terminating behavior is now concentrated in email, datetime, pickle,
  decimal, set, sqlite3, and tracing

Closure evidence:

- focused runs stop reporting `process_error` / `process_timeout`
- the module starts producing ordinary case-level failures
- targeted tests capture the old crash or hang trigger

### 2. Import/bootstrap load errors

Why second:

- `54` runnable modules currently stop at `load_error`
- those rows hide `10,332` discoverable cases
- remaining failures are shared parser/import/class-subclass substrate defects

Current snapshot signatures still to address:

- `test.test_capi` load/import substrate gaps
- `test.test_ctypes` load/import substrate gaps
- `test.test_idle` configparser/string-method substrate gaps

Resolved locally since this snapshot and awaiting the next full benchmark rerun:

- `RuntimeError: parse error in module 'test.test_pathlib.test_pathlib' ... expected Colon`
- `TypeError: object.__init_subclass__() takes no keyword arguments`
- `AttributeError: module '_frozen_importlib' has no attribute '_ModuleLock'`
- builtin `os.path.splitext()` now matches CPython’s leading-dot behavior (`'..' -> ('..', '')`), which cleared `pathlib.types` stem/suffix failures
- builtin `slice` now exposes `start` / `stop` / `step` plus bound and unbound `indices()`, which cleared `Path.parents` slicing in `test.test_pathlib`
- builtin `os.path.splitroot()` is now exposed with CPython-compatible POSIX results, which cleared `Path.with_name()` / `Path.with_segments()` fallback failures in `test.test_pathlib`
- when the CPython stdlib path is active, `os.path` now aliases the real platform path module (`posixpath` on POSIX), so `pathlib.Path.parser` matches CPython and Windows-only `pathlib` cases stop running on POSIX
- bootstrap `_frozen_importlib.ModuleSpec` now has the CPython constructor/property surface needed by fresh `importlib` bootstrap, so the source-importlib lane no longer dies on placeholder-spec objects before hitting real loader behavior
- meta-path loader execution now applies CPython `module_from_spec` semantics when `create_module()` returns `None` or a module object, registering the module before `exec_module()`, so `test.test_importlib` no longer falls back to `ModuleNotFoundError` at the first PEP 451 loader case
- builtin `__import__` now honors explicit `globals['__package__']` / `globals['__spec__']` package context for relative imports, so `test.test_importlib.import_.test___package__` moved past the earlier caller-frame resolution failure and is now down to later package-attribute semantics
- Rust-built package `ModuleSpec` instances now carry CPython-style `parent` semantics (`spec.parent == spec.name` for packages), which cleared the source-importlib `__package__` restoration failure for `test.test_importlib.import_.test___package__.Setting__package__PEP451`
- builtin `__import__` now performs CPython-style `fromlist` submodule handling, including propagating `ModuleNotFoundError` when `sys.modules['pkg.submod']` is explicitly blocked with `None`, which cleared `test.test_importlib.import_.test_api.*.test_blocked_fromlist`
- builtin `__import__` now raises `ValueError` instead of a generic runtime error for negative import levels, which cleared `test.test_importlib.import_.test_api.*.test_negative_level`
- builtin `__import__` now returns non-module `sys.modules[name]` cache entries directly for direct builtin calls, which cleared `test.test_importlib.import_.test_caching.*.test_using_cache`
- module initialization from `ModuleSpec` now preserves existing import-set attrs like `__path__` / `__package__` when they are already populated, while still refreshing `__spec__`, which cleared the source-importlib meta-path parent-path identity failure in `test.test_importlib.import_.test_meta_path.*.test_with_path`
- builtin `__import__` now re-checks `sys.modules[fullname]` after importing a parent for plain dotted imports, matching CPython’s side-effect handling when a non-package parent injects the child entry, which cleared `test.test_importlib.import_.test_packages.*.test_module_not_package_but_side_effects`
- `from email import policy` no longer leaks a partially initialized `email.policy` module during bootstrap import
- `test.test_email.test_pickleable` no longer overflows while materializing address headers (`header_store_parse` pyc name layout and `Message.__setitem__` operator dispatch fixed locally)

High-value modules after this checkpoint:

- `test.test_capi`
- `test.test_ctypes`
- `test.test_idle`
- `test.test_email`
- `test.test_datetime`

Closure evidence:

- a module moves from `load_error` into ordinary pass/fail execution
- the missing substrate is covered by targeted regressions
- the fix is in shared runtime code, not a one-off compatibility patch

### 3. Asyncio / OS / socket transport parity

Why third:

- biggest user-visible failed cluster among modules that already run
- now overlaps asyncio task/future coverage plus `os`, `socket`, `mailbox`,
  and related transport paths

Largest modules:

- `test.test_asyncio.test_tasks`: `850` non-pass
- `test.test_socket`: `734` non-pass
- `test.test_mailbox`: `347` non-pass
- `test.test_os`: `313` non-pass
- `test.test_asyncio.test_futures`: `129` non-pass
- `test.test_imaplib`: `109` non-pass

Current signatures:

- `requires the C _asyncio module`
- missing socket operations: `bind`, `connect`, `recvmsg`, `recvmsg_into`, `sendmsg`
- missing filesystem surface: `DirEntry.is_junction`
- filesystem API gaps: `os.fwalk`
- OS semantic mismatches around error mapping and directory lifecycle

Closure evidence:

- asyncio task/future runs stop failing on missing `_asyncio`
- `os-fs-socket` stops failing on missing primitive APIs
- failures shift from missing methods/constants to narrower semantic deltas
- path and directory-lifecycle semantics land with targeted regression tests

### 4. Object model / container / call / format parity

Why fourth:

- broadest cross-cutting runtime lane after concurrency/transport work
- fixes here fan out into import-time blockers and already-runnable modules
- `array` semantics now sit in this lane because the import blockers are gone
  and the remaining regressions are runtime-shape defects

Largest modules:

- `test.test_array`: `816` non-pass
- `test.test_enum`: `348` non-pass
- `test.test_unittest`: `343` non-pass
- `test.test_configparser`: `268` non-pass
- `test.test_traceback`: `219` non-pass
- `test.test_call`: `181` non-pass
- `test.test_functools`: `160` non-pass

Current signatures:

- `TypeError: subscript unsupported type`
- `RuntimeError: store subscript unsupported type`
- `AttributeError: module '__array__' has no attribute 'append'`
- `RuntimeError: string must be string`
- `AttributeError: str has no attribute 'format_map'`
- `SystemError: SystemError: NULL result without error in __get__()`
- enum recursion / repr / reverse-iteration mismatches
- call/vectorcall and descriptor mismatches in `_testcapi`-backed paths
- missing code-object debug/introspection support used by traceback/dis

Closure evidence:

- object-model/container regressions collapse across multiple unrelated modules
- fixes land in shared runtime substrate, not per-module patching
- constructor/call/container/format/introspection paths get direct regression tests

### 5. Text/codecs/XML substrate

Why fifth:

- smaller than the lanes above, but unusually concentrated
- a few substrate fixes can retire a large cluster quickly

Largest modules:

- `test.test_codecs`: `241` non-pass
- `test.test_xml_etree_c`: `140` non-pass
- `test.test_sax`: `136` non-pass
- `test.test_xml_etree`: `122` non-pass

Current signatures:

- `LookupError: unsupported encoding`
- missing codec helpers such as `ascii_encode`, `latin_1_encode`,
  `utf_16_encode`, and `utf_7_decode`
- XML parser API mismatches such as `ParserCreate(..., intern=...)`

Closure evidence:

- codec helper failures disappear from `test_codecs`-family runs
- XML failures move from API-surface gaps to narrower semantic deltas
- targeted regressions cover both substrate and stdlib-facing behavior

## Promote Above Raw Benchmark Count

These should stay near the front of the queue even when raw benchmark counts
are lower, because they already have dedicated implementation surfaces and
targeted validation:

- `json`
  - source: `src/vm/stdlib/json.rs`
  - evidence: `tests/vm.rs`, `tests/cpython_harness.rs`
- `_csv` / `csv`
  - source: `src/vm/stdlib/csv.rs`
  - evidence: `tests/vm.rs`, `tests/cpython_harness.rs`
- `_sre` / `re`
  - source: `src/vm/stdlib/re.rs`
  - evidence: `tests/vm.rs`, `tests/cpython_harness.rs`
- `pickle` / `pickletools` / `copyreg`
  - source: `src/vm/stdlib/pickle.rs`
  - evidence: `tests/vm.rs`, `tests/cpython_harness.rs`
- `_io`
  - source: `src/vm/builtins_io.rs`
  - evidence: `tests/vm.rs`, `tests/cpython_harness.rs`
- extension bridge / scientific stack
  - source: `src/vm/vm_extensions.rs`, `src/vm/vm_extensions/*`
  - evidence: `tests/extension_smoke.rs`, `scripts/probe_numpy_gate.py`

## Deprioritize Until Product-Facing Gaps Shrink

These rows can move the benchmark number, but they should usually trail the
product-facing runtime lanes above:

- `test.test_clinic`
- `test.test_capi`
- `_testinternalcapi`-specific gaps
- `_opcode` specialization/test-helper-only flags
- generic `implementation detail specific to cpython` failures with no clear
  user-facing runtime impact

## Focused Suite Cadence

Use `scripts/run_cpython_compat_focus.py` for short loops.

Recommended cadence:

- `smoke` after benchmark-runner or command-path changes
- one thematic suite per root-cause wave
- `high-leverage` after shared runtime fixes
- full benchmark only at checkpoint boundaries

Useful commands:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 scripts/run_cpython_compat_focus.py --list-suites
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 scripts/run_cpython_compat_focus.py --suite high-leverage
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 scripts/run_cpython_compat_focus.py --suite os-fs-socket --jobs 4
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 scripts/run_cpython_compat_focus.py --suite timeouts-crashes
```

The focused runner writes `selected_entries.txt` and `focus_request.json` into
its output directory and refuses to reuse a directory with a different request
unless `--force` is passed.
