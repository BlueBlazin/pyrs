# Public Microbenchmarks

## Goal

Run a small, website-facing microbenchmark suite for:

- `pyrs`
- CPython `3.14.3`
- CPython `3.10.8`

This suite is intended for public comparison on a single host. It is not the
full CPython compatibility benchmark and it is not the repo's default local
validation lane.

## Benchmark Set

The checked-in manifest lives at:

- `benchmarks/public_micro/benchmarks.json`

The initial public suite contains these benchmark ids:

- `startup_pass`
- `startup_nosite`
- `function_call`
- `method_call`
- `attr_read`
- `int_loop`
- `list_build_iterate`
- `dict_get_set`
- `json_loads_dumps`
- `regex_match`

All benchmarks are intentionally version-neutral so the exact same workload can
run under `pyrs`, CPython `3.14.3`, and CPython `3.10.8`.

## Runner

Use:

- `scripts/run_public_micro_benchmarks.py`

The runner:

- validates that the CPython binaries are exactly `3.14.3` and `3.10.8`
- records raw samples plus summary stats for each interpreter
- emits one website-ready JSON artifact
- fingerprints the compared binaries so published results can be traced back to
  exact executables

The runner defaults to:

- `pyrs`: `target/release/pyrs`
- CPython `3.14.3`: `/Library/Frameworks/Python.framework/Versions/3.14/bin/python3`
- CPython `3.10.8`: `python3.10`

## Example Command

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/run_public_micro_benchmarks.py \
  --pyrs target/release/pyrs \
  --python314-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --python310-bin /path/to/python3.10.8/bin/python3.10 \
  --warmup 2 \
  --iterations 7 \
  --out perf/public_micro_latest.json
```

List the suite without running it:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/run_public_micro_benchmarks.py \
  --list
```

Run only a subset:

```bash
/Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  scripts/run_public_micro_benchmarks.py \
  --benchmark startup_pass \
  --benchmark function_call \
  --benchmark json_loads_dumps
```

## Artifact

Default artifact path:

- `perf/public_micro_latest.json`

Top-level structure:

- `schema_version`
- `benchmark_suite`
- `generated_at_utc`
- `git`
- `host`
- `config`
- `interpreters`
- `benchmarks`
- `summary`

Per benchmark row:

- benchmark id, name, category, and description
- raw `samples_seconds` for each interpreter
- `summary_seconds` (`min`, `max`, `mean`, `median`, `stddev`)
- `ratios_by_median`
- `winner_by_median`

## Website Guidance

Recommended presentation:

- primary table: benchmark, `pyrs`, CPython `3.14.3`, CPython `3.10.8`
- ratio columns based on median time
- one summary card for the geometric mean of median ratios
- a footnote that the suite is a focused microbenchmark run on one machine

When publishing results, include at least:

- CPU and OS from the artifact `host` block
- git commit from the artifact `git` block
- exact binary versions from the artifact `interpreters` block
