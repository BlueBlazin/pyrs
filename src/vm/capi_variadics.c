#if defined(__linux__) && !defined(_GNU_SOURCE)
#define _GNU_SOURCE 1
#endif

#if defined(__unix__) && !defined(__APPLE__) && !defined(_POSIX_C_SOURCE)
#define _POSIX_C_SOURCE 200809L
#endif

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <ctype.h>
#include <math.h>
#include <signal.h>
#include <limits.h>
#if defined(__unix__) || defined(__APPLE__)
#include <dlfcn.h>
#define PYRS_HAVE_DLADDR 1
#endif
#include <wchar.h>
#include <sys/types.h>
#include <time.h>
#include <errno.h>
#include <locale.h>
#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#endif

#if defined(_WIN32) && defined(_MSC_VER)
#pragma section(".CRT$XCU", read)
#define PYRS_CONSTRUCTOR_FUNC(func_name) \
    static void __cdecl func_name(void); \
    __declspec(allocate(".CRT$XCU")) void (__cdecl *func_name##_init_)(void) = func_name; \
    static void __cdecl func_name(void)
#else
#define PYRS_CONSTRUCTOR_FUNC(func_name) static void __attribute__((constructor)) func_name(void)
#endif

static int pyrs_clock_gettime_monotonic(struct timespec *ts);
static int pyrs_clock_gettime_realtime(struct timespec *ts);

typedef intptr_t Py_ssize_t;
typedef intptr_t Py_hash_t;
typedef int64_t PyTime_t;
typedef uintptr_t Py_uhash_t;
typedef struct {
    double real;
    double imag;
} Py_complex;
typedef void (*PyOS_sighandler_t)(int);
typedef struct {
    Py_ssize_t ob_refcnt;
    void *ob_type;
} PyObject;
typedef struct {
    const char *name;
    int member_type;
    Py_ssize_t offset;
    int flags;
    const char *doc;
} PyMemberDef;
typedef struct {
    int slot;
    void *pfunc;
} PyType_Slot;
typedef struct {
    const char *name;
    int basicsize;
    int itemsize;
    unsigned int flags;
    PyType_Slot *slots;
} PyType_Spec;

typedef struct pyrs_pytypeobject PyTypeObject;

#define Py_TPFLAGS_DEFAULT 0u
#define Py_TPFLAGS_BASETYPE (1u << 10)

#define Py_tp_new 65
#define Py_tp_members 72

#define Py_READONLY 1

#define Py_T_SHORT 0
#define Py_T_INT 1
#define Py_T_LONG 2
#define Py_T_FLOAT 3
#define Py_T_DOUBLE 4
#define Py_T_CHAR 7
#define Py_T_BYTE 8
#define Py_T_UBYTE 9
#define Py_T_USHORT 10
#define Py_T_UINT 11
#define Py_T_ULONG 12
#define Py_T_STRING_INPLACE 13
#define Py_T_BOOL 14
#define Py_T_LONGLONG 17
#define Py_T_ULONGLONG 18
#define Py_T_PYSSIZET 19

#ifndef PY_SSIZE_T_MAX
#define PY_SSIZE_T_MAX INTPTR_MAX
#endif

extern void *pyrs_capi_tuple_pack_from_array(Py_ssize_t n, void *const *items);
extern void pyrs_capi_set_error_message(const char *message);
extern void pyrs_capi_sys_write_stdout(const char *text);
extern void pyrs_capi_sys_write_stderr(const char *text);
extern int pyrs_capi_sys_audit_noargs(const char *event);
extern int pyrs_capi_sys_audit_object(const char *event, void *args);
extern void *Py_VaBuildValue(const char *format, va_list ap);

extern void *PyTuple_New(Py_ssize_t size);
extern int PyTuple_SetItem(void *tuple, Py_ssize_t index, void *item);
extern Py_ssize_t PyTuple_Size(void *tuple);
extern void *PyTuple_GetItem(void *tuple, Py_ssize_t index);
extern int PyTuple_Resize(void **tuple, Py_ssize_t size);
extern void *PyList_New(Py_ssize_t size);
extern Py_ssize_t PyList_Size(void *list);
extern void *PyList_GetItem(void *list, Py_ssize_t index);
extern int PyList_Append(void *list, void *item);
extern int PyList_Extend(void *list, void *iterable);
extern void *PyDict_New(void);
extern Py_ssize_t PyDict_Size(void *dict);
extern void *PyDict_GetItem(void *dict, void *key);
extern void *PyDict_Keys(void *dict);
extern int PyDict_SetItem(void *dict, void *key, void *value);
extern void *PyDict_GetItemString(void *dict, const char *key);
extern void *PyLong_FromLong(long value);
extern void *PyLong_FromUnsignedLong(unsigned long value);
extern void *PyLong_FromLongLong(long long value);
extern void *PyLong_FromUnsignedLongLong(unsigned long long value);
extern void *PyLong_FromSsize_t(Py_ssize_t value);
extern long PyLong_AsLong(void *value);
extern unsigned long PyLong_AsUnsignedLong(void *value);
extern unsigned long PyLong_AsUnsignedLongMask(void *value);
extern unsigned long long PyLong_AsUnsignedLongLong(void *value);
extern unsigned long long PyLong_AsUnsignedLongLongMask(void *value);
extern long long PyLong_AsLongLong(void *value);
extern Py_ssize_t PyLong_AsSsize_t(void *value);
extern Py_ssize_t PyLong_AsNativeBytes(void *v, void *buffer, Py_ssize_t n_bytes, int flags);
extern void *PyLong_FromNativeBytes(const void *buffer, size_t n_bytes, int flags);
extern void *PyFloat_FromDouble(double value);
extern double PyFloat_AsDouble(void *value);
extern Py_complex PyComplex_AsCComplex(void *value);
extern void *PyBool_FromLong(long value);
extern void *PyUnicode_FromString(const char *value);
extern void *PyUnicode_FromStringAndSize(const char *value, Py_ssize_t size);
extern void *PyUnicode_FromWideChar(const wchar_t *value, Py_ssize_t len);
extern const char *PyUnicode_AsUTF8AndSize(void *object, Py_ssize_t *size);
extern void *PyUnicode_AsUTF8String(void *object);
extern void *PyUnicode_AsEncodedString(void *unicode, const char *encoding, const char *errors);
extern const char *PyUnicode_GetDefaultEncoding(void);
extern void *PyUnicode_FromOrdinal(int ordinal);
extern void *PyUnicode_Concat(void *left, void *right);
extern Py_ssize_t PyUnicode_GetLength(void *object);
extern void *PyUnicode_Substring(void *object, Py_ssize_t start, Py_ssize_t end);
extern int PyBytes_AsStringAndSize(void *obj, char **buffer, Py_ssize_t *len);
extern void *PyBytes_FromStringAndSize(const char *value, Py_ssize_t size);
extern Py_ssize_t PyByteArray_Size(void *object);
extern char *PyByteArray_AsString(void *object);
extern void *PyImport_ImportModule(const char *name);
extern void *PyObject_Call(void *callable, void *args, void *kwargs);
extern void *PyObject_CallObject(void *callable, void *args);
extern void *PyObject_CallMethod(void *object, const char *name, const char *format, ...);
extern void *PyObject_GetAttr(void *object, void *name);
extern void *PyObject_GetAttrString(void *object, const char *name);
extern int PyObject_GetBuffer(void *object, void *view, int flags);
extern void *PyObject_Type(void *object);
extern void *PyObject_GenericHash(void *object);
extern void *_PyType_Lookup(void *type, void *name);
extern void *PyObject_Str(void *object);
extern void *PyObject_Repr(void *object);
extern void *PyObject_ASCII(void *object);
extern Py_hash_t PyObject_Hash(void *object);
extern int PyObject_IsTrue(void *object);
extern int PyType_IsSubtype(void *subtype, void *type);
extern void *PyType_FromModuleAndSpec(void *module, void *spec, void *bases);
extern int PyModule_AddType(void *module, void *type_obj);
extern void *PyType_GenericAlloc(void *subtype, Py_ssize_t nitems);
extern int PyErr_ExceptionMatches(void *exception);
extern const char *PyUnicode_AsUTF8(void *object);
extern void *PyThreadState_GetUnchecked(void);
extern void *PyThreadState_Get(void);
extern void *PyNumber_Index(void *object);
extern Py_ssize_t PyNumber_AsSsize_t(void *object, void *exception);
extern void *PyNumber_ToBase(void *object, int base);
extern void *PyNumber_Absolute(void *object);
extern void *PyNumber_Remainder(void *left, void *right);
extern void *PyNumber_Negative(void *object);
extern int PyTime_PerfCounterRaw(PyTime_t *result);
extern void PyErr_BadInternalCall(void);
extern void *PyErr_Occurred(void);
extern void PyErr_SetString(void *exception, const char *message);
extern void PyErr_SetObject(void *exception, void *value);
extern void *PyErr_GetRaisedException(void);
extern void PyErr_SetRaisedException(void *exception);
extern void PyException_SetContext(void *exception, void *context);
extern void *PyErr_NoMemory(void);
extern void PyErr_Clear(void);
extern int PyErr_WarnEx(void *category, const char *message, int stacklevel);
extern int PyErr_WarnFormat(void *category, int stacklevel, const char *format, ...);
extern void PyErr_WriteUnraisable(void *object);
extern void Py_IncRef(void *object);
extern void Py_DecRef(void *object);
extern int PyUnicode_FSConverter(void *arg, void *addr);
extern int PyUnicode_EqualToUTF8(void *unicode, const char *text);
extern Py_ssize_t PySequence_Size(void *object);
extern void *PySequence_GetItem(void *object, Py_ssize_t index);
extern void *PySequence_Tuple(void *object);
extern void *PySequence_Fast(void *object, const char *message);
extern int PyBuffer_FillInfo(void *view, void *object, void *buf, Py_ssize_t len, int readonly, int flags);
extern int PyBuffer_IsContiguous(const void *view, char order);
extern void PyBuffer_Release(void *view);
extern void *PyMem_Malloc(size_t size);
extern void PyMem_Free(void *ptr);
extern char _Py_NoneStruct;

// Some lightweight test binaries don't pull every Rust-exported C-API symbol
// into the final link. Provide weak fallbacks so capi_variadics can still link;
// strong Rust exports override these when present.
__attribute__((weak)) void pyrs_capi_set_error_message(const char *message)
{
    (void)message;
}

__attribute__((weak)) void *pyrs_capi_pyerr_format_fallback(void *exception, const char *format)
{
    (void)exception;
    if (format != NULL) {
        pyrs_capi_set_error_message(format);
    } else {
        pyrs_capi_set_error_message("error");
    }
    return NULL;
}

__attribute__((weak)) void *pyrs_capi_pyerr_formatv_fallback(void *exception, const char *format, void *vargs)
{
    (void)vargs;
    return pyrs_capi_pyerr_format_fallback(exception, format);
}

__attribute__((weak)) void *PyObject_Str(void *object)
{
    (void)object;
    return NULL;
}

__attribute__((weak)) void *PyObject_Repr(void *object)
{
    (void)object;
    return NULL;
}

__attribute__((weak)) void *PyObject_ASCII(void *object)
{
    (void)object;
    return NULL;
}

__attribute__((weak)) const char *PyUnicode_AsUTF8(void *object)
{
    (void)object;
    return NULL;
}

__attribute__((weak)) void *PyUnicode_FromWideChar(const wchar_t *value, Py_ssize_t len)
{
    (void)value;
    (void)len;
    return NULL;
}

__attribute__((weak)) void Py_DecRef(void *object)
{
    (void)object;
}
extern char PyList_Type;
extern char PyDict_Type;
extern char PyTuple_Type;
extern char PyUnicode_Type;
extern char PyBytes_Type;
extern char PyByteArray_Type;
extern char PyLong_Type;
extern char PyExc_TypeError;
extern char PyExc_OverflowError;
extern char PyExc_AttributeError;
extern char PyExc_AssertionError;
extern char PyExc_BufferError;
extern char PyExc_ValueError;
extern char PyExc_SystemError;
extern char PyExc_DeprecationWarning;

static void pyrs_sys_vwrite(void (*sink)(const char *), const char *format, va_list ap)
{
    char stack_buf[4096];
    va_list copy;
    va_copy(copy, ap);
    int needed = vsnprintf(stack_buf, sizeof(stack_buf), format ? format : "", copy);
    va_end(copy);

    if (needed < 0) {
        sink("");
        return;
    }
    if ((size_t)needed < sizeof(stack_buf)) {
        sink(stack_buf);
        return;
    }

    size_t dynamic_len = (size_t)needed + 1;
    char *dynamic_buf = (char *)malloc(dynamic_len);
    if (dynamic_buf == NULL) {
        sink("");
        return;
    }
    vsnprintf(dynamic_buf, dynamic_len, format ? format : "", ap);
    sink(dynamic_buf);
    free(dynamic_buf);
}

void PySys_WriteStdout(const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    pyrs_sys_vwrite(pyrs_capi_sys_write_stdout, format, ap);
    va_end(ap);
}

void PySys_WriteStderr(const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    pyrs_sys_vwrite(pyrs_capi_sys_write_stderr, format, ap);
    va_end(ap);
}

void PySys_FormatStdout(const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    pyrs_sys_vwrite(pyrs_capi_sys_write_stdout, format, ap);
    va_end(ap);
}

void PySys_FormatStderr(const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    pyrs_sys_vwrite(pyrs_capi_sys_write_stderr, format, ap);
    va_end(ap);
}

int PySys_Audit(const char *event, const char *format, ...)
{
    if (format == NULL || format[0] == '\0') {
        return pyrs_capi_sys_audit_noargs(event);
    }

    va_list ap;
    va_start(ap, format);
    void *args = Py_VaBuildValue(format, ap);
    va_end(ap);
    if (args == NULL) {
        return -1;
    }
    int result = pyrs_capi_sys_audit_object(event, args);
    Py_DecRef(args);
    return result;
}

typedef struct {
    char *data;
    size_t len;
    size_t cap;
} PyrsFormatBuffer;

static int pyrs_format_buffer_ensure(PyrsFormatBuffer *buf, size_t additional)
{
    size_t needed = buf->len + additional + 1;
    if (needed <= buf->cap) {
        return 1;
    }
    size_t next_cap = buf->cap == 0 ? 128 : buf->cap;
    while (next_cap < needed) {
        next_cap *= 2;
    }
    char *next = (char *)realloc(buf->data, next_cap);
    if (next == NULL) {
        return 0;
    }
    buf->data = next;
    buf->cap = next_cap;
    return 1;
}

static int pyrs_format_buffer_append_bytes(PyrsFormatBuffer *buf, const char *text, size_t len)
{
    if (!pyrs_format_buffer_ensure(buf, len)) {
        return 0;
    }
    if (len > 0) {
        memcpy(buf->data + buf->len, text, len);
        buf->len += len;
    }
    buf->data[buf->len] = '\0';
    return 1;
}

static int pyrs_format_buffer_append_cstr(PyrsFormatBuffer *buf, const char *text)
{
    if (text == NULL) {
        text = "(null)";
    }
    return pyrs_format_buffer_append_bytes(buf, text, strlen(text));
}

static int pyrs_format_buffer_append_object_text(PyrsFormatBuffer *buf, void *object, char format_code)
{
    if (object == NULL) {
        return pyrs_format_buffer_append_cstr(buf, "(null)");
    }
    void *rendered = NULL;
    if (format_code == 'R') {
        rendered = PyObject_Repr(object);
    } else if (format_code == 'A') {
        rendered = PyObject_ASCII(object);
    } else {
        rendered = PyObject_Str(object);
    }
    if (rendered == NULL) {
        return pyrs_format_buffer_append_cstr(buf, "<object>");
    }
    const char *utf8 = PyUnicode_AsUTF8(rendered);
    int ok = pyrs_format_buffer_append_cstr(buf, utf8 != NULL ? utf8 : "<object>");
    Py_DecRef(rendered);
    return ok;
}

static int pyrs_format_buffer_append_unicode_text(PyrsFormatBuffer *buf, void *unicode_obj)
{
    if (unicode_obj == NULL) {
        return pyrs_format_buffer_append_cstr(buf, "(null)");
    }
    const char *utf8 = PyUnicode_AsUTF8(unicode_obj);
    if (utf8 != NULL) {
        return pyrs_format_buffer_append_cstr(buf, utf8);
    }
    return pyrs_format_buffer_append_object_text(buf, unicode_obj, 'S');
}

static int pyrs_format_buffer_append_wchar_text(PyrsFormatBuffer *buf, const wchar_t *text)
{
    if (text == NULL) {
        return pyrs_format_buffer_append_cstr(buf, "(null)");
    }
    void *unicode_obj = PyUnicode_FromWideChar(text, -1);
    if (unicode_obj == NULL) {
        return 0;
    }
    const char *utf8 = PyUnicode_AsUTF8(unicode_obj);
    int ok = pyrs_format_buffer_append_cstr(buf, utf8 != NULL ? utf8 : "(null)");
    Py_DecRef(unicode_obj);
    return ok;
}

static char *pyrs_format_pyerr_message(const char *format, va_list ap)
{
    PyrsFormatBuffer buf = {0};
    const char *cursor = format != NULL ? format : "error";
    while (*cursor != '\0') {
        if (*cursor != '%') {
            if (!pyrs_format_buffer_append_bytes(&buf, cursor, 1)) {
                free(buf.data);
                return NULL;
            }
            cursor++;
            continue;
        }
        cursor++;
        if (*cursor == '\0') {
            if (!pyrs_format_buffer_append_bytes(&buf, "%", 1)) {
                free(buf.data);
                return NULL;
            }
            break;
        }
        if (*cursor == '%') {
            if (!pyrs_format_buffer_append_bytes(&buf, "%", 1)) {
                free(buf.data);
                return NULL;
            }
            cursor++;
            continue;
        }

        while (*cursor == '-' || *cursor == '+' || *cursor == ' ' || *cursor == '#' || *cursor == '0') {
            cursor++;
        }
        while (isdigit((unsigned char)*cursor)) {
            cursor++;
        }
        if (*cursor == '.') {
            cursor++;
            while (isdigit((unsigned char)*cursor)) {
                cursor++;
            }
        }

        int length_l = 0;
        int length_ll = 0;
        int length_z = 0;
        if (*cursor == 'l') {
            length_l = 1;
            cursor++;
            if (*cursor == 'l') {
                length_ll = 1;
                cursor++;
            }
        } else if (*cursor == 'z') {
            length_z = 1;
            cursor++;
        }

        char spec = *cursor;
        if (spec == '\0') {
            break;
        }
        cursor++;

        char num_buf[128];
        int wrote = 0;
        switch (spec) {
        case 's': {
            const char *text = va_arg(ap, const char *);
            if (!pyrs_format_buffer_append_cstr(&buf, text)) {
                free(buf.data);
                return NULL;
            }
            break;
        }
        case 'S':
        case 'R':
        case 'A': {
            void *object = va_arg(ap, void *);
            if (!pyrs_format_buffer_append_object_text(&buf, object, spec)) {
                free(buf.data);
                return NULL;
            }
            break;
        }
        case 'U': {
            void *unicode_obj = va_arg(ap, void *);
            if (!pyrs_format_buffer_append_unicode_text(&buf, unicode_obj)) {
                free(buf.data);
                return NULL;
            }
            break;
        }
        case 'V': {
            void *unicode_obj = va_arg(ap, void *);
            if (length_l) {
                const wchar_t *fallback = va_arg(ap, const wchar_t *);
                if (unicode_obj != NULL) {
                    if (!pyrs_format_buffer_append_unicode_text(&buf, unicode_obj)) {
                        free(buf.data);
                        return NULL;
                    }
                } else if (!pyrs_format_buffer_append_wchar_text(&buf, fallback)) {
                    free(buf.data);
                    return NULL;
                }
            } else {
                const char *fallback = va_arg(ap, const char *);
                if (unicode_obj != NULL) {
                    if (!pyrs_format_buffer_append_unicode_text(&buf, unicode_obj)) {
                        free(buf.data);
                        return NULL;
                    }
                } else if (!pyrs_format_buffer_append_cstr(&buf, fallback)) {
                    free(buf.data);
                    return NULL;
                }
            }
            break;
        }
        case 'c': {
            int ch = va_arg(ap, int);
            char out = (char)ch;
            if (!pyrs_format_buffer_append_bytes(&buf, &out, 1)) {
                free(buf.data);
                return NULL;
            }
            break;
        }
        case 'd':
        case 'i': {
            if (length_ll) {
                long long value = va_arg(ap, long long);
                wrote = snprintf(num_buf, sizeof(num_buf), "%lld", value);
            } else if (length_l) {
                long value = va_arg(ap, long);
                wrote = snprintf(num_buf, sizeof(num_buf), "%ld", value);
            } else if (length_z) {
                Py_ssize_t value = va_arg(ap, Py_ssize_t);
                wrote = snprintf(num_buf, sizeof(num_buf), "%lld", (long long)value);
            } else {
                int value = va_arg(ap, int);
                wrote = snprintf(num_buf, sizeof(num_buf), "%d", value);
            }
            if (wrote < 0 || !pyrs_format_buffer_append_bytes(&buf, num_buf, (size_t)wrote)) {
                free(buf.data);
                return NULL;
            }
            break;
        }
        case 'u':
        case 'x':
        case 'X': {
            if (length_ll) {
                unsigned long long value = va_arg(ap, unsigned long long);
                wrote = snprintf(num_buf, sizeof(num_buf), spec == 'u' ? "%llu" : (spec == 'x' ? "%llx" : "%llX"), value);
            } else if (length_l) {
                unsigned long value = va_arg(ap, unsigned long);
                wrote = snprintf(num_buf, sizeof(num_buf), spec == 'u' ? "%lu" : (spec == 'x' ? "%lx" : "%lX"), value);
            } else if (length_z) {
                size_t value = va_arg(ap, size_t);
                wrote = snprintf(num_buf, sizeof(num_buf), spec == 'u' ? "%zu" : (spec == 'x' ? "%zx" : "%zX"), value);
            } else {
                unsigned int value = va_arg(ap, unsigned int);
                wrote = snprintf(num_buf, sizeof(num_buf), spec == 'u' ? "%u" : (spec == 'x' ? "%x" : "%X"), value);
            }
            if (wrote < 0 || !pyrs_format_buffer_append_bytes(&buf, num_buf, (size_t)wrote)) {
                free(buf.data);
                return NULL;
            }
            break;
        }
        case 'p': {
            void *value = va_arg(ap, void *);
            wrote = snprintf(num_buf, sizeof(num_buf), "%p", value);
            if (wrote < 0 || !pyrs_format_buffer_append_bytes(&buf, num_buf, (size_t)wrote)) {
                free(buf.data);
                return NULL;
            }
            break;
        }
        default: {
            if (!pyrs_format_buffer_append_bytes(&buf, "%", 1) ||
                !pyrs_format_buffer_append_bytes(&buf, &spec, 1)) {
                free(buf.data);
                return NULL;
            }
            break;
        }
        }
    }
    if (buf.data == NULL) {
        buf.data = (char *)malloc(1);
        if (buf.data == NULL) {
            return NULL;
        }
        buf.data[0] = '\0';
    }
    return buf.data;
}

void *PyErr_Format(void *exception, const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    char *message = pyrs_format_pyerr_message(format, ap);
    va_end(ap);
#if defined(PYRS_HAVE_DLADDR) && (defined(__GNUC__) || defined(__clang__))
    if (message != NULL &&
        getenv("PYRS_TRACE_PYERR_FORMAT_CALLER") != NULL &&
        (strstr(message, "not subscriptable") != NULL ||
         strstr(message, "dot() missing required argument") != NULL)) {
        void *ret0 = __builtin_return_address(0);
        Dl_info info0;
        if (dladdr(ret0, &info0) != 0) {
            fprintf(
                stderr,
                "[pyerr-format-caller] msg=%s return0=%p sym0=%s image0=%s\n",
                message,
                ret0,
                info0.dli_sname != NULL ? info0.dli_sname : "<unknown>",
                info0.dli_fname != NULL ? info0.dli_fname : "<unknown>"
            );
        } else {
            fprintf(stderr, "[pyerr-format-caller] msg=%s return0=%p\n", message, ret0);
        }
    }
#endif
    if (message == NULL) {
        return pyrs_capi_pyerr_format_fallback(exception, format);
    }
    void *result = pyrs_capi_pyerr_format_fallback(exception, message);
    free(message);
    return result;
}

void *PyErr_FormatV(void *exception, const char *format, va_list vargs)
{
    va_list ap;
    va_copy(ap, vargs);
    char *message = pyrs_format_pyerr_message(format, ap);
    va_end(ap);
    if (message == NULL) {
        return pyrs_capi_pyerr_formatv_fallback(exception, format, NULL);
    }
    void *result = pyrs_capi_pyerr_formatv_fallback(exception, message, NULL);
    free(message);
    return result;
}

__attribute__((used, visibility("default")))
void *_PyErr_Format(void *tstate, void *exception, const char *format, ...)
{
    (void)tstate;
    va_list ap;
    va_start(ap, format);
    char *message = pyrs_format_pyerr_message(format, ap);
    va_end(ap);
    if (message == NULL) {
        return pyrs_capi_pyerr_format_fallback(exception, format);
    }
    void *result = pyrs_capi_pyerr_format_fallback(exception, message);
    free(message);
    return result;
}

__attribute__((used, visibility("default")))
void *_PyErr_FormatFromCause(void *exception, const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    char *message = pyrs_format_pyerr_message(format, ap);
    va_end(ap);
    if (message == NULL) {
        return pyrs_capi_pyerr_format_fallback(exception, format);
    }
    void *result = pyrs_capi_pyerr_format_fallback(exception, message);
    free(message);
    return result;
}

typedef struct {
    Py_ssize_t ob_refcnt;
    void *ob_type;
} PyObjectHeadCompat;

#define PYBUF_SIMPLE 0
#define PYBUF_WRITABLE 0x0001

typedef struct {
    void *buf;
    void *obj;
    Py_ssize_t len;
    Py_ssize_t itemsize;
    int readonly;
    int ndim;
    char *format;
    Py_ssize_t *shape;
    Py_ssize_t *strides;
    Py_ssize_t *suboffsets;
    void *internal;
} PyrsPyBuffer;

static int is_tuple_object(void *object)
{
    if (object == NULL) {
        return 0;
    }
    PyObjectHeadCompat *head = (PyObjectHeadCompat *)object;
    if (head->ob_type == (void *)&PyTuple_Type) {
        return 1;
    }
    if (head->ob_type == NULL) {
        return 0;
    }
    return PyType_IsSubtype(head->ob_type, (void *)&PyTuple_Type);
}

static void *build_none(void)
{
    void *none = (void *)&_Py_NoneStruct;
    Py_IncRef(none);
    return none;
}

static int object_is_instance_of_type(void *value, void *type_obj)
{
    if (value == NULL || type_obj == NULL) {
        return 0;
    }
    PyObjectHeadCompat *head = (PyObjectHeadCompat *)value;
    if (head->ob_type == type_obj) {
        return 1;
    }
    if (head->ob_type == NULL) {
        return 0;
    }
    return PyType_IsSubtype(head->ob_type, type_obj);
}

void PyOS_BeforeFork(void)
{
}

void PyOS_AfterFork_Parent(void)
{
}

void PyOS_AfterFork_Child(void)
{
}

void PyOS_AfterFork(void)
{
    PyOS_AfterFork_Child();
}

int PyOS_CheckStack(void)
{
    return 0;
}

void *PyOS_FSPath(void *path)
{
    if (path == NULL) {
        pyrs_capi_set_error_message("expected str, bytes or os.PathLike object");
        return NULL;
    }
    if (object_is_instance_of_type(path, (void *)&PyUnicode_Type) ||
        object_is_instance_of_type(path, (void *)&PyBytes_Type)) {
        Py_IncRef(path);
        return path;
    }

    void *fspath = PyObject_GetAttrString(path, "__fspath__");
    if (fspath == NULL) {
        pyrs_capi_set_error_message("expected str, bytes or os.PathLike object");
        return NULL;
    }
    void *args = PyTuple_New(0);
    if (args == NULL) {
        Py_DecRef(fspath);
        return NULL;
    }
    void *out = PyObject_CallObject(fspath, args);
    Py_DecRef(args);
    Py_DecRef(fspath);
    if (out == NULL) {
        return NULL;
    }
    if (!object_is_instance_of_type(out, (void *)&PyUnicode_Type) &&
        !object_is_instance_of_type(out, (void *)&PyBytes_Type)) {
        Py_DecRef(out);
        pyrs_capi_set_error_message("__fspath__() must return str or bytes");
        return NULL;
    }
    return out;
}

int PyOS_InterruptOccurred(void)
{
    return 0;
}

char *PyOS_double_to_string(double val, char format_code, int precision, int flags, int *type)
{
    if (type != NULL) {
        if (isnan(val)) {
            *type = 2;
        }
        else if (isinf(val)) {
            *type = 1;
        }
        else {
            *type = 0;
        }
    }

    if ((flags & 0x08) && val == 0.0) {
        val = 0.0;
    }

    char text_buf[256];
    if (isnan(val)) {
        snprintf(text_buf, sizeof(text_buf), "nan");
    }
    else if (isinf(val)) {
        snprintf(text_buf, sizeof(text_buf), "%sinf", val < 0 ? "-" : "");
    }
    else {
        int p = precision >= 0 ? precision : 6;
        int with_sign = (flags & 0x01) != 0;
        char code = (char)tolower((unsigned char)format_code);
        if (code == 'r') {
            code = 'g';
            p = 17;
        }
        if (code != 'e' && code != 'f' && code != 'g') {
            code = 'g';
        }
        char fmt[24];
        snprintf(fmt, sizeof(fmt), with_sign ? "%%+.%d%c" : "%%.%d%c", p, code);
        snprintf(text_buf, sizeof(text_buf), fmt, val);
        if ((flags & 0x02) && strchr(text_buf, '.') == NULL &&
            strchr(text_buf, 'e') == NULL && strchr(text_buf, 'E') == NULL) {
            size_t used = strlen(text_buf);
            if (used + 2 < sizeof(text_buf)) {
                text_buf[used] = '.';
                text_buf[used + 1] = '0';
                text_buf[used + 2] = '\0';
            }
        }
    }

    size_t n = strlen(text_buf);
    char *out = (char *)malloc(n + 1);
    if (out == NULL) {
        pyrs_capi_set_error_message("PyOS_double_to_string failed allocating output");
        return NULL;
    }
    memcpy(out, text_buf, n + 1);
    return out;
}

PyOS_sighandler_t PyOS_getsig(int sig)
{
    PyOS_sighandler_t old = signal(sig, SIG_IGN);
    if (old != SIG_ERR) {
        signal(sig, old);
    }
    return old;
}

PyOS_sighandler_t PyOS_setsig(int sig, PyOS_sighandler_t handler)
{
    return signal(sig, handler);
}

int PyOS_mystrnicmp(const char *s1, const char *s2, Py_ssize_t size)
{
    const unsigned char *p1, *p2;
    if (size == 0) {
        return 0;
    }
    p1 = (const unsigned char *)s1;
    p2 = (const unsigned char *)s2;
    for (; (--size > 0) && *p1 && *p2 &&
           (tolower(*p1) == tolower(*p2)); p1++, p2++) {
    }
    return tolower(*p1) - tolower(*p2);
}

int PyOS_mystricmp(const char *s1, const char *s2)
{
    const unsigned char *p1 = (const unsigned char *)s1;
    const unsigned char *p2 = (const unsigned char *)s2;
    for (; *p1 && *p2 && (tolower(*p1) == tolower(*p2)); p1++, p2++) {
    }
    return tolower(*p1) - tolower(*p2);
}

int PyOS_vsnprintf(char *str, size_t size, const char *format, va_list va)
{
    if (str == NULL || size == 0) {
        return 0;
    }
    int rc = vsnprintf(str, size, format ? format : "", va);
    str[size - 1] = '\0';
    return rc;
}

typedef struct {
    char *buf;
    Py_ssize_t len;
    Py_ssize_t cap;
} bytes_builder;

static int bytes_builder_init(bytes_builder *builder, Py_ssize_t initial_cap)
{
    if (initial_cap < 64) {
        initial_cap = 64;
    }
    builder->buf = (char *)malloc((size_t)initial_cap);
    if (builder->buf == NULL) {
        pyrs_capi_set_error_message("PyBytes_FromFormatV failed allocating output buffer");
        builder->len = 0;
        builder->cap = 0;
        return 0;
    }
    builder->len = 0;
    builder->cap = initial_cap;
    builder->buf[0] = '\0';
    return 1;
}

static int bytes_builder_reserve(bytes_builder *builder, Py_ssize_t extra)
{
    if (extra < 0) {
        return 0;
    }
    Py_ssize_t needed = builder->len + extra + 1;
    if (needed <= builder->cap) {
        return 1;
    }
    Py_ssize_t next_cap = builder->cap;
    while (next_cap < needed) {
        if (next_cap > (Py_ssize_t)(SIZE_MAX / 2)) {
            next_cap = needed;
            break;
        }
        next_cap *= 2;
    }
    char *grown = (char *)realloc(builder->buf, (size_t)next_cap);
    if (grown == NULL) {
        pyrs_capi_set_error_message("PyBytes_FromFormatV failed growing output buffer");
        return 0;
    }
    builder->buf = grown;
    builder->cap = next_cap;
    return 1;
}

static int bytes_builder_append_bytes(bytes_builder *builder, const char *data, Py_ssize_t len)
{
    if (len <= 0) {
        return 1;
    }
    if (!bytes_builder_reserve(builder, len)) {
        return 0;
    }
    memcpy(builder->buf + builder->len, data, (size_t)len);
    builder->len += len;
    builder->buf[builder->len] = '\0';
    return 1;
}

static int bytes_builder_append_cstr(bytes_builder *builder, const char *data)
{
    if (data == NULL) {
        return bytes_builder_append_bytes(builder, "(null)", 6);
    }
    return bytes_builder_append_bytes(builder, data, (Py_ssize_t)strlen(data));
}

static int bytes_builder_append_char(bytes_builder *builder, unsigned char ch)
{
    if (!bytes_builder_reserve(builder, 1)) {
        return 0;
    }
    builder->buf[builder->len++] = (char)ch;
    builder->buf[builder->len] = '\0';
    return 1;
}

static int bytes_builder_append_object_text(bytes_builder *builder, void *object, char format_code)
{
    if (object == NULL) {
        return bytes_builder_append_cstr(builder, "(null)");
    }
    void *text_obj = NULL;
    if (format_code == 'R') {
        text_obj = PyObject_Repr(object);
    }
    else if (format_code == 'A') {
        text_obj = PyObject_ASCII(object);
    }
    else {
        text_obj = PyObject_Str(object);
    }
    if (text_obj == NULL) {
        return 0;
    }
    const char *utf8 = PyUnicode_AsUTF8(text_obj);
    int ok = bytes_builder_append_cstr(builder, utf8 != NULL ? utf8 : "<object>");
    Py_DecRef(text_obj);
    return ok;
}

static int bytes_builder_append_unicode_text(bytes_builder *builder, void *unicode_obj)
{
    if (unicode_obj == NULL) {
        return bytes_builder_append_cstr(builder, "(null)");
    }
    const char *utf8 = PyUnicode_AsUTF8(unicode_obj);
    if (utf8 != NULL) {
        return bytes_builder_append_cstr(builder, utf8);
    }
    return bytes_builder_append_object_text(builder, unicode_obj, 'S');
}

static int bytes_builder_append_wchar_text(bytes_builder *builder, const wchar_t *text)
{
    if (text == NULL) {
        return bytes_builder_append_cstr(builder, "(null)");
    }
    void *unicode_obj = PyUnicode_FromWideChar(text, -1);
    if (unicode_obj == NULL) {
        return 0;
    }
    const char *utf8 = PyUnicode_AsUTF8(unicode_obj);
    int ok = bytes_builder_append_cstr(builder, utf8 != NULL ? utf8 : "(null)");
    Py_DecRef(unicode_obj);
    return ok;
}

static void bytes_builder_dealloc(bytes_builder *builder)
{
    if (builder->buf != NULL) {
        free(builder->buf);
    }
    builder->buf = NULL;
    builder->len = 0;
    builder->cap = 0;
}

static Py_ssize_t countformat(const char *format, char endchar)
{
    Py_ssize_t count = 0;
    int level = 0;
    while (level > 0 || *format != endchar) {
        switch (*format) {
            case '\0':
                pyrs_capi_set_error_message("unmatched paren in format");
                return -1;
            case '(':
            case '[':
            case '{':
                if (level == 0) {
                    count++;
                }
                level++;
                break;
            case ')':
            case ']':
            case '}':
                level--;
                break;
            case '#':
            case '&':
            case ',':
            case ':':
            case ' ':
            case '\t':
                break;
            default:
                if (level == 0) {
                    count++;
                }
                break;
        }
        format++;
    }
    return count;
}

static int check_end(const char **format, char endchar)
{
    const char *cursor = *format;
    while (*cursor != endchar) {
        if (*cursor != ' ' && *cursor != '\t' && *cursor != ',' && *cursor != ':') {
            pyrs_capi_set_error_message("unmatched paren in format");
            return 0;
        }
        cursor++;
    }
    if (endchar != '\0') {
        cursor++;
    }
    *format = cursor;
    return 1;
}

static void *do_mkvalue(const char **format, va_list *args);

static void *do_mktuple(const char **format, va_list *args, char endchar, Py_ssize_t n)
{
    if (n < 0) {
        return NULL;
    }
    void *tuple = PyTuple_New(n);
    if (tuple == NULL) {
        return NULL;
    }
    for (Py_ssize_t i = 0; i < n; i++) {
        void *item = do_mkvalue(format, args);
        if (item == NULL) {
            Py_DecRef(tuple);
            return NULL;
        }
        if (PyTuple_SetItem(tuple, i, item) != 0) {
            Py_DecRef(item);
            Py_DecRef(tuple);
            return NULL;
        }
    }
    if (!check_end(format, endchar)) {
        Py_DecRef(tuple);
        return NULL;
    }
    return tuple;
}

static void *do_mklist(const char **format, va_list *args, char endchar, Py_ssize_t n)
{
    if (n < 0) {
        return NULL;
    }
    void *list = PyList_New(0);
    if (list == NULL) {
        return NULL;
    }
    for (Py_ssize_t i = 0; i < n; i++) {
        void *item = do_mkvalue(format, args);
        if (item == NULL) {
            Py_DecRef(list);
            return NULL;
        }
        if (PyList_Append(list, item) != 0) {
            Py_DecRef(item);
            Py_DecRef(list);
            return NULL;
        }
        Py_DecRef(item);
    }
    if (!check_end(format, endchar)) {
        Py_DecRef(list);
        return NULL;
    }
    return list;
}

static void *do_mkdict(const char **format, va_list *args, char endchar, Py_ssize_t n)
{
    if (n < 0) {
        return NULL;
    }
    if ((n % 2) != 0) {
        pyrs_capi_set_error_message("bad dict format");
        return NULL;
    }
    void *dict = PyDict_New();
    if (dict == NULL) {
        return NULL;
    }
    for (Py_ssize_t i = 0; i < n; i += 2) {
        void *key = do_mkvalue(format, args);
        if (key == NULL) {
            Py_DecRef(dict);
            return NULL;
        }
        void *value = do_mkvalue(format, args);
        if (value == NULL) {
            Py_DecRef(key);
            Py_DecRef(dict);
            return NULL;
        }
        if (PyDict_SetItem(dict, key, value) != 0) {
            Py_DecRef(key);
            Py_DecRef(value);
            Py_DecRef(dict);
            return NULL;
        }
        Py_DecRef(key);
        Py_DecRef(value);
    }
    if (!check_end(format, endchar)) {
        Py_DecRef(dict);
        return NULL;
    }
    return dict;
}

static void *do_mkvalue(const char **format, va_list *args)
{
    int trace = getenv("PYRS_TRACE_CAPI_CALLF") != NULL;
    for (;;) {
        char token = *(*format)++;
        if (trace) {
            unsigned char u = (unsigned char)token;
            fprintf(stderr,
                    "[capi-do_mkvalue] token='%c' (0x%02x)\n",
                    (u >= 32 && u < 127) ? token : '?',
                    u);
        }
        switch (token) {
            case '(':
                return do_mktuple(format, args, ')', countformat(*format, ')'));
            case '[':
                return do_mklist(format, args, ']', countformat(*format, ']'));
            case '{':
                return do_mkdict(format, args, '}', countformat(*format, '}'));
            case 'b':
            case 'B':
            case 'h':
            case 'i':
                return PyLong_FromLong((long)va_arg(*args, int));
            case 'H':
                return PyLong_FromLong((long)va_arg(*args, unsigned int));
            case 'I':
                return PyLong_FromUnsignedLong((unsigned long)va_arg(*args, unsigned int));
            case 'n':
                return PyLong_FromSsize_t(va_arg(*args, Py_ssize_t));
            case 'l':
                return PyLong_FromLong(va_arg(*args, long));
            case 'k':
                return PyLong_FromUnsignedLong(va_arg(*args, unsigned long));
            case 'L':
                return PyLong_FromLongLong(va_arg(*args, long long));
            case 'K':
                return PyLong_FromUnsignedLongLong(va_arg(*args, unsigned long long));
            case 'f':
            case 'd':
                return PyFloat_FromDouble(va_arg(*args, double));
            case 'p':
                return PyBool_FromLong((long)va_arg(*args, int));
            case 'c': {
                char one[1];
                one[0] = (char)va_arg(*args, int);
                return PyBytes_FromStringAndSize(one, 1);
            }
            case 's':
            case 'z':
            case 'U':
            case 'y': {
                int trace = getenv("PYRS_TRACE_CAPI_CALLF") != NULL;
                const char *text = va_arg(*args, const char *);
                Py_ssize_t len = -1;
                if (**format == '#') {
                    (*format)++;
                    len = va_arg(*args, Py_ssize_t);
                }
                if (text == NULL) {
                    return build_none();
                }
                if (len < 0) {
                    size_t measured = strlen(text);
                    if (measured > (size_t)INTPTR_MAX) {
                        pyrs_capi_set_error_message("string too long for Py_BuildValue");
                        return NULL;
                    }
                    len = (Py_ssize_t)measured;
                }
                if (token == 'y') {
                    void *bytes_value = PyBytes_FromStringAndSize(text, len);
                    if (trace) {
                        fprintf(stderr,
                                "[capi-callf-str] token=%c text_ptr=%p len=%zd result=%p\n",
                                token,
                                (const void *)text,
                                (ssize_t)len,
                                bytes_value);
                    }
                    return bytes_value;
                }
                void *unicode_value = PyUnicode_FromStringAndSize(text, len);
                if (trace) {
                    fprintf(stderr,
                            "[capi-callf-str] token=%c text_ptr=%p len=%zd result=%p\n",
                            token,
                            (const void *)text,
                            (ssize_t)len,
                            unicode_value);
                }
                return unicode_value;
            }
            case 'N':
            case 'S':
            case 'O': {
                if (**format == '&') {
                    pyrs_capi_set_error_message(
                        "Py_BuildValue converter callbacks (O&) are not implemented"
                    );
                    return NULL;
                }
                void *object = va_arg(*args, void *);
                if (object == NULL) {
                    pyrs_capi_set_error_message("NULL object passed to Py_BuildValue");
                    return NULL;
                }
                if (token != 'N') {
                    Py_IncRef(object);
                }
                return object;
            }
            case ':':
            case ',':
            case ' ':
            case '\t':
                break;
            case '\0':
                pyrs_capi_set_error_message("bad format char passed to Py_BuildValue");
                return NULL;
            default:
                pyrs_capi_set_error_message("bad format char passed to Py_BuildValue");
                return NULL;
        }
    }
}

static void *va_build_value(const char *format, va_list *args)
{
    const char *cursor = format;
    Py_ssize_t n = countformat(cursor, '\0');
    if (n < 0) {
        return NULL;
    }
    if (n == 0) {
        return build_none();
    }
    if (n == 1) {
        return do_mkvalue(&cursor, args);
    }
    return do_mktuple(&cursor, args, '\0', n);
}

static void *call_with_borrowed_stack(void *callable, void *const *stack, Py_ssize_t nargs)
{
    if (nargs == 0) {
        return PyObject_CallObject(callable, NULL);
    }
    void *args_tuple = PyTuple_New(nargs);
    if (args_tuple == NULL) {
        return NULL;
    }
    for (Py_ssize_t i = 0; i < nargs; i++) {
        void *item = stack[i];
        if (item == NULL) {
            pyrs_capi_set_error_message("PyObject_CallFunction received NULL argument");
            Py_DecRef(args_tuple);
            return NULL;
        }
        Py_IncRef(item);
        if (PyTuple_SetItem(args_tuple, i, item) != 0) {
            Py_DecRef(item);
            Py_DecRef(args_tuple);
            return NULL;
        }
    }
    void *result = PyObject_CallObject(callable, args_tuple);
    Py_DecRef(args_tuple);
    return result;
}

static void *callfunction_va(void *callable, const char *format, va_list *ap)
{
    int trace = getenv("PYRS_TRACE_CAPI_CALLF") != NULL;
    if (format == NULL || *format == '\0') {
        return PyObject_CallObject(callable, NULL);
    }
    Py_ssize_t nargs = countformat(format, '\0');
    if (nargs < 0) {
        return NULL;
    }
    if (nargs == 0) {
        return PyObject_CallObject(callable, NULL);
    }

    const char *cursor = format;
    void *stack_items[16];
    void **stack = stack_items;
    if (nargs > (Py_ssize_t)(sizeof(stack_items) / sizeof(stack_items[0]))) {
        size_t bytes = (size_t)nargs * sizeof(void *);
        stack = (void **)malloc(bytes);
        if (stack == NULL) {
            pyrs_capi_set_error_message("PyObject_CallFunction failed allocating argument stack");
            return NULL;
        }
    }

    for (Py_ssize_t i = 0; i < nargs; i++) {
        if (trace) {
            unsigned char token = (unsigned char)*cursor;
            fprintf(stderr, "[capi-callf-token] i=%zd token='%c' (0x%02x)\n",
                    (ssize_t)i,
                    (token >= 32 && token < 127) ? token : '?',
                    token);
        }
        stack[i] = do_mkvalue(&cursor, ap);
        if (stack[i] == NULL) {
            for (Py_ssize_t j = 0; j < i; j++) {
                Py_DecRef(stack[j]);
            }
            if (stack != stack_items) {
                free(stack);
            }
            return NULL;
        }
    }
    if (!check_end(&cursor, '\0')) {
        for (Py_ssize_t i = 0; i < nargs; i++) {
            Py_DecRef(stack[i]);
        }
        if (stack != stack_items) {
            free(stack);
        }
        return NULL;
    }
    if (trace) {
        fprintf(stderr, "[capi-callf] callable=%p format=%s nargs=%zd", callable, format, (ssize_t)nargs);
        for (Py_ssize_t i = 0; i < nargs; i++) {
            fprintf(stderr, " arg%zd=%p", (ssize_t)i, stack[i]);
        }
        fprintf(stderr, "\n");
    }

    void *result = NULL;
    if (nargs == 1 && is_tuple_object(stack[0])) {
        /* CPython quirk compatibility:
         *   - PyObject_CallFunction(f, "O", tuple) => f(*tuple)
         *   - PyObject_CallFunction(f, "(OO)", a, b) => f(*(a, b)) == f(a, b)
         */
        result = PyObject_CallObject(callable, stack[0]);
    } else {
        result = call_with_borrowed_stack(callable, (void *const *)stack, nargs);
    }
    for (Py_ssize_t i = 0; i < nargs; i++) {
        Py_DecRef(stack[i]);
    }
    if (stack != stack_items) {
        free(stack);
    }
    return result;
}

void *PyTuple_Pack(Py_ssize_t n, ...)
{
    if (n < 0) {
        pyrs_capi_set_error_message("PyTuple_Pack requires non-negative size");
        return NULL;
    }
    if (n == 0) {
        return pyrs_capi_tuple_pack_from_array(0, NULL);
    }

    void *stack_items[16];
    void **items = stack_items;
    if (n > (Py_ssize_t)(sizeof(stack_items) / sizeof(stack_items[0]))) {
        size_t bytes = (size_t)n * sizeof(void *);
        items = (void **)malloc(bytes);
        if (items == NULL) {
            pyrs_capi_set_error_message("PyTuple_Pack failed allocating argument array");
            return NULL;
        }
    }

    va_list ap;
    va_start(ap, n);
    for (Py_ssize_t i = 0; i < n; i++) {
        items[i] = va_arg(ap, void *);
    }
    va_end(ap);

    void *result = pyrs_capi_tuple_pack_from_array(n, (void *const *)items);
    if (items != stack_items) {
        free(items);
    }
    return result;
}

void *Py_BuildValue(const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    void *result = va_build_value(format, &ap);
    va_end(ap);
    return result;
}

void *_Py_BuildValue_SizeT(const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    void *result = va_build_value(format, &ap);
    va_end(ap);
    return result;
}

void *Py_VaBuildValue(const char *format, va_list ap)
{
    va_list copy;
    va_copy(copy, ap);
    void *result = va_build_value(format, &copy);
    va_end(copy);
    return result;
}

void *_Py_VaBuildValue_SizeT(const char *format, va_list ap)
{
    va_list copy;
    va_copy(copy, ap);
    void *result = va_build_value(format, &copy);
    va_end(copy);
    return result;
}

void *PyBytes_FromFormatV(const char *format, va_list vargs)
{
    if (format == NULL) {
        pyrs_capi_set_error_message("PyBytes_FromFormatV received null format");
        return NULL;
    }

    bytes_builder out;
    if (!bytes_builder_init(&out, (Py_ssize_t)strlen(format) + 1)) {
        return NULL;
    }

    for (const char *f = format; *f != '\0'; f++) {
        if (*f != '%') {
            if (!bytes_builder_append_char(&out, (unsigned char)*f)) {
                goto error;
            }
            continue;
        }

        const char *p = f++;
        while (isdigit((unsigned char)*f)) {
            f++;
        }

        Py_ssize_t prec = 0;
        if (*f == '.') {
            f++;
            while (isdigit((unsigned char)*f)) {
                prec = (prec * 10) + (*f - '0');
                f++;
            }
        }

        while (*f != '\0' && *f != '%' && !isalpha((unsigned char)*f)) {
            f++;
        }

        int longflag = 0;
        if (*f == 'l' && (f[1] == 'd' || f[1] == 'u' || f[1] == 's' || f[1] == 'V')) {
            longflag = 1;
            f++;
        }

        int size_tflag = 0;
        if (*f == 'z' && (f[1] == 'd' || f[1] == 'u')) {
            size_tflag = 1;
            f++;
        }

        switch (*f) {
            case 'c': {
                int c = va_arg(vargs, int);
                if (c < 0 || c > 255) {
                    pyrs_capi_set_error_message(
                        "PyBytes_FromFormatV(): %c format expects an integer in range [0; 255]"
                    );
                    goto error;
                }
                if (!bytes_builder_append_char(&out, (unsigned char)c)) {
                    goto error;
                }
                break;
            }
            case 'd': {
                char buffer[64];
                if (longflag) {
                    snprintf(buffer, sizeof(buffer), "%ld", va_arg(vargs, long));
                } else if (size_tflag) {
                    snprintf(buffer, sizeof(buffer), "%zd", va_arg(vargs, Py_ssize_t));
                } else {
                    snprintf(buffer, sizeof(buffer), "%d", va_arg(vargs, int));
                }
                if (!bytes_builder_append_cstr(&out, buffer)) {
                    goto error;
                }
                break;
            }
            case 'u': {
                char buffer[64];
                if (longflag) {
                    snprintf(buffer, sizeof(buffer), "%lu", va_arg(vargs, unsigned long));
                } else if (size_tflag) {
                    snprintf(buffer, sizeof(buffer), "%zu", va_arg(vargs, size_t));
                } else {
                    snprintf(buffer, sizeof(buffer), "%u", va_arg(vargs, unsigned int));
                }
                if (!bytes_builder_append_cstr(&out, buffer)) {
                    goto error;
                }
                break;
            }
            case 'i': {
                char buffer[64];
                snprintf(buffer, sizeof(buffer), "%i", va_arg(vargs, int));
                if (!bytes_builder_append_cstr(&out, buffer)) {
                    goto error;
                }
                break;
            }
            case 'x': {
                char buffer[64];
                snprintf(buffer, sizeof(buffer), "%x", va_arg(vargs, int));
                if (!bytes_builder_append_cstr(&out, buffer)) {
                    goto error;
                }
                break;
            }
            case 's': {
                const char *text = va_arg(vargs, const char *);
                if (text == NULL) {
                    text = "(null)";
                }
                Py_ssize_t text_len = 0;
                if (prec <= 0) {
                    text_len = (Py_ssize_t)strlen(text);
                } else {
                    while (text_len < prec && text[text_len] != '\0') {
                        text_len++;
                    }
                }
                if (!bytes_builder_append_bytes(&out, text, text_len)) {
                    goto error;
                }
                break;
            }
            case 'S':
            case 'R':
            case 'A': {
                void *object = va_arg(vargs, void *);
                if (!bytes_builder_append_object_text(&out, object, *f)) {
                    goto error;
                }
                break;
            }
            case 'U': {
                void *unicode_obj = va_arg(vargs, void *);
                if (!bytes_builder_append_unicode_text(&out, unicode_obj)) {
                    goto error;
                }
                break;
            }
            case 'V': {
                void *unicode_obj = va_arg(vargs, void *);
                if (longflag) {
                    const wchar_t *fallback = va_arg(vargs, const wchar_t *);
                    if (unicode_obj != NULL) {
                        if (!bytes_builder_append_unicode_text(&out, unicode_obj)) {
                            goto error;
                        }
                    } else if (!bytes_builder_append_wchar_text(&out, fallback)) {
                        goto error;
                    }
                } else {
                    const char *fallback = va_arg(vargs, const char *);
                    if (unicode_obj != NULL) {
                        if (!bytes_builder_append_unicode_text(&out, unicode_obj)) {
                            goto error;
                        }
                    } else if (!bytes_builder_append_cstr(&out, fallback)) {
                        goto error;
                    }
                }
                break;
            }
            case 'p': {
                char buffer[64];
                snprintf(buffer, sizeof(buffer), "%p", va_arg(vargs, void *));
                if (buffer[0] == '0' && (buffer[1] == 'x' || buffer[1] == 'X')) {
                    buffer[1] = 'x';
                } else {
                    size_t len = strlen(buffer);
                    memmove(buffer + 2, buffer, len + 1);
                    buffer[0] = '0';
                    buffer[1] = 'x';
                }
                if (!bytes_builder_append_cstr(&out, buffer)) {
                    goto error;
                }
                break;
            }
            case '%':
                if (!bytes_builder_append_char(&out, '%')) {
                    goto error;
                }
                break;
            default:
                if (!bytes_builder_append_cstr(&out, p)) {
                    goto error;
                }
                {
                    void *result = PyBytes_FromStringAndSize(out.buf, out.len);
                    bytes_builder_dealloc(&out);
                    return result;
                }
        }
    }

    {
        void *result = PyBytes_FromStringAndSize(out.buf, out.len);
        bytes_builder_dealloc(&out);
        return result;
    }

error:
    bytes_builder_dealloc(&out);
    return NULL;
}

void *PyBytes_FromFormat(const char *format, ...)
{
    va_list vargs;
    va_start(vargs, format);
    void *result = PyBytes_FromFormatV(format, vargs);
    va_end(vargs);
    return result;
}

void *PyUnicode_FromFormatV(const char *format, va_list vargs)
{
    void *bytes_value = PyBytes_FromFormatV(format, vargs);
    if (bytes_value == NULL) {
        return NULL;
    }
    char *payload = NULL;
    Py_ssize_t payload_len = 0;
    if (PyBytes_AsStringAndSize(bytes_value, &payload, &payload_len) != 0) {
        Py_DecRef(bytes_value);
        return NULL;
    }
    void *unicode_value = PyUnicode_FromStringAndSize(payload, payload_len);
    Py_DecRef(bytes_value);
    return unicode_value;
}

void *PyUnicode_FromFormat(const char *format, ...)
{
    va_list vargs;
    va_start(vargs, format);
    void *result = PyUnicode_FromFormatV(format, vargs);
    va_end(vargs);
    return result;
}

void *PyObject_CallFunction(void *callable, const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    void *result = callfunction_va(callable, format, &ap);
    va_end(ap);
    return result;
}

void *_PyObject_CallFunction_SizeT(void *callable, const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    void *result = callfunction_va(callable, format, &ap);
    va_end(ap);
    return result;
}

void *PyObject_CallMethod(void *object, const char *name, const char *format, ...)
{
    if (object == NULL || name == NULL) {
        pyrs_capi_set_error_message("PyObject_CallMethod received null object/name");
        return NULL;
    }
    void *callable = PyObject_GetAttrString(object, name);
    if (callable == NULL) {
        return NULL;
    }
    va_list ap;
    va_start(ap, format);
    void *result = callfunction_va(callable, format, &ap);
    va_end(ap);
    Py_DecRef(callable);
    return result;
}

void *_PyObject_CallMethod_SizeT(void *object, const char *name, const char *format, ...)
{
    if (object == NULL || name == NULL) {
        pyrs_capi_set_error_message("_PyObject_CallMethod_SizeT received null object/name");
        return NULL;
    }
    void *callable = PyObject_GetAttrString(object, name);
    if (callable == NULL) {
        return NULL;
    }
    va_list ap;
    va_start(ap, format);
    void *result = callfunction_va(callable, format, &ap);
    va_end(ap);
    Py_DecRef(callable);
    return result;
}

void *PyEval_CallFunction(void *callable, const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    void *result = callfunction_va(callable, format, &ap);
    va_end(ap);
    return result;
}

void *PyEval_CallMethod(void *object, const char *name, const char *format, ...)
{
    if (object == NULL || name == NULL) {
        pyrs_capi_set_error_message("PyEval_CallMethod received null object/name");
        return NULL;
    }
    void *callable = PyObject_GetAttrString(object, name);
    if (callable == NULL) {
        return NULL;
    }
    va_list ap;
    va_start(ap, format);
    void *result = callfunction_va(callable, format, &ap);
    va_end(ap);
    Py_DecRef(callable);
    return result;
}

void *PyObject_CallFunctionObjArgs(void *callable, ...)
{
    void *stack_items[16];
    void **stack = stack_items;
    Py_ssize_t nargs = 0;
    Py_ssize_t capacity = (Py_ssize_t)(sizeof(stack_items) / sizeof(stack_items[0]));

    va_list ap;
    va_start(ap, callable);
    for (;;) {
        void *arg = va_arg(ap, void *);
        if (arg == NULL) {
            break;
        }
        if (nargs == capacity) {
            Py_ssize_t next_capacity = capacity * 2;
            size_t bytes = (size_t)next_capacity * sizeof(void *);
            if (stack == stack_items) {
                stack = (void **)malloc(bytes);
                if (stack == NULL) {
                    va_end(ap);
                    pyrs_capi_set_error_message(
                        "PyObject_CallFunctionObjArgs failed allocating stack"
                    );
                    return NULL;
                }
                for (Py_ssize_t i = 0; i < nargs; i++) {
                    stack[i] = stack_items[i];
                }
            } else {
                void **grown = (void **)realloc(stack, bytes);
                if (grown == NULL) {
                    free(stack);
                    va_end(ap);
                    pyrs_capi_set_error_message(
                        "PyObject_CallFunctionObjArgs failed growing stack"
                    );
                    return NULL;
                }
                stack = grown;
            }
            capacity = next_capacity;
        }
        stack[nargs++] = arg;
    }
    va_end(ap);

    void *result = call_with_borrowed_stack(callable, (void *const *)stack, nargs);
    if (stack != stack_items) {
        free(stack);
    }
    return result;
}

void *PyObject_CallMethodObjArgs(void *object, void *name, ...)
{
    void *stack_items[16];
    void **stack = stack_items;
    Py_ssize_t nargs = 0;
    Py_ssize_t capacity = (Py_ssize_t)(sizeof(stack_items) / sizeof(stack_items[0]));

    if (object == NULL || name == NULL) {
        pyrs_capi_set_error_message("PyObject_CallMethodObjArgs received null object/name");
        return NULL;
    }
    void *callable = PyObject_GetAttr(object, name);
    if (callable == NULL) {
        return NULL;
    }

    va_list ap;
    va_start(ap, name);
    for (;;) {
        void *arg = va_arg(ap, void *);
        if (arg == NULL) {
            break;
        }
        if (nargs == capacity) {
            Py_ssize_t next_capacity = capacity * 2;
            size_t bytes = (size_t)next_capacity * sizeof(void *);
            if (stack == stack_items) {
                stack = (void **)malloc(bytes);
                if (stack == NULL) {
                    Py_DecRef(callable);
                    va_end(ap);
                    pyrs_capi_set_error_message(
                        "PyObject_CallMethodObjArgs failed allocating stack"
                    );
                    return NULL;
                }
                for (Py_ssize_t i = 0; i < nargs; i++) {
                    stack[i] = stack_items[i];
                }
            } else {
                void **grown = (void **)realloc(stack, bytes);
                if (grown == NULL) {
                    Py_DecRef(callable);
                    free(stack);
                    va_end(ap);
                    pyrs_capi_set_error_message(
                        "PyObject_CallMethodObjArgs failed growing stack"
                    );
                    return NULL;
                }
                stack = grown;
            }
            capacity = next_capacity;
        }
        stack[nargs++] = arg;
    }
    va_end(ap);

    void *result = call_with_borrowed_stack(callable, (void *const *)stack, nargs);
    Py_DecRef(callable);
    if (stack != stack_items) {
        free(stack);
    }
    return result;
}

typedef int (*arg_converter_fn)(void *value, void *output);
#define PYRS_PY_CLEANUP_SUPPORTED 0x20000
typedef enum {
    PYRS_ARG_CLEANUP_CONVERTER = 0,
    PYRS_ARG_CLEANUP_BUFFER = 1,
    PYRS_ARG_CLEANUP_PTR = 2,
} PyrsArgCleanupKind;

typedef struct {
    PyrsArgCleanupKind kind;
    arg_converter_fn converter;
    void *target;
} PyArgCleanupEntry;

static void pyrs_run_arg_cleanups(PyArgCleanupEntry *entries, size_t count)
{
    while (count > 0) {
        PyArgCleanupEntry entry = entries[--count];
        switch (entry.kind) {
            case PYRS_ARG_CLEANUP_CONVERTER:
                if (entry.converter != NULL) {
                    entry.converter(NULL, entry.target);
                }
                break;
            case PYRS_ARG_CLEANUP_BUFFER:
                if (entry.target != NULL) {
                    PyBuffer_Release(entry.target);
                }
                break;
            case PYRS_ARG_CLEANUP_PTR:
                if (entry.target != NULL) {
                    char **slot = (char **)entry.target;
                    if (*slot != NULL) {
                        PyMem_Free(*slot);
                        *slot = NULL;
                    }
                }
                break;
            default:
                break;
        }
    }
}

static int pyrs_push_converter_cleanup(
    PyArgCleanupEntry *entries,
    size_t *count,
    arg_converter_fn converter,
    void *target
)
{
    if (*count >= 16) {
        pyrs_capi_set_error_message("PyArg_ParseTupleAndKeywords cleanup stack overflow");
        return 0;
    }
    entries[*count].kind = PYRS_ARG_CLEANUP_CONVERTER;
    entries[*count].converter = converter;
    entries[*count].target = target;
    (*count)++;
    return 1;
}

static int pyrs_push_buffer_cleanup(
    PyArgCleanupEntry *entries,
    size_t *count,
    void *target
)
{
    if (*count >= 16) {
        pyrs_capi_set_error_message("PyArg_ParseTupleAndKeywords cleanup stack overflow");
        return 0;
    }
    entries[*count].kind = PYRS_ARG_CLEANUP_BUFFER;
    entries[*count].converter = NULL;
    entries[*count].target = target;
    (*count)++;
    return 1;
}

static int pyrs_push_ptr_cleanup(
    PyArgCleanupEntry *entries,
    size_t *count,
    char **target
)
{
    if (*count >= 16) {
        pyrs_capi_set_error_message("PyArg_ParseTupleAndKeywords cleanup stack overflow");
        return 0;
    }
    entries[*count].kind = PYRS_ARG_CLEANUP_PTR;
    entries[*count].converter = NULL;
    entries[*count].target = target;
    (*count)++;
    return 1;
}

static int count_old_style_format_args(const char *format, int *min_count, int *max_count);
static int parse_args_and_keywords_va(
    void *args,
    void *kwargs,
    const char *format,
    const char *const *keywords,
    va_list *ap
);

static const char *pyrs_tuple_error_type_name(void *value)
{
    if (value == (void *)&_Py_NoneStruct) {
        return "None";
    }
    if (object_is_instance_of_type(value, (void *)&PyLong_Type)) {
        return "int";
    }
    if (object_is_instance_of_type(value, (void *)&PyUnicode_Type)) {
        return "str";
    }
    if (object_is_instance_of_type(value, (void *)&PyBytes_Type)) {
        return "bytes";
    }
    if (object_is_instance_of_type(value, (void *)&PyByteArray_Type)) {
        return "bytearray";
    }
    if (object_is_instance_of_type(value, (void *)&PyList_Type)) {
        return "list";
    }
    if (object_is_instance_of_type(value, (void *)&PyDict_Type)) {
        return "dict";
    }
    return "object";
}

static const char *pyrs_find_matching_paren(const char *format)
{
    int depth = 0;
    for (const char *cursor = format; *cursor != '\0'; cursor++) {
        if (*cursor == '(') {
            depth++;
        } else if (*cursor == ')') {
            depth--;
            if (depth == 0) {
                return cursor;
            }
        }
    }
    return NULL;
}

static void pyrs_zero_buffer(PyrsPyBuffer *view)
{
    if (view != NULL) {
        memset(view, 0, sizeof(*view));
    }
}

static void pyrs_skip_ignorable_format_chars(const char **p_format)
{
    const char *format = *p_format;
    while (*format == ' ' || *format == '\t' || *format == ',') {
        format++;
    }
    *p_format = format;
}

static int pyrs_require_c_contiguous(PyrsPyBuffer *view)
{
    if (view == NULL) {
        return 0;
    }
    if (PyBuffer_IsContiguous(view, 'C') != 0) {
        return 1;
    }
    PyBuffer_Release(view);
    pyrs_zero_buffer(view);
    PyErr_SetString((void *)&PyExc_BufferError, "object is not C-contiguous");
    return 0;
}

static int pyrs_get_unicode_utf8_data(void *value, const char **out, Py_ssize_t *out_len)
{
    Py_ssize_t len = 0;
    const char *text = PyUnicode_AsUTF8AndSize(value, &len);
    if (text == NULL) {
        return 0;
    }
    if (out != NULL) {
        *out = text;
    }
    if (out_len != NULL) {
        *out_len = len;
    }
    return 1;
}

static int pyrs_get_bytes_data(void *value, char **out, Py_ssize_t *out_len)
{
    char *data = NULL;
    Py_ssize_t len = 0;
    if (PyBytes_AsStringAndSize(value, &data, &len) != 0) {
        return 0;
    }
    if (out != NULL) {
        *out = data;
    }
    if (out_len != NULL) {
        *out_len = len;
    }
    return 1;
}

static int pyrs_get_bytearray_data(void *value, char **out, Py_ssize_t *out_len)
{
    Py_ssize_t len = PyByteArray_Size(value);
    if (len < 0) {
        return 0;
    }
    char *data = PyByteArray_AsString(value);
    if (data == NULL) {
        return 0;
    }
    if (out != NULL) {
        *out = data;
    }
    if (out_len != NULL) {
        *out_len = len;
    }
    return 1;
}

static int pyrs_fill_unicode_buffer(void *value, PyrsPyBuffer *view)
{
    const char *text = NULL;
    Py_ssize_t len = 0;
    pyrs_zero_buffer(view);
    if (!pyrs_get_unicode_utf8_data(value, &text, &len)) {
        return 0;
    }
    return PyBuffer_FillInfo(view, value, (void *)text, len, 1, PYBUF_SIMPLE) == 0;
}

static int pyrs_get_simple_buffer(void *value, PyrsPyBuffer *view)
{
    pyrs_zero_buffer(view);
    if (PyObject_GetBuffer(value, view, PYBUF_SIMPLE) != 0) {
        return 0;
    }
    return pyrs_require_c_contiguous(view);
}

static int pyrs_get_writable_buffer(void *value, PyrsPyBuffer *view)
{
    pyrs_zero_buffer(view);
    if (PyObject_GetBuffer(value, view, PYBUF_WRITABLE) != 0) {
        return 0;
    }
    if (PyBuffer_IsContiguous(view, 'C') != 0) {
        return 1;
    }
    PyBuffer_Release(view);
    pyrs_zero_buffer(view);
    return 0;
}

static int pyrs_decode_utf8_codepoint(const char *text, Py_ssize_t len, int *out)
{
    if (text == NULL || len <= 0 || out == NULL) {
        return 0;
    }
    const unsigned char *bytes = (const unsigned char *)text;
    if ((bytes[0] & 0x80u) == 0) {
        *out = bytes[0];
        return 1;
    }
    if ((bytes[0] & 0xE0u) == 0xC0u && len >= 2) {
        *out = ((bytes[0] & 0x1Fu) << 6) | (bytes[1] & 0x3Fu);
        return 1;
    }
    if ((bytes[0] & 0xF0u) == 0xE0u && len >= 3) {
        *out = ((bytes[0] & 0x0Fu) << 12) | ((bytes[1] & 0x3Fu) << 6) | (bytes[2] & 0x3Fu);
        return 1;
    }
    if ((bytes[0] & 0xF8u) == 0xF0u && len >= 4) {
        *out = ((bytes[0] & 0x07u) << 18)
            | ((bytes[1] & 0x3Fu) << 12)
            | ((bytes[2] & 0x3Fu) << 6)
            | (bytes[3] & 0x3Fu);
        return 1;
    }
    return 0;
}

static int pyrs_nested_tuple_requires_tuple(const char *group_start, const char *group_end)
{
    for (const char *cursor = group_start + 1; cursor < group_end; cursor++) {
        switch (*cursor) {
            case '(':
                return 1;
            case 'S':
            case 'Y':
            case 'U':
                return 1;
            case 'O':
                if (cursor[1] != '&') {
                    return 1;
                }
                cursor++;
                break;
            default:
                break;
        }
    }
    return 0;
}

static int pyrs_lookup_keyword_value(
    void *kwargs,
    const char *keyword_name,
    void **out_value
)
{
    if (out_value != NULL) {
        *out_value = NULL;
    }
    if (kwargs == NULL || keyword_name == NULL || keyword_name[0] == '\0') {
        return 1;
    }
    void *key_obj = PyUnicode_FromString(keyword_name);
    if (key_obj == NULL) {
        return 0;
    }
    void *value = PyDict_GetItem(kwargs, key_obj);
    Py_DecRef(key_obj);
    if (out_value != NULL) {
        *out_value = value;
    }
    return 1;
}

static int pyrs_parse_nested_tuple_item(
    void *value,
    Py_ssize_t argument_index,
    const char *group_start,
    const char **group_end_out,
    va_list *ap
)
{
    const char *group_end = pyrs_find_matching_paren(group_start);
    if (group_end == NULL) {
        pyrs_capi_set_error_message("unmatched paren in format");
        return 0;
    }

    size_t inner_len = (size_t)(group_end - group_start - 1);
    char *inner_format = (char *)malloc(inner_len + 1);
    if (inner_format == NULL) {
        pyrs_capi_set_error_message("PyArg_ParseTupleAndKeywords failed allocating format copy");
        return 0;
    }
    memcpy(inner_format, group_start + 1, inner_len);
    inner_format[inner_len] = '\0';

    int expected_min = 0;
    int expected_max = 0;
    if (!count_old_style_format_args(inner_format, &expected_min, &expected_max)) {
        free(inner_format);
        return 0;
    }

    void *nested_args = NULL;
    int release_nested_args = 0;
    if (object_is_instance_of_type(value, (void *)&PyTuple_Type)) {
        Py_ssize_t tuple_len = PyTuple_Size(value);
        if (tuple_len < 0) {
            free(inner_format);
            return 0;
        }
        if (tuple_len != expected_max) {
            free(inner_format);
            PyErr_Format(
                (void *)&PyExc_TypeError,
                "argument %zd must be tuple of length %d, not %zd",
                argument_index,
                expected_max,
                tuple_len
            );
            return 0;
        }
        nested_args = value;
        Py_IncRef(nested_args);
        release_nested_args = 1;
    } else {
        int must_be_tuple = pyrs_nested_tuple_requires_tuple(group_start, group_end);
        if (object_is_instance_of_type(value, (void *)&PyUnicode_Type) ||
            object_is_instance_of_type(value, (void *)&PyBytes_Type) ||
            object_is_instance_of_type(value, (void *)&PyByteArray_Type) ||
            object_is_instance_of_type(value, (void *)&PyDict_Type) ||
            object_is_instance_of_type(value, (void *)&PyLong_Type) ||
            value == (void *)&_Py_NoneStruct) {
            free(inner_format);
            PyErr_Format(
                (void *)&PyExc_TypeError,
                "argument %zd must be %d-item tuple, not %s",
                argument_index,
                expected_max,
                pyrs_tuple_error_type_name(value)
            );
            return 0;
        }
        void *fast = PySequence_Fast(value, "");
        if (fast == NULL) {
            free(inner_format);
            return 0;
        }
        Py_ssize_t sequence_len = PySequence_Size(fast);
        if (sequence_len < 0) {
            Py_DecRef(fast);
            free(inner_format);
            return 0;
        }
        if (must_be_tuple) {
            char warning_message[160];
            snprintf(
                warning_message,
                sizeof(warning_message),
                "argument must be %d-item tuple, not %s",
                expected_max,
                pyrs_tuple_error_type_name(value)
            );
            if (PyErr_WarnEx(
                    (void *)&PyExc_DeprecationWarning,
                    warning_message,
                    1
                ) != 0) {
                Py_DecRef(fast);
                free(inner_format);
                return 0;
            }
        }
        if (sequence_len != expected_max) {
            Py_DecRef(fast);
            free(inner_format);
            PyErr_Format(
                (void *)&PyExc_TypeError,
                "argument %zd must be %s of length %d, not %zd",
                argument_index,
                must_be_tuple ? "tuple" : "sequence",
                expected_max,
                sequence_len
            );
            return 0;
        }
        nested_args = PySequence_Tuple(fast);
        Py_DecRef(fast);
        if (nested_args == NULL) {
            free(inner_format);
            return 0;
        }
        release_nested_args = 1;
    }

    int ok = parse_args_and_keywords_va(nested_args, NULL, inner_format, NULL, ap);
    if (release_nested_args) {
        Py_DecRef(nested_args);
    }
    free(inner_format);
    if (group_end_out != NULL) {
        *group_end_out = group_end;
    }
    return ok;
}

#define PYRS_IS_END_OF_FORMAT(ch) ((ch) == '\0' || (ch) == ':' || (ch) == ';')

static const char *pyrs_skip_format_unit(const char **p_format, va_list *p_va)
{
    const char *format = *p_format;
    pyrs_skip_ignorable_format_chars(&format);
    char token = *format++;
    switch (token) {
        case '(':
            while (*format != ')') {
                if (PYRS_IS_END_OF_FORMAT(*format)) {
                    return "unmatched paren in format";
                }
                const char *msg = pyrs_skip_format_unit(&format, p_va);
                if (msg != NULL) {
                    return msg;
                }
            }
            format++;
            break;
        case 'O':
            if (*format == '!' || *format == '&') {
                if (p_va != NULL) {
                    (void)va_arg(*p_va, void *);
                    (void)va_arg(*p_va, void *);
                }
                format++;
            } else if (p_va != NULL) {
                (void)va_arg(*p_va, void *);
            }
            break;
        case 'e':
            if (*format != 's' && *format != 't') {
                return "impossible<bad format char>";
            }
            if (p_va != NULL) {
                (void)va_arg(*p_va, void *);
                (void)va_arg(*p_va, void *);
            }
            format++;
            if (*format == '#') {
                if (p_va != NULL) {
                    (void)va_arg(*p_va, void *);
                }
                format++;
            }
            break;
        case 's':
        case 'z':
        case 'y':
            if (*format == '#' ) {
                if (p_va != NULL) {
                    (void)va_arg(*p_va, void *);
                    (void)va_arg(*p_va, void *);
                }
                format++;
            } else if (*format == '*') {
                if (p_va != NULL) {
                    (void)va_arg(*p_va, void *);
                }
                format++;
            } else if (p_va != NULL) {
                (void)va_arg(*p_va, void *);
            }
            break;
        case 'w':
            if (*format != '*') {
                return "impossible<bad format char>";
            }
            if (p_va != NULL) {
                (void)va_arg(*p_va, void *);
            }
            format++;
            break;
        case 'S':
        case 'Y':
        case 'U':
        case 'p':
        case 'b':
        case 'B':
        case 'h':
        case 'H':
        case 'i':
        case 'I':
        case 'k':
        case 'l':
        case 'n':
        case 'L':
        case 'K':
        case 'f':
        case 'd':
        case 'D':
        case 'c':
        case 'C':
            if (p_va != NULL) {
                (void)va_arg(*p_va, void *);
            }
            break;
        default:
            return "impossible<bad format char>";
    }
    *p_format = format;
    return NULL;
}

static int count_old_style_format_args(const char *format, int *min_count, int *max_count)
{
    int min = -1;
    int max = 0;
    const char *cursor = format;
    while (!PYRS_IS_END_OF_FORMAT(*cursor)) {
        pyrs_skip_ignorable_format_chars(&cursor);
        if (*cursor == '|') {
            if (min < 0) {
                min = max;
            }
            cursor++;
            continue;
        }
        const char *msg = pyrs_skip_format_unit(&cursor, NULL);
        if (msg != NULL) {
            pyrs_capi_set_error_message(msg);
            return 0;
        }
        max++;
    }
    if (min < 0) {
        min = max;
    }
    *min_count = min;
    *max_count = max;
    return 1;
}

static int pyrs_scan_keywords(
    const char *const *keywords,
    Py_ssize_t *total_out,
    Py_ssize_t *positional_only_out
)
{
    Py_ssize_t positional_only = 0;
    while (keywords[positional_only] != NULL && keywords[positional_only][0] == '\0') {
        positional_only++;
    }
    Py_ssize_t total = positional_only;
    while (keywords[total] != NULL) {
        if (keywords[total][0] == '\0') {
            PyErr_SetString((void *)&PyExc_SystemError, "Empty keyword parameter name");
            return 0;
        }
        total++;
    }

    *total_out = total;
    *positional_only_out = positional_only;
    return 1;
}

static int pyrs_convert_encoded_string(
    void *value,
    int recode_strings,
    const char *encoding,
    char **buffer,
    Py_ssize_t *buffer_len,
    int use_hash,
    PyArgCleanupEntry *cleanups,
    size_t *cleanup_count
)
{
    if (encoding == NULL) {
        encoding = PyUnicode_GetDefaultEncoding();
    }

    void *encoded_owner = NULL;
    const char *ptr = NULL;
    Py_ssize_t size = 0;
    if (!recode_strings &&
        (object_is_instance_of_type(value, (void *)&PyBytes_Type) ||
         object_is_instance_of_type(value, (void *)&PyByteArray_Type))) {
        encoded_owner = value;
        Py_IncRef(encoded_owner);
        if (object_is_instance_of_type(value, (void *)&PyBytes_Type)) {
            if (!pyrs_get_bytes_data(value, (char **)&ptr, &size)) {
                Py_DecRef(encoded_owner);
                return 0;
            }
        } else {
            if (!pyrs_get_bytearray_data(value, (char **)&ptr, &size)) {
                Py_DecRef(encoded_owner);
                return 0;
            }
        }
    } else if (object_is_instance_of_type(value, (void *)&PyUnicode_Type)) {
        encoded_owner = PyUnicode_AsEncodedString(value, encoding, NULL);
        if (encoded_owner == NULL) {
            return 0;
        }
        if (!pyrs_get_bytes_data(encoded_owner, (char **)&ptr, &size)) {
            Py_DecRef(encoded_owner);
            return 0;
        }
    } else {
        PyErr_SetString(
            (void *)&PyExc_TypeError,
            recode_strings ? "expected str" : "expected str, bytes or bytearray"
        );
        return 0;
    }

    if (use_hash) {
        if (buffer_len == NULL) {
            Py_DecRef(encoded_owner);
            PyErr_SetString((void *)&PyExc_SystemError, "buffer_len is NULL");
            return 0;
        }
        if (*buffer == NULL) {
            *buffer = (char *)PyMem_Malloc((size_t)size + 1);
            if (*buffer == NULL) {
                Py_DecRef(encoded_owner);
                PyErr_NoMemory();
                return 0;
            }
            if (!pyrs_push_ptr_cleanup(cleanups, cleanup_count, buffer)) {
                Py_DecRef(encoded_owner);
                PyMem_Free(*buffer);
                *buffer = NULL;
                return 0;
            }
        } else if (size + 1 > *buffer_len) {
            Py_DecRef(encoded_owner);
            PyErr_Format(
                (void *)&PyExc_ValueError,
                "encoded string too long (%zd, maximum length %zd)",
                size,
                *buffer_len - 1
            );
            return 0;
        }
        memcpy(*buffer, ptr, (size_t)size);
        (*buffer)[size] = '\0';
        *buffer_len = size;
    } else {
        if (memchr(ptr, '\0', (size_t)size) != NULL) {
            Py_DecRef(encoded_owner);
            PyErr_SetString((void *)&PyExc_TypeError, "encoded string without null bytes");
            return 0;
        }
        *buffer = (char *)PyMem_Malloc((size_t)size + 1);
        if (*buffer == NULL) {
            Py_DecRef(encoded_owner);
            PyErr_NoMemory();
            return 0;
        }
        if (!pyrs_push_ptr_cleanup(cleanups, cleanup_count, buffer)) {
            Py_DecRef(encoded_owner);
            PyMem_Free(*buffer);
            *buffer = NULL;
            return 0;
        }
        memcpy(*buffer, ptr, (size_t)size);
        (*buffer)[size] = '\0';
    }

    Py_DecRef(encoded_owner);
    return 1;
}

static int pyrs_convert_value_for_format_unit(
    void *value,
    const char **p_format,
    va_list *ap,
    Py_ssize_t argument_index,
    PyArgCleanupEntry *cleanups,
    size_t *cleanup_count
)
{
    const char *format = *p_format;
    char token = *format++;
    switch (token) {
        case 'p': {
            int *output = va_arg(*ap, int *);
            int truth = PyObject_IsTrue(value);
            if (truth < 0) {
                return 0;
            }
            if (output != NULL) {
                *output = truth ? 1 : 0;
            }
            break;
        }
        case 'b': {
            unsigned char *output = va_arg(*ap, unsigned char *);
            long parsed = PyLong_AsLong(value);
            if (parsed == -1 && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (parsed < 0) {
                PyErr_SetString((void *)&PyExc_OverflowError, "unsigned byte integer is less than minimum");
                return 0;
            }
            if ((unsigned long)parsed > UCHAR_MAX) {
                PyErr_SetString((void *)&PyExc_OverflowError, "unsigned byte integer is greater than maximum");
                return 0;
            }
            if (output != NULL) {
                *output = (unsigned char)parsed;
            }
            break;
        }
        case 'B': {
            unsigned char *output = va_arg(*ap, unsigned char *);
            unsigned long parsed = PyLong_AsUnsignedLongMask(value);
            if (parsed == ULONG_MAX && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = (unsigned char)parsed;
            }
            break;
        }
        case 'h': {
            short *output = va_arg(*ap, short *);
            long parsed = PyLong_AsLong(value);
            if (parsed == -1 && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (parsed < SHRT_MIN) {
                PyErr_SetString((void *)&PyExc_OverflowError, "signed short integer is less than minimum");
                return 0;
            }
            if (parsed > SHRT_MAX) {
                PyErr_SetString((void *)&PyExc_OverflowError, "signed short integer is greater than maximum");
                return 0;
            }
            if (output != NULL) {
                *output = (short)parsed;
            }
            break;
        }
        case 'H': {
            unsigned short *output = va_arg(*ap, unsigned short *);
            unsigned long parsed = PyLong_AsUnsignedLongMask(value);
            if (parsed == ULONG_MAX && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = (unsigned short)parsed;
            }
            break;
        }
        case 'i': {
            int *output = va_arg(*ap, int *);
            long parsed = PyLong_AsLong(value);
            if (parsed == -1 && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (parsed < INT_MIN || parsed > INT_MAX) {
                PyErr_SetString(
                    (void *)&PyExc_OverflowError,
                    parsed < INT_MIN ? "signed integer is less than minimum" : "signed integer is greater than maximum"
                );
                return 0;
            }
            if (output != NULL) {
                *output = (int)parsed;
            }
            break;
        }
        case 'I': {
            unsigned int *output = va_arg(*ap, unsigned int *);
            unsigned long parsed = PyLong_AsUnsignedLongMask(value);
            if (parsed == ULONG_MAX && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = (unsigned int)parsed;
            }
            break;
        }
        case 'k': {
            unsigned long *output = va_arg(*ap, unsigned long *);
            unsigned long parsed = PyLong_AsUnsignedLongMask(value);
            if (parsed == ULONG_MAX && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = parsed;
            }
            break;
        }
        case 'l': {
            long *output = va_arg(*ap, long *);
            long parsed = PyLong_AsLong(value);
            if (parsed == -1 && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = parsed;
            }
            break;
        }
        case 'n': {
            Py_ssize_t *output = va_arg(*ap, Py_ssize_t *);
            Py_ssize_t parsed = PyLong_AsSsize_t(value);
            if (parsed == -1 && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = parsed;
            }
            break;
        }
        case 'L': {
            long long *output = va_arg(*ap, long long *);
            long long parsed = PyLong_AsLongLong(value);
            if (parsed == -1 && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = parsed;
            }
            break;
        }
        case 'K': {
            unsigned long long *output = va_arg(*ap, unsigned long long *);
            unsigned long long parsed = PyLong_AsUnsignedLongLongMask(value);
            if (parsed == ULLONG_MAX && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = parsed;
            }
            break;
        }
        case 'f': {
            float *output = va_arg(*ap, float *);
            double parsed = PyFloat_AsDouble(value);
            if (parsed == -1.0 && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = (float)parsed;
            }
            break;
        }
        case 'd': {
            double *output = va_arg(*ap, double *);
            double parsed = PyFloat_AsDouble(value);
            if (parsed == -1.0 && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = parsed;
            }
            break;
        }
        case 'D': {
            Py_complex *output = va_arg(*ap, Py_complex *);
            Py_complex parsed = PyComplex_AsCComplex(value);
            if (parsed.real == -1.0 && parsed.imag == 0.0 && PyErr_Occurred() != NULL) {
                return 0;
            }
            if (output != NULL) {
                *output = parsed;
            }
            break;
        }
        case 'c': {
            char *output = va_arg(*ap, char *);
            char *data = NULL;
            Py_ssize_t len = 0;
            if (object_is_instance_of_type(value, (void *)&PyBytes_Type)) {
                if (!pyrs_get_bytes_data(value, &data, &len)) {
                    return 0;
                }
            } else if (object_is_instance_of_type(value, (void *)&PyByteArray_Type)) {
                if (!pyrs_get_bytearray_data(value, &data, &len)) {
                    return 0;
                }
            } else {
                PyErr_SetString((void *)&PyExc_TypeError, "expected a byte string of length 1");
                return 0;
            }
            if (len != 1) {
                PyErr_SetString((void *)&PyExc_TypeError, "expected a byte string of length 1");
                return 0;
            }
            if (output != NULL) {
                *output = data[0];
            }
            break;
        }
        case 'C': {
            int *output = va_arg(*ap, int *);
            const char *data = NULL;
            Py_ssize_t len = 0;
            int codepoint = 0;
            if (!object_is_instance_of_type(value, (void *)&PyUnicode_Type)) {
                PyErr_SetString((void *)&PyExc_TypeError, "expected a unicode character");
                return 0;
            }
            if (PyUnicode_GetLength(value) != 1) {
                PyErr_SetString((void *)&PyExc_TypeError, "expected a unicode character");
                return 0;
            }
            if (!pyrs_get_unicode_utf8_data(value, &data, &len) ||
                !pyrs_decode_utf8_codepoint(data, len, &codepoint)) {
                PyErr_SetString((void *)&PyExc_TypeError, "expected a unicode character");
                return 0;
            }
            if (output != NULL) {
                *output = codepoint;
            }
            break;
        }
        case 's':
        case 'z': {
            char suffix = *format;
            if (suffix == '*') {
                PyrsPyBuffer *output = va_arg(*ap, PyrsPyBuffer *);
                if (token == 'z' && value == (void *)&_Py_NoneStruct) {
                    pyrs_zero_buffer(output);
                    if (PyBuffer_FillInfo(output, NULL, NULL, 0, 1, PYBUF_SIMPLE) != 0) {
                        return 0;
                    }
                } else if (object_is_instance_of_type(value, (void *)&PyUnicode_Type)) {
                    if (!pyrs_fill_unicode_buffer(value, output)) {
                        return 0;
                    }
                } else if (!pyrs_get_simple_buffer(value, output)) {
                    return 0;
                }
                if (!pyrs_push_buffer_cleanup(cleanups, cleanup_count, output)) {
                    PyBuffer_Release(output);
                    return 0;
                }
                format++;
            } else if (suffix == '#') {
                char **output = va_arg(*ap, char **);
                Py_ssize_t *output_len = va_arg(*ap, Py_ssize_t *);
                if (token == 'z' && value == (void *)&_Py_NoneStruct) {
                    if (output != NULL) {
                        *output = NULL;
                    }
                    if (output_len != NULL) {
                        *output_len = 0;
                    }
                } else if (object_is_instance_of_type(value, (void *)&PyUnicode_Type)) {
                    const char *text = NULL;
                    Py_ssize_t len = 0;
                    if (!pyrs_get_unicode_utf8_data(value, &text, &len)) {
                        return 0;
                    }
                    if (output != NULL) {
                        *output = (char *)text;
                    }
                    if (output_len != NULL) {
                        *output_len = len;
                    }
                } else if (object_is_instance_of_type(value, (void *)&PyBytes_Type)) {
                    char *text = NULL;
                    Py_ssize_t len = 0;
                    if (!pyrs_get_bytes_data(value, &text, &len)) {
                        return 0;
                    }
                    if (output != NULL) {
                        *output = text;
                    }
                    if (output_len != NULL) {
                        *output_len = len;
                    }
                } else {
                    PyErr_SetString(
                        (void *)&PyExc_TypeError,
                        token == 'z' ? "expected str or None" : "expected str or bytes"
                    );
                    return 0;
                }
                format++;
            } else {
                char **output = va_arg(*ap, char **);
                if (token == 'z' && value == (void *)&_Py_NoneStruct) {
                    if (output != NULL) {
                        *output = NULL;
                    }
                } else if (object_is_instance_of_type(value, (void *)&PyUnicode_Type)) {
                    const char *text = NULL;
                    Py_ssize_t len = 0;
                    if (!pyrs_get_unicode_utf8_data(value, &text, &len)) {
                        return 0;
                    }
                    if (memchr(text, '\0', (size_t)len) != NULL) {
                        PyErr_SetString((void *)&PyExc_ValueError, "embedded null character");
                        return 0;
                    }
                    if (output != NULL) {
                        *output = (char *)text;
                    }
                } else {
                    PyErr_SetString(
                        (void *)&PyExc_TypeError,
                        token == 'z' ? "expected str or None" : "expected str"
                    );
                    return 0;
                }
            }
            break;
        }
        case 'y': {
            char suffix = *format;
            if (suffix == '*') {
                PyrsPyBuffer *output = va_arg(*ap, PyrsPyBuffer *);
                if (!pyrs_get_simple_buffer(value, output)) {
                    return 0;
                }
                if (!pyrs_push_buffer_cleanup(cleanups, cleanup_count, output)) {
                    PyBuffer_Release(output);
                    return 0;
                }
                format++;
            } else if (suffix == '#') {
                char **output = va_arg(*ap, char **);
                Py_ssize_t *output_len = va_arg(*ap, Py_ssize_t *);
                char *text = NULL;
                Py_ssize_t len = 0;
                if (!object_is_instance_of_type(value, (void *)&PyBytes_Type) ||
                    !pyrs_get_bytes_data(value, &text, &len)) {
                    PyErr_SetString((void *)&PyExc_TypeError, "expected bytes");
                    return 0;
                }
                if (output != NULL) {
                    *output = text;
                }
                if (output_len != NULL) {
                    *output_len = len;
                }
                format++;
            } else {
                char **output = va_arg(*ap, char **);
                char *text = NULL;
                Py_ssize_t len = 0;
                if (!object_is_instance_of_type(value, (void *)&PyBytes_Type) ||
                    !pyrs_get_bytes_data(value, &text, &len)) {
                    PyErr_SetString((void *)&PyExc_TypeError, "expected bytes");
                    return 0;
                }
                if (memchr(text, '\0', (size_t)len) != NULL) {
                    PyErr_SetString((void *)&PyExc_ValueError, "embedded null byte");
                    return 0;
                }
                if (output != NULL) {
                    *output = text;
                }
            }
            break;
        }
        case 'e': {
            const char *encoding = (const char *)va_arg(*ap, void *);
            int recode_strings = 0;
            if (*format == 's') {
                recode_strings = 1;
            } else if (*format == 't') {
                recode_strings = 0;
            } else {
                PyErr_Format(
                    (void *)&PyExc_SystemError,
                    "argument %zd (impossible<bad format char>)",
                    argument_index
                );
                return 0;
            }
            char **buffer = va_arg(*ap, char **);
            format++;
            if (*format == '#') {
                Py_ssize_t *buffer_len = va_arg(*ap, Py_ssize_t *);
                if (!pyrs_convert_encoded_string(
                        value,
                        recode_strings,
                        encoding,
                        buffer,
                        buffer_len,
                        1,
                        cleanups,
                        cleanup_count
                    )) {
                    return 0;
                }
                format++;
            } else if (!pyrs_convert_encoded_string(
                           value,
                           recode_strings,
                           encoding,
                           buffer,
                           NULL,
                           0,
                           cleanups,
                           cleanup_count
                       )) {
                return 0;
            }
            break;
        }
        case 'w': {
            if (*format != '*') {
                PyErr_Format(
                    (void *)&PyExc_SystemError,
                    "argument %zd (impossible<bad format char>)",
                    argument_index
                );
                return 0;
            }
            PyrsPyBuffer *output = va_arg(*ap, PyrsPyBuffer *);
            if (!pyrs_get_writable_buffer(value, output)) {
                PyErr_Clear();
                PyErr_SetString((void *)&PyExc_TypeError, "expected read-write bytes-like object");
                return 0;
            }
            if (!pyrs_push_buffer_cleanup(cleanups, cleanup_count, output)) {
                PyBuffer_Release(output);
                return 0;
            }
            format++;
            break;
        }
        case '(':
            if (!pyrs_parse_nested_tuple_item(value, argument_index, format - 1, &format, ap)) {
                return 0;
            }
            format++;
            break;
        case 'O':
            if (*format == '!') {
                void *expected_type = va_arg(*ap, void *);
                void **output = va_arg(*ap, void **);
                if (!object_is_instance_of_type(value, expected_type)) {
                    PyErr_SetString(
                        (void *)&PyExc_TypeError,
                        "PyArg_ParseTupleAndKeywords argument has incorrect type"
                    );
                    return 0;
                }
                if (output != NULL) {
                    *output = value;
                }
                format++;
            } else if (*format == '&') {
                arg_converter_fn converter = va_arg(*ap, arg_converter_fn);
                void *output = va_arg(*ap, void *);
                int converted = converter != NULL ? converter(value, output) : 0;
                if (converter == NULL || converted == 0) {
                    if (PyErr_Occurred() == NULL) {
                        pyrs_capi_set_error_message("PyArg_ParseTupleAndKeywords converter failed");
                    }
                    return 0;
                }
                if (converted == PYRS_PY_CLEANUP_SUPPORTED &&
                    !pyrs_push_converter_cleanup(cleanups, cleanup_count, converter, output)) {
                    return 0;
                }
                format++;
            } else {
                void **output = va_arg(*ap, void **);
                if (output != NULL) {
                    *output = value;
                }
            }
            break;
        case 'S': {
            void **output = va_arg(*ap, void **);
            if (!object_is_instance_of_type(value, (void *)&PyBytes_Type)) {
                PyErr_SetString((void *)&PyExc_TypeError, "expected bytes");
                return 0;
            }
            if (output != NULL) {
                *output = value;
            }
            break;
        }
        case 'Y': {
            void **output = va_arg(*ap, void **);
            if (!object_is_instance_of_type(value, (void *)&PyByteArray_Type)) {
                PyErr_SetString((void *)&PyExc_TypeError, "expected bytearray");
                return 0;
            }
            if (output != NULL) {
                *output = value;
            }
            break;
        }
        case 'U': {
            void **output = va_arg(*ap, void **);
            if (!object_is_instance_of_type(value, (void *)&PyUnicode_Type)) {
                PyErr_SetString((void *)&PyExc_TypeError, "expected str");
                return 0;
            }
            if (output != NULL) {
                *output = value;
            }
            break;
        }
        default:
            PyErr_Format(
                (void *)&PyExc_SystemError,
                "argument %zd (impossible<bad format char>)",
                argument_index
            );
            return 0;
    }

    *p_format = format;
    return 1;
}

static int parse_args_and_keywords_va(
    void *args,
    void *kwargs,
    const char *format,
    const char *const *keywords,
    va_list *ap
)
{
    if (getenv("PYRS_TRACE_PYARG_VA") != NULL) {
        const char *fmt = (format == NULL) ? "<null>" : format;
        fprintf(
            stderr,
            "[pyarg-va] parse args=%p kwargs=%p format=%p keywords=%p spec=\"%.80s\"\n",
            args,
            kwargs,
            (const void *)format,
            (const void *)keywords,
            fmt
        );
    }
    if (format == NULL) {
        pyrs_capi_set_error_message("PyArg_ParseTupleAndKeywords received null format");
        return 0;
    }

    const char *cursor = format;
    const char *function_name = "function";
    while (*cursor != '\0' && *cursor != ':' && *cursor != ';') {
        cursor++;
    }
    if (*cursor == ':' && cursor[1] != '\0') {
        function_name = cursor + 1;
    }
    size_t format_len = (size_t)(cursor - format);
    char *spec = (char *)malloc(format_len + 1);
    if (spec == NULL) {
        pyrs_capi_set_error_message("PyArg_ParseTupleAndKeywords failed allocating format copy");
        return 0;
    }
    memcpy(spec, format, format_len);
    spec[format_len] = '\0';

    if (args != NULL && !is_tuple_object(args)) {
        free(spec);
        PyErr_BadInternalCall();
        return 0;
    }
    if (kwargs != NULL && !object_is_instance_of_type(kwargs, (void *)&PyDict_Type)) {
        free(spec);
        PyErr_BadInternalCall();
        return 0;
    }

    Py_ssize_t positional_total = args == NULL ? 0 : PyTuple_Size(args);
    if (positional_total < 0) {
        free(spec);
        return 0;
    }
    PyArgCleanupEntry cleanups[16];
    size_t cleanup_count = 0;

    if (keywords == NULL) {
        int optional = 0;
        Py_ssize_t positional_index = 0;
        Py_ssize_t arg_index = 0;
        const char *format_cursor = spec;
        while (!PYRS_IS_END_OF_FORMAT(*format_cursor)) {
            if (*format_cursor == ' ' || *format_cursor == '\t' || *format_cursor == ',') {
                format_cursor++;
                continue;
            }
            if (*format_cursor == '|') {
                optional = 1;
                format_cursor++;
                continue;
            }
            if (*format_cursor == '$') {
                format_cursor++;
                continue;
            }
            arg_index++;
            if (positional_index < positional_total) {
                void *value = PyTuple_GetItem(args, positional_index++);
                if (!pyrs_convert_value_for_format_unit(
                        value,
                        &format_cursor,
                        ap,
                        arg_index,
                        cleanups,
                        &cleanup_count
                    )) {
                    pyrs_run_arg_cleanups(cleanups, cleanup_count);
                    free(spec);
                    return 0;
                }
                continue;
            }
            if (!optional) {
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                PyErr_Format(
                    (void *)&PyExc_TypeError,
                    "%s missing required argument '' (pos %zd)",
                    function_name,
                    arg_index
                );
                return 0;
            }
            const char *remaining = format_cursor;
            const char *msg = pyrs_skip_format_unit(&format_cursor, ap);
            if (msg != NULL) {
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                PyErr_Format((void *)&PyExc_SystemError, "%s: '%s'", msg, remaining);
                free(spec);
                return 0;
            }
        }
        if (positional_index < positional_total) {
            pyrs_run_arg_cleanups(cleanups, cleanup_count);
            free(spec);
            PyErr_Format(
                (void *)&PyExc_TypeError,
                "%s takes at most %zd positional argument%s (%zd given)",
                function_name,
                arg_index,
                arg_index == 1 ? "" : "s",
                positional_total
            );
            return 0;
        }
        free(spec);
        return 1;
    }

    Py_ssize_t positional_only = 0;
    Py_ssize_t total = 0;
    if (!pyrs_scan_keywords(keywords, &total, &positional_only)) {
        free(spec);
        return 0;
    }

    Py_ssize_t keyword_count = kwargs == NULL ? 0 : PyDict_Size(kwargs);
    if (keyword_count < 0) {
        free(spec);
        return 0;
    }
    if (positional_total + keyword_count > total) {
        free(spec);
        PyErr_Format(
            (void *)&PyExc_TypeError,
            "%s takes at most %zd %sargument%s (%zd given)",
            function_name,
            total,
            positional_total == 0 ? "keyword " : "",
            total == 1 ? "" : "s",
            positional_total + keyword_count
        );
        return 0;
    }

    int skip = 0;
    Py_ssize_t min_required = PY_SSIZE_T_MAX;
    Py_ssize_t max_positional = PY_SSIZE_T_MAX;
    Py_ssize_t i = 0;
    Py_ssize_t remaining_keywords = keyword_count;
    const char *format_cursor = spec;
    for (i = 0; i < total; i++) {
        pyrs_skip_ignorable_format_chars(&format_cursor);
        if (*format_cursor == '|') {
            if (min_required != PY_SSIZE_T_MAX) {
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                PyErr_SetString(
                    (void *)&PyExc_SystemError,
                    "Invalid format string (| specified twice)"
                );
                return 0;
            }
            min_required = i;
            format_cursor++;
            pyrs_skip_ignorable_format_chars(&format_cursor);
            if (max_positional != PY_SSIZE_T_MAX) {
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                PyErr_SetString(
                    (void *)&PyExc_SystemError,
                    "Invalid format string ($ before |)"
                );
                return 0;
            }
        }
        if (*format_cursor == '$') {
            if (max_positional != PY_SSIZE_T_MAX) {
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                PyErr_SetString(
                    (void *)&PyExc_SystemError,
                    "Invalid format string ($ specified twice)"
                );
                return 0;
            }
            max_positional = i;
            format_cursor++;
            pyrs_skip_ignorable_format_chars(&format_cursor);
            if (skip) {
                break;
            }
            if (max_positional < positional_only) {
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                PyErr_SetString((void *)&PyExc_SystemError, "Empty parameter name after $");
                return 0;
            }
            if (max_positional < positional_total) {
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                if (max_positional == 0) {
                    PyErr_Format(
                        (void *)&PyExc_TypeError,
                        "%s takes no positional arguments",
                        function_name
                    );
                } else {
                    PyErr_Format(
                        (void *)&PyExc_TypeError,
                        "%s takes %s %zd positional argument%s (%zd given)",
                        function_name,
                        min_required != PY_SSIZE_T_MAX ? "at most" : "exactly",
                        max_positional,
                        max_positional == 1 ? "" : "s",
                        positional_total
                    );
                }
                return 0;
            }
        }
        if (PYRS_IS_END_OF_FORMAT(*format_cursor)) {
            pyrs_run_arg_cleanups(cleanups, cleanup_count);
            free(spec);
            PyErr_Format(
                (void *)&PyExc_SystemError,
                "More keyword list entries (%zd) than format specifiers (%zd)",
                total,
                i
            );
            return 0;
        }

        if (!skip) {
            void *current_arg = NULL;
            if (i < positional_total) {
                current_arg = PyTuple_GetItem(args, i);
            } else if (remaining_keywords > 0 && i >= positional_only) {
                if (!pyrs_lookup_keyword_value(kwargs, keywords[i], &current_arg)) {
                    pyrs_run_arg_cleanups(cleanups, cleanup_count);
                    free(spec);
                    return 0;
                }
                if (current_arg != NULL) {
                    remaining_keywords--;
                }
            }

            if (current_arg != NULL) {
                if (!pyrs_convert_value_for_format_unit(
                        current_arg,
                        &format_cursor,
                        ap,
                        i + 1,
                        cleanups,
                        &cleanup_count
                    )) {
                    pyrs_run_arg_cleanups(cleanups, cleanup_count);
                    free(spec);
                    return 0;
                }
                continue;
            }

            if (i < min_required) {
                if (i < positional_only) {
                    skip = 1;
                } else {
                    pyrs_run_arg_cleanups(cleanups, cleanup_count);
                    free(spec);
                    PyErr_Format(
                        (void *)&PyExc_TypeError,
                        "%s missing required argument '%s' (pos %zd)",
                        function_name,
                        keywords[i],
                        i + 1
                    );
                    return 0;
                }
            }

            if (remaining_keywords == 0 && !skip) {
                free(spec);
                return 1;
            }
        }

        const char *remaining = format_cursor;
        const char *msg = pyrs_skip_format_unit(&format_cursor, ap);
        if (msg != NULL) {
            pyrs_run_arg_cleanups(cleanups, cleanup_count);
            PyErr_Format((void *)&PyExc_SystemError, "%s: '%s'", msg, remaining);
            free(spec);
            return 0;
        }
    }

    if (skip) {
        Py_ssize_t required = positional_only < min_required
            ? positional_only
            : min_required;
        pyrs_run_arg_cleanups(cleanups, cleanup_count);
        free(spec);
        PyErr_Format(
            (void *)&PyExc_TypeError,
            "%s takes %s %zd positional argument%s (%zd given)",
            function_name,
            required < i ? "at least" : "exactly",
            required,
            required == 1 ? "" : "s",
            positional_total
        );
        return 0;
    }

    pyrs_skip_ignorable_format_chars(&format_cursor);
    if (!PYRS_IS_END_OF_FORMAT(*format_cursor) &&
        *format_cursor != '|' &&
        *format_cursor != '$') {
        pyrs_run_arg_cleanups(cleanups, cleanup_count);
        PyErr_Format(
            (void *)&PyExc_SystemError,
            "more argument specifiers than keyword list entries (remaining format:'%s')",
            format_cursor
        );
        free(spec);
        return 0;
    }

    if (remaining_keywords > 0) {
        for (Py_ssize_t dup_index = positional_only;
             dup_index < positional_total && dup_index < total;
             dup_index++) {
            void *duplicate = NULL;
            if (!pyrs_lookup_keyword_value(kwargs, keywords[dup_index], &duplicate)) {
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                return 0;
            }
            if (duplicate != NULL) {
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                PyErr_Format(
                    (void *)&PyExc_TypeError,
                    "argument for %s given by name ('%s') and position (%zd)",
                    function_name,
                    keywords[dup_index],
                    dup_index + 1
                );
                return 0;
            }
        }

        void *keys = PyDict_Keys(kwargs);
        if (keys == NULL) {
            pyrs_run_arg_cleanups(cleanups, cleanup_count);
            free(spec);
            return 0;
        }
        Py_ssize_t key_count = PyList_Size(keys);
        if (key_count < 0) {
            Py_DecRef(keys);
            pyrs_run_arg_cleanups(cleanups, cleanup_count);
            free(spec);
            return 0;
        }
        for (Py_ssize_t key_index = 0; key_index < key_count; key_index++) {
            void *key_obj = PyList_GetItem(keys, key_index);
            if (key_obj == NULL) {
                Py_DecRef(keys);
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                return 0;
            }
            if (!object_is_instance_of_type(key_obj, (void *)&PyUnicode_Type)) {
                Py_DecRef(keys);
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                PyErr_SetString((void *)&PyExc_TypeError, "keywords must be strings");
                return 0;
            }
            int matched = 0;
            for (Py_ssize_t keyword_index = positional_only;
                 keyword_index < total;
                 keyword_index++) {
                int eq = PyUnicode_EqualToUTF8(key_obj, keywords[keyword_index]);
                if (eq < 0) {
                    Py_DecRef(keys);
                    pyrs_run_arg_cleanups(cleanups, cleanup_count);
                    free(spec);
                    return 0;
                }
                if (eq > 0) {
                    matched = 1;
                    break;
                }
            }
            if (!matched) {
                Py_DecRef(keys);
                pyrs_run_arg_cleanups(cleanups, cleanup_count);
                free(spec);
                PyErr_Format(
                    (void *)&PyExc_TypeError,
                    "this function got an unexpected keyword argument '%S'",
                    key_obj
                );
                return 0;
            }
        }
        Py_DecRef(keys);
    }

    free(spec);
    return 1;
}

int PyArg_Parse(void *args, const char *format, ...)
{
    if (format == NULL) {
        pyrs_capi_set_error_message("PyArg_Parse received null format");
        return 0;
    }

    int min_count = 0;
    int max_count = 0;
    count_old_style_format_args(format, &min_count, &max_count);

    va_list ap;
    va_start(ap, format);
    int result = 0;

    if (max_count == 0) {
        if (args == NULL) {
            result = 1;
        } else {
            pyrs_capi_set_error_message("function takes no arguments");
            result = 0;
        }
        va_end(ap);
        return result;
    }

    if (min_count == 1 && max_count == 1) {
        if (args == NULL) {
            pyrs_capi_set_error_message("function takes at least one argument");
            va_end(ap);
            return 0;
        }
        void *single = PyTuple_New(1);
        if (single == NULL) {
            va_end(ap);
            return 0;
        }
        Py_IncRef(args);
        if (PyTuple_SetItem(single, 0, args) != 0) {
            Py_DecRef(args);
            Py_DecRef(single);
            va_end(ap);
            return 0;
        }
        result = parse_args_and_keywords_va(single, NULL, format, NULL, &ap);
        Py_DecRef(single);
        va_end(ap);
        return result;
    }

    pyrs_capi_set_error_message("old style getargs format uses new features");
    va_end(ap);
    return 0;
}

int _PyArg_Parse_SizeT(void *args, const char *format, ...)
{
    if (format == NULL) {
        pyrs_capi_set_error_message("_PyArg_Parse_SizeT received null format");
        return 0;
    }

    int min_count = 0;
    int max_count = 0;
    count_old_style_format_args(format, &min_count, &max_count);

    va_list ap;
    va_start(ap, format);
    int result = 0;

    if (max_count == 0) {
        if (args == NULL) {
            result = 1;
        } else {
            pyrs_capi_set_error_message("function takes no arguments");
            result = 0;
        }
        va_end(ap);
        return result;
    }

    if (min_count == 1 && max_count == 1) {
        if (args == NULL) {
            pyrs_capi_set_error_message("function takes at least one argument");
            va_end(ap);
            return 0;
        }
        void *single = PyTuple_New(1);
        if (single == NULL) {
            va_end(ap);
            return 0;
        }
        Py_IncRef(args);
        if (PyTuple_SetItem(single, 0, args) != 0) {
            Py_DecRef(args);
            Py_DecRef(single);
            va_end(ap);
            return 0;
        }
        result = parse_args_and_keywords_va(single, NULL, format, NULL, &ap);
        Py_DecRef(single);
        va_end(ap);
        return result;
    }

    pyrs_capi_set_error_message("old style getargs format uses new features");
    va_end(ap);
    return 0;
}

int PyArg_VaParse(void *args, const char *format, va_list va)
{
    if (!is_tuple_object(args)) {
        pyrs_capi_set_error_message("new style getargs format but argument is not a tuple");
        return 0;
    }
    va_list lva;
    va_copy(lva, va);
    int result = parse_args_and_keywords_va(args, NULL, format, NULL, &lva);
    va_end(lva);
    return result;
}

int _PyArg_VaParse_SizeT(void *args, const char *format, va_list va)
{
    if (!is_tuple_object(args)) {
        pyrs_capi_set_error_message("new style getargs format but argument is not a tuple");
        return 0;
    }
    va_list lva;
    va_copy(lva, va);
    int result = parse_args_and_keywords_va(args, NULL, format, NULL, &lva);
    va_end(lva);
    return result;
}

int PyArg_ValidateKeywordArguments(void *kwargs)
{
    if (!object_is_instance_of_type(kwargs, (void *)&PyDict_Type)) {
        PyErr_BadInternalCall();
        return 0;
    }
    void *keys = PyDict_Keys(kwargs);
    if (keys == NULL) {
        return 0;
    }
    Py_ssize_t count = PyList_Size(keys);
    if (count < 0) {
        Py_DecRef(keys);
        return 0;
    }
    for (Py_ssize_t i = 0; i < count; i++) {
        void *key = PyList_GetItem(keys, i);
        if (!object_is_instance_of_type(key, (void *)&PyUnicode_Type)) {
            Py_DecRef(keys);
            pyrs_capi_set_error_message("keywords must be strings");
            return 0;
        }
    }
    Py_DecRef(keys);
    return 1;
}

int PyArg_ParseTuple(void *args, const char *format, ...)
{
    if (getenv("PYRS_TRACE_PYARG_VA") != NULL) {
        const char *fmt = (format == NULL) ? "<null>" : format;
        fprintf(
            stderr,
            "[pyarg-va] PyArg_ParseTuple args=%p format=%p spec=\"%.80s\"\n",
            args,
            (const void *)format,
            fmt
        );
    }
    va_list ap;
    va_start(ap, format);
    int result = parse_args_and_keywords_va(args, NULL, format, NULL, &ap);
    va_end(ap);
    return result;
}

int _PyArg_ParseTuple_SizeT(void *args, const char *format, ...)
{
    if (getenv("PYRS_TRACE_PYARG_VA") != NULL) {
        const char *fmt = (format == NULL) ? "<null>" : format;
        fprintf(
            stderr,
            "[pyarg-va] _PyArg_ParseTuple_SizeT args=%p format=%p spec=\"%.80s\"\n",
            args,
            (const void *)format,
            fmt
        );
    }
    va_list ap;
    va_start(ap, format);
    int result = parse_args_and_keywords_va(args, NULL, format, NULL, &ap);
    va_end(ap);
    return result;
}

int PyArg_ParseTupleAndKeywords(
    void *args,
    void *kwargs,
    const char *format,
    const char *const *keywords,
    ...
)
{
    if (getenv("PYRS_TRACE_PYARG_VA") != NULL) {
        const char *fmt = (format == NULL) ? "<null>" : format;
        fprintf(
            stderr,
            "[pyarg-va] PyArg_ParseTupleAndKeywords args=%p kwargs=%p format=%p keywords=%p spec=\"%.80s\"\n",
            args,
            kwargs,
            (const void *)format,
            (const void *)keywords,
            fmt
        );
    }
    va_list ap;
    va_start(ap, keywords);
    int result = parse_args_and_keywords_va(args, kwargs, format, keywords, &ap);
    va_end(ap);
    return result;
}

int _PyArg_ParseTupleAndKeywords_SizeT(
    void *args,
    void *kwargs,
    const char *format,
    const char *const *keywords,
    ...
)
{
    if (getenv("PYRS_TRACE_PYARG_VA") != NULL) {
        const char *fmt = (format == NULL) ? "<null>" : format;
        fprintf(
            stderr,
            "[pyarg-va] _PyArg_ParseTupleAndKeywords_SizeT args=%p kwargs=%p format=%p keywords=%p spec=\"%.80s\"\n",
            args,
            kwargs,
            (const void *)format,
            (const void *)keywords,
            fmt
        );
    }
    va_list ap;
    va_start(ap, keywords);
    int result = parse_args_and_keywords_va(args, kwargs, format, keywords, &ap);
    va_end(ap);
    return result;
}

int PyArg_VaParseTupleAndKeywords(
    void *args,
    void *kwargs,
    const char *format,
    const char *const *keywords,
    va_list va
)
{
    if (getenv("PYRS_TRACE_PYARG_VA") != NULL) {
        const char *fmt = (format == NULL) ? "<null>" : format;
        fprintf(
            stderr,
            "[pyarg-va] PyArg_VaParseTupleAndKeywords args=%p kwargs=%p format=%p keywords=%p spec=\"%.80s\"\n",
            args,
            kwargs,
            (const void *)format,
            (const void *)keywords,
            fmt
        );
    }
    va_list lva;
    va_copy(lva, va);
    int result = parse_args_and_keywords_va(args, kwargs, format, keywords, &lva);
    va_end(lva);
    return result;
}

int _PyArg_VaParseTupleAndKeywords_SizeT(
    void *args,
    void *kwargs,
    const char *format,
    const char *const *keywords,
    va_list va
)
{
    if (getenv("PYRS_TRACE_PYARG_VA") != NULL) {
        const char *fmt = (format == NULL) ? "<null>" : format;
        fprintf(
            stderr,
            "[pyarg-va] _PyArg_VaParseTupleAndKeywords_SizeT args=%p kwargs=%p format=%p keywords=%p spec=\"%.80s\"\n",
            args,
            kwargs,
            (const void *)format,
            (const void *)keywords,
            fmt
        );
    }
    va_list lva;
    va_copy(lva, va);
    int result = parse_args_and_keywords_va(args, kwargs, format, keywords, &lva);
    va_end(lva);
    return result;
}

typedef struct {
    uint8_t v;
} _PyOnceFlag;

typedef struct _PyArg_Parser {
    const char *format;
    const char * const *keywords;
    const char *fname;
    const char *custom_msg;
    _PyOnceFlag once;
    int is_kwtuple_owned;
    int pos;
    int min;
    int max;
    void *kwtuple;
    struct _PyArg_Parser *next;
} _PyArg_Parser;

typedef struct {
    void *buffer;
    void *data;
    int kind;
    uint32_t maxchar;
    Py_ssize_t size;
    Py_ssize_t pos;
    Py_ssize_t min_length;
    uint32_t min_char;
    unsigned char overallocate;
    unsigned char readonly;
} _PyUnicodeWriter;

typedef struct _Py_slist_item_s {
    struct _Py_slist_item_s *next;
} _Py_slist_item_t;

typedef struct {
    _Py_slist_item_t *head;
} _Py_slist_t;

typedef struct _Py_hashtable_entry_t {
    _Py_slist_item_t _Py_slist_item;
    Py_uhash_t key_hash;
    const void *key;
    void *value;
} _Py_hashtable_entry_t;

struct _Py_hashtable_t;
typedef struct _Py_hashtable_t _Py_hashtable_t;

typedef Py_uhash_t (*_Py_hashtable_hash_func)(const void *key);
typedef int (*_Py_hashtable_compare_func)(const void *key1, const void *key2);
typedef void (*_Py_hashtable_destroy_func)(void *key);
typedef _Py_hashtable_entry_t* (*_Py_hashtable_get_entry_func)(
    _Py_hashtable_t *ht,
    const void *key
);

typedef struct {
    void* (*malloc)(size_t size);
    void (*free)(void *ptr);
} _Py_hashtable_allocator_t;

struct _Py_hashtable_t {
    size_t nentries;
    size_t nbuckets;
    _Py_slist_t *buckets;

    _Py_hashtable_get_entry_func get_entry_func;
    _Py_hashtable_hash_func hash_func;
    _Py_hashtable_compare_func compare_func;
    _Py_hashtable_destroy_func key_destroy_func;
    _Py_hashtable_destroy_func value_destroy_func;
    _Py_hashtable_allocator_t alloc;
};

typedef struct {
    uint8_t bits_per_digit;
    uint8_t digit_size;
    int8_t digits_order;
    int8_t digit_endianness;
} PyLongLayout;

typedef struct {
    int64_t value;
    uint8_t negative;
    Py_ssize_t ndigits;
    const void *digits;
    uintptr_t _reserved;
} PyLongExport;

typedef struct {
    int negative;
    Py_ssize_t ndigits;
    uint32_t *digits;
} _PyLongWriter;

static int pyrs_lookup_keyword_index(const char * const *keywords, const char *name)
{
    if (keywords == NULL || name == NULL) {
        return -1;
    }
    for (int i = 0; keywords[i] != NULL; i++) {
        if (strcmp(keywords[i], name) == 0) {
            return i;
        }
    }
    return -1;
}

static int pyrs_keyword_count(const char * const *keywords)
{
    if (keywords == NULL) {
        return 0;
    }
    int count = 0;
    while (keywords[count] != NULL) {
        count++;
    }
    return count;
}

int _PyArg_NoKeywords(const char *funcname, void *kwargs)
{
    if (kwargs == NULL) {
        return 1;
    }
    Py_ssize_t size = PyDict_Size(kwargs);
    if (size <= 0) {
        return 1;
    }
    if (funcname == NULL) {
        funcname = "<function>";
    }
    PyErr_Format((void *)&PyExc_TypeError, "%s() takes no keyword arguments", funcname);
    return 0;
}

static int object_type_is_exact(void *object, void *type_obj)
{
    if (object == NULL || type_obj == NULL) {
        return 0;
    }
    PyObjectHeadCompat *head = (PyObjectHeadCompat *)object;
    return head->ob_type == type_obj;
}

void *pyrs_testcapi_get_args(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("get_args", kwargs)) {
        return NULL;
    }
    if (args == NULL) {
        return build_none();
    }
    Py_IncRef(args);
    return args;
}

void *pyrs_testcapi_get_kwargs(void *args, void *kwargs)
{
    (void)args;
    if (kwargs == NULL) {
        return build_none();
    }
    Py_IncRef(kwargs);
    return kwargs;
}

void *pyrs_testcapi_getargs_empty(void *args, void *kwargs)
{
    Py_ssize_t positional = args == NULL ? 0 : PyTuple_Size(args);
    if (positional < 0) {
        return NULL;
    }
    if (positional != 0) {
        PyErr_Format(
            (void *)&PyExc_TypeError,
            "getargs_empty() takes exactly 0 arguments (%zd given)",
            positional
        );
        return NULL;
    }
    Py_ssize_t keyword_count = kwargs == NULL ? 0 : PyDict_Size(kwargs);
    if (keyword_count < 0) {
        return NULL;
    }
    if (keyword_count != 0) {
        PyErr_Format(
            (void *)&PyExc_TypeError,
            "getargs_empty() takes at most 0 keyword arguments (%zd given)",
            keyword_count
        );
        return NULL;
    }
    int result;
    if (kwargs != NULL && keyword_count > 0) {
        const char *kwlist[] = {NULL};
        result = PyArg_ParseTupleAndKeywords(args, kwargs, "|:getargs_empty", kwlist);
    } else {
        result = PyArg_ParseTuple(args, "|:getargs_empty");
    }
    if (!result) {
        return NULL;
    }
    return PyLong_FromLong((long)result);
}

void *pyrs_testcapi_getargs_tuple(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_tuple", kwargs)) {
        return NULL;
    }
    int a;
    int b;
    int c;
    if (!PyArg_ParseTuple(args, "i(ii)", &a, &b, &c)) {
        return NULL;
    }
    return Py_BuildValue("iii", a, b, c);
}

void *pyrs_testcapi_getargs_c(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_c", kwargs)) {
        return NULL;
    }
    char c;
    if (!PyArg_ParseTuple(args, "c", &c)) {
        return NULL;
    }
    return PyLong_FromLong((unsigned char)c);
}

void *pyrs_testcapi_getargs_C(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_C", kwargs)) {
        return NULL;
    }
    int c;
    if (!PyArg_ParseTuple(args, "C", &c)) {
        return NULL;
    }
    return PyLong_FromLong(c);
}

