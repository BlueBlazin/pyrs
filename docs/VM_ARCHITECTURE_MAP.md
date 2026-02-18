# VM Architecture Map

This document defines the current structure and ownership boundaries for the VM implementation.

## Purpose
- Keep VM code reviewable and maintainable after monolith decomposition.
- Prevent regressions back to a single massive implementation file.
- Make it clear where new behavior belongs.

## Top-Level Layout

### Orchestration and shared internals
- `/Users/$USER/pyrs/src/vm/mod.rs`
  - VM type/state definitions (`Vm`, `Frame`, core structs/enums/constants)
  - global/shared helper functions used across VM domains
  - module declarations and shared orchestration wiring
  - should not absorb large domain logic blocks

### Execution and dispatch
- `/Users/$USER/pyrs/src/vm/vm_execution.rs`
  - bytecode execution loop and core execution flow control
  - CPython-style fast-locals handling (slot-backed locals similar to `f_localsplus`, with dict-style locals synced on demand)
- `/Users/$USER/pyrs/src/vm/vm_native_dispatch.rs`
  - dispatch from `BuiltinFunction`/native method kinds to VM handlers
- `/Users/$USER/pyrs/src/vm/vm_builtin_metadata.rs`
  - builtin metadata tables and helper mappings

### Bootstrap and imports
- `/Users/$USER/pyrs/src/vm/vm_bootstrap_import.rs`
  - VM/module bootstrap wiring
  - import-system construction and import-path foundations
- `/Users/$USER/pyrs/src/vm/vm_extensions.rs`
  - extension manifest loader execution path (`.pyrs-ext` scaffolding)
  - direct shared-library extension execution path (`.so/.dylib/.pyd`)
  - CPython ABI/runtime interop substrate and proxy runtime behavior
  - extension module metadata/entrypoint wiring
  - owns extension-loader behavior inside VM import execution
- `/Users/$USER/pyrs/src/vm/vm_extensions/capi_v1.rs`
  - v1 extension C-API callback bridge (`include/pyrs_capi.h`)
  - exported C-API v1 function-pointer table wiring (`Vm::capi_api_v1`)
  - C-API handle/object/module/buffer/capsule call surface for native extension callbacks
- `/Users/$USER/pyrs/src/vm/vm_extensions/proxy_runtime.rs`
  - CPython proxy object runtime bridge (`call`, numeric ops, attr lookup, iter/getitem/setitem)
  - proxy attribute and slot fallback dispatch paths used by cross-module VM runtime surfaces
- `/Users/$USER/pyrs/src/vm/vm_extensions/callable_runtime.rs`
  - extension callable registration + dispatch runtime (`register_extension_callable`, `call_extension_callable`)
  - native/cpython callback invocation path ownership for extension-bound methods/functions
- `/Users/$USER/pyrs/src/vm/vm_extensions/loader_runtime.rs`
  - extension loader/exec runtime (`exec_extension_module`, dynamic shared-object init flow)
  - CPython-style module-def method registration + `PyInit_*` slot execution flow ownership
  - extension init metadata publication and init-state failure tracking
