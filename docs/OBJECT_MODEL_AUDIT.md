# Object Model Audit (CPython 3.14)

This document tracks CPython data-model parity for runtime protocol dispatch and special-method behavior.

Status values:
- `OPEN`
- `IN_PROGRESS`
- `CLOSED`

## Reference Map
- Python language reference:
  - `https://docs.python.org/3/reference/datamodel.html`
- CPython runtime/protocol sources:
  - `Objects/object.c` (`PyObject_IsTrue`)
  - `Objects/typeobject.c` (`slot_nb_bool`, `slot_sq_contains`, slot wiring)
  - `Objects/abstract.c` (`PySequence_Contains`, `PyObject_GetIter`)
  - `Objects/boolobject.c`
  - `Python/bltinmodule.c`
- CPython behavioral probes:
  - `Lib/test/test_bool.py`
  - `Lib/test/test_contains.py`
  - `Lib/test/test_descr.py`

## Audit Items

| ID | Area | Gap summary | Closure criteria | Required evidence | Status |
|---|---|---|---|---|---|
| OM-001 | Truthiness protocol (`__bool__`/`__len__`) | Baseline protocol behavior is implemented. | Keep CPython truthiness order and error classification parity stable. | VM regressions + differential truthiness probes | CLOSED |
| OM-002 | Membership protocol fallback (`in`/`not in`) | Baseline fallback order is implemented. | Preserve fallback order parity (`__contains__` -> iterator -> sequence). | VM regressions + differential membership probes | CLOSED |
| OM-003 | Descriptor/metaclass/slots long-tail | Long-tail edge semantics remain open. | Close remaining descriptor/attribute/metaclass/slots edge behavior in scope. | `tests/vm.rs` regressions + curated/strict harness evidence | IN_PROGRESS |
| OM-004 | Membership blocking/error edges | Blocking/error cases (for example `__contains__ = None`) need full parity closure. | Match CPython exception type/ordering/message expectations for remaining membership edges. | targeted regressions + differential probes from `test_contains`/`test_descr` | IN_PROGRESS |
| OM-005 | Special-method dispatch (`__repr__`/`__str__`/`__format__` and related hooks) | Baseline print path now routes through `str()`, and `object.__format__` now follows CPython empty-spec semantics used by unittest subtest rendering; long-tail dispatch edges remain. | Close special-method dispatch parity required by stdlib/tests in scope. | targeted regressions + strict harness coverage | IN_PROGRESS |
| OM-006 | Builtin attribute binding on instances | Inherited builtin attribute binding bug was fixed. | Keep inherited builtin attribute access parity stable (`instance.attr is builtin` where applicable). | regression tests for inherited builtin attribute access | CLOSED |
| OM-007 | Buffer protocol scalar semantics (`memoryview`) | Typed scalar index/store parity is now landed for cast formats (`b`/`B`/`H`/`h`/`I`/`i`/`L`/`l`/`Q`/`q`/`f`/`d`/`c`) including CPython-style invalid-type/value errors; multi-dimensional scalar subviews now raise `NotImplementedError` parity messages; first-axis multidim slice/tolist now preserve shape/stride semantics. | Preserve landed scalar/multidim-first-axis behavior while closing remaining multi-dimensional indexing/slice-assignment long-tail behavior. | `tests/vm.rs` memoryview cast/index/store/slice regressions + CPython differential probes | IN_PROGRESS |

## Validation Rules
1. Protocol behavior changes must ship with targeted `tests/vm.rs` regressions.
2. Each non-trivial closure should include at least one CPython differential probe.
3. Any temporary divergence must be tracked in `docs/STUB_ACCOUNTING.md` with closure criteria.