void *pyrs_testcapi_getargs_s(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_s", kwargs)) {
        return NULL;
    }
    char *text;
    if (!PyArg_ParseTuple(args, "s", &text)) {
        return NULL;
    }
    return PyBytes_FromStringAndSize(text, (Py_ssize_t)strlen(text));
}

void *pyrs_testcapi_getargs_s_star(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_s_star", kwargs)) {
        return NULL;
    }
    PyrsPyBuffer buffer;
    if (!PyArg_ParseTuple(args, "s*", &buffer)) {
        return NULL;
    }
    void *bytes = PyBytes_FromStringAndSize(buffer.buf, buffer.len);
    PyBuffer_Release(&buffer);
    return bytes;
}

void *pyrs_testcapi_getargs_s_hash(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_s_hash", kwargs)) {
        return NULL;
    }
    char *text;
    Py_ssize_t size;
    if (!PyArg_ParseTuple(args, "s#", &text, &size)) {
        return NULL;
    }
    return PyBytes_FromStringAndSize(text, size);
}

void *pyrs_testcapi_getargs_z(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_z", kwargs)) {
        return NULL;
    }
    char *text;
    if (!PyArg_ParseTuple(args, "z", &text)) {
        return NULL;
    }
    if (text == NULL) {
        return build_none();
    }
    return PyBytes_FromStringAndSize(text, (Py_ssize_t)strlen(text));
}

