# NumPy Bring-Up Gate

Status: active (Milestone 15).

Purpose: track direct native-extension execution gates for NumPy and scientific-stack bring-up.

Execution model: see `docs/CAPI_PLAN.md`.
- Lane A: Stable ABI (`abi3`) closure.
- Lane B: non-`abi3` CPython C-API/runtime surfaces required by real scientific-stack wheels.

## Gate Definitions

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

Optional scientific-stack probe cases (`--include-scientific-stack`):
- `scipy_import`
- `pandas_import`
- `pandas_series_sum`
- `matplotlib_import`
- `matplotlib_pyplot_smoke`

## Probe Commands

Base gate:

```bash
python3 scripts/probe_numpy_gate.py \
  --pyrs target/debug/pyrs \
  --cpython-lib /Users/$USER/Downloads/Python-3.14.3/Lib \
  --out perf/numpy_gate_latest.json \
  --timeout 20
```

Direct-mode gate + scientific stack:

```bash
python3 scripts/probe_numpy_gate.py \
  --pyrs target/debug/pyrs \
  --cpython-lib /Users/$USER/Downloads/Python-3.14.3/Lib \
  --include-scientific-stack \
  --probe-local-stack \
  --python-probe-bin .venv-ext314/bin/python \
  --out perf/numpy_gate_direct_latest.json \
  --timeout 30
```

Local site-packages probe mode:

```bash
python3 scripts/probe_numpy_gate.py \
  --pyrs target/debug/pyrs \
  --cpython-lib /Users/$USER/Downloads/Python-3.14.3/Lib \
  --probe-local-numpy \
  --python-probe-bin python3 \
  --out perf/numpy_gate_local_probe_latest.json \
  --timeout 20
```

Source-build mode:

```bash
python3 scripts/probe_numpy_gate.py \
  --pyrs target/debug/pyrs \
  --cpython-lib /Users/$USER/Downloads/Python-3.14.3/Lib \
  --numpy-src /path/to/numpy/source/tree \
  --python-build-bin python3 \
  --build-timeout 1800 \
  --out perf/numpy_gate_source_build_latest.json \
  --timeout 30
```

## Current Snapshot (2026-02-22)

- Direct mode only; CPython bridge mode was removed.
- Base NumPy gate is green:
  - `numpy_import`: `PASS`
  - `numpy_ndarray_sum`: `PASS`
  - `numpy_numerictypes_core`: `PASS`
- Direct smoke sanity is green for:
  - `import numpy as np`
  - `np.dtype('int8')`
  - `np.random.default_rng()` construction
  - `np.random.default_rng().random()` execution
  - `np.random.default_rng().integers(0, 2, size=4)`
  - `np.random.default_rng().integers(0, 2, 4)`
  - `callable(np.random.default_rng().integers)` is `True`
  - `repr(np.random.default_rng().integers)` now renders without unresolved format markers
    (`%U` / `%V`) and matches cyfunction-style output.
- Lifetime stability update:
  - repeated subprocess runs of `import numpy as np; np.random.default_rng()` are now stable
    in debug mode (20/20 local runs, no segfault).
  - escaped compat-object pinning now preserves `ob_type` children, preventing freed type-pointer
    reuse in NumPy random context-manager paths.
  - `_thread` lock substrate now reuses a heap-cached lock class instead of allocating a new class
    object per lock instance, removing a core source of stale type-pointer churn.
- Latest direct scientific-stack probe (`perf/numpy_gate_direct_latest.json`):
  - `scipy_import`: `PASS`
  - `pandas_import`: `FAIL`
  - `pandas_series_sum`: `FAIL`
  - `matplotlib_import`: `FAIL`
  - `matplotlib_pyplot_smoke`: `FAIL`
- Lifetime substrate is in active migration (`docs/CAPI_LIFETIME_MODEL.md`):
  - VM-global pointer registry is authoritative for pointer provenance/liveness.
  - Per-context owned-pointer shadow set has been removed.
  - Owned-pointer free transitions are centralized.

## Current P0 Blockers

1. Pandas import path still fails in direct mode.
   - `pandas_import` and `pandas_series_sum` currently fail during chained extension init in
     `pandas._libs._cyutility` with `proxy object is not callable`.
   - active root-cause lane: proxy-call/vectorcall parity in nested extension init.
2. Matplotlib import path still fails in direct mode.
   - `matplotlib_import` / `matplotlib_pyplot_smoke` currently fail with repeated tuple-unpack
     shape errors (`expected at least 256`) and a later `attempted to call non-function`.
   - active root-cause lane: extension call-arg/value materialization parity for Cython-heavy modules.

## Operating Rules

1. No bridge fallback, shim patching, or test-by-test attr patch churn.
2. Fix shared substrate root causes first, then close downstream module failures.
3. Update this document and `perf/numpy_gate_direct_latest.json` in the same checkpoint as behavior changes.
4. Add/adjust targeted regression tests with every blocker closure.

## Closure Criteria

NumPy bring-up baseline is closed only when all are true:

1. All base gate cases are `PASS` on required platforms.
2. No open P0 blockers remain for exercised extension surfaces in `docs/EXTENSION_CAPABILITY_MATRIX.md`.
3. CI includes the probe in a dedicated extension bring-up lane.
