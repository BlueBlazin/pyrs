# C-API Lifetime Model (P0)

Status: `IN_PROGRESS` (execution lock, Phase 1/2 in progress).

## Latest Checkpoint (2026-02-21)

- teardown-safety closure (latest):
  - fixed deterministic `SIGABRT` in VM teardown (pointer-not-allocated in `Vm::drop`) after NumPy workloads.
  - root-cause fixed:
    - list-buffer `realloc` paths now migrate owned-pointer/pin/registry state when buffer addresses change,
    - context-drop free paths now remove compat/list-buffer/aux pointers from `cpython_owned_ptrs` before free, preventing stale-ownership reuse.
    - owned-pointer pin checks are now provenance-specific so externally pinned proxies are not misclassified as owned-compat objects in iterator-call paths.
  - stress evidence:
    - repeated subprocess probes of `np.array(...).reshape(...).sum(axis=0)` now exit cleanly in debug and release builds,
    - `tests/vm.rs::numpy_repeated_axis_sum_remains_stable_across_calls` now passes repeatedly.
    - `tests/vm.rs::numpy_axis_sum_and_repr_stress_stays_stable` now passes.
    - NumPy ndarray repr parity regression (`numpy_float_ndarray_repr_does_not_fall_back_to_instance_placeholder`) is now closed.

- VM-global registry substrate is now wired (`src/vm/capi_registry.rs`) with:
  - pointer provenance (`OwnedCompat`, `ExternalRef`, `StaticSingleton`),
  - lifecycle state (`Alive`, `PendingFree`, `Freed`),
  - reference-kind accounting (`Borrowed`, `Owned`, `Stolen`),
  - pointer/object-id index + stats surface.
- Registry integration now covers core ownership flows:
  - compat allocation registration in `ModuleCapiContext` allocation paths,
  - external proxy registration + explicit pin accounting on first materialization,
  - context-drop + VM-drop deallocation paths now mark pending/free through registry APIs.
- High-traffic pointer-conversion paths now record ownership kind explicitly:
  - proxy/callable call-result conversion paths use owned-reference wrappers,
  - call/attr/vectorcall argument conversion paths use borrowed-reference wrappers.
- Regression hardening:
  - added NumPy lifetime stress probes in `tests/vm.rs`:
    - `numpy_axis_sum_and_repr_stress_stays_stable`
    - `numpy_repeated_array_ops_and_reprs_stay_stable`
- CI hardening:
  - added `sanitizer-stability` job in `.github/workflows/parity-gate.yml`
    (nightly ASan lane with NumPy lifetime + extension vectorcall smoke probes).
- Legacy-lifetime cleanup progress:
  - removed VM-side legacy external-pin/freed allocation sets from `Vm`,
    and moved those state transitions to the VM-global CAPI registry.

## Problem Statement

The current CPython-compat runtime can still produce pointer-lifetime corruption (use-after-free class bugs) under scientific-stack workloads.

Recent concrete example:
- repeated `numpy.ndarray.sum(axis=0)` calls could crash or fail on second/subsequent calls because a callable wrapper pointer escaped one `ModuleCapiContext` and was reclaimed at context teardown.

This is not a NumPy-specific semantics bug. It is a substrate lifetime bug.

## Why This Happened

Current architecture mixes two incompatible ownership models:

1. Context-scoped object ownership
- `ModuleCapiContext::drop` reclaims many CPython-compat allocations.

2. Refcount-scoped object usage
- Native extensions assume CPython rules (`Py_INCREF` / `Py_DECREF`) where object lifetime is tied to reference ownership, not call-context scope.

Because wrappers can escape a context and still be used later, context teardown can reclaim memory that remains logically alive to extension code.

## CPython Reference Baseline

