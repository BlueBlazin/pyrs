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
- Support reading and writing .pyc files.
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

## Immediate next steps
- Create crate layout for parser, AST, compiler, VM, runtime, stdlib, CLI.
- Add a vendor area for CPython 3.14 grammar and opcode metadata.
- Set up a minimal test harness for parser and bytecode tests.
