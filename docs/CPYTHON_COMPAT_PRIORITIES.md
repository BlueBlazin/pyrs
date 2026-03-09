# CPython Compatibility Priorities

This document turns the full CPython compatibility benchmark into an execution
order. The source snapshot is the checked-in artifact under
`perf/cpython_compat_benchmark_latest`:

- `summary.json`
- `derived_summary.json`

Snapshot metadata:

- Generated: `2026-03-07T12:20:56Z`
- Benchmark git head: `112191cfb20934ae59a28efa2890d8780cb2878a`
- Host: macOS `arm64`

## Headline Readout

- Discoverable benchmark entries: `492`
- Runnable entries after inventory: `452`
- Discoverable test cases: `47,040`
- Executed case outcomes: `19,793`
- Passed case outcomes: `10,524`
- Executed subtest outcomes: `40,662`
- Passed subtest outcomes: `38,203`

Two numbers matter most for prioritization:

1. Only `22.4%` of discoverable test cases currently pass (`10,524 / 47,040`).
2. `26,439` discoverable cases are trapped behind `load_error`,
   `process_error`, or `process_timeout` before they can contribute meaningful
   case results at all.

That second number is the highest-leverage signal in the whole benchmark.

## Priority Order

### 1. Make blocked modules execute at all

This is the biggest benchmark lever.

- `79` runnable modules end in `load_error` and cover `16,320` discoverable
  cases.
- `34` modules end in `process_error` and cover `5,006` discoverable cases.
- `25` modules end in `process_timeout` and cover `5,113` discoverable cases.

Shared root causes visible in the checked-in results:

- Missing socket constants/features block large asyncio surfaces.
  - `AttributeError: module 'socket' has no attribute 'IPPROTO_TCP'`
  - Hits `19` modules and blocks `2,444` discoverable cases.
- Constructor/object-model mismatches prevent import-time class setup.
  - `TypeError: object.__init__() takes exactly one argument`
  - Blocks `test.test_datetime`, `test.test_ast`, and `test.test_strptime`
    for `1,370` cases.
- Stdlib object-method gaps still stop imports early.
  - `AttributeError: 'datetime' object has no attribute 'timetuple'`
    blocks `test.test_email` alone for `1,781` cases.
  - `ImportError: cannot import name '_array_reconstructor' from 'array'`
    blocks `test.test_array` for `890` cases.
- Threading/concurrency surface holes fan out into multiprocessing and futures.
  - Missing `concurrent.futures.InterpreterPoolExecutor`
  - Missing `threading._HAVE_THREAD_NATIVE_ID`

Recommended local suite:

- `scripts/run_cpython_compat_focus.py --suite high-leverage`
- Current coverage from the checked-in benchmark: `15` modules, `11,638`
  discoverable cases.

### 2. OS, filesystem, socket, and transport parity

This is the largest user-visible failed cluster among modules that already run.

Largest non-pass modules in this area:

- `test.test_socket`: `736`
- `test.test_mailbox`: `347`
- `test.test_os`: `313`
- `test.test_imaplib`: `109`
- `test.test_selectors`: `99`

Shared failure signatures:

- Missing socket operations:
  - `bind`
  - `connect`
  - `recvmsg`
  - `recvmsg_into`
  - `sendmsg`
- Filesystem API gaps:
  - `DirEntry.is_junction`
  - `os.fwalk`
- OS semantic mismatches:
  - file creation/removal error mapping
  - directory lifecycle semantics

Why this is high ROI:

- It affects core stdlib surfaces, not just benchmark-only tests.
- It also overlaps the blocked asyncio/import bootstrap cases above.

Recommended local suite:

- `scripts/run_cpython_compat_focus.py --suite os-fs-socket`
- Current coverage: `15` modules, `5,579` discoverable cases.

### 3. Core object model, call protocol, and string/format semantics

This is the broadest cross-cutting runtime bucket after OS/socket work.

Largest modules and signatures here:

- `test.test_enum`: `348` non-pass
- `test.test_configparser`: `268` non-pass
- `test.test_traceback`: `219` non-pass
- `test.test_call`: `181` non-pass
- `test.test_functools`: `164` non-pass
- `test.test_memoryview`: `119` non-pass

Shared failure signatures:

- `RuntimeError: string must be string`
- `AttributeError: str has no attribute 'format_map'`
- `TypeError: object.__init__() takes exactly one argument`
- enum recursion / repr / reverse-iteration mismatches
- call/vectorcall mismatches in `_testcapi`-backed paths
- missing code-object debug/introspection support used by traceback/dis

