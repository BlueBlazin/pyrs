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
extern PyObject *PyUnicode_FromString(const char *s);
extern PyObject *PyLong_FromLong(long value);
extern PyObject *PyFloat_FromDouble(double value);
extern void Py_IncRef(PyObject *obj);
extern void Py_DecRef(PyObject *obj);
extern void PyErr_SetString(PyObject *exc, const char *msg);
extern PyObject *PyExc_RuntimeError;

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
