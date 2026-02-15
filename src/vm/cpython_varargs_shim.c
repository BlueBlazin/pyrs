#include <stdarg.h>
#include <stddef.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>

typedef void PyObject;

extern PyObject *PyTuple_New(long size);
extern int PyTuple_SetItem(PyObject *tuple, long index, PyObject *item);
extern PyObject *PyDict_New(void);
extern int PyDict_SetItem(PyObject *dict, PyObject *key, PyObject *value);
extern PyObject *PyDict_GetItemString(PyObject *dict, const char *key);
extern PyObject *PyObject_CallObject(PyObject *callable, PyObject *args);
extern PyObject *PyObject_GetAttrString(PyObject *obj, const char *attr);
extern PyObject *PyUnicode_FromString(const char *s);
extern const char *PyUnicode_AsUTF8(PyObject *obj);
extern PyObject *PyLong_FromLong(long value);
extern long PyLong_AsLong(PyObject *value);
extern unsigned long PyLong_AsUnsignedLong(PyObject *value);
extern PyObject *PyFloat_FromDouble(double value);
extern double PyFloat_AsDouble(PyObject *value);
extern int PyBytes_AsStringAndSize(PyObject *obj, char **buffer, long *size);
extern long PyTuple_Size(PyObject *tuple);
extern PyObject *PyTuple_GetItem(PyObject *tuple, long index);
extern int PyObject_IsTrue(PyObject *obj);
extern int PyObject_IsInstance(PyObject *obj, PyObject *cls);
extern PyObject *PyErr_Occurred(void);
extern void PyErr_Clear(void);
extern void Py_IncRef(PyObject *obj);
extern void Py_DecRef(PyObject *obj);
extern void PyErr_SetString(PyObject *exc, const char *msg);
extern PyObject *PyExc_RuntimeError;
extern PyObject _Py_NoneStruct;

static void pyrs_py_buildvalue_error(const char *msg) {
    if (PyExc_RuntimeError != NULL) {
        PyErr_SetString(PyExc_RuntimeError, msg);
    }
}

