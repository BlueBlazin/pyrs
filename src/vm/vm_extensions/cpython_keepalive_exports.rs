use std::ffi::{c_char, c_double, c_int, c_long, c_uint, c_ulong, c_void};

use super::{
    CpythonBuffer, CpythonComplexValue, CpythonInittabInitFunc, CpythonTypeObject, Cwchar,
};

#[used]
static KEEP2_PYINSTANCEMETHOD_TYPE: unsafe extern "C" fn() -> *mut CpythonTypeObject =
    keep2_pyinstancemethod_type;

unsafe extern "C" fn keep2_pyinstancemethod_type() -> *mut CpythonTypeObject {
    std::ptr::addr_of_mut!(super::PyInstanceMethod_Type)
}

#[used]
static KEEP2_PYLONG_FROMSSIZE_T: unsafe extern "C" fn(isize) -> *mut c_void =
    super::PyLong_FromSsize_t;
#[used]
static KEEP2_PYLONG_FROMSIZE_T: unsafe extern "C" fn(usize) -> *mut c_void =
    super::PyLong_FromSize_t;
#[used]
static KEEP2_PYLONG_FROMINT32: unsafe extern "C" fn(i32) -> *mut c_void = super::PyLong_FromInt32;
#[used]
static KEEP2_PYLONG_FROMUINT32: unsafe extern "C" fn(u32) -> *mut c_void = super::PyLong_FromUInt32;
#[used]
static KEEP2_PYLONG_FROMINT64: unsafe extern "C" fn(i64) -> *mut c_void = super::PyLong_FromInt64;
#[used]
static KEEP2_PYLONG_FROMUINT64: unsafe extern "C" fn(u64) -> *mut c_void = super::PyLong_FromUInt64;
#[used]
static KEEP2_PYLONG_FROMUNSIGNEDLONG: unsafe extern "C" fn(u64) -> *mut c_void =
    super::PyLong_FromUnsignedLong;
#[used]
static KEEP2_PYLONG_FROMUNSIGNEDLONGLONG: unsafe extern "C" fn(u64) -> *mut c_void =
    super::PyLong_FromUnsignedLongLong;
#[used]
static KEEP2_PYLONG_FROMVOIDPTR: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyLong_FromVoidPtr;
#[used]
static KEEP2_PYLONG_FROMUNICODEOBJECT: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    super::PyLong_FromUnicodeObject;
#[used]
static KEEP2_PYLONG_FROMSTRING: unsafe extern "C" fn(
    *const c_char,
    *mut *mut c_char,
    i32,
) -> *mut c_void = super::PyLong_FromString;
#[used]
static KEEP2_PYLONG_GETINFO: unsafe extern "C" fn() -> *mut c_void = super::PyLong_GetInfo;
#[used]
static KEEP2_PYMODULE_GETDICT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyModule_GetDict;
#[used]
static KEEP2_PYMODULE_NEWOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyModule_NewObject;
#[used]
static KEEP2_PYMODULE_NEW: unsafe extern "C" fn(*const c_char) -> *mut c_void = super::PyModule_New;
#[used]
static KEEP2_PYMODULE_GETNAMEOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyModule_GetNameObject;
#[used]
static KEEP2_PYMODULE_GETNAME: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    super::PyModule_GetName;
#[used]
static KEEP2_PYMODULE_GETFILENAMEOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyModule_GetFilenameObject;
#[used]
static KEEP2_PYMODULE_GETFILENAME: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    super::PyModule_GetFilename;
#[used]
static KEEP2_PYMODULE_SETDOCSTRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    super::PyModule_SetDocString;
#[used]
static KEEP2_PYMODULE_ADD: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> i32 =
    super::PyModule_Add;
#[used]
static KEEP2_PYMODULE_ADDFUNCTIONS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyModule_AddFunctions;
#[used]
static KEEP2_PYMODULE_ADDTYPE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyModule_AddType;
#[used]
static KEEP2_PYMODULE_FROMDEFANDSPEC2: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    i32,
) -> *mut c_void = super::PyModule_FromDefAndSpec2;
#[used]
static KEEP2_PYMODULE_EXECDEF: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyModule_ExecDef;
#[used]
static KEEP2_PYMODULE_GETDEF: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyModule_GetDef;
#[used]
static KEEP2_PYMODULE_GETSTATE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyModule_GetState;
#[used]
static KEEP2_PYSTRUCTSEQUENCE_NEWTYPE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyStructSequence_NewType;
#[used]
static KEEP2_PYSTRUCTSEQUENCE_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyStructSequence_New;
#[used]
static KEEP2_PYSTRUCTSEQUENCE_SETITEM: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) =
    super::PyStructSequence_SetItem;
#[used]
static KEEP2_PYSTRUCTSEQUENCE_GETITEM: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    super::PyStructSequence_GetItem;
#[used]
static KEEP2_PYTUPLE_NEW: unsafe extern "C" fn(isize) -> *mut c_void = super::PyTuple_New;
#[used]
static KEEP2_PYTUPLE_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = super::PyTuple_Size;
#[used]
static KEEP2_PYTUPLE_GETITEM: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    super::PyTuple_GetItem;
#[used]
static KEEP2_PYTUPLE_SETITEM: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) -> i32 =
    super::PyTuple_SetItem;
#[used]
static KEEP2_PYTUPLE_GETSLICE: unsafe extern "C" fn(*mut c_void, isize, isize) -> *mut c_void =
    super::PyTuple_GetSlice;
#[used]
static KEEP2_PYTUPLE_PACK: unsafe extern "C" fn(isize, ...) -> *mut c_void = super::PyTuple_Pack;
#[used]
static KEEP2_PYOBJECT_GETITEM: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyObject_GetItem;
#[used]
static KEEP2_PYOBJECT_SETITEM: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
    super::PyObject_SetItem;
#[used]
static KEEP2_PYOBJECT_DELITEM: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyObject_DelItem;
#[used]
static KEEP2_PYOBJECT_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = super::PyObject_Size;
#[used]
static KEEP2_PYOBJECT_LENGTH: unsafe extern "C" fn(*mut c_void) -> isize = super::PyObject_Length;
#[used]
static KEEP2_PYOBJECT_LENGTHHINT: unsafe extern "C" fn(*mut c_void, isize) -> isize =
    super::PyObject_LengthHint;
#[used]
static KEEP2_PYOBJECT_TYPE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyObject_Type;
#[used]
static KEEP2__PYOBJECT_TYPE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::_PyObject_Type;
#[used]
static KEEP2_PYOBJECT_GETTYPEDATA: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyObject_GetTypeData;
#[used]
static KEEP2_PYOBJECT_HASH: unsafe extern "C" fn(*mut c_void) -> isize = super::PyObject_Hash;
#[used]
static KEEP2_PYOBJECT_HASHNOTIMPLEMENTED: unsafe extern "C" fn(*mut c_void) -> isize =
    super::PyObject_HashNotImplemented;
#[used]
static KEEP2_PYOBJECT_RICHCOMPARE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    i32,
) -> *mut c_void = super::PyObject_RichCompare;
#[used]
static KEEP2_PYOBJECT_RICHCOMPAREBOOL: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 =
    super::PyObject_RichCompareBool;
#[used]
static KEEP2_PYOBJECT_ISINSTANCE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyObject_IsInstance;
#[used]
static KEEP2_PYOBJECT_ISSUBCLASS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyObject_IsSubclass;
#[used]
static KEEP2_PYOBJECT_GETOPTIONALATTR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32 = super::PyObject_GetOptionalAttr;
#[used]
static KEEP2_PYSEQUENCE_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = super::PySequence_Check;
#[used]
static KEEP2_PYSEQUENCE_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = super::PySequence_Size;
#[used]
static KEEP2_PYSEQUENCE_LENGTH: unsafe extern "C" fn(*mut c_void) -> isize =
    super::PySequence_Length;
#[used]
static KEEP2_PYSEQUENCE_GETITEM: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    super::PySequence_GetItem;
#[used]
static KEEP2_PYSEQUENCE_GETSLICE: unsafe extern "C" fn(*mut c_void, isize, isize) -> *mut c_void =
    super::PySequence_GetSlice;
#[used]
static KEEP2_PYSEQUENCE_SETITEM: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) -> i32 =
    super::PySequence_SetItem;
#[used]
static KEEP2_PYSEQUENCE_DELITEM: unsafe extern "C" fn(*mut c_void, isize) -> i32 =
    super::PySequence_DelItem;
#[used]
static KEEP2_PYSEQUENCE_SETSLICE: unsafe extern "C" fn(
    *mut c_void,
    isize,
    isize,
    *mut c_void,
) -> i32 = super::PySequence_SetSlice;
#[used]
static KEEP2_PYSEQUENCE_DELSLICE: unsafe extern "C" fn(*mut c_void, isize, isize) -> i32 =
    super::PySequence_DelSlice;
#[used]
static KEEP2_PYSEQUENCE_CONTAINS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PySequence_Contains;
#[used]
static KEEP2_PYSEQUENCE_IN: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PySequence_In;
#[used]
static KEEP2_PYSEQUENCE_TUPLE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PySequence_Tuple;
#[used]
static KEEP2_PYSEQUENCE_LIST: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PySequence_List;
#[used]
static KEEP2_PYSEQUENCE_FAST: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
    super::PySequence_Fast;
#[used]
static KEEP2_PYSEQUENCE_CONCAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PySequence_Concat;
#[used]
static KEEP2_PYSEQUENCE_INPLACECONCAT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PySequence_InPlaceConcat;
#[used]
static KEEP2_PYSEQUENCE_REPEAT: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    super::PySequence_Repeat;
#[used]
static KEEP2_PYSEQUENCE_INPLACEREPEAT: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    super::PySequence_InPlaceRepeat;
#[used]
static KEEP2_PYSEQUENCE_COUNT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize =
    super::PySequence_Count;
#[used]
static KEEP2_PYSEQUENCE_INDEX: unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize =
    super::PySequence_Index;
#[used]
static KEEP2_PYMAPPING_GETITEMSTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyMapping_GetItemString;
#[used]
static KEEP2_PYMAPPING_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyMapping_Check;
#[used]
static KEEP2_PYMAPPING_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = super::PyMapping_Size;
#[used]
static KEEP2_PYMAPPING_LENGTH: unsafe extern "C" fn(*mut c_void) -> isize = super::PyMapping_Length;
#[used]
static KEEP2_PYMAPPING_KEYS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyMapping_Keys;
#[used]
static KEEP2_PYMAPPING_ITEMS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyMapping_Items;
#[used]
static KEEP2_PYMAPPING_VALUES: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyMapping_Values;
#[used]
static KEEP2_PYMAPPING_GETOPTIONALITEM: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32 = super::PyMapping_GetOptionalItem;
#[used]
static KEEP2_PYMAPPING_GETOPTIONALITEMSTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut *mut c_void,
) -> i32 = super::PyMapping_GetOptionalItemString;
#[used]
static KEEP2_PYMAPPING_SETITEMSTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = super::PyMapping_SetItemString;
#[used]
static KEEP2_PYMAPPING_HASKEYWITHERROR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyMapping_HasKeyWithError;
#[used]
static KEEP2_PYMAPPING_HASKEYSTRINGWITHERROR: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> i32 = super::PyMapping_HasKeyStringWithError;
#[used]
static KEEP2_PYMAPPING_HASKEY: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyMapping_HasKey;
#[used]
static KEEP2_PYMAPPING_HASKEYSTRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    super::PyMapping_HasKeyString;
#[used]
static KEEP2_PYSEQITER_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PySeqIter_New;
#[used]
static KEEP2_PYCALLITER_NEW: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyCallIter_New;
#[used]
static KEEP2_PYOBJECT_ASFILEDESCRIPTOR: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyObject_AsFileDescriptor;
#[used]
static KEEP2_PYOBJECT_CHECKBUFFER: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyObject_CheckBuffer;
#[used]
static KEEP2_PYOBJECT_CHECKREADBUFFER: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyObject_CheckReadBuffer;
#[used]
static KEEP2_PYOBJECT_ASREADBUFFER: unsafe extern "C" fn(
    *mut c_void,
    *mut *const c_void,
    *mut isize,
) -> i32 = super::PyObject_AsReadBuffer;
#[used]
static KEEP2_PYOBJECT_ASWRITEBUFFER: unsafe extern "C" fn(
    *mut c_void,
    *mut *mut c_void,
    *mut isize,
) -> i32 = super::PyObject_AsWriteBuffer;
#[used]
static KEEP2_PYOBJECT_ASCHARBUFFER: unsafe extern "C" fn(
    *mut c_void,
    *mut *const c_char,
    *mut isize,
) -> i32 = super::PyObject_AsCharBuffer;
#[used]
static KEEP2_PYOBJECT_COPYDATA: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyObject_CopyData;
#[used]
static KEEP2_PYMEMORYVIEW_FROMOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyMemoryView_FromObject;
#[used]
static KEEP2_PYMEMORYVIEW_FROMMEMORY: unsafe extern "C" fn(*mut c_char, isize, i32) -> *mut c_void =
    super::PyMemoryView_FromMemory;
#[used]
static KEEP2_PYMEMORYVIEW_FROMBUFFER: unsafe extern "C" fn(*const CpythonBuffer) -> *mut c_void =
    super::PyMemoryView_FromBuffer;
#[used]
static KEEP2_PYMEMORYVIEW_GETCONTIGUOUS: unsafe extern "C" fn(
    *mut c_void,
    i32,
    c_char,
) -> *mut c_void = super::PyMemoryView_GetContiguous;
#[used]
static KEEP2_PYOBJECT_GETBUFFER: unsafe extern "C" fn(*mut c_void, *mut CpythonBuffer, i32) -> i32 =
    super::PyObject_GetBuffer;
#[used]
static KEEP2_PYBUFFER_ISCONTIGUOUS: unsafe extern "C" fn(*const CpythonBuffer, c_char) -> i32 =
    super::PyBuffer_IsContiguous;
#[used]
static KEEP2_PYBUFFER_GETPOINTER: unsafe extern "C" fn(
    *const CpythonBuffer,
    *const isize,
) -> *mut c_void = super::PyBuffer_GetPointer;
#[used]
static KEEP2_PYBUFFER_SIZEFROMFORMAT: unsafe extern "C" fn(*const c_char) -> isize =
    super::PyBuffer_SizeFromFormat;
