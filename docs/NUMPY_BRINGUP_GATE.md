# NumPy Bring-Up Gate

Status: active (Milestone 15).

Purpose: track first real extension-ecosystem execution gates for NumPy.

## Gate Definitions

1. `numpy_import`
   - snippet: `import numpy as np`
2. `numpy_ndarray_sum`
   - snippet:
     - `import numpy as np`
     - `a = np.array([1, 2, 3])`
     - `assert int(a.sum()) == 6`

These are intentionally small but strict: they verify import path + first ndarray runtime path.

## Probe Command

```bash
python3 scripts/probe_numpy_gate.py \
  --pyrs target/debug/pyrs \
  --cpython-lib /Users/$USER/Downloads/Python-3.14.3/Lib \
  --out perf/numpy_gate_latest.json \
  --timeout 20
```

Optional strict mode (`--strict`) returns non-zero if any gate fails.

## Current Expected State

- Before real C-extension substrate closure, this probe is expected to report failures.
- Failures are signal, not noise; they should be used to drive substrate work in:
  - `docs/EXTENSION_CAPABILITY_MATRIX.md`
  - `docs/EXTENSION_PACKAGING_CONTRACT.md`
  - `docs/EXTENSION_ECOSYSTEM_DESIGN.md`

## Closure Criteria

NumPy bring-up baseline is closed only when all are true:

1. Both gate cases are `PASS` on required platforms.
2. No open P0 blockers remain for exercised extension surfaces in `docs/EXTENSION_CAPABILITY_MATRIX.md`.
3. CI includes the probe in a dedicated extension bring-up lane.