PyObject *Py_BuildValue(const char *format, ...) {
    int trace = getenv("PYRS_TRACE_BUILDVALUE") != NULL;
    if (format == NULL) {
        pyrs_py_buildvalue_error("Py_BuildValue received null format");
        return NULL;
    }
    char normalized[128];
    size_t n = strlen(format);
    while (n > 0 && (format[n - 1] == '\n' || format[n - 1] == '\r' ||
                     format[n - 1] == '\t' || format[n - 1] == ' ')) {
        n--;
    }
    while (n >= 2 && format[n - 2] == '\\\\' &&
           (format[n - 1] == 'n' || format[n - 1] == 'r' || format[n - 1] == 't')) {
        n -= 2;
    }
    if (n >= sizeof(normalized)) {
        n = sizeof(normalized) - 1;
    }
    memcpy(normalized, format, n);
    normalized[n] = '\0';
    const char *spec = normalized;
    if (trace) {
        fprintf(stderr, "[py_buildvalue] format=%s\\n", spec);
    }

    va_list ap;
    va_start(ap, format);

    PyObject *result = NULL;

    if (strcmp(spec, "()") == 0) {
        result = PyTuple_New(0);
        va_end(ap);
        return result;
    }

    if (strcmp(spec, "O") == 0) {
        PyObject *value = va_arg(ap, PyObject *);
        if (value != NULL) {
            Py_IncRef(value);
        }
        va_end(ap);
        return value;
    }

    if (strcmp(spec, "N") == 0) {
        PyObject *value = va_arg(ap, PyObject *);
        va_end(ap);
        return value;
    }

    if (strcmp(spec, "s") == 0) {
        const char *text = va_arg(ap, const char *);
        result = PyUnicode_FromString(text != NULL ? text : "");
        va_end(ap);
        return result;
    }

    if (spec[0] == '(') {
        size_t len = strlen(spec);
        if (len >= 2 && spec[len - 1] == ')') {
            size_t item_count = 0;
            for (size_t i = 1; i + 1 < len; i++) {
                char unit = spec[i];
                if (unit == ',' || unit == ' ' || unit == '\\t') {
                    continue;
                }
                item_count++;
            }
            PyObject *tuple = PyTuple_New((long)item_count);
            if (tuple == NULL) {
                va_end(ap);
                return NULL;
            }
            size_t out_index = 0;
            for (size_t i = 1; i + 1 < len; i++) {
                char unit = spec[i];
                if (unit == ',' || unit == ' ' || unit == '\\t') {
                    continue;
                }
                PyObject *value = NULL;
                if (unit == 'O' || unit == 'N') {
                    value = va_arg(ap, PyObject *);
                    if (trace) {
                        fprintf(stderr, "[py_buildvalue] tuple unit=%c value=%p\\n", unit, (void *)value);
                    }
                    if (unit == 'O' && value != NULL) {
                        Py_IncRef(value);
                    }
                } else if (unit == 'i') {
                    int v = va_arg(ap, int);
                    if (trace) {
                        fprintf(stderr, "[py_buildvalue] tuple unit=i value=%d\\n", v);
                    }
                    value = PyLong_FromLong((long)v);
                } else if (unit == 'l' || unit == 'k' || unit == 'n') {
                    long v = va_arg(ap, long);
                    if (trace) {
                        fprintf(stderr, "[py_buildvalue] tuple unit=%c value=%ld\\n", unit, v);
                    }
                    value = PyLong_FromLong(v);
                } else if (unit == 'd' || unit == 'f') {
                    double v = va_arg(ap, double);
                    if (trace) {
                        fprintf(stderr, "[py_buildvalue] tuple unit=%c value=%f\\n", unit, v);
                    }
                    value = PyFloat_FromDouble(v);
                } else if (unit == 's') {
                    const char *text = va_arg(ap, const char *);
                    if (trace) {
                        fprintf(stderr, "[py_buildvalue] tuple unit=s value=%s\\n", text ? text : "<null>");
                    }
                    value = PyUnicode_FromString(text != NULL ? text : "");
                } else {
                    Py_DecRef(tuple);
                    va_end(ap);
                    pyrs_py_buildvalue_error("Py_BuildValue tuple format is not implemented");
                    return NULL;
                }
                if (value == NULL || PyTuple_SetItem(tuple, (long)out_index, value) != 0) {
                    Py_DecRef(tuple);
                    va_end(ap);
                    return NULL;
                }
                out_index++;
            }
            va_end(ap);
            return tuple;
        }
    }

    if (strcmp(spec, "{ON}") == 0) {
        PyObject *key = va_arg(ap, PyObject *);
        PyObject *value = va_arg(ap, PyObject *);
        if (trace) {
            fprintf(stderr, "[py_buildvalue] {ON} key=%p value=%p\\n", (void *)key, (void *)value);
        }
        PyObject *dict = PyDict_New();
        if (dict == NULL) {
            va_end(ap);
            return NULL;
        }
        if (key == NULL || value == NULL || PyDict_SetItem(dict, key, value) != 0) {
            if (trace) {
                fprintf(stderr, "[py_buildvalue] {ON} failed key/value insert\\n");
            }
            Py_DecRef(dict);
            va_end(ap);
            return NULL;
        }
        // N steals the value reference.
        Py_DecRef(value);
        va_end(ap);
        return dict;
    }

    if (strcmp(spec, "{s:O}") == 0 || strcmp(spec, "{s:N}") == 0) {
        const char *key_text = va_arg(ap, const char *);
        PyObject *value = va_arg(ap, PyObject *);
        PyObject *dict = PyDict_New();
        if (dict == NULL) {
            va_end(ap);
            return NULL;
        }
        PyObject *key = PyUnicode_FromString(key_text != NULL ? key_text : "");
        if (key == NULL || value == NULL || PyDict_SetItem(dict, key, value) != 0) {
            if (key != NULL) {
                Py_DecRef(key);
            }
            Py_DecRef(dict);
            va_end(ap);
            return NULL;
        }
        Py_DecRef(key);
        if (strcmp(spec, "{s:N}") == 0) {
            // N steals the value reference.
            Py_DecRef(value);
        }
        va_end(ap);
        return dict;
    }

    va_end(ap);
    pyrs_py_buildvalue_error("Py_BuildValue format is not implemented");
    return NULL;
}

