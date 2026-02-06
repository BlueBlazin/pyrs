# Compatibility Tracker (CPython 3.14)

This document tracks progress toward source and bytecode compatibility with CPython 3.14.
For a full production-readiness accounting (beyond compatibility deltas), see `docs/PRODUCTION_READINESS.md`.

## Parser & Grammar
- [x] Vendored `Grammar/python.gram` and `Grammar/Tokens` (synced from CPython 3.14.3)
- [x] Indentation + baseline tokenization (names with Unicode identifier support, explicit line-join backslashes, ints/floats with underscores/base prefixes/exponents, strings with prefixes, operators, and soft-keyword handling for `match`/`case`/`type`)
- [~] Tokenizer parity for current curated CPython suites (additional long-tail lexical parity still pending)
- [x] Statements subset: pass, expr, assign/augassign (incl chained assignment, tuple/list destructuring targets, and generalized attribute/subscript targets), `del`, if/elif/else, while/for/else (tuple/list targets), break/continue, def/return, import/from (dotted modules supported), global/nonlocal, raise (including `raise ... from ...`), assert, try/except/else, with (including multi-item forms), class (bases + `metaclass=` keyword path supported), decorators, `match`/`case` (core subset), `except*` parsing, and core async statement semantics (`async def`/`async for`/`async with`)
- [x] Expressions subset: arithmetic (incl `**`, `/`, `//`, `%`), comparisons (incl `in`/`not in`/`is`/`is not`), boolean ops, conditional expr, calls (including generator-expression argument form), literals (including implicit adjacent string concatenation and imaginary-number literal lowering), attribute/subscript/slice, lambda, `yield`, `yield from`, assignment expressions (`:=`), await semantics, list/dict comprehensions, generator expressions, starred tuple/list displays, and f-string lowering
- [x] Type annotations / hints (variable annotations, function parameter + return annotations; eager evaluation only)
- [~] Type-parameter/type-alias syntax baseline (`def`/`class` type params plus `type Name = ...` parsing/lowering; full PEP 695 runtime semantics pending)
- [x] `__future__` import placement + unknown-feature compile-time validation
- [~] Advanced grammar/runtime parity gaps remain (full pattern variants, full exception-group semantics, full f-string/PEP 701 coverage)

## Bytecode
- [x] Opcode source files synced (`opcode.py`, `bytecodes.c`, `opcode.h`)
- [x] Opcode table synced from CPython 3.14 (generated `opcode_table.csv`)
- [x] Internal bytecode IR + compiler for subset (non-CPython)
- [x] `.pyc` header parsing
- [x] CPython bytecode decoder + translator for supported opcode subset (explicit unsupported-opcode errors; no silent fallback behavior)
- [x] `.pyc` loader + executor for supported opcode subset (including marshal float constants)
- [x] CPython `.pyc` writer for supported code-object subset (header + marshal)
- [ ] CPython bytecode encoder for full 3.14 opcode families
- [ ] Opcode execution parity (full 3.14 coverage)

