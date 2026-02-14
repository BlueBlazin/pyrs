# Extension Capability Matrix (Milestone 15)

Status: active (source-of-truth for extension surface claims).

Scope: native-extension runtime path needed for NumPy/SciPy/Pandas/Matplotlib.

Legend:
- `DONE`: implemented and test-covered.
- `IN PROGRESS`: partial implementation in-tree.
- `PLANNED`: not implemented yet.
- `BLOCKED`: known blocker outside current substrate.

## Runtime and Loader Substrate

| Surface | Status | Owner | Evidence | Notes |
|---|---|---|---|---|
| `.pyrs-ext` extension manifest discovery on import path | DONE | VM/import | `tests/extension_smoke.rs::imports_manifest_backed_hello_extension` | Supports static (`hello_ext`) and dynamic (`dynamic:<symbol>` + `library=...`) entrypoints. |
| Extension module loader dispatch (`pyrs.ExtensionFileLoader`) | DONE | VM/import | `tests/extension_smoke.rs::imports_manifest_backed_hello_extension` | Module metadata includes extension ABI + entrypoint markers. |
| Minimal extension entrypoint registry (`hello_ext`) | DONE | VM/extensions | `tests/extension_smoke.rs::imports_manifest_backed_hello_extension` | Smoke substrate only; not a user-facing compatibility claim. |
| Dynamic shared-library loader (`.so/.dylib/.pyd`) | DONE | VM/extensions | `tests/extension_smoke.rs::imports_compiled_dynamic_extension_from_manifest` | Runtime loads shared objects via native loader (`dlopen`/`dlsym` on unix) and executes extension init symbols. |
| Direct shared-object import without manifest | DONE | VM/import + VM/extensions | `tests/extension_smoke.rs::imports_direct_shared_object_extension_without_manifest` | Importer detects `module.so` / `module.dylib` / `module.pyd` on `sys.path` and uses default symbol `pyrs_extension_init_v1`; CPython-style `PyInit_*` symbol-only modules now fail with explicit unsupported diagnostics, and loaded modules expose symbol-family metadata for ABI-mode debugging. |
| `_sysconfigdata__*` extension-build vars baseline | IN PROGRESS | VM/bootstrap | `tests/extension_smoke.rs::sysconfigdata_builtin_exposes_extension_build_keys`, `tests/extension_smoke.rs::sysconfig_build_vars_can_compile_and_import_extension` | Baseline keys (`SOABI`, `EXT_SUFFIX`, `CC`, `LDSHARED`, include/lib hints) are populated and now validated by compile+import smoke; full CPython/distutils parity remains open. |
| PEP 489 multi-phase init | PLANNED | VM/extensions | - | Required for production extension parity. |
| Extension module state lifecycle hooks | PLANNED | VM/extensions | - | Needs finalize/teardown semantics. |

## C Runtime Surface (`libpyrs-capi`)