void *pyrs_testcapi_getargs_z_star(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_z_star", kwargs)) {
        return NULL;
    }
    PyrsPyBuffer buffer;
    if (!PyArg_ParseTuple(args, "z*", &buffer)) {
        return NULL;
    }
    void *result = NULL;
    if (buffer.buf != NULL) {
        result = PyBytes_FromStringAndSize(buffer.buf, buffer.len);
    } else {
        result = build_none();
    }
    PyBuffer_Release(&buffer);
    return result;
}

void *pyrs_testcapi_getargs_z_hash(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_z_hash", kwargs)) {
        return NULL;
    }
    char *text;
    Py_ssize_t size;
    if (!PyArg_ParseTuple(args, "z#", &text, &size)) {
        return NULL;
    }
    if (text == NULL) {
        return build_none();
    }
    return PyBytes_FromStringAndSize(text, size);
}

void *pyrs_testcapi_getargs_y(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_y", kwargs)) {
        return NULL;
    }
    char *text;
    if (!PyArg_ParseTuple(args, "y", &text)) {
        return NULL;
    }
    return PyBytes_FromStringAndSize(text, (Py_ssize_t)strlen(text));
}

void *pyrs_testcapi_getargs_y_star(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_y_star", kwargs)) {
        return NULL;
    }
    PyrsPyBuffer buffer;
    if (!PyArg_ParseTuple(args, "y*", &buffer)) {
        return NULL;
    }
    void *result = PyBytes_FromStringAndSize(buffer.buf, buffer.len);
    PyBuffer_Release(&buffer);
    return result;
}

