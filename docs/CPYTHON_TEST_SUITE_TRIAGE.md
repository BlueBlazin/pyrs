# CPython Test Suite Triage

Purpose: turn the CPython compatibility benchmark into an engineering work queue
for closing the full discoverable CPython 3.14 test suite on this host.

This is a product-facing benchmark lane, but the artifact is also the right
input for a systematic interpreter-fix loop.

## Current Baseline

Current full artifact:
- `perf/cpython_compat_benchmark_latest/summary.json`
- `perf/cpython_compat_benchmark_latest/derived_summary.json`

Current full release-binary baseline on this host (artifact generated March 7,
2026 against runner commit `112191cf`):
- discoverable test cases: `47,040`
- executed case outcomes: `19,793`
- executed subtest outcomes: `40,662`
- module statuses:
  - `passed=38`
  - `failed=246`
  - `load_error=79`
  - `process_error=34`
  - `process_timeout=25`
  - `skipped=15`
  - `host_skip=15`

Missing case execution is currently dominated by modules that never reach test
case execution at all:
- `16,320` blocked in `load_error`
- `5,006` blocked in `process_error`
- `5,113` blocked in `process_timeout`
- `431` blocked in `host_skip`

That is `26,870 / 27,247` missing cases, about `98.6%`, blocked before any
test case method executes.

## What The Counts Mean

- `discoverable test cases`
  - Cases discoverable from the host CPython 3.14 test suite on this platform.
  - This is the denominator for product-facing compatibility.
- `executed case outcomes`
  - Cases that actually emitted a case-level result under `pyrs`.
- `executed subtest outcomes`
  - Runtime subtest events emitted under `pyrs`.

Important: discoverable and executed are intentionally different.

If a module:
- fails during import/discovery,
- crashes the interpreter process,
- hangs until timeout,
- or dies in `setUpModule` before any test method runs,

then those cases remain in the discoverable denominator but do not count as
executed case outcomes.

## Status Meanings

- `load_error`
  - The module/suite does not load cleanly under `pyrs`.
  - Typical causes: missing stdlib/runtime surface, import-time semantic drift,
    loader errors in nested test packages.
  - Highest-priority class today because it blocks the most cases.
- `process_error`
  - The `pyrs` subprocess crashed or aborted while running the module.
  - Treat as interpreter bugs first, not test bugs.
- `process_timeout`
  - The module hung until the per-entry timeout.
  - Usually deadlock, infinite loop, or pathological slow path.
- `failed`
  - The module ran and emitted case, subtest, or fixture failures.
  - Some `failed` modules still execute zero cases because `setUpModule` failed.
- `skipped`
  - The module executed but only produced skips.
- `host_skip`
  - The module is discoverable on host CPython but intentionally skips under
    `pyrs` because a required feature/module is unavailable.

## Priority Order

Work in this order unless a user-visible regression changes the priority:

1. `load_error` modules by blocked-case count.
2. `failed` modules with `tests_run = 0` because they are operationally the same
   as load blockers.
3. `process_error` modules by blocked-case count.
4. `process_timeout` modules by blocked-case count.
5. High-fanout runtime failures among modules that already execute cases.

Rationale:
- unblocking load/crash/timeout modules unlocks whole case families at once,
- shared substrate fixes usually collapse many modules together,
- chasing low-count leaf failures too early burns time without moving the
  compatibility denominator much.

## Current Highest-Impact Queues

Top `load_error` modules by blocked discoverable cases:
- `1781` `test.test_email`
- `1369` `test.test_pathlib`
- `1345` `test.test_importlib`
- `1105` `test.test_capi`
- `1098` `test.test_datetime`
- `1036` `test.test_asyncio.test_tasks`
- `890` `test.test_array`
- `612` `test.test_ctypes`
- `511` `test.test_idle`
- `379` `test.test_zipfile`

Top `process_error` modules:
- `1089` `test.test_unittest`
- `738` `test.test_tarfile`
- `667` `test.test_io`
- `400` `test.test_statistics`
- `274` `test.test_dataclasses`

Top `process_timeout` modules:
- `929` `test.test_pickle`
- `630` `test.test_set`
- `501` `test.test_sqlite3`
- `448` `test.test_sys_settrace`
- `353` `test.test_subprocess`

