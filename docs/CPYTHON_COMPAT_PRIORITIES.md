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

- generated at: `2026-03-07T12:20:56Z`
- git head: `112191cfb20934ae59a28efa2890d8780cb2878a`
- host: macOS `arm64`

Headline counts:

- discoverable benchmark entries: `492`
- runnable entries after inventory: `452`
- clean-pass modules: `38`
- modules that execute but still fail cases: `246`
- blocked modules (`load_error` + `process_error` + `process_timeout`): `138`
- discoverable test cases: `47,040`
- executed case outcomes: `19,793`
- passed case outcomes: `10,524`
- executed subtest outcomes: `40,662`
- passed subtest outcomes: `38,203`

The main leverage signal is still blocked execution coverage:

- discoverable cases currently trapped behind blocked modules: `26,439`

Recent closures since this snapshot:

- import/bootstrap substrate fixes have landed for missing `socket.IPPROTO_TCP`
  and related socket protocol constants
- `datetime.date.timetuple()` / `datetime.datetime.timetuple()` and
  `datetime.datetime.utctimetuple()` now exist for stdlib-facing callers
- `array._array_reconstructor` is now present and rebuilds arrays from
  CPython machine-format payloads in the builtin `array` module
- builtin `threading` / `_thread` now expose native-id surface, including
  `threading._HAVE_THREAD_NATIVE_ID`

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
| 1 | Import/bootstrap load errors | Largest blocked bucket: `79` modules, `16,320` discoverable cases | `high-leverage`, `import-bootstrap` |
| 2 | Process errors and timeouts | Stability failures still hide `10,119` discoverable cases | `timeouts-crashes`, `high-leverage` |
| 3 | OS/filesystem/socket transport parity | Largest user-visible failure cluster among runnable modules | `os-fs-socket` |
| 4 | Object model/call/descriptor/format parity | Broadest cross-cutting runtime lane after transport work | `object-model-call` |
| 5 | Text/codecs/XML substrate | Concentrated cluster with good focused payoff after broader runtime lanes move | `text-codecs-xml` |

## Lane Details

### 1. Import/bootstrap load errors

Why first:

- `79` runnable modules currently stop at `load_error`
- those rows hide `16,320` discoverable cases
- many failures are shared substrate defects rather than per-module bugs

Current signatures:

- `AttributeError: module 'socket' has no attribute 'IPPROTO_TCP'`
- `TypeError: object.__init__() takes exactly one argument`
- `AttributeError: 'datetime' object has no attribute 'timetuple'`
- `ImportError: cannot import name '_array_reconstructor' from 'array'`
- missing `concurrent.futures.InterpreterPoolExecutor`
- missing `threading._HAVE_THREAD_NATIVE_ID`

High-value modules:

- `test.test_email`
- `test.test_pathlib`
- `test.test_importlib`
- `test.test_datetime`
- `test.test_ast`
- `test.test_array`
- `test.test_asyncio.test_tasks`

Closure evidence:

- a module moves from `load_error` into ordinary pass/fail execution
- the missing substrate is covered by targeted regressions
- the fix is in shared runtime code, not a one-off compatibility patch

### 2. Process errors and timeouts

Why second:

- `34` modules currently end in `process_error`
- `25` modules currently end in `process_timeout`
- together they hide `10,119` discoverable cases

Largest blocked rows:

- timeouts:
  - `test.test_pickle`
  - `test.test_set`
  - `test.test_sqlite3`
  - `test.test_sys_settrace`
  - `test.test_threading`
- process errors:
  - `test.test_unittest`
  - `test.test_tarfile`
  - `test.test_io`
  - `test.test_statistics`

Observed patterns:

- hard crashes and negative return codes
- runaway recursion / stack overflow
- non-terminating behavior in pickle, set, sqlite3, tracing, and threading

Closure evidence:

- focused runs stop reporting `process_error` / `process_timeout`
- the module starts producing ordinary case-level failures
- targeted tests capture the old crash or hang trigger

### 3. OS/filesystem/socket transport parity

Why third:

- biggest user-visible failed cluster among modules that already run
- overlaps `os`, `pathlib`, `socket`, `subprocess`, `selectors`, and `asyncio`

Largest modules:

- `test.test_socket`: `736` non-pass
- `test.test_mailbox`: `347` non-pass
- `test.test_os`: `313` non-pass
- `test.test_imaplib`: `109` non-pass
- `test.test_selectors`: `99` non-pass

Current signatures:

- missing socket operations: `bind`, `connect`, `recvmsg`, `recvmsg_into`, `sendmsg`
- filesystem API gaps: `DirEntry.is_junction`, `os.fwalk`
- OS semantic mismatches around error mapping and directory lifecycle

Closure evidence:

- `os-fs-socket` stops failing on missing primitive APIs
- failures shift from missing methods/constants to narrower semantic deltas
- path and directory-lifecycle semantics land with targeted regression tests

### 4. Object model/call/descriptor/format parity

Why fourth:

- broadest cross-cutting runtime lane after OS/transport work
- fixes here fan out into import-time blockers and already-runnable modules

Largest modules:

- `test.test_enum`: `348` non-pass
- `test.test_configparser`: `268` non-pass
- `test.test_traceback`: `219` non-pass
- `test.test_call`: `181` non-pass
- `test.test_functools`: `164` non-pass
- `test.test_memoryview`: `119` non-pass

Current signatures:

- `RuntimeError: string must be string`
- `AttributeError: str has no attribute 'format_map'`
- `TypeError: object.__init__() takes exactly one argument`
- enum recursion / repr / reverse-iteration mismatches
- call/vectorcall mismatches in `_testcapi`-backed paths
- missing code-object debug/introspection support used by traceback/dis

Closure evidence:

- object-model-call regressions collapse across multiple unrelated modules
- fixes land in shared runtime substrate, not per-module patching
- constructor/call/format/introspection paths get direct regression tests

### 5. Text/codecs/XML substrate

Why fifth:

- smaller than the lanes above, but unusually concentrated
- a few substrate fixes can retire a large cluster quickly

Largest modules:

- `test.test_codecs`: `242` non-pass
- `test.test_xml_etree_c`: `141` non-pass
- `test.test_sax`: `136` non-pass
- `test.test_xml_etree`: `121` non-pass

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
