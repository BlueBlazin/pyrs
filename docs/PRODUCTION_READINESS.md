# Production Readiness Accounting (CPython 3.14)

This is the living, exhaustive checklist of what must exist for a fully capable, production‑ready CPython‑compatible interpreter. It is intentionally broader than current milestones so we never lose sight of full parity.

Status flags: `[ ]` not started, `[~]` in progress, `[x]` complete.
Priority tags: `P0` (blocking), `P1` (major), `P2` (performance/QoL), `P3` (future‑proofing).

## Milestone Coverage Map
Every category below is mapped to the revised roadmap milestones in `docs/ROADMAP.md`, so the remaining plan has no known gaps.
Milestone 12 closure record is tracked in `docs/MILESTONE_12_BACKLOG.md`.

- Language & Grammar -> Milestones 7, 12, and 13
- Semantic Analysis & Compilation -> Milestones 7, 12, and 13
- Bytecode & VM Execution -> Milestones 5, 12, and 13
- Runtime Object Model & Data Model -> Milestones 8, 9, 12, and 13
- Builtins -> Milestones 9 and 13
- Import System -> Milestones 6 and 13
- Standard Library Coverage -> Milestones 9, 10, and 13
- Tooling & UX -> Milestones 13 and 14
- Testing & QA -> Milestones 11, 13, and 16
- Performance & Profiling -> Milestone 14
- Concurrency & Platform -> Milestones 10 and 16
- Interop & Extensibility -> Milestones 14 and 15
- Packaging & Distribution -> Milestones 13 and 16
- Security, Reliability, and Release Engineering -> Milestone 16

**Engineering Quality Gates**
- [~] P0: Mandatory quality-gate process is defined and active (`docs/ENGINEERING_GATES.md`) with tracked issues in `docs/ALGO_AUDIT_BACKLOG.md`.
- [ ] P0: All P0 semantic-contract audit items are closed with CPython differential proof (for example mutation-sensitive in-place operations).
- [ ] P1: Algorithmic complexity and clone/allocation gate automation is enforced in CI for runtime hot paths.

**Language & Grammar**
- [~] P0: Full 3.14 tokenizer parity (strings, bytes, numeric literals, f‑strings, comments, indents).
- [~] P0: Full 3.14 grammar coverage (all statements/expressions).
- [x] P0: Decorators on functions/classes.
- [x] P0: Assignment expressions (`:=`).
- [x] P0: `yield`, `yield from` (lazy suspension/resume with delegation semantics).
- [~] P0: `async`/`await`, async comprehensions, async generators (core coroutine/runtime semantics implemented; async comprehensions and deep parity edge cases pending).
- [~] P0: `try/except*` (exception groups) and `ExceptionGroup` semantics (group splitting, per-handler subgroup execution, and remainder reraising implemented; full edge/API parity pending).
- [~] P0: Pattern matching (`match`/`case`) broad families implemented (literal/capture/guard/sequence/mapping/class/or/as/star); full edge parity and remaining pattern forms pending.
- [x] P0: Comprehensions + generator expressions with correct scoping.
- [~] P0: f‑strings + format spec mini‑language (PEP 701 compatible) (baseline interpolation lowering implemented; full spec pending).
- [x] P1: Type annotations (`x: T`, `def f(x: T) -> U`, class/instance annotations).
- [~] P1: Annotation evaluation semantics matching 3.14 (baseline `from __future__ import annotations` deferral-to-string behavior implemented; full parity still pending).
- [~] P1: Type parameter syntax / `type` statements (PEP 695 family) (`def`/`class` header type params and baseline `type` statement lowering are implemented; full runtime semantics pending).

**Semantic Analysis & Compilation**
- [~] P0: Scope analysis (locals/globals/nonlocals/freevars/cellvars).
- [x] P0: Correct comprehension scope isolation.
- [ ] P0: `exec`/`eval` semantics and dynamic scope effects.
- [~] P0: `__future__` flags and compiler feature gating (placement + unknown-feature validation implemented; future-import statements are compile-time-only, and baseline stdlib `__future__.all_feature_names` compatibility is wired; full flag/object parity pending).
- [x] P1: Annotation capture into `__annotations__` (module/class/function, eager evaluation path currently implemented).
- [ ] P1: Constant folding and peephole optimizations (no semantic changes).
- [~] P2: Bytecode verification pass (jump-target + stack-shape checks implemented for supported translation paths; full verifier coverage pending).