- `/Users/$USER/pyrs/src/vm/vm_extensions/module_context_state.rs`
  - `ModuleCapiContext` module-attribute/state/capsule-registry lifecycle helpers
  - owns module state finalize/free wiring and exported capsule synchronization paths
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_context_runtime.rs`
  - CPython active-context bridge helpers (`with_active_cpython_context_mut`, `cpython_set_active_context`)
  - CPython pointer/error conversion helpers (`cpython_value_from_ptr*`, `cpython_set_error`, typed-error helpers)
  - builtin C-function bridge shim callback wiring (`cpython_builtin_cfunction_varargs_kwargs`)
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_contextvar_api.rs`
  - exported `PyContextVar_*` C-API entrypoints (`PyContextVar_New`, `PyContextVar_Get`, `PyContextVar_Set`)
  - delegates shared pointer/state behavior to active-context/runtime helpers
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_eval_api.rs`
  - exported `PyEval_*` frame/global/locals/function-descriptor C-API entrypoints
  - delegates shared frame/module lookup behavior to active-context/runtime helpers
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_gc_alloc_api.rs`
  - exported object allocator/GC C-API entrypoints (`PyObject_Malloc/Calloc/Realloc/Free`, `PyObject_GC_*`, `PyGC_*`)
  - shared VM GC state toggle/collect wiring and object trackedness checks
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_iter_api.rs`
  - exported `PyIter_*` C-API entrypoints (`PyIter_Check`, `PyIter_NextItem`, `PyIter_Send`, `PyIter_Next`)
  - shared iterator/StopIteration compatibility helpers (`cpython_*iterator*_for_capi`, active-exception probes/clears)
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_capsule_api.rs`
  - exported `PyCapsule_*` C-API entrypoints (`New`, `GetPointer`, `Get/SetName`, `Get/SetContext`, `Get/SetDestructor`, `IsValid`, `Import`)
  - shared external-capsule pointer validation bridge for non-owned capsule objects
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_list_api.rs`
  - exported `PyList_*` C-API entrypoints (`New`, `Size`, `Append`, `Get/SetItem`, `Insert`, `Get/SetSlice`, `Sort`, `Reverse`, `AsTuple`, `GetItemRef`)
  - shared list-storage synchronization paths for CPython-compatible list backing (`ob_size`/`ob_item`)
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_long_float_api.rs`
  - exported `PyLong_*` / `PyBool_FromLong` / `PyFloat_*` C-API entrypoints (constructors, parse helpers, native-bytes conversions, `PyLong_GetInfo`)
  - shared bigint/two's-complement conversion wiring and typed-error parity for numeric C-API surfaces
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_mem_api.rs`
  - exported memory allocator C-API entrypoints (`PyMem_Raw*`, `PyMem_*`)
  - shared allocator forwarding + CPython-allocation ownership guard behavior
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_tuple_api.rs`
  - exported `PyTuple_*` C-API entrypoints (`New`, `Size`, `GetItem`, `SetItem`, `GetSlice`)
  - shared tuple-storage compatibility paths for CPython tuple backing (`ob_size` + item-slot mirror)
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_dict_api.rs`
  - exported `PyDict_*` / `_PyDict_*` / `PyDictProxy_New` C-API entrypoints (set/get/pop/contains/merge/view/next helpers)
  - shared dict mutation + mapping-slot fallback paths and CPython-style error-state behavior
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_set_api.rs`
  - exported `PySet_*` / `PyFrozenSet_New` C-API entrypoints (`new`, `size`, `contains`, `add`, `discard`, `clear`, `pop`)
  - delegates set/frozenset method semantics to native set runtime dispatch
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_object_call_api.rs`
  - exported call/iterator/vectorcall/object-core C-API entrypoints (`PyObject_IsTrue/Not/Str/Repr/ASCII`, `PyObject_GetIter/GetAIter`, `PyObject_Call*`, `PyObject_Vectorcall*`, `PyMethod_New`, `PyCode_New*`)
  - shared vectorcall decode/materialization and managed-dict/finalizer helper behavior
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_object_item_compare_api.rs`
  - exported object item/hash/compare C-API entrypoints (`PyObject_Get/Set/DelItem`, `PyObject_Size/Length/LengthHint`, `PyObject_Hash*`, `PyObject_RichCompare*`, `PyObject_IsInstance/IsSubclass`, `PyObject_GetOptionalAttr`)
  - shared rich-compare slot fallback and compare-debug/type-name helpers used by remaining object C-API surfaces
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_object_buffer_api.rs`
  - exported object buffer/memoryview/print C-API entrypoints (`PyObject_Check*Buffer`, `PyObject_As*Buffer`, `PyObject_GetBuffer`, `PyObject_CopyData`, `PyObject_Print`, `PyBuffer_*`, `PyMemoryView_*`)
  - shared contiguous-layout helpers and CPython buffer struct wiring for bytes/bytearray/memoryview interop
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_object_lifecycle_api.rs`
  - exported object lifecycle C-API entrypoints (`PyObject_Init*`, `_PyObject_New*`, `_PyObject_GC_New`, `_Py_Dealloc`)
  - shared raw object header initialization and CPython pointer-handle dealloc bridging behavior
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_weakref_api.rs`
  - exported weakref C-API entrypoints (`PyWeakref_NewRef`, `PyWeakref_NewProxy`, `PyWeakref_GetRef`, `PyWeakref_GetObject`, `PyObject_ClearWeakRefs`)
  - shared weakref-target extraction and callable-callback validation for CPython weakref helper semantics
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_runtime_misc_api.rs`
  - exported CPython runtime/misc C-API entrypoints (`Py_Repr*`, `Py_AddPendingCall`/`Py_MakePendingCalls`/`Py_AtExit`, version/build/platform getters, path/config wide-char APIs, `Py_{Initialize,Finalize,Main,BytesMain,CompileString,Exit}`, fatal-error APIs, `_PyErr_BadInternalCall`, `_Py_HashDouble`, `_PyUnicode_Is*`)
  - shared pending-call queue/atexit lifecycle, path-config storage, and compile/CLI bridge behavior
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_refcount_api.rs`
  - exported refcount/internal GC C-API entrypoints (`Py_{IncRef,DecRef,XIncRef,XDecRef}`, `_Py_{IncRef,DecRef,SetRefcnt,NegativeRefcount,CheckRecursiveCall}`, `_PyObject_GC_{NewVar,Resize}`)
  - shared CPython header-refcount mutation and active-context handle decref/incref synchronization behavior
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_type_api.rs`
  - exported type-object C-API entrypoints (`PyType_*`, `_PyType_Lookup`, generic type call/new/alloc helpers)
  - shared heap-type registry/type-slot application + metaclass/from-spec construction behavior
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_object_attr_api.rs`
  - exported `PyObject_*` attribute/introspection C-API entrypoints (`Get/Set/DelAttr*`, `Type`, `HasAttr*`, `GetOptionalAttrString`, generic attr/dict helpers)
  - shared native-slot fallback (`tp_getattro`/`tp_setattro`) + CPython-style missing-attribute error handling
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_bytes_api.rs`
  - exported `PyBytes_*` / `_PyBytes_Join` / `PyByteArray_*` C-API entrypoints (construction, concat, repr/decode, size/data access, resize, buffer-based concat)
  - shared bytes/bytearray storage interop with CPython layout mirrors and buffer-release error semantics
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_args_runtime.rs`
  - CPython tuple/dict argument conversion helpers (`cpython_positional_args_from_tuple_object`, `cpython_keyword_args_from_dict_object`)
  - shared argument normalization path used by CPython ABI call entrypoints and shims
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_module_runtime.rs`
  - CPython module-def/runtime helpers (`cpython_bind_module_def`, `cpython_new_module_data`)
  - module-state allocation/free bridge used by CPython module creation/exec paths
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_module_api.rs`
  - exported `PyModule_*` C-API entrypoints (create/from-spec/exec/get/new/add/add-constants/add-functions/add-type/get-dict)
  - delegates module-def binding/state setup and name normalization to module runtime/helper substrates
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_import_runtime.rs`
  - CPython import helper substrate (`cpython_import_add_module_by_name`, inittab registry/lookup, exec-code-in-module flow)
  - shared import-state wiring used by `PyImport_*` C-API entrypoints
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_import_api.rs`
  - exported `PyImport_*` C-API entrypoints (magic, module import/add/get, inittab, frozen/import-exec, importer/reload)
  - delegates shared state/update logic to import runtime + module-name helper substrates
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_module_name_runtime.rs`
  - CPython module-name/value conversion helpers for `PyImport_*` and `PyModule_*` paths
  - short type-name derivation + optional pointer-to-value conversion helpers
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_exception_name_runtime.rs`
  - CPython exception-name parsing helpers (`runtime-message -> exception name`, `module.class split`)
  - shared normalization logic used by `PyErr_*` creation and error propagation paths
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_call_runtime.rs`
  - CPython active-context call helpers (`cpython_call_internal_in_context`, `cpython_getattr_in_context`)
  - shared call/attribute dispatch substrate used by codec and C-API helper flows
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_codec_runtime.rs`
  - CPython codec helper substrate (`cpython_codec_*` lookup/call/error helpers, built-in codec error handler method defs)
  - shared codec C-API runtime used by `PyCodec_*` entrypoints
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_codec_api.rs`
  - exported `PyCodec_*` C-API entrypoints (register/lookup/encode/decode/stream/error handlers)
  - delegates shared behavior to codec/call/context helper substrates
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_unicode_error_runtime.rs`
  - CPython unicode-error helper substrate (`cpython_unicode_error_*`, `CpythonUnicodeErrorFlavor`)
  - shared unicode-error C-API getter/setter and validation logic used by `PyUnicode*Error_*` entrypoints
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_unicode_error_api.rs`
  - exported `PyUnicode*Error_*` C-API entrypoints (`PyUnicodeDecodeError_Create`, `Get/SetEncoding|Object|Start|End|Reason`)
  - delegates shared behavior to `cpython_unicode_error_runtime.rs` and active-context helpers
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_numeric_runtime.rs`
  - CPython numeric op helper substrate (`cpython_unary_numeric_op`, `cpython_binary_numeric_op`, `cpython_binary_numeric_op_with_heap`)
  - shared pointer->value->numeric-dispatch conversion paths used by `PyNumber_*` entrypoints
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_numeric_api.rs`
  - exported `PyNumber_*` C-API entrypoints (numeric predicates, binary/unary/in-place ops, conversion helpers)
  - delegates shared dispatch/conversion behavior to numeric runtime/context helper modules