void *pyrs_testcapi_getargs_y_hash(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_y_hash", kwargs)) {
        return NULL;
    }
    char *text;
    Py_ssize_t size;
    if (!PyArg_ParseTuple(args, "y#", &text, &size)) {
        return NULL;
    }
    return PyBytes_FromStringAndSize(text, size);
}

void *pyrs_testcapi_getargs_es(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_es", kwargs)) {
        return NULL;
    }
    void *arg;
    const char *encoding = NULL;
    char *text = NULL;
    if (!PyArg_ParseTuple(args, "O|s", &arg, &encoding)) {
        return NULL;
    }
    if (!PyArg_Parse(arg, "es", encoding, &text)) {
        return NULL;
    }
    void *result = PyBytes_FromStringAndSize(text, (Py_ssize_t)strlen(text));
    PyMem_Free(text);
    return result;
}

void *pyrs_testcapi_getargs_et(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_et", kwargs)) {
        return NULL;
    }
    void *arg;
    const char *encoding = NULL;
    char *text = NULL;
    if (!PyArg_ParseTuple(args, "O|s", &arg, &encoding)) {
        return NULL;
    }
    if (!PyArg_Parse(arg, "et", encoding, &text)) {
        return NULL;
    }
    void *result = PyBytes_FromStringAndSize(text, (Py_ssize_t)strlen(text));
    PyMem_Free(text);
    return result;
}

