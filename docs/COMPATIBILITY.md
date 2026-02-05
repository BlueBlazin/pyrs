# Compatibility Tracker (CPython 3.14)

This document tracks progress toward source and bytecode compatibility with CPython 3.14.

## Parser & Grammar
- [x] Vendored `Grammar/python.gram` and `Grammar/Tokens` (synced from CPython 3.14.3)
- [x] Indentation + basic tokenization (names, ints, strings, operators, keywords for implemented subset)
- [ ] Full tokenizer parity (string prefixes, numeric literals, f-strings, comments, etc.)
- [x] Statements subset: pass, expr, assign/augassign (incl tuple/list destructuring targets), if/elif/else, while/for/else (tuple/list targets), break/continue, def/return, import/from (dotted modules supported), global, raise, assert, try/except/else, with, class (bases supported, no keywords)
- [x] Expressions subset: arithmetic (incl `**`), comparisons (incl `in`/`not in`/`is`/`is not`), boolean ops, conditional expr, calls, literals, attribute/subscript/slice, lambda
- [ ] Comprehensions, generators, pattern matching, async/await, etc.

## Bytecode
- [x] Opcode source files synced (`opcode.py`, `bytecodes.c`, `opcode.h`)
- [ ] Opcode table synced from CPython 3.14 (CSV generation pending)
- [x] Internal bytecode IR + compiler for subset (non-CPython)
- [x] `.pyc` header parsing
- [ ] CPython bytecode decoder/encoder
- [ ] Opcode execution parity

## Runtime & Object Model
- [x] Core types subset (None, bool, int, str, tuple, list, dict)
- [ ] bytes, set, frozenset, memoryview, complex, etc.
- [x] Function + frame model (positional-only params, positional params, defaults, keyword args, keyword-only params, *args/**kwargs; no closures)
- [x] Exceptions subset (raise/try/except/else; simple exception types)
- [ ] Tracebacks + exception chaining
- [x] Module/import system (file-based, dotted modules, lazy submodule loading on attribute access)
- [x] Classes subset (single inheritance, instance attrs + bound methods)
- [x] Object identity (`id`, `is`/`is not`) + refcount + basic cycle GC

## Stdlib Coverage
- [x] `builtins` subset (print `sep`/`end`, len `obj`, range keywords, sum `start`, sorted `reverse`, enumerate `start`, slice, bool/int/str, abs/sum/min/max/all/any/pow, list/tuple, divmod, sorted)
- [ ] `sys`, `types`, `inspect`
- [ ] `os`, `pathlib`, `io`
- [ ] `math`, `random`, `itertools`
- [ ] `json`, `re`, `datetime`

## CPython Tests
- [ ] Establish test harness runner
- [ ] Smoke tests passing
- [ ] Incremental `Lib/test` coverage

## Real-world Apps
- [ ] CLI tools (simple)
- [ ] Web apps (minimal framework)
- [ ] Data processing (pure Python)

## Production Readiness Checklist (Living)
Status flags: `[ ]` not started, `[x]` complete.

### P0 (Production Blocking)
- [x] Object identity + stable headers (`id`, `is` semantics).
- [x] Reference counting + cycle GC.
- [ ] CPython opcode table decoder/encoder (3.14).
- [ ] `.pyc` load/serialize parity with CPython 3.14.
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
