# Native Extension Import Performance Plan

## Goal
Reduce native scientific-stack import overhead (starting with `import numpy`) while keeping CPython 3.14 semantics as the default behavior.

Primary user-facing gate:
- `target/release/pyrs -S -c "import sys; sys.path.insert(0, './.venv-ext314/lib/python3.14/site-packages'); import numpy as np"`

## Current Baseline (2026-02-21, latest local)
- `pyrs` (default pyc policy): ~`0.79s` user
- `pyrs` with `PYRS_IMPORT_PREFER_PYC=0`: ~`0.64s` user
- CPython 3.14: ~`0.05s` user

## Root-Cause Buckets
1. C-API compatibility runtime overhead in hot entrypoints used during extension init.
2. Repeated pointer/value mapping and ownership checks in compare/call/attr flows.
3. Module attribute lookup overhead during import-time bootstrap.
4. Remaining Python-source execution/parsing during import graph walk.

## Execution Plan

### P0-1: C-API Hotpath Cost Model Closure
- Inline/fast-path the most frequent C-API operations from flamegraphs.
- Keep ownership/lifetime invariants, but remove repeated linear scans and redundant map conversions.
- Target surfaces:
  - `PyObject_RichCompare*`
  - `PyObject_GetAttrString`
  - tuple/list/dict read paths used during extension registration loops.

Status: `in progress`

### P0-2: Module Attribute Import-Path Closure
- Avoid full frame scans unless module is currently initializing.
- Avoid full globals snapshot cloning in normal attr loads.
- Add fast-path for top-of-stack module frame lookup.

Status: `in progress`

### P0-3: Source-vs-Bytecode Policy + Runtime Cost
- Keep CPython-default policy: prefer validated source-bound `.pyc` by default.
- Measure and close runtime overhead of our `.pyc` translation/execution path so default policy is non-regressing.
- Ensure source fallback remains correct for stale/invalid cache.

Status: `in progress`

### P1-1: Import Graph Work Reduction
- Expand safe cacheing for module resolution and importer state signatures.
- Reduce repeated parser/compile execution on warm runs.

Status: `planned`

### P1-2: Toolchain/Build Optimizations
- Evaluate profile settings for import-heavy workloads (LTO/debug-symbol settings for profiling vs release tuning).
- Keep as optional unless semantic/runtime wins are demonstrated.

Status: `planned`

## Safety Gates
For each optimization slice:
1. Run targeted NumPy regression probes from `tests/vm.rs`.
2. Keep sanitizer/lifetime guards unchanged or strengthened.
3. Re-profile with `cargo flamegraph` and record delta in `perf/`.
4. Commit checkpoint with exact measured impact.

## Implemented In This Round
- Module attr lookup now skips frame scans when module is not actively initializing.
- `PyObject_RichCompare` now attempts slot dispatch directly before pointer/value conversion fallback.
- pyc compatibility closure for import-path blockers:
  - marshal `TYPE_ELLIPSIS`, `TYPE_STOPITER`, and bigint `TYPE_LONG` decode support.
  - bytes constants now translate directly from pyc constants.
  - opcode mapping support for `DELETE_ATTR` and `LOAD_FROM_DICT_OR_DEREF`.
  - opcode mapping/runtime support for `CALL_INTRINSIC_2` (`arg=4` function type-params intrinsic).
- NumPy import graph pyc fallback counters improved from:
  - `source_compiles=30`, `pyc_fallbacks=29`
  - to `source_compiles=12`, `pyc_fallbacks=11`.
