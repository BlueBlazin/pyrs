# Design and Roadmap

## Summary
We are building a production-grade Python interpreter in Rust with full source and CPython 3.14 bytecode compatibility. The system should be correct first, fast second, and architected so JIT and extension support remain feasible later.

## Goals
- Run Python source code with CPython 3.14 semantics.
- Execute CPython 3.14 bytecode (.pyc).
- Be usable for real-world apps, not just toy programs.
- Keep dependencies small and well-justified.

## Non-goals (now)
- JIT compilation.
- Full CPython C-API compatibility and C-extensions (deferred until Milestone 15).
- GIL-free runtime.

## Future-friendly constraints
- IR and VM boundaries should allow a JIT tier later.
- Runtime object model should be compatible with extension support later.

## High-level architecture
- Parser (packrat) -> AST -> Compiler -> Bytecode IR -> VM -> Runtime
- Standard library implemented incrementally.

## Parsing
- Implement CPython 3.14 grammar as a packrat parser.
- Vendor the grammar into the repo for stability and reproducibility.
- Keep parser dependencies minimal, prefer bespoke code for core parsing.

## Bytecode compatibility
- Target CPython 3.14 bytecode format and semantics.
- Maintain an opcode table in-repo and generate decoder/encoder tables.
- Support reading and writing .pyc files (supported-subset writer now in place).
- Keep stack effect metadata in one place to enable verification and tooling.

## Compiler and IR
- AST -> Bytecode IR that mirrors CPython behavior.
- Allow light optimizations that do not change observable semantics.
- Preserve debug and location info for tracebacks and tooling.

## Runtime model
- CPython-compatible object model.
- Reference-counted memory management with cycle detection where needed.
- GIL for correctness and simplicity.

## Standard library
- Start with a practical subset needed for common apps.
- Track progress by module and feature tests.
- Gradually expand toward full coverage.

## Dependency policy
- Add dependencies only with clear justification.
- Prefer small crates and isolate them behind internal APIs.
- Avoid heavy parser generators if they add significant weight.

## Testing strategy
- Golden tests for parser and AST.
- Bytecode round-trip tests and opcode-level unit tests.
- Integrate CPython test suite incrementally with a compatibility tracker.
- Real-world app smoke tests.

## Milestones (Revised for 100% CPython 3.14)
Acceptance rule for all remaining milestones: no milestone is complete at "basic compat"; completion requires behavior-level parity for in-scope features plus targeted CPython tests.
Release-complete target: Milestone 16; ecosystem-complete target (including native-extension packages): Milestone 15 + Milestone 16.

1. Milestone 0: Parser + AST + minimal evaluator. (complete)
2. Milestone 1: Runtime core + identity + GC foundations. (complete)
3. Milestone 2: CPython bytecode intake foundations (`opcode_table.csv`, marshal reader, `.pyc` load + execution subset). (complete)
4. Milestone 3: Closures + frame metadata + traceback foundations. (complete)
5. Milestone 4: Generator and iteration parity (complete, P0).
6. Milestone 5: Opcode execution hardening + `.pyc` read/write parity for supported bytecode paths (complete, P0).
7. Milestone 6: Import system parity (`importlib`/`ModuleSpec`/hooks/packages, P0). (complete)
8. Milestone 7: Full language surface parity (tokenizer + grammar + compiler semantics, P0). (complete)
9. Milestone 8: Runtime data model semantics (descriptor protocol, attribute model hooks, MRO/super, exception chaining, P0). (complete)
10. Milestone 9: Core runtime types + builtins + stdlib bootstrap required for real apps (P0/P1). (complete)
11. Milestone 10: Async/concurrency/runtime integration (`async`/`await`, async generators, event loop and threading semantics, P1).
12. Milestone 11: Test and parity gate (CPython harness, fuzzing, differential tests, real app suites, P0/P1).
13. Milestone 12: Performance and observability baseline (P2).
14. Milestone 13: Packaging/distribution and ecosystem usability (P1/P2).
15. Milestone 14: Future hooks and extension-path architecture documentation (P3).
16. Milestone 15: Native extension ecosystem compatibility (limited C-API/abi3 + HPy path, P1 with P0 ecosystem parity gate).
17. Milestone 16: Release hardening and production certification (security/reliability/CI gates, P0/P1).

