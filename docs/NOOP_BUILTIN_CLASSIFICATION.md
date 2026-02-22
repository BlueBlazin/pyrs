# NoOp Builtin Classification

This classifies the current entries in `docs/NOOP_BUILTIN_INVENTORY.txt` into:
- `production-facing`: runtime-visible surfaces we should implement for CPython-compat behavior.
- `test-only`: CPython test helper module surfaces (`_testcapi`/`_testinternalcapi`).

Snapshot date: 2026-02-22

## Production-Facing Symbols (15)

- `__main__.__builtins__.object.__init_subclass__`
- `sys.__breakpointhook__`
- `sys.__unraisablehook__`
- `sys._clear_type_descriptors`
- `sys.audit`
- `sys.breakpointhook`
- `sys.monitoring.clear_tool_id`
- `sys.monitoring.free_tool_id`
- `sys.monitoring.get_tool`
- `sys.monitoring.register_callback`
- `sys.monitoring.restart_events`
- `sys.monitoring.set_events`
- `sys.monitoring.set_local_events`
- `sys.monitoring.use_tool_id`
- `sys.unraisablehook`

Notes:
- `sys.monitoring.*` is advanced tooling/debugging API, but it is still runtime-facing.
- `sys._clear_type_descriptors` is private-prefixed, but currently exposed in the runtime surface and not `_test*`-scoped.

## Test-Only Symbols (26)

- `_testcapi.MethClass.meth_fastcall`
- `_testcapi.MethClass.meth_fastcall_keywords`
- `_testcapi.MethClass.meth_noargs`
- `_testcapi.MethClass.meth_o`
- `_testcapi.MethClass.meth_varargs`
- `_testcapi.MethClass.meth_varargs_keywords`
- `_testcapi.MethInstance.meth_fastcall`
- `_testcapi.MethInstance.meth_fastcall_keywords`
- `_testcapi.MethInstance.meth_noargs`
- `_testcapi.MethInstance.meth_o`
- `_testcapi.MethInstance.meth_varargs`
- `_testcapi.MethInstance.meth_varargs_keywords`
- `_testcapi.MethStatic.meth_fastcall`
- `_testcapi.MethStatic.meth_fastcall_keywords`
- `_testcapi.MethStatic.meth_noargs`
- `_testcapi.MethStatic.meth_o`
- `_testcapi.MethStatic.meth_varargs`
- `_testcapi.MethStatic.meth_varargs_keywords`
- `_testcapi.meth_fastcall`
- `_testcapi.meth_fastcall_keywords`
- `_testcapi.meth_noargs`
- `_testcapi.meth_o`
- `_testcapi.meth_varargs`
- `_testcapi.meth_varargs_keywords`
- `_testinternalcapi.has_inline_values`
- `_testinternalcapi.set_eval_frame_default`

## Policy

- Production-facing no-op symbols should be prioritized for real implementation.
- Test-only no-op symbols may remain minimal where acceptable, but must stay explicitly documented and constrained to `_test*` modules.
