# Extension C-API v1 Slice

Status: active baseline (Milestone 15).

This is the first shipped `libpyrs-capi` contract slice used by compiled extension smoke tests.

## Header

- `/Users/$USER/pyrs/include/pyrs_capi.h`
- Buffer view struct:
  - `PyrsBufferViewV1 { const uint8_t* data; uintptr_t len; int readonly; }`

## ABI Version

- `PYRS_CAPI_ABI_VERSION = 1`

## Exposed API Table (`PyrsApiV1`)

- `api_has_capability(void* module_ctx, const char* name)` (`1` supported, `0` unsupported)
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
- `object_new_tuple(void* module_ctx, uintptr_t len, const PyrsObjectHandle* items)`
- `object_new_list(void* module_ctx, uintptr_t len, const PyrsObjectHandle* items)`
- `object_new_dict(void* module_ctx)`
- `object_new_string(void* module_ctx, const char* value)`
- `object_incref(void* module_ctx, PyrsObjectHandle handle)`
- `object_decref(void* module_ctx, PyrsObjectHandle handle)`
- `module_set_object(void* module_ctx, const char* name, PyrsObjectHandle handle)`
- `module_get_object(void* module_ctx, const char* name, PyrsObjectHandle* out_handle)`
- `module_import(void* module_ctx, const char* module_name, PyrsObjectHandle* out_handle)`
- `module_get_attr(void* module_ctx, PyrsObjectHandle module_handle, const char* attr_name, PyrsObjectHandle* out_handle)`
- `module_set_attr(void* module_ctx, PyrsObjectHandle module_handle, const char* attr_name, PyrsObjectHandle value_handle)`
- `module_del_attr(void* module_ctx, PyrsObjectHandle module_handle, const char* attr_name)`
- `module_has_attr(void* module_ctx, PyrsObjectHandle module_handle, const char* attr_name)` (`1`/`0` on success, `-1` on error)
- `object_type(void* module_ctx, PyrsObjectHandle handle)`
- `object_is_instance(void* module_ctx, PyrsObjectHandle object_handle, PyrsObjectHandle classinfo_handle)` (`1`/`0` on success, `-1` on error)
- `object_is_subclass(void* module_ctx, PyrsObjectHandle class_handle, PyrsObjectHandle classinfo_handle)` (`1`/`0` on success, `-1` on error)
- `object_get_int(void* module_ctx, PyrsObjectHandle handle, int64_t* out)`
- `object_get_float(void* module_ctx, PyrsObjectHandle handle, double* out)`
- `object_get_bool(void* module_ctx, PyrsObjectHandle handle, int* out)`
- `object_get_bytes(void* module_ctx, PyrsObjectHandle handle, const uint8_t** out_data, uintptr_t* out_len)`
- `object_len(void* module_ctx, PyrsObjectHandle handle, uintptr_t* out_len)`
- `object_get_item(void* module_ctx, PyrsObjectHandle object_handle, PyrsObjectHandle key_handle, PyrsObjectHandle* out_handle)`
- `object_set_item(void* module_ctx, PyrsObjectHandle object_handle, PyrsObjectHandle key_handle, PyrsObjectHandle value_handle)`
- `object_del_item(void* module_ctx, PyrsObjectHandle object_handle, PyrsObjectHandle key_handle)`
- `object_contains(void* module_ctx, PyrsObjectHandle object_handle, PyrsObjectHandle needle_handle)` (`1`/`0` on success, `-1` on error)
- `object_dict_keys(void* module_ctx, PyrsObjectHandle dict_handle, PyrsObjectHandle* out_handle)`
- `object_dict_items(void* module_ctx, PyrsObjectHandle dict_handle, PyrsObjectHandle* out_handle)`
- `object_get_buffer(void* module_ctx, PyrsObjectHandle object_handle, PyrsBufferViewV1* out_view)`
- `object_release_buffer(void* module_ctx, PyrsObjectHandle object_handle)`
- `capsule_new(void* module_ctx, void* pointer, const char* name)`
- `capsule_get_pointer(void* module_ctx, PyrsObjectHandle capsule_handle, const char* name)`
- `capsule_get_name(void* module_ctx, PyrsObjectHandle capsule_handle)`
- `object_sequence_len(void* module_ctx, PyrsObjectHandle handle, uintptr_t* out_len)`
- `object_sequence_get_item(void* module_ctx, PyrsObjectHandle handle, uintptr_t index, PyrsObjectHandle* out_handle)`
- `object_get_iter(void* module_ctx, PyrsObjectHandle handle, PyrsObjectHandle* out_handle)`
- `object_iter_next(void* module_ctx, PyrsObjectHandle iter_handle, PyrsObjectHandle* out_handle)` (`1` yielded, `0` exhausted, `-1` error)
- `object_list_append(void* module_ctx, PyrsObjectHandle list_handle, PyrsObjectHandle item_handle)`
- `object_list_set_item(void* module_ctx, PyrsObjectHandle list_handle, uintptr_t index, PyrsObjectHandle item_handle)`
- `object_dict_len(void* module_ctx, PyrsObjectHandle handle, uintptr_t* out_len)`
- `object_dict_set_item(void* module_ctx, PyrsObjectHandle dict_handle, PyrsObjectHandle key_handle, PyrsObjectHandle value_handle)`
- `object_dict_get_item(void* module_ctx, PyrsObjectHandle dict_handle, PyrsObjectHandle key_handle, PyrsObjectHandle* out_handle)`
- `object_dict_contains(void* module_ctx, PyrsObjectHandle dict_handle, PyrsObjectHandle key_handle)` (`1`/`0` on success, `-1` on error)
- `object_dict_del_item(void* module_ctx, PyrsObjectHandle dict_handle, PyrsObjectHandle key_handle)`
- `object_get_attr(void* module_ctx, PyrsObjectHandle object_handle, const char* attr_name, PyrsObjectHandle* out_handle)`
- `object_set_attr(void* module_ctx, PyrsObjectHandle object_handle, const char* attr_name, PyrsObjectHandle value_handle)`
- `object_del_attr(void* module_ctx, PyrsObjectHandle object_handle, const char* attr_name)`
- `object_has_attr(void* module_ctx, PyrsObjectHandle object_handle, const char* attr_name)` (`1`/`0` on success, `-1` on error)
- `object_call_noargs(void* module_ctx, PyrsObjectHandle callable_handle, PyrsObjectHandle* out_handle)`
- `object_call_onearg(void* module_ctx, PyrsObjectHandle callable_handle, PyrsObjectHandle arg_handle, PyrsObjectHandle* out_handle)`
- `object_call(void* module_ctx, PyrsObjectHandle callable_handle, uintptr_t argc, const PyrsObjectHandle* argv, uintptr_t kwargc, const char* const* kwarg_names, const PyrsObjectHandle* kwarg_values, PyrsObjectHandle* out_handle)`
- `object_get_string(void* module_ctx, PyrsObjectHandle handle)`
- `error_set(void* module_ctx, const char* message)`
- `error_get_message(void* module_ctx)` (null when no error is set)
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
- extension code can re-read module globals as handles via `module_get_object(...)`.
- extension code can import modules during init/call paths via `module_import(...)`.
- extension code can load/mutate module attributes via `module_get_attr(...)`, `module_set_attr(...)`, `module_del_attr(...)`, and `module_has_attr(...)`.
- extension code can perform type relation checks via `object_is_instance(...)` and `object_is_subclass(...)`.
- generic length/subscript helpers are available through `object_len(...)`, `object_get_item(...)`, `object_set_item(...)`, and `object_del_item(...)`.
- generic membership probes are available through `object_contains(...)`.
- dict-view helpers are available through `object_dict_keys(...)` and `object_dict_items(...)`.
- buffer access helpers are available through `object_get_buffer(...)` and `object_release_buffer(...)`.
- current buffer helper coverage is `bytes`/`bytearray`/`memoryview` handles with pointer+length+readonly metadata.
- capsule helpers are available through `capsule_new(...)`, `capsule_get_pointer(...)`, and `capsule_get_name(...)`.
- capsule handles are C-API-only handles (not Python object values) and follow handle `incref`/`decref` semantics.
- iterator helpers are available through `object_get_iter(...)` and `object_iter_next(...)`.
- extension error state set via `error_set(...)` is propagated into import-time runtime errors.
- callable registration via `module_add_function(...)` is supported for positional-only callbacks.
- callable registration via `module_add_function_kw(...)` is supported for callbacks receiving keyword-name/value arrays.
- list/dict mutation through handles is available (`object_list_append`, `object_list_set_item`, `object_dict_contains`, `object_dict_del_item`).
- handle-based object attribute access/mutation is available (`object_get_attr`, `object_set_attr`, `object_del_attr`, `object_has_attr`).
- extension callbacks can invoke Python callables through `object_call(...)` using handle-based positional/keyword argument arrays.
- call fast paths are available for common forms (`object_call_noargs`, `object_call_onearg`).
- `object_call*` now returns explicit non-callable errors and surfaces active Python exceptions via runtime error text.
- last error text can be queried via `error_get_message(...)` before `error_clear(...)`.
- capability checks are available via `api_has_capability(...)` for runtime feature probing.
- ABI mismatch must be handled by extension code and reflected via non-zero return.