**Bytecode & VM Execution**
- [x] P0: CPython 3.14 opcode table decode.
- [x] P0: Supported-subset `.pyc` reader/writer parity (headers + marshal code object read/write) and translation validation (jump/stack checks).
- [ ] P0: Full opcode execution parity (all 3.14 opcodes).
- [ ] P0: `.pyc` read/write parity with CPython 3.14 (flags, hash/timestamp, marshal).
- [ ] P0: Precise exception propagation and frame unwinding semantics.
- [~] P0: Tracebacks with filename/line/column and frame names.
- [ ] P1: `sys.settrace` / `sys.setprofile` hooks.
- [ ] P2: Inline cache / adaptive opcode support.

**Runtime Object Model & Data Model**
- [~] P0: Core objects (int/float/str/list/tuple/dict/bool/None) + identity + refcount + cycle GC (runtime dict/set now use hash-indexed container objects with insertion-order backing vectors; long-tail hash/equality semantics and perf closure are tracked separately below).
- [~] P0: Full numeric tower (int big‑ints, float, complex) + coercion rules (`Value::BigInt` now covers core large-int arithmetic/bitwise/shift/comparison paths, Python-floor `//`/`%`/`divmod`, large `int(...)` parsing (including stricter base-0/underscore validation), `%x`/`%X`/`%o` formatting, bigint-aware `int.bit_length`, arbitrary-size `int.from_bytes`/`int.to_bytes`, and lazy large-stop `range` support; long-tail arbitrary-precision conversion/format/error-text edges remain pending).
- [~] P0: bytes/bytearray/memoryview and buffer protocol (core bytes-like runtime types implemented; full buffer protocol pending).
- [x] P0: set/frozenset.
- [~] P0: Hash-based dict/set/frozenset semantic parity (`__hash__` contract, unhashable key/item rejection, CPython-compatible lookup/update behavior) (core unhashable key/item rejection is now enforced on constructor/update/assignment/membership flows, literal dict construction, `dict.fromkeys(...)`, and `collections.Counter(...)`; dict keyed operations now use a CPython-style open-addressing probe table (`Empty`/`Dummy`/`Occupied` slots with perturb probing), set relationship checks enforce hashability, and set/frozenset keep compact hash-bucket indexing; dict equality is insertion-order independent and set/frozenset equality is value-based including cross-type equality; hash-table storage growth/load-factor and long-tail edge parity remain pending).
- [ ] P0: `json` semantic/safety/perf closure (full CPython behavior for encode/decode options and error contracts, malformed-input hardening, and benchmark baselines).
- [ ] P0: `_csv`/`csv` semantic/safety/perf closure (full parser/writer behavior and error-text parity, malformed-input hardening, and benchmark baselines).
- [ ] P0: `pickle`/`pickletools`/`copyreg` semantic/safety/perf closure (protocol/opcode/runtime parity plus benchmark baselines).
- [~] P0: Unicode/codec behavior parity (including error handlers) (`codecs.encode`/`decode` foundations for `utf-8`/`utf-16`/`utf-32`/`ascii`/`latin-1` plus `raw-unicode-escape`/`unicode-escape`, and baseline `codecs.escape_decode`, with `strict`/`ignore`/`replace` implemented; full parity pending).
- [~] P0: Descriptor protocol (`__get__`, `__set__`, `__delete__`) (core VM descriptor hooks implemented; metaclass/slot edge parity pending).
- [~] P0: Attribute lookup parity (`__getattribute__`, `__getattr__`, `__setattr__`, `__delattr__`) (instance hooks plus custom `__getattribute__` override and `object.__getattribute__` baseline are implemented; full fallback/error-edge parity pending).
- [~] P0: MRO + metaclasses + `super()` semantics (C3 MRO + explicit `super(type, obj)` implemented; class-header `metaclass=` and `__build_class__` metaclass kwargs supported; resolved-metaclass tracking and base-metaclass conflict detection are implemented; full metaclass precedence semantics pending).
- [~] P0: `__slots__` and instance layout rules (core restrictions implemented, including empty-slot and `__dict__` slot behavior; full layout parity pending).
- [ ] P1: Weakrefs, `gc` module hooks, finalizers.
- [ ] P1: Frame objects + `inspect` compatibility (locals/globals/stack).

