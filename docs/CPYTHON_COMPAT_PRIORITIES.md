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

- completed at: `2026-03-10T12:40:12Z`
- git head: `1349056a0795ab231f7f91ccf5a9ebff0d7adc3e`
- host: macOS `arm64`

Headline counts:

- discoverable benchmark entries: `492`
- runnable entries after inventory: `452`
- clean-pass modules: `43`
- modules that execute but still fail cases: `252`
- blocked modules (`load_error` + `process_error` + `process_timeout`): `131`
- discoverable test cases: `47,040`
- executed case outcomes: `16,080`
- passed case outcomes: `8,477`
- executed subtest outcomes: `35,152`
- passed subtest outcomes: `32,656`

The main leverage signal is blocked execution coverage, and this snapshot is
timeout-heavy:

- discoverable cases currently trapped behind blocked modules: `30,342`
- timeout rows alone now hide `17,153` discoverable cases

Movement from the 2026-03-09 checked-in snapshot:

- clean-pass modules moved from `41` to `43`
- runnable entries after inventory moved from `454` to `452`
- modules that execute but still fail cases moved from `265` to `252`
- blocked modules moved from `118` to `131`
- discoverable cases hidden behind blocked modules moved from `23,051` to
  `30,342`
- executed case outcomes moved from `23,303` to `16,080`
- major drivers were new timeout-heavy regressions in
  `test.test_importlib`, `test.test_unittest`,
  `test.test_asyncio.test_tasks`, `test.test_array`, `test.test_socket`, and
  `test.test_io`, plus fresh load blockers in `test.test_argparse` and
  `test.test_decimal`

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
| 1 | Process timeouts and crashes | `process_timeout` + `process_error` now hide `21,267` discoverable cases, overwhelmingly more than any other bucket | `timeouts-crashes`, `high-leverage` |
| 2 | Import/bootstrap load errors | Remaining `load_error` rows still hide `9,075` discoverable cases | `high-leverage`, `import-bootstrap` |
| 3 | OS / pathlib / socket filesystem parity | Largest user-visible runnable cluster is now centered on `pathlib`, `os`, selectors, and socket-adjacent filesystem behavior | `high-leverage`, `os-fs-socket` |
| 4 | Call / descriptor / warnings / object model parity | Broadest shared runtime lane after blocked execution and filesystem transport work | `object-model-call` |
| 5 | Codecs / XML / annotation substrate | Concentrated bootstrap and runtime signature cluster with good focused payoff once the larger blockers move | `text-codecs-xml` |

## Lane Details

### 1. Process errors and timeouts

Why first:

- `34` modules currently end in `process_error`
- `52` modules currently end in `process_timeout`
- together they hide `21,267` discoverable cases

Largest blocked rows:

- timeouts:
  - `test.test_email`
  - `test.test_importlib`
  - `test.test_datetime`
  - `test.test_unittest`
  - `test.test_asyncio.test_tasks`
  - `test.test_pickle`
  - `test.test_array`
  - `test.test_socket`
  - `test.test_io`
  - `test.test_set`
- process errors:
  - `test.test_enum`
  - `test.test_tarfile`
  - `test.test_statistics`
  - `test.test_posix`
  - `test.test_str`
  - `test.test_itertools`

Observed patterns:

- `40` timeout rows currently end with no stderr at all, which points to
  event-loop or scheduler stalls instead of ordinary assertion failures
- other timeouts still surface destructor churn or unreaped-child warnings in
  asyncio, `_io`, subprocess, and XML cleanup paths
- process errors are dominated by stack overflows / panics (`-6`) and
  codec-path crashes (`-11`)

Closure evidence:

- focused runs stop reporting `process_error` / `process_timeout`
- the module starts producing ordinary case-level failures
- targeted tests capture the old crash or hang trigger

### 2. Import/bootstrap load errors

Why second:

- `45` runnable modules currently stop at `load_error`
- those rows hide `9,075` discoverable cases
- remaining failures are still shared parser/import/bootstrap substrate defects

Current snapshot signatures still to address:

- negative-complex literal import-time unary-minus is still wrong in
  `test.test_argparse` (`TypeError: unsupported operand type for -`)
- `test.test_decimal` still hits `SignalDict.keys()` mapping-surface gaps
- missing `codecs.latin_1_encode` still blocks multiprocessing-forkserver and
  related bootstrap lanes
- `_opcode.ENABLE_SPECIALIZATION*` flags are still missing for compile and
  monitoring imports
- parser / AST / inspection substrate gaps remain visible in `test.test_ast`,
  `test.test_grammar`, `test.test_dis`, and
  `test.test_inspect.test_inspect`

High-value modules after this checkpoint:

- `test.test_argparse`
- `test.test_capi`
- `test.test_decimal`
- `test.test_ctypes`
- `test.test_idle`
- `test.test_inspect.test_inspect`

Closure evidence:

- a module moves from `load_error` into ordinary pass/fail execution
- the missing substrate is covered by targeted regressions
- the fix is in shared runtime code, not a one-off compatibility patch

### 3. OS / pathlib / socket filesystem parity

Why third:

- biggest user-visible failed cluster among modules that already run now sits
  in `pathlib`, `os`, selectors, and socket-adjacent filesystem behavior
- `test.test_socket` itself has moved back into the timeout bucket, so address
  family and socket primitive work should stay tied to the filesystem lane

Largest modules:

- `test.test_pathlib`: `436` non-pass
- `test.test_os`: `253` non-pass
- `test.test_asyncio.test_futures`: `129` non-pass
- `test.test_imaplib`: `109` non-pass
- `test.test_selectors`: `99` non-pass
- `test.test_ftplib`: `93` non-pass

Current signatures:

- `RuntimeError: bind() address family is unsupported`
- `RuntimeError: utime() times must be a 2-sequence`
- `AttributeError: 'socket' object has no attribute 'setsockopt'`
- `TypeError: protocol must be int`
- platform-path handling is still too eager in `pathlib`
  (`requires Windows-flavoured path class`, Windows-only cases not being
  pruned early enough)

Closure evidence:

- `os-fs-socket` stops failing on missing or incorrectly typed primitive APIs
- `pathlib` moves from broad API-shape mismatches to narrower semantic deltas
- bind/setsockopt/address-family and `utime()` semantics land with targeted
  regression tests

### 4. Call / descriptor / warnings / object model parity

Why fourth:

- broadest cross-cutting runtime lane after blocked execution and filesystem
  transport work
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

### 5. Codecs / XML / annotation substrate

Why fifth:

- smaller than the lanes above, but still unusually concentrated
- fixes here affect XML parsers, source encoding, multiprocessing bootstrap,
  and annotation/AST exposure together

Largest modules:

- `test.test_sax`: `136` non-pass
- `test.test_minidom`: `82` non-pass
- `test.test_source_encoding`: `81` non-pass
- `test.test_annotationlib` is now runnable and exposes annotation/AST
  surface mismatches instead of import-time failure
- `test.test_xml_etree` / `test.test_xml_etree_c` are still noisy, but most of
  that lane currently shows up as subtest-level failures rather than headline
  case-level non-pass counts
- codec helper gaps still show up indirectly across import/bootstrap lanes even
  when `test.test_codecs` is not the headline module

Current signatures:

- missing codec helpers such as `latin_1_encode`
- XML parser API mismatches such as `xmlparser.intern`
- struct-format substrate still rejects `F` and `D`
- annotation / AST helper surfaces are still off
  (`function.__annotate__`, `Constant.kind`)

Closure evidence:

- codec helper failures disappear from bootstrap and runtime lanes
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
