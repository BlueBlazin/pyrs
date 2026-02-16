#ifndef PYRS_CPYTHON_COMPAT_H
#define PYRS_CPYTHON_COMPAT_H

#include <stddef.h>
#include <stdarg.h>
#include <stdint.h>
#include <wchar.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Minimal CPython-compatible module-init surface for pyrs extension bring-up.
 * Reference: CPython 3.14 module-init docs and PyModuleDef/PyMethodDef layout.
 */

typedef struct _object PyObject;
typedef struct _typeobject PyTypeObject;

typedef struct PyMethodDef {
    const char *ml_name;
    PyObject *(*ml_meth)(PyObject *, PyObject *);
    int ml_flags;
    const char *ml_doc;
} PyMethodDef;

typedef struct PyGetSetDef {
    const char *name;
    void *get;
    void *set;
    const char *doc;
    void *closure;
} PyGetSetDef;

typedef struct PyMemberDef {
    const char *name;
    int type;
    long long offset;
    int flags;
    const char *doc;
} PyMemberDef;

typedef struct PyStructSequence_Field {
    const char *name;
    const char *doc;
} PyStructSequence_Field;

typedef struct PyStructSequence_Desc {
    const char *name;
    const char *doc;
    PyStructSequence_Field *fields;
    int n_in_sequence;
} PyStructSequence_Desc;

typedef PyObject *(*PyCFunction)(PyObject *, PyObject *);
typedef void (*PyOS_sighandler_t)(int);

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

#ifndef Py_DTSF_SIGN
#define Py_DTSF_SIGN 0x01
#define Py_DTSF_ADD_DOT_0 0x02
#define Py_DTSF_ALT 0x04
#define Py_DTSF_NO_NEG_0 0x08
#endif

#ifndef Py_DTST_FINITE
#define Py_DTST_FINITE 0
#define Py_DTST_INFINITE 1
#define Py_DTST_NAN 2
#endif

#ifndef Py_RELATIVE_OFFSET
#define Py_RELATIVE_OFFSET 8
#endif

#ifndef Py_READONLY
#define Py_READONLY 1
#endif

#ifndef Py_T_SHORT
#define Py_T_SHORT 0
#define Py_T_INT 1
#define Py_T_LONG 2
#define Py_T_FLOAT 3
#define Py_T_DOUBLE 4
#define Py_T_STRING 5
#define Py_T_OBJECT 6
#define _Py_T_OBJECT 6
#define Py_T_CHAR 7
#define Py_T_BYTE 8
#define Py_T_UBYTE 9
#define Py_T_USHORT 10
#define Py_T_UINT 11
#define Py_T_ULONG 12
#define Py_T_STRING_INPLACE 13
#define Py_T_BOOL 14
#define Py_T_OBJECT_EX 16
#define Py_T_LONGLONG 17
#define Py_T_ULONGLONG 18
#define Py_T_PYSSIZET 19
#define _Py_T_NONE 20
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
PyObject *PyModule_FromDefAndSpec2(PyModuleDef *module, PyObject *spec, int module_api_version);
int PyModule_ExecDef(PyObject *module, PyModuleDef *def);
PyModuleDef *PyModule_GetDef(PyObject *module);
void *PyModule_GetState(PyObject *module);

PyObject *PyModule_NewObject(PyObject *name);
PyObject *PyModule_New(const char *name);
PyObject *PyModule_GetNameObject(PyObject *module);
const char *PyModule_GetName(PyObject *module);
PyObject *PyModule_GetFilenameObject(PyObject *module);
const char *PyModule_GetFilename(PyObject *module);
int PyModule_SetDocString(PyObject *module, const char *doc);
int PyModule_Add(PyObject *module, const char *name, PyObject *value);
int PyModule_AddFunctions(PyObject *module, PyMethodDef *functions);
int PyModule_AddType(PyObject *module, PyTypeObject *type);
int PyModule_AddObjectRef(PyObject *module, const char *name, PyObject *value);
int PyModule_AddObject(PyObject *module, const char *name, PyObject *value);
int PyModule_AddIntConstant(PyObject *module, const char *name, long long value);
int PyModule_AddStringConstant(PyObject *module, const char *name, const char *value);

