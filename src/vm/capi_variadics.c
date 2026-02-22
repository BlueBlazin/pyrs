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
#include <dlfcn.h>
#include <wchar.h>

typedef intptr_t Py_ssize_t;
typedef void (*PyOS_sighandler_t)(int);

extern void *pyrs_capi_tuple_pack_from_array(Py_ssize_t n, void *const *items);
extern void pyrs_capi_set_error_message(const char *message);
extern void *pyrs_capi_pyerr_format_fallback(void *exception, const char *format);
extern void *pyrs_capi_pyerr_formatv_fallback(void *exception, const char *format, void *vargs);
extern void pyrs_capi_sys_write_stdout(const char *text);
extern void pyrs_capi_sys_write_stderr(const char *text);
extern int pyrs_capi_sys_audit_noargs(const char *event);
extern int pyrs_capi_sys_audit_object(const char *event, void *args);
extern void *Py_VaBuildValue(const char *format, va_list ap);

extern void *PyTuple_New(Py_ssize_t size);
extern int PyTuple_SetItem(void *tuple, Py_ssize_t index, void *item);
extern Py_ssize_t PyTuple_Size(void *tuple);
extern void *PyTuple_GetItem(void *tuple, Py_ssize_t index);
extern void *PyList_New(Py_ssize_t size);
extern Py_ssize_t PyList_Size(void *list);
extern void *PyList_GetItem(void *list, Py_ssize_t index);
extern int PyList_Append(void *list, void *item);
extern void *PyDict_New(void);
extern void *PyDict_Keys(void *dict);
extern int PyDict_SetItem(void *dict, void *key, void *value);
extern void *PyDict_GetItemString(void *dict, const char *key);
extern void *PyLong_FromLong(long value);
extern void *PyLong_FromUnsignedLong(unsigned long value);
extern void *PyLong_FromLongLong(long long value);
extern void *PyLong_FromUnsignedLongLong(unsigned long long value);
extern void *PyLong_FromSsize_t(Py_ssize_t value);
extern long PyLong_AsLong(void *value);
extern void *PyFloat_FromDouble(double value);
extern void *PyBool_FromLong(long value);
extern void *PyUnicode_FromStringAndSize(const char *value, Py_ssize_t size);
extern void *PyUnicode_FromWideChar(const wchar_t *value, Py_ssize_t len);
extern int PyBytes_AsStringAndSize(void *obj, char **buffer, Py_ssize_t *len);
extern void *PyBytes_FromStringAndSize(const char *value, Py_ssize_t size);
extern void *PyObject_Call(void *callable, void *args, void *kwargs);
extern void *PyObject_CallObject(void *callable, void *args);
extern void *PyObject_GetAttr(void *object, void *name);
extern void *PyObject_GetAttrString(void *object, const char *name);
extern void *PyObject_Str(void *object);
extern void *PyObject_Repr(void *object);
extern void *PyObject_ASCII(void *object);
extern int PyObject_IsTrue(void *object);
extern int PyType_IsSubtype(void *subtype, void *type);
extern const char *PyUnicode_AsUTF8(void *object);
extern void PyErr_BadInternalCall(void);
extern void *PyErr_Occurred(void);
extern void Py_IncRef(void *object);
extern void Py_DecRef(void *object);
extern char _Py_NoneStruct;
extern char PyDict_Type;
extern char PyTuple_Type;
extern char PyUnicode_Type;
extern char PyBytes_Type;

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
    if (message != NULL &&
        getenv("PYRS_TRACE_PYERR_FORMAT_CALLER") != NULL &&
        (strstr(message, "not subscriptable") != NULL ||
         strstr(message, "dot() missing required argument") != NULL)) {
        void *ret0 = __builtin_return_address(0);
        void *ret1 = __builtin_return_address(1);
        void *ret2 = __builtin_return_address(2);
        Dl_info info0;
        Dl_info info1;
        Dl_info info2;
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
        if (ret1 != NULL && dladdr(ret1, &info1) != 0) {
            fprintf(
                stderr,
                "[pyerr-format-caller] return1=%p sym1=%s image1=%s\n",
                ret1,
                info1.dli_sname != NULL ? info1.dli_sname : "<unknown>",
                info1.dli_fname != NULL ? info1.dli_fname : "<unknown>"
            );
        }
        if (ret2 != NULL && dladdr(ret2, &info2) != 0) {
            fprintf(
                stderr,
                "[pyerr-format-caller] return2=%p sym2=%s image2=%s\n",
                ret2,
                info2.dli_sname != NULL ? info2.dli_sname : "<unknown>",
                info2.dli_fname != NULL ? info2.dli_fname : "<unknown>"
            );
        }
    }
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

