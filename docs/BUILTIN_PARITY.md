# Builtin Parity Gate (CPython 3.14)

This document defines the canonical gate for builtin-surface parity against CPython.

## Why This Gate Exists

Builtin drift is a production risk:
- missing symbols break stdlib and user code unexpectedly,
- callable/arity drift causes hard-to-debug runtime errors,
- silent semantic drift in common builtins can pass broad suites but still break apps.

## Source-of-Truth

- CPython builtin implementation: `/Users/$USER/Downloads/Python-3.14.3/Python/bltinmodule.c`
- Runtime target for comparison: `target/debug/pyrs` (or `PYRS_BIN`)

## Gate Command

Use the wrapper:
- `./scripts/run_builtin_parity_gate.sh`

Direct tool:
- `python3 scripts/check_builtin_parity.py --check`

Output artifact:
- `perf/builtin_parity_report.json`

## What The Gate Checks

1. Builtin inventory parity
- compares non-dunder names in `builtins.__dict__`
- records names missing in `pyrs`
- records extra names in `pyrs`

2. Builtin metadata parity
- callable classification (`callable(obj)`)
- type name
- `__text_signature__`
- signature shape (min/max positional, varargs/varkw, required kw-only) when signature data is available on both runtimes

3. Builtin semantic probes
- executes curated probes on both runtimes for high-risk contracts
- compares success/failure, result type+repr, and exception type

## Allowlist Policy

Allowlist files:
- `tests/builtin_missing_allowlist.txt`
- `tests/builtin_probe_allowlist.txt`

Rules:
1. allowlists are temporary and must only shrink
2. net-new missing builtins fail the gate
3. net-new semantic probe mismatches fail the gate
4. stale allowlist entries fail the gate

## Current Baseline

- CPython builtin count: `145`
- pyrs builtin count: `145`
- Missing builtin names in `pyrs`: none
- Unexpected semantic probe mismatches: none
- Allowlists:
  - `tests/builtin_missing_allowlist.txt`: empty
  - `tests/builtin_probe_allowlist.txt`: empty

## Closure Criteria

This gate is considered closed when all are true:
1. `tests/builtin_missing_allowlist.txt` is empty
2. `tests/builtin_probe_allowlist.txt` is empty
3. `perf/builtin_parity_report.json` shows zero unexpected/stale findings
4. parity gate remains green in CI