void *pyrs_testcapi_getargs_es_hash(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_es_hash", kwargs)) {
        return NULL;
    }
    void *arg;
    const char *encoding = NULL;
    void *buffer_obj = NULL;
    char *text = NULL;
    Py_ssize_t size = 0;
    if (!PyArg_ParseTuple(args, "O|sY", &arg, &encoding, &buffer_obj)) {
        return NULL;
    }
    if (buffer_obj != NULL) {
        text = PyByteArray_AsString(buffer_obj);
        size = PyByteArray_Size(buffer_obj);
        if (text == NULL || size < 0) {
            return NULL;
        }
    }
    if (!PyArg_Parse(arg, "es#", encoding, &text, &size)) {
        return NULL;
    }
    void *result = PyBytes_FromStringAndSize(text, size);
    if (buffer_obj == NULL) {
        PyMem_Free(text);
    }
    return result;
}

void *pyrs_testcapi_getargs_et_hash(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_et_hash", kwargs)) {
        return NULL;
    }
    void *arg;
    const char *encoding = NULL;
    void *buffer_obj = NULL;
    char *text = NULL;
    Py_ssize_t size = 0;
    if (!PyArg_ParseTuple(args, "O|sY", &arg, &encoding, &buffer_obj)) {
        return NULL;
    }
    if (buffer_obj != NULL) {
        text = PyByteArray_AsString(buffer_obj);
        size = PyByteArray_Size(buffer_obj);
        if (text == NULL || size < 0) {
            return NULL;
        }
    }
    if (!PyArg_Parse(arg, "et#", encoding, &text, &size)) {
        return NULL;
    }
    void *result = PyBytes_FromStringAndSize(text, size);
    if (buffer_obj == NULL) {
        PyMem_Free(text);
    }
    return result;
}