#[used]
static KEEP2_PYBUFFER_FROMCONTIGUOUS: unsafe extern "C" fn(
    *const CpythonBuffer,
    *const c_void,
    isize,
    c_char,
) -> i32 = super::PyBuffer_FromContiguous;
#[used]
static KEEP2_PYBUFFER_TOCONTIGUOUS: unsafe extern "C" fn(
    *mut c_void,
    *const CpythonBuffer,
    isize,
    c_char,
) -> i32 = super::PyBuffer_ToContiguous;
#[used]
static KEEP2_PYBUFFER_FILLCONTIGUOUSSTRIDES: unsafe extern "C" fn(
    i32,
    *const isize,
    *mut isize,
    i32,
    c_char,
) = super::PyBuffer_FillContiguousStrides;
#[used]
static KEEP2_PYBUFFER_FILLINFO: unsafe extern "C" fn(
    *mut CpythonBuffer,
    *mut c_void,
    *mut c_void,
    isize,
    i32,
    i32,
) -> i32 = super::PyBuffer_FillInfo;
#[used]
static KEEP2_PYOBJECT_CALLMETHODOBJARGS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    ...
) -> *mut c_void = super::PyObject_CallMethodObjArgs;
#[used]
static KEEP2_PYOBJECT_PRINT: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 =
    super::PyObject_Print;
#[used]
static KEEP2_PYTYPE_GETFLAGS: unsafe extern "C" fn(*mut c_void) -> usize = super::PyType_GetFlags;
#[used]
static KEEP2_PYTYPE_ISSUBTYPE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyType_IsSubtype;
#[used]
static KEEP2_PYTYPE_READY: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyType_Ready;
#[used]
static KEEP2_PYTYPE_GENERICALLOC: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    super::PyType_GenericAlloc;
#[used]
static KEEP2_PYTYPE_GENERICNEW: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyType_GenericNew;
#[used]
static KEEP2_PYTYPE_FROMMETACLASS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyType_FromMetaclass;
#[used]
static KEEP2_PYTYPE_FROMMODULEANDSPEC: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyType_FromModuleAndSpec;
#[used]
static KEEP2_PYTYPE_FROMSPECWITHBASES: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyType_FromSpecWithBases;
#[used]
static KEEP2_PYTYPE_FROMSPEC: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyType_FromSpec;
#[used]
static KEEP2_PYTYPE_GETNAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyType_GetName;
#[used]
static KEEP2_PYTYPE_GETQUALNAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyType_GetQualName;
#[used]
static KEEP2_PYTYPE_GETMODULENAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyType_GetModuleName;
#[used]
static KEEP2_PYTYPE_GETFULLYQUALIFIEDNAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyType_GetFullyQualifiedName;
#[used]
static KEEP2_PYTYPE_GETSLOT: unsafe extern "C" fn(*mut c_void, c_int) -> *mut c_void =
    super::PyType_GetSlot;
#[used]
static KEEP2__PYTYPE_LOOKUP: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::_PyType_Lookup;
#[used]
static KEEP2_PYTYPE_GETMODULE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyType_GetModule;
#[used]
static KEEP2_PYTYPE_GETMODULESTATE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyType_GetModuleState;
#[used]
static KEEP2_PYTYPE_GETMODULEBYDEF: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyType_GetModuleByDef;
#[used]
static KEEP2_PYTYPE_GETTYPEDATASIZE: unsafe extern "C" fn(*mut c_void) -> isize =
    super::PyType_GetTypeDataSize;
#[used]
static KEEP2_PYTYPE_GETBASEBYTOKEN: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> c_int = super::PyType_GetBaseByToken;
#[used]
static KEEP2_PYTYPE_CLEARCACHE: unsafe extern "C" fn() -> c_uint = super::PyType_ClearCache;
#[used]
static KEEP2_PYTYPE_MODIFIED: unsafe extern "C" fn(*mut c_void) = super::PyType_Modified;
#[used]
static KEEP2_PYTYPE_FREEZE: unsafe extern "C" fn(*mut c_void) -> c_int = super::PyType_Freeze;
#[used]
static KEEP2_PYOBJECT_MALLOC: unsafe extern "C" fn(usize) -> *mut c_void = super::PyObject_Malloc;
#[used]
static KEEP2_PYOBJECT_CALLOC: unsafe extern "C" fn(usize, usize) -> *mut c_void =
    super::PyObject_Calloc;
#[used]
static KEEP2_PYOBJECT_REALLOC: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void =
    super::PyObject_Realloc;
#[used]
static KEEP2_PYOBJECT_FREE: unsafe extern "C" fn(*mut c_void) = super::PyObject_Free;
#[used]
static KEEP2_PYOBJECT_GC_TRACK: unsafe extern "C" fn(*mut c_void) = super::PyObject_GC_Track;
#[used]
static KEEP2_PYOBJECT_GC_UNTRACK: unsafe extern "C" fn(*mut c_void) = super::PyObject_GC_UnTrack;
#[used]
static KEEP2_PYOBJECT_GC_ISTRACKED: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyObject_GC_IsTracked;
#[used]
static KEEP2_PYOBJECT_GC_ISFINALIZED: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyObject_GC_IsFinalized;
#[used]
static KEEP2_PYOBJECT_GC_DEL: unsafe extern "C" fn(*mut c_void) = super::PyObject_GC_Del;
#[used]
static KEEP2_PYGC_COLLECT: unsafe extern "C" fn() -> isize = super::PyGC_Collect;
#[used]
static KEEP2_PYGC_ENABLE: unsafe extern "C" fn() -> i32 = super::PyGC_Enable;
#[used]
static KEEP2_PYGC_DISABLE: unsafe extern "C" fn() -> i32 = super::PyGC_Disable;
#[used]
static KEEP2_PYGC_IS_ENABLED: unsafe extern "C" fn() -> i32 = super::PyGC_IsEnabled;
#[used]
static KEEP2_PYOBJECT_CLEAR_WEAKREFS: unsafe extern "C" fn(*mut c_void) =
    super::PyObject_ClearWeakRefs;
#[used]
static KEEP2_PYWEAKREF_NEWREF: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyWeakref_NewRef;
#[used]
static KEEP2_PYWEAKREF_NEWPROXY: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyWeakref_NewProxy;
#[used]
static KEEP2_PYWEAKREF_GETREF: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> c_int =
    super::PyWeakref_GetRef;
#[used]
static KEEP2_PYWEAKREF_GETOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyWeakref_GetObject;
#[used]
static KEEP2_PY_ADDPENDINGCALL: unsafe extern "C" fn(
    Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
    *mut c_void,
) -> c_int = super::Py_AddPendingCall;
#[used]
static KEEP2_PY_MAKEPENDINGCALLS: unsafe extern "C" fn() -> c_int = super::Py_MakePendingCalls;
#[used]
static KEEP2_PY_ATEXIT: unsafe extern "C" fn(Option<unsafe extern "C" fn()>) -> c_int =
    super::Py_AtExit;
#[used]
static KEEP2_PY_GETRECURSIONLIMIT: unsafe extern "C" fn() -> c_int = super::Py_GetRecursionLimit;
#[used]
static KEEP2_PY_SETRECURSIONLIMIT: unsafe extern "C" fn(c_int) = super::Py_SetRecursionLimit;
#[used]
static KEEP2_PY_GETVERSION: unsafe extern "C" fn() -> *const c_char = super::Py_GetVersion;
#[used]
static KEEP2_PY_GETBUILDINFO: unsafe extern "C" fn() -> *const c_char = super::Py_GetBuildInfo;
#[used]
static KEEP2_PY_GETCOMPILER: unsafe extern "C" fn() -> *const c_char = super::Py_GetCompiler;
#[used]
static KEEP2_PY_GETPLATFORM: unsafe extern "C" fn() -> *const c_char = super::Py_GetPlatform;
#[used]
static KEEP2_PY_GETCOPYRIGHT: unsafe extern "C" fn() -> *const c_char = super::Py_GetCopyright;
#[used]
static KEEP2_PY_GETARGCARGV: unsafe extern "C" fn(*mut c_int, *mut *mut *mut Cwchar) =
    super::Py_GetArgcArgv;
#[used]
static KEEP2_PY_SETPROGRAMNAME: unsafe extern "C" fn(*const Cwchar) = super::Py_SetProgramName;
#[used]
static KEEP2_PY_GETPROGRAMNAME: unsafe extern "C" fn() -> *mut Cwchar = super::Py_GetProgramName;
#[used]
static KEEP2_PY_SETPYTHONHOME: unsafe extern "C" fn(*const Cwchar) = super::Py_SetPythonHome;
#[used]
static KEEP2_PY_GETPYTHONHOME: unsafe extern "C" fn() -> *mut Cwchar = super::Py_GetPythonHome;
#[used]
static KEEP2_PY_SETPATH: unsafe extern "C" fn(*const Cwchar) = super::Py_SetPath;
#[used]
static KEEP2_PY_GETPATH: unsafe extern "C" fn() -> *mut Cwchar = super::Py_GetPath;
#[used]
static KEEP2_PY_GETPREFIX: unsafe extern "C" fn() -> *mut Cwchar = super::Py_GetPrefix;
#[used]
static KEEP2_PY_GETEXECPREFIX: unsafe extern "C" fn() -> *mut Cwchar = super::Py_GetExecPrefix;
#[used]
static KEEP2_PY_GETPROGRAMFULLPATH: unsafe extern "C" fn() -> *mut Cwchar =
    super::Py_GetProgramFullPath;
#[used]
static KEEP2_PY_ENCODELOCALE: unsafe extern "C" fn(*const Cwchar, *mut usize) -> *mut c_char =
    super::Py_EncodeLocale;
#[used]
static KEEP2_PY_DECODELOCALE: unsafe extern "C" fn(*const c_char, *mut usize) -> *mut Cwchar =
    super::Py_DecodeLocale;
#[used]
static KEEP2_PY_PACK_FULL_VERSION: unsafe extern "C" fn(c_int, c_int, c_int, c_int, c_int) -> u32 =
    super::Py_PACK_FULL_VERSION;
#[used]
static KEEP2_PY_PACK_VERSION: unsafe extern "C" fn(c_int, c_int) -> u32 = super::Py_PACK_VERSION;
#[used]
static KEEP2_PY_INITIALIZE: unsafe extern "C" fn() = super::Py_Initialize;
#[used]
static KEEP2_PY_INITIALIZEEX: unsafe extern "C" fn(c_int) = super::Py_InitializeEx;
#[used]
static KEEP2_PY_MAIN: unsafe extern "C" fn(c_int, *mut *mut Cwchar) -> c_int = super::Py_Main;
#[used]
static KEEP2_PY_BYTESMAIN: unsafe extern "C" fn(c_int, *mut *mut c_char) -> c_int =
    super::Py_BytesMain;
#[used]
static KEEP2_PY_COMPILESTRING: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    c_int,
) -> *mut c_void = super::Py_CompileString;
#[used]
static KEEP2_PY_FINALIZE: unsafe extern "C" fn() = super::Py_Finalize;
#[used]
static KEEP2_PY_FINALIZEEX: unsafe extern "C" fn() -> c_int = super::Py_FinalizeEx;
#[used]
static KEEP2_PY_EXIT: unsafe extern "C" fn(c_int) = super::Py_Exit;
#[used]
static KEEP2_PY_FATALERROR: unsafe extern "C" fn(*const c_char) = super::Py_FatalError;
#[used]
static KEEP2_PY_FATALERRORFUNC: unsafe extern "C" fn(*const c_char, *const c_char) =
    super::_Py_FatalErrorFunc;
#[used]
static KEEP2_PY_NEWINTERPRETER: unsafe extern "C" fn() -> *mut c_void = super::Py_NewInterpreter;
#[used]
static KEEP2_PY_ENDINTERPRETER: unsafe extern "C" fn(*mut c_void) = super::Py_EndInterpreter;
#[used]
static KEEP2_PY_ISFINALIZING: unsafe extern "C" fn() -> c_int = super::Py_IsFinalizing;
#[used]
static KEEP2_PY_REPRENTER: unsafe extern "C" fn(*mut c_void) -> c_int = super::Py_ReprEnter;
#[used]
static KEEP2_PY_REPRLEAVE: unsafe extern "C" fn(*mut c_void) = super::Py_ReprLeave;
#[used]
static KEEP2_PYOBJECT_INIT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyObject_Init;
#[used]
static KEEP2_PYOBJECT_INITVAR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
) -> *mut c_void = super::PyObject_InitVar;
#[used]
static KEEP2_PYSLICE_NEW: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PySlice_New;
#[used]
static KEEP2_PYSLICE_UNPACK: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
    *mut isize,
    *mut isize,
) -> i32 = super::PySlice_Unpack;
#[used]
static KEEP2_PYSLICE_ADJUSTINDICES: unsafe extern "C" fn(
    isize,
    *mut isize,
    *mut isize,
    isize,
) -> isize = super::PySlice_AdjustIndices;
#[used]
static KEEP2_PYSLICE_GETINDICES: unsafe extern "C" fn(
    *mut c_void,
    isize,
    *mut isize,
    *mut isize,
    *mut isize,
) -> i32 = super::PySlice_GetIndices;
#[used]
static KEEP2_PYSLICE_GETINDICESEX: unsafe extern "C" fn(
    *mut c_void,
    isize,
    *mut isize,
    *mut isize,
    *mut isize,
    *mut isize,
) -> i32 = super::PySlice_GetIndicesEx;
#[used]
static KEEP2_PYSYS_GETOBJECT: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PySys_GetObject;
#[used]
static KEEP2_PYSYS_SETOBJECT: unsafe extern "C" fn(*const c_char, *mut c_void) -> i32 =
    super::PySys_SetObject;
#[used]
static KEEP2_PYSYS_GETXOPTIONS: unsafe extern "C" fn() -> *mut c_void = super::PySys_GetXOptions;
#[used]
static KEEP2_PYSYS_ADDXOPTION: unsafe extern "C" fn(*const Cwchar) = super::PySys_AddXOption;
#[used]
static KEEP2_PYSYS_HASWARNOPTIONS: unsafe extern "C" fn() -> i32 = super::PySys_HasWarnOptions;
#[used]
static KEEP2_PYSYS_RESETWARNOPTIONS: unsafe extern "C" fn() = super::PySys_ResetWarnOptions;
#[used]
static KEEP2_PYSYS_ADDWARNOPTION: unsafe extern "C" fn(*const Cwchar) = super::PySys_AddWarnOption;
#[used]
static KEEP2_PYSYS_ADDWARNOPTIONUNICODE: unsafe extern "C" fn(*const Cwchar) =
    super::PySys_AddWarnOptionUnicode;
#[used]
static KEEP2_PYSYS_AUDITTUPLE: unsafe extern "C" fn(*const c_char, *mut c_void) -> i32 =
    super::PySys_AuditTuple;
#[used]
static KEEP2_PYSYS_SETARGV: unsafe extern "C" fn(i32, *mut *mut Cwchar) = super::PySys_SetArgv;
#[used]
static KEEP2_PYSYS_SETARGVEX: unsafe extern "C" fn(i32, *mut *mut Cwchar, i32) =
    super::PySys_SetArgvEx;
