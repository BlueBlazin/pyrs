# C-API NoOp Execution Order

Purpose: ordered implementation checklist for C-API no-op/placeholder closures in
`docs/CAPI_NOOP_INVENTORY.md`.

Last updated: 2026-02-22

## Re-check Snapshot

- Empty-body rows: `17`
- Placeholder rows: `12`
- Total rows: `29`
- Total symbols represented: `31`

Note: placeholder rows are now `12` (not `13`) because `PySys_Audit` /
`PySys_AuditTuple` were closed and removed from the no-op inventory.

## Ordered Checklist

### Batch 1: Thread/GIL Substrate (dependency base)

- [ ] `PyGILState_Ensure`
- [ ] `PyGILState_Release`
- [ ] `PyEval_SaveThread`
- [ ] `PyEval_RestoreThread`
- [ ] `PyEval_AcquireThread`
- [ ] `PyEval_ReleaseThread`
- [ ] `PyEval_AcquireLock`
- [ ] `PyEval_ReleaseLock`
- [ ] `PyMutex_Lock`
- [ ] `PyMutex_Unlock`
- [ ] `PyEval_InitThreads`
- [ ] `PyEval_ThreadsInitialized`
- [ ] `PyThread_init_thread`
- [ ] `PyThread_ReInitTLS`

### Batch 2: Signal + Recursion Control

- [ ] `PyErr_CheckSignals`
- [ ] `Py_EnterRecursiveCall`
- [ ] `Py_LeaveRecursiveCall`
- [ ] `_Py_CheckRecursiveCall`

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
