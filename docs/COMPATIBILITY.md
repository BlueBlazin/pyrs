# Compatibility Tracker (CPython 3.14)

This document tracks progress toward source and bytecode compatibility with CPython 3.14.

## Parser & Grammar
- [ ] Vendored `Grammar/python.gram` and `Grammar/Tokens`
- [ ] Tokenizer parity (indentation, string prefixes, numeric literals, f-strings)
- [ ] Statement coverage (simple, compound, pattern matching)
- [ ] Expression coverage (operators, comprehensions, lambdas)

## Bytecode
- [ ] Opcode table synced from CPython 3.14
- [ ] `.pyc` header parsing
- [ ] Bytecode decoder/encoder
- [ ] Opcode execution parity

## Runtime & Object Model
- [ ] Core types (None, bool, int, str, bytes, tuple, list, dict, set)
- [ ] Function + frame model
- [ ] Exceptions + tracebacks
- [ ] Module/import system
- [ ] Reference counting + cycle handling

## Stdlib Coverage
- [ ] `builtins`
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