**Builtins**
- [x] P0: Stub accounting is enforced (`docs/STUB_ACCOUNTING.md`, generated `docs/NOOP_BUILTIN_INVENTORY.txt`, and CI gate `tests/noop_inventory.rs`).
- [~] P0: Core builtin set (print, len, range, float/int coercions, numeric ops, `set`/`frozenset`, bytes-like constructors, `complex`, `iter`/`next`, `type`, `ord`/`chr`, and random module foundations; `range` now supports lazy bigint-backed large-stop iteration to avoid eager huge-list materialization).
- [ ] P0: Full builtin set (open, iter, next, vars, locals, globals, dir, help, input, etc.; `getattr`/`setattr`/`delattr`/`hasattr` and explicit-args `super` implemented).
- [x] P1: `__import__` baseline (`name`/`fromlist`/`level` semantics wired to loader path).

**Import System**
- [x] P0: File‑based imports + module cache + basic packages (including relative `from .` resolution, `sys.path` lookup, `sys.modules` exposure, package `__path__` lookup for submodules, `sys.meta_path`/`sys.path_hooks`/`sys.path_importer_cache` contracts, and module metadata/spec population).
- [~] P0: Full importlib machinery (`ModuleSpec`, `__loader__`, `__package__`, `__path__`) for supported pure-Python loaders (`pyrs.SourceFileLoader`, `pyrs.NamespaceLoader`) and `importlib` helper APIs (`import_module`, `find_spec`, `importlib.util.find_spec`).
- [x] P0: Namespace packages (filesystem directory namespace package loading with aggregated `__path__`).
- [~] P0: Zip/bytecode imports (sourceless `.pyc` module/package imports implemented; zip-import parity pending).
- [~] P1: `importlib.resources`, `pkgutil`, entry points (fallback shim workflows implemented for stdlib-less environments; full CPython parity pending).

**Standard Library Coverage**
- [~] P0: Minimal builtins subset.
- [~] P0: `random` module foundations (`seed`, `random`, `randrange`, `randint`, `getrandbits`, `choice`, `choices`, `shuffle`).
- [~] P0: `codecs` foundations (`encode`/`decode` for `utf-8`/`utf-16`/`utf-32`/`ascii`/`latin-1` with `strict`/`ignore`/`replace`).
- [~] P0: `sys`, `types`, `inspect`, `io` (foundation for many libs).
- [~] P0: `os`, `pathlib`, `stat`, `errno`, `time`, `datetime` (process/FS core; `os`/`posix` now include non-`NoOp` `open`/`close`/`write`/`getenv`/`isatty`/`stat`/`lstat`/`rmdir`/`utime`/`scandir` + wait-status helpers, and `datetime` now exports baseline `date`/`timedelta` symbols; full module parity pending).
- [~] P0: `_io.open` CPython file-object semantics (`TextIOWrapper.__init__` wrapping and `readlines()` are implemented; mode validation, binary/text argument compatibility checks, buffering guardrails, and opener/FD closefd handling now track CPython `_io_open_impl` baselines. Full buffered-class behavior, newline translation details, and wider stream-object parity are still required for complete `tempfile`/stdlib closure).
- [ ] P0: Full `json` parity and hardening (`test_json` closure, differential malformed-input coverage, performance baselines).
- [ ] P0: Full `_csv`/`csv` parity and hardening (`test_csv` closure, malformed-input coverage, performance baselines).
- [ ] P0: Full `pickle`/`pickletools`/`copyreg` parity and hardening (`test_pickle`, `test_pickletools`, `test_copyreg` closure, protocol coverage, performance baselines).
- [~] P0: Prefer official CPython pure-Python stdlib implementations wherever feasible and keep native VM handlers minimal/isolated (`src/vm/stdlib/` extraction in progress; `json`/`re`/`_csv` plus pickle object-protocol helpers are isolated).
- [~] P1: `re`, `math`, `decimal`, `fractions`, `collections`, `functools`, `itertools`, `operator` (`math` core stub surface removed; long-tail parity still pending).
- [~] P1: `threading`, `multiprocessing`, `asyncio`, `concurrent.futures` (`asyncio`/`threading` foundations implemented; broader module parity pending).
- [ ] P1: `subprocess`, `socket`, `ssl`, `http`, `urllib`.
- [ ] P2: `logging`, `argparse`, `unittest`, `doctest`.
- [ ] P2: `typing`, `dataclasses`, `enum`, `contextvars`.