## Production Readiness Checklist (Living)
Canonical checklist lives in `docs/PRODUCTION_READINESS.md`. The list below is a snapshot of P0-P3 items we are actively tracking in the roadmap.
Status flags: `[ ]` not started, `[x]` complete.

### P0 (Production Blocking)
- [x] Object identity + stable headers (`id`, `is` semantics).
- [x] Reference counting + cycle GC.
- [x] CPython opcode table decoder (3.14).
- [x] CPython opcode translation hardening for supported paths (fail-fast unsupported opcodes, no silent fallback behavior).
- [x] `.pyc` load/serialize parity for supported code-object subset (header + marshal reader/writer).
- [ ] Full CPython opcode encode/decode parity for all 3.14 opcode families.
- [x] Closures + `nonlocal` (cell/free vars).
- [x] Generators (`yield`, `yield from`) + protocol (lazy suspension/resume + delegation semantics implemented).
- [x] Tracebacks + accurate frames (file/line/col).
- [x] Import system parity for supported pure-Python scenarios (`importlib`, specs, hooks).
- [ ] Native extension loading parity for limited C-API/abi3 modules.
- [ ] Production release gate (security + reliability): sanitizers, deterministic crash repros, and parity-regression blocking CI.

### P1 (Major Ecosystem Enablers)
- [ ] Async/await + async generators.
- [x] Comprehensions with correct scoping.
- [~] Pattern matching (`match`/`case`) core subset (literal/capture/guard) implemented; full families pending.
- [x] Type annotations (parse + `__annotations__` on modules/classes/functions; eager evaluation only).
- [x] Exception chaining (`__cause__`, `__context__`, suppression metadata) for explicit/implicit raises.
- [~] Descriptor protocol + attribute lookup parity (descriptor hooks plus `__getattr__`/`__setattr__`/`__delattr__` implemented; full `__getattribute__`/metaclass parity pending).
- [~] Core stdlib: `sys`, `types`, `inspect`, `io`.
- [~] Stdlib base: `os`, `pathlib`, `re`, `json`, `datetime`, `collections`, `math`, `codecs`.
- [~] Utility stdlib foundations: `random`.
- [ ] HPy extension loading/execution path.
- [ ] Cross-platform release qualification matrix (Linux/macOS/Windows) with parity gates.

### P2 (Performance & QoL)
- [ ] Peephole / constant-folding bytecode optimizations.
- [ ] Attribute lookup caches.
- [ ] Efficient list/tuple/dict internals.
- [ ] Stable REPL + improved error messages.
- [ ] CPython `Lib/test` subset runner.

### P3 (Future-Proofing)
- [ ] JIT hooks in IR/VM boundaries (no implementation).
- [ ] Debug hooks (`sys.settrace`, `sys.setprofile`).
- [ ] Profiling/benchmark harness.

## Milestone Definitions of Done (DoD)

### Milestone 1 — Runtime Core & Identity (P0)
DoD:
- `id()` returns stable values across object lifetime.
- `is`/`is not` are identity-based.
- Refcounting correct for all objects; deterministic dealloc in tests.
- Cycle GC handles self-referential containers.
Status: complete

### Milestone 2 — CPython Bytecode Compatibility (P0)
DoD:
- `opcode_table.csv` generated and used (3.14).
 - `.pyc` loader can execute CPython-compiled bytecode for a basic module.
 - Stack effects + jumps match CPython for supported opcode subset (calls, attr load/store, loops, class/function defs).
 - Marshal reader + translator cover the subset needed for smoke tests.
Status: complete

### Milestone 3 — Closures & Frames (P0)
DoD:
- `nonlocal` works in nested functions.
- Cells/free vars capture correctly across calls.
- Tracebacks show filename/line/column and frame names.
- `locals()`/`globals()` reflect correct scopes.
Status: complete

### Milestone 4 — Generators & Iteration Parity (P0)
DoD:
- Generators store suspended execution state (frame, instruction pointer, value stack, blocks), not precomputed output vectors.
- `send`, `throw`, and `close` semantics match CPython, including `GeneratorExit` and `finally` handling.
- `yield from` delegation semantics match CPython, including propagation of `StopIteration.value`.
- Reentrancy and terminal-state errors match CPython (`generator already executing`, post-close behavior, etc.).
- Targeted CPython generator tests pass (or are explicitly documented as blocked by out-of-scope work).
Status: complete

