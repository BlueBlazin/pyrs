#ifndef PYRS_CPYTHON_COMPAT_H
#define PYRS_CPYTHON_COMPAT_H

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Minimal CPython-compatible module-init surface for pyrs extension bring-up.
 * Reference: CPython 3.14 module-init docs and PyModuleDef/PyMethodDef layout.
 */

typedef struct _object PyObject;

typedef struct PyMethodDef {
    const char *ml_name;
    PyObject *(*ml_meth)(PyObject *, PyObject *);
    int ml_flags;
    const char *ml_doc;
} PyMethodDef;

typedef struct PyModuleDef_Base {
    unsigned long _ob_refcnt;
    void *_ob_type;
    PyObject *(*m_init)(void);
    long long m_index;
    PyObject *m_copy;
} PyModuleDef_Base;

typedef struct PyModuleDef {
    PyModuleDef_Base m_base;
    const char *m_name;
    const char *m_doc;
    long long m_size;
    PyMethodDef *m_methods;
    void *m_slots;
    void *m_traverse;
    void *m_clear;
    void *m_free;
} PyModuleDef;

#define PyModuleDef_HEAD_INIT {0, 0, 0, 0, 0}

#ifndef PYTHON_API_VERSION
#define PYTHON_API_VERSION 1013
#endif

PyObject *PyModuleDef_Init(PyModuleDef *module);
PyObject *PyModule_Create2(PyModuleDef *module, int apiver);
#define PyModule_Create(module) PyModule_Create2((module), PYTHON_API_VERSION)

int PyModule_AddObjectRef(PyObject *module, const char *name, PyObject *value);
int PyModule_AddObject(PyObject *module, const char *name, PyObject *value);
int PyModule_AddIntConstant(PyObject *module, const char *name, long long value);
int PyModule_AddStringConstant(PyObject *module, const char *name, const char *value);

PyObject *PyLong_FromLong(long long value);
PyObject *PyLong_FromLongLong(long long value);
long long PyLong_AsLongLong(PyObject *object);
PyObject *PyBool_FromLong(long long value);
PyObject *PyFloat_FromDouble(double value);
double PyFloat_AsDouble(PyObject *object);
PyObject *PyUnicode_FromString(const char *value);
const char *PyUnicode_AsUTF8(PyObject *object);
PyObject *PyBytes_FromStringAndSize(const char *value, long long len);
int PyBytes_AsStringAndSize(PyObject *object, char **buffer, long long *len);
PyObject *PyTuple_New(long long size);
long long PyTuple_Size(PyObject *tuple);
PyObject *PyTuple_GetItem(PyObject *tuple, long long index);
int PyTuple_SetItem(PyObject *tuple, long long index, PyObject *item);
PyObject *Py_BuildValue(const char *format, ...);
int PyArg_ParseTuple(PyObject *args, const char *format, ...);
int PyArg_ParseTupleAndKeywords(
    PyObject *args,
    PyObject *kwargs,
    const char *format,
    const char *const *keywords,
    ...
);
PyObject *PyObject_GetAttrString(PyObject *object, const char *name);
PyObject *PyObject_CallMethod(PyObject *object, const char *name, const char *format, ...);
PyObject *PyObject_CallFunctionObjArgs(PyObject *callable, ...);
int PyObject_IsTrue(PyObject *object);
int PyObject_IsInstance(PyObject *object, PyObject *class_obj);

void PyErr_SetString(PyObject *exception, const char *message);
PyObject *PyErr_Occurred(void);
void PyErr_Clear(void);

void Py_IncRef(PyObject *object);
void Py_DecRef(PyObject *object);
void Py_XIncRef(PyObject *object);
void Py_XDecRef(PyObject *object);

#define Py_INCREF(op) Py_IncRef((PyObject *)(op))
#define Py_DECREF(op) Py_DecRef((PyObject *)(op))
#define Py_XINCREF(op) Py_XIncRef((PyObject *)(op))
#define Py_XDECREF(op) Py_XDecRef((PyObject *)(op))

extern PyObject *PyExc_Exception;
extern PyObject *PyExc_ImportError;
extern PyObject *PyExc_RuntimeError;
extern PyObject *PyExc_TypeError;
extern PyObject *PyExc_ValueError;

#define PyMODINIT_FUNC PyObject *

#ifdef __cplusplus
}
#endif

#endif /* PYRS_CPYTHON_COMPAT_H */
