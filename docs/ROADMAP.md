# Design and Roadmap

## Summary
We are building a production-grade Python interpreter in Rust with full source and CPython 3.14 bytecode compatibility. The system should be correct first, fast second, and architected so JIT and extension support remain feasible later.

## Goals
- Run Python source code with CPython 3.14 semantics.
- Execute CPython 3.14 bytecode (.pyc).
- Be usable for real-world apps, not just toy programs.
- Keep dependencies small and well-justified.

## Non-goals (now)
- JIT compilation.
- Full CPython C-API compatibility and C-extensions (deferred until Milestone 15).
- GIL-free runtime.

## Future-friendly constraints
- IR and VM boundaries should allow a JIT tier later.
- Runtime object model should be compatible with extension support later.

## High-level architecture
- Parser (packrat) -> AST -> Compiler -> Bytecode IR -> VM -> Runtime
- Standard library implemented incrementally.

## Parsing
- Implement CPython 3.14 grammar as a packrat parser.
- Vendor the grammar into the repo for stability and reproducibility.
- Keep parser dependencies minimal, prefer bespoke code for core parsing.

## Bytecode compatibility
- Target CPython 3.14 bytecode format and semantics.
- Maintain an opcode table in-repo and generate decoder/encoder tables.
- Support reading and writing .pyc files (supported-subset writer now in place).
- Keep stack effect metadata in one place to enable verification and tooling.

## Compiler and IR
- AST -> Bytecode IR that mirrors CPython behavior.
- Allow light optimizations that do not change observable semantics.
- Preserve debug and location info for tracebacks and tooling.

## Runtime model
- CPython-compatible object model.
- Reference-counted memory management with cycle detection where needed.
- GIL for correctness and simplicity.

## Standard library
- Start with a practical subset needed for common apps.
- Track progress by module and feature tests.
- Gradually expand toward full coverage.

## Dependency policy
- Add dependencies only with clear justification.
- Prefer small crates and isolate them behind internal APIs.
- Avoid heavy parser generators if they add significant weight.

## Testing strategy
- Golden tests for parser and AST.
- Bytecode round-trip tests and opcode-level unit tests.
- Integrate CPython test suite incrementally with a compatibility tracker.
- Real-world app smoke tests.

## Engineering gate policy
- Mandatory quality gates are defined in `docs/ENGINEERING_GATES.md`.
- Active algorithmic and semantic audit backlog is tracked in `docs/ALGO_AUDIT_BACKLOG.md`.
- Milestone closure requires these gates for in-scope runtime and stdlib paths; test pass alone is not sufficient.

## Milestones (Revised for 100% CPython 3.14)
Acceptance rule for all remaining milestones: no milestone is complete at "basic compat"; completion requires behavior-level parity for in-scope features plus targeted CPython tests.
Release-complete target: Milestone 16; ecosystem-complete target (including native-extension packages): Milestone 15 + Milestone 16.

1. Milestone 0: Parser + AST + minimal evaluator. (complete)
2. Milestone 1: Runtime core + identity + GC foundations. (complete)
3. Milestone 2: CPython bytecode intake foundations (`opcode_table.csv`, marshal reader, `.pyc` load + execution subset). (complete)
4. Milestone 3: Closures + frame metadata + traceback foundations. (complete)
5. Milestone 4: Generator and iteration parity (complete, P0).
6. Milestone 5: Opcode execution hardening + `.pyc` read/write parity for supported bytecode paths (complete, P0).
7. Milestone 6: Import system parity (`importlib`/`ModuleSpec`/hooks/packages, P0). (complete)
8. Milestone 7: Language surface foundations (major syntax/compiler support for modern Python constructs, P0). (complete)
9. Milestone 8: Runtime data model semantics (descriptor protocol, attribute model hooks, MRO/super, exception chaining, P0). (complete)
10. Milestone 9: Core runtime types + builtins + stdlib bootstrap required for real apps (P0/P1). (complete)
11. Milestone 10: Async/concurrency/runtime integration (`async`/`await`, async generators, event loop and threading semantics, P1). (complete)
12. Milestone 11: Test and parity gate (CPython harness, fuzzing, differential tests, real app suites, P0/P1). (complete)
13. Milestone 12: Core CPython parity closure for current harness-owned P0 gaps (complete, P0).
14. Milestone 13: Remaining language/runtime long-tail parity + stdlib/packaging usability closure for pure-Python ecosystem workloads (P0/P1).
15. Milestone 14: Performance, observability, and runtime hooks (P1/P2/P3).
16. Milestone 15: Native extension ecosystem compatibility (limited C-API/abi3 + HPy path, P0/P1).
17. Milestone 16: Release hardening and production certification (security/reliability/CI/distribution gates, P0/P1).

## Production Readiness Checklist (Living)
Canonical checklist lives in `docs/PRODUCTION_READINESS.md`. The list below is a snapshot of P0-P3 items we are actively tracking in the roadmap.
Status flags: `[ ]` not started, `[x]` complete.

### P0 (Production Blocking)
- [x] Object identity + stable headers (`id`, `is` semantics).
- [x] Reference counting + cycle GC.
- [ ] Full tokenizer/grammar parity for CPython 3.14 language surface.
- [x] CPython opcode table decoder (3.14).
- [x] CPython opcode translation hardening for supported paths (fail-fast unsupported opcodes, no silent fallback behavior).
- [x] `.pyc` load/serialize parity for supported code-object subset (header + marshal reader/writer).
- [ ] Full CPython opcode encode/decode parity for all 3.14 opcode families.
- [x] Closures + `nonlocal` (cell/free vars).
- [x] Generators (`yield`, `yield from`) + protocol (lazy suspension/resume + delegation semantics implemented).
- [x] Tracebacks + accurate frames (file/line/col).
- [x] Import system parity for supported pure-Python scenarios (`importlib`, specs, hooks).
- [x] Curated CPython harness parity closure for current language/import suites (allowlist burn-down to zero).
- [ ] Runtime semantic closure for remaining data-model gaps (`__getattribute__`, metaclass precedence, `__slots__` layout edges, full codecs behavior).
- [ ] Hash-based dict/set/frozenset semantic parity (`__hash__` contract, unhashable key/item rejection, CPython-compatible key/item lookup/update behavior).
- [ ] `json` semantic/safety/perf closure (full CPython encode/decode behavior and error contracts, malformed-input hardening, and benchmark baselines).
- [ ] `_csv`/`csv` semantic/safety/perf closure (full parser/writer semantics and error-text parity, malformed-input hardening, and benchmark baselines).
- [ ] `pickle`/`pickletools`/`copyreg` semantic/safety/perf closure (protocol/opcode/runtime parity plus benchmark baselines).
- [ ] Native extension loading parity for limited C-API/abi3 modules.
- [ ] Production release gate (security + reliability): sanitizers, deterministic crash repros, and parity-regression blocking CI.

