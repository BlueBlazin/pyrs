# Production Readiness Accounting (CPython 3.14)

This is the living, exhaustive checklist of what must exist for a fully capable, production‑ready CPython‑compatible interpreter. It is intentionally broader than current milestones so we never lose sight of full parity.

Status flags: `[ ]` not started, `[~]` in progress, `[x]` complete.
Priority tags: `P0` (blocking), `P1` (major), `P2` (performance/QoL), `P3` (future‑proofing).

## Milestone Coverage Map
Every category below is mapped to the revised roadmap milestones in `docs/ROADMAP.md`, so the remaining plan has no known gaps.

- Language & Grammar -> Milestone 7
- Semantic Analysis & Compilation -> Milestones 5 and 7
- Bytecode & VM Execution -> Milestones 4 and 5
- Runtime Object Model & Data Model -> Milestone 8
- Builtins -> Milestone 9
- Import System -> Milestone 6
- Standard Library Coverage -> Milestones 9 and 10
- Tooling & UX -> Milestones 12 and 13
- Testing & QA -> Milestone 11
- Performance & Profiling -> Milestone 12
- Concurrency & Platform -> Milestone 10
- Interop & Extensibility -> Milestone 14
- Packaging & Distribution -> Milestone 13

**Language & Grammar**
- [~] P0: Full 3.14 tokenizer parity (strings, bytes, numeric literals, f‑strings, comments, indents).
- [~] P0: Full 3.14 grammar coverage (all statements/expressions).
- [ ] P0: Decorators on functions/classes.
- [ ] P0: Assignment expressions (`:=`).
- [x] P0: `yield`, `yield from` (lazy suspension/resume with delegation semantics).
- [ ] P0: `async`/`await`, async comprehensions, async generators.
- [ ] P0: `try/except*` (exception groups) and `ExceptionGroup` semantics.
- [ ] P0: Pattern matching (`match`/`case`).
- [ ] P0: Comprehensions + generator expressions with correct scoping.
- [ ] P0: f‑strings + format spec mini‑language (PEP 701 compatible).
- [x] P1: Type annotations (`x: T`, `def f(x: T) -> U`, class/instance annotations).
- [ ] P1: Annotation evaluation semantics matching 3.14 (deferred vs eager).
- [ ] P1: Type parameter syntax / `type` statements (PEP 695 family).

**Semantic Analysis & Compilation**
- [~] P0: Scope analysis (locals/globals/nonlocals/freevars/cellvars).
- [ ] P0: Correct comprehension scope isolation.
- [ ] P0: `exec`/`eval` semantics and dynamic scope effects.
- [ ] P0: `__future__` flags and compiler feature gating.
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
- [~] P0: Core objects (int/str/list/tuple/dict/bool/None) + identity + refcount + cycle GC.
- [ ] P0: Full numeric tower (int big‑ints, float, complex) + coercion rules.
- [ ] P0: bytes/bytearray/memoryview and buffer protocol.
- [ ] P0: set/frozenset.
- [ ] P0: Unicode/codec behavior parity (including error handlers).
- [ ] P0: Descriptor protocol (`__get__`, `__set__`, `__delete__`).
- [ ] P0: Attribute lookup parity (`__getattribute__`, `__getattr__`, `__setattr__`, `__delattr__`).
- [ ] P0: MRO + metaclasses + `super()` semantics.
- [ ] P0: `__slots__` and instance layout rules.
- [ ] P1: Weakrefs, `gc` module hooks, finalizers.
- [ ] P1: Frame objects + `inspect` compatibility (locals/globals/stack).

**Builtins**
- [~] P0: Core builtin set (print, len, range, etc.).
- [ ] P0: Full builtin set (open, iter, next, vars, locals, globals, getattr/setattr/delattr, dir, help, input, etc.).
- [~] P1: `__import__` baseline (`name`/`fromlist`/`level` semantics wired to current loader; full importlib semantics pending).

**Import System**
- [~] P0: File‑based imports + module cache + basic packages (including relative `from .` resolution, `sys.path` lookup, `sys.modules` exposure, package `__path__` lookup for submodules, baseline `sys.meta_path` path-finder control, and baseline module metadata fields).
- [ ] P0: Full importlib machinery (`ModuleSpec`, `__loader__`, `__package__`, `__path__`).
- [~] P0: Namespace packages (filesystem directory namespace package loading with aggregated `__path__`; full importlib namespace semantics pending).
- [ ] P0: Zip/bytecode imports.
- [ ] P1: `importlib.resources`, `pkgutil`, entry points.

**Standard Library Coverage**
- [~] P0: Minimal builtins subset.
- [ ] P0: `sys`, `types`, `inspect`, `io` (foundation for many libs).
- [ ] P0: `os`, `pathlib`, `stat`, `errno`, `time`, `datetime` (process/FS core).
- [ ] P1: `re`, `json`, `math`, `decimal`, `fractions`, `collections`, `functools`, `itertools`, `operator`.
- [ ] P1: `threading`, `multiprocessing`, `asyncio`, `concurrent.futures`.
- [ ] P1: `subprocess`, `socket`, `ssl`, `http`, `urllib`.
- [ ] P2: `logging`, `argparse`, `unittest`, `doctest`.
- [ ] P2: `typing`, `dataclasses`, `enum`, `contextvars`.

**Tooling & UX**
- [ ] P1: REPL parity (interactive hooks, displayhook, completion hooks).
- [ ] P1: `pydoc`/help output parity.
- [ ] P1: `site` initialization and `ensurepip`/venv story.
- [ ] P2: Rich error messages with caret spans and suggestions.

**Testing & QA**
- [~] P0: CPython `Lib/test` subset harness.
- [ ] P0: Large `Lib/test` subset + CI gating.
- [ ] P1: Differential tests vs CPython on real‑world scripts.
- [ ] P1: Fuzzing for parser + VM (syntax + runtime).
- [ ] P2: Deterministic reproduction harness for crash bugs.

**Performance & Profiling**
- [ ] P1: Baseline performance suite (pyperformance subset).
- [ ] P1: Profiling hooks + flamegraph support.
- [ ] P2: Adaptive opcodes / inline caches.
- [ ] P2: GC/allocator tuning and object layout optimizations.

**Concurrency & Platform**
- [ ] P0: GIL correctness and thread safety.
- [ ] P1: Signals, `signal` module semantics.
- [ ] P1: Cross‑platform parity (Linux/macOS/Windows).

**Interop & Extensibility**
- [ ] P1: Stable ABI/FFI plan (HPy or limited C‑API) documented.
- [ ] P2: Embedding API for Rust and C/C++ hosts.
- [ ] P3: JIT hooks at IR/VM boundaries (no implementation yet).

**Packaging & Distribution**
- [ ] P1: `pip` compatibility (pure‑Python wheels).
- [ ] P2: Binary distribution artifacts and reproducible builds.
