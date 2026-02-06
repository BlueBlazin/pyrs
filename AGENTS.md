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
4. Milestone 6 complete (P0): import-system parity delivered for supported pure-Python scenarios (relative imports, namespace packages, `sys.path`/`sys.meta_path`/`sys.path_hooks`/`sys.path_importer_cache` contracts, module metadata/spec fields, `importlib` helper APIs).
5. Milestone 7 complete (P0): language-surface milestone delivered (decorators, assignment expressions, list/dict comprehensions + generator expressions with scope isolation, core `match`/`case` subset, async syntax lowering, `except*` parsing, type parameters on `def`/`class`, f-string lowering, and `__future__` placement/unknown-feature compile checks).
6. Remaining milestones 8-14 cover: runtime data model parity, builtins+stdlib bootstrap, full async/concurrency semantics, CPython parity test gates, performance/observability, packaging/distribution, and future JIT/extension hooks.

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
- Parser: packrat-style memoization with indentation tokens; statements include `if`/`elif`/`else`/`while`/`for`, `break`/`continue`, `with`, `match`/`case` (core subset), `try` with `except*` parsing, decorators, `def`/`class` (including type parameters on headers), async statement forms (`async def`/`async for`/`async with`), imports, and annotation syntax. Expressions include assignment expressions (`:=`), list/dict comprehensions + generator expressions, await syntax, `yield`/`yield from`, and f-string lowering.
- Bytecode: minimal opcodes for source compiler plus CPython 3.14 decoder/translator (`opcode_table.csv` from CPython 3.14).
- Compiler/VM: emits and executes bytecode for prior core feature set plus Milestone 7 additions: decorator application lowering, assignment expressions (`:=`), list/dict comprehensions and generator expressions via isolated synthetic function scopes, core `match`/`case` lowering (literal/capture/guard), async syntax lowering (`async def`/`await`/`async for`/`async with` to current runtime model), and f-string lowering to concatenation + `str(...)` conversion. `__future__` compile checks enforce top-of-file placement and unknown-feature rejection.
- Exceptions: `try/except` handles explicit `raise`; VM runtime errors are mapped to coarse exception types (`RuntimeError`, `TypeError`, `IndexError`, `KeyError`, `ZeroDivisionError`, `NameError`, `AttributeError`) based on message heuristics; tracebacks include filename/line/column and frame names.
- Identity: `id()` builtin returns stable ids; `is`/`is not` are identity-based (heap objects carry stable ids).
- Classes: class bodies execute in a class namespace module while resolving missing names against the defining module; methods capture the defining module as globals.
- CPython bytecode: marshal reader + `.pyc` loader/writer, decoder, and translator covering a core opcode subset (`RESUME`, `LOAD_CONST`, `LOAD_SMALL_INT`, `LOAD_NAME`, `LOAD_LOCALS`, `LOAD_GLOBAL`, `LOAD_FAST*`, `LOAD_DEREF`, `LOAD_CLOSURE`, `STORE_DEREF`, `LOAD_ATTR` with encoded null flag, `STORE_*`, `BINARY_OP*`, `COMPARE_OP*`, `CONTAINS_OP*`, `IS_OP`, `CALL`/`CALL_KW` + specialized call forms, `MAKE_FUNCTION`, `SET_FUNCTION_ATTRIBUTE`, `LOAD_BUILD_CLASS`, `PUSH_NULL`, `GET_ITER`/`FOR_ITER`, `SEND`, `YIELD_VALUE`/`YIELD_FROM`, jumps, `RETURN_*`, `IMPORT_NAME`/`IMPORT_FROM`). Unsupported opcodes fail translation explicitly; decode/translation run jump-target and stack-shape validation.
- Modules: new `Value::Module` with per-module globals; VM maintains module cache and search paths (default CWD, configurable via `Vm::add_module_path`, and synchronized with `sys.path`). `sys` foundation module is bootstrapped (`path`, `meta_path`, `path_hooks`, `path_importer_cache`, `modules`), with `meta_path` defaulting to `pyrs.PathFinder` and `path_hooks` defaulting to `pyrs.FileFinder`. Imports resolve through finder/loader contracts: meta path finder -> path hooks/importer cache -> loader create/exec (`pyrs.SourceFileLoader` / `pyrs.NamespaceLoader`). Import loads `<name>.py` (or package `__init__.py`) into a module frame, returning module objects; filesystem namespace packages (directories without `__init__.py`) are supported with aggregated `__path__` across module roots; submodule resolution consults loaded parent-package `__path__` before global `sys.path`; module attribute access attempts to lazy-load submodules; source `from .` relative imports resolve via package context; module metadata/spec (`__name__`, `__package__`, `__spec__`, `__loader__`, `__file__`, `__path__`) is populated for supported loader scenarios; functions capture defining module globals.
- Numeric compatibility: `bool` participates in int arithmetic/comparisons (`True == 1`, `True + 1`, etc.).
- Scoping: `global` and `nonlocal` statements supported inside functions; closures wired via cell/free vars; assignments to globals emit `StoreGlobal`.
- Language-surface caveats tracked for later milestones: full coroutine semantics, full `ExceptionGroup` splitting behavior, full pattern families, and full PEP 701 formatting edge cases remain pending (see roadmap/readiness docs).
- `.pyc` header parsing/writing (hash-based + timestamp) + executor (`Vm::execute_pyc_*`), with CLI support for `.pyc` paths.
- Code objects carry `filename` plus per-instruction source locations for tracebacks.
- Tests: parser smoke tests, bytecode metadata loader test, pyc header read/write tests, CPython `.pyc` execution + rewrite roundtrip tests, translation validation tests (jump/stack guards), arithmetic fuzz/property tests, integration package test, import-system tests (`sys.path` mutation, `sys.modules`, `__import__` top-level/fromlist/relative level, namespace packages including multi-root `__path__`, submodule resolution through package `__path__`, `sys.meta_path` and `sys.path_hooks` object/string contract entries, and `sys.path_importer_cache` behavior), and optional CPython `Lib/test` harnesses (`tests/cpython_subset.txt` and import-focused `tests/cpython_subset_imports.txt`).
- CLI: `--ast`, `--bytecode`, and `.pyc` execution by file extension.
