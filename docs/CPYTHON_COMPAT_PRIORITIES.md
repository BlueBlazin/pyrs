# CPython Compatibility Priorities

## Purpose
This document turns the checked-in CPython compatibility benchmark into a
working execution order for runtime + stdlib parity work.

It is not the canonical release tracker. Use:

- `docs/PRODUCTION_READINESS.md` for release blockers and closure criteria
- `docs/COMPATIBILITY.md` for subsystem status
- `docs/CPYTHON_COMPAT_BENCHMARK.md` for benchmark generation and artifact layout

The benchmark is most useful when multiple open parity lanes are plausible and
we need to choose the next highest-leverage root-cause wave.

## Current Snapshot

Source snapshot: `perf/cpython_compat_benchmark_latest`

- `summary.json`
- `derived_summary.json`

Snapshot metadata:

- Generated: `2026-03-07T12:20:56Z`
- Benchmark git head: `112191cfb20934ae59a28efa2890d8780cb2878a`
- Host: macOS `arm64`

Headline counts:

- Discoverable benchmark entries: `492`
- Runnable entries after inventory: `452`
- Clean-pass modules: `38`
- Modules that execute but still fail cases: `246`
- Blocked modules (`load_error` + `process_error` + `process_timeout`): `138`
- Discoverable test cases: `47,040`
- Executed case outcomes: `19,793`
- Passed case outcomes: `10,524`
- Executed subtest outcomes: `40,662`
- Passed subtest outcomes: `38,203`

The two benchmark signals that should drive sequencing are:

1. Only `22.4%` of discoverable cases currently pass (`10,524 / 47,040`).
2. `26,439` discoverable cases are still trapped behind module-level
   `load_error`, `process_error`, or `process_timeout`.

That second number is the main leverage source. Moving blocked modules into
normal pass/fail execution is usually worth more than shaving small failure
clusters inside already-runnable modules.

## Prioritization Rules

1. Fix root causes that convert blocked modules into runnable modules before
   chasing isolated assertion deltas.
2. If a benchmark lane overlaps an open P0 blocker in
   `docs/PRODUCTION_READINESS.md`, treat the P0 blocker as the primary closure
   target.
3. Prefer user-visible stdlib/runtime behavior over CPython-internal
   test-helper-only surfaces.
4. Use focused suites for local loops; use the full benchmark only at
   checkpoint boundaries.

## Current Execution Order

| Order | Lane | Why now | Primary focused suite(s) |
|---|---|---|---|
| 1 | Import/bootstrap load errors | Largest single blocked bucket: `79` modules, `16,320` discoverable cases | `high-leverage`, `import-bootstrap` |
| 2 | Process errors and timeouts | Stability failures still hide `10,119` discoverable cases and overlap active P0 blockers | `timeouts-crashes`, `high-leverage` |
| 3 | OS/filesystem/socket transport parity | Largest user-visible failure cluster among already-runnable modules | `os-fs-socket` |
| 4 | Object model/call/descriptor/format parity | Broadest cross-cutting runtime lane after transport/OS work | `object-model-call` |
| 5 | Text/codecs/XML substrate | Concentrated cluster with good focused payoff after broader runtime lanes move | `text-codecs-xml` |

## Lane Details

### 1. Import/bootstrap load errors

Why this is first:

- `79` runnable modules currently stop at `load_error`.
- Those rows alone hide `16,320` discoverable cases.
- Many of these failures are shared substrate problems rather than
  module-specific bugs.

Current signatures and examples:

- Missing socket constants/features:
  - `AttributeError: module 'socket' has no attribute 'IPPROTO_TCP'`
- Constructor/object-model mismatches during import-time class setup:
  - `TypeError: object.__init__() takes exactly one argument`
- Stdlib object-method gaps that break import-time initialization:
  - `AttributeError: 'datetime' object has no attribute 'timetuple'`
- Native substrate holes:
  - `ImportError: cannot import name '_array_reconstructor' from 'array'`
- Threading/concurrency bootstrap gaps:
  - missing `concurrent.futures.InterpreterPoolExecutor`
  - missing `threading._HAVE_THREAD_NATIVE_ID`

High-value modules in this lane:

- `test.test_email`
- `test.test_pathlib`
- `test.test_importlib`
- `test.test_datetime`
- `test.test_ast`
- `test.test_array`
- `test.test_asyncio.test_tasks`

Release-gate overlap:

- importlib/pkgutil/resources long-tail parity
- descriptor/object-model long-tail parity
- asyncio/subprocess/threading bootstrap behavior

Closure evidence:

- the module moves from `load_error` into normal pass/fail execution
- the underlying missing substrate is covered by targeted regression tests
- the same fix does not rely on a local compatibility shim that diverges from
  CPython

### 2. Process errors and timeouts

Why this is second:

- `34` modules currently end in `process_error`.
- `25` modules currently end in `process_timeout`.
- Together they hide `10,119` discoverable cases and prevent trustworthy
  semantic debugging in those areas.

Largest blocked rows:

- Timeouts:
  - `test.test_pickle`
  - `test.test_set`
  - `test.test_sqlite3`
  - `test.test_sys_settrace`
  - `test.test_threading`
- Process errors:
  - `test.test_unittest`
  - `test.test_tarfile`
  - `test.test_io`
  - `test.test_statistics`

Observed patterns:

- hard crashes / negative return codes in codec, tar, and statistics paths
- stack overflow or runaway recursion in copy/exception-group style paths
- non-terminating behavior in pickle, set, sqlite3, tracing, and threading

