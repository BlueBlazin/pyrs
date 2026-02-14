#ifndef PYRS_CAPI_H
#define PYRS_CAPI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define PYRS_CAPI_ABI_VERSION 1u

typedef struct PyrsApiV1 {
    uint32_t abi_version;
    int (*module_set_int)(void* module_ctx, const char* name, int64_t value);
    int (*module_set_bool)(void* module_ctx, const char* name, int value);
    int (*module_set_string)(void* module_ctx, const char* name, const char* value);
} PyrsApiV1;

typedef int (*PyrsExtensionInitV1)(const PyrsApiV1* api, void* module_ctx);

#ifdef __cplusplus
}
#endif

#endif
