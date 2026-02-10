# Design and Roadmap

## Purpose
This roadmap is the canonical plan for delivering production-grade CPython 3.14 compatibility.
It is intentionally forward-looking and should not be used as an append-only changelog.

## Project Direction
- Correctness first, then performance.
- Prefer CPython behavior fidelity over local convenience APIs.
- Keep dependencies minimal and justified.
- Keep runtime boundaries clean: parser, compiler, VM, runtime, stdlib.
- Avoid "basic compat" completion criteria. A milestone is complete only when in-scope parity gates are met.

## Milestone State
| Milestone | Scope | Status |
|---|---|---|
| 0 | Parser/AST bootstrap | Complete |
| 1 | Runtime identity + GC foundations | Complete |
| 2 | CPython bytecode intake foundations | Complete |
| 3 | Closures + frames + traceback foundations | Complete |
| 4 | Generator/iteration parity core | Complete |
| 5 | Opcode hardening + supported `.pyc` write/read path | Complete |
| 6 | Import-system parity foundations | Complete |
| 7 | Language surface expansion (core modern syntax) | Complete |
| 8 | Data-model semantics foundations | Complete |
| 9 | Core runtime types + stdlib bootstrap | Complete |
| 10 | Async/concurrency foundations | Complete |
| 11 | Test/parity gate infrastructure | Complete |
| 12 | Curated language/import harness closure | Complete |
| 13 | Long-tail parity + stdlib usability closure | In Progress (Temporarily Paused for Perf Sprint) |
| 14 | Performance + observability + architecture hardening | Pending |
| 15 | Native extension ecosystem compatibility | Pending |
| 16 | Release hardening/certification | Pending |

## Active Priority Override: Performance Sprint

Milestone 13 functional closure is temporarily paused while performance is brought to a usable baseline.

Primary gate:
- `time target/release/pyrs -c "fib = lambda n: n if n < 2 else fib(n-1) + fib(n-2); print(fib(29))"`
- Target: `< 0.10s` (user-time reference target)
- Current measured baseline after recent fixes: ~`0.59s` user-time

This sprint is implementation-driven from CPython internals:
- Eval loop and opcode specialization patterns: `Python/ceval.c`, `Python/generated_cases.c.h`
- Frame/local layout patterns (`f_localsplus`, frame lifecycle): `Include/internal/pycore_frame.h`, `Python/frame.c`
- Call/fastcall/vectorcall behavior: `Objects/call.c`, `Include/cpython/abstract.h`
- Integer fast paths and small-int behavior: `Objects/longobject.c`

Detailed execution plan: `docs/OPTIMIZATION_PLAN.md`
Canonical optimization status tracker: `docs/OPTIMIZATION_BACKLOG.md`

Mandatory foundational optimization scope during sprint includes:
- small-int/immortal integer strategy closure (`OPT-021`)
- explicit string interning strategy closure (`OPT-022`)
- `LOAD_ATTR`/method-call cache specialization (`OPT-023`)

## Active Milestone: 13 (Paused During Perf Sprint)

### Milestone 13 Exit Criteria
Milestone 13 is complete only when all are true:
1. P0 stdlib blockers are closed: `json`, `_csv`/`csv`, `pickle`/`pickletools`/`copyreg`.
2. Native-core runtime surfaces needed by pure stdlib are implemented with CPython-referenced semantics (`_io`, `_csv`, `_sre`, `_pickle`, object protocol hooks).
3. Remaining in-scope runtime/language parity gaps are closed (long-tail attribute/data-model/pattern/exception edges tracked in readiness docs).
4. Strict stdlib harness lane for active modules is green with empty allowlist.
5. Deferred pickle strict lane is re-enabled and closed.
6. Engineering gates in `docs/ENGINEERING_GATES.md` and P0 audit backlog in `docs/ALGO_AUDIT_BACKLOG.md` are satisfied for Milestone 13 scope.

### Milestone 13 Implementation Strategy
1. Native-core-first, then pure-stdlib expansion.
2. Use CPython sources as implementation references:
   - `Modules/*.c`
   - `Objects/*.c`
   - `Lib/*.py`
3. Prefer official pure-Python stdlib modules for high-level behavior.
4. Keep native VM stdlib code as accelerator/runtime substrate only.

### Milestone 13 Workstreams
- Runtime/native core parity:
  - `_io` semantic closure required by stdlib
  - `_csv` parity for `Lib/csv.py`
  - `_sre` parity for `Lib/re/*`
  - `_pickle` and object reduction protocol parity for `Lib/pickle.py`
  - object-model protocol closure (`__bool__`/`__len__` truthiness and core membership fallback order landed; long-tail slot/error edges pending)
- Pure-stdlib handoff:
  - Make CPython pure modules primary where available
  - Remove compatibility shims once corresponding native core is sufficient
- Test gates:
  - Fast loops: targeted/unit + curated harness
  - Strict stdlib: opt-in locally, mandatory in parity-gate profiles
  - Deferred pickle lane: tracked until full closure
- Robustness/performance proof:
  - malformed-input differential tests
  - benchmark deltas and hotspot profiling for blockers
  - landed optimization baseline: CPython-style fast-locals design (`f_localsplus`-like slot-backed locals with lazy dict sync), object-backed function-call dispatch (no full `FunctionObject` clone in opcode call paths), per-site `LOAD_GLOBAL` inline caching with namespace-version guards, arity-2/arity-3 call-path specialization, and release build tuning (`lto = "thin"`, `codegen-units = 1`)

## Milestone 14 (Performance and Architecture)
Deliverables:
- Hot-path algorithmic closure for runtime containers and VM dispatch paths.
- Clone/allocation discipline closure for hot code paths.
- Continued decomposition of large VM/runtime modules into focused components.
- Benchmark and observability gates integrated into CI policy.

## Milestone 15 (Extension Ecosystem)
Deliverables:
- Limited C-API/abi3 execution path for supported extension surfaces.
- HPy path and compatibility matrix.
- Extension-backed ecosystem smoke suites and explicit unsupported-surface diagnostics.

## Milestone 16 (Release Hardening)
Deliverables:
- Security/reliability CI gates on release branches.
- Cross-platform qualification matrix (Linux/macOS/Windows).
- Reproducible/signed artifacts and release playbook.

## Operating Rules
- Commit in small focused checkpoints.
- Do not keep long-lived dirty worktrees.
- Update docs in the same checkpoint as behavior changes.
- Keep this file as plan/state, not history log.

## Canonical Companion Docs
- `docs/PRODUCTION_READINESS.md`: global production checklist and blocker status.
- `docs/STUB_ACCOUNTING.md`: explicit partial/stub ledger and closure ownership.
- `docs/STDLIB_MIGRATION_PLAN.md`: pure-stdlib migration policy.
- `docs/ENGINEERING_GATES.md`: mandatory quality gates.
- `docs/ALGO_AUDIT_BACKLOG.md`: algorithmic/semantic audit tasks.
- `docs/OPTIMIZATION_BACKLOG.md`: permanent optimization checklist and status ledger.
