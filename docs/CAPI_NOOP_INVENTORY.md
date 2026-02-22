# C-API NoOp Inventory

Purpose: track intentional C-API no-op/placeholder behavior that still exists in `pyrs` and define explicit closure criteria.

Last updated: 2026-02-22

## Scope

This document covers C-API exports in `src/vm/vm_extensions/*` whose current behavior is intentionally no-op or placeholder semantics.

It does **not** cover Python-level `BuiltinFunction::NoOp` placeholders; those are tracked by:
- `docs/NOOP_BUILTIN_INVENTORY.txt`
- `tests/noop_inventory.rs`

## A. Empty-body C-API Exports (true no-op)

| Symbol | File | Current behavior | Closure criteria |
|---|---|---|---|
| `PyObject_ClearWeakRefs` | `src/vm/vm_extensions/cpython_weakref_api.rs` | no-op | Implement weakref-list clear parity for deallocation paths (including callback suppression/order) and add extension regression coverage. |
| `PyObject_GC_Track` | `src/vm/vm_extensions/cpython_gc_alloc_api.rs` | no-op | Wire into explicit GC-tracked state model for C-API objects and ensure `IsTracked/IsFinalized` parity. |
| `PyObject_GC_UnTrack` | `src/vm/vm_extensions/cpython_gc_alloc_api.rs` | no-op | Same closure as `PyObject_GC_Track`; must preserve safe transitions and no-UAF invariants. |
| `Py_EndInterpreter` | `src/vm/vm_extensions/cpython_runtime_misc_api.rs` | no-op | Close with real interpreter teardown semantics or explicit subinterpreter policy + hard error behavior. |
| `PyType_Modified` | `src/vm/vm_extensions/cpython_type_api.rs` | no-op | Implement type-cache invalidation/update propagation expected by CPython extension behavior. |

## B. Placeholder C-API Exports (non-empty, still no-op semantics)

| Symbol | File | Current behavior | Closure criteria |
|---|---|---|---|
| `PyObject_GC_IsFinalized` | `src/vm/vm_extensions/cpython_gc_alloc_api.rs` | always returns `0` | Return finalized-state parity for GC objects and prove via GC lifecycle tests. |
| `PyTraceMalloc_Track` / `PyTraceMalloc_Untrack` | `src/vm/vm_extensions/cpython_thread_interp_api.rs` | always returns `0` | Implement tracemalloc domain/pointer tracking semantics or explicit unsupported policy with deterministic behavior. |
| `PyType_ClearCache` | `src/vm/vm_extensions/cpython_type_api.rs` | always returns `0` | Implement cache clear/invalidation accounting consistent with type mutation semantics. |
| `Py_NewInterpreter` | `src/vm/vm_extensions/cpython_runtime_misc_api.rs` | returns current thread state (no new interpreter) | Implement real subinterpreter creation or enforce explicit unsupported contract. |
| `PyUnstable_Object_IsUniquelyReferenced` / `PyUnstable_Object_IsUniqueReferencedTemporary` | `src/vm/vm_extensions/cpython_error_numeric_api.rs` | always return `0` | Implement CPython-compatible unique-reference query semantics (or explicit unsupported policy) without violating ownership invariants. |
| `PyUnstable_Object_EnableDeferredRefcount` | `src/vm/vm_extensions/cpython_object_call_api.rs` | always returns `0` | Implement deferred-refcount enablement semantics or explicit unsupported policy consistent with CPython unstable API expectations. |

## Closure and Ownership

- Canonical progress tracker: `docs/STUB_ACCOUNTING.md` (Milestone 15 extension ecosystem row).
- Safety constraints: `docs/CAPI_LIFETIME_MODEL.md`.
- Execution plan: `docs/CAPI_PLAN.md`.
- Ordered implementation checklist: `docs/CAPI_NOOP_EXECUTION_ORDER.md`.

A C-API no-op is only considered closed when:
1. behavior matches CPython 3.14 semantics (or explicit supported-scope policy is documented and enforced),
2. targeted regression tests exist and are green,
3. strict/scientific extension gates show no regressions from the change.
