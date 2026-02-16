# CPython C-API Plan (ABI-First, NumPy-Focused)

Status: active (Milestone 15 execution strategy).

## Goal

Provide a production-grade native extension substrate in `pyrs` that:

1. Implements CPython 3.14 Stable ABI (`abi3`) rigorously.
2. Supports NumPy/scientific stack native execution without CPython runtime fallback.

## Why Not “Implement All of Python.h”

`Python.h` is too large and includes non-stable/private/internal surfaces that do not define a bounded release target.  
The practical release target is:

- **Lane A**: full CPython 3.14 Stable ABI (`Misc/stable_abi.toml`).
- **Lane B**: explicit non-abi3 surfaces required by real target extensions (NumPy first).

This keeps scope finite and testable while still unblocking scientific stack support.

## Critical Constraint

Stable ABI completion alone is **necessary but not sufficient** for NumPy/scientific stack:

- Local NumPy wheel here is `cp314-cp314` (not `abi3`):  
  `/Users/$USER/pyrs/.venv-ext314/lib/python3.14/site-packages/numpy-2.4.2.dist-info/WHEEL`

Therefore, Lane B remains required.

## Two-Lane Execution Model

### Lane A (P0): Stable ABI (`abi3`) closure

Source of truth:
- CPython 3.14 `Misc/stable_abi.toml`

Required outputs:
- Machine-generated manifest of Stable ABI symbols and current `pyrs` export coverage:
  - `perf/abi3_manifest_latest.json`
- Coverage script:
  - `scripts/generate_abi3_manifest.py`
- ABI conformance tests (symbol presence + semantics for implemented slices).

Acceptance criteria:
1. `function` and `data` coverage in manifest reaches 100%.
2. Stable ABI behavior tests are green in CI.
3. No untracked temporary compatibility patches in C-API core paths.

### Lane B (P0): NumPy/scientific-stack non-abi3 closure

Source of truth:
- Real extension artifacts (wheel/source), dynamic-link symbol usage, runtime call traces.

Required outputs:
- Explicit gap list in `docs/NUMPY_BRINGUP_GATE.md` mapped by subsystem.
- Deterministic gate progression in:
  - `perf/numpy_gate_direct_latest.json`
- Subsystem-first fixes (object/type model, call protocol, error semantics, module init), no attr-by-attr patch churn.

Acceptance criteria:
1. `import numpy` passes in direct mode.
2. Baseline ndarray smoke passes (`int(np.array([1,2,3]).sum()) == 6`).
3. Scientific stack import/smoke cases pass per gate policy.

## Engineering Rules for This Plan

1. No trial-and-error symbol patch churn.
2. Fix root semantics in shared substrate first.
3. Every temporary workaround must be tracked in:
   - `docs/STUB_ACCOUNTING.md` or
   - `docs/ALGO_AUDIT_BACKLOG.md`
4. Every C-API behavior change must include:
   - targeted test(s),
   - manifest/gate update,
   - docs update in the same checkpoint.

## Current Baseline Snapshot

From `perf/abi3_manifest_latest.json`:
- Stable ABI functions implemented/exported: `252 / 782`
- Stable ABI data symbols implemented/exported: `47 / 143`

Recent Lane A slice:
- Added Stable-ABI exports and semantics for `PyDict_{Clear,Update,Keys,Values,Items,MergeFromSeq2}`.
- Added Stable-ABI exports and semantics for `PyCapsule_{GetName,SetPointer,GetDestructor,SetDestructor}`.
- Added Stable-ABI exports and semantics for `PyByteArray_*` constructor/access/resize/concat APIs.
- Fixed `PyBuffer_Release` to release pin/ref state (was previously a no-op).

Interpretation:
- Lane A is substantial remaining work.
- Lane B continues in parallel for NumPy bring-up because current NumPy artifacts are not abi3 wheels.
