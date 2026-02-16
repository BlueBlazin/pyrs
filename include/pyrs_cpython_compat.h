#ifndef PYRS_CPYTHON_COMPAT_H
#define PYRS_CPYTHON_COMPAT_H

#include <stddef.h>
#include <stdint.h>

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

typedef PyObject *(*PyCFunction)(PyObject *, PyObject *);

typedef enum {
    PYGEN_RETURN = 0,
    PYGEN_ERROR = -1,
    PYGEN_NEXT = 1
} PySendResult;

#ifndef METH_VARARGS
#define METH_VARARGS 0x0001
#define METH_KEYWORDS 0x0002
#define METH_NOARGS 0x0004
#define METH_O 0x0008
#define METH_FASTCALL 0x0080
#define METH_METHOD 0x0200
#endif

#ifndef Py_PRINT_RAW
#define Py_PRINT_RAW 0x0001
#endif

#ifndef Py_ASNATIVEBYTES_DEFAULTS
#define Py_ASNATIVEBYTES_DEFAULTS -1
#define Py_ASNATIVEBYTES_BIG_ENDIAN 0
#define Py_ASNATIVEBYTES_LITTLE_ENDIAN 1
#define Py_ASNATIVEBYTES_NATIVE_ENDIAN 3
#define Py_ASNATIVEBYTES_UNSIGNED_BUFFER 4
#define Py_ASNATIVEBYTES_REJECT_NEGATIVE 8
#define Py_ASNATIVEBYTES_ALLOW_INDEX 16
#endif

#ifndef PyBUF_SIMPLE
#define PyBUF_SIMPLE 0
#define PyBUF_WRITABLE 0x0001
#define PyBUF_FORMAT 0x0004
#define PyBUF_ND 0x0008
#define PyBUF_STRIDES (0x0010 | PyBUF_ND)
#define PyBUF_C_CONTIGUOUS (0x0020 | PyBUF_STRIDES)
#define PyBUF_F_CONTIGUOUS (0x0040 | PyBUF_STRIDES)
#define PyBUF_ANY_CONTIGUOUS (0x0080 | PyBUF_STRIDES)
#define PyBUF_INDIRECT (0x0100 | PyBUF_STRIDES)
#define PyBUF_CONTIG (PyBUF_ND | PyBUF_WRITABLE)
#define PyBUF_CONTIG_RO (PyBUF_ND)
#define PyBUF_STRIDED (PyBUF_STRIDES | PyBUF_WRITABLE)
#define PyBUF_STRIDED_RO (PyBUF_STRIDES)
#define PyBUF_RECORDS (PyBUF_STRIDES | PyBUF_WRITABLE | PyBUF_FORMAT)
#define PyBUF_RECORDS_RO (PyBUF_STRIDES | PyBUF_FORMAT)
#define PyBUF_FULL (PyBUF_INDIRECT | PyBUF_WRITABLE | PyBUF_FORMAT)
#define PyBUF_FULL_RO (PyBUF_INDIRECT | PyBUF_FORMAT)
#define PyBUF_READ 0x0100
#define PyBUF_WRITE 0x0200
#endif

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

typedef struct {
    void *buf;
    PyObject *obj;
    long long len;
    long long itemsize;
    int readonly;
    int ndim;
    char *format;
    long long *shape;
    long long *strides;
    long long *suboffsets;
    void *internal;
} Py_buffer;

typedef void (*PyCapsule_Destructor)(PyObject *);

#define PyModuleDef_HEAD_INIT {0, 0, 0, 0, 0}

#ifndef PYTHON_API_VERSION
#define PYTHON_API_VERSION 1013
#endif

PyObject *PyModuleDef_Init(PyModuleDef *module);
PyObject *PyModule_Create2(PyModuleDef *module, int apiver);
#define PyModule_Create(module) PyModule_Create2((module), PYTHON_API_VERSION)

