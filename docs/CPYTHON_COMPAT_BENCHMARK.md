# CPython Compatibility Benchmark

## Goal
Run a dedicated, product-facing CPython compatibility benchmark against `pyrs`
without coupling it to the frequent CI/dev probe lanes.

This benchmark is intended to answer:
- what CPython test entries are discoverable on this host,
- how many discoverable test cases exist on this host,
- how many case outcomes and subtest outcomes `pyrs` currently executes,
- where the current failures and crashes are.

## Scope
- Uses the local CPython 3.14 `Lib/test` suite as the source of truth.
- Keeps two top-level counts separate:
  - discoverable test cases,
  - executed outcome events (cases + subtests).
- Is not part of the default CI/dev validation cadence.

## Scripts
- Host orchestrator:
  - `scripts/run_cpython_compat_benchmark.py`
- Batched dispatcher:
  - `scripts/dispatch_cpython_compat_benchmark.py`
- In-interpreter worker:
  - `scripts/cpython_compat_benchmark_worker.py`
- Focused preset runner:
  - `scripts/run_cpython_compat_focus.py`

## Latest Artifact

- root: `perf/cpython_compat_benchmark_latest`
- completed at: `2026-03-09T05:36:36Z`
- git head: `775b50502268024d67fee75758348f1fdbff8a69`
- host: macOS `arm64`
- elapsed: `2,196.7s` (`36m 37s`)

## Example Run

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/run_cpython_compat_benchmark.py \
  --runner-bin target/release/pyrs \
  --cpython-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --cpython-lib .local/Python-3.14.3/Lib \
  --out-dir perf/cpython_compat_benchmark_latest \
  --jobs 0
```

For a dry subset:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/run_cpython_compat_benchmark.py \
  --runner-bin target/release/pyrs \
  --cpython-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --cpython-lib .local/Python-3.14.3/Lib \
  --entry test.test_augassign \
  --out-dir /tmp/cpython_compat_benchmark_smoke \
  --jobs 1 \
  --run-timeout 60
```

For a curated batch file:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/run_cpython_compat_benchmark.py \
  --runner-bin target/release/pyrs \
  --cpython-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --cpython-lib .local/Python-3.14.3/Lib \
  --entry-file perf/cpython_compat_batch1.txt \
  --allow-missing-entries \
  --out-dir /tmp/cpython_compat_benchmark_batch1 \
  --jobs 4 \
  --run-timeout 120
```

Entry files are newline-delimited module names. Blank lines and `#` comments are
ignored.

For focused benchmark slices instead of the current ~37-minute full sharded run on this host:

```bash
python3 scripts/run_cpython_compat_focus.py --list-suites
python3 scripts/run_cpython_compat_focus.py --suite high-leverage
python3 scripts/run_cpython_compat_focus.py --suite os-fs-socket --jobs 4
```

The focused runner wraps `scripts/run_cpython_compat_benchmark.py`, writes
`selected_entries.txt` plus `focus_request.json`, and refuses to reuse an
existing output directory with a different focused request unless `--force` is
passed.

For a full sharded run:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/dispatch_cpython_compat_benchmark.py \
  --runner-bin target/release/pyrs \
  --cpython-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --cpython-lib .local/Python-3.14.3/Lib \
  --entries-per-batch 25 \
  --out-dir perf/cpython_compat_benchmark_latest \
  --jobs 4 \
  --run-timeout 120
```

## Output Layout

The orchestrator or dispatcher writes a directory, not a single monolithic file:

- `manifest.json`
  - run metadata, discovery/selection provenance, timeout config, and completion state
- `plan.json`
  - dispatcher batch layout and per-batch entry files when using the sharded dispatcher
- `progress.json`
  - live phase/count/status snapshot for long-running or interrupted benchmark runs
- `summary.json`
  - top-level counts, host/git metadata, config, and entry index
- `derived_summary.json`
  - grouped failure signatures, top non-pass modules, and slowest cases/subtests
- `inventory/*.json`
  - host-CPython inventory shards per test entry
- `results/*.json`
  - `pyrs` execution shards per test entry
- `batches/*`
  - nested batch runs when using the sharded dispatcher

Each result shard includes:
- discoverable case ids for that entry,
- per-case outcome records,
- per-subtest outcome records,
- fixture/module-level error records,
- run timing and a tail of unittest runner output.

## Current Design Notes
- The worker disables CPython test resources by default (`test.support.use_resources = {}`),
  mirroring the resource-disabled default used in other probe lanes.
- CPython `libregrtest` process setup is treated as best-effort under `pyrs`.
  The benchmark worker keeps the minimal test-support defaults it needs even if
  the full CPython setup path relies on runtime features `pyrs` does not yet implement.
- JSON is written by the worker to a file path provided by the orchestrator so
  test stdout/stderr chatter does not corrupt result parsing.
- Explicit `--entry` / `--entry-file` selections are strict by default. If a
  requested entry is not discoverable on the current host, the run exits before
  starting. Use `--allow-missing-entries` when a curated batch may contain
  platform-specific rows; the unmatched names are then recorded in
  `manifest.json` and `summary.json`.
- The sharded dispatcher writes one nested orchestrator run per batch and then
  merges those batch summaries into one top-level `summary.json` /
  `derived_summary.json`, so the website can consume a single combined artifact.
- Completed benchmark output is reused only when the current `--runner-bin`
  fingerprint matches the existing artifact. Changing the `pyrs` binary
  automatically invalidates the cached output directory so stale results are not
  mixed across interpreter builds.
- If one batch hits an internal runner/orchestrator failure, the dispatcher now
  records that batch status and continues with later batches so the overall
  benchmark still produces a full top-level artifact for all attempted batches.
- A derived rollup can be generated after a run with:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/summarize_cpython_compat_benchmark.py \
  --benchmark-dir perf/cpython_compat_benchmark_latest
```
