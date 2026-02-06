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
- Object identity uses stable `id()` values; `is`/`is not` are identity-based with a basic cycle GC for self-referential containers.
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
Roadmap milestones are now maintained in `docs/ROADMAP.md` and are structured to guarantee full CPython 3.14 parity with no "basic compat" completion criteria.

Current milestone state summary:
1. Milestones 0-3 complete (foundations: parser/AST, runtime identity+GC, bytecode intake, closures+frames).
2. Milestone 4 complete (P0): lazy generator suspension/resume + `yield from` send/throw delegation + `StopIteration` return-value propagation + close/`GeneratorExit` handling + reentrancy guard coverage.
3. Milestone 5 complete (P0): supported-path opcode hardening (explicit unsupported-opcode errors + decode/translation jump/stack validation) and supported-subset `.pyc` writer (header + marshal).
4. Milestone 6 in progress (P0): relative `from .` imports landed; baseline module metadata population (`__package__`, `__spec__`, `__loader__`, `__path__`) landed; `sys` import-state foundations (`path`, `meta_path`, `path_hooks`, `modules`) and VM-level `__import__` baseline landed; filesystem namespace packages (including multi-root `__path__`) landed; package `__path__` is used for child-module lookup; importlib/finder-hook parity still pending.
5. Remaining milestones 7-14 cover: full language/tokenizer/compiler parity, runtime data model parity, builtins+stdlib bootstrap, async/concurrency semantics, CPython parity test gates, performance/observability, packaging/distribution, and future JIT/extension hooks.

## Engineering Principles
- Maintain correctness first, then performance.
- Keep dependencies minimal and well‑justified.
- Prefer clear architecture boundaries: parser, AST, compiler/IR, VM, runtime, stdlib.

## Project Artifacts
- Roadmap: `docs/ROADMAP.md`
- Compatibility tracker: `docs/COMPATIBILITY.md`
- Production readiness accounting: `docs/PRODUCTION_READINESS.md`
- CPython vendor sync script: `scripts/sync_cpython.py`
- Vendor snapshot: CPython 3.14.3 grammar + opcode sources synced into `vendor/cpython-3.14/` (opcode table CSV generated).

