# Builtin Optimization Policy

This document defines how builtin performance work is prioritized and validated.

## Goals

1. Keep builtin semantics CPython-correct first.
2. Optimize high-frequency call paths without adding semantic shortcuts.
3. Keep optimization decisions explicit and benchmark-backed.

## CPython References

- Eval/dispatch: `/Users/$USER/Downloads/Python-3.14.3/Python/ceval.c`
- Specialized opcode cases: `/Users/$USER/Downloads/Python-3.14.3/Python/generated_cases.c.h`
- Call/vectorcall behavior: `/Users/$USER/Downloads/Python-3.14.3/Objects/call.c`
- Builtins implementation: `/Users/$USER/Downloads/Python-3.14.3/Python/bltinmodule.c`

## Heat Classes

### HOT (P0)
Builtins that appear in tight loops and core runtime paths.

Examples:
- `len`, `bool`, `isinstance`, `issubclass`, `iter`, `next`
- `list`, `tuple`, `dict`, `set`, `range`
- `getattr`, `setattr`, `hasattr`
- `min`, `max`, `sum`

Targets:
1. no-kwargs small-arity direct fast lanes in call dispatch
2. avoid per-call container cloning and temporary argument allocation churn
3. preserve version-guard/cache invalidation correctness

### WARM (P1)
Builtins used frequently but not usually in the innermost loop.

Examples:
- `enumerate`, `zip`, `sorted`, `reversed`, `map`, `filter`
- `abs`, `round`, `pow`, `divmod`
- `format`, `repr`, `str`, `int`, `float`

Targets:
1. lower overhead via shared call fast paths
2. avoid avoidable intermediate objects
3. keep error-path behavior byte-for-byte compatible where practical

### COLD (P2)
Builtins rarely on throughput-critical paths.

Examples:
- `help`, `breakpoint`, `globals`, `locals`, `dir`, `vars`

Targets:
1. correctness and signature parity first
2. no special-case fast lanes unless profiler evidence justifies it

## Implementation Rules

1. No optimization merges without parity tests.
2. No semantic compromises for benchmark-only gains.
3. Every builtin fast path must have a generic fallback path.
4. Any new fast path must be tracked in `docs/OPTIMIZATION_BACKLOG.md`.

## Validation

Minimum validation for builtin optimization checkpoints:
1. `cargo test -q --test vm`
2. `cargo test -q --test differential_cpython`
3. `./scripts/run_builtin_parity_gate.sh`
4. benchmark deltas:
   - `scripts/bench_fib_gate.sh 5`
   - `scripts/bench_dispatch_hotpath.sh 5`
   - `scripts/bench_dict_backend.sh 5`

## Current Status (2026-02-13)

- HOT path checkpoint landed:
  - direct no-keyword fast lanes for `len` and `bool`
  - builtin fast-lane attempts in `CALL_FUNCTION`/`CALL_FUNCTION1` before generic fallback
- Builtin surface parity checkpoint landed:
  - builtin parity gate is green (`145/145`, zero probe mismatches, empty allowlists)
  - `True`/`False`/`None`, `eval`, `hash`, `vars`, and `breakpoint` are now present in `builtins`
- Remaining priority:
  - expand call specialization coverage (`OPT-024`)
  - continue container/dispatch throughput closure (`OPT-023`, `OPT-025`)
