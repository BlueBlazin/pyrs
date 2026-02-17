# NumPy Bring-Up Gate

Status: active (Milestone 15).

Purpose: track direct native-extension execution gates for NumPy and the scientific stack.

Execution model: see `docs/CAPI_PLAN.md`.
- Lane A: Stable ABI (`abi3`) closure.
- Lane B: explicit non-abi3 surfaces required by NumPy/scientific stack.

## Gate Definitions

1. `numpy_import`
   - snippet: `import numpy as np`
2. `numpy_ndarray_sum`
   - snippet:
     - `import numpy as np`
     - `a = np.array([1, 2, 3])`
     - `assert int(a.sum()) == 6`

These are intentionally small but strict: they verify import path + first ndarray runtime path.

Optional scientific-stack cases are also available via `--include-scientific-stack`:
- `scipy_import`
- `pandas_import`
- `pandas_series_sum`
- `matplotlib_import`
- `matplotlib_pyplot_smoke`

## Import-Probe Command

```bash
python3 scripts/probe_numpy_gate.py \
  --pyrs target/debug/pyrs \
  --cpython-lib /Users/$USER/Downloads/Python-3.14.3/Lib \
  --out perf/numpy_gate_latest.json \
  --timeout 20
```

Optional strict mode (`--strict`) returns non-zero if any gate fails.

## Local-Install Probe Mode

To distinguish environment absence (`module-not-found`) from runtime ABI issues, you can probe a local Python for an installed NumPy and reuse its site-packages root:

```bash
python3 scripts/probe_numpy_gate.py \
  --pyrs target/debug/pyrs \
  --cpython-lib /Users/$USER/Downloads/Python-3.14.3/Lib \
  --probe-local-numpy \
  --python-probe-bin python3 \
  --out perf/numpy_gate_local_probe_latest.json \
  --timeout 20
```

Report field:
- `local_numpy_probe.status = FOUND|NOT_FOUND|ERROR|SKIP`
- `local_module_probe.modules.<name>.status = FOUND|NOT_FOUND`

When `FOUND`, the probe injects the detected site-packages root into `PYTHONPATH` for gate cases.

## Source-Build Bring-Up Command

When a local NumPy source checkout is available, run:

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

This performs:
1. Source-build attempt (`pip install --target ...`) from the provided NumPy source tree.
2. `import numpy` + first ndarray smoke against the resulting site-packages path.

If `--numpy-src` does not exist, the build stage is recorded as `SKIP` and the report still captures runtime probe results.

## Scientific-Stack Probe Command (Direct Mode)

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

If a probed local module is not installed, its dependent cases are recorded as `SKIP` rather than `FAIL`.

## Current Expected State

- CPython ABI bridge mode has been removed; probes run in direct mode only.
- Lane B remains required even with Lane A progress; local NumPy artifacts are currently `cp314-cp314` wheels (not abi3-only wheels).
- Import-probe and source-build modes both produce actionable failure diagnostics in JSON.
- Local-install probe mode helps classify failures as environment/setup (`NOT_FOUND`) vs substrate/ABI (`missing-symbol`, `abi-mismatch`, `init-failure`).
- Probe output classifies common failure kinds (`module-not-found`, `missing-symbol`, `abi-mismatch`, `init-failure`) to guide C-API/loader closure work.
- Dynamic-link symbol closure for `_multiarray_umath` is now in place (public `Py*` and internal `_Py*` surfaces exported by `pyrs`).
- Latest direct-mode gate status (`perf/numpy_gate_direct_latest.json`):
  - `numpy_import`: `PASS`
  - `numpy_ndarray_sum`: `FAIL` with `RuntimeError: PyNumber_Long requires int-compatible object`
  - current blocker is numeric conversion parity for extension-backed scalar return paths in ndarray reduction flows (`a.sum()` -> `int(...)`).
- Extension-init failure reporting now preserves the first meaningful per-module `Py_mod_exec` failure across retry attempts, preventing fallback noise like `cannot load module more than once per process` from masking the root blocker.
- Recent direct-mode bring-up deltas:
  - `_PyType_Lookup` now preserves no-new-error semantics and falls back to runtime MRO lookup when `tp_dict`-only lookup misses.
  - attribute optional/presence helpers now treat CPython-style "missing attribute" message paths as non-fatal misses (`HasAttr*WithError`, `GetOptionalAttr*`).
  - pure-stdlib preference logic now includes `typing` (not only `types`) when CPython `Lib` sources are present.
  - `datetime.datetime_CAPI` capsule baseline is now registered for `PyCapsule_Import`.
  - `math.trunc` landed for stdlib parity used during NumPy init.
  - `sys.modules` identity is now stable across imports (no dict-object replacement on each register/unregister).
  - `_Py_BuildValue` now routes through a C varargs shim (`build.rs` + `src/vm/capi_variadics.c`) with partial format coverage (`()`, `O`, `N`, `s`, tuple `(...)` for `O/N/i/l/k/n/d/f/s`, `{ON}`, `{s:O}`, `{s:N}`).
  - C-side varargs parser/call surfaces now include `PyArg_ParseTuple`, `PyArg_ParseTupleAndKeywords`, `PyObject_CallFunctionObjArgs`, and `PyObject_CallMethod`.
  - `PyTypeObject` compat layout now includes init/alloc/new/call slots used by direct extension type construction paths.
- Failures are signal, not noise; they should be used to drive substrate work in:
  - `docs/EXTENSION_CAPABILITY_MATRIX.md`
  - `docs/EXTENSION_PACKAGING_CONTRACT.md`
  - `docs/EXTENSION_ECOSYSTEM_DESIGN.md`

## Closure Criteria

NumPy bring-up baseline is closed only when all are true:

1. Both gate cases are `PASS` on required platforms.
2. No open P0 blockers remain for exercised extension surfaces in `docs/EXTENSION_CAPABILITY_MATRIX.md`.
3. CI includes the probe in a dedicated extension bring-up lane.
