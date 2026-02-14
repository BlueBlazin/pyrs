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
  - v1 C-API callback bridge (`include/pyrs_capi.h`)
  - extension module metadata/entrypoint wiring
  - owns extension-loader behavior inside VM import execution

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
- Shared VM helper for multiple domains: `vm_runtime_methods.rs`.
- Native stdlib substrate behavior: matching module in `/Users/$USER/pyrs/src/vm/stdlib/`.

## Guardrails
- Keep `mod.rs` under strict size pressure; do not regress toward monolith.
- Prefer cohesive refactors over one-off patches.
- For major structural changes, update this document and relevant roadmap/readiness docs.

## Current Follow-Up Decomposition Targets
- Move large free-function clusters currently still in `mod.rs` into focused helper modules by domain (regex/codecs/formatting/time utilities).
- Continue reducing clone-heavy hot paths identified in `/Users/$USER/pyrs/docs/CLONE_AUDIT.md`.
