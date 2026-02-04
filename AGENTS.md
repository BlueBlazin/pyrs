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
- Vendor snapshot: CPython 3.14.3 grammar + opcode sources synced into `vendor/cpython-3.14/` (opcode table CSV still pending).

## Current Scaffolding (Early Stage)
- Parser: packrat-style memoization with a minimal lexer, indentation tokens, `if`/`elif`/`else`/`while`/`for`, `break`/`continue`, function defs, returns, and calls.
- Bytecode: minimal opcodes for constants/names + metadata loader (`opcode_table.csv`).
- Compiler/VM: emits and executes bytecode for `pass`, assignments (including name-based subscripts with negative index assignment, module attribute assignment, and augmented `+=`, `-=`, `*=`), literals (`True`, `False`, `None`), unary ops (`+`, `-`, `not`), binary ops (`+`, `-`, `*`, `//`, `%`), comparisons (`==`, `!=`, `<`, `<=`, `>`, `>=`, `in`, `not in`, `is`, `is not`), boolean ops (`and`, `or`), conditional expressions (`a if cond else b`), lambdas, `if/else`/`elif`, `while/for` (with `else` clauses), `break`/`continue`, `raise`, `assert`, `try/except/else`, `try/finally` (but not combined), simple functions (positional params only), basic class definitions with single inheritance, instance attributes and bound methods, list/tuple/dict literals, subscripts with negative indexing plus slicing for list/tuple/str, attribute access (`module.attr`, `instance.attr`, `Class.attr`), and basic `import` / `from ... import ...` statements with optional `as` aliases (simple module names). Builtins: `print`, `len` (strings, lists, tuples, dicts), `range` (1-3 args), `slice`, `bool`, `int`, `str`, `abs`, `sum`, `min`, `max`, `all`, `any`, `pow`, `list`, `tuple`, `divmod`, `sorted`, and basic exception types (`Exception`, `ValueError`, `TypeError`, `IndexError`, `KeyError`, `AssertionError`, `RuntimeError`, `NameError`, `AttributeError`, `ZeroDivisionError`).
- Exceptions: `try/except` handles explicit `raise`; VM runtime errors are mapped to coarse exception types (`RuntimeError`, `TypeError`, `IndexError`, `KeyError`, `ZeroDivisionError`, `NameError`, `AttributeError`) based on message heuristics.
- Exceptions: `try/finally` supported; `try/except/finally` not yet.
- Identity: `is`/`is not` currently reuse `Value` equality (no stable object identity yet).
- Classes: class bodies execute in a class namespace module while resolving missing names against the defining module; methods capture the defining module as globals.
- TODO: generate `vendor/cpython-3.14/opcode/opcode_table.csv` from synced opcode sources.
- Modules: new `Value::Module` with per-module globals; VM maintains module cache and search paths (default CWD, configurable via `Vm::add_module_path`). Import loads `<name>.py` into a module frame, returning module objects; functions capture defining module globals.
- Numeric compatibility: `bool` participates in int arithmetic/comparisons (`True == 1`, `True + 1`, etc.).
- Scoping: `global` statements supported inside functions; assignments to globals emit `StoreGlobal`.
- `.pyc` header parser stub (hash-based and timestamp-based variants).
- Tests: parser smoke tests, bytecode metadata loader test, and pyc header tests.
- CLI: `--ast` and `--bytecode` flags to inspect parsed AST and bytecode.
