# Dict Backend Migration Benchmark (Milestone 13)

This is a point-in-time snapshot and may be stale.
Refresh with `scripts/bench_dict_backend.sh` and update this file in the same commit.

## Environment

1. `pyrs` release binary: `target/release/pyrs`
2. CPython binary: `/Library/Frameworks/Python.framework/Versions/3.14/bin/python3`
3. CPython stdlib path: `/Users/$USER/Downloads/Python-3.14.3/Lib`

## Command

```bash
PREV_PICKLE_SEC=39.87 scripts/bench_dict_backend.sh
```

## Results

Source: `perf/dict_backend_bench.txt`

1. `pyrs_dict_microbench_sec=0.25`
2. `cpython_dict_microbench_sec=0.01`
3. `pyrs_pickle_hotspot_sec=5.24`
4. `cpython_pickle_hotspot_sec=0.44`
5. `pyrs_vs_cpython_pickle_ratio=11.9091`
6. `pickle_delta_vs_prev_sec=-34.6300`

## Interpretation

1. The new dict backend removed the previous major pickle bottleneck.
2. Pickle hotspot runtime improved from `39.87s` baseline to `5.24s` (`-34.63s`).
3. Significant gap to CPython remains and is now dominated by VM call/attribute and clone-heavy paths.

## Profiling Artifact

1. Post-migration flamegraph:
   `perf/pickle_delayed_writer_new_backend.svg`
