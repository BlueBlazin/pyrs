# Project Context: Python Interpreter in Rust (`pyrs`)

## Vision
Build a production-grade Python interpreter in Rust with full source + bytecode compatibility for CPython 3.14, minimal third-party dependencies, and an architecture that can support future JIT/extension work.

## Non-Negotiable Engineering Rule
- Do not make \"fast changes\" or \"quick fixes\" as a substitute for proper design.
- Favor careful, fundamental fixes over tactical patches, even if they take longer.
- If a temporary workaround is unavoidable, it must be:
  1. explicitly marked temporary in code/docs,
  2. tracked with owner + closure criteria in `docs/STUB_ACCOUNTING.md` or `docs/ALGO_AUDIT_BACKLOG.md`,
  3. scheduled for near-term removal.

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
  - CPython-like runtime object model, refcount + cycle GC, GIL
  - Keep dependencies minimal and justified

## Milestone Status (Canonical Summary)
- Milestones 0-12: complete
- Milestone 13: in progress, but temporarily paused while the performance sprint is active
- Milestones 14-16: pending (performance/observability, extension ecosystem, release hardening)

Milestone 13 completion is blocked on P0 closure of:
- `json`
- `_csv` / `csv`
- `pickle` / `pickletools` / `copyreg`

## Execution Policy
- Follow CPython source-of-truth for behavior:
  - `Modules/*.c`
  - `Objects/*.c`
  - `Lib/*.py`
- Sequence Milestone 13 work as native-core-first:
  1. Native/runtime core surfaces (`_io`, `_csv`, `_sre`, `_pickle`, object protocol)
  2. Then strict pure-stdlib suite expansion and closure
- Performance override rule:
  - If optimization sprint is active, performance work takes precedence over Milestone 13 functional closure tasks.
  - Required benchmark gate: `fib(29) x5` under `0.15s` user-time before returning to normal Milestone 13 execution order.
- Prefer official CPython pure-Python stdlib implementations where feasible.
- Keep native VM handlers as accelerator/runtime layers, not full high-level reimplementations.
- Commit frequently in small focused checkpoints.
- Do not leave long-lived dirty worktrees.
- After behavior changes, update docs in the same checkpoint.
- End every assistant turn with immediate next `3-6` concrete steps.

## Test Loop Policy
- Fast local loops should run targeted/unit/integration tests first.
- Strict stdlib harness is opt-in for frequent local loops and reserved for deliberate parity passes:
  - `PYRS_RUN_STRICT_STDLIB=1`
  - `PYRS_PARITY_STRICT=1`
- Keep strict harness subprocess timeout protections enabled to avoid runaway hangs.

## Canonical Documents (Do Not Duplicate Their Contents Here)
- Roadmap and milestone definitions: `docs/ROADMAP.md`
- Production checklist and release blockers: `docs/PRODUCTION_READINESS.md`
- Stub/partial implementation ledger: `docs/STUB_ACCOUNTING.md`
- Object-model parity audit log: `docs/OBJECT_MODEL_AUDIT.md`
- Stdlib pure-Python migration strategy: `docs/STDLIB_MIGRATION_PLAN.md`
- Engineering quality gates: `docs/ENGINEERING_GATES.md`
- Algorithmic/semantic audit backlog: `docs/ALGO_AUDIT_BACKLOG.md`
- VM architecture map: `docs/VM_ARCHITECTURE_MAP.md`
- Compatibility matrix: `docs/COMPATIBILITY.md`
- Coverage gate workflow: `scripts/run_coverage_gate.sh`
- Optimization execution plan: `docs/OPTIMIZATION_PLAN.md`
- Optimization backlog and status ledger: `docs/OPTIMIZATION_BACKLOG.md`

## Reference Artifacts
- Milestone 12 closure report: `docs/MILESTONE_12_BACKLOG.md`
- Dict backend CPython mapping: `docs/DICT_BACKEND_CPYTHON_MAPPING.md`
- Dict backend benchmark snapshot: `docs/DICT_BACKEND_BENCHMARK.md`
- Clone audit baseline/report: `docs/CLONE_BASELINE.txt`, `docs/CLONE_AUDIT.md`
- No-op inventory snapshot: `docs/NOOP_BUILTIN_INVENTORY.txt`

## Current Focus
- Active top priority: optimization sprint.
- Performance gate:
  - `time target/release/pyrs -c "fib = lambda n: n if n < 2 else fib(n-1) + fib(n-2); [fib(29) for _ in range(5)]"`
  - Target: `< 0.15s` user-time
  - Current baseline: ~`0.63-0.64s` user-time (`~0.65-0.67s` wall, warm local release runs); single-run reference `print(fib(29))` is ~`0.14-0.15s` user-time.
- Optimization work must reference CPython internals directly (`Python/ceval.c`, `Python/generated_cases.c.h`, `Include/internal/pycore_frame.h`, `Objects/call.c`, `Objects/longobject.c`) and track decisions in `docs/OPTIMIZATION_PLAN.md`.
- Optimization item status must be updated in `docs/OPTIMIZATION_BACKLOG.md` in the same checkpoint as performance changes.
- Optimization sprint must explicitly close foundational missing surfaces tracked in backlog (`OPT-022` string interning strategy and remaining `OPT-023+` dispatch/call/container items) before being considered complete.