### P1 (Major Ecosystem Enablers)
- [x] Async/await + async generators (core coroutine protocol/runtime + async iteration/context-manager semantics implemented).
- [x] Comprehensions with correct scoping.
- [~] Pattern matching (`match`/`case`) broad families implemented (literal/capture/guard/sequence/mapping/class/or/as/star); remaining edge/form parity pending.
- [x] Type annotations (parse + `__annotations__` on modules/classes/functions; eager evaluation only).
- [x] Exception chaining (`__cause__`, `__context__`, suppression metadata) for explicit/implicit raises.
- [~] Descriptor protocol + attribute lookup parity (descriptor hooks plus `__getattribute__` override baseline and `__getattr__`/`__setattr__`/`__delattr__` implemented; full `__getattribute__` fallback-edge and metaclass parity pending).
- [~] Core stdlib: `sys`, `types`, `inspect`, `io`.
- [~] Stdlib base: `os`, `pathlib`, `re`, `datetime`, `collections`, `math`, `codecs`.
- [~] Utility stdlib foundations: `random`.
- [ ] HPy extension loading/execution path.
- [ ] Cross-platform release qualification matrix (Linux/macOS/Windows) with parity gates.

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

## Milestone Definitions of Done (DoD)

### Milestone 1 — Runtime Core & Identity (P0)
DoD:
- `id()` returns stable values across object lifetime.
- `is`/`is not` are identity-based.
- Refcounting correct for all objects; deterministic dealloc in tests.
- Cycle GC handles self-referential containers.
Status: complete

### Milestone 2 — CPython Bytecode Compatibility (P0)
DoD:
- `opcode_table.csv` generated and used (3.14).
 - `.pyc` loader can execute CPython-compiled bytecode for a basic module.
 - Stack effects + jumps match CPython for supported opcode subset (calls, attr load/store, loops, class/function defs).
 - Marshal reader + translator cover the subset needed for smoke tests.
Status: complete

### Milestone 3 — Closures & Frames (P0)
DoD:
- `nonlocal` works in nested functions.
- Cells/free vars capture correctly across calls.
- Tracebacks show filename/line/column and frame names.
- `locals()`/`globals()` reflect correct scopes.
Status: complete

### Milestone 4 — Generators & Iteration Parity (P0)
DoD:
- Generators store suspended execution state (frame, instruction pointer, value stack, blocks), not precomputed output vectors.
- `send`, `throw`, and `close` semantics match CPython, including `GeneratorExit` and `finally` handling.
- `yield from` delegation semantics match CPython, including propagation of `StopIteration.value`.
- Reentrancy and terminal-state errors match CPython (`generator already executing`, post-close behavior, etc.).
- Targeted CPython generator tests pass (or are explicitly documented as blocked by out-of-scope work).
Status: complete

### Milestone 5 — Opcode Hardening & `.pyc` Writer Parity (P0)
DoD:
- All required CPython opcodes for the supported source/bytecode surface execute with correct semantics; no silent fallback `Nop` in supported code paths (unsupported opcodes fail translation explicitly).
- Stack-effect and jump validation checks are implemented for decode/translation.
- `.pyc` writer implemented with CPython-compatible headers and marshal output for supported code objects.
- CPython-compiled pure-Python modules execute end-to-end through the `.pyc` path.
Status: complete

### Milestone 6 — Import System Parity (P0)
DoD:
- `importlib` machinery is functional for pure-Python modules/packages.
- `sys.path`, `sys.meta_path`, `sys.path_hooks`, and loader/finder contracts work.
- `ModuleSpec` fields and module metadata (`__spec__`, `__package__`, `__loader__`, `__path__`) are populated correctly.
- Relative imports and namespace package behavior match CPython for supported scenarios.
Status: complete

### Milestone 7 — Language Surface Foundations (P0)
DoD:
- Tokenizer/grammar/compiler surface expanded for milestone targets: decorators, assignment expressions (`:=`), list/dict comprehensions and generator expressions (with scope isolation), `match`/`case` (core literal/capture/guard subset), async syntax (`async def`/`await`/`async for`/`async with`), `except*` parsing, type parameters on functions/classes, and f-string lowering.
- `__future__` import placement/unknown-feature gating checks are enforced at compile time, and future-import statements are now treated as compile-time directives (no runtime import side effects).
- Targeted parser/VM regression tests cover all newly added language-surface features.
Status: complete
Notes:
- Core coroutine/event-loop semantics are implemented in Milestone 10.
- Full tokenizer/grammar parity, full `ExceptionGroup` splitting semantics, and full PEP 701 formatting edge cases are tracked under Milestone 13 and production-readiness checklist items.