## Out of Scope (not yet implemented)

- General `PyObject` constructors/surfaces beyond int/bool/string and module-global assignment path.
- Full CPython-style argument parsing helpers/signature metadata for native callables.
- Thread/GIL APIs.
- Multi-phase module lifecycle APIs.

These are tracked in `/Users/$USER/pyrs/docs/EXTENSION_CAPABILITY_MATRIX.md`.

## Smoke Coverage Map

| Surface Group | Primary Smoke Evidence |
|---|---|
| module setters/getters/import/attr-load | `dynamic_extension_can_set_module_values_via_object_handles`, `dynamic_extension_can_import_module_and_export_attribute`, `dynamic_extension_mixed_surface_roundtrip` |
| handle constructors + typed getters | `dynamic_extension_can_set_module_values_via_object_handles` |
| module attr mutation helpers (`set`/`del`/`has`) | `dynamic_extension_can_set_module_attrs_and_items` |
| generic len/item helpers (`get`/`set`/`del`) | `dynamic_extension_can_use_len_and_getitem_apis`, `dynamic_extension_can_set_module_attrs_and_items`, `dynamic_extension_item_mutation_falls_back_to_special_methods` |
| membership + dict-view helpers (`contains`/`dict_keys`/`dict_items`) | `dynamic_extension_can_use_contains_and_dict_view_apis` |
| buffer helpers (`get_buffer`/`release_buffer`) | `dynamic_extension_can_use_buffer_apis`, `dynamic_extension_buffer_api_handles_memoryview_slices_and_release` |
| capsule helpers (`capsule_new`/`capsule_get_pointer`/`capsule_get_name`) | `dynamic_extension_can_use_capsule_apis` |
| iterator helpers (`get_iter`/`iter_next`) | `dynamic_extension_can_iterate_with_iterator_apis` |
| list/dict sequence+mapping mutation | `dynamic_extension_can_set_module_values_via_object_handles`, `dynamic_extension_mixed_surface_roundtrip` |
| object attribute helpers (`get`/`set`/`del`/`has`) | `dynamic_extension_can_get_set_and_del_object_attributes` |
| callable invocation (`object_call`, fast helpers) | `dynamic_extension_can_call_python_callable_handles`, `dynamic_extension_can_use_object_call_fastpaths`, `dynamic_extension_mixed_surface_roundtrip` |
| type relation checks (`isinstance`/`issubclass`) | `dynamic_extension_can_check_isinstance_and_issubclass`, `dynamic_extension_mixed_surface_roundtrip` |
| error state + message retrieval | `dynamic_extension_error_state_is_propagated_to_import_failure`, `dynamic_extension_can_read_and_clear_error_message` |
| invalid handle/error resilience | `dynamic_extension_invalid_handles_report_errors_consistently` |
| capability introspection | `dynamic_extension_can_query_capabilities` |