| Surface | Status | Owner | Evidence | Notes |
|---|---|---|---|---|
| Exported C ABI artifact (`libpyrs-capi`) | IN PROGRESS | runtime/ffi | `include/pyrs_capi.h`, `docs/EXTENSION_CAPI_V1.md` | Header/symbol slice is landed and consumed by compiled-extension smoke; distributable ABI artifact packaging is pending. |
| Header surface + versioned symbol manifest | IN PROGRESS | runtime/ffi | `include/pyrs_capi.h`, `docs/EXTENSION_CAPI_V1.md` | v1 includes module-global setters/getters/import/attr-load, callable registration, init-scoped object handles/refcount ops, object type/introspection getters, and error-state hooks. |
| `PyObject`/refcount ownership APIs | IN PROGRESS | runtime/ffi | `tests/extension_smoke.rs::dynamic_extension_can_set_module_values_via_object_handles`, `tests/extension_smoke.rs::dynamic_extension_can_iterate_with_iterator_apis`, `tests/extension_smoke.rs::dynamic_extension_can_call_python_callable_handles`, `tests/extension_smoke.rs::dynamic_extension_can_use_object_call_fastpaths`, `tests/extension_smoke.rs::dynamic_extension_can_get_set_and_del_object_attributes`, `tests/extension_smoke.rs::dynamic_extension_can_check_isinstance_and_issubclass` | v1 handle model supports init-scoped object creation (`none`/`bool`/`int`/`float`/`bytes`/`tuple`/`list`/`dict`/`str`), typed getters, sequence+dict access, iterator helpers (`object_get_iter`/`object_iter_next`), list mutation (`append`/indexed set), dict mutation (`set`/`get`/`contains`/`del`), object attribute get/set/del/has by handle+name, type relation checks (`object_is_instance`/`object_is_subclass`), handle-based callable invocation (`object_call`, `object_call_noargs`, `object_call_onearg`), and `incref`/`decref`; full CPython object surface remains open. |
| Exception indicator/thread-local error state APIs | IN PROGRESS | runtime/ffi | `tests/extension_smoke.rs::dynamic_extension_error_state_is_propagated_to_import_failure`, `tests/extension_smoke.rs::dynamic_extension_can_read_and_clear_error_message` | Import-time extension error state is surfaced and can now be inspected through `error_get_message(...)`; broader CPython exception APIs remain open. |
| Native callable registration/invocation substrate | IN PROGRESS | runtime/ffi + VM | `tests/extension_smoke.rs::dynamic_extension_can_register_callable`, `tests/extension_smoke.rs::dynamic_extension_can_register_kw_callable` | Positional and keyword callback baselines landed via `module_add_function` + `module_add_function_kw`, including keyword validation/error propagation coverage; vectorcall/signature introspection parity is still open. |
| GIL attach/detach APIs (`PyGILState_*`) | PLANNED | runtime/threading | - | Required for threaded native callers. |

## Interop Protocols

| Surface | Status | Owner | Evidence | Notes |
|---|---|---|---|---|
| Buffer protocol producer/consumer parity | PLANNED | runtime/object-model | - | High-priority dependency for NumPy. |
| Capsule/callback interop primitives | PLANNED | runtime/ffi | - | Needed by ecosystem modules that pass opaque handles. |
| ABI capability introspection API | IN PROGRESS | runtime/ffi | `tests/extension_smoke.rs::dynamic_extension_can_query_capabilities` | Baseline `api_has_capability(...)` probe now includes callable registration, dict/list handle mutation, and legacy v1 surfaces; richer capability taxonomy/versioning is still open. |

## Ecosystem Gates

| Gate | Status | Owner | Evidence | Notes |
|---|---|---|---|---|
| Extension smoke gate (compiled native fixture + `hello_ext`) | DONE | VM/extensions | `tests/extension_smoke.rs` + CI `Extension smoke lane` | CI covers manifest-only, compiled-manifest, direct shared-object, tagged-filename, iterator+mixed-surface cross-API fixtures, invalid-handle resilience fixture, and error-path fixtures. |
| NumPy import gate (`import numpy`) | IN PROGRESS | milestone-15 bring-up | `scripts/probe_numpy_gate.py` + `docs/NUMPY_BRINGUP_GATE.md` | Probe scaffold is landed; gate currently expected-red until C-extension substrate matures. |
| NumPy ndarray smoke (`np.array([...]).sum()`) | IN PROGRESS | milestone-15 bring-up | `scripts/probe_numpy_gate.py` + `docs/NUMPY_BRINGUP_GATE.md` | Same as above. |
| Pandas/matplotlib/scipy smoke gates | PLANNED | milestone-15 bring-up | - | Starts after NumPy substrate closure. |

## Policy

1. A surface cannot be marked `DONE` without deterministic test evidence.
2. Temporary scaffolding must be tracked in `docs/STUB_ACCOUNTING.md` with closure criteria.
3. This matrix must be updated in the same checkpoint as any extension-surface behavior change.