void *pyrs_testcapi_getargs_w_star(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_w_star", kwargs)) {
        return NULL;
    }
    PyrsPyBuffer buffer;
    if (!PyArg_ParseTuple(args, "w*:getargs_w_star", &buffer)) {
        return NULL;
    }
    if (buffer.len >= 2) {
        char *text = (char *)buffer.buf;
        text[0] = '[';
        text[buffer.len - 1] = ']';
    }
    void *result = PyBytes_FromStringAndSize(buffer.buf, buffer.len);
    PyBuffer_Release(&buffer);
    return result;
}

void *pyrs_testcapi_getargs_w_star_opt(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("getargs_w_star_opt", kwargs)) {
        return NULL;
    }
    PyrsPyBuffer buffer;
    PyrsPyBuffer buffer2;
    pyrs_zero_buffer(&buffer2);
    int number = 1;
    if (!PyArg_ParseTuple(args, "w*|w*i:getargs_w_star", &buffer, &buffer2, &number)) {
        return NULL;
    }
    if (buffer.len >= 2) {
        char *text = (char *)buffer.buf;
        text[0] = '[';
        text[buffer.len - 1] = ']';
    }
    void *result = PyBytes_FromStringAndSize(buffer.buf, buffer.len);
    PyBuffer_Release(&buffer);
    if (buffer2.obj != NULL || buffer2.buf != NULL) {
        PyBuffer_Release(&buffer2);
    }
    (void)number;
    return result;
}

void *pyrs_testcapi_gh_99240_clear_args(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("gh_99240_clear_args", kwargs)) {
        return NULL;
    }
    char *a = NULL;
    char *b = NULL;
    if (!PyArg_ParseTuple(args, "eses", "idna", &a, "idna", &b)) {
        if (a != NULL || b != NULL) {
            PyErr_Clear();
            PyErr_SetString((void *)&PyExc_AssertionError, "Arguments are not cleared.");
        }
        return NULL;
    }
    PyMem_Free(a);
    PyMem_Free(b);
    return build_none();
}

void *pyrs_testcapi_parse_tuple_and_keywords(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("parse_tuple_and_keywords", kwargs)) {
        return NULL;
    }
    Py_ssize_t positional = args == NULL ? 0 : PyTuple_Size(args);
    if (positional < 0) {
        return NULL;
    }
    if (positional != 4) {
        PyErr_Format(
            (void *)&PyExc_TypeError,
            "parse_tuple_and_keywords() takes exactly 4 arguments (%zd given)",
            positional
        );
        return NULL;
    }

    void *sub_args = NULL;
    void *sub_kwargs = NULL;
    const char *sub_format = NULL;
    void *sub_keywords = NULL;

#define PYRS_TESTCAPI_MAX_PARAMS 8
    double buffers[PYRS_TESTCAPI_MAX_PARAMS][4];
    const char *keywords[PYRS_TESTCAPI_MAX_PARAMS + 1];

    if (!PyArg_ParseTuple(
            args,
            "OOsO:parse_tuple_and_keywords",
            &sub_args,
            &sub_kwargs,
            &sub_format,
            &sub_keywords
        )) {
        return NULL;
    }

    int keywords_is_exact_list = object_type_is_exact(sub_keywords, (void *)&PyList_Type);
    int keywords_is_exact_tuple = object_type_is_exact(sub_keywords, (void *)&PyTuple_Type);
    if (!keywords_is_exact_list && !keywords_is_exact_tuple) {
        PyErr_SetString(
            (void *)&PyExc_ValueError,
            "parse_tuple_and_keywords: sub_keywords must be either list or tuple"
        );
        return NULL;
    }

    memset(buffers, 0, sizeof(buffers));
    memset(keywords, 0, sizeof(keywords));

    Py_ssize_t size = keywords_is_exact_list ? PyList_Size(sub_keywords) : PyTuple_Size(sub_keywords);
    if (size > PYRS_TESTCAPI_MAX_PARAMS) {
        PyErr_SetString(
            (void *)&PyExc_ValueError,
            "parse_tuple_and_keywords: too many keywords in sub_keywords"
        );
        return NULL;
    }

    for (Py_ssize_t i = 0; i < size; i++) {
        void *item = keywords_is_exact_list ? PyList_GetItem(sub_keywords, i) : PyTuple_GetItem(sub_keywords, i);
        if (item == NULL) {
            return NULL;
        }
        if (object_is_instance_of_type(item, (void *)&PyUnicode_Type)) {
            keywords[i] = (char *)PyUnicode_AsUTF8(item);
            if (keywords[i] == NULL) {
                return NULL;
            }
        } else if (object_is_instance_of_type(item, (void *)&PyBytes_Type)) {
            char *buffer = NULL;
            if (PyBytes_AsStringAndSize(item, &buffer, NULL) < 0) {
                return NULL;
            }
            keywords[i] = buffer;
        } else {
            PyErr_SetString(
                (void *)&PyExc_ValueError,
                "parse_tuple_and_keywords: keywords must be str or bytes"
            );
            return NULL;
        }
    }

    int result = PyArg_ParseTupleAndKeywords(
        sub_args,
        sub_kwargs,
        sub_format,
        keywords,
        buffers + 0,
        buffers + 1,
        buffers + 2,
        buffers + 3,
        buffers + 4,
        buffers + 5,
        buffers + 6,
        buffers + 7
    );
    if (!result) {
        return NULL;
    }

    int objects_only = 1;
    int count = 0;
    for (const char *cursor = sub_format; *cursor != '\0'; cursor++) {
        if (isalnum((unsigned char)*cursor)) {
            if (strchr("OSUY", *cursor) == NULL) {
                objects_only = 0;
                break;
            }
            count++;
        }
    }
    if (!objects_only) {
        return build_none();
    }

    void *return_value = PyTuple_New((Py_ssize_t)count);
    if (return_value == NULL) {
        return NULL;
    }
    for (Py_ssize_t i = 0; i < count; i++) {
        void *arg = *(void **)(buffers + i);
        if (arg == NULL) {
            arg = build_none();
        } else {
            Py_IncRef(arg);
        }
        if (PyTuple_SetItem(return_value, i, arg) < 0) {
            Py_DecRef(arg);
            Py_DecRef(return_value);
            return NULL;
        }
    }
    return return_value;
}

static void *pyrs_testcapi_argparsing_str1 = NULL;
static void *pyrs_testcapi_argparsing_str2 = NULL;

static int pyrs_testcapi_failing_converter(void *obj, void *arg)
{
    (void)obj;
    (void)arg;
    if (pyrs_testcapi_argparsing_str1 == NULL) {
        return 0;
    }
    Py_IncRef(pyrs_testcapi_argparsing_str1);
    pyrs_testcapi_argparsing_str2 = pyrs_testcapi_argparsing_str1;
    return 0;
}

void *pyrs_testcapi_argparsing(void *args, void *kwargs)
{
    if (!_PyArg_NoKeywords("argparsing", kwargs)) {
        return NULL;
    }
    Py_ssize_t positional = args == NULL ? 0 : PyTuple_Size(args);
    if (positional < 0) {
        return NULL;
    }
    if (positional != 2) {
        PyErr_Format(
            (void *)&PyExc_TypeError,
            "function takes exactly 2 arguments (%zd given)",
            positional
        );
        return NULL;
    }
    pyrs_testcapi_argparsing_str1 = NULL;
    pyrs_testcapi_argparsing_str2 = NULL;
    if (!PyArg_ParseTuple(
            args,
            "O&O&",
            PyUnicode_FSConverter,
            &pyrs_testcapi_argparsing_str1,
            pyrs_testcapi_failing_converter,
            &pyrs_testcapi_argparsing_str2
        )) {
        if (pyrs_testcapi_argparsing_str2 == NULL) {
            return NULL;
        }
        Py_ssize_t refcnt = ((PyObjectHeadCompat *)pyrs_testcapi_argparsing_str2)->ob_refcnt;
        void *result = PyLong_FromSsize_t(refcnt);
        Py_DecRef(pyrs_testcapi_argparsing_str2);
        pyrs_testcapi_argparsing_str2 = NULL;
        PyErr_Clear();
        return result;
    }
    return build_none();
}

typedef struct {
    char bool_member;
    char byte_member;
    unsigned char ubyte_member;
    short short_member;
    unsigned short ushort_member;
    int int_member;
    unsigned int uint_member;
    long long_member;
    unsigned long ulong_member;
    Py_ssize_t pyssizet_member;
    float float_member;
    double double_member;
    char inplace_member[6];
    long long longlong_member;
    unsigned long long ulonglong_member;
    char char_member;
} pyrs_all_structmembers;

typedef struct {
    PyObject ob_base;
    pyrs_all_structmembers structmembers;
} pyrs_test_structmembers;

static PyMemberDef pyrs_test_structmembers_members[] = {
    {"T_BOOL", Py_T_BOOL, offsetof(pyrs_test_structmembers, structmembers.bool_member), 0, NULL},
    {"T_BYTE", Py_T_BYTE, offsetof(pyrs_test_structmembers, structmembers.byte_member), 0, NULL},
    {"T_UBYTE", Py_T_UBYTE, offsetof(pyrs_test_structmembers, structmembers.ubyte_member), 0, NULL},
    {"T_SHORT", Py_T_SHORT, offsetof(pyrs_test_structmembers, structmembers.short_member), 0, NULL},
    {"T_USHORT", Py_T_USHORT, offsetof(pyrs_test_structmembers, structmembers.ushort_member), 0, NULL},
    {"T_INT", Py_T_INT, offsetof(pyrs_test_structmembers, structmembers.int_member), 0, NULL},
    {"T_UINT", Py_T_UINT, offsetof(pyrs_test_structmembers, structmembers.uint_member), 0, NULL},
    {"T_LONG", Py_T_LONG, offsetof(pyrs_test_structmembers, structmembers.long_member), 0, NULL},
    {"T_ULONG", Py_T_ULONG, offsetof(pyrs_test_structmembers, structmembers.ulong_member), 0, NULL},
    {"T_PYSSIZET", Py_T_PYSSIZET, offsetof(pyrs_test_structmembers, structmembers.pyssizet_member), 0, NULL},
    {"T_FLOAT", Py_T_FLOAT, offsetof(pyrs_test_structmembers, structmembers.float_member), 0, NULL},
    {"T_DOUBLE", Py_T_DOUBLE, offsetof(pyrs_test_structmembers, structmembers.double_member), 0, NULL},
    {"T_STRING_INPLACE", Py_T_STRING_INPLACE, offsetof(pyrs_test_structmembers, structmembers.inplace_member), 0, NULL},
    {"T_LONGLONG", Py_T_LONGLONG, offsetof(pyrs_test_structmembers, structmembers.longlong_member), 0, NULL},
    {"T_ULONGLONG", Py_T_ULONGLONG, offsetof(pyrs_test_structmembers, structmembers.ulonglong_member), 0, NULL},
    {"T_CHAR", Py_T_CHAR, offsetof(pyrs_test_structmembers, structmembers.char_member), 0, NULL},
    {NULL, 0, 0, 0, NULL},
};

static void *
pyrs_test_structmembers_new(PyTypeObject *type, void *args, void *kwargs)
{
    static const char *keywords[] = {
        "T_BOOL", "T_BYTE", "T_UBYTE", "T_SHORT", "T_USHORT",
        "T_INT", "T_UINT", "T_LONG", "T_ULONG", "T_PYSSIZET",
        "T_FLOAT", "T_DOUBLE", "T_STRING_INPLACE",
        "T_LONGLONG", "T_ULONGLONG", "T_CHAR",
        NULL
    };
    static const char fmt[] = "|bbBhHiIlknfds#LKc";
    const char *inplace_src = NULL;
    Py_ssize_t inplace_len = 0;
    pyrs_test_structmembers *self =
        (pyrs_test_structmembers *)PyType_GenericAlloc((void *)type, 0);
    if (self == NULL) {
        return NULL;
    }
    memset(&self->structmembers, 0, sizeof(self->structmembers));
    if (!PyArg_ParseTupleAndKeywords(
            args,
            kwargs,
            fmt,
            keywords,
            &self->structmembers.bool_member,
            &self->structmembers.byte_member,
            &self->structmembers.ubyte_member,
            &self->structmembers.short_member,
            &self->structmembers.ushort_member,
            &self->structmembers.int_member,
            &self->structmembers.uint_member,
            &self->structmembers.long_member,
            &self->structmembers.ulong_member,
            &self->structmembers.pyssizet_member,
            &self->structmembers.float_member,
            &self->structmembers.double_member,
            &inplace_src,
            &inplace_len,
            &self->structmembers.longlong_member,
            &self->structmembers.ulonglong_member,
            &self->structmembers.char_member)) {
        Py_DecRef(self);
        return NULL;
    }
    if (inplace_src != NULL) {
        if (inplace_len > 5) {
            Py_DecRef(self);
            PyErr_SetString((void *)&PyExc_ValueError, "string too long");
            return NULL;
        }
        memcpy(self->structmembers.inplace_member, inplace_src, (size_t)inplace_len);
        self->structmembers.inplace_member[inplace_len] = '\0';
    }
    return (void *)self;
}

static PyType_Slot pyrs_test_structmembers_slots[] = {
    {Py_tp_new, pyrs_test_structmembers_new},
    {Py_tp_members, pyrs_test_structmembers_members},
    {0, NULL},
};

static PyType_Spec pyrs_test_structmembers_old_spec = {
    "_testcapi._test_structmembersType_OldAPI",
    (int)sizeof(pyrs_test_structmembers),
    0,
    Py_TPFLAGS_DEFAULT,
    pyrs_test_structmembers_slots,
};

static PyType_Spec pyrs_test_structmembers_new_spec = {
    "_testcapi._test_structmembersType_NewAPI",
    (int)sizeof(pyrs_test_structmembers),
    0,
    Py_TPFLAGS_DEFAULT,
    pyrs_test_structmembers_slots,
};

int pyrs_testcapi_init_structmember_types(void *module)
{
    void *type_obj = PyType_FromModuleAndSpec(module, &pyrs_test_structmembers_old_spec, NULL);
    if (type_obj == NULL) {
        return -1;
    }
    if (PyModule_AddType(module, type_obj) < 0) {
        Py_DecRef(type_obj);
        return -1;
    }
    Py_DecRef(type_obj);

    type_obj = PyType_FromModuleAndSpec(module, &pyrs_test_structmembers_new_spec, NULL);
    if (type_obj == NULL) {
        return -1;
    }
    if (PyModule_AddType(module, type_obj) < 0) {
        Py_DecRef(type_obj);
        return -1;
    }
    Py_DecRef(type_obj);
    return 0;
}

int _PyArg_CheckPositional(const char *name, Py_ssize_t nargs, Py_ssize_t min, Py_ssize_t max)
{
    if (max != PY_SSIZE_T_MAX && nargs >= min && nargs <= max) {
        return 1;
    }
    if (max == PY_SSIZE_T_MAX && nargs >= min) {
        return 1;
    }
    if (name == NULL) {
        name = "<function>";
    }
    if (nargs < min) {
        if (min == max) {
            PyErr_Format(
                (void *)&PyExc_TypeError,
                "%s() takes exactly %zd positional argument%s (%zd given)",
                name,
                min,
                min == 1 ? "" : "s",
                nargs
            );
        } else {
            PyErr_Format(
                (void *)&PyExc_TypeError,
                "%s() takes at least %zd positional argument%s (%zd given)",
                name,
                min,
                min == 1 ? "" : "s",
                nargs
            );
        }
        return 0;
    }
    if (max != PY_SSIZE_T_MAX) {
        PyErr_Format(
            (void *)&PyExc_TypeError,
            "%s() takes at most %zd positional argument%s (%zd given)",
            name,
            max,
            max == 1 ? "" : "s",
            nargs
        );
        return 0;
    }
    return 1;
}

void _PyArg_BadArgument(
    const char *fname,
    const char *displayname,
    const char *expected,
    void *arg
)
{
    (void)arg;
    if (fname == NULL) {
        fname = "<function>";
    }
    if (displayname == NULL || displayname[0] == '\0') {
        displayname = "argument";
    }
    if (expected == NULL || expected[0] == '\0') {
        expected = "object";
    }
    PyErr_Format(
        (void *)&PyExc_TypeError,
        "%s(): %s must be %s",
        fname,
        displayname,
        expected
    );
}

void *PyImport_ImportModuleAttrString(const char *modname, const char *attrname)
{
    if (modname == NULL || attrname == NULL) {
        PyErr_SetString((void *)&PyExc_TypeError, "PyImport_ImportModuleAttrString received null argument");
        return NULL;
    }
    void *module = PyImport_ImportModule(modname);
    if (module == NULL) {
        return NULL;
    }
    void *attr = PyObject_GetAttrString(module, attrname);
    Py_DecRef(module);
    return attr;
}

Py_hash_t Py_HashBuffer(const void *ptr, Py_ssize_t len)
{
    if (len < 0) {
        PyErr_SetString((void *)&PyExc_TypeError, "Py_HashBuffer length must be non-negative");
        return -1;
    }
    const char *raw = (const char *)ptr;
    void *bytes = PyBytes_FromStringAndSize(raw, len);
    if (bytes == NULL) {
        return -1;
    }
    Py_hash_t hash = PyObject_Hash(bytes);
    Py_DecRef(bytes);
    return hash;
}

int _PyLong_UnsignedInt_Converter(void *object, void *address)
{
    if (address == NULL) {
        PyErr_SetString((void *)&PyExc_TypeError, "_PyLong_UnsignedInt_Converter missing destination");
        return 0;
    }
    unsigned long value = PyLong_AsUnsignedLong(object);
    if (PyErr_Occurred() != NULL) {
        return 0;
    }
    if (value > UINT_MAX) {
        PyErr_SetString((void *)&PyExc_TypeError, "integer out of range for unsigned int");
        return 0;
    }
    *((unsigned int *)address) = (unsigned int)value;
    return 1;
}

