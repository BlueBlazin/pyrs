#ifndef PYRS_CAPI_H
#define PYRS_CAPI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define PYRS_CAPI_ABI_VERSION 1u
#define PYRS_TYPE_NONE 1
#define PYRS_TYPE_BOOL 2
#define PYRS_TYPE_INT 3
#define PYRS_TYPE_STR 4
#define PYRS_TYPE_FLOAT 5
#define PYRS_TYPE_BYTES 6
#define PYRS_TYPE_TUPLE 7
#define PYRS_TYPE_LIST 8
#define PYRS_TYPE_DICT 9

typedef uint64_t PyrsObjectHandle;
typedef struct PyrsApiV1 PyrsApiV1;

typedef int (*PyrsCFunctionV1)(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
);
typedef int (*PyrsCFunctionKwV1)(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    uintptr_t kwargc,
    const char* const* kwarg_names,
    const PyrsObjectHandle* kwarg_values,
    PyrsObjectHandle* result
);

struct PyrsApiV1 {
    uint32_t abi_version;
    int (*api_has_capability)(void* module_ctx, const char* name);

    int (*module_set_int)(void* module_ctx, const char* name, int64_t value);
    int (*module_set_bool)(void* module_ctx, const char* name, int value);
    int (*module_set_string)(void* module_ctx, const char* name, const char* value);
    int (*module_add_function)(void* module_ctx, const char* name, PyrsCFunctionV1 callback);
    int (*module_add_function_kw)(void* module_ctx, const char* name, PyrsCFunctionKwV1 callback);

    PyrsObjectHandle (*object_new_int)(void* module_ctx, int64_t value);
    PyrsObjectHandle (*object_new_none)(void* module_ctx);
    PyrsObjectHandle (*object_new_bool)(void* module_ctx, int value);
    PyrsObjectHandle (*object_new_float)(void* module_ctx, double value);
    PyrsObjectHandle (*object_new_bytes)(void* module_ctx, const uint8_t* data, uintptr_t len);
    PyrsObjectHandle (*object_new_tuple)(void* module_ctx, uintptr_t len, const PyrsObjectHandle* items);
    PyrsObjectHandle (*object_new_list)(void* module_ctx, uintptr_t len, const PyrsObjectHandle* items);
    PyrsObjectHandle (*object_new_dict)(void* module_ctx);
    PyrsObjectHandle (*object_new_string)(void* module_ctx, const char* value);

    int (*object_incref)(void* module_ctx, PyrsObjectHandle handle);
    int (*object_decref)(void* module_ctx, PyrsObjectHandle handle);
    int (*module_set_object)(void* module_ctx, const char* name, PyrsObjectHandle handle);
    int (*module_get_object)(void* module_ctx, const char* name, PyrsObjectHandle* out_handle);
    int (*module_import)(void* module_ctx, const char* module_name, PyrsObjectHandle* out_handle);
    int (*module_get_attr)(void* module_ctx, PyrsObjectHandle module_handle, const char* attr_name, PyrsObjectHandle* out_handle);
    int (*object_type)(void* module_ctx, PyrsObjectHandle handle);
    int (*object_is_instance)(void* module_ctx, PyrsObjectHandle object_handle, PyrsObjectHandle classinfo_handle);
    int (*object_is_subclass)(void* module_ctx, PyrsObjectHandle class_handle, PyrsObjectHandle classinfo_handle);
    int (*object_get_int)(void* module_ctx, PyrsObjectHandle handle, int64_t* out);
    int (*object_get_float)(void* module_ctx, PyrsObjectHandle handle, double* out);
    int (*object_get_bool)(void* module_ctx, PyrsObjectHandle handle, int* out);
    int (*object_get_bytes)(void* module_ctx, PyrsObjectHandle handle, const uint8_t** out_data, uintptr_t* out_len);
    int (*object_sequence_len)(void* module_ctx, PyrsObjectHandle handle, uintptr_t* out_len);
    int (*object_sequence_get_item)(void* module_ctx, PyrsObjectHandle handle, uintptr_t index, PyrsObjectHandle* out_handle);
    int (*object_get_iter)(void* module_ctx, PyrsObjectHandle handle, PyrsObjectHandle* out_handle);
    int (*object_iter_next)(void* module_ctx, PyrsObjectHandle iter_handle, PyrsObjectHandle* out_handle);
    int (*object_list_append)(void* module_ctx, PyrsObjectHandle list_handle, PyrsObjectHandle item_handle);
    int (*object_list_set_item)(void* module_ctx, PyrsObjectHandle list_handle, uintptr_t index, PyrsObjectHandle item_handle);
    int (*object_dict_len)(void* module_ctx, PyrsObjectHandle handle, uintptr_t* out_len);
    int (*object_dict_set_item)(void* module_ctx, PyrsObjectHandle dict_handle, PyrsObjectHandle key_handle, PyrsObjectHandle value_handle);
    int (*object_dict_get_item)(void* module_ctx, PyrsObjectHandle dict_handle, PyrsObjectHandle key_handle, PyrsObjectHandle* out_handle);
    int (*object_dict_contains)(void* module_ctx, PyrsObjectHandle dict_handle, PyrsObjectHandle key_handle);
    int (*object_dict_del_item)(void* module_ctx, PyrsObjectHandle dict_handle, PyrsObjectHandle key_handle);
    int (*object_get_attr)(void* module_ctx, PyrsObjectHandle object_handle, const char* attr_name, PyrsObjectHandle* out_handle);
    int (*object_set_attr)(void* module_ctx, PyrsObjectHandle object_handle, const char* attr_name, PyrsObjectHandle value_handle);
    int (*object_del_attr)(void* module_ctx, PyrsObjectHandle object_handle, const char* attr_name);
    int (*object_has_attr)(void* module_ctx, PyrsObjectHandle object_handle, const char* attr_name);
    int (*object_call_noargs)(void* module_ctx, PyrsObjectHandle callable_handle, PyrsObjectHandle* out_handle);
    int (*object_call_onearg)(void* module_ctx, PyrsObjectHandle callable_handle, PyrsObjectHandle arg_handle, PyrsObjectHandle* out_handle);
    int (*object_call)(
        void* module_ctx,
        PyrsObjectHandle callable_handle,
        uintptr_t argc,
        const PyrsObjectHandle* argv,
        uintptr_t kwargc,
        const char* const* kwarg_names,
        const PyrsObjectHandle* kwarg_values,
        PyrsObjectHandle* out_handle
    );
    const char* (*object_get_string)(void* module_ctx, PyrsObjectHandle handle);

    int (*error_set)(void* module_ctx, const char* message);
    const char* (*error_get_message)(void* module_ctx);
    int (*error_clear)(void* module_ctx);
    int (*error_occurred)(void* module_ctx);
};

typedef int (*PyrsExtensionInitV1)(const PyrsApiV1* api, void* module_ctx);

#ifdef __cplusplus
}
#endif

#endif