### Milestone 8 — Runtime Data Model Parity (P0)
DoD:
- Descriptor protocol foundations are implemented in the VM for data and non-data descriptors (`__get__`, `__set__`, `__delete__`) in class/instance attribute paths.
- Attribute model hooks are wired for supported scenarios: `__getattr__`, `__setattr__`, and `__delattr__` on instances, plus builtin `getattr`/`setattr`/`delattr`/`hasattr`.
- C3 MRO computation is implemented for class creation with `__mro__`/`__bases__` metadata and `super(type, obj)` lookup support.
- Exception chaining/context semantics are implemented for `raise ... from ...` and implicit chaining (`__cause__`, `__context__`, `__suppress_context__`).
Status: complete
Notes:
- Full metaclass precedence/selection semantics and custom metaclass class-object call edge cases are tracked in Milestone 13 parity closure.
- Core builtin type parity (`bytes`/`set`/`float`/unicode codecs foundations) is delivered in Milestone 9.

### Milestone 9 — Builtins + Stdlib Bootstrap (P0/P1)
DoD:
- Core runtime builtin type parity required by stdlib foundations (`set`, `frozenset`, `bytes`, `bytearray`, `memoryview`, `float`, `complex`, unicode/codecs behavior), plus remaining data-model gaps (`metaclass` path and `__slots__` core behavior).
- Builtins required by stdlib and common apps are present with correct semantics.
- Foundational stdlib modules are usable: `sys`, `types`, `inspect`, `io`, `os`, `pathlib`, `time`, `datetime`, `collections`, `math`, `re`, `json`, `codecs`, `functools`, `itertools`, `operator`, `random`.
- Pure-Python package installation/execution works across curated package classes (CLI, web, and data-style workloads) for the non-C-extension scope.
Status: complete
Progress:
- Float foundations landed end-to-end: parser/AST/compiler/VM/runtime support float literals, `/`, `//`, `%`, `**`, unary `+/-`, mixed int-bool-float comparisons, and `float()` builtin conversion paths.
- CPython marshal/translation now supports float constants (`g` binary float marshal tag) for `.pyc` decode/execute flows.
- Builtin `random` module foundations landed (`seed`, `random`, `randrange`, `randint`, `getrandbits`, `choice`, `shuffle`) with deterministic seed behavior and regression tests.
- Runtime types and builtins landed for `set`/`frozenset`, `bytes`/`bytearray`/`memoryview`, `complex`, `iter`, `next`, and dynamic `type(...)` class creation with `__slots__` enforcement.
- Class header metaclass keyword path landed (`class C(metaclass=...)`) with VM frame plumbing and `__build_class__` metaclass kwargs handling.
- `codecs` stdlib foundation landed (`encode`/`decode` for `utf-8`/`ascii`/`latin-1` with `strict`/`ignore`/`replace` error modes).

### Milestone 10 — Async and Concurrency Semantics (P1)
DoD:
- `async def`/`await`/`async for`/`async with` semantics are implemented.
- Async generators and coroutine protocol behavior match CPython for milestone-scope features.
- Core runtime support for `asyncio` basic task scheduling exists; threading + signals semantics are implemented to required compatibility level.
Status: complete
Progress:
- Coroutine runtime semantics landed for `async def` and `await` via dedicated `GET_AWAITABLE` VM handling and coroutine-aware generator objects (`__await__`, `inspect.iscoroutine`, `inspect.isawaitable`).
- Async iteration/context-manager flows are operational: `aiter`/`anext` builtins, `StopAsyncIteration` exception wiring, async-generator protocol hooks (`__aiter__`, `__anext__`), and `async for`/`async with` execution paths.
- Builtin stdlib foundations for `asyncio` (`run`, `sleep`, `create_task`, `gather`), `threading` (`get_ident`, `current_thread`, `main_thread`, `active_count`), and `signal` (`signal`, `getsignal`, `raise_signal`) are integrated and covered by VM regression tests.

### Milestone 11 — Testing and Parity Gate (P0/P1)
DoD:
- CPython `Lib/test` harness is first-class (not ignored by default in CI/profile used for parity).
- Curated `Lib/test` language/import/strict-stdlib subsets pass with strict ownership tracking for any expected gaps.
- Differential tests versus CPython and parser/VM fuzzing run continuously.
- Real-world pure-Python applications pass curated smoke/regression suites.
- Residual Milestone 8/9 semantic gaps (`__getattribute__` edge parity, metaclass precedence/selection, `__slots__` layout edge cases, full codecs behavior) are either closed or explicitly scoped with failing tests and ownership.
Status: complete
Progress:
- CPython harness is now first-class and non-ignored (`tests/cpython_harness.rs`) with split suites (`tests/cpython_suite_language.txt`, `tests/cpython_suite_imports.txt`, `tests/cpython_suite_strict_stdlib.txt`) plus strict allowlist ownership/category tracking (`tests/cpython_allowlist.txt`, `tests/cpython_allowlist_strict.txt`) and stale-allowlist detection.
- Current curated language/import harness suites pass with an empty allowlist (`tests/cpython_allowlist.txt`), and the strict stdlib suite is active with owned entries in `tests/cpython_allowlist_strict.txt`.
- Differential-vs-CPython coverage landed in `tests/differential_cpython.rs` for curated corpus and arithmetic fuzz expressions.
- Parser/compiler/VM no-panic fuzzing landed in `tests/fuzz_parser_vm.rs`.
- Curated real-world smoke coverage landed in `tests/realworld_smoke.rs` and is executed in a constrained subprocess profile (`env_clear`, isolated temp cwd/home, and timeout enforcement).
- Milestone parity profile is codified in `scripts/run_parity_gate.sh` (`PYRS_PARITY_STRICT=1`) and currently passes.

### Milestone 12 — Core CPython Parity Closure (P0)
DoD:
- Close all harness-owned parser/runtime/stdlib allowlist gaps for current curated CPython language/import suites.
- Land parser/compiler/runtime fixes plus targeted regressions required to run those suites with no expected-failure allowlist entries.
- Keep parity profile (`scripts/run_parity_gate.sh`) green after closure.
- Record explicit carry-forward ownership for any remaining long-tail parity work.
Status: complete
Progress:
- CPython harness allowlist has been burned down to zero (`tests/cpython_allowlist.txt`).
- Curated CPython language/import suites pass under `tests/cpython_harness.rs`.
- Runtime and stdlib compatibility closure landed for harness blockers (plus broad regression additions in `tests/vm.rs` and `tests/parser.rs`).
- Parity profile script remains green.
- Baseline CI parity workflow is present (`.github/workflows/parity-gate.yml`) and runs full tests plus parity profile against CPython 3.14.3 `Lib`.
Execution record:
- Milestone closure details are tracked in `docs/MILESTONE_12_BACKLOG.md`.