#[used]
static KEEP2_PYSYS_SETPATH: unsafe extern "C" fn(*const Cwchar) = super::PySys_SetPath;
#[used]
static KEEP2_PYTHREAD_INIT_THREAD: unsafe extern "C" fn() = super::PyThread_init_thread;
#[used]
static KEEP2_PYTHREAD_START_NEW_THREAD: unsafe extern "C" fn(
    Option<unsafe extern "C" fn(*mut c_void)>,
    *mut c_void,
) -> c_ulong = super::PyThread_start_new_thread;
#[used]
static KEEP2_PYTHREAD_EXIT_THREAD: unsafe extern "C" fn() = super::PyThread_exit_thread;
#[used]
static KEEP2_PYTHREAD_GET_THREAD_IDENT: unsafe extern "C" fn() -> c_ulong =
    super::PyThread_get_thread_ident;
#[used]
static KEEP2_PYTHREAD_GET_THREAD_NATIVE_ID: unsafe extern "C" fn() -> c_ulong =
    super::PyThread_get_thread_native_id;
#[used]
static KEEP2_PYTHREAD_ALLOCATE_LOCK: unsafe extern "C" fn() -> *mut c_void =
    super::PyThread_allocate_lock;
#[used]
static KEEP2_PYTHREAD_FREE_LOCK: unsafe extern "C" fn(*mut c_void) = super::PyThread_free_lock;
#[used]
static KEEP2_PYTHREAD_ACQUIRE_LOCK: unsafe extern "C" fn(*mut c_void, c_int) -> c_int =
    super::PyThread_acquire_lock;
#[used]
static KEEP2_PYTHREAD_ACQUIRE_LOCK_TIMED: unsafe extern "C" fn(*mut c_void, i64, c_int) -> c_int =
    super::PyThread_acquire_lock_timed;
#[used]
static KEEP2_PYTHREAD_RELEASE_LOCK: unsafe extern "C" fn(*mut c_void) =
    super::PyThread_release_lock;
#[used]
static KEEP2_PYTHREAD_GET_STACKSIZE: unsafe extern "C" fn() -> usize =
    super::PyThread_get_stacksize;
#[used]
static KEEP2_PYTHREAD_SET_STACKSIZE: unsafe extern "C" fn(usize) -> c_int =
    super::PyThread_set_stacksize;
#[used]
static KEEP2_PYTHREAD_GETINFO: unsafe extern "C" fn() -> *mut c_void = super::PyThread_GetInfo;
#[used]
static KEEP2_PYTHREAD_CREATE_KEY: unsafe extern "C" fn() -> c_int = super::PyThread_create_key;
#[used]
static KEEP2_PYTHREAD_DELETE_KEY: unsafe extern "C" fn(c_int) = super::PyThread_delete_key;
#[used]
static KEEP2_PYTHREAD_SET_KEY_VALUE: unsafe extern "C" fn(c_int, *mut c_void) -> c_int =
    super::PyThread_set_key_value;
#[used]
static KEEP2_PYTHREAD_GET_KEY_VALUE: unsafe extern "C" fn(c_int) -> *mut c_void =
    super::PyThread_get_key_value;
#[used]
static KEEP2_PYTHREAD_DELETE_KEY_VALUE: unsafe extern "C" fn(c_int) =
    super::PyThread_delete_key_value;
#[used]
static KEEP2_PYTHREAD_REINIT_TLS: unsafe extern "C" fn() = super::PyThread_ReInitTLS;
#[used]
static KEEP2_PYTHREAD_TSS_ALLOC: unsafe extern "C" fn() -> *mut c_void = super::PyThread_tss_alloc;
#[used]
static KEEP2_PYTHREAD_TSS_FREE: unsafe extern "C" fn(*mut c_void) = super::PyThread_tss_free;
#[used]
static KEEP2_PYTHREAD_TSS_IS_CREATED: unsafe extern "C" fn(*mut c_void) -> c_int =
    super::PyThread_tss_is_created;
#[used]
static KEEP2_PYTHREAD_TSS_CREATE: unsafe extern "C" fn(*mut c_void) -> c_int =
    super::PyThread_tss_create;
#[used]
static KEEP2_PYTHREAD_TSS_DELETE: unsafe extern "C" fn(*mut c_void) = super::PyThread_tss_delete;
#[used]
static KEEP2_PYTHREAD_TSS_SET: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
    super::PyThread_tss_set;
#[used]
static KEEP2_PYTHREAD_TSS_GET: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyThread_tss_get;
#[used]
static KEEP2_PYTHREADSTATE_GET: unsafe extern "C" fn() -> *mut c_void = super::PyThreadState_Get;
#[used]
static KEEP2_PYTHREADSTATE_GETUNCHECKED: unsafe extern "C" fn() -> *mut c_void =
    super::PyThreadState_GetUnchecked;
#[used]
static KEEP2_PYTHREADSTATE_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyThreadState_New;
#[used]
static KEEP2__PYTHREADSTATE_PREALLOC: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::_PyThreadState_Prealloc;
#[used]
static KEEP2__PYTHREADSTATE_INIT: unsafe extern "C" fn(*mut c_void) = super::_PyThreadState_Init;
#[used]
static KEEP2_PYTHREADSTATE_SWAP: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyThreadState_Swap;
#[used]
static KEEP2_PYTHREADSTATE_CLEAR: unsafe extern "C" fn(*mut c_void) = super::PyThreadState_Clear;
#[used]
static KEEP2_PYTHREADSTATE_DELETE: unsafe extern "C" fn(*mut c_void) = super::PyThreadState_Delete;
#[used]
static KEEP2_PYTHREADSTATE_DELETECURRENT: unsafe extern "C" fn() =
    super::PyThreadState_DeleteCurrent;
#[used]
static KEEP2_PYTHREADSTATE_SETASYNCEXC: unsafe extern "C" fn(u64, *mut c_void) -> i32 =
    super::PyThreadState_SetAsyncExc;
#[used]
static KEEP2_PYTHREADSTATE_GETFRAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyThreadState_GetFrame;
#[used]
static KEEP2_PYFRAME_NEW: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyFrame_New;
#[used]
static KEEP2_PYFRAME_GETCODE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyFrame_GetCode;
#[used]
static KEEP2_PYFRAME_GETBACK: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::cpython_thread_interp_api::PyFrame_GetBack;
#[used]
static KEEP2_PYFRAME_GETLINENUMBER: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyFrame_GetLineNumber;
#[used]
static KEEP2_PYTHREADSTATE_GETINTERPRETER: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyThreadState_GetInterpreter;
#[used]
static KEEP2_PYTHREADSTATE_GETID: unsafe extern "C" fn(*mut c_void) -> u64 =
    super::PyThreadState_GetID;
#[used]
static KEEP2_PYTHREADSTATE_GETDICT: unsafe extern "C" fn() -> *mut c_void =
    super::PyThreadState_GetDict;
#[used]
static KEEP2_PYINTERPRETERSTATE_GET: unsafe extern "C" fn() -> *mut c_void =
    super::PyInterpreterState_Get;
#[used]
static KEEP2_PYINTERPRETERSTATE_NEW: unsafe extern "C" fn() -> *mut c_void =
    super::PyInterpreterState_New;
#[used]
static KEEP2_PYINTERPRETERSTATE_CLEAR: unsafe extern "C" fn(*mut c_void) =
    super::PyInterpreterState_Clear;
#[used]
static KEEP2_PYINTERPRETERSTATE_DELETE: unsafe extern "C" fn(*mut c_void) =
    super::PyInterpreterState_Delete;
#[used]
static KEEP2_PYINTERPRETERSTATE_GETID: unsafe extern "C" fn(*mut c_void) -> i64 =
    super::PyInterpreterState_GetID;
#[used]
static KEEP2_PYINTERPRETERSTATE_GETDICT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyInterpreterState_GetDict;
#[used]
static KEEP2_PYSTATE_ADDMODULE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyState_AddModule;
#[used]
static KEEP2__PYSTATE_ADDMODULE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> i32 = super::_PyState_AddModule;
#[used]
static KEEP2_PYSTATE_FINDMODULE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyState_FindModule;
#[used]
static KEEP2_PYSTATE_REMOVEMODULE: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyState_RemoveModule;
#[used]
static KEEP2_PYTRACEMALLOC_TRACK: unsafe extern "C" fn(usize, usize, usize) -> i32 =
    super::PyTraceMalloc_Track;
#[used]
static KEEP2_PYTRACEMALLOC_UNTRACK: unsafe extern "C" fn(usize, usize) -> i32 =
    super::PyTraceMalloc_Untrack;
#[used]
static KEEP2_PY_ENTERRECURSIVECALL: unsafe extern "C" fn(*const c_char) -> i32 =
    super::Py_EnterRecursiveCall;
#[used]
static KEEP2_PY_LEAVERECURSIVECALL: unsafe extern "C" fn() = super::Py_LeaveRecursiveCall;
#[used]
static KEEP2_PY_ISINITIALIZED: unsafe extern "C" fn() -> i32 = super::Py_IsInitialized;
#[used]
static KEEP2_PY_GETCONSTANT: unsafe extern "C" fn(c_uint) -> *mut c_void = super::Py_GetConstant;
#[used]
static KEEP2_PY_GETCONSTANTBORROWED: unsafe extern "C" fn(c_uint) -> *mut c_void =
    super::Py_GetConstantBorrowed;
#[used]
static KEEP2_PY_IS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int = super::Py_Is;
#[used]
static KEEP2_PY_ISNONE: unsafe extern "C" fn(*mut c_void) -> c_int = super::Py_IsNone;
#[used]
static KEEP2_PY_ISTRUE: unsafe extern "C" fn(*mut c_void) -> c_int = super::Py_IsTrue;
#[used]
static KEEP2_PY_ISFALSE: unsafe extern "C" fn(*mut c_void) -> c_int = super::Py_IsFalse;
#[used]
static KEEP2_PY_NEWREF: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::Py_NewRef;
#[used]
static KEEP2_PY_XNEWREF: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::Py_XNewRef;
#[used]
static KEEP2_PY_REFCNT: unsafe extern "C" fn(*mut c_void) -> isize = super::Py_REFCNT;
#[used]
static KEEP2_PY_TYPE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::Py_TYPE;
#[used]
static KEEP2_PYVECTORCALL_NARGS: unsafe extern "C" fn(usize) -> usize = super::PyVectorcall_NARGS;
#[used]
static KEEP3_PYCONTEXTVAR_NEW: unsafe extern "C" fn(*const c_char, *mut c_void) -> *mut c_void =
    super::PyContextVar_New;
#[used]
static KEEP3_PYCONTEXTVAR_GET: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32 = super::PyContextVar_Get;
#[used]
static KEEP3_PYCONTEXTVAR_SET: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyContextVar_Set;
#[used]
static KEEP3_PYMETHOD_NEW: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyMethod_New;
#[used]
static KEEP3_PYINSTANCEMETHOD_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::cpython_object_call_api::PyInstanceMethod_New;
#[used]
static KEEP3_PYCODE_NEWEMPTY: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    c_int,
) -> *mut c_void = super::PyCode_NewEmpty;
#[used]
static KEEP3_PYUNSTABLE_CODE_NEW: unsafe extern "C" fn(
    c_int,
    c_int,
    c_int,
    c_int,
    c_int,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    c_int,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyUnstable_Code_New;
#[used]
static KEEP3_PYUNSTABLE_CODE_NEWWITHPOSONLYARGS: unsafe extern "C" fn(
    c_int,
    c_int,
    c_int,
    c_int,
    c_int,
    c_int,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    c_int,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyUnstable_Code_NewWithPosOnlyArgs;
#[used]
static KEEP3_PYOBJECT_CALLFUNCTION: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    ...
) -> *mut c_void = super::PyObject_CallFunction;
#[used]
static KEEP3_PYOBJECT_CALLFUNCTIONOBJARGS: unsafe extern "C" fn(*mut c_void, ...) -> *mut c_void =
    super::PyObject_CallFunctionObjArgs;
#[used]
static KEEP3_PYOBJECT_CALLMETHOD: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    ...
) -> *mut c_void = super::PyObject_CallMethod;
#[used]
static KEEP3_PYEVAL_CALLFUNCTION: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    ...
) -> *mut c_void = super::PyEval_CallFunction;
#[used]
static KEEP3_PYEVAL_CALLMETHOD: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    ...
) -> *mut c_void = super::PyEval_CallMethod;
#[used]
static KEEP3_PYEVAL_EVALFRAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyEval_EvalFrame;
#[used]
static KEEP3_PYEVAL_EVALFRAMEEX: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    super::PyEval_EvalFrameEx;
#[used]
static KEEP3_PYARG_PARSE: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> i32 =
    super::PyArg_Parse;
#[used]
static KEEP3__PYARG_PARSE_SIZET: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> i32 =
    super::_PyArg_Parse_SizeT;
#[used]
static KEEP3_PYARG_VAPARSE: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> i32 =
    super::PyArg_VaParse;
#[used]
static KEEP3__PYARG_VAPARSE_SIZET: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = super::_PyArg_VaParse_SizeT;
#[used]
static KEEP3_PYARG_VALIDATEKEYWORDARGUMENTS: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyArg_ValidateKeywordArguments;
#[used]
static KEEP3_PYARG_PARSETUPLE: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> i32 =
    super::PyArg_ParseTuple;
#[used]
static KEEP3__PYARG_PARSETUPLE_SIZET: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> i32 =
    super::_PyArg_ParseTuple_SizeT;
#[used]
static KEEP3_PYARG_PARSETUPLEANDKEYWORDS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
    ...
) -> i32 = super::PyArg_ParseTupleAndKeywords;
#[used]
static KEEP3__PYARG_PARSETUPLEANDKEYWORDS_SIZET: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
    ...
) -> i32 = super::_PyArg_ParseTupleAndKeywords_SizeT;
#[used]
static KEEP3_PYARG_VAPARSETUPLEANDKEYWORDS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
    *mut c_void,
) -> i32 = super::PyArg_VaParseTupleAndKeywords;
#[used]
static KEEP3__PYARG_VAPARSETUPLEANDKEYWORDS_SIZET: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
    *mut c_void,
) -> i32 = super::_PyArg_VaParseTupleAndKeywords_SizeT;
#[used]
static KEEP3_PYARG_UNPACKTUPLE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    isize,
    isize,
) -> i32 = super::PyArg_UnpackTuple;
#[used]
static KEEP3_PY_BUILDVALUE: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void =
    super::Py_BuildValue;
#[used]
static KEEP3__PY_BUILDVALUE_SIZET: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void =
    super::_Py_BuildValue_SizeT;
#[used]
static KEEP3_PY_VABUILDVALUE: unsafe extern "C" fn(*const c_char, *mut c_void) -> *mut c_void =
    super::Py_VaBuildValue;
