# Compatibility Tracker (CPython 3.14)

This document tracks progress toward source and bytecode compatibility with CPython 3.14.
For a full production-readiness accounting (beyond compatibility deltas), see `docs/PRODUCTION_READINESS.md`.

## Parser & Grammar
- [x] Vendored `Grammar/python.gram` and `Grammar/Tokens` (synced from CPython 3.14.3)
- [x] Indentation + baseline tokenization (names with Unicode identifier support, explicit line-join backslashes, ints/floats with underscores/base prefixes/exponents, strings with prefixes, operators, and soft-keyword handling for `match`/`case`/`type`)
- [~] Tokenizer parity for current curated CPython suites (additional long-tail lexical parity still pending)
- [x] Statements subset: pass, expr, assign/augassign (incl chained assignment, tuple/list destructuring targets, and generalized attribute/subscript targets), `del`, if/elif/else, while/for/else (tuple/list targets), break/continue, def/return, import/from (dotted modules supported), global/nonlocal, raise (including `raise ... from ...`), assert, try/except/else, with (including multi-item forms), class (bases + `metaclass=` keyword path supported), decorators, `match`/`case` (literal/capture/guard plus sequence/mapping/class/or/as/star families, class-pattern positional-after-keyword rejection, duplicate-capture and OR-binding parity checks, and irrefutable reachability checks), `except*` parsing, and core async statement semantics (`async def`/`async for`/`async with`)
- [x] Expressions subset: arithmetic (incl `**`, `/`, `//`, `%`), comparisons (incl `in`/`not in`/`is`/`is not`), boolean ops, conditional expr, calls (including generator-expression argument form), literals (including implicit adjacent string concatenation and imaginary-number literal lowering), attribute/subscript/slice, lambda, `yield`, `yield from`, assignment expressions (`:=`), await semantics, list/dict comprehensions, generator expressions, starred tuple/list displays, and f-string lowering
- [~] Type annotations / hints (variable annotations, function parameter + return annotations; baseline `from __future__ import annotations` defers annotation evaluation to strings, but full 3.14 annotation-evaluation parity remains pending)
- [~] Type-parameter/type-alias syntax baseline (`def`/`class` type params plus `type Name = ...` parsing/lowering; full PEP 695 runtime semantics pending)
- [x] `__future__` import placement + unknown-feature compile-time validation (future imports treated as compile-time directives; no runtime import side effects)
- [~] Advanced grammar/runtime parity gaps remain (remaining pattern edge/form semantics, full exception-group edge semantics, full f-string/PEP 701 coverage)

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
- [~] Arbitrary-precision `int` parity (`Value::BigInt` now powers core large-int arithmetic/bitwise/shift/comparison paths, Python-floor `//`/`%`/`divmod` semantics, large-decimal/base `int(...)` parsing (including stricter base-0/underscore validation), `%x`/`%X`/`%o` bigint formatting, bigint-aware `int.bit_length`, arbitrary-size `int.from_bytes`/`int.to_bytes`, and lazy large-stop `range` iteration; long-tail conversion/format/error-text edges remain pending)
- [x] `bytes`, `bytearray`, `memoryview`, `set`, `frozenset`, `complex`
- [x] Function + frame model (positional-only params, positional params, defaults, keyword args, keyword-only params, *args/**kwargs; closures + `nonlocal`)
- [x] Generators (lazy suspended-frame protocol: `__next__`, `send`, `throw`, `close`)
- [x] Coroutines + async generators (core `__await__` / `__aiter__` / `__anext__`, `aiter`/`anext`, `StopAsyncIteration`)
- [x] Exceptions subset (raise/try/except/else; simple exception types)
- [x] `except` matching supports builtin and user-defined exception classes (including tuple handlers and subclass matching)
- [x] `except*` runtime split semantics (matching subgroup delivery, multi-handler accumulation, and unmatched remainder reraising)
- [x] Tracebacks with filename/line/col + frame names
- [x] Exception chaining/context metadata (`__cause__`, `__context__`, `__suppress_context__`; `raise ... from ...` and implicit chaining)
- [x] `__annotations__` storage for modules/classes/functions
- [x] Module/import system parity for supported pure-Python scenarios (file-based imports, dotted modules, lazy submodule loading on attribute access, relative `from .` imports, `sys.path`-driven source lookup, `sys.modules` exposure, filesystem namespace-package loading, submodule lookup via package `__path__`, `sys.meta_path` default path-finder control, `sys.path_hooks` + `sys.path_importer_cache` contracts)
- [x] Sourceless `.pyc` import fallback (module and package `__init__` paths under `__pycache__` or direct `.pyc`)
- [x] Module metadata/spec fields for supported loaders (`__package__`, `__spec__`, `__loader__`, `__path__`, `has_location`, `cached`)
- [x] Classes subset (multiple inheritance with C3 MRO metadata, instance attrs + bound methods, descriptor-aware attribute load/store paths, explicit `super(type, obj)` support, `__slots__` restrictions including empty-slot and `__dict__` slot behavior, class-header `metaclass=` keyword path, metaclass conflict detection, and metaclass method lookup fallback)
- [~] Attribute-hook parity (`__getattribute__` custom override path + `object.__getattribute__` baseline are implemented; full CPython fallback/error-edge semantics remain pending)
- [x] Object identity (`id`, `is`/`is not`) + refcount + basic cycle GC
- [~] Hash-container parity (`dict`/`set`/`frozenset` now use hash-indexed runtime container objects with insertion-order backing vectors; unhashable key/item rejection is enforced on constructor/update/assignment/membership flows, literal dict construction, `dict.fromkeys(...)`, and `collections.Counter(...)`; dict keyed operations now route through hash-indexed lookup/update/delete helpers (`get`/`setdefault`/`pop`/delete), and membership/set-relationship checks use hash-based paths with hashability enforcement; dict equality is insertion-order independent, and set/frozenset equality is value-based including cross-type equality; full CPython hash/equality edge parity and container-performance closure remain pending)

## Stdlib Coverage
- [x] Stub/partial accounting gate (`docs/STUB_ACCOUNTING.md` + generated `docs/NOOP_BUILTIN_INVENTORY.txt` enforced by `tests/noop_inventory.rs`)
- [x] `builtins` subset (print `sep`/`end`, len `obj`, range keywords + lazy large-stop bigint iterator fallback, sum `start`, sorted `reverse`, enumerate `start`, `filter` (`callable`/`None` predicate), slice, bool/int/float/str (including `float.fromhex`, `float.hex`, `str.maketrans`, `str.find`, `str.rsplit`, `str.isalnum` baseline behavior), abs/sum/min/max/all/any/pow, list/tuple/set/frozenset (`list.reverse` baseline), bytes/bytearray/memoryview, complex, divmod, iter/next/`aiter`/`anext`, `type` (1-arg + 3-arg), locals, globals, `exec` (source/code with explicit globals/locals handling), `getattr`/`setattr`/`delattr`/`hasattr`, `ord`/`chr`, `super` (explicit args plus baseline zero-arg runtime path), baseline `object.__reduce_ex__`, basic `__import__` name/fromlist/level semantics)
- [x] `sys` import foundations (`path`, `meta_path`, `path_hooks`, `path_importer_cache`, `modules`, baseline `sys.exit`)
- [~] `importlib` foundations (`import_module`, `find_spec`, `importlib.util.find_spec`, `importlib.invalidate_caches`, baseline `importlib.util.spec_from_file_location`, `_frozen_importlib.spec_from_loader`/`_verbose_message`, and `_frozen_importlib_external` `_path_*` + `_unpack_uint*`; module-level `__getattr__` fallback is wired; full spec/loader object parity still pending)
- [~] `pkgutil` / `importlib.resources` foundations (fallback shim workflows for stdlib-less environments: `pkgutil.get_data`, `importlib.resources.files/read_text/read_binary/open_*`; full CPython parity still pending)
- [~] `site` startup behavior foundations (CLI baseline startup import when stdlib paths are discoverable plus `-S`/`--no-site` opt-out; full CPython startup/site initialization parity still pending)
- [~] `types`, `inspect` (`inspect.signature` now executes a non-`NoOp` path returning a `Signature` instance with baseline parameter-kind/default metadata; full CPython object/method parity still pending)
- [~] `os`, `pathlib`, `io` (`open`/`close`/`isatty`/`stat`/`lstat`/`rmdir`/`utime`/`scandir` and wait-status helpers now execute non-`NoOp` paths; broader module parity still pending)
- [~] `platform`, `binascii`, `atexit`, `collections` (`platform.win32_is_iot` and baseline `platform.libc_ver`, `binascii.crc32`, `atexit.register`/`unregister`/`_run_exitfuncs`/`_clear`, and `collections._count_elements` now execute non-`NoOp` paths; full module parity still pending)
- [~] `_csv` foundations (dialect registry helpers, `field_size_limit`, `Dialect` validation hook, and baseline `reader`/`writer` paths are wired to unblock stdlib `csv` imports; full C-level parser/writer behavior parity remains pending)
- [~] `_opcode` helpers (`stack_effect`, `has_arg`, `has_const`, `has_name`, `has_jump`, `has_free`, `has_local`, `has_exc`, `get_executor` now execute non-`NoOp` metadata-backed paths; full edge parity still pending)
- [~] `decimal`, `_pylong`, `_thread`, `_warnings` (`decimal.getcontext`/`setcontext`/`localcontext`, `_pylong` conversion/division helpers (`int_to_decimal_string`, `int_divmod`, `int_from_string`, `compute_powers`, `_dec_str_to_int_inner`), `_thread.start_new_thread`, and `_warnings._acquire_lock`/`_release_lock` now execute non-`NoOp` baseline paths; full semantics still pending)
- [~] `random` foundations (`seed`, `random`, `randrange`, `randint`, `getrandbits`, `choice`, `shuffle`)
- [~] `math`, `itertools` (`math` core transcendentals/aggregates now execute non-`NoOp` paths; `itertools` long-tail helpers now execute non-`NoOp` paths (`accumulate`, `combinations*`, `compress`, `dropwhile`, `filterfalse`, `groupby`, `islice`, `pairwise`, `starmap`, `takewhile`, `tee`, `zip_longest`); full iterator/laziness edge parity still pending)
- [~] `operator`, `functools` (`operator.itemgetter`/`attrgetter`/`methodcaller` and `functools.cmp_to_key` now execute non-`NoOp` paths with `sorted`/`min`/`max` key interoperability; `functools.partial` unwraps `staticmethod(...)` and `classmethod(...)` wrappers for partial/partialmethod class-body compatibility; `functools.wraps` now copies wrapper metadata (`__dict__`, `__wrapped__`) for function and bound-method inputs; `functools.cached_property` now executes descriptor-backed cache semantics used by stdlib paths like `ipaddress`; long-tail API parity still pending)
- [~] `json`, `re`, `datetime` (`datetime` now exports baseline `date`/`timedelta` class symbols for stdlib import paths; full runtime parity still pending)
- [~] `codecs` foundations (`encode`/`decode` for `utf-8`/`utf-16`/`utf-32`/`ascii`/`latin-1` with `strict`/`ignore`/`replace`)
- [~] `asyncio` foundations (`run`, `sleep`, `create_task`, `gather`)
- [~] `threading` foundations (`get_ident`, `current_thread`, `main_thread`, `active_count`, `local`, plus baseline class methods for `Thread`, `Event`, `Condition`, `Semaphore`, `BoundedSemaphore`, `Barrier`)
- [~] `signal` foundations (`signal`, `getsignal`, `raise_signal`, core constants)
- [~] `socket` / `_socket` foundations (`gethostname`, `gethostbyname`, `getaddrinfo`, `fromfd`, `getdefaulttimeout`/`setdefaulttimeout`, and `hton*`/`ntoh*` module-level paths plus baseline `socket.__init__`/`close`/`detach`/`fileno`; full socket API parity still pending)
- [~] `uuid` foundations (`UUID.__init__`, `uuid1/3/4/5/6/7/8`, `getnode`, and namespace constants with baseline object attributes; full CPython algorithm/edge parity pending)
- [~] `dataclasses` foundations (`field`, `is_dataclass`, `fields`, `asdict`, `astuple`, `replace`, `make_dataclass` baseline non-`NoOp` paths implemented, including keyword-only decorator form and `make_dataclass(..., module=...)`; full decorator/Field/default-factory semantics pending)
- [~] `subprocess` / `_posixsubprocess` foundations (`_posixsubprocess.fork_exec` now fails explicitly as unsupported instead of silent `NoOp`; full process-spawn parity pending)

## CPython Tests
- [x] First-class CPython harness with split suites (`tests/cpython_suite_language.txt`, `tests/cpython_suite_imports.txt`)
- [x] Owned allowlist tracking (`tests/cpython_allowlist.txt`) with stale-entry detection in harness
- [~] Current curated language/import harness suites are near-empty-allowlist; latest expansion includes `test/test_set.py`, `test/test_list.py`, `test/test_tuple.py`, `test/test_slice.py`, `test/test_format.py`, `test/test_configparser.py`, `test/test_base64.py`, `test/test_binascii.py`, `test/test_bisect.py`, `test/test_copy.py`, `test/test_csv.py`, `test/test_fnmatch.py`, `test/test_genericalias.py`, `test/test_heapq.py`, `test/test_pprint.py`, `test/test_reprlib.py`, `test/test_sched.py`, `test/test_statistics.py`, `test/test_textwrap.py`, `test/test_tokenize.py`, `test/test_json/__init__.py`, `test/test_dataclasses/__init__.py`, and `test/test_enum.py`
- [x] Differential tests vs CPython (`tests/differential_cpython.rs`)
- [x] Parser/compiler/VM fuzzing (`tests/fuzz_parser_vm.rs` + existing arithmetic fuzz)
- [~] Incremental `Lib/test` coverage expansion (broader suite growth and allowlist reduction ongoing; latest language-suite expansion adds `test_set`, `test_list`, `test_tuple`, `test_slice`, `test_format`, `test_configparser`, `test_base64`, `test_binascii`, `test_bisect`, `test_copy`, `test_csv`, `test_fnmatch`, `test_genericalias`, `test_heapq`, `test_pprint`, `test_reprlib`, `test_sched`, `test_statistics`, `test_textwrap`, `test_tokenize`, `test_json/__init__`, `test_dataclasses/__init__`, and `test_enum`)

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
- [ ] Hash-based dict/set/frozenset semantic parity (`__hash__` contract + unhashable key/item rejection).
- [ ] Native extension loading parity for limited C-API/abi3 modules.
- [ ] Production release gate (security + reliability): sanitizers, deterministic crash repros, parity-regression blocking CI.

### P1 (Major Ecosystem Enablers)
- [x] Async/await + async generators (core coroutine/async-iterator protocol and runtime semantics implemented for milestone scope).
- [x] Comprehensions with correct scoping.
- [~] Pattern matching (`match`/`case`) broad families are implemented (literal/capture/guard/sequence/mapping/class/or/as/star) with compile-time binding/reachability validation; long-tail edge/form parity remains pending.
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