void * const *_PyArg_UnpackKeywords(
    void *const *args,
    Py_ssize_t nargs,
    void *kwargs,
    void *kwnames,
    _PyArg_Parser *parser,
    int minpos,
    int maxpos,
    int minkw,
    int varpos,
    void **buf
)
{
    if (parser == NULL || buf == NULL) {
        PyErr_SetString((void *)&PyExc_TypeError, "_PyArg_UnpackKeywords received null parser/buffer");
        return NULL;
    }
    if (kwnames != NULL && !is_tuple_object(kwnames)) {
        PyErr_BadInternalCall();
        return NULL;
    }
    if (args == NULL) {
        if (nargs == 0 && kwargs == NULL && kwnames == NULL) {
            args = (void *const *)buf;
        } else {
            PyErr_BadInternalCall();
            return NULL;
        }
    }
    const char * const *keywords = parser->keywords;
    if (keywords == NULL) {
        PyErr_BadInternalCall();
        return NULL;
    }
    int keyword_count = pyrs_keyword_count(keywords);
    int posonly = parser->pos;
    if (posonly < 0) {
        posonly = 0;
    }
    int minposonly = posonly < minpos ? posonly : minpos;
    int maxargs = posonly + keyword_count;
    int reqlimit = minkw ? maxpos + minkw : minpos;
    for (int i = 0; i < keyword_count; i++) {
        buf[i] = NULL;
    }

    Py_ssize_t nkwargs = 0;
    void * const *kwstack = NULL;
    if (kwargs != NULL) {
        nkwargs = PyDict_Size(kwargs);
        if (nkwargs < 0) {
            return NULL;
        }
    } else if (kwnames != NULL) {
        nkwargs = PyTuple_Size(kwnames);
        if (nkwargs < 0) {
            return NULL;
        }
        kwstack = args + nargs;
    }

    if (nkwargs == 0 && minkw == 0 && minpos <= nargs && (varpos || nargs <= maxpos)) {
        return args;
    }

    if (!varpos && (nargs + nkwargs) > (Py_ssize_t)maxargs) {
        PyErr_Format(
            (void *)&PyExc_TypeError,
            "%s() takes at most %d %sargument%s (%zd given)",
            parser->fname ? parser->fname : "<function>",
            maxargs,
            nargs == 0 ? "keyword " : "",
            maxargs == 1 ? "" : "s",
            nargs + nkwargs
        );
        return NULL;
    }

    if (!varpos && maxpos >= 0 && nargs > (Py_ssize_t)maxpos) {
        _PyArg_CheckPositional(parser->fname ? parser->fname : "<function>", nargs, minpos, maxpos);
        return NULL;
    }

    if (nargs < (Py_ssize_t)minposonly) {
        _PyArg_CheckPositional(
            parser->fname ? parser->fname : "<function>",
            nargs,
            minposonly,
            (varpos || minposonly < maxpos) ? PY_SSIZE_T_MAX : maxpos
        );
        return NULL;
    }

    if (varpos && nargs > (Py_ssize_t)maxpos) {
        nargs = maxpos;
    }

    for (Py_ssize_t i = 0; i < nargs && i < (Py_ssize_t)maxargs; i++) {
        buf[i] = args ? args[i] : NULL;
    }

    for (int i = (nargs > posonly) ? (int)nargs : posonly; i < maxargs; i++) {
        void *current_arg = NULL;
        const char *keyword = keywords[i - posonly];
        if (nkwargs > 0) {
            if (kwargs != NULL) {
                current_arg = PyDict_GetItemString(kwargs, keyword);
            } else if (kwnames != NULL && kwstack != NULL) {
                Py_ssize_t kw_count = PyTuple_Size(kwnames);
                if (kw_count < 0) {
                    return NULL;
                }
                for (Py_ssize_t j = 0; j < kw_count; j++) {
                    void *name_obj = PyTuple_GetItem(kwnames, j);
                    const char *name = PyUnicode_AsUTF8(name_obj);
                    if (name == NULL) {
                        return NULL;
                    }
                    if (strcmp(name, keyword) == 0) {
                        current_arg = kwstack[j];
                        break;
                    }
                }
            }
        } else if (i >= reqlimit) {
            break;
        }

        buf[i] = current_arg;
        if (current_arg != NULL) {
            nkwargs--;
        } else if (i < minpos || (maxpos <= i && i < reqlimit)) {
            PyErr_Format(
                (void *)&PyExc_TypeError,
                "%s() missing required argument '%s' (pos %d)",
                parser->fname ? parser->fname : "<function>",
                keyword,
                i + 1
            );
            return NULL;
        }
    }

    if (nkwargs > 0) {
        for (int i = posonly; i < (int)nargs && i < maxargs; i++) {
            const char *keyword = keywords[i - posonly];
            void *current_arg = NULL;
            if (kwargs != NULL) {
                current_arg = PyDict_GetItemString(kwargs, keyword);
            } else if (kwnames != NULL && kwstack != NULL) {
                Py_ssize_t kw_count = PyTuple_Size(kwnames);
                if (kw_count < 0) {
                    return NULL;
                }
                for (Py_ssize_t j = 0; j < kw_count; j++) {
                    void *name_obj = PyTuple_GetItem(kwnames, j);
                    const char *name = PyUnicode_AsUTF8(name_obj);
                    if (name == NULL) {
                        return NULL;
                    }
                    if (strcmp(name, keyword) == 0) {
                        current_arg = kwstack[j];
                        break;
                    }
                }
            }
            if (current_arg != NULL) {
                PyErr_Format(
                    (void *)&PyExc_TypeError,
                    "argument for %s() given by name ('%s') and position (%d)",
                    parser->fname ? parser->fname : "<function>",
                    keyword,
                    i + 1
                );
                return NULL;
            }
        }

        if (kwargs != NULL) {
            void *keys = PyDict_Keys(kwargs);
            if (keys == NULL) {
                return NULL;
            }
            Py_ssize_t key_count = PyList_Size(keys);
            if (key_count < 0) {
                Py_DecRef(keys);
                return NULL;
            }
            for (Py_ssize_t i = 0; i < key_count; i++) {
                void *key_obj = PyList_GetItem(keys, i);
                const char *name = PyUnicode_AsUTF8(key_obj);
                if (name == NULL) {
                    Py_DecRef(keys);
                    return NULL;
                }
                if (pyrs_lookup_keyword_index(keywords, name) < 0) {
                    Py_DecRef(keys);
                    PyErr_Format(
                        (void *)&PyExc_TypeError,
                        "%s() got an unexpected keyword argument '%s'",
                        parser->fname ? parser->fname : "<function>",
                        name
                    );
                    return NULL;
                }
            }
            Py_DecRef(keys);
        } else if (kwnames != NULL) {
            Py_ssize_t kw_count = PyTuple_Size(kwnames);
            if (kw_count < 0) {
                return NULL;
            }
            for (Py_ssize_t i = 0; i < kw_count; i++) {
                void *name_obj = PyTuple_GetItem(kwnames, i);
                const char *name = PyUnicode_AsUTF8(name_obj);
                if (name == NULL) {
                    return NULL;
                }
                if (pyrs_lookup_keyword_index(keywords, name) < 0) {
                    PyErr_Format(
                        (void *)&PyExc_TypeError,
                        "%s() got an unexpected keyword argument '%s'",
                        parser->fname ? parser->fname : "<function>",
                        name
                    );
                    return NULL;
                }
            }
        } else {
            PyErr_Format(
                (void *)&PyExc_TypeError,
                "%s() got unexpected keyword arguments",
                parser->fname ? parser->fname : "<function>"
            );
            return NULL;
        }
        if (nkwargs > 0) {
            PyErr_Format(
                (void *)&PyExc_TypeError,
                "%s() got unexpected keyword arguments",
                parser->fname ? parser->fname : "<function>"
            );
            return NULL;
        }
    }
    return (void * const *)buf;
}

int _PyArg_NoPositional(const char *funcname, void *args)
{
    if (args == NULL) {
        return 1;
    }
    Py_ssize_t nargs = PyTuple_Size(args);
    if (nargs <= 0) {
        return 1;
    }
    if (funcname == NULL) {
        funcname = "<function>";
    }
    PyErr_Format(
        (void *)&PyExc_TypeError,
        "%s() takes no positional arguments",
        funcname
    );
    return 0;
}

int _PyArg_ParseStack(
    void *const *args,
    Py_ssize_t nargs,
    const char *format,
    ...
)
{
    void *tuple = pyrs_capi_tuple_pack_from_array(nargs, args);
    if (tuple == NULL) {
        return 0;
    }
    va_list ap;
    va_start(ap, format);
    int ok = _PyArg_VaParse_SizeT(tuple, format, ap);
    va_end(ap);
    Py_DecRef(tuple);
    return ok;
}

int _PyArg_ParseStackAndKeywords(
    void *const *args,
    Py_ssize_t nargs,
    void *kwnames,
    _PyArg_Parser *parser,
    ...
)
{
    if (parser == NULL) {
        PyErr_SetString((void *)&PyExc_TypeError, "_PyArg_ParseStackAndKeywords missing parser");
        return 0;
    }
    void *tuple = pyrs_capi_tuple_pack_from_array(nargs, args);
    if (tuple == NULL) {
        return 0;
    }
    void *kwargs = NULL;
    if (kwnames != NULL) {
        kwargs = PyDict_New();
        if (kwargs == NULL) {
            Py_DecRef(tuple);
            return 0;
        }
        Py_ssize_t kw_count = PyTuple_Size(kwnames);
        if (kw_count < 0) {
            Py_DecRef(tuple);
            Py_DecRef(kwargs);
            return 0;
        }
        for (Py_ssize_t i = 0; i < kw_count; i++) {
            void *key = PyTuple_GetItem(kwnames, i);
            void *value = args ? args[nargs + i] : NULL;
            if (key == NULL || value == NULL || PyDict_SetItem(kwargs, key, value) < 0) {
                Py_DecRef(tuple);
                Py_DecRef(kwargs);
                return 0;
            }
        }
    }

    const char *format = parser->format ? parser->format : "";
    va_list ap;
    va_start(ap, parser);
    int ok = _PyArg_VaParseTupleAndKeywords_SizeT(
        tuple,
        kwargs,
        format,
        parser->keywords,
        ap
    );
    va_end(ap);
    Py_DecRef(tuple);
    if (kwargs != NULL) {
        Py_DecRef(kwargs);
    }
    return ok;
}

int _Py_convert_optional_to_ssize_t(void *obj, void *result)
{
    if (result == NULL) {
        PyErr_SetString((void *)&PyExc_TypeError, "_Py_convert_optional_to_ssize_t missing result");
        return 0;
    }
    if (obj == (void *)&_Py_NoneStruct) {
        return 1;
    }
    void *index_value = PyNumber_Index(obj);
    if (index_value == NULL) {
        return 0;
    }
    Py_ssize_t converted = PyNumber_AsSsize_t(index_value, (void *)&PyExc_OverflowError);
    Py_DecRef(index_value);
    if (converted == -1 && PyErr_Occurred() != NULL) {
        return 0;
    }
    *((Py_ssize_t *)result) = converted;
    return 1;
}

void *_PyNumber_Index(void *object)
{
    return PyNumber_Index(object);
}

int _PyLong_UnsignedLong_Converter(void *object, void *address)
{
    if (address == NULL) {
        PyErr_SetString((void *)&PyExc_TypeError, "_PyLong_UnsignedLong_Converter missing destination");
        return 0;
    }
    unsigned long value = PyLong_AsUnsignedLong(object);
    if (PyErr_Occurred() != NULL) {
        return 0;
    }
    *((unsigned long *)address) = value;
    return 1;
}

int _PyLong_UnsignedLongLong_Converter(void *object, void *address)
{
    if (address == NULL) {
        PyErr_SetString((void *)&PyExc_TypeError, "_PyLong_UnsignedLongLong_Converter missing destination");
        return 0;
    }
    unsigned long long value = PyLong_AsUnsignedLongLong(object);
    if (PyErr_Occurred() != NULL) {
        return 0;
    }
    *((unsigned long long *)address) = value;
    return 1;
}

int _PyLong_AsByteArray(
    void *long_obj,
    unsigned char *bytes,
    size_t n,
    int little_endian,
    int is_signed,
    int with_exceptions
)
{
    if (long_obj == NULL || (bytes == NULL && n != 0)) {
        PyErr_BadInternalCall();
        return -1;
    }
    int flags = little_endian ? 1 : 0;
    if (!is_signed) {
        flags |= 4;
        flags |= 8;
    }
    Py_ssize_t required = PyLong_AsNativeBytes(long_obj, bytes, (Py_ssize_t)n, flags);
    if (required < 0) {
        return -1;
    }
    if (required > (Py_ssize_t)n) {
        if (with_exceptions) {
            PyErr_SetString((void *)&PyExc_OverflowError, "int too big to convert");
        }
        return -1;
    }
    return 0;
}

#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
#define PYRS_LONG_DIGIT_ENDIANNESS 1
#else
#define PYRS_LONG_DIGIT_ENDIANNESS -1
#endif

static const PyLongLayout PYRS_LONG_LAYOUT = {
    .bits_per_digit = 30,
    .digit_size = 4,
    .digits_order = -1,
    .digit_endianness = PYRS_LONG_DIGIT_ENDIANNESS,
};

static int pyrs_long_digits_from_unsigned_bytes(
    const unsigned char *bytes,
    size_t nbytes,
    uint32_t **out_digits,
    Py_ssize_t *out_ndigits
)
{
    if (out_digits == NULL || out_ndigits == NULL) {
        PyErr_BadInternalCall();
        return -1;
    }
    *out_digits = NULL;
    *out_ndigits = 0;
    if (nbytes == 0) {
        uint32_t *digits = (uint32_t *)calloc(1, sizeof(uint32_t));
        if (digits == NULL) {
            PyErr_NoMemory();
            return -1;
        }
        *out_digits = digits;
        *out_ndigits = 1;
        return 0;
    }

    size_t ndigits = (nbytes * 8 + 29) / 30;
    if (ndigits == 0) {
        ndigits = 1;
    }
    uint32_t *digits = (uint32_t *)calloc(ndigits, sizeof(uint32_t));
    if (digits == NULL) {
        PyErr_NoMemory();
        return -1;
    }

    for (size_t byte_index = 0; byte_index < nbytes; byte_index++) {
        uint32_t chunk = (uint32_t)bytes[byte_index];
        size_t bit_offset = byte_index * 8;
        size_t digit_index = bit_offset / 30;
        size_t shift = bit_offset % 30;
        digits[digit_index] |= chunk << shift;
        if (shift > 22 && digit_index + 1 < ndigits) {
            digits[digit_index + 1] |= chunk >> (30 - shift);
        }
    }

    while (ndigits > 1 && digits[ndigits - 1] == 0) {
        ndigits--;
    }

    *out_digits = digits;
    *out_ndigits = (Py_ssize_t)ndigits;
    return 0;
}

static int pyrs_unsigned_bytes_from_long_digits(
    const uint32_t *digits,
    Py_ssize_t ndigits,
    unsigned char **out_bytes,
    size_t *out_nbytes
)
{
    if (out_bytes == NULL || out_nbytes == NULL) {
        PyErr_BadInternalCall();
        return -1;
    }
    *out_bytes = NULL;
    *out_nbytes = 0;
    if (ndigits <= 0 || digits == NULL) {
        unsigned char *bytes = (unsigned char *)calloc(1, 1);
        if (bytes == NULL) {
            PyErr_NoMemory();
            return -1;
        }
        *out_bytes = bytes;
        *out_nbytes = 1;
        return 0;
    }

    size_t nbits = (size_t)ndigits * 30;
    size_t nbytes = (nbits + 7) / 8;
    if (nbytes == 0) {
        nbytes = 1;
    }
    unsigned char *bytes = (unsigned char *)calloc(nbytes, 1);
    if (bytes == NULL) {
        PyErr_NoMemory();
        return -1;
    }

    for (Py_ssize_t i = 0; i < ndigits; i++) {
        uint32_t digit = digits[i] & 0x3FFFFFFFu;
        size_t bit_offset = (size_t)i * 30;
        size_t byte_index = bit_offset / 8;
        size_t shift = bit_offset % 8;

        uint64_t window = ((uint64_t)digit) << shift;
        size_t window_len = 5;
        if (byte_index + window_len > nbytes) {
            window_len = nbytes - byte_index;
        }
        for (size_t j = 0; j < window_len; j++) {
            bytes[byte_index + j] |= (unsigned char)((window >> (8 * j)) & 0xFFu);
        }
    }

    while (nbytes > 1 && bytes[nbytes - 1] == 0) {
        nbytes--;
    }

    *out_bytes = bytes;
    *out_nbytes = nbytes;
    return 0;
}

const PyLongLayout *_PyLong_GetNativeLayout(void)
{
    return &PYRS_LONG_LAYOUT;
}

int _PyLong_Export(void *obj, PyLongExport *export_long)
{
    if (export_long == NULL) {
        PyErr_BadInternalCall();
        return -1;
    }
    memset(export_long, 0, sizeof(*export_long));
    if (obj == NULL) {
        PyErr_BadInternalCall();
        return -1;
    }

    void *obj_type = PyObject_Type(obj);
    if (obj_type == NULL) {
        return -1;
    }
    int is_long = PyType_IsSubtype(obj_type, (void *)&PyLong_Type);
    Py_DecRef(obj_type);
    if (!is_long) {
        PyErr_SetString((void *)&PyExc_TypeError, "expect int");
        return -1;
    }

    long long small_value = PyLong_AsLongLong(obj);
    if (!(small_value == -1 && PyErr_Occurred() != NULL)) {
        export_long->value = (int64_t)small_value;
        export_long->negative = 0;
        export_long->ndigits = 0;
        export_long->digits = NULL;
        export_long->_reserved = 0;
        return 0;
    }
    if (!PyErr_ExceptionMatches((void *)&PyExc_OverflowError)) {
        return -1;
    }
    PyErr_Clear();

    const int flags = 1 | 4 | 8;
    void *source = obj;
    int negative = 0;
    Py_ssize_t nbytes = PyLong_AsNativeBytes(source, NULL, 0, flags);
    if (nbytes < 0) {
        if (!PyErr_ExceptionMatches((void *)&PyExc_ValueError)) {
            return -1;
        }
        PyErr_Clear();
        source = PyNumber_Negative(obj);
        if (source == NULL) {
            return -1;
        }
        negative = 1;
        nbytes = PyLong_AsNativeBytes(source, NULL, 0, flags);
        if (nbytes < 0) {
            Py_DecRef(source);
            return -1;
        }
    }

    if (nbytes == 0) {
        if (source != obj) {
            Py_DecRef(source);
        }
        export_long->value = 0;
        export_long->negative = 0;
        export_long->ndigits = 0;
        export_long->digits = NULL;
        export_long->_reserved = 0;
        return 0;
    }

    unsigned char *bytes = (unsigned char *)malloc((size_t)nbytes);
    if (bytes == NULL) {
        if (source != obj) {
            Py_DecRef(source);
        }
        PyErr_NoMemory();
        return -1;
    }
    if (PyLong_AsNativeBytes(source, bytes, nbytes, flags) < 0) {
        if (source != obj) {
            Py_DecRef(source);
        }
        free(bytes);
        return -1;
    }
    if (source != obj) {
        Py_DecRef(source);
    }

    uint32_t *digits = NULL;
    Py_ssize_t ndigits = 0;
    if (pyrs_long_digits_from_unsigned_bytes(bytes, (size_t)nbytes, &digits, &ndigits) < 0) {
        free(bytes);
        return -1;
    }
    free(bytes);

    export_long->value = 0;
    export_long->negative = (uint8_t)(negative ? 1 : 0);
    export_long->ndigits = ndigits;
    export_long->digits = digits;
    export_long->_reserved = (uintptr_t)digits;
    return 0;
}

void _PyLong_FreeExport(PyLongExport *export_long)
{
    if (export_long == NULL) {
        return;
    }
    if (export_long->_reserved != 0) {
        free((void *)export_long->_reserved);
    }
    export_long->_reserved = 0;
    export_long->digits = NULL;
    export_long->ndigits = 0;
}

_PyLongWriter *_PyLongWriter_Create(int negative, Py_ssize_t ndigits, void **digits)
{
    if (digits == NULL) {
        PyErr_BadInternalCall();
        return NULL;
    }
    *digits = NULL;
    if (ndigits <= 0) {
        PyErr_SetString((void *)&PyExc_ValueError, "ndigits must be positive");
        return NULL;
    }

    _PyLongWriter *writer = (_PyLongWriter *)calloc(1, sizeof(_PyLongWriter));
    if (writer == NULL) {
        PyErr_NoMemory();
        return NULL;
    }
    writer->digits = (uint32_t *)calloc((size_t)ndigits, sizeof(uint32_t));
    if (writer->digits == NULL) {
        free(writer);
        PyErr_NoMemory();
        return NULL;
    }
    writer->negative = negative ? 1 : 0;
    writer->ndigits = ndigits;
    *digits = writer->digits;
    return writer;
}

void *_PyLongWriter_Finish(_PyLongWriter *writer)
{
    if (writer == NULL) {
        PyErr_BadInternalCall();
        return NULL;
    }
    unsigned char *bytes = NULL;
    size_t nbytes = 0;
    if (pyrs_unsigned_bytes_from_long_digits(writer->digits, writer->ndigits, &bytes, &nbytes) < 0) {
        free(writer->digits);
        free(writer);
        return NULL;
    }
    int flags = 1 | 4;
    void *value = PyLong_FromNativeBytes(bytes, nbytes, flags);
    free(bytes);
    if (value == NULL) {
        free(writer->digits);
        free(writer);
        return NULL;
    }

    if (writer->negative && PyObject_IsTrue(value) == 1) {
        void *negated = PyNumber_Negative(value);
        Py_DecRef(value);
        value = negated;
    }

    free(writer->digits);
    free(writer);
    return value;
}

void _PyLongWriter_Discard(_PyLongWriter *writer)
{
    if (writer == NULL) {
        return;
    }
    free(writer->digits);
    writer->digits = NULL;
    free(writer);
}

const PyLongLayout *PyLong_GetNativeLayout(void)
{
    return _PyLong_GetNativeLayout();
}

int PyLong_Export(void *obj, PyLongExport *export_long)
{
    return _PyLong_Export(obj, export_long);
}

void PyLong_FreeExport(PyLongExport *export_long)
{
    _PyLong_FreeExport(export_long);
}

_PyLongWriter *PyLongWriter_Create(int negative, Py_ssize_t ndigits, void **digits)
{
    return _PyLongWriter_Create(negative, ndigits, digits);
}

void *PyLongWriter_Finish(_PyLongWriter *writer)
{
    return _PyLongWriter_Finish(writer);
}

void PyLongWriter_Discard(_PyLongWriter *writer)
{
    _PyLongWriter_Discard(writer);
}

void *_PyLong_FromByteArray(
    const unsigned char *bytes,
    size_t n,
    int little_endian,
    int is_signed
)
{
    int flags = little_endian ? 1 : 0;
    if (!is_signed) {
        flags |= 4;
    }
    return PyLong_FromNativeBytes(bytes, n, flags);
}

void *_PyLong_FromGid(gid_t gid)
{
    return PyLong_FromUnsignedLong((unsigned long)gid);
}

void *_PyLong_Format(void *obj, int base)
{
    return PyNumber_ToBase(obj, base);
}

void *_PyLong_GCD(void *a, void *b)
{
    if (a == NULL || b == NULL) {
        PyErr_BadInternalCall();
        return NULL;
    }
    void *left = PyNumber_Absolute(a);
    if (left == NULL) {
        return NULL;
    }
    void *right = PyNumber_Absolute(b);
    if (right == NULL) {
        Py_DecRef(left);
        return NULL;
    }

    while (1) {
        int is_nonzero = PyObject_IsTrue(right);
        if (is_nonzero < 0) {
            Py_DecRef(left);
            Py_DecRef(right);
            return NULL;
        }
        if (!is_nonzero) {
            Py_DecRef(right);
            return left;
        }
        void *next = PyNumber_Remainder(left, right);
        if (next == NULL) {
            Py_DecRef(left);
            Py_DecRef(right);
            return NULL;
        }
        Py_DecRef(left);
        left = right;
        right = next;
    }
}

void *_PyThreadState_GetCurrent(void)
{
    return PyThreadState_GetUnchecked();
}

void _PyErr_ChainExceptions1(void *exc)
{
    if (exc == NULL) {
        return;
    }
    if (PyErr_Occurred() != NULL) {
        void *current = PyErr_GetRaisedException();
        if (current == NULL) {
            PyErr_SetRaisedException(exc);
            return;
        }
        PyException_SetContext(current, exc);
        PyErr_SetRaisedException(current);
        return;
    }
    PyErr_SetRaisedException(exc);
}

int _PyEval_SetProfile(void *tstate, void *func, void *arg)
{
    (void)tstate;
    (void)func;
    (void)arg;
    return 0;
}

int _PySys_GetOptionalAttrString(const char *name, void **result)
{
    if (result == NULL) {
        PyErr_SetString((void *)&PyExc_TypeError, "_PySys_GetOptionalAttrString missing result");
        return -1;
    }
    *result = NULL;
    if (name == NULL) {
        PyErr_SetString((void *)&PyExc_TypeError, "_PySys_GetOptionalAttrString missing name");
        return -1;
    }
    void *sys_mod = PyImport_ImportModule("sys");
    if (sys_mod == NULL) {
        return -1;
    }
    void *attr = PyObject_GetAttrString(sys_mod, name);
    Py_DecRef(sys_mod);
    if (attr == NULL) {
        if (PyErr_ExceptionMatches((void *)&PyExc_AttributeError)) {
            PyErr_Clear();
            *result = NULL;
            return 0;
        }
        return -1;
    }
    *result = attr;
    return 1;
}

void *_PyUnicode_AsUTF8String(void *object)
{
    return PyUnicode_AsUTF8String(object);
}