static int pyrs_count_call_units(const char *format) {
    if (format == NULL) {
        return -1;
    }
    size_t len = strlen(format);
    size_t start = 0;
    size_t end = len;
    if (len >= 2 && format[0] == '(' && format[len - 1] == ')') {
        start = 1;
        end = len - 1;
    }
    int count = 0;
    for (size_t i = start; i < end; i++) {
        char unit = format[i];
        if (unit == ',' || unit == ' ' || unit == '\t') {
            continue;
        }
        if (unit == '(' || unit == ')' || unit == '{' || unit == '}' || unit == ':') {
            return -1;
        }
        count++;
    }
    return count;
}

static PyObject *pyrs_pack_call_units(const char *spec, va_list *ap) {
    int unit_count = pyrs_count_call_units(spec);
    if (unit_count < 0) {
        pyrs_py_buildvalue_error("call format is not implemented");
        return NULL;
    }
    if (unit_count == 0) {
        return PyTuple_New(0);
    }
    PyObject *args_tuple = PyTuple_New((long)unit_count);
    if (args_tuple == NULL) {
        return NULL;
    }
    size_t len = strlen(spec);
    size_t start = 0;
    size_t end = len;
    if (len >= 2 && spec[0] == '(' && spec[len - 1] == ')') {
        start = 1;
        end = len - 1;
    }
    int out_index = 0;
    for (size_t i = start; i < end; i++) {
        char unit = spec[i];
        if (unit == ',' || unit == ' ' || unit == '\t') {
            continue;
        }
        PyObject *value = NULL;
        if (unit == 'O' || unit == 'N') {
            value = va_arg(*ap, PyObject *);
            if (unit == 'O' && value != NULL) {
                Py_IncRef(value);
            }
        } else if (unit == 'i') {
            int v = va_arg(*ap, int);
            value = PyLong_FromLong((long)v);
        } else if (unit == 'l' || unit == 'k' || unit == 'n') {
            long v = va_arg(*ap, long);
            value = PyLong_FromLong(v);
        } else if (unit == 'd' || unit == 'f') {
            double v = va_arg(*ap, double);
            value = PyFloat_FromDouble(v);
        } else if (unit == 's') {
            const char *text = va_arg(*ap, const char *);
            value = PyUnicode_FromString(text != NULL ? text : "");
        } else {
            Py_DecRef(args_tuple);
            pyrs_py_buildvalue_error("call format is not implemented");
            return NULL;
        }
        if (value == NULL || PyTuple_SetItem(args_tuple, (long)out_index, value) != 0) {
            Py_DecRef(args_tuple);
            return NULL;
        }
        out_index++;
    }
    return args_tuple;
}

__attribute__((used)) PyObject *PyObject_CallFunction(PyObject *callable, const char *format, ...) {
    if (callable == NULL) {
        pyrs_py_buildvalue_error("PyObject_CallFunction received null callable");
        return NULL;
    }
    if (format == NULL || format[0] == '\0' || strcmp(format, "()") == 0) {
        return PyObject_CallObject(callable, NULL);
    }

    va_list ap;
    va_start(ap, format);
    PyObject *args_tuple = pyrs_pack_call_units(format, &ap);
    va_end(ap);
    if (args_tuple == NULL) {
        return NULL;
    }

    PyObject *result = PyObject_CallObject(callable, args_tuple);
    Py_DecRef(args_tuple);
    return result;
}

