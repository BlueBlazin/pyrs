# Compatibility Tracker (CPython 3.14)

This file tracks compatibility by subsystem.
For release blockers, see `docs/PRODUCTION_READINESS.md`.
For partial implementations and owners, see `docs/STUB_ACCOUNTING.md`.

Status:
- `[ ]` not started
- `[~]` in progress
- `[x]` complete

## Language and Parser
- `[x]` Core parser/compiler foundations through Milestone 12
- `[x]` Major modern syntax baseline (`match`/`case`, comprehensions, assignment expressions, async syntax, decorators, annotations baseline)
- `[x]` `\N{...}` Unicode-name escape support (canonical + alias accepted, named sequence rejected)
- `[~]` Full tokenizer/grammar long-tail parity

## Bytecode and VM
- `[x]` CPython 3.14 opcode-table foundation and decode/translate/execute support
- `[x]` `.pyc` read/write baseline
- `[~]` CPython exception-table execution baseline for translated `.pyc` is landed (`PUSH_EXC_INFO`/`POP_EXCEPT`/`WITH_EXCEPT_START`/`RERAISE`/`CHECK_EXC_MATCH` + table-driven handler dispatch); remaining `.pyc` long-tail opcode/state parity is still in progress
- `[~]` Full opcode-family parity

## Runtime and Object Model
- `[x]` Identity/refcount/cycle-GC foundations
- `[x]` Core object/class/function/frame foundations
- `[x]` Core truthiness and membership fallback baselines
- `[x]` `memoryview` typed scalar index/store baseline (`cast('b'/'H'/'f'/'c')` semantics + scalar multidim `NotImplementedError` parity), first-axis multidim slice/tolist shape preservation (`view[0:1]`, `view[::2]`), strided byte-export/iteration parity (`bytes(view[::2])`, `bytes(view[::-1])`, typed 1-D iter decode), and zero-length multidim contiguity flags
- `[~]` Descriptor/attribute/metaclass/slots long-tail parity
- `[~]` Numeric long-tail parity (big-int conversion/formatting/error edges)
- `[~]` Hash-container semantic/perf closure (`dict`/`set`/`frozenset`)

## Import System
- `[x]` Curated import-system foundations
- `[x]` Curated language/import harness suites with empty allowlist
- `[~]` Full importlib/resources/pkgutil/packaging long-tail parity

## Stdlib
- `[x]` Top-stdlib common-usecase baseline (`26/26` import + smoke; enforced in `tests/stdlib_common_usecases.rs`)
- `[x]` Builtin surface parity gate (`145/145`, no allowlist entries)
- `[x]` `hashlib` md5/sha2 baseline path (`_md5`, `_sha2`)
- `[x]` Native compression baseline modules (`zlib`, `_bz2`, `_lzma`) for common import + one-shot workflows
- `[~]` Native SSL baseline (`_ssl` + bootstrap `ssl`) is in place; full CPython `Lib/ssl.py` path remains blocked by namedtuple/super object-model parity
- `[~]` Extended stdlib matrix: `50/50` import, `50/50` smoke (`docs/STDLIB_EXTENDED_COMMON_USECASE_CHECKLIST.md`)
- `[~]` P0 closure still pending for `json`, `_csv`/`csv`, `pickle`/`pickletools`/`copyreg`, `_io`, and `_sre`
- `[~]` Deferred strict pickle harness lane closure

## Test/Gate Status
- `[x]` Differential tests and fuzz foundations active
- `[x]` Coverage/no-op inventory/builtin parity gates active
- `[~]` Full strict stdlib closure pending deferred pickle harness lane

## Extension Ecosystem (Milestone 15)
- `[~]` Extension subsystem scaffolding landed (`.pyrs-ext` manifest discovery + `pyrs.ExtensionFileLoader` + `hello_ext` smoke test)
- `[x]` Native shared-library loader baseline (`.so/.dylib/.pyd`, tagged filename variants) with compiled-extension smoke coverage
- `[~]` `libpyrs-capi` v1 header/symbol slice landed (`include/pyrs_capi.h`, `docs/EXTENSION_CAPI_V1.md`), including positional+keyword callable registration, init-scoped object handles/type getters, and import-time error state; broader runtime contract still pending
- `[~]` Extension source-build packaging substrate is in progress (`_sysconfigdata__*` baseline now provides `SOABI`/`EXT_SUFFIX`/`CC`/`LDSHARED` and include/lib hints)
- `[~]` NumPy bring-up import + source-build probes landed (`scripts/probe_numpy_gate.py`, `docs/NUMPY_BRINGUP_GATE.md`)
- `[~]` CPython-ABI bridge mode for NumPy (`PYRS_ENABLE_CPYTHON_ABI_BRIDGE=1`) now passes local probe gates (`import numpy`, `int(np.array([1,2,3]).sum())`)
- `[ ]` PEP 489 multi-phase init and module lifecycle closure
- `[ ]` NumPy/SciPy/Pandas/Matplotlib production import + functional gate closure

## Notes
- Active strict harness lane suite: `tests/cpython_suite_strict_stdlib.txt`
- Deferred strict pickle harness lane suite: `tests/cpython_suite_deferred_pickle.txt`
- Deferred strict pickle harness lane opt-in: `PYRS_RUN_DEFERRED_PICKLE=1 cargo test -q --test cpython_harness runs_cpython_deferred_pickle_suite`