## Current Scaffolding (Early Stage)
- Parser: packrat-style memoization with a minimal lexer, indentation tokens, `if`/`elif`/`else`/`while`/`for`, `break`/`continue`, `with`, function defs, returns, calls, `yield`/`yield from`, `global`/`nonlocal`, tuple/list destructuring targets, and type annotations (vars, function params/returns, lambda params).
- Bytecode: minimal opcodes for source compiler plus CPython 3.14 decoder/translator (`opcode_table.csv` from CPython 3.14).
- Compiler/VM: emits and executes bytecode for `pass`, assignments (including tuple/list destructuring, name-based subscripts with negative index assignment, module attribute assignment, and augmented `+=`, `-=`, `*=`, `%=`, `//=`, `**=`), annotated assignments (populate `__annotations__` for module/class/function locals), literals (`True`, `False`, `None`), unary ops (`+`, `-`, `not`), binary ops (`+`, `-`, `*`, `**`, `//`, `%`), comparisons (`==`, `!=`, `<`, `<=`, `>`, `>=`, `in`, `not in`, `is`, `is not`), boolean ops (`and`, `or`), conditional expressions (`a if cond else b`), lambdas, lazy generators (`yield` / `yield from`) with suspended-frame resume and `__next__`/`send`/`throw`/`close` protocol coverage, `if/else`/`elif`, `while/for` (with `else` clauses and iterator protocol opcodes), `break`/`continue`, `raise`, `assert`, `try/except/else`, `try/finally`, `try/except/finally`, `with` (calls `__enter__`/`__exit__`), functions with positional-only params (`/`), positional/defaults, keyword-only params, and `*args`/`**kwargs` in definitions, keyword arguments at call sites, `*args`/`**kwargs` call expansion, function annotations stored on `__annotations__`, closures (`nonlocal`, cell/free vars, `LOAD/STORE_DEREF`, `LOAD_CLOSURE`), basic class definitions with single inheritance, instance attributes and bound methods (enforces `__init__` returns `None`), list/tuple/dict literals, subscripts with negative indexing plus slicing for list/tuple/str, attribute access (`module.attr`, `instance.attr`, `Class.attr`, `function.__annotations__`, generator methods), and basic `import` / `from ... import ...` statements with optional `as` aliases (dotted module names supported). Builtins: `print` (supports `sep`/`end` keywords), `len` (strings, lists, tuples, dicts, `obj` keyword), `range` (1-3 args, keyword args), `slice`, `bool`, `int`, `str`, `abs`, `sum` (supports `start` keyword), `min`, `max`, `all`, `any`, `pow`, `list`, `tuple`, `divmod`, `sorted` (supports `reverse` keyword), `enumerate` (supports `start` keyword), `id`, `locals`, `globals`, `__import__` (name/fromlist/level baseline), and basic exception types (`Exception`, `ValueError`, `TypeError`, `IndexError`, `KeyError`, `AssertionError`, `RuntimeError`, `NameError`, `AttributeError`, `ZeroDivisionError`, `StopIteration`, `GeneratorExit`).
- Exceptions: `try/except` handles explicit `raise`; VM runtime errors are mapped to coarse exception types (`RuntimeError`, `TypeError`, `IndexError`, `KeyError`, `ZeroDivisionError`, `NameError`, `AttributeError`) based on message heuristics; tracebacks include filename/line/column and frame names.
- Identity: `id()` builtin returns stable ids; `is`/`is not` are identity-based (heap objects carry stable ids).
- Classes: class bodies execute in a class namespace module while resolving missing names against the defining module; methods capture the defining module as globals.
- CPython bytecode: marshal reader + `.pyc` loader/writer, decoder, and translator covering a core opcode subset (`RESUME`, `LOAD_CONST`, `LOAD_SMALL_INT`, `LOAD_NAME`, `LOAD_LOCALS`, `LOAD_GLOBAL`, `LOAD_FAST*`, `LOAD_DEREF`, `LOAD_CLOSURE`, `STORE_DEREF`, `LOAD_ATTR` with encoded null flag, `STORE_*`, `BINARY_OP*`, `COMPARE_OP*`, `CONTAINS_OP*`, `IS_OP`, `CALL`/`CALL_KW` + specialized call forms, `MAKE_FUNCTION`, `SET_FUNCTION_ATTRIBUTE`, `LOAD_BUILD_CLASS`, `PUSH_NULL`, `GET_ITER`/`FOR_ITER`, `SEND`, `YIELD_VALUE`/`YIELD_FROM`, jumps, `RETURN_*`, `IMPORT_NAME`/`IMPORT_FROM`). Unsupported opcodes fail translation explicitly; decode/translation run jump-target and stack-shape validation.
- Modules: new `Value::Module` with per-module globals; VM maintains module cache and search paths (default CWD, configurable via `Vm::add_module_path`, and synchronized with `sys.path`). `sys` foundation module is bootstrapped (`path`, `meta_path`, `path_hooks`, `path_importer_cache`, `modules`). Import loads `<name>.py` (or package `__init__.py`) into a module frame, returning module objects; filesystem namespace packages (directories without `__init__.py`) are supported with aggregated `__path__` across module roots; submodule resolution consults loaded parent-package `__path__` before global `sys.path`; module attribute access attempts to lazy-load submodules; source `from .` relative imports resolve via package context; baseline module metadata (`__name__`, `__package__`, `__spec__`, `__loader__`, `__file__`, `__path__`) is populated for loaded modules/packages; functions capture defining module globals.
- Numeric compatibility: `bool` participates in int arithmetic/comparisons (`True == 1`, `True + 1`, etc.).
- Scoping: `global` and `nonlocal` statements supported inside functions; closures wired via cell/free vars; assignments to globals emit `StoreGlobal`.
- `.pyc` header parsing/writing (hash-based + timestamp) + executor (`Vm::execute_pyc_*`), with CLI support for `.pyc` paths.
- Code objects carry `filename` plus per-instruction source locations for tracebacks.
- Tests: parser smoke tests, bytecode metadata loader test, pyc header read/write tests, CPython `.pyc` execution + rewrite roundtrip tests, translation validation tests (jump/stack guards), arithmetic fuzz/property tests, integration package test, import-system tests (`sys.path` mutation, `sys.modules`, `__import__` top-level/fromlist/relative level, namespace packages including multi-root `__path__`, and submodule resolution through package `__path__`), and an optional CPython `Lib/test` harness (see `tests/cpython_harness.rs` + `tests/cpython_subset.txt`).
- CLI: `--ast`, `--bytecode`, and `.pyc` execution by file extension.
