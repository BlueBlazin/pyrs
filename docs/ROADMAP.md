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
8. Milestone 7: Language surface foundations (major syntax/compiler support for modern Python constructs, P0). (complete)
9. Milestone 8: Runtime data model semantics (descriptor protocol, attribute model hooks, MRO/super, exception chaining, P0). (complete)
10. Milestone 9: Core runtime types + builtins + stdlib bootstrap required for real apps (P0/P1). (complete)
11. Milestone 10: Async/concurrency/runtime integration (`async`/`await`, async generators, event loop and threading semantics, P1). (complete)
12. Milestone 11: Test and parity gate (CPython harness, fuzzing, differential tests, real app suites, P0/P1). (complete)
13. Milestone 12: Core CPython parity closure for current harness-owned P0 gaps (complete, P0).
14. Milestone 13: Remaining language/runtime long-tail parity + stdlib/packaging usability closure for pure-Python ecosystem workloads (P0/P1).
15. Milestone 14: Performance, observability, and runtime hooks (P1/P2/P3).
16. Milestone 15: Native extension ecosystem compatibility (limited C-API/abi3 + HPy path, P0/P1).
17. Milestone 16: Release hardening and production certification (security/reliability/CI/distribution gates, P0/P1).

## Production Readiness Checklist (Living)
Canonical checklist lives in `docs/PRODUCTION_READINESS.md`. The list below is a snapshot of P0-P3 items we are actively tracking in the roadmap.
Status flags: `[ ]` not started, `[x]` complete.

### P0 (Production Blocking)
- [x] Object identity + stable headers (`id`, `is` semantics).
- [x] Reference counting + cycle GC.
- [ ] Full tokenizer/grammar parity for CPython 3.14 language surface.
- [x] CPython opcode table decoder (3.14).
- [x] CPython opcode translation hardening for supported paths (fail-fast unsupported opcodes, no silent fallback behavior).
- [x] `.pyc` load/serialize parity for supported code-object subset (header + marshal reader/writer).
- [ ] Full CPython opcode encode/decode parity for all 3.14 opcode families.
- [x] Closures + `nonlocal` (cell/free vars).
- [x] Generators (`yield`, `yield from`) + protocol (lazy suspension/resume + delegation semantics implemented).
- [x] Tracebacks + accurate frames (file/line/col).
- [x] Import system parity for supported pure-Python scenarios (`importlib`, specs, hooks).
- [x] Curated CPython harness parity closure for current language/import suites (allowlist burn-down to zero).
- [ ] Runtime semantic closure for remaining data-model gaps (`__getattribute__`, metaclass precedence, `__slots__` layout edges, full codecs behavior).
- [ ] Native extension loading parity for limited C-API/abi3 modules.
- [ ] Production release gate (security + reliability): sanitizers, deterministic crash repros, and parity-regression blocking CI.

### P1 (Major Ecosystem Enablers)
- [x] Async/await + async generators (core coroutine protocol/runtime + async iteration/context-manager semantics implemented).
- [x] Comprehensions with correct scoping.
- [~] Pattern matching (`match`/`case`) core subset (literal/capture/guard) implemented; full families pending.
- [x] Type annotations (parse + `__annotations__` on modules/classes/functions; eager evaluation only).
- [x] Exception chaining (`__cause__`, `__context__`, suppression metadata) for explicit/implicit raises.
- [~] Descriptor protocol + attribute lookup parity (descriptor hooks plus `__getattribute__` override baseline and `__getattr__`/`__setattr__`/`__delattr__` implemented; full `__getattribute__` fallback-edge and metaclass parity pending).
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

### Milestone 7 — Language Surface Foundations (P0)
DoD:
- Tokenizer/grammar/compiler surface expanded for milestone targets: decorators, assignment expressions (`:=`), list/dict comprehensions and generator expressions (with scope isolation), `match`/`case` (core literal/capture/guard subset), async syntax (`async def`/`await`/`async for`/`async with`), `except*` parsing, type parameters on functions/classes, and f-string lowering.
- `__future__` import placement/unknown-feature gating checks are enforced at compile time.
- Targeted parser/VM regression tests cover all newly added language-surface features.
Status: complete
Notes:
- Core coroutine/event-loop semantics are implemented in Milestone 10.
- Full tokenizer/grammar parity, full `ExceptionGroup` splitting semantics, and full PEP 701 formatting edge cases are tracked under Milestone 13 and production-readiness checklist items.

