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

    int (*module_set_int)(void* module_ctx, const char* name, int64_t value);
    int (*module_set_bool)(void* module_ctx, const char* name, int value);
    int (*module_set_string)(void* module_ctx, const char* name, const char* value);
    int (*module_add_function)(void* module_ctx, const char* name, PyrsCFunctionV1 callback);
    int (*module_add_function_kw)(void* module_ctx, const char* name, PyrsCFunctionKwV1 callback);

    PyrsObjectHandle (*object_new_int)(void* module_ctx, int64_t value);
    PyrsObjectHandle (*object_new_bool)(void* module_ctx, int value);
    PyrsObjectHandle (*object_new_string)(void* module_ctx, const char* value);

    int (*object_incref)(void* module_ctx, PyrsObjectHandle handle);
    int (*object_decref)(void* module_ctx, PyrsObjectHandle handle);
    int (*module_set_object)(void* module_ctx, const char* name, PyrsObjectHandle handle);
    int (*object_type)(void* module_ctx, PyrsObjectHandle handle);
    int (*object_get_int)(void* module_ctx, PyrsObjectHandle handle, int64_t* out);
    int (*object_get_bool)(void* module_ctx, PyrsObjectHandle handle, int* out);
    const char* (*object_get_string)(void* module_ctx, PyrsObjectHandle handle);

    int (*error_set)(void* module_ctx, const char* message);
    int (*error_clear)(void* module_ctx);
    int (*error_occurred)(void* module_ctx);
};

typedef int (*PyrsExtensionInitV1)(const PyrsApiV1* api, void* module_ctx);

#ifdef __cplusplus
}
#endif

#endif
