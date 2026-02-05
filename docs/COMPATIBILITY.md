# Compatibility Tracker (CPython 3.14)

This document tracks progress toward source and bytecode compatibility with CPython 3.14.

## Parser & Grammar
- [x] Vendored `Grammar/python.gram` and `Grammar/Tokens` (synced from CPython 3.14.3)
- [x] Indentation + basic tokenization (names, ints, strings, operators, keywords for implemented subset)
- [ ] Full tokenizer parity (string prefixes, numeric literals, f-strings, comments, etc.)
- [x] Statements subset: pass, expr, assign/augassign, if/elif/else, while/for/else, break/continue, def/return, import/from (dotted modules supported), global, raise, assert, try/except/else, class (bases supported, no keywords)
- [x] Expressions subset: arithmetic (incl `**`), comparisons (incl `in`/`not in`/`is`/`is not`), boolean ops, conditional expr, calls, literals, attribute/subscript/slice, lambda
- [ ] Comprehensions, generators, pattern matching, async/await, with, etc.

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
- [ ] Reference counting + cycle handling

## Stdlib Coverage
- [x] `builtins` subset (print `sep`/`end`, len `obj`, range keywords, sum `start`, sorted `reverse`, slice, bool/int/str, abs/sum/min/max/all/any/pow, list/tuple, divmod, sorted)
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