## Runtime & Object Model
- [x] Core types subset (None, bool, int, float, str, tuple, list, dict)
- [x] `bytes`, `bytearray`, `memoryview`, `set`, `frozenset`, `complex`
- [x] Function + frame model (positional-only params, positional params, defaults, keyword args, keyword-only params, *args/**kwargs; closures + `nonlocal`)
- [x] Generators (lazy suspended-frame protocol: `__next__`, `send`, `throw`, `close`)
- [x] Coroutines + async generators (core `__await__` / `__aiter__` / `__anext__`, `aiter`/`anext`, `StopAsyncIteration`)
- [x] Exceptions subset (raise/try/except/else; simple exception types)
- [x] Tracebacks with filename/line/col + frame names
- [x] Exception chaining/context metadata (`__cause__`, `__context__`, `__suppress_context__`; `raise ... from ...` and implicit chaining)
- [x] `__annotations__` storage for modules/classes/functions
- [x] Module/import system parity for supported pure-Python scenarios (file-based imports, dotted modules, lazy submodule loading on attribute access, relative `from .` imports, `sys.path`-driven source lookup, `sys.modules` exposure, filesystem namespace-package loading, submodule lookup via package `__path__`, `sys.meta_path` default path-finder control, `sys.path_hooks` + `sys.path_importer_cache` contracts)
- [x] Module metadata/spec fields for supported loaders (`__package__`, `__spec__`, `__loader__`, `__path__`, `has_location`, `cached`)
- [x] Classes subset (multiple inheritance with C3 MRO metadata, instance attrs + bound methods, descriptor-aware attribute load/store paths, explicit `super(type, obj)` support, `__slots__` restrictions, class-header `metaclass=` keyword path)
- [~] Attribute-hook parity (`__getattribute__` custom override path + `object.__getattribute__` baseline are implemented; full CPython fallback/error-edge semantics remain pending)
- [x] Object identity (`id`, `is`/`is not`) + refcount + basic cycle GC

## Stdlib Coverage
- [x] `builtins` subset (print `sep`/`end`, len `obj`, range keywords, sum `start`, sorted `reverse`, enumerate `start`, slice, bool/int/float/str, abs/sum/min/max/all/any/pow, list/tuple/set/frozenset, bytes/bytearray/memoryview, complex, divmod, iter/next/`aiter`/`anext`, `type` (1-arg + 3-arg), locals, globals, `getattr`/`setattr`/`delattr`/`hasattr`, explicit-args `super`, basic `__import__` name/fromlist/level semantics)
- [x] `sys` import foundations (`path`, `meta_path`, `path_hooks`, `path_importer_cache`, `modules`)
- [x] `importlib` foundations (`import_module`, `find_spec`, `importlib.util.find_spec`)
- [~] `types`, `inspect`
- [~] `os`, `pathlib`, `io`
- [~] `random` foundations (`seed`, `random`, `randrange`, `randint`, `getrandbits`, `choice`, `shuffle`)
- [~] `math`, `itertools`
- [~] `json`, `re`, `datetime`
- [~] `codecs` foundations (`encode`/`decode` for `utf-8`/`ascii`/`latin-1` with `strict`/`ignore`/`replace`)
- [~] `asyncio` foundations (`run`, `sleep`, `create_task`, `gather`)
- [~] `threading` foundations (`get_ident`, `current_thread`, `main_thread`, `active_count`)
- [~] `signal` foundations (`signal`, `getsignal`, `raise_signal`, core constants)

## CPython Tests
- [x] First-class CPython harness with split suites (`tests/cpython_suite_language.txt`, `tests/cpython_suite_imports.txt`)
- [x] Owned allowlist tracking (`tests/cpython_allowlist.txt`) with stale-entry detection in harness
- [x] Current curated language/import harness suites pass with an empty allowlist
- [x] Differential tests vs CPython (`tests/differential_cpython.rs`)
- [x] Parser/compiler/VM fuzzing (`tests/fuzz_parser_vm.rs` + existing arithmetic fuzz)
- [~] Incremental `Lib/test` coverage expansion (broader suite growth and allowlist reduction ongoing)

## Real-world Apps
- [x] Curated pure-Python smoke/regression suite (`tests/realworld_smoke.rs`)
- [~] CLI tools (simple; foundational smoke coverage in place)
- [ ] Web apps (minimal framework)
- [ ] Data processing (pure Python first, extension-backed packages once Milestone 15 starts)
- [x] Sandboxed parity profile script (`scripts/run_parity_gate.sh`) for constrained local execution (`env_clear`, isolated temp dirs, timeout in smoke tests)

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
- [x] Async/await + async generators (core coroutine/async-iterator protocol and runtime semantics implemented for milestone scope).
- [x] Comprehensions with correct scoping.
- [~] Pattern matching (`match`/`case`) core subset (literal/capture/guard) implemented; full pattern families pending.
- [x] Exception chaining (`__cause__`, `__context__`, suppression metadata).
- [~] Descriptor protocol + attribute lookup parity (descriptor hooks + `__getattr__`/`__setattr__`/`__delattr__` implemented; class-header `metaclass=` path and `__slots__` restrictions implemented; full `__getattribute__` and metaclass-precedence edge parity pending).
- [~] Core stdlib: `sys`, `types`, `inspect`, `io`.
- [~] Stdlib base: `os`, `pathlib`, `re`, `json`, `datetime`, `collections`, `math`, `codecs`.
- [~] Utility stdlib foundations: `random`.
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
