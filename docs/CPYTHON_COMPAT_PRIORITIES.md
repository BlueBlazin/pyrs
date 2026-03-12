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

- completed at: `2026-03-12T06:50:30Z`
- git head: `613b9d546121ba07b7f52e9b663587492fdc9ed6`
- host: macOS `arm64`

Headline counts:

- discoverable benchmark entries: `492`
- runnable entries after inventory: `394`
- clean-pass modules: `45`
- modules that execute but still fail cases: `245`
- blocked modules (`load_error` + `process_error` + `process_timeout`): `83`
- discoverable test cases: `37,076`
- executed case outcomes: `19,651`
- passed case outcomes: `10,278`
- executed subtest outcomes: `46,034`
- passed subtest outcomes: `40,343`

Blocked execution is still the clearest leverage signal, but this rerun also
exposed broader inventory-stage fallout:

- discoverable cases currently trapped behind blocked modules: `16,005`
- timeout rows alone now hide `6,077` discoverable cases
- inventory-stage failures now stop `62` modules before they contribute
  runnable coverage

Movement from the 2026-03-10 checked-in snapshot:

- clean-pass modules moved from `43` to `45`
- runnable entries after inventory moved from `452` to `394`
- modules that execute but still fail cases moved from `252` to `245`
- blocked modules moved from `131` to `83`
- discoverable cases hidden behind blocked modules moved from `30,342` to
  `16,005`
- timeout-hidden discoverable cases moved from `17,153` to `6,077`
- executed case outcomes moved from `16,080` to `19,651`
- passed case outcomes moved from `8,477` to `10,278`
- executed subtest outcomes moved from `35,152` to `46,034`
- major drivers were fewer run-phase blockers overall, with more rows
  reaching ordinary case/subtest execution, but broad inventory-stage failures
  across asyncio-family modules reduced runnable-after-inventory coverage and
  the discoverable-case total

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
| 1 | Process blockers and inventory fallout | `process_timeout` + `process_error` still hide `11,738` discoverable cases, and `inventory_process_error` now suppresses `62` modules before run time | `timeouts-crashes`, `high-leverage` |
| 2 | Import/bootstrap load errors | Remaining `load_error` rows still hide `4,267` discoverable cases | `high-leverage`, `import-bootstrap` |
| 3 | Array / socket / datetime / OS parity | Largest user-visible runnable cluster now centers on `array`, `socket`, `datetime`, and `os` semantics | `high-leverage`, `os-fs-socket` |
| 4 | Call / descriptor / warnings / object model parity | Broadest shared runtime lane after blocked execution and protocol/substrate work | `object-model-call` |
| 5 | Codecs / XML / AST / annotation substrate | Concentrated signature cluster with good focused payoff once the larger blockers move | `text-codecs-xml` |

## Lane Details

### 1. Process blockers and inventory fallout

Why first:

- `32` modules currently end in `process_error`
- `25` modules currently end in `process_timeout`
- those run-phase blockers still hide `11,738` discoverable cases
- another `62` modules now fail during inventory before contributing runnable
  coverage

Largest blocked rows:

- timeouts:
  - `test.test_unittest`
  - `test.test_pickle`
  - `test.test_set`
  - `test.test_sqlite3`
  - `test.test_sys_settrace`
  - `test.test_zipfile`
  - `test.test_subprocess`
  - `test.test_bytes`
  - `test.test_threading`
  - `test.test_pdb`
- process errors:
  - `test.test_enum`
  - `test.test_tarfile`
  - `test.test_decimal`
  - `test.test_io`
  - `test.test_statistics`
  - `test.test_ordered_dict`
  - `test.test_posix`
  - `test.test_re`
  - `test.test_descr`
  - `test.test_str`
- inventory failures:
  - `test.test_asyncio.test_base_events`
  - `test.test_asyncio.test_events`
  - `test.test_asyncio.test_futures`
  - `test.test_asyncio.test_selector_events`

Observed patterns:

- inventory failures now suppress broad asyncio coverage before the benchmark
  reaches ordinary run-time case execution
- remaining timeouts cluster in `unittest`, `pickle`, `set`, `sqlite3`,
  tracing, zipfile, and subprocess-heavy rows rather than the older
  importlib/email-heavy set
- process errors are still concentrated in enum, tarfile, decimal, `_io`, and
  regex-adjacent runtime surfaces

Closure evidence:

- focused runs stop reporting `inventory_process_error`, `process_error`, and
  `process_timeout`
- the module starts producing ordinary case-level failures
- targeted tests capture the old crash, hang, or inventory bootstrap trigger

### 2. Import/bootstrap load errors

Why second:

