# Production Readiness Accounting (CPython 3.14)

This is the living, exhaustive checklist of what must exist for a fully capable, production‑ready CPython‑compatible interpreter. It is intentionally broader than current milestones so we never lose sight of full parity.

Status flags: `[ ]` not started, `[~]` in progress, `[x]` complete.
Priority tags: `P0` (blocking), `P1` (major), `P2` (performance/QoL), `P3` (future‑proofing).

**Language & Grammar**
- [~] P0: Full 3.14 tokenizer parity (strings, bytes, numeric literals, f‑strings, comments, indents).
- [~] P0: Full 3.14 grammar coverage (all statements/expressions).
- [ ] P0: `yield`, `yield from`.
- [ ] P0: `async`/`await`, async comprehensions, async generators.
- [ ] P0: `try/except*` (exception groups) and `ExceptionGroup` semantics.
- [ ] P0: Pattern matching (`match`/`case`).
- [ ] P0: Comprehensions + generator expressions with correct scoping.
- [ ] P1: Type annotations (`x: T`, `def f(x: T) -> U`, class/instance annotations).
- [ ] P1: Annotation evaluation semantics matching 3.14 (deferred vs eager).

**Semantic Analysis & Compilation**
- [~] P0: Scope analysis (locals/globals/nonlocals/freevars/cellvars).
- [ ] P0: Correct comprehension scope isolation.
- [ ] P0: `exec`/`eval` semantics and dynamic scope effects.
- [ ] P1: Annotation capture into `__annotations__` (module/class/function).
- [ ] P1: Constant folding and peephole optimizations (no semantic changes).
- [ ] P2: Bytecode verification pass (stack effects, jump targets, invalid op sequences).

**Bytecode & VM Execution**
- [~] P0: CPython 3.14 opcode table decode.
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
- [ ] P0: Descriptor protocol (`__get__`, `__set__`, `__delete__`).
- [ ] P0: Attribute lookup parity (`__getattribute__`, `__getattr__`, `__setattr__`, `__delattr__`).
- [ ] P0: MRO + metaclasses + `super()` semantics.
- [ ] P0: `__slots__` and instance layout rules.
- [ ] P1: Weakrefs, `gc` module hooks, finalizers.
- [ ] P1: Frame objects + `inspect` compatibility (locals/globals/stack).

**Builtins**
- [~] P0: Core builtin set (print, len, range, etc.).
- [ ] P0: Full builtin set (open, iter, next, vars, locals, globals, getattr/setattr/delattr, dir, help, input, etc.).
- [ ] P1: `__import__` with importlib semantics.

**Import System**
- [~] P0: File‑based imports + module cache + basic packages.
- [ ] P0: Full importlib machinery (`ModuleSpec`, `__loader__`, `__package__`, `__path__`).
- [ ] P0: Namespace packages.
- [ ] P0: Zip/bytecode imports.
- [ ] P1: `importlib.resources`, `pkgutil`, entry points.

**Standard Library Coverage**
- [~] P0: Minimal builtins subset.
- [ ] P0: `sys`, `types`, `inspect`, `io` (foundation for many libs).
- [ ] P0: `os`, `pathlib`, `stat`, `errno`, `time`, `datetime` (process/FS core).
- [ ] P1: `re`, `json`, `math`, `decimal`, `fractions`, `collections`.
- [ ] P1: `threading`, `multiprocessing`, `asyncio`, `concurrent.futures`.
- [ ] P1: `subprocess`, `socket`, `ssl`, `http`, `urllib`.
- [ ] P2: `logging`, `argparse`, `unittest`, `doctest`.

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