PyObject *PySys_GetObject(const char *name);
int PySys_SetObject(const char *name, PyObject *value);
PyObject *PySys_GetXOptions(void);
void PySys_AddXOption(const wchar_t *option);
int PySys_HasWarnOptions(void);
void PySys_ResetWarnOptions(void);
void PySys_AddWarnOption(const wchar_t *option);
void PySys_AddWarnOptionUnicode(const wchar_t *option);
void PySys_WriteStdout(const char *format, ...);
void PySys_WriteStderr(const char *format, ...);
void PySys_FormatStdout(const char *format, ...);
void PySys_FormatStderr(const char *format, ...);
int PySys_Audit(const char *event, const char *format, ...);
int PySys_AuditTuple(const char *event, PyObject *args);
void PySys_SetArgv(int argc, wchar_t **argv);
void PySys_SetArgvEx(int argc, wchar_t **argv, int updatepath);
void PySys_SetPath(const wchar_t *path);

int PyCodec_Register(PyObject *search_function);
int PyCodec_Unregister(PyObject *search_function);
int PyCodec_KnownEncoding(const char *encoding);
PyObject *PyCodec_Encode(PyObject *object, const char *encoding, const char *errors);
PyObject *PyCodec_Decode(PyObject *object, const char *encoding, const char *errors);
PyObject *PyCodec_Encoder(const char *encoding);
PyObject *PyCodec_Decoder(const char *encoding);
PyObject *PyCodec_IncrementalEncoder(const char *encoding, const char *errors);
PyObject *PyCodec_IncrementalDecoder(const char *encoding, const char *errors);
PyObject *PyCodec_StreamReader(const char *encoding, PyObject *stream, const char *errors);
PyObject *PyCodec_StreamWriter(const char *encoding, PyObject *stream, const char *errors);
int PyCodec_RegisterError(const char *name, PyObject *error);
PyObject *PyCodec_LookupError(const char *name);
PyObject *PyCodec_StrictErrors(PyObject *exc);
PyObject *PyCodec_IgnoreErrors(PyObject *exc);
PyObject *PyCodec_ReplaceErrors(PyObject *exc);
PyObject *PyCodec_XMLCharRefReplaceErrors(PyObject *exc);
PyObject *PyCodec_BackslashReplaceErrors(PyObject *exc);
PyObject *PyCodec_NameReplaceErrors(PyObject *exc);

