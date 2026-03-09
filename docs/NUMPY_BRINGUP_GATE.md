# NumPy Bring-Up Gate

## Purpose

This document tracks the probe cases and current checked-in artifact for native
extension bring-up around NumPy and the local scientific stack.

Primary evidence:

- `scripts/probe_numpy_gate.py`
- `perf/numpy_gate_direct_latest.json`
- `tests/extension_smoke.rs`
- targeted NumPy regressions in `tests/vm.rs`

## Probe Cases

Required base cases:

1. `numpy_import`
   - snippet: `import numpy as np`
2. `numpy_ndarray_sum`
   - snippet:
     - `import numpy as np`
     - `a = np.array([1, 2, 3])`
     - `assert int(a.sum()) == 6`
3. `numpy_numerictypes_core`
   - snippet:
     - `import numpy._core.numerictypes as nt`
     - `assert nt.int8 is not None`
     - `assert nt.float64 is not None`
     - `assert nt.bool_ is not None`

Optional scientific-stack cases:

- `scipy_import`
- `pandas_import`
- `pandas_series_sum`
- `matplotlib_import`
- `matplotlib_pyplot_smoke`

## Commands

Base/direct-mode probe:

```bash
python3 scripts/probe_numpy_gate.py \
  --pyrs target/debug/pyrs \
  --cpython-lib .local/Python-3.14.3/Lib \
  --out perf/numpy_gate_latest.json \
  --timeout 20
```

Direct mode with local scientific stack:

```bash
python3 scripts/probe_numpy_gate.py \
  --pyrs target/debug/pyrs \
  --cpython-lib .local/Python-3.14.3/Lib \
  --include-scientific-stack \
  --probe-local-stack \
  --python-probe-bin .venv-ext314/bin/python \
  --out perf/numpy_gate_direct_latest.json \
  --timeout 30
```

Source-build probe:

```bash
python3 scripts/probe_numpy_gate.py \
  --pyrs target/debug/pyrs \
  --cpython-lib .local/Python-3.14.3/Lib \
  --numpy-src /path/to/numpy/source/tree \
  --python-build-bin python3 \
  --build-timeout 1800 \
  --out perf/numpy_gate_source_build_latest.json \
  --timeout 30
```

## Current Checked-In Artifact

Artifact: `perf/numpy_gate_direct_latest.json`

- timestamp: `2026-02-24T01:57:38Z`
- total cases: `8`
- passed: `4`
- failed: `4`
- skipped: `0`

Base NumPy gate status:

- `numpy_import`: `PASS`
- `numpy_ndarray_sum`: `PASS`
- `numpy_numerictypes_core`: `PASS`

Scientific-stack status:

- `scipy_import`: `PASS`
- `pandas_import`: `FAIL`
- `pandas_series_sum`: `FAIL`
- `matplotlib_import`: `FAIL`
- `matplotlib_pyplot_smoke`: `FAIL`

Failure shapes recorded in the artifact:

- pandas cases currently fail because `pyarrow` is not found during
  `pandas._libs.lib` initialization
- matplotlib cases currently fail with stack overflow

Local-module discovery in the same artifact confirms that `numpy`, `scipy`,
`pandas`, and `matplotlib` are all visible from the configured local
`site-packages`, so the current failures are beyond simple import-path wiring.

## Regression Anchors In Tests

Targeted NumPy runtime regressions currently include:

- `tests/vm.rs::numpy_random_default_rng_constructs_without_non_function_call_errors`
- `tests/vm.rs::numpy_random_default_rng_random_path_preserves_context_manager_specials`

Native-extension substrate coverage lives primarily in:

- `tests/extension_smoke.rs`

## Closure Rule

For the checked-in direct probe, this gate is only considered closed when all
of the following are true:

1. the three required NumPy base cases stay `PASS`
2. scientific-stack failures are either removed or replaced by narrower,
   well-understood missing-dependency failures that are explicitly visible in
   the probe artifact
3. targeted regressions exist for every root-cause fix that changes the direct
   extension runtime path