- `26` runnable modules currently stop at `load_error`
- those rows hide `4,267` discoverable cases
- remaining failures are still shared parser/import/bootstrap and helper-surface
  defects

Current snapshot signatures still to address:

- `_testclinic` is still missing, which keeps `test.test_capi` and nearby
  helper-backed rows from importing cleanly
- `_testcapi` helper gaps remain visible in compile and monitoring-adjacent
  modules
- `multiprocessing.forkserver` still hits a parse error during fixture setup
- `_opcode.ENABLE_SPECIALIZATION*` flags are still missing for compile and
  monitoring imports
- parser / AST / inspection substrate gaps remain visible in
  `test.test_compile`, `test.test_dis`, `test.test_inspect.test_inspect`, and
  `test.test_pyrepl`

High-value modules after this checkpoint:

- `test.test_capi`
- `test.test_ctypes`
- `test.test_idle`
- `test.test_inspect.test_inspect`
- `test.test_pyrepl`
- `test.test_compile`
- `test.test_dis`

Closure evidence:

- a module moves from `load_error` into ordinary pass/fail execution
- the missing substrate is covered by targeted regressions
- the fix is in shared runtime code, not a one-off compatibility patch

### 3. Array / socket / datetime / OS parity

Why third:

- biggest user-visible failed cluster among modules that already run now sits
  in `array`, `socket`, `datetime`, and `os`-adjacent protocol behavior
- these failures map to shared buffer/socket/datetime substrate instead of
  isolated assertion deltas

Largest modules:

- `test.test_array`: `762` non-pass
- `test.test_socket`: `732` non-pass
- `test.test_datetime`: `640` non-pass
- `test.test_os`: `252` non-pass
- `test.test_imaplib`: `109` non-pass
- `test.test_selectors`: `99` non-pass
- `test.test_ftplib`: `93` non-pass

Current signatures:

- `RuntimeError: bind() address family is unsupported`
- `don't have recvmsg_into`
- `don't have recvmsg`
- `AttributeError: 'socket' object has no attribute 'connect'`
- `TypeError: expected bytes-like payload`
- `AttributeError: class 'datetime' has no attribute 'fromisoformat'`

Closure evidence:

- socket and bytes-like primitives stop failing on missing APIs or unsupported
  payload handling
- datetime moves from missing ISO helpers to narrower semantic deltas
- array, OS, and network-adjacent modules drop broad non-pass counts after
  shared substrate fixes

### 4. Call / descriptor / warnings / object model parity

Why fourth:

- broadest cross-cutting runtime lane after blocked execution and core protocol
  work
- fixes here fan out into warnings, call machinery, interpreters,
  `memoryview`, and process-pool behavior

Largest modules:

- `test.test_call`: `181` non-pass
- `test.test_functools`: `160` non-pass
- `test.test_warnings`: `132` non-pass
- `test.test_interpreters`: `121` non-pass
- `test.test_concurrent_futures.test_process_pool`: `115` non-pass
- `test.test_memoryview`: `113` non-pass

Current signatures:

- `SystemError: SystemError: NULL result without error in __get__()`
- `RuntimeError: module attribute '__warningregistry__' does not exist`
- `TypeError: object.__init__() takes exactly one argument`
- `requires the C _functools module`
- `NotImplementedError: subinterpreters are not implemented yet`
- vectorcall / `_testcapi` helper mismatches are still visible in subtest
  failure signatures

Closure evidence:

- call/descriptor/warnings regressions collapse across multiple unrelated
  modules
- fixes land in shared runtime substrate, not per-module patching
- constructor/call/descriptor/warning paths get direct regression tests

### 5. Codecs / XML / AST / annotation substrate

Why fifth:

- smaller than the lanes above, but still unusually concentrated
- fixes here affect source encoding, XML parsers, annotation/AST exposure, and
  codec-adjacent bootstrap paths together

Largest modules:

- `test.test_codecs`: `226` non-pass
- `test.test_ast`: `169` non-pass
- `test.test_xml_etree_c`: `127` non-pass
- `test.test_xml_etree`: `108` non-pass
- `test.test_source_encoding`: `90` non-pass
- `test.test_annotationlib` is runnable and still exposes annotation/AST
  surface mismatches instead of import-time failure

Current signatures:

- `AttributeError: code has no attribute '_varname_from_oparg'`
- `AssertionError: <bound method function.__annotate__> is not None`
- `RuntimeError: bad char in struct format: F`
- `RuntimeError: bad char in struct format: D`
- `ValueError: Element.remove(x): element not found`

Closure evidence:

- codec and AST helper failures disappear from bootstrap and runtime lanes
- XML failures move from API-surface gaps to narrower semantic deltas
- targeted regressions cover codec, XML, and annotation-facing behavior

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