#[used]
static KEEP3__PY_VABUILDVALUE_SIZET: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
) -> *mut c_void = super::_Py_VaBuildValue_SizeT;
#[used]
static KEEP3__PYOBJECT_CALLFUNCTION_SIZET: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    ...
) -> *mut c_void = super::_PyObject_CallFunction_SizeT;
#[used]
static KEEP3__PYOBJECT_CALLMETHOD_SIZET: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    ...
) -> *mut c_void = super::_PyObject_CallMethod_SizeT;
#[used]
static KEEP3_PYVECTORCALL_CALL: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyVectorcall_Call;
#[used]
static KEEP3_PYOBJECT_VECTORCALL: unsafe extern "C" fn(
    *mut c_void,
    *const *mut c_void,
    usize,
    *mut c_void,
) -> *mut c_void = super::PyObject_Vectorcall;
#[used]
static KEEP3_PYOBJECT_VECTORCALLDICT: unsafe extern "C" fn(
    *mut c_void,
    *const *mut c_void,
    usize,
    *mut c_void,
) -> *mut c_void = super::PyObject_VectorcallDict;
#[used]
static KEEP3_PYOBJECT_VECTORCALLMETHOD: unsafe extern "C" fn(
    *mut c_void,
    *const *mut c_void,
    usize,
    *mut c_void,
) -> *mut c_void = super::PyObject_VectorcallMethod;
#[used]
static KEEP3_PYMUTEX_LOCK: unsafe extern "C" fn(*mut c_void) = super::PyMutex_Lock;
#[used]
static KEEP3_PYMUTEX_UNLOCK: unsafe extern "C" fn(*mut c_void) = super::PyMutex_Unlock;
#[used]
static KEEP3_PYOS_SNPRINTF: unsafe extern "C" fn(*mut c_char, usize, *const c_char) -> i32 =
    super::PyOS_snprintf;
#[used]
static KEEP3_PYOS_STRING_TO_DOUBLE: unsafe extern "C" fn(
    *const c_char,
    *mut *mut c_char,
    *mut c_void,
) -> c_double = super::PyOS_string_to_double;
#[used]
static KEEP3_PYOS_STRTOL: unsafe extern "C" fn(*const c_char, *mut *mut c_char, i32) -> c_long =
    super::PyOS_strtol;
#[used]
static KEEP3_PYOS_STRTOUL: unsafe extern "C" fn(*const c_char, *mut *mut c_char, i32) -> c_ulong =
    super::PyOS_strtoul;
#[used]
static KEEP3_PYOS_BEFOREFORK: unsafe extern "C" fn() = super::PyOS_BeforeFork;
#[used]
static KEEP3_PYOS_AFTERFORK_PARENT: unsafe extern "C" fn() = super::PyOS_AfterFork_Parent;
#[used]
static KEEP3_PYOS_AFTERFORK_CHILD: unsafe extern "C" fn() = super::PyOS_AfterFork_Child;
#[used]
static KEEP3_PYOS_AFTERFORK: unsafe extern "C" fn() = super::PyOS_AfterFork;
#[used]
static KEEP3_PYOS_CHECKSTACK: unsafe extern "C" fn() -> c_int = super::PyOS_CheckStack;
#[used]
static KEEP3_PYOS_FSPATH: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyOS_FSPath;
#[used]
static KEEP3_PYOS_INTERRUPTOCCURRED: unsafe extern "C" fn() -> c_int =
    super::PyOS_InterruptOccurred;
#[used]
static KEEP3_PYOS_DOUBLE_TO_STRING: unsafe extern "C" fn(
    c_double,
    c_char,
    c_int,
    c_int,
    *mut c_int,
) -> *mut c_char = super::PyOS_double_to_string;
#[used]
static KEEP3_PYOS_GETSIG: unsafe extern "C" fn(c_int) -> *mut c_void = super::PyOS_getsig;
#[used]
static KEEP3_PYOS_SETSIG: unsafe extern "C" fn(c_int, *mut c_void) -> *mut c_void =
    super::PyOS_setsig;
#[used]
static KEEP3_PYOS_MYSTRICMP: unsafe extern "C" fn(*const c_char, *const c_char) -> c_int =
    super::PyOS_mystricmp;
#[used]
static KEEP3_PYOS_MYSTRNICMP: unsafe extern "C" fn(*const c_char, *const c_char, isize) -> c_int =
    super::PyOS_mystrnicmp;
#[used]
static KEEP3_PYOS_VSNPRINTF: unsafe extern "C" fn(
    *mut c_char,
    usize,
    *const c_char,
    *mut c_void,
) -> c_int = super::PyOS_vsnprintf;
#[used]
static KEEP3_PYERR_EXCEPTIONMATCHES: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyErr_ExceptionMatches;
#[used]
static KEEP3_PYERR_FETCH: unsafe extern "C" fn(
    *mut *mut c_void,
    *mut *mut c_void,
    *mut *mut c_void,
) = super::PyErr_Fetch;
#[used]
static KEEP3_PYERR_FORMAT: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> *mut c_void =
    super::PyErr_Format;
#[used]
static KEEP3_PYERR_FORMATV: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> *mut c_void = super::PyErr_FormatV;
#[used]
static KEEP3_PYSYS_WRITESTDOUT: unsafe extern "C" fn(*const c_char, ...) = super::PySys_WriteStdout;
#[used]
static KEEP3_PYSYS_WRITESTDERR: unsafe extern "C" fn(*const c_char, ...) = super::PySys_WriteStderr;
#[used]
static KEEP3_PYSYS_FORMATSTDOUT: unsafe extern "C" fn(*const c_char, ...) =
    super::PySys_FormatStdout;
#[used]
static KEEP3_PYSYS_FORMATSTDERR: unsafe extern "C" fn(*const c_char, ...) =
    super::PySys_FormatStderr;
#[used]
static KEEP3_PYSYS_AUDIT: unsafe extern "C" fn(*const c_char, *const c_char, ...) -> i32 =
    super::PySys_Audit;
#[used]
static KEEP3_PYCODEC_REGISTER: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyCodec_Register;
#[used]
static KEEP3_PYCODEC_UNREGISTER: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyCodec_Unregister;
#[used]
static KEEP3_PYCODEC_KNOWNENCODING: unsafe extern "C" fn(*const c_char) -> i32 =
    super::PyCodec_KnownEncoding;
#[used]
static KEEP3_PYCODEC_ENCODE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyCodec_Encode;
#[used]
static KEEP3_PYCODEC_DECODE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyCodec_Decode;
#[used]
static KEEP3_PYCODEC_ENCODER: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyCodec_Encoder;
#[used]
static KEEP3_PYCODEC_DECODER: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyCodec_Decoder;
#[used]
static KEEP3_PYCODEC_INCREMENTALENCODER: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyCodec_IncrementalEncoder;
#[used]
static KEEP3_PYCODEC_INCREMENTALDECODER: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyCodec_IncrementalDecoder;
#[used]
static KEEP3_PYCODEC_STREAMREADER: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyCodec_StreamReader;
#[used]
static KEEP3_PYCODEC_STREAMWRITER: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyCodec_StreamWriter;
#[used]
static KEEP3_PYCODEC_REGISTERERROR: unsafe extern "C" fn(*const c_char, *mut c_void) -> i32 =
    super::PyCodec_RegisterError;
#[used]
static KEEP3_PYCODEC_LOOKUPERROR: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyCodec_LookupError;
#[used]
static KEEP3_PYCODEC_STRICTERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyCodec_StrictErrors;
#[used]
static KEEP3_PYCODEC_IGNOREERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyCodec_IgnoreErrors;
#[used]
static KEEP3_PYCODEC_REPLACEERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyCodec_ReplaceErrors;
#[used]
static KEEP3_PYCODEC_XMLCHARREFREPLACEERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyCodec_XMLCharRefReplaceErrors;
#[used]
static KEEP3_PYCODEC_BACKSLASHREPLACEERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyCodec_BackslashReplaceErrors;
#[used]
static KEEP3_PYCODEC_NAMEREPLACEERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyCodec_NameReplaceErrors;
#[used]
static KEEP3_PYERR_GIVENEXCEPTIONMATCHES: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyErr_GivenExceptionMatches;
#[used]
static KEEP3_PYERR_NORMALIZEEXCEPTION: unsafe extern "C" fn(
    *mut *mut c_void,
    *mut *mut c_void,
    *mut *mut c_void,
) = super::PyErr_NormalizeException;
#[used]
static KEEP3_PYERR_PRINT: unsafe extern "C" fn() = super::PyErr_Print;
#[used]
static KEEP3_PYERR_RESTORE: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
    super::PyErr_Restore;
#[used]
static KEEP3_PYERR_SETFROMERRNO: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyErr_SetFromErrno;
#[used]
static KEEP3_PYERR_WARNEX: unsafe extern "C" fn(*mut c_void, *const c_char, isize) -> i32 =
    super::PyErr_WarnEx;
#[used]
static KEEP3_PYERR_WARNFORMAT: unsafe extern "C" fn(*mut c_void, isize, *const c_char) -> i32 =
    super::PyErr_WarnFormat;
#[used]
static KEEP3_PYERR_WARNEXPLICIT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    i32,
    *const c_char,
    *mut c_void,
) -> i32 = super::PyErr_WarnExplicit;
#[used]
static KEEP3_PYERR_RESOURCEWARNING: unsafe extern "C" fn(*mut c_void, isize, *const c_char) -> i32 =
    super::PyErr_ResourceWarning;
#[used]
static KEEP3_PYERR_WRITEUNRAISABLE: unsafe extern "C" fn(*mut c_void) =
    super::PyErr_WriteUnraisable;
#[used]
static KEEP3_PYTRACEBACK_HERE: unsafe extern "C" fn(*mut c_void) -> c_int = super::PyTraceBack_Here;
#[used]
static KEEP3_PYTRACEBACK_PRINT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
    super::PyTraceBack_Print;
#[used]
static KEEP3_PYEXCEPTION_GETTRACEBACK: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyException_GetTraceback;
#[used]
static KEEP3_PYEXCEPTION_GETCAUSE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyException_GetCause;
#[used]
static KEEP3_PYEXCEPTION_GETCONTEXT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyException_GetContext;
#[used]
static KEEP3_PYEXCEPTION_GETARGS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyException_GetArgs;
#[used]
static KEEP3_PYEXCEPTION_SETARGS: unsafe extern "C" fn(*mut c_void, *mut c_void) =
    super::PyException_SetArgs;
#[used]
static KEEP3_PYEXCEPTION_SETCAUSE: unsafe extern "C" fn(*mut c_void, *mut c_void) =
    super::PyException_SetCause;
#[used]
static KEEP3_PYEXCEPTION_SETCONTEXT: unsafe extern "C" fn(*mut c_void, *mut c_void) =
    super::PyException_SetContext;
#[used]
static KEEP3_PYEXCEPTION_SETTRACEBACK: unsafe extern "C" fn(*mut c_void, *mut c_void) =
    super::PyException_SetTraceback;
#[used]
static KEEP3_PYUNICODE_FROMSTRINGANDSIZE: unsafe extern "C" fn(
    *const c_char,
    isize,
) -> *mut c_void = super::PyUnicode_FromStringAndSize;
#[used]
static KEEP3_PYUNICODE_FROMWIDECHAR: unsafe extern "C" fn(*const Cwchar, isize) -> *mut c_void =
    super::PyUnicode_FromWideChar;
#[used]
static KEEP3_PYUNICODE_FROMENCODEDOBJECT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyUnicode_FromEncodedObject;
#[used]
static KEEP3_PYUNICODE_FROMKINDANDDATA: unsafe extern "C" fn(
    i32,
    *const c_void,
    isize,
) -> *mut c_void = super::PyUnicode_FromKindAndData;
#[used]
static KEEP3_PYUNICODE_ASUTF8: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    super::PyUnicode_AsUTF8;
#[used]
static KEEP3_PYUNICODE_ASUTF8ANDSIZE: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
) -> *const c_char = super::PyUnicode_AsUTF8AndSize;
#[used]
static KEEP3_PYUNICODE_ASUTF8STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_AsUTF8String;
#[used]
static KEEP3_PYUNICODE_ASASCIISTRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_AsASCIIString;
#[used]
static KEEP3_PYUNICODE_ASLATIN1STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_AsLatin1String;
#[used]
static KEEP3_PYUNICODE_ASMBCSSTRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_AsMBCSString;
#[used]
static KEEP3_PYUNICODE_ASCHARMAPSTRING: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyUnicode_AsCharmapString;
#[used]
static KEEP3_PYUNICODE_ASRAWUNICODEESCAPESTRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_AsRawUnicodeEscapeString;
#[used]
static KEEP3_PYUNICODE_ASUNICODEESCAPESTRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_AsUnicodeEscapeString;
#[used]
static KEEP3_PYUNICODE_ASUTF16STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_AsUTF16String;
#[used]
static KEEP3_PYUNICODE_ASUTF32STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_AsUTF32String;
#[used]
static KEEP3_PYUNICODE_ASENCODEDSTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyUnicode_AsEncodedString;
#[used]
static KEEP3_PYUNICODE_ASWIDECHARSTRING: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
) -> *mut Cwchar = super::PyUnicode_AsWideCharString;
#[used]
static KEEP3_PYUNICODE_ASWIDECHAR: unsafe extern "C" fn(*mut c_void, *mut Cwchar, isize) -> isize =
    super::PyUnicode_AsWideChar;
#[used]
static KEEP3_PYUNICODE_COMPARE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyUnicode_Compare;
#[used]
static KEEP3_PYUNICODE_COMPAREWITHASCIISTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> i32 = super::PyUnicode_CompareWithASCIIString;
#[used]
static KEEP3_PYUNICODE_CONCAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyUnicode_Concat;
#[used]
static KEEP3_PYUNICODE_CONTAINS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyUnicode_Contains;
#[used]
static KEEP3_PYUNICODE_FORMAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyUnicode_Format;
#[used]
static KEEP3_PYUNICODE_GETLENGTH: unsafe extern "C" fn(*mut c_void) -> isize =
    super::PyUnicode_GetLength;
#[used]
static KEEP3_PYUNICODE_INTERNFROMSTRING: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyUnicode_InternFromString;
#[used]
static KEEP3_PYUNICODE_NEW: unsafe extern "C" fn(isize, c_uint) -> *mut c_void =
    super::PyUnicode_New;
#[used]
static KEEP3_PYUNICODE_FROMFORMAT: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void =
    super::PyUnicode_FromFormat;
#[used]
static KEEP3_PYUNICODE_FROMFORMATV: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
) -> *mut c_void = super::PyUnicode_FromFormatV;
#[used]
static KEEP3_PYUNICODE_FROMOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_FromObject;
#[used]
static KEEP3_PYUNICODE_FROMORDINAL: unsafe extern "C" fn(c_int) -> *mut c_void =
    super::PyUnicode_FromOrdinal;
#[used]
static KEEP3_PYUNICODE_GETDEFAULTENCODING: unsafe extern "C" fn() -> *const c_char =
    super::PyUnicode_GetDefaultEncoding;
#[used]
static KEEP3_PYUNICODE_EQUAL: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
    super::PyUnicode_Equal;
#[used]
static KEEP3_PYUNICODE_EQUALTOUTF8: unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int =
    super::PyUnicode_EqualToUTF8;
