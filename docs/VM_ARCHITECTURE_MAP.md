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
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_args_runtime.rs`
  - CPython tuple/dict argument conversion helpers (`cpython_positional_args_from_tuple_object`, `cpython_keyword_args_from_dict_object`)
  - shared argument normalization path used by CPython ABI call entrypoints and shims
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_module_runtime.rs`
  - CPython module-def/runtime helpers (`cpython_bind_module_def`, `cpython_new_module_data`)
  - module-state allocation/free bridge used by CPython module creation/exec paths
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_import_runtime.rs`
  - CPython import helper substrate (`cpython_import_add_module_by_name`, inittab registry/lookup, exec-code-in-module flow)
  - shared import-state wiring used by `PyImport_*` C-API entrypoints
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_module_name_runtime.rs`
  - CPython module-name/value conversion helpers for `PyImport_*` and `PyModule_*` paths
  - short type-name derivation + optional pointer-to-value conversion helpers
- `/Users/$USER/pyrs/src/vm/vm_extensions/cpython_exception_name_runtime.rs`
  - CPython exception-name parsing helpers (`runtime-message -> exception name`, `module.class split`)
  - shared normalization logic used by `PyErr_*` creation and error propagation paths

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
- New CPython C-API arg conversion behavior: `vm_extensions/cpython_args_runtime.rs`.
- New CPython module-def/state helper behavior: `vm_extensions/cpython_module_runtime.rs`.
- New CPython import helper behavior: `vm_extensions/cpython_import_runtime.rs`.
- New CPython module-name/value helper behavior: `vm_extensions/cpython_module_name_runtime.rs`.
- New CPython exception-name helper behavior: `vm_extensions/cpython_exception_name_runtime.rs`.
- Shared VM helper for multiple domains: `vm_runtime_methods.rs`.
- Native stdlib substrate behavior: matching module in `/Users/$USER/pyrs/src/vm/stdlib/`.

## Guardrails
- Keep `mod.rs` under strict size pressure; do not regress toward monolith.
- Prefer cohesive refactors over one-off patches.
- For major structural changes, update this document and relevant roadmap/readiness docs.

## Current Follow-Up Decomposition Targets
- Move large free-function clusters currently still in `mod.rs` into focused helper modules by domain (regex/codecs/formatting/time utilities).
- Continue decomposing `/Users/$USER/pyrs/src/vm/vm_extensions.rs` into focused submodules (proxy runtime, ABI symbol surfaces, extension loader phases) without `include!` chunking.
- Next decomposition slice target: move CPython compatibility helper clusters (pointer/object conversion and legacy C-API helper blocks) into focused `vm_extensions/*` modules.
- Continue reducing clone-heavy hot paths identified in `/Users/$USER/pyrs/docs/CLONE_AUDIT.md`.