### Milestone 13 — Long-Tail Parity + Stdlib/Packaging Usability Closure (P0/P1)
DoD:
- Remaining language/runtime long-tail parity closes for production blocking semantics (`__getattribute__`, metaclass precedence/selection, `__slots__` edge layout behavior, full codecs behavior, pattern-family completion, `ExceptionGroup` split semantics, and full PEP 701 behavior).
- Runtime container semantic parity closes for hash-driven types (`dict`/`set`/`frozenset`) including unhashable rejection behavior.
- Semantic-contract quality gate is closed for mutation-sensitive core operations (for example `list.sort`/container mutators) with CPython differential coverage and no known contract violations in scope.
- Stdlib coverage expands to unblock mainstream pure-Python ecosystems (`subprocess`, `socket`, `ssl`, `http`, `urllib`, `typing`, `dataclasses`, `enum`, `contextvars`, and importlib resource/package helpers).
- `json`/`_csv`/`pickle` stacks close to full CPython semantics with explicit robustness/performance gates (`test_json`, `test_csv`, `test_pickle`, `test_pickletools`, and `test_copyreg` in harness scope, plus malformed-input/differential coverage and benchmark reporting).
- Native stdlib VM handlers are isolated and progressively retired in favor of official CPython pure-Python stdlib implementations wherever feasible.
- Import and packaging paths close remaining usability gaps (zip/bytecode imports, `importlib.resources`, `pkgutil`, and `site` startup behavior).
- venv + pip pure-Python package workflows are production-usable and covered by regression tests.
- Real-world smoke/regression matrix includes CLI/web/data pure-Python app classes running under constrained sandbox profile.
- `NoOp` and non-`NoOp` partial implementation ledgers stay current (`docs/NOOP_BUILTIN_INVENTORY.txt` + `docs/STUB_ACCOUNTING.md`) with CI drift checks.
Progress:
- Metaclass/runtime parity batch landed: resolved-metaclass tracking on class objects, metaclass conflict detection across base classes, and metaclass attribute fallback for class-object attribute access.
- Class-instance data-model parity batch landed for slot edges: empty `__slots__` classes block dynamic attributes, and `__slots__ = ('__dict__',)` enables dynamic attribute assignment.
- Codec parity batch expanded foundations to include `utf-32`, `utf-32-le`, and `utf-32-be` encode/decode paths with existing error-mode handling.
- Exception-parity batch landed for `except` handlers over user-defined classes: tuple handlers and subclass matching now work with runtime exception-parent tracking.
- `os`/`posix` parity batch landed for core filesystem/process helpers: `open`, `close`, `isatty`, `stat`, `lstat`, `rmdir`, `utime`, `scandir`, and wait-status helper functions (`WIF*`/`W*SIG`/`WEXITSTATUS`) now execute real logic instead of `NoOp`.
- `math` parity batch landed for previously stubbed numeric helpers: `ldexp`, `hypot`, `fabs`, `exp`, `erfc`, `log`, `fsum`, `sumprod`, `cos`, `sin`, `tan`, `cosh`, `asin`, `atan`, `acos`, and `isclose` now execute non-`NoOp` logic with new regression tests.
- `operator`/`functools` parity batch landed for callable adapters: `operator.itemgetter`, `operator.attrgetter`, `operator.methodcaller`, and `functools.cmp_to_key` now execute non-`NoOp` logic (including `sorted`/`min`/`max` key-ordering interoperability), with dedicated VM regressions.
- `itertools` parity batch landed for previously stubbed helpers: `accumulate`, `combinations`, `combinations_with_replacement`, `compress`, `dropwhile`, `filterfalse`, `groupby`, `islice`, `pairwise`, `starmap`, `takewhile`, `tee`, and `zip_longest` now execute non-`NoOp` logic with dedicated VM regressions.
- `inspect` parity batch landed for core signature introspection: `inspect.signature` now executes a non-`NoOp` path and returns an `inspect.Signature` instance with baseline parameter-kind/default metadata and return-annotation capture.
- `importlib` parity batch landed for cache/spec helpers: `importlib.invalidate_caches` and baseline `importlib.util.spec_from_file_location` now execute non-`NoOp` logic with dedicated VM regressions; full spec/loader object parity remains tracked in Milestone 13.
- `platform`/`binascii`/`atexit`/`collections` parity batch landed for previously stubbed helpers: `platform.win32_is_iot`, `binascii.crc32`, `atexit.register`/`unregister`/`_run_exitfuncs`/`_clear`, and `collections._count_elements` now execute non-`NoOp` logic with dedicated VM regressions.
- CPython harness unblock batch landed: `platform.libc_ver` now returns a baseline `(lib, version)` tuple and `functools.partial` now unwraps `staticmethod(...)` wrappers to preserve `partialmethod(staticmethod(...), ...)` import-time class-body behavior.
- `functools.cached_property` parity batch landed: decorator path now builds descriptor-backed cached properties (non-identity), unblocking stdlib `ipaddress` attribute/value flows and associated VM regressions.
- Descriptor/string/container parity batch landed: runtime now wraps and binds `classmethod(...)` descriptors correctly (class/instance/super access), `functools.partial` unwraps `classmethod(...)` wrappers in addition to staticmethod wrappers, string helper coverage now includes `str.rsplit`, `str.find`, and `str.isalnum`, list helper coverage now includes `list.reverse`, dict helper coverage now accepts mapping-like objects and iterable key/value pairs in `dict.update(...)`, and `threading.local` is now available as a baseline type.
- `_frozen_importlib`/`_frozen_importlib_external` parity batch landed for import bootstrap helpers: `spec_from_loader`/`_verbose_message`, `_path_join`/`_path_split`/`_path_stat`, and `_unpack_uint16`/`_unpack_uint32`/`_unpack_uint64` now execute non-`NoOp` logic with dedicated VM regressions.
- `_opcode` parity batch landed for metadata helpers: `stack_effect`, `has_arg`, `has_const`, `has_name`, `has_jump`, `has_free`, `has_local`, `has_exc`, and `get_executor` now execute non-`NoOp` metadata-backed logic with dedicated VM regressions.
- `decimal`/`_thread`/`_warnings` parity batch landed for foundational runtime helpers: `decimal.getcontext`/`setcontext`/`localcontext`, `_thread.start_new_thread`, and `_warnings._acquire_lock`/`_release_lock` now execute non-`NoOp` baseline logic with dedicated VM regressions.
- `builtins.exec` parity batch landed for executable code paths: `exec` now executes source strings and code objects with explicit `globals`/`locals` namespace handling and closure arity validation, replacing prior `NoOp` behavior with dedicated VM regressions.
- `_socket` parity batch expanded to socket object paths: module-level helpers plus baseline `socket.__init__`/`close`/`detach`/`fileno` now execute non-`NoOp` logic with dedicated VM regressions; broader socket API/option semantics remain tracked in Milestone 13.
- `_pylong` parity batch landed for conversion/division helpers: `int_to_decimal_string`, `int_divmod`, `int_from_string`, `compute_powers`, and `_dec_str_to_int_inner` now execute non-`NoOp` baseline logic with dedicated VM regressions; full bigint-scale semantics remain tracked in Milestone 13.
- `threading` class-method parity batch landed for synchronization primitives: baseline non-`NoOp` behavior now exists for `Thread`, `Event`, `Condition`, `Semaphore`/`BoundedSemaphore`, and `Barrier` method surfaces, with dedicated VM regressions.
- `uuid` parity batch landed for baseline object/model behavior: `UUID.__init__`, `uuid1/3/4/5/6/7/8`, `getnode`, and namespace constants now execute non-`NoOp` logic with dedicated VM regressions; full algorithm/text/edge parity remains tracked in Milestone 13.
- `object.__reduce_ex__` no-op removal landed with baseline tuple-return semantics and regression coverage; full pickling protocol parity remains tracked in Milestone 13.
- Dataclasses/runtime helper batch landed: `dataclasses.field`/`is_dataclass`/`fields`/`asdict`/`astuple`/`replace`/`make_dataclass` now execute baseline non-`NoOp` paths, stdio stream `write`/`flush`/`isatty` paths execute real behavior, `float.fromhex`/`float.hex` and `str.maketrans` execute non-`NoOp` helper logic, and `_posixsubprocess.fork_exec` now fails explicitly as unsupported instead of silently succeeding.
- CLI startup/import usability batch landed: CLI now supports baseline `site` startup import behavior when stdlib paths are discoverable, including `-S`/`--no-site` opt-out and explicit strict failure when `PYRS_CPYTHON_LIB` is user-configured but `site` import fails; script-directory module path precedence is now maintained via `Vm::add_module_path_front`.
- Exception-group parity batch landed: `except*` now performs subgroup splitting, executes all matching handlers with subgroup values, and reraises unmatched remainder groups.
- Pattern-family parity batch landed: parser/compiler/VM support now covers sequence (`*rest`), mapping (`**rest`), class, `or`, and `as` patterns with dedicated parser/VM regressions.
- Pattern-validation parity batch landed: compiler now enforces duplicate-capture rejection, OR-alternative binding-set equality, and irrefutable-pattern reachability checks (with guarded-case allowance), and parser now rejects class-pattern positional arguments after keyword patterns with match-statement error surfacing.
- Pattern precedence fix landed: parser now treats `A | B as name` as `(A | B) as name` (matching CPython/PEP 634), eliminating false OR-binding-set compile errors.
- Import packaging parity batch landed: module/package import now supports sourceless `.pyc` fallback (`__pycache__` and direct `.pyc` forms), and stdlib-less fallback shims now provide baseline `pkgutil.get_data` and `importlib.resources` (`files`/`read_text`/`read_binary`/`open_*`) workflows.
- No-op accounting hardening landed: inventory walk is recursive across module/class/instance/container graphs so stub drift cannot hide outside top-level module globals.
- Super/MRO parity fix landed: `super(...).__attr__` now checks direct attrs at each MRO step (instead of recursively re-walking parent MROs), eliminating incorrect early fallback to `object.__init__` in cooperative multiple-inheritance paths.
- CPython harness breadth expansion now includes `test/test_set.py`, `test/test_list.py`, `test/test_tuple.py`, `test/test_slice.py`, `test/test_format.py`, `test/test_configparser.py`, `test/test_base64.py`, `test/test_bisect.py`, `test/test_copy.py`, `test/test_fnmatch.py`, `test/test_genericalias.py`, `test/test_heapq.py`, `test/test_pprint.py`, `test/test_reprlib.py`, `test/test_sched.py`, `test/test_statistics.py`, `test/test_textwrap.py`, and `test/test_tokenize.py`.
- CPython harness regression-closure batch landed: `_colorize.decolor`, `functools.wraps` metadata propagation for bound-method inputs, VM-native `enumerate`/`filter` iterable handling, list slice assignment semantics, and `sys.exit` baseline behavior are now implemented with dedicated regressions.
- Bigint parity slice landed: runtime now has `Value::BigInt` and VM operator parity for large-int arithmetic/bitwise/shift/comparison paths (`+`/`-`/`*`/`**`/`~`/`&`/`|`/`^`/`<<`/`>>` and ordering) with dedicated regressions.
- Large-stop `range(...)` semantics no longer force eager list materialization: VM now provides a lazy bigint-backed `range_iterator` fallback for large ranges while preserving eager list behavior for small ranges.
- Expanded CPython probe runs outside the curated harness now carry forward remaining Milestone 13 blockers tracked in `docs/STUB_ACCOUNTING.md` (`_testinternalcapi.hamt` remains outstanding).
- Class-statement inheritance hang closure landed: exception-parent ancestry traversal now has cycle guards, unblocking `class X(Base): ...` over `seq_tests`-style bases and re-enabling `test/test_list.py` in curated harness runs.
- Bigint long-tail parity slice landed: VM/runtime now use arbitrary-precision floor-division semantics for `//`, `%`, and `divmod`, `%x`/`%X`/`%o` formatting now supports large integers, and `_pylong` conversion/division helpers execute bigint-capable paths (`int_to_decimal_string`, `int_divmod`, `int_from_string`, `compute_powers`, `_dec_str_to_int_inner`) with dedicated regressions.
- Future-annotation/stdlib bootstrap parity batch landed: baseline `from __future__ import annotations` now defers function/variable annotation evaluation to strings, `__future__.all_feature_names` compatibility is wired for stdlib `codeop` import paths, and curated CPython harness expansion now includes `test/test_json/__init__.py`, `test/test_dataclasses/__init__.py`, and `test/test_enum.py` with empty allowlist.
- Dataclasses/datetime import-path closure landed for harness expansion: `make_dataclass(..., module=...)` is supported, `dataclass(...)` keyword-only decorator form no longer fails baseline call paths, and `datetime` now exports baseline `date`/`timedelta` symbols required by stdlib imports.
- Container semantics hardening batch landed: hashability guards now enforce `TypeError` for unhashable `dict` keys and `set`/`frozenset` items across core constructor/update/assignment/membership flows, with dedicated VM regressions.
- Hashability-parity follow-up landed: unhashable-key rejection now also applies to literal dict construction (`{...}`), `dict.fromkeys(...)`, and `collections.Counter(...)`, with dedicated VM regressions.
- VM refactor kickoff landed: container/hashability helpers are extracted into `src/vm/containers.rs` to begin decomposing `src/vm/mod.rs` without behavior regressions.
- Runtime container upgrade landed: `dict`/`set`/`frozenset` now use dedicated hash-indexed runtime container objects with insertion-order backing vectors.
- Container equality parity batch landed: dict equality is now insertion-order independent, and set/frozenset equality is now value-based (including cross-type `set == frozenset` semantics), with dedicated VM regressions.
- VM decomposition batch landed: arithmetic/comparison/type-union operator kernels were extracted from `src/vm/mod.rs` into `src/vm/ops.rs` with zero-regression full-suite validation, reducing monolith pressure ahead of bigint work.
- VM stdlib decomposition batch landed: native `json`, `re`, and `_csv`/`csv` builtin handler methods were extracted from `src/vm/mod.rs` into `src/vm/stdlib/{json,re,csv}.rs` with zero-regression validation, improving diffability against CPython implementations and reducing monolithic VM surface.
- CSV helper extraction follow-up landed: CSV parser/quote/value-coercion helper kernels (`validate_csv_parameter_consistency`, row parser/state tracking, quoting logic, char/value coercions) were moved from `src/vm/mod.rs` into `src/vm/stdlib/csv.rs` with behavior-preserving validation and direct module-local unit tests.
- JSON/pickle parity follow-up landed: JSON handlers now support baseline `dumps` kwargs (`sort_keys`, `separators`, `ensure_ascii`, `allow_nan`, `default`) plus UTF-8 `bytes`/`bytearray` `loads` and bigint-aware integer parsing, and object pickling protocol helpers (`__getstate__`, `__reduce_ex__`) were extracted into `src/vm/stdlib/pickle.rs` with builtin-payload reconstruction regressions.
- Bigint conversion/format follow-up landed: `int(...)` now enforces base-0 and underscore validation rules more closely (including base-prefix forms), float-to-int conversion now rejects NaN/infinity and uses truncation toward zero with bigint fallback, `int.bit_length` now supports bigint receivers directly, and `int.from_bytes`/`int.to_bytes` now support arbitrary-size values with signed/unsigned overflow guards.
- Hash-container lookup hardening landed in VM hot paths: dict keyed operations (`get`, `setdefault`, `pop`, delete, and string-key helpers) now route through hash-indexed container APIs, `in` checks for dict/set/frozenset use hash-based lookups with explicit hashability checks, and set relationship operations reject unhashable iterables consistently.
- Container hot-path follow-up landed in runtime internals: dict/set delete paths now update hash-index buckets in-place instead of full index rebuilds, reducing churn in pop/remove-heavy workloads while preserving existing semantics and regression coverage.
- Stdlib/import-path unblock batch landed for CSV/hypothesis-family imports: module-level `__getattr__` fallback is now honored, zero-argument `super()` now has a baseline runtime path, builtin `chr()` is available, `_csv` module foundations are installed (`Dialect`, registry helpers, reader/writer baseline), and curated CPython harness expansion now includes `test/test_binascii.py` and `test/test_csv.py` with empty allowlist.
- Zero-argument `super()` owner-class tracking landed for classmethod contexts: class-body functions now retain their defining class and zero-arg `super()` uses that owner metadata when `__class__` cells are unavailable, preventing recursive `__init_subclass__` dispatch in class-creation paths.
- Copyreg/pickletools harness closure landed: comprehension first-iterable evaluation now follows CPython scope behavior, baseline regex character-class/range/anchor matching is supported for stdlib call paths, set binary operators (`-`, `&`, `^`) are wired for set/frozenset values, and `test/test_copyreg.py` is now in the curated language suite with empty allowlist.
- `_csv` hardening landed beyond bootstrap: reader now supports `skipinitialspace`, `quotechar=None`, `escapechar`, and `field_size_limit` enforcement; writer now supports `quotechar=None`, `escapechar`, and `quoting` mode handling (`QUOTE_MINIMAL`, `QUOTE_ALL`, `QUOTE_NONE`) with dedicated regressions.
- Container internals advanced toward production-performance closure: dict/set index maps now use direct hash-key lookup entries (instead of hash-bucket vectors), reducing lookup/update overhead and preserving insertion-order/value semantics; remaining closure is load-factor/growth-policy tuning and long-tail hash/equality parity.
- Regression coverage added for all above behaviors in `tests/vm.rs`; parity gate remains green on core profile while expanded CPython harness closure continues against tracked Milestone 13 blockers.
- Dedicated early regression gates now cover helper internals directly (`src/vm/containers.rs`, `src/vm/stdlib/json.rs`, `src/vm/stdlib/csv.rs` module-local unit tests) so semantic drift is caught before broad harness runs.
- Dedicated GC/leak regression lane is active (`tests/gc_regression.rs`), and strict stdlib harness entries now execute in isolated subprocesses with timeout (`PYRS_STRICT_HARNESS_TIMEOUT_SECS`, default 120s) to prevent runaway hangs/memory growth from masking regressions.
- Remaining mandatory P0 gate in Milestone 13: full `json`/`_csv`/`pickle` semantic closure with explicit robustness/performance proof; current implementations are still partial and tracked in `docs/STUB_ACCOUNTING.md`.
- VM call/import stack-discipline fix landed for nested frame-producing paths: builtin/import opcode result delivery now targets caller frames robustly, removing `builtin caller frame missing` regressions while preserving nested import behavior.
- Runtime compatibility slice landed for stdlib bootstrap: `__getattr__` fallback now triggers when `__getattribute__` raises `AttributeError`, `str()`/`float()` now accept zero-argument forms, `io.text_encoding` is implemented, and `os.getenv`/`os.write` plus `random.choices` (including `Random` instance method path) are available with regressions.
- Enum/import closure follow-up landed: metaclass `__call__` dispatch for class invocation is now wired through unified call paths, enum shim functional-call import paths no longer fail, and baseline `datetime.date(...)` constructor support is in place for stdlib import-time usage.
- Active Milestone 13 blockers remain explicit: `_io.open` still lacks CPython file-object semantics needed by `tempfile`/`test_csv` execution paths, and strict standalone `test_csv` unittest execution still fails under `unittest.runner` (the prior `_csv` `StopIteration` propagation failure is fixed; remaining failures are broader unittest/io/re/parity gaps).
- Strict stdlib bootstrap follow-up landed: builtin `os.path` now provides `relpath` and `isabs` (unittest discovery blockers), `_io.TextIOWrapper.__init__` and file-object `readlines()` are wired for wrapped binary-buffer flows (`tokenize.open`/`linecache` call paths), and wrapped-file descriptor lookup now walks `buffer`/`raw` chains.
- Strict stdlib allowlist remains intentionally non-empty: current owned failures are now narrowed to `test_pickle` and `test_pickletools` (`stdlib-strict-pickle` and `stdlib-strict-pickletools` categories in `tests/cpython_allowlist_strict.txt`); prior `json.scanner.make_scanner`, `_csv` iterator propagation, and pickle `_class_cleanups` blockers are closed, while remaining gaps are concentrated in unittest-runner integration paths, `pickle`/`io` protocol behavior, and pickletools `super().__getattr__` execution-path parity. These are tracked in `tests/cpython_allowlist_strict.txt` and `docs/STUB_ACCOUNTING.md`.
- Strict stdlib harness lane is now first-class: `tests/cpython_suite_strict_stdlib.txt` executes modules via `unittest` in `tests/cpython_harness.rs`, with owned allowlist tracking in `tests/cpython_allowlist_strict.txt`; `test/test_csv.py` is currently executed in an isolated subprocess with a timeout guard to avoid runaway hangs while csv/io parity blockers remain open. The strict lane is opt-in in local frequent loops (`PYRS_RUN_STRICT_STDLIB=1` or `PYRS_PARITY_STRICT=1`) and remains enabled by parity-gate runs.
- Metaclass/type parity follow-up landed for strict pickle paths: type-derived metaclass invocation now builds class objects through class-construction logic (not instance fallback), and `type(ClassObj)` now reports non-builtin metaclasses correctly. This closes the prior `object.__init__() takes exactly one argument` strict-lane failure in dynamic-class pickle flows.
- Coverage quality gate moved from advisory to enforced soft-floor policy in CI: `.github/workflows/parity-gate.yml` now runs `scripts/run_coverage_gate.sh` with floor envs (`regions >= 70`, `functions >= 65`, `lines >= 70`), while local runs remain report-only unless explicit enforcement env vars are set.
- Coverage gate defaults are now encoded directly in `scripts/run_coverage_gate.sh` (`70/65/70`) when enforcement mode is enabled, reducing CI/local drift risk from missing env wiring.
- Differential malformed-input coverage was expanded in `tests/differential_cpython.rs` for `json`, `_csv`, and pickle object-protocol malformed-contract behavior to catch parser/decoder contract drift earlier.
- Unit-coverage expansion batch landed for low-coverage core modules: new module-local tests now cover runtime container/index helpers and truthiness/repr edges (`src/runtime/mod.rs`), arithmetic/comparison/set-op edge semantics (`src/vm/ops.rs`), regex match-object internals + CSV-sniffer regex shims (`src/vm/stdlib/re.rs`), and stricter JSON number grammar/kwargs paths (`src/vm/stdlib/json.rs`).
- Leak/hang guard coverage expanded: `tests/gc_regression.rs` now includes repeated stdlib import/exec + GC bounded-growth checks, and `tests/cpython_harness.rs` now regression-tests strict subprocess timeout behavior for hanging programs.
- Core `try/finally` return-flow parity fix landed: compiler lowering now defers `return` inside `try` blocks through `finally` epilogues (including nested `try/finally` and `try/except/finally`), restoring required cleanup behavior for stdlib helpers like `import_helper.import_fresh_module`; this closes the `_frozen_importlib_external` import-suite blocker and removes the prior strict-lane runaway-growth failure mode tied to skipped `finally` cleanup.
- Async control-flow parity fix landed: lowered `async for` now clears `StopAsyncIteration` through normal handler flow and exits via an explicit exhaustion flag, preventing stale active-exception leakage into subsequent calls.
- User-defined exception class-constructor parity fix landed: exception subclasses without explicit `__init__` now accept positional args and populate `.args` (instead of failing with `class constructor takes no arguments`).
- Fallback resource-shim correctness fix landed: `pkgutil.get_data` and `importlib.resources` shim `read_*` paths now return bytes/text payloads, not file-handle objects.
- Import-suite closure landed for prior `zipfile`/`struct.Struct` constructor blockers: curated import harness now passes with empty allowlist (`test_pkgutil.py` and `test_importlib/resources/test_resource.py` are green).
- `_csv` reader/dialect hardening landed for strict-parity paths: `register_dialect(..., dialect, **kwargs)` override semantics now merge/validate correctly; reader handles `strict`, `quoting` (`QUOTE_NONE`/`QUOTE_NONNUMERIC`/`QUOTE_STRINGS`/`QUOTE_NOTNULL`), skip-initial-space edge cases, EOF escape behavior, and unquoted-newline error contracts with dedicated regressions.
- Small-thread-stack sensitivity remains tracked: the `ipaddress` bigint import regression test now runs in a dedicated larger-stack thread; runtime stack-depth hardening for constrained embedding stacks remains an explicit follow-up in `docs/STUB_ACCOUNTING.md`.
- Engineering audit backlog is now explicit in `docs/ALGO_AUDIT_BACKLOG.md`; current open P0 item includes `list.sort` mutation/clone parity closure (`AQ-001`).
- Strict-pickle timeout reduction follow-up landed: `object.__reduce_ex__` now returns builtin singleton names for `Ellipsis`/`NotImplemented`, removing the prior singleton pickling hang path and adding direct unit/regression coverage.
- Bytes API parity follow-up landed: `bytes.join(...)` now supports iterable bytes-like inputs (`bytes`/`bytearray`/`memoryview`) with dedicated VM regression coverage, closing a strict `test_pickle` blocker (`AttributeError: bytes has no attribute 'join'`).
- Strict stdlib lane remains P0-blocked on timeouts for `test_pickle.py` and `test_pickletools.py` at the current per-module watchdog (`PYRS_STRICT_HARNESS_TIMEOUT_SECS=120`); no runaway harness loop is observed, and remaining closure work is concentrated in pickle/unittest runtime behavior and performance.

