# C-API NoOp Execution Order

Purpose: ordered implementation checklist for C-API no-op/placeholder closures in
`docs/CAPI_NOOP_INVENTORY.md`.

Last updated: 2026-02-22

## Re-check Snapshot

- Empty-body rows: `5`
- Placeholder rows: `6`
- Total rows: `11`
- Total symbols represented: `13`

Note: Batch 1 and Batch 2 symbols are now closed and removed from the no-op
inventory.

## Ordered Checklist

### Batch 1: Thread/GIL Substrate (dependency base) ✅ complete

- [x] `PyGILState_Ensure`
- [x] `PyGILState_Release`
- [x] `PyEval_SaveThread`
- [x] `PyEval_RestoreThread`
- [x] `PyEval_AcquireThread`
- [x] `PyEval_ReleaseThread`
- [x] `PyEval_AcquireLock`
- [x] `PyEval_ReleaseLock`
- [x] `PyMutex_Lock`
- [x] `PyMutex_Unlock`
- [x] `PyEval_InitThreads`
- [x] `PyEval_ThreadsInitialized`
- [x] `PyThread_init_thread`
- [x] `PyThread_ReInitTLS`

### Batch 2: Signal + Recursion Control ✅ complete

- [x] `PyErr_CheckSignals`
- [x] `Py_EnterRecursiveCall`
- [x] `Py_LeaveRecursiveCall`
- [x] `_Py_CheckRecursiveCall`

### Batch 3: GC + Weakref Lifecycle

- [ ] `PyObject_GC_Track`
- [ ] `PyObject_GC_UnTrack`
- [ ] `PyObject_GC_IsFinalized`
- [ ] `PyObject_ClearWeakRefs`

### Batch 4: Type Cache Coherence

- [ ] `PyType_Modified`
- [ ] `PyType_ClearCache`

### Batch 5: Interpreter Lifecycle

- [ ] `Py_NewInterpreter`
- [ ] `Py_EndInterpreter`

### Batch 6: Observability + Unstable APIs

- [ ] `PyTraceMalloc_Track`
- [ ] `PyTraceMalloc_Untrack`
- [ ] `PyUnstable_Object_IsUniquelyReferenced`
- [ ] `PyUnstable_Object_IsUniqueReferencedTemporary`
- [ ] `PyUnstable_Object_EnableDeferredRefcount`

## Batch Gate

For each batch, do not mark complete until all are true:

- behavior matches CPython 3.14 semantics,
- targeted tests are added/updated and green,
- no-op inventory docs are updated in the same checkpoint commit.