Top executed-but-nonpassing modules after blockers:
- `test.test_socket`
- `test.test_enum`
- `test.test_mailbox`
- `test.test_os`
- `test.test_configparser`

## Triage Heuristics

Prefer shared root-cause closure over test-by-test churn.

Good targets:
- missing foundational stdlib/runtime methods that appear in import-time paths,
- object-model and descriptor semantics,
- import machinery and package loading,
- core builtins used during module initialization,
- interpreter crashes and hangs,
- fixture/setup blockers that prevent any case execution.

Bad targets:
- one-off leaf assertions in a low-count module while bigger loaders still fail,
- test-only shims that diverge from CPython architecture,
- ad hoc skips or per-test patches to make the benchmark greener.

Always verify against CPython 3.14 semantics first:
- local CPython source root: `.local/Python-3.14.3`
- local CPython stdlib root: `.local/Python-3.14.3/Lib`

## Development Loop

Always use the release interpreter for this lane:
- `target/release/pyrs`

Fastest single-module loop:

```bash
target/release/pyrs -S scripts/cpython_compat_benchmark_worker.py \
  --mode run \
  --module test.test_email \
  --sys-path .local/Python-3.14.3/Lib
```

This is the fastest way to see:
- `status`
- `load_state.error.detail`
- `runner_output_tail`
- `tests_run`
- fixture records

If you also want the host-CPython discoverable inventory for the same module:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 -S \
  scripts/cpython_compat_benchmark_worker.py \
  --mode inventory \
  --module test.test_email \
  --sys-path .local/Python-3.14.3/Lib
```

If you want the benchmark-style inventory + result shards for one module:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/run_cpython_compat_benchmark.py \
  --runner-bin target/release/pyrs \
  --cpython-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --cpython-lib .local/Python-3.14.3/Lib \
  --entry test.test_email \
  --out-dir /tmp/cpython_debug_test_email \
  --jobs 1 \
  --force
```

Recommended fix loop for one module:

1. Pick the highest blocked-case module in the current queue.
2. Run the direct worker under `pyrs`.
3. Fix the earliest import/load/setup failure by matching CPython 3.14 behavior.
4. Rerun the worker until the module stops being `load_error` or zero-test `failed`.
5. Once cases start executing, use the single-entry orchestrator run for richer
   case/subtest output.
6. Add targeted regression tests for the root cause.
7. Move to the next module in the same subsystem cluster before rerunning the
   full benchmark.

## Batch Rechecks

After closing a shared subsystem issue, rerun a related batch instead of the
entire suite.

Generate the current `load_error` queue from the latest artifact:

```bash
python3 - <<'PY' > /tmp/cpython_load_errors.txt
import json
with open("perf/cpython_compat_benchmark_latest/summary.json") as f:
    summary = json.load(f)
for entry in summary["entries"]:
    if entry.get("run_status") == "load_error":
        print(entry["module"])
PY
```

Rerun just the current load-error modules:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/run_cpython_compat_benchmark.py \
  --runner-bin target/release/pyrs \
  --cpython-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --cpython-lib .local/Python-3.14.3/Lib \
  --entry-file /tmp/cpython_load_errors.txt \
  --out-dir /tmp/cpython_load_errors_recheck \
  --jobs 4 \
  --force
```

Do the same for `process_error` and `process_timeout` after those become the
active lane.

## Practical Notes

- Treat `load_error` and `failed` with `tests_run = 0` as one blocker class.
- Keep fixes grouped by subsystem:
  - importlib/import/package loading,
  - datetime/email/timezone,
  - array/ctypes/C-API substrate,
  - asyncio/task/runtime scheduling,
  - GC/finalization/setup semantics,
  - socket/os/pathlib/filesystem semantics.
- Use the benchmark artifact to choose where to work; use targeted tests and the
  direct worker loop to actually develop the fix.
- Only rerun the full sharded benchmark at meaningful checkpoints.

## Relevant Files

- `docs/CPYTHON_COMPAT_BENCHMARK.md`
- `scripts/cpython_compat_benchmark_worker.py`
- `scripts/run_cpython_compat_benchmark.py`
- `scripts/dispatch_cpython_compat_benchmark.py`
- `perf/cpython_compat_benchmark_latest/summary.json`
- `perf/cpython_compat_benchmark_latest/derived_summary.json`