PyObject *PyImport_ImportModule(const char *name);
long PyImport_GetMagicNumber(void);
const char *PyImport_GetMagicTag(void);
PyObject *PyImport_Import(PyObject *name);
PyObject *PyImport_GetModuleDict(void);
PyObject *PyImport_AddModuleRef(const char *name);
PyObject *PyImport_AddModuleObject(PyObject *name);
PyObject *PyImport_AddModule(const char *name);
int PyImport_AppendInittab(const char *name, PyObject *(*initfunc)(void));
int PyImport_ImportFrozenModule(const char *name);
int PyImport_ImportFrozenModuleObject(PyObject *name);
PyObject *PyImport_ExecCodeModule(const char *name, PyObject *co);
PyObject *PyImport_ExecCodeModuleEx(const char *name, PyObject *co, const char *pathname);
PyObject *PyImport_ExecCodeModuleObject(
    PyObject *name,
    PyObject *co,
    PyObject *pathname,
    PyObject *cpathname
);
PyObject *PyImport_ExecCodeModuleWithPathnames(
    const char *name,
    PyObject *co,
    const char *pathname,
    const char *cpathname
);
PyObject *PyImport_GetModule(PyObject *name);
PyObject *PyImport_ImportModuleNoBlock(const char *name);
PyObject *PyImport_GetImporter(PyObject *path);
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
PyObject *PyBytes_FromFormat(const char *format, ...);
PyObject *PyBytes_FromFormatV(const char *format, va_list vargs);
PyObject *PyMarshal_ReadObjectFromString(const char *data, long long len);
PyObject *PyMarshal_WriteObjectToString(PyObject *object, int version);
PyObject *PyBytes_FromObject(PyObject *object);
long long PyBytes_Size(PyObject *object);
char *PyBytes_AsString(PyObject *object);
int PyBytes_AsStringAndSize(PyObject *object, char **buffer, long long *len);
PyObject *PyBytes_Repr(PyObject *object, int smartquotes);
PyObject *PyBytes_DecodeEscape(
    const char *s,
    long long len,
    const char *errors,
    long long unicode,
    const char *recode_encoding
);
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
int PyArg_Parse(PyObject *args, const char *format, ...);
int PyArg_VaParse(PyObject *args, const char *format, va_list va);
int PyArg_ValidateKeywordArguments(PyObject *kwargs);
int PyArg_ParseTuple(PyObject *args, const char *format, ...);
int PyArg_ParseTupleAndKeywords(
    PyObject *args,
    PyObject *kwargs,
    const char *format,
    const char *const *keywords,
    ...
);
int PyNumber_Check(PyObject *object);
PyObject *PyNumber_Absolute(PyObject *object);
PyObject *PyNumber_Add(PyObject *left, PyObject *right);
PyObject *PyNumber_Subtract(PyObject *left, PyObject *right);
PyObject *PyNumber_Multiply(PyObject *left, PyObject *right);
PyObject *PyNumber_MatrixMultiply(PyObject *left, PyObject *right);
PyObject *PyNumber_TrueDivide(PyObject *left, PyObject *right);
PyObject *PyNumber_FloorDivide(PyObject *left, PyObject *right);
PyObject *PyNumber_Remainder(PyObject *left, PyObject *right);
PyObject *PyNumber_Divmod(PyObject *left, PyObject *right);
PyObject *PyNumber_Power(PyObject *left, PyObject *right, PyObject *modulo);
PyObject *PyNumber_Lshift(PyObject *left, PyObject *right);
PyObject *PyNumber_Rshift(PyObject *left, PyObject *right);
PyObject *PyNumber_And(PyObject *left, PyObject *right);
PyObject *PyNumber_Or(PyObject *left, PyObject *right);
PyObject *PyNumber_Xor(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceAdd(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceSubtract(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceMultiply(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceMatrixMultiply(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceFloorDivide(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceTrueDivide(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceRemainder(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlacePower(PyObject *left, PyObject *right, PyObject *modulo);
PyObject *PyNumber_InPlaceLshift(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceRshift(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceAnd(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceOr(PyObject *left, PyObject *right);
PyObject *PyNumber_InPlaceXor(PyObject *left, PyObject *right);
PyObject *PyNumber_Negative(PyObject *object);
PyObject *PyNumber_Positive(PyObject *object);
PyObject *PyNumber_Invert(PyObject *object);
PyObject *PyNumber_Long(PyObject *object);
PyObject *PyNumber_Float(PyObject *object);
PyObject *PyNumber_Index(PyObject *object);
long long PyNumber_AsSsize_t(PyObject *object, PyObject *exc);
PyObject *PyNumber_ToBase(PyObject *object, int base);
PyObject *PyObject_GetAttrString(PyObject *object, const char *name);
PyObject *PyObject_GetAttr(PyObject *object, PyObject *name);
PyObject *PyObject_Type(PyObject *object);
PyObject *PyObject_CallFunction(PyObject *callable, const char *format, ...);
PyObject *PyObject_CallMethod(PyObject *object, const char *name, const char *format, ...);
PyObject *PyObject_CallObject(PyObject *callable, PyObject *args);
PyObject *PyObject_CallFunctionObjArgs(PyObject *callable, ...);
PyObject *PyObject_CallMethodObjArgs(PyObject *object, PyObject *name, ...);
PyObject *PyObject_CallNoArgs(PyObject *callable);
PyObject *PyEval_CallObjectWithKeywords(PyObject *callable, PyObject *args, PyObject *kwargs);
PyObject *PyEval_CallFunction(PyObject *callable, const char *format, ...);
PyObject *PyEval_CallMethod(PyObject *object, const char *name, const char *format, ...);
PyObject *PyEval_EvalCode(PyObject *code, PyObject *globals, PyObject *locals);
PyObject *PyEval_EvalCodeEx(
    PyObject *code,
    PyObject *globals,
    PyObject *locals,
    PyObject *const *args,
    int argcount,
    PyObject *const *kws,
    int kwcount,
    PyObject *const *defs,
    int defcount,
    PyObject *kwdefs,
    PyObject *closure
);
PyObject *PyEval_EvalFrame(void *frame);
PyObject *PyEval_EvalFrameEx(void *frame, int throwflag);
void *PyEval_GetFrame(void);
PyObject *PyEval_GetGlobals(void);
PyObject *PyEval_GetLocals(void);
PyObject *PyEval_GetFrameBuiltins(void);
PyObject *PyEval_GetFrameGlobals(void);
PyObject *PyEval_GetFrameLocals(void);
const char *PyEval_GetFuncName(PyObject *func);
const char *PyEval_GetFuncDesc(PyObject *func);
PyObject *PyFrame_GetCode(PyObject *frame);
int PyFrame_GetLineNumber(PyObject *frame);
int PyGILState_Ensure(void);
void *PyGILState_GetThisThreadState(void);
void PyGILState_Release(int state);
void *PyThreadState_Get(void);
void *PyThreadState_New(void *interp);
void *PyThreadState_Swap(void *state);
void PyThreadState_Clear(void *state);
void PyThreadState_Delete(void *state);
void PyThreadState_DeleteCurrent(void);
int PyThreadState_SetAsyncExc(unsigned long long id, PyObject *exc);
PyObject *PyThreadState_GetFrame(void *state);
void *PyThreadState_GetInterpreter(void *state);
unsigned long long PyThreadState_GetID(void *state);
PyObject *PyThreadState_GetDict(void);
void *PyInterpreterState_Get(void);
void *PyInterpreterState_New(void);
void PyInterpreterState_Clear(void *interp);
void PyInterpreterState_Delete(void *interp);
long long PyInterpreterState_GetID(void *interp);
PyObject *PyInterpreterState_GetDict(void *interp);
int PyState_AddModule(PyObject *module, PyModuleDef *def);
int PyState_RemoveModule(PyModuleDef *def);
PyObject *PyState_FindModule(PyModuleDef *def);
void PyEval_AcquireLock(void);
void PyEval_ReleaseLock(void);
void PyEval_AcquireThread(void *state);
void PyEval_ReleaseThread(void *state);
void PyEval_InitThreads(void);
int PyEval_ThreadsInitialized(void);
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
PyObject *PyCallIter_New(PyObject *callable, PyObject *sentinel);
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

PyObject *PyDict_New(void);
int PyDict_SetItem(PyObject *dict, PyObject *key, PyObject *value);
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
PyObject *PyErr_NewException(const char *name, PyObject *base, PyObject *dict);
PyObject *PyErr_NewExceptionWithDoc(
    const char *name,
    const char *doc,
    PyObject *base,
    PyObject *dict
);
const char *PyExceptionClass_Name(PyObject *exception_class);
PyObject *PyErr_Occurred(void);
PyObject *PyErr_GetRaisedException(void);
void PyErr_SetRaisedException(PyObject *exc);
PyObject *PyErr_GetHandledException(void);
void PyErr_SetHandledException(PyObject *exc);
void PyErr_GetExcInfo(PyObject **ptype, PyObject **pvalue, PyObject **ptraceback);
void PyErr_SetExcInfo(PyObject *type, PyObject *value, PyObject *traceback);
PyObject *PyErr_SetFromErrno(PyObject *exc);
PyObject *PyErr_SetFromErrnoWithFilename(PyObject *exc, const char *filename);
PyObject *PyErr_SetFromErrnoWithFilenameObject(PyObject *exc, PyObject *filename);
PyObject *PyErr_SetFromErrnoWithFilenameObjects(
    PyObject *exc,
    PyObject *filename1,
    PyObject *filename2
);
PyObject *PyErr_SetExcFromWindowsErr(PyObject *exc, int ierr);
PyObject *PyErr_SetExcFromWindowsErrWithFilename(
    PyObject *exc,
    int ierr,
    const char *filename
);
PyObject *PyErr_SetExcFromWindowsErrWithFilenameObject(
    PyObject *exc,
    int ierr,
    PyObject *filename
);
PyObject *PyErr_SetExcFromWindowsErrWithFilenameObjects(
    PyObject *exc,
    int ierr,
    PyObject *filename1,
    PyObject *filename2
);
PyObject *PyErr_SetFromWindowsErr(int ierr);
PyObject *PyErr_SetFromWindowsErrWithFilename(int ierr, const char *filename);
void PyErr_SetInterrupt(void);
int PyErr_SetInterruptEx(int signum);
int PyErr_WarnEx(PyObject *category, const char *message, long long stack_level);
int PyErr_WarnFormat(PyObject *category, long long stack_level, const char *format);
int PyErr_WarnExplicit(
    PyObject *category,
    const char *message,
    const char *filename,
    int lineno,
    const char *module,
    PyObject *registry
);
int PyErr_ResourceWarning(PyObject *source, long long stack_level, const char *format);
int PyErr_BadArgument(void);
void PyErr_BadInternalCall(void);
void PyErr_PrintEx(int set_sys_last_vars);
void PyErr_Display(PyObject *unused, PyObject *value, PyObject *tb);
void PyErr_DisplayException(PyObject *exc);
void PyErr_Clear(void);
void PyErr_SyntaxLocation(const char *filename, int lineno);
void PyErr_SyntaxLocationEx(const char *filename, int lineno, int col_offset);
PyObject *PyErr_ProgramText(const char *filename, int lineno);
PyObject *PyErr_SetImportError(PyObject *msg, PyObject *name, PyObject *path);
PyObject *PyErr_SetImportErrorSubclass(
    PyObject *exception,
    PyObject *msg,
    PyObject *name,
    PyObject *path
);
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

PyObject *PyFile_FromFd(
    int fd,
    const char *name,
    const char *mode,
    int buffering,
    const char *encoding,
    const char *errors,
    const char *newline,
    int closefd
);
PyObject *PyFile_GetLine(PyObject *f, int n);
int PyFile_WriteObject(PyObject *v, PyObject *f, int flags);
int PyFile_WriteString(const char *s, PyObject *f);
void PyOS_BeforeFork(void);
void PyOS_AfterFork_Parent(void);
void PyOS_AfterFork_Child(void);
void PyOS_AfterFork(void);
int PyOS_CheckStack(void);
PyObject *PyOS_FSPath(PyObject *path);
int PyOS_InterruptOccurred(void);
char *PyOS_double_to_string(double val, char format_code, int precision, int flags, int *type);
PyOS_sighandler_t PyOS_getsig(int sig);
PyOS_sighandler_t PyOS_setsig(int sig, PyOS_sighandler_t handler);
int PyOS_mystricmp(const char *str1, const char *str2);
int PyOS_mystrnicmp(const char *str1, const char *str2, long long size);
int PyOS_vsnprintf(char *str, size_t size, const char *format, va_list va);

PyObject *PyCFunction_Call(PyObject *callable, PyObject *args, PyObject *kwargs);
PyObject *PyCFunction_New(PyMethodDef *ml, PyObject *self);
PyObject *PyCFunction_NewEx(PyMethodDef *ml, PyObject *self, PyObject *module);
PyObject *PyCMethod_New(PyMethodDef *ml, PyObject *self, PyObject *module, void *cls);
PyObject *PyDescr_NewMethod(PyTypeObject *type, PyMethodDef *method);
PyObject *PyDescr_NewClassMethod(PyTypeObject *type, PyMethodDef *method);
PyObject *PyDescr_NewMember(PyTypeObject *type, PyMemberDef *member);
PyObject *PyMember_GetOne(const char *obj_addr, PyMemberDef *member);
int PyMember_SetOne(char *obj_addr, PyMemberDef *member, PyObject *value);
PyObject *PyDescr_NewGetSet(PyTypeObject *type, PyGetSetDef *getset);
PyTypeObject *PyStructSequence_NewType(PyStructSequence_Desc *desc);
PyObject *PyStructSequence_New(PyTypeObject *type);
void PyStructSequence_SetItem(PyObject *object, long long index, PyObject *value);
PyObject *PyStructSequence_GetItem(PyObject *object, long long index);
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
extern PyObject *PyExc_BaseException;
extern PyObject *PyExc_BaseExceptionGroup;
extern PyObject *PyExc_GeneratorExit;
extern PyObject *PyExc_KeyboardInterrupt;
extern PyObject *PyExc_SystemExit;
extern PyObject *PyExc_StopIteration;
extern PyObject *PyExc_StopAsyncIteration;
extern PyObject *PyExc_ArithmeticError;
extern PyObject *PyExc_OverflowError;
extern PyObject *PyExc_FloatingPointError;
extern PyObject *PyExc_ZeroDivisionError;
extern PyObject *PyExc_AssertionError;
extern PyObject *PyExc_ImportError;
extern PyObject *PyExc_ModuleNotFoundError;
extern PyObject *PyExc_LookupError;
extern PyObject *PyExc_IndexError;
extern PyObject *PyExc_KeyError;
extern PyObject *PyExc_MemoryError;
extern PyObject *PyExc_NameError;
extern PyObject *PyExc_UnboundLocalError;
extern PyObject *PyExc_OSError;
extern PyObject *PyExc_BlockingIOError;
extern PyObject *PyExc_BrokenPipeError;
extern PyObject *PyExc_ChildProcessError;
extern PyObject *PyExc_ConnectionError;
extern PyObject *PyExc_ConnectionAbortedError;
extern PyObject *PyExc_ConnectionRefusedError;
extern PyObject *PyExc_ConnectionResetError;
extern PyObject *PyExc_FileExistsError;
extern PyObject *PyExc_FileNotFoundError;
extern PyObject *PyExc_InterruptedError;
extern PyObject *PyExc_IsADirectoryError;
extern PyObject *PyExc_NotADirectoryError;
extern PyObject *PyExc_PermissionError;
extern PyObject *PyExc_ProcessLookupError;
extern PyObject *PyExc_TimeoutError;
extern PyObject *PyExc_ReferenceError;
extern PyObject *PyExc_RuntimeError;
extern PyObject *PyExc_NotImplementedError;
extern PyObject *PyExc_RecursionError;
extern PyObject *PyExc_SyntaxError;
extern PyObject *PyExc_IndentationError;
extern PyObject *PyExc_TabError;
extern PyObject *PyExc_SystemError;
extern PyObject *PyExc_TypeError;
extern PyObject *PyExc_ValueError;
extern PyObject *PyExc_UnicodeError;
extern PyObject *PyExc_UnicodeDecodeError;
extern PyObject *PyExc_UnicodeEncodeError;
extern PyObject *PyExc_UnicodeTranslateError;
extern PyObject *PyExc_Warning;
extern PyObject *PyExc_DeprecationWarning;
extern PyObject *PyExc_PendingDeprecationWarning;
extern PyObject *PyExc_RuntimeWarning;
extern PyObject *PyExc_SyntaxWarning;
extern PyObject *PyExc_UserWarning;
extern PyObject *PyExc_FutureWarning;
extern PyObject *PyExc_ImportWarning;
extern PyObject *PyExc_UnicodeWarning;
extern PyObject *PyExc_BytesWarning;
extern PyObject *PyExc_EOFError;
extern PyObject *PyExc_ResourceWarning;
extern PyObject *PyExc_EncodingWarning;
extern PyObject *PyExc_EnvironmentError;
extern PyObject *PyExc_IOError;
extern PyObject *PyExc_WindowsError;
extern PyObject *PyExc_AttributeError;
extern PyObject *PyExc_BufferError;
extern const char PyStructSequence_UnnamedField[];
extern int (*PyOS_InputHook)(void);
extern const char *Py_FileSystemDefaultEncoding;
extern const char *Py_FileSystemDefaultEncodeErrors;
extern int Py_HasFileSystemDefaultEncoding;
extern int Py_UTF8Mode;
extern const unsigned long Py_Version;
extern long long _Py_RefTotal;
extern int _Py_SwappedOp[];
extern void *PyByteArrayIter_Type;
extern void *PyByteArray_Type;
extern void *PyBytesIter_Type;
extern void *PyCallIter_Type;
extern void *PyClassMethodDescr_Type;
extern void *PyDictItems_Type;
extern void *PyDictIterItem_Type;
extern void *PyDictIterKey_Type;
extern void *PyDictIterValue_Type;
extern void *PyDictKeys_Type;
extern void *PyDictRevIterItem_Type;
extern void *PyDictRevIterKey_Type;
extern void *PyDictRevIterValue_Type;
extern void *PyDictValues_Type;
extern void *PyEllipsis_Type;
extern void *PyEnum_Type;
extern void *PyFilter_Type;
extern void *PyGetSetDescr_Type;
extern void *PyListIter_Type;
extern void *PyListRevIter_Type;
extern void *PyLongRangeIter_Type;
extern void *PyMap_Type;
extern void *PyMemberDescr_Type;
extern void *PyMethodDescr_Type;
extern void *PyModuleDef_Type;
extern void *PyModule_Type;
extern void *PyProperty_Type;
extern void *PyRangeIter_Type;
extern void *PyRange_Type;
extern void *PyReversed_Type;
extern void *PySeqIter_Type;
extern void *PySetIter_Type;
extern void *PySuper_Type;
extern void *PyTraceBack_Type;
extern void *PyTupleIter_Type;
extern void *PyUnicodeIter_Type;
extern void *PyWrapperDescr_Type;
extern void *PyZip_Type;
extern void *Py_GenericAliasType;
extern void *_PyWeakref_CallableProxyType;
extern void *_PyWeakref_ProxyType;
extern void *_PyWeakref_RefType;

#define PyMODINIT_FUNC PyObject *

#ifdef __cplusplus
}
#endif

#endif /* PYRS_CPYTHON_COMPAT_H */