### Milestone 5 — Opcode Hardening & `.pyc` Writer Parity (P0)
DoD:
- All required CPython opcodes for the supported source/bytecode surface execute with correct semantics; no silent fallback `Nop` in supported code paths (unsupported opcodes fail translation explicitly).
- Stack-effect and jump validation checks are implemented for decode/translation.
- `.pyc` writer implemented with CPython-compatible headers and marshal output for supported code objects.
- CPython-compiled pure-Python modules execute end-to-end through the `.pyc` path.
Status: complete

### Milestone 6 — Import System Parity (P0)
DoD:
- `importlib` machinery is functional for pure-Python modules/packages.
- `sys.path`, `sys.meta_path`, `sys.path_hooks`, and loader/finder contracts work.
- `ModuleSpec` fields and module metadata (`__spec__`, `__package__`, `__loader__`, `__path__`) are populated correctly.
- Relative imports and namespace package behavior match CPython for supported scenarios.
Status: complete

### Milestone 7 — Language Surface Parity (P0)
DoD:
- Tokenizer/grammar/compiler surface expanded for milestone targets: decorators, assignment expressions (`:=`), list/dict comprehensions and generator expressions (with scope isolation), `match`/`case` (core literal/capture/guard subset), async syntax (`async def`/`await`/`async for`/`async with`), `except*` parsing, type parameters on functions/classes, and f-string lowering.
- `__future__` import placement/unknown-feature gating checks are enforced at compile time.
- Targeted parser/VM regression tests cover all newly added language-surface features.
Status: complete
Notes:
- Full coroutine/event-loop semantics remain in Milestone 10.
- Full `ExceptionGroup` splitting semantics and full PEP 701 formatting edge cases remain tracked under Milestones 8-10 and production-readiness checklist items.

### Milestone 8 — Runtime Data Model Parity (P0)
DoD:
- Descriptor protocol foundations are implemented in the VM for data and non-data descriptors (`__get__`, `__set__`, `__delete__`) in class/instance attribute paths.
- Attribute model hooks are wired for supported scenarios: `__getattr__`, `__setattr__`, and `__delattr__` on instances, plus builtin `getattr`/`setattr`/`delattr`/`hasattr`.
- C3 MRO computation is implemented for class creation with `__mro__`/`__bases__` metadata and `super(type, obj)` lookup support.
- Exception chaining/context semantics are implemented for `raise ... from ...` and implicit chaining (`__cause__`, `__context__`, `__suppress_context__`).
Status: complete
Notes:
- Full metaclass precedence/selection semantics and custom metaclass class-object call edge cases are tracked in Milestone 11 parity closure.
- Core builtin type parity (`bytes`/`set`/`float`/unicode codecs foundations) is delivered in Milestone 9.

### Milestone 9 — Builtins + Stdlib Bootstrap (P0/P1)
DoD:
- Core runtime builtin type parity required by stdlib foundations (`set`, `frozenset`, `bytes`, `bytearray`, `memoryview`, `float`, `complex`, unicode/codecs behavior), plus remaining data-model gaps (`metaclass` path and `__slots__` core behavior).
- Builtins required by stdlib and common apps are present with correct semantics.
- Foundational stdlib modules are usable: `sys`, `types`, `inspect`, `io`, `os`, `pathlib`, `time`, `datetime`, `collections`, `math`, `re`, `json`, `codecs`, `functools`, `itertools`, `operator`, `random`.
- Pure-Python package installation/execution works for representative no-C-extension packages.
Status: complete
Progress:
- Float foundations landed end-to-end: parser/AST/compiler/VM/runtime support float literals, `/`, `//`, `%`, `**`, unary `+/-`, mixed int-bool-float comparisons, and `float()` builtin conversion paths.
- CPython marshal/translation now supports float constants (`g` binary float marshal tag) for `.pyc` decode/execute flows.
- Builtin `random` module foundations landed (`seed`, `random`, `randrange`, `randint`, `getrandbits`, `choice`, `shuffle`) with deterministic seed behavior and regression tests.
- Runtime types and builtins landed for `set`/`frozenset`, `bytes`/`bytearray`/`memoryview`, `complex`, `iter`, `next`, and dynamic `type(...)` class creation with `__slots__` enforcement.
- Class header metaclass keyword path landed (`class C(metaclass=...)`) with VM frame plumbing and `__build_class__` metaclass kwargs handling.
- `codecs` stdlib foundation landed (`encode`/`decode` for `utf-8`/`ascii`/`latin-1` with `strict`/`ignore`/`replace` error modes).