PyObject *PyModule_NewObject(PyObject *name);
PyObject *PyModule_New(const char *name);
PyObject *PyModule_GetNameObject(PyObject *module);
const char *PyModule_GetName(PyObject *module);
PyObject *PyModule_GetFilenameObject(PyObject *module);
const char *PyModule_GetFilename(PyObject *module);
int PyModule_SetDocString(PyObject *module, const char *doc);
int PyModule_Add(PyObject *module, const char *name, PyObject *value);
int PyModule_AddObjectRef(PyObject *module, const char *name, PyObject *value);
int PyModule_AddObject(PyObject *module, const char *name, PyObject *value);
int PyModule_AddIntConstant(PyObject *module, const char *name, long long value);
int PyModule_AddStringConstant(PyObject *module, const char *name, const char *value);

PyObject *PyImport_ImportModule(const char *name);
PyObject *PyImport_Import(PyObject *name);
PyObject *PyImport_GetModuleDict(void);
PyObject *PyImport_AddModuleRef(const char *name);
PyObject *PyImport_AddModuleObject(PyObject *name);
PyObject *PyImport_AddModule(const char *name);
PyObject *PyImport_GetModule(PyObject *name);
PyObject *PyImport_ImportModuleNoBlock(const char *name);
PyObject *PyImport_ImportModuleLevelObject(
    PyObject *name,
    PyObject *globals,
    PyObject *locals,
    PyObject *fromlist,
    int level
);
PyObject *PyImport_ImportModuleLevel(
    const char *name,
    PyObject *globals,
    PyObject *locals,
    PyObject *fromlist,
    int level
);
PyObject *PyImport_ReloadModule(PyObject *module);

