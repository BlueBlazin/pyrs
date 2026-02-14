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
- `module_add_function(void* module_ctx, const char* name, PyrsCFunctionV1 callback)`
- `module_add_function_kw(void* module_ctx, const char* name, PyrsCFunctionKwV1 callback)`
- `object_new_int(void* module_ctx, int64_t value)`
- `object_new_none(void* module_ctx)`
- `object_new_bool(void* module_ctx, int value)`
- `object_new_float(void* module_ctx, double value)`
- `object_new_bytes(void* module_ctx, const uint8_t* data, uintptr_t len)`
- `object_new_string(void* module_ctx, const char* value)`
- `object_incref(void* module_ctx, PyrsObjectHandle handle)`
- `object_decref(void* module_ctx, PyrsObjectHandle handle)`
- `module_set_object(void* module_ctx, const char* name, PyrsObjectHandle handle)`
- `object_type(void* module_ctx, PyrsObjectHandle handle)`
- `object_get_int(void* module_ctx, PyrsObjectHandle handle, int64_t* out)`
- `object_get_float(void* module_ctx, PyrsObjectHandle handle, double* out)`
- `object_get_bool(void* module_ctx, PyrsObjectHandle handle, int* out)`
- `object_get_bytes(void* module_ctx, PyrsObjectHandle handle, const uint8_t** out_data, uintptr_t* out_len)`
- `object_get_string(void* module_ctx, PyrsObjectHandle handle)`
- `error_set(void* module_ctx, const char* message)`
- `error_clear(void* module_ctx)`
- `error_occurred(void* module_ctx)`

Return semantics:
- setter/refcount/error functions return `0` on success and non-zero on failure.
- object constructor functions return non-zero handle on success; `0` indicates failure.
- callable callbacks return `0` on success and set `*result` to a non-zero object handle.

## Extension Init Symbol

- Default symbol: `pyrs_extension_init_v1`
- Signature:
  - `int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx)`

## Current Guarantees

- API table pointers are valid only during init call.
- `module_ctx` points to the target Python module object context.
- object handles are init-call scoped; module globals retain values after handle release.
- extension error state set via `error_set(...)` is propagated into import-time runtime errors.
- callable registration via `module_add_function(...)` is supported for positional-only callbacks.
- callable registration via `module_add_function_kw(...)` is supported for callbacks receiving keyword-name/value arrays.
- ABI mismatch must be handled by extension code and reflected via non-zero return.

## Out of Scope (not yet implemented)

- General `PyObject` constructors/surfaces beyond int/bool/string and module-global assignment path.
- Full CPython-style argument parsing helpers/signature metadata for native callables.
- Thread/GIL APIs.
- Multi-phase module lifecycle APIs.

These are tracked in `/Users/$USER/pyrs/docs/EXTENSION_CAPABILITY_MATRIX.md`.