void *_PyType_LookupRef(void *type, void *name)
{
    void *value = _PyType_Lookup(type, name);
    if (value != NULL) {
        Py_IncRef(value);
    }
    return value;
}

void *_PyList_AppendTakeRefListResize(void *list, void *item)
{
    if (list == NULL || item == NULL) {
        PyErr_BadInternalCall();
        return NULL;
    }
    if (PyList_Append(list, item) < 0) {
        Py_DecRef(item);
        return NULL;
    }
    Py_DecRef(item);
    return list;
}

void *_PyObject_CallMethod(void *object, void *name, const char *format, ...)
{
    if (object == NULL || name == NULL) {
        PyErr_BadInternalCall();
        return NULL;
    }
    void *callable = PyObject_GetAttr(object, name);
    if (callable == NULL) {
        return NULL;
    }

    void *args_tuple = NULL;
    va_list ap;
    va_start(ap, format);
    if (format == NULL || format[0] == '\0') {
        args_tuple = PyTuple_New(0);
    } else {
        void *built = Py_VaBuildValue(format, ap);
        if (built == NULL) {
            va_end(ap);
            Py_DecRef(callable);
            return NULL;
        }
        void *built_type = PyObject_Type(built);
        int is_tuple = 0;
        if (built_type != NULL) {
            is_tuple = PyType_IsSubtype(built_type, (void *)&PyTuple_Type);
            Py_DecRef(built_type);
        }
        if (is_tuple) {
            args_tuple = built;
        } else {
            void *items[1] = {built};
            args_tuple = pyrs_capi_tuple_pack_from_array(1, items);
            Py_DecRef(built);
        }
    }
    va_end(ap);

    if (args_tuple == NULL) {
        Py_DecRef(callable);
        return NULL;
    }
    void *result = PyObject_CallObject(callable, args_tuple);
    Py_DecRef(args_tuple);
    Py_DecRef(callable);
    return result;
}

void *_PyObject_MakeTpCall(
    void *callable,
    void *const *args,
    Py_ssize_t nargs,
    void *keywords
)
{
    void *tuple = pyrs_capi_tuple_pack_from_array(nargs, args);
    if (tuple == NULL) {
        return NULL;
    }
    void *result = PyObject_Call(callable, tuple, keywords);
    Py_DecRef(tuple);
    return result;
}

void *_Py_CheckFunctionResult(
    void *tstate,
    void *callable,
    void *result,
    const char *where
)
{
    (void)tstate;
    if (result == NULL) {
        if (PyErr_Occurred() == NULL) {
            if (callable != NULL) {
                PyErr_Format(
                    (void *)&PyExc_SystemError,
                    "%R returned NULL without setting an exception",
                    callable
                );
            } else if (where != NULL) {
                PyErr_Format(
                    (void *)&PyExc_SystemError,
                    "%s returned NULL without setting an exception",
                    where
                );
            } else {
                PyErr_SetString(
                    (void *)&PyExc_SystemError,
                    "function returned NULL without setting an exception"
                );
            }
        }
        return NULL;
    }
    if (PyErr_Occurred() != NULL) {
        Py_DecRef(result);
        if (callable != NULL) {
            PyErr_Format(
                (void *)&PyExc_SystemError,
                "%R returned a result with an exception set",
                callable
            );
        } else if (where != NULL) {
            PyErr_Format(
                (void *)&PyExc_SystemError,
                "%s returned a result with an exception set",
                where
            );
        } else {
            PyErr_SetString(
                (void *)&PyExc_SystemError,
                "function returned a result with an exception set"
            );
        }
        return NULL;
    }
    return result;
}

int PyList_Clear(void *list)
{
    if (list == NULL) {
        PyErr_BadInternalCall();
        return -1;
    }
    void *result = PyObject_CallMethod(list, "clear", "");
    if (result == NULL) {
        return -1;
    }
    Py_DecRef(result);
    return 0;
}

int PyTuple_Resize(void **ptuple, Py_ssize_t newsize)
{
    if (ptuple == NULL || newsize < 0) {
        PyErr_BadInternalCall();
        return -1;
    }
    void *old_tuple = *ptuple;
    if (old_tuple == NULL) {
        *ptuple = PyTuple_New(newsize);
        return *ptuple == NULL ? -1 : 0;
    }

    Py_ssize_t old_size = PyTuple_Size(old_tuple);
    if (old_size < 0) {
        return -1;
    }

    void *new_tuple = PyTuple_New(newsize);
    if (new_tuple == NULL) {
        return -1;
    }

    Py_ssize_t copy_size = old_size < newsize ? old_size : newsize;
    for (Py_ssize_t i = 0; i < copy_size; i++) {
        void *item = PyTuple_GetItem(old_tuple, i);
        if (item == NULL) {
            Py_DecRef(new_tuple);
            return -1;
        }
        Py_IncRef(item);
        if (PyTuple_SetItem(new_tuple, i, item) < 0) {
            Py_DecRef(item);
            Py_DecRef(new_tuple);
            return -1;
        }
    }

    Py_DecRef(old_tuple);
    *ptuple = new_tuple;
    return 0;
}

int _PyTuple_Resize(void **ptuple, Py_ssize_t newsize)
{
    return PyTuple_Resize(ptuple, newsize);
}

static int pyrs_unicode_writer_append(_PyUnicodeWriter *writer, void *piece)
{
    if (writer == NULL || piece == NULL) {
        return -1;
    }
    if (writer->buffer == NULL) {
        writer->buffer = piece;
        return 0;
    }
    void *combined = PyUnicode_Concat(writer->buffer, piece);
    Py_DecRef(piece);
    if (combined == NULL) {
        return -1;
    }
    Py_DecRef(writer->buffer);
    writer->buffer = combined;
    return 0;
}

void _PyUnicodeWriter_Init(_PyUnicodeWriter *writer)
{
    if (writer == NULL) {
        return;
    }
    memset(writer, 0, sizeof(*writer));
    writer->kind = 1;
    writer->maxchar = 127;
    writer->min_char = 127;
}

int _PyUnicodeWriter_PrepareInternal(
    _PyUnicodeWriter *writer,
    Py_ssize_t length,
    uint32_t maxchar
)
{
    if (writer == NULL || length < 0) {
        PyErr_BadInternalCall();
        return -1;
    }
    if (maxchar > writer->maxchar) {
        writer->maxchar = maxchar;
    }
    if (maxchar > 0xFFFF) {
        writer->kind = 4;
    } else if (maxchar > 0xFF) {
        writer->kind = 2;
    } else {
        writer->kind = 1;
    }
    if (writer->size < writer->pos + length) {
        writer->size = writer->pos + length;
    }
    return 0;
}

int _PyUnicodeWriter_WriteChar(_PyUnicodeWriter *writer, uint32_t ch)
{
    if (_PyUnicodeWriter_PrepareInternal(writer, 1, ch) < 0) {
        return -1;
    }
    void *piece = PyUnicode_FromOrdinal((int)ch);
    if (piece == NULL) {
        return -1;
    }
    if (pyrs_unicode_writer_append(writer, piece) < 0) {
        return -1;
    }
    writer->pos += 1;
    return 0;
}

int _PyUnicodeWriter_WriteStr(_PyUnicodeWriter *writer, void *str)
{
    if (writer == NULL || str == NULL) {
        PyErr_BadInternalCall();
        return -1;
    }
    Py_ssize_t len = PyUnicode_GetLength(str);
    if (len < 0) {
        return -1;
    }
    if (_PyUnicodeWriter_PrepareInternal(writer, len, writer->maxchar) < 0) {
        return -1;
    }
    Py_IncRef(str);
    if (pyrs_unicode_writer_append(writer, str) < 0) {
        return -1;
    }
    writer->pos += len;
    return 0;
}

void *_PyUnicodeWriter_Finish(_PyUnicodeWriter *writer)
{
    if (writer == NULL) {
        PyErr_BadInternalCall();
        return NULL;
    }
    if (writer->buffer == NULL) {
        writer->buffer = PyUnicode_FromStringAndSize("", 0);
        if (writer->buffer == NULL) {
            return NULL;
        }
    }
    void *result = writer->buffer;
    writer->buffer = NULL;
    writer->data = NULL;
    writer->size = 0;
    writer->pos = 0;
    writer->readonly = 0;
    return result;
}

void _PyUnicodeWriter_Dealloc(_PyUnicodeWriter *writer)
{
    if (writer == NULL) {
        return;
    }
    if (writer->buffer != NULL) {
        Py_DecRef(writer->buffer);
        writer->buffer = NULL;
    }
    writer->data = NULL;
    writer->size = 0;
    writer->pos = 0;
}

void *_Py_strhex(const char *argbuf, const Py_ssize_t arglen)
{
    if (arglen < 0) {
        PyErr_SetString((void *)&PyExc_ValueError, "negative arglen");
        return NULL;
    }
    static const char hexdigits[] = "0123456789abcdef";
    size_t out_len = (size_t)arglen * 2;
    char *out = (char *)malloc(out_len + 1);
    if (out == NULL) {
        PyErr_NoMemory();
        return NULL;
    }
    for (Py_ssize_t i = 0; i < arglen; i++) {
        unsigned char b = (unsigned char)argbuf[i];
        out[(size_t)i * 2] = hexdigits[b >> 4];
        out[(size_t)i * 2 + 1] = hexdigits[b & 0x0F];
    }
    out[out_len] = '\0';
    void *result = PyUnicode_FromStringAndSize(out, (Py_ssize_t)out_len);
    free(out);
    return result;
}

Py_uhash_t _Py_hashtable_hash_ptr(const void *key)
{
    uintptr_t x = (uintptr_t)key;
    if (sizeof(uintptr_t) >= 2) {
        x = (x >> 4) | (x << (8 * sizeof(uintptr_t) - 4));
    }
    return (Py_uhash_t)x;
}

int _Py_hashtable_compare_direct(const void *key1, const void *key2)
{
    return key1 == key2;
}

static _Py_hashtable_entry_t *pyrs_hashtable_get_entry(_Py_hashtable_t *ht, const void *key)
{
    if (ht == NULL || ht->nbuckets == 0) {
        return NULL;
    }
    Py_uhash_t hash = ht->hash_func ? ht->hash_func(key) : _Py_hashtable_hash_ptr(key);
    size_t bucket_index = (size_t)(hash % ht->nbuckets);
    _Py_slist_item_t *item = ht->buckets[bucket_index].head;
    while (item != NULL) {
        _Py_hashtable_entry_t *entry = (_Py_hashtable_entry_t *)item;
        if (entry->key_hash == hash) {
            int equal = ht->compare_func
                ? ht->compare_func(entry->key, key)
                : _Py_hashtable_compare_direct(entry->key, key);
            if (equal) {
                return entry;
            }
        }
        item = item->next;
    }
    return NULL;
}

static int pyrs_hashtable_resize(_Py_hashtable_t *ht, size_t next_buckets)
{
    _Py_slist_t *new_buckets = (_Py_slist_t *)ht->alloc.malloc(sizeof(_Py_slist_t) * next_buckets);
    if (new_buckets == NULL) {
        return -1;
    }
    memset(new_buckets, 0, sizeof(_Py_slist_t) * next_buckets);
    for (size_t i = 0; i < ht->nbuckets; i++) {
        _Py_slist_item_t *item = ht->buckets[i].head;
        while (item != NULL) {
            _Py_slist_item_t *next = item->next;
            _Py_hashtable_entry_t *entry = (_Py_hashtable_entry_t *)item;
            size_t bucket_index = (size_t)(entry->key_hash % next_buckets);
            entry->_Py_slist_item.next = new_buckets[bucket_index].head;
            new_buckets[bucket_index].head = &entry->_Py_slist_item;
            item = next;
        }
    }
    ht->alloc.free(ht->buckets);
    ht->buckets = new_buckets;
    ht->nbuckets = next_buckets;
    return 0;
}

_Py_hashtable_t *_Py_hashtable_new_full(
    _Py_hashtable_hash_func hash_func,
    _Py_hashtable_compare_func compare_func,
    _Py_hashtable_destroy_func key_destroy_func,
    _Py_hashtable_destroy_func value_destroy_func,
    _Py_hashtable_allocator_t *allocator
)
{
    _Py_hashtable_allocator_t alloc;
    alloc.malloc = allocator && allocator->malloc ? allocator->malloc : malloc;
    alloc.free = allocator && allocator->free ? allocator->free : free;

    _Py_hashtable_t *ht = (_Py_hashtable_t *)alloc.malloc(sizeof(_Py_hashtable_t));
    if (ht == NULL) {
        PyErr_NoMemory();
        return NULL;
    }
    memset(ht, 0, sizeof(*ht));
    ht->alloc = alloc;
    ht->nbuckets = 16;
    ht->buckets = (_Py_slist_t *)ht->alloc.malloc(sizeof(_Py_slist_t) * ht->nbuckets);
    if (ht->buckets == NULL) {
        ht->alloc.free(ht);
        PyErr_NoMemory();
        return NULL;
    }
    memset(ht->buckets, 0, sizeof(_Py_slist_t) * ht->nbuckets);
    ht->hash_func = hash_func ? hash_func : _Py_hashtable_hash_ptr;
    ht->compare_func = compare_func ? compare_func : _Py_hashtable_compare_direct;
    ht->key_destroy_func = key_destroy_func;
    ht->value_destroy_func = value_destroy_func;
    ht->get_entry_func = pyrs_hashtable_get_entry;
    return ht;
}

_Py_hashtable_t *_Py_hashtable_new(
    _Py_hashtable_hash_func hash_func,
    _Py_hashtable_compare_func compare_func
)
{
    return _Py_hashtable_new_full(hash_func, compare_func, NULL, NULL, NULL);
}

void _Py_hashtable_clear(_Py_hashtable_t *ht)
{
    if (ht == NULL) {
        return;
    }
    for (size_t i = 0; i < ht->nbuckets; i++) {
        _Py_slist_item_t *item = ht->buckets[i].head;
        while (item != NULL) {
            _Py_slist_item_t *next = item->next;
            _Py_hashtable_entry_t *entry = (_Py_hashtable_entry_t *)item;
            if (ht->key_destroy_func != NULL && entry->key != NULL) {
                ht->key_destroy_func((void *)entry->key);
            }
            if (ht->value_destroy_func != NULL && entry->value != NULL) {
                ht->value_destroy_func(entry->value);
            }
            ht->alloc.free(entry);
            item = next;
        }
        ht->buckets[i].head = NULL;
    }
    ht->nentries = 0;
}

void _Py_hashtable_destroy(_Py_hashtable_t *ht)
{
    if (ht == NULL) {
        return;
    }
    _Py_hashtable_clear(ht);
    if (ht->buckets != NULL) {
        ht->alloc.free(ht->buckets);
    }
    ht->alloc.free(ht);
}

int _Py_hashtable_set(_Py_hashtable_t *ht, const void *key, void *value)
{
    if (ht == NULL || key == NULL) {
        PyErr_BadInternalCall();
        return -1;
    }
    _Py_hashtable_entry_t *existing = pyrs_hashtable_get_entry(ht, key);
    if (existing != NULL) {
        if (ht->value_destroy_func != NULL && existing->value != NULL) {
            ht->value_destroy_func(existing->value);
        }
        existing->value = value;
        return 0;
    }
    if (ht->nentries > ht->nbuckets * 3 / 4) {
        if (pyrs_hashtable_resize(ht, ht->nbuckets * 2) < 0) {
            PyErr_NoMemory();
            return -1;
        }
    }
    _Py_hashtable_entry_t *entry = (_Py_hashtable_entry_t *)ht->alloc.malloc(sizeof(_Py_hashtable_entry_t));
    if (entry == NULL) {
        PyErr_NoMemory();
        return -1;
    }
    memset(entry, 0, sizeof(*entry));
    entry->key = key;
    entry->value = value;
    entry->key_hash = ht->hash_func ? ht->hash_func(key) : _Py_hashtable_hash_ptr(key);
    size_t bucket_index = (size_t)(entry->key_hash % ht->nbuckets);
    entry->_Py_slist_item.next = ht->buckets[bucket_index].head;
    ht->buckets[bucket_index].head = &entry->_Py_slist_item;
    ht->nentries += 1;
    return 0;
}

void *_Py_hashtable_get(_Py_hashtable_t *ht, const void *key)
{
    _Py_hashtable_entry_t *entry = pyrs_hashtable_get_entry(ht, key);
    return entry ? entry->value : NULL;
}

double _Py_c_abs(Py_complex z)
{
    errno = 0;
    return hypot(z.real, z.imag);
}

Py_complex _Py_c_diff(Py_complex a, Py_complex b)
{
    Py_complex out = {a.real - b.real, a.imag - b.imag};
    return out;
}

Py_complex _Py_c_neg(Py_complex z)
{
    Py_complex out = {-z.real, -z.imag};
    return out;
}

Py_complex _Py_c_quot(Py_complex a, Py_complex b)
{
    Py_complex out = {0.0, 0.0};
    double denom = b.real * b.real + b.imag * b.imag;
    if (denom == 0.0) {
        errno = EDOM;
        return out;
    }
    out.real = (a.real * b.real + a.imag * b.imag) / denom;
    out.imag = (a.imag * b.real - a.real * b.imag) / denom;
    return out;
}

int PyTime_PerfCounterRaw(PyTime_t *result)
{
    if (result == NULL) {
        PyErr_BadInternalCall();
        return -1;
    }
    struct timespec ts;
    if (pyrs_clock_gettime_monotonic(&ts) != 0) {
        return -1;
    }
    *result = (PyTime_t)ts.tv_sec * 1000000000LL + (PyTime_t)ts.tv_nsec;
    return 0;
}

PyTime_t _PyTime_FromSeconds(int seconds)
{
    return (PyTime_t)seconds * 1000000000LL;
}

int _PyTime_FromLong(PyTime_t *t, void *obj)
{
    if (t == NULL || obj == NULL) {
        PyErr_BadInternalCall();
        return -1;
    }
    long long value = PyLong_AsLongLong(obj);
    if (value == -1 && PyErr_Occurred() != NULL) {
        return -1;
    }
    *t = (PyTime_t)value;
    return 0;
}

int _PyTime_FromSecondsObject(PyTime_t *t, void *obj, int round)
{
    if (t == NULL || obj == NULL) {
        PyErr_BadInternalCall();
        return -1;
    }
    double seconds = 0.0;
    if (obj == (void *)&_Py_NoneStruct) {
        PyErr_SetString((void *)&PyExc_TypeError, "None is not a valid timeout value");
        return -1;
    }
    seconds = PyFloat_AsDouble(obj);
    if (seconds == -1.0 && PyErr_Occurred() != NULL) {
        PyErr_Clear();
        long long as_int = PyLong_AsLongLong(obj);
        if (as_int == -1 && PyErr_Occurred() != NULL) {
            return -1;
        }
        seconds = (double)as_int;
    }
    double nanos = seconds * 1000000000.0;
    if (round == 1) {
        nanos = ceil(nanos);
    } else if (round == 0) {
        nanos = floor(nanos);
    } else {
        nanos = nearbyint(nanos);
    }
    *t = (PyTime_t)nanos;
    return 0;
}

PyTime_t _PyDeadline_Init(PyTime_t timeout)
{
    PyTime_t now = 0;
    (void)PyTime_PerfCounterRaw(&now);
    return now + timeout;
}

PyTime_t _PyDeadline_Get(PyTime_t deadline)
{
    PyTime_t now = 0;
    (void)PyTime_PerfCounterRaw(&now);
    return deadline - now;
}

int _PyParkingLot_Park(
    const void *address,
    const void *expected,
    size_t address_size,
    PyTime_t timeout_ns,
    void *park_arg,
    int detach
)
{
    (void)park_arg;
    (void)detach;
    if (address == NULL || expected == NULL) {
        return -1;
    }
    if (!(address_size == 1 || address_size == 2 || address_size == 4 || address_size == 8)) {
        return -1;
    }
    if (memcmp(address, expected, address_size) != 0) {
        return -1;
    }
    if (timeout_ns == 0) {
        return -2;
    }
    return 0;
}

typedef void _Py_unpark_fn_t(void *arg, void *park_arg, int has_more_waiters);

void _PyParkingLot_Unpark(const void *address, _Py_unpark_fn_t *fn, void *arg)
{
    (void)address;
    if (fn != NULL) {
        fn(arg, NULL, 0);
    }
}

void _PyParkingLot_UnparkAll(const void *address)
{
    (void)address;
}

void _PyParkingLot_AfterFork(void)
{
}

void PyErr_FormatUnraisable(const char *format, ...)
{
    if (format != NULL && format[0] != '\0') {
        va_list ap;
        va_start(ap, format);
        void *message = PyErr_FormatV((void *)&PyExc_TypeError, format, ap);
        va_end(ap);
        if (message != NULL) {
            Py_DecRef(message);
        }
        PyErr_Clear();
    }
    PyErr_WriteUnraisable(NULL);
}

const unsigned int _Py_ctype_table[256] = {
    0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 8, 8, 8, 8, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 0, 0, 0, 0, 0, 0,
    0, 18, 18, 18, 18, 18, 18, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0,
    0, 17, 17, 17, 17, 17, 17, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
};

unsigned char _Py_ctype_tolower[256];
unsigned char _Py_ctype_toupper[256];

typedef union {
    unsigned char uc[24];
    struct {
        Py_hash_t prefix;
        Py_hash_t suffix;
    } fnv;
    struct {
        uint64_t k0;
        uint64_t k1;
    } siphash;
    struct {
        unsigned char padding[16];
        Py_hash_t suffix;
    } djbx33a;
    struct {
        unsigned char padding[16];
        Py_hash_t hashsalt;
    } expat;
} _Py_HashSecret_t;

_Py_HashSecret_t _Py_HashSecret = {0};

__attribute__((used, visibility("default")))
char *(*PyOS_ReadlineFunctionPointer)(FILE *, FILE *, const char *) = NULL;
__attribute__((used, visibility("default")))
char *(*_PyOS_ReadlineFunctionPointer)(FILE *, FILE *, const char *) = NULL;
__attribute__((used, visibility("default")))
void *_PyOS_ReadlineTState = NULL;

__attribute__((used, visibility("default")))
char *_Py_SetLocaleFromEnv(int category)
{
    return setlocale(category, "");
}

int _Py_Gid_Converter(void *obj, gid_t *out)
{
    if (out == NULL) {
        PyErr_BadInternalCall();
        return 0;
    }
    unsigned long value = PyLong_AsUnsignedLong(obj);
    if (PyErr_Occurred() != NULL) {
        return 0;
    }
    *out = (gid_t)value;
    return 1;
}

static int pyrs_clock_gettime_monotonic(struct timespec *ts)
{
    if (ts == NULL) {
        return -1;
    }
#if defined(_WIN32)
    LARGE_INTEGER frequency;
    LARGE_INTEGER counter;
    if (QueryPerformanceFrequency(&frequency) == 0 ||
        QueryPerformanceCounter(&counter) == 0 ||
        frequency.QuadPart <= 0) {
        return -1;
    }
    ts->tv_sec = (time_t)(counter.QuadPart / frequency.QuadPart);
    ts->tv_nsec = (long)((counter.QuadPart % frequency.QuadPart) * 1000000000LL / frequency.QuadPart);
    return 0;
#elif defined(CLOCK_MONOTONIC)
    return clock_gettime(CLOCK_MONOTONIC, ts);
#else
    return timespec_get(ts, TIME_UTC) == TIME_UTC ? 0 : -1;
#endif
}

static int pyrs_clock_gettime_realtime(struct timespec *ts)
{
    if (ts == NULL) {
        return -1;
    }
#if defined(CLOCK_REALTIME)
    return clock_gettime(CLOCK_REALTIME, ts);
#else
    return timespec_get(ts, TIME_UTC) == TIME_UTC ? 0 : -1;
#endif
}

PYRS_CONSTRUCTOR_FUNC(pyrs_init_pyctype_tables)
{
    for (int i = 0; i < 256; i++) {
        if (i >= 'A' && i <= 'Z') {
            _Py_ctype_tolower[i] = (unsigned char)(i + ('a' - 'A'));
        } else {
            _Py_ctype_tolower[i] = (unsigned char)i;
        }
        if (i >= 'a' && i <= 'z') {
            _Py_ctype_toupper[i] = (unsigned char)(i - ('a' - 'A'));
        } else {
            _Py_ctype_toupper[i] = (unsigned char)i;
        }
    }

    struct timespec ts;
    if (pyrs_clock_gettime_realtime(&ts) == 0) {
        _Py_HashSecret.siphash.k0 = (uint64_t)ts.tv_sec ^ ((uint64_t)ts.tv_nsec << 16);
        _Py_HashSecret.siphash.k1 = ((uint64_t)ts.tv_nsec << 32) ^ (uint64_t)(uintptr_t)&_Py_HashSecret;
        _Py_HashSecret.expat.hashsalt = (Py_hash_t)(_Py_HashSecret.siphash.k0 ^ _Py_HashSecret.siphash.k1);
    }
}