### Milestone 8 — Runtime Data Model Parity (P0)
DoD:
- Descriptor protocol foundations are implemented in the VM for data and non-data descriptors (`__get__`, `__set__`, `__delete__`) in class/instance attribute paths.
- Attribute model hooks are wired for supported scenarios: `__getattr__`, `__setattr__`, and `__delattr__` on instances, plus builtin `getattr`/`setattr`/`delattr`/`hasattr`.
- C3 MRO computation is implemented for class creation with `__mro__`/`__bases__` metadata and `super(type, obj)` lookup support.
- Exception chaining/context semantics are implemented for `raise ... from ...` and implicit chaining (`__cause__`, `__context__`, `__suppress_context__`).
Status: complete
Notes:
- Full metaclass precedence/selection semantics and custom metaclass class-object call edge cases are tracked in Milestone 13 parity closure.
- Core builtin type parity (`bytes`/`set`/`float`/unicode codecs foundations) is delivered in Milestone 9.

### Milestone 9 — Builtins + Stdlib Bootstrap (P0/P1)
DoD:
- Core runtime builtin type parity required by stdlib foundations (`set`, `frozenset`, `bytes`, `bytearray`, `memoryview`, `float`, `complex`, unicode/codecs behavior), plus remaining data-model gaps (`metaclass` path and `__slots__` core behavior).
- Builtins required by stdlib and common apps are present with correct semantics.
- Foundational stdlib modules are usable: `sys`, `types`, `inspect`, `io`, `os`, `pathlib`, `time`, `datetime`, `collections`, `math`, `re`, `json`, `codecs`, `functools`, `itertools`, `operator`, `random`.
- Pure-Python package installation/execution works across curated package classes (CLI, web, and data-style workloads) for the non-C-extension scope.
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
- Async generators and coroutine protocol behavior match CPython for milestone-scope features.
- Core runtime support for `asyncio` basic task scheduling exists; threading + signals semantics are implemented to required compatibility level.
Status: complete
Progress:
- Coroutine runtime semantics landed for `async def` and `await` via dedicated `GET_AWAITABLE` VM handling and coroutine-aware generator objects (`__await__`, `inspect.iscoroutine`, `inspect.isawaitable`).
- Async iteration/context-manager flows are operational: `aiter`/`anext` builtins, `StopAsyncIteration` exception wiring, async-generator protocol hooks (`__aiter__`, `__anext__`), and `async for`/`async with` execution paths.
- Builtin stdlib foundations for `asyncio` (`run`, `sleep`, `create_task`, `gather`), `threading` (`get_ident`, `current_thread`, `main_thread`, `active_count`), and `signal` (`signal`, `getsignal`, `raise_signal`) are integrated and covered by VM regression tests.

### Milestone 11 — Testing and Parity Gate (P0/P1)
DoD:
- CPython `Lib/test` harness is first-class (not ignored by default in CI/profile used for parity).
- Curated `Lib/test` language/import subsets pass with strict ownership tracking for any expected gaps.
- Differential tests versus CPython and parser/VM fuzzing run continuously.
- Real-world pure-Python applications pass curated smoke/regression suites.
- Residual Milestone 8/9 semantic gaps (`__getattribute__` edge parity, metaclass precedence/selection, `__slots__` layout edge cases, full codecs behavior) are either closed or explicitly scoped with failing tests and ownership.
Status: complete
Progress:
- CPython harness is now first-class and non-ignored (`tests/cpython_harness.rs`) with split suites (`tests/cpython_suite_language.txt`, `tests/cpython_suite_imports.txt`) plus strict allowlist ownership/category tracking (`tests/cpython_allowlist.txt`) and stale-allowlist detection.
- Current curated language/import harness suites pass with an empty allowlist (`tests/cpython_allowlist.txt`).
- Differential-vs-CPython coverage landed in `tests/differential_cpython.rs` for curated corpus and arithmetic fuzz expressions.
- Parser/compiler/VM no-panic fuzzing landed in `tests/fuzz_parser_vm.rs`.
- Curated real-world smoke coverage landed in `tests/realworld_smoke.rs` and is executed in a constrained subprocess profile (`env_clear`, isolated temp cwd/home, and timeout enforcement).
- Milestone parity profile is codified in `scripts/run_parity_gate.sh` (`PYRS_PARITY_STRICT=1`) and currently passes.

