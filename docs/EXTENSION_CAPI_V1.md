# Extension C-API v1 Slice

Status: active baseline (Milestone 15).

This is the first shipped `libpyrs-capi` contract slice used by compiled extension smoke tests.

## Header

- `/Users/$USER/pyrs/include/pyrs_capi.h`

## ABI Version

- `PYRS_CAPI_ABI_VERSION = 1`

## Exposed API Table (`PyrsApiV1`)

- `module_set_int(void* module_ctx, const char* name, int64_t value)`
- `module_set_bool(void* module_ctx, const char* name, int value)`
- `module_set_string(void* module_ctx, const char* name, const char* value)`

All functions return `0` on success and non-zero on failure.

## Extension Init Symbol

- Default symbol: `pyrs_extension_init_v1`
- Signature:
  - `int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx)`

## Current Guarantees

- API table pointers are valid only during init call.
- `module_ctx` points to the target Python module object context and supports setting module globals through API functions.
- ABI mismatch must be handled by extension code and reflected via non-zero return.

## Out of Scope (not yet implemented)

- Refcount/object constructors beyond primitive module global setters.
- Exception indicator APIs.
- Callable/type registration APIs.
- Thread/GIL APIs.
- Multi-phase module lifecycle APIs.

These are tracked in `/Users/$USER/pyrs/docs/EXTENSION_CAPABILITY_MATRIX.md`.