Source references used for this model:
- CPython 3.14 C-API docs:
  - [Intro / reference count details](https://docs.python.org/3.14/c-api/intro.html#reference-count-details)
  - [Refcounting](https://docs.python.org/3.14/c-api/refcounting.html)
  - [Defining extension modules](https://docs.python.org/3.14/c-api/extension-modules.html)
  - [Module definitions](https://docs.python.org/3.14/c-api/module.html#module-definitions)
- CPython source in local tree:
  - `/Users/$USER/Downloads/Python-3.14.3/Include/object.h`
  - `/Users/$USER/Downloads/Python-3.14.3/Include/refcount.h`
  - `/Users/$USER/Downloads/Python-3.14.3/Objects/object.c`

Key contract we must follow:
- API boundaries explicitly define borrowed/new/stolen references.
- Deallocation occurs on refcount transition to zero (plus type dealloc policy), not arbitrary context teardown.

## Required Invariants

1. Every pointer known to pyrs has one canonical ownership record.
2. Pointers cannot be freed by per-call context teardown if they are externally reachable.
3. Borrowed/new/stolen semantics are encoded explicitly at API boundaries.
4. Pointer -> value/object identity mapping is centralized and generation-safe.
5. Freeing policy is centralized and idempotent.
6. Pointer-probability heuristics must not be relied on for correctness.

## Target Architecture

## 1) VM-global CAPI Object Registry

Introduce a VM-owned registry for all compat/external CPython pointers.

Each entry stores:
- `ptr: usize`
- `generation: u64` (ABA/stale-handle guard)
- `provenance`:
  - `OwnedCompat` (pyrs-allocated compat object)
  - `ExternalRef` (foreign object tracked via ref ownership)
  - `StaticSingleton`
- `lifetime_state`:
  - `Alive`
  - `PendingFree`
  - `Freed`
- `ref_state`:
  - compat-refcount mirror
  - external pinned refs count
- canonical `Value`/handle linkage (where applicable)

`ModuleCapiContext` becomes an execution scratch/context carrier, not owner of long-lived CPython object memory.

## 2) Explicit Reference-Kind Wrappers

Introduce typed API wrappers for call boundaries:
- `BorrowedRef<T>`
- `OwnedRef<T>`
- `StolenRef<T>`

No raw pointer transfer across major runtime boundaries without one of these wrappers.

## 3) Centralized Allocation / Free Paths

- Allocation: registry insertion only.
- Free: one central path that validates entry state + generation + ownership contract.
- Context drop: only release scratch buffers and strictly context-owned temporaries.

## 4) Remove Correctness Dependence on Pointer Heuristics

Functions like `is_probable_external_cpython_object_ptr` can remain as diagnostics only.
They must not gate correctness decisions on object identity/lifetime.

## Migration Plan

### Phase 0 (now): Execution lock + detection
- Keep current stabilization fix to avoid immediate crashes.
- Add strict UAF stress probes (repeat call/repr/import cycles).
- Add registry-growth and live-entry telemetry.

### Phase 1: Registry scaffolding
- Add VM-global registry types and APIs.
- Route new compat allocations through registry create path.
- Start dual-write mapping from old maps to new registry (temporary compatibility lane).

### Phase 2: Ownership migration
- Migrate call/attr/vectorcall/object-call surfaces to registry ownership APIs.
- Remove direct frees from `ModuleCapiContext::drop` for shared wrappers.

### Phase 3: Ref semantics tightening
- Encode borrowed/new/stolen flow for highest-traffic C-API entrypoints.
- Add assertions for illegal ownership transitions.

### Phase 4: Heuristic retirement
- Downgrade pointer-probability checks to trace/debug only.
- Replace correctness branches with registry/provenance checks.

### Phase 5: Cleanup + closure
- Remove temporary pin-forever behavior used as crash stopgap.
- Verify bounded memory growth in long-running NumPy loops.

## Validation Strategy

Required green evidence before closing this item:

1. Safety/stability
- no segfaults in repeat-loop scientific-stack probes
- no `unknown callable object pointer`/stale-pointer errors in regression paths

2. Correctness
- repeated NumPy reduction/repr/import cycles preserve CPython behavior
- no semantic regressions in `tests/vm.rs` and extension smoke lanes

3. Memory behavior
- bounded live-entry growth under repeated stable workloads
- no unbounded pin-set growth after warm-up

4. CI/tooling
- add sanitizer lane (ASan/UBSan where available)
- add stress lane for repeated extension-call cycles

## Non-Goals

- This plan does not change Python-level semantics.
- This plan does not defer correctness in favor of benchmark gains.
- This plan does not expand to unrelated stdlib long-tail work while lock is active.

## Immediate Batch (next coding slice)

1. Introduce VM-global registry structs + entry states.
2. Wire compat allocation paths to registry creation (dual-write mode).
3. Move context-drop free policy behind registry ownership checks.
4. Add repeat-loop UAF stress tests for NumPy callable/repr/reduction paths.
5. Add telemetry assertions for live-entry and pinned-entry growth bounds.