### Milestone 14 — Performance, Observability, and Runtime Hooks (P1/P2/P3)
DoD:
- Baseline benchmark suite (including pyperformance subset plus project workloads) is automated and tracked for regressions.
- Production optimization stack is implemented and validated (compiler-level simplifications, VM dispatch/lookup caches, and container/runtime hot-path improvements), with parity gates proving no semantic regressions.
- Algorithmic-complexity and clone/allocation quality gates from `docs/ENGINEERING_GATES.md` are enforced in CI for hot-path runtime/container operations, and all Milestone 14 items in `docs/ALGO_AUDIT_BACKLOG.md` are closed.
- VM/runtime codebase is decomposed into cohesive modules (reducing monolithic file hotspots) with no behavior regressions.
- Debug and profiling hooks (`sys.settrace`, `sys.setprofile`, runtime metrics, profiling workflow) are operational.
- JIT/embedding hook points in IR/VM boundaries are explicitly documented and covered by non-regression tests.

### Milestone 15 — Native Extension Ecosystem Compatibility (P0/P1)
DoD:
- Limited C-API/abi3 extension loading works across the supported API surface (import, type/object creation, method dispatch, memory/error propagation) with compatibility tests per surface.
- HPy execution path is available with explicit compatibility matrix and tests.
- Ecosystem smoke/regression tests cover multiple extension-backed package classes (numeric, crypto, parsing, and data-processing families) with documented pass/fail matrix and blockers.
- Any unsupported extension surfaces fail with explicit diagnostics, not silent misbehavior.

