# Stdlib Migration Plan (Pure-Python First)

This document defines the migration strategy for stdlib parity work in Milestone 13.

Primary rule:
- Use CPython's official pure-Python stdlib implementations wherever feasible.
- Keep Rust-native code limited to runtime primitives and CPython C-module surfaces.

## Module Map

| Module area | CPython source of truth | Rust responsibility | Current state | Exit criteria |
|---|---|---|---|---|
| `json` | `Lib/json/*` + optional `_json` accelerator | `_json` compatibility surface only (`scanstring`/`make_scanner`) | VM now has pure `json` preference wiring as explicit opt-in (`Vm::enable_pure_json_preference`); native `json` remains default fallback | Strict/parity suites pass with pure `json` path when opt-in is enabled, then promote pure path to default and retire native fallback |
| `csv` | `Lib/csv.py` + `_csv` C module | `_csv` behavior-compatible accelerator surface | Pure `csv.py` path is primary; `_csv` is native | `test_csv` strict closure + `_csv` edge/perf parity |
| `pickle`/`pickletools`/`copyreg` | `Lib/pickle.py`, `Lib/pickletools.py`, `Lib/copyreg.py` + optional `_pickle` accelerator | object protocol hooks + `_pickle` compatibility surface | Pure modules are primary; strict suite still timeout-blocked on `test_pickle`/`test_pickletools` | Burn strict allowlist to zero and meet perf proof gates |
| `re` | `Lib/re/*` + `_sre` C module | `_sre`-equivalent runtime surface | Still native-heavy because `_sre` parity layer is incomplete | Implement `_sre`-equivalent surface and switch to pure `Lib/re/*` |

## Engineering Policy

1. Pure-first parity:
- New semantic parity fixes for these modules must target CPython pure-Python paths first.

2. Native handler freeze:
- No net-new feature surface in native `json`/`csv`/`pickle`/`re` handlers unless required for:
  - bootstrap correctness,
  - C-accelerator compatibility,
  - or a confirmed runtime primitive gap.
- Every exception must be tracked in `docs/STUB_ACCOUNTING.md` with closure criteria.

3. Accelerator-only direction:
- For modules with CPython C accelerators, Rust should converge on the accelerator role only.
- High-level module behavior should come from CPython pure-Python code.

4. Parity gate:
- For covered modules, runtime must support preferring pure module implementations; rollout may be gated behind explicit opt-in until parity/perf blockers are closed.
- Regression tests must verify this preference explicitly.

## Current Migration Steps Landed

- VM now supports removing preinstalled native `json` modules when CPython `Lib/json/__init__.py` is present and pure-json preference is explicitly enabled.
- This keeps migration behavior testable while avoiding default strict-suite regressions during closure work.

## Remaining P0 Work

1. Close strict `pickle` and `pickletools` timeout blockers.
2. Complete `_csv` and `_pickle` parity for remaining strict/unittest paths.
3. Implement `_sre`-equivalent surface and migrate `re` to pure CPython package.
