# Extension Library Lifetime Plan

Status: `PROPOSED` (pre-implementation checklist + safety review).

Owner: VM / extension loader workstream.

## Purpose

Define the implementation plan for fixing extension-library lifetime so detached native
threads, stored extension callbacks, and teardown callbacks cannot execute after the
underlying shared library has been unloaded.

This plan exists before code changes so the implementation can be reviewed against a
clear safety checklist instead of landing as a tactical patch.

## Current Problem

Today, `pyrs` loads extension shared libraries with `dlopen`, stores resolved function
symbols and callback pointers in VM-owned registries, and then `dlclose`s the library
when the owning `Vm` drops.

That is unsafe for at least one concrete case already present in the test suite:

- `PyThread_start_new_thread()` can detach a native thread whose entrypoint lives inside
  the loaded extension shared object.
- the subprocess/test harness may return from the Python call and begin VM teardown
  immediately afterward.
- VM teardown can then drop the extension handle and call `dlclose` while the detached
  thread has not yet run or is still running extension code.

That is a use-after-unload hazard. On Unix targets it can surface as a silent
segmentation fault during shutdown.

## Correctness Target

Successful extension loads must keep their code mapped for the full process lifetime,
not merely for the lifetime of one `Vm`.

This implies:

- no `dlclose` during normal `Vm` teardown for successfully loaded extension libraries.
- extension function pointers and teardown callbacks must always point into still-mapped
  code while they remain callable.
- detached threads created from extension entrypoints must not require VM teardown to
  wait or join.

## Required Invariants

1. A successfully loaded extension library outlives every function pointer resolved from it.
2. A successfully loaded extension library outlives every detached native thread whose
   entrypoint was resolved from that library.
3. `Vm::drop` may invoke extension-provided callbacks only while the corresponding code
   remains mapped.
4. Repeated imports / repeated VMs must not produce duplicate unload ordering hazards.
5. The lifetime fix must not rely on joining arbitrary extension-owned detached threads.
6. The lifetime fix must preserve existing extension module-state finalizer/free and
   capsule-destructor execution semantics.
7. Any cache that reuses compiled extension smoke binaries across CI runs must include
   enough ABI/header identity to avoid stale-binary reuse after compat-surface changes.

## Affected Runtime Surfaces

Primary code paths to update or re-check:

- `src/extensions/mod.rs`
- `src/vm/mod.rs`
- `src/vm/vm_extensions/loader_runtime.rs`
- `src/vm/vm_extensions/cpython_sys_thread_api.rs`
- `tests/extension_smoke.rs`

Function-pointer / callback storage that depends on library lifetime:

- `extension_libraries`
- `extension_callable_registry`
- `extension_module_state_registry`
- `extension_capsule_registry`

## Proposed Design

### 1. Promote Extension Library Handles To Process Lifetime

Replace VM-owned successful extension handles with a process-global keepalive registry.

Design requirements:

- key entries by canonical library identity (`PathBuf` after canonicalization where possible).
- keep the underlying shared-library handle alive until process exit.
- allow multiple VMs to reuse the same already-loaded handle without duplicate unload behavior.
- do not expose ownership patterns that can drop the underlying handle during `Vm::drop`.

Acceptable implementation strategies:

- dedicated process-global registry storing keepalive handles for the life of the process, or
- an equivalent process-lifetime retained-handle design.

Non-goal for this change:

- implementing full runtime unload / hot-reload semantics for native extensions.

### 2. Stop Treating `Vm::drop` As The Library-Unload Boundary

`Vm::drop` must continue to run VM-owned cleanup:

- module-state `finalize_func`
- module-state `free_func`
- capsule destructors
- pointer/free bookkeeping

But it must no longer be the place that unloads extension code.

### 3. Keep Detached Thread Semantics

Do not change `PyThread_start_new_thread()` into a join-on-drop model.

Rationale:

- detached extension threads may legitimately outlive the Python call that spawned them.
- joining during VM teardown can deadlock or hang indefinitely.
- the real bug is library lifetime, not the existence of detached threads.

### 4. Harden The Batch53 Smoke Probe After The Runtime Fix

The thread ABI smoke test should stop depending on scheduler luck.

Test change requirements:

- replace the no-op thread entrypoint with a flag-setting entrypoint.
- wait in the probe until the spawned thread has definitely executed.
- keep the test validating successful thread start without leaving correctness up to
  immediate shutdown timing.

This is a regression-hardening step, not the primary fix.

### 5. Harden Extension Smoke Cache Identity

The extension-smoke build cache should include:

- probe source bytes
- compiler/build-var identity
- `include/pyrs_cpython_compat.h` contents or digest
- an explicit ABI/version salt for compat-surface changes

This is separate from the shutdown race, but it prevents stale cached probe binaries
from obscuring extension-surface regressions in CI.

## Implementation Checklist

- [ ] Introduce a process-global extension-library keepalive registry.
- [ ] Define canonical library-identity logic for registry keys.
- [ ] Update successful dynamic-load paths to register handles in the process-global registry.
- [ ] Remove VM-lifetime ownership of successful extension library handles.
- [ ] Preserve current symbol-resolution error behavior for failed loads.
- [ ] Verify module-state callbacks still run during `Vm::drop`.
- [ ] Verify capsule destructors still run during `Vm::drop`.
- [ ] Verify stored extension callable entries cannot outlive mapped code.
- [ ] Keep `PyThread_start_new_thread()` detached; do not add teardown joins.
- [ ] Add a targeted regression test for detached thread entrypoints surviving VM teardown.
- [ ] Harden batch53 smoke with an explicit thread-executed handshake.
- [ ] Salt extension-smoke cache keys with compat-header + ABI identity.
- [ ] Re-run targeted extension smoke and full CI parity gates after implementation.

## Safety Audit Checklist

- [ ] Confirm no path can call `dlclose` for a successfully loaded extension during
      `Vm::drop`.
- [ ] Confirm resolved function pointers are never used after library unload.
- [ ] Confirm module-state finalizer/free function pointers remain valid for the full
      duration of teardown callbacks.
- [ ] Confirm capsule destructor pointers remain valid for the full duration of teardown.
- [ ] Confirm detached native thread entrypoints remain mapped after the originating VM drops.
- [ ] Confirm the new process-global registry does not create double-close or double-drop
      behavior for the same raw handle.
- [ ] Confirm failure paths do not retain partially initialized libraries in a way that
      leaks callback state or invalid handles.
- [ ] Confirm no new mutable-global unsafety is introduced by the keepalive registry.
- [ ] Confirm any path canonicalizing library names does not accidentally alias distinct
      libraries onto one handle entry.
- [ ] Confirm test-only synchronization does not become the only guard for runtime safety.

## Unsafe Code Review Focus

Unsafe blocks that must be re-reviewed as part of implementation:

- `dlopen` / `dlsym` / `dlclose` FFI in `src/extensions/mod.rs`
- transmute-based symbol conversion in `SharedLibraryHandle::symbol`
- teardown callback invocation in `Vm::drop`
- detached thread entrypoint invocation in `PyThread_start_new_thread`
- any new process-global registry handling raw library handles

Questions to answer during review:

1. Does the implementation guarantee that code pointers cannot outlive the mapping they
   were resolved from?
2. Does any new registry require explicit synchronization or `Send`/`Sync` boundaries?
3. Can a failed import path leave behind a handle that is no longer referenced but still
   relied on by stored callbacks?
4. Are there any existing callback registries that still indirectly assume VM-lifetime
   library ownership after the change?

## Regression Checklist

- [ ] `cargo nextest run --test extension_smoke cpython_compat_thread_abi_batch53_apis_work`
- [ ] repeated local loop of the batch53 smoke test
- [ ] targeted extension-smoke batches covering module state and capsules
- [ ] `cargo nextest run --status-level fail --final-status-level fail`
- [ ] CI parity-gate rerun on Linux after implementation

## Explicit Non-Goals

- dynamic unloading of extension code before process exit
- hot reloading / replacement of already loaded shared libraries
- changing the public default of detached thread semantics
- solving all extension-lifetime issues unrelated to shared-library code mapping

## Exit Criteria

This plan is complete only when:

- successful extension libraries are not unloaded during `Vm::drop`
- extension teardown callbacks remain valid throughout teardown
- detached extension thread entrypoints cannot execute from unmapped code
- batch53 has deterministic regression coverage
- extension smoke cache identity is strong enough to avoid stale compat-probe reuse
