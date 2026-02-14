# Extension Packaging and Build Contract

Status: active draft (Milestone 15 substrate).

This document defines how native-extension packaging/build will work for `pyrs` and what is currently supported.

## Compatibility Modes

1. `pyrs314` (source-build mode, primary):
   - Extensions are built against `pyrs` headers/libraries.
   - This is the only mode that will be advertised until ABI evidence is green.
2. `cp314` wheel-compat mode (deferred):
   - Reuse of CPython wheels is permitted only for explicitly verified ABI subsets.

## Current Implemented Surface

- Import path can discover extension manifests with suffix `.pyrs-ext`.
- Loader can instantiate a manifest-backed extension module via `pyrs.ExtensionFileLoader`.
- Manifest dynamic entrypoint is supported via `entrypoint=dynamic:<symbol>` + `library=<path>`.
- Direct shared-object imports are supported (`module.so` / `module.dylib` / `module.pyd`) using default init symbol `pyrs_extension_init_v1`.
- Tagged CPython-style shared-object names are recognized for import resolution (e.g. `module.cpython-314-*.so`).
- First C-API header/symbol slice is shipped in `/Users/$USER/pyrs/include/pyrs_capi.h`.

This is still an early substrate, not full C-extension compatibility.

## Manifest Contract (`.pyrs-ext`)

Manifest format is line-based key/value (`key=value`) with comments (`#`) allowed.

Required keys:
- `module`: fully-qualified module name.
- `abi`: currently must be `pyrs314`.
- `entrypoint`: supported values:
  - `hello_ext` (internal smoke entrypoint),
  - `dynamic:<symbol>` (shared-library symbol).

Conditional keys:
- `library`: required when using `entrypoint=dynamic:<symbol>`; may be absolute or manifest-relative.

Supported dynamic init symbol contract (v1):
- receives `PyrsApiV1` and `module_ctx`.
- can set module globals directly or via init-scoped object handles.
- can report import-time failure details via `error_set(...)`.

Example:

```text
module=hello_ext
abi=pyrs314
entrypoint=hello_ext
```

Dynamic example:

```text
module=native_mod
abi=pyrs314
entrypoint=dynamic:pyrs_extension_init_v1
library=libnative_mod.so
```

## Planned Build Contract (next phases)

1. Produce `libpyrs-capi` with versioned symbols.
2. Publish headers and `sysconfig` values for extension compilation.
3. Add build metadata for PEP 517 backends to compile against `pyrs314`.
4. Add wheel tag policy and acceptance matrix (`pyrs314-*`).
5. Gate any future `cp314` claims on symbol-level ABI conformance tests.

## Diagnostics Contract

Unsupported extension paths must fail explicitly with:
- unsupported ABI tag,
- unsupported entrypoint,
- missing/invalid manifest keys,
- unimplemented C-ABI surfaces.

No silent fallback is allowed for native-extension errors.

## CI and Quality Gates

- `hello_ext` smoke path is required green in CI.
- compiled native extension smoke path is required green in CI.
- NumPy bring-up probe artifacts are generated and tracked separately.
- Any extension-surface change must update:
  - `docs/EXTENSION_CAPABILITY_MATRIX.md`
  - `docs/EXTENSION_ECOSYSTEM_DESIGN.md`
  - this file.