typedef struct {
    Py_ssize_t ob_refcnt;
    void *ob_type;
} PyObjectHeadCompat;

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
    while (*cursor != '\0' && *cursor != ':' && *cursor != ';') {
        cursor++;
    }
    size_t format_len = (size_t)(cursor - format);
    char *spec = (char *)malloc(format_len + 1);
    if (spec == NULL) {
        pyrs_capi_set_error_message("PyArg_ParseTupleAndKeywords failed allocating format copy");
        return 0;
    }
    memcpy(spec, format, format_len);
    spec[format_len] = '\0';

    Py_ssize_t positional_total = 0;
    if (args != NULL) {
        positional_total = PyTuple_Size(args);
        if (positional_total < 0) {
            free(spec);
            return 0;
        }
    }

    int optional = 0;
    int keyword_only = 0;
    Py_ssize_t positional_index = 0;
    Py_ssize_t token_index = 0;
    for (const char *p = spec; *p != '\0'; p++) {
        char token = *p;
        if (token == ' ' || token == '\t' || token == ',' || token == ':') {
            continue;
        }
        if (token == '|') {
            optional = 1;
            continue;
        }
        if (token == '$') {
            keyword_only = 1;
            continue;
        }

        const char *keyword_name = NULL;
        if (keywords != NULL) {
            keyword_name = keywords[token_index];
        }

        void *value = NULL;
        int present = 0;
        if (!keyword_only && positional_index < positional_total) {
            value = PyTuple_GetItem(args, positional_index++);
            present = 1;
            if (kwargs != NULL && keyword_name != NULL) {
                void *duplicate = PyDict_GetItemString(kwargs, keyword_name);
                if (duplicate != NULL) {
                    free(spec);
                    pyrs_capi_set_error_message(
                        "PyArg_ParseTupleAndKeywords received duplicate positional/keyword argument"
                    );
                    return 0;
                }
            }
        } else if (kwargs != NULL && keyword_name != NULL) {
            value = PyDict_GetItemString(kwargs, keyword_name);
            if (value != NULL) {
                present = 1;
            }
        }

        if (token == 'O' && p[1] == '!') {
            p++;
            void *expected_type = va_arg(*ap, void *);
            void **output = va_arg(*ap, void **);
            if (!present) {
                if (!optional) {
                    free(spec);
                    pyrs_capi_set_error_message("missing required argument");
                    return 0;
                }
                token_index++;
                continue;
            }
            if (!object_is_instance_of_type(value, expected_type)) {
                free(spec);
                pyrs_capi_set_error_message(
                    "PyArg_ParseTupleAndKeywords argument has incorrect type"
                );
                return 0;
            }
            if (output != NULL) {
                *output = value;
            }
            token_index++;
            continue;
        }

        if (token == 'O' && p[1] == '&') {
            p++;
            arg_converter_fn converter = va_arg(*ap, arg_converter_fn);
            void *output = va_arg(*ap, void *);
            if (getenv("PYRS_TRACE_PYARG_VA") != NULL) {
                fprintf(
                    stderr,
                    "[pyarg-va] O& token present=%d optional=%d converter=%p output=%p\n",
                    present,
                    optional,
                    (const void *)converter,
                    output
                );
            }
            if (!present) {
                if (!optional) {
                    free(spec);
                    pyrs_capi_set_error_message("missing required argument");
                    return 0;
                }
                token_index++;
                continue;
            }
            int converted = 0;
            if (converter != NULL) {
                converted = converter(value, output);
            }
            if (getenv("PYRS_TRACE_PYARG_VA") != NULL) {
                fprintf(
                    stderr,
                    "[pyarg-va] O& converter result=%d value=%p\n",
                    converted,
                    value
                );
            }
            if (converter == NULL || converted == 0) {
                free(spec);
                if (PyErr_Occurred() == NULL) {
                    pyrs_capi_set_error_message("PyArg_ParseTupleAndKeywords converter failed");
                }
                return 0;
            }
            token_index++;
            continue;
        }

        if (token == 'p') {
            int *output = va_arg(*ap, int *);
            if (!present) {
                if (!optional) {
                    free(spec);
                    pyrs_capi_set_error_message("missing required argument");
                    return 0;
                }
                token_index++;
                continue;
            }
            int truth = PyObject_IsTrue(value);
            if (truth < 0) {
                free(spec);
                return 0;
            }
            if (output != NULL) {
                *output = truth ? 1 : 0;
            }
            token_index++;
            continue;
        }

        if (token == 'i') {
            int *output = va_arg(*ap, int *);
            if (!present) {
                if (!optional) {
                    free(spec);
                    pyrs_capi_set_error_message("missing required argument");
                    return 0;
                }
                token_index++;
                continue;
            }
            long parsed = PyLong_AsLong(value);
            if (parsed == -1 && PyErr_Occurred() != NULL) {
                free(spec);
                return 0;
            }
            if (parsed < INT_MIN || parsed > INT_MAX) {
                free(spec);
                pyrs_capi_set_error_message(
                    "PyArg_ParseTupleAndKeywords integer out of range for 'i'"
                );
                return 0;
            }
            if (output != NULL) {
                *output = (int)parsed;
            }
            token_index++;
            continue;
        }

        if (token == 'O') {
            void **output = va_arg(*ap, void **);
            if (!present) {
                if (!optional) {
                    free(spec);
                    pyrs_capi_set_error_message("missing required argument");
                    return 0;
                }
                token_index++;
                continue;
            }
            if (output != NULL) {
                *output = value;
            }
            token_index++;
            continue;
        }

        if (token == 'U') {
            void **output = va_arg(*ap, void **);
            if (!present) {
                if (!optional) {
                    free(spec);
                    pyrs_capi_set_error_message("missing required argument");
                    return 0;
                }
                token_index++;
                continue;
            }
            if (!object_is_instance_of_type(value, (void *)&PyUnicode_Type)) {
                free(spec);
                pyrs_capi_set_error_message(
                    "PyArg_ParseTupleAndKeywords argument has incorrect type"
                );
                return 0;
            }
            if (output != NULL) {
                *output = value;
            }
            token_index++;
            continue;
        }

        {
            char message[160];
            snprintf(
                message,
                sizeof(message),
                "PyArg_ParseTupleAndKeywords token '%c' is not implemented (spec=\"%s\")",
                token,
                spec
            );
            free(spec);
            pyrs_capi_set_error_message(message);
        }
        return 0;
    }

    if (positional_index < positional_total) {
        free(spec);
        pyrs_capi_set_error_message("too many positional arguments");
        return 0;
    }

    free(spec);
    return 1;
}

static int count_old_style_format_args(const char *format, int *min_count, int *max_count)
{
    int min = -1;
    int max = 0;
    int level = 0;
    const char *cursor = format;
    while (*cursor != '\0' && *cursor != ':' && *cursor != ';') {
        char token = *cursor++;
        switch (token) {
            case '(':
                if (level == 0) {
                    max++;
                }
                level++;
                break;
            case ')':
                if (level > 0) {
                    level--;
                }
                break;
            case '|':
                if (level == 0) {
                    min = max;
                }
                break;
            default:
                if (level == 0 && ((token >= 'A' && token <= 'Z') || (token >= 'a' && token <= 'z'))) {
                    if (token != 'e') {
                        max++;
                    }
                }
                break;
        }
    }
    if (min < 0) {
        min = max;
    }
    *min_count = min;
    *max_count = max;
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