**Tooling & UX**
- [ ] P1: REPL parity (interactive hooks, displayhook, completion hooks).
- [ ] P1: `pydoc`/help output parity.
- [ ] P1: `site` initialization and `ensurepip`/venv story.
- [ ] P2: Rich error messages with caret spans and suggestions.
- [~] P2: VM/runtime module decomposition away from monolithic hotspots (`src/vm/ops.rs` extracted for arithmetic/comparison kernels and native stdlib handlers are now split into `src/vm/stdlib/{json,re,csv}.rs`; further split required).

**Testing & QA**
- [x] P0: CPython `Lib/test` subset harness first-class (`tests/cpython_harness.rs`) with split language/import/strict-stdlib suites and owned allowlists.
- [~] P0: Current curated CPython language/import harness suites are near zero-allowlist and now include `test/test_set.py`, `test/test_list.py`, `test/test_tuple.py`, `test/test_slice.py`, `test/test_format.py`, `test/test_configparser.py`, `test/test_base64.py`, `test/test_binascii.py`, `test/test_bisect.py`, `test/test_copy.py`, `test/test_copyreg.py`, `test/test_csv.py`, `test/test_fnmatch.py`, `test/test_genericalias.py`, `test/test_heapq.py`, `test/test_pprint.py`, `test/test_reprlib.py`, `test/test_sched.py`, `test/test_statistics.py`, `test/test_textwrap.py`, `test/test_tokenize.py`, `test/test_json/__init__.py`, `test/test_dataclasses/__init__.py`, and `test/test_enum.py`; `tests/cpython_allowlist.txt` and `tests/cpython_allowlist_strict.txt` are currently empty, while deferred pickle strict coverage is tracked in `tests/cpython_suite_deferred_pickle.txt`.
- [x] P0: Prior curated language-suite blocker (`test/test_enum.py`) is closed.
- [x] P0: Prior curated import-suite blockers (`test/test_pkgutil.py`, `test/test_importlib/resources/test_resource.py`) on `zipfile`/`struct.Struct` constructor paths are closed; curated language+import suites are green with empty allowlist.
- [x] P0: Strict standalone `test_csv` unittest execution now passes under the strict subprocess harness (`tests/cpython_suite_strict_stdlib.txt`), including `BadWriter.write` `OSError` propagation and prior `_csv` iterator/path blockers.
- [~] P0: Large `Lib/test` subset + CI gating (suite growth + allowlist reduction in progress).
- [ ] P0: Full CPython module-suite closure for `json`/`csv`/`pickle` stack (`test_json`, `test_csv`, `test_pickle`, `test_pickletools`, `test_copyreg`) with active strict lane closure (`tests/cpython_suite_strict_stdlib.txt`) plus deferred pickle lane burn-down (`tests/cpython_suite_deferred_pickle.txt`).
- [~] P0: Pure-Python stdlib-first migration is active per `docs/STDLIB_MIGRATION_PLAN.md` (`json` now prefers CPython pure-import paths by default when stdlib is discoverable, while native fallback remains for stdlib-less environments). Continue this pattern for remaining `csv`/`pickle`/`re` closure work.
- [x] P0: Prior strict pickle metaclass-construction blocker is closed (`type`-derived metaclass invocation no longer falls through instance/object-init paths; dynamic-class pickle flow no longer fails with `object.__init__() takes exactly one argument`).
- [x] P0: Prior strict pickle singleton/reducer and bytes API blockers are closed (`object.__reduce_ex__` now returns builtin singleton names for `Ellipsis`/`NotImplemented`; `bytes.join` is implemented for bytes-like iterables).
- [x] P0: Prior strict pickle frame/dispatch blockers are closed (protocol 4/5 frame splitting no longer corrupts large-opcode payloads, and class/instance `dispatch_table` now forwards to pure fallback picklers).
- [x] P0: Prior strict pickle `myint` copy-equality blocker is closed (runtime `int`-subclass equality now compares int-backed instance payloads, so `myint(4) == myint(4)` and strict `test_pickle` `test_misc` equality paths match CPython behavior).
- [x] P1: Differential tests vs CPython on curated script corpus (`tests/differential_cpython.rs`).
- [x] P1: Fuzzing for parser + VM (syntax + runtime) (`tests/fuzz_parser_vm.rs` + arithmetic fuzz suites).
- [x] P1: Module-local unit tests for high-risk helper internals (`src/vm/containers.rs`, `src/vm/stdlib/json.rs`, `src/vm/stdlib/csv.rs`) are now in place as an early regression gate.
- [~] P1: Module-local unit tests for high-risk helper internals are expanded (`src/vm/containers.rs`, `src/vm/ops.rs`, `src/runtime/mod.rs`, `src/vm/stdlib/json.rs`, `src/vm/stdlib/re.rs`, `src/vm/stdlib/csv.rs`); continue closing low-coverage branches in runtime/import paths.
- [~] P1: Dedicated GC/leak regression lane (`tests/gc_regression.rs`) is active; continue expanding it with strict-stdlib reproductions until all historical growth/hang incidents are root-caused and closed.
- [x] P1: Strict-harness subprocess timeout behavior is regression-tested (`tests/cpython_harness.rs`) to prevent hang-driven unbounded RAM growth from hiding parity failures.
- [x] P1: Coverage gate automation exists in CI (`scripts/run_coverage_gate.sh` with 70/65/70 soft floors when enforcement is enabled).
- [x] P1: Curated real-world smoke/regression suite with constrained subprocess profile (`tests/realworld_smoke.rs`, `scripts/run_parity_gate.sh`).
- [~] P1: Stdlib-import regression probes for bigint-heavy paths (`ipaddress` import path has active regression coverage and now executes in a dedicated larger-stack test thread; remaining constrained-stack hardening plus `_testinternalcapi.hamt` parity are tracked in `docs/STUB_ACCOUNTING.md`).
- [ ] P2: Deterministic reproduction harness for crash bugs.

