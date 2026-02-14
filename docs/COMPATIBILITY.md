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
- `[~]` Full opcode-family parity

## Runtime and Object Model
- `[x]` Identity/refcount/cycle-GC foundations
- `[x]` Core object/class/function/frame foundations
- `[x]` Core truthiness and membership fallback baselines
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

## Notes
- Active strict harness lane suite: `tests/cpython_suite_strict_stdlib.txt`
- Deferred strict pickle harness lane suite: `tests/cpython_suite_deferred_pickle.txt`
- Deferred strict pickle harness lane opt-in: `PYRS_RUN_DEFERRED_PICKLE=1 cargo test -q --test cpython_harness runs_cpython_deferred_pickle_suite`
