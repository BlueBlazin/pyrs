#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <ctype.h>
#include <math.h>
#include <signal.h>

typedef intptr_t Py_ssize_t;
typedef void (*PyOS_sighandler_t)(int);

extern void *pyrs_capi_tuple_pack_from_array(Py_ssize_t n, void *const *items);
extern void pyrs_capi_set_error_message(const char *message);
extern void *pyrs_capi_pyerr_format_fallback(void *exception, const char *format);
extern void *pyrs_capi_pyerr_formatv_fallback(void *exception, const char *format, void *vargs);
extern void pyrs_capi_sys_write_stdout(const char *text);
extern void pyrs_capi_sys_write_stderr(const char *text);
extern int pyrs_capi_sys_audit_noargs(const char *event);

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
extern void *PyFloat_FromDouble(double value);
extern void *PyBool_FromLong(long value);
extern void *PyUnicode_FromStringAndSize(const char *value, Py_ssize_t size);
extern int PyBytes_AsStringAndSize(void *obj, char **buffer, Py_ssize_t *len);
extern void *PyBytes_FromStringAndSize(const char *value, Py_ssize_t size);
extern void *PyObject_Call(void *callable, void *args, void *kwargs);
extern void *PyObject_CallObject(void *callable, void *args);
extern void *PyObject_GetAttr(void *object, void *name);
extern void *PyObject_GetAttrString(void *object, const char *name);
extern int PyObject_IsTrue(void *object);
extern int PyType_IsSubtype(void *subtype, void *type);
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
    (void)format;
    va_list ap;
    va_start(ap, format);
    va_end(ap);
    return pyrs_capi_sys_audit_noargs(event);
}

void *PyErr_Format(void *exception, const char *format, ...)
{
    (void)exception;
    va_list ap;
    va_start(ap, format);
    va_end(ap);
    return pyrs_capi_pyerr_format_fallback(exception, format);
}

void *PyErr_FormatV(void *exception, const char *format, va_list vargs)
{
    (void)vargs;
    return pyrs_capi_pyerr_formatv_fallback(exception, format, NULL);
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
        if (*f == 'l' && (f[1] == 'd' || f[1] == 'u')) {
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
                Py_IncRef(value);
            }
            token_index++;
            continue;
        }

        if (token == 'O' && p[1] == '&') {
            p++;
            arg_converter_fn converter = va_arg(*ap, arg_converter_fn);
            void *output = va_arg(*ap, void *);
            if (!present) {
                if (!optional) {
                    free(spec);
                    pyrs_capi_set_error_message("missing required argument");
                    return 0;
                }
                token_index++;
                continue;
            }
            if (converter == NULL || converter(value, output) == 0) {
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
                Py_IncRef(value);
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
    va_list ap;
    va_start(ap, keywords);
    int result = parse_args_and_keywords_va(args, kwargs, format, keywords, &ap);
    va_end(ap);
    return result;
}