### Milestone 16 — Release Hardening and Production Certification (P0/P1)
DoD:
- Security/reliability CI gates are mandatory for release branches (ASan/UBSan, differential CPython runs, crash reproducer retention).
- Cross-platform qualification gates pass on Linux, macOS, and Windows for defined compatibility suite.
- Release pipeline produces reproducible and signed artifacts with SBOM generation and semantic-versioning policy.
- Production playbook exists for incident triage, rollback strategy, reproducible artifact verification, and regression-response SLA.

## Immediate next steps
- Execute Milestone 13 closure batches: long-tail language/runtime parity semantics plus stdlib/import/packaging usability blockers.
- Execute the engineering audit backlog in `docs/ALGO_AUDIT_BACKLOG.md` in priority order (starting with P0 semantic-contract violations).
- Expand CPython harness breadth beyond current curated suites while preserving empty-allowlist policy for in-scope tests.
- Harden CI parity policy beyond baseline workflow (`.github/workflows/parity-gate.yml`) for release-branch blocking and multi-platform parity lanes.
- Keep Milestone 15 and Milestone 16 acceptance criteria visible during architecture choices so extension and release hardening paths remain unblocked.

## Testing Focus Note
After Milestone 2 (CPython bytecode compatibility), prioritize a testing push:
- CPython `Lib/test` subset harness.
- Property/fuzz tests for parser + VM semantics.
- Integration tests with real scripts and package layouts.

Status:
- CPython `Lib/test` subset harness is active by default (set `PYRS_CPYTHON_LIB` to point at CPython `Lib`; set `PYRS_CPYTHON_OPTIONAL=1` only when intentionally skipping on machines without local CPython sources).
- Differential testing is active (`tests/differential_cpython.rs`) and parser/VM fuzzing is active (`tests/fuzz_parser_vm.rs`).
- Curated real-world smoke tests are active and execute in a constrained subprocess profile (`tests/realworld_smoke.rs`).
