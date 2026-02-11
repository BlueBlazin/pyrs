# Compatibility Tracker (CPython 3.14)

This file tracks compatibility state.
For release blockers, use `docs/PRODUCTION_READINESS.md`.
For partial implementation ownership, use `docs/STUB_ACCOUNTING.md`.

Status:
- `[ ]` not started
- `[~]` in progress
- `[x]` complete

## Parser and Language Surface
- `[x]` Core parser/compiler foundations through Milestone 12
- `[x]` Major modern syntax landed (`match`/`case`, comprehensions, assignment expressions, async syntax, decorators, annotations baseline)
- `[x]` `\N{...}` Unicode-name escapes: canonical names + aliases accepted, named-sequence escapes rejected (CPython parity)
- `[~]` Full tokenizer/grammar long-tail parity still pending
- `[~]` Remaining pattern/exception-group/f-string edge parity still pending

## Bytecode and VM
- `[x]` CPython 3.14 opcode table and supported decode/translate/execute paths
- `[x]` Supported `.pyc` read/write foundation
- `[~]` Full opcode family parity still pending

## Runtime and Object Model
- `[x]` Identity/refcount/cycle-GC foundations
- `[x]` Core runtime object model and class/function/frame foundations
- `[x]` Core truth-value protocol semantics (`__bool__` then `__len__`) in VM control flow and key builtins
- `[x]` Core membership protocol fallback order (`__contains__` -> iterator -> `__getitem__`) in `in`/`not in`
- `[~]` Long-tail data-model parity (descriptor/attribute/metaclass/slots edges) pending
- `[~]` Numeric long-tail parity (big-int conversion/formatting/error-edge behavior) pending
- `[~]` Hash-container semantic/perf closure (`dict`/`set`/`frozenset`) pending

## Import and Module System
- `[x]` Curated import-system foundations (`sys.path`, hooks, namespace packages, module metadata)
- `[x]` Curated language/import CPython harness suites with empty allowlist
- `[~]` Full importlib/resources/pkgutil/packaging long-tail behavior pending

## Stdlib Compatibility
- `[x]` Foundational stdlib bootstrap in place (math/time/os/pathlib/io/json/re/etc. at varying depth)
- `[~]` P0 closure still pending for `json`, `_csv`/`csv`, `pickle`/`pickletools`/`copyreg`
- `[~]` `_io` parity advanced (`io.FileIO` + `_io.FileIO.__init__`, `_io.StringIO`/`_io.BytesIO` close/context/open-state/readable/writable/seekable, `read1`/`readlines`/`writelines`/`truncate`/`flush`/`isatty`, `getbuffer`/`detach`, `__getstate__`/`__setstate__`, and buffer-export resize guards); remaining pure-`_pyio`/codec long-tail still pending
- `[~]` Native-core-first parity work in progress (`_io`, `_csv`, `_sre`, `_pickle`)
- `[~]` Strict stdlib lane active for non-pickle scope; deferred strict pickle lane still open

## Test and Gate Status
- `[x]` Differential tests and fuzz foundations active
- `[x]` Coverage gate and no-op inventory gates active
- `[~]` Full strict stdlib closure remains pending due deferred pickle lane

## Notes
- Active strict suite: `tests/cpython_suite_strict_stdlib.txt`
- Deferred strict pickle suite: `tests/cpython_suite_deferred_pickle.txt`
- Deferred strict pickle opt-in run: `PYRS_RUN_DEFERRED_PICKLE=1 cargo test -q --test cpython_harness runs_cpython_deferred_pickle_suite`
- Canonical milestone plan: `docs/ROADMAP.md`