#[used]
static KEEP3_PYUNICODE_EQUALTOUTF8ANDSIZE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    isize,
) -> c_int = super::PyUnicode_EqualToUTF8AndSize;
#[used]
static KEEP3_PYUNICODE_READCHAR: unsafe extern "C" fn(*mut c_void, isize) -> u32 =
    super::PyUnicode_ReadChar;
#[used]
static KEEP3_PYUNICODE_FIND: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
    isize,
    c_int,
) -> isize = super::PyUnicode_Find;
#[used]
static KEEP3_PYUNICODE_FINDCHAR: unsafe extern "C" fn(
    *mut c_void,
    u32,
    isize,
    isize,
    c_int,
) -> isize = super::PyUnicode_FindChar;
#[used]
static KEEP3_PYUNICODE_COUNT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
    isize,
) -> isize = super::PyUnicode_Count;
#[used]
static KEEP3_PYUNICODE_JOIN: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyUnicode_Join;
#[used]
static KEEP3_PYUNICODE_SPLIT: unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void =
    super::PyUnicode_Split;
#[used]
static KEEP3_PYUNICODE_RSPLIT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
) -> *mut c_void = super::PyUnicode_RSplit;
#[used]
static KEEP3_PYUNICODE_SPLITLINES: unsafe extern "C" fn(*mut c_void, c_int) -> *mut c_void =
    super::PyUnicode_Splitlines;
#[used]
static KEEP3_PYUNICODE_PARTITION: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyUnicode_Partition;
#[used]
static KEEP3_PYUNICODE_RPARTITION: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyUnicode_RPartition;
#[used]
static KEEP3_PYUNICODE_ISIDENTIFIER: unsafe extern "C" fn(*mut c_void) -> c_int =
    super::PyUnicode_IsIdentifier;
#[used]
static KEEP3_PYUNICODE_GETSIZE: unsafe extern "C" fn(*mut c_void) -> isize =
    super::PyUnicode_GetSize;
#[used]
static KEEP3_PYUNICODE_INTERNINPLACE: unsafe extern "C" fn(*mut *mut c_void) =
    super::PyUnicode_InternInPlace;
#[used]
static KEEP3_PYUNICODE_INTERNIMMORTAL: unsafe extern "C" fn(*mut *mut c_void) =
    super::PyUnicode_InternImmortal;
#[used]
static KEEP3_PYUNICODE_APPEND: unsafe extern "C" fn(*mut *mut c_void, *mut c_void) =
    super::PyUnicode_Append;
#[used]
static KEEP3_PYUNICODE_APPENDANDDEL: unsafe extern "C" fn(*mut *mut c_void, *mut c_void) =
    super::PyUnicode_AppendAndDel;
#[used]
static KEEP3_PYUNICODE_RICHCOMPARE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    c_int,
) -> *mut c_void = super::PyUnicode_RichCompare;
#[used]
static KEEP3_PYUNICODE_WRITECHAR: unsafe extern "C" fn(*mut c_void, isize, c_uint) -> c_int =
    super::PyUnicode_WriteChar;
#[used]
static KEEP3_PYUNICODE_COPYCHARACTERS: unsafe extern "C" fn(
    *mut c_void,
    isize,
    *mut c_void,
    isize,
    isize,
) -> isize = super::PyUnicode_CopyCharacters;
#[used]
static KEEP3_PYUNICODE_TRANSLATE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyUnicode_Translate;
#[used]
static KEEP3_PYUNICODE_RESIZE: unsafe extern "C" fn(*mut *mut c_void, isize) -> c_int =
    super::PyUnicode_Resize;
#[used]
static KEEP3_PYUNICODE_REPLACE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    isize,
) -> *mut c_void = super::PyUnicode_Replace;
#[used]
static KEEP3_PYUNICODE_SUBSTRING: unsafe extern "C" fn(*mut c_void, isize, isize) -> *mut c_void =
    super::PyUnicode_Substring;
#[used]
static KEEP3_PYUNICODE_TAILMATCH: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
    isize,
    i32,
) -> isize = super::PyUnicode_Tailmatch;
#[used]
static KEEP3_PYUNICODE_ASUCS4: unsafe extern "C" fn(*mut c_void, *mut u32, isize, i32) -> *mut u32 =
    super::PyUnicode_AsUCS4;
#[used]
static KEEP3_PYUNICODE_ASUCS4COPY: unsafe extern "C" fn(*mut c_void) -> *mut u32 =
    super::PyUnicode_AsUCS4Copy;
#[used]
static KEEP3_PYUNICODE_DECODE: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyUnicode_Decode;
#[used]
static KEEP3_PYUNICODE_DECODEASCII: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = super::PyUnicode_DecodeASCII;
#[used]
static KEEP3_PYUNICODE_DECODELATIN1: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = super::PyUnicode_DecodeLatin1;
#[used]
static KEEP3_PYUNICODE_DECODEUTF8: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = super::PyUnicode_DecodeUTF8;
#[used]
static KEEP3_PYUNICODE_DECODEUTF8STATEFUL: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut isize,
) -> *mut c_void = super::PyUnicode_DecodeUTF8Stateful;
#[used]
static KEEP3_PYUNICODE_DECODEUTF7: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = super::PyUnicode_DecodeUTF7;
#[used]
static KEEP3_PYUNICODE_DECODEUTF7STATEFUL: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut isize,
) -> *mut c_void = super::PyUnicode_DecodeUTF7Stateful;
#[used]
static KEEP3_PYUNICODE_DECODEMBCS: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = super::PyUnicode_DecodeMBCS;
#[used]
static KEEP3_PYUNICODE_DECODEMBCSSTATEFUL: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut isize,
) -> *mut c_void = super::PyUnicode_DecodeMBCSStateful;
#[used]
static KEEP3_PYUNICODE_DECODECODEPAGESTATEFUL: unsafe extern "C" fn(
    c_int,
    *const c_char,
    isize,
    *const c_char,
    *mut isize,
) -> *mut c_void = super::PyUnicode_DecodeCodePageStateful;
#[used]
static KEEP3_PYUNICODE_DECODECHARMAP: unsafe extern "C" fn(
    *const c_char,
    isize,
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyUnicode_DecodeCharmap;
#[used]
static KEEP3_PYUNICODE_BUILDENCODINGMAP: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_BuildEncodingMap;
#[used]
static KEEP3_PYUNICODE_DECODERAWUNICODEESCAPE: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = super::PyUnicode_DecodeRawUnicodeEscape;
#[used]
static KEEP3_PYUNICODE_DECODEUNICODEESCAPE: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = super::PyUnicode_DecodeUnicodeEscape;
#[used]
static KEEP3_PYUNICODE_DECODEUTF16: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut c_int,
) -> *mut c_void = super::PyUnicode_DecodeUTF16;
#[used]
static KEEP3_PYUNICODE_DECODEUTF16STATEFUL: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut c_int,
    *mut isize,
) -> *mut c_void = super::PyUnicode_DecodeUTF16Stateful;
#[used]
static KEEP3_PYUNICODE_DECODEUTF32: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut c_int,
) -> *mut c_void = super::PyUnicode_DecodeUTF32;
#[used]
static KEEP3_PYUNICODE_DECODEUTF32STATEFUL: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut c_int,
    *mut isize,
) -> *mut c_void = super::PyUnicode_DecodeUTF32Stateful;
#[used]
static KEEP3_PYUNICODE_DECODEFSDEFAULT: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyUnicode_DecodeFSDefault;
#[used]
static KEEP3_PYUNICODE_DECODEFSDEFAULTANDSIZE: unsafe extern "C" fn(
    *const c_char,
    isize,
) -> *mut c_void = super::PyUnicode_DecodeFSDefaultAndSize;
#[used]
static KEEP3_PYUNICODE_DECODELOCALE: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyUnicode_DecodeLocale;
#[used]
static KEEP3_PYUNICODE_DECODELOCALEANDSIZE: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = super::PyUnicode_DecodeLocaleAndSize;
#[used]
static KEEP3_PYUNICODE_ENCODEFSDEFAULT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicode_EncodeFSDefault;
#[used]
static KEEP3_PYUNICODE_ENCODELOCALE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyUnicode_EncodeLocale;
#[used]
static KEEP3_PYUNICODE_ENCODECODEPAGE: unsafe extern "C" fn(
    c_int,
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyUnicode_EncodeCodePage;
#[used]
static KEEP3_PYUNICODE_ASDECODEDOBJECT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyUnicode_AsDecodedObject;
#[used]
static KEEP3_PYUNICODE_ASDECODEDUNICODE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyUnicode_AsDecodedUnicode;
#[used]
static KEEP3_PYUNICODE_ASENCODEDOBJECT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyUnicode_AsEncodedObject;
#[used]
static KEEP3_PYUNICODE_ASENCODEDUNICODE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyUnicode_AsEncodedUnicode;
#[used]
static KEEP3_PYUNICODE_FSCONVERTER: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
    super::PyUnicode_FSConverter;
#[used]
static KEEP3_PYUNICODE_FSDECODER: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
    super::PyUnicode_FSDecoder;
#[used]
static KEEP3_PYUNICODEDECODEERROR_CREATE: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    isize,
    isize,
    isize,
    *const c_char,
) -> *mut c_void = super::PyUnicodeDecodeError_Create;
#[used]
static KEEP3_PYUNICODEENCODEERROR_GETENCODING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicodeEncodeError_GetEncoding;
#[used]
static KEEP3_PYUNICODEDECODEERROR_GETENCODING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicodeDecodeError_GetEncoding;
#[used]
static KEEP3_PYUNICODEENCODEERROR_GETOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicodeEncodeError_GetObject;
#[used]
static KEEP3_PYUNICODEDECODEERROR_GETOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicodeDecodeError_GetObject;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_GETOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicodeTranslateError_GetObject;
#[used]
static KEEP3_PYUNICODEENCODEERROR_GETSTART: unsafe extern "C" fn(*mut c_void, *mut isize) -> c_int =
    super::PyUnicodeEncodeError_GetStart;
#[used]
static KEEP3_PYUNICODEDECODEERROR_GETSTART: unsafe extern "C" fn(*mut c_void, *mut isize) -> c_int =
    super::PyUnicodeDecodeError_GetStart;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_GETSTART: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
) -> c_int = super::PyUnicodeTranslateError_GetStart;
#[used]
static KEEP3_PYUNICODEENCODEERROR_SETSTART: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    super::PyUnicodeEncodeError_SetStart;
#[used]
static KEEP3_PYUNICODEDECODEERROR_SETSTART: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    super::PyUnicodeDecodeError_SetStart;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_SETSTART: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    super::PyUnicodeTranslateError_SetStart;
#[used]
static KEEP3_PYUNICODEENCODEERROR_GETEND: unsafe extern "C" fn(*mut c_void, *mut isize) -> c_int =
    super::PyUnicodeEncodeError_GetEnd;
#[used]
static KEEP3_PYUNICODEDECODEERROR_GETEND: unsafe extern "C" fn(*mut c_void, *mut isize) -> c_int =
    super::PyUnicodeDecodeError_GetEnd;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_GETEND: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
) -> c_int = super::PyUnicodeTranslateError_GetEnd;
#[used]
static KEEP3_PYUNICODEENCODEERROR_SETEND: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    super::PyUnicodeEncodeError_SetEnd;
#[used]
static KEEP3_PYUNICODEDECODEERROR_SETEND: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    super::PyUnicodeDecodeError_SetEnd;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_SETEND: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    super::PyUnicodeTranslateError_SetEnd;
#[used]
static KEEP3_PYUNICODEENCODEERROR_GETREASON: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicodeEncodeError_GetReason;
#[used]
static KEEP3_PYUNICODEDECODEERROR_GETREASON: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicodeDecodeError_GetReason;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_GETREASON: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyUnicodeTranslateError_GetReason;
#[used]
static KEEP3_PYUNICODEENCODEERROR_SETREASON: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> c_int = super::PyUnicodeEncodeError_SetReason;
#[used]
static KEEP3_PYUNICODEDECODEERROR_SETREASON: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> c_int = super::PyUnicodeDecodeError_SetReason;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_SETREASON: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> c_int = super::PyUnicodeTranslateError_SetReason;
#[used]
static KEEP3_PYUNSTABLE_OBJECT_ISUNIQUELYREFERENCED: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyUnstable_Object_IsUniquelyReferenced;
#[used]
static KEEP3_PYUNSTABLE_OBJECT_ISUNIQUEREFERENCEDTEMPORARY: unsafe extern "C" fn(
    *mut c_void,
) -> i32 = super::PyUnstable_Object_IsUniqueReferencedTemporary;
#[used]
static KEEP3_PY_GENERICALIAS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::Py_GenericAlias;
#[used]
static KEEP4__PYOBJECT_NEW: unsafe extern "C" fn(*mut CpythonTypeObject) -> *mut c_void =
    super::_PyObject_New;
#[used]
static KEEP4__PYOBJECT_NEWVAR: unsafe extern "C" fn(*mut CpythonTypeObject, isize) -> *mut c_void =
    super::_PyObject_NewVar;
#[used]
static KEEP4__PYOBJECT_GC_NEW: unsafe extern "C" fn(*mut CpythonTypeObject) -> *mut c_void =
    super::_PyObject_GC_New;
#[used]
static KEEP4__PY_DEALLOC: unsafe extern "C" fn(*mut c_void) = super::_Py_Dealloc;
#[used]
static KEEP4__PYERR_BADINTERNALCALL: unsafe extern "C" fn(*const c_char, i32) =
    super::_PyErr_BadInternalCall;
#[used]
static KEEP4__PY_HASHDOUBLE: unsafe extern "C" fn(*mut c_void, f64) -> isize =
    super::_Py_HashDouble;
#[used]
static KEEP4__PYUNICODE_ISWHITESPACE: unsafe extern "C" fn(u32) -> i32 =
    super::_PyUnicode_IsWhitespace;
#[used]
static KEEP4__PYUNICODE_ISALPHA: unsafe extern "C" fn(u32) -> i32 = super::_PyUnicode_IsAlpha;
#[used]
static KEEP4__PYUNICODE_ISDECIMALDIGIT: unsafe extern "C" fn(u32) -> i32 =
    super::_PyUnicode_IsDecimalDigit;
#[used]
static KEEP4__PYUNICODE_ISDIGIT: unsafe extern "C" fn(u32) -> i32 = super::_PyUnicode_IsDigit;
#[used]
static KEEP4__PYUNICODE_ISNUMERIC: unsafe extern "C" fn(u32) -> i32 = super::_PyUnicode_IsNumeric;
#[used]
static KEEP4__PYUNICODE_ISLOWERCASE: unsafe extern "C" fn(u32) -> i32 =
    super::_PyUnicode_IsLowercase;
#[used]
static KEEP4__PYUNICODE_ISUPPERCASE: unsafe extern "C" fn(u32) -> i32 =
    super::_PyUnicode_IsUppercase;
#[used]
static KEEP4__PYUNICODE_ISTITLECASE: unsafe extern "C" fn(u32) -> i32 =
    super::_PyUnicode_IsTitlecase;

