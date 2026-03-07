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
- In-interpreter worker:
  - `scripts/cpython_compat_benchmark_worker.py`

## Example Run

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/run_cpython_compat_benchmark.py \
  --runner-bin target/debug/pyrs \
  --cpython-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --cpython-lib .local/Python-3.14.3/Lib \
  --out-dir perf/cpython_compat_benchmark_latest \
  --jobs 0
```

For a dry subset:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/run_cpython_compat_benchmark.py \
  --runner-bin target/debug/pyrs \
  --cpython-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --cpython-lib .local/Python-3.14.3/Lib \
  --entry test.test_augassign \
  --out-dir /tmp/cpython_compat_benchmark_smoke \
  --jobs 1 \
  --run-timeout 60
```

## Output Layout

The orchestrator writes a directory, not a single monolithic file:

- `manifest.json`
  - run metadata, selected entries, timeout config, and completion state
- `summary.json`
  - top-level counts, host/git metadata, config, and entry index
- `inventory/*.json`
  - host-CPython inventory shards per test entry
- `results/*.json`
  - `pyrs` execution shards per test entry

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