Release-gate overlap:

- `pickle` / `pickletools` / `copyreg`
- `_io` behavioral parity
- hash-container closure (`set`)
- threading / multiprocessing / sqlite3 stability

Closure evidence:

- focused runs no longer report `process_error` or `process_timeout`
- the affected modules produce ordinary case-level pass/fail results
- targeted tests capture the prior crash or hang trigger

### 3. OS/filesystem/socket transport parity

Why this is third:

- It is the largest user-visible failed cluster among modules that already run.
- The same substrate affects `os`, `pathlib`, `socket`, `subprocess`,
  `selectors`, and `asyncio`.
- These are common-usage stdlib surfaces, not benchmark-only helpers.

Largest modules in this lane:

- `test.test_socket`: `736` non-pass
- `test.test_mailbox`: `347` non-pass
- `test.test_os`: `313` non-pass
- `test.test_imaplib`: `109` non-pass
- `test.test_selectors`: `99` non-pass

Current signatures:

- missing socket operations:
  - `bind`
  - `connect`
  - `recvmsg`
  - `recvmsg_into`
  - `sendmsg`
- filesystem API gaps:
  - `DirEntry.is_junction`
  - `os.fwalk`
- OS semantic mismatches:
  - file creation/removal error mapping
  - directory lifecycle semantics

Release-gate overlap:

- top-stdlib baseline credibility for `os`, `pathlib`, `subprocess`, `asyncio`
- import/bootstrap failures that depend on socket and filesystem substrate

Closure evidence:

- `os-fs-socket` stops failing on missing primitive APIs
- `socket`/`selectors`/`asyncio` failures shift from missing methods/constants
  to narrower semantic deltas
- path and directory-lifecycle behavior is covered by targeted regression tests

### 4. Object model/call/descriptor/format parity

Why this is fourth:

- This is the broadest cross-cutting runtime bucket after OS/transport work.
- Fixes here fan out into import-time blockers and already-runnable modules.
- It directly overlaps the open object-model long tail in
  `docs/COMPATIBILITY.md`.

Largest modules in this lane:

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

Release-gate overlap:

- descriptor / metaclass / slots long-tail parity
- constructor and call-protocol correctness used by stdlib bootstrap
- traceback / inspect / enum / dataclass follow-on behavior

Closure evidence:

- object-model-call regressions collapse across multiple unrelated modules
- fixes are implemented in shared runtime substrate, not per-module patching
- targeted tests cover the relevant constructor/call/format/introspection paths

### 5. Text/codecs/XML substrate

Why this is fifth:

- It is smaller than the lanes above but unusually concentrated.
- A few substrate fixes can retire a large cluster quickly.
- It is valuable once the higher-leverage bootstrap/runtime lanes stop hiding
  broader failures.

Largest modules in this lane:

- `test.test_codecs`: `242` non-pass
- `test.test_xml_etree_c`: `141` non-pass
- `test.test_sax`: `136` non-pass
- `test.test_xml_etree`: `121` non-pass

Current signatures:

- `LookupError: unsupported encoding`
- missing `codecs` helpers:
  - `ascii_encode`
  - `latin_1_encode`
  - `utf_16_encode`
  - `utf_7_decode`
- XML parser API mismatches:
  - `ParserCreate(..., intern=...)`
  - element removal / namespace canonicalization differences

Closure evidence:

- codec helper failures disappear from `test_codecs`-family runs
- XML parser failures move from API-surface gaps to narrower semantic deltas
- targeted regressions cover both the native substrate and the stdlib-facing
  behavior

## Promote Above Raw Benchmark Count

These areas should be promoted even when their benchmark row counts are not at
the top of the table:

- `json`
- `_csv` / `csv`
- `_sre`
- `pickle` / `pickletools` / `copyreg`
- `_io`
- hash-container semantic/perf closure

Reason: these are explicit release gates in `docs/PRODUCTION_READINESS.md`.
When a fix overlaps one of these blockers and a benchmark lane, use the blocker
module as the primary closure target and use the benchmark lane as supporting
evidence.

## Deprioritize Until Product-Facing Gaps Shrink

These rows can move the raw benchmark number, but they usually should not lead
the queue unless they block shared shipped behavior:

- `test.test_clinic`
- `test.test_capi`
- `_testinternalcapi`-specific gaps
- `_opcode` specialization/test-helper-only flags
- generic `implementation detail specific to cpython` cases with no direct
  runtime product impact

## Focused Suite Cadence

Use `scripts/run_cpython_compat_focus.py` instead of hand-building
`--entry-file` lists for routine loops.

Recommended cadence:

- Use `smoke` first when changing benchmark plumbing or runner invocation.
- Use one thematic suite per root-cause wave until the main failure signature
  moves.
- Re-run `high-leverage` after cross-cutting fixes to confirm broader impact.
- Re-run the full benchmark only at checkpoint boundaries after focused suites
  show material movement.

Useful commands:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 scripts/run_cpython_compat_focus.py --list-suites
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 scripts/run_cpython_compat_focus.py --suite high-leverage
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 scripts/run_cpython_compat_focus.py --suite os-fs-socket --jobs 4
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 scripts/run_cpython_compat_focus.py --suite timeouts-crashes
```

The focused runner writes `selected_entries.txt` and `focus_request.json` into
its output directory and refuses to reuse a directory with a different request
unless `--force` is passed. That keeps focused artifacts from being mixed across
different slices.