__attribute__((used)) PyObject *PyObject_CallFunctionObjArgs(PyObject *callable, ...) {
    if (callable == NULL) {
        pyrs_py_buildvalue_error("PyObject_CallFunctionObjArgs received null callable");
        return NULL;
    }

    va_list ap;
    va_start(ap, callable);
    va_list copy;
    va_copy(copy, ap);

    long argc = 0;
    while (1) {
        PyObject *value = va_arg(copy, PyObject *);
        if (value == NULL) {
            break;
        }
        argc++;
    }
    va_end(copy);

    if (argc == 0) {
        va_end(ap);
        return PyObject_CallObject(callable, NULL);
    }

    PyObject *args_tuple = PyTuple_New(argc);
    if (args_tuple == NULL) {
        va_end(ap);
        return NULL;
    }
    for (long i = 0; i < argc; i++) {
        PyObject *value = va_arg(ap, PyObject *);
        if (value == NULL) {
            Py_DecRef(args_tuple);
            va_end(ap);
            pyrs_py_buildvalue_error("PyObject_CallFunctionObjArgs missing trailing NULL terminator");
            return NULL;
        }
        Py_IncRef(value);
        if (PyTuple_SetItem(args_tuple, i, value) != 0) {
            Py_DecRef(args_tuple);
            va_end(ap);
            return NULL;
        }
    }
    // Consume trailing NULL terminator.
    (void)va_arg(ap, PyObject *);
    va_end(ap);

    PyObject *result = PyObject_CallObject(callable, args_tuple);
    Py_DecRef(args_tuple);
    return result;
}

__attribute__((used)) PyObject *PyObject_CallMethod(
    PyObject *object,
    const char *method,
    const char *format,
    ...
) {
    if (object == NULL || method == NULL) {
        pyrs_py_buildvalue_error("PyObject_CallMethod received null object/method");
        return NULL;
    }
    PyObject *callable = PyObject_GetAttrString(object, method);
    if (callable == NULL) {
        return NULL;
    }

    PyObject *result = NULL;
    if (format == NULL || format[0] == '\0' || strcmp(format, "()") == 0) {
        result = PyObject_CallObject(callable, NULL);
        Py_DecRef(callable);
        return result;
    }

    va_list ap;
    va_start(ap, format);
    PyObject *args_tuple = pyrs_pack_call_units(format, &ap);
    va_end(ap);
    if (args_tuple == NULL) {
        Py_DecRef(callable);
        return NULL;
    }
    result = PyObject_CallObject(callable, args_tuple);
    Py_DecRef(args_tuple);
    Py_DecRef(callable);
    return result;
}

typedef int (*pyrs_arg_converter)(PyObject *, void *);

static int pyrs_parse_consumed_outputs(char unit, const char **cursor, va_list *ap) {
    (void)cursor;
    switch (unit) {
        case 'O':
            if (**cursor == '!') {
                (*cursor)++;
                (void)va_arg(*ap, PyObject *);
                (void)va_arg(*ap, PyObject **);
                return 1;
            }
            if (**cursor == '&') {
                (*cursor)++;
                (void)va_arg(*ap, pyrs_arg_converter);
                (void)va_arg(*ap, void *);
                return 1;
            }
            (void)va_arg(*ap, PyObject **);
            return 1;
        case 'p':
        case 'i':
            (void)va_arg(*ap, int *);
            return 1;
        case 'l':
        case 'n':
            (void)va_arg(*ap, long *);
            return 1;
        case 'k':
            (void)va_arg(*ap, unsigned long *);
            return 1;
        case 'd':
        case 'f':
            (void)va_arg(*ap, double *);
            return 1;
        case 's':
        case 'y':
        case 'z':
            if (**cursor == '#') {
                (*cursor)++;
                (void)va_arg(*ap, const char **);
                (void)va_arg(*ap, long *);
                return 1;
            }
            (void)va_arg(*ap, const char **);
            return 1;
        default:
            return 0;
    }
}