### Core method helpers
- `/Users/$USER/pyrs/src/vm/vm_runtime_methods.rs`
  - VM-side runtime helper methods shared by multiple domains

### Builtins by domain
- `/Users/$USER/pyrs/src/vm/builtins_core.rs`
  - core builtins and foundational object behavior
- `/Users/$USER/pyrs/src/vm/builtins_import.rs`
  - importlib/builtin import helpers
- `/Users/$USER/pyrs/src/vm/builtins_numeric_time.rs`
  - numeric/time/random/math oriented builtins
- `/Users/$USER/pyrs/src/vm/builtins_os.rs`
  - os/path/process/fs/network-adjacent builtins
- `/Users/$USER/pyrs/src/vm/builtins_collections.rs`
  - list/tuple/dict/set/itertools/functools/collections behavior
- `/Users/$USER/pyrs/src/vm/builtins_io.rs`
  - `_io`/file/stream/text-wrapper builtins
- `/Users/$USER/pyrs/src/vm/builtins_system_misc.rs`
  - threading/signal/socket/uuid/warnings/colorize/misc system surfaces

### Existing focused modules
- `/Users/$USER/pyrs/src/vm/ops.rs`
  - arithmetic/comparison/operator kernels
- `/Users/$USER/pyrs/src/vm/containers.rs`
  - container helper kernels and parity-sensitive container utilities