#[used]
static KEEP_PYMODULEDEF_INIT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyModuleDef_Init;
#[used]
static KEEP_PYMODULE_CREATE2: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    super::PyModule_Create2;
#[used]
static KEEP_PYMODULE_FROM_DEF_AND_SPEC2: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    i32,
) -> *mut c_void = super::PyModule_FromDefAndSpec2;
#[used]
static KEEP_PYMODULE_EXEC_DEF: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyModule_ExecDef;
#[used]
static KEEP_PYMODULE_GET_DEF: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyModule_GetDef;
#[used]
static KEEP_PYMODULE_GET_STATE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyModule_GetState;
#[used]
static KEEP_PYMODULE_ADD_OBJECT_REF: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = super::PyModule_AddObjectRef;
#[used]
static KEEP_PYMODULE_ADD_OBJECT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = super::PyModule_AddObject;
#[used]
static KEEP_PYMODULE_ADD_INT_CONSTANT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    i64,
) -> i32 = super::PyModule_AddIntConstant;
#[used]
static KEEP_PYMODULE_ADD_STRING_CONSTANT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> i32 = super::PyModule_AddStringConstant;
#[used]
static KEEP_PYLONG_FROM_LONG: unsafe extern "C" fn(i64) -> *mut c_void = super::PyLong_FromLong;
#[used]
static KEEP_PYLONG_FROM_LONGLONG: unsafe extern "C" fn(i64) -> *mut c_void =
    super::PyLong_FromLongLong;
#[used]
static KEEP__PYLONG_COPY: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::_PyLong_Copy;
#[used]
static KEEP_PYBOOL_FROM_LONG: unsafe extern "C" fn(i64) -> *mut c_void = super::PyBool_FromLong;
#[used]
static KEEP_PYFLOAT_FROM_DOUBLE: unsafe extern "C" fn(f64) -> *mut c_void =
    super::PyFloat_FromDouble;
#[used]
static KEEP_PYUNICODE_FROM_STRING: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyUnicode_FromString;
#[used]
static KEEP_PYBYTES_FROM_STRING_AND_SIZE: unsafe extern "C" fn(
    *const c_char,
    isize,
) -> *mut c_void = super::PyBytes_FromStringAndSize;
#[used]
static KEEP_PYBYTES_FROM_OBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyBytes_FromObject;
#[used]
static KEEP_PYBYTES_FROM_FORMAT: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void =
    super::PyBytes_FromFormat;
#[used]
static KEEP_PYBYTES_FROM_FORMATV: unsafe extern "C" fn(*const c_char, *mut c_void) -> *mut c_void =
    super::PyBytes_FromFormatV;
#[used]
static KEEP_PYBYTES_CONCAT: unsafe extern "C" fn(*mut *mut c_void, *mut c_void) =
    super::PyBytes_Concat;
#[used]
static KEEP_PYBYTES_CONCAT_AND_DEL: unsafe extern "C" fn(*mut *mut c_void, *mut c_void) =
    super::PyBytes_ConcatAndDel;
#[used]
static KEEP_PYERR_SET_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) =
    super::PyErr_SetString;
#[used]
static KEEP_PYERR_NEW_EXCEPTION: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyErr_NewException;
#[used]
static KEEP_PYERR_NEW_EXCEPTION_WITH_DOC: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyErr_NewExceptionWithDoc;
#[used]
static KEEP_PYEXCEPTIONCLASS_NAME: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    super::PyExceptionClass_Name;
#[used]
static KEEP_PYERR_OCCURRED: unsafe extern "C" fn() -> *mut c_void = super::PyErr_Occurred;
#[used]
static KEEP_PYERR_CLEAR: unsafe extern "C" fn() = super::PyErr_Clear;
#[used]
static KEEP_PYERR_BAD_ARGUMENT: unsafe extern "C" fn() -> i32 = super::PyErr_BadArgument;
#[used]
static KEEP_PYERR_BAD_INTERNAL_CALL: unsafe extern "C" fn() = super::PyErr_BadInternalCall;
#[used]
static KEEP_PYERR_PRINT_EX: unsafe extern "C" fn(i32) = super::PyErr_PrintEx;
#[used]
static KEEP_PYERR_DISPLAY: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
    super::PyErr_Display;
#[used]
static KEEP_PYERR_DISPLAY_EXCEPTION: unsafe extern "C" fn(*mut c_void) =
    super::PyErr_DisplayException;
#[used]
static KEEP_PYERR_GET_RAISED_EXCEPTION: unsafe extern "C" fn() -> *mut c_void =
    super::PyErr_GetRaisedException;
#[used]
static KEEP_PYERR_SET_RAISED_EXCEPTION: unsafe extern "C" fn(*mut c_void) =
    super::PyErr_SetRaisedException;
#[used]
static KEEP_PYERR_GET_HANDLED_EXCEPTION: unsafe extern "C" fn() -> *mut c_void =
    super::PyErr_GetHandledException;
#[used]
static KEEP_PYERR_SET_HANDLED_EXCEPTION: unsafe extern "C" fn(*mut c_void) =
    super::PyErr_SetHandledException;
#[used]
static KEEP_PYERR_GET_EXCINFO: unsafe extern "C" fn(
    *mut *mut c_void,
    *mut *mut c_void,
    *mut *mut c_void,
) = super::PyErr_GetExcInfo;
#[used]
static KEEP_PYERR_SET_EXCINFO: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
    super::PyErr_SetExcInfo;
#[used]
static KEEP_PYERR_SET_FROM_ERRNO: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyErr_SetFromErrno;
#[used]
static KEEP_PYERR_SET_FROM_ERRNO_WITH_FILENAME: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyErr_SetFromErrnoWithFilename;
#[used]
static KEEP_PYERR_SET_FROM_ERRNO_WITH_FILENAME_OBJECT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyErr_SetFromErrnoWithFilenameObject;
#[used]
static KEEP_PYERR_SET_FROM_ERRNO_WITH_FILENAME_OBJECTS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyErr_SetFromErrnoWithFilenameObjects;
#[used]
static KEEP_PYERR_SET_EXC_FROM_WINDOWS_ERR: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    super::PyErr_SetExcFromWindowsErr;
#[used]
static KEEP_PYERR_SET_EXC_FROM_WINDOWS_ERR_WITH_FILENAME: unsafe extern "C" fn(
    *mut c_void,
    i32,
    *const c_char,
) -> *mut c_void = super::PyErr_SetExcFromWindowsErrWithFilename;
#[used]
static KEEP_PYERR_SET_EXC_FROM_WINDOWS_ERR_WITH_FILENAME_OBJECT:
    unsafe extern "C" fn(*mut c_void, i32, *mut c_void) -> *mut c_void =
    super::PyErr_SetExcFromWindowsErrWithFilenameObject;
#[used]
static KEEP_PYERR_SET_EXC_FROM_WINDOWS_ERR_WITH_FILENAME_OBJECTS:
    unsafe extern "C" fn(*mut c_void, i32, *mut c_void, *mut c_void) -> *mut c_void =
    super::PyErr_SetExcFromWindowsErrWithFilenameObjects;
#[used]
static KEEP_PYERR_SET_FROM_WINDOWS_ERR: unsafe extern "C" fn(i32) -> *mut c_void =
    super::PyErr_SetFromWindowsErr;
#[used]
static KEEP_PYERR_SET_FROM_WINDOWS_ERR_WITH_FILENAME: unsafe extern "C" fn(
    i32,
    *const c_char,
) -> *mut c_void = super::PyErr_SetFromWindowsErrWithFilename;
#[used]
static KEEP_PYERR_SET_INTERRUPT: unsafe extern "C" fn() = super::PyErr_SetInterrupt;
#[used]
static KEEP_PYERR_SET_INTERRUPT_EX: unsafe extern "C" fn(i32) -> i32 = super::PyErr_SetInterruptEx;
#[used]
static KEEP_PYERR_SYNTAX_LOCATION: unsafe extern "C" fn(*const c_char, i32) =
    super::PyErr_SyntaxLocation;
#[used]
static KEEP_PYERR_SYNTAX_LOCATION_EX: unsafe extern "C" fn(*const c_char, i32, i32) =
    super::PyErr_SyntaxLocationEx;
#[used]
static KEEP_PYERR_PROGRAM_TEXT: unsafe extern "C" fn(*const c_char, i32) -> *mut c_void =
    super::PyErr_ProgramText;
#[used]
static KEEP_PYERR_SET_IMPORT_ERROR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyErr_SetImportError;
#[used]
static KEEP_PYERR_SET_IMPORT_ERROR_SUBCLASS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyErr_SetImportErrorSubclass;
#[used]
static KEEP_PYFILE_FROM_FD: unsafe extern "C" fn(
    i32,
    *const c_char,
    *const c_char,
    i32,
    *const c_char,
    *const c_char,
    *const c_char,
    i32,
) -> *mut c_void = super::PyFile_FromFd;
#[used]
static KEEP_PYFILE_GET_LINE: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    super::PyFile_GetLine;
#[used]
static KEEP_PYFILE_WRITE_OBJECT: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 =
    super::PyFile_WriteObject;
#[used]
static KEEP_PYFILE_WRITE_STRING: unsafe extern "C" fn(*const c_char, *mut c_void) -> i32 =
    super::PyFile_WriteString;
#[used]
static KEEP_PY_INCREF: unsafe extern "C" fn(*mut c_void) = super::Py_IncRef;
#[used]
static KEEP_PY_DECREF: unsafe extern "C" fn(*mut c_void) = super::Py_DecRef;
#[used]
static KEEP_PY__INCREF: unsafe extern "C" fn(*mut c_void) = super::_Py_IncRef;
#[used]
static KEEP_PY__DECREF: unsafe extern "C" fn(*mut c_void) = super::_Py_DecRef;
#[used]
static KEEP_PY__SETREFCNT: unsafe extern "C" fn(*mut c_void, isize) = super::_Py_SetRefcnt;
#[used]
static KEEP_PY__NEGATIVEREFCOUNT: unsafe extern "C" fn(*const c_char, c_int, *mut c_void) =
    super::_Py_NegativeRefcount;
#[used]
static KEEP_PY__CHECKRECURSIVECALL: unsafe extern "C" fn(*const c_char) -> c_int =
    super::_Py_CheckRecursiveCall;
#[used]
static KEEP_PY__OBJECT_GC_NEWVAR: unsafe extern "C" fn(
    *mut CpythonTypeObject,
    isize,
) -> *mut c_void = super::_PyObject_GC_NewVar;
#[used]
static KEEP_PY__OBJECT_GC_RESIZE: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    super::_PyObject_GC_Resize;
#[used]
static KEEP_PY_XINCREF: unsafe extern "C" fn(*mut c_void) = super::Py_XIncRef;
#[used]
static KEEP_PY_XDECREF: unsafe extern "C" fn(*mut c_void) = super::Py_XDecRef;
#[used]
static KEEP_PYBYTES_FROM_STRING: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyBytes_FromString;
#[used]
static KEEP_PYBYTES_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = super::PyBytes_Size;
#[used]
static KEEP_PYBYTES_AS_STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_char =
    super::PyBytes_AsString;
#[used]
static KEEP_PYBYTES_AS_STRING_AND_SIZE: unsafe extern "C" fn(
    *mut c_void,
    *mut *mut c_char,
    *mut isize,
) -> i32 = super::PyBytes_AsStringAndSize;
#[used]
static KEEP_PYBYTES_REPR: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    super::PyBytes_Repr;
#[used]
static KEEP_PYBYTES_DECODE_ESCAPE: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = super::PyBytes_DecodeEscape;
#[used]
static KEEP_PYBYTES_JOIN: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyBytes_Join;
#[used]
static KEEP_PY_BYTES_JOIN_PRIVATE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::_PyBytes_Join;
#[used]
static KEEP_PYBYTES_RESIZE: unsafe extern "C" fn(*mut *mut c_void, isize) -> i32 =
    super::PyBytes_Resize;
#[used]
static KEEP_PY_BYTES_RESIZE_PRIVATE: unsafe extern "C" fn(*mut *mut c_void, isize) -> i32 =
    super::_PyBytes_Resize;
#[used]
static KEEP_PYBYTEARRAY_FROM_STRING_AND_SIZE: unsafe extern "C" fn(
    *const c_char,
    isize,
) -> *mut c_void = super::PyByteArray_FromStringAndSize;
#[used]
static KEEP_PYBYTEARRAY_FROM_OBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyByteArray_FromObject;
#[used]
static KEEP_PYBYTEARRAY_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = super::PyByteArray_Size;
#[used]
static KEEP_PYBYTEARRAY_AS_STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_char =
    super::PyByteArray_AsString;
#[used]
static KEEP_PYBYTEARRAY_RESIZE: unsafe extern "C" fn(*mut c_void, isize) -> i32 =
    super::PyByteArray_Resize;
#[used]
static KEEP_PYBYTEARRAY_CONCAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyByteArray_Concat;
#[used]
static KEEP_PYBUFFER_RELEASE: unsafe extern "C" fn(*mut c_void) = super::PyBuffer_Release;
#[used]
static KEEP_PYCALLABLE_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyCallable_Check;
#[used]
static KEEP_PYINDEX_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyIndex_Check;
#[used]
static KEEP_PYFLOAT_AS_DOUBLE: unsafe extern "C" fn(*mut c_void) -> f64 = super::PyFloat_AsDouble;
#[used]
static KEEP_PYFLOAT_GET_MAX: unsafe extern "C" fn() -> f64 = super::PyFloat_GetMax;
#[used]
static KEEP_PYFLOAT_GET_MIN: unsafe extern "C" fn() -> f64 = super::PyFloat_GetMin;
#[used]
static KEEP_PYFLOAT_GET_INFO: unsafe extern "C" fn() -> *mut c_void = super::PyFloat_GetInfo;
#[used]
static KEEP_PYFLOAT_FROM_STRING: unsafe extern "C" fn(
    *mut c_void,
    *mut *mut c_char,
) -> *mut c_void = super::PyFloat_FromString;
#[used]
static KEEP_PYLONG_AS_LONG: unsafe extern "C" fn(*mut c_void) -> i64 = super::PyLong_AsLong;
#[used]
static KEEP_PYLONG_AS_LONGLONG: unsafe extern "C" fn(*mut c_void) -> i64 = super::PyLong_AsLongLong;
#[used]
static KEEP_PYLONG_AS_INT: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyLong_AsInt;
#[used]
static KEEP_PYLONG_AS_INT32: unsafe extern "C" fn(*mut c_void, *mut i32) -> i32 =
    super::PyLong_AsInt32;
#[used]
static KEEP_PYLONG_AS_INT64: unsafe extern "C" fn(*mut c_void, *mut i64) -> i32 =
    super::PyLong_AsInt64;
#[used]
static KEEP_PYLONG_AS_UINT32: unsafe extern "C" fn(*mut c_void, *mut u32) -> i32 =
    super::PyLong_AsUInt32;
