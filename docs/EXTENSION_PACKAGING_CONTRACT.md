# Native Extension Packaging and Build Contract

## Purpose

This document describes the extension loading and source-build contract that is
implemented today.

Primary evidence:

- `include/pyrs_capi.h`
- `tests/extension_smoke.rs`
- `src/vm/vm_extensions.rs`
- `src/vm/vm_extensions/capi_v1.rs`

## Current Supported Import Forms

Manifest-backed import:

- extension manifest suffix: `.pyrs-ext`
- loader entrypoint: `pyrs.ExtensionFileLoader`
- supported manifest entrypoints:
  - `hello_ext`
  - `dynamic:<symbol>`

Direct shared-object import:

- recognized filename families:
  - `module.so`
  - `module.dylib`
  - `module.pyd`
  - tagged CPython-style names such as `module.cpython-314-*.so`
- runtime tries `pyrs_extension_init_v1` first
- runtime falls back to `PyInit_<module>` when present

## Manifest Contract

Manifest format is line-based `key=value` with `#` comments.

Required keys:

- `module`
- `abi`
- `entrypoint`

Conditional key:

- `library`
  - required for `entrypoint=dynamic:<symbol>`
  - may be absolute or manifest-relative

Current accepted ABI tag:

- `pyrs314`

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

## Source-Build Contract

The current source-build substrate is exposed through builtin
`_sysconfigdata__*` values exercised by `tests/extension_smoke.rs`.

Implemented build keys include:

- `SOABI`
- `EXT_SUFFIX`
- `CC`
- `LDSHARED`
- `AR`
- `ARFLAGS`
- `CCSHARED`
- `BLDSHARED`
- `CPPFLAGS`
- `LDFLAGS`
- `LIBPL`
- `INCLUDEDIR`
- `Py_ENABLE_SHARED`

The smoke suite includes a compile-and-import round trip that consumes those
values end to end.

## Current C-API Entry Surface

For direct CPython-style `PyInit_<module>` entrypoints, the implemented
single-phase compatibility slice currently includes:

- `PyModule_Create2`
- `PyModule_AddObjectRef`
- `PyModule_AddIntConstant`
- `PyModule_AddStringConstant`
- core scalar/bytes constructors used by the smoke fixtures
- `PyErr_*` baseline helpers
- `Py_[X]IncRef` / `Py_[X]DecRef`

For manifest-backed native modules using `pyrs_extension_init_v1`, the current
handle-based API surface is documented in `docs/EXTENSION_CAPI_V1.md`.

## Diagnostics Contract

Unsupported extension paths must fail explicitly for cases such as:

- unsupported ABI tag
- unsupported entrypoint
- missing manifest keys
- missing shared library
- missing init symbol
- unimplemented runtime/C-ABI surface

No silent fallback is allowed for native-extension errors.

## Explicitly Out Of Scope

These are not current support claims:

- PEP 489 multi-phase init
- general CPython wheel compatibility
- broad CPython C-extension compatibility beyond the tested surface
