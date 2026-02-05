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
- Support reading and writing .pyc files (reader in place; writer TBD).
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

## Milestones
1. Milestone 0: Parser + AST + minimal evaluator.
2. Milestone 1: Core language features and module system.
3. Milestone 2: Bytecode VM with CPython-compatible opcodes.
4. Milestone 3: Stdlib expansion + CPython test suite integration.
5. Milestone 4: Performance profiling + tooling and hardening.

## Production Readiness Checklist (Living)
Status flags: `[ ]` not started, `[x]` complete.

### P0 (Production Blocking)
- [x] Object identity + stable headers (`id`, `is` semantics).
- [x] Reference counting + cycle GC.
- [x] CPython opcode table decoder (3.14).
- [ ] CPython opcode encoder (3.14).
- [ ] `.pyc` load/serialize parity with CPython 3.14 (subset implemented).
- [ ] Closures + `nonlocal` (cell/free vars).
- [ ] Generators (`yield`, `yield from`) + protocol.
- [ ] Tracebacks + accurate frames (file/line/col).
- [ ] Import system parity (`importlib`, specs, hooks).

### P1 (Major Ecosystem Enablers)
- [ ] Async/await + async generators.
- [ ] Comprehensions with correct scoping.
- [ ] Pattern matching (`match`/`case`).
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

### Milestone 4 — Generators & Iteration (P0)
DoD:
- `yield` and `yield from` match CPython for basic cases.
- Generator `send`/`throw`/`close` behave correctly.
- `for` loops iterate over generators and custom iterators.

### Milestone 5 — Import System Parity (P0)
DoD:
- `importlib` can import pure-Python stdlib modules.
- `sys.path`, `sys.meta_path`, `sys.path_hooks` are functional.
- Packages with `__init__.py` and submodules load correctly.
- `__spec__`, `__package__`, `__loader__` populated.

### Milestone 6 — P1 Language Features
DoD:
- Comprehensions (list/dict/set/gen) with correct scoping.
- Pattern matching parses and executes core patterns.
- Exception chaining semantics match CPython.

### Milestone 7 — Async & Concurrency (P1)
DoD:
- `async def`, `await`, `async for`, `async with` work with a minimal loop.
- Async generators conform to protocol.
- Basic `asyncio` tasks can run simple coroutines.

### Milestone 8 — Stdlib Core (P1)
DoD:
- `sys`, `types`, `inspect`, `io` minimally functional.
- `os`, `pathlib`, `re`, `json`, `datetime`, `collections`, `math` run basics.
- Pure-Python package installs can execute (no C-extensions).

### Milestone 9 — Performance Baseline (P2)
DoD:
- Peephole optimization pass implemented.
- Attribute lookup cache measurable in microbench.
- Baseline benchmark suite established.

### Milestone 10 — Testing & Hardening (P2)
DoD:
- CPython `Lib/test` subset runner.
- ≥ 500 tests passing in CI or local harness.
- Crash-free on curated real-world scripts.

### Milestone 11 — Ecosystem Reach (P3)
DoD:
- ABI-stable extension plan documented.
- JIT hooks documented in IR + VM pipeline.

## Immediate next steps
- Create crate layout for parser, AST, compiler, VM, runtime, stdlib, CLI.
- Add a vendor area for CPython 3.14 grammar and opcode metadata.
- Set up a minimal test harness for parser and bytecode tests.

## Testing Focus Note
After Milestone 2 (CPython bytecode compatibility), prioritize a testing push:
- CPython `Lib/test` subset harness.
- Property/fuzz tests for parser + VM semantics.
- Integration tests with real scripts and package layouts.

Status:
- CPython `Lib/test` subset harness stub in place (ignored by default; set `PYRS_CPYTHON_LIB`).
- Property/fuzz tests added for arithmetic expressions.
- Integration test added for multi-module package execution.
