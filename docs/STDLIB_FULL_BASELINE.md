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
  --test-timeout 120 \
  --jobs 0
```

`--jobs 0` uses all CPU cores (`os.cpu_count()` workers).

## Latest Baseline (2026-02-24)
- Total stdlib modules (CPython inventory): `297`
- Host-supported modules (CPython imports successfully on this machine): `288`
- pyrs import pass on host-supported modules: `278/288`
- Import failures on host-supported modules: `10`
- Modules with direct mapped CPython test modules: `236`
- Modules eligible for comprehensive phase (`supported_on_host && import_ok && mapped_tests`): `222`
- Comprehensive status on eligible modules:
  - `PASS`: `29`
  - `FAIL`: `173`
  - `TIMEOUT`: `20`
- Total probe wall time (parallel): `544.65s`

## Host-Unsupported Modules (CPython baseline on this machine)
`_gdbm`, `_overlapped`, `_winapi`, `_wmi`, `genericpath`, `msvcrt`, `nt`, `winreg`, `winsound`

## Import Failure Shape (pyrs on host-supported modules)
- Error-type distribution:
  - `ImportError`: `9`
  - `AttributeError`: `1`
- Remaining import failures are concentrated in unsupported/partial native modules:
  - `_asyncio`, `_curses`, `_interpchannels`, `_interpqueues`, `_interpreters`, `_zoneinfo`
  - forced `_elementtree` disable (tracked fallback while `pyexpat.expat_CAPI` is pending)
  - `rlcompleter` via partial `readline` API

## Latest Closure Deltas
- Added native `_scproxy` bootstrap module (`_get_proxy_settings`, `_get_proxies`) so urllib/ssl import flows no longer hard-fail on missing macOS proxy extension.
- Expanded `errno` bootstrap constants to CPython 3.14/macOS baseline (including `EALREADY`, `EWOULDBLOCK` alias) to close import blockers in ssl/network paths.
- Added `inspect.isabstract` to bootstrap inspect surface to unblock `test_abc` import path.
- `CALL_FUNCTION_EX` pyc bound-method regression and xml fallback regressions remain covered in `tests/vm.rs`.

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
