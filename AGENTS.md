# Project Context: Python Interpreter in Rust (`pyrs`)

## Vision
Build a production-grade Python interpreter in Rust with source + bytecode compatibility for CPython 3.14, minimal third-party dependencies, and architecture that can later support JIT and extension work.

## Non-Negotiable Engineering Rule
- Do not use quick fixes as a substitute for correct design.
- Prioritize root-cause, foundational solutions over tactical patches.
- Any temporary workaround must be explicitly marked and tracked with closure criteria in:
  - `docs/STUB_ACCOUNTING.md`, or
  - `docs/ALGO_AUDIT_BACKLOG.md`.

## Scope and Constraints
- Target version: CPython 3.14
- Current goals:
  - Run Python source code
  - Execute CPython 3.14 bytecode (`.pyc`)
- Current non-goals:
  - JIT implementation
  - Full CPython C-API / C-extension compatibility
- Architecture constraints:
  - Packrat parser aligned to CPython grammar
  - AST -> bytecode IR pipeline
  - CPython-like runtime object model, refcount + cycle GC, and GIL
  - Minimal, justified dependencies only

## Milestone State
- Milestones 0-12: complete
- Milestone 13: in progress (active)
- Milestones 14-16: pending

Milestone 13 closes only when P0 blockers in `docs/PRODUCTION_READINESS.md` and `docs/STUB_ACCOUNTING.md` are fully closed.

## Current Snapshot (2026-02-13)
- Top-stdlib common-usecase gate: `26/26` import, `26/26` smoke.
- Extended stdlib probe: `44/50` import, `39/50` smoke (`perf/stdlib_compat_extended_latest.json`).
- Newly landed parity checkpoints:
  - `math.gcd()` baseline (unblocks `fractions` common path).
  - `threading.Condition.__enter__/__exit__` baseline.
  - `datetime.date/datetime.strftime()` baseline.
  - `_operator._compare_digest` baseline and `_operator` module registration.
- Extended probe remaining red modules:
  - `statistics`, `decimal`, `queue`, `ssl`, `email`, `smtplib`, `imaplib`, `xml`, `gzip`, `bz2`, `lzma`.

## Execution Policy
- CPython behavior is the source of truth:
  - `Modules/*.c`
  - `Objects/*.c`
  - `Lib/*.py`
- Sequence Milestone 13 work as native-core-first:
  1. Native/runtime substrate closure (`_io`, `_csv`, `_sre`, `_pickle`, object protocol)
  2. Pure-stdlib strict-lane expansion/closure
- Prefer official CPython pure-Python stdlib implementations where feasible.
- Keep native handlers as substrate/accelerator layers, not replacement semantics.
- Local shim policy:
  - CPython `Lib/enum.py` path is now the default.
  - `enum` shim is emergency fallback only and must be enabled explicitly with `PYRS_ENABLE_ENUM_SHIM=1`.
  - `pkgutil`/`importlib.resources` local shims are fallback-only and require `PYRS_ENABLE_LOCAL_SHIMS=1`.
  - CPython enum probe regression: `tests/vm.rs::cpython_enum_path_supports_member_value_and_name`.
- Keep docs updated in the same checkpoint as behavior changes.
- Keep worktrees clean; commit small focused checkpoints.
- End every assistant turn with immediate next `3-6` concrete steps.

## Test Loop Policy
- Fast local loops: targeted unit/integration tests first.
- Strict stdlib harness is opt-in for frequent local loops:
  - `PYRS_RUN_STRICT_STDLIB=1`
  - `PYRS_PARITY_STRICT=1`
- Deferred strict pickle lane is opt-in until closure:
  - `PYRS_RUN_DEFERRED_PICKLE=1`
  - `PYRS_DEFERRED_PICKLE_TIMEOUT_SECS` (default `max(PYRS_STRICT_HARNESS_TIMEOUT_SECS, 600)`)

## Performance Policy
- Optimization phase-1 checkpoint is complete.
- Functional Milestone 13 closure is active with benchmark regression protection.
- Canonical benchmark gates:
  - `scripts/bench_fib_gate.sh 5`
  - `scripts/bench_dispatch_hotpath.sh 5`
  - `scripts/bench_dict_backend.sh 5`
- All optimization work must update `docs/OPTIMIZATION_BACKLOG.md` in the same checkpoint.

## Canonical Documents
- Docs index and ownership map: `docs/README.md`
- Milestones and sequencing: `docs/ROADMAP.md`
- Production blockers and release criteria: `docs/PRODUCTION_READINESS.md`
- Partial/stub ledger: `docs/STUB_ACCOUNTING.md`
- Top stdlib common-usecase tracker: `docs/STDLIB_COMMON_USECASE_CHECKLIST.md`
- Object-model parity audit: `docs/OBJECT_MODEL_AUDIT.md`
- Pure-stdlib migration policy: `docs/STDLIB_MIGRATION_PLAN.md`
- Engineering gates: `docs/ENGINEERING_GATES.md`
- Algorithmic/semantic audit backlog: `docs/ALGO_AUDIT_BACKLOG.md`
- Compatibility matrix: `docs/COMPATIBILITY.md`
- VM architecture map: `docs/VM_ARCHITECTURE_MAP.md`
- Optimization execution plan: `docs/OPTIMIZATION_PLAN.md`
- Optimization backlog/status: `docs/OPTIMIZATION_BACKLOG.md`
- Builtin parity gate and policy: `docs/BUILTIN_PARITY.md`, `docs/BUILTIN_OPTIMIZATION_POLICY.md`
- Unicode-name table provenance: `docs/UNICODE_NAME_DATA.md`

## Reference Artifacts
- Milestone 12 closure report: `docs/MILESTONE_12_BACKLOG.md`
- Dict backend CPython mapping: `docs/DICT_BACKEND_CPYTHON_MAPPING.md`
- Dict backend benchmark snapshot: `docs/DICT_BACKEND_BENCHMARK.md`
- Clone audit artifacts: `docs/CLONE_BASELINE.txt`, `docs/CLONE_AUDIT.md`
- No-op inventory snapshot: `docs/NOOP_BUILTIN_INVENTORY.txt`
