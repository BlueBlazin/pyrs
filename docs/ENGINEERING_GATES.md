# Engineering Quality Gates

This document defines mandatory process gates to prevent and detect semantic and algorithmic regressions in core interpreter/runtime code.

These gates are release-relevant, not advisory.

## Scope

Applies to all changes under:
- `src/runtime/`
- `src/vm/`
- `src/compiler/`
- native stdlib handlers in `src/vm/stdlib/`

## Gate 0: Core Helper Unit Coverage (P0)

Critical helper algorithms must have direct module-local tests so regressions are caught
before broad integration/harness runs.

Required modules:
- `src/vm/containers.rs`
- `src/vm/ops.rs`
- `src/runtime/mod.rs`
- `src/vm/stdlib/json.rs`
- `src/vm/stdlib/re.rs`
- `src/vm/stdlib/csv.rs`

Required evidence:
1. `#[cfg(test)]` coverage in each module for argument validation + edge/error paths
2. focused regression tests for previously broken contracts (escape/encoding, exception forwarding, strict parsing)
3. no helper-only semantic changes merged without adding or updating unit coverage in the same commit

## Gate 1: Semantic Contract Conformance (P0)

No operation is considered done until CPython-visible behavior matches for the in-scope contract.

Required for mutable/container APIs:
- in-place mutation guarantees (`list.sort`, `list.reverse`, `dict.update`, etc.)
- mutation-during-operation behavior (error type/message/state guarantees)
- exception ordering/propagation on partial progress
- aliasing behavior (object identity preservation where required)

Required evidence:
1. targeted regression tests in `tests/vm.rs`
2. differential tests against CPython for the exact API surface
3. explicit edge-case list (empty/singleton/large, mixed comparable types, exceptional callbacks)

## Gate 2: Algorithmic Complexity Conformance (P0/P1)

For fundamental operations, intended complexity must be declared and verified.

Required for core structures:
- `dict`/`set`/`frozenset` lookup/insert/delete/update/membership
- `list` mutating operations (`append`, `extend`, `insert`, `pop`, `sort`)
- bigint conversion/formatting primitives on large values

Required evidence:
1. operation complexity table in `docs/ALGO_AUDIT_BACKLOG.md`
2. adversarial-size regression tests (small/medium/large)
3. benchmark deltas recorded before/after implementation changes

## Gate 3: Clone/Allocation Discipline (P1)

Hot-path code must justify full-data clones.

Rules:
- cloning a full container/string in hot paths requires explicit justification comment or tracked issue
- if an operation is in-place by CPython contract, implementations must avoid copy-then-replace unless required for safety and parity
- avoid hidden quadratic behavior from repeated cloning in loops

Required evidence:
1. targeted audit entries in `docs/ALGO_AUDIT_BACKLOG.md`
2. regression/perf tests for clone-sensitive paths

## Gate 4: Native Stdlib Handler Policy (P0)

Default policy is to use official CPython pure-Python stdlib implementations when feasible.

Native VM handlers are allowed only when:
- needed for bootstrap, performance, or missing runtime capability
- behavior is tracked in `docs/STUB_ACCOUNTING.md`
- parity tests exist for the handler surface

Additional Milestone 13 controls:
- Follow module ownership in `docs/STDLIB_MIGRATION_PLAN.md`.
- For `json`/`csv`/`pickle`/`re`, treat native handlers as accelerator/runtime layers, not primary semantics.
- Net-new native feature work in those handlers requires a tracked gap with explicit closure criteria.

## Gate 5: Monolith and Reviewability Control (P1)

Large monolithic files hide semantic defects.

Rules:
- refactor by concern (ops, containers, import, stdlib handlers)
- each extraction must be behavior-preserving with regression proof
- new functionality should target focused modules, not `src/vm/mod.rs` when a focused module exists

## Detection Pipeline

Run this pipeline continuously during Milestone 13 and Milestone 14:

1. `cargo test --quiet --lib` (module-local helper gates)
2. `cargo test --quiet`
3. curated CPython harness suites (`tests/cpython_harness.rs`)
4. differential corpus tests (`tests/differential_cpython.rs`)
5. fuzz/no-panic suites (`tests/fuzz_parser_vm.rs`)
6. runtime leak regression lane (`tests/gc_regression.rs`)
7. targeted algorithmic audits from `docs/ALGO_AUDIT_BACKLOG.md`
8. stub/no-op drift gate (`tests/noop_inventory.rs`)
9. coverage gate summary (`scripts/run_coverage_gate.sh`; CI enforces soft floors at 70% regions / 65% functions / 70% lines, local runs remain report-only unless `PYRS_COVERAGE_ENFORCE=1`)
10. strict-harness timeout regression (`tests/cpython_harness.rs::subprocess_harness_helper_times_out_hanging_program`) so hang/memory-growth incidents fail fast

Strict stdlib harness policy:
- `tests/cpython_harness.rs` strict suite runs in isolated subprocesses with a per-entry timeout (`PYRS_STRICT_HARNESS_TIMEOUT_SECS`, default 120s) to prevent unbounded hangs/memory growth from masking regressions.

## Completion Criteria

A quality item is only closed when all are true:
1. CPython parity tests pass for the item
2. algorithmic behavior is documented and validated
3. no untracked shortcuts remain in implementation/docs
4. backlog entry is marked closed with commit reference

## Milestone Ownership

- Milestone 13 (P0): semantic contract violations and release blockers in core runtime/stdlib paths.
- Milestone 14 (P1/P2): algorithmic/perf architecture closure and broad hot-path optimization/verification.
- Milestone 16 (P0): release certification gates enforce the above in CI policy.
