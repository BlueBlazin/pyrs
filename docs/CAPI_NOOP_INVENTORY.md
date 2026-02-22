# C-API NoOp Inventory

Purpose: track intentional C-API no-op/placeholder behavior that still exists in `pyrs` and define explicit closure criteria.

Last updated: 2026-02-22

## Scope

This document covers C-API exports in `src/vm/vm_extensions/*` whose current behavior is intentionally no-op or placeholder semantics.

It does **not** cover Python-level `BuiltinFunction::NoOp` placeholders; those are tracked by:
- `docs/NOOP_BUILTIN_INVENTORY.txt`
- `tests/noop_inventory.rs`

## A. Empty-body C-API Exports (true no-op)

None.

## B. Placeholder C-API Exports (non-empty, still no-op semantics)

| Symbol | File | Current behavior | Closure criteria |
|---|---|---|---|
| `PyTraceMalloc_Track` / `PyTraceMalloc_Untrack` | `src/vm/vm_extensions/cpython_thread_interp_api.rs` | always returns `0` | Implement tracemalloc domain/pointer tracking semantics or explicit unsupported policy with deterministic behavior. |
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
