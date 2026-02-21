# Design and Roadmap

## Purpose
This is the canonical forward plan for delivering a production-grade CPython 3.14-compatible interpreter.
It is intentionally state-oriented, not a historical changelog.

## Project Direction
- Correctness first, then performance.
- CPython behavior fidelity over local convenience APIs.
- Minimal, justified dependencies.
- Clean boundaries between parser, compiler, VM, runtime, and stdlib substrate.
- Milestones close only when parity gates are satisfied (not "basic compat").

## Milestone State
| Milestone | Scope | Status |
|---|---|---|
| 0 | Parser/AST bootstrap | Complete |
| 1 | Runtime identity + GC foundations | Complete |
| 2 | CPython bytecode intake foundations | Complete |
| 3 | Closures + frames + traceback foundations | Complete |
| 4 | Generator/iteration parity core | Complete |
| 5 | Opcode hardening + supported `.pyc` read/write | Complete |
| 6 | Import-system parity foundations | Complete |
| 7 | Language surface expansion (core modern syntax) | Complete |
| 8 | Data-model semantics foundations | Complete |
| 9 | Core runtime types + stdlib bootstrap | Complete |
| 10 | Async/concurrency foundations | Complete |
| 11 | Test/parity gate infrastructure | Complete |
| 12 | Curated language/import harness closure | Complete |
| 13 | Long-tail parity + stdlib usability closure | In Progress (background) |
| 14 | Performance + observability + architecture hardening | Pending |
| 15 | Native extension ecosystem compatibility | In Progress (active) |
| 16 | Release hardening/certification | Pending |

## Active Execution Lock: Milestone 15

### Exit Criteria
Milestone 15 is complete only when all are true:
1. Extension capability matrix P0 rows are closed (`docs/EXTENSION_CAPABILITY_MATRIX.md`).
2. `docs/NUMPY_BRINGUP_GATE.md` base gates are green and scientific-stack blockers are either closed or explicitly downgraded with accepted scope.
3. C-API lifetime-model closure criteria are satisfied (`docs/CAPI_LIFETIME_MODEL.md`).
4. No bridge fallback/shim-only workaround is required for primary scientific-stack gates.

### Implementation Strategy
1. CPython-semantics-first; no pyrs-specific behavior where CPython differs.
2. Substrate-first closure: fix shared C-API/runtime invariants before per-module symptoms.
3. Keep pointer ownership and lifetime authority in the VM-global registry.
4. Update docs + tests + probe artifacts in the same checkpoint.

### Workstreams
- C-API substrate:
  - lifetime/ownership invariants
  - exception-indicator/thread-state parity
  - type/call/descriptor parity
- Scientific-stack bring-up:
  - NumPy direct-mode blockers first
  - pandas/scipy/matplotlib closure after NumPy random stack is stable
- Quality gates:
  - targeted regression tests in `tests/vm.rs`
  - probe artifacts in `perf/numpy_gate_direct_latest.json`
  - CI gate integration and stability lanes

## Milestone 13 (Still Open)

Milestone 13 remains open and is tracked in parallel for stdlib/runtime long-tail parity.
Its closure criteria remain in:
- `docs/PRODUCTION_READINESS.md`
- `docs/STUB_ACCOUNTING.md`
- `docs/ENGINEERING_GATES.md`

## Milestone 14 (Performance and Architecture)
Deliverables:
- close remaining throughput backlog in `docs/OPTIMIZATION_BACKLOG.md`
- enforce clone/allocation discipline on hot paths
- continue VM/runtime decomposition for maintainability
- keep benchmark and observability gates integrated into CI

## Milestone 15 (Extension Ecosystem)
Deliverables:
- limited C-API/abi3 execution path for supported surfaces
- extension capability matrix and packaging/build contract:
  - `docs/EXTENSION_CAPABILITY_MATRIX.md`
  - `docs/EXTENSION_PACKAGING_CONTRACT.md`
- first C-API header/symbol slice and compiled-extension fixture:
  - `include/pyrs_capi.h`
  - `docs/EXTENSION_CAPI_V1.md`
- extension-backed ecosystem smoke suites + explicit unsupported-surface diagnostics
- baseline extension loader smoke gate (`hello_ext`) in CI
- NumPy bring-up gate scaffold (`import numpy` + first ndarray smoke)
- architecture and delivery gates follow `docs/EXTENSION_ECOSYSTEM_DESIGN.md`

## Milestone 16 (Release Hardening)
Deliverables:
- security/reliability release gates
- Linux/macOS/Windows qualification matrix
- reproducible/signed artifacts and release playbook

## Operating Rules
- Commit in small focused checkpoints.
- Keep worktree clean.
- Update docs in the same checkpoint as behavior changes.

## Companion Docs
- `docs/README.md`
- `docs/PRODUCTION_READINESS.md`
- `docs/STUB_ACCOUNTING.md`
- `docs/STDLIB_COMMON_USECASE_CHECKLIST.md`
- `docs/STDLIB_MIGRATION_PLAN.md`
- `docs/ENGINEERING_GATES.md`
- `docs/ALGO_AUDIT_BACKLOG.md`
- `docs/OPTIMIZATION_PLAN.md`
- `docs/OPTIMIZATION_BACKLOG.md`
- `docs/BUILTIN_PARITY.md`
- `docs/BUILTIN_OPTIMIZATION_POLICY.md`
