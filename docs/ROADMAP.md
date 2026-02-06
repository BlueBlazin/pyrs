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
- Full CPython C-API compatibility and C-extensions.
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

1. Milestone 0: Parser + AST + minimal evaluator. (complete)
2. Milestone 1: Runtime core + identity + GC foundations. (complete)
3. Milestone 2: CPython bytecode intake foundations (`opcode_table.csv`, marshal reader, `.pyc` load + execution subset). (complete)
4. Milestone 3: Closures + frame metadata + traceback foundations. (complete)
5. Milestone 4: Generator and iteration parity (complete, P0).
6. Milestone 5: Opcode execution hardening + `.pyc` read/write parity for supported bytecode paths (complete, P0).
7. Milestone 6: Import system parity (`importlib`/`ModuleSpec`/hooks/packages, P0).
8. Milestone 7: Full language surface parity (tokenizer + grammar + compiler semantics, P0).
9. Milestone 8: Runtime data model parity (descriptor protocol, attribute model, metaclasses/MRO, core types, P0).
10. Milestone 9: Builtins + stdlib bootstrap required for real apps (P0/P1).
11. Milestone 10: Async/concurrency/runtime integration (`async`/`await`, async generators, event loop and threading semantics, P1).
12. Milestone 11: Test and parity gate (CPython harness, fuzzing, differential tests, real app suites, P0/P1).
13. Milestone 12: Performance and observability baseline (P2).
14. Milestone 13: Packaging/distribution and ecosystem usability (P1/P2).
15. Milestone 14: Future hooks and extension path documentation (P3).

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
- [ ] Import system parity (`importlib`, specs, hooks).

### P1 (Major Ecosystem Enablers)
- [ ] Async/await + async generators.
- [ ] Comprehensions with correct scoping.
- [ ] Pattern matching (`match`/`case`).
- [x] Type annotations (parse + `__annotations__` on modules/classes/functions; eager evaluation only).
- [ ] Exception chaining (`__cause__`, `__context__`, suppression).
- [ ] Descriptor protocol + attribute lookup parity.
- [ ] Core stdlib: `sys`, `types`, `inspect`, `io`.
- [ ] Stdlib base: `os`, `pathlib`, `re`, `json`, `datetime`, `collections`, `math`.

### P2 (Performance & QoL)
- [ ] Peephole / constant-folding bytecode optimizations.
- [ ] Attribute lookup caches.
- [ ] Efficient list/tuple/dict internals.
- [ ] Stable REPL + improved error messages.
- [ ] CPython `Lib/test` subset runner.

### P3 (Future-Proofing)
- [ ] ABI-stable extension story (HPy or limited C-API).
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

### Milestone 7 — Language Surface Parity (P0)
DoD:
- Tokenizer reaches CPython-level behavior for strings/bytes/f-strings/numeric literals/comments/indentation edge cases.
- Remaining grammar and compiler features implemented: decorators, assignment expressions, comprehensions (with correct scope isolation), pattern matching, async syntax, exception groups (`except*`), and type-parameter syntax.
- f-string behavior aligns with PEP 701 semantics for supported expressions.
- `__future__` flags and feature gating behaviors are implemented.

### Milestone 8 — Runtime Data Model Parity (P0)
DoD:
- Descriptor protocol and full attribute access semantics (`__getattribute__`, `__getattr__`, `__setattr__`, `__delattr__`) match CPython.
- MRO/metaclass/`super()`/`__slots__` behavior is CPython-compatible for core use cases.
- Core builtin types reach parity needed by stdlib foundations (`set`, `frozenset`, `bytes`, `bytearray`, `memoryview`, `float`, `complex`, unicode/codecs behavior).
- Exception chaining/context behavior (`__cause__`, `__context__`, suppression) is correct.

### Milestone 9 — Builtins + Stdlib Bootstrap (P0/P1)
DoD:
- Builtins required by stdlib and common apps are present with correct semantics.
- Foundational stdlib modules are usable: `sys`, `types`, `inspect`, `io`, `os`, `pathlib`, `time`, `datetime`, `collections`, `math`, `re`, `json`, `functools`, `itertools`, `operator`.
- Pure-Python package installation/execution works for representative no-C-extension packages.

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

## Immediate next steps
- Start Milestone 6 import-system parity (`importlib`, `ModuleSpec`, loader/finder contracts).
- Expand opcode-family coverage for remaining 3.14 domains (async, exception-table-heavy paths, and pattern-matching families) under Milestones 7-10.
- Continue broad CPython parity tests while landing import/language/runtime milestones.

## Testing Focus Note
After Milestone 2 (CPython bytecode compatibility), prioritize a testing push:
- CPython `Lib/test` subset harness.
- Property/fuzz tests for parser + VM semantics.
- Integration tests with real scripts and package layouts.

Status:
- CPython `Lib/test` subset harness stub in place (ignored by default; set `PYRS_CPYTHON_LIB`).
- Property/fuzz tests added for arithmetic expressions.
- Integration test added for multi-module package execution.