PyObject *PyLong_FromLong(long long value);
PyObject *PyLong_FromLongLong(long long value);
PyObject *PyLong_FromSize_t(size_t value);
PyObject *PyLong_FromInt32(int32_t value);
PyObject *PyLong_FromUInt32(uint32_t value);
PyObject *PyLong_FromInt64(int64_t value);
PyObject *PyLong_FromUInt64(uint64_t value);
PyObject *PyLong_FromString(const char *value, char **pend, int base);
PyObject *PyLong_GetInfo(void);
long long PyLong_AsLong(PyObject *object);
long long PyLong_AsLongLong(PyObject *object);
int PyLong_AsInt(PyObject *object);
int PyLong_AsInt32(PyObject *object, int32_t *value);
int PyLong_AsUInt32(PyObject *object, uint32_t *value);
int PyLong_AsInt64(PyObject *object, int64_t *value);
int PyLong_AsUInt64(PyObject *object, uint64_t *value);
size_t PyLong_AsSize_t(PyObject *object);
double PyLong_AsDouble(PyObject *object);
unsigned long long PyLong_AsUnsignedLongMask(PyObject *object);
unsigned long long PyLong_AsUnsignedLongLongMask(PyObject *object);
long long PyLong_AsNativeBytes(PyObject *object, void *buffer, long long n_bytes, int flags);
PyObject *PyLong_FromNativeBytes(const void *buffer, size_t n_bytes, int flags);
PyObject *PyLong_FromUnsignedNativeBytes(const void *buffer, size_t n_bytes, int flags);
PyObject *PyBool_FromLong(long long value);
PyObject *PyFloat_FromDouble(double value);
double PyFloat_AsDouble(PyObject *object);
double PyFloat_GetMax(void);
double PyFloat_GetMin(void);
PyObject *PyFloat_GetInfo(void);
PyObject *PyUnicode_FromString(const char *value);
const char *PyUnicode_AsUTF8(PyObject *object);
PyObject *PyBytes_FromString(const char *value);
PyObject *PyBytes_FromStringAndSize(const char *value, long long len);
PyObject *PyBytes_FromObject(PyObject *object);
long long PyBytes_Size(PyObject *object);
char *PyBytes_AsString(PyObject *object);
int PyBytes_AsStringAndSize(PyObject *object, char **buffer, long long *len);
void PyBytes_Concat(PyObject **bytes, PyObject *newpart);
void PyBytes_ConcatAndDel(PyObject **bytes, PyObject *newpart);
PyObject *PyByteArray_FromObject(PyObject *object);
PyObject *PyByteArray_FromStringAndSize(const char *value, long long len);
long long PyByteArray_Size(PyObject *object);
char *PyByteArray_AsString(PyObject *object);
int PyByteArray_Resize(PyObject *object, long long requested_size);
PyObject *PyByteArray_Concat(PyObject *left, PyObject *right);
PyObject *PyTuple_New(long long size);
PyObject *PyTuple_Pack(long long size, ...);
long long PyTuple_Size(PyObject *tuple);
PyObject *PyTuple_GetItem(PyObject *tuple, long long index);
int PyTuple_SetItem(PyObject *tuple, long long index, PyObject *item);
PyObject *PyList_New(long long size);
long long PyList_Size(PyObject *list);
PyObject *PyList_GetItem(PyObject *list, long long index);
int PyList_SetItem(PyObject *list, long long index, PyObject *item);
int PyList_Insert(PyObject *list, long long index, PyObject *item);
PyObject *PyList_GetSlice(PyObject *list, long long low, long long high);
int PyList_SetSlice(PyObject *list, long long low, long long high, PyObject *itemlist);
int PyList_Sort(PyObject *list);
int PyList_Reverse(PyObject *list);
int PySequence_Check(PyObject *object);
long long PySequence_Size(PyObject *object);
long long PySequence_Length(PyObject *object);
PyObject *PySequence_GetItem(PyObject *object, long long index);
PyObject *PySequence_GetSlice(PyObject *object, long long low, long long high);
int PySequence_SetItem(PyObject *object, long long index, PyObject *value);
int PySequence_DelItem(PyObject *object, long long index);
int PySequence_SetSlice(PyObject *object, long long low, long long high, PyObject *value);
int PySequence_DelSlice(PyObject *object, long long low, long long high);
PyObject *PySequence_Tuple(PyObject *object);
PyObject *PySequence_List(PyObject *object);
PyObject *PySequence_Fast(PyObject *object, const char *msg);
PyObject *PySequence_Concat(PyObject *left, PyObject *right);
PyObject *PySequence_InPlaceConcat(PyObject *left, PyObject *right);
PyObject *PySequence_Repeat(PyObject *object, long long count);
PyObject *PySequence_InPlaceRepeat(PyObject *object, long long count);
long long PySequence_Count(PyObject *object, PyObject *value);
long long PySequence_Index(PyObject *object, PyObject *value);
int PySequence_Contains(PyObject *object, PyObject *value);
int PySequence_In(PyObject *object, PyObject *value);
int PyMapping_Check(PyObject *object);
long long PyMapping_Size(PyObject *object);
long long PyMapping_Length(PyObject *object);
PyObject *PyMapping_GetItemString(PyObject *mapping, const char *key);
PyObject *PyMapping_Keys(PyObject *object);
PyObject *PyMapping_Items(PyObject *object);
PyObject *PyMapping_Values(PyObject *object);
int PyMapping_GetOptionalItem(PyObject *object, PyObject *key, PyObject **result);
int PyMapping_GetOptionalItemString(PyObject *object, const char *key, PyObject **result);
int PyMapping_SetItemString(PyObject *object, const char *key, PyObject *value);
int PyMapping_HasKeyWithError(PyObject *object, PyObject *key);
int PyMapping_HasKeyStringWithError(PyObject *object, const char *key);
int PyMapping_HasKey(PyObject *object, PyObject *key);
int PyMapping_HasKeyString(PyObject *object, const char *key);
PyObject *PySlice_New(PyObject *start, PyObject *stop, PyObject *step);
int PySlice_Unpack(
    PyObject *slice,
    long long *start,
    long long *stop,
    long long *step
);
long long PySlice_AdjustIndices(long long length, long long *start, long long *stop, long long step);
int PySlice_GetIndices(
    PyObject *slice,
    long long length,
    long long *start,
    long long *stop,
    long long *step
);
int PySlice_GetIndicesEx(
    PyObject *slice,
    long long length,
    long long *start,
    long long *stop,
    long long *step,
    long long *slice_length
);
PyObject *PySet_New(PyObject *iterable);
PyObject *PyFrozenSet_New(PyObject *iterable);
long long PySet_Size(PyObject *anyset);
int PySet_Contains(PyObject *anyset, PyObject *key);
int PySet_Add(PyObject *set, PyObject *key);
int PySet_Discard(PyObject *set, PyObject *key);
int PySet_Clear(PyObject *set);
PyObject *PySet_Pop(PyObject *set);
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
PyObject *PyObject_GetAttr(PyObject *object, PyObject *name);
PyObject *PyObject_CallFunction(PyObject *callable, const char *format, ...);
PyObject *PyObject_CallMethod(PyObject *object, const char *name, const char *format, ...);
PyObject *PyObject_CallFunctionObjArgs(PyObject *callable, ...);
PyObject *PyObject_CallMethodObjArgs(PyObject *object, PyObject *name, ...);
PyObject *PyObject_CallNoArgs(PyObject *callable);
int PyAIter_Check(PyObject *object);
PyObject *PyObject_GetAIter(PyObject *object);
PyObject *PySeqIter_New(PyObject *object);
PyObject *PyObject_GetItem(PyObject *object, PyObject *key);
int PyObject_SetItem(PyObject *object, PyObject *key, PyObject *value);
int PyObject_DelItem(PyObject *object, PyObject *key);
int PyObject_DelItemString(PyObject *object, const char *key);
int PyObject_SetAttr(PyObject *object, PyObject *name, PyObject *value);
int PyObject_DelAttr(PyObject *object, PyObject *name);
int PyObject_DelAttrString(PyObject *object, const char *name);
int PyObject_HasAttr(PyObject *object, PyObject *name);
int PyObject_HasAttrWithError(PyObject *object, PyObject *name);
int PyObject_HasAttrStringWithError(PyObject *object, const char *name);
int PyObject_GetOptionalAttrString(PyObject *object, const char *name, PyObject **result);
PyObject *PyObject_Repr(PyObject *object);
PyObject *PyObject_ASCII(PyObject *object);
PyObject *PyObject_Dir(PyObject *object);
int PyObject_IsTrue(PyObject *object);
int PyObject_IsInstance(PyObject *object, PyObject *class_obj);
void *PyObject_GetTypeData(PyObject *object, PyObject *cls);
long long PyObject_HashNotImplemented(PyObject *object);
long long PyObject_Size(PyObject *object);
long long PyObject_Length(PyObject *object);
long long PyObject_LengthHint(PyObject *object, long long default_value);
void *PyObject_Malloc(size_t size);
void *PyObject_Calloc(size_t nelem, size_t elsize);
void *PyObject_Realloc(void *ptr, size_t new_size);
void PyObject_Free(void *ptr);
int PyIter_Check(PyObject *object);
PyObject *PyIter_Next(PyObject *iter);
int PyIter_NextItem(PyObject *iter, PyObject **item);
PySendResult PyIter_Send(PyObject *iter, PyObject *arg, PyObject **result);
int PyObject_CheckBuffer(PyObject *object);
int PyObject_CheckReadBuffer(PyObject *object);
int PyObject_AsReadBuffer(PyObject *object, const void **buffer, long long *buffer_len);
int PyObject_AsWriteBuffer(PyObject *object, void **buffer, long long *buffer_len);
int PyObject_AsCharBuffer(PyObject *object, const char **buffer, long long *buffer_len);
int PyObject_CopyData(PyObject *dest, PyObject *src);
PyObject *PyMemoryView_FromObject(PyObject *object);
PyObject *PyMemoryView_FromMemory(char *mem, long long size, int flags);
PyObject *PyMemoryView_FromBuffer(const Py_buffer *view);
PyObject *PyMemoryView_GetContiguous(PyObject *object, int buffertype, char order);
int PyObject_GetBuffer(PyObject *object, Py_buffer *view, int flags);
int PyBuffer_IsContiguous(const Py_buffer *view, char order);
void *PyBuffer_GetPointer(const Py_buffer *view, const long long *indices);
long long PyBuffer_SizeFromFormat(const char *format);
int PyBuffer_FromContiguous(const Py_buffer *view, const void *buf, long long len, char order);
int PyBuffer_ToContiguous(void *buf, const Py_buffer *src, long long len, char order);
void PyBuffer_FillContiguousStrides(
    int ndim,
    const long long *shape,
    long long *strides,
    int itemsize,
    char fort
);
int PyBuffer_FillInfo(
    Py_buffer *view,
    PyObject *object,
    void *buf,
    long long len,
    int readonly,
    int flags
);
void PyBuffer_Release(Py_buffer *view);