**Performance & Profiling**
- [ ] P1: Baseline performance suite (pyperformance subset).
- [ ] P1: Profiling hooks + flamegraph support.
- [ ] P1: Close all P1 items in `docs/ALGO_AUDIT_BACKLOG.md` with benchmark-backed evidence.
- [ ] P2: Adaptive opcodes / inline caches.
- [ ] P2: GC/allocator tuning and object layout optimizations.
- [ ] P2: Container hot-path architecture/performance parity (dict/set hash-table internals, growth strategy, and memory behavior).

**Concurrency & Platform**
- [ ] P0: GIL correctness and thread safety.
- [~] P1: Signals, `signal` module semantics (handler registration/lookup/raise foundations implemented; full OS-level parity pending).
- [ ] P1: Cross‑platform parity (Linux/macOS/Windows).

**Interop & Extensibility**
- [ ] P0: Limited C-API/abi3 extension loading and execution parity for supported API surface.
- [ ] P1: HPy loading/execution path with explicit compatibility matrix.
- [ ] P1: Stable ABI/FFI plan (HPy or limited C‑API) documented.
- [ ] P2: Embedding API for Rust and C/C++ hosts.
- [ ] P3: JIT hooks at IR/VM boundaries (no implementation yet).

**Packaging & Distribution**
- [ ] P1: `pip` compatibility (pure‑Python wheels).
- [ ] P2: Binary distribution artifacts and reproducible builds.

**Security, Reliability, and Release Engineering**
- [ ] P0: Sanitizer-gated CI on release profiles (ASan/UBSan and platform-appropriate thread/memory checks).
- [~] P0: Parity-regression blocker policy in CI (baseline workflow in `.github/workflows/parity-gate.yml`; release-branch policy hardening pending).
- [ ] P1: Incident triage runbook + crash reproducer retention for release lines.
- [ ] P1: Cross-platform release qualification matrix (Linux/macOS/Windows).
- [ ] P2: Signed artifacts + SBOM generation for release bundles.