### Milestone 10 — Async and Concurrency Semantics (P1)
DoD:
- `async def`/`await`/`async for`/`async with` semantics are implemented.
- Async generators and coroutine protocol behavior match CPython.
- Core runtime support for `asyncio` basic task scheduling exists; threading + signals semantics are implemented to required compatibility level.

### Milestone 11 — Testing and Parity Gate (P0/P1)
DoD:
- CPython `Lib/test` harness is first-class (not ignored by default in CI/profile used for parity).
- Broad `Lib/test` coverage passes with documented allowlist only for explicit non-goals.
- Differential tests versus CPython and parser/VM fuzzing run continuously.
- Real-world pure-Python applications pass curated smoke/regression suites.
- Residual Milestone 8/9 semantic gaps (`__getattribute__` edge parity, metaclass precedence/selection, `__slots__` layout edge cases, full codecs behavior) are either closed or explicitly scoped with failing tests and ownership.

### Milestone 12 — Performance and Observability Baseline (P2)
DoD:
- Baseline benchmark suite (including pyperformance subset) is automated.
- At least one production-relevant optimization tier lands (peephole/inlining/caches) without semantic regressions.
- Profiling and debug observability hooks (`sys.settrace`, `sys.setprofile`, runtime metrics) are operational.

### Milestone 13 — Packaging, Distribution, and Developer UX (P1/P2)
DoD:
- Distribution artifacts and reproducible builds are documented and automated.
- `site` startup behavior, venv/pip pure-Python workflows, and REPL quality are production-usable.
- Documentation and compatibility reporting are publishable for external contributors/users.

### Milestone 14 — Future Hooks (P3)
DoD:
- JIT hook points in IR/VM are explicitly documented and tested for non-regression.
- Extension strategy (HPy / limited C-API path) is documented with architectural constraints.
- Embedding API direction for Rust/C hosts is documented.

### Milestone 15 — Native Extension Ecosystem Compatibility (P1 with P0 gate)
DoD:
- Limited C-API/abi3 extension loading works for representative extension modules (import, type/object creation, method dispatch, error propagation).
- HPy execution path is available (even if partial) with explicit compatibility matrix and tests.
- Ecosystem smoke tests include at least one extension-backed package class (numeric/crypto/parsing families) with documented pass/fail matrix.
- Any unsupported extension surfaces fail with explicit diagnostics, not silent misbehavior.

### Milestone 16 — Release Hardening and Production Certification (P0/P1)
DoD:
- Security/reliability CI gates are mandatory for release branches (ASan/UBSan, differential CPython runs, crash reproducer retention).
- Cross-platform qualification gates pass on Linux, macOS, and Windows for defined compatibility suite.
- Release policy defines semantic versioning, compatibility guarantees, and regression-response SLA.
- Production playbook exists for incident triage, rollback strategy, and reproducible artifact verification.

## Immediate next steps
- Start Milestone 10 work: full coroutine runtime semantics (`async`/`await`, async iterators/generators, cancellation/finalization behavior) and event-loop integration foundations.
- Expand opcode-family coverage for remaining 3.14 domains (async, exception-table-heavy paths, and pattern-matching families) under Milestones 10-11.
- Run Milestone 11 parity closure for remaining Milestone 8/9 semantic deltas while CPython harness coverage expands.
- Continue broad CPython parity tests while landing language/runtime milestones.
- Keep Milestone 15 and Milestone 16 acceptance criteria visible during architecture choices so extension and release hardening paths remain unblocked.

## Testing Focus Note
After Milestone 2 (CPython bytecode compatibility), prioritize a testing push:
- CPython `Lib/test` subset harness.
- Property/fuzz tests for parser + VM semantics.
- Integration tests with real scripts and package layouts.

Status:
- CPython `Lib/test` subset harness stub in place (ignored by default; set `PYRS_CPYTHON_LIB`).
- Property/fuzz tests added for arithmetic expressions.
- Integration test added for multi-module package execution.
