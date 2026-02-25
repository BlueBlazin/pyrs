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
- Comprehensive test mode:
  - runs mapped CPython `test.test_*` modules with `test.support.use_resources = {}` (resource-disabled baseline, CPython regrtest-aligned default for this probe).
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

## Latest Baseline (2026-02-25)
- Total stdlib modules (CPython inventory): `297`
- Host-supported modules (CPython imports successfully on this machine): `288`
- pyrs import pass on host-supported modules: `278/288`
- Import failures on host-supported modules: `10`
- Modules with direct mapped CPython test modules: `235`
- Modules eligible for comprehensive phase (`supported_on_host && import_ok && mapped_tests`): `222`
- Comprehensive status on eligible modules:
  - `PASS`: `35`
  - `FAIL`: `170`
  - `TIMEOUT`: `17`
- Total probe wall time (parallel): `516.44s`

## Host-Unsupported Modules (CPython baseline on this machine)
`_gdbm`, `_overlapped`, `_winapi`, `_wmi`, `genericpath`, `msvcrt`, `nt`, `winreg`, `winsound`

## Import Failure Shape (pyrs on host-supported modules)
- Remaining import failures are concentrated in missing/partial native extension surfaces
  when running against the local CPython-stdlib root in isolated-path mode.
- Import-path isolation note:
  - when `PYRS_CPYTHON_LIB` is set, pyrs now limits stdlib roots to that explicit path
    and no longer auto-injects host framework stdlib roots.
  - pyrs will use host `lib-dynload` only as a fallback when the isolated root does not contain
    its own `lib-dynload` directory.

## Latest Closure Deltas
- Probe runner now sets `test.support.use_resources = {}` before mapped unittest execution,
  preventing network/resource-heavy CPython tests from running in the baseline lane by default.
- `_PyArg_UnpackKeywords` was rewritten to follow CPython semantics for mixed positional/keyword
  argument binding, including required-argument handling and duplicate/unexpected keyword checks.
- `PyErr_Format` fallback now routes through `PyErr_SetObject` so typed exceptions propagate through
  `PyErr_Occurred` correctly instead of degrading into `SystemError: NULL result without error`.
- `_hmac` keyword-call parity is now restored in extension mode:
  - `_hmac.compute_digest(..., digest='md5')` works,
  - unknown digests raise `UnknownHashError` (not `SystemError`),
  - `test.test_hmac` now passes under the resource-disabled lane (`145` run, `3` skipped).
- Bootstrap `inspect.Signature` now exposes `bind` and `bind_partial` with `BoundArguments`
  materialization so autospec/patch flows in stdlib tests no longer fail on missing bind APIs.
- `_hashlib`/`hmac` comprehensive lanes are now green in this probe mode:
  - `hashlib` (`test.test_hashlib`): `PASS` (`82` run, `15` skipped),
  - `hmac` (`test.test_hmac`): `PASS` (`145` run, `3` skipped).
- `capi_variadics` now exports weak fallback C-API stubs used only when Rust-side strong symbols
  are not linked into lightweight test binaries; this closes linker failures in
  `tests/cli_site_startup.rs` while preserving runtime semantics in normal VM builds.
- `_thread` bootstrap now exports `_local` and thread identity objects are stable per ident
  (instead of allocating a fresh pseudo-thread object per call), closing `_threading_local.local`
  stack-overflow behavior in cycle-collection/attribute-access paths.
- `_threading_local` import finalization now rebinds `_thread._local` to `_threading_local.local`
  so `_thread` local behavior follows CPython's pure-Python fallback semantics instead of the
  previous placeholder class.
- synthetic-thread identity objects are now released at synthetic-thread exit to avoid retention
  leaks through long-running `_threading_local` test loops.
- instance `__dict__` handling was tightened for slot-bearing classes:
  - slot-storage attrs no longer leak into `__dict__`,
  - inherited-slot boundaries are preserved on `__dict__` assignment.
- pickle object state now preserves dynamic dict and slot state together in object get/set-state
  flows, restoring slot+dict roundtrip coverage in `SlotList`-style tests.
- `threading/_threading_local` lane closure status:
  - `_thread._local` now points at `_threading_local.local`,
  - local-ref cleanup and constructor-argument handling regressions were fixed,
  - `_testcapi.call_in_temporary_c_thread`/`join_temporary_c_thread` are present as compatibility no-ops,
  - one known residual mismatch remains in `test_threading_local.*.test_derived_cycle_dealloc`
    due synthetic-thread scheduling limits (no true concurrent thread execution yet).
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