static int pyrs_store_converted_arg(char unit, const char **cursor, va_list *ap, PyObject *value) {
    if (unit == 'O') {
        if (**cursor == '!') {
            (*cursor)++;
            PyObject *expected_type = va_arg(*ap, PyObject *);
            PyObject **out = va_arg(*ap, PyObject **);
            if (expected_type == NULL || out == NULL) {
                pyrs_py_buildvalue_error("PyArg parser received null O! output");
                return 0;
            }
            int is_instance = PyObject_IsInstance(value, expected_type);
            if (is_instance != 1) {
                pyrs_py_buildvalue_error("PyArg parser O! type check failed");
                return 0;
            }
            *out = value;
            return 1;
        }
        if (**cursor == '&') {
            (*cursor)++;
            pyrs_arg_converter converter = va_arg(*ap, pyrs_arg_converter);
            void *out = va_arg(*ap, void *);
            if (converter == NULL || out == NULL) {
                pyrs_py_buildvalue_error("PyArg parser received null O& converter/output");
                return 0;
            }
            return converter(value, out);
        }
        PyObject **out = va_arg(*ap, PyObject **);
        if (out == NULL) {
            pyrs_py_buildvalue_error("PyArg parser received null O output");
            return 0;
        }
        *out = value;
        return 1;
    }

    if (unit == 'p') {
        int *out = va_arg(*ap, int *);
        if (out == NULL) {
            pyrs_py_buildvalue_error("PyArg parser received null p output");
            return 0;
        }
        int truthy = PyObject_IsTrue(value);
        if (truthy < 0) {
            return 0;
        }
        *out = truthy ? 1 : 0;
        return 1;
    }

    if (unit == 'i') {
        int *out = va_arg(*ap, int *);
        if (out == NULL) {
            pyrs_py_buildvalue_error("PyArg parser received null i output");
            return 0;
        }
        long parsed = PyLong_AsLong(value);
        if (parsed == -1 && PyErr_Occurred() != NULL) {
            return 0;
        }
        *out = (int)parsed;
        return 1;
    }

    if (unit == 'l' || unit == 'n') {
        long *out = va_arg(*ap, long *);
        if (out == NULL) {
            pyrs_py_buildvalue_error("PyArg parser received null long output");
            return 0;
        }
        long parsed = PyLong_AsLong(value);
        if (parsed == -1 && PyErr_Occurred() != NULL) {
            return 0;
        }
        *out = parsed;
        return 1;
    }

    if (unit == 'k') {
        unsigned long *out = va_arg(*ap, unsigned long *);
        if (out == NULL) {
            pyrs_py_buildvalue_error("PyArg parser received null unsigned long output");
            return 0;
        }
        unsigned long parsed = PyLong_AsUnsignedLong(value);
        if (parsed == (unsigned long)-1 && PyErr_Occurred() != NULL) {
            return 0;
        }
        *out = parsed;
        return 1;
    }

    if (unit == 'd' || unit == 'f') {
        double *out = va_arg(*ap, double *);
        if (out == NULL) {
            pyrs_py_buildvalue_error("PyArg parser received null float output");
            return 0;
        }
        double parsed = PyFloat_AsDouble(value);
        if (parsed == -1.0 && PyErr_Occurred() != NULL) {
            return 0;
        }
        *out = parsed;
        return 1;
    }

    if (unit == 's' || unit == 'z') {
        const char **out_text = va_arg(*ap, const char **);
        long *out_len = NULL;
        int wants_len = 0;
        if (**cursor == '#') {
            (*cursor)++;
            out_len = va_arg(*ap, long *);
            wants_len = 1;
        }
        if (out_text == NULL || (wants_len && out_len == NULL)) {
            pyrs_py_buildvalue_error("PyArg parser received null string output");
            return 0;
        }
        if (unit == 'z' && value == &_Py_NoneStruct) {
            *out_text = NULL;
            if (wants_len) {
                *out_len = 0;
            }
            return 1;
        }
        const char *parsed = PyUnicode_AsUTF8(value);
        if (parsed == NULL) {
            return 0;
        }
        *out_text = parsed;
        if (wants_len) {
            *out_len = (long)strlen(parsed);
        }
        return 1;
    }

    if (unit == 'y') {
        const char **out_bytes = va_arg(*ap, const char **);
        long *out_len = NULL;
        int wants_len = 0;
        if (**cursor == '#') {
            (*cursor)++;
            out_len = va_arg(*ap, long *);
            wants_len = 1;
        }
        if (out_bytes == NULL || (wants_len && out_len == NULL)) {
            pyrs_py_buildvalue_error("PyArg parser received null bytes output");
            return 0;
        }
        char *buffer = NULL;
        long size = 0;
        if (PyBytes_AsStringAndSize(value, &buffer, &size) != 0) {
            return 0;
        }
        *out_bytes = (const char *)buffer;
        if (wants_len) {
            *out_len = size;
        }
        return 1;
    }

    return 0;
}

