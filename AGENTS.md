# Project Context: Python Interpreter in Rust

## Vision
Build a production‑grade Python interpreter in Rust with **full source and bytecode compatibility** with **CPython 3.14**, designed for long‑term evolution (JIT and extension support possible later), and minimal third‑party dependencies.

## Target Compatibility
- **Python version:** CPython **3.14**
- **Compatibility goals (now):**
  - Run Python source code.
  - Execute CPython 3.14 bytecode.
- **Non‑goals (now):**
  - JIT compilation.
  - Full CPython C‑API compatibility / C‑extensions.
- **Future‑friendly design:**
  - Keep architecture flexible so **JIT** and **extension support** can be added later.
  - Expect that popular packages (e.g., NumPy) typically need C‑extension support; plan for this later.

## Parsing
- Use CPython 3.14 grammar with a **packrat parser**.
- Grammar reference: https://docs.python.org/3/reference/grammar.html
- **Minimize dependencies**: prefer small, well‑scoped crates or bespoke implementations.

## Compiler & Execution Pipeline
- **AST → Bytecode IR** (CPython‑compatible bytecode).
- Bytecode IR should support **light optimizations** while preserving semantics.
- Design IR with a path to **JIT** in the future.

## Runtime Model
- Match CPython object model.
- **Reference‑counted GC** similar to CPython.
- **GIL** (single‑threaded execution model for now).
- GIL‑free execution is **not** a current goal.

## Standard Library
- Start with a **practical subset** of stdlib.
- Expand toward full coverage over time.

## Success Criteria
We measure success by:
- Running **real‑world apps**.
- Passing **CPython test suites** (as compatible as possible).
- Competitive **performance benchmarks**.

## Milestone Sketch (Iterative)
1. Minimal language + parser + AST + evaluator.
2. Core features: control flow, exceptions, classes, modules.
3. Bytecode interpreter with increasing CPython compatibility.
4. Stdlib expansion + test suite integration.
5. Performance profiling + tooling + hardening.

## Engineering Principles
- Maintain correctness first, then performance.
- Keep dependencies minimal and well‑justified.
- Prefer clear architecture boundaries: parser, AST, compiler/IR, VM, runtime, stdlib.

## Project Artifacts
- Roadmap: `docs/ROADMAP.md`
- Compatibility tracker: `docs/COMPATIBILITY.md`
- CPython vendor sync script: `scripts/sync_cpython.py`

## Current Scaffolding (Early Stage)
- Parser: packrat-style memoization with a minimal lexer, indentation tokens, `if`/`else`/`while`, function defs, returns, and calls.
- Bytecode: minimal opcodes for constants/names + metadata loader (`opcode_table.csv`).
- Compiler/VM: emits and executes bytecode for `pass`, assignments, literals (`True`, `False`, `None`), unary minus, binary ops (`+`, `-`, `*`), comparisons (`==`, `<`), `if/else`, `while`, and simple functions (positional params only).
- `.pyc` header parser stub (hash-based and timestamp-based variants).
- Tests: parser smoke tests, bytecode metadata loader test, and pyc header tests.
- CLI: `--ast` and `--bytecode` flags to inspect parsed AST and bytecode.
