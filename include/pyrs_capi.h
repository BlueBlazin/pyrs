#ifndef PYRS_CAPI_H
#define PYRS_CAPI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define PYRS_CAPI_ABI_VERSION 1u

typedef uint64_t PyrsObjectHandle;

typedef struct PyrsApiV1 {
    uint32_t abi_version;

    int (*module_set_int)(void* module_ctx, const char* name, int64_t value);
    int (*module_set_bool)(void* module_ctx, const char* name, int value);
    int (*module_set_string)(void* module_ctx, const char* name, const char* value);

    PyrsObjectHandle (*object_new_int)(void* module_ctx, int64_t value);
    PyrsObjectHandle (*object_new_bool)(void* module_ctx, int value);
    PyrsObjectHandle (*object_new_string)(void* module_ctx, const char* value);

    int (*object_incref)(void* module_ctx, PyrsObjectHandle handle);
    int (*object_decref)(void* module_ctx, PyrsObjectHandle handle);
    int (*module_set_object)(void* module_ctx, const char* name, PyrsObjectHandle handle);

    int (*error_set)(void* module_ctx, const char* message);
    int (*error_clear)(void* module_ctx);
    int (*error_occurred)(void* module_ctx);
} PyrsApiV1;

typedef int (*PyrsExtensionInitV1)(const PyrsApiV1* api, void* module_ctx);

#ifdef __cplusplus
}
#endif

#endif