- `/Users/$USER/pyrs/src/vm/stdlib/`
  - native stdlib substrate modules (`json`, `re`, `csv`, `pickle`)

## Dependency and Ownership Rules
1. Keep domain behavior in its owning file; avoid cross-domain leakage.
2. Shared helper logic belongs in `vm_runtime_methods.rs` or focused helper modules, not copied across builtin files.
3. `mod.rs` is orchestration only; large new behavior should not be added directly there.
4. If a new domain exceeds reviewable size, split by coherent sub-domain (not arbitrary chunking).
5. Behavior changes must ship with tests in the same commit.

## Placement Rules for New Code
- New opcode execution behavior: `vm_execution.rs` (or `ops.rs` if pure operator kernel).
- New builtin function dispatch path: `vm_native_dispatch.rs` + owning `builtins_*.rs` implementation.
- New import/bootstrap wiring: `vm_bootstrap_import.rs`.
- New extension-loader runtime behavior: `vm_extensions.rs` (and `src/extensions/` for manifest/types).
- New extension C-API v1 entrypoints/table wiring: `vm_extensions/capi_v1.rs`.
- New CPython proxy runtime behavior and proxy-special operation dispatch: `vm_extensions/proxy_runtime.rs`.
- New extension callable register/dispatch behavior: `vm_extensions/callable_runtime.rs`.
- New extension loader/exec phase behavior: `vm_extensions/loader_runtime.rs`.
- New `ModuleCapiContext` state/capsule lifecycle behavior: `vm_extensions/module_context_state.rs`.
- New CPython active-context pointer/error bridge behavior: `vm_extensions/cpython_context_runtime.rs`.
- New CPython contextvar C-API entrypoint behavior: `vm_extensions/cpython_contextvar_api.rs`.
- New CPython eval C-API entrypoint behavior: `vm_extensions/cpython_eval_api.rs`.
- New CPython object allocator/GC C-API entrypoint behavior: `vm_extensions/cpython_gc_alloc_api.rs`.
- New CPython iter C-API entrypoint behavior: `vm_extensions/cpython_iter_api.rs`.
- New CPython capsule C-API entrypoint behavior: `vm_extensions/cpython_capsule_api.rs`.
- New CPython list C-API entrypoint behavior: `vm_extensions/cpython_list_api.rs`.
- New CPython long/float C-API entrypoint behavior: `vm_extensions/cpython_long_float_api.rs`.
- New CPython memory allocator C-API entrypoint behavior: `vm_extensions/cpython_mem_api.rs`.
- New CPython tuple C-API entrypoint behavior: `vm_extensions/cpython_tuple_api.rs`.
- New CPython dict C-API entrypoint behavior: `vm_extensions/cpython_dict_api.rs`.
- New CPython set C-API entrypoint behavior: `vm_extensions/cpython_set_api.rs`.
- New CPython object-core call/vectorcall C-API entrypoint behavior: `vm_extensions/cpython_object_call_api.rs`.
- New CPython object item/hash/compare C-API entrypoint behavior: `vm_extensions/cpython_object_item_compare_api.rs`.
- New CPython object buffer/memoryview C-API entrypoint behavior: `vm_extensions/cpython_object_buffer_api.rs`.
- New CPython object lifecycle C-API entrypoint behavior: `vm_extensions/cpython_object_lifecycle_api.rs`.
- New CPython weakref C-API entrypoint behavior: `vm_extensions/cpython_weakref_api.rs`.
- New CPython runtime/misc C-API entrypoint behavior: `vm_extensions/cpython_runtime_misc_api.rs`.
- New CPython refcount/internal-GC C-API entrypoint behavior: `vm_extensions/cpython_refcount_api.rs`.
- New CPython type-object C-API entrypoint behavior: `vm_extensions/cpython_type_api.rs`.
- New CPython object-attr C-API entrypoint behavior: `vm_extensions/cpython_object_attr_api.rs`.
- New CPython bytes/bytearray C-API entrypoint behavior: `vm_extensions/cpython_bytes_api.rs`.
- New CPython C-API arg conversion behavior: `vm_extensions/cpython_args_runtime.rs`.
- New CPython module-def/state helper behavior: `vm_extensions/cpython_module_runtime.rs`.
- New CPython module C-API entrypoint behavior: `vm_extensions/cpython_module_api.rs`.
- New CPython import helper behavior: `vm_extensions/cpython_import_runtime.rs`.
- New CPython import C-API entrypoint behavior: `vm_extensions/cpython_import_api.rs`.
- New CPython module-name/value helper behavior: `vm_extensions/cpython_module_name_runtime.rs`.
- New CPython exception-name helper behavior: `vm_extensions/cpython_exception_name_runtime.rs`.
- New CPython active-context call helper behavior: `vm_extensions/cpython_call_runtime.rs`.
- New CPython codec helper behavior: `vm_extensions/cpython_codec_runtime.rs`.
- New CPython codec C-API entrypoint behavior: `vm_extensions/cpython_codec_api.rs`.
- New CPython unicode-error helper behavior: `vm_extensions/cpython_unicode_error_runtime.rs`.
- New CPython unicode-error C-API entrypoint behavior: `vm_extensions/cpython_unicode_error_api.rs`.
- New CPython numeric-op helper behavior: `vm_extensions/cpython_numeric_runtime.rs`.
- New CPython numeric C-API entrypoint behavior: `vm_extensions/cpython_numeric_api.rs`.
- Shared VM helper for multiple domains: `vm_runtime_methods.rs`.
- Native stdlib substrate behavior: matching module in `/Users/$USER/pyrs/src/vm/stdlib/`.

## Guardrails
- Keep `mod.rs` under strict size pressure; do not regress toward monolith.
- Prefer cohesive refactors over one-off patches.
- For major structural changes, update this document and relevant roadmap/readiness docs.

## Current Follow-Up Decomposition Targets
- Move large free-function clusters currently still in `mod.rs` into focused helper modules by domain (regex/codecs/formatting/time utilities).
- Continue decomposing `/Users/$USER/pyrs/src/vm/vm_extensions.rs` into focused submodules (proxy runtime, ABI symbol surfaces, extension loader phases) without `include!` chunking.
- Next decomposition slice target: move remaining CPython C-API entrypoint clusters still in `vm_extensions.rs` (unicode/threadstate utility clusters) into focused `vm_extensions/*_api.rs` modules.
- Continue reducing clone-heavy hot paths identified in `/Users/$USER/pyrs/docs/CLONE_AUDIT.md`.
