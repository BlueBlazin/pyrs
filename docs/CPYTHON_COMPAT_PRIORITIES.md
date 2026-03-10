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
- builtin `__import__('')` / `importlib.import_module('')` / `find_spec('')` now raise CPython-style `ValueError('Empty module name')`, which cleared `test.test_importlib.import_.test_relative_imports.*.test_empty_name_w_level_0`
- builtin `__import__` now returns non-module `sys.modules[name]` cache entries directly for direct builtin calls, which cleared `test.test_importlib.import_.test_caching.*.test_using_cache`
- module initialization from `ModuleSpec` now preserves existing import-set attrs like `__path__` / `__package__` when they are already populated, while still refreshing `__spec__`, which cleared the source-importlib meta-path parent-path identity failure in `test.test_importlib.import_.test_meta_path.*.test_with_path`
- builtin `__import__` now re-checks `sys.modules[fullname]` after importing a parent for plain dotted imports, matching CPython’s side-effect handling when a non-package parent injects the child entry, which cleared `test.test_importlib.import_.test_packages.*.test_module_not_package_but_side_effects`
- builtin `marshal.loads()` / `marshal.dumps()` now cover CPython marshal payloads plus the importlib module-code round-trip needed by `_bootstrap_external`, so the source-importlib lane no longer dies in `_code_to_timestamp_pyc` before real import semantics run
- builtin `os.stat(..., follow_symlinks=False)` now accepts the CPython keyword surface and `os.lstat()` / `stat_result.st_mode` preserve the actual file type instead of forcing symlink bits, which cleared the false-symlink `pathlib.copy(..., follow_symlinks=True)` failure shape and moved `test.test_pathlib` on to `bytes(Path(...))`
- bytes-like conversion now honors `__bytes__()` before fallback iteration and `os.fsencode()` / `os.fsdecode()` apply `os.fspath()` first, which cleared `bytes(Path(...))` and the `Path.with_name(b'...')` / `Path.with_suffix(b'...')` CPython TypeError lane in `test.test_pathlib`
- builtin `bytes.startswith()` / `bytes.endswith()` now raise CPython-style first-argument `TypeError`, and string `repr()` now switches quote style like CPython when that avoids escaping, which preserves the exact `KeyError.__str__()` surface needed by `test.test_importlib.import_.test_relative_imports.*.test_malicious_relative_import`
- builtin `__import__` now returns the parent object for relative dotted imports from raw `sys.modules` instead of re-importing the parent package, matching CPython’s malicious-cache behavior and clearing the prior `ModuleNotFoundError: module 'a' not found` failure in `test.test_importlib`
- builtin `os.symlink()` / `posix.symlink()` are now exposed with the CPython positional `target_is_directory` surface on Unix, which clears `Path.symlink_to()` fixture setup and moves `test.test_pathlib` on to deeper `posixpath.realpath()` / string-method parity instead of missing-substrate failures
- builtin `str` now exposes CPython-style bound and unbound `find()` / `rfind()` / `rindex()` descriptors, and `str.index()` / `str.rindex()` now raise `ValueError('substring not found')` for empty reversed slices instead of returning `-1`, which moved `test.test_pathlib` past the `str has no attribute 'rindex'` realpath failure and down to OS-error subclassing
- builtin `__import__` now treats omitted relative-import globals like CPython’s empty-globals path, raising `KeyError(\"'__name__' not in globals\")` when `__name__` is absent and `ImportError('attempted relative import with no known parent package')` when `__package__ == ''`, which cleared the remaining caller-frame leakage in `test.test_importlib.import_.test_api`
- builtin `os.scandir()` now maps `std::io::Error` through the CPython OSError subclass surface instead of raising generic `OSError`, which cleared the `PermissionError` mismatch in `test.test_pathlib` directory-copy permission cases
- builtin `os.stat()` / `os.lstat()` now populate CPython-style `stat_result.st_atime_ns` / `st_mtime_ns` / `st_ctime_ns` fields from the underlying OS metadata, and Unix `st_ctime` now comes from inode status-change time instead of file creation time, which moved `test.test_pathlib` past metadata-copy attribute gaps and down to `os.utime(ns=..., follow_symlinks=...)`
- builtin `os.utime()` now accepts the CPython `ns=` / `follow_symlinks=` keyword surface on Unix, advertises `os.utime` through `os.supports_follow_symlinks`, and uses path-based timestamp updates instead of file-descriptor writes, which cleared the `pathlib.copy(..., preserve_metadata=True)` keyword/dir-path blocker
- builtin `_operator` now mirrors the existing accelerated operator function surface instead of exposing only `_compare_digest`, so CPython `Lib/operator.py` replaces its pure-Python `add()` / `lt()` / etc. definitions with builtin callables and `test.test_pathlib` moved past the bogus `concat_path = operator.add` bound-method failure into deeper `glob` recursion-limit semantics
- CLI execution now runs inside a dedicated large-stack worker thread, and CLI-created VMs carry a CPython-sized stack-safe recursion ceiling instead of the default conservative `100`-frame clamp, which cleared the deep `Path.glob('../..../..')` recursion failure in `test.test_pathlib` and moved the module on to later teardown / `pwd` substrate issues
- builtin `os.walk()` now invokes its `onerror` callback with the synthesized `OSError` installed as the active exception context, so bare `raise` inside the callback re-raises the original `PermissionError` like CPython instead of failing with `RuntimeError: no active exception to reraise`; this moved `test.test_pathlib` past the `rmtree()` teardown callback failure and on to later `os.listdir()` / `pwd` substrate gaps
- builtin `os.listdir()` now maps `std::io::Error` through the CPython `OSError` subclass surface and fills the exception `filename` attribute from the requested path, which cleared the later `test.test_pathlib` cleanup failure and moved the module on to real device-type predicate semantics
- Unix `stat_result.st_mode` now preserves the full native `MetadataExt::mode()` bitfield instead of collapsing everything into regular-file / directory / symlink buckets, which cleared `Path.is_char_device()` and the related special-file predicate lane in `test.test_pathlib`
- pure path-string parsing now distinguishes CPython path-manipulation helpers from real filesystem calls: `posix._path_splitroot_ex` accepts embedded NUL in string inputs like CPython, so `Path('fileA\\x00').is_junction()` now returns `False` instead of raising `ValueError`; `test.test_pathlib` is down to the broader socket primitive gap (`socket.bind`)
- `_frozen_importlib_external` now exposes the CPython `_LoaderBasics`, `FileLoader`, `SourceLoader`, and `PathFinder.find_distributions()` surface needed by direct `importlib.abc` / `importlib.metadata` imports, so the importlib metadata lane is no longer blocked on missing loader-class bootstrap and is now down to `collections.defaultdict` constructor/subclass semantics during fixture discovery
- builtin `codecs` now exposes the CPython BOM aliases plus the buffered incremental/base helper surface needed by `encodings.*` modules (`BufferedIncrementalDecoder`, `BufferedIncrementalEncoder`, stream init helpers, and `getencoder` / `getdecoder` / `getreader` / `getwriter` wrappers), which clears the strict `json` BOM-detection crash and restores builtin `codecs.lookup('utf-8')` when `codecs` is already resident before stdlib `encodings` modules import
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