#[used]
static KEEP_PYLONG_AS_UINT64: unsafe extern "C" fn(*mut c_void, *mut u64) -> i32 =
    super::PyLong_AsUInt64;
#[used]
static KEEP_PYLONG_AS_SSIZE_T: unsafe extern "C" fn(*mut c_void) -> isize = super::PyLong_AsSsize_t;
#[used]
static KEEP_PYLONG_AS_SIZE_T: unsafe extern "C" fn(*mut c_void) -> usize = super::PyLong_AsSize_t;
#[used]
static KEEP_PYLONG_AS_UNSIGNED_LONG: unsafe extern "C" fn(*mut c_void) -> u64 =
    super::PyLong_AsUnsignedLong;
#[used]
static KEEP_PYLONG_AS_UNSIGNED_LONGLONG: unsafe extern "C" fn(*mut c_void) -> u64 =
    super::PyLong_AsUnsignedLongLong;
#[used]
static KEEP_PYLONG_AS_UNSIGNED_LONG_MASK: unsafe extern "C" fn(*mut c_void) -> u64 =
    super::PyLong_AsUnsignedLongMask;
#[used]
static KEEP_PYLONG_AS_UNSIGNED_LONGLONG_MASK: unsafe extern "C" fn(*mut c_void) -> u64 =
    super::PyLong_AsUnsignedLongLongMask;
#[used]
static KEEP_PYLONG_AS_NATIVE_BYTES: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
    i32,
) -> isize = super::PyLong_AsNativeBytes;
#[used]
static KEEP_PYLONG_FROM_NATIVE_BYTES: unsafe extern "C" fn(
    *const c_void,
    usize,
    i32,
) -> *mut c_void = super::PyLong_FromNativeBytes;
#[used]
static KEEP_PYLONG_FROM_UNSIGNED_NATIVE_BYTES: unsafe extern "C" fn(
    *const c_void,
    usize,
    i32,
) -> *mut c_void = super::PyLong_FromUnsignedNativeBytes;
#[used]
static KEEP_PYLONG_AS_VOID_PTR: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyLong_AsVoidPtr;
#[used]
static KEEP_PYLONG_AS_LONG_AND_OVERFLOW: unsafe extern "C" fn(*mut c_void, *mut i32) -> i64 =
    super::PyLong_AsLongAndOverflow;
#[used]
static KEEP_PYLONG_AS_LONGLONG_AND_OVERFLOW: unsafe extern "C" fn(*mut c_void, *mut i32) -> i64 =
    super::PyLong_AsLongLongAndOverflow;
#[used]
static KEEP_PYLONG_AS_DOUBLE: unsafe extern "C" fn(*mut c_void) -> f64 = super::PyLong_AsDouble;
#[used]
static KEEP_PYLONG_FROM_DOUBLE: unsafe extern "C" fn(f64) -> *mut c_void = super::PyLong_FromDouble;
#[used]
static KEEP_PYCOMPLEX_FROM_DOUBLES: unsafe extern "C" fn(f64, f64) -> *mut c_void =
    super::PyComplex_FromDoubles;
#[used]
static KEEP_PYCOMPLEX_FROM_CCOMPLEX: unsafe extern "C" fn(CpythonComplexValue) -> *mut c_void =
    super::PyComplex_FromCComplex;
#[used]
static KEEP_PYCOMPLEX_AS_CCOMPLEX: unsafe extern "C" fn(*mut c_void) -> CpythonComplexValue =
    super::PyComplex_AsCComplex;
#[used]
static KEEP_PYCOMPLEX_REAL_AS_DOUBLE: unsafe extern "C" fn(*mut c_void) -> f64 =
    super::PyComplex_RealAsDouble;
#[used]
static KEEP_PYCOMPLEX_IMAG_AS_DOUBLE: unsafe extern "C" fn(*mut c_void) -> f64 =
    super::PyComplex_ImagAsDouble;
#[used]
static KEEP_PYIMPORT_GET_MAGIC_NUMBER: unsafe extern "C" fn() -> c_long =
    super::PyImport_GetMagicNumber;
#[used]
static KEEP_PYIMPORT_GET_MAGIC_TAG: unsafe extern "C" fn() -> *const c_char =
    super::PyImport_GetMagicTag;
#[used]
static KEEP_PYIMPORT_IMPORT_MODULE: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyImport_ImportModule;
#[used]
static KEEP_PYIMPORT_IMPORT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyImport_Import;
#[used]
static KEEP_PYIMPORT_GET_MODULE_DICT: unsafe extern "C" fn() -> *mut c_void =
    super::PyImport_GetModuleDict;
#[used]
static KEEP_PYIMPORT_ADD_MODULE_REF: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyImport_AddModuleRef;
#[used]
static KEEP_PYIMPORT_ADD_MODULE_OBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyImport_AddModuleObject;
#[used]
static KEEP_PYIMPORT_ADD_MODULE: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyImport_AddModule;
#[used]
static KEEP_PYIMPORT_APPEND_INITTAB: unsafe extern "C" fn(
    *const c_char,
    Option<CpythonInittabInitFunc>,
) -> i32 = super::PyImport_AppendInittab;
#[used]
static KEEP_PYIMPORT_IMPORT_FROZEN_MODULE: unsafe extern "C" fn(*const c_char) -> i32 =
    super::PyImport_ImportFrozenModule;
#[used]
static KEEP_PYIMPORT_IMPORT_FROZEN_MODULE_OBJECT: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyImport_ImportFrozenModuleObject;
#[used]
static KEEP_PYIMPORT_EXECCODEMODULE: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
) -> *mut c_void = super::PyImport_ExecCodeModule;
#[used]
static KEEP_PYIMPORT_EXECCODEMODULE_EX: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyImport_ExecCodeModuleEx;
#[used]
static KEEP_PYIMPORT_EXECCODEMODULE_OBJECT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyImport_ExecCodeModuleObject;
#[used]
static KEEP_PYIMPORT_EXECCODEMODULE_WITH_PATHNAMES: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = super::PyImport_ExecCodeModuleWithPathnames;
#[used]
static KEEP_PYIMPORT_GET_MODULE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyImport_GetModule;
#[used]
static KEEP_PYIMPORT_IMPORT_MODULE_NO_BLOCK: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    super::PyImport_ImportModuleNoBlock;
#[used]
static KEEP_PYIMPORT_GET_IMPORTER: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyImport_GetImporter;
#[used]
static KEEP_PYIMPORT_IMPORT_MODULE_LEVEL_OBJECT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    i32,
) -> *mut c_void = super::PyImport_ImportModuleLevelObject;
#[used]
static KEEP_PYIMPORT_IMPORT_MODULE_LEVEL: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    i32,
) -> *mut c_void = super::PyImport_ImportModuleLevel;
#[used]
static KEEP_PYIMPORT_RELOAD_MODULE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyImport_ReloadModule;
#[used]
static KEEP_PYEVAL_GET_FRAME: unsafe extern "C" fn() -> *mut c_void = super::PyEval_GetFrame;
#[used]
static KEEP_PYEVAL_GET_BUILTINS: unsafe extern "C" fn() -> *mut c_void = super::PyEval_GetBuiltins;
#[used]
static KEEP_PYEVAL_GET_GLOBALS: unsafe extern "C" fn() -> *mut c_void = super::PyEval_GetGlobals;
#[used]
static KEEP_PYEVAL_GET_LOCALS: unsafe extern "C" fn() -> *mut c_void = super::PyEval_GetLocals;
#[used]
static KEEP_PYEVAL_GET_FRAME_BUILTINS: unsafe extern "C" fn() -> *mut c_void =
    super::PyEval_GetFrameBuiltins;
#[used]
static KEEP_PYEVAL_GET_FRAME_GLOBALS: unsafe extern "C" fn() -> *mut c_void =
    super::PyEval_GetFrameGlobals;
#[used]
static KEEP_PYEVAL_GET_FRAME_LOCALS: unsafe extern "C" fn() -> *mut c_void =
    super::PyEval_GetFrameLocals;
#[used]
static KEEP_PYEVAL_GET_FUNC_NAME: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    super::PyEval_GetFuncName;
#[used]
static KEEP_PYEVAL_GET_FUNC_DESC: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    super::PyEval_GetFuncDesc;
#[used]
static KEEP_PYITER_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyIter_Check;
#[used]
static KEEP_PYITER_NEXTITEM: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> i32 =
    super::PyIter_NextItem;
#[used]
static KEEP_PYITER_SEND: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32 =
    super::PyIter_Send;
#[used]
static KEEP_PYITER_NEXT: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyIter_Next;
#[used]
static KEEP_PYCAPSULE_NEW: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    Option<unsafe extern "C" fn(*mut c_void)>,
) -> *mut c_void = super::PyCapsule_New;
#[used]
static KEEP_PYCAPSULE_GET_POINTER: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
    super::PyCapsule_GetPointer;
#[used]
static KEEP_PYCAPSULE_GET_NAME: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    super::PyCapsule_GetName;
#[used]
static KEEP_PYCAPSULE_SET_POINTER: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyCapsule_SetPointer;
#[used]
static KEEP_PYCAPSULE_GET_DESTRUCTOR: unsafe extern "C" fn(
    *mut c_void,
) -> Option<
    unsafe extern "C" fn(*mut c_void),
> = super::PyCapsule_GetDestructor;
#[used]
static KEEP_PYCAPSULE_SET_DESTRUCTOR: unsafe extern "C" fn(
    *mut c_void,
    Option<unsafe extern "C" fn(*mut c_void)>,
) -> i32 = super::PyCapsule_SetDestructor;
#[used]
static KEEP_PYCAPSULE_SET_CONTEXT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyCapsule_SetContext;
#[used]
static KEEP_PYCAPSULE_GET_CONTEXT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyCapsule_GetContext;
#[used]
static KEEP_PYCAPSULE_SET_NAME: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    super::PyCapsule_SetName;
#[used]
static KEEP_PYCAPSULE_IS_VALID: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    super::PyCapsule_IsValid;
#[used]
static KEEP_PYCAPSULE_IMPORT: unsafe extern "C" fn(*const c_char, i32) -> *mut c_void =
    super::PyCapsule_Import;
#[used]
static KEEP_PYLIST_NEW: unsafe extern "C" fn(isize) -> *mut c_void = super::PyList_New;
#[used]
static KEEP_PYLIST_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = super::PyList_Size;
#[used]
static KEEP_PYLIST_APPEND: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyList_Append;
#[used]
static KEEP_PYLIST_GET_ITEM: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    super::PyList_GetItem;
#[used]
static KEEP_PYLIST_GET_ITEM_REF: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    super::PyList_GetItemRef;
#[used]
static KEEP_PYLIST_SET_ITEM: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) -> i32 =
    super::PyList_SetItem;
#[used]
static KEEP_PYLIST_INSERT: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) -> i32 =
    super::PyList_Insert;
#[used]
static KEEP_PYLIST_GET_SLICE: unsafe extern "C" fn(*mut c_void, isize, isize) -> *mut c_void =
    super::PyList_GetSlice;
#[used]
static KEEP_PYLIST_SET_SLICE: unsafe extern "C" fn(*mut c_void, isize, isize, *mut c_void) -> i32 =
    super::PyList_SetSlice;
#[used]
static KEEP_PYLIST_SORT: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyList_Sort;
#[used]
static KEEP_PYLIST_REVERSE: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyList_Reverse;
#[used]
static KEEP_PYLIST_AS_TUPLE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyList_AsTuple;
#[used]
static KEEP_PYSET_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PySet_New;
#[used]
static KEEP_PYFROZENSET_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyFrozenSet_New;
#[used]
static KEEP_PYSET_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = super::PySet_Size;
#[used]
static KEEP_PYSET_CONTAINS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PySet_Contains;
#[used]
static KEEP_PYSET_ADD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 = super::PySet_Add;
#[used]
static KEEP_PYSET_DISCARD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PySet_Discard;
#[used]
static KEEP_PYSET_CLEAR: unsafe extern "C" fn(*mut c_void) -> i32 = super::PySet_Clear;
#[used]
static KEEP_PYSET_POP: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PySet_Pop;
#[used]
static KEEP_PYDICT_NEW: unsafe extern "C" fn() -> *mut c_void = super::PyDict_New;
#[used]
static KEEP_PYDICT_NEW_PRESIZED: unsafe extern "C" fn(isize) -> *mut c_void =
    super::_PyDict_NewPresized;
#[used]
static KEEP_PYDICT_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = super::PyDict_Size;
#[used]
static KEEP_PYDICT_SET_ITEM: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
    super::PyDict_SetItem;
#[used]
static KEEP_PYDICT_SET_DEFAULT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyDict_SetDefault;
#[used]
static KEEP_PYDICT_SET_DEFAULT_REF: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32 = super::PyDict_SetDefaultRef;
#[used]
static KEEP_PYDICT_GET_ITEM: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyDict_GetItem;
#[used]
static KEEP_PYDICT_GET_ITEM_WITH_ERROR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyDict_GetItemWithError;
#[used]
static KEEP__PYDICT_GET_ITEM_KNOWNHASH: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
) -> *mut c_void = super::_PyDict_GetItem_KnownHash;
#[used]
static KEEP_PYDICT_CONTAINS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyDict_Contains;
#[used]
static KEEP_PYDICT_SET_ITEM_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = super::PyDict_SetItemString;
#[used]
static KEEP_PYDICT_GET_ITEM_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyDict_GetItemString;
#[used]
static KEEP_PYDICT_GET_ITEM_REF: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32 = super::PyDict_GetItemRef;
#[used]
static KEEP_PYDICT_GET_ITEM_STRING_REF: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut *mut c_void,
) -> i32 = super::PyDict_GetItemStringRef;
#[used]
static KEEP_PYDICT_POP: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32 =
    super::PyDict_Pop;
#[used]
static KEEP_PYDICT_POP_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut *mut c_void,
) -> i32 = super::PyDict_PopString;
#[used]
static KEEP_PY_DICT_POP_PRIVATE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::_PyDict_Pop;
#[used]
static KEEP_PYDICT_DEL_ITEM: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyDict_DelItem;
#[used]
static KEEP_PYDICT_DEL_ITEM_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    super::PyDict_DelItemString;
#[used]
static KEEP_PYDICT_CONTAINS_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    super::PyDict_ContainsString;
#[used]
static KEEP_PYDICT_COPY: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyDict_Copy;
#[used]
static KEEP_PYDICT_CLEAR: unsafe extern "C" fn(*mut c_void) = super::PyDict_Clear;
#[used]
static KEEP_PYDICT_MERGE: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 =
    super::PyDict_Merge;
#[used]
static KEEP_PYDICT_UPDATE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyDict_Update;
#[used]
static KEEP_PYDICT_MERGE_FROM_SEQ2: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 =
    super::PyDict_MergeFromSeq2;