PyObject *PyDict_Keys(PyObject *dict);
PyObject *PyDict_Values(PyObject *dict);
PyObject *PyDict_Items(PyObject *dict);
void PyDict_Clear(PyObject *dict);
int PyDict_Update(PyObject *dict, PyObject *other);
int PyDict_Merge(PyObject *dict, PyObject *other, int override_existing);
int PyDict_MergeFromSeq2(PyObject *dict, PyObject *seq2, int override_existing);

PyObject *PyCapsule_New(void *pointer, const char *name, PyCapsule_Destructor destructor);
void *PyCapsule_GetPointer(PyObject *capsule, const char *name);
const char *PyCapsule_GetName(PyObject *capsule);
PyCapsule_Destructor PyCapsule_GetDestructor(PyObject *capsule);
int PyCapsule_SetPointer(PyObject *capsule, void *pointer);
int PyCapsule_SetName(PyObject *capsule, const char *name);
int PyCapsule_SetDestructor(PyObject *capsule, PyCapsule_Destructor destructor);
void *PyCapsule_GetContext(PyObject *capsule);
int PyCapsule_SetContext(PyObject *capsule, void *context);
int PyCapsule_IsValid(PyObject *capsule, const char *name);
void *PyCapsule_Import(const char *name, int no_block);