### Milestone 12 — Core CPython Parity Closure (P0)
DoD:
- Close all harness-owned parser/runtime/stdlib allowlist gaps for current curated CPython language/import suites.
- Land parser/compiler/runtime fixes plus targeted regressions required to run those suites with no expected-failure allowlist entries.
- Keep parity profile (`scripts/run_parity_gate.sh`) green after closure.
- Record explicit carry-forward ownership for any remaining long-tail parity work.
Status: complete
Progress:
- CPython harness allowlist has been burned down to zero (`tests/cpython_allowlist.txt`).
- Curated CPython language/import suites pass under `tests/cpython_harness.rs`.
- Runtime and stdlib compatibility closure landed for harness blockers (plus broad regression additions in `tests/vm.rs` and `tests/parser.rs`).
- Parity profile script remains green.
- Baseline CI parity workflow is present (`.github/workflows/parity-gate.yml`) and runs full tests plus parity profile against CPython 3.14.3 `Lib`.
Execution record:
- Milestone closure details are tracked in `docs/MILESTONE_12_BACKLOG.md`.

### Milestone 13 — Long-Tail Parity + Stdlib/Packaging Usability Closure (P0/P1)
DoD:
- Remaining language/runtime long-tail parity closes for production blocking semantics (`__getattribute__`, metaclass precedence/selection, `__slots__` edge layout behavior, full codecs behavior, pattern-family completion, `ExceptionGroup` split semantics, and full PEP 701 behavior).
- Stdlib coverage expands to unblock mainstream pure-Python ecosystems (`subprocess`, `socket`, `ssl`, `http`, `urllib`, `typing`, `dataclasses`, `enum`, `contextvars`, and importlib resource/package helpers).
- Import and packaging paths close remaining usability gaps (zip/bytecode imports, `importlib.resources`, `pkgutil`, and `site` startup behavior).
- venv + pip pure-Python package workflows are production-usable and covered by regression tests.
- Real-world smoke/regression matrix includes CLI/web/data pure-Python app classes running under constrained sandbox profile.
- `NoOp` and non-`NoOp` partial implementation ledgers stay current (`docs/NOOP_BUILTIN_INVENTORY.txt` + `docs/STUB_ACCOUNTING.md`) with CI drift checks.
Progress:
- Metaclass/runtime parity batch landed: resolved-metaclass tracking on class objects, metaclass conflict detection across base classes, and metaclass attribute fallback for class-object attribute access.
- Class-instance data-model parity batch landed for slot edges: empty `__slots__` classes block dynamic attributes, and `__slots__ = ('__dict__',)` enables dynamic attribute assignment.
- Codec parity batch expanded foundations to include `utf-32`, `utf-32-le`, and `utf-32-be` encode/decode paths with existing error-mode handling.
- Exception-parity batch landed for `except` handlers over user-defined classes: tuple handlers and subclass matching now work with runtime exception-parent tracking.
- `os`/`posix` parity batch landed for core filesystem/process helpers: `open`, `close`, `isatty`, `stat`, `lstat`, `rmdir`, `utime`, `scandir`, and wait-status helper functions (`WIF*`/`W*SIG`/`WEXITSTATUS`) now execute real logic instead of `NoOp`.
- `math` parity batch landed for previously stubbed numeric helpers: `ldexp`, `hypot`, `fabs`, `exp`, `erfc`, `log`, `fsum`, `sumprod`, `cos`, `sin`, `tan`, `cosh`, `asin`, `atan`, `acos`, and `isclose` now execute non-`NoOp` logic with new regression tests.
- `operator`/`functools` parity batch landed for callable adapters: `operator.itemgetter`, `operator.attrgetter`, `operator.methodcaller`, and `functools.cmp_to_key` now execute non-`NoOp` logic (including `sorted`/`min`/`max` key-ordering interoperability), with dedicated VM regressions.
- `itertools` parity batch landed for previously stubbed helpers: `accumulate`, `combinations`, `combinations_with_replacement`, `compress`, `dropwhile`, `filterfalse`, `groupby`, `islice`, `pairwise`, `starmap`, `takewhile`, `tee`, and `zip_longest` now execute non-`NoOp` logic with dedicated VM regressions.
- `inspect` parity batch landed for core signature introspection: `inspect.signature` now executes a non-`NoOp` path and returns an `inspect.Signature` instance with baseline parameter-kind/default metadata and return-annotation capture.
- `importlib` parity batch landed for cache/spec helpers: `importlib.invalidate_caches` and baseline `importlib.util.spec_from_file_location` now execute non-`NoOp` logic with dedicated VM regressions; full spec/loader object parity remains tracked in Milestone 13.
- Regression coverage added for all above behaviors in `tests/vm.rs`; full suite and parity gate remain green.

