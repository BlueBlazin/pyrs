# Compatibility Tracker (CPython 3.14)

This document tracks progress toward source and bytecode compatibility with CPython 3.14.
For a full production-readiness accounting (beyond compatibility deltas), see `docs/PRODUCTION_READINESS.md`.

## Parser & Grammar
- [x] Vendored `Grammar/python.gram` and `Grammar/Tokens` (synced from CPython 3.14.3)
- [x] Indentation + baseline tokenization (names, ints with underscores/base prefixes, strings with prefixes, operators, keywords for implemented subset)
- [ ] Full tokenizer parity (string prefixes, numeric literals, f-strings, comments, etc.)
- [x] Statements subset: pass, expr, assign/augassign (incl tuple/list destructuring targets), if/elif/else, while/for/else (tuple/list targets), break/continue, def/return, import/from (dotted modules supported), global/nonlocal, raise, assert, try/except/else, with, class (bases supported), decorators, `match`/`case` (core subset), `except*` parsing, and async statement syntax (`async def`/`async for`/`async with` with lowering semantics)
- [x] Expressions subset: arithmetic (incl `**`), comparisons (incl `in`/`not in`/`is`/`is not`), boolean ops, conditional expr, calls, literals, attribute/subscript/slice, lambda, `yield`, `yield from`, assignment expressions (`:=`), await syntax lowering, list/dict comprehensions, generator expressions, and f-string lowering
- [x] Type annotations / hints (variable annotations, function parameter + return annotations; eager evaluation only)
- [x] Type parameter syntax on `def`/`class` headers (`def f[T](...)`, `class C[T]: ...`)
- [x] `__future__` import placement + unknown-feature compile-time validation
- [~] Advanced grammar parity gaps remain (`type` statements, full pattern variants, full exception-group semantics, full f-string/PEP 701 coverage)

## Bytecode
- [x] Opcode source files synced (`opcode.py`, `bytecodes.c`, `opcode.h`)
- [x] Opcode table synced from CPython 3.14 (generated `opcode_table.csv`)
- [x] Internal bytecode IR + compiler for subset (non-CPython)
- [x] `.pyc` header parsing
- [x] CPython bytecode decoder + translator for supported opcode subset (explicit unsupported-opcode errors; no silent fallback behavior)
- [x] `.pyc` loader + executor for supported opcode subset
- [x] CPython `.pyc` writer for supported code-object subset (header + marshal)
- [ ] CPython bytecode encoder for full 3.14 opcode families
- [ ] Opcode execution parity (full 3.14 coverage)

## Runtime & Object Model
- [x] Core types subset (None, bool, int, str, tuple, list, dict)
- [ ] bytes, set, frozenset, memoryview, complex, etc.
- [x] Function + frame model (positional-only params, positional params, defaults, keyword args, keyword-only params, *args/**kwargs; closures + `nonlocal`)
- [x] Generators (lazy suspended-frame protocol: `__next__`, `send`, `throw`, `close`)
- [x] Exceptions subset (raise/try/except/else; simple exception types)
- [x] Tracebacks with filename/line/col + frame names
- [ ] Exception chaining
- [x] `__annotations__` storage for modules/classes/functions
- [x] Module/import system parity for supported pure-Python scenarios (file-based imports, dotted modules, lazy submodule loading on attribute access, relative `from .` imports, `sys.path`-driven source lookup, `sys.modules` exposure, filesystem namespace-package loading, submodule lookup via package `__path__`, `sys.meta_path` default path-finder control, `sys.path_hooks` + `sys.path_importer_cache` contracts)
- [x] Module metadata/spec fields for supported loaders (`__package__`, `__spec__`, `__loader__`, `__path__`, `has_location`, `cached`)
- [x] Classes subset (single inheritance, instance attrs + bound methods)
- [x] Object identity (`id`, `is`/`is not`) + refcount + basic cycle GC

## Stdlib Coverage
- [x] `builtins` subset (print `sep`/`end`, len `obj`, range keywords, sum `start`, sorted `reverse`, enumerate `start`, slice, bool/int/str, abs/sum/min/max/all/any/pow, list/tuple, divmod, sorted, locals, globals, basic `__import__` name/fromlist/level semantics)
- [x] `sys` import foundations (`path`, `meta_path`, `path_hooks`, `path_importer_cache`, `modules`)
- [x] `importlib` foundations (`import_module`, `find_spec`, `importlib.util.find_spec`)
- [ ] `types`, `inspect`
- [ ] `os`, `pathlib`, `io`
- [ ] `math`, `random`, `itertools`
- [ ] `json`, `re`, `datetime`

## CPython Tests
- [x] Establish test harness runner (optional; set `PYRS_CPYTHON_LIB`)
- [x] Smoke tests passing (local harness + integration tests)
- [ ] Incremental `Lib/test` coverage

## Real-world Apps
- [ ] CLI tools (simple)
- [ ] Web apps (minimal framework)
- [ ] Data processing (pure Python first, extension-backed packages once Milestone 15 starts)

## Native Extension Ecosystem
- [ ] Limited C-API/abi3 extension loading/execution parity for supported API surface
- [ ] HPy execution path with explicit compatibility matrix
- [ ] Extension-backed ecosystem smoke suite (numeric/parsing/crypto package classes)

## Production Readiness Checklist (Living)
Status flags: `[ ]` not started, `[x]` complete.

### P0 (Production Blocking)
- [x] Object identity + stable headers (`id`, `is` semantics).
- [x] Reference counting + cycle GC.
- [x] CPython opcode table decoder (3.14).
- [x] CPython opcode translation hardening for supported paths (fail-fast unsupported opcodes, jump/stack validation).
- [x] `.pyc` load/serialize parity for supported code-object subset.
- [ ] CPython opcode encoder (3.14 full family parity).
- [x] Closures + `nonlocal` (cell/free vars).
- [x] Generators (`yield`, `yield from`) + protocol (lazy suspension/resume + delegation semantics implemented).
- [x] Tracebacks + accurate frames (file/line/col).
- [x] Import system parity for supported pure-Python import scenarios (`importlib`, specs, hooks).
- [ ] Native extension loading parity for limited C-API/abi3 modules.
- [ ] Production release gate (security + reliability): sanitizers, deterministic crash repros, parity-regression blocking CI.

### P1 (Major Ecosystem Enablers)
- [~] Async/await + async generators (syntax lowering implemented; full coroutine semantics pending).
- [x] Comprehensions with correct scoping.
- [~] Pattern matching (`match`/`case`) core subset (literal/capture/guard) implemented; full pattern families pending.
- [ ] Exception chaining (`__cause__`, `__context__`, suppression).
- [ ] Descriptor protocol + attribute lookup parity.
- [ ] Core stdlib: `sys`, `types`, `inspect`, `io`.
- [ ] Stdlib base: `os`, `pathlib`, `re`, `json`, `datetime`, `collections`, `math`.
- [ ] HPy extension loading/execution path.
- [ ] Cross-platform release qualification matrix (Linux/macOS/Windows).

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