static int pyrs_parse_args_internal(
    PyObject *args,
    PyObject *kwargs,
    const char *format,
    const char *const *keywords,
    va_list ap
) {
    int trace = getenv("PYRS_TRACE_PYARG") != NULL;
    if (format == NULL) {
        pyrs_py_buildvalue_error("PyArg parser received null format");
        return 0;
    }
    long positional_count = args != NULL ? PyTuple_Size(args) : 0;
    if (positional_count < 0) {
        return 0;
    }
    long arg_index = 0;
    long max_positional = -1;
    int optional = 0;
    int keyword_only = 0;

    for (const char *cursor = format; *cursor != '\0'; cursor++) {
        char unit = *cursor;
        if (unit == ':' || unit == ';') {
            break;
        }
        if (unit == ' ' || unit == '\t' || unit == ',') {
            continue;
        }
        if (unit == '|') {
            optional = 1;
            continue;
        }
        if (unit == '$') {
            keyword_only = 1;
            if (max_positional < 0) {
                max_positional = arg_index;
            }
            continue;
        }

        const char *keyword_name = NULL;
        if (keywords != NULL) {
            keyword_name = keywords[arg_index];
        }

        PyObject *value = NULL;
        int has_value = 0;
        if (!keyword_only && arg_index < positional_count) {
            value = PyTuple_GetItem(args, arg_index);
            has_value = value != NULL;
        } else if (kwargs != NULL && keyword_name != NULL && keyword_name[0] != '\0') {
            value = PyDict_GetItemString(kwargs, keyword_name);
            has_value = value != NULL;
        }

        if (!has_value && !optional) {
            if (trace) {
                fprintf(stderr, "[pyarg] missing required arg index=%ld format=%s\n", arg_index, format);
            }
            pyrs_py_buildvalue_error("PyArg parser missing required argument");
            return 0;
        }

        if (!has_value) {
            const char *unit_cursor = cursor + 1;
            if (!pyrs_parse_consumed_outputs(unit, &unit_cursor, &ap)) {
                if (trace) {
                    fprintf(stderr, "[pyarg] unsupported optional unit=%c index=%ld format=%s\n", unit, arg_index, format);
                }
                pyrs_py_buildvalue_error("PyArg parser format unit is not implemented");
                return 0;
            }
            cursor = unit_cursor - 1;
            arg_index++;
            continue;
        }

        const char *unit_cursor = cursor + 1;
        if (!pyrs_store_converted_arg(unit, &unit_cursor, &ap, value)) {
            if (trace) {
                fprintf(stderr, "[pyarg] conversion failed unit=%c index=%ld format=%s\n", unit, arg_index, format);
            }
            if (PyErr_Occurred() == NULL) {
                pyrs_py_buildvalue_error("PyArg parser conversion failed");
            }
            return 0;
        }
        cursor = unit_cursor - 1;
        arg_index++;
    }

    if (max_positional < 0) {
        max_positional = arg_index;
    }
    if (positional_count > max_positional) {
        pyrs_py_buildvalue_error("PyArg parser received too many positional arguments");
        return 0;
    }

    return 1;
}

__attribute__((used)) int PyArg_ParseTuple(PyObject *args, const char *format, ...) {
    va_list ap;
    va_start(ap, format);
    int ok = pyrs_parse_args_internal(args, NULL, format, NULL, ap);
    va_end(ap);
    return ok;
}

__attribute__((used)) int PyArg_ParseTupleAndKeywords(
    PyObject *args,
    PyObject *kwargs,
    const char *format,
    const char *const *keywords,
    ...
) {
    va_list ap;
    va_start(ap, keywords);
    int ok = pyrs_parse_args_internal(args, kwargs, format, keywords, ap);
    va_end(ap);
    return ok;
}