### Milestone 14 — Performance, Observability, and Runtime Hooks (P1/P2/P3)
DoD:
- Baseline benchmark suite (including pyperformance subset plus project workloads) is automated and tracked for regressions.
- Production optimization stack is implemented and validated (compiler-level simplifications, VM dispatch/lookup caches, and container/runtime hot-path improvements), with parity gates proving no semantic regressions.
- Debug and profiling hooks (`sys.settrace`, `sys.setprofile`, runtime metrics, profiling workflow) are operational.
- JIT/embedding hook points in IR/VM boundaries are explicitly documented and covered by non-regression tests.

### Milestone 15 — Native Extension Ecosystem Compatibility (P0/P1)
DoD:
- Limited C-API/abi3 extension loading works across the supported API surface (import, type/object creation, method dispatch, memory/error propagation) with compatibility tests per surface.
- HPy execution path is available with explicit compatibility matrix and tests.
- Ecosystem smoke/regression tests cover multiple extension-backed package classes (numeric, crypto, parsing, and data-processing families) with documented pass/fail matrix and blockers.
- Any unsupported extension surfaces fail with explicit diagnostics, not silent misbehavior.

### Milestone 16 — Release Hardening and Production Certification (P0/P1)
DoD:
- Security/reliability CI gates are mandatory for release branches (ASan/UBSan, differential CPython runs, crash reproducer retention).
- Cross-platform qualification gates pass on Linux, macOS, and Windows for defined compatibility suite.
- Release pipeline produces reproducible and signed artifacts with SBOM generation and semantic-versioning policy.
- Production playbook exists for incident triage, rollback strategy, reproducible artifact verification, and regression-response SLA.

## Immediate next steps
- Execute Milestone 13 closure batches: long-tail language/runtime parity semantics plus stdlib/import/packaging usability blockers.
- Expand CPython harness breadth beyond current curated suites while preserving empty-allowlist policy for in-scope tests.
- Harden CI parity policy beyond baseline workflow (`.github/workflows/parity-gate.yml`) for release-branch blocking and multi-platform parity lanes.
- Keep Milestone 15 and Milestone 16 acceptance criteria visible during architecture choices so extension and release hardening paths remain unblocked.

## Testing Focus Note
After Milestone 2 (CPython bytecode compatibility), prioritize a testing push:
- CPython `Lib/test` subset harness.
- Property/fuzz tests for parser + VM semantics.
- Integration tests with real scripts and package layouts.

Status:
- CPython `Lib/test` subset harness is active by default (set `PYRS_CPYTHON_LIB` to point at CPython `Lib`; set `PYRS_CPYTHON_OPTIONAL=1` only when intentionally skipping on machines without local CPython sources).
- Differential testing is active (`tests/differential_cpython.rs`) and parser/VM fuzzing is active (`tests/fuzz_parser_vm.rs`).
- Curated real-world smoke tests are active and execute in a constrained subprocess profile (`tests/realworld_smoke.rs`).