Why this is high ROI:

- These are runtime semantics that fan out into many unrelated stdlib modules.
- Fixes here tend to convert both import-time blockers and already-running
  failures.

Recommended local suite:

- `scripts/run_cpython_compat_focus.py --suite object-model-call`
- Current coverage: `15` modules, `8,716` discoverable cases.

### 4. Text, codecs, and XML substrate

This is a smaller cluster than object-model or OS work, but it is unusually
concentrated and therefore a good focused closure lane.

Largest modules and signatures:

- `test.test_codecs`: `242` non-pass
- `test.test_xml_etree_c`: `141` non-pass
- `test.test_sax`: `136` non-pass
- `test.test_xml_etree`: `121` non-pass

Shared failure signatures:

- `LookupError: unsupported encoding`
- missing `codecs` helpers such as:
  - `ascii_encode`
  - `latin_1_encode`
  - `utf_16_encode`
  - `utf_7_decode`
- XML parser API mismatches:
  - `ParserCreate(..., intern=...)`
  - element removal / namespace canonicalization differences

Why this is high ROI:

- The failures are clustered enough that a single root-cause fix can retire many
  benchmark rows quickly.
- It directly improves stdlib serialization/parsing credibility.

Recommended local suite:

- `scripts/run_cpython_compat_focus.py --suite text-codecs-xml`
- Current coverage: `9` modules, `1,331` discoverable cases.

### 5. Crash and timeout elimination

This is the stability-first lane. It may not always yield immediate pass-rate
wins, but it unlocks execution coverage and removes blind spots.

Largest blocked modules here:

- Timeouts:
  - `test.test_pickle`: `929`
  - `test.test_set`: `630`
  - `test.test_sqlite3`: `501`
  - `test.test_sys_settrace`: `448`
  - `test.test_threading`: `228`
- Process errors:
  - `test.test_unittest`: `1,089`
  - `test.test_tarfile`: `738`
  - `test.test_io`: `667`
  - `test.test_statistics`: `400`

Observed patterns:

- hard crashes / negative return codes in codec/tar/statistics areas
- stack overflow in copy/exception-group style recursion paths
- long-running or non-terminating behavior in pickle/set/sqlite3/tracing/threading

Why this is high ROI:

- Every crash/timeout removed expands the set of modules that can produce normal
  case-level pass/fail data.
- This is prerequisite work before trusting smaller semantic deltas in those
  areas.

Recommended local suite:

- `scripts/run_cpython_compat_focus.py --suite timeouts-crashes`
- Current coverage: `15` modules, `6,834` discoverable cases.

## Secondary Score Movers

Some high-count benchmark failures are real, but they should usually trail the
work above unless they unblock stdlib/runtime behavior directly:

- `test.test_clinic`
- `test.test_capi`
- `_testinternalcapi`-specific gaps
- `_opcode` specialization flags

These improve benchmark numbers, but a large fraction of the surface is
CPython-internal or test-helper-specific rather than user-visible runtime
behavior.

## Focused Benchmark Suites

Use `scripts/run_cpython_compat_focus.py` instead of hand-building
`--entry-file` lists for every loop.

Available benchmark-oriented presets:

- `smoke`: command-path verification before larger runs
- `high-leverage`: biggest current headline movers
- `import-bootstrap`: import/load blockers
- `os-fs-socket`: filesystem, socket, selectors, subprocess, asyncio transport
- `object-model-call`: object model, call protocol, formatting, traceback
- `text-codecs-xml`: codecs and XML stack
- `timeouts-crashes`: stability-first lane

Useful commands:

```bash
python3 scripts/run_cpython_compat_focus.py --list-suites
python3 scripts/run_cpython_compat_focus.py --suite high-leverage
python3 scripts/run_cpython_compat_focus.py --suite os-fs-socket --jobs 4
python3 scripts/run_cpython_compat_focus.py --suite text-codecs-xml --inventory-only
```

The focused runner writes `selected_entries.txt` and `focus_request.json` into
the output directory and refuses to reuse a directory with a different focused
request unless `--force` is passed. That avoids mixing cached artifacts from
different focused slices.

## Recommended Cadence

- Use `smoke` first when changing the runner or benchmark plumbing.
- Use one thematic suite per root-cause wave until the underlying failure
  pattern is closed.
- Re-run the full `46` minute dispatch only at checkpoint boundaries, after one
  or more focus suites show real movement.
