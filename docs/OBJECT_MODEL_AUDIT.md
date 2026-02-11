# Object Model Audit (CPython 3.14)

This document tracks data-model parity work and references used for implementation decisions.

## Reference Sources

Primary references used for this audit:
- Python language reference: `https://docs.python.org/3/reference/datamodel.html`
- CPython C runtime truthiness and protocol dispatch:
  - `Objects/object.c` (`PyObject_IsTrue`)
  - `Objects/typeobject.c` (`slot_nb_bool`, `slot_sq_contains`, slot wiring)
  - `Objects/abstract.c` (`PySequence_Contains`, `PyObject_GetIter`)
  - `Objects/boolobject.c` (bool constructor path)
  - `Python/bltinmodule.c` (`all`, `any`, and iterator-based builtins)
- CPython tests used as behavioral probes:
  - `Lib/test/test_bool.py`
  - `Lib/test/test_contains.py`
  - `Lib/test/test_descr.py`

## Completed In This Pass

- Implemented VM-level truth-value protocol dispatch for custom objects:
  - `__bool__` is consulted first.
  - If absent, `__len__` is consulted.
  - Default truthiness falls back to `True` for objects without either hook.
- Wired control-flow/runtime sites to protocol-aware truthiness:
  - bytecode `UnaryNot`, `ToBool`, `JumpIfFalse`, `JumpIfTrue`
  - builtin `bool()`
  - builtin `all()` / `any()` / `filter()`
  - rich-compare result coercion path used by comparison fallbacks
- Aligned key error behavior with CPython-facing exception typing:
  - `__bool__` non-bool return -> `TypeError`
  - non-integer `__len__` in truth context -> `TypeError`
  - negative `__len__` in truth context -> `ValueError`
- Implemented membership protocol baseline for `in` / `not in`:
  - direct fast path for native containers remains
  - fallback to `__contains__` when available
  - fallback to iterator protocol (`__iter__`)
  - fallback to sequence protocol (`__getitem__`-driven iteration)
- Fixed inherited class-attribute builtin binding for user classes:
  - inherited `str`/`len`-style builtin attributes now stay unbound on instance access (`instance.attr is str/len`) instead of being turned into bound methods.

## Remaining Object-Model Parity Work (Milestone 13)

- Long-tail membership edge parity from CPython tests (`test_contains`, `test_descr`), including explicit blocking cases (for example `__contains__ = None`) and exact error text parity.
- Broader slot edge behavior from CPython tests in `test_bool` and `test_descr`.
- Continue reducing static truthiness shortcuts in call sites where Python-level coercion is required.

## Validation Expectations

- New behavior must be covered by VM tests before rollout.
- For protocol changes, include at least one CPython-differential probe.
- Any intentional temporary divergence must be tracked in `docs/STUB_ACCOUNTING.md`.
