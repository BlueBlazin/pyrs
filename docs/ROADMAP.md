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
| 13 | Long-tail parity + stdlib usability closure | In Progress |
| 14 | Performance + observability + architecture hardening | Pending |
| 15 | Native extension ecosystem compatibility | Pending |
| 16 | Release hardening/certification | Pending |

## Active Milestone: 13

### Exit Criteria
Milestone 13 is complete only when all are true:
1. P0 blockers in `docs/PRODUCTION_READINESS.md` are closed.
2. Milestone-13 P0 rows in `docs/STUB_ACCOUNTING.md` are closed.
3. Active strict stdlib lane is green with empty allowlist.
4. Deferred strict pickle lane is re-enabled and green.
5. Engineering gates in `docs/ENGINEERING_GATES.md` are satisfied for Milestone 13 scope.
6. Builtin parity gate (`docs/BUILTIN_PARITY.md`) is green with empty allowlists.

### Implementation Strategy
1. Native-core-first, then strict pure-stdlib expansion.
2. Use CPython sources as implementation references:
   - `Modules/*.c`
   - `Objects/*.c`
   - `Lib/*.py`
3. Prefer official pure-Python stdlib modules for high-level semantics.
4. Keep native VM stdlib code as accelerator/runtime substrate only.
5. Track all partial behavior in `docs/STUB_ACCOUNTING.md`.

### Workstreams
- Runtime/native core parity:
  - `_io`, `_csv`, `_sre`, `_pickle`
  - translated `.pyc` long-tail opcode/state parity closure (exception-table runtime baseline is landed)
  - object-model protocol long-tail parity
- Pure-stdlib handoff:
  - make CPython pure modules the default behavior path where feasible
  - retire temporary shims once parity blockers close
- Gate-driven closure:
  - targeted unit/regression tests
  - curated + strict harness lanes
  - differential tests against CPython

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