void PyErr_SetString(PyObject *exception, const char *message);
PyObject *PyErr_Occurred(void);
PyObject *PyErr_GetRaisedException(void);
void PyErr_SetRaisedException(PyObject *exc);
PyObject *PyErr_GetHandledException(void);
void PyErr_SetHandledException(PyObject *exc);
void PyErr_GetExcInfo(PyObject **ptype, PyObject **pvalue, PyObject **ptraceback);
void PyErr_SetExcInfo(PyObject *type, PyObject *value, PyObject *traceback);
int PyErr_BadArgument(void);
void PyErr_BadInternalCall(void);
void PyErr_PrintEx(int set_sys_last_vars);
void PyErr_Display(PyObject *unused, PyObject *value, PyObject *tb);
void PyErr_DisplayException(PyObject *exc);
void PyErr_Clear(void);
PyObject *PyException_GetTraceback(PyObject *exception);
PyObject *PyException_GetCause(PyObject *exception);
PyObject *PyException_GetContext(PyObject *exception);
PyObject *PyException_GetArgs(PyObject *exception);
void PyException_SetArgs(PyObject *exception, PyObject *args);
void PyException_SetCause(PyObject *exception, PyObject *cause);
void PyException_SetContext(PyObject *exception, PyObject *context);
void PyException_SetTraceback(PyObject *exception, PyObject *traceback);

long long PyGC_Collect(void);
int PyGC_Enable(void);
int PyGC_Disable(void);
int PyGC_IsEnabled(void);
int PyObject_GC_IsTracked(PyObject *object);
int PyObject_GC_IsFinalized(PyObject *object);

PyObject *PyFile_GetLine(PyObject *f, int n);
int PyFile_WriteObject(PyObject *v, PyObject *f, int flags);
int PyFile_WriteString(const char *s, PyObject *f);

PyObject *PyCFunction_Call(PyObject *callable, PyObject *args, PyObject *kwargs);
PyObject *PyCFunction_New(PyMethodDef *ml, PyObject *self);
PyObject *PyCFunction_NewEx(PyMethodDef *ml, PyObject *self, PyObject *module);
PyObject *PyCMethod_New(PyMethodDef *ml, PyObject *self, PyObject *module, void *cls);
PyCFunction PyCFunction_GetFunction(PyObject *op);
PyObject *PyCFunction_GetSelf(PyObject *op);
int PyCFunction_GetFlags(PyObject *op);

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
extern PyObject *PyExc_EOFError;
extern PyObject *PyExc_AttributeError;
extern PyObject *PyExc_BufferError;
extern void *PyByteArray_Type;

#define PyMODINIT_FUNC PyObject *

#ifdef __cplusplus
}
#endif

#endif /* PYRS_CPYTHON_COMPAT_H */