#[used]
static KEEP_PYDICT_KEYS: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyDict_Keys;
#[used]
static KEEP_PYDICT_VALUES: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyDict_Values;
#[used]
static KEEP_PYDICT_ITEMS: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyDict_Items;
#[used]
static KEEP_PYDICT_NEXT: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
    *mut *mut c_void,
    *mut *mut c_void,
) -> i32 = super::PyDict_Next;
#[used]
static KEEP_PYDICT_PROXY_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyDictProxy_New;
#[used]
static KEEP_PYOBJECT_GETATTR_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = super::PyObject_GetAttrString;
#[used]
static KEEP_PYOBJECT_GETATTR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyObject_GetAttr;
#[used]
static KEEP_PYOBJECT_GENERIC_GETATTR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyObject_GenericGetAttr;
#[used]
static KEEP_PYOBJECT_GENERIC_SETATTR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> i32 = super::PyObject_GenericSetAttr;
#[used]
static KEEP_PYOBJECT_GENERIC_GETDICT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyObject_GenericGetDict;
#[used]
static KEEP_PYOBJECT_GENERIC_SETDICT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> i32 = super::PyObject_GenericSetDict;
#[used]
static KEEP__PYOBJECT_GETDICTPTR: unsafe extern "C" fn(*mut c_void) -> *mut *mut c_void =
    super::cpython_object_attr_api::_PyObject_GetDictPtr;
#[used]
static KEEP_PYOBJECT_SETATTR_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = super::PyObject_SetAttrString;
#[used]
static KEEP_PYOBJECT_SETATTR: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
    super::PyObject_SetAttr;
#[used]
static KEEP_PYOBJECT_DELATTR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyObject_DelAttr;
#[used]
static KEEP_PYOBJECT_DELATTR_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    super::PyObject_DelAttrString;
#[used]
static KEEP_PYOBJECT_DELITEM_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    super::PyObject_DelItemString;
#[used]
static KEEP_PYOBJECT_HASATTR_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    super::PyObject_HasAttrString;
#[used]
static KEEP_PYOBJECT_HASATTR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyObject_HasAttr;
#[used]
static KEEP_PYOBJECT_HASATTR_WITH_ERROR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    super::PyObject_HasAttrWithError;
#[used]
static KEEP_PYOBJECT_HASATTR_STRING_WITH_ERROR: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> i32 = super::PyObject_HasAttrStringWithError;
#[used]
static KEEP_PYOBJECT_GETOPTIONALATTR_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut *mut c_void,
) -> i32 = super::PyObject_GetOptionalAttrString;
#[used]
static KEEP_PYOBJECT_ISTRUE: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyObject_IsTrue;
#[used]
static KEEP_PYOBJECT_NOT: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyObject_Not;
#[used]
static KEEP_PYOBJECT_STR: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyObject_Str;
#[used]
static KEEP_PYOBJECT_REPR: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyObject_Repr;
#[used]
static KEEP_PYOBJECT_ASCII: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyObject_ASCII;
#[used]
static KEEP_PYOBJECT_DIR: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyObject_Dir;
#[used]
static KEEP_PYOBJECT_BYTES: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyObject_Bytes;
#[used]
static KEEP_PYOBJECT_FORMAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyObject_Format;
#[used]
static KEEP_PYOBJECT_GETITER: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyObject_GetIter;
#[used]
static KEEP_PYAITER_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyAIter_Check;
#[used]
static KEEP_PYOBJECT_GETAITER: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyObject_GetAIter;
#[used]
static KEEP_PYOBJECT_SELFITER: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyObject_SelfIter;
#[used]
static KEEP_PYOBJECT_CALLOBJECT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyObject_CallObject;
#[used]
static KEEP_PYOBJECT_CALL: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyObject_Call;
#[used]
static KEEP_PYOBJECT_CALL_ONEARG: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyObject_CallOneArg;
#[used]
static KEEP_PYOBJECT_CALL_NOARGS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyObject_CallNoArgs;
#[used]
static KEEP_PYOBJECT_CALL_FINALIZER: unsafe extern "C" fn(*mut c_void) =
    super::PyObject_CallFinalizer;
#[used]
static KEEP_PYOBJECT_CALL_FINALIZER_FROM_DEALLOC: unsafe extern "C" fn(*mut c_void) -> c_int =
    super::PyObject_CallFinalizerFromDealloc;
#[used]
static KEEP_PYOBJECT_CLEARMANAGEDDICT: unsafe extern "C" fn(*mut c_void) =
    super::PyObject_ClearManagedDict;
#[used]
static KEEP_PYOBJECT_VISITMANAGEDDICT: unsafe extern "C" fn(
    *mut c_void,
    Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    *mut c_void,
) -> c_int = super::PyObject_VisitManagedDict;
#[used]
static KEEP_PYUNSTABLE_OBJECT_ENABLE_DEFERRED_REFCOUNT: unsafe extern "C" fn(*mut c_void) -> c_int =
    super::PyUnstable_Object_EnableDeferredRefcount;
#[used]
static KEEP_PYCFUNCTION_CALL: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyCFunction_Call;
#[used]
static KEEP_PYCFUNCTION_NEW: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyCFunction_New;
#[used]
static KEEP_PYCFUNCTION_NEW_EX: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyCFunction_NewEx;
#[used]
static KEEP_PYCMETHOD_NEW: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyCMethod_New;
#[used]
static KEEP_PYDESCR_NEW_METHOD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyDescr_NewMethod;
#[used]
static KEEP_PYDESCR_NEW_CLASS_METHOD: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyDescr_NewClassMethod;
#[used]
static KEEP_PYCLASSMETHOD_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyClassMethod_New;
#[used]
static KEEP__PYCLASSMETHOD_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::_PyClassMethod_New;
#[used]
static KEEP_PYSTATICMETHOD_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyStaticMethod_New;
#[used]
static KEEP__PYSTATICMETHOD_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::_PyStaticMethod_New;
#[used]
static KEEP_PYWRAPPER_NEW: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyWrapper_New;
#[used]
static KEEP_PYDESCR_NEW_MEMBER: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyDescr_NewMember;
#[used]
static KEEP_PYMEMBER_GET_ONE: unsafe extern "C" fn(*const c_char, *mut c_void) -> *mut c_void =
    super::PyMember_GetOne;
#[used]
static KEEP_PYMEMBER_SET_ONE: unsafe extern "C" fn(*mut c_char, *mut c_void, *mut c_void) -> c_int =
    super::PyMember_SetOne;
#[used]
static KEEP_PYDESCR_NEW_GETSET: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyDescr_NewGetSet;
#[used]
static KEEP_PYCFUNCTION_GET_FUNCTION: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyCFunction_GetFunction;
#[used]
static KEEP_PYCFUNCTION_GET_SELF: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyCFunction_GetSelf;
#[used]
static KEEP_PYCFUNCTION_GET_FLAGS: unsafe extern "C" fn(*mut c_void) -> i32 =
    super::PyCFunction_GetFlags;
#[used]
static KEEP_PYERR_SET_OBJECT: unsafe extern "C" fn(*mut c_void, *mut c_void) =
    super::PyErr_SetObject;
#[used]
static KEEP_PYERR_SET_NONE: unsafe extern "C" fn(*mut c_void) = super::PyErr_SetNone;
#[used]
static KEEP_PYERR_NOMEMORY: unsafe extern "C" fn() -> *mut c_void = super::PyErr_NoMemory;
#[used]
static KEEP_PYERR_CHECK_SIGNALS: unsafe extern "C" fn() -> i32 = super::PyErr_CheckSignals;
#[used]
static KEEP_PYGILSTATE_ENSURE: unsafe extern "C" fn() -> i32 = super::PyGILState_Ensure;
#[used]
static KEEP_PYGILSTATE_GET_THIS_THREAD_STATE: unsafe extern "C" fn() -> *mut c_void =
    super::PyGILState_GetThisThreadState;
#[used]
static KEEP_PYGILSTATE_RELEASE: unsafe extern "C" fn(i32) = super::PyGILState_Release;
#[used]
static KEEP_PYEVAL_ACQUIRE_LOCK: unsafe extern "C" fn() = super::PyEval_AcquireLock;
#[used]
static KEEP_PYEVAL_RELEASE_LOCK: unsafe extern "C" fn() = super::PyEval_ReleaseLock;
#[used]
static KEEP_PYEVAL_ACQUIRE_THREAD: unsafe extern "C" fn(*mut c_void) = super::PyEval_AcquireThread;
#[used]
static KEEP_PYEVAL_RELEASE_THREAD: unsafe extern "C" fn(*mut c_void) = super::PyEval_ReleaseThread;
#[used]
static KEEP_PYEVAL_INIT_THREADS: unsafe extern "C" fn() = super::PyEval_InitThreads;
#[used]
static KEEP_PYEVAL_THREADS_INITIALIZED: unsafe extern "C" fn() -> i32 =
    super::PyEval_ThreadsInitialized;
#[used]
static KEEP_PYEVAL_CALL_OBJECT_WITH_KEYWORDS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyEval_CallObjectWithKeywords;
#[used]
static KEEP_PYEVAL_EVAL_CODE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyEval_EvalCode;
#[used]
static KEEP_PYEVAL_EVAL_CODE_EX: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *const *mut c_void,
    i32,
    *const *mut c_void,
    i32,
    *const *mut c_void,
    i32,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyEval_EvalCodeEx;
#[used]
static KEEP_PYEVAL_SAVE_THREAD: unsafe extern "C" fn() -> *mut c_void = super::PyEval_SaveThread;
#[used]
static KEEP_PYEVAL_RESTORE_THREAD: unsafe extern "C" fn(*mut c_void) = super::PyEval_RestoreThread;
#[used]
static KEEP_PYINTERPRETERSTATE_MAIN: unsafe extern "C" fn() -> *mut c_void =
    super::PyInterpreterState_Main;
#[used]
static KEEP_PYNUMBER_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = super::PyNumber_Check;
#[used]
static KEEP_PYNUMBER_ABSOLUTE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyNumber_Absolute;
#[used]
static KEEP_PYNUMBER_ADD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_Add;
#[used]
static KEEP_PYNUMBER_SUBTRACT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_Subtract;
#[used]
static KEEP_PYNUMBER_MULTIPLY: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_Multiply;
#[used]
static KEEP_PYNUMBER_TRUE_DIVIDE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_TrueDivide;
#[used]
static KEEP_PYNUMBER_FLOOR_DIVIDE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_FloorDivide;
#[used]
static KEEP_PYNUMBER_REMAINDER: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_Remainder;
#[used]
static KEEP_PYNUMBER_DIVMOD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_Divmod;
#[used]
static KEEP_PYNUMBER_POWER: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyNumber_Power;
#[used]
static KEEP_PYNUMBER_MATRIX_MULTIPLY: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyNumber_MatrixMultiply;
#[used]
static KEEP_PYNUMBER_LSHIFT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_Lshift;
#[used]
static KEEP_PYNUMBER_RSHIFT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_Rshift;
#[used]
static KEEP_PYNUMBER_AND: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_And;
#[used]
static KEEP_PYNUMBER_OR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_Or;
#[used]
static KEEP_PYNUMBER_XOR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_Xor;
#[used]
static KEEP_PYNUMBER_INPLACE_ADD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_InPlaceAdd;
#[used]
static KEEP_PYNUMBER_INPLACE_SUBTRACT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyNumber_InPlaceSubtract;
#[used]
static KEEP_PYNUMBER_INPLACE_MULTIPLY: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyNumber_InPlaceMultiply;
#[used]
static KEEP_PYNUMBER_INPLACE_MATRIX_MULTIPLY: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyNumber_InPlaceMatrixMultiply;
#[used]
static KEEP_PYNUMBER_INPLACE_FLOOR_DIVIDE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyNumber_InPlaceFloorDivide;
#[used]
static KEEP_PYNUMBER_INPLACE_TRUE_DIVIDE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyNumber_InPlaceTrueDivide;
#[used]
static KEEP_PYNUMBER_INPLACE_REMAINDER: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyNumber_InPlaceRemainder;
#[used]
static KEEP_PYNUMBER_INPLACE_POWER: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = super::PyNumber_InPlacePower;
#[used]
static KEEP_PYNUMBER_INPLACE_LSHIFT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_InPlaceLshift;
#[used]
static KEEP_PYNUMBER_INPLACE_RSHIFT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_InPlaceRshift;
#[used]
static KEEP_PYNUMBER_INPLACE_AND: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_InPlaceAnd;
#[used]
static KEEP_PYNUMBER_INPLACE_OR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_InPlaceOr;
#[used]
static KEEP_PYNUMBER_INPLACE_XOR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    super::PyNumber_InPlaceXor;
#[used]
static KEEP_PYNUMBER_NEGATIVE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyNumber_Negative;
#[used]
static KEEP_PYNUMBER_POSITIVE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyNumber_Positive;
#[used]
static KEEP_PYNUMBER_INVERT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyNumber_Invert;
#[used]
static KEEP_PYNUMBER_LONG: unsafe extern "C" fn(*mut c_void) -> *mut c_void = super::PyNumber_Long;
#[used]
static KEEP_PYNUMBER_FLOAT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyNumber_Float;
#[used]
static KEEP_PYNUMBER_INDEX: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    super::PyNumber_Index;
#[used]
static KEEP_PYNUMBER_AS_SSIZE_T: unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize =
    super::PyNumber_AsSsize_t;
#[used]
static KEEP_PYNUMBER_TO_BASE: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    super::PyNumber_ToBase;
#[used]
static KEEP_PYMEM_RAW_MALLOC: unsafe extern "C" fn(usize) -> *mut c_void = super::PyMem_RawMalloc;
#[used]
static KEEP_PYMEM_RAW_CALLOC: unsafe extern "C" fn(usize, usize) -> *mut c_void =
    super::PyMem_RawCalloc;
#[used]
static KEEP_PYMEM_RAW_REALLOC: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void =
    super::PyMem_RawRealloc;
#[used]
static KEEP_PYMEM_RAW_FREE: unsafe extern "C" fn(*mut c_void) = super::PyMem_RawFree;
#[used]
static KEEP_PYMARSHAL_READ_OBJECT_FROM_STRING: unsafe extern "C" fn(
    *const c_char,
    isize,
) -> *mut c_void = super::PyMarshal_ReadObjectFromString;
#[used]
static KEEP_PYMARSHAL_WRITE_OBJECT_TO_STRING: unsafe extern "C" fn(
    *mut c_void,
    i32,
) -> *mut c_void = super::PyMarshal_WriteObjectToString;
#[used]
static KEEP_PYMEM_MALLOC: unsafe extern "C" fn(usize) -> *mut c_void = super::PyMem_Malloc;
#[used]
static KEEP_PYMEM_CALLOC: unsafe extern "C" fn(usize, usize) -> *mut c_void = super::PyMem_Calloc;
#[used]
static KEEP_PYMEM_REALLOC: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void =
    super::PyMem_Realloc;
#[used]
static KEEP_PYMEM_FREE: unsafe extern "C" fn(*mut c_void) = super::PyMem_Free;
