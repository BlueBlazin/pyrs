# Full Stdlib Baseline (CPython 3.14 Inventory)

## Goal
Programmatically baseline **all CPython 3.14 stdlib modules** in pyrs before fix waves:
- import viability for every stdlib module name,
- comprehensive CPython test-module coverage per stdlib module (by naming-convention mapping).

This is the TDD baseline: failures are expected and become the ordered fix backlog.

## Inventory Source
- CPython 3.14 authoritative inventory:
  - `sys.stdlib_module_names` from `/Library/Frameworks/Python.framework/Versions/3.14/bin/python3`

## Probe Script
- Script: `scripts/probe_stdlib_full.py`
- Artifact: `perf/stdlib_full_probe_latest.json`
- Command:

```bash
python3 scripts/probe_stdlib_full.py \
  --pyrs target/debug/pyrs \
  --cpython-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --cpython-lib .local/Python-3.14.3/Lib \
  --out perf/stdlib_full_probe_latest.json \
  --import-timeout 20 \
  --test-timeout 90 \
  --jobs 0
```

`--jobs 0` uses all CPU cores (`os.cpu_count()` workers).

## Latest Baseline (2026-02-24)
- Total stdlib modules (CPython inventory): `297`
- Host-supported modules (CPython imports successfully on this machine): `288`
- pyrs import pass on host-supported modules: `233/288`
- Import failures on host-supported modules: `55`
- Modules with direct mapped CPython test modules: `235`
- Modules eligible for comprehensive phase (`supported_on_host && import_ok && mapped_tests`): `190`
- Comprehensive status on eligible modules:
  - `PASS`: `28`
  - `FAIL`: `147`
  - `TIMEOUT`: `15`
- Total probe wall time (parallel): `343.6s`

## Host-Unsupported Modules (CPython baseline on this machine)
`_gdbm`, `_overlapped`, `_winapi`, `_wmi`, `genericpath`, `msvcrt`, `nt`, `winreg`, `winsound`

## Import Failure Shape (pyrs on host-supported modules)
- Error-type distribution:
  - `ImportError`: `39`
  - `ModuleNotFoundError`: `15`
  - `RuntimeError`: `1`
- Most frequent unresolved symbols in failure traces:
  - `_PyUnicodeWriter_WriteChar` (`12`)
  - `_PyUnicodeWriter_PrepareInternal` (`12`)
  - `_PyRuntime` (`12`)
  - `_Py_ctype_tolower` (`12`)
  - `_PyNumber_Index` (`8`)
  - `_PyLong_UnsignedLong_Converter` (`8`)
  - `PyTime_PerfCounterRaw` (`8`)

## Latest Closure Deltas
- Added parallelized full-stdlib probe execution (`--jobs 0` defaulting to all cores).
- Added CPython-style `lib-dynload` auto-path insertion in CLI startup path discovery.
- Extension load failures now raise `ImportError` (not generic runtime errors), restoring CPython fallback behavior (`try: import _mod except ImportError: ...`) for pure-Python modules.
- Added C-API compatibility symbol exports in `capi_variadics.c` + keepalive wiring for:
  - `_PyArg_BadArgument`
  - `_PyArg_CheckPositional`
  - `_PyArg_NoKeywords`
  - `_PyArg_UnpackKeywords`
  - `PyImport_ImportModuleAttrString`
  - `PyErr_FormatUnraisable`
  - `Py_HashBuffer`
  - `_PyLong_UnsignedInt_Converter`
  - `_Py_ctype_table`

## Notes on Comprehensive Mapping
- Mapping is systematic and programmatic:
  - for stdlib module `X`, mapped tests include `test.test_X` and `test.test_X_*`
  - plus underscore/package normalization (`X.Y` -> `X_Y`, `_x` -> `x`) for CPython test naming conventions.
- This gives broad CPython test coverage without maintaining hand-curated per-module lists.

## Fix Loop (TDD)
1. Re-run `scripts/probe_stdlib_full.py` to refresh baseline artifact.
2. Pick a failing import/test cluster and fix by CPython source parity.
3. Add/extend targeted regressions.
4. Re-run full probe in parallel.
5. Repeat until import/comprehensive counts converge to closure targets.
