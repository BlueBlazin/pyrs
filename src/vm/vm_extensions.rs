use std::backtrace::Backtrace;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_long, c_uint, c_ulong, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use crate::bytecode::cpython::{
    PyObject as CpythonMarshalObject, marshal_dump_object, marshal_load_object,
};
use crate::extensions::{
    PYRS_CAPI_ABI_VERSION, PYRS_TYPE_BOOL, PYRS_TYPE_BYTES, PYRS_TYPE_DICT, PYRS_TYPE_FLOAT,
    PYRS_TYPE_INT, PYRS_TYPE_LIST, PYRS_TYPE_NONE, PYRS_TYPE_STR, PYRS_TYPE_TUPLE, PyrsApiV1,
    PyrsBufferInfoV1, PyrsBufferInfoV2, PyrsBufferViewV1, PyrsCFunctionKwV1, PyrsCFunctionV1,
    PyrsCapsuleDestructorV1, PyrsModuleStateFinalizeV1, PyrsModuleStateFreeV1, PyrsObjectHandle,
    PyrsWritableBufferViewV1,
};
use crate::runtime::{
    BigInt, BoundMethod, BuiltinFunction, ClassObject, InstanceObject, IteratorKind,
    IteratorObject, NativeMethodKind, NativeMethodObject, Object, RuntimeError,
    SliceValue, Value,
};
use super::{
    BYTES_BACKING_STORAGE_ATTR, ExtensionCallableKind, GeneratorResumeOutcome, InternalCallOutcome,
    NativeCallResult, ObjRef, STR_BACKING_STORAGE_ATTR, Vm, add_values, dict_contains_key_checked,
    dict_get_value, dict_remove_value, dict_set_value_checked, exception_type_is_subclass,
    is_truthy, memoryview_bounds, mul_values, value_to_int, vm_current_thread_ident,
    vm_os_thread_ident,
};

#[cfg(windows)]
type Cwchar = u16;
#[cfg(not(windows))]
type Cwchar = i32;

const CPY_PROXY_CLASS_NAME: &str = "__pyrs_cpython_proxy__";
const CPY_PROXY_PTR_ATTR: &str = "__pyrs_cpython_proxy_ptr__";
const CPY_PROXY_MARKER_ATTR: &str = "__pyrs_cpython_proxy_marker__";
const CPY_EXCEPTION_TYPE_PTR_ATTR: &str = "__pyrs_cpython_exception_type_ptr__";
static TRACE_NUMPY_TYPEDICT_PTR: AtomicUsize = AtomicUsize::new(0);
mod callable_runtime;
mod capi_v1;
mod cpython_args_runtime;
mod cpython_bytes_api;
mod cpython_call_runtime;
mod cpython_capsule_api;
mod cpython_codec_api;
mod cpython_codec_runtime;
mod cpython_context_runtime;
mod cpython_contextvar_api;
mod cpython_dict_api;
mod cpython_eval_api;
mod cpython_exception_name_runtime;
mod cpython_gc_alloc_api;
mod cpython_import_api;
mod cpython_import_runtime;
mod cpython_iter_api;
mod cpython_list_api;
mod cpython_long_float_api;
mod cpython_mem_api;
mod cpython_module_api;
mod cpython_module_name_runtime;
mod cpython_module_runtime;
mod cpython_numeric_api;
mod cpython_numeric_runtime;
mod cpython_object_attr_api;
mod cpython_object_buffer_api;
mod cpython_object_call_api;
mod cpython_object_item_compare_api;
mod cpython_object_lifecycle_api;
mod cpython_refcount_api;
mod cpython_runtime_misc_api;
mod cpython_set_api;
mod cpython_thread_interp_api;
mod cpython_type_api;
mod cpython_tuple_api;
mod cpython_unicode_error_api;
mod cpython_unicode_error_runtime;
mod cpython_weakref_api;
mod loader_runtime;
mod module_context_state;
mod proxy_runtime;

use self::cpython_args_runtime::{
    cpython_keyword_args_from_dict_object, cpython_positional_args_from_tuple_object,
};
use self::cpython_bytes_api::{
    _PyBytes_Join, PyByteArray_AsString, PyByteArray_Concat, PyByteArray_FromObject,
    PyByteArray_FromStringAndSize, PyByteArray_Resize, PyByteArray_Size, PyBytes_AsString,
    PyBytes_AsStringAndSize, PyBytes_Concat, PyBytes_ConcatAndDel, PyBytes_DecodeEscape,
    PyBytes_FromObject, PyBytes_FromString, PyBytes_FromStringAndSize, PyBytes_Join, PyBytes_Repr,
    PyBytes_Size,
};
use self::cpython_call_runtime::{cpython_call_internal_in_context, cpython_getattr_in_context};
use self::cpython_capsule_api::{
    PyCapsule_GetContext, PyCapsule_GetDestructor, PyCapsule_GetName, PyCapsule_GetPointer,
    PyCapsule_Import, PyCapsule_IsValid, PyCapsule_New, PyCapsule_SetContext,
    PyCapsule_SetDestructor, PyCapsule_SetName, PyCapsule_SetPointer,
};
use self::cpython_codec_api::{
    PyCodec_BackslashReplaceErrors, PyCodec_Decode, PyCodec_Decoder, PyCodec_Encode,
    PyCodec_Encoder, PyCodec_IgnoreErrors, PyCodec_IncrementalDecoder, PyCodec_IncrementalEncoder,
    PyCodec_KnownEncoding, PyCodec_LookupError, PyCodec_NameReplaceErrors, PyCodec_Register,
    PyCodec_RegisterError, PyCodec_ReplaceErrors, PyCodec_StreamReader, PyCodec_StreamWriter,
    PyCodec_StrictErrors, PyCodec_Unregister, PyCodec_XMLCharRefReplaceErrors,
};
use self::cpython_codec_runtime::{
    cpython_codec_builtin_handler_ptr, cpython_codec_call_callable_in_context,
    cpython_codec_error_info, cpython_codec_handler_tuple_result,
    cpython_codec_lookup_attr_in_context, cpython_codec_module_in_context,
    cpython_codec_optional_name, cpython_codec_required_name,
    cpython_codec_stream_fallback_in_context,
};
use self::cpython_context_runtime::{
    cpython_builtin_cfunction_varargs_kwargs, cpython_call_builtin,
    cpython_error_message_indicates_missing_attribute, cpython_is_reduce_probe_name,
    cpython_new_bytes_ptr, cpython_new_ptr_for_value, cpython_set_active_context,
    cpython_set_error, cpython_set_typed_error, cpython_trace_numpy_reduce_enabled,
    cpython_value_from_ptr, cpython_value_from_ptr_or_proxy, with_active_cpython_context_mut,
};
use self::cpython_contextvar_api::{PyContextVar_Get, PyContextVar_New, PyContextVar_Set};
use self::cpython_dict_api::{
    _PyDict_GetItem_KnownHash, _PyDict_Pop, PyDict_Clear, PyDict_Contains, PyDict_ContainsString,
    PyDict_Copy, PyDict_DelItem, PyDict_DelItemString, PyDict_GetItem, PyDict_GetItemRef,
    PyDict_GetItemString, PyDict_GetItemStringRef, PyDict_GetItemWithError, PyDict_Items,
    PyDict_Keys, PyDict_Merge, PyDict_MergeFromSeq2, PyDict_New, PyDict_Next, PyDict_Pop,
    PyDict_PopString, PyDict_SetDefault, PyDict_SetDefaultRef, PyDict_SetItem,
    PyDict_SetItemString, PyDict_Size, PyDict_Update, PyDict_Values, PyDictProxy_New,
};
use self::cpython_eval_api::{
    PyEval_GetBuiltins, PyEval_GetFrame, PyEval_GetFrameBuiltins, PyEval_GetFrameGlobals,
    PyEval_GetFrameLocals, PyEval_GetFuncDesc, PyEval_GetFuncName, PyEval_GetGlobals,
    PyEval_GetLocals,
};
use self::cpython_exception_name_runtime::{
    cpython_exception_name_from_runtime_message, cpython_exception_name_parts,
};
use self::cpython_gc_alloc_api::{
    PyGC_Collect, PyGC_Disable, PyGC_Enable, PyGC_IsEnabled, PyObject_Calloc, PyObject_Free,
    PyObject_GC_Del, PyObject_GC_IsFinalized, PyObject_GC_IsTracked, PyObject_GC_Track,
    PyObject_GC_UnTrack, PyObject_Malloc, PyObject_Realloc,
};
use self::cpython_import_api::{
    PyImport_AddModule, PyImport_AddModuleObject, PyImport_AddModuleRef, PyImport_AppendInittab,
    PyImport_ExecCodeModule, PyImport_ExecCodeModuleEx, PyImport_ExecCodeModuleObject,
    PyImport_ExecCodeModuleWithPathnames, PyImport_GetImporter, PyImport_GetMagicNumber,
    PyImport_GetMagicTag, PyImport_GetModule, PyImport_GetModuleDict, PyImport_Import,
    PyImport_ImportFrozenModule, PyImport_ImportFrozenModuleObject, PyImport_ImportModule,
    PyImport_ImportModuleLevel, PyImport_ImportModuleLevelObject, PyImport_ImportModuleNoBlock,
    PyImport_ReloadModule,
};
use self::cpython_import_runtime::CpythonInittabInitFunc;
use self::cpython_iter_api::{
    PyIter_Check, PyIter_Next, PyIter_NextItem, PyIter_Send, cpython_active_exception_is,
    cpython_clear_active_exception,
};
use self::cpython_list_api::{
    PyList_Append, PyList_AsTuple, PyList_GetItem, PyList_GetItemRef, PyList_GetSlice,
    PyList_Insert, PyList_New, PyList_Reverse, PyList_SetItem, PyList_SetSlice, PyList_Size,
    PyList_Sort,
};
use self::cpython_long_float_api::{
    PyBool_FromLong, PyFloat_FromDouble, PyFloat_FromString, PyLong_AsNativeBytes,
    PyLong_FromInt32, PyLong_FromInt64, PyLong_FromLong, PyLong_FromLongLong,
    PyLong_FromNativeBytes, PyLong_FromSize_t, PyLong_FromSsize_t, PyLong_FromString,
    PyLong_FromUInt32, PyLong_FromUInt64, PyLong_FromUnicodeObject, PyLong_FromUnsignedLong,
    PyLong_FromUnsignedLongLong, PyLong_FromUnsignedNativeBytes, PyLong_FromVoidPtr,
    PyLong_GetInfo,
};
use self::cpython_mem_api::{
    PyMem_Calloc, PyMem_Free, PyMem_Malloc, PyMem_RawCalloc, PyMem_RawFree, PyMem_RawMalloc,
    PyMem_RawRealloc, PyMem_Realloc,
};
use self::cpython_module_api::{
    PyModule_Add, PyModule_AddFunctions, PyModule_AddIntConstant, PyModule_AddObject,
    PyModule_AddObjectRef, PyModule_AddStringConstant, PyModule_AddType, PyModule_Create2,
    PyModule_ExecDef, PyModule_FromDefAndSpec2, PyModule_GetDef, PyModule_GetDict,
    PyModule_GetFilename, PyModule_GetFilenameObject, PyModule_GetName, PyModule_GetNameObject,
    PyModule_GetState, PyModule_New, PyModule_NewObject, PyModule_SetDocString, PyModuleDef_Init,
};
use self::cpython_module_runtime::cpython_bind_module_def;
use self::cpython_numeric_api::{
    PyNumber_Absolute, PyNumber_Add, PyNumber_And, PyNumber_AsSsize_t, PyNumber_Check,
    PyNumber_Divmod, PyNumber_Float, PyNumber_FloorDivide, PyNumber_InPlaceAdd,
    PyNumber_InPlaceAnd, PyNumber_InPlaceFloorDivide, PyNumber_InPlaceLshift,
    PyNumber_InPlaceMatrixMultiply, PyNumber_InPlaceMultiply, PyNumber_InPlaceOr,
    PyNumber_InPlacePower, PyNumber_InPlaceRemainder, PyNumber_InPlaceRshift,
    PyNumber_InPlaceSubtract, PyNumber_InPlaceTrueDivide, PyNumber_InPlaceXor, PyNumber_Index,
    PyNumber_Invert, PyNumber_Long, PyNumber_Lshift, PyNumber_MatrixMultiply, PyNumber_Multiply,
    PyNumber_Negative, PyNumber_Or, PyNumber_Positive, PyNumber_Power, PyNumber_Remainder,
    PyNumber_Rshift, PyNumber_Subtract, PyNumber_ToBase, PyNumber_TrueDivide, PyNumber_Xor,
};
use self::cpython_numeric_runtime::cpython_binary_numeric_op_with_heap;
use self::cpython_object_attr_api::{
    _PyObject_Type, PyObject_DelAttr, PyObject_DelAttrString, PyObject_DelItemString,
    PyObject_GenericGetAttr, PyObject_GenericGetDict, PyObject_GenericSetAttr,
    PyObject_GenericSetDict, PyObject_GetAttr, PyObject_GetAttrString,
    PyObject_GetOptionalAttrString, PyObject_GetTypeData, PyObject_HasAttr, PyObject_HasAttrString,
    PyObject_HasAttrStringWithError, PyObject_HasAttrWithError, PyObject_SetAttr,
    PyObject_SetAttrString, PyObject_Type,
};
use self::cpython_object_buffer_api::{
    PyBuffer_FillContiguousStrides, PyBuffer_FillInfo, PyBuffer_FromContiguous,
    PyBuffer_GetPointer, PyBuffer_IsContiguous, PyBuffer_SizeFromFormat, PyBuffer_ToContiguous,
    PyMemoryView_FromBuffer, PyMemoryView_FromMemory, PyMemoryView_FromObject,
    PyMemoryView_GetContiguous, PyObject_AsCharBuffer, PyObject_AsFileDescriptor,
    PyObject_AsReadBuffer, PyObject_AsWriteBuffer, PyObject_CheckBuffer, PyObject_CheckReadBuffer,
    PyObject_CopyData, PyObject_GetBuffer, PyObject_Print,
};
use self::cpython_object_call_api::{
    PyAIter_Check, PyArg_UnpackTuple, PyCode_NewEmpty, PyMethod_New, PyObject_ASCII,
    PyObject_Bytes, PyObject_Call, PyObject_CallFinalizer, PyObject_CallFinalizerFromDealloc,
    PyObject_CallNoArgs, PyObject_CallObject, PyObject_CallOneArg, PyObject_ClearManagedDict,
    PyObject_Dir, PyObject_Format, PyObject_GetAIter, PyObject_GetIter, PyObject_IsTrue,
    PyObject_Not, PyObject_Repr, PyObject_SelfIter, PyObject_Str, PyObject_Vectorcall,
    PyObject_VectorcallDict, PyObject_VectorcallMethod, PyObject_VisitManagedDict,
    PyUnstable_Code_New, PyUnstable_Code_NewWithPosOnlyArgs,
    PyUnstable_Object_EnableDeferredRefcount, PyVectorcall_Call,
};
use self::cpython_object_item_compare_api::{
    PyObject_DelItem, PyObject_GetItem, PyObject_GetOptionalAttr, PyObject_Hash,
    PyObject_HashNotImplemented, PyObject_IsInstance, PyObject_IsSubclass, PyObject_Length,
    PyObject_LengthHint, PyObject_RichCompare, PyObject_RichCompareBool, PyObject_SetItem,
    PyObject_Size, cpython_debug_compare_value, cpython_type_name_for_object_ptr,
    cpython_value_type_name_from_ptr,
};
use self::cpython_object_lifecycle_api::{
    _PyObject_GC_New, _PyObject_New, _PyObject_NewVar, _Py_Dealloc, PyObject_Init,
    PyObject_InitVar,
};
use self::cpython_refcount_api::{
    _Py_CheckRecursiveCall, _Py_DecRef, _Py_IncRef, _Py_NegativeRefcount, _Py_SetRefcnt,
    _PyObject_GC_NewVar, _PyObject_GC_Resize, Py_IncRef, Py_XDecRef, Py_XIncRef,
};
use self::cpython_runtime_misc_api::{
    _PyErr_BadInternalCall, _Py_FatalErrorFunc, _Py_HashDouble, _PyUnicode_IsAlpha,
    _PyUnicode_IsDecimalDigit, _PyUnicode_IsDigit, _PyUnicode_IsLowercase, _PyUnicode_IsNumeric,
    _PyUnicode_IsTitlecase, _PyUnicode_IsUppercase, _PyUnicode_IsWhitespace, Py_AddPendingCall,
    Py_AtExit, Py_BytesMain, Py_CompileString, Py_DecodeLocale, Py_EncodeLocale, Py_EndInterpreter,
    Py_Exit, Py_FatalError, Py_Finalize, Py_FinalizeEx, Py_GetArgcArgv, Py_GetBuildInfo,
    Py_GetCompiler, Py_GetCopyright, Py_GetExecPrefix, Py_GetPath, Py_GetPlatform, Py_GetPrefix,
    Py_GetProgramFullPath, Py_GetProgramName, Py_GetPythonHome, Py_GetRecursionLimit, Py_GetVersion,
    Py_Initialize, Py_InitializeEx, Py_IsFinalizing, Py_Main, Py_MakePendingCalls,
    Py_NewInterpreter, Py_PACK_FULL_VERSION, Py_PACK_VERSION, Py_ReprEnter, Py_ReprLeave,
    Py_SetPath, Py_SetProgramName, Py_SetPythonHome, Py_SetRecursionLimit, PyErr_BadArgument,
    PyErr_BadInternalCall,
};
pub use self::cpython_refcount_api::Py_DecRef;
use self::cpython_set_api::{
    PyFrozenSet_New, PySet_Add, PySet_Clear, PySet_Contains, PySet_Discard, PySet_New, PySet_Pop,
    PySet_Size,
};
use self::cpython_thread_interp_api::{
    _PyState_AddModule, _PyThreadState_Init, _PyThreadState_Prealloc, PyFrame_GetCode,
    PyFrame_GetLineNumber, PyFrame_New, PyInterpreterState_Clear, PyInterpreterState_Delete,
    PyInterpreterState_Get, PyInterpreterState_GetDict, PyInterpreterState_GetID,
    PyInterpreterState_New, PyState_AddModule, PyState_FindModule, PyState_RemoveModule,
    PyThreadState_Clear, PyThreadState_Delete, PyThreadState_DeleteCurrent, PyThreadState_Get,
    PyThreadState_GetDict, PyThreadState_GetFrame, PyThreadState_GetID,
    PyThreadState_GetInterpreter, PyThreadState_GetUnchecked, PyThreadState_New,
    PyThreadState_SetAsyncExc, PyThreadState_Swap, PyTraceMalloc_Track, PyTraceMalloc_Untrack,
    PyVectorcall_NARGS, Py_EnterRecursiveCall, Py_GetConstant, Py_GetConstantBorrowed, Py_Is,
    Py_IsFalse, Py_IsInitialized, Py_IsNone, Py_IsTrue, Py_LeaveRecursiveCall, Py_NewRef,
    Py_REFCNT, Py_TYPE, Py_XNewRef,
};
use self::cpython_type_api::{
    _PyType_Lookup, PyType_ClearCache, PyType_Freeze, PyType_FromMetaclass,
    PyType_FromModuleAndSpec, PyType_FromSpec, PyType_FromSpecWithBases, PyType_GenericAlloc,
    PyType_GenericNew, PyType_GetBaseByToken, PyType_GetFlags, PyType_GetFullyQualifiedName,
    PyType_GetModule, PyType_GetModuleByDef, PyType_GetModuleName, PyType_GetModuleState,
    PyType_GetName, PyType_GetQualName, PyType_GetSlot, PyType_GetTypeDataSize, PyType_IsSubtype,
    PyType_Modified, PyType_Ready, cpython_is_type_object_ptr, cpython_type_tp_call,
};
use self::cpython_tuple_api::{
    PyTuple_GetItem, PyTuple_GetSlice, PyTuple_New, PyTuple_SetItem, PyTuple_Size,
};
use self::cpython_unicode_error_api::{
    PyUnicodeDecodeError_Create, PyUnicodeDecodeError_GetEncoding, PyUnicodeDecodeError_GetEnd,
    PyUnicodeDecodeError_GetObject, PyUnicodeDecodeError_GetReason, PyUnicodeDecodeError_GetStart,
    PyUnicodeDecodeError_SetEnd, PyUnicodeDecodeError_SetReason, PyUnicodeDecodeError_SetStart,
    PyUnicodeEncodeError_GetEncoding, PyUnicodeEncodeError_GetEnd, PyUnicodeEncodeError_GetObject,
    PyUnicodeEncodeError_GetReason, PyUnicodeEncodeError_GetStart, PyUnicodeEncodeError_SetEnd,
    PyUnicodeEncodeError_SetReason, PyUnicodeEncodeError_SetStart, PyUnicodeTranslateError_GetEnd,
    PyUnicodeTranslateError_GetObject, PyUnicodeTranslateError_GetReason,
    PyUnicodeTranslateError_GetStart, PyUnicodeTranslateError_SetEnd,
    PyUnicodeTranslateError_SetReason, PyUnicodeTranslateError_SetStart,
};
use self::cpython_unicode_error_runtime::cpython_exception_value_attr;
use self::cpython_weakref_api::{
    PyObject_ClearWeakRefs, PyWeakref_GetObject, PyWeakref_GetRef, PyWeakref_NewProxy,
    PyWeakref_NewRef,
};

thread_local! {
    static PROXY_REFRESH_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static CPY_PROXY_GET_ITER_ACTIVE: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static CPY_PROXY_ATTR_LOOKUP_ACTIVE: RefCell<Vec<(usize, String, bool)>> = const { RefCell::new(Vec::new()) };
}

struct ProxyRefreshReentryGuard;

impl ProxyRefreshReentryGuard {
    fn enter() -> Option<Self> {
        let allowed = PROXY_REFRESH_ACTIVE.with(|flag| {
            if flag.get() {
                false
            } else {
                flag.set(true);
                true
            }
        });
        if allowed { Some(Self) } else { None }
    }
}

impl Drop for ProxyRefreshReentryGuard {
    fn drop(&mut self) {
        PROXY_REFRESH_ACTIVE.with(|flag| flag.set(false));
    }
}

struct ProxyAttrLookupReentryGuard {
    raw_ptr: usize,
    attr_name: String,
    is_type_object: bool,
}

impl ProxyAttrLookupReentryGuard {
    fn enter(raw_ptr: usize, attr_name: &str, is_type_object: bool) -> Option<Self> {
        let mut reentered = false;
        CPY_PROXY_ATTR_LOOKUP_ACTIVE.with(|active| {
            let mut stack = active.borrow_mut();
            if stack.iter().any(|(ptr, name, is_type)| {
                *ptr == raw_ptr && *is_type == is_type_object && name == attr_name
            }) {
                reentered = true;
                return;
            }
            stack.push((raw_ptr, attr_name.to_string(), is_type_object));
        });
        if reentered {
            None
        } else {
            Some(Self {
                raw_ptr,
                attr_name: attr_name.to_string(),
                is_type_object,
            })
        }
    }
}

impl Drop for ProxyAttrLookupReentryGuard {
    fn drop(&mut self) {
        CPY_PROXY_ATTR_LOOKUP_ACTIVE.with(|active| {
            let mut stack = active.borrow_mut();
            if let Some(index) = stack.iter().rposition(|(ptr, name, is_type)| {
                *ptr == self.raw_ptr && *is_type == self.is_type_object && *name == self.attr_name
            }) {
                stack.remove(index);
            }
        });
    }
}

fn is_cpython_proxy_class(class_data: &ClassObject) -> bool {
    matches!(
        class_data.attrs.get(CPY_PROXY_MARKER_ATTR),
        Some(Value::Bool(true))
    ) || class_data.name == CPY_PROXY_CLASS_NAME
}

fn cpython_type_name_parts(tp_name: &str) -> (String, Option<String>) {
    match tp_name.rsplit_once('.') {
        Some((module, name)) => (name.to_string(), Some(module.to_string())),
        None => (tp_name.to_string(), None),
    }
}

struct CapiObjectSlot {
    value: Value,
    refcount: usize,
}

struct CapiCapsuleSlot {
    pointer: usize,
    context: usize,
    name: Option<CString>,
    destructor: Option<PyrsCapsuleDestructorV1>,
    cpython_destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    exported_name: Option<String>,
    refcount: usize,
}

#[derive(Clone, Copy)]
struct CpythonErrorState {
    ptype: *mut c_void,
    pvalue: *mut c_void,
    ptraceback: *mut c_void,
}

#[derive(Clone, Copy)]
enum CpythonDescriptorKind {
    Method {
        owner_type: *mut CpythonTypeObject,
        method_def: *mut CpythonMethodDef,
        class_method: bool,
    },
    GetSet {
        owner_type: *mut CpythonTypeObject,
        getset: *mut CpythonGetSetDef,
    },
    Member {
        owner_type: *mut CpythonTypeObject,
        member: *mut CpythonMemberDef,
    },
}

struct BufferInfoSnapshot {
    data: *const u8,
    len: usize,
    readonly: bool,
    itemsize: usize,
    shape: Vec<isize>,
    strides: Vec<isize>,
    contiguous: bool,
    format_text: String,
}

struct ExtensionInitScopeGuard {
    vm: *mut Vm,
    module_name: String,
}

impl ExtensionInitScopeGuard {
    fn new(vm: &mut Vm, module_name: &str) -> Self {
        Self {
            vm: vm as *mut Vm,
            module_name: module_name.to_string(),
        }
    }
}

impl Drop for ExtensionInitScopeGuard {
    fn drop(&mut self) {
        if self.vm.is_null() {
            return;
        }
        // SAFETY: `vm` points to the active VM for the scope of extension initialization.
        unsafe {
            (*self.vm)
                .extension_init_in_progress
                .remove(&self.module_name);
        }
    }
}

#[repr(C)]
struct CpythonCompatObject {
    ob_base: CpythonVarObjectHead,
}

#[repr(C)]
struct CpythonDictCompatObject {
    ob_base: CpythonObjectHead,
    ma_used: isize,
    ma_watcher_tag: u64,
    ma_keys: *mut c_void,
    ma_values: *mut c_void,
}

#[repr(C)]
struct CpythonListCompatObject {
    ob_base: CpythonVarObjectHead,
    ob_item: *mut *mut c_void,
    allocated: isize,
}

#[repr(C)]
struct CpythonTupleCompatObject {
    ob_base: CpythonVarObjectHead,
    ob_hash: isize,
}

#[repr(C)]
struct CpythonBytesCompatObject {
    ob_base: CpythonVarObjectHead,
    ob_shash: isize,
    ob_sval: [u8; 1],
}

#[repr(C)]
struct CpythonFloatCompatObject {
    ob_base: CpythonObjectHead,
    ob_fval: f64,
}

#[repr(C)]
struct CpythonCapsuleCompatObject {
    ob_base: CpythonObjectHead,
    pointer: *mut c_void,
    name: *const c_char,
    context: *mut c_void,
    destructor: Option<unsafe extern "C" fn(*mut c_void)>,
}

#[repr(C)]
struct CpythonCFunctionCompatObject {
    ob_base: CpythonObjectHead,
    m_ml: *mut CpythonMethodDef,
    m_self: *mut c_void,
    m_module: *mut c_void,
    m_class: *mut c_void,
}

#[repr(C)]
struct CpythonForeignLongValue {
    lv_tag: usize,
    ob_digit: [u32; 1],
}

#[repr(C)]
struct CpythonForeignLongObject {
    ob_base: CpythonObjectHead,
    long_value: CpythonForeignLongValue,
}

#[repr(C)]
struct CpythonModuleDefBase {
    _ob_refcnt: usize,
    _ob_type: *mut c_void,
    _m_init: Option<unsafe extern "C" fn() -> *mut c_void>,
    _m_index: isize,
    _m_copy: *mut c_void,
}

#[repr(C)]
struct CpythonMethodDef {
    ml_name: *const c_char,
    ml_meth: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void>,
    ml_flags: i32,
    ml_doc: *const c_char,
}

#[repr(C)]
struct CpythonGetSetDef {
    name: *const c_char,
    get: *mut c_void,
    set: *mut c_void,
    doc: *const c_char,
    closure: *mut c_void,
}

#[repr(C)]
struct CpythonMemberDef {
    name: *const c_char,
    member_type: c_int,
    offset: isize,
    flags: c_int,
    doc: *const c_char,
}

#[repr(C)]
struct CpythonStructSequenceField {
    name: *const c_char,
    doc: *const c_char,
}

#[repr(C)]
struct CpythonStructSequenceDesc {
    name: *const c_char,
    doc: *const c_char,
    fields: *mut CpythonStructSequenceField,
    n_in_sequence: c_int,
}

#[repr(C)]
struct CpythonTypeSlot {
    slot: c_int,
    pfunc: *mut c_void,
}

#[repr(C)]
struct CpythonTypeSpec {
    name: *const c_char,
    basicsize: c_int,
    itemsize: c_int,
    flags: c_uint,
    slots: *mut CpythonTypeSlot,
}

#[repr(C)]
struct CpythonModuleDef {
    _m_base: CpythonModuleDefBase,
    m_name: *const c_char,
    m_doc: *const c_char,
    m_size: isize,
    m_methods: *mut CpythonMethodDef,
    m_slots: *mut c_void,
    _m_traverse: *mut c_void,
    _m_clear: *mut c_void,
    _m_free: Option<unsafe extern "C" fn(*mut c_void)>,
}

#[repr(C)]
struct CpythonModuleDefSlot {
    slot: i32,
    value: *mut c_void,
}

#[repr(C)]
struct CpythonDateTimeCapi {
    date_type: *mut c_void,
    datetime_type: *mut c_void,
    time_type: *mut c_void,
    delta_type: *mut c_void,
    tzinfo_type: *mut c_void,
    timezone_utc: *mut c_void,
    date_from_date: unsafe extern "C" fn(i32, i32, i32, *mut c_void) -> *mut c_void,
    datetime_from_date_and_time: unsafe extern "C" fn(
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        *mut c_void,
        *mut c_void,
    ) -> *mut c_void,
    time_from_time:
        unsafe extern "C" fn(i32, i32, i32, i32, *mut c_void, *mut c_void) -> *mut c_void,
    delta_from_delta: unsafe extern "C" fn(i32, i32, i32, i32, *mut c_void) -> *mut c_void,
    timezone_from_timezone: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
    datetime_from_timestamp:
        unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void,
    date_from_timestamp: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
    datetime_from_date_and_time_and_fold: unsafe extern "C" fn(
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        *mut c_void,
        i32,
        *mut c_void,
    ) -> *mut c_void,
    time_from_time_and_fold:
        unsafe extern "C" fn(i32, i32, i32, i32, *mut c_void, i32, *mut c_void) -> *mut c_void,
}

#[repr(C)]
pub struct CpythonTypeObject {
    ob_refcnt: isize,
    ob_type: *mut c_void,
    ob_size: isize,
    tp_name: *const c_char,
    tp_basicsize: isize,
    tp_itemsize: isize,
    tp_dealloc: *mut c_void,
    tp_vectorcall_offset: isize,
    tp_getattr: *mut c_void,
    tp_setattr: *mut c_void,
    tp_as_async: *mut c_void,
    tp_repr: *mut c_void,
    tp_as_number: *mut c_void,
    tp_as_sequence: *mut c_void,
    tp_as_mapping: *mut c_void,
    tp_hash: *mut c_void,
    tp_call: *mut c_void,
    tp_str: *mut c_void,
    tp_getattro: *mut c_void,
    tp_setattro: *mut c_void,
    tp_as_buffer: *mut c_void,
    tp_flags: usize,
    tp_doc: *const c_char,
    tp_traverse: *mut c_void,
    tp_clear: *mut c_void,
    tp_richcompare: *mut c_void,
    tp_weaklistoffset: isize,
    tp_iter: *mut c_void,
    tp_iternext: *mut c_void,
    tp_methods: *mut c_void,
    tp_members: *mut c_void,
    tp_getset: *mut c_void,
    tp_base: *mut CpythonTypeObject,
    tp_dict: *mut c_void,
    tp_descr_get: *mut c_void,
    tp_descr_set: *mut c_void,
    tp_dictoffset: isize,
    tp_init: *mut c_void,
    tp_alloc: *mut c_void,
    tp_new: *mut c_void,
    tp_free: *mut c_void,
    tp_is_gc: *mut c_void,
    tp_bases: *mut c_void,
    tp_mro: *mut c_void,
    tp_cache: *mut c_void,
    tp_subclasses: *mut c_void,
    tp_weaklist: *mut c_void,
    tp_del: *mut c_void,
    tp_version_tag: u32,
    tp_finalize: *mut c_void,
    tp_vectorcall: *mut c_void,
    tp_watched: u8,
    tp_versions_used: u16,
}

#[repr(C)]
struct CpythonNumberMethods {
    nb_add: *mut c_void,
    nb_subtract: *mut c_void,
    nb_multiply: *mut c_void,
    nb_remainder: *mut c_void,
    nb_divmod: *mut c_void,
    nb_power: *mut c_void,
    nb_negative: *mut c_void,
    nb_positive: *mut c_void,
    nb_absolute: *mut c_void,
    nb_bool: *mut c_void,
    nb_invert: *mut c_void,
    nb_lshift: *mut c_void,
    nb_rshift: *mut c_void,
    nb_and: *mut c_void,
    nb_xor: *mut c_void,
    nb_or: *mut c_void,
    nb_int: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
    nb_reserved: *mut c_void,
    nb_float: *mut c_void,
    nb_inplace_add: *mut c_void,
    nb_inplace_subtract: *mut c_void,
    nb_inplace_multiply: *mut c_void,
    nb_inplace_remainder: *mut c_void,
    nb_inplace_power: *mut c_void,
    nb_inplace_lshift: *mut c_void,
    nb_inplace_rshift: *mut c_void,
    nb_inplace_and: *mut c_void,
    nb_inplace_xor: *mut c_void,
    nb_inplace_or: *mut c_void,
    nb_floor_divide: *mut c_void,
    nb_true_divide: *mut c_void,
    nb_inplace_floor_divide: *mut c_void,
    nb_inplace_true_divide: *mut c_void,
    nb_index: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
    nb_matrix_multiply: *mut c_void,
    nb_inplace_matrix_multiply: *mut c_void,
}

#[repr(C)]
struct CpythonMappingMethods {
    mp_length: *mut c_void,
    mp_subscript: *mut c_void,
    mp_ass_subscript: *mut c_void,
}

#[repr(C)]
struct CpythonSequenceMethods {
    sq_length: *mut c_void,
    sq_concat: *mut c_void,
    sq_repeat: *mut c_void,
    sq_item: *mut c_void,
    was_sq_slice: *mut c_void,
    sq_ass_item: *mut c_void,
    was_sq_ass_slice: *mut c_void,
    sq_contains: *mut c_void,
    sq_inplace_concat: *mut c_void,
    sq_inplace_repeat: *mut c_void,
}

#[repr(C)]
pub struct CpythonComplexValue {
    real: f64,
    imag: f64,
}

const PYRS_DATETIME_CAPSULE_NAME: &str = "datetime.datetime_CAPI";
const PY_MEMBER_T_SHORT: c_int = 0;
const PY_MEMBER_T_INT: c_int = 1;
const PY_MEMBER_T_LONG: c_int = 2;
const PY_MEMBER_T_FLOAT: c_int = 3;
const PY_MEMBER_T_DOUBLE: c_int = 4;
const PY_MEMBER_T_STRING: c_int = 5;
const PY_MEMBER_T_OBJECT: c_int = 6;
const PY_MEMBER_T_CHAR: c_int = 7;
const PY_MEMBER_T_BYTE: c_int = 8;
const PY_MEMBER_T_UBYTE: c_int = 9;
const PY_MEMBER_T_USHORT: c_int = 10;
const PY_MEMBER_T_UINT: c_int = 11;
const PY_MEMBER_T_ULONG: c_int = 12;
const PY_MEMBER_T_STRING_INPLACE: c_int = 13;
const PY_MEMBER_T_BOOL: c_int = 14;
const PY_MEMBER_T_OBJECT_EX: c_int = 16;
const PY_MEMBER_T_LONGLONG: c_int = 17;
const PY_MEMBER_T_ULONGLONG: c_int = 18;
const PY_MEMBER_T_PYSSIZET: c_int = 19;
const PY_MEMBER_T_NONE: c_int = 20;
const PY_MEMBER_READONLY: c_int = 1;
const PY_MEMBER_RELATIVE_OFFSET: c_int = 8;
const PY_TYPE_SLOT_TP_ALLOC: c_int = 47;
const PY_TYPE_SLOT_TP_BASE: c_int = 48;
const PY_TYPE_SLOT_TP_BASES: c_int = 49;
const PY_TYPE_SLOT_TP_CALL: c_int = 50;
const PY_TYPE_SLOT_TP_CLEAR: c_int = 51;
const PY_TYPE_SLOT_TP_DEALLOC: c_int = 52;
const PY_TYPE_SLOT_TP_DEL: c_int = 53;
const PY_TYPE_SLOT_TP_DESCR_GET: c_int = 54;
const PY_TYPE_SLOT_TP_DESCR_SET: c_int = 55;
const PY_TYPE_SLOT_TP_DOC: c_int = 56;
const PY_TYPE_SLOT_TP_GETATTR: c_int = 57;
const PY_TYPE_SLOT_TP_GETATTRO: c_int = 58;
const PY_TYPE_SLOT_TP_HASH: c_int = 59;
const PY_TYPE_SLOT_TP_INIT: c_int = 60;
const PY_TYPE_SLOT_TP_IS_GC: c_int = 61;
const PY_TYPE_SLOT_TP_ITER: c_int = 62;
const PY_TYPE_SLOT_TP_ITERNEXT: c_int = 63;
const PY_TYPE_SLOT_TP_METHODS: c_int = 64;
const PY_TYPE_SLOT_TP_NEW: c_int = 65;
const PY_TYPE_SLOT_TP_REPR: c_int = 66;
const PY_TYPE_SLOT_TP_RICHCOMPARE: c_int = 67;
const PY_TYPE_SLOT_TP_SETATTR: c_int = 68;
const PY_TYPE_SLOT_TP_SETATTRO: c_int = 69;
const PY_TYPE_SLOT_TP_STR: c_int = 70;
const PY_TYPE_SLOT_TP_TRAVERSE: c_int = 71;
const PY_TYPE_SLOT_TP_MEMBERS: c_int = 72;
const PY_TYPE_SLOT_TP_GETSET: c_int = 73;
const PY_TYPE_SLOT_TP_FREE: c_int = 74;
const PY_TYPE_SLOT_TP_FINALIZE: c_int = 80;
const PY_TYPE_SLOT_TP_VECTORCALL: c_int = 82;
const PY_TYPE_SLOT_TP_TOKEN: c_int = 83;
const PY_TYPE_SLOT_MAX: c_int = 83;

unsafe extern "C" fn datetime_capi_unimplemented() -> *mut c_void {
    cpython_set_error("datetime C-API constructor is not implemented");
    std::ptr::null_mut()
}

unsafe extern "C" fn datetime_capi_date_from_date(
    _year: i32,
    _month: i32,
    _day: i32,
    _typ: *mut c_void,
) -> *mut c_void {
    unsafe { datetime_capi_unimplemented() }
}

unsafe extern "C" fn datetime_capi_datetime_from_date_and_time(
    _year: i32,
    _month: i32,
    _day: i32,
    _hour: i32,
    _minute: i32,
    _second: i32,
    _microsecond: i32,
    _tzinfo: *mut c_void,
    _typ: *mut c_void,
) -> *mut c_void {
    unsafe { datetime_capi_unimplemented() }
}

unsafe extern "C" fn datetime_capi_time_from_time(
    _hour: i32,
    _minute: i32,
    _second: i32,
    _microsecond: i32,
    _tzinfo: *mut c_void,
    _typ: *mut c_void,
) -> *mut c_void {
    unsafe { datetime_capi_unimplemented() }
}

unsafe extern "C" fn datetime_capi_delta_from_delta(
    _days: i32,
    _seconds: i32,
    _microseconds: i32,
    _normalize: i32,
    _typ: *mut c_void,
) -> *mut c_void {
    unsafe { datetime_capi_unimplemented() }
}

unsafe extern "C" fn datetime_capi_timezone_from_timezone(
    _offset: *mut c_void,
    _name: *mut c_void,
) -> *mut c_void {
    unsafe { datetime_capi_unimplemented() }
}

unsafe extern "C" fn datetime_capi_datetime_from_timestamp(
    _typ: *mut c_void,
    _args: *mut c_void,
    _kwargs: *mut c_void,
) -> *mut c_void {
    unsafe { datetime_capi_unimplemented() }
}

unsafe extern "C" fn datetime_capi_date_from_timestamp(
    _typ: *mut c_void,
    _args: *mut c_void,
) -> *mut c_void {
    unsafe { datetime_capi_unimplemented() }
}

unsafe extern "C" fn datetime_capi_datetime_from_date_and_time_and_fold(
    _year: i32,
    _month: i32,
    _day: i32,
    _hour: i32,
    _minute: i32,
    _second: i32,
    _microsecond: i32,
    _tzinfo: *mut c_void,
    _fold: i32,
    _typ: *mut c_void,
) -> *mut c_void {
    unsafe { datetime_capi_unimplemented() }
}

unsafe extern "C" fn datetime_capi_time_from_time_and_fold(
    _hour: i32,
    _minute: i32,
    _second: i32,
    _microsecond: i32,
    _tzinfo: *mut c_void,
    _fold: i32,
    _typ: *mut c_void,
) -> *mut c_void {
    unsafe { datetime_capi_unimplemented() }
}

static mut PYRS_DATETIME_CAPI: CpythonDateTimeCapi = CpythonDateTimeCapi {
    date_type: std::ptr::null_mut(),
    datetime_type: std::ptr::null_mut(),
    time_type: std::ptr::null_mut(),
    delta_type: std::ptr::null_mut(),
    tzinfo_type: std::ptr::null_mut(),
    timezone_utc: std::ptr::null_mut(),
    date_from_date: datetime_capi_date_from_date,
    datetime_from_date_and_time: datetime_capi_datetime_from_date_and_time,
    time_from_time: datetime_capi_time_from_time,
    delta_from_delta: datetime_capi_delta_from_delta,
    timezone_from_timezone: datetime_capi_timezone_from_timezone,
    datetime_from_timestamp: datetime_capi_datetime_from_timestamp,
    date_from_timestamp: datetime_capi_date_from_timestamp,
    datetime_from_date_and_time_and_fold: datetime_capi_datetime_from_date_and_time_and_fold,
    time_from_time_and_fold: datetime_capi_time_from_time_and_fold,
};

static PYBYTES_ASSTRING_MISMATCH_BT_COUNT: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
pub struct CpythonBuffer {
    buf: *mut c_void,
    obj: *mut c_void,
    len: isize,
    itemsize: isize,
    readonly: i32,
    ndim: i32,
    format: *mut c_char,
    shape: *mut isize,
    strides: *mut isize,
    suboffsets: *mut isize,
    internal: *mut c_void,
}

#[repr(C)]
struct CpythonBufferInternal {
    handle: PyrsObjectHandle,
}

#[repr(C)]
pub struct CpythonObjectHead {
    ob_refcnt: isize,
    ob_type: *mut c_void,
}

#[repr(C)]
struct CpythonVarObjectHead {
    ob_base: CpythonObjectHead,
    ob_size: isize,
}

#[inline]
fn cpython_tuple_storage_bytes(tuple_len: usize) -> usize {
    std::mem::size_of::<CpythonTupleCompatObject>()
        .saturating_add(tuple_len.saturating_mul(std::mem::size_of::<*mut c_void>()))
}

#[inline]
fn cpython_bytes_storage_bytes(len: usize) -> usize {
    std::mem::size_of::<CpythonVarObjectHead>()
        .saturating_add(std::mem::size_of::<isize>())
        .saturating_add(len)
        .saturating_add(1)
}

#[inline]
unsafe fn cpython_tuple_items_ptr(tuple: *mut c_void) -> *mut *mut c_void {
    // SAFETY: caller guarantees `tuple` points to writable tuple-compatible storage.
    unsafe {
        tuple
            .cast::<u8>()
            .add(std::mem::size_of::<CpythonTupleCompatObject>())
            .cast::<*mut c_void>()
    }
}

#[inline]
unsafe fn cpython_bytes_data_ptr(object: *mut c_void) -> *mut c_char {
    // SAFETY: caller guarantees `object` points to bytes-compatible storage.
    unsafe {
        object
            .cast::<u8>()
            .add(std::mem::size_of::<CpythonVarObjectHead>() + std::mem::size_of::<isize>())
            .cast::<c_char>()
    }
}

type CpythonVectorcallFn =
    unsafe extern "C" fn(*mut c_void, *const *mut c_void, usize, *mut c_void) -> *mut c_void;

unsafe fn cpython_resolve_vectorcall(callable: *mut c_void) -> Option<CpythonVectorcallFn> {
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if callable.is_null() {
        return None;
    }
    if (callable as usize) < MIN_VALID_PTR
        || (callable as usize) % std::mem::align_of::<usize>() != 0
    {
        return None;
    }
    // SAFETY: caller provides a candidate PyObject*.
    let head = unsafe { callable.cast::<CpythonObjectHead>().as_ref() }?;
    let type_ptr = head.ob_type.cast::<CpythonTypeObject>();
    if type_ptr.is_null() {
        return None;
    }
    if (type_ptr as usize) < MIN_VALID_PTR
        || (type_ptr as usize) % std::mem::align_of::<usize>() != 0
    {
        return None;
    }
    // SAFETY: `type_ptr` is non-null and points to a type object header.
    let mut raw = unsafe { (*type_ptr).tp_vectorcall };
    if raw.is_null() {
        // SAFETY: `type_ptr` is valid for metadata reads.
        let offset = unsafe { (*type_ptr).tp_vectorcall_offset };
        if offset > 0 {
            // SAFETY: CPython stores vectorcall function pointer at object+offset.
            let slot_ptr =
                unsafe { callable.cast::<u8>().add(offset as usize) }.cast::<*mut c_void>();
            // SAFETY: slot address computed from object + valid offset.
            raw = unsafe { *slot_ptr };
        }
    }
    if raw.is_null() {
        None
    } else {
        // SAFETY: resolved pointer follows vectorcall ABI contract.
        Some(unsafe { std::mem::transmute(raw) })
    }
}

unsafe fn cpython_foreign_long_to_i64(object: *mut c_void) -> Option<i64> {
    if object.is_null() {
        return None;
    }
    // SAFETY: caller provides a foreign PyObject*.
    let head = unsafe { object.cast::<CpythonObjectHead>().as_ref() }?;
    let type_ptr = head.ob_type.cast::<CpythonTypeObject>();
    if type_ptr.is_null() {
        return None;
    }
    let is_long = type_ptr == std::ptr::addr_of_mut!(PyLong_Type)
        // SAFETY: type pointers are valid for subtype checks.
        || unsafe {
            PyType_IsSubtype(
                type_ptr.cast::<c_void>(),
                std::ptr::addr_of_mut!(PyLong_Type).cast::<c_void>(),
            ) != 0
        };
    if !is_long {
        return None;
    }
    // CPython 3.14 long layout uses lv_tag low bits for sign and high bits for ndigits.
    let raw = object.cast::<CpythonForeignLongObject>();
    // SAFETY: layout matches CPython long object memory representation.
    let lv_tag = unsafe { (*raw).long_value.lv_tag };
    let sign_bits = lv_tag & 0x3;
    if sign_bits == 1 {
        return Some(0);
    }
    let sign = if sign_bits == 2 { -1i128 } else { 1i128 };
    let ndigits = lv_tag >> 3;
    if ndigits == 0 {
        return Some(0);
    }
    // SAFETY: CPython allocates at least `ndigits` digits for normalized longs.
    let digits = unsafe { (*raw).long_value.ob_digit.as_ptr() };
    let mut acc: i128 = 0;
    for idx in 0..ndigits {
        // SAFETY: `idx < ndigits` within normalized digit buffer.
        let digit = unsafe { *digits.add(idx) } as i128;
        let shift = 30usize.saturating_mul(idx);
        if shift >= 126 {
            return None;
        }
        acc = acc.checked_add(digit.checked_shl(shift as u32)?)?;
    }
    let signed = if sign < 0 { -acc } else { acc };
    i64::try_from(signed).ok()
}

unsafe fn cpython_foreign_long_to_u64(object: *mut c_void) -> Option<u64> {
    if object.is_null() {
        return None;
    }
    // SAFETY: caller provides a foreign PyObject*.
    let head = unsafe { object.cast::<CpythonObjectHead>().as_ref() }?;
    let type_ptr = head.ob_type.cast::<CpythonTypeObject>();
    if type_ptr.is_null() {
        return None;
    }
    let is_long = type_ptr == std::ptr::addr_of_mut!(PyLong_Type)
        // SAFETY: type pointers are valid for subtype checks.
        || unsafe {
            PyType_IsSubtype(
                type_ptr.cast::<c_void>(),
                std::ptr::addr_of_mut!(PyLong_Type).cast::<c_void>(),
            ) != 0
        };
    if !is_long {
        return None;
    }
    // CPython 3.14 long layout uses lv_tag low bits for sign and high bits for ndigits.
    let raw = object.cast::<CpythonForeignLongObject>();
    // SAFETY: layout matches CPython long object memory representation.
    let lv_tag = unsafe { (*raw).long_value.lv_tag };
    let sign_bits = lv_tag & 0x3;
    if sign_bits == 1 {
        return Some(0);
    }
    if sign_bits == 2 {
        return None;
    }
    let ndigits = lv_tag >> 3;
    if ndigits == 0 {
        return Some(0);
    }
    // SAFETY: CPython allocates at least `ndigits` digits for normalized longs.
    let digits = unsafe { (*raw).long_value.ob_digit.as_ptr() };
    let mut acc: u128 = 0;
    for idx in 0..ndigits {
        // SAFETY: `idx < ndigits` within normalized digit buffer.
        let digit = unsafe { *digits.add(idx) } as u128;
        let shift = 30usize.saturating_mul(idx);
        if shift >= 128 {
            return None;
        }
        acc = acc.checked_add(digit.checked_shl(shift as u32)?)?;
    }
    u64::try_from(acc).ok()
}

fn cpython_bigint_to_u64(value: &BigInt) -> Option<u64> {
    if value.is_negative() {
        return None;
    }
    let bytes = value.to_abs_le_bytes();
    if bytes.len() > std::mem::size_of::<u64>() {
        return None;
    }
    let mut acc = 0u64;
    for (idx, byte) in bytes.iter().enumerate() {
        acc |= (*byte as u64) << (idx * 8);
    }
    Some(acc)
}

fn cpython_bigint_low_u64(value: &BigInt) -> u64 {
    let bytes = value.to_abs_le_bytes();
    let mut acc = 0u64;
    for (idx, byte) in bytes.iter().take(std::mem::size_of::<u64>()).enumerate() {
        acc |= (*byte as u64) << (idx * 8);
    }
    acc
}

fn cpython_bigint_from_value(value: Value) -> Result<BigInt, String> {
    match value {
        Value::Int(v) => Ok(BigInt::from_i64(v)),
        Value::Bool(v) => Ok(BigInt::from_i64(if v { 1 } else { 0 })),
        Value::BigInt(v) => Ok(*v),
        _ => Err("expect int".to_string()),
    }
}

fn cpython_bigint_is_power_of_two(value: &BigInt) -> bool {
    if value.is_zero() || value.is_negative() {
        return false;
    }
    let one = BigInt::one();
    let minus_one = value.sub(&one);
    value.bitand(&minus_one).is_zero()
}

fn cpython_required_signed_bytes_for_bigint(value: &BigInt) -> usize {
    if value.is_zero() {
        return 1;
    }
    if !value.is_negative() {
        let bits = value.bit_length();
        return (bits + 1).div_ceil(8).max(1);
    }
    let abs = value.abs();
    let bits = abs.bit_length();
    if cpython_bigint_is_power_of_two(&abs) {
        bits.div_ceil(8).max(1)
    } else {
        (bits + 1).div_ceil(8).max(1)
    }
}

fn cpython_required_unsigned_bytes_for_bigint(value: &BigInt) -> usize {
    if value.is_zero() {
        1
    } else {
        value.bit_length().div_ceil(8).max(1)
    }
}

fn cpython_bigint_to_unsigned_le_bytes(value: &BigInt) -> Vec<u8> {
    let abs = value.abs();
    abs.to_abs_le_bytes()
}

fn cpython_bigint_to_twos_complement_le(value: &BigInt, n_bytes: usize) -> Vec<u8> {
    if n_bytes == 0 {
        return Vec::new();
    }
    if !value.is_negative() {
        let raw = cpython_bigint_to_unsigned_le_bytes(value);
        let mut out = vec![0u8; n_bytes];
        let copy_len = std::cmp::min(n_bytes, raw.len());
        out[..copy_len].copy_from_slice(&raw[..copy_len]);
        return out;
    }
    let raw = cpython_bigint_to_unsigned_le_bytes(value);
    let mut out = vec![0u8; n_bytes];
    let copy_len = std::cmp::min(n_bytes, raw.len());
    out[..copy_len].copy_from_slice(&raw[..copy_len]);
    for byte in &mut out {
        *byte = !*byte;
    }
    let mut carry = 1u16;
    for byte in &mut out {
        let sum = *byte as u16 + carry;
        *byte = sum as u8;
        carry = sum >> 8;
        if carry == 0 {
            break;
        }
    }
    out
}

fn cpython_bigint_from_unsigned_le_bytes(bytes: &[u8]) -> BigInt {
    let mut out = BigInt::zero();
    for byte in bytes.iter().rev() {
        out = out.mul_small(256);
        out = out.add_small(*byte as u32);
    }
    out
}

fn cpython_bigint_from_twos_complement_le(bytes: &[u8], signed: bool) -> BigInt {
    if bytes.is_empty() {
        return BigInt::zero();
    }
    if !signed {
        return cpython_bigint_from_unsigned_le_bytes(bytes);
    }
    let sign_set = (bytes[bytes.len() - 1] & 0x80) != 0;
    if !sign_set {
        return cpython_bigint_from_unsigned_le_bytes(bytes);
    }
    let mut mag = bytes.to_vec();
    for byte in &mut mag {
        *byte = !*byte;
    }
    let mut carry = 1u16;
    for byte in &mut mag {
        let sum = *byte as u16 + carry;
        *byte = sum as u8;
        carry = sum >> 8;
        if carry == 0 {
            break;
        }
    }
    cpython_bigint_from_unsigned_le_bytes(&mag).negated()
}

fn cpython_asnativebytes_resolve_endian(flags: i32) -> i32 {
    if flags == -1 || (flags & 0x2) != 0 {
        if cfg!(target_endian = "little") { 1 } else { 0 }
    } else {
        flags & 0x1
    }
}

fn cpython_type_for_value(value: &Value) -> *mut c_void {
    match value {
        Value::None => std::ptr::addr_of_mut!(PyNone_Type).cast(),
        Value::Bool(_) => std::ptr::addr_of_mut!(PyBool_Type).cast(),
        Value::Int(_) | Value::BigInt(_) => std::ptr::addr_of_mut!(PyLong_Type).cast(),
        Value::Float(_) => std::ptr::addr_of_mut!(PyFloat_Type).cast(),
        Value::Complex { .. } => std::ptr::addr_of_mut!(PyComplex_Type).cast(),
        Value::Str(_) => std::ptr::addr_of_mut!(PyUnicode_Type).cast(),
        Value::List(_) => std::ptr::addr_of_mut!(PyList_Type).cast(),
        Value::Tuple(_) => std::ptr::addr_of_mut!(PyTuple_Type).cast(),
        Value::Dict(_) => std::ptr::addr_of_mut!(PyDict_Type).cast(),
        Value::DictKeys(_) => std::ptr::addr_of_mut!(PyDictProxy_Type).cast(),
        Value::Set(_) => std::ptr::addr_of_mut!(PySet_Type).cast(),
        Value::FrozenSet(_) => std::ptr::addr_of_mut!(PyFrozenSet_Type).cast(),
        Value::Bytes(_) => std::ptr::addr_of_mut!(PyBytes_Type).cast(),
        Value::ByteArray(_) => std::ptr::addr_of_mut!(PyByteArray_Type).cast(),
        Value::MemoryView(_) => std::ptr::addr_of_mut!(PyMemoryView_Type).cast(),
        Value::Module(_) => std::ptr::addr_of_mut!(PyModule_Type).cast(),
        Value::Slice(_) => std::ptr::addr_of_mut!(PySlice_Type).cast(),
        Value::Super(_) => std::ptr::addr_of_mut!(PySuper_Type).cast(),
        Value::BoundMethod(_) => std::ptr::addr_of_mut!(PyMethod_Type).cast(),
        Value::Class(_) => std::ptr::addr_of_mut!(PyType_Type).cast(),
        Value::Builtin(_) => std::ptr::addr_of_mut!(PyBaseObject_Type).cast(),
        _ => std::ptr::addr_of_mut!(PyBaseObject_Type).cast(),
    }
}

fn cpython_objref_from_value(value: Value) -> Option<ObjRef> {
    match value {
        Value::List(obj)
        | Value::Tuple(obj)
        | Value::Dict(obj)
        | Value::DictKeys(obj)
        | Value::Set(obj)
        | Value::FrozenSet(obj)
        | Value::Bytes(obj)
        | Value::ByteArray(obj)
        | Value::MemoryView(obj)
        | Value::Iterator(obj)
        | Value::Generator(obj)
        | Value::Module(obj)
        | Value::Class(obj)
        | Value::Instance(obj)
        | Value::Super(obj)
        | Value::BoundMethod(obj)
        | Value::Function(obj)
        | Value::Cell(obj) => Some(obj),
        _ => None,
    }
}

fn cpython_builtin_type_ptr_for_class_name(class_name: &str) -> Option<*mut c_void> {
    Some(match class_name {
        "type" => std::ptr::addr_of_mut!(PyType_Type).cast(),
        "object" => std::ptr::addr_of_mut!(PyBaseObject_Type).cast(),
        "bool" => std::ptr::addr_of_mut!(PyBool_Type).cast(),
        "int" => std::ptr::addr_of_mut!(PyLong_Type).cast(),
        "float" => std::ptr::addr_of_mut!(PyFloat_Type).cast(),
        "complex" => std::ptr::addr_of_mut!(PyComplex_Type).cast(),
        "str" => std::ptr::addr_of_mut!(PyUnicode_Type).cast(),
        "bytes" => std::ptr::addr_of_mut!(PyBytes_Type).cast(),
        "bytearray" => std::ptr::addr_of_mut!(PyByteArray_Type).cast(),
        "memoryview" => std::ptr::addr_of_mut!(PyMemoryView_Type).cast(),
        "list" => std::ptr::addr_of_mut!(PyList_Type).cast(),
        "tuple" => std::ptr::addr_of_mut!(PyTuple_Type).cast(),
        "dict" => std::ptr::addr_of_mut!(PyDict_Type).cast(),
        "set" => std::ptr::addr_of_mut!(PySet_Type).cast(),
        "frozenset" => std::ptr::addr_of_mut!(PyFrozenSet_Type).cast(),
        "slice" => std::ptr::addr_of_mut!(PySlice_Type).cast(),
        _ => return None,
    })
}

fn cpython_builtin_type_name_for_ptr(ptr: *mut c_void) -> Option<&'static str> {
    if ptr == std::ptr::addr_of_mut!(PyType_Type).cast() {
        Some("type")
    } else if ptr == std::ptr::addr_of_mut!(PyBaseObject_Type).cast() {
        Some("object")
    } else if ptr == std::ptr::addr_of_mut!(PyBool_Type).cast() {
        Some("bool")
    } else if ptr == std::ptr::addr_of_mut!(PyLong_Type).cast() {
        Some("int")
    } else if ptr == std::ptr::addr_of_mut!(PyFloat_Type).cast() {
        Some("float")
    } else if ptr == std::ptr::addr_of_mut!(PyComplex_Type).cast() {
        Some("complex")
    } else if ptr == std::ptr::addr_of_mut!(PyUnicode_Type).cast() {
        Some("str")
    } else if ptr == std::ptr::addr_of_mut!(PyBytes_Type).cast() {
        Some("bytes")
    } else if ptr == std::ptr::addr_of_mut!(PyByteArray_Type).cast() {
        Some("bytearray")
    } else if ptr == std::ptr::addr_of_mut!(PyMemoryView_Type).cast() {
        Some("memoryview")
    } else if ptr == std::ptr::addr_of_mut!(PyList_Type).cast() {
        Some("list")
    } else if ptr == std::ptr::addr_of_mut!(PyTuple_Type).cast() {
        Some("tuple")
    } else if ptr == std::ptr::addr_of_mut!(PyDict_Type).cast() {
        Some("dict")
    } else if ptr == std::ptr::addr_of_mut!(PySet_Type).cast() {
        Some("set")
    } else if ptr == std::ptr::addr_of_mut!(PyFrozenSet_Type).cast() {
        Some("frozenset")
    } else if ptr == std::ptr::addr_of_mut!(PySlice_Type).cast() {
        Some("slice")
    } else {
        None
    }
}

fn cpython_builtin_type_ptr_for_builtin(builtin: &BuiltinFunction) -> Option<*mut c_void> {
    Some(match builtin {
        BuiltinFunction::Type => std::ptr::addr_of_mut!(PyType_Type).cast(),
        BuiltinFunction::Slice => std::ptr::addr_of_mut!(PySlice_Type).cast(),
        BuiltinFunction::Bool => std::ptr::addr_of_mut!(PyBool_Type).cast(),
        BuiltinFunction::Int => std::ptr::addr_of_mut!(PyLong_Type).cast(),
        BuiltinFunction::Float => std::ptr::addr_of_mut!(PyFloat_Type).cast(),
        BuiltinFunction::Complex => std::ptr::addr_of_mut!(PyComplex_Type).cast(),
        BuiltinFunction::Str => std::ptr::addr_of_mut!(PyUnicode_Type).cast(),
        BuiltinFunction::List => std::ptr::addr_of_mut!(PyList_Type).cast(),
        BuiltinFunction::Tuple => std::ptr::addr_of_mut!(PyTuple_Type).cast(),
        BuiltinFunction::Dict => std::ptr::addr_of_mut!(PyDict_Type).cast(),
        BuiltinFunction::Set => std::ptr::addr_of_mut!(PySet_Type).cast(),
        BuiltinFunction::FrozenSet => std::ptr::addr_of_mut!(PyFrozenSet_Type).cast(),
        BuiltinFunction::Bytes => std::ptr::addr_of_mut!(PyBytes_Type).cast(),
        BuiltinFunction::ByteArray => std::ptr::addr_of_mut!(PyByteArray_Type).cast(),
        BuiltinFunction::MemoryView => std::ptr::addr_of_mut!(PyMemoryView_Type).cast(),
        _ => return None,
    })
}

fn cpython_value_debug_tag(value: &Value) -> String {
    match value {
        Value::None => "None".to_string(),
        Value::Bool(flag) => format!("Bool({flag})"),
        Value::Int(_) => "Int".to_string(),
        Value::BigInt(_) => "BigInt".to_string(),
        Value::Float(_) => "Float".to_string(),
        Value::Complex { .. } => "Complex".to_string(),
        Value::Str(_) => "Str".to_string(),
        Value::List(_) => "List".to_string(),
        Value::Tuple(_) => "Tuple".to_string(),
        Value::Dict(_) => "Dict".to_string(),
        Value::DictKeys(_) => "DictKeys".to_string(),
        Value::Set(_) => "Set".to_string(),
        Value::FrozenSet(_) => "FrozenSet".to_string(),
        Value::Bytes(_) => "Bytes".to_string(),
        Value::ByteArray(_) => "ByteArray".to_string(),
        Value::MemoryView(_) => "MemoryView".to_string(),
        Value::Iterator(_) => "Iterator".to_string(),
        Value::Generator(_) => "Generator".to_string(),
        Value::Module(module) => {
            if let Object::Module(data) = &*module.kind() {
                format!("Module({})", data.name)
            } else {
                "Module(<invalid>)".to_string()
            }
        }
        Value::Class(class) => {
            if let Object::Class(data) = &*class.kind() {
                format!("Class({})", data.name)
            } else {
                "Class(<invalid>)".to_string()
            }
        }
        Value::Instance(_) => "Instance".to_string(),
        Value::Super(_) => "Super".to_string(),
        Value::BoundMethod(bound_obj) => {
            if let Object::BoundMethod(bound_data) = &*bound_obj.kind()
                && let Object::Function(func_data) = &*bound_data.function.kind()
            {
                format!("BoundMethod({})", func_data.code.name)
            } else {
                "BoundMethod".to_string()
            }
        }
        Value::Function(func_obj) => {
            if let Object::Function(func_data) = &*func_obj.kind() {
                format!(
                    "Function({}@{})",
                    func_data.code.name, func_data.code.filename
                )
            } else {
                "Function".to_string()
            }
        }
        Value::Cell(_) => "Cell".to_string(),
        Value::Exception(err) => format!("Exception({})", err.name),
        Value::ExceptionType(name) => format!("ExceptionType({name})"),
        Value::Slice(_) => "Slice".to_string(),
        Value::Code(_) => "Code".to_string(),
        Value::Builtin(builtin) => format!("Builtin({builtin:?})"),
    }
}

fn cpython_debug_ufunc_attr_summary(value: &Value, depth: usize) -> String {
    if depth == 0 {
        return cpython_value_debug_tag(value);
    }
    match value {
        Value::None => "None".to_string(),
        Value::Bool(flag) => format!("Bool({flag})"),
        Value::Int(number) => format!("Int({number})"),
        Value::Float(number) => format!("Float({number})"),
        Value::Str(text) => format!("Str({text})"),
        Value::Class(class_obj) => {
            if let Object::Class(class_data) = &*class_obj.kind() {
                format!("Class({})", class_data.name)
            } else {
                "Class(<invalid>)".to_string()
            }
        }
        Value::Instance(instance_obj) => {
            if let Object::Instance(instance_data) = &*instance_obj.kind() {
                if let Object::Class(class_data) = &*instance_data.class.kind() {
                    return format!("Instance({})", class_data.name);
                }
            }
            "Instance".to_string()
        }
        Value::Tuple(tuple_obj) => {
            if let Object::Tuple(items) = &*tuple_obj.kind() {
                let rendered = items
                    .iter()
                    .take(6)
                    .map(|item| cpython_debug_ufunc_attr_summary(item, depth - 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                if items.len() > 6 {
                    format!("Tuple(len={}, [{} ...])", items.len(), rendered)
                } else {
                    format!("Tuple([{}])", rendered)
                }
            } else {
                "Tuple(<invalid>)".to_string()
            }
        }
        Value::List(list_obj) => {
            if let Object::List(items) = &*list_obj.kind() {
                let rendered = items
                    .iter()
                    .take(6)
                    .map(|item| cpython_debug_ufunc_attr_summary(item, depth - 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                if items.len() > 6 {
                    format!("List(len={}, [{} ...])", items.len(), rendered)
                } else {
                    format!("List([{}])", rendered)
                }
            } else {
                "List(<invalid>)".to_string()
            }
        }
        _ => cpython_value_debug_tag(value),
    }
}

fn cpython_debug_ufunc_exception_summary(value: &Value) -> String {
    match value {
        Value::Exception(exception_obj) => {
            let attrs = exception_obj.attrs.borrow();
            let mut parts = Vec::new();
            for key in ["ufunc", "dtypes", "casting", "signature"] {
                if let Some(attr_value) = attrs.get(key) {
                    parts.push(format!(
                        "{}={}",
                        key,
                        cpython_debug_ufunc_attr_summary(attr_value, 3)
                    ));
                }
            }
            if parts.is_empty() {
                format!("Exception({})", exception_obj.name)
            } else {
                format!("Exception({}; {})", exception_obj.name, parts.join(", "))
            }
        }
        _ => cpython_debug_ufunc_attr_summary(value, 3),
    }
}

macro_rules! for_each_cpython_exception_symbol {
    ($apply:ident) => {
        $apply!(PyExc_BaseException, "BaseException");
        $apply!(PyExc_Exception, "Exception");
        $apply!(PyExc_BaseExceptionGroup, "BaseExceptionGroup");
        $apply!(PyExc_GeneratorExit, "GeneratorExit");
        $apply!(PyExc_KeyboardInterrupt, "KeyboardInterrupt");
        $apply!(PyExc_SystemExit, "SystemExit");
        $apply!(PyExc_StopIteration, "StopIteration");
        $apply!(PyExc_StopAsyncIteration, "StopAsyncIteration");
        $apply!(PyExc_ArithmeticError, "ArithmeticError");
        $apply!(PyExc_OverflowError, "OverflowError");
        $apply!(PyExc_FloatingPointError, "FloatingPointError");
        $apply!(PyExc_ZeroDivisionError, "ZeroDivisionError");
        $apply!(PyExc_AssertionError, "AssertionError");
        $apply!(PyExc_AttributeError, "AttributeError");
        $apply!(PyExc_BufferError, "BufferError");
        $apply!(PyExc_EOFError, "EOFError");
        $apply!(PyExc_ImportError, "ImportError");
        $apply!(PyExc_ModuleNotFoundError, "ModuleNotFoundError");
        $apply!(PyExc_LookupError, "LookupError");
        $apply!(PyExc_IndexError, "IndexError");
        $apply!(PyExc_KeyError, "KeyError");
        $apply!(PyExc_MemoryError, "MemoryError");
        $apply!(PyExc_NameError, "NameError");
        $apply!(PyExc_UnboundLocalError, "UnboundLocalError");
        $apply!(PyExc_OSError, "OSError");
        $apply!(PyExc_BlockingIOError, "BlockingIOError");
        $apply!(PyExc_BrokenPipeError, "BrokenPipeError");
        $apply!(PyExc_ChildProcessError, "ChildProcessError");
        $apply!(PyExc_ConnectionError, "ConnectionError");
        $apply!(PyExc_ConnectionAbortedError, "ConnectionAbortedError");
        $apply!(PyExc_ConnectionRefusedError, "ConnectionRefusedError");
        $apply!(PyExc_ConnectionResetError, "ConnectionResetError");
        $apply!(PyExc_FileExistsError, "FileExistsError");
        $apply!(PyExc_FileNotFoundError, "FileNotFoundError");
        $apply!(PyExc_InterruptedError, "InterruptedError");
        $apply!(PyExc_IsADirectoryError, "IsADirectoryError");
        $apply!(PyExc_NotADirectoryError, "NotADirectoryError");
        $apply!(PyExc_PermissionError, "PermissionError");
        $apply!(PyExc_ProcessLookupError, "ProcessLookupError");
        $apply!(PyExc_TimeoutError, "TimeoutError");
        $apply!(PyExc_ReferenceError, "ReferenceError");
        $apply!(PyExc_RuntimeError, "RuntimeError");
        $apply!(PyExc_NotImplementedError, "NotImplementedError");
        $apply!(PyExc_RecursionError, "RecursionError");
        $apply!(PyExc_SyntaxError, "SyntaxError");
        $apply!(PyExc_IndentationError, "IndentationError");
        $apply!(PyExc_TabError, "TabError");
        $apply!(PyExc_SystemError, "SystemError");
        $apply!(PyExc_TypeError, "TypeError");
        $apply!(PyExc_ValueError, "ValueError");
        $apply!(PyExc_UnicodeError, "UnicodeError");
        $apply!(PyExc_UnicodeDecodeError, "UnicodeDecodeError");
        $apply!(PyExc_UnicodeEncodeError, "UnicodeEncodeError");
        $apply!(PyExc_UnicodeTranslateError, "UnicodeTranslateError");
        $apply!(PyExc_Warning, "Warning");
        $apply!(PyExc_DeprecationWarning, "DeprecationWarning");
        $apply!(PyExc_PendingDeprecationWarning, "PendingDeprecationWarning");
        $apply!(PyExc_RuntimeWarning, "RuntimeWarning");
        $apply!(PyExc_SyntaxWarning, "SyntaxWarning");
        $apply!(PyExc_UserWarning, "UserWarning");
        $apply!(PyExc_FutureWarning, "FutureWarning");
        $apply!(PyExc_ImportWarning, "ImportWarning");
        $apply!(PyExc_UnicodeWarning, "UnicodeWarning");
        $apply!(PyExc_BytesWarning, "BytesWarning");
        $apply!(PyExc_ResourceWarning, "ResourceWarning");
        $apply!(PyExc_EncodingWarning, "EncodingWarning");
        $apply!(PyExc_EnvironmentError, "EnvironmentError");
        $apply!(PyExc_IOError, "IOError");
        $apply!(PyExc_WindowsError, "WindowsError");
    };
}

fn cpython_exception_value_from_ptr(raw: usize) -> Option<Value> {
    macro_rules! match_exception_symbol {
        ($symbol:ident, $name:literal) => {
            // SAFETY: exception symbol pointers are process-global and stable.
            unsafe {
                let symbol = $symbol as usize;
                if symbol != 0 && raw == symbol {
                    return Some(Value::ExceptionType($name.to_string()));
                }
            }
        };
    }
    for_each_cpython_exception_symbol!(match_exception_symbol);
    None
}

fn cpython_exception_ptr_for_name(name: &str) -> Option<*mut c_void> {
    macro_rules! match_exception_name {
        ($symbol:ident, $exception_name:literal) => {
            if name == $exception_name {
                // SAFETY: exception symbol pointers are process-global.
                let ptr = unsafe { $symbol };
                if !ptr.is_null() {
                    return Some(ptr);
                }
            }
        };
    }
    for_each_cpython_exception_symbol!(match_exception_name);
    None
}

unsafe fn ensure_cpython_exception_symbol(slot: *mut *mut c_void, type_ptr: *mut c_void) {
    // SAFETY: caller passes valid pointer to static exception symbol slot.
    if unsafe { (*slot).is_null() } {
        // SAFETY: allocate and initialize stable sentinel object for exception symbol export.
        let raw =
            unsafe { malloc(std::mem::size_of::<CpythonObjectHead>()) }.cast::<CpythonObjectHead>();
        if raw.is_null() {
            return;
        }
        // SAFETY: `raw` points to valid writable object head storage.
        unsafe {
            (*raw).ob_refcnt = 1;
            (*raw).ob_type = type_ptr;
            *slot = raw.cast();
        }
    }
}

unsafe extern "C" fn cpython_object_tp_repr(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr_or_proxy(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_new_ptr_for_value(Value::Str(crate::runtime::format_value(&value)))
}

unsafe extern "C" fn cpython_object_tp_str(object: *mut c_void) -> *mut c_void {
    unsafe { cpython_object_tp_repr(object) }
}

unsafe extern "C" fn cpython_float_tp_new(
    subtype: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    if subtype.is_null() {
        cpython_set_error("TypeError: float.__new__ received null subtype");
        return std::ptr::null_mut();
    }
    let float_type_ptr = std::ptr::addr_of_mut!(PyFloat_Type).cast::<c_void>();
    if subtype != float_type_ptr && unsafe { PyType_IsSubtype(subtype, float_type_ptr) } == 0 {
        // SAFETY: `subtype` is non-null and expected to point at a type object.
        let subtype_name = unsafe {
            let subtype_ptr = subtype.cast::<CpythonTypeObject>();
            c_name_to_string((*subtype_ptr).tp_name).unwrap_or_else(|_| "<unnamed>".to_string())
        };
        cpython_set_error(format!(
            "TypeError: float.__new__({subtype_name}): {subtype_name} is not a subtype of float"
        ));
        return std::ptr::null_mut();
    }

    let positional_count = if args.is_null() {
        0
    } else {
        let size = unsafe { PyTuple_Size(args) };
        if size < 0 {
            return std::ptr::null_mut();
        }
        size as usize
    };
    if positional_count > 1 {
        cpython_set_error(format!(
            "TypeError: float expected at most 1 argument, got {positional_count}"
        ));
        return std::ptr::null_mut();
    }

    let mut input = if positional_count == 1 {
        let value = unsafe { PyTuple_GetItem(args, 0) };
        if value.is_null() {
            return std::ptr::null_mut();
        }
        value
    } else {
        std::ptr::null_mut()
    };

    if !kwargs.is_null() {
        let keyword_count = unsafe { PyDict_Size(kwargs) };
        if keyword_count < 0 {
            return std::ptr::null_mut();
        }
        if keyword_count > 1 {
            cpython_set_error("TypeError: float() takes at most 1 keyword argument");
            return std::ptr::null_mut();
        }
        if keyword_count == 1 {
            let x_name = b"x\0".as_ptr().cast::<c_char>();
            let keyword_value = unsafe { PyDict_GetItemString(kwargs, x_name) };
            if keyword_value.is_null() {
                cpython_set_error("TypeError: float() got an unexpected keyword argument");
                return std::ptr::null_mut();
            }
            if !input.is_null() {
                cpython_set_error("TypeError: float() got multiple values for argument 'x'");
                return std::ptr::null_mut();
            }
            input = keyword_value;
        }
    }

    let base_float = if input.is_null() {
        unsafe { PyFloat_FromDouble(0.0) }
    } else {
        // SAFETY: `input` is a non-null candidate PyObject pointer supplied by parser/call-site.
        let input_type = unsafe {
            input
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type)
                .unwrap_or(std::ptr::null_mut())
        };
        if input_type == std::ptr::addr_of_mut!(PyUnicode_Type).cast() {
            unsafe { PyFloat_FromString(input, std::ptr::null_mut()) }
        } else {
            unsafe { PyNumber_Float(input) }
        }
    };
    if base_float.is_null() {
        return std::ptr::null_mut();
    }

    if subtype == float_type_ptr {
        return base_float;
    }

    let subtype_ptr = subtype.cast::<CpythonTypeObject>();
    if unsafe { (*subtype_ptr).tp_basicsize }
        < std::mem::size_of::<CpythonFloatCompatObject>() as isize
    {
        unsafe { Py_DecRef(base_float) };
        cpython_set_error("TypeError: float subtype has incompatible instance layout");
        return std::ptr::null_mut();
    }

    let alloc_slot = unsafe { (*subtype_ptr).tp_alloc };
    if alloc_slot.is_null() {
        unsafe { Py_DecRef(base_float) };
        cpython_set_error("TypeError: float subtype has no allocator");
        return std::ptr::null_mut();
    }
    let alloc_fn: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
        // SAFETY: `tp_alloc` uses CPython allocfunc ABI.
        unsafe { std::mem::transmute(alloc_slot) };
    let new_obj = unsafe { alloc_fn(subtype, 0) };
    if new_obj.is_null() {
        unsafe { Py_DecRef(base_float) };
        return std::ptr::null_mut();
    }

    let value = unsafe { PyFloat_AsDouble(base_float) };
    if value == -1.0 && !unsafe { PyErr_Occurred() }.is_null() {
        unsafe {
            Py_DecRef(base_float);
            Py_DecRef(new_obj);
        }
        return std::ptr::null_mut();
    }

    // SAFETY: float subtypes are guaranteed to share `PyFloatObject` prefix layout.
    unsafe {
        (*new_obj.cast::<CpythonFloatCompatObject>()).ob_fval = value;
        Py_DecRef(base_float);
    }
    new_obj
}

fn initialize_cpython_compat_type_objects() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        let type_ptr = std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>();
        PyType_Type.ob_type = type_ptr;
        PyType_Type.tp_call = cpython_type_tp_call as *mut c_void;
        PyType_Type.tp_alloc = PyType_GenericAlloc as *mut c_void;
        PyType_Type.tp_new = PyType_GenericNew as *mut c_void;
        PyType_Type.tp_base = std::ptr::addr_of_mut!(PyBaseObject_Type);
        PyBaseObject_Type.tp_getattro = PyObject_GenericGetAttr as *mut c_void;
        PyBaseObject_Type.tp_setattro = PyObject_GenericSetAttr as *mut c_void;
        PyBaseObject_Type.tp_repr = cpython_object_tp_repr as *mut c_void;
        PyBaseObject_Type.tp_str = cpython_object_tp_str as *mut c_void;
        PyCFunction_Type.tp_call = cpython_cfunction_tp_call as *mut c_void;
        PyCFunction_Type.tp_getattro = cpython_cfunction_tp_getattro as *mut c_void;
        PyFloat_Type.tp_new = cpython_float_tp_new as *mut c_void;
        PyUnicode_Type.tp_richcompare = PyUnicode_RichCompare as *mut c_void;

        let type_objects: &mut [*mut CpythonTypeObject] = &mut [
            std::ptr::addr_of_mut!(PyBaseObject_Type),
            std::ptr::addr_of_mut!(PyBool_Type),
            std::ptr::addr_of_mut!(PyByteArrayIter_Type),
            std::ptr::addr_of_mut!(PyByteArray_Type),
            std::ptr::addr_of_mut!(PyBytesIter_Type),
            std::ptr::addr_of_mut!(PyBytes_Type),
            std::ptr::addr_of_mut!(PyCallIter_Type),
            std::ptr::addr_of_mut!(PyCFunction_Type),
            std::ptr::addr_of_mut!(PyCapsule_Type),
            std::ptr::addr_of_mut!(PyClassMethodDescr_Type),
            std::ptr::addr_of_mut!(PyComplex_Type),
            std::ptr::addr_of_mut!(PyDictItems_Type),
            std::ptr::addr_of_mut!(PyDictIterItem_Type),
            std::ptr::addr_of_mut!(PyDictIterKey_Type),
            std::ptr::addr_of_mut!(PyDictIterValue_Type),
            std::ptr::addr_of_mut!(PyDictKeys_Type),
            std::ptr::addr_of_mut!(PyDictProxy_Type),
            std::ptr::addr_of_mut!(PyDictRevIterItem_Type),
            std::ptr::addr_of_mut!(PyDictRevIterKey_Type),
            std::ptr::addr_of_mut!(PyDictRevIterValue_Type),
            std::ptr::addr_of_mut!(PyDict_Type),
            std::ptr::addr_of_mut!(PyDictValues_Type),
            std::ptr::addr_of_mut!(PyEllipsis_Type),
            std::ptr::addr_of_mut!(PyEnum_Type),
            std::ptr::addr_of_mut!(PyFilter_Type),
            std::ptr::addr_of_mut!(PyFloat_Type),
            std::ptr::addr_of_mut!(PyFrozenSet_Type),
            std::ptr::addr_of_mut!(PyGetSetDescr_Type),
            std::ptr::addr_of_mut!(Py_GenericAliasType),
            std::ptr::addr_of_mut!(PyList_Type),
            std::ptr::addr_of_mut!(PyListIter_Type),
            std::ptr::addr_of_mut!(PyListRevIter_Type),
            std::ptr::addr_of_mut!(PyLong_Type),
            std::ptr::addr_of_mut!(PyLongRangeIter_Type),
            std::ptr::addr_of_mut!(PyMap_Type),
            std::ptr::addr_of_mut!(PyMemberDescr_Type),
            std::ptr::addr_of_mut!(PyMemoryView_Type),
            std::ptr::addr_of_mut!(PyMethod_Type),
            std::ptr::addr_of_mut!(PyMethodDescr_Type),
            std::ptr::addr_of_mut!(PyModuleDef_Type),
            std::ptr::addr_of_mut!(PyModule_Type),
            std::ptr::addr_of_mut!(PyNone_Type),
            std::ptr::addr_of_mut!(PyProperty_Type),
            std::ptr::addr_of_mut!(PyRangeIter_Type),
            std::ptr::addr_of_mut!(PyRange_Type),
            std::ptr::addr_of_mut!(PyReversed_Type),
            std::ptr::addr_of_mut!(PySeqIter_Type),
            std::ptr::addr_of_mut!(PySet_Type),
            std::ptr::addr_of_mut!(PySetIter_Type),
            std::ptr::addr_of_mut!(PySlice_Type),
            std::ptr::addr_of_mut!(PySuper_Type),
            std::ptr::addr_of_mut!(_PyWeakref_CallableProxyType),
            std::ptr::addr_of_mut!(_PyWeakref_ProxyType),
            std::ptr::addr_of_mut!(_PyWeakref_RefType),
            std::ptr::addr_of_mut!(PyTraceBack_Type),
            std::ptr::addr_of_mut!(PyTuple_Type),
            std::ptr::addr_of_mut!(PyTupleIter_Type),
            std::ptr::addr_of_mut!(PyUnicode_Type),
            std::ptr::addr_of_mut!(PyUnicodeIter_Type),
            std::ptr::addr_of_mut!(PyWrapperDescr_Type),
            std::ptr::addr_of_mut!(PyZip_Type),
        ];
        for ty in type_objects {
            (**ty).ob_type = type_ptr;
            if (**ty).tp_base.is_null() && *ty != std::ptr::addr_of_mut!(PyBaseObject_Type) {
                (**ty).tp_base = std::ptr::addr_of_mut!(PyBaseObject_Type);
            }
            (**ty).tp_flags |= PY_TPFLAGS_READY;
        }
        // Baseline tp_basicsize/tp_itemsize values mirror CPython 3.14 ABI on 64-bit
        // builds; Cython extension imports validate these and fail hard when they stay
        // at the default compact-object placeholder values.
        PyBaseObject_Type.tp_basicsize = 16;
        PyBaseObject_Type.tp_itemsize = 0;
        PyType_Type.tp_basicsize = 936;
        PyType_Type.tp_itemsize = 40;
        PyLong_Type.tp_basicsize = 24;
        PyLong_Type.tp_itemsize = 4;
        PyBool_Type.tp_basicsize = 24;
        PyBool_Type.tp_itemsize = 4;
        PyFloat_Type.tp_basicsize = 24;
        PyFloat_Type.tp_itemsize = 0;
        PyBytes_Type.tp_basicsize = 33;
        PyBytes_Type.tp_itemsize = 1;
        PyUnicode_Type.tp_basicsize = 64;
        PyUnicode_Type.tp_itemsize = 0;
        PyTuple_Type.tp_basicsize = 32;
        PyTuple_Type.tp_itemsize = 8;
        PyList_Type.tp_basicsize = 40;
        PyList_Type.tp_itemsize = 0;
        PyDict_Type.tp_basicsize = 48;
        PyDict_Type.tp_itemsize = 0;
        PySet_Type.tp_basicsize = 200;
        PySet_Type.tp_itemsize = 0;
        PySlice_Type.tp_basicsize = 40;
        PySlice_Type.tp_itemsize = 0;
        PyModule_Type.tp_basicsize = 56;
        PyModule_Type.tp_itemsize = 0;
        PyMethod_Type.tp_basicsize = 48;
        PyMethod_Type.tp_itemsize = 0;
        PyCapsule_Type.tp_basicsize = 64;
        PyCapsule_Type.tp_itemsize = 0;
        PyBaseObject_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_READY;
        PyType_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_TYPE_SUBCLASS | PY_TPFLAGS_READY;

        PyLong_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_LONG_SUBCLASS | PY_TPFLAGS_READY;
        PyBool_Type.tp_flags |= PY_TPFLAGS_LONG_SUBCLASS | PY_TPFLAGS_READY;
        PyList_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_LIST_SUBCLASS | PY_TPFLAGS_READY;
        PyTuple_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_TUPLE_SUBCLASS | PY_TPFLAGS_READY;
        PyBytes_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_BYTES_SUBCLASS | PY_TPFLAGS_READY;
        PyByteArray_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_READY;
        PyUnicode_Type.tp_flags |=
            PY_TPFLAGS_BASETYPE | PY_TPFLAGS_UNICODE_SUBCLASS | PY_TPFLAGS_READY;
        PyDict_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_DICT_SUBCLASS | PY_TPFLAGS_READY;
        PyNone_Type.tp_flags |= PY_TPFLAGS_READY;

        _Py_NoneStruct.ob_type = std::ptr::addr_of_mut!(PyNone_Type).cast();
        _Py_NotImplementedStruct.ob_type = std::ptr::addr_of_mut!(PyBaseObject_Type).cast();
        _Py_EllipsisObject.ob_type = std::ptr::addr_of_mut!(PyEllipsis_Type).cast();
        _Py_FalseStruct.ob_type = std::ptr::addr_of_mut!(PyBool_Type).cast();
        _Py_TrueStruct.ob_type = std::ptr::addr_of_mut!(PyBool_Type).cast();

        macro_rules! init_exception_symbol {
            ($symbol:ident, $name:literal) => {
                ensure_cpython_exception_symbol(std::ptr::addr_of_mut!($symbol), type_ptr);
            };
        }
        for_each_cpython_exception_symbol!(init_exception_symbol);
        PyExc_EnvironmentError = PyExc_OSError;
        PyExc_IOError = PyExc_OSError;
        PyExc_WindowsError = PyExc_OSError;
    });
}

thread_local! {
    static ACTIVE_CPYTHON_INIT_CONTEXT: Cell<*mut ModuleCapiContext> = const { Cell::new(std::ptr::null_mut()) };
    static CPYTHON_REPR_STACK: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

const CPYTHON_THREAD_STATE_COMPAT_SIZE: usize = 4096;

#[repr(C, align(16))]
struct CpythonThreadStateCompat {
    prev: *mut c_void,
    next: *mut c_void,
    interp: *mut c_void,
    _bytes: [u8; CPYTHON_THREAD_STATE_COMPAT_SIZE - (3 * std::mem::size_of::<*mut c_void>())],
}

static mut MAIN_THREAD_STATE_STORAGE: CpythonThreadStateCompat = CpythonThreadStateCompat {
    prev: std::ptr::null_mut(),
    next: std::ptr::null_mut(),
    interp: std::ptr::null_mut(),
    _bytes: [0; CPYTHON_THREAD_STATE_COMPAT_SIZE - (3 * std::mem::size_of::<*mut c_void>())],
};
static CURRENT_THREAD_STATE_PTR: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_THREAD_STATE_ALLOCATIONS: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
static CPYTHON_THREAD_LOCK_REGISTRY: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
static CPYTHON_THREAD_STACK_SIZE: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_THREAD_NEXT_IDENT: AtomicU64 = AtomicU64::new(1);
static CPYTHON_THREAD_TLS_NEXT_KEY: AtomicUsize = AtomicUsize::new(1);
static CPYTHON_THREAD_TLS_KEY_REGISTRY: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
static CPYTHON_THREAD_TLS_VALUES: OnceLock<Mutex<HashMap<(u64, usize), usize>>> = OnceLock::new();
static CPYTHON_THREAD_TSS_REGISTRY: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
static CPYTHON_THREAD_TSS_VALUES: OnceLock<Mutex<HashMap<(u64, usize), usize>>> = OnceLock::new();
static CPYTHON_PENDING_CALLS: OnceLock<Mutex<VecDeque<CpythonPendingCall>>> = OnceLock::new();
static CPYTHON_ATEXIT_CALLBACKS: OnceLock<Mutex<Vec<unsafe extern "C" fn()>>> = OnceLock::new();
static CPYTHON_RECURSION_LIMIT: AtomicI64 = AtomicI64::new(1000);
static CPYTHON_VERSION_TEXT: OnceLock<CString> = OnceLock::new();
static CPYTHON_BUILD_INFO_TEXT: OnceLock<CString> = OnceLock::new();
static CPYTHON_COMPILER_TEXT: OnceLock<CString> = OnceLock::new();
static CPYTHON_PLATFORM_TEXT: OnceLock<CString> = OnceLock::new();
static CPYTHON_PROGRAM_NAME_WIDE: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_PROGRAM_FULL_PATH_WIDE: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_PYTHON_HOME_WIDE: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_PATH_WIDE: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_PREFIX_WIDE: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_EXEC_PREFIX_WIDE: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_ARGC: AtomicI64 = AtomicI64::new(0);
static CPYTHON_ARGV: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_IS_INITIALIZED: AtomicUsize = AtomicUsize::new(1);
static CPYTHON_IS_FINALIZING: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_CONSTANT_ZERO_PTR: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_CONSTANT_ONE_PTR: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_CONSTANT_EMPTY_STR_PTR: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_CONSTANT_EMPTY_BYTES_PTR: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_CONSTANT_EMPTY_TUPLE_PTR: AtomicUsize = AtomicUsize::new(0);
static MAIN_INTERPRETER_STATE_TOKEN: u8 = 0;
static CPYTHON_INTERPRETER_STATE_ALLOCATIONS: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
static CPYTHON_STRUCTSEQ_TYPE_REGISTRY: OnceLock<Mutex<HashMap<usize, CpythonStructSeqTypeInfo>>> =
    OnceLock::new();
static CPYTHON_HEAP_TYPE_REGISTRY: OnceLock<Mutex<HashMap<usize, CpythonHeapTypeInfo>>> =
    OnceLock::new();
static CPYTHON_INTERNED_UNICODE_REGISTRY: OnceLock<Mutex<CpythonInternedUnicodeRegistry>> =
    OnceLock::new();

struct CpythonThreadLock {
    state: Mutex<bool>,
    condvar: Condvar,
}

#[repr(C)]
struct CpythonThreadTss {
    initialized: c_int,
    key: usize,
}

struct CpythonStructSeqTypeInfo {
    field_count: usize,
    _visible_count: usize,
    _name: CString,
}

struct CpythonHeapTypeInfo {
    _owned_name: CString,
    qualname: String,
    module_name: String,
    module_ptr: usize,
    module_def_ptr: usize,
    token: usize,
    type_data_size: isize,
    slots: HashMap<c_int, usize>,
}

#[derive(Clone, Copy)]
struct CpythonPendingCall {
    func: unsafe extern "C" fn(*mut c_void) -> c_int,
    arg: usize,
}

#[derive(Default)]
struct CpythonInternedUnicodeRegistry {
    by_text: HashMap<String, usize>,
    by_ptr: HashMap<usize, String>,
}

fn cpython_main_thread_state_ptr() -> usize {
    // SAFETY: main thread-state storage is process-global static memory.
    unsafe {
        MAIN_THREAD_STATE_STORAGE.interp = cpython_main_interpreter_state_ptr() as *mut c_void;
    }
    (&raw mut MAIN_THREAD_STATE_STORAGE) as usize
}

fn cpython_thread_state_allocations() -> &'static Mutex<HashSet<usize>> {
    CPYTHON_THREAD_STATE_ALLOCATIONS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn cpython_interned_unicode_registry() -> &'static Mutex<CpythonInternedUnicodeRegistry> {
    CPYTHON_INTERNED_UNICODE_REGISTRY
        .get_or_init(|| Mutex::new(CpythonInternedUnicodeRegistry::default()))
}

fn cpython_lookup_interned_unicode_ptr(text: &str) -> Option<*mut c_void> {
    cpython_interned_unicode_registry()
        .lock()
        .ok()
        .and_then(|registry| registry.by_text.get(text).copied())
        .map(|raw| raw as *mut c_void)
}

fn cpython_lookup_interned_unicode_text(ptr: *mut c_void) -> Option<String> {
    cpython_interned_unicode_registry()
        .lock()
        .ok()
        .and_then(|registry| registry.by_ptr.get(&(ptr as usize)).cloned())
}

fn cpython_register_interned_unicode(text: &str, ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    if let Ok(mut registry) = cpython_interned_unicode_registry().lock() {
        registry.by_text.insert(text.to_string(), ptr as usize);
        registry.by_ptr.insert(ptr as usize, text.to_string());
    }
}

fn cpython_is_interned_unicode_ptr(ptr: *mut c_void) -> bool {
    if ptr.is_null() {
        return false;
    }
    cpython_interned_unicode_registry()
        .lock()
        .ok()
        .is_some_and(|registry| registry.by_ptr.contains_key(&(ptr as usize)))
}

fn cpython_thread_lock_registry() -> &'static Mutex<HashSet<usize>> {
    CPYTHON_THREAD_LOCK_REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

fn cpython_thread_tls_key_registry() -> &'static Mutex<HashSet<usize>> {
    CPYTHON_THREAD_TLS_KEY_REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

fn cpython_thread_tls_values() -> &'static Mutex<HashMap<(u64, usize), usize>> {
    CPYTHON_THREAD_TLS_VALUES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cpython_thread_tss_registry() -> &'static Mutex<HashSet<usize>> {
    CPYTHON_THREAD_TSS_REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

fn cpython_thread_tss_values() -> &'static Mutex<HashMap<(u64, usize), usize>> {
    CPYTHON_THREAD_TSS_VALUES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cpython_pending_calls() -> &'static Mutex<VecDeque<CpythonPendingCall>> {
    CPYTHON_PENDING_CALLS.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn cpython_atexit_callbacks() -> &'static Mutex<Vec<unsafe extern "C" fn()>> {
    CPYTHON_ATEXIT_CALLBACKS.get_or_init(|| Mutex::new(Vec::new()))
}

fn cpython_leak_wide_string(text: &str) -> *mut Cwchar {
    let mut units = cpython_string_to_wide_units(text);
    units.push(0);
    let boxed = units.into_boxed_slice();
    Box::into_raw(boxed).cast::<Cwchar>()
}

fn cpython_set_wide_storage(storage: &AtomicUsize, text: &str) -> *mut Cwchar {
    let pointer = cpython_leak_wide_string(text);
    storage.store(pointer as usize, Ordering::Relaxed);
    pointer
}

fn cpython_get_or_init_wide_storage(
    storage: &AtomicUsize,
    fallback: impl FnOnce() -> String,
) -> *mut Cwchar {
    let current = storage.load(Ordering::Relaxed) as *mut Cwchar;
    if !current.is_null() {
        return current;
    }
    cpython_set_wide_storage(storage, &fallback())
}

fn cpython_read_sys_string(name: &str) -> Option<String> {
    let mut value = None;
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        if let Ok(sys_module) = cpython_sys_module_obj(context)
            && let Object::Module(data) = &*sys_module.kind()
            && let Some(Value::Str(text)) = data.globals.get(name)
        {
            value = Some(text.clone());
        }
    });
    value
}

fn cpython_read_sys_path_string() -> Option<String> {
    let mut value = None;
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        if let Ok(sys_module) = cpython_sys_module_obj(context)
            && let Object::Module(data) = &*sys_module.kind()
            && let Some(Value::List(path_list)) = data.globals.get("path")
            && let Object::List(entries) = &*path_list.kind()
        {
            #[cfg(windows)]
            let delimiter = ';';
            #[cfg(not(windows))]
            let delimiter = ':';
            let joined = entries
                .iter()
                .filter_map(|entry| match entry {
                    Value::Str(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(&delimiter.to_string());
            value = Some(joined);
        }
    });
    value
}

fn cpython_store_argv_wide(arguments: &[String]) {
    let argc = arguments.len() as i64;
    let pointers: Vec<*mut Cwchar> = arguments
        .iter()
        .map(|arg| cpython_leak_wide_string(arg))
        .collect();
    let argv = if pointers.is_empty() {
        std::ptr::null_mut()
    } else {
        let boxed = pointers.into_boxed_slice();
        Box::into_raw(boxed).cast::<*mut Cwchar>()
    };
    CPYTHON_ARGC.store(argc, Ordering::Relaxed);
    CPYTHON_ARGV.store(argv as usize, Ordering::Relaxed);
}

fn cpython_collect_sys_argv() -> Option<Vec<String>> {
    let mut argv = None;
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        if let Ok(sys_module) = cpython_sys_module_obj(context)
            && let Object::Module(data) = &*sys_module.kind()
            && let Some(Value::List(list_obj)) = data.globals.get("argv")
            && let Object::List(entries) = &*list_obj.kind()
        {
            let mut extracted = Vec::with_capacity(entries.len());
            for entry in entries {
                if let Value::Str(text) = entry {
                    extracted.push(text.clone());
                }
            }
            argv = Some(extracted);
        }
    });
    argv
}

fn cpython_get_or_init_constant_ptr(
    storage: &AtomicUsize,
    init: impl FnOnce() -> *mut c_void,
) -> *mut c_void {
    let current = storage.load(Ordering::Relaxed) as *mut c_void;
    if !current.is_null() {
        return current;
    }
    let value = init();
    if value.is_null() {
        return std::ptr::null_mut();
    }
    storage.store(value as usize, Ordering::Relaxed);
    value
}

fn cpython_main_interpreter_state_ptr() -> usize {
    std::ptr::addr_of!(MAIN_INTERPRETER_STATE_TOKEN) as usize
}

fn cpython_interpreter_state_allocations() -> &'static Mutex<HashSet<usize>> {
    CPYTHON_INTERPRETER_STATE_ALLOCATIONS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn cpython_structseq_registry() -> &'static Mutex<HashMap<usize, CpythonStructSeqTypeInfo>> {
    CPYTHON_STRUCTSEQ_TYPE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cpython_heap_type_registry() -> &'static Mutex<HashMap<usize, CpythonHeapTypeInfo>> {
    CPYTHON_HEAP_TYPE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cpython_is_known_interpreter_state_ptr(ptr: usize) -> bool {
    if ptr == 0 || ptr == cpython_main_interpreter_state_ptr() {
        return ptr != 0;
    }
    cpython_interpreter_state_allocations()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&ptr))
}

fn cpython_current_thread_state_ptr() -> usize {
    let current = CURRENT_THREAD_STATE_PTR.load(Ordering::Relaxed);
    if current != 0 {
        return current;
    }
    let main_ptr = cpython_main_thread_state_ptr();
    CURRENT_THREAD_STATE_PTR
        .compare_exchange(0, main_ptr, Ordering::Relaxed, Ordering::Relaxed)
        .ok();
    CURRENT_THREAD_STATE_PTR.load(Ordering::Relaxed)
}

fn cpython_is_known_thread_state_ptr(ptr: usize) -> bool {
    if ptr == 0 || ptr == cpython_main_thread_state_ptr() {
        return ptr != 0;
    }
    cpython_thread_state_allocations()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&ptr))
}

fn cpython_current_thread_ident_u64() -> u64 {
    let ident = vm_current_thread_ident();
    if ident >= 0 {
        ident as u64
    } else {
        ident.unsigned_abs()
    }
}

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double;
    fn strtol(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_long;
    fn strtoul(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_ulong;
}

struct ModuleCapiContext {
    vm: *mut Vm,
    module: ObjRef,
    run_capsule_destructors_on_drop: bool,
    strict_capsule_refcount: bool,
    keep_cpython_allocations_on_drop: bool,
    next_object_handle: PyrsObjectHandle,
    objects: HashMap<PyrsObjectHandle, CapiObjectSlot>,
    capsules: HashMap<PyrsObjectHandle, CapiCapsuleSlot>,
    current_error: Option<CpythonErrorState>,
    handled_exception: Option<Value>,
    last_error: Option<String>,
    first_error: Option<String>,
    scratch_strings: Vec<CString>,
    scratch_isize_arrays: Vec<Vec<isize>>,
    buffer_pins: HashMap<PyrsObjectHandle, usize>,
    cpython_objects_by_ptr: HashMap<usize, PyrsObjectHandle>,
    cpython_ptr_by_handle: HashMap<PyrsObjectHandle, *mut c_void>,
    cpython_object_handles_by_id: HashMap<u64, PyrsObjectHandle>,
    cpython_allocations: Vec<*mut CpythonCompatObject>,
    cpython_aux_allocations: Vec<*mut c_void>,
    cpython_owned_ptrs: HashSet<usize>,
    cpython_descriptors: HashMap<usize, CpythonDescriptorKind>,
    cpython_cfunction_ptr_cache: HashMap<(usize, usize, usize, usize), *mut c_void>,
    cpython_builtin_cfunction_ptr_cache: HashMap<BuiltinFunction, *mut c_void>,
    cpython_builtin_method_defs: HashMap<BuiltinFunction, *mut CpythonMethodDef>,
    cpython_builtin_by_method_def: HashMap<usize, BuiltinFunction>,
    cpython_list_buffers: HashMap<PyrsObjectHandle, (*mut *mut c_void, usize)>,
    cpython_sync_in_progress: HashSet<PyrsObjectHandle>,
    module_dict_handles: HashMap<PyrsObjectHandle, ObjRef>,
    module_dict_handle_by_module_id: HashMap<u64, PyrsObjectHandle>,
    thread_state_dict_handle: Option<PyrsObjectHandle>,
    interpreter_state_dict_handle: Option<PyrsObjectHandle>,
    codec_error_handlers: HashMap<String, usize>,
    state_modules_by_def: HashMap<usize, usize>,
    exception_type_ptr_by_name: HashMap<String, usize>,
}

impl Drop for ModuleCapiContext {
    fn drop(&mut self) {
        self.codec_error_handlers.clear();
        if !self.vm.is_null() && !self.buffer_pins.is_empty() {
            let mut stale_pins: Vec<(ObjRef, usize)> = Vec::new();
            for (handle, pins) in &self.buffer_pins {
                if *pins == 0 {
                    continue;
                }
                if let Some(value) = self.object_value(*handle)
                    && let Some(source) = Self::mutable_buffer_source_from_value(&value)
                {
                    stale_pins.push((source, *pins));
                }
            }
            // SAFETY: VM pointer is valid for the C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            for (source, pins) in stale_pins {
                vm.heap.unpin_external_buffer_source_by_count(&source, pins);
            }
        }
        let mut capsules = HashMap::new();
        std::mem::swap(&mut capsules, &mut self.capsules);
        for (handle, slot) in capsules.into_iter() {
            let capsule_ptr = self
                .cpython_ptr_by_handle
                .get(&handle)
                .copied()
                .unwrap_or(std::ptr::null_mut());
            let pinned = if self.vm.is_null() {
                false
            } else {
                let ptr = self.cpython_ptr_by_handle.get(&handle).copied();
                if let Some(ptr) = ptr {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    vm.extension_pinned_cpython_allocation_set
                        .contains(&(ptr as usize))
                } else {
                    false
                }
            };
            if pinned {
                continue;
            }
            if slot.exported_name.is_some() {
                continue;
            }
            if self.run_capsule_destructors_on_drop
                && let Some(cpython_destructor) = slot.cpython_destructor
                && !capsule_ptr.is_null()
            {
                // SAFETY: destructor pointer was provided by extension code.
                unsafe {
                    cpython_destructor(capsule_ptr);
                }
            }
            if self.run_capsule_destructors_on_drop
                && let Some(destructor) = slot.destructor
            {
                // SAFETY: destructor pointer was provided by extension code.
                unsafe {
                    destructor(slot.pointer as *mut c_void, slot.context as *mut c_void);
                }
            }
        }
        let mut escaped_handles: HashSet<PyrsObjectHandle> = HashSet::new();
        let mut preserve_aux_allocations = false;
        let drained_cpython_allocations = std::mem::take(&mut self.cpython_allocations);
        for raw in drained_cpython_allocations {
            const MIN_VALID_PTR: usize = 0x1_0000_0000;
            let raw_addr = raw as usize;
            if raw_addr < MIN_VALID_PTR || raw_addr % std::mem::align_of::<CpythonObjectHead>() != 0
            {
                if std::env::var_os("PYRS_TRACE_CPY_DROP").is_some() {
                    eprintln!(
                        "[cpy-drop] skipping invalid compat allocation ptr={:p}",
                        raw.cast::<c_void>()
                    );
                }
                continue;
            }
            if self.keep_cpython_allocations_on_drop && !self.vm.is_null() {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *self.vm };
                if vm
                    .extension_pinned_cpython_allocation_set
                    .insert(raw as usize)
                {
                    vm.extension_pinned_cpython_allocations.push(raw.cast());
                }
                if let Some(handle) = self.cpython_objects_by_ptr.get(&(raw as usize)).copied() {
                    escaped_handles.insert(handle);
                    if let Some(value) = self.object_value(handle) {
                        if let Some(object_id) = Self::identity_object_id(&value) {
                            vm.extension_cpython_ptr_by_object_id
                                .insert(object_id, raw as usize);
                        }
                        vm.extension_cpython_ptr_values.insert(raw as usize, value);
                    }
                }
                preserve_aux_allocations = true;
                continue;
            }
            let mut keep_pinned = false;
            let interned_unicode = cpython_is_interned_unicode_ptr(raw.cast::<c_void>());
            if !self.vm.is_null() {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *self.vm };
                keep_pinned = vm
                    .extension_pinned_cpython_allocation_set
                    .contains(&(raw as usize));
                if !keep_pinned {
                    if interned_unicode {
                        if vm
                            .extension_pinned_cpython_allocation_set
                            .insert(raw as usize)
                        {
                            vm.extension_pinned_cpython_allocations.push(raw.cast());
                        }
                        if let Some(handle) =
                            self.cpython_objects_by_ptr.get(&(raw as usize)).copied()
                        {
                            escaped_handles.insert(handle);
                            if let Some(value) = self.object_value(handle) {
                                if let Some(object_id) = Self::identity_object_id(&value) {
                                    vm.extension_cpython_ptr_by_object_id
                                        .insert(object_id, raw as usize);
                                }
                                vm.extension_cpython_ptr_values.insert(raw as usize, value);
                            }
                        }
                        preserve_aux_allocations = true;
                        keep_pinned = true;
                    }
                }
                if !keep_pinned {
                    // CPython objects that survive past this call context increment refcount.
                    // Keep those allocations process-stable instead of freeing them on context drop.
                    // SAFETY: `raw` points to a CPython-compatible object head allocation.
                    let refcount = unsafe { (*raw.cast::<CpythonObjectHead>()).ob_refcnt };
                    if refcount > 1 {
                        // Drop this context's implicit temporary ownership while preserving
                        // the externally retained references.
                        // SAFETY: `raw` points to a writable CPython object head.
                        unsafe {
                            (*raw.cast::<CpythonObjectHead>()).ob_refcnt = refcount - 1;
                        }
                        if vm
                            .extension_pinned_cpython_allocation_set
                            .insert(raw as usize)
                        {
                            vm.extension_pinned_cpython_allocations.push(raw.cast());
                        }
                        if let Some(handle) =
                            self.cpython_objects_by_ptr.get(&(raw as usize)).copied()
                        {
                            escaped_handles.insert(handle);
                            if let Some(value) = self.object_value(handle) {
                                if let Some(object_id) = Self::identity_object_id(&value) {
                                    vm.extension_cpython_ptr_by_object_id
                                        .insert(object_id, raw as usize);
                                }
                                vm.extension_cpython_ptr_values.insert(raw as usize, value);
                            }
                        }
                        preserve_aux_allocations = true;
                        keep_pinned = true;
                    }
                }
            }
            if keep_pinned {
                if !self.vm.is_null() {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    self.pin_container_children_for_vm(vm, raw);
                }
                continue;
            }
            if !self.vm.is_null() {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *self.vm };
                if let Some(value) = vm.extension_cpython_ptr_values.remove(&(raw as usize))
                    && let Some(object_id) = Self::identity_object_id(&value)
                    && vm
                        .extension_cpython_ptr_by_object_id
                        .get(&object_id)
                        .copied()
                        == Some(raw as usize)
                {
                    vm.extension_cpython_ptr_by_object_id.remove(&object_id);
                }
            }
            // SAFETY: pointers were allocated via C allocator in this context.
            unsafe {
                free(raw.cast());
            }
        }
        for (handle, (buffer, _)) in self.cpython_list_buffers.drain() {
            if buffer.is_null() {
                continue;
            }
            let keep_pinned =
                if self.keep_cpython_allocations_on_drop || escaped_handles.contains(&handle) {
                    true
                } else if self.vm.is_null() {
                    false
                } else {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    vm.extension_pinned_cpython_allocation_set
                        .contains(&(buffer as usize))
                };
            if keep_pinned {
                if !self.vm.is_null() {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    if vm
                        .extension_pinned_cpython_allocation_set
                        .insert(buffer as usize)
                    {
                        vm.extension_pinned_cpython_allocations.push(buffer.cast());
                    }
                }
                continue;
            }
            // SAFETY: list item buffers were allocated through C allocator in this context.
            unsafe {
                free(buffer.cast());
            }
        }
        for raw in self.cpython_aux_allocations.drain(..) {
            if raw.is_null() {
                continue;
            }
            let keep_pinned = if self.keep_cpython_allocations_on_drop || preserve_aux_allocations {
                true
            } else if self.vm.is_null() {
                false
            } else {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.extension_pinned_cpython_allocation_set
                    .contains(&(raw as usize))
            };
            if keep_pinned {
                if !self.vm.is_null() {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    if vm
                        .extension_pinned_cpython_allocation_set
                        .insert(raw as usize)
                    {
                        vm.extension_pinned_cpython_allocations.push(raw);
                    }
                }
                continue;
            }
            // SAFETY: auxiliary raw buffers were allocated via C allocator in this context.
            unsafe {
                free(raw);
            }
        }
    }
}

impl ModuleCapiContext {
    fn is_probable_type_object_without_metatype(object: *mut c_void) -> bool {
        const MIN_VALID_PTR: usize = 0x1_0000_0000;
        if object.is_null() {
            return false;
        }
        let object_addr = object as usize;
        if object_addr < MIN_VALID_PTR || object_addr % std::mem::align_of::<usize>() != 0 {
            return false;
        }
        // SAFETY: guarded by non-null + minimum-address + alignment checks.
        unsafe {
            let ty = object.cast::<CpythonTypeObject>();
            let Some(_head) = ty.cast::<CpythonObjectHead>().as_ref() else {
                return false;
            };
            let tp_name = (*ty).tp_name;
            if tp_name.is_null() {
                return false;
            }
            if c_name_to_string(tp_name).is_err() {
                return false;
            }
            let basicsize = (*ty).tp_basicsize;
            basicsize > 0 && basicsize < (1 << 20)
        }
    }

    fn is_probable_external_cpython_object_ptr(object: *mut c_void) -> bool {
        const MIN_VALID_PTR: usize = 0x1_0000_0000;
        if object.is_null() {
            return false;
        }
        let object_addr = object as usize;
        if object_addr < MIN_VALID_PTR || object_addr % std::mem::align_of::<usize>() != 0 {
            return false;
        }
        // SAFETY: guarded by non-null + minimum-address + alignment checks.
        unsafe {
            let Some(head) = object.cast::<CpythonObjectHead>().as_ref() else {
                return false;
            };
            let refcnt = head.ob_refcnt;
            if refcnt == 0 {
                return false;
            }
            let type_ptr = head.ob_type.cast::<CpythonTypeObject>();
            if type_ptr.is_null() {
                return Self::is_probable_type_object_without_metatype(object);
            }
            let type_addr = type_ptr as usize;
            if type_addr < MIN_VALID_PTR || type_addr % std::mem::align_of::<usize>() != 0 {
                return false;
            }
            let Some(type_head) = type_ptr.cast::<CpythonObjectHead>().as_ref() else {
                return false;
            };
            if type_head.ob_refcnt == 0 {
                return false;
            }
            let tp_name = (*type_ptr).tp_name;
            if tp_name.is_null() {
                return false;
            }
            c_name_to_string(tp_name).is_ok()
        }
    }

    fn pin_owned_child_pointer_for_vm(&mut self, vm: &mut Vm, child_ptr: *mut c_void) {
        if child_ptr.is_null() || !self.owns_cpython_allocation_ptr(child_ptr) {
            return;
        }
        if vm
            .extension_pinned_cpython_allocation_set
            .insert(child_ptr as usize)
        {
            vm.extension_pinned_cpython_allocations
                .push(child_ptr.cast::<c_void>());
        }
        if let Some(handle) = self
            .cpython_objects_by_ptr
            .get(&(child_ptr as usize))
            .copied()
            && let Some(value) = self.object_value(handle)
        {
            vm.extension_cpython_ptr_values
                .insert(child_ptr as usize, value.clone());
            if let Some(object_id) = Self::identity_object_id(&value) {
                vm.extension_cpython_ptr_by_object_id
                    .insert(object_id, child_ptr as usize);
            }
        }
    }

    fn pin_container_children_for_vm(&mut self, vm: &mut Vm, raw: *mut CpythonCompatObject) {
        if raw.is_null() {
            return;
        }
        // SAFETY: `raw` points to an owned CPython-compatible allocation.
        unsafe {
            let head = raw.cast::<CpythonObjectHead>();
            let Some(head_ref) = head.as_ref() else {
                return;
            };
            if head_ref.ob_type == std::ptr::addr_of_mut!(PyTuple_Type).cast() {
                let len = (*raw.cast::<CpythonVarObjectHead>()).ob_size.max(0) as usize;
                let items_ptr = cpython_tuple_items_ptr(raw.cast());
                for index in 0..len {
                    self.pin_owned_child_pointer_for_vm(vm, *items_ptr.add(index));
                }
                return;
            }
            if head_ref.ob_type == std::ptr::addr_of_mut!(PyList_Type).cast() {
                let list_ptr = raw.cast::<CpythonListCompatObject>();
                let len = (*list_ptr).ob_base.ob_size.max(0) as usize;
                let items_ptr = (*list_ptr).ob_item;
                if items_ptr.is_null() {
                    return;
                }
                for index in 0..len {
                    self.pin_owned_child_pointer_for_vm(vm, *items_ptr.add(index));
                }
            }
        }
    }

    fn owned_pointer_allows_proxy_fallback(object: *mut c_void) -> bool {
        // SAFETY: caller validates that `object` points to a CPython object header.
        let object_type = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if object_type.is_null() {
            return false;
        }
        let allowed = [
            std::ptr::addr_of_mut!(PyCFunction_Type),
            std::ptr::addr_of_mut!(PyMethodDescr_Type),
            std::ptr::addr_of_mut!(PyGetSetDescr_Type),
            std::ptr::addr_of_mut!(PyMemberDescr_Type),
            std::ptr::addr_of_mut!(PyWrapperDescr_Type),
        ];
        allowed.iter().any(|candidate| {
            if object_type == *candidate {
                return true;
            }
            // SAFETY: both pointers are valid type objects in this path.
            unsafe { PyType_IsSubtype(object_type.cast(), (*candidate).cast()) != 0 }
        })
    }

    fn new(vm: *mut Vm, module: ObjRef) -> Self {
        initialize_cpython_compat_type_objects();
        Self {
            vm,
            module,
            run_capsule_destructors_on_drop: true,
            strict_capsule_refcount: true,
            keep_cpython_allocations_on_drop: false,
            next_object_handle: 1,
            objects: HashMap::new(),
            capsules: HashMap::new(),
            current_error: None,
            handled_exception: None,
            last_error: None,
            first_error: None,
            scratch_strings: Vec::new(),
            scratch_isize_arrays: Vec::new(),
            buffer_pins: HashMap::new(),
            cpython_objects_by_ptr: HashMap::new(),
            cpython_ptr_by_handle: HashMap::new(),
            cpython_object_handles_by_id: HashMap::new(),
            cpython_allocations: Vec::new(),
            cpython_aux_allocations: Vec::new(),
            cpython_owned_ptrs: HashSet::new(),
            cpython_descriptors: HashMap::new(),
            cpython_cfunction_ptr_cache: HashMap::new(),
            cpython_builtin_cfunction_ptr_cache: HashMap::new(),
            cpython_builtin_method_defs: HashMap::new(),
            cpython_builtin_by_method_def: HashMap::new(),
            cpython_list_buffers: HashMap::new(),
            cpython_sync_in_progress: HashSet::new(),
            module_dict_handles: HashMap::new(),
            module_dict_handle_by_module_id: HashMap::new(),
            thread_state_dict_handle: None,
            interpreter_state_dict_handle: None,
            codec_error_handlers: HashMap::new(),
            state_modules_by_def: HashMap::new(),
            exception_type_ptr_by_name: HashMap::new(),
        }
    }

    fn ensure_thread_state_dict_pointer(&mut self) -> *mut c_void {
        if let Some(handle) = self.thread_state_dict_handle
            && let Some(ptr) = self.cpython_ptr_by_handle.get(&handle).copied()
        {
            return ptr;
        }
        if self.vm.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *self.vm };
        let ptr = self.alloc_cpython_ptr_for_value(vm.heap.alloc_dict(Vec::new()));
        if let Some(handle) = self.cpython_objects_by_ptr.get(&(ptr as usize)).copied() {
            self.thread_state_dict_handle = Some(handle);
        }
        ptr
    }

    fn ensure_interpreter_state_dict_pointer(&mut self) -> *mut c_void {
        if let Some(handle) = self.interpreter_state_dict_handle
            && let Some(ptr) = self.cpython_ptr_by_handle.get(&handle).copied()
        {
            return ptr;
        }
        if self.vm.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *self.vm };
        let ptr = self.alloc_cpython_ptr_for_value(vm.heap.alloc_dict(Vec::new()));
        if let Some(handle) = self.cpython_objects_by_ptr.get(&(ptr as usize)).copied() {
            self.interpreter_state_dict_handle = Some(handle);
        }
        ptr
    }

    fn error_message_from_ptr(&mut self, value: *mut c_void) -> String {
        if value.is_null() {
            return "error".to_string();
        }
        match self.cpython_value_from_ptr(value) {
            Some(Value::Str(message)) => message,
            Some(Value::Exception(err)) => {
                if err
                    .message
                    .as_deref()
                    .map_or(true, |message| message.is_empty())
                {
                    err.name
                } else {
                    format!(
                        "{}: {}",
                        err.name,
                        err.message.unwrap_or_else(|| "error".to_string())
                    )
                }
            }
            Some(other) => {
                if self.vm.is_null() {
                    "error".to_string()
                } else {
                    // SAFETY: VM pointer is valid while CPython init context is active.
                    let vm = unsafe { &mut *self.vm };
                    if let Ok(text) =
                        vm.call_builtin(BuiltinFunction::Str, vec![other.clone()], HashMap::new())
                        && let Value::Str(message) = text
                        && !message.is_empty()
                    {
                        message
                    } else {
                        format!("{} object", vm.value_type_name_for_error(&other))
                    }
                }
            }
            None => "error".to_string(),
        }
    }

    #[track_caller]
    fn set_error_state(
        &mut self,
        ptype: *mut c_void,
        pvalue: *mut c_void,
        ptraceback: *mut c_void,
        message: String,
    ) {
        if std::env::var_os("PYRS_TRACE_CPY_SET_ERROR_STATE").is_some() {
            let caller = std::panic::Location::caller();
            let exception_type_name = cpython_exception_class_name_from_ptr(ptype)
                .unwrap_or_else(|| "<none>".to_string());
            eprintln!(
                "[cpy-err-state] caller={}:{} ptype={:p}({}) exc_type={} pvalue={:p} msg={}",
                caller.file(),
                caller.line(),
                ptype,
                cpython_type_name_for_object_ptr(ptype),
                exception_type_name,
                pvalue,
                message
            );
        }
        if !ptype.is_null()
            && let Some(type_name) = cpython_exception_class_name_from_ptr(ptype)
        {
            let type_ptr = cpython_exception_type_ptr(ptype);
            if !type_ptr.is_null() {
                self.exception_type_ptr_by_name
                    .insert(type_name, type_ptr as usize);
            }
        }
        if !ptype.is_null()
            && !pvalue.is_null()
            && let Some(value) = self.cpython_value_from_ptr_or_proxy(pvalue)
        {
            self.stamp_exception_type_hint_on_value(&value, ptype);
        }
        if std::env::var_os("PYRS_TRACE_CPY_UFUNC_ERRORS").is_some() && message.contains("_UFunc") {
            let caller = std::panic::Location::caller();
            let ptype_name = cpython_exception_class_name_from_ptr(ptype)
                .unwrap_or_else(|| "<none>".to_string());
            let stack = if self.vm.is_null() {
                String::new()
            } else {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &*self.vm };
                vm.frames
                    .iter()
                    .rev()
                    .take(8)
                    .map(|frame| format!("{}@{}", frame.code.name, frame.code.filename))
                    .collect::<Vec<_>>()
                    .join(" <- ")
            };
            eprintln!(
                "[cpy-ufunc-err] caller={}:{} ptype={:p} ptype_name={} pvalue={:p} stack={}",
                caller.file(),
                caller.line(),
                ptype,
                ptype_name,
                pvalue,
                stack
            );
            if let Some(value) = self.cpython_value_from_ptr_or_proxy(pvalue) {
                eprintln!(
                    "[cpy-ufunc-err-value] {}",
                    cpython_debug_ufunc_exception_summary(&value)
                );
            }
        }
        self.current_error = Some(CpythonErrorState {
            ptype,
            pvalue,
            ptraceback,
        });
        self.set_error_message(message);
    }

    fn exception_name_from_value(&self, value: &Value) -> Option<String> {
        match value {
            Value::Exception(exception_obj) => Some(exception_obj.name.clone()),
            Value::ExceptionType(name) => Some(name.clone()),
            Value::Class(class_obj) => match &*class_obj.kind() {
                Object::Class(class_data) => Some(class_data.name.clone()),
                _ => None,
            },
            Value::Instance(instance_obj) => {
                let Object::Instance(instance_data) = &*instance_obj.kind() else {
                    return None;
                };
                let Object::Class(class_data) = &*instance_data.class.kind() else {
                    return None;
                };
                Some(class_data.name.clone())
            }
            _ => None,
        }
    }

    fn stamp_exception_type_hint_on_value(&mut self, value: &Value, ptype: *mut c_void) {
        if ptype.is_null() {
            return;
        }
        let raw_ptr = ptype as usize;
        let raw_ptr_i64 = raw_ptr as i64;
        match value {
            Value::Exception(exception_obj) => {
                exception_obj.attrs.borrow_mut().insert(
                    CPY_EXCEPTION_TYPE_PTR_ATTR.to_string(),
                    Value::Int(raw_ptr_i64),
                );
            }
            Value::Instance(instance_obj) => {
                if let Object::Instance(instance_data) = &mut *instance_obj.kind_mut() {
                    instance_data.attrs.insert(
                        CPY_EXCEPTION_TYPE_PTR_ATTR.to_string(),
                        Value::Int(raw_ptr_i64),
                    );
                }
            }
            _ => {}
        }
        if let Some(name) = self.exception_name_from_value(value) {
            self.exception_type_ptr_by_name.insert(name, raw_ptr);
        }
    }

    #[track_caller]
    fn set_error_message(&mut self, message: String) {
        let is_reentry_guard = message == "cannot load module more than once per process"
            || message == "extension returned null module";
        if self.first_error.is_none() {
            if !is_reentry_guard {
                self.first_error = Some(message.clone());
            }
        } else if is_reentry_guard {
            // Keep first_error pinned to the earliest meaningful diagnostic.
        } else if self.first_error.as_deref().is_some_and(|first| {
            first == "cannot load module more than once per process"
                || first == "extension returned null module"
        }) {
            self.first_error = Some(message.clone());
        }
        if self.last_error.is_some() && is_reentry_guard {
            return;
        }
        if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
            let caller = std::panic::Location::caller();
            eprintln!(
                "[cpy-err] {} (at {}:{})",
                message,
                caller.file(),
                caller.line()
            );
        }
        self.last_error = Some(message);
    }

    #[track_caller]
    fn set_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.current_error = Some(CpythonErrorState {
            ptype: unsafe { PyExc_RuntimeError },
            pvalue: std::ptr::null_mut(),
            ptraceback: std::ptr::null_mut(),
        });
        self.set_error_message(message);
    }

    fn clear_error(&mut self) {
        self.current_error = None;
        self.last_error = None;
        self.first_error = None;
    }

    fn fetch_error_state(&mut self) -> CpythonErrorState {
        let state = self.current_error.take().unwrap_or(CpythonErrorState {
            ptype: std::ptr::null_mut(),
            pvalue: std::ptr::null_mut(),
            ptraceback: std::ptr::null_mut(),
        });
        self.last_error = None;
        self.first_error = None;
        state
    }

    fn restore_error_state(&mut self, state: CpythonErrorState) {
        if state.ptype.is_null() && state.pvalue.is_null() && state.ptraceback.is_null() {
            self.clear_error();
            return;
        }
        let message = self.error_message_from_ptr(state.pvalue);
        self.current_error = Some(state);
        self.set_error_message(message);
    }

    fn handled_exception_get(&self) -> Option<Value> {
        self.handled_exception.clone()
    }

    fn handled_exception_set(&mut self, value: Option<Value>) {
        self.handled_exception = match value {
            Some(Value::None) | None => None,
            Some(value) => Some(value),
        };
    }

    fn allocate_handle(&mut self) -> PyrsObjectHandle {
        let handle = self.next_object_handle;
        self.next_object_handle = self.next_object_handle.wrapping_add(1);
        if self.next_object_handle == 0 {
            self.next_object_handle = 1;
        }
        handle
    }

    fn alloc_object(&mut self, value: Value) -> PyrsObjectHandle {
        if let Some(object_id) = Self::identity_object_id(&value)
            && let Some(existing) = self.cpython_object_handles_by_id.get(&object_id).copied()
            && let Some(slot) = self.objects.get_mut(&existing)
        {
            slot.refcount = slot.refcount.saturating_add(1);
            return existing;
        }
        let handle = self.allocate_handle();
        if let Some(object_id) = Self::identity_object_id(&value) {
            self.cpython_object_handles_by_id.insert(object_id, handle);
        }
        self.objects
            .insert(handle, CapiObjectSlot { value, refcount: 1 });
        handle
    }

    fn alloc_cpython_ptr_for_handle(&mut self, handle: PyrsObjectHandle) -> *mut c_void {
        if let Some(existing) = self.cpython_ptr_by_handle.get(&handle).copied() {
            if std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some() {
                eprintln!("[cpy-ptr] reuse handle={} ptr={:p}", handle, existing);
            }
            return existing.cast();
        }
        let capsule_state = self.capsules.get(&handle).map(|slot| {
            (
                slot.refcount.max(1) as isize,
                slot.pointer as *mut c_void,
                slot.name
                    .as_ref()
                    .map_or(std::ptr::null(), |name| name.as_ptr()),
                slot.context as *mut c_void,
                slot.cpython_destructor,
            )
        });
        let (
            refcount,
            ob_type,
            tuple_items,
            list_items,
            dict_len,
            bytes_payload,
            class_state,
            float_value,
        ) = match self.objects.get(&handle).map(|slot| {
            (
                slot.refcount.max(1) as isize,
                cpython_type_for_value(&slot.value),
                match &slot.value {
                    Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                        Object::Tuple(items) => Some(items.clone()),
                        _ => None,
                    },
                    _ => None,
                },
                match &slot.value {
                    Value::List(list_obj) => match &*list_obj.kind() {
                        Object::List(items) => Some(items.clone()),
                        _ => None,
                    },
                    _ => None,
                },
                match &slot.value {
                    Value::Dict(dict_obj) => match &*dict_obj.kind() {
                        Object::Dict(entries) => Some(entries.len()),
                        _ => None,
                    },
                    _ => None,
                },
                match &slot.value {
                    Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                        Object::Bytes(values) => Some(values.clone()),
                        _ => None,
                    },
                    Value::ByteArray(bytes_obj) => match &*bytes_obj.kind() {
                        Object::ByteArray(values) => Some(values.clone()),
                        _ => None,
                    },
                    _ => None,
                },
                match &slot.value {
                    Value::Class(class_obj) => match &*class_obj.kind() {
                        Object::Class(class_data) => {
                            Some((class_data.name.clone(), class_data.attrs.clone()))
                        }
                        _ => None,
                    },
                    _ => None,
                },
                match &slot.value {
                    Value::Float(value) => Some(*value),
                    _ => None,
                },
            )
        }) {
            Some(state) => state,
            None if capsule_state.is_some() => {
                (1, std::ptr::null_mut(), None, None, None, None, None, None)
            }
            None => {
                self.set_error(format!("invalid object handle {handle}"));
                return std::ptr::null_mut();
            }
        };
        let raw = if let Some((capsule_refcount, pointer, name, context, cpython_destructor)) =
            capsule_state
        {
            // SAFETY: allocate storage for CPython capsule-compatible header.
            let raw_capsule = unsafe { malloc(std::mem::size_of::<CpythonCapsuleCompatObject>()) }
                .cast::<CpythonCapsuleCompatObject>();
            if raw_capsule.is_null() {
                self.set_error("out of memory allocating CPython capsule compat object");
                return std::ptr::null_mut();
            }
            // SAFETY: initialize capsule header and payload fields.
            unsafe {
                raw_capsule.write(CpythonCapsuleCompatObject {
                    ob_base: CpythonObjectHead {
                        ob_refcnt: capsule_refcount,
                        ob_type: std::ptr::addr_of_mut!(PyCapsule_Type).cast(),
                    },
                    pointer,
                    name,
                    context,
                    destructor: cpython_destructor,
                });
            }
            raw_capsule.cast::<CpythonCompatObject>()
        } else if let Some(value) = float_value {
            // SAFETY: allocate storage for CPython float-compatible header.
            let raw_float = unsafe { malloc(std::mem::size_of::<CpythonFloatCompatObject>()) }
                .cast::<CpythonFloatCompatObject>();
            if raw_float.is_null() {
                self.set_error("out of memory allocating CPython float compat object");
                return std::ptr::null_mut();
            }
            // SAFETY: initialize float header and payload fields.
            unsafe {
                raw_float.write(CpythonFloatCompatObject {
                    ob_base: CpythonObjectHead {
                        ob_refcnt: refcount,
                        ob_type,
                    },
                    ob_fval: value,
                });
            }
            raw_float.cast::<CpythonCompatObject>()
        } else if let Some(dict_len) = dict_len {
            // SAFETY: allocate storage for CPython dict-compatible header.
            let raw_dict = unsafe { malloc(std::mem::size_of::<CpythonDictCompatObject>()) }
                .cast::<CpythonDictCompatObject>();
            if raw_dict.is_null() {
                self.set_error("out of memory allocating CPython dict compat object");
                return std::ptr::null_mut();
            }
            // Allocate a tiny stable keys sentinel so native code that touches `ma_keys`
            // does not immediately dereference NULL.
            let keys_stub = unsafe { calloc(1, std::mem::size_of::<u64>()) };
            if keys_stub.is_null() {
                // SAFETY: `raw_dict` was allocated above and is owned here.
                unsafe {
                    free(raw_dict.cast());
                }
                self.set_error("out of memory allocating CPython dict keys stub");
                return std::ptr::null_mut();
            }
            self.cpython_aux_allocations.push(keys_stub);
            self.cpython_owned_ptrs.insert(keys_stub as usize);
            // SAFETY: initialize dict header fields.
            unsafe {
                raw_dict.write(CpythonDictCompatObject {
                    ob_base: CpythonObjectHead {
                        ob_refcnt: refcount,
                        ob_type,
                    },
                    ma_used: dict_len as isize,
                    ma_watcher_tag: 0,
                    ma_keys: keys_stub,
                    ma_values: std::ptr::null_mut(),
                });
            }
            raw_dict.cast::<CpythonCompatObject>()
        } else if let Some(items) = list_items.as_ref() {
            // SAFETY: allocate storage for CPython list-compatible header.
            let raw_list = unsafe { malloc(std::mem::size_of::<CpythonListCompatObject>()) }
                .cast::<CpythonListCompatObject>();
            if raw_list.is_null() {
                self.set_error("out of memory allocating CPython list compat object");
                return std::ptr::null_mut();
            }
            let mut buffer_ptr: *mut *mut c_void = std::ptr::null_mut();
            let capacity = items.len();
            if capacity > 0 {
                // SAFETY: allocate contiguous pointer array for list item storage.
                let raw_items =
                    unsafe { malloc(capacity.saturating_mul(std::mem::size_of::<*mut c_void>())) }
                        .cast::<*mut c_void>();
                if raw_items.is_null() {
                    self.set_error("out of memory allocating CPython list item buffer");
                    // SAFETY: `raw_list` was allocated above and is owned here.
                    unsafe {
                        free(raw_list.cast());
                    }
                    return std::ptr::null_mut();
                }
                buffer_ptr = raw_items;
                // SAFETY: list item buffer has `capacity` writable entries.
                unsafe {
                    for (idx, item) in items.iter().enumerate() {
                        *buffer_ptr.add(idx) = self.alloc_cpython_ptr_for_value(item.clone());
                    }
                }
            }
            // SAFETY: initialize list header fields.
            unsafe {
                raw_list.write(CpythonListCompatObject {
                    ob_base: CpythonVarObjectHead {
                        ob_base: CpythonObjectHead {
                            ob_refcnt: refcount,
                            ob_type,
                        },
                        ob_size: items.len() as isize,
                    },
                    ob_item: buffer_ptr,
                    allocated: capacity as isize,
                });
            }
            self.cpython_list_buffers
                .insert(handle, (buffer_ptr, capacity));
            raw_list.cast::<CpythonCompatObject>()
        } else if let Some(bytes) = bytes_payload.as_ref() {
            let storage_bytes = cpython_bytes_storage_bytes(bytes.len());
            // SAFETY: allocate storage for CPython bytes-compatible header + payload.
            let raw_bytes = unsafe { malloc(storage_bytes) }.cast::<CpythonBytesCompatObject>();
            if raw_bytes.is_null() {
                self.set_error("out of memory allocating CPython bytes compat object");
                return std::ptr::null_mut();
            }
            // SAFETY: initialize bytes header and payload; `storage_bytes` includes trailing NUL.
            unsafe {
                (*raw_bytes).ob_base = CpythonVarObjectHead {
                    ob_base: CpythonObjectHead {
                        ob_refcnt: refcount,
                        ob_type,
                    },
                    ob_size: bytes.len() as isize,
                };
                (*raw_bytes).ob_shash = -1;
                let data = cpython_bytes_data_ptr(raw_bytes.cast());
                if !bytes.is_empty() {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
                }
                *data.add(bytes.len()) = 0;
            }
            raw_bytes.cast::<CpythonCompatObject>()
        } else if let Some((class_name, class_attrs)) = class_state.as_ref() {
            // SAFETY: allocate storage for CPython type-compatible header.
            let raw_type = unsafe { malloc(std::mem::size_of::<CpythonTypeObject>()) }
                .cast::<CpythonTypeObject>();
            if raw_type.is_null() {
                self.set_error("out of memory allocating CPython type compat object");
                return std::ptr::null_mut();
            }
            let name_ptr = match self.alloc_owned_c_string_for_capi(class_name) {
                Ok(ptr) => ptr,
                Err(err) => {
                    // SAFETY: `raw_type` was allocated above and is owned here.
                    unsafe {
                        free(raw_type.cast());
                    }
                    self.set_error(err);
                    return std::ptr::null_mut();
                }
            };
            let dict_ptr = if self.vm.is_null() {
                std::ptr::null_mut()
            } else {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &mut *self.vm };
                let entries = class_attrs
                    .iter()
                    .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                    .collect::<Vec<_>>();
                self.alloc_cpython_ptr_for_value(vm.heap.alloc_dict(entries))
            };
            // SAFETY: `raw_type` points to writable CpythonTypeObject storage.
            unsafe {
                raw_type.write(CpythonTypeObject {
                    ob_refcnt: refcount,
                    ob_type: std::ptr::addr_of_mut!(PyType_Type).cast(),
                    ob_size: 0,
                    tp_name: name_ptr,
                    tp_basicsize: std::mem::size_of::<CpythonCompatObject>() as isize,
                    tp_itemsize: 0,
                    tp_dealloc: std::ptr::null_mut(),
                    tp_vectorcall_offset: 0,
                    tp_getattr: std::ptr::null_mut(),
                    tp_setattr: std::ptr::null_mut(),
                    tp_as_async: std::ptr::null_mut(),
                    tp_repr: std::ptr::null_mut(),
                    tp_as_number: std::ptr::null_mut(),
                    tp_as_sequence: std::ptr::null_mut(),
                    tp_as_mapping: std::ptr::null_mut(),
                    tp_hash: std::ptr::null_mut(),
                    tp_call: cpython_type_tp_call as *mut c_void,
                    tp_str: std::ptr::null_mut(),
                    tp_getattro: std::ptr::null_mut(),
                    tp_setattro: std::ptr::null_mut(),
                    tp_as_buffer: std::ptr::null_mut(),
                    tp_flags: PY_TPFLAGS_HEAPTYPE
                        | PY_TPFLAGS_BASETYPE
                        | PY_TPFLAGS_TYPE_SUBCLASS
                        | PY_TPFLAGS_READY,
                    tp_doc: std::ptr::null(),
                    tp_traverse: std::ptr::null_mut(),
                    tp_clear: std::ptr::null_mut(),
                    tp_richcompare: std::ptr::null_mut(),
                    tp_weaklistoffset: 0,
                    tp_iter: std::ptr::null_mut(),
                    tp_iternext: std::ptr::null_mut(),
                    tp_methods: std::ptr::null_mut(),
                    tp_members: std::ptr::null_mut(),
                    tp_getset: std::ptr::null_mut(),
                    tp_base: std::ptr::addr_of_mut!(PyBaseObject_Type),
                    tp_dict: dict_ptr,
                    tp_descr_get: std::ptr::null_mut(),
                    tp_descr_set: std::ptr::null_mut(),
                    tp_dictoffset: 0,
                    tp_init: std::ptr::null_mut(),
                    tp_alloc: PyType_GenericAlloc as *mut c_void,
                    tp_new: PyType_GenericNew as *mut c_void,
                    tp_free: std::ptr::null_mut(),
                    tp_is_gc: std::ptr::null_mut(),
                    tp_bases: std::ptr::null_mut(),
                    tp_mro: std::ptr::null_mut(),
                    tp_cache: std::ptr::null_mut(),
                    tp_subclasses: std::ptr::null_mut(),
                    tp_weaklist: std::ptr::null_mut(),
                    tp_del: std::ptr::null_mut(),
                    tp_version_tag: 0,
                    tp_finalize: std::ptr::null_mut(),
                    tp_vectorcall: std::ptr::null_mut(),
                    tp_watched: 0,
                    tp_versions_used: 0,
                });
            }
            raw_type.cast::<CpythonCompatObject>()
        } else {
            let tuple_len = tuple_items.as_ref().map_or(0, Vec::len);
            let storage_bytes = cpython_tuple_storage_bytes(tuple_len);
            // SAFETY: allocates raw storage for C-compatible object header.
            let raw = unsafe { malloc(storage_bytes) }.cast::<CpythonCompatObject>();
            if raw.is_null() {
                self.set_error("out of memory allocating CPython compat object");
                return std::ptr::null_mut();
            }
            // SAFETY: `raw` points to writable storage with correct layout.
            unsafe {
                raw.write(CpythonCompatObject {
                    ob_base: CpythonVarObjectHead {
                        ob_base: CpythonObjectHead {
                            ob_refcnt: refcount,
                            ob_type,
                        },
                        ob_size: tuple_len as isize,
                    },
                });
                let tuple_raw = raw.cast::<CpythonTupleCompatObject>();
                (*tuple_raw).ob_hash = -1;
                if let Some(items) = tuple_items.as_ref() {
                    let items_ptr = cpython_tuple_items_ptr(raw.cast());
                    for (idx, item) in items.iter().enumerate() {
                        *items_ptr.add(idx) = self.alloc_cpython_ptr_for_value(item.clone());
                    }
                }
            }
            raw
        };
        if raw.is_null() {
            self.set_error("out of memory allocating CPython compat object");
            return std::ptr::null_mut();
        }
        if std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some() {
            eprintln!(
                "[cpy-ptr] alloc handle={} ptr={:p}",
                handle,
                raw.cast::<c_void>()
            );
        }
        if let Some(previous) = self.cpython_objects_by_ptr.insert(raw as usize, handle)
            && std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some()
        {
            eprintln!(
                "[cpy-ptr] overwrite ptr={:p} previous_handle={} new_handle={}",
                raw.cast::<c_void>(),
                previous,
                handle
            );
        }
        self.cpython_ptr_by_handle.insert(handle, raw.cast());
        self.cpython_allocations.push(raw);
        self.cpython_owned_ptrs.insert(raw as usize);
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            if let Some(value) = self.object_value(handle) {
                vm.extension_cpython_ptr_values
                    .insert(raw as usize, value.clone());
                if let Some(object_id) = Self::identity_object_id(&value) {
                    vm.extension_cpython_ptr_by_object_id
                        .insert(object_id, raw as usize);
                }
            }
        }
        raw.cast()
    }

    pub(super) fn cpython_proxy_raw_ptr_from_value(value: &Value) -> Option<*mut c_void> {
        match value {
            Value::Class(class_obj) => {
                let Object::Class(class_data) = &*class_obj.kind() else {
                    return None;
                };
                if !is_cpython_proxy_class(class_data) {
                    return None;
                }
                match class_data.attrs.get(CPY_PROXY_PTR_ATTR) {
                    Some(Value::Int(raw_ptr)) if *raw_ptr >= 0 => {
                        Some(*raw_ptr as usize as *mut c_void)
                    }
                    _ => None,
                }
            }
            Value::Instance(instance_obj) => {
                let Object::Instance(instance_data) = &*instance_obj.kind() else {
                    return None;
                };
                let Object::Class(class_data) = &*instance_data.class.kind() else {
                    return None;
                };
                if !is_cpython_proxy_class(class_data) {
                    return None;
                }
                match instance_data.attrs.get(CPY_PROXY_PTR_ATTR) {
                    Some(Value::Int(raw_ptr)) if *raw_ptr >= 0 => {
                        Some(*raw_ptr as usize as *mut c_void)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn alloc_cpython_ptr_for_value(&mut self, value: Value) -> *mut c_void {
        if let Value::BoundMethod(bound_obj) = &value
            && let Object::BoundMethod(bound_method) = &*bound_obj.kind()
            && let Object::NativeMethod(native_method) = &*bound_method.function.kind()
            && let NativeMethodKind::ExtensionFunctionCall(function_id) = native_method.kind
            && !self.vm.is_null()
        {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            if let Some(entry) = vm.extension_callable_registry.get(&function_id)
                && let ExtensionCallableKind::CpythonMethod { method_def } = entry.kind
            {
                if std::env::var_os("PYRS_TRACE_CPY_CFUNCTION_WRAP").is_some() {
                    // SAFETY: method_def originates from extension-owned PyMethodDef table.
                    let method_name = unsafe {
                        c_name_to_string((*(method_def as *mut CpythonMethodDef)).ml_name)
                            .unwrap_or_else(|_| "<invalid>".to_string())
                    };
                    // SAFETY: method_def pointer is valid for metadata reads.
                    let method_doc = unsafe { (*(method_def as *mut CpythonMethodDef)).ml_doc };
                    eprintln!(
                        "[cpy-cfunc-wrap] function_id={} name={} method_def={:p} ml_doc={:p}",
                        function_id, method_name, method_def as *mut CpythonMethodDef, method_doc
                    );
                }
                let self_ptr =
                    self.alloc_cpython_ptr_for_value(Value::Module(bound_method.receiver.clone()));
                let cfunction_ptr = self.alloc_cpython_method_cfunction_ptr(
                    method_def as *mut CpythonMethodDef,
                    self_ptr,
                    self_ptr,
                    std::ptr::null_mut(),
                );
                if !cfunction_ptr.is_null() {
                    self.cache_cpython_ptr_value(cfunction_ptr, &value);
                    return cfunction_ptr;
                }
            }
        }
        if let Some(raw_ptr) = Self::cpython_proxy_raw_ptr_from_value(&value) {
            return raw_ptr;
        }
        if !self.vm.is_null()
            && let Some(object_id) = Self::identity_object_id(&value)
        {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            if let Some(raw_ptr) = vm
                .extension_cpython_ptr_by_object_id
                .get(&object_id)
                .copied()
            {
                if vm
                    .extension_pinned_cpython_allocation_set
                    .contains(&raw_ptr)
                    && vm.extension_cpython_ptr_values.contains_key(&raw_ptr)
                {
                    vm.extension_cpython_ptr_values
                        .insert(raw_ptr, value.clone());
                    return raw_ptr as *mut c_void;
                }
                vm.extension_cpython_ptr_by_object_id.remove(&object_id);
            }
        }
        if let Value::Class(class_obj) = &value
            && let Object::Class(class_data) = &*class_obj.kind()
            && is_cpython_proxy_class(class_data)
        {
            if std::env::var_os("PYRS_TRACE_CPY_PROXY_PTRS").is_some() {
                eprintln!(
                    "[cpy-proxy] missing raw pointer attr for proxy class id={} attrs_keys={:?}",
                    class_obj.id(),
                    class_data.attrs.keys().collect::<Vec<_>>()
                );
            }
        }
        if let Value::Class(class_obj) = &value
            && let Object::Class(class_data) = &*class_obj.kind()
            && let Some(type_ptr) = cpython_builtin_type_ptr_for_class_name(&class_data.name)
        {
            return type_ptr;
        }
        if let Value::Builtin(builtin) = &value {
            if let Some(type_ptr) = cpython_builtin_type_ptr_for_builtin(builtin) {
                return type_ptr;
            }
            let cfunction_ptr = self.alloc_cpython_builtin_cfunction_ptr(*builtin);
            if !cfunction_ptr.is_null() {
                self.cache_cpython_ptr_value(cfunction_ptr, &value);
                return cfunction_ptr;
            }
        }
        match value {
            Value::None => {
                // SAFETY: singleton addresses are process-lifetime stable.
                return std::ptr::addr_of_mut!(_Py_NoneStruct).cast();
            }
            Value::Bool(true) => {
                // SAFETY: singleton addresses are process-lifetime stable.
                return std::ptr::addr_of_mut!(_Py_TrueStruct).cast();
            }
            Value::Bool(false) => {
                // SAFETY: singleton addresses are process-lifetime stable.
                return std::ptr::addr_of_mut!(_Py_FalseStruct).cast();
            }
            _ => {}
        }
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &*self.vm };
            if vm
                .builtins
                .get("Ellipsis")
                .is_some_and(|built| built == &value)
            {
                // SAFETY: singleton addresses are process-lifetime stable.
                return std::ptr::addr_of_mut!(_Py_EllipsisObject).cast();
            }
            if vm
                .builtins
                .get("NotImplemented")
                .is_some_and(|built| built == &value)
            {
                // SAFETY: singleton addresses are process-lifetime stable.
                return std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast();
            }
        }
        let handle = self.alloc_object(value);
        self.alloc_cpython_ptr_for_handle(handle)
    }

    fn cache_cpython_ptr_value(&mut self, ptr: *mut c_void, value: &Value) {
        if ptr.is_null() || self.cpython_objects_by_ptr.contains_key(&(ptr as usize)) {
            return;
        }
        let handle = self.alloc_object(value.clone());
        self.cpython_objects_by_ptr.insert(ptr as usize, handle);
        self.cpython_ptr_by_handle.insert(handle, ptr);
    }

    fn cpython_handle_from_ptr(&mut self, object: *mut c_void) -> Option<PyrsObjectHandle> {
        if let Some(handle) = self.cpython_objects_by_ptr.get(&(object as usize)).copied() {
            return Some(handle);
        }
        if !self.cpython_owned_ptrs.contains(&(object as usize)) {
            return None;
        }
        let recovered = self
            .cpython_ptr_by_handle
            .iter()
            .find_map(|(handle, ptr)| ((*ptr as usize) == (object as usize)).then_some(*handle));
        if let Some(handle) = recovered {
            self.cpython_objects_by_ptr.insert(object as usize, handle);
        }
        recovered
    }

    fn cpython_builtin_type_value_from_ptr(&self, object: *mut c_void) -> Option<Value> {
        let type_name = cpython_builtin_type_name_for_ptr(object)?;
        if self.vm.is_null() {
            return None;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &*self.vm };
        vm.builtins.get(type_name).cloned()
    }

    fn cpython_value_from_ptr(&mut self, object: *mut c_void) -> Option<Value> {
        if object.is_null() {
            return None;
        }
        let raw = object as usize;
        // Support direct singleton pointers used by C extensions.
        if raw == std::ptr::addr_of!(_Py_NoneStruct) as usize {
            return Some(Value::None);
        }
        if raw == std::ptr::addr_of!(_Py_TrueStruct) as usize {
            return Some(Value::Bool(true));
        }
        if raw == std::ptr::addr_of!(_Py_FalseStruct) as usize {
            return Some(Value::Bool(false));
        }
        if raw == std::ptr::addr_of!(_Py_EllipsisObject) as usize {
            if !self.vm.is_null() {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &*self.vm };
                if let Some(ellipsis) = vm.builtins.get("Ellipsis").cloned() {
                    return Some(ellipsis);
                }
            }
        }
        if raw == std::ptr::addr_of!(_Py_NotImplementedStruct) as usize {
            if !self.vm.is_null() {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &*self.vm };
                if let Some(not_implemented) = vm.builtins.get("NotImplemented").cloned() {
                    return Some(not_implemented);
                }
            }
        }
        if let Some(type_value) = self.cpython_builtin_type_value_from_ptr(object) {
            return Some(type_value);
        }
        if let Some(exception_value) = cpython_exception_value_from_ptr(raw) {
            if let Value::ExceptionType(name) = &exception_value
                && !self.vm.is_null()
            {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &*self.vm };
                if let Some(class_value) = vm.builtins.get(name).cloned() {
                    return Some(class_value);
                }
            }
            return Some(exception_value);
        }
        if let Some(handle) = self.cpython_handle_from_ptr(object) {
            if !self.objects.contains_key(&handle) && self.capsules.contains_key(&handle) {
                return self.cpython_external_proxy_value(object);
            }
            self.sync_value_from_cpython_storage(handle, object);
            self.refresh_external_proxy_instance_type(handle, object);
            return self.object_value(handle);
        }
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            if let Some(value) = vm.extension_cpython_ptr_values.get(&raw).cloned() {
                return Some(value);
            }
        }
        if let Some(text) = cpython_lookup_interned_unicode_text(object) {
            return Some(Value::Str(text));
        }
        None
    }

    fn refresh_external_proxy_instance_type(
        &mut self,
        handle: PyrsObjectHandle,
        object: *mut c_void,
    ) {
        let Some(_refresh_guard) = ProxyRefreshReentryGuard::enter() else {
            return;
        };
        if object.is_null() || self.vm.is_null() || self.owns_cpython_allocation_ptr(object) {
            return;
        }
        let Some(slot) = self.objects.get(&handle) else {
            return;
        };
        let Value::Instance(instance_obj) = &slot.value else {
            return;
        };
        let (cached_class_obj, cached_class_ptr) = match &*instance_obj.kind() {
            Object::Instance(instance_data) => {
                let class_obj = instance_data.class.clone();
                let class_ptr =
                    Self::cpython_proxy_raw_ptr_from_value(&Value::Class(class_obj.clone()));
                (class_obj, class_ptr)
            }
            _ => return,
        };
        let Some(cached_class_ptr) = cached_class_ptr else {
            return;
        };
        // SAFETY: pointer came from native extension object flow.
        let current_type_ptr = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type)
                .unwrap_or(std::ptr::null_mut())
        };
        if current_type_ptr.is_null() || current_type_ptr == cached_class_ptr {
            return;
        }
        let Some(Value::Class(updated_class)) =
            self.cpython_value_from_ptr_or_proxy(current_type_ptr)
        else {
            return;
        };
        if updated_class.id() == cached_class_obj.id() {
            return;
        }
        if std::env::var_os("PYRS_TRACE_PROXY_CLASS_SOURCE").is_some() {
            let old_name = match &*cached_class_obj.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "<non-class>".to_string(),
            };
            let new_name = match &*updated_class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "<non-class>".to_string(),
            };
            eprintln!(
                "[cpy-proxy] refresh instance class object_ptr={:p} old_class={} old_ptr={:p} new_class={} new_ptr={:p}",
                object, old_name, cached_class_ptr, new_name, current_type_ptr
            );
        }
        if let Some(slot_mut) = self.objects.get_mut(&handle)
            && let Value::Instance(instance_mut) = &mut slot_mut.value
            && let Object::Instance(instance_data_mut) = &mut *instance_mut.kind_mut()
        {
            instance_data_mut.class = updated_class;
        }
    }

    fn cpython_external_proxy_value(&mut self, object: *mut c_void) -> Option<Value> {
        if object.is_null() || self.vm.is_null() {
            return None;
        }
        // SAFETY: best-effort diagnostics/type probe on candidate external PyObject*.
        let object_type = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        let expected_type = std::ptr::addr_of_mut!(PyType_Type).cast::<CpythonTypeObject>();
        // PyType_Check(op): treat any object whose metatype is `type` (or subtype) as a type.
        // NumPy DType classes use `_DTypeMeta`, so strict pointer-equality with `PyType_Type`
        // is insufficient and would misclassify those type objects as plain instances.
        let is_type_object = if object_type.is_null() {
            Self::is_probable_type_object_without_metatype(object)
        } else if object_type == expected_type {
            true
        } else {
            // SAFETY: `object_type`/`expected_type` are candidate type pointers; subtype test
            // is guarded internally against null/unaligned/invalid pointers.
            unsafe {
                PyType_IsSubtype(object_type.cast::<c_void>(), expected_type.cast::<c_void>()) != 0
            }
        };
        if std::env::var_os("PYRS_TRACE_CPY_PROXY_PTRS").is_some() {
            let object_type_name = unsafe {
                object_type
                    .as_ref()
                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string())
            };
            if object_type == expected_type {
                // SAFETY: object_type indicates `object` has PyTypeObject layout.
                let (tp_name, tp_dict, tp_base) = unsafe {
                    let ty = object.cast::<CpythonTypeObject>();
                    let tp_name =
                        c_name_to_string((*ty).tp_name).unwrap_or_else(|_| "<invalid>".to_string());
                    (tp_name, (*ty).tp_dict, (*ty).tp_base)
                };
                eprintln!(
                    "[cpy-proxy] create proxy type-object ptr={:p} object_type={:p} object_type_name={} tp_name={} tp_dict={:p} tp_base={:p}",
                    object, object_type, object_type_name, tp_name, tp_dict, tp_base
                );
            } else {
                eprintln!(
                    "[cpy-proxy] create proxy ptr={:p} object_type={:p} object_type_name={}",
                    object, object_type, object_type_name
                );
            }
        }
        if !is_type_object && !object_type.is_null() {
            let mapped_type = self.cpython_value_from_ptr_or_proxy(object_type.cast::<c_void>());
            if let Some(Value::Class(type_proxy_class)) = mapped_type {
                let is_proxy_type_class = matches!(
                    &*type_proxy_class.kind(),
                    Object::Class(class_data) if is_cpython_proxy_class(class_data)
                );
                if !is_proxy_type_class {
                    if std::env::var_os("PYRS_TRACE_PROXY_CLASS_SOURCE").is_some() {
                        let class_name = match &*type_proxy_class.kind() {
                            Object::Class(class_data) => class_data.name.clone(),
                            _ => "<non-class>".to_string(),
                        };
                        eprintln!(
                            "[cpy-proxy] non-proxy type class fallback object_ptr={:p} type_ptr={:p} class={}",
                            object, object_type, class_name
                        );
                    }
                    // Continue through generic fallback path below.
                } else {
                    if std::env::var_os("PYRS_TRACE_PROXY_CLASS_SOURCE").is_some() {
                        let class_name = match &*type_proxy_class.kind() {
                            Object::Class(class_data) => class_data.name.clone(),
                            _ => "<non-class>".to_string(),
                        };
                        eprintln!(
                            "[cpy-proxy] instance uses type-proxy class={} object_ptr={:p} type_ptr={:p}",
                            class_name, object, object_type
                        );
                    }
                    // SAFETY: VM pointer is valid for the C-API context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    match vm
                        .heap
                        .alloc_instance(InstanceObject::new(type_proxy_class.clone()))
                    {
                        Value::Instance(instance_obj) => {
                            if let Object::Instance(instance_data) = &mut *instance_obj.kind_mut() {
                                instance_data.attrs.insert(
                                    CPY_PROXY_PTR_ATTR.to_string(),
                                    Value::Int(object as usize as i64),
                                );
                            }
                            return Some(Value::Instance(instance_obj));
                        }
                        other => return Some(other),
                    }
                }
            } else if std::env::var_os("PYRS_TRACE_PROXY_CLASS_SOURCE").is_some() {
                let owns_type_ptr = self.owns_cpython_allocation_ptr(object_type.cast::<c_void>());
                let probable_external =
                    Self::is_probable_external_cpython_object_ptr(object_type.cast::<c_void>());
                let mapped_kind = mapped_type
                    .as_ref()
                    .map(|value| match value {
                        Value::None => "None",
                        Value::Bool(_) => "Bool",
                        Value::Int(_) => "Int",
                        Value::BigInt(_) => "BigInt",
                        Value::Float(_) => "Float",
                        Value::Complex { .. } => "Complex",
                        Value::Str(_) => "Str",
                        Value::List(_) => "List",
                        Value::Tuple(_) => "Tuple",
                        Value::Dict(_) => "Dict",
                        Value::DictKeys(_) => "DictKeys",
                        Value::Set(_) => "Set",
                        Value::FrozenSet(_) => "FrozenSet",
                        Value::Slice(_) => "Slice",
                        Value::Iterator(_) => "Iterator",
                        Value::Code(_) => "Code",
                        Value::Function(_) => "Function",
                        Value::Builtin(_) => "Builtin",
                        Value::BoundMethod(_) => "BoundMethod",
                        Value::Cell(_) => "Cell",
                        Value::Class(_) => "Class",
                        Value::Instance(_) => "Instance",
                        Value::Super(_) => "Super",
                        Value::Module(_) => "Module",
                        Value::Exception(_) => "Exception",
                        Value::ExceptionType(_) => "ExceptionType",
                        Value::Generator(_) => "Generator",
                        Value::Bytes(_) => "Bytes",
                        Value::ByteArray(_) => "ByteArray",
                        Value::MemoryView(_) => "MemoryView",
                    })
                    .unwrap_or("<none>");
                eprintln!(
                    "[cpy-proxy] missing type-proxy class object_ptr={:p} type_ptr={:p} mapped={} owns_type_ptr={} probable_external={}",
                    object, object_type, mapped_kind, owns_type_ptr, probable_external
                );
                // SAFETY: diagnostics only; pointer provenance already gated by caller context.
                unsafe {
                    if let Some(type_head) = object_type.cast::<CpythonObjectHead>().as_ref() {
                        let meta_ptr = type_head.ob_type.cast::<CpythonTypeObject>();
                        let meta_refcnt = meta_ptr
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_refcnt)
                            .unwrap_or(0);
                        let tp_name_ptr = (*object_type).tp_name;
                        let tp_name = if tp_name_ptr.is_null() {
                            "<null>".to_string()
                        } else {
                            c_name_to_string(tp_name_ptr)
                                .unwrap_or_else(|_| "<invalid>".to_string())
                        };
                        eprintln!(
                            "[cpy-proxy] missing type-proxy details type_ptr={:p} type_refcnt={} meta_ptr={:p} meta_refcnt={} tp_name={}",
                            object_type, type_head.ob_refcnt, meta_ptr, meta_refcnt, tp_name
                        );
                    }
                }
            }
        }
        if std::env::var_os("PYRS_TRACE_PROXY_CLASS_SOURCE").is_some() && !is_type_object {
            let type_name = unsafe {
                object_type
                    .as_ref()
                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string())
            };
            eprintln!(
                "[cpy-proxy] instance fallback generic object_ptr={:p} type_ptr={:p} type_name={}",
                object, object_type, type_name
            );
        }
        let (proxy_name, proxy_module) = if is_type_object {
            // SAFETY: `object` is a candidate type object in this branch.
            let type_name = unsafe {
                c_name_to_string((*object.cast::<CpythonTypeObject>()).tp_name)
                    .unwrap_or_else(|_| CPY_PROXY_CLASS_NAME.to_string())
            };
            cpython_type_name_parts(&type_name)
        } else {
            (CPY_PROXY_CLASS_NAME.to_string(), None)
        };
        let proxy_base_value = if is_type_object {
            // SAFETY: `object` is a candidate type object in this branch.
            let tp_base = unsafe { (*object.cast::<CpythonTypeObject>()).tp_base };
            if tp_base.is_null() || tp_base.cast::<c_void>() == object {
                None
            } else {
                self.cpython_value_from_ptr_or_proxy(tp_base.cast::<c_void>())
            }
        } else {
            None
        };
        // SAFETY: VM pointer is valid for the C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        let mut proxy_bases = Vec::new();
        if let Some(base_value) = proxy_base_value
            && let Ok(base_class) = vm.class_from_base_value(base_value)
        {
            proxy_bases.push(base_class);
        }
        let is_base_object_type =
            is_type_object && object == std::ptr::addr_of_mut!(PyBaseObject_Type).cast::<c_void>();
        if proxy_bases.is_empty()
            && !is_base_object_type
            && let Some(Value::Class(object_class)) = vm.builtins.get("object")
        {
            proxy_bases.push(object_class.clone());
        }
        let proxy_class = match vm
            .heap
            .alloc_class(ClassObject::new(proxy_name.clone(), proxy_bases.clone()))
        {
            Value::Class(class_obj) => class_obj,
            other => return Some(other),
        };
        if let Object::Class(class_data) = &mut *proxy_class.kind_mut() {
            class_data
                .attrs
                .insert(CPY_PROXY_MARKER_ATTR.to_string(), Value::Bool(true));
            class_data
                .attrs
                .insert("__name__".to_string(), Value::Str(proxy_name.clone()));
            class_data
                .attrs
                .insert("__qualname__".to_string(), Value::Str(proxy_name.clone()));
            class_data.attrs.insert(
                "__bases__".to_string(),
                vm.heap.alloc_tuple(
                    class_data
                        .bases
                        .iter()
                        .cloned()
                        .map(Value::Class)
                        .collect::<Vec<_>>(),
                ),
            );
            if let Some(module_name) = &proxy_module {
                class_data
                    .attrs
                    .insert("__module__".to_string(), Value::Str(module_name.clone()));
            }
        }
        let proxy_mro = vm
            .build_class_mro(&proxy_class, &proxy_bases)
            .unwrap_or_else(|_| {
                let mut fallback = vec![proxy_class.clone()];
                for base in &proxy_bases {
                    if !fallback.iter().any(|entry| entry.id() == base.id()) {
                        fallback.push(base.clone());
                    }
                }
                fallback
            });
        if let Object::Class(class_data) = &mut *proxy_class.kind_mut() {
            class_data.mro = proxy_mro.clone();
            class_data.attrs.insert(
                "__mro__".to_string(),
                vm.heap
                    .alloc_tuple(proxy_mro.into_iter().map(Value::Class).collect::<Vec<_>>()),
            );
        }
        if is_type_object {
            if let Object::Class(class_data) = &mut *proxy_class.kind_mut() {
                class_data.attrs.insert(
                    CPY_PROXY_PTR_ATTR.to_string(),
                    Value::Int(object as usize as i64),
                );
            }
            return Some(Value::Class(proxy_class));
        }
        if let Object::Class(class_data) = &mut *proxy_class.kind_mut()
            && !object_type.is_null()
        {
            class_data.attrs.insert(
                CPY_PROXY_PTR_ATTR.to_string(),
                Value::Int(object_type.cast::<c_void>() as usize as i64),
            );
        }
        match vm.heap.alloc_instance(InstanceObject::new(proxy_class)) {
            Value::Instance(instance_obj) => {
                if let Object::Instance(instance_data) = &mut *instance_obj.kind_mut() {
                    instance_data.attrs.insert(
                        CPY_PROXY_PTR_ATTR.to_string(),
                        Value::Int(object as usize as i64),
                    );
                }
                Some(Value::Instance(instance_obj))
            }
            other => Some(other),
        }
    }

    #[track_caller]
    fn cpython_value_from_ptr_or_proxy(&mut self, object: *mut c_void) -> Option<Value> {
        if let Some(value) = self.cpython_value_from_ptr(object) {
            return Some(value);
        }
        if object.is_null() || self.vm.is_null() {
            return None;
        }
        let owns_allocation = self.owns_cpython_allocation_ptr(object);
        let probable_external = Self::is_probable_external_cpython_object_ptr(object);
        if !probable_external {
            if std::env::var_os("PYRS_TRACE_CPY_UNKNOWN_PTR").is_some() {
                eprintln!(
                    "[cpy-proxy-reject] ptr={:p} owns={} probable=false",
                    object, owns_allocation
                );
            }
            return None;
        }
        if owns_allocation && !Self::owned_pointer_allows_proxy_fallback(object) {
            // Unknown owned pointers are not safe to treat as proxy-backed objects.
            // Restrict owned-pointer fallback to known descriptor/cfunction surfaces.
            return None;
        }
        if !owns_allocation {
            // Keep external PyObject* proxies alive for the VM lifetime once they are
            // materialized into runtime values. Context-scoped incref/decref churn can
            // leave escaped proxy values pointing at reclaimed CPython objects.
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            let inserted = vm
                .extension_pinned_external_cpython_refs
                .insert(object as usize);
            if std::env::var_os("PYRS_TRACE_CPY_PIN").is_some() {
                eprintln!(
                    "[cpy-pin] ptr={:p} branch=external inserted={}",
                    object, inserted
                );
            }
            if inserted {
                // SAFETY: the pointer passed probability checks above and is now pinned
                // for one matching decref in Vm::drop.
                unsafe {
                    Py_IncRef(object);
                }
            }
        }
        let proxy = self.cpython_external_proxy_value(object)?;
        if owns_allocation {
            self.pin_owned_cpython_allocation_for_vm(object);
        }
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.extension_cpython_ptr_values
                .entry(object as usize)
                .or_insert_with(|| proxy.clone());
        }
        let handle = self.alloc_object(proxy.clone());
        if std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some() {
            let caller = std::panic::Location::caller();
            eprintln!(
                "[cpy-ptr] proxy-map handle={} external_ptr={:p} caller={}:{}",
                handle,
                object,
                caller.file(),
                caller.line()
            );
        }
        if let Some(previous) = self.cpython_objects_by_ptr.insert(object as usize, handle)
            && std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some()
        {
            eprintln!(
                "[cpy-ptr] overwrite external ptr={:p} previous_handle={} new_handle={}",
                object, previous, handle
            );
        }
        self.cpython_ptr_by_handle.insert(handle, object);
        Some(proxy)
    }

    fn cpython_module_obj_from_ptr(&mut self, object: *mut c_void) -> Result<ObjRef, String> {
        let value = self
            .cpython_value_from_ptr(object)
            .ok_or_else(|| "invalid CPython object pointer".to_string())?;
        match value {
            Value::Module(module) => Ok(module),
            _ => Err("CPython object is not a module".to_string()),
        }
    }

    fn module_dict_module_for_ptr(&mut self, dict_ptr: *mut c_void) -> Option<ObjRef> {
        let handle = self.cpython_handle_from_ptr(dict_ptr)?;
        self.module_dict_handles.get(&handle).cloned()
    }

    fn module_dict_handle_for_module(&self, module: &ObjRef) -> Option<PyrsObjectHandle> {
        self.module_dict_handle_by_module_id
            .get(&module.id())
            .copied()
    }

    fn sync_module_dict_set(
        &mut self,
        module: &ObjRef,
        key: &str,
        value: &Value,
    ) -> Result<(), String> {
        let Some(dict_handle) = self.module_dict_handle_for_module(module) else {
            return Ok(());
        };
        let Some(slot) = self.objects.get(&dict_handle) else {
            return Ok(());
        };
        let Value::Dict(dict_obj) = &slot.value else {
            return Ok(());
        };
        dict_set_value_checked(dict_obj, Value::Str(key.to_string()), value.clone())
            .map_err(|err| err.message)
    }

    fn alloc_aux_buffer(&mut self, size: usize) -> *mut c_void {
        // SAFETY: allocates raw storage managed by this context.
        let raw = unsafe { malloc(size) };
        if raw.is_null() {
            self.set_error("out of memory allocating CPython auxiliary buffer");
            return std::ptr::null_mut();
        }
        self.cpython_aux_allocations.push(raw);
        raw
    }

    fn alloc_owned_c_string_for_capi(&mut self, text: &str) -> Result<*const c_char, String> {
        if text.as_bytes().contains(&0) {
            return Err("string contains interior NUL byte".to_string());
        }
        let len = text.len().saturating_add(1);
        let raw = self.alloc_aux_buffer(len).cast::<u8>();
        if raw.is_null() {
            return Err("out of memory allocating C string".to_string());
        }
        // SAFETY: `raw` points to writable buffer of `len` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), raw, text.len());
            *raw.add(text.len()) = 0;
        }
        Ok(raw.cast::<c_char>())
    }

    fn builtin_runtime_name_for_capi(&mut self, builtin: BuiltinFunction) -> String {
        if self.vm.is_null() {
            return "builtin".to_string();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.builtin_runtime_name(builtin)
    }

    fn ensure_builtin_method_def(&mut self, builtin: BuiltinFunction) -> *mut CpythonMethodDef {
        if let Some(existing) = self.cpython_builtin_method_defs.get(&builtin).copied() {
            return existing;
        }
        let method_name = self.builtin_runtime_name_for_capi(builtin);
        let method_name_ptr = match self.alloc_owned_c_string_for_capi(&method_name) {
            Ok(ptr) => ptr,
            Err(err) => {
                self.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let method_def = self
            .alloc_aux_buffer(std::mem::size_of::<CpythonMethodDef>())
            .cast::<CpythonMethodDef>();
        if method_def.is_null() {
            return std::ptr::null_mut();
        }
        let method_callable: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *mut c_void,
        ) -> *mut c_void = cpython_builtin_cfunction_varargs_kwargs;
        // SAFETY: `method_def` points to writable PyMethodDef-compatible storage.
        unsafe {
            method_def.write(CpythonMethodDef {
                ml_name: method_name_ptr,
                ml_meth: Some(std::mem::transmute::<
                    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void,
                    unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
                >(method_callable)),
                ml_flags: METH_VARARGS | METH_KEYWORDS,
                ml_doc: std::ptr::null(),
            });
        }
        self.cpython_builtin_method_defs.insert(builtin, method_def);
        self.cpython_builtin_by_method_def
            .insert(method_def as usize, builtin);
        method_def
    }

    fn alloc_cpython_builtin_cfunction_ptr(&mut self, builtin: BuiltinFunction) -> *mut c_void {
        if let Some(existing) = self
            .cpython_builtin_cfunction_ptr_cache
            .get(&builtin)
            .copied()
        {
            return existing;
        }
        let method_def = self.ensure_builtin_method_def(builtin);
        if method_def.is_null() {
            return std::ptr::null_mut();
        }
        // Encode builtin identity in `m_self` via method-def pointer; the shim resolves
        // the builtin enum from context-owned method-def metadata.
        let ptr = self.alloc_cpython_method_cfunction_ptr(
            method_def,
            method_def.cast::<c_void>(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        self.cpython_builtin_cfunction_ptr_cache
            .insert(builtin, ptr);
        ptr
    }

    fn alloc_cpython_method_cfunction_ptr(
        &mut self,
        method_def: *mut CpythonMethodDef,
        self_ptr: *mut c_void,
        module_ptr: *mut c_void,
        class_ptr: *mut c_void,
    ) -> *mut c_void {
        let cache_key = (
            method_def as usize,
            self_ptr as usize,
            module_ptr as usize,
            class_ptr as usize,
        );
        if let Some(existing) = self.cpython_cfunction_ptr_cache.get(&cache_key).copied() {
            return existing;
        }
        // SAFETY: allocates C-compatible storage for cfunction object payload.
        let raw = unsafe { malloc(std::mem::size_of::<CpythonCFunctionCompatObject>()) }
            .cast::<CpythonCFunctionCompatObject>();
        if raw.is_null() {
            self.set_error("out of memory allocating CPython cfunction object");
            return std::ptr::null_mut();
        }
        // SAFETY: `raw` points to writable storage with cfunction layout.
        unsafe {
            raw.write(CpythonCFunctionCompatObject {
                ob_base: CpythonObjectHead {
                    ob_refcnt: 1,
                    ob_type: std::ptr::addr_of_mut!(PyCFunction_Type).cast(),
                },
                m_ml: method_def,
                m_self: self_ptr,
                m_module: module_ptr,
                m_class: class_ptr,
            });
        }
        let ptr = raw.cast::<c_void>();
        self.cpython_allocations
            .push(raw.cast::<CpythonCompatObject>());
        self.cpython_owned_ptrs.insert(ptr as usize);
        self.cpython_cfunction_ptr_cache.insert(cache_key, ptr);
        ptr
    }

    fn alloc_cpython_descriptor_ptr(
        &mut self,
        descriptor_type: *mut CpythonTypeObject,
        descriptor_kind: CpythonDescriptorKind,
    ) -> *mut c_void {
        if descriptor_type.is_null() {
            self.set_error("descriptor allocation missing descriptor type");
            return std::ptr::null_mut();
        }
        // SAFETY: allocates C-compatible storage for descriptor object payload.
        let raw = unsafe { malloc(std::mem::size_of::<CpythonCompatObject>()) }
            .cast::<CpythonCompatObject>();
        if raw.is_null() {
            self.set_error("out of memory allocating CPython descriptor object");
            return std::ptr::null_mut();
        }
        // SAFETY: raw points to writable CpythonCompatObject storage.
        unsafe {
            raw.write(CpythonCompatObject {
                ob_base: CpythonVarObjectHead {
                    ob_base: CpythonObjectHead {
                        ob_refcnt: 1,
                        ob_type: descriptor_type.cast(),
                    },
                    ob_size: 0,
                },
            });
        }
        let ptr = raw.cast::<c_void>();
        self.cpython_allocations.push(raw);
        self.cpython_owned_ptrs.insert(ptr as usize);
        self.cpython_descriptors
            .insert(ptr as usize, descriptor_kind);
        ptr
    }

    fn resolve_descriptor_attr_ptr(
        &mut self,
        descriptor_ptr: *mut c_void,
        object: *mut c_void,
        object_type: *mut CpythonTypeObject,
        is_type_object: bool,
    ) -> Option<*mut c_void> {
        let descriptor_kind = self
            .cpython_descriptors
            .get(&(descriptor_ptr as usize))
            .copied()?;
        match descriptor_kind {
            CpythonDescriptorKind::Method {
                owner_type,
                method_def,
                class_method,
            } => {
                if owner_type.is_null() || method_def.is_null() {
                    self.set_error("descriptor method metadata is invalid");
                    return Some(std::ptr::null_mut());
                }
                let matches_owner = if is_type_object {
                    unsafe { PyType_IsSubtype(object, owner_type.cast()) != 0 }
                } else if object_type.is_null() {
                    false
                } else {
                    unsafe { PyType_IsSubtype(object_type.cast(), owner_type.cast()) != 0 }
                };
                if !matches_owner {
                    // SAFETY: method name pointer is extension-owned and expected to be NUL-terminated.
                    let method_name = unsafe { c_name_to_string((*method_def).ml_name) }
                        .unwrap_or_else(|_| "<unnamed>".to_string());
                    // SAFETY: owner type pointer is validated non-null above.
                    let owner_name = unsafe { c_name_to_string((*owner_type).tp_name) }
                        .unwrap_or_else(|_| "<unknown>".to_string());
                    let got_name = if is_type_object {
                        // SAFETY: object is expected to be a type pointer in this branch.
                        unsafe {
                            object
                                .cast::<CpythonTypeObject>()
                                .as_ref()
                                .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                                .unwrap_or_else(|| "<unknown>".to_string())
                        }
                    } else {
                        // SAFETY: object_type is read-only inspected when present.
                        unsafe {
                            object_type
                                .as_ref()
                                .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                                .unwrap_or_else(|| "<unknown>".to_string())
                        }
                    };
                    self.set_error(format!(
                        "TypeError: descriptor '{}' for '{}.{}' doesn't apply to '{}'",
                        method_name, owner_name, method_name, got_name
                    ));
                    return Some(std::ptr::null_mut());
                }

                if !class_method && is_type_object {
                    return Some(descriptor_ptr);
                }
                let flags = unsafe { (*method_def).ml_flags };
                let has_meth_method = (flags & METH_METHOD) != 0;
                let (self_ptr, class_ptr) = if class_method {
                    let receiver = if is_type_object {
                        object
                    } else {
                        object_type.cast()
                    };
                    (
                        receiver,
                        if has_meth_method {
                            owner_type.cast()
                        } else {
                            std::ptr::null_mut()
                        },
                    )
                } else {
                    (
                        object,
                        if has_meth_method {
                            owner_type.cast()
                        } else {
                            std::ptr::null_mut()
                        },
                    )
                };
                let callable = self.alloc_cpython_method_cfunction_ptr(
                    method_def,
                    self_ptr,
                    std::ptr::null_mut(),
                    class_ptr,
                );
                Some(callable)
            }
            CpythonDescriptorKind::GetSet { owner_type, getset } => {
                if owner_type.is_null() || getset.is_null() {
                    self.set_error("descriptor getset metadata is invalid");
                    return Some(std::ptr::null_mut());
                }
                if is_type_object {
                    return Some(descriptor_ptr);
                }
                if object_type.is_null()
                    || unsafe { PyType_IsSubtype(object_type.cast(), owner_type.cast()) } == 0
                {
                    self.set_error("descriptor getset requires subtype instance");
                    return Some(std::ptr::null_mut());
                }
                let getter = unsafe { (*getset).get };
                if getter.is_null() {
                    self.set_error("AttributeError: attribute is not readable");
                    return Some(std::ptr::null_mut());
                }
                let get: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                    unsafe { std::mem::transmute(getter) };
                let closure = unsafe { (*getset).closure };
                Some(unsafe { get(object, closure) })
            }
            CpythonDescriptorKind::Member { owner_type, member } => {
                if owner_type.is_null() || member.is_null() {
                    self.set_error("descriptor member metadata is invalid");
                    return Some(std::ptr::null_mut());
                }
                if is_type_object {
                    return Some(descriptor_ptr);
                }
                if object_type.is_null()
                    || unsafe { PyType_IsSubtype(object_type.cast(), owner_type.cast()) } == 0
                {
                    self.set_error("descriptor member requires subtype instance");
                    return Some(std::ptr::null_mut());
                }
                let basicsize = unsafe { (*owner_type).tp_basicsize };
                // SAFETY: member pointer is stable for extension lifetime.
                let member_ref = unsafe { &*member };
                Some(
                    self.load_member_attr_ptr(object, member_ref, basicsize)
                        .unwrap_or(std::ptr::null_mut()),
                )
            }
        }
    }

    fn load_member_attr_ptr(
        &mut self,
        object: *mut c_void,
        member: &CpythonMemberDef,
        type_basicsize: isize,
    ) -> Option<*mut c_void> {
        if object.is_null() || member.offset < 0 || type_basicsize <= 0 {
            return None;
        }
        if member.offset >= type_basicsize {
            return None;
        }
        // SAFETY: member offsets come from extension type metadata for this object layout.
        let field_ptr = unsafe { object.cast::<u8>().add(member.offset as usize) };
        match member.member_type {
            PY_MEMBER_T_OBJECT | PY_MEMBER_T_OBJECT_EX => {
                // SAFETY: OBJECT/OBJECT_EX members store a `PyObject*` at the configured offset.
                let value_ptr =
                    unsafe { std::ptr::read_unaligned(field_ptr.cast::<*mut c_void>()) };
                if value_ptr.is_null() {
                    if member.member_type == PY_MEMBER_T_OBJECT {
                        return Some(self.alloc_cpython_ptr_for_value(Value::None));
                    }
                    return None;
                }
                Some(value_ptr)
            }
            PY_MEMBER_T_CHAR => {
                // SAFETY: CHAR members store a native byte value at the configured offset.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u8>()) };
                let value = Value::Str((raw as char).to_string());
                Some(self.alloc_cpython_ptr_for_value(value))
            }
            PY_MEMBER_T_BOOL => {
                // SAFETY: BOOL members store an unsigned byte.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u8>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Bool(raw != 0)))
            }
            PY_MEMBER_T_INT => {
                // SAFETY: INT members store a native c_int.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_int>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Int(raw as i64)))
            }
            PY_MEMBER_T_UINT => {
                // SAFETY: UINT members store a native `unsigned int`.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u32>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Int(raw as i64)))
            }
            PY_MEMBER_T_LONG => {
                // SAFETY: LONG members store a native c_long.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_long>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Int(raw as i64)))
            }
            PY_MEMBER_T_ULONG => {
                // SAFETY: ULONG members store a native c_ulong.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_ulong>()) };
                if raw <= i64::MAX as c_ulong {
                    Some(self.alloc_cpython_ptr_for_value(Value::Int(raw as i64)))
                } else {
                    Some(self.alloc_cpython_ptr_for_value(Value::BigInt(Box::new(
                        BigInt::from_u64(raw as u64),
                    ))))
                }
            }
            PY_MEMBER_T_LONGLONG => {
                // SAFETY: LONGLONG members store a signed 64-bit integer.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<i64>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Int(raw)))
            }
            PY_MEMBER_T_ULONGLONG => {
                // SAFETY: ULONGLONG members store an unsigned 64-bit integer.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u64>()) };
                if raw <= i64::MAX as u64 {
                    Some(self.alloc_cpython_ptr_for_value(Value::Int(raw as i64)))
                } else {
                    Some(self.alloc_cpython_ptr_for_value(Value::BigInt(Box::new(
                        BigInt::from_u64(raw),
                    ))))
                }
            }
            PY_MEMBER_T_PYSSIZET => {
                // SAFETY: PYSSIZET members store a native `isize`.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<isize>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Int(raw as i64)))
            }
            _ => None,
        }
    }

    fn member_attr_name(member: &CpythonMemberDef) -> String {
        if member.name.is_null() {
            return "<unnamed>".to_string();
        }
        // SAFETY: descriptor metadata provides a stable C string for member name.
        unsafe { c_name_to_string(member.name).unwrap_or_else(|_| "<unnamed>".to_string()) }
    }

    fn member_field_ptr(object: *mut c_void, member: &CpythonMemberDef) -> Result<*mut u8, String> {
        if object.is_null() {
            return Err("member object pointer is null".to_string());
        }
        if member.offset < 0 {
            return Err("member offset must be non-negative".to_string());
        }
        // SAFETY: offset validated non-negative and pointer arithmetic is byte-based.
        Ok(unsafe { object.cast::<u8>().add(member.offset as usize) })
    }

    fn lookup_type_attr_via_tp_dict(
        &mut self,
        object: *mut c_void,
        attr_name: &str,
    ) -> Option<*mut c_void> {
        if object.is_null() {
            return None;
        }
        // SAFETY: object is expected to be a valid PyObject* when entering C-API attr lookup.
        let object_type = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type)
                .unwrap_or(std::ptr::null_mut())
        };
        let expected_type = std::ptr::addr_of_mut!(PyType_Type).cast();
        let is_type_object = object_type == expected_type;
        let trace_type_attr =
            attr_name == "type" && std::env::var_os("PYRS_TRACE_PROXY_TYPE_ATTR").is_some();
        let trace_lookup_branch = std::env::var_os("PYRS_TRACE_PROXY_LOOKUP_BRANCH").is_some();
        let trace_repr_lookup = std::env::var_os("PYRS_TRACE_PROXY_REPR_LOOKUP").is_some()
            && matches!(attr_name, "__repr__" | "__str__");
        let is_proxy_trace = attr_name == "__array_finalize__"
            && std::env::var_os("PYRS_TRACE_CPY_PROXY_PTRS").is_some();
        if is_proxy_trace {
            eprintln!(
                "[cpy-proxy] tp_dict lookup object_ptr={:p} object_type={:p} expected_type={:p}",
                object, object_type, expected_type
            );
        }
        if object_type.is_null() {
            return None;
        }
        // Type objects walk their own MRO; instances walk their runtime type MRO.
        let mut current = if is_type_object {
            object.cast::<CpythonTypeObject>()
        } else {
            object_type.cast::<CpythonTypeObject>()
        };
        let key = Value::Str(attr_name.to_string());
        for _ in 0..64 {
            if current.is_null() {
                break;
            }
            // SAFETY: current points to a PyTypeObject-compatible header.
            let dict_ptr = unsafe { (*current).tp_dict };
            if trace_repr_lookup {
                // SAFETY: current points to a PyTypeObject-compatible header.
                let type_name = unsafe {
                    c_name_to_string((*current).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
                };
                eprintln!(
                    "[proxy-repr-lookup] attr={} object={:p} current={:p} type_name={} dict={:p}",
                    attr_name, object, current, type_name, dict_ptr
                );
            }
            if trace_type_attr {
                // SAFETY: current points to a type object header.
                let (type_name, base_ptr, methods_ptr, getset_ptr) = unsafe {
                    let type_name = c_name_to_string((*current).tp_name)
                        .unwrap_or_else(|_| "<invalid>".to_string());
                    (
                        type_name,
                        (*current).tp_base,
                        (*current).tp_methods,
                        (*current).tp_getset,
                    )
                };
                eprintln!(
                    "[cpy-proxy-attr] scan type={:p} name={} dict={:p} methods={:p} getset={:p} base={:p}",
                    current, type_name, dict_ptr, methods_ptr, getset_ptr, base_ptr
                );
            }
            if is_proxy_trace {
                // SAFETY: current points to a PyTypeObject-compatible header.
                let base_ptr = unsafe { (*current).tp_base };
                eprintln!(
                    "[cpy-proxy] tp_dict lookup current={:p} dict={:p} base={:p}",
                    current, dict_ptr, base_ptr
                );
            }
            if !dict_ptr.is_null()
                && let Some(Value::Dict(dict_obj)) = self.cpython_value_from_ptr(dict_ptr)
                && let Some(value) = dict_get_value(&dict_obj, &key)
            {
                if let Some(raw_descriptor_ptr) = Self::cpython_proxy_raw_ptr_from_value(&value)
                    && let Some(bound_ptr) = self.resolve_descriptor_attr_ptr(
                        raw_descriptor_ptr,
                        object,
                        object_type.cast(),
                        is_type_object,
                    )
                {
                    if is_proxy_trace {
                        eprintln!(
                            "[cpy-proxy] tp_dict descriptor hit current={:p} dict={:p} descriptor_ptr={:p} bound_ptr={:p}",
                            current, dict_ptr, raw_descriptor_ptr, bound_ptr
                        );
                    }
                    if trace_lookup_branch {
                        eprintln!(
                            "[proxy-lookup-branch] attr={} branch=dict_descriptor object={:p} current={:p} value_ptr={:p}",
                            attr_name, object, current, bound_ptr
                        );
                    }
                    return Some(bound_ptr);
                }
                if is_proxy_trace {
                    eprintln!(
                        "[cpy-proxy] tp_dict lookup hit current={:p} dict={:p} value_tag={}",
                        current,
                        dict_ptr,
                        cpython_value_debug_tag(&value)
                    );
                }
                if std::env::var_os("PYRS_TRACE_PROXY_ATTR_CALL").is_some() {
                    eprintln!(
                        "[proxy-attr-map] source=tp_dict_value target={:p} attr={} value_tag={}",
                        object,
                        attr_name,
                        cpython_value_debug_tag(&value)
                    );
                }
                let value_ptr = self.alloc_cpython_ptr_for_value(value.clone());
                if trace_lookup_branch {
                    eprintln!(
                        "[proxy-lookup-branch] attr={} branch=dict_value object={:p} current={:p} value_ptr={:p}",
                        attr_name, object, current, value_ptr
                    );
                }
                return Some(value_ptr);
            } else if !dict_ptr.is_null()
                && let Ok(attr_c_name) = CString::new(attr_name)
            {
                // External tp_dict pointers are common for proxied native types.
                // Probe through the C-API dictionary surface so slot-wrapper descriptors
                // materialized by PyType_Ready are visible here.
                let external_value_ptr =
                    unsafe { PyDict_GetItemString(dict_ptr, attr_c_name.as_ptr()) };
                if !external_value_ptr.is_null() {
                    // SAFETY: best-effort descriptor probe on external tp_dict entry.
                    let descriptor_type = unsafe {
                        external_value_ptr
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                            .unwrap_or(std::ptr::null_mut())
                    };
                    if !descriptor_type.is_null() {
                        // SAFETY: descriptor type pointer was loaded from object header above.
                        let descriptor_get = unsafe { (*descriptor_type).tp_descr_get };
                        if !descriptor_get.is_null() {
                            let descriptor_get: unsafe extern "C" fn(
                                *mut c_void,
                                *mut c_void,
                                *mut c_void,
                            ) -> *mut c_void =
                                // SAFETY: descriptor getter follows CPython descriptor ABI.
                                unsafe { std::mem::transmute(descriptor_get) };
                            let owner_ptr = object_type.cast::<c_void>();
                            let self_ptr = if is_type_object {
                                std::ptr::null_mut()
                            } else {
                                object
                            };
                            // SAFETY: descriptor access mirrors CPython descriptor invocation.
                            let bound =
                                unsafe { descriptor_get(external_value_ptr, self_ptr, owner_ptr) };
                            if !bound.is_null() {
                                if trace_lookup_branch {
                                    eprintln!(
                                        "[proxy-lookup-branch] attr={} branch=dict_external_descriptor object={:p} current={:p} value_ptr={:p}",
                                        attr_name, object, current, bound
                                    );
                                }
                                return Some(bound);
                            }
                        }
                    }
                    if trace_lookup_branch {
                        eprintln!(
                            "[proxy-lookup-branch] attr={} branch=dict_external_value object={:p} current={:p} value_ptr={:p}",
                            attr_name, object, current, external_value_ptr
                        );
                    }
                    return Some(external_value_ptr);
                } else if trace_repr_lookup {
                    eprintln!(
                        "[proxy-repr-lookup] attr={} dict_external_miss object={:p} current={:p}",
                        attr_name, object, current
                    );
                }
            } else if is_proxy_trace && !dict_ptr.is_null() {
                eprintln!(
                    "[cpy-proxy] tp_dict lookup miss current={:p} dict_ptr={:p}",
                    current, dict_ptr
                );
            }
            // SAFETY: current points to a PyTypeObject-compatible header.
            let methods_ptr = unsafe { (*current).tp_methods }.cast::<CpythonMethodDef>();
            if !methods_ptr.is_null() {
                let mut method = methods_ptr;
                let mut traced_methods = 0usize;
                loop {
                    // SAFETY: methods table is terminated by null `ml_name`.
                    let method_name_ptr = unsafe { (*method).ml_name };
                    if method_name_ptr.is_null() {
                        break;
                    }
                    // SAFETY: `ml_name` is NUL-terminated as part of CPython method-table ABI.
                    let method_name = unsafe { CStr::from_ptr(method_name_ptr) }
                        .to_str()
                        .ok()
                        .unwrap_or("");
                    if trace_type_attr && traced_methods < 8 {
                        eprintln!(
                            "[cpy-proxy-attr] method candidate current={:p} name={}",
                            current, method_name
                        );
                        traced_methods += 1;
                    }
                    if method_name == attr_name {
                        let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                            method,
                            object,
                            std::ptr::null_mut(),
                            current.cast::<c_void>(),
                        );
                        if is_proxy_trace {
                            eprintln!(
                                "[cpy-proxy] tp_methods lookup hit current={:p} method={} callable_ptr={:p}",
                                current, attr_name, callable_ptr
                            );
                        }
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=tp_methods object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                    // SAFETY: method table entries are contiguous.
                    method = unsafe { method.add(1) };
                }
            }
            // SAFETY: current points to a PyTypeObject-compatible header.
            let getset_ptr = unsafe { (*current).tp_getset }.cast::<CpythonGetSetDef>();
            if !getset_ptr.is_null() {
                let mut getset = getset_ptr;
                let mut traced_getsets = 0usize;
                loop {
                    // SAFETY: getset table is terminated by null `name`.
                    let getset_name_ptr = unsafe { (*getset).name };
                    if getset_name_ptr.is_null() {
                        break;
                    }
                    // SAFETY: `name` is NUL-terminated as part of CPython getset ABI.
                    let getset_name = unsafe { CStr::from_ptr(getset_name_ptr) }
                        .to_str()
                        .ok()
                        .unwrap_or("");
                    if trace_type_attr && traced_getsets < 12 {
                        eprintln!(
                            "[cpy-proxy-attr] getset candidate current={:p} name={}",
                            current, getset_name
                        );
                        traced_getsets += 1;
                    }
                    if getset_name == attr_name {
                        // SAFETY: getset entry layout follows CPython ABI.
                        let getter = unsafe { (*getset).get };
                        if !getter.is_null() {
                            let get: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                                // SAFETY: getter pointer follows CPython getset ABI.
                                unsafe { std::mem::transmute(getter) };
                            // SAFETY: closure value is provided by extension type definition.
                            let closure = unsafe { (*getset).closure };
                            let value_ptr = unsafe { get(object, closure) };
                            if !value_ptr.is_null() {
                                if is_proxy_trace {
                                    eprintln!(
                                        "[cpy-proxy] tp_getset lookup hit current={:p} name={} value_ptr={:p}",
                                        current, attr_name, value_ptr
                                    );
                                }
                                if trace_lookup_branch {
                                    eprintln!(
                                        "[proxy-lookup-branch] attr={} branch=tp_getset object={:p} current={:p} value_ptr={:p}",
                                        attr_name, object, current, value_ptr
                                    );
                                }
                                return Some(value_ptr);
                            }
                        }
                    }
                    // SAFETY: getset table entries are contiguous.
                    getset = unsafe { getset.add(1) };
                }
            }
            // SAFETY: current points to a PyTypeObject-compatible header.
            let (members_ptr, current_basicsize) = unsafe {
                (
                    (*current).tp_members.cast::<CpythonMemberDef>(),
                    (*current).tp_basicsize,
                )
            };
            if !members_ptr.is_null() {
                let mut member = members_ptr;
                let mut traced_members = 0usize;
                loop {
                    // SAFETY: member table is terminated by null `name`.
                    let member_name_ptr = unsafe { (*member).name };
                    if member_name_ptr.is_null() {
                        break;
                    }
                    // SAFETY: member name is NUL-terminated as part of CPython member ABI.
                    let member_name = unsafe { CStr::from_ptr(member_name_ptr) }
                        .to_str()
                        .ok()
                        .unwrap_or("");
                    if trace_type_attr && traced_members < 12 {
                        eprintln!(
                            "[cpy-proxy-attr] member candidate current={:p} name={} kind={}",
                            current,
                            member_name,
                            unsafe { (*member).member_type }
                        );
                        traced_members += 1;
                    }
                    if member_name == attr_name {
                        // SAFETY: `member` points to a valid descriptor for `current`.
                        let member_ref = unsafe { &*member };
                        if let Some(value_ptr) =
                            self.load_member_attr_ptr(object, member_ref, current_basicsize)
                        {
                            if trace_lookup_branch {
                                eprintln!(
                                    "[proxy-lookup-branch] attr={} branch=tp_members object={:p} current={:p} value_ptr={:p}",
                                    attr_name, object, current, value_ptr
                                );
                            }
                            return Some(value_ptr);
                        }
                    }
                    // SAFETY: member table entries are contiguous.
                    member = unsafe { member.add(1) };
                }
            }
            // SAFETY: current points to a PyTypeObject-compatible header.
            let next = unsafe { (*current).tp_base };
            if trace_repr_lookup {
                eprintln!(
                    "[proxy-repr-lookup] attr={} advance current={:p} next={:p}",
                    attr_name, current, next
                );
            }
            if next.is_null() || next == current {
                break;
            }
            current = next;
        }
        if trace_repr_lookup {
            eprintln!(
                "[proxy-repr-lookup] attr={} miss object={:p}",
                attr_name, object
            );
        }
        None
    }

    fn class_attr_walk_for_type_lookup(class: &ObjRef) -> Vec<ObjRef> {
        let class_kind = class.kind();
        let class_data = match &*class_kind {
            Object::Class(class_data) => class_data,
            _ => return Vec::new(),
        };
        if !class_data.mro.is_empty() {
            return class_data.mro.clone();
        }
        let mut out = vec![class.clone()];
        for base in &class_data.bases {
            out.extend(Self::class_attr_walk_for_type_lookup(base));
        }
        out
    }

    fn lookup_type_attr_via_runtime_mro(
        &mut self,
        ty: *mut c_void,
        attr_name: &str,
    ) -> Option<*mut c_void> {
        if ty.is_null() {
            return None;
        }
        let Value::Class(class_obj) = self.cpython_value_from_ptr_or_proxy(ty)? else {
            return None;
        };
        for candidate in Self::class_attr_walk_for_type_lookup(&class_obj) {
            let attr_value = {
                let candidate_kind = candidate.kind();
                let Object::Class(class_data) = &*candidate_kind else {
                    continue;
                };
                class_data.attrs.get(attr_name).cloned()
            };
            if let Some(value) = attr_value {
                if let Some(raw_ptr) = Self::cpython_proxy_raw_ptr_from_value(&value) {
                    return Some(raw_ptr);
                }
                return Some(self.alloc_cpython_ptr_for_value(value));
            }
        }
        None
    }

    fn try_native_vectorcall(
        &mut self,
        callable: *mut c_void,
        args: &[Value],
        kwargs: &HashMap<String, Value>,
    ) -> Option<*mut c_void> {
        if callable.is_null() || self.vm.is_null() {
            return None;
        }
        let vectorcall = match unsafe { cpython_resolve_vectorcall(callable) } {
            Some(vectorcall) => vectorcall,
            None => return None,
        };
        let trace_vectorcall = std::env::var_os("PYRS_TRACE_CPY_VECTORCALL").is_some();
        if trace_vectorcall {
            // SAFETY: callable is a candidate PyObject pointer in vectorcall path.
            let type_name = unsafe {
                callable
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .and_then(|head| {
                        head.ob_type
                            .cast::<CpythonTypeObject>()
                            .as_ref()
                            .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                    })
                    .unwrap_or_else(|| "<unknown>".to_string())
            };
            let arg_summary = args
                .iter()
                .map(cpython_value_debug_tag)
                .collect::<Vec<_>>()
                .join(", ");
            let mut kw_entries = kwargs.iter().collect::<Vec<_>>();
            kw_entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let kw_summary = kw_entries
                .into_iter()
                .map(|(name, value)| format!("{name}={}", cpython_value_debug_tag(value)))
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!(
                "[cpy-vectorcall] callable={:p} type={} args=[{}] kwargs=[{}]",
                callable, type_name, arg_summary, kw_summary
            );
            if type_name.contains("_ArrayFunctionDispatcher") {
                // SAFETY: diagnostic metadata lookup on callable object.
                unsafe {
                    let name_ptr = PyObject_GetAttrString(callable, c"__name__".as_ptr());
                    let implementation_ptr =
                        PyObject_GetAttrString(callable, c"_implementation".as_ptr());
                    let implementation_name_ptr = if implementation_ptr.is_null() {
                        std::ptr::null_mut()
                    } else {
                        PyObject_GetAttrString(implementation_ptr, c"__name__".as_ptr())
                    };
                    let name_text = if name_ptr.is_null() {
                        "<none>".to_string()
                    } else {
                        self.cpython_value_from_ptr_or_proxy(name_ptr)
                            .map(|value| match value {
                                Value::Str(text) => text,
                                other => cpython_value_debug_tag(&other),
                            })
                            .unwrap_or_else(|| "<unknown>".to_string())
                    };
                    let implementation_name_text = if implementation_name_ptr.is_null() {
                        "<none>".to_string()
                    } else {
                        self.cpython_value_from_ptr_or_proxy(implementation_name_ptr)
                            .map(|value| match value {
                                Value::Str(text) => text,
                                other => cpython_value_debug_tag(&other),
                            })
                            .unwrap_or_else(|| "<unknown>".to_string())
                    };
                    eprintln!(
                        "[cpy-vectorcall-dispatcher] callable={:p} __name__={} implementation_name={}",
                        callable, name_text, implementation_name_text
                    );
                    if !name_ptr.is_null() {
                        Py_DecRef(name_ptr);
                    }
                    if !implementation_name_ptr.is_null() {
                        Py_DecRef(implementation_name_ptr);
                    }
                    if !implementation_ptr.is_null() {
                        Py_DecRef(implementation_ptr);
                    }
                }
            }
            if kwargs.contains_key("dtype") {
                // SAFETY: VM pointer is valid for active context lifetime.
                let vm = unsafe { &*self.vm };
                let stack = vm
                    .frames
                    .iter()
                    .rev()
                    .take(10)
                    .map(|frame| {
                        format!("{}:{}:{}", frame.code.filename, frame.code.name, frame.ip)
                    })
                    .collect::<Vec<_>>()
                    .join(" <- ");
                eprintln!("[cpy-vectorcall-stack] {}", stack);
            }
        }
        let positional_count = args.len();
        let kw_count = kwargs.len();
        let mut stack: Vec<*mut c_void> =
            Vec::with_capacity(positional_count.saturating_add(kw_count));
        for value in args {
            let used_proxy_ptr = if trace_vectorcall {
                Self::cpython_proxy_raw_ptr_from_value(value).is_some()
            } else {
                false
            };
            let ptr = self.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                self.set_error("failed to materialize positional vectorcall argument");
                return Some(std::ptr::null_mut());
            }
            if trace_vectorcall {
                // SAFETY: `ptr` is materialized in this context.
                let ptr_type = unsafe {
                    ptr.cast::<CpythonObjectHead>()
                        .as_ref()
                        .and_then(|head| {
                            head.ob_type
                                .cast::<CpythonTypeObject>()
                                .as_ref()
                                .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                        })
                        .unwrap_or_else(|| "<unknown>".to_string())
                };
                eprintln!(
                    "[cpy-vectorcall-arg] value={} proxy_raw={} ptr={:p} ptr_type={}",
                    cpython_value_debug_tag(value),
                    used_proxy_ptr,
                    ptr,
                    ptr_type
                );
            }
            stack.push(ptr);
        }
        let mut kw_name_ptrs: Vec<*mut c_void> = Vec::with_capacity(kw_count);
        for (name, value) in kwargs {
            let c_name = match CString::new(name.as_str()) {
                Ok(c_name) => c_name,
                Err(_) => {
                    self.set_error("keyword argument name contains interior NUL byte");
                    return Some(std::ptr::null_mut());
                }
            };
            // SAFETY: C string is NUL-terminated and valid for the duration of this call.
            let name_ptr = unsafe { PyUnicode_InternFromString(c_name.as_ptr()) };
            if name_ptr.is_null() {
                self.set_error("failed to intern vectorcall keyword name");
                return Some(std::ptr::null_mut());
            }
            kw_name_ptrs.push(name_ptr);
            let used_proxy_ptr = if trace_vectorcall {
                Self::cpython_proxy_raw_ptr_from_value(value).is_some()
            } else {
                false
            };
            let ptr = self.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                self.set_error("failed to materialize keyword vectorcall argument");
                return Some(std::ptr::null_mut());
            }
            if trace_vectorcall {
                // SAFETY: `ptr` is materialized in this context.
                let ptr_type = unsafe {
                    ptr.cast::<CpythonObjectHead>()
                        .as_ref()
                        .and_then(|head| {
                            head.ob_type
                                .cast::<CpythonTypeObject>()
                                .as_ref()
                                .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                        })
                        .unwrap_or_else(|| "<unknown>".to_string())
                };
                eprintln!(
                    "[cpy-vectorcall-kw] name={} value={} proxy_raw={} ptr={:p} ptr_type={}",
                    name,
                    cpython_value_debug_tag(value),
                    used_proxy_ptr,
                    ptr,
                    ptr_type
                );
                if matches!(value, Value::Class(_)) {
                    // SAFETY: class pointers are PyTypeObject-compatible.
                    let class_tp_name = unsafe {
                        ptr.cast::<CpythonTypeObject>()
                            .as_ref()
                            .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                            .unwrap_or_else(|| "<unknown>".to_string())
                    };
                    eprintln!(
                        "[cpy-vectorcall-kw-class] name={} ptr={:p} class_tp_name={}",
                        name, ptr, class_tp_name
                    );
                }
            }
            stack.push(ptr);
        }
        let kwnames_ptr = if kw_name_ptrs.is_empty() {
            std::ptr::null_mut()
        } else {
            // SAFETY: tuple allocation follows CPython tuple ABI.
            let tuple = unsafe { PyTuple_New(kw_name_ptrs.len() as isize) };
            if tuple.is_null() {
                self.set_error("failed to allocate vectorcall keyword names tuple");
                return Some(std::ptr::null_mut());
            }
            for (index, name_ptr) in kw_name_ptrs.into_iter().enumerate() {
                // SAFETY: tuple is newly allocated and index is in-bounds.
                let status = unsafe { PyTuple_SetItem(tuple, index as isize, name_ptr) };
                if status != 0 {
                    // SAFETY: on failure the tuple still owns any previously-set references.
                    unsafe { Py_DecRef(tuple) };
                    self.set_error("failed to populate vectorcall keyword names tuple");
                    return Some(std::ptr::null_mut());
                }
            }
            tuple
        };
        if !kwargs.is_empty() && kwnames_ptr.is_null() {
            self.set_error("failed to materialize vectorcall keyword names");
            return Some(std::ptr::null_mut());
        }
        let args_ptr = if stack.is_empty() {
            std::ptr::null()
        } else {
            stack.as_ptr()
        };
        let result = unsafe { vectorcall(callable, args_ptr, positional_count, kwnames_ptr) };
        if !kwnames_ptr.is_null() {
            // SAFETY: kwnames tuple is call-local materialization and no longer needed after call.
            unsafe { Py_DecRef(kwnames_ptr) };
        }
        Some(result)
    }

    fn try_native_tp_call(
        &mut self,
        callable: *mut c_void,
        args: &[Value],
        kwargs: &HashMap<String, Value>,
    ) -> Option<*mut c_void> {
        const MIN_VALID_PTR: usize = 0x1_0000_0000;
        let trace_calls = std::env::var_os("PYRS_TRACE_CPY_CALLS").is_some();
        if callable.is_null() || self.vm.is_null() {
            if trace_calls {
                eprintln!(
                    "[cpy-call] skip native callable={:p} reason=null-callable-or-vm",
                    callable
                );
            }
            return None;
        }
        if (callable as usize) < MIN_VALID_PTR
            || (callable as usize) % std::mem::align_of::<usize>() != 0
        {
            return None;
        }
        // CPython exception globals (`PyExc_*`) are exported as process-stable symbol
        // objects in this compatibility layer. They are callable via VM class/exctype
        // dispatch and must not be interpreted as concrete PyTypeObject instances.
        if cpython_exception_value_from_ptr(callable as usize).is_some() {
            return None;
        }
        if let Some(result) = self.try_native_vectorcall(callable, args, kwargs) {
            if trace_calls {
                eprintln!("[cpy-call] native vectorcall callable={:p}", callable);
            }
            return Some(result);
        }
        // SAFETY: caller passes a potential PyObject pointer; guard nulls above.
        let type_ptr = unsafe { callable.cast::<CpythonObjectHead>().as_ref() }
            .map(|head| head.ob_type)
            .unwrap_or(std::ptr::null_mut())
            .cast::<CpythonTypeObject>();
        if trace_calls && let Some(tag_value) = self.cpython_value_from_ptr(callable) {
            eprintln!(
                "[cpy-call] callable={:p} tag={}",
                callable,
                cpython_value_debug_tag(&tag_value)
            );
        }
        if type_ptr.is_null() {
            if trace_calls {
                eprintln!(
                    "[cpy-call] skip native callable={:p} reason=null-type",
                    callable
                );
            }
            return None;
        }
        if (type_ptr as usize) < MIN_VALID_PTR
            || (type_ptr as usize) % std::mem::align_of::<usize>() != 0
        {
            return None;
        }
        let trace_numpy_ufunc_call = std::env::var_os("PYRS_TRACE_NUMPY_UFUNC_CALL").is_some();
        if trace_numpy_ufunc_call {
            // SAFETY: `type_ptr` was validated above.
            let type_name = unsafe {
                c_name_to_string((*type_ptr).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
            };
            if type_name.contains("ufunc") {
                let arg_summary = args
                    .iter()
                    .map(cpython_value_debug_tag)
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut kw_entries = kwargs.iter().collect::<Vec<_>>();
                kw_entries.sort_by(|(left, _), (right, _)| left.cmp(right));
                let kw_summary = kw_entries
                    .into_iter()
                    .map(|(name, value)| format!("{name}={}", cpython_value_debug_tag(value)))
                    .collect::<Vec<_>>()
                    .join(", ");
                eprintln!(
                    "[numpy-ufunc-call] callable={:p} type={} args=[{}] kwargs=[{}]",
                    callable, type_name, arg_summary, kw_summary
                );
            }
        }
        if self.owns_cpython_allocation_ptr(callable)
            && type_ptr == std::ptr::addr_of_mut!(PyType_Type)
        {
            if trace_calls {
                eprintln!(
                    "[cpy-call] skip native callable={:p} reason=owned-compat-type-object",
                    callable
                );
            }
            return None;
        }
        // SAFETY: `type_ptr` is derived from object header and validated non-null.
        let tp_call_raw = unsafe { (*type_ptr).tp_call };
        if tp_call_raw.is_null() {
            if trace_calls {
                eprintln!(
                    "[cpy-call] skip native callable={:p} type={:p} reason=null-tp_call (PyType_Type={:p} tp_call={:p})",
                    callable,
                    type_ptr,
                    (&raw mut PyType_Type),
                    unsafe { PyType_Type.tp_call }
                );
            }
            return None;
        }
        if tp_call_raw == PyVectorcall_Call as *mut c_void {
            self.set_error(
                "native tp_call resolved to PyVectorcall_Call without vectorcall target",
            );
            return Some(std::ptr::null_mut());
        }
        let call: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: `tp_call` is a C ABI function pointer with standard PyObject call signature.
            unsafe { std::mem::transmute(tp_call_raw) };
        // SAFETY: VM pointer is valid for this C-API context.
        let vm = unsafe { &mut *self.vm };
        let args_ptr = self.alloc_cpython_ptr_for_value(vm.heap.alloc_tuple(args.to_vec()));
        if args_ptr.is_null() {
            self.set_error("failed to materialize args tuple for native tp_call");
            return Some(std::ptr::null_mut());
        }
        let kwargs_ptr = if kwargs.is_empty() {
            std::ptr::null_mut()
        } else {
            let entries = kwargs
                .iter()
                .map(|(key, value)| (Value::Str(key.clone()), value.clone()))
                .collect::<Vec<_>>();
            self.alloc_cpython_ptr_for_value(vm.heap.alloc_dict(entries))
        };
        if trace_calls {
            eprintln!(
                "[cpy-call] native callable={:p} type={:p} tp_call={:p} args={} kwargs={}",
                callable,
                type_ptr,
                tp_call_raw,
                args.len(),
                kwargs.len()
            );
        }
        Some(unsafe { call(callable, args_ptr, kwargs_ptr) })
    }

    fn identity_object_id(value: &Value) -> Option<u64> {
        match value {
            Value::List(obj)
            | Value::Tuple(obj)
            | Value::Dict(obj)
            | Value::DictKeys(obj)
            | Value::Set(obj)
            | Value::FrozenSet(obj)
            | Value::Bytes(obj)
            | Value::ByteArray(obj)
            | Value::MemoryView(obj)
            | Value::Iterator(obj)
            | Value::Generator(obj)
            | Value::Module(obj)
            | Value::Class(obj)
            | Value::Instance(obj)
            | Value::Super(obj)
            | Value::Function(obj)
            | Value::BoundMethod(obj)
            | Value::Cell(obj) => Some(obj.id()),
            _ => None,
        }
    }

    fn alloc_capsule(
        &mut self,
        pointer: *mut c_void,
        name: *const c_char,
        cpython_destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> Result<PyrsObjectHandle, String> {
        if pointer.is_null() {
            return Err("capsule_new requires non-null pointer".to_string());
        }
        let name = if name.is_null() {
            None
        } else {
            // SAFETY: pointer is validated by caller as NUL-terminated C string.
            let raw = unsafe { CStr::from_ptr(name) };
            Some(
                CString::new(
                    raw.to_str()
                        .map_err(|_| "capsule name must be utf-8".to_string())?,
                )
                .map_err(|_| "capsule name contains interior NUL".to_string())?,
            )
        };
        let handle = self.allocate_handle();
        self.capsules.insert(
            handle,
            CapiCapsuleSlot {
                pointer: pointer as usize,
                context: 0,
                name,
                destructor: None,
                cpython_destructor,
                exported_name: None,
                refcount: 1,
            },
        );
        Ok(handle)
    }

    fn object_slot(&self, handle: PyrsObjectHandle) -> Option<&CapiObjectSlot> {
        self.objects.get(&handle)
    }

    fn object_value(&self, handle: PyrsObjectHandle) -> Option<Value> {
        self.object_slot(handle).map(|slot| slot.value.clone())
    }

    fn module_get_value(&self, name: &str) -> Result<Value, String> {
        let Object::Module(module_data) = &*self.module.kind() else {
            return Err("module context no longer points to a module".to_string());
        };
        module_data
            .globals
            .get(name)
            .cloned()
            .ok_or_else(|| format!("module attribute '{}' not found", name))
    }

    fn module_get_object(&mut self, name: &str) -> Result<PyrsObjectHandle, String> {
        let value = self.module_get_value(name)?;
        Ok(self.alloc_object(value))
    }

    fn module_import(&mut self, module_name: &str) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("module_import missing VM context".to_string());
        }
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .builtin_import_module(vec![Value::Str(module_name.to_string())], HashMap::new())
            .map_err(|err| err.message)?;
        Ok(self.alloc_object(value))
    }

    fn module_get_attr(
        &mut self,
        module_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("module_get_attr missing VM context".to_string());
        }
        let module = self
            .object_value(module_handle)
            .ok_or_else(|| format!("invalid module handle {}", module_handle))?;
        let module_obj = match module {
            Value::Module(module_obj) => module_obj,
            _ => {
                return Err(format!("object handle {} is not a module", module_handle));
            }
        };
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .load_attr_module(&module_obj, attr_name)
            .map_err(|err| err.message)?;
        Ok(self.alloc_object(value))
    }

    fn incref(&mut self, handle: PyrsObjectHandle) -> Result<(), String> {
        if let Some(slot) = self.objects.get_mut(&handle) {
            slot.refcount = slot.refcount.saturating_add(1);
            self.sync_cpython_refcount(handle);
            return Ok(());
        }
        if let Some(slot) = self.capsules.get_mut(&handle) {
            slot.refcount = slot.refcount.saturating_add(1);
            self.sync_cpython_refcount(handle);
            return Ok(());
        }
        Err(format!("invalid object handle {}", handle))
    }

    fn decref(&mut self, handle: PyrsObjectHandle) -> Result<(), String> {
        if let Some(slot) = self.objects.get_mut(&handle) {
            // CPython ABI callers can keep raw pointers alive across opaque C paths where we
            // cannot accurately mirror ownership in this shim. Keep handles pinned for the
            // entire C-API context lifetime to preserve pointer stability.
            if slot.refcount > 1 {
                slot.refcount -= 1;
            }
            self.sync_cpython_refcount(handle);
            return Ok(());
        }
        if let Some(slot) = self.capsules.get_mut(&handle) {
            if !self.strict_capsule_refcount {
                if slot.refcount > 1 {
                    slot.refcount -= 1;
                }
                self.sync_cpython_refcount(handle);
                return Ok(());
            }
            if slot.refcount == 0 {
                self.capsules.remove(&handle);
                return Ok(());
            }
            slot.refcount -= 1;
            if slot.refcount > 0 {
                self.sync_cpython_refcount(handle);
                return Ok(());
            }
            let capsule_ptr = self
                .cpython_ptr_by_handle
                .get(&handle)
                .copied()
                .unwrap_or(std::ptr::null_mut());
            let slot = self
                .capsules
                .remove(&handle)
                .ok_or_else(|| format!("invalid object handle {}", handle))?;
            if slot.exported_name.is_none() {
                if let Some(cpython_destructor) = slot.cpython_destructor
                    && !capsule_ptr.is_null()
                {
                    // SAFETY: destructor pointer was provided by extension code.
                    unsafe {
                        cpython_destructor(capsule_ptr);
                    }
                }
                if let Some(destructor) = slot.destructor {
                    // SAFETY: destructor pointer was provided by extension code.
                    unsafe {
                        destructor(slot.pointer as *mut c_void, slot.context as *mut c_void);
                    }
                }
            }
            return Ok(());
        }
        Err(format!("invalid object handle {}", handle))
    }

    fn sync_value_from_cpython_storage(&mut self, handle: PyrsObjectHandle, ptr: *mut c_void) {
        if ptr.is_null() || !self.owns_cpython_allocation_ptr(ptr) {
            return;
        }
        if !self.cpython_sync_in_progress.insert(handle) {
            return;
        }

        enum SyncPayload {
            Tuple(Vec<*mut c_void>),
            List(Vec<*mut c_void>),
            Bytes(Vec<u8>),
        }

        let payload = if let Some(slot) = self.objects.get(&handle) {
            match &slot.value {
                Value::Tuple(_) => {
                    // SAFETY: `ptr` is an owned tuple-compatible allocation for this handle.
                    let item_ptrs = unsafe {
                        let head = ptr.cast::<CpythonVarObjectHead>();
                        let len = (*head).ob_size.max(0) as usize;
                        let items = cpython_tuple_items_ptr(ptr);
                        let mut values = Vec::with_capacity(len);
                        for idx in 0..len {
                            values.push(*items.add(idx));
                        }
                        values
                    };
                    Some(SyncPayload::Tuple(item_ptrs))
                }
                Value::List(_) => {
                    // SAFETY: `ptr` is an owned list-compatible allocation for this handle.
                    let item_ptrs = unsafe {
                        let raw_list = ptr.cast::<CpythonListCompatObject>();
                        let len = (*raw_list).ob_base.ob_size.max(0) as usize;
                        let mut values = Vec::with_capacity(len);
                        let item_buf = (*raw_list).ob_item;
                        if item_buf.is_null() {
                            values.resize(len, std::ptr::null_mut());
                        } else {
                            for idx in 0..len {
                                values.push(*item_buf.add(idx));
                            }
                        }
                        values
                    };
                    Some(SyncPayload::List(item_ptrs))
                }
                Value::Bytes(_) | Value::ByteArray(_) => {
                    // SAFETY: `ptr` is an owned bytes-compatible allocation for this handle.
                    let bytes = unsafe {
                        let raw = ptr.cast::<CpythonBytesCompatObject>();
                        let len = (*raw).ob_base.ob_size.max(0) as usize;
                        let data = cpython_bytes_data_ptr(ptr);
                        std::slice::from_raw_parts(data.cast::<u8>(), len).to_vec()
                    };
                    Some(SyncPayload::Bytes(bytes))
                }
                _ => None,
            }
        } else {
            None
        };

        match payload {
            Some(SyncPayload::Tuple(item_ptrs)) => {
                let trace_raw = std::env::var_os("PYRS_TRACE_CPY_TUPLE_RAW").is_some();
                let existing_values = self
                    .objects
                    .get(&handle)
                    .and_then(|slot| match &slot.value {
                        Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                            Object::Tuple(items) => Some(items.clone()),
                            _ => None,
                        },
                        _ => None,
                    })
                    .unwrap_or_default();
                let mut values = Vec::with_capacity(item_ptrs.len());
                for (idx, item_ptr) in item_ptrs.iter().copied().enumerate() {
                    if item_ptr.is_null() {
                        if trace_raw {
                            eprintln!(
                                "[cpy-sync-tuple] handle={} tuple_ptr={:p} idx={} item_ptr=<null> value=None",
                                handle, ptr, idx
                            );
                        }
                        values.push(existing_values.get(idx).cloned().unwrap_or(Value::None));
                        continue;
                    }
                    match self.cpython_value_from_ptr_or_proxy(item_ptr) {
                        Some(value) => {
                            if trace_raw {
                                eprintln!(
                                    "[cpy-sync-tuple] handle={} tuple_ptr={:p} idx={} item_ptr={:p} value={}",
                                    handle,
                                    ptr,
                                    idx,
                                    item_ptr,
                                    cpython_debug_compare_value(&value)
                                );
                            }
                            values.push(value)
                        }
                        None => {
                            if trace_raw {
                                eprintln!(
                                    "[cpy-sync-tuple] handle={} tuple_ptr={:p} idx={} item_ptr={:p} value=<unknown>",
                                    handle, ptr, idx, item_ptr
                                );
                            }
                            values.push(existing_values.get(idx).cloned().unwrap_or(Value::None))
                        }
                    }
                }
                if let Some(slot) = self.objects.get_mut(&handle)
                    && let Value::Tuple(tuple_obj) = &mut slot.value
                    && let Object::Tuple(items) = &mut *tuple_obj.kind_mut()
                {
                    *items = values;
                }
            }
            Some(SyncPayload::List(item_ptrs)) => {
                let existing_values = self
                    .objects
                    .get(&handle)
                    .and_then(|slot| match &slot.value {
                        Value::List(list_obj) => match &*list_obj.kind() {
                            Object::List(items) => Some(items.clone()),
                            _ => None,
                        },
                        _ => None,
                    })
                    .unwrap_or_default();
                let mut values = Vec::with_capacity(item_ptrs.len());
                for (idx, item_ptr) in item_ptrs.into_iter().enumerate() {
                    if item_ptr.is_null() {
                        values.push(existing_values.get(idx).cloned().unwrap_or(Value::None));
                        continue;
                    }
                    match self.cpython_value_from_ptr_or_proxy(item_ptr) {
                        Some(value) => values.push(value),
                        None => {
                            values.push(existing_values.get(idx).cloned().unwrap_or(Value::None))
                        }
                    }
                }
                if let Some(slot) = self.objects.get_mut(&handle)
                    && let Value::List(list_obj) = &mut slot.value
                    && let Object::List(items) = &mut *list_obj.kind_mut()
                {
                    *items = values;
                }
            }
            Some(SyncPayload::Bytes(bytes)) => {
                if let Some(slot) = self.objects.get_mut(&handle) {
                    match &mut slot.value {
                        Value::Bytes(bytes_obj) => {
                            if let Object::Bytes(values) = &mut *bytes_obj.kind_mut() {
                                *values = bytes;
                            }
                        }
                        Value::ByteArray(bytes_obj) => {
                            if let Object::ByteArray(values) = &mut *bytes_obj.kind_mut() {
                                *values = bytes;
                            }
                        }
                        _ => {}
                    }
                }
            }
            None => {}
        }

        self.cpython_sync_in_progress.remove(&handle);
    }

    fn sync_cpython_refcount(&mut self, handle: PyrsObjectHandle) {
        self.sync_cpython_storage(handle);
    }

    fn sync_cpython_storage_from_value(&mut self, handle: PyrsObjectHandle) {
        self.sync_cpython_storage_inner(handle, false);
    }

    fn sync_cpython_storage(&mut self, handle: PyrsObjectHandle) {
        self.sync_cpython_storage_inner(handle, true);
    }

    fn sync_cpython_storage_inner(&mut self, handle: PyrsObjectHandle, pull_from_raw: bool) {
        let Some(ptr) = self.cpython_ptr_by_handle.get(&handle).copied() else {
            return;
        };
        // Only write object headers for pointers owned by this context.
        let Some(raw) = self
            .cpython_allocations
            .iter()
            .copied()
            .find(|owned| (*owned).cast::<c_void>() == ptr)
        else {
            return;
        };
        if pull_from_raw {
            // Pull direct raw-storage writes (e.g. macro-style tuple/list mutations in native
            // code) back into the Value graph before mirroring Value state into raw headers.
            self.sync_value_from_cpython_storage(handle, ptr);
        }
        if let Some(slot) = self.capsules.get(&handle) {
            let pinned_name_ptr = if self.vm.is_null() {
                None
            } else {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.extension_pinned_capsule_names
                    .get(&(ptr as usize))
                    .map(|name| name.as_ptr())
            };
            // SAFETY: `raw` points to owned capsule-compatible storage for this handle.
            unsafe {
                let raw_capsule = raw.cast::<CpythonCapsuleCompatObject>();
                (*raw_capsule).ob_base.ob_refcnt = slot.refcount.max(1) as isize;
                (*raw_capsule).ob_base.ob_type = std::ptr::addr_of_mut!(PyCapsule_Type).cast();
                (*raw_capsule).pointer = slot.pointer as *mut c_void;
                (*raw_capsule).name = pinned_name_ptr
                    .or_else(|| slot.name.as_ref().map(|n| n.as_ptr()))
                    .unwrap_or(std::ptr::null());
                (*raw_capsule).context = slot.context as *mut c_void;
                (*raw_capsule).destructor = slot.cpython_destructor;
            }
            return;
        }
        let Some(slot) = self.objects.get(&handle) else {
            return;
        };
        let tuple_items = match &slot.value {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(items) => Some(items.clone()),
                _ => None,
            },
            _ => None,
        };
        let list_items = match &slot.value {
            Value::List(list_obj) => match &*list_obj.kind() {
                Object::List(items) => Some(items.clone()),
                _ => None,
            },
            _ => None,
        };
        let bytes_payload = match &slot.value {
            Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(values) => Some(values.clone()),
                _ => None,
            },
            Value::ByteArray(bytes_obj) => match &*bytes_obj.kind() {
                Object::ByteArray(values) => Some(values.clone()),
                _ => None,
            },
            _ => None,
        };
        // SAFETY: `raw` is owned allocation with `CpythonCompatObject` layout.
        unsafe {
            (*raw).ob_base.ob_base.ob_refcnt = slot.refcount.max(1) as isize;
            (*raw).ob_base.ob_base.ob_type = cpython_type_for_value(&slot.value);
            if let Some(bytes) = bytes_payload.as_ref() {
                let raw_bytes = raw.cast::<CpythonBytesCompatObject>();
                (*raw_bytes).ob_base.ob_size = bytes.len() as isize;
                (*raw_bytes).ob_shash = -1;
                let data = cpython_bytes_data_ptr(raw.cast());
                if !bytes.is_empty() {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
                }
                *data.add(bytes.len()) = 0;
                return;
            }
            if let Some(items) = list_items.as_ref() {
                let raw_list = raw.cast::<CpythonListCompatObject>();
                let (mut buffer_ptr, mut capacity) = self
                    .cpython_list_buffers
                    .get(&handle)
                    .copied()
                    .unwrap_or((std::ptr::null_mut(), 0));
                if capacity < items.len() {
                    let bytes = items
                        .len()
                        .saturating_mul(std::mem::size_of::<*mut c_void>());
                    let previous_ptr = buffer_ptr;
                    let grown = if buffer_ptr.is_null() {
                        // SAFETY: allocate list item storage.
                        malloc(bytes).cast::<*mut c_void>()
                    } else {
                        // SAFETY: grow list item storage in place when possible.
                        realloc(buffer_ptr.cast(), bytes).cast::<*mut c_void>()
                    };
                    if grown.is_null() {
                        self.set_error("out of memory resizing CPython list item buffer");
                        return;
                    }
                    buffer_ptr = grown;
                    capacity = items.len();
                    if !previous_ptr.is_null() {
                        self.cpython_owned_ptrs.remove(&(previous_ptr as usize));
                    }
                    if !buffer_ptr.is_null() {
                        self.cpython_owned_ptrs.insert(buffer_ptr as usize);
                    }
                    self.cpython_list_buffers
                        .insert(handle, (buffer_ptr, capacity));
                }
                if !buffer_ptr.is_null() {
                    for (idx, item) in items.iter().enumerate() {
                        *buffer_ptr.add(idx) = self.alloc_cpython_ptr_for_value(item.clone());
                    }
                }
                (*raw_list).ob_base.ob_size = items.len() as isize;
                (*raw_list).ob_item = buffer_ptr;
                (*raw_list).allocated = capacity as isize;
                return;
            }
            if let Some(items) = tuple_items.as_ref() {
                let capacity = (*raw).ob_base.ob_size.max(0) as usize;
                let writable = items.len().min(capacity);
                let items_ptr = cpython_tuple_items_ptr(raw.cast());
                for (idx, item) in items.iter().take(writable).enumerate() {
                    *items_ptr.add(idx) = self.alloc_cpython_ptr_for_value(item.clone());
                }
                return;
            }
            (*raw).ob_base.ob_size = 0;
        }
    }

    fn owns_cpython_allocation_ptr(&self, ptr: *mut c_void) -> bool {
        self.cpython_owned_ptrs.contains(&(ptr as usize))
    }

    fn pin_owned_cpython_allocation_for_vm(&mut self, ptr: *mut c_void) {
        if ptr.is_null() || self.vm.is_null() || !self.owns_cpython_allocation_ptr(ptr) {
            return;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        if vm
            .extension_pinned_cpython_allocation_set
            .insert(ptr as usize)
        {
            vm.extension_pinned_cpython_allocations.push(ptr);
        }
    }

    fn pin_capsule_allocation_for_vm(&mut self, ptr: *mut c_void) {
        if ptr.is_null() || self.vm.is_null() {
            return;
        }
        if !self.owns_cpython_allocation_ptr(ptr) {
            return;
        }
        let Some(handle) = self.cpython_objects_by_ptr.get(&(ptr as usize)).copied() else {
            return;
        };
        let Some(slot) = self.capsules.get(&handle) else {
            return;
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        if vm
            .extension_pinned_cpython_allocation_set
            .insert(ptr as usize)
        {
            vm.extension_pinned_cpython_allocations.push(ptr);
        }
        if let Some(name) = slot.name.as_ref() {
            let cloned_name = name.clone();
            let name_ptr = cloned_name.as_ptr();
            vm.extension_pinned_capsule_names
                .insert(ptr as usize, cloned_name);
            // SAFETY: `ptr` points to a capsule-compatible object allocation.
            unsafe {
                (*ptr.cast::<CpythonCapsuleCompatObject>()).name = name_ptr;
            }
        }
    }

    fn capsule_export(&mut self, capsule_handle: PyrsObjectHandle) -> Result<(), String> {
        let (name, pointer, context, destructor) = {
            let Some(slot) = self.capsules.get(&capsule_handle) else {
                return Err(format!("invalid capsule handle {}", capsule_handle));
            };
            let Some(name) = slot.name.as_ref() else {
                return Err("capsule_export requires named capsule".to_string());
            };
            let name = name
                .to_str()
                .map_err(|_| "capsule name must be utf-8".to_string())?
                .to_string();
            (name, slot.pointer, slot.context, slot.destructor)
        };
        self.sync_exported_capsule(Some(name.as_str()), pointer, context, destructor, true)?;
        let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        slot.exported_name = Some(name);
        Ok(())
    }

    fn capsule_import(
        &mut self,
        name: *const c_char,
        _no_block: i32,
    ) -> Result<*mut c_void, String> {
        if name.is_null() {
            return Err("capsule_import requires non-null name".to_string());
        }
        // SAFETY: caller provides valid NUL-terminated string pointer.
        let raw = unsafe { CStr::from_ptr(name) };
        let requested_name = raw
            .to_str()
            .map_err(|_| "capsule name must be utf-8".to_string())?;
        if self.vm.is_null() {
            return Err("capsule_import missing VM context".to_string());
        }
        // SAFETY: VM pointer is valid for the context lifetime.
        let vm = unsafe { &mut *self.vm };
        if requested_name == PYRS_DATETIME_CAPSULE_NAME {
            vm.ensure_builtin_datetime_capi_capsule();
        }
        if let Some(entry) = vm.extension_capsule_registry.get(requested_name) {
            return Ok(entry.pointer as *mut c_void);
        }
        let mut parts = requested_name.split('.');
        let Some(module_name) = parts.next() else {
            return Err(format!(
                "PyCapsule_Import \"{}\" is not valid",
                requested_name
            ));
        };
        if module_name.is_empty() {
            return Err(format!(
                "PyCapsule_Import \"{}\" is not valid",
                requested_name
            ));
        }
        let mut object = vm
            .builtin_import_module(vec![Value::Str(module_name.to_string())], HashMap::new())
            .map_err(|_| {
                format!(
                    "PyCapsule_Import could not import module \"{}\"",
                    module_name
                )
            })?;
        for part in parts {
            object = vm
                .builtin_getattr(vec![object, Value::Str(part.to_string())], HashMap::new())
                .map_err(|_| format!("PyCapsule_Import \"{}\" is not valid", requested_name))?;
        }
        let _ = object;
        Err(format!(
            "PyCapsule_Import \"{}\" is not valid",
            requested_name
        ))
    }

    fn capsule_new(
        &mut self,
        pointer: *mut c_void,
        name: *const c_char,
        cpython_destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> Result<PyrsObjectHandle, String> {
        self.alloc_capsule(pointer, name, cpython_destructor)
    }

    fn capsule_get_pointer(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        name: *const c_char,
    ) -> Result<*mut c_void, String> {
        if let Some(slot) = self.capsules.get(&capsule_handle) {
            if !self.capsule_name_matches(slot, name)? {
                return Err("capsule name mismatch".to_string());
            }
            return Ok(slot.pointer as *mut c_void);
        }
        if !name.is_null() && !self.vm.is_null() {
            let is_proxy = self
                .objects
                .get(&capsule_handle)
                .map(|slot| {
                    if let Value::Class(class_obj) = &slot.value
                        && let Object::Class(class_data) = &*class_obj.kind()
                    {
                        return is_cpython_proxy_class(class_data);
                    }
                    false
                })
                .unwrap_or(false);
            if is_proxy {
                // SAFETY: caller provides a valid NUL-terminated C string for capsule names.
                let requested = unsafe { CStr::from_ptr(name) }
                    .to_str()
                    .map_err(|_| "capsule name must be utf-8".to_string())?;
                // SAFETY: VM pointer is valid for active context lifetime.
                let vm = unsafe { &*self.vm };
                if let Some(entry) = vm.extension_capsule_registry.get(requested) {
                    return Ok(entry.pointer as *mut c_void);
                }
            }
        }
        Err(format!("invalid capsule handle {}", capsule_handle))
    }

    fn capsule_set_pointer(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        pointer: *mut c_void,
    ) -> Result<(), String> {
        if pointer.is_null() {
            return Err("capsule_set_pointer requires non-null pointer".to_string());
        }
        let (exported_name, context, destructor) = {
            let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
                return Err(format!("invalid capsule handle {}", capsule_handle));
            };
            slot.pointer = pointer as usize;
            (slot.exported_name.clone(), slot.context, slot.destructor)
        };
        self.sync_exported_capsule(
            exported_name.as_deref(),
            pointer as usize,
            context,
            destructor,
            false,
        )?;
        Ok(())
    }

    fn capsule_name_matches(
        &self,
        slot: &CapiCapsuleSlot,
        name: *const c_char,
    ) -> Result<bool, String> {
        let requested_name = if name.is_null() {
            None
        } else {
            // SAFETY: caller provides a valid NUL-terminated C string.
            let raw = unsafe { CStr::from_ptr(name) };
            Some(
                raw.to_str()
                    .map_err(|_| "capsule name must be utf-8".to_string())?,
            )
        };
        let expected_name = slot.name.as_ref().map(|value| value.to_string_lossy());
        Ok(match (expected_name.as_ref(), requested_name) {
            (None, None) => true,
            (Some(expected), Some(requested)) => expected.as_ref() == requested,
            _ => false,
        })
    }

    fn capsule_get_name_ptr(
        &mut self,
        capsule_handle: PyrsObjectHandle,
    ) -> Result<*const c_char, String> {
        let Some(slot) = self.capsules.get(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        Ok(slot
            .name
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr()))
    }

    fn capsule_set_context(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        context: *mut c_void,
    ) -> Result<(), String> {
        let (exported_name, pointer, destructor) = {
            let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
                return Err(format!("invalid capsule handle {}", capsule_handle));
            };
            slot.context = context as usize;
            (slot.exported_name.clone(), slot.pointer, slot.destructor)
        };
        self.sync_exported_capsule(
            exported_name.as_deref(),
            pointer,
            context as usize,
            destructor,
            false,
        )?;
        Ok(())
    }

    fn capsule_get_context(
        &mut self,
        capsule_handle: PyrsObjectHandle,
    ) -> Result<*mut c_void, String> {
        let Some(slot) = self.capsules.get(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        Ok(slot.context as *mut c_void)
    }

    fn capsule_set_destructor(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        destructor: Option<PyrsCapsuleDestructorV1>,
    ) -> Result<(), String> {
        let (exported_name, pointer, context) = {
            let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
                return Err(format!("invalid capsule handle {}", capsule_handle));
            };
            slot.destructor = destructor;
            (slot.exported_name.clone(), slot.pointer, slot.context)
        };
        self.sync_exported_capsule(
            exported_name.as_deref(),
            pointer,
            context,
            destructor,
            false,
        )?;
        Ok(())
    }

    fn capsule_get_destructor(
        &mut self,
        capsule_handle: PyrsObjectHandle,
    ) -> Result<Option<PyrsCapsuleDestructorV1>, String> {
        let Some(slot) = self.capsules.get(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        Ok(slot.destructor)
    }

    fn capsule_set_cpython_destructor(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> Result<(), String> {
        let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        slot.cpython_destructor = destructor;
        Ok(())
    }

    fn capsule_get_cpython_destructor(
        &self,
        capsule_handle: PyrsObjectHandle,
    ) -> Result<Option<unsafe extern "C" fn(*mut c_void)>, String> {
        let Some(slot) = self.capsules.get(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        Ok(slot.cpython_destructor)
    }

    fn capsule_set_name(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        name: *const c_char,
    ) -> Result<(), String> {
        let (old_exported_name, new_name, pointer, context, destructor) = {
            let Some(slot) = self.capsules.get_mut(&capsule_handle) else {
                return Err(format!("invalid capsule handle {}", capsule_handle));
            };
            let old_exported_name = slot.exported_name.clone();
            let new_name = if name.is_null() {
                slot.name = None;
                None
            } else {
                // SAFETY: caller provides valid NUL-terminated string pointer.
                let raw = unsafe { CStr::from_ptr(name) };
                let text = raw
                    .to_str()
                    .map_err(|_| "capsule name must be utf-8".to_string())?
                    .to_string();
                let value = CString::new(text.as_str())
                    .map_err(|_| "capsule name contains interior NUL".to_string())?;
                slot.name = Some(value);
                Some(text)
            };
            if old_exported_name.is_some() {
                slot.exported_name = new_name.clone();
            }
            (
                old_exported_name,
                new_name,
                slot.pointer,
                slot.context,
                slot.destructor,
            )
        };
        if let Some(old) = old_exported_name.as_deref() {
            if new_name.as_deref() != Some(old) {
                if self.vm.is_null() {
                    return Err("capsule_set_name missing VM context".to_string());
                }
                // SAFETY: VM pointer is valid for the context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.extension_capsule_registry.remove(old);
            }
        }
        self.sync_exported_capsule(new_name.as_deref(), pointer, context, destructor, false)?;
        Ok(())
    }

    fn capsule_is_valid(
        &mut self,
        capsule_handle: PyrsObjectHandle,
        name: *const c_char,
    ) -> Result<i32, String> {
        let Some(slot) = self.capsules.get(&capsule_handle) else {
            return Err(format!("invalid capsule handle {}", capsule_handle));
        };
        if self.capsule_name_matches(slot, name)? {
            Ok(1)
        } else {
            Ok(0)
        }
    }

    fn object_type(&self, handle: PyrsObjectHandle) -> Result<i32, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        let ty = match slot.value {
            Value::None => PYRS_TYPE_NONE,
            Value::Bool(_) => PYRS_TYPE_BOOL,
            Value::Int(_) => PYRS_TYPE_INT,
            Value::Str(_) => PYRS_TYPE_STR,
            Value::Float(_) => PYRS_TYPE_FLOAT,
            Value::Bytes(_) | Value::ByteArray(_) => PYRS_TYPE_BYTES,
            Value::Tuple(_) => PYRS_TYPE_TUPLE,
            Value::List(_) => PYRS_TYPE_LIST,
            Value::Dict(_) => PYRS_TYPE_DICT,
            _ => 0,
        };
        Ok(ty)
    }

    fn object_is_instance(
        &mut self,
        object_handle: PyrsObjectHandle,
        classinfo_handle: PyrsObjectHandle,
    ) -> Result<i32, String> {
        if self.vm.is_null() {
            return Err("object_is_instance missing VM context".to_string());
        }
        let object = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let classinfo = self
            .object_value(classinfo_handle)
            .ok_or_else(|| format!("invalid classinfo handle {}", classinfo_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .builtin_isinstance(vec![object, classinfo], HashMap::new())
            .map_err(|err| err.message)?;
        match value {
            Value::Bool(true) => Ok(1),
            Value::Bool(false) => Ok(0),
            other => Err(format!("isinstance returned non-bool value: {other:?}")),
        }
    }

    fn object_is_subclass(
        &mut self,
        class_handle: PyrsObjectHandle,
        classinfo_handle: PyrsObjectHandle,
    ) -> Result<i32, String> {
        if self.vm.is_null() {
            return Err("object_is_subclass missing VM context".to_string());
        }
        let class = self
            .object_value(class_handle)
            .ok_or_else(|| format!("invalid class handle {}", class_handle))?;
        let classinfo = self
            .object_value(classinfo_handle)
            .ok_or_else(|| format!("invalid classinfo handle {}", classinfo_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .builtin_issubclass(vec![class, classinfo], HashMap::new())
            .map_err(|err| err.message)?;
        match value {
            Value::Bool(true) => Ok(1),
            Value::Bool(false) => Ok(0),
            other => Err(format!("issubclass returned non-bool value: {other:?}")),
        }
    }

    fn object_get_int(&self, handle: PyrsObjectHandle) -> Result<i64, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match slot.value {
            Value::Int(value) => Ok(value),
            _ => Err(format!("object handle {} is not an int", handle)),
        }
    }

    fn object_get_bool(&self, handle: PyrsObjectHandle) -> Result<i32, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match slot.value {
            Value::Bool(value) => Ok(if value { 1 } else { 0 }),
            _ => Err(format!("object handle {} is not a bool", handle)),
        }
    }

    fn object_get_float(&self, handle: PyrsObjectHandle) -> Result<f64, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match slot.value {
            Value::Float(value) => Ok(value),
            _ => Err(format!("object handle {} is not a float", handle)),
        }
    }

    fn object_get_bytes_parts(
        &self,
        handle: PyrsObjectHandle,
    ) -> Result<(*const u8, usize), String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match &slot.value {
            Value::Bytes(bytes_obj) | Value::ByteArray(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => {
                    Ok((values.as_ptr(), values.len()))
                }
                _ => Err(format!(
                    "object handle {} has invalid bytes storage",
                    handle
                )),
            },
            _ => Err(format!("object handle {} is not bytes-like", handle)),
        }
    }

    fn object_len(&mut self, handle: PyrsObjectHandle) -> Result<usize, String> {
        if self.vm.is_null() {
            return Err("object_len missing VM context".to_string());
        }
        let value = self
            .object_value(handle)
            .ok_or_else(|| format!("invalid object handle {}", handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let length_value = vm
            .builtin_len(vec![value], HashMap::new())
            .map_err(|err| err.message)?;
        match length_value {
            Value::Int(length) => usize::try_from(length)
                .map_err(|_| format!("length {} is out of range for usize", length)),
            Value::BigInt(bigint) => {
                let text = bigint.to_string();
                let parsed = text
                    .parse::<usize>()
                    .map_err(|_| format!("length {} is out of range for usize", text))?;
                Ok(parsed)
            }
            other => Err(format!("len() returned non-int value: {other:?}")),
        }
    }

    fn object_get_item(
        &mut self,
        object_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_get_item missing VM context".to_string());
        }
        let object = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm.getitem_value(object, key).map_err(|err| err.message)?;
        Ok(self.alloc_object(value))
    }

    fn object_set_item(
        &mut self,
        object_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
        value_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("object_set_item missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        let value = self
            .object_value(value_handle)
            .ok_or_else(|| format!("invalid value handle {}", value_handle))?;
        match &target {
            Value::Dict(dict_obj) => {
                return dict_set_value_checked(dict_obj, key, value).map_err(|err| err.message);
            }
            Value::List(list_obj) => {
                if let Ok(raw_idx) = value_to_int(key.clone()) {
                    let mut list_kind = list_obj.kind_mut();
                    let Object::List(values) = &mut *list_kind else {
                        return Err(format!(
                            "object handle {} has invalid list storage",
                            object_handle
                        ));
                    };
                    let mut idx = raw_idx as isize;
                    if idx < 0 {
                        idx += values.len() as isize;
                    }
                    if idx < 0 || idx as usize >= values.len() {
                        return Err("index out of range".to_string());
                    }
                    values[idx as usize] = value;
                    return Ok(());
                }
            }
            Value::ByteArray(bytearray_obj) => {
                if let Ok(raw_idx) = value_to_int(key.clone()) {
                    let mut bytes_kind = bytearray_obj.kind_mut();
                    let Object::ByteArray(values) = &mut *bytes_kind else {
                        return Err(format!(
                            "object handle {} has invalid bytearray storage",
                            object_handle
                        ));
                    };
                    let mut idx = raw_idx as isize;
                    if idx < 0 {
                        idx += values.len() as isize;
                    }
                    if idx < 0 || idx as usize >= values.len() {
                        return Err("index out of range".to_string());
                    }
                    let byte = value_to_int(value.clone()).map_err(|err| err.message)?;
                    if !(0..=255).contains(&byte) {
                        return Err("byte must be in range(0, 256)".to_string());
                    }
                    values[idx as usize] = byte as u8;
                    return Ok(());
                }
            }
            _ => {}
        }
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let Some(setitem) = vm
            .lookup_bound_special_method(&target, "__setitem__")
            .map_err(|err| err.message)?
        else {
            return Err("object does not support item assignment".to_string());
        };
        match vm
            .call_internal(setitem, vec![key, value], HashMap::new())
            .map_err(|err| err.message)?
        {
            InternalCallOutcome::Value(_) => Ok(()),
            InternalCallOutcome::CallerExceptionHandled => Err(vm
                .runtime_error_from_active_exception("object_set_item() failed")
                .message),
        }
    }

    fn object_del_item(
        &mut self,
        object_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("object_del_item missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        match &target {
            Value::Dict(dict_obj) => {
                if dict_remove_value(dict_obj, &key).is_none() {
                    return Err("dict key not found".to_string());
                }
                return Ok(());
            }
            Value::List(list_obj) => {
                if let Ok(raw_idx) = value_to_int(key.clone()) {
                    let mut list_kind = list_obj.kind_mut();
                    let Object::List(values) = &mut *list_kind else {
                        return Err(format!(
                            "object handle {} has invalid list storage",
                            object_handle
                        ));
                    };
                    let mut idx = raw_idx as isize;
                    if idx < 0 {
                        idx += values.len() as isize;
                    }
                    if idx < 0 || idx as usize >= values.len() {
                        return Err("index out of range".to_string());
                    }
                    values.remove(idx as usize);
                    return Ok(());
                }
            }
            Value::ByteArray(bytearray_obj) => {
                if let Ok(raw_idx) = value_to_int(key.clone()) {
                    let mut bytes_kind = bytearray_obj.kind_mut();
                    let Object::ByteArray(values) = &mut *bytes_kind else {
                        return Err(format!(
                            "object handle {} has invalid bytearray storage",
                            object_handle
                        ));
                    };
                    let mut idx = raw_idx as isize;
                    if idx < 0 {
                        idx += values.len() as isize;
                    }
                    if idx < 0 || idx as usize >= values.len() {
                        return Err("index out of range".to_string());
                    }
                    values.remove(idx as usize);
                    return Ok(());
                }
            }
            _ => {}
        }
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let Some(delitem) = vm
            .lookup_bound_special_method(&target, "__delitem__")
            .map_err(|err| err.message)?
        else {
            return Err("object does not support item deletion".to_string());
        };
        match vm
            .call_internal(delitem, vec![key], HashMap::new())
            .map_err(|err| err.message)?
        {
            InternalCallOutcome::Value(_) => Ok(()),
            InternalCallOutcome::CallerExceptionHandled => Err(vm
                .runtime_error_from_active_exception("object_del_item() failed")
                .message),
        }
    }

    fn object_contains(
        &mut self,
        object_handle: PyrsObjectHandle,
        needle_handle: PyrsObjectHandle,
    ) -> Result<i32, String> {
        if self.vm.is_null() {
            return Err("object_contains missing VM context".to_string());
        }
        let container = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let needle = self
            .object_value(needle_handle)
            .ok_or_else(|| format!("invalid needle handle {}", needle_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let contains = vm
            .compare_in_runtime(needle, container)
            .map_err(|err| err.message)?;
        Ok(if contains { 1 } else { 0 })
    }

    fn object_dict_keys(
        &mut self,
        dict_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_dict_keys missing VM context".to_string());
        }
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let entries = match &*dict_obj.kind() {
            Object::Dict(entries) => entries.to_vec(),
            _ => {
                return Err(format!(
                    "object handle {} has invalid dict storage",
                    dict_handle
                ));
            }
        };
        let mut keys = Vec::with_capacity(entries.len());
        for (key, _) in entries {
            keys.push(key);
        }
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        Ok(self.alloc_object(vm.heap.alloc_list(keys)))
    }

    fn object_dict_items(
        &mut self,
        dict_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_dict_items missing VM context".to_string());
        }
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let entries = match &*dict_obj.kind() {
            Object::Dict(entries) => entries.to_vec(),
            _ => {
                return Err(format!(
                    "object handle {} has invalid dict storage",
                    dict_handle
                ));
            }
        };
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let mut items = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            items.push(vm.heap.alloc_tuple(vec![key, value]));
        }
        Ok(self.alloc_object(vm.heap.alloc_list(items)))
    }

    fn mutable_buffer_source_from_obj(source: &ObjRef) -> Option<ObjRef> {
        match &*source.kind() {
            Object::ByteArray(_) => Some(source.clone()),
            Object::MemoryView(view) => Self::mutable_buffer_source_from_obj(&view.source),
            _ => None,
        }
    }

    fn mutable_buffer_source_from_value(value: &Value) -> Option<ObjRef> {
        match value {
            Value::ByteArray(obj) => Some(obj.clone()),
            Value::MemoryView(obj) => match &*obj.kind() {
                Object::MemoryView(view) => Self::mutable_buffer_source_from_obj(&view.source),
                _ => None,
            },
            _ => None,
        }
    }

    fn readable_buffer_from_source(
        source: &ObjRef,
        start: usize,
        length: Option<usize>,
    ) -> Result<(*const u8, usize, bool), String> {
        match &*source.kind() {
            Object::Bytes(values) => {
                let (start, end) = memoryview_bounds(start, length, values.len());
                Ok((
                    values.as_ptr().wrapping_add(start),
                    end.saturating_sub(start),
                    true,
                ))
            }
            Object::ByteArray(values) => {
                let (start, end) = memoryview_bounds(start, length, values.len());
                Ok((
                    values.as_ptr().wrapping_add(start),
                    end.saturating_sub(start),
                    false,
                ))
            }
            Object::MemoryView(view) => {
                if view.released {
                    return Err("memoryview is released".to_string());
                }
                let (ptr, len, readonly) =
                    Self::readable_buffer_from_source(&view.source, view.start, view.length)?;
                let (start, end) = memoryview_bounds(start, length, len);
                Ok((ptr.wrapping_add(start), end.saturating_sub(start), readonly))
            }
            _ => Err("memoryview source is not bytes-like".to_string()),
        }
    }

    fn writable_buffer_from_source(
        source: &ObjRef,
        start: usize,
        length: Option<usize>,
    ) -> Result<(*mut u8, usize), String> {
        let mut source_kind = source.kind_mut();
        match &mut *source_kind {
            Object::ByteArray(values) => {
                let (start, end) = memoryview_bounds(start, length, values.len());
                Ok((
                    values.as_mut_ptr().wrapping_add(start),
                    end.saturating_sub(start),
                ))
            }
            Object::Bytes(_) => Err("buffer is read-only".to_string()),
            Object::MemoryView(view) => {
                if view.released {
                    return Err("memoryview is released".to_string());
                }
                let nested_source = view.source.clone();
                let nested_start = view.start;
                let nested_length = view.length;
                drop(source_kind);
                let (ptr, len) =
                    Self::writable_buffer_from_source(&nested_source, nested_start, nested_length)?;
                let (start, end) = memoryview_bounds(start, length, len);
                Ok((ptr.wrapping_add(start), end.saturating_sub(start)))
            }
            _ => Err("memoryview source is not writable bytes-like".to_string()),
        }
    }

    fn object_get_buffer(
        &mut self,
        object_handle: PyrsObjectHandle,
    ) -> Result<PyrsBufferViewV1, String> {
        let value = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let (data, len, readonly) = match &value {
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(values) => (values.as_ptr(), values.len(), true),
                _ => {
                    return Err(format!(
                        "object handle {} has invalid bytes storage",
                        object_handle
                    ));
                }
            },
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(values) => (values.as_ptr(), values.len(), false),
                _ => {
                    return Err(format!(
                        "object handle {} has invalid bytearray storage",
                        object_handle
                    ));
                }
            },
            Value::MemoryView(obj) => match &*obj.kind() {
                Object::MemoryView(view) => {
                    if view.released {
                        return Err("memoryview is released".to_string());
                    }
                    if !view.contiguous {
                        return Err("memoryview is not C-contiguous".to_string());
                    }
                    let (ptr, len, readonly) =
                        Self::readable_buffer_from_source(&view.source, view.start, view.length)?;
                    (ptr, len, readonly)
                }
                _ => {
                    return Err(format!(
                        "object handle {} has invalid memoryview storage",
                        object_handle
                    ));
                }
            },
            _ => {
                return Err(format!(
                    "object handle {} does not support buffer access",
                    object_handle
                ));
            }
        };
        if let Some(source) = Self::mutable_buffer_source_from_value(&value) {
            // SAFETY: the VM pointer is initialized for the extension context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.heap.pin_external_buffer_source(&source);
        }
        self.incref(object_handle)?;
        *self.buffer_pins.entry(object_handle).or_insert(0) += 1;
        Ok(PyrsBufferViewV1 {
            data,
            len,
            readonly: if readonly { 1 } else { 0 },
        })
    }

    fn object_get_writable_buffer(
        &mut self,
        object_handle: PyrsObjectHandle,
    ) -> Result<PyrsWritableBufferViewV1, String> {
        let value = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let (data, len) = match &value {
            Value::ByteArray(obj) => Self::writable_buffer_from_source(obj, 0, None)?,
            Value::MemoryView(obj) => {
                let (source, start, length, contiguous, released) = match &*obj.kind() {
                    Object::MemoryView(view) => (
                        view.source.clone(),
                        view.start,
                        view.length,
                        view.contiguous,
                        view.released,
                    ),
                    _ => {
                        return Err(format!(
                            "object handle {} has invalid memoryview storage",
                            object_handle
                        ));
                    }
                };
                if released {
                    return Err("memoryview is released".to_string());
                }
                if !contiguous {
                    return Err("memoryview is not C-contiguous".to_string());
                }
                Self::writable_buffer_from_source(&source, start, length)?
            }
            Value::Bytes(_) => return Err("buffer is read-only".to_string()),
            _ => {
                return Err(format!(
                    "object handle {} does not support writable buffer access",
                    object_handle
                ));
            }
        };
        if let Some(source) = Self::mutable_buffer_source_from_value(&value) {
            // SAFETY: the VM pointer is initialized for the extension context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.heap.pin_external_buffer_source(&source);
        }
        self.incref(object_handle)?;
        *self.buffer_pins.entry(object_handle).or_insert(0) += 1;
        Ok(PyrsWritableBufferViewV1 { data, len })
    }

    fn object_get_buffer_info(
        &mut self,
        object_handle: PyrsObjectHandle,
    ) -> Result<PyrsBufferInfoV1, String> {
        let value = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let snapshot = self.buffer_info_snapshot_from_value(object_handle, &value)?;
        let format_ptr = self.scratch_c_string_ptr(&snapshot.format_text)?;
        let ndim = snapshot.shape.len();
        let shape0 = snapshot.shape.first().copied().unwrap_or(0);
        let stride0 = snapshot.strides.first().copied().unwrap_or(0);
        if let Some(source) = Self::mutable_buffer_source_from_value(&value) {
            // SAFETY: the VM pointer is initialized for the extension context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.heap.pin_external_buffer_source(&source);
        }
        self.incref(object_handle)?;
        *self.buffer_pins.entry(object_handle).or_insert(0) += 1;
        Ok(PyrsBufferInfoV1 {
            data: snapshot.data,
            len: snapshot.len,
            readonly: if snapshot.readonly { 1 } else { 0 },
            itemsize: snapshot.itemsize,
            ndim,
            shape0,
            stride0,
            format: format_ptr,
            contiguous: if snapshot.contiguous { 1 } else { 0 },
        })
    }

    fn object_get_buffer_info_v2(
        &mut self,
        object_handle: PyrsObjectHandle,
    ) -> Result<PyrsBufferInfoV2, String> {
        let value = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let snapshot = self.buffer_info_snapshot_from_value(object_handle, &value)?;
        let format_ptr = self.scratch_c_string_ptr(&snapshot.format_text)?;
        let shape_ptr = self.scratch_isize_ptr(&snapshot.shape)?;
        let strides_ptr = self.scratch_isize_ptr(&snapshot.strides)?;
        if let Some(source) = Self::mutable_buffer_source_from_value(&value) {
            // SAFETY: the VM pointer is initialized for the extension context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.heap.pin_external_buffer_source(&source);
        }
        self.incref(object_handle)?;
        *self.buffer_pins.entry(object_handle).or_insert(0) += 1;
        Ok(PyrsBufferInfoV2 {
            data: snapshot.data,
            len: snapshot.len,
            readonly: if snapshot.readonly { 1 } else { 0 },
            itemsize: snapshot.itemsize,
            ndim: snapshot.shape.len(),
            shape: shape_ptr,
            strides: strides_ptr,
            format: format_ptr,
            contiguous: if snapshot.contiguous { 1 } else { 0 },
        })
    }

    fn default_buffer_shape_and_strides(len: usize, itemsize: usize) -> (Vec<isize>, Vec<isize>) {
        let safe_itemsize = itemsize.max(1);
        let logical_len = if len % safe_itemsize == 0 {
            len / safe_itemsize
        } else {
            len
        };
        (vec![logical_len as isize], vec![safe_itemsize as isize])
    }

    fn logical_nbytes_from_shape(shape: &[isize], itemsize: usize) -> Option<usize> {
        let mut elements = 1usize;
        for dim in shape {
            if *dim < 0 {
                return None;
            }
            let dim_usize = usize::try_from(*dim).ok()?;
            elements = elements.checked_mul(dim_usize)?;
        }
        elements.checked_mul(itemsize.max(1))
    }

    fn buffer_info_snapshot_from_value(
        &self,
        object_handle: PyrsObjectHandle,
        value: &Value,
    ) -> Result<BufferInfoSnapshot, String> {
        match value {
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(values) => {
                    let (shape, strides) = Self::default_buffer_shape_and_strides(values.len(), 1);
                    Ok(BufferInfoSnapshot {
                        data: values.as_ptr(),
                        len: values.len(),
                        readonly: true,
                        itemsize: 1,
                        shape,
                        strides,
                        contiguous: true,
                        format_text: "B".to_string(),
                    })
                }
                _ => Err(format!(
                    "object handle {} has invalid bytes storage",
                    object_handle
                )),
            },
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(values) => {
                    let (shape, strides) = Self::default_buffer_shape_and_strides(values.len(), 1);
                    Ok(BufferInfoSnapshot {
                        data: values.as_ptr(),
                        len: values.len(),
                        readonly: false,
                        itemsize: 1,
                        shape,
                        strides,
                        contiguous: true,
                        format_text: "B".to_string(),
                    })
                }
                _ => Err(format!(
                    "object handle {} has invalid bytearray storage",
                    object_handle
                )),
            },
            Value::MemoryView(obj) => {
                let (
                    source,
                    start,
                    length,
                    itemsize,
                    contiguous,
                    format,
                    shape_override,
                    strides_override,
                    released,
                ) = match &*obj.kind() {
                    Object::MemoryView(view) => (
                        view.source.clone(),
                        view.start,
                        view.length,
                        view.itemsize.max(1),
                        view.contiguous,
                        view.format.clone(),
                        view.shape.clone(),
                        view.strides.clone(),
                        view.released,
                    ),
                    _ => {
                        return Err(format!(
                            "object handle {} has invalid memoryview storage",
                            object_handle
                        ));
                    }
                };
                if released {
                    return Err("memoryview is released".to_string());
                }
                let (data, len, readonly) =
                    Self::readable_buffer_from_source(&source, start, length)?;
                let (shape, strides) = match (shape_override, strides_override) {
                    (Some(shape_values), Some(stride_values))
                        if !shape_values.is_empty()
                            && shape_values.len() == stride_values.len() =>
                    {
                        (shape_values, stride_values)
                    }
                    _ => Self::default_buffer_shape_and_strides(len, itemsize),
                };
                let logical_len = Self::logical_nbytes_from_shape(&shape, itemsize).unwrap_or(len);
                Ok(BufferInfoSnapshot {
                    data,
                    len: logical_len,
                    readonly,
                    itemsize,
                    shape,
                    strides,
                    contiguous,
                    format_text: format.unwrap_or_else(|| "B".to_string()),
                })
            }
            _ => Err(format!(
                "object handle {} does not support buffer info access",
                object_handle
            )),
        }
    }

    fn object_release_buffer(&mut self, object_handle: PyrsObjectHandle) -> Result<(), String> {
        let Some(pins) = self.buffer_pins.get_mut(&object_handle) else {
            return Err("buffer was not acquired for this handle".to_string());
        };
        if *pins == 0 {
            self.buffer_pins.remove(&object_handle);
            return Err("buffer was not acquired for this handle".to_string());
        }
        *pins -= 1;
        if *pins == 0 {
            self.buffer_pins.remove(&object_handle);
        }
        if let Some(value) = self.object_value(object_handle)
            && let Some(source) = Self::mutable_buffer_source_from_value(&value)
        {
            // SAFETY: the VM pointer is initialized for the extension context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.heap.unpin_external_buffer_source(&source);
        }
        self.decref(object_handle)
    }

    fn object_sequence_len(&self, handle: PyrsObjectHandle) -> Result<usize, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match &slot.value {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => Ok(values.len()),
                _ => Err(format!(
                    "object handle {} has invalid tuple storage",
                    handle
                )),
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => Ok(values.len()),
                _ => Err(format!("object handle {} has invalid list storage", handle)),
            },
            _ => Err(format!("object handle {} is not tuple/list", handle)),
        }
    }

    fn object_sequence_get_item(
        &self,
        handle: PyrsObjectHandle,
        index: usize,
    ) -> Result<Value, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match &slot.value {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("sequence index {} out of range", index)),
                _ => Err(format!(
                    "object handle {} has invalid tuple storage",
                    handle
                )),
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => values
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("sequence index {} out of range", index)),
                _ => Err(format!("object handle {} has invalid list storage", handle)),
            },
            _ => Err(format!("object handle {} is not tuple/list", handle)),
        }
    }

    fn object_get_iter(&mut self, handle: PyrsObjectHandle) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_get_iter missing VM context".to_string());
        }
        let source = self
            .object_value(handle)
            .ok_or_else(|| format!("invalid object handle {}", handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let iterator = vm
            .builtin_iter(vec![source], HashMap::new())
            .map_err(|err| err.message)?;
        Ok(self.alloc_object(iterator))
    }

    fn object_iter_next(
        &mut self,
        iter_handle: PyrsObjectHandle,
    ) -> Result<Option<PyrsObjectHandle>, String> {
        if self.vm.is_null() {
            return Err("object_iter_next missing VM context".to_string());
        }
        let iterator = self
            .object_value(iter_handle)
            .ok_or_else(|| format!("invalid iterator handle {}", iter_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        match vm
            .next_from_iterator_value(&iterator)
            .map_err(|err| err.message)?
        {
            GeneratorResumeOutcome::Yield(value) => Ok(Some(self.alloc_object(value))),
            GeneratorResumeOutcome::Complete(_) => Ok(None),
            GeneratorResumeOutcome::PropagatedException => Err(vm
                .runtime_error_from_active_exception("object_iter_next() failed")
                .message),
        }
    }

    fn object_list_obj(&self, handle: PyrsObjectHandle) -> Result<ObjRef, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match &slot.value {
            Value::List(obj) => Ok(obj.clone()),
            _ => Err(format!("object handle {} is not list", handle)),
        }
    }

    fn object_list_append(
        &mut self,
        list_handle: PyrsObjectHandle,
        item_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let list_obj = self.object_list_obj(list_handle)?;
        let item = self
            .object_value(item_handle)
            .ok_or_else(|| format!("invalid item handle {}", item_handle))?;
        let mut list_kind = list_obj.kind_mut();
        let Object::List(values) = &mut *list_kind else {
            return Err(format!(
                "object handle {} has invalid list storage",
                list_handle
            ));
        };
        values.push(item);
        Ok(())
    }

    fn object_list_set_item(
        &mut self,
        list_handle: PyrsObjectHandle,
        index: usize,
        item_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let list_obj = self.object_list_obj(list_handle)?;
        let item = self
            .object_value(item_handle)
            .ok_or_else(|| format!("invalid item handle {}", item_handle))?;
        let mut list_kind = list_obj.kind_mut();
        let Object::List(values) = &mut *list_kind else {
            return Err(format!(
                "object handle {} has invalid list storage",
                list_handle
            ));
        };
        let Some(slot) = values.get_mut(index) else {
            return Err(format!("list index {} out of range", index));
        };
        *slot = item;
        Ok(())
    }

    fn object_dict_obj(&self, handle: PyrsObjectHandle) -> Result<ObjRef, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match &slot.value {
            Value::Dict(obj) => Ok(obj.clone()),
            _ => Err(format!("object handle {} is not dict", handle)),
        }
    }

    fn object_dict_len(&self, handle: PyrsObjectHandle) -> Result<usize, String> {
        let dict_obj = self.object_dict_obj(handle)?;
        match &*dict_obj.kind() {
            Object::Dict(entries) => Ok(entries.len()),
            _ => Err(format!("object handle {} has invalid dict storage", handle)),
        }
    }

    fn object_dict_set_item(
        &mut self,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
        value_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        let value = self
            .object_value(value_handle)
            .ok_or_else(|| format!("invalid value handle {}", value_handle))?;
        dict_set_value_checked(&dict_obj, key, value).map_err(|err| err.message)
    }

    fn object_dict_get_item(
        &mut self,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        let value =
            dict_get_value(&dict_obj, &key).ok_or_else(|| "dict key not found".to_string())?;
        Ok(self.alloc_object(value))
    }

    fn object_dict_contains(
        &mut self,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> Result<i32, String> {
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        let contains = dict_contains_key_checked(&dict_obj, &key).map_err(|err| err.message)?;
        Ok(if contains { 1 } else { 0 })
    }

    fn object_dict_del_item(
        &mut self,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let dict_obj = self.object_dict_obj(dict_handle)?;
        let key = self
            .object_value(key_handle)
            .ok_or_else(|| format!("invalid key handle {}", key_handle))?;
        let removed = dict_remove_value(&dict_obj, &key);
        if removed.is_none() {
            return Err("dict key not found".to_string());
        }
        Ok(())
    }

    fn object_get_attr(
        &mut self,
        object_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_get_attr missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .builtin_getattr(
                vec![target, Value::Str(attr_name.to_string())],
                HashMap::new(),
            )
            .map_err(|err| err.message)?;
        Ok(self.alloc_object(value))
    }

    fn object_set_attr(
        &mut self,
        object_handle: PyrsObjectHandle,
        attr_name: &str,
        value_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("object_set_attr missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        let value = self
            .object_value(value_handle)
            .ok_or_else(|| format!("invalid object handle {}", value_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.builtin_setattr(
            vec![target, Value::Str(attr_name.to_string()), value],
            HashMap::new(),
        )
        .map_err(|err| err.message)?;
        Ok(())
    }

    fn object_del_attr(
        &mut self,
        object_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("object_del_attr missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.builtin_delattr(
            vec![target, Value::Str(attr_name.to_string())],
            HashMap::new(),
        )
        .map_err(|err| err.message)?;
        Ok(())
    }

    fn object_has_attr(
        &mut self,
        object_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<i32, String> {
        if self.vm.is_null() {
            return Err("object_has_attr missing VM context".to_string());
        }
        let target = self
            .object_value(object_handle)
            .ok_or_else(|| format!("invalid object handle {}", object_handle))?;
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        let value = vm
            .builtin_hasattr(
                vec![target, Value::Str(attr_name.to_string())],
                HashMap::new(),
            )
            .map_err(|err| err.message)?;
        match value {
            Value::Bool(true) => Ok(1),
            Value::Bool(false) => Ok(0),
            other => Err(format!("hasattr returned non-bool value: {other:?}")),
        }
    }

    fn object_call_noargs(
        &mut self,
        callable_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        self.object_call(callable_handle, &[], &[])
    }

    fn object_call_onearg(
        &mut self,
        callable_handle: PyrsObjectHandle,
        arg_handle: PyrsObjectHandle,
    ) -> Result<PyrsObjectHandle, String> {
        self.object_call(callable_handle, &[arg_handle], &[])
    }

    fn object_call(
        &mut self,
        callable_handle: PyrsObjectHandle,
        arg_handles: &[PyrsObjectHandle],
        kwarg_handles: &[(String, PyrsObjectHandle)],
    ) -> Result<PyrsObjectHandle, String> {
        if self.vm.is_null() {
            return Err("object_call missing VM context".to_string());
        }
        let callable = self
            .object_value(callable_handle)
            .ok_or_else(|| format!("invalid callable handle {}", callable_handle))?;
        let mut args = Vec::with_capacity(arg_handles.len());
        for handle in arg_handles {
            let value = self
                .object_value(*handle)
                .ok_or_else(|| format!("invalid argument handle {}", handle))?;
            args.push(value);
        }
        let mut kwargs = HashMap::with_capacity(kwarg_handles.len());
        for (name, handle) in kwarg_handles {
            let value = self
                .object_value(*handle)
                .ok_or_else(|| format!("invalid keyword argument handle {}", handle))?;
            if kwargs.insert(name.clone(), value).is_some() {
                return Err(format!("duplicate keyword argument '{}'", name));
            }
        }
        // SAFETY: the VM pointer is initialized for the extension context lifetime.
        let vm = unsafe { &mut *self.vm };
        if !vm.is_callable_value(&callable) {
            return Err("object_call target is not callable".to_string());
        }
        let result = match vm
            .call_internal(callable, args, kwargs)
            .map_err(|err| err.message)?
        {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(vm
                    .runtime_error_from_active_exception("object_call() failed")
                    .message);
            }
        };
        Ok(self.alloc_object(result))
    }

    fn error_get_message_ptr(&mut self) -> *const c_char {
        let Some(message) = self.last_error.clone() else {
            return std::ptr::null();
        };
        match self.scratch_c_string_ptr(&message) {
            Ok(ptr) => ptr,
            Err(_) => self
                .scratch_c_string_ptr("error message contains interior NUL")
                .unwrap_or(std::ptr::null()),
        }
    }

    fn scratch_c_string_ptr(&mut self, text: &str) -> Result<*const c_char, String> {
        let cstring =
            CString::new(text).map_err(|_| "string contains interior NUL byte".to_string())?;
        self.scratch_strings.push(cstring);
        self.scratch_strings
            .last()
            .map(|value| value.as_ptr())
            .ok_or_else(|| "failed to materialize string pointer".to_string())
    }

    fn scratch_isize_ptr(&mut self, values: &[isize]) -> Result<*const isize, String> {
        if values.is_empty() {
            return Ok(std::ptr::null());
        }
        self.scratch_isize_arrays.push(values.to_vec());
        self.scratch_isize_arrays
            .last()
            .map(|value| value.as_ptr())
            .ok_or_else(|| "failed to materialize isize array pointer".to_string())
    }

    fn object_get_string_ptr(&mut self, handle: PyrsObjectHandle) -> Result<*const c_char, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        let Value::Str(text) = &slot.value else {
            return Err(format!("object handle {} is not a str", handle));
        };
        let text = text.clone();
        self.scratch_c_string_ptr(&text)
    }
}

unsafe fn capi_context_mut<'a>(module_ctx: *mut c_void) -> Option<&'a mut ModuleCapiContext> {
    if module_ctx.is_null() {
        return None;
    }
    // SAFETY: caller guarantees `module_ctx` points to a valid `ModuleCapiContext`.
    Some(unsafe { &mut *(module_ctx as *mut ModuleCapiContext) })
}

fn cpython_valid_type_ptr(type_ptr: *mut CpythonTypeObject) -> bool {
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if type_ptr.is_null() {
        return false;
    }
    let addr = type_ptr as usize;
    addr >= MIN_VALID_PTR && addr % std::mem::align_of::<CpythonTypeObject>() == 0
}

unsafe fn cpython_number_binop_slot(
    type_ptr: *mut CpythonTypeObject,
    slot_offset: usize,
) -> Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void> {
    if !cpython_valid_type_ptr(type_ptr) {
        return None;
    }
    // SAFETY: caller guarantees `type_ptr` points to a readable PyTypeObject.
    let as_number = unsafe { (*type_ptr).tp_as_number }.cast::<CpythonNumberMethods>();
    if as_number.is_null() {
        return None;
    }
    // SAFETY: `slot_offset` is an offset within `CpythonNumberMethods`.
    let slot_ptr = unsafe {
        as_number
            .cast::<u8>()
            .add(slot_offset)
            .cast::<*mut c_void>()
    };
    // SAFETY: slot pointer is readable when `tp_as_number` points to a valid table.
    let raw = unsafe { *slot_ptr };
    if raw.is_null() {
        return None;
    }
    // SAFETY: number binary slots all have `binaryfunc` signature.
    Some(unsafe { std::mem::transmute(raw) })
}

unsafe fn cpython_mapping_ass_subscript_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32> {
    if !cpython_valid_type_ptr(type_ptr) {
        return None;
    }
    // SAFETY: caller guarantees `type_ptr` points to a readable PyTypeObject.
    let as_mapping = unsafe { (*type_ptr).tp_as_mapping }.cast::<CpythonMappingMethods>();
    if as_mapping.is_null() {
        return None;
    }
    // SAFETY: mapping slot table is readable when `tp_as_mapping` is non-null.
    let raw = unsafe { (*as_mapping).mp_ass_subscript };
    if raw.is_null() {
        return None;
    }
    // SAFETY: mapping assign slot follows `objobjargproc` signature.
    Some(unsafe { std::mem::transmute(raw) })
}

unsafe fn cpython_mapping_subscript_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void> {
    if !cpython_valid_type_ptr(type_ptr) {
        return None;
    }
    // SAFETY: caller guarantees `type_ptr` points to a readable PyTypeObject.
    let as_mapping = unsafe { (*type_ptr).tp_as_mapping }.cast::<CpythonMappingMethods>();
    if as_mapping.is_null() {
        return None;
    }
    // SAFETY: mapping slot table is readable when `tp_as_mapping` is non-null.
    let raw = unsafe { (*as_mapping).mp_subscript };
    if raw.is_null() {
        return None;
    }
    // SAFETY: mapping subscript slot follows `binaryfunc` object/key ABI.
    Some(unsafe { std::mem::transmute(raw) })
}

unsafe fn cpython_sequence_item_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void> {
    if !cpython_valid_type_ptr(type_ptr) {
        return None;
    }
    // SAFETY: caller guarantees `type_ptr` points to a readable PyTypeObject.
    let as_sequence = unsafe { (*type_ptr).tp_as_sequence }.cast::<CpythonSequenceMethods>();
    if as_sequence.is_null() {
        return None;
    }
    // SAFETY: sequence slot table is readable when `tp_as_sequence` is non-null.
    let raw = unsafe { (*as_sequence).sq_item };
    if raw.is_null() {
        return None;
    }
    // SAFETY: `sq_item` follows `ssizeargfunc` ABI.
    Some(unsafe { std::mem::transmute(raw) })
}

unsafe fn cpython_richcompare_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> *mut c_void> {
    if !cpython_valid_type_ptr(type_ptr) {
        return None;
    }
    // SAFETY: caller guarantees `type_ptr` points to a readable PyTypeObject.
    let raw = unsafe { (*type_ptr).tp_richcompare };
    if raw.is_null() {
        return None;
    }
    // SAFETY: tp_richcompare ABI matches richcmpfunc.
    Some(unsafe { std::mem::transmute(raw) })
}

fn cpython_try_richcompare_slot(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> Option<*mut c_void> {
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if left.is_null() || right.is_null() {
        return None;
    }
    let left_addr = left as usize;
    let right_addr = right as usize;
    if left_addr < MIN_VALID_PTR
        || right_addr < MIN_VALID_PTR
        || left_addr % std::mem::align_of::<usize>() != 0
        || right_addr % std::mem::align_of::<usize>() != 0
    {
        return None;
    }
    // SAFETY: pointer validity checked above for non-null/alignment/min-address.
    let left_type = unsafe {
        left.cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    // SAFETY: pointer validity checked above for non-null/alignment/min-address.
    let right_type = unsafe {
        right
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if left_type.is_null() || right_type.is_null() {
        return None;
    }
    if !cpython_valid_type_ptr(left_type) || !cpython_valid_type_ptr(right_type) {
        return None;
    }
    let trace = std::env::var_os("PYRS_TRACE_RICH_SLOT").is_some();
    if trace {
        // SAFETY: type pointers validated above.
        let left_name = unsafe {
            c_name_to_string((*left_type).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
        };
        // SAFETY: type pointers validated above.
        let right_name = unsafe {
            c_name_to_string((*right_type).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
        };
        eprintln!(
            "[cpy-rich-slot] begin op={op} left={left:p}({left_name}) right={right:p}({right_name})"
        );
    }
    // SAFETY: type pointers are non-null and read-only inspected.
    let slotv = unsafe { cpython_richcompare_slot(left_type) };
    // SAFETY: type pointers are non-null and read-only inspected.
    let mut slotw = if right_type != left_type {
        unsafe { cpython_richcompare_slot(right_type) }
    } else {
        None
    };
    if matches!((slotv, slotw), (Some(a), Some(b)) if (a as usize) == (b as usize)) {
        slotw = None;
    }
    if trace {
        eprintln!(
            "[cpy-rich-slot] slotv={} slotw={}",
            slotv.is_some(),
            slotw.is_some()
        );
    }
    if slotv.is_none() && slotw.is_none() {
        return None;
    }
    let not_implemented = std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast::<c_void>();
    if let Some(slotv_fn) = slotv {
        if let Some(slotw_fn) = slotw
            // SAFETY: type pointers are valid for subtype test.
            && unsafe { PyType_IsSubtype(right_type.cast(), left_type.cast()) != 0 }
        {
            // SAFETY: richcompare slot ABI matches richcmpfunc.
            let result = unsafe { slotw_fn(left, right, op) };
            if result.is_null() {
                if trace {
                    eprintln!("[cpy-rich-slot] subtype-right slot returned error");
                }
                return Some(std::ptr::null_mut());
            }
            if result != not_implemented {
                if trace {
                    eprintln!(
                        "[cpy-rich-slot] subtype-right slot returned value {:p}",
                        result
                    );
                }
                return Some(result);
            }
            // SAFETY: slot returned new reference to NotImplemented.
            unsafe { Py_DecRef(result) };
            slotw = None;
        }
        // SAFETY: richcompare slot ABI matches richcmpfunc.
        let result = unsafe { slotv_fn(left, right, op) };
        if result.is_null() {
            if trace {
                eprintln!("[cpy-rich-slot] left slot returned error");
            }
            return Some(std::ptr::null_mut());
        }
        if result != not_implemented {
            if trace {
                eprintln!("[cpy-rich-slot] left slot returned value {:p}", result);
            }
            return Some(result);
        }
        // SAFETY: slot returned new reference to NotImplemented.
        unsafe { Py_DecRef(result) };
    }
    if let Some(slotw_fn) = slotw {
        // SAFETY: richcompare slot ABI matches richcmpfunc.
        let result = unsafe { slotw_fn(left, right, op) };
        if result.is_null() {
            if trace {
                eprintln!("[cpy-rich-slot] right slot returned error");
            }
            return Some(std::ptr::null_mut());
        }
        if result != not_implemented {
            if trace {
                eprintln!("[cpy-rich-slot] right slot returned value {:p}", result);
            }
            return Some(result);
        }
        // SAFETY: slot returned new reference to NotImplemented.
        unsafe { Py_DecRef(result) };
    }
    None
}

fn cpython_try_binary_number_slot(
    left: *mut c_void,
    right: *mut c_void,
    slot_offset: usize,
) -> Option<*mut c_void> {
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if left.is_null() || right.is_null() {
        return None;
    }
    let left_addr = left as usize;
    let right_addr = right as usize;
    if left_addr < MIN_VALID_PTR
        || right_addr < MIN_VALID_PTR
        || left_addr % std::mem::align_of::<usize>() != 0
        || right_addr % std::mem::align_of::<usize>() != 0
    {
        return None;
    }
    // SAFETY: pointer validity checked above for non-null/alignment/min-address.
    let left_type = unsafe {
        left.cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    // SAFETY: pointer validity checked above for non-null/alignment/min-address.
    let right_type = unsafe {
        right
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if left_type.is_null() || right_type.is_null() {
        return None;
    }
    // SAFETY: type pointers are non-null and read-only inspected.
    let slotv = unsafe { cpython_number_binop_slot(left_type, slot_offset) };
    // SAFETY: type pointers are non-null and read-only inspected.
    let mut slotw = if right_type != left_type {
        unsafe { cpython_number_binop_slot(right_type, slot_offset) }
    } else {
        None
    };
    if matches!((slotv, slotw), (Some(a), Some(b)) if (a as usize) == (b as usize)) {
        slotw = None;
    }
    if slotv.is_none() && slotw.is_none() {
        return None;
    }
    let not_implemented = std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast::<c_void>();
    if let Some(slotv_fn) = slotv {
        if let Some(slotw_fn) = slotw
            // SAFETY: type pointers are valid for subtype test.
            && unsafe { PyType_IsSubtype(right_type.cast(), left_type.cast()) != 0 }
        {
            // SAFETY: binary slot ABI matches `binaryfunc`.
            let result = unsafe { slotw_fn(left, right) };
            if result.is_null() {
                return Some(std::ptr::null_mut());
            }
            if result != not_implemented {
                return Some(result);
            }
            // SAFETY: slot returned new reference to NotImplemented.
            unsafe { Py_DecRef(result) };
            slotw = None;
        }
        // SAFETY: binary slot ABI matches `binaryfunc`.
        let result = unsafe { slotv_fn(left, right) };
        if result.is_null() {
            return Some(std::ptr::null_mut());
        }
        if result != not_implemented {
            return Some(result);
        }
        // SAFETY: slot returned new reference to NotImplemented.
        unsafe { Py_DecRef(result) };
    }
    if let Some(slotw_fn) = slotw {
        // SAFETY: binary slot ABI matches `binaryfunc`.
        let result = unsafe { slotw_fn(left, right) };
        if result.is_null() {
            return Some(std::ptr::null_mut());
        }
        if result != not_implemented {
            return Some(result);
        }
        // SAFETY: slot returned new reference to NotImplemented.
        unsafe { Py_DecRef(result) };
    }
    None
}

fn cpython_call_object(
    callable: *mut c_void,
    args: Vec<Value>,
    kwargs: HashMap<String, Value>,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let callable_ptr = callable;
        if context.vm.is_null() {
            context.set_error("missing VM context for object call");
            return std::ptr::null_mut();
        }
        if let Some(result) = context.try_native_tp_call(callable_ptr, &args, &kwargs) {
            return result;
        }
        let Some(mut callable) = context.cpython_value_from_ptr_or_proxy(callable_ptr) else {
            context.set_error("unknown callable object pointer");
            return std::ptr::null_mut();
        };
        // C-API exception globals resolve to `Value::ExceptionType`; call dispatch
        // expects concrete class objects for constructor invocation.
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        if let Value::ExceptionType(name) = callable {
            callable = Value::Class(vm.alloc_synthetic_exception_class(&name));
        }
        let trace_ufunc_errors = std::env::var_os("PYRS_TRACE_CPY_UFUNC_ERRORS").is_some();
        let callable_tag = if trace_ufunc_errors {
            Some(cpython_value_debug_tag(&callable))
        } else {
            None
        };
        let arg_count = args.len();
        let kwarg_count = kwargs.len();
        if std::env::var_os("PYRS_TRACE_CPY_API").is_some() {
            eprintln!(
                "[cpy-api] cpython_call_object ptr={:p} callable={}",
                callable_ptr,
                cpython_value_debug_tag(&callable)
            );
        }
        match vm.call_internal(callable, args, kwargs) {
            Ok(InternalCallOutcome::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                if context.current_error.is_some() {
                    cpython_clear_active_exception(vm);
                    return std::ptr::null_mut();
                }
                let active_exception = vm
                    .frames
                    .last()
                    .and_then(|frame| frame.active_exception.clone());
                if let Some(exception_value) = active_exception {
                    if std::env::var_os("PYRS_TRACE_CPY_CALL_EXC").is_some() {
                        eprintln!(
                            "[cpy-call-exc] value={}",
                            cpython_value_debug_tag(&exception_value)
                        );
                    }
                    let pvalue = context.alloc_cpython_ptr_for_value(exception_value.clone());
                    let ptype = cpython_exception_type_ptr_for_value(context, &exception_value)
                        .or_else(|| {
                            let inferred = cpython_exception_type_ptr(pvalue);
                            if inferred.is_null() {
                                None
                            } else {
                                Some(inferred)
                            }
                        })
                        .unwrap_or_else(|| unsafe { PyExc_RuntimeError });
                    let ptraceback = cpython_exception_traceback_ptr_for_value(
                        context,
                        &exception_value,
                    )
                    .unwrap_or(std::ptr::null_mut());
                    let message = vm
                        .runtime_error_from_active_exception("object call failed")
                        .message;
                    if trace_ufunc_errors
                        && message.contains("_UFunc")
                    {
                        let stack = vm
                            .frames
                            .iter()
                            .rev()
                            .take(8)
                            .map(|frame| {
                                format!("{}@{}", frame.code.name, frame.code.filename)
                            })
                            .collect::<Vec<_>>()
                            .join(" <- ");
                        eprintln!(
                            "[cpy-call-ufunc] callable_ptr={:p} callable={} args={} kwargs={} stack={}",
                            callable_ptr,
                            callable_tag.as_deref().unwrap_or("<unknown>"),
                            arg_count,
                            kwarg_count,
                            stack
                        );
                    }
                    if std::env::var_os("PYRS_TRACE_CPY_CTYPES_ERROR").is_some()
                        && message.contains("ModuleNotFoundError: module '_ctypes' not found")
                    {
                        let stack = vm
                            .frames
                            .iter()
                            .rev()
                            .take(8)
                            .map(|frame| format!("{}@{}", frame.code.name, frame.code.filename))
                            .collect::<Vec<_>>()
                            .join(" <- ");
                        eprintln!(
                            "[cpy-call-ctypes] callable_ptr={:p} callable={} args={} kwargs={} stack={}",
                            callable_ptr,
                            callable_tag.as_deref().unwrap_or("<unknown>"),
                            arg_count,
                            kwarg_count,
                            stack
                        );
                    }
                    context.set_error_state(ptype, pvalue, ptraceback, message);
                } else {
                    context.set_error(
                        vm.runtime_error_from_active_exception("object call failed")
                            .message,
                    );
                }
                std::ptr::null_mut()
            }
            Err(err) => {
                if context.current_error.is_some() {
                    return std::ptr::null_mut();
                }
                let message = err.message;
                if let Some(exception_name) = cpython_exception_name_from_runtime_message(&message)
                {
                    if trace_ufunc_errors && message.contains("_UFunc") {
                        let map_hit = context
                            .exception_type_ptr_by_name
                            .get(&exception_name)
                            .copied();
                        eprintln!(
                            "[cpy-call-ufunc-err] exception_name={} map_hit={map_hit:?}",
                            exception_name
                        );
                    }
                    let ptype = context
                        .exception_type_ptr_by_name
                        .get(&exception_name)
                        .copied()
                        .map(|ptr| ptr as *mut c_void)
                        .or_else(|| cpython_exception_ptr_for_name(&exception_name))
                        .unwrap_or_else(|| unsafe { PyExc_RuntimeError });
                    let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
                    context.set_error_state(ptype, pvalue, std::ptr::null_mut(), message);
                } else {
                    context.set_error(message);
                }
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_structseq_count_fields(
    fields: *mut CpythonStructSequenceField,
) -> Result<usize, String> {
    if fields.is_null() {
        return Err("PyStructSequence_NewType expected non-null fields".to_string());
    }
    let mut count = 0usize;
    let mut cursor = fields;
    // Field table is null-name terminated per CPython contract.
    while count < 8192 {
        // SAFETY: `cursor` points into caller-owned contiguous field table.
        let name_ptr = unsafe { (*cursor).name };
        if name_ptr.is_null() {
            return Ok(count);
        }
        count += 1;
        // SAFETY: `cursor` advances over contiguous field entries.
        cursor = unsafe { cursor.add(1) };
    }
    Err("PyStructSequence_NewType field table is not terminated".to_string())
}

fn cpython_unicode_text_from_value(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.clone()),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get(STR_BACKING_STORAGE_ATTR) {
                    Some(Value::Str(text)) => Some(text.clone()),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

fn cpython_call_method_for_capi(
    context: &mut ModuleCapiContext,
    receiver: Value,
    method: &str,
    args: Vec<Value>,
    api_name: &str,
) -> Option<Value> {
    let callable = match cpython_getattr_in_context(context, receiver, method) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(format!("{api_name} {err}"));
            return None;
        }
    };
    match cpython_call_internal_in_context(context, callable, args, HashMap::new()) {
        Ok(value) => Some(value),
        Err(err) => {
            context.set_error(format!("{api_name} {err}"));
            None
        }
    }
}

fn cpython_codec_name_or_default(
    encoding: *const c_char,
    default_name: &str,
    api_name: &str,
) -> Result<String, String> {
    if encoding.is_null() {
        return Ok(default_name.to_string());
    }
    // SAFETY: caller passes NUL-terminated non-null C string for encoding names.
    unsafe { c_name_to_string(encoding) }.map_err(|err| format!("{api_name} {err}"))
}

fn cpython_codec_error_name_optional(
    errors: *const c_char,
    api_name: &str,
) -> Result<Option<String>, String> {
    if errors.is_null() {
        return Ok(None);
    }
    // SAFETY: caller passes NUL-terminated non-null C string for error names.
    unsafe { c_name_to_string(errors) }
        .map(Some)
        .map_err(|err| format!("{api_name} {err}"))
}

fn cpython_unicode_decode_with_codec_in_context(
    context: &mut ModuleCapiContext,
    source: Value,
    encoding_name: String,
    errors_name: Option<String>,
    api_name: &str,
) -> Option<Value> {
    let mut args = vec![source, Value::Str(encoding_name.clone())];
    if let Some(errors_name) = errors_name {
        args.push(Value::Str(errors_name));
    }
    let decoded = match cpython_call_internal_in_context(
        context,
        Value::Builtin(BuiltinFunction::CodecsDecode),
        args,
        HashMap::new(),
    ) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(format!("{api_name} {err}"));
            return None;
        }
    };
    if cpython_unicode_text_from_value(&decoded).is_none() {
        let got = if context.vm.is_null() {
            "object".to_string()
        } else {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            unsafe { (&mut *context.vm).value_type_name_for_error(&decoded) }
        };
        context.set_error(format!(
            "'{encoding_name}' decoder returned '{got}' instead of 'str'; use codecs.decode() to decode to arbitrary types"
        ));
        return None;
    }
    Some(decoded)
}

fn cpython_unicode_encode_with_codec_in_context(
    context: &mut ModuleCapiContext,
    source: Value,
    encoding_name: String,
    errors_name: Option<String>,
    api_name: &str,
) -> Option<Value> {
    let mut args = vec![source, Value::Str(encoding_name.clone())];
    if let Some(errors_name) = errors_name {
        args.push(Value::Str(errors_name));
    }
    let encoded = match cpython_call_internal_in_context(
        context,
        Value::Builtin(BuiltinFunction::CodecsEncode),
        args,
        HashMap::new(),
    ) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(format!("{api_name} {err}"));
            return None;
        }
    };
    Some(encoded)
}

fn cpython_codec_name_normalized(name: &str) -> String {
    name.chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' {
                Some(ch.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect()
}

fn cpython_codec_is_rot13(name: &str) -> bool {
    matches!(
        cpython_codec_name_normalized(name).as_str(),
        "rot13" | "rot.13"
    )
}

fn cpython_rot13_text(text: &str) -> String {
    fn rotate(ch: char, base: char) -> char {
        let offset = ch as u32 - base as u32;
        let rotated = (offset + 13) % 26 + base as u32;
        char::from_u32(rotated).unwrap_or(ch)
    }

    text.chars()
        .map(|ch| {
            if ch.is_ascii_lowercase() {
                rotate(ch, 'a')
            } else if ch.is_ascii_uppercase() {
                rotate(ch, 'A')
            } else {
                ch
            }
        })
        .collect()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromString(value: *const c_char) -> *mut c_void {
    match unsafe { c_name_to_string(value) } {
        Ok(text) => with_active_cpython_context_mut(|context| {
            context.alloc_cpython_ptr_for_value(Value::Str(text))
        })
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            std::ptr::null_mut()
        }),
        Err(err) => {
            cpython_set_error(format!(
                "PyUnicode_FromString received invalid string: {err}"
            ));
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromStringAndSize(
    value: *const c_char,
    len: isize,
) -> *mut c_void {
    if len < 0 {
        cpython_set_error("PyUnicode_FromStringAndSize received negative length");
        return std::ptr::null_mut();
    }
    if value.is_null() && len != 0 {
        cpython_set_error("PyUnicode_FromStringAndSize received null pointer with non-zero length");
        return std::ptr::null_mut();
    }
    let bytes = if len == 0 {
        Vec::new()
    } else {
        // SAFETY: caller guarantees `value` points to at least len bytes.
        unsafe { std::slice::from_raw_parts(value.cast::<u8>(), len as usize).to_vec() }
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    cpython_new_ptr_for_value(Value::Str(text))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromWideChar(value: *const Cwchar, len: isize) -> *mut c_void {
    let text = match unsafe { cpython_wide_ptr_to_string(value, len, "PyUnicode_FromWideChar") } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_new_ptr_for_value(Value::Str(text))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsWideCharString(
    unicode: *mut c_void,
    size: *mut isize,
) -> *mut Cwchar {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            context.set_error("PyUnicode_AsWideCharString received unknown object pointer");
            return std::ptr::null_mut();
        };
        let Some(text) = cpython_unicode_text_from_value(&value) else {
            context.set_error("PyUnicode_AsWideCharString expected str object");
            return std::ptr::null_mut();
        };
        if size.is_null() && text.chars().any(|ch| ch == '\0') {
            cpython_set_typed_error(unsafe { PyExc_ValueError }, "embedded null character");
            return std::ptr::null_mut();
        }
        let units = cpython_string_to_wide_units(&text);
        if !size.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe {
                *size = units.len() as isize;
            }
        }
        let Some(byte_len) = units
            .len()
            .checked_add(1)
            .and_then(|count| count.checked_mul(std::mem::size_of::<Cwchar>()))
        else {
            context.set_error("PyUnicode_AsWideCharString size overflow");
            return std::ptr::null_mut();
        };
        // SAFETY: allocated by CPython-compatible allocator and owned by caller.
        let raw = unsafe { PyMem_Malloc(byte_len) }.cast::<Cwchar>();
        if raw.is_null() {
            unsafe { PyErr_NoMemory() };
            return std::ptr::null_mut();
        }
        if !units.is_empty() {
            // SAFETY: destination has capacity for `units.len()` elements.
            unsafe {
                std::ptr::copy_nonoverlapping(units.as_ptr(), raw, units.len());
            }
        }
        // SAFETY: destination has at least one trailing slot for NUL terminator.
        unsafe {
            *raw.add(units.len()) = 0;
        }
        raw
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsWideChar(
    unicode: *mut c_void,
    value: *mut Cwchar,
    size: isize,
) -> isize {
    if size < 0 {
        cpython_set_error("PyUnicode_AsWideChar received negative size");
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        let Some(unicode_value) = context.cpython_value_from_ptr(unicode) else {
            context.set_error("PyUnicode_AsWideChar received unknown object pointer");
            return -1;
        };
        let Some(text) = cpython_unicode_text_from_value(&unicode_value) else {
            context.set_error("PyUnicode_AsWideChar expected str object");
            return -1;
        };
        let units = cpython_string_to_wide_units(&text);
        if value.is_null() {
            if size == 0 {
                return units.len() as isize;
            }
            context.set_error("PyUnicode_AsWideChar requires non-null output buffer");
            return -1;
        }
        let write_cap = size as usize;
        let write_len = write_cap.min(units.len());
        if write_len > 0 {
            // SAFETY: caller-provided output buffer has `write_cap` writable elements.
            unsafe {
                std::ptr::copy_nonoverlapping(units.as_ptr(), value, write_len);
            }
        }
        if write_len < write_cap {
            // SAFETY: `write_len < write_cap` guarantees valid trailing slot.
            unsafe {
                *value.add(write_len) = 0;
            }
        }
        write_len as isize
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromEncodedObject(
    object: *mut c_void,
    _encoding: *const c_char,
    _errors: *const c_char,
) -> *mut c_void {
    match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => cpython_new_ptr_for_value(Value::Str(text)),
        Ok(Value::Bytes(bytes_obj)) | Ok(Value::ByteArray(bytes_obj)) => match &*bytes_obj.kind() {
            Object::Bytes(values) | Object::ByteArray(values) => {
                let text = String::from_utf8_lossy(values).into_owned();
                cpython_new_ptr_for_value(Value::Str(text))
            }
            _ => {
                cpython_set_error("PyUnicode_FromEncodedObject encountered invalid bytes storage");
                std::ptr::null_mut()
            }
        },
        Ok(_) => {
            cpython_set_error("PyUnicode_FromEncodedObject expects str/bytes-like object");
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromKindAndData(
    kind: i32,
    buffer: *const c_void,
    size: isize,
) -> *mut c_void {
    if size < 0 {
        cpython_set_error("PyUnicode_FromKindAndData received negative size");
        return std::ptr::null_mut();
    }
    if buffer.is_null() && size != 0 {
        cpython_set_error("PyUnicode_FromKindAndData received null buffer with non-zero size");
        return std::ptr::null_mut();
    }
    let text = match kind {
        1 => {
            let bytes = if size == 0 {
                &[][..]
            } else {
                // SAFETY: caller guarantees buffer points to `size` bytes.
                unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), size as usize) }
            };
            String::from_utf8_lossy(bytes).into_owned()
        }
        2 => {
            let units = if size == 0 {
                &[][..]
            } else {
                // SAFETY: caller guarantees buffer points to `size` u16 values.
                unsafe { std::slice::from_raw_parts(buffer.cast::<u16>(), size as usize) }
            };
            String::from_utf16_lossy(units)
        }
        4 => {
            let units = if size == 0 {
                &[][..]
            } else {
                // SAFETY: caller guarantees buffer points to `size` u32 values.
                unsafe { std::slice::from_raw_parts(buffer.cast::<u32>(), size as usize) }
            };
            units
                .iter()
                .filter_map(|codepoint| char::from_u32(*codepoint))
                .collect()
        }
        _ => {
            cpython_set_error("PyUnicode_FromKindAndData received unsupported kind");
            return std::ptr::null_mut();
        }
    };
    cpython_new_ptr_for_value(Value::Str(text))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_New(size: isize, maxchar: c_uint) -> *mut c_void {
    if size == 0 {
        return cpython_new_ptr_for_value(Value::Str(String::new()));
    }
    if size < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "Negative size passed to PyUnicode_New",
        );
        return std::ptr::null_mut();
    }
    if maxchar > 0x10ffff {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "invalid maximum character passed to PyUnicode_New",
        );
        return std::ptr::null_mut();
    }
    cpython_new_ptr_for_value(Value::Str("\0".repeat(size as usize)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF8(object: *mut c_void) -> *const c_char {
    match with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyUnicode_AsUTF8 received unknown object pointer");
            return std::ptr::null();
        };
        let Value::Str(text) = value else {
            context.set_error("PyUnicode_AsUTF8 expected str object");
            return std::ptr::null();
        };
        context
            .scratch_c_string_ptr(&text)
            .unwrap_or(std::ptr::null())
    }) {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF8AndSize(
    object: *mut c_void,
    out_len: *mut isize,
) -> *const c_char {
    if out_len.is_null() {
        cpython_set_error("PyUnicode_AsUTF8AndSize requires non-null size output");
        return std::ptr::null();
    }
    let ptr = unsafe { PyUnicode_AsUTF8(object) };
    if ptr.is_null() {
        return std::ptr::null();
    }
    match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => {
            // SAFETY: caller provided writable out pointer.
            unsafe { *out_len = text.len() as isize };
            ptr
        }
        _ => std::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF8String(object: *mut c_void) -> *mut c_void {
    let ptr = unsafe { PyUnicode_AsUTF8(object) };
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: returned pointer is NUL-terminated scratch string.
    let bytes = unsafe { CStr::from_ptr(ptr).to_bytes().to_vec() };
    cpython_new_bytes_ptr(bytes)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsASCIIString(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsUTF8String(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsLatin1String(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsUTF8String(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsMBCSString(object: *mut c_void) -> *mut c_void {
    cpython_unicode_encode_with_encoding_name(
        object,
        cpython_codepage_encoding_name(0),
        std::ptr::null(),
        "PyUnicode_AsMBCSString",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsCharmapString(
    object: *mut c_void,
    mapping: *mut c_void,
) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if mapping.is_null() {
        let value = match cpython_value_from_ptr(object) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
        let Some(text) = cpython_unicode_text_from_value(&value) else {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "PyUnicode_AsCharmapString expected str object",
            );
            return std::ptr::null_mut();
        };
        let mut out = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let code = ch as u32;
            if code > 0xFF {
                cpython_set_typed_error(
                    unsafe { PyExc_UnicodeEncodeError },
                    "character maps outside latin-1 range",
                );
                return std::ptr::null_mut();
            }
            out.push(code as u8);
        }
        return cpython_new_bytes_ptr(out);
    }
    with_active_cpython_context_mut(|context| {
        let Some(unicode_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyUnicode_AsCharmapString received unknown object");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&unicode_value).is_none() {
            context.set_error("PyUnicode_AsCharmapString expected str object");
            return std::ptr::null_mut();
        }
        let mapping_value = if mapping.is_null() {
            Value::None
        } else if let Some(value) = context.cpython_value_from_ptr_or_proxy(mapping) {
            value
        } else {
            context.set_error("PyUnicode_AsCharmapString received unknown mapping object");
            return std::ptr::null_mut();
        };
        let codec_module = match cpython_codec_module_in_context(context) {
            Ok(module) => module,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let charmap_encode = {
            let Object::Module(module_data) = &*codec_module.kind() else {
                context.set_error("codecs module is invalid");
                return std::ptr::null_mut();
            };
            match module_data.globals.get("charmap_encode").cloned() {
                Some(value) => value,
                None => {
                    context.set_error("codecs.charmap_encode unavailable");
                    return std::ptr::null_mut();
                }
            }
        };
        let encoded_tuple = match cpython_call_internal_in_context(
            context,
            charmap_encode,
            vec![
                unicode_value,
                Value::Str("strict".to_string()),
                mapping_value,
            ],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let Value::Tuple(parts) = encoded_tuple else {
            context.set_error("codecs.charmap_encode() returned non-tuple");
            return std::ptr::null_mut();
        };
        let Object::Tuple(items) = &*parts.kind() else {
            context.set_error("codecs.charmap_encode() returned invalid tuple");
            return std::ptr::null_mut();
        };
        let Some(encoded) = items.first().cloned() else {
            context.set_error("codecs.charmap_encode() returned empty tuple");
            return std::ptr::null_mut();
        };
        match &encoded {
            Value::Bytes(_) => context.alloc_cpython_ptr_for_value(encoded),
            _ => {
                context.set_error("codecs.charmap_encode() did not return bytes");
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsRawUnicodeEscapeString(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedObject(object, c"raw_unicode_escape".as_ptr(), std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUnicodeEscapeString(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedObject(object, c"unicode_escape".as_ptr(), std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF16String(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedObject(object, c"utf-16".as_ptr(), std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF32String(object: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedObject(object, c"utf-32".as_ptr(), std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsEncodedString(
    object: *mut c_void,
    _encoding: *const c_char,
    _errors: *const c_char,
) -> *mut c_void {
    unsafe { PyUnicode_AsUTF8String(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Compare(left: *mut c_void, right: *mut c_void) -> i32 {
    let left = match cpython_value_from_ptr(left) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Compare expected str left operand");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let right = match cpython_value_from_ptr(right) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Compare expected str right operand");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    use std::cmp::Ordering;
    match left.cmp(&right) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_CompareWithASCIIString(
    left: *mut c_void,
    right: *const c_char,
) -> i32 {
    let right = unsafe { PyUnicode_FromString(right) };
    if right.is_null() {
        return -1;
    }
    let result = unsafe { PyUnicode_Compare(left, right) };
    unsafe { Py_DecRef(right) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Concat(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    let left = match cpython_value_from_ptr(left) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Concat expected str left operand");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let right = match cpython_value_from_ptr(right) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Concat expected str right operand");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_new_ptr_for_value(Value::Str(format!("{left}{right}")))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Contains(container: *mut c_void, element: *mut c_void) -> i32 {
    let haystack = match cpython_value_from_ptr(container) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Contains expected str container");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let needle = match cpython_value_from_ptr(element) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Contains expected str element");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    i32::from(haystack.contains(&needle))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Format(format: *mut c_void, arg: *mut c_void) -> *mut c_void {
    unsafe { PyObject_Format(arg, format) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_GetLength(object: *mut c_void) -> isize {
    match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text.chars().count() as isize,
        Ok(_) => {
            cpython_set_error("PyUnicode_GetLength expected str object");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_InternFromString(value: *const c_char) -> *mut c_void {
    if value.is_null() {
        cpython_set_error("PyUnicode_InternFromString received null string");
        return std::ptr::null_mut();
    }
    let text = match unsafe { c_name_to_string(value) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(format!(
                "PyUnicode_InternFromString received invalid string: {err}"
            ));
            return std::ptr::null_mut();
        }
    };
    if let Some(existing) = cpython_lookup_interned_unicode_ptr(&text) {
        unsafe { Py_IncRef(existing) };
        return existing;
    }
    let created = unsafe { PyUnicode_FromString(value) };
    if created.is_null() {
        return std::ptr::null_mut();
    }
    cpython_register_interned_unicode(&text, created);
    created
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromObject(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if object.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyUnicode_FromObject received unknown object pointer");
            return std::ptr::null_mut();
        };
        match value {
            Value::Str(_) => context.alloc_cpython_ptr_for_value(value),
            other => match cpython_unicode_text_from_value(&other) {
                Some(text) => context.alloc_cpython_ptr_for_value(Value::Str(text)),
                None => {
                    let got = if context.vm.is_null() {
                        "object".to_string()
                    } else {
                        // SAFETY: VM pointer is valid for active C-API context lifetime.
                        unsafe { (&mut *context.vm).value_type_name_for_error(&other) }
                    };
                    context.set_error(format!("Can't convert '{got}' object to str implicitly"));
                    std::ptr::null_mut()
                }
            },
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromOrdinal(ordinal: c_int) -> *mut c_void {
    if !(0..=0x10FFFF).contains(&ordinal) {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "chr() arg not in range(0x110000)",
        );
        return std::ptr::null_mut();
    }
    let Some(ch) = char::from_u32(ordinal as u32) else {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "chr() arg not in range(0x110000)",
        );
        return std::ptr::null_mut();
    };
    cpython_new_ptr_for_value(Value::Str(ch.to_string()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_GetDefaultEncoding() -> *const c_char {
    static UTF8: &[u8] = b"utf-8\0";
    UTF8.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Equal(str1: *mut c_void, str2: *mut c_void) -> c_int {
    with_active_cpython_context_mut(|context| {
        let Some(value1) = context.cpython_value_from_ptr(str1) else {
            context.set_error("PyUnicode_Equal received unknown first pointer");
            return -1;
        };
        let Some(value2) = context.cpython_value_from_ptr(str2) else {
            context.set_error("PyUnicode_Equal received unknown second pointer");
            return -1;
        };
        let Some(text1) = cpython_unicode_text_from_value(&value1) else {
            let got = if context.vm.is_null() {
                "object".to_string()
            } else {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                unsafe { (&mut *context.vm).value_type_name_for_error(&value1) }
            };
            context.set_error(format!("first argument must be str, not {got}"));
            return -1;
        };
        let Some(text2) = cpython_unicode_text_from_value(&value2) else {
            let got = if context.vm.is_null() {
                "object".to_string()
            } else {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                unsafe { (&mut *context.vm).value_type_name_for_error(&value2) }
            };
            context.set_error(format!("second argument must be str, not {got}"));
            return -1;
        };
        if text1 == text2 { 1 } else { 0 }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EqualToUTF8(unicode: *mut c_void, text: *const c_char) -> c_int {
    if text.is_null() {
        return 0;
    }
    let utf8 = match unsafe { CStr::from_ptr(text).to_str() } {
        Ok(value) => value,
        Err(_) => return 0,
    };
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            return 0;
        };
        let Some(unicode_text) = cpython_unicode_text_from_value(&value) else {
            return 0;
        };
        if unicode_text == utf8 { 1 } else { 0 }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EqualToUTF8AndSize(
    unicode: *mut c_void,
    text: *const c_char,
    size: isize,
) -> c_int {
    if text.is_null() || size < 0 {
        return 0;
    }
    let bytes = if size == 0 {
        &[][..]
    } else {
        // SAFETY: caller guarantees readable `size` bytes at `text`.
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), size as usize) }
    };
    let utf8 = match std::str::from_utf8(bytes) {
        Ok(value) => value,
        Err(_) => return 0,
    };
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            return 0;
        };
        let Some(unicode_text) = cpython_unicode_text_from_value(&value) else {
            return 0;
        };
        if unicode_text == utf8 { 1 } else { 0 }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_ReadChar(unicode: *mut c_void, index: isize) -> u32 {
    let text = match cpython_value_from_ptr(unicode) {
        Ok(value) => match cpython_unicode_text_from_value(&value) {
            Some(text) => text,
            None => {
                unsafe { PyErr_BadArgument() };
                return u32::MAX;
            }
        },
        Err(err) => {
            cpython_set_error(err);
            return u32::MAX;
        }
    };
    if index < 0 || index >= text.chars().count() as isize {
        cpython_set_typed_error(unsafe { PyExc_IndexError }, "string index out of range");
        return u32::MAX;
    }
    text.chars()
        .nth(index as usize)
        .map(|ch| ch as u32)
        .unwrap_or(u32::MAX)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Find(
    str_obj: *mut c_void,
    substr: *mut c_void,
    start: isize,
    end: isize,
    direction: c_int,
) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(str_obj) else {
            context.set_error("PyUnicode_Find received unknown string pointer");
            return -2;
        };
        let Some(needle) = context.cpython_value_from_ptr(substr) else {
            context.set_error("PyUnicode_Find received unknown substring pointer");
            return -2;
        };
        if cpython_unicode_text_from_value(&receiver).is_none()
            || cpython_unicode_text_from_value(&needle).is_none()
        {
            context.set_error("PyUnicode_Find expects str arguments");
            return -2;
        }
        let method = if direction >= 0 { "find" } else { "rfind" };
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            method,
            vec![needle, Value::Int(start as i64), Value::Int(end as i64)],
            "PyUnicode_Find",
        ) else {
            return -2;
        };
        match value_to_int(result) {
            Ok(index) => index as isize,
            Err(_) => {
                context.set_error("PyUnicode_Find expected integer return value");
                -2
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -2
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FindChar(
    str_obj: *mut c_void,
    ch: u32,
    start: isize,
    end: isize,
    direction: c_int,
) -> isize {
    let Some(ch) = char::from_u32(ch) else {
        return -1;
    };
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(str_obj) else {
            context.set_error("PyUnicode_FindChar received unknown string pointer");
            return -1;
        };
        if cpython_unicode_text_from_value(&receiver).is_none() {
            context.set_error("PyUnicode_FindChar expects str object");
            return -1;
        }
        let method = if direction >= 0 { "find" } else { "rfind" };
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            method,
            vec![
                Value::Str(ch.to_string()),
                Value::Int(start as i64),
                Value::Int(end as i64),
            ],
            "PyUnicode_FindChar",
        ) else {
            return -1;
        };
        match value_to_int(result) {
            Ok(index) => index as isize,
            Err(_) => {
                context.set_error("PyUnicode_FindChar expected integer return value");
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Count(
    str_obj: *mut c_void,
    substr: *mut c_void,
    start: isize,
    end: isize,
) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(str_obj) else {
            context.set_error("PyUnicode_Count received unknown string pointer");
            return -1;
        };
        let Some(needle) = context.cpython_value_from_ptr(substr) else {
            context.set_error("PyUnicode_Count received unknown substring pointer");
            return -1;
        };
        if cpython_unicode_text_from_value(&receiver).is_none()
            || cpython_unicode_text_from_value(&needle).is_none()
        {
            context.set_error("PyUnicode_Count expects str arguments");
            return -1;
        }
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            "count",
            vec![needle, Value::Int(start as i64), Value::Int(end as i64)],
            "PyUnicode_Count",
        ) else {
            return -1;
        };
        match value_to_int(result) {
            Ok(count) => count as isize,
            Err(_) => {
                context.set_error("PyUnicode_Count expected integer return value");
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Join(separator: *mut c_void, seq: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(separator_value) = context.cpython_value_from_ptr(separator) else {
            context.set_error("PyUnicode_Join received unknown separator pointer");
            return std::ptr::null_mut();
        };
        let Some(seq_value) = context.cpython_value_from_ptr_or_proxy(seq) else {
            context.set_error("PyUnicode_Join received unknown sequence pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&separator_value).is_none() {
            context.set_error("PyUnicode_Join expects str separator");
            return std::ptr::null_mut();
        }
        let Some(result) = cpython_call_method_for_capi(
            context,
            separator_value,
            "join",
            vec![seq_value],
            "PyUnicode_Join",
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Split(
    string: *mut c_void,
    sep: *mut c_void,
    maxsplit: isize,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(string) else {
            context.set_error("PyUnicode_Split received unknown string pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&receiver).is_none() {
            context.set_error("PyUnicode_Split expects str receiver");
            return std::ptr::null_mut();
        }
        let args = if sep.is_null() {
            if maxsplit < 0 {
                Vec::new()
            } else {
                vec![Value::None, Value::Int(maxsplit as i64)]
            }
        } else {
            let Some(sep_value) = context.cpython_value_from_ptr(sep) else {
                context.set_error("PyUnicode_Split received unknown separator pointer");
                return std::ptr::null_mut();
            };
            vec![sep_value, Value::Int(maxsplit as i64)]
        };
        let Some(result) =
            cpython_call_method_for_capi(context, receiver, "split", args, "PyUnicode_Split")
        else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_RSplit(
    string: *mut c_void,
    sep: *mut c_void,
    maxsplit: isize,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(string) else {
            context.set_error("PyUnicode_RSplit received unknown string pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&receiver).is_none() {
            context.set_error("PyUnicode_RSplit expects str receiver");
            return std::ptr::null_mut();
        }
        let args = if sep.is_null() {
            if maxsplit < 0 {
                Vec::new()
            } else {
                vec![Value::None, Value::Int(maxsplit as i64)]
            }
        } else {
            let Some(sep_value) = context.cpython_value_from_ptr(sep) else {
                context.set_error("PyUnicode_RSplit received unknown separator pointer");
                return std::ptr::null_mut();
            };
            vec![sep_value, Value::Int(maxsplit as i64)]
        };
        let Some(result) =
            cpython_call_method_for_capi(context, receiver, "rsplit", args, "PyUnicode_RSplit")
        else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Splitlines(string: *mut c_void, keepends: c_int) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(string) else {
            context.set_error("PyUnicode_Splitlines received unknown string pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&receiver).is_none() {
            context.set_error("PyUnicode_Splitlines expects str receiver");
            return std::ptr::null_mut();
        }
        let args = if keepends == 0 {
            Vec::new()
        } else {
            vec![Value::Bool(true)]
        };
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            "splitlines",
            args,
            "PyUnicode_Splitlines",
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Partition(string: *mut c_void, sep: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(string) else {
            context.set_error("PyUnicode_Partition received unknown string pointer");
            return std::ptr::null_mut();
        };
        let Some(sep_value) = context.cpython_value_from_ptr(sep) else {
            context.set_error("PyUnicode_Partition received unknown separator pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&receiver).is_none()
            || cpython_unicode_text_from_value(&sep_value).is_none()
        {
            context.set_error("PyUnicode_Partition expects str arguments");
            return std::ptr::null_mut();
        }
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            "partition",
            vec![sep_value],
            "PyUnicode_Partition",
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_RPartition(
    string: *mut c_void,
    sep: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(string) else {
            context.set_error("PyUnicode_RPartition received unknown string pointer");
            return std::ptr::null_mut();
        };
        let Some(sep_value) = context.cpython_value_from_ptr(sep) else {
            context.set_error("PyUnicode_RPartition received unknown separator pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&receiver).is_none()
            || cpython_unicode_text_from_value(&sep_value).is_none()
        {
            context.set_error("PyUnicode_RPartition expects str arguments");
            return std::ptr::null_mut();
        }
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            "rpartition",
            vec![sep_value],
            "PyUnicode_RPartition",
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_IsIdentifier(object: *mut c_void) -> c_int {
    with_active_cpython_context_mut(|context| {
        let Some(receiver) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyUnicode_IsIdentifier received unknown string pointer");
            return -1;
        };
        if cpython_unicode_text_from_value(&receiver).is_none() {
            context.set_error("PyUnicode_IsIdentifier expects str object");
            return -1;
        }
        let Some(result) = cpython_call_method_for_capi(
            context,
            receiver,
            "isidentifier",
            Vec::new(),
            "PyUnicode_IsIdentifier",
        ) else {
            return -1;
        };
        match result {
            Value::Bool(value) => i32::from(value),
            other => {
                let got = if context.vm.is_null() {
                    "object".to_string()
                } else {
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    unsafe { (&mut *context.vm).value_type_name_for_error(&other) }
                };
                context.set_error(format!(
                    "PyUnicode_IsIdentifier expected bool return value, got {got}"
                ));
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_GetSize(unicode: *mut c_void) -> isize {
    let _ = unicode;
    cpython_set_typed_error(
        unsafe { PyExc_RuntimeError },
        "PyUnicode_GetSize has been removed.",
    );
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_InternInPlace(unicode: *mut *mut c_void) {
    if unicode.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            unsafe { PyErr_BadInternalCall() };
        }
        return;
    }
    // SAFETY: caller provides writable pointer slot.
    let object = unsafe { *unicode };
    if object.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            unsafe { PyErr_BadInternalCall() };
        }
        return;
    }
    let _ = with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(object) else {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            return;
        };
        if cpython_unicode_text_from_value(&value).is_none()
            && unsafe { PyErr_Occurred() }.is_null()
        {
            unsafe { PyErr_BadInternalCall() };
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_InternImmortal(unicode: *mut *mut c_void) {
    unsafe { PyUnicode_InternInPlace(unicode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Append(left: *mut *mut c_void, right: *mut c_void) {
    if left.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            unsafe { PyErr_BadInternalCall() };
        }
        return;
    }
    // SAFETY: caller provides writable pointer slot.
    let left_ptr = unsafe { *left };
    let mut should_clear_left = false;
    let mut replacement: Option<*mut c_void> = None;
    let status = with_active_cpython_context_mut(|context| {
        if left_ptr.is_null() || right.is_null() {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            should_clear_left = true;
            return;
        }
        let Some(left_value) = context.cpython_value_from_ptr(left_ptr) else {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            should_clear_left = true;
            return;
        };
        let Some(right_value) = context.cpython_value_from_ptr(right) else {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            should_clear_left = true;
            return;
        };
        let Some(left_text) = cpython_unicode_text_from_value(&left_value) else {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            should_clear_left = true;
            return;
        };
        let Some(right_text) = cpython_unicode_text_from_value(&right_value) else {
            if unsafe { PyErr_Occurred() }.is_null() {
                unsafe { PyErr_BadInternalCall() };
            }
            should_clear_left = true;
            return;
        };
        if left_text.is_empty() {
            unsafe { Py_IncRef(right) };
            replacement = Some(right);
            return;
        }
        if right_text.is_empty() {
            return;
        }
        let combined = format!("{left_text}{right_text}");
        replacement = Some(context.alloc_cpython_ptr_for_value(Value::Str(combined)));
    });
    if status.is_err() {
        if unsafe { PyErr_Occurred() }.is_null() {
            cpython_set_error("PyUnicode_Append failed due to missing active C-API context");
        }
        should_clear_left = true;
    }
    if should_clear_left {
        // SAFETY: left points to writable slot; Py_CLEAR semantics.
        unsafe {
            if !(*left).is_null() {
                Py_DecRef(*left);
            }
            *left = std::ptr::null_mut();
        }
        return;
    }
    if let Some(new_left) = replacement {
        // SAFETY: left points to writable slot.
        unsafe {
            Py_DecRef(*left);
            *left = new_left;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AppendAndDel(left: *mut *mut c_void, right: *mut c_void) {
    unsafe { PyUnicode_Append(left, right) };
    unsafe { Py_XDecRef(right) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_RichCompare(
    left: *mut c_void,
    right: *mut c_void,
    op: c_int,
) -> *mut c_void {
    const PY_LT: c_int = 0;
    const PY_LE: c_int = 1;
    const PY_EQ: c_int = 2;
    const PY_NE: c_int = 3;
    const PY_GT: c_int = 4;
    const PY_GE: c_int = 5;

    with_active_cpython_context_mut(|context| {
        let Some(left_value) = context.cpython_value_from_ptr(left) else {
            context.set_error("PyUnicode_RichCompare received unknown left pointer");
            return std::ptr::null_mut();
        };
        let Some(right_value) = context.cpython_value_from_ptr(right) else {
            context.set_error("PyUnicode_RichCompare received unknown right pointer");
            return std::ptr::null_mut();
        };
        let left_text = cpython_unicode_text_from_value(&left_value);
        let right_text = cpython_unicode_text_from_value(&right_value);
        if left_text.is_none() || right_text.is_none() {
            let not_impl = std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast::<c_void>();
            unsafe { Py_IncRef(not_impl) };
            return not_impl;
        }
        let left_text = left_text.unwrap_or_default();
        let right_text = right_text.unwrap_or_default();
        let result = if left == right {
            match op {
                PY_EQ | PY_LE | PY_GE => Some(true),
                PY_NE | PY_LT | PY_GT => Some(false),
                _ => None,
            }
        } else {
            match op {
                PY_EQ => Some(left_text == right_text),
                PY_NE => Some(left_text != right_text),
                PY_LT => Some(left_text < right_text),
                PY_LE => Some(left_text <= right_text),
                PY_GT => Some(left_text > right_text),
                PY_GE => Some(left_text >= right_text),
                _ => None,
            }
        };
        let Some(result) = result else {
            unsafe { PyErr_BadArgument() };
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(Value::Bool(result))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_WriteChar(
    unicode: *mut c_void,
    index: isize,
    character: c_uint,
) -> c_int {
    if index < 0 {
        cpython_set_typed_error(unsafe { PyExc_IndexError }, "string index out of range");
        return -1;
    }
    let Some(ch) = char::from_u32(character) else {
        cpython_set_typed_error(unsafe { PyExc_ValueError }, "character out of range");
        return -1;
    };
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(unicode) else {
            context.set_error("PyUnicode_WriteChar received unknown object pointer");
            return -1;
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyUnicode_WriteChar received unknown object handle");
            return -1;
        };
        let Value::Str(text) = &slot.value else {
            context.set_error("PyUnicode_WriteChar expected str object");
            return -1;
        };
        let mut chars: Vec<char> = text.chars().collect();
        let idx = index as usize;
        if idx >= chars.len() {
            cpython_set_typed_error(unsafe { PyExc_IndexError }, "string index out of range");
            return -1;
        }
        chars[idx] = ch;
        slot.value = Value::Str(chars.into_iter().collect());
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_CopyCharacters(
    to: *mut c_void,
    to_start: isize,
    from: *mut c_void,
    from_start: isize,
    how_many: isize,
) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(to_handle) = context.cpython_handle_from_ptr(to) else {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        };
        let to_text = {
            let Some(to_slot) = context.objects.get(&to_handle) else {
                unsafe { PyErr_BadInternalCall() };
                return -1;
            };
            let Value::Str(to_text) = &to_slot.value else {
                unsafe { PyErr_BadInternalCall() };
                return -1;
            };
            to_text.clone()
        };
        let Some(from_value) = context.cpython_value_from_ptr_or_proxy(from) else {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        };
        let Value::Str(from_text) = from_value else {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        };

        let from_chars: Vec<char> = from_text.chars().collect();
        let mut to_chars: Vec<char> = to_text.chars().collect();

        if from_start < 0 || (from_start as usize) > from_chars.len() {
            cpython_set_typed_error(unsafe { PyExc_IndexError }, "string index out of range");
            return -1;
        }
        if to_start < 0 || (to_start as usize) > to_chars.len() {
            cpython_set_typed_error(unsafe { PyExc_IndexError }, "string index out of range");
            return -1;
        }
        if how_many < 0 {
            cpython_set_typed_error(unsafe { PyExc_SystemError }, "how_many cannot be negative");
            return -1;
        }

        let from_index = from_start as usize;
        let to_index = to_start as usize;
        let requested = how_many as usize;
        let available = from_chars.len().saturating_sub(from_index);
        let copy_len = available.min(requested);
        if to_index + copy_len > to_chars.len() {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                format!(
                    "Cannot write {} characters at {} in a string of {} characters",
                    copy_len,
                    to_start,
                    to_chars.len()
                ),
            );
            return -1;
        }
        if copy_len == 0 {
            return 0;
        }

        for offset in 0..copy_len {
            to_chars[to_index + offset] = from_chars[from_index + offset];
        }

        if let Some(slot) = context.objects.get_mut(&to_handle) {
            slot.value = Value::Str(to_chars.into_iter().collect());
            context.sync_cpython_storage_from_value(to_handle);
            copy_len as isize
        } else {
            unsafe { PyErr_BadInternalCall() };
            -1
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

fn cpython_unicode_decode_common(
    bytes_ptr: *const c_char,
    size: isize,
    encoding: *const c_char,
    errors: *const c_char,
    default_encoding: &str,
    api_name: &str,
) -> *mut c_void {
    if size < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            format!("{api_name} received negative size"),
        );
        return std::ptr::null_mut();
    }
    if bytes_ptr.is_null() && size != 0 {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let encoding_name = match cpython_codec_name_or_default(encoding, default_encoding, api_name) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let errors_name = match cpython_codec_error_name_optional(errors, api_name) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error(format!("{api_name} missing VM context"));
            return std::ptr::null_mut();
        }
        let raw = if size == 0 {
            Vec::new()
        } else {
            // SAFETY: caller guarantees readable `size` bytes from `bytes_ptr`.
            unsafe { std::slice::from_raw_parts(bytes_ptr.cast::<u8>(), size as usize).to_vec() }
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let source = vm.heap.alloc_bytes(raw);
        let Some(decoded) = cpython_unicode_decode_with_codec_in_context(
            context,
            source,
            encoding_name,
            errors_name,
            api_name,
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(decoded)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_unicode_decode_with_encoding_name(
    bytes_ptr: *const c_char,
    size: isize,
    encoding_name: String,
    errors: *const c_char,
    api_name: &str,
) -> *mut c_void {
    if size < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            format!("{api_name} received negative size"),
        );
        return std::ptr::null_mut();
    }
    if bytes_ptr.is_null() && size != 0 {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let errors_name = match cpython_codec_error_name_optional(errors, api_name) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error(format!("{api_name} missing VM context"));
            return std::ptr::null_mut();
        }
        let raw = if size == 0 {
            Vec::new()
        } else {
            // SAFETY: caller guarantees readable `size` bytes from `bytes_ptr`.
            unsafe { std::slice::from_raw_parts(bytes_ptr.cast::<u8>(), size as usize).to_vec() }
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let source = vm.heap.alloc_bytes(raw);
        let Some(decoded) = cpython_unicode_decode_with_codec_in_context(
            context,
            source,
            encoding_name,
            errors_name,
            api_name,
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(decoded)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_unicode_encode_with_encoding_name(
    unicode: *mut c_void,
    encoding_name: String,
    errors: *const c_char,
    api_name: &str,
) -> *mut c_void {
    if unicode.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let errors_name = match cpython_codec_error_name_optional(errors, api_name) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let Some(unicode_value) = context.cpython_value_from_ptr_or_proxy(unicode) else {
            context.set_error(format!("{api_name} received unknown unicode pointer"));
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&unicode_value).is_none() {
            context.set_error(format!("{api_name} expected str object"));
            return std::ptr::null_mut();
        }
        let Some(encoded) = cpython_unicode_encode_with_codec_in_context(
            context,
            unicode_value,
            encoding_name,
            errors_name,
            api_name,
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(encoded)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_codepage_encoding_name(code_page: c_int) -> String {
    if code_page <= 0 || code_page == 65001 {
        "utf-8".to_string()
    } else {
        format!("cp{code_page}")
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Decode(
    bytes_ptr: *const c_char,
    size: isize,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        encoding,
        errors,
        "utf-8",
        "PyUnicode_Decode",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeASCII(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"ascii".as_ptr(),
        errors,
        "ascii",
        "PyUnicode_DecodeASCII",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeLatin1(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"latin-1".as_ptr(),
        errors,
        "latin-1",
        "PyUnicode_DecodeLatin1",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF8(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-8".as_ptr(),
        errors,
        "utf-8",
        "PyUnicode_DecodeUTF8",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF8Stateful(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    consumed: *mut isize,
) -> *mut c_void {
    let result = cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-8".as_ptr(),
        errors,
        "utf-8",
        "PyUnicode_DecodeUTF8Stateful",
    );
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable pointer for consumed output.
        unsafe { *consumed = size };
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF7(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    let result = cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-7".as_ptr(),
        errors,
        "utf-7",
        "PyUnicode_DecodeUTF7",
    );
    if !result.is_null() {
        return result;
    }
    if !unsafe { PyErr_Occurred() }.is_null() {
        unsafe { PyErr_Clear() };
    }
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-8".as_ptr(),
        errors,
        "utf-8",
        "PyUnicode_DecodeUTF7",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF7Stateful(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    consumed: *mut isize,
) -> *mut c_void {
    let result = unsafe { PyUnicode_DecodeUTF7(bytes_ptr, size, errors) };
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable consumed output pointer.
        unsafe {
            *consumed = size;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeMBCS(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_with_encoding_name(
        bytes_ptr,
        size,
        cpython_codepage_encoding_name(0),
        errors,
        "PyUnicode_DecodeMBCS",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeMBCSStateful(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    consumed: *mut isize,
) -> *mut c_void {
    let result = unsafe { PyUnicode_DecodeMBCS(bytes_ptr, size, errors) };
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable consumed output pointer.
        unsafe {
            *consumed = size;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeCodePageStateful(
    code_page: c_int,
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    consumed: *mut isize,
) -> *mut c_void {
    let result = cpython_unicode_decode_with_encoding_name(
        bytes_ptr,
        size,
        cpython_codepage_encoding_name(code_page),
        errors,
        "PyUnicode_DecodeCodePageStateful",
    );
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable consumed output pointer.
        unsafe {
            *consumed = size;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeCharmap(
    bytes_ptr: *const c_char,
    size: isize,
    mapping: *mut c_void,
    errors: *const c_char,
) -> *mut c_void {
    if size < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyUnicode_DecodeCharmap received negative size",
        );
        return std::ptr::null_mut();
    }
    if bytes_ptr.is_null() && size != 0 {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if mapping.is_null() {
        let raw = if size == 0 {
            Vec::new()
        } else {
            // SAFETY: caller guarantees readable `size` bytes from `bytes_ptr`.
            unsafe { std::slice::from_raw_parts(bytes_ptr.cast::<u8>(), size as usize).to_vec() }
        };
        let decoded: String = raw.into_iter().map(char::from).collect();
        return cpython_new_ptr_for_value(Value::Str(decoded));
    }
    let errors_name = match cpython_codec_error_name_optional(errors, "PyUnicode_DecodeCharmap") {
        Ok(name) => name.unwrap_or_else(|| "strict".to_string()),
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let mapping_value = if mapping.is_null() {
            Value::None
        } else if let Some(value) = context.cpython_value_from_ptr_or_proxy(mapping) {
            value
        } else {
            context.set_error("PyUnicode_DecodeCharmap received unknown mapping object");
            return std::ptr::null_mut();
        };
        let codec_module = match cpython_codec_module_in_context(context) {
            Ok(module) => module,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let charmap_decode = {
            let Object::Module(module_data) = &*codec_module.kind() else {
                context.set_error("codecs module is invalid");
                return std::ptr::null_mut();
            };
            match module_data.globals.get("charmap_decode").cloned() {
                Some(value) => value,
                None => {
                    context.set_error("codecs.charmap_decode unavailable");
                    return std::ptr::null_mut();
                }
            }
        };
        if context.vm.is_null() {
            context.set_error("PyUnicode_DecodeCharmap missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let raw = if size == 0 {
            Vec::new()
        } else {
            // SAFETY: caller guarantees readable `size` bytes from `bytes_ptr`.
            unsafe { std::slice::from_raw_parts(bytes_ptr.cast::<u8>(), size as usize).to_vec() }
        };
        let bytes_value = vm.heap.alloc_bytes(raw);
        let decoded_tuple = match cpython_call_internal_in_context(
            context,
            charmap_decode,
            vec![bytes_value, Value::Str(errors_name), mapping_value],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let Value::Tuple(parts) = decoded_tuple else {
            context.set_error("codecs.charmap_decode() returned non-tuple");
            return std::ptr::null_mut();
        };
        let Object::Tuple(items) = &*parts.kind() else {
            context.set_error("codecs.charmap_decode() returned invalid tuple");
            return std::ptr::null_mut();
        };
        let Some(decoded) = items.first().cloned() else {
            context.set_error("codecs.charmap_decode() returned empty tuple");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&decoded).is_none() {
            context.set_error("codecs.charmap_decode() did not return unicode text");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(decoded)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_BuildEncodingMap(string: *mut c_void) -> *mut c_void {
    if string.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let unicode_value = match cpython_value_from_ptr(string) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let Some(text) = cpython_unicode_text_from_value(&unicode_value) else {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "PyUnicode_BuildEncodingMap expected str object",
        );
        return std::ptr::null_mut();
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyUnicode_BuildEncodingMap missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let dict = vm.heap.alloc_dict(Vec::new());
        let Value::Dict(dict_obj) = &dict else {
            context.set_error("PyUnicode_BuildEncodingMap internal dict allocation failed");
            return std::ptr::null_mut();
        };
        for (index, ch) in text.chars().enumerate() {
            let key = Value::Int(ch as i64);
            let value = Value::Int(index as i64);
            if let Err(err) = dict_set_value_checked(dict_obj, key, value) {
                context.set_error(err.message);
                return std::ptr::null_mut();
            }
        }
        context.alloc_cpython_ptr_for_value(dict)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeRawUnicodeEscape(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"raw_unicode_escape".as_ptr(),
        errors,
        "raw_unicode_escape",
        "PyUnicode_DecodeRawUnicodeEscape",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUnicodeEscape(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"unicode_escape".as_ptr(),
        errors,
        "unicode_escape",
        "PyUnicode_DecodeUnicodeEscape",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF16(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    byteorder: *mut c_int,
) -> *mut c_void {
    let result = cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-16".as_ptr(),
        errors,
        "utf-16",
        "PyUnicode_DecodeUTF16",
    );
    if !result.is_null() && !byteorder.is_null() {
        // SAFETY: caller provided writable byteorder output pointer.
        unsafe {
            *byteorder = 0;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF16Stateful(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    byteorder: *mut c_int,
    consumed: *mut isize,
) -> *mut c_void {
    let result = unsafe { PyUnicode_DecodeUTF16(bytes_ptr, size, errors, byteorder) };
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable consumed output pointer.
        unsafe {
            *consumed = size;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF32(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    byteorder: *mut c_int,
) -> *mut c_void {
    let result = cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-32".as_ptr(),
        errors,
        "utf-32",
        "PyUnicode_DecodeUTF32",
    );
    if !result.is_null() && !byteorder.is_null() {
        // SAFETY: caller provided writable byteorder output pointer.
        unsafe {
            *byteorder = 0;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF32Stateful(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
    byteorder: *mut c_int,
    consumed: *mut isize,
) -> *mut c_void {
    let result = unsafe { PyUnicode_DecodeUTF32(bytes_ptr, size, errors, byteorder) };
    if !result.is_null() && !consumed.is_null() {
        // SAFETY: caller provided writable consumed output pointer.
        unsafe {
            *consumed = size;
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeFSDefaultAndSize(
    bytes_ptr: *const c_char,
    size: isize,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-8".as_ptr(),
        std::ptr::null(),
        "utf-8",
        "PyUnicode_DecodeFSDefaultAndSize",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeFSDefault(bytes_ptr: *const c_char) -> *mut c_void {
    if bytes_ptr.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: bytes_ptr is expected to be NUL-terminated.
    let size = unsafe { CStr::from_ptr(bytes_ptr).to_bytes().len() as isize };
    unsafe { PyUnicode_DecodeFSDefaultAndSize(bytes_ptr, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeLocaleAndSize(
    bytes_ptr: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_decode_common(
        bytes_ptr,
        size,
        c"utf-8".as_ptr(),
        errors,
        "utf-8",
        "PyUnicode_DecodeLocaleAndSize",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeLocale(
    bytes_ptr: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    if bytes_ptr.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: bytes_ptr is expected to be NUL-terminated.
    let size = unsafe { CStr::from_ptr(bytes_ptr).to_bytes().len() as isize };
    unsafe { PyUnicode_DecodeLocaleAndSize(bytes_ptr, size, errors) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EncodeFSDefault(unicode: *mut c_void) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedString(unicode, c"utf-8".as_ptr(), std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EncodeLocale(
    unicode: *mut c_void,
    errors: *const c_char,
) -> *mut c_void {
    unsafe { PyUnicode_AsEncodedString(unicode, c"utf-8".as_ptr(), errors) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EncodeCodePage(
    code_page: c_int,
    unicode: *mut c_void,
    errors: *const c_char,
) -> *mut c_void {
    cpython_unicode_encode_with_encoding_name(
        unicode,
        cpython_codepage_encoding_name(code_page),
        errors,
        "PyUnicode_EncodeCodePage",
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsDecodedObject(
    unicode: *mut c_void,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    if unicode.is_null() {
        unsafe { PyErr_BadArgument() };
        return std::ptr::null_mut();
    }
    let encoding_name =
        match cpython_codec_name_or_default(encoding, "utf-8", "PyUnicode_AsDecodedObject") {
            Ok(name) => name,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
    let errors_name = match cpython_codec_error_name_optional(errors, "PyUnicode_AsDecodedObject") {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            context.set_error("PyUnicode_AsDecodedObject received unknown object pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&value).is_none() {
            unsafe { PyErr_BadArgument() };
            return std::ptr::null_mut();
        }
        if let Some(text) = cpython_unicode_text_from_value(&value)
            && cpython_codec_is_rot13(&encoding_name)
        {
            return context.alloc_cpython_ptr_for_value(Value::Str(cpython_rot13_text(&text)));
        }
        let mut args = vec![value, Value::Str(encoding_name)];
        if let Some(errors_name) = errors_name {
            args.push(Value::Str(errors_name));
        }
        match cpython_call_internal_in_context(
            context,
            Value::Builtin(BuiltinFunction::CodecsDecode),
            args,
            HashMap::new(),
        ) {
            Ok(decoded) => context.alloc_cpython_ptr_for_value(decoded),
            Err(err) => {
                context.set_error(err);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsDecodedUnicode(
    unicode: *mut c_void,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    let result = unsafe { PyUnicode_AsDecodedObject(unicode, encoding, errors) };
    if result.is_null() {
        return std::ptr::null_mut();
    }
    match cpython_value_from_ptr(result) {
        Ok(value) if cpython_unicode_text_from_value(&value).is_some() => result,
        Ok(value) => {
            let got = with_active_cpython_context_mut(|context| {
                if context.vm.is_null() {
                    "object".to_string()
                } else {
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    unsafe { (&mut *context.vm).value_type_name_for_error(&value) }
                }
            })
            .unwrap_or_else(|_| "object".to_string());
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                format!("decoder returned '{got}' instead of 'str'"),
            );
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsEncodedObject(
    unicode: *mut c_void,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    if unicode.is_null() {
        unsafe { PyErr_BadArgument() };
        return std::ptr::null_mut();
    }
    let encoding_name =
        match cpython_codec_name_or_default(encoding, "utf-8", "PyUnicode_AsEncodedObject") {
            Ok(name) => name,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
    let errors_name = match cpython_codec_error_name_optional(errors, "PyUnicode_AsEncodedObject") {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(unicode) else {
            context.set_error("PyUnicode_AsEncodedObject received unknown object pointer");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&value).is_none() {
            unsafe { PyErr_BadArgument() };
            return std::ptr::null_mut();
        }
        let Some(encoded) = cpython_unicode_encode_with_codec_in_context(
            context,
            value,
            encoding_name,
            errors_name,
            "PyUnicode_AsEncodedObject",
        ) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(encoded)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsEncodedUnicode(
    unicode: *mut c_void,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut c_void {
    let result = unsafe { PyUnicode_AsEncodedObject(unicode, encoding, errors) };
    if result.is_null() {
        return std::ptr::null_mut();
    }
    match cpython_value_from_ptr(result) {
        Ok(value) if cpython_unicode_text_from_value(&value).is_some() => result,
        Ok(value) => {
            let got = with_active_cpython_context_mut(|context| {
                if context.vm.is_null() {
                    "object".to_string()
                } else {
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    unsafe { (&mut *context.vm).value_type_name_for_error(&value) }
                }
            })
            .unwrap_or_else(|_| "object".to_string());
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                format!("encoder returned '{got}' instead of 'str'"),
            );
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FSConverter(arg: *mut c_void, addr: *mut c_void) -> c_int {
    if addr.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return 0;
    }
    if arg.is_null() {
        // SAFETY: caller passes output slot pointer when requesting cleanup.
        unsafe {
            let slot = addr.cast::<*mut c_void>();
            if !(*slot).is_null() {
                Py_DecRef(*slot);
            }
            *slot = std::ptr::null_mut();
        }
        return 1;
    }
    let path = unsafe { PyOS_FSPath(arg) };
    if path.is_null() {
        return 0;
    }
    let output = match cpython_value_from_ptr(path) {
        Ok(Value::Bytes(_)) => path,
        Ok(value) if cpython_unicode_text_from_value(&value).is_some() => {
            let encoded = unsafe { PyUnicode_EncodeFSDefault(path) };
            unsafe { Py_DecRef(path) };
            if encoded.is_null() {
                return 0;
            }
            encoded
        }
        _ => {
            unsafe { Py_DecRef(path) };
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "path should be string, bytes, or os.PathLike",
            );
            return 0;
        }
    };
    // SAFETY: caller provides writable PyObject** slot in addr.
    unsafe {
        let slot = addr.cast::<*mut c_void>();
        *slot = output;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FSDecoder(arg: *mut c_void, addr: *mut c_void) -> c_int {
    if addr.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return 0;
    }
    if arg.is_null() {
        // SAFETY: caller passes output slot pointer when requesting cleanup.
        unsafe {
            let slot = addr.cast::<*mut c_void>();
            if !(*slot).is_null() {
                Py_DecRef(*slot);
            }
            *slot = std::ptr::null_mut();
        }
        return 1;
    }
    let path = unsafe { PyOS_FSPath(arg) };
    if path.is_null() {
        return 0;
    }
    let output = match cpython_value_from_ptr(path) {
        Ok(value) if cpython_unicode_text_from_value(&value).is_some() => path,
        Ok(Value::Bytes(bytes_obj)) => {
            let size = match &*bytes_obj.kind() {
                Object::Bytes(values) => values.len() as isize,
                _ => {
                    unsafe { Py_DecRef(path) };
                    cpython_set_error("PyUnicode_FSDecoder encountered invalid bytes storage");
                    return 0;
                }
            };
            let data = unsafe { PyBytes_AsString(path) };
            let decoded = unsafe { PyUnicode_DecodeFSDefaultAndSize(data, size) };
            unsafe { Py_DecRef(path) };
            if decoded.is_null() {
                return 0;
            }
            decoded
        }
        _ => {
            unsafe { Py_DecRef(path) };
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "path should be string, bytes, or os.PathLike",
            );
            return 0;
        }
    };
    // SAFETY: caller provides writable PyObject** slot in addr.
    unsafe {
        let slot = addr.cast::<*mut c_void>();
        *slot = output;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Translate(
    object: *mut c_void,
    table: *mut c_void,
    errors: *const c_char,
) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if let Err(err) = cpython_codec_error_name_optional(errors, "PyUnicode_Translate") {
        cpython_set_error(err);
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyUnicode_Translate missing VM context");
            return std::ptr::null_mut();
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyUnicode_Translate received unknown object");
            return std::ptr::null_mut();
        };
        if cpython_unicode_text_from_value(&object_value).is_none() {
            context.set_error("PyUnicode_Translate expected str receiver");
            return std::ptr::null_mut();
        }
        let table_value = if table.is_null() {
            Value::None
        } else if let Some(value) = context.cpython_value_from_ptr_or_proxy(table) {
            value
        } else {
            context.set_error("PyUnicode_Translate received unknown mapping table");
            return std::ptr::null_mut();
        };
        let translate = match cpython_getattr_in_context(context, object_value, "translate") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let translated = match cpython_call_internal_in_context(
            context,
            translate,
            vec![table_value],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        if cpython_unicode_text_from_value(&translated).is_none() {
            context.set_error("str.translate() returned non-str result");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(translated)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Resize(unicode: *mut *mut c_void, length: isize) -> c_int {
    if unicode.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if length < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyUnicode_Resize received negative size",
        );
        return -1;
    }
    // SAFETY: caller provides writable unicode pointer.
    let current = unsafe { *unicode };
    if current.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let value = match cpython_value_from_ptr(current) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let Some(text) = cpython_unicode_text_from_value(&value) else {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "PyUnicode_Resize expected str object",
        );
        return -1;
    };
    let mut chars: Vec<char> = text.chars().collect();
    if (length as usize) < chars.len() {
        chars.truncate(length as usize);
    } else if (length as usize) > chars.len() {
        chars.resize(length as usize, '\0');
    }
    let resized = chars.into_iter().collect::<String>();
    let resized_ptr = cpython_new_ptr_for_value(Value::Str(resized));
    if resized_ptr.is_null() {
        return -1;
    }
    // SAFETY: output slot and old object pointer are valid for update/decref.
    unsafe {
        *unicode = resized_ptr;
        Py_DecRef(current);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Replace(
    object: *mut c_void,
    substr: *mut c_void,
    repl: *mut c_void,
    count: isize,
) -> *mut c_void {
    let object = match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Replace expected str receiver");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let substr = match cpython_value_from_ptr(substr) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Replace expected str search value");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let repl = match cpython_value_from_ptr(repl) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Replace expected str replacement");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let replaced = if count < 0 {
        object.replace(&substr, &repl)
    } else {
        object.replacen(&substr, &repl, count as usize)
    };
    cpython_new_ptr_for_value(Value::Str(replaced))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Substring(
    object: *mut c_void,
    start: isize,
    end: isize,
) -> *mut c_void {
    let text = match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Substring expected str receiver");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as isize;
    let lo = start.clamp(0, len) as usize;
    let hi = end.clamp(0, len) as usize;
    let slice = if hi >= lo {
        chars[lo..hi].iter().collect::<String>()
    } else {
        String::new()
    };
    cpython_new_ptr_for_value(Value::Str(slice))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Tailmatch(
    object: *mut c_void,
    substr: *mut c_void,
    start: isize,
    end: isize,
    direction: i32,
) -> isize {
    let text = match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Tailmatch expected str receiver");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let suffix = match cpython_value_from_ptr(substr) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_Tailmatch expected str suffix");
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as isize;
    let lo = start.clamp(0, len) as usize;
    let hi = end.clamp(0, len) as usize;
    let section = if hi >= lo {
        chars[lo..hi].iter().collect::<String>()
    } else {
        String::new()
    };
    let matched = if direction >= 0 {
        section.ends_with(&suffix)
    } else {
        section.starts_with(&suffix)
    };
    if matched { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUCS4(
    object: *mut c_void,
    buffer: *mut u32,
    buflen: isize,
    copy_null: i32,
) -> *mut u32 {
    if buffer.is_null() || buflen < 0 {
        cpython_set_error("PyUnicode_AsUCS4 received invalid output buffer");
        return std::ptr::null_mut();
    }
    let text = match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_AsUCS4 expected str object");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let mut units: Vec<u32> = text.chars().map(|ch| ch as u32).collect();
    if copy_null != 0 {
        units.push(0);
    }
    if units.len() > buflen as usize {
        cpython_set_error("PyUnicode_AsUCS4 output buffer too small");
        return std::ptr::null_mut();
    }
    // SAFETY: caller provided writable buffer with buflen entries.
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), buffer, units.len());
    }
    buffer
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUCS4Copy(object: *mut c_void) -> *mut u32 {
    let text = match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_error("PyUnicode_AsUCS4Copy expected str object");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let mut units: Vec<u32> = text.chars().map(|ch| ch as u32).collect();
    units.push(0);
    let bytes = units
        .len()
        .checked_mul(std::mem::size_of::<u32>())
        .unwrap_or(0);
    if bytes == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: allocate and copy raw u32 buffer for caller-owned lifetime.
    let raw = unsafe { PyMem_Malloc(bytes) }.cast::<u32>();
    if raw.is_null() {
        cpython_set_error("PyUnicode_AsUCS4Copy allocation failed");
        return std::ptr::null_mut();
    }
    // SAFETY: raw buffer has at least `units.len()` u32 slots.
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), raw, units.len());
    }
    raw
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_Release(view: *mut c_void) {
    if view.is_null() {
        return;
    }
    // SAFETY: caller provided a valid Py_buffer-compatible pointer.
    let view_ref = unsafe { &mut *view.cast::<CpythonBuffer>() };
    let object = view_ref.obj;
    let internal = view_ref.internal;
    if !internal.is_null() {
        // SAFETY: `internal` is allocated in `PyObject_GetBuffer` as `CpythonBufferInternal`.
        let internal = unsafe { Box::from_raw(internal.cast::<CpythonBufferInternal>()) };
        let _ = with_active_cpython_context_mut(|context| {
            let _ = context.object_release_buffer(internal.handle);
        });
    }
    if !object.is_null() {
        unsafe { Py_XDecRef(object) };
    }
    view_ref.buf = std::ptr::null_mut();
    view_ref.obj = std::ptr::null_mut();
    view_ref.len = 0;
    view_ref.itemsize = 0;
    view_ref.readonly = 1;
    view_ref.ndim = 0;
    view_ref.format = std::ptr::null_mut();
    view_ref.shape = std::ptr::null_mut();
    view_ref.strides = std::ptr::null_mut();
    view_ref.suboffsets = std::ptr::null_mut();
    view_ref.internal = std::ptr::null_mut();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCallable_Check(object: *mut c_void) -> i32 {
    match with_active_cpython_context_mut(|context| {
        let raw_ptr_is_callable = |ptr: *mut c_void| -> bool {
            if ptr.is_null() {
                return false;
            }
            // SAFETY: pointer is inspected as a CPython object header.
            let type_ptr = unsafe {
                ptr.cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if type_ptr.is_null() {
                return false;
            }
            // SAFETY: type pointer is valid for call-slot metadata inspection.
            let has_tp_call = unsafe { !(*type_ptr).tp_call.is_null() };
            if has_tp_call {
                return true;
            }
            // SAFETY: vectorcall resolver only inspects callable layout metadata.
            unsafe { cpython_resolve_vectorcall(ptr).is_some() }
        };
        let value = context.cpython_value_from_ptr(object);
        if context.vm.is_null() {
            context.set_error("PyCallable_Check missing VM context");
            return -1;
        }
        let result = if let Some(value) = value.as_ref() {
            // SAFETY: VM pointer is valid for context lifetime.
            let vm = unsafe { &mut *context.vm };
            if vm.is_callable_value(&value) {
                1
            } else if let Some(raw_proxy) =
                ModuleCapiContext::cpython_proxy_raw_ptr_from_value(&value)
            {
                if raw_ptr_is_callable(raw_proxy) { 1 } else { 0 }
            } else if raw_ptr_is_callable(object) {
                1
            } else {
                0
            }
        } else if raw_ptr_is_callable(object) {
            1
        } else {
            0
        };
        if std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some() {
            let value_tag = value
                .as_ref()
                .map(cpython_value_debug_tag)
                .unwrap_or_else(|| "<raw-foreign>".to_string());
            eprintln!(
                "[numpy-init] PyCallable_Check object={:p} value_tag={} result={}",
                object, value_tag, result
            );
        }
        result
    }) {
        Ok(result) => result,
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIndex_Check(object: *mut c_void) -> i32 {
    match cpython_value_from_ptr(object) {
        Ok(Value::Bool(_) | Value::Int(_) | Value::BigInt(_)) => 1,
        Ok(_) => 0,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_AsDouble(object: *mut c_void) -> f64 {
    match cpython_value_from_ptr_or_proxy(object) {
        Ok(Value::Float(value)) => value,
        Ok(Value::Int(value)) => value as f64,
        Ok(Value::Bool(value)) => {
            if value {
                1.0
            } else {
                0.0
            }
        }
        Ok(Value::BigInt(value)) => value.to_f64(),
        Ok(value) => match cpython_call_builtin(BuiltinFunction::Float, vec![value]) {
            Ok(Value::Float(value)) => value,
            Ok(Value::Int(value)) => value as f64,
            Ok(Value::Bool(value)) => {
                if value {
                    1.0
                } else {
                    0.0
                }
            }
            Ok(Value::BigInt(value)) => value.to_f64(),
            Ok(_) => {
                cpython_set_error("__float__ returned non-float-compatible result");
                -1.0
            }
            Err(err) => {
                cpython_set_error(err);
                -1.0
            }
        },
        Err(err) => {
            cpython_set_error(err);
            -1.0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_GetMax() -> f64 {
    f64::MAX
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_GetMin() -> f64 {
    f64::MIN_POSITIVE
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_GetInfo() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyFloat_GetInfo missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(sys_module) = vm.modules.get("sys").cloned() else {
            context.set_error("PyFloat_GetInfo missing sys module");
            return std::ptr::null_mut();
        };
        let float_info = match &*sys_module.kind() {
            Object::Module(module_data) => module_data.globals.get("float_info").cloned(),
            _ => None,
        };
        let Some(float_info) = float_info else {
            context.set_error("PyFloat_GetInfo missing sys.float_info");
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(float_info)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsLong(object: *mut c_void) -> i64 {
    match cpython_value_from_ptr(object) {
        Ok(value) => match value_to_int(value) {
            Ok(value) => {
                if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
                    eprintln!(
                        "[cpy-long] mapped value object={:p} value={}",
                        object, value
                    );
                }
                value
            }
            Err(err) => {
                if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
                    eprintln!(
                        "[cpy-long] mapped conversion failed object={:p} err={}",
                        object, err.message
                    );
                }
                cpython_set_error(err.message);
                -1
            }
        },
        Err(err) => {
            if let Some(value) = unsafe { cpython_foreign_long_to_i64(object) } {
                if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
                    eprintln!(
                        "[cpy-long] foreign fallback object={:p} value={}",
                        object, value
                    );
                }
                return value;
            }
            if std::env::var_os("PYRS_TRACE_CPY_LONG").is_some() {
                eprintln!("[cpy-long] foreign fallback failed object={:p}", object);
            }
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsLongLong(object: *mut c_void) -> i64 {
    unsafe { PyLong_AsLong(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsSsize_t(object: *mut c_void) -> isize {
    unsafe { PyLong_AsLong(object) as isize }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLong(object: *mut c_void) -> u64 {
    match cpython_value_from_ptr(object) {
        Ok(Value::Bool(value)) => {
            if value {
                1
            } else {
                0
            }
        }
        Ok(Value::Int(value)) => {
            if value < 0 {
                cpython_set_typed_error(
                    unsafe { PyExc_OverflowError },
                    "can't convert negative value to unsigned int",
                );
                return u64::MAX;
            }
            value as u64
        }
        Ok(Value::BigInt(value)) => {
            if value.is_negative() {
                cpython_set_typed_error(
                    unsafe { PyExc_OverflowError },
                    "can't convert negative value to unsigned int",
                );
                return u64::MAX;
            }
            match cpython_bigint_to_u64(&value) {
                Some(compact) => compact,
                None => {
                    cpython_set_typed_error(
                        unsafe { PyExc_OverflowError },
                        "Python int too large to convert to C unsigned long",
                    );
                    u64::MAX
                }
            }
        }
        Ok(value) => match value_to_int(value) {
            Ok(compact) => {
                if compact < 0 {
                    cpython_set_typed_error(
                        unsafe { PyExc_OverflowError },
                        "can't convert negative value to unsigned int",
                    );
                    return u64::MAX;
                }
                compact as u64
            }
            Err(err) => {
                cpython_set_error(err.message);
                u64::MAX
            }
        },
        Err(err) => {
            if let Some(compact) = unsafe { cpython_foreign_long_to_u64(object) } {
                return compact;
            }
            cpython_set_error(err);
            u64::MAX
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLongLong(object: *mut c_void) -> u64 {
    unsafe { PyLong_AsUnsignedLong(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLongMask(object: *mut c_void) -> u64 {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            if let Some(compact) = unsafe { cpython_foreign_long_to_i64(object) } {
                return compact as u64;
            }
            cpython_set_error(err);
            return u64::MAX;
        }
    };
    let normalized = match value {
        Value::Int(_) | Value::Bool(_) | Value::BigInt(_) => value,
        other => match cpython_call_builtin(BuiltinFunction::Int, vec![other]) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return u64::MAX;
            }
        },
    };
    match normalized {
        Value::Bool(flag) => {
            if flag {
                1
            } else {
                0
            }
        }
        Value::Int(compact) => compact as u64,
        Value::BigInt(bigint) => {
            let lower = cpython_bigint_low_u64(&bigint);
            if bigint.is_negative() {
                (0u64).wrapping_sub(lower)
            } else {
                lower
            }
        }
        _ => u64::MAX,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLongLongMask(object: *mut c_void) -> u64 {
    unsafe { PyLong_AsUnsignedLongMask(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsSize_t(object: *mut c_void) -> usize {
    let value = unsafe { PyLong_AsUnsignedLongLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return usize::MAX;
    }
    if value > usize::MAX as u64 {
        cpython_set_typed_error(
            unsafe { PyExc_OverflowError },
            "Python int too large to convert to C size_t",
        );
        return usize::MAX;
    }
    value as usize
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsInt(object: *mut c_void) -> i32 {
    let value = unsafe { PyLong_AsLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    if value < i32::MIN as i64 || value > i32::MAX as i64 {
        cpython_set_typed_error(
            unsafe { PyExc_OverflowError },
            "Python int too large to convert to C int",
        );
        return -1;
    }
    value as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsInt32(object: *mut c_void, out: *mut i32) -> i32 {
    if out.is_null() {
        cpython_set_error("PyLong_AsInt32 requires non-null output pointer");
        return -1;
    }
    let value = unsafe { PyLong_AsLongLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    if value < i32::MIN as i64 || value > i32::MAX as i64 {
        cpython_set_typed_error(
            unsafe { PyExc_OverflowError },
            "Python int too large to convert to C int32_t",
        );
        return -1;
    }
    // SAFETY: caller provided writable output pointer.
    unsafe { *out = value as i32 };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsInt64(object: *mut c_void, out: *mut i64) -> i32 {
    if out.is_null() {
        cpython_set_error("PyLong_AsInt64 requires non-null output pointer");
        return -1;
    }
    let value = unsafe { PyLong_AsLongLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    // SAFETY: caller provided writable output pointer.
    unsafe { *out = value };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUInt32(object: *mut c_void, out: *mut u32) -> i32 {
    if out.is_null() {
        cpython_set_error("PyLong_AsUInt32 requires non-null output pointer");
        return -1;
    }
    let value = unsafe { PyLong_AsUnsignedLongLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    if value > u32::MAX as u64 {
        cpython_set_typed_error(
            unsafe { PyExc_OverflowError },
            "Python int too large to convert to C uint32_t",
        );
        return -1;
    }
    // SAFETY: caller provided writable output pointer.
    unsafe { *out = value as u32 };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUInt64(object: *mut c_void, out: *mut u64) -> i32 {
    if out.is_null() {
        cpython_set_error("PyLong_AsUInt64 requires non-null output pointer");
        return -1;
    }
    let value = unsafe { PyLong_AsUnsignedLongLong(object) };
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    // SAFETY: caller provided writable output pointer.
    unsafe { *out = value };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsVoidPtr(object: *mut c_void) -> *mut c_void {
    unsafe { PyLong_AsUnsignedLongLong(object) as usize as *mut c_void }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsLongAndOverflow(object: *mut c_void, overflow: *mut i32) -> i64 {
    if !overflow.is_null() {
        // SAFETY: caller provided pointer is writable.
        unsafe { *overflow = 0 };
    }
    match cpython_value_from_ptr(object) {
        Ok(Value::BigInt(value)) => {
            if let Some(compact) = value.to_i64() {
                compact
            } else {
                if !overflow.is_null() {
                    // SAFETY: caller provided pointer is writable.
                    unsafe { *overflow = if value.is_negative() { -1 } else { 1 } };
                }
                -1
            }
        }
        Ok(value) => match value_to_int(value) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err.message);
                -1
            }
        },
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsLongLongAndOverflow(
    object: *mut c_void,
    overflow: *mut i32,
) -> i64 {
    unsafe { PyLong_AsLongAndOverflow(object, overflow) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsDouble(object: *mut c_void) -> f64 {
    match cpython_value_from_ptr(object) {
        Ok(Value::Bool(value)) => {
            if value {
                1.0
            } else {
                0.0
            }
        }
        Ok(Value::Int(value)) => value as f64,
        Ok(Value::BigInt(value)) => {
            let as_double = value.to_f64();
            if !as_double.is_finite() {
                cpython_set_typed_error(
                    unsafe { PyExc_OverflowError },
                    "int too large to convert to float",
                );
                return -1.0;
            }
            as_double
        }
        Ok(value) => match value_to_int(value) {
            Ok(compact) => compact as f64,
            Err(err) => {
                cpython_set_error(err.message);
                -1.0
            }
        },
        Err(err) => {
            if let Some(compact) = unsafe { cpython_foreign_long_to_i64(object) } {
                return compact as f64;
            }
            cpython_set_error(err);
            -1.0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromDouble(value: f64) -> *mut c_void {
    if !value.is_finite() {
        cpython_set_error("PyLong_FromDouble cannot convert inf/nan");
        return std::ptr::null_mut();
    }
    let truncated = value.trunc();
    if truncated < i64::MIN as f64 || truncated > i64::MAX as f64 {
        cpython_set_error("PyLong_FromDouble overflow");
        return std::ptr::null_mut();
    }
    cpython_new_ptr_for_value(Value::Int(truncated as i64))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_FromDoubles(real: f64, imag: f64) -> *mut c_void {
    cpython_new_ptr_for_value(Value::Complex { real, imag })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_FromCComplex(value: CpythonComplexValue) -> *mut c_void {
    unsafe { PyComplex_FromDoubles(value.real, value.imag) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_AsCComplex(object: *mut c_void) -> CpythonComplexValue {
    let err_value = CpythonComplexValue {
        real: -1.0,
        imag: 0.0,
    };
    if object.is_null() {
        cpython_set_error("PyComplex_AsCComplex received null object");
        return err_value;
    }
    match cpython_value_from_ptr(object) {
        Ok(Value::Complex { real, imag }) => CpythonComplexValue { real, imag },
        Ok(Value::Float(real)) => CpythonComplexValue { real, imag: 0.0 },
        Ok(Value::Int(real)) => CpythonComplexValue {
            real: real as f64,
            imag: 0.0,
        },
        Ok(Value::Bool(flag)) => CpythonComplexValue {
            real: if flag { 1.0 } else { 0.0 },
            imag: 0.0,
        },
        Ok(Value::BigInt(real)) => CpythonComplexValue {
            real: real.to_f64(),
            imag: 0.0,
        },
        Ok(_) => {
            // CPython behavior:
            // 1) If __complex__ exists, call it and require a complex result.
            // 2) Otherwise, fall back to PyFloat_AsDouble(op) + 0j.
            let method_name = b"__complex__\0";
            let method = unsafe { PyObject_GetAttrString(object, method_name.as_ptr().cast()) };
            if !method.is_null() {
                let result = unsafe { PyObject_CallObject(method, std::ptr::null_mut()) };
                unsafe { Py_DecRef(method) };
                if result.is_null() {
                    return err_value;
                }
                let complex_value = match cpython_value_from_ptr(result) {
                    Ok(Value::Complex { real, imag }) => CpythonComplexValue { real, imag },
                    Ok(_) => {
                        cpython_set_error("__complex__ returned non-complex object");
                        err_value
                    }
                    Err(err) => {
                        cpython_set_error(err);
                        err_value
                    }
                };
                unsafe { Py_DecRef(result) };
                return complex_value;
            }
            let attribute_missing = with_active_cpython_context_mut(|context| {
                context
                    .last_error
                    .as_deref()
                    .is_some_and(|message| message.contains("has no attribute"))
            })
            .unwrap_or(false);
            if attribute_missing {
                unsafe { PyErr_Clear() };
            } else if !unsafe { PyErr_Occurred() }.is_null() {
                return err_value;
            }
            let real = unsafe { PyFloat_AsDouble(object) };
            CpythonComplexValue { real, imag: 0.0 }
        }
        Err(err) => {
            cpython_set_error(err);
            err_value
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_RealAsDouble(object: *mut c_void) -> f64 {
    unsafe { PyComplex_AsCComplex(object) }.real
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_ImagAsDouble(object: *mut c_void) -> f64 {
    unsafe { PyComplex_AsCComplex(object) }.imag
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyStructSequence_NewType(desc: *mut c_void) -> *mut c_void {
    if desc.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyStructSequence_NewType expected non-null descriptor",
        );
        return std::ptr::null_mut();
    }
    // SAFETY: descriptor pointer is validated non-null.
    let desc_ref = unsafe { &*desc.cast::<CpythonStructSequenceDesc>() };
    if desc_ref.name.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyStructSequence_NewType expected descriptor name",
        );
        return std::ptr::null_mut();
    }
    let type_name = match unsafe { c_name_to_string(desc_ref.name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(format!("PyStructSequence_NewType invalid name: {err}"));
            return std::ptr::null_mut();
        }
    };
    let owned_name = match CString::new(type_name.clone()) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(format!("PyStructSequence_NewType invalid name: {err}"));
            return std::ptr::null_mut();
        }
    };
    let field_count = match cpython_structseq_count_fields(desc_ref.fields) {
        Ok(count) => count,
        Err(err) => {
            cpython_set_typed_error(unsafe { PyExc_SystemError }, err);
            return std::ptr::null_mut();
        }
    };
    let visible_count = if desc_ref.n_in_sequence < 0 {
        field_count
    } else {
        (desc_ref.n_in_sequence as usize).min(field_count)
    };

    // SAFETY: static tuple type can be copied by value to seed a heap-like type shell.
    let mut type_value = unsafe { std::ptr::read(std::ptr::addr_of!(PyTuple_Type)) };
    type_value.ob_refcnt = 1;
    type_value.ob_type = std::ptr::addr_of_mut!(PyType_Type).cast();
    type_value.ob_size = 0;
    type_value.tp_name = owned_name.as_ptr();
    type_value.tp_doc = desc_ref.doc;
    type_value.tp_base = std::ptr::addr_of_mut!(PyTuple_Type);
    type_value.tp_members = std::ptr::null_mut();
    type_value.tp_dict = std::ptr::null_mut();
    type_value.tp_flags |= PY_TPFLAGS_BASETYPE;
    type_value.tp_flags &= !PY_TPFLAGS_READY;

    let type_ptr = Box::into_raw(Box::new(type_value));
    if unsafe { PyType_Ready(type_ptr.cast()) } != 0 {
        // SAFETY: type_ptr allocated above and not published on failure path.
        unsafe {
            let _ = Box::from_raw(type_ptr);
        }
        return std::ptr::null_mut();
    }
    if let Ok(mut registry) = cpython_structseq_registry().lock() {
        registry.insert(
            type_ptr as usize,
            CpythonStructSeqTypeInfo {
                field_count,
                _visible_count: visible_count,
                _name: owned_name,
            },
        );
    }
    type_ptr.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyStructSequence_New(type_obj: *mut c_void) -> *mut c_void {
    if type_obj.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyStructSequence_New expected non-null type",
        );
        return std::ptr::null_mut();
    }
    let field_count = cpython_structseq_registry()
        .lock()
        .ok()
        .and_then(|registry| {
            registry
                .get(&(type_obj as usize))
                .map(|entry| entry.field_count)
        })
        .unwrap_or(0);
    unsafe { PyTuple_New(field_count as isize) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyStructSequence_SetItem(
    object: *mut c_void,
    index: isize,
    value: *mut c_void,
) {
    if object.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyStructSequence_SetItem expected non-null object",
        );
        return;
    }
    let _ = unsafe { PyTuple_SetItem(object, index, value) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyStructSequence_GetItem(
    object: *mut c_void,
    index: isize,
) -> *mut c_void {
    if object.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyStructSequence_GetItem expected non-null object",
        );
        return std::ptr::null_mut();
    }
    unsafe { PyTuple_GetItem(object, index) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_set_error_message(message: *const c_char) {
    if message.is_null() {
        cpython_set_error("received null error message from C shim");
        return;
    }
    // SAFETY: caller provides a valid NUL-terminated error string.
    let text = unsafe { CStr::from_ptr(message) };
    match text.to_str() {
        Ok(message) => cpython_set_error(message.to_string()),
        Err(_) => cpython_set_error("received invalid UTF-8 error message from C shim"),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_tuple_pack_from_array(
    size: isize,
    items: *const *mut c_void,
) -> *mut c_void {
    if size < 0 {
        cpython_set_error("PyTuple_Pack requires non-negative size");
        return std::ptr::null_mut();
    }
    let tuple = unsafe { PyTuple_New(size) };
    if tuple.is_null() {
        return std::ptr::null_mut();
    }
    if size == 0 {
        return tuple;
    }
    if items.is_null() {
        cpython_set_error("PyTuple_Pack received null items array");
        unsafe { Py_DecRef(tuple) };
        return std::ptr::null_mut();
    }
    for idx in 0..(size as usize) {
        // SAFETY: `items` has at least `size` entries supplied by the C shim.
        let item = unsafe { *items.add(idx) };
        if item.is_null() {
            cpython_set_error("PyTuple_Pack received null item pointer");
            unsafe { Py_DecRef(tuple) };
            return std::ptr::null_mut();
        }
        // PyTuple_Pack consumes borrowed inputs, so incref before handing off to
        // PyTuple_SetItem (which steals one reference by CPython contract).
        unsafe { Py_XIncRef(item) };
        if unsafe { PyTuple_SetItem(tuple, idx as isize, item) } != 0 {
            unsafe { Py_DecRef(tuple) };
            return std::ptr::null_mut();
        }
    }
    tuple
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Check(object: *mut c_void) -> i32 {
    if object.is_null() {
        return 0;
    }
    if let Ok(result) = with_active_cpython_context_mut(|context| {
        if let Some(value) = context.cpython_value_from_ptr_or_proxy(object) {
            if matches!(
                value,
                Value::Tuple(_)
                    | Value::List(_)
                    | Value::Str(_)
                    | Value::Bytes(_)
                    | Value::ByteArray(_)
                    | Value::MemoryView(_)
            ) {
                return 1;
            }
            if matches!(value, Value::Dict(_)) {
                return 0;
            }
        }
        const MIN_VALID_PTR: usize = 0x1_0000_0000;
        if (object as usize) < MIN_VALID_PTR
            || (object as usize) % std::mem::align_of::<CpythonObjectHead>() != 0
        {
            return 0;
        }
        // SAFETY: best-effort slot lookup for CPython-compatible object pointers.
        let type_ptr = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if type_ptr.is_null()
            || (type_ptr as usize) < MIN_VALID_PTR
            || (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0
        {
            return 0;
        }
        // SAFETY: `type_ptr` is validated non-null/aligned.
        let as_sequence = unsafe {
            (*type_ptr)
                .tp_as_sequence
                .cast::<CpythonSequenceMethods>()
                .as_ref()
        };
        if let Some(methods) = as_sequence
            && (!methods.sq_item.is_null() || !methods.sq_length.is_null())
        {
            return 1;
        }
        0
    }) {
        if result != 0 {
            return result;
        }
    }
    let getitem = unsafe { PyObject_HasAttrString(object, c"__getitem__".as_ptr()) };
    if getitem <= 0 {
        return 0;
    }
    let len = unsafe { PyObject_HasAttrString(object, c"__len__".as_ptr()) };
    if len <= 0 { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Size(object: *mut c_void) -> isize {
    unsafe { PyObject_Size(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Length(object: *mut c_void) -> isize {
    unsafe { PySequence_Size(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_GetItem(object: *mut c_void, index: isize) -> *mut c_void {
    let index = unsafe { PyLong_FromSsize_t(index) };
    if index.is_null() {
        return std::ptr::null_mut();
    }
    let value = unsafe { PyObject_GetItem(object, index) };
    unsafe { Py_DecRef(index) };
    value
}

unsafe fn cpython_sequence_build_slice(low: isize, high: isize) -> *mut c_void {
    let start = unsafe { PyLong_FromSsize_t(low) };
    if start.is_null() {
        return std::ptr::null_mut();
    }
    let stop = unsafe { PyLong_FromSsize_t(high) };
    if stop.is_null() {
        unsafe { Py_DecRef(start) };
        return std::ptr::null_mut();
    }
    let slice = unsafe { PySlice_New(start, stop, std::ptr::null_mut()) };
    unsafe {
        Py_DecRef(start);
        Py_DecRef(stop);
    }
    slice
}

fn cpython_slice_bounds_step_one(
    len: usize,
    lower: Option<i64>,
    upper: Option<i64>,
) -> (usize, usize) {
    let len_isize = len as isize;
    let mut start = lower.unwrap_or(0) as isize;
    if start < 0 {
        start += len_isize;
    }
    if start < 0 {
        start = 0;
    } else if start > len_isize {
        start = len_isize;
    }

    let mut stop = upper.unwrap_or(len as i64) as isize;
    if stop < 0 {
        stop += len_isize;
    }
    if stop < 0 {
        stop = 0;
    } else if stop > len_isize {
        stop = len_isize;
    }

    let start = start as usize;
    let stop = (if stop < start as isize {
        start as isize
    } else {
        stop
    }) as usize;
    (start, stop)
}

fn cpython_slice_indices_for_len(
    len: usize,
    lower: Option<i64>,
    upper: Option<i64>,
    step: Option<i64>,
) -> Result<Vec<usize>, String> {
    let len_isize = len as isize;
    let step = step.unwrap_or(1);
    if step == 0 {
        return Err("slice step cannot be zero".to_string());
    }
    let step = step as isize;

    let (start, stop) = if step > 0 {
        let mut start = lower.unwrap_or(0) as isize;
        if start < 0 {
            start += len_isize;
        }
        if start < 0 {
            start = 0;
        } else if start > len_isize {
            start = len_isize;
        }

        let mut stop = upper.unwrap_or(len as i64) as isize;
        if stop < 0 {
            stop += len_isize;
        }
        if stop < 0 {
            stop = 0;
        } else if stop > len_isize {
            stop = len_isize;
        }
        (start, stop)
    } else {
        let mut start = lower.unwrap_or(len as i64 - 1) as isize;
        if start < 0 {
            start += len_isize;
        }
        if start < -1 {
            start = -1;
        } else if start >= len_isize {
            start = len_isize - 1;
        }

        let mut stop = upper.unwrap_or(-1) as isize;
        if upper.is_some() && stop < 0 {
            stop += len_isize;
        }
        if stop < -1 {
            stop = -1;
        } else if stop >= len_isize {
            stop = len_isize - 1;
        }
        (start, stop)
    };

    let mut out = Vec::new();
    if step > 0 {
        let mut idx = start;
        while idx < stop {
            out.push(idx as usize);
            idx += step;
        }
    } else {
        let mut idx = start;
        while idx > stop {
            out.push(idx as usize);
            idx += step;
        }
    }
    Ok(out)
}

unsafe fn cpython_sequence_del_item_with_key(object: *mut c_void, key: *mut c_void) -> i32 {
    unsafe { PyObject_DelItem(object, key) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_GetSlice(
    object: *mut c_void,
    low: isize,
    high: isize,
) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let slice = unsafe { cpython_sequence_build_slice(low, high) };
    if slice.is_null() {
        return std::ptr::null_mut();
    }
    let result = unsafe { PyObject_GetItem(object, slice) };
    unsafe { Py_DecRef(slice) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_SetItem(
    object: *mut c_void,
    index: isize,
    value: *mut c_void,
) -> i32 {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if value.is_null() {
        return unsafe { PySequence_DelItem(object, index) };
    }
    let key = unsafe { PyLong_FromSsize_t(index) };
    if key.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_SetItem(object, key, value) };
    unsafe { Py_DecRef(key) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_DelItem(object: *mut c_void, index: isize) -> i32 {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let key = unsafe { PyLong_FromSsize_t(index) };
    if key.is_null() {
        return -1;
    }
    let status = unsafe { cpython_sequence_del_item_with_key(object, key) };
    unsafe { Py_DecRef(key) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_SetSlice(
    object: *mut c_void,
    low: isize,
    high: isize,
    value: *mut c_void,
) -> i32 {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if value.is_null() {
        return unsafe { PySequence_DelSlice(object, low, high) };
    }
    let slice = unsafe { cpython_sequence_build_slice(low, high) };
    if slice.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_SetItem(object, slice, value) };
    unsafe { Py_DecRef(slice) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_DelSlice(object: *mut c_void, low: isize, high: isize) -> i32 {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let slice = unsafe { cpython_sequence_build_slice(low, high) };
    if slice.is_null() {
        return -1;
    }
    let status = unsafe { cpython_sequence_del_item_with_key(object, slice) };
    unsafe { Py_DecRef(slice) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Contains(container: *mut c_void, value: *mut c_void) -> i32 {
    let container = match cpython_value_from_ptr(container) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let value = match cpython_value_from_ptr(value) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySequence_Contains missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_operator_contains(vec![container, value], HashMap::new()) {
            Ok(Value::Bool(flag)) => i32::from(flag),
            Ok(other) => {
                context.set_error(format!(
                    "PySequence_Contains expected bool result, got {other:?}"
                ));
                -1
            }
            Err(err) => {
                context.set_error(err.message);
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_In(container: *mut c_void, value: *mut c_void) -> i32 {
    unsafe { PySequence_Contains(container, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Tuple(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::Tuple, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_List(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::List, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Fast(object: *mut c_void, msg: *const c_char) -> *mut c_void {
    match cpython_value_from_ptr(object) {
        Ok(Value::Tuple(_) | Value::List(_)) => {
            unsafe { Py_XIncRef(object) };
            object
        }
        Ok(value) => match cpython_call_builtin(BuiltinFunction::Tuple, vec![value]) {
            Ok(value) => cpython_new_ptr_for_value(value),
            Err(err) => {
                if !msg.is_null()
                    && let Ok(text) = unsafe { c_name_to_string(msg) }
                {
                    cpython_set_error(text);
                } else {
                    cpython_set_error(err);
                }
                std::ptr::null_mut()
            }
        },
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Concat(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op_with_heap(left, right, add_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_InPlaceConcat(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    unsafe { PySequence_Concat(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Repeat(object: *mut c_void, count: isize) -> *mut c_void {
    let count = unsafe { PyLong_FromSsize_t(count) };
    if count.is_null() {
        return std::ptr::null_mut();
    }
    let result = cpython_binary_numeric_op_with_heap(object, count, mul_values);
    unsafe { Py_DecRef(count) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_InPlaceRepeat(
    object: *mut c_void,
    count: isize,
) -> *mut c_void {
    unsafe { PySequence_Repeat(object, count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Count(object: *mut c_void, value: *mut c_void) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(sequence_handle) = context.cpython_handle_from_ptr(object) else {
            context.set_error("PySequence_Count received unknown sequence pointer");
            return -1;
        };
        let Some(value_handle) = context.cpython_handle_from_ptr(value) else {
            context.set_error("PySequence_Count received unknown value pointer");
            return -1;
        };
        let Some(needle) = context.object_value(value_handle) else {
            context.set_error("PySequence_Count value handle is not available");
            return -1;
        };
        let iterator_handle = match context.object_get_iter(sequence_handle) {
            Ok(handle) => handle,
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        };
        let mut count: isize = 0;
        loop {
            let next_handle = match context.object_iter_next(iterator_handle) {
                Ok(next) => next,
                Err(err) => {
                    context.set_error(err);
                    let _ = context.decref(iterator_handle);
                    return -1;
                }
            };
            let Some(item_handle) = next_handle else {
                break;
            };
            let Some(item_value) = context.object_value(item_handle) else {
                context.set_error("PySequence_Count iterator item handle is not available");
                let _ = context.decref(item_handle);
                let _ = context.decref(iterator_handle);
                return -1;
            };
            let is_match = {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *context.vm };
                match vm.compare_eq_runtime(item_value, needle.clone()) {
                    Ok(Value::Bool(flag)) => flag,
                    Ok(other) => is_truthy(&other),
                    Err(err) => {
                        context.set_error(err.message);
                        let _ = context.decref(item_handle);
                        let _ = context.decref(iterator_handle);
                        return -1;
                    }
                }
            };
            let _ = context.decref(item_handle);
            if is_match {
                count = match count.checked_add(1) {
                    Some(next) => next,
                    None => {
                        context.set_error("count exceeds C integer size");
                        let _ = context.decref(iterator_handle);
                        return -1;
                    }
                };
            }
        }
        let _ = context.decref(iterator_handle);
        count
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Index(object: *mut c_void, value: *mut c_void) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(sequence_handle) = context.cpython_handle_from_ptr(object) else {
            context.set_error("PySequence_Index received unknown sequence pointer");
            return -1;
        };
        let Some(value_handle) = context.cpython_handle_from_ptr(value) else {
            context.set_error("PySequence_Index received unknown value pointer");
            return -1;
        };
        let Some(needle) = context.object_value(value_handle) else {
            context.set_error("PySequence_Index value handle is not available");
            return -1;
        };
        let iterator_handle = match context.object_get_iter(sequence_handle) {
            Ok(handle) => handle,
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        };
        let mut index: isize = 0;
        loop {
            let next_handle = match context.object_iter_next(iterator_handle) {
                Ok(next) => next,
                Err(err) => {
                    context.set_error(err);
                    let _ = context.decref(iterator_handle);
                    return -1;
                }
            };
            let Some(item_handle) = next_handle else {
                break;
            };
            let Some(item_value) = context.object_value(item_handle) else {
                context.set_error("PySequence_Index iterator item handle is not available");
                let _ = context.decref(item_handle);
                let _ = context.decref(iterator_handle);
                return -1;
            };
            let is_match = {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *context.vm };
                match vm.compare_eq_runtime(item_value, needle.clone()) {
                    Ok(Value::Bool(flag)) => flag,
                    Ok(other) => is_truthy(&other),
                    Err(err) => {
                        context.set_error(err.message);
                        let _ = context.decref(item_handle);
                        let _ = context.decref(iterator_handle);
                        return -1;
                    }
                }
            };
            let _ = context.decref(item_handle);
            if is_match {
                let _ = context.decref(iterator_handle);
                return index;
            }
            index = match index.checked_add(1) {
                Some(next) => next,
                None => {
                    context.set_error("index exceeds C integer size");
                    let _ = context.decref(iterator_handle);
                    return -1;
                }
            };
        }
        let _ = context.decref(iterator_handle);
        context.set_error("sequence.index(x): x not in sequence");
        -1
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_GetItemString(
    mapping: *mut c_void,
    key: *const c_char,
) -> *mut c_void {
    if mapping.is_null() || key.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let key = unsafe { PyUnicode_FromString(key) };
    if key.is_null() {
        return std::ptr::null_mut();
    }
    let result = unsafe { PyObject_GetItem(mapping, key) };
    unsafe { Py_DecRef(key) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Check(object: *mut c_void) -> i32 {
    if object.is_null() {
        return 0;
    }
    match cpython_value_from_ptr(object) {
        Ok(
            Value::Dict(_)
            | Value::List(_)
            | Value::Tuple(_)
            | Value::Str(_)
            | Value::Bytes(_)
            | Value::ByteArray(_),
        ) => 1,
        Ok(_) => {
            let status =
                unsafe { PyObject_HasAttrStringWithError(object, c"__getitem__".as_ptr()) };
            if status < 0 {
                unsafe { PyErr_Clear() };
                0
            } else {
                status
            }
        }
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Size(object: *mut c_void) -> isize {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if unsafe { PyMapping_Check(object) } == 0 {
        let type_name = cpython_value_type_name_from_ptr(object);
        let has_len = unsafe { PyObject_HasAttrStringWithError(object, c"__len__".as_ptr()) };
        if has_len < 0 {
            return -1;
        }
        if has_len == 1 {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                format!("{type_name} is not a mapping"),
            );
        } else {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                format!("object of type '{type_name}' has no len()"),
            );
        }
        return -1;
    }
    unsafe { PyObject_Size(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Length(object: *mut c_void) -> isize {
    unsafe { PyMapping_Size(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_GetOptionalItem(
    object: *mut c_void,
    key: *mut c_void,
    result: *mut *mut c_void,
) -> i32 {
    if result.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe { *result = std::ptr::null_mut() };
    if object.is_null() || key.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyMapping_GetOptionalItem missing VM context");
            return -1;
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyMapping_GetOptionalItem received unknown object pointer");
            return -1;
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyMapping_GetOptionalItem received unknown key pointer");
            return -1;
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.getitem_value(object_value, key_value) {
            Ok(value) => {
                let ptr = context.alloc_cpython_ptr_for_value(value);
                if ptr.is_null() {
                    -1
                } else {
                    unsafe { *result = ptr };
                    1
                }
            }
            Err(err) => {
                if cpython_active_exception_is(vm, "KeyError")
                    || err.message.contains("key not found")
                    || err.message.contains("KeyError")
                {
                    cpython_clear_active_exception(vm);
                    0
                } else {
                    context.set_error(err.message);
                    -1
                }
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_GetOptionalItemString(
    object: *mut c_void,
    key: *const c_char,
    result: *mut *mut c_void,
) -> i32 {
    if key.is_null() || result.is_null() {
        if !result.is_null() {
            unsafe { *result = std::ptr::null_mut() };
        }
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        unsafe { *result = std::ptr::null_mut() };
        return -1;
    }
    let status = unsafe { PyMapping_GetOptionalItem(object, key_obj, result) };
    unsafe { Py_DecRef(key_obj) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_SetItemString(
    object: *mut c_void,
    key: *const c_char,
    value: *mut c_void,
) -> i32 {
    if key.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_SetItem(object, key_obj, value) };
    unsafe { Py_DecRef(key_obj) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKeyWithError(object: *mut c_void, key: *mut c_void) -> i32 {
    let mut value: *mut c_void = std::ptr::null_mut();
    let status = unsafe { PyMapping_GetOptionalItem(object, key, &mut value) };
    unsafe { Py_XDecRef(value) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKeyStringWithError(
    object: *mut c_void,
    key: *const c_char,
) -> i32 {
    let mut value: *mut c_void = std::ptr::null_mut();
    let status = unsafe { PyMapping_GetOptionalItemString(object, key, &mut value) };
    unsafe { Py_XDecRef(value) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKey(object: *mut c_void, key: *mut c_void) -> i32 {
    let status = unsafe { PyMapping_HasKeyWithError(object, key) };
    if status < 0 {
        unsafe { PyErr_Clear() };
        0
    } else {
        status
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKeyString(object: *mut c_void, key: *const c_char) -> i32 {
    let status = unsafe { PyMapping_HasKeyStringWithError(object, key) };
    if status < 0 {
        unsafe { PyErr_Clear() };
        0
    } else {
        status
    }
}

fn cpython_mapping_method_output_as_list(object: *mut c_void, method_name: &str) -> *mut c_void {
    let method = match CString::new(method_name) {
        Ok(name) => unsafe { PyObject_GetAttrString(object, name.as_ptr()) },
        Err(err) => {
            cpython_set_error(err.to_string());
            return std::ptr::null_mut();
        }
    };
    if method.is_null() {
        return std::ptr::null_mut();
    }
    let output = unsafe { PyObject_CallNoArgs(method) };
    unsafe { Py_DecRef(method) };
    if output.is_null() {
        return std::ptr::null_mut();
    }
    let list = match cpython_value_from_ptr(output) {
        Ok(Value::List(_)) => output,
        Ok(_) => {
            let list = unsafe { PySequence_List(output) };
            unsafe { Py_DecRef(output) };
            list
        }
        Err(err) => {
            cpython_set_error(err);
            unsafe { Py_DecRef(output) };
            std::ptr::null_mut()
        }
    };
    list
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Keys(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if let Ok(Value::Dict(_)) = cpython_value_from_ptr(object) {
        return unsafe { PyDict_Keys(object) };
    }
    cpython_mapping_method_output_as_list(object, "keys")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Items(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if let Ok(Value::Dict(_)) = cpython_value_from_ptr(object) {
        return unsafe { PyDict_Items(object) };
    }
    cpython_mapping_method_output_as_list(object, "items")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Values(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if let Ok(Value::Dict(_)) = cpython_value_from_ptr(object) {
        return unsafe { PyDict_Values(object) };
    }
    cpython_mapping_method_output_as_list(object, "values")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySeqIter_New(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if object.is_null() {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            context.set_error("PySeqIter_New missing VM context");
            return std::ptr::null_mut();
        }
        if unsafe { PySequence_Check(object) } == 0 {
            context.set_error("PySeqIter_New() argument must be a sequence");
            return std::ptr::null_mut();
        }
        let Some(target_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PySeqIter_New received unknown object pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let iterator = vm.heap.alloc(Object::Iterator(IteratorObject {
            kind: IteratorKind::CpythonSequence {
                target: target_value,
            },
            index: 0,
        }));
        context.alloc_cpython_ptr_for_value(Value::Iterator(iterator))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCallIter_New(
    callable: *mut c_void,
    sentinel: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if callable.is_null() || sentinel.is_null() {
            context.set_error("PyCallIter_New received null callable/sentinel");
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            context.set_error("PyCallIter_New missing VM context");
            return std::ptr::null_mut();
        }
        let Some(callable_value) = context.cpython_value_from_ptr_or_proxy(callable) else {
            context.set_error("PyCallIter_New received unknown callable pointer");
            return std::ptr::null_mut();
        };
        let Some(sentinel_value) = context.cpython_value_from_ptr_or_proxy(sentinel) else {
            context.set_error("PyCallIter_New received unknown sentinel pointer");
            return std::ptr::null_mut();
        };
        let callable_check = unsafe { PyCallable_Check(callable) };
        if callable_check < 0 {
            return std::ptr::null_mut();
        }
        if callable_check == 0 {
            context.set_error("TypeError: iter(v, w): v must be callable");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let iterator = vm.heap.alloc(Object::Iterator(IteratorObject {
            kind: IteratorKind::CallIter {
                callable: callable_value,
                sentinel: sentinel_value,
            },
            index: 0,
        }));
        context.alloc_cpython_ptr_for_value(Value::Iterator(iterator))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_invoke_method_from_values(
    context: &mut ModuleCapiContext,
    method_def: *mut CpythonMethodDef,
    self_obj: *mut c_void,
    class_obj: *mut c_void,
    args: Vec<Value>,
    kwargs: HashMap<String, Value>,
) -> *mut c_void {
    if method_def.is_null() {
        context.set_error("missing method definition");
        return std::ptr::null_mut();
    }
    // SAFETY: method pointer comes from an extension-provided PyMethodDef table.
    let Some(method) = (unsafe { (*method_def).ml_meth }) else {
        context.set_error("missing method callback");
        return std::ptr::null_mut();
    };
    let trace_calls = std::env::var_os("PYRS_TRACE_CPY_METHOD_CALLS").is_some();
    let trace_numpy_empty = std::env::var_os("PYRS_TRACE_NUMPY_EMPTY_CALL").is_some();
    let trace_numpy_result_type = std::env::var_os("PYRS_TRACE_NUMPY_RESULT_TYPE").is_some();
    let trace_set_typedict = std::env::var_os("PYRS_TRACE_NUMPY_TYPEDICT").is_some();
    let trace_numpy_subtract = std::env::var_os("PYRS_TRACE_NUMPY_SUBTRACT").is_some();
    let trace_array_function_dispatcher =
        std::env::var_os("PYRS_TRACE_ARRAY_FUNCTION_DISPATCHER").is_some();
    let method_name = if trace_calls
        || trace_numpy_empty
        || trace_numpy_result_type
        || trace_set_typedict
        || trace_numpy_subtract
        || trace_array_function_dispatcher
        || std::env::var_os("PYRS_TRACE_COPYTO_CALL").is_some()
    {
        // SAFETY: method definition pointer is valid for metadata reads.
        unsafe {
            c_name_to_string((*method_def).ml_name).unwrap_or_else(|_| "<invalid>".to_string())
        }
    } else {
        String::new()
    };
    if trace_numpy_subtract && method_name == "subtract" {
        let arg_summary = args
            .iter()
            .map(cpython_value_debug_tag)
            .collect::<Vec<_>>()
            .join(", ");
        let mut kwargs_sorted = kwargs.iter().collect::<Vec<_>>();
        kwargs_sorted.sort_by(|(left, _), (right, _)| left.cmp(right));
        let kw_summary = kwargs_sorted
            .into_iter()
            .map(|(name, value)| format!("{name}={}", cpython_value_debug_tag(value)))
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "[numpy-subtract] args=[{}] kwargs=[{}] self={:p} class={:p}",
            arg_summary, kw_summary, self_obj, class_obj
        );
    }
    if trace_array_function_dispatcher && method_name == "_ArrayFunctionDispatcher" {
        let arg_tags = args
            .iter()
            .map(cpython_value_debug_tag)
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "[array-func-dispatcher-call] self={:p} class={:p} flags={} args_len={} args=[{}] kwargs_len={}",
            self_obj,
            class_obj,
            // SAFETY: method definition layout follows CPython ABI.
            unsafe { (*method_def).ml_flags },
            args.len(),
            arg_tags,
            kwargs.len()
        );
    }
    // SAFETY: method definition layout follows CPython ABI.
    let flags = unsafe { (*method_def).ml_flags };
    if std::env::var_os("PYRS_TRACE_COPYTO_CALL").is_some() && method_name == "copyto" {
        let mut kw_names = kwargs.keys().cloned().collect::<Vec<_>>();
        kw_names.sort();
        let arg_tags = args
            .iter()
            .map(cpython_value_debug_tag)
            .collect::<Vec<_>>()
            .join(", ");
        let kw_entries = kwargs
            .iter()
            .map(|(name, value)| format!("{name}={}", cpython_value_debug_tag(value)))
            .collect::<Vec<_>>()
            .join(", ");
        let self_tag = context
            .cpython_value_from_ptr_or_proxy(self_obj)
            .map(|value| cpython_value_debug_tag(&value))
            .unwrap_or_else(|| "<unknown-self>".to_string());
        let self_type =
            cpython_safe_object_type_name(self_obj).unwrap_or_else(|| "<unknown-type>".to_string());
        eprintln!(
            "[copyto-call] flags={} self={:p} self_tag={} self_type={} class={:p} def={:p} args_len={} args=[{}] kwargs=[{}]",
            flags,
            self_obj,
            self_tag,
            self_type,
            class_obj,
            method_def,
            args.len(),
            arg_tags,
            kw_entries
        );
    }
    if std::env::var_os("PYRS_TRACE_NUMPY_METHOD_BINDING").is_some()
        && matches!(
            method_name.as_str(),
            "copyto" | "dot" | "arange" | "empty_like" | "empty" | "result_type"
        )
    {
        let arg_tags = args
            .iter()
            .map(cpython_value_debug_tag)
            .collect::<Vec<_>>()
            .join(", ");
        let mut kw_entries = kwargs
            .iter()
            .map(|(name, value)| format!("{name}={}", cpython_value_debug_tag(value)))
            .collect::<Vec<_>>();
        kw_entries.sort();
        let self_tag = context
            .cpython_value_from_ptr_or_proxy(self_obj)
            .map(|value| cpython_value_debug_tag(&value))
            .unwrap_or_else(|| "<unknown-self>".to_string());
        let self_type =
            cpython_safe_object_type_name(self_obj).unwrap_or_else(|| "<unknown-type>".to_string());
        let class_type = cpython_safe_object_type_name(class_obj)
            .unwrap_or_else(|| "<unknown-class>".to_string());
        eprintln!(
            "[numpy-method-binding] name={} flags={} self={:p} self_tag={} self_type={} class={:p} class_type={} args=[{}] kwargs=[{}]",
            method_name,
            flags,
            self_obj,
            self_tag,
            self_type,
            class_obj,
            class_type,
            arg_tags,
            kw_entries.join(", ")
        );
    }
    if flags & METH_METHOD != 0 {
        if flags & (METH_FASTCALL | METH_KEYWORDS) != (METH_FASTCALL | METH_KEYWORDS) {
            context.set_error("METH_METHOD requires METH_FASTCALL|METH_KEYWORDS");
            return std::ptr::null_mut();
        }
        if class_obj.is_null() {
            context.set_error("METH_METHOD call missing defining class");
            return std::ptr::null_mut();
        }
        let mut stack: Vec<*mut c_void> =
            Vec::with_capacity(args.len().saturating_add(kwargs.len()));
        for value in &args {
            let ptr = context.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                context.set_error("failed to materialize METH_METHOD positional argument");
                return std::ptr::null_mut();
            }
            stack.push(ptr);
        }
        let mut kw_name_ptrs: Vec<*mut c_void> = Vec::with_capacity(kwargs.len());
        for (name, value) in &kwargs {
            let c_name = match CString::new(name.as_str()) {
                Ok(c_name) => c_name,
                Err(_) => {
                    context.set_error("METH_METHOD keyword name contains interior NUL byte");
                    return std::ptr::null_mut();
                }
            };
            // SAFETY: C string is NUL-terminated and valid for this call.
            let name_ptr = unsafe { PyUnicode_InternFromString(c_name.as_ptr()) };
            if name_ptr.is_null() {
                context.set_error("failed to intern METH_METHOD keyword name");
                return std::ptr::null_mut();
            }
            kw_name_ptrs.push(name_ptr);
            let ptr = context.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                context.set_error("failed to materialize METH_METHOD keyword argument");
                return std::ptr::null_mut();
            }
            stack.push(ptr);
        }
        if context.vm.is_null() {
            context.set_error("METH_METHOD call missing VM context");
            return std::ptr::null_mut();
        }
        let kwnames_ptr = if kw_name_ptrs.is_empty() {
            std::ptr::null_mut()
        } else {
            // SAFETY: tuple allocation follows CPython tuple ABI.
            let tuple = unsafe { PyTuple_New(kw_name_ptrs.len() as isize) };
            if tuple.is_null() {
                context.set_error("failed to allocate METH_METHOD keyword names tuple");
                return std::ptr::null_mut();
            }
            for (index, name_ptr) in kw_name_ptrs.into_iter().enumerate() {
                // SAFETY: tuple is newly allocated and index is in-bounds.
                let status = unsafe { PyTuple_SetItem(tuple, index as isize, name_ptr) };
                if status != 0 {
                    // SAFETY: tuple owns any already-inserted references.
                    unsafe { Py_DecRef(tuple) };
                    context.set_error("failed to populate METH_METHOD keyword names tuple");
                    return std::ptr::null_mut();
                }
            }
            tuple
        };
        if !kwargs.is_empty() && kwnames_ptr.is_null() {
            context.set_error("failed to materialize METH_METHOD keyword names");
            return std::ptr::null_mut();
        }
        let call: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *const *mut c_void,
            usize,
            *mut c_void,
        ) -> *mut c_void =
            // SAFETY: flags indicate `PyCMethod`-compatible signature.
            unsafe { std::mem::transmute(method) };
        let args_ptr = if stack.is_empty() {
            std::ptr::null()
        } else {
            stack.as_ptr()
        };
        let result = unsafe { call(self_obj, class_obj, args_ptr, args.len(), kwnames_ptr) };
        if !kwnames_ptr.is_null() {
            // SAFETY: kwnames tuple is call-local materialization and no longer needed after call.
            unsafe { Py_DecRef(kwnames_ptr) };
        }
        if trace_calls {
            eprintln!(
                "[cpy-method-call] name={} flags={} cmethod nargs={} kwargs={} class={:p} result={:p}",
                method_name,
                flags,
                args.len(),
                kwargs.len(),
                class_obj,
                result
            );
        }
        return result;
    }
    if flags & METH_FASTCALL != 0 {
        if context.vm.is_null() {
            context.set_error("METH_FASTCALL call missing VM context");
            return std::ptr::null_mut();
        }
        let accepts_keywords = (flags & METH_KEYWORDS) != 0;
        if !accepts_keywords && !kwargs.is_empty() {
            context.set_error("METH_FASTCALL call does not accept keyword arguments");
            return std::ptr::null_mut();
        }
        if trace_numpy_empty && method_name == "empty" {
            let mut names: Vec<String> = kwargs.keys().cloned().collect();
            names.sort();
            eprintln!(
                "[numpy-empty] fastcall args_len={} kwargs={:?}",
                args.len(),
                names
            );
        }
        let mut stack: Vec<*mut c_void> =
            Vec::with_capacity(args.len().saturating_add(if accepts_keywords {
                kwargs.len()
            } else {
                0
            }));
        for value in &args {
            let ptr = context.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                context.set_error("failed to materialize FASTCALL positional argument");
                return std::ptr::null_mut();
            }
            stack.push(ptr);
        }
        let mut kw_name_ptrs: Vec<*mut c_void> = Vec::new();
        if accepts_keywords {
            kw_name_ptrs = Vec::with_capacity(kwargs.len());
            for (name, value) in &kwargs {
                let c_name = match CString::new(name.as_str()) {
                    Ok(c_name) => c_name,
                    Err(_) => {
                        context.set_error("FASTCALL keyword name contains interior NUL byte");
                        return std::ptr::null_mut();
                    }
                };
                // SAFETY: C string is NUL-terminated and valid for this call.
                let name_ptr = unsafe { PyUnicode_InternFromString(c_name.as_ptr()) };
                if name_ptr.is_null() {
                    context.set_error("failed to intern FASTCALL keyword name");
                    return std::ptr::null_mut();
                }
                kw_name_ptrs.push(name_ptr);
                let ptr = context.alloc_cpython_ptr_for_value(value.clone());
                if ptr.is_null() {
                    context.set_error("failed to materialize FASTCALL keyword argument");
                    return std::ptr::null_mut();
                }
                stack.push(ptr);
            }
        }
        let args_ptr = if stack.is_empty() {
            std::ptr::null()
        } else {
            stack.as_ptr()
        };
        let result = if accepts_keywords {
            let kwnames_ptr = if kw_name_ptrs.is_empty() {
                std::ptr::null_mut()
            } else {
                // SAFETY: tuple allocation follows CPython tuple ABI.
                let tuple = unsafe { PyTuple_New(kw_name_ptrs.len() as isize) };
                if tuple.is_null() {
                    context.set_error("failed to allocate FASTCALL keyword names tuple");
                    return std::ptr::null_mut();
                }
                for (index, name_ptr) in kw_name_ptrs.into_iter().enumerate() {
                    // SAFETY: tuple is newly allocated and index is in-bounds.
                    let status = unsafe { PyTuple_SetItem(tuple, index as isize, name_ptr) };
                    if status != 0 {
                        // SAFETY: tuple owns any already-inserted references.
                        unsafe { Py_DecRef(tuple) };
                        context.set_error("failed to populate FASTCALL keyword names tuple");
                        return std::ptr::null_mut();
                    }
                }
                tuple
            };
            if !kwargs.is_empty() && kwnames_ptr.is_null() {
                context.set_error("failed to materialize FASTCALL keyword names");
                return std::ptr::null_mut();
            }
            let call: unsafe extern "C" fn(*mut c_void, *const *mut c_void, usize, *mut c_void) -> *mut c_void =
                // SAFETY: method flags indicate FASTCALL|KEYWORDS signature.
                unsafe { std::mem::transmute(method) };
            let result = unsafe { call(self_obj, args_ptr, args.len(), kwnames_ptr) };
            if !kwnames_ptr.is_null() {
                // SAFETY: kwnames tuple is call-local materialization and no longer needed after call.
                unsafe { Py_DecRef(kwnames_ptr) };
            }
            result
        } else {
            let call: unsafe extern "C" fn(*mut c_void, *const *mut c_void, usize) -> *mut c_void =
                // SAFETY: method flags indicate FASTCALL-only signature.
                unsafe { std::mem::transmute(method) };
            unsafe { call(self_obj, args_ptr, args.len()) }
        };
        if trace_numpy_result_type && method_name == "result_type" {
            let mut kw_names = kwargs.keys().cloned().collect::<Vec<_>>();
            kw_names.sort();
            let mapped_summary = context
                .cpython_value_from_ptr_or_proxy(result)
                .map(|value| {
                    let raw = ModuleCapiContext::cpython_proxy_raw_ptr_from_value(&value)
                        .map(|ptr| format!("{:p}", ptr))
                        .unwrap_or_else(|| "<none>".to_string());
                    let class_raw = match &value {
                        Value::Instance(instance_obj) => match &*instance_obj.kind() {
                            Object::Instance(instance_data) => match &*instance_data.class.kind() {
                                Object::Class(class_data) => {
                                    match class_data.attrs.get(CPY_PROXY_PTR_ATTR) {
                                        Some(Value::Int(raw_ptr)) if *raw_ptr >= 0 => {
                                            format!("{:p}", *raw_ptr as usize as *mut c_void)
                                        }
                                        _ => "<none>".to_string(),
                                    }
                                }
                                _ => "<none>".to_string(),
                            },
                            _ => "<none>".to_string(),
                        },
                        _ => "<none>".to_string(),
                    };
                    let class_tp_name = if let Ok(class_ptr) =
                        usize::from_str_radix(class_raw.trim_start_matches("0x"), 16)
                    {
                        let class_ptr = class_ptr as *mut c_void;
                        if class_ptr.is_null() {
                            "<none>".to_string()
                        } else {
                            // SAFETY: diagnostics on proxy raw class pointer.
                            unsafe {
                                class_ptr
                                    .cast::<CpythonTypeObject>()
                                    .as_ref()
                                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                                    .unwrap_or_else(|| "<unknown>".to_string())
                            }
                        }
                    } else {
                        "<none>".to_string()
                    };
                    format!(
                        "value_type={} value_raw={} class_raw={} class_tp_name={}",
                        cpython_debug_ufunc_attr_summary(&value, 2),
                        raw,
                        class_raw,
                        class_tp_name
                    )
                })
                .unwrap_or_else(|| "<none>".to_string());
            eprintln!(
                "[numpy-result-type] flags={} nargs={} kwargs={:?} result_ptr={:p} result_ob_type={:p} result_type_name={} mapped={}",
                flags,
                args.len(),
                kw_names,
                result,
                if result.is_null() {
                    std::ptr::null_mut()
                } else {
                    // SAFETY: diagnostics only for returned PyObject pointer.
                    unsafe {
                        result
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type)
                            .unwrap_or(std::ptr::null_mut())
                    }
                },
                cpython_type_name_for_object_ptr(result),
                mapped_summary
            );
        }
        if trace_calls {
            eprintln!(
                "[cpy-method-call] name={} flags={} fastcall nargs={} kwargs={} result={:p}",
                method_name,
                flags,
                args.len(),
                kwargs.len(),
                result
            );
        }
        return result;
    }
    if flags & METH_KEYWORDS != 0 {
        if context.vm.is_null() {
            context.set_error("METH_KEYWORDS call missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let args_ptr = context.alloc_cpython_ptr_for_value(vm.heap.alloc_tuple(args));
        if args_ptr.is_null() {
            context.set_error("failed to materialize cfunction args tuple");
            return std::ptr::null_mut();
        }
        let kwargs_empty = kwargs.is_empty();
        let kwargs_ptr = if kwargs_empty {
            std::ptr::null_mut()
        } else {
            let entries = kwargs
                .into_iter()
                .map(|(name, value)| (Value::Str(name), value))
                .collect::<Vec<_>>();
            context.alloc_cpython_ptr_for_value(vm.heap.alloc_dict(entries))
        };
        if !kwargs_ptr.is_null() {
            // no-op: kwargs materialized successfully.
        } else if !kwargs_empty {
            context.set_error("failed to materialize cfunction kwargs dict");
            return std::ptr::null_mut();
        }
        let call: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: method flags indicate VARARGS|KEYWORDS signature.
            unsafe { std::mem::transmute(method) };
        return unsafe { call(self_obj, args_ptr, kwargs_ptr) };
    }
    if flags & METH_VARARGS != 0 {
        if !kwargs.is_empty() {
            context.set_error("METH_VARARGS call does not accept keyword arguments");
            return std::ptr::null_mut();
        }
        if trace_set_typedict && method_name == "set_typeDict" {
            let arg_len = args.len();
            if let Some(Value::Dict(dict_obj)) = args.first()
                && let Object::Dict(dict_data) = &*dict_obj.kind()
            {
                let sample = dict_data
                    .iter()
                    .take(8)
                    .map(|(k, _)| cpython_debug_compare_value(k))
                    .collect::<Vec<_>>()
                    .join(", ");
                let has_int8 = dict_data.contains_key(&Value::Str("int8".to_string()));
                let has_bool = dict_data.contains_key(&Value::Str("bool".to_string()));
                let has_float64 = dict_data.contains_key(&Value::Str("float64".to_string()));
                eprintln!(
                    "[numpy-typedict] incoming dict entries={} has_int8={} has_bool={} has_float64={} sample=[{}]",
                    dict_data.len(),
                    has_int8,
                    has_bool,
                    has_float64,
                    sample
                );
            } else {
                eprintln!(
                    "[numpy-typedict] incoming args_len={} first={}",
                    arg_len,
                    args.first()
                        .map(cpython_value_debug_tag)
                        .unwrap_or_else(|| "<none>".to_string())
                );
            }
        }
        if context.vm.is_null() {
            context.set_error("METH_VARARGS call missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let args_ptr = context.alloc_cpython_ptr_for_value(vm.heap.alloc_tuple(args));
        if args_ptr.is_null() {
            context.set_error("failed to materialize cfunction args tuple");
            return std::ptr::null_mut();
        }
        if trace_set_typedict && method_name == "set_typeDict" {
            let dict_ptr = unsafe { PyTuple_GetItem(args_ptr, 0) };
            TRACE_NUMPY_TYPEDICT_PTR.store(dict_ptr as usize, Ordering::Relaxed);
            let probe_key = unsafe { PyUnicode_FromString(c"int8".as_ptr()) };
            let probe_value = if probe_key.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { PyDict_GetItemWithError(dict_ptr, probe_key) }
            };
            let probe_error = unsafe { PyErr_Occurred() };
            if !probe_key.is_null() {
                unsafe { Py_DecRef(probe_key) };
            }
            eprintln!(
                "[numpy-typedict] c-arg tuple={:p} dict_ptr={:p} probe_int8={:p} probe_err={:p}",
                args_ptr, dict_ptr, probe_value, probe_error
            );
        }
        let call: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: method flags indicate VARARGS signature.
            unsafe { std::mem::transmute(method) };
        let result = unsafe { call(self_obj, args_ptr) };
        if trace_set_typedict && method_name == "set_typeDict" {
            eprintln!(
                "[numpy-typedict] result={:p} last_error={:?}",
                result, context.last_error
            );
        }
        return result;
    }
    if flags & METH_NOARGS != 0 {
        if !args.is_empty() || !kwargs.is_empty() {
            context.set_error("METH_NOARGS call expected no arguments");
            return std::ptr::null_mut();
        }
        let call: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: method flags indicate NOARGS signature.
            unsafe { std::mem::transmute(method) };
        return unsafe { call(self_obj, std::ptr::null_mut()) };
    }
    if flags & METH_O != 0 {
        if args.len() != 1 || !kwargs.is_empty() {
            context.set_error("METH_O call expected exactly one positional argument");
            return std::ptr::null_mut();
        }
        let arg_ptr = context.alloc_cpython_ptr_for_value(args[0].clone());
        if arg_ptr.is_null() {
            context.set_error("failed to materialize cfunction single argument");
            return std::ptr::null_mut();
        }
        let call: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: method flags indicate METH_O signature.
            unsafe { std::mem::transmute(method) };
        return unsafe { call(self_obj, arg_ptr) };
    }
    context.set_error(format!("unsupported cfunction method flags: {flags}"));
    std::ptr::null_mut()
}

fn cpython_method_call_flags_are_valid(flags: i32) -> bool {
    let call_flags =
        flags & (METH_VARARGS | METH_FASTCALL | METH_NOARGS | METH_O | METH_KEYWORDS | METH_METHOD);
    call_flags == METH_VARARGS
        || call_flags == (METH_VARARGS | METH_KEYWORDS)
        || call_flags == METH_FASTCALL
        || call_flags == (METH_FASTCALL | METH_KEYWORDS)
        || call_flags == METH_NOARGS
        || call_flags == METH_O
        || call_flags == (METH_METHOD | METH_FASTCALL | METH_KEYWORDS)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCMethod_New(
    method_def: *mut c_void,
    self_obj: *mut c_void,
    module_obj: *mut c_void,
    class_obj: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if method_def.is_null() {
            context.set_error("bad internal call");
            return std::ptr::null_mut();
        }
        let method_def = method_def.cast::<CpythonMethodDef>();
        // SAFETY: method table pointer is non-null and points to extension-owned definition.
        let flags = unsafe { (*method_def).ml_flags };
        let has_meth_method = (flags & METH_METHOD) != 0;
        if !cpython_method_call_flags_are_valid(flags) {
            // SAFETY: `ml_name` is expected NUL-terminated by PyMethodDef contract.
            let method_name = unsafe {
                c_name_to_string((*method_def).ml_name).unwrap_or_else(|_| "<unnamed>".to_string())
            };
            context.set_error(format!(
                "SystemError: {}() method: bad call flags",
                method_name
            ));
            return std::ptr::null_mut();
        }
        if has_meth_method && class_obj.is_null() {
            context.set_error(
                "SystemError: attempting to create PyCMethod with a METH_METHOD flag but no class",
            );
            return std::ptr::null_mut();
        }
        if !has_meth_method && !class_obj.is_null() {
            context.set_error(
                "SystemError: attempting to create PyCFunction with class but no METH_METHOD flag",
            );
            return std::ptr::null_mut();
        }
        context.alloc_cpython_method_cfunction_ptr(method_def, self_obj, module_obj, class_obj)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_NewEx(
    method_def: *mut c_void,
    self_obj: *mut c_void,
    module_obj: *mut c_void,
) -> *mut c_void {
    unsafe { PyCMethod_New(method_def, self_obj, module_obj, std::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_New(
    method_def: *mut c_void,
    self_obj: *mut c_void,
) -> *mut c_void {
    unsafe { PyCFunction_NewEx(method_def, self_obj, std::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewMethod(
    type_obj: *mut c_void,
    method: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if type_obj.is_null() || method.is_null() {
            context.set_error("bad internal call");
            return std::ptr::null_mut();
        }
        if !cpython_ptr_is_type_object(type_obj) {
            context.set_error("PyDescr_NewMethod expected type object");
            return std::ptr::null_mut();
        }
        let owner_type = type_obj.cast::<CpythonTypeObject>();
        let method_def = method.cast::<CpythonMethodDef>();
        // SAFETY: method pointer is caller-owned and expected to outlive descriptor.
        let method_name_ptr = unsafe { (*method_def).ml_name };
        if method_name_ptr.is_null() {
            context.set_error("SystemError: <unnamed>() method: bad call flags");
            return std::ptr::null_mut();
        }
        // SAFETY: method definition pointer is non-null and points to extension-owned definition.
        let flags = unsafe { (*method_def).ml_flags };
        if !cpython_method_call_flags_are_valid(flags) {
            // SAFETY: `ml_name` is expected NUL-terminated by PyMethodDef contract.
            let method_name = unsafe { c_name_to_string(method_name_ptr) }
                .unwrap_or_else(|_| "<unnamed>".to_string());
            context.set_error(format!(
                "SystemError: {}() method: bad call flags",
                method_name
            ));
            return std::ptr::null_mut();
        }
        context.alloc_cpython_descriptor_ptr(
            std::ptr::addr_of_mut!(PyMethodDescr_Type),
            CpythonDescriptorKind::Method {
                owner_type,
                method_def,
                class_method: false,
            },
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewClassMethod(
    type_obj: *mut c_void,
    method: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if type_obj.is_null() || method.is_null() {
            context.set_error("bad internal call");
            return std::ptr::null_mut();
        }
        if !cpython_ptr_is_type_object(type_obj) {
            context.set_error("PyDescr_NewClassMethod expected type object");
            return std::ptr::null_mut();
        }
        let owner_type = type_obj.cast::<CpythonTypeObject>();
        let method_def = method.cast::<CpythonMethodDef>();
        // SAFETY: method pointer is caller-owned and expected to outlive descriptor.
        let method_name_ptr = unsafe { (*method_def).ml_name };
        if method_name_ptr.is_null() {
            context.set_error("SystemError: <unnamed>() method: bad call flags");
            return std::ptr::null_mut();
        }
        // CPython does not reject classmethod definitions here, but we retain
        // method call-flag validation to prevent invalid call ABI dispatch later.
        let flags = unsafe { (*method_def).ml_flags };
        if !cpython_method_call_flags_are_valid(flags) {
            // SAFETY: `ml_name` is expected NUL-terminated by PyMethodDef contract.
            let method_name = unsafe { c_name_to_string(method_name_ptr) }
                .unwrap_or_else(|_| "<unnamed>".to_string());
            context.set_error(format!(
                "SystemError: {}() method: bad call flags",
                method_name
            ));
            return std::ptr::null_mut();
        }
        context.alloc_cpython_descriptor_ptr(
            std::ptr::addr_of_mut!(PyClassMethodDescr_Type),
            CpythonDescriptorKind::Method {
                owner_type,
                method_def,
                class_method: true,
            },
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewMember(
    type_obj: *mut c_void,
    member: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if type_obj.is_null() || member.is_null() {
            context.set_error("bad internal call");
            return std::ptr::null_mut();
        }
        if !cpython_ptr_is_type_object(type_obj) {
            context.set_error("PyDescr_NewMember expected type object");
            return std::ptr::null_mut();
        }
        let owner_type = type_obj.cast::<CpythonTypeObject>();
        let member_def = member.cast::<CpythonMemberDef>();
        // SAFETY: member definition pointer is non-null and extension-owned.
        let member_name_ptr = unsafe { (*member_def).name };
        if member_name_ptr.is_null() {
            context.set_error("PyDescr_NewMember expected member name");
            return std::ptr::null_mut();
        }
        // SAFETY: member definition pointer is non-null and extension-owned.
        let flags = unsafe { (*member_def).flags };
        if (flags & PY_MEMBER_RELATIVE_OFFSET) != 0 {
            context.set_error("PyDescr_NewMember used with Py_RELATIVE_OFFSET");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_descriptor_ptr(
            std::ptr::addr_of_mut!(PyMemberDescr_Type),
            CpythonDescriptorKind::Member {
                owner_type,
                member: member_def,
            },
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMember_GetOne(
    obj_addr: *const c_char,
    member: *mut c_void,
) -> *mut c_void {
    if obj_addr.is_null() || member.is_null() {
        cpython_set_typed_error(unsafe { PyExc_SystemError }, "bad internal call");
        return std::ptr::null_mut();
    }
    let member_def = unsafe { &*member.cast::<CpythonMemberDef>() };
    if (member_def.flags & PY_MEMBER_RELATIVE_OFFSET) != 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyMember_GetOne used with Py_RELATIVE_OFFSET",
        );
        return std::ptr::null_mut();
    }
    let field_ptr =
        match ModuleCapiContext::member_field_ptr(obj_addr.cast_mut().cast(), member_def) {
            Ok(ptr) => ptr,
            Err(err) => {
                cpython_set_typed_error(unsafe { PyExc_SystemError }, err);
                return std::ptr::null_mut();
            }
        };
    let none_ptr = std::ptr::addr_of_mut!(_Py_NoneStruct).cast::<c_void>();
    match member_def.member_type {
        PY_MEMBER_T_BOOL => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_char>()) };
            unsafe { PyBool_FromLong((raw != 0) as c_long) }
        }
        PY_MEMBER_T_BYTE => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<i8>()) };
            unsafe { PyLong_FromLong(raw as i64) }
        }
        PY_MEMBER_T_UBYTE => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u8>()) };
            unsafe { PyLong_FromUnsignedLong(raw as u64) }
        }
        PY_MEMBER_T_SHORT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<i16>()) };
            unsafe { PyLong_FromLong(raw as i64) }
        }
        PY_MEMBER_T_USHORT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u16>()) };
            unsafe { PyLong_FromUnsignedLong(raw as u64) }
        }
        PY_MEMBER_T_INT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_int>()) };
            unsafe { PyLong_FromLong(raw as i64) }
        }
        PY_MEMBER_T_UINT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u32>()) };
            unsafe { PyLong_FromUnsignedLong(raw as u64) }
        }
        PY_MEMBER_T_LONG => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_long>()) };
            unsafe { PyLong_FromLong(raw as i64) }
        }
        PY_MEMBER_T_ULONG => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<c_ulong>()) };
            unsafe { PyLong_FromUnsignedLong(raw as u64) }
        }
        PY_MEMBER_T_PYSSIZET => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<isize>()) };
            unsafe { PyLong_FromSsize_t(raw) }
        }
        PY_MEMBER_T_FLOAT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<f32>()) };
            unsafe { PyFloat_FromDouble(raw as f64) }
        }
        PY_MEMBER_T_DOUBLE => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<f64>()) };
            unsafe { PyFloat_FromDouble(raw) }
        }
        PY_MEMBER_T_STRING => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*const c_char>()) };
            if raw.is_null() {
                none_ptr
            } else {
                unsafe { PyUnicode_FromString(raw) }
            }
        }
        PY_MEMBER_T_STRING_INPLACE => unsafe { PyUnicode_FromString(field_ptr.cast()) },
        PY_MEMBER_T_CHAR => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u8>()) };
            let text = [raw];
            unsafe { PyUnicode_FromStringAndSize(text.as_ptr().cast(), 1) }
        }
        PY_MEMBER_T_OBJECT => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*mut c_void>()) };
            if raw.is_null() {
                none_ptr
            } else {
                unsafe { Py_XIncRef(raw) };
                raw
            }
        }
        PY_MEMBER_T_OBJECT_EX => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*mut c_void>()) };
            if raw.is_null() {
                let member_name = ModuleCapiContext::member_attr_name(member_def);
                cpython_set_typed_error(
                    unsafe { PyExc_AttributeError },
                    format!("attribute '{member_name}' is not set"),
                );
                std::ptr::null_mut()
            } else {
                unsafe { Py_XIncRef(raw) };
                raw
            }
        }
        PY_MEMBER_T_LONGLONG => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<i64>()) };
            unsafe { PyLong_FromLongLong(raw) }
        }
        PY_MEMBER_T_ULONGLONG => {
            let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u64>()) };
            unsafe { PyLong_FromUnsignedLongLong(raw) }
        }
        PY_MEMBER_T_NONE => none_ptr,
        _ => {
            cpython_set_typed_error(unsafe { PyExc_SystemError }, "bad memberdescr type");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMember_SetOne(
    obj_addr: *mut c_char,
    member: *mut c_void,
    value: *mut c_void,
) -> c_int {
    if obj_addr.is_null() || member.is_null() {
        cpython_set_typed_error(unsafe { PyExc_SystemError }, "bad internal call");
        return -1;
    }
    let member_def = unsafe { &*member.cast::<CpythonMemberDef>() };
    if (member_def.flags & PY_MEMBER_RELATIVE_OFFSET) != 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyMember_SetOne used with Py_RELATIVE_OFFSET",
        );
        return -1;
    }
    if (member_def.flags & PY_MEMBER_READONLY) != 0 {
        cpython_set_typed_error(unsafe { PyExc_AttributeError }, "readonly attribute");
        return -1;
    }
    let field_ptr = match ModuleCapiContext::member_field_ptr(obj_addr.cast(), member_def) {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_typed_error(unsafe { PyExc_SystemError }, err);
            return -1;
        }
    };
    let member_name = ModuleCapiContext::member_attr_name(member_def);
    if value.is_null() {
        if member_def.member_type == PY_MEMBER_T_OBJECT_EX {
            let current = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*mut c_void>()) };
            if current.is_null() {
                cpython_set_typed_error(unsafe { PyExc_AttributeError }, member_name);
                return -1;
            }
        } else if member_def.member_type != PY_MEMBER_T_OBJECT {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "can't delete numeric/char attribute",
            );
            return -1;
        }
    }
    match member_def.member_type {
        PY_MEMBER_T_BOOL => {
            let Ok(py_value) = cpython_value_from_ptr(value) else {
                cpython_set_typed_error(
                    unsafe { PyExc_TypeError },
                    "attribute value type must be bool",
                );
                return -1;
            };
            let Value::Bool(flag) = py_value else {
                cpython_set_typed_error(
                    unsafe { PyExc_TypeError },
                    "attribute value type must be bool",
                );
                return -1;
            };
            unsafe {
                std::ptr::write_unaligned(field_ptr.cast::<c_char>(), if flag { 1 } else { 0 })
            };
            0
        }
        PY_MEMBER_T_BYTE => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<i8>(), raw as i8) };
            0
        }
        PY_MEMBER_T_UBYTE => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<u8>(), raw as u8) };
            0
        }
        PY_MEMBER_T_SHORT => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<i16>(), raw as i16) };
            0
        }
        PY_MEMBER_T_USHORT => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<u16>(), raw as u16) };
            0
        }
        PY_MEMBER_T_INT => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<c_int>(), raw as c_int) };
            0
        }
        PY_MEMBER_T_UINT => {
            let numeric_value = match cpython_value_from_ptr(value) {
                Ok(v) => v,
                Err(err) => {
                    cpython_set_error(err);
                    return -1;
                }
            };
            let raw = if let Ok(signed) = value_to_int(numeric_value.clone()) {
                signed as u32
            } else {
                let unsigned = unsafe { PyLong_AsUnsignedLong(value) };
                if !unsafe { PyErr_Occurred() }.is_null() {
                    return -1;
                }
                unsigned as u32
            };
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<u32>(), raw) };
            0
        }
        PY_MEMBER_T_LONG => {
            let raw = unsafe { PyLong_AsLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<c_long>(), raw as c_long) };
            0
        }
        PY_MEMBER_T_ULONG => {
            let numeric_value = match cpython_value_from_ptr(value) {
                Ok(v) => v,
                Err(err) => {
                    cpython_set_error(err);
                    return -1;
                }
            };
            let raw = if let Ok(signed) = value_to_int(numeric_value.clone()) {
                signed as c_ulong
            } else {
                let unsigned = unsafe { PyLong_AsUnsignedLong(value) };
                if !unsafe { PyErr_Occurred() }.is_null() {
                    return -1;
                }
                unsigned as c_ulong
            };
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<c_ulong>(), raw) };
            0
        }
        PY_MEMBER_T_PYSSIZET => {
            let raw = unsafe { PyLong_AsSsize_t(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<isize>(), raw) };
            0
        }
        PY_MEMBER_T_FLOAT => {
            let raw = unsafe { PyFloat_AsDouble(value) };
            if raw == -1.0 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<f32>(), raw as f32) };
            0
        }
        PY_MEMBER_T_DOUBLE => {
            let raw = unsafe { PyFloat_AsDouble(value) };
            if raw == -1.0 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<f64>(), raw) };
            0
        }
        PY_MEMBER_T_OBJECT | PY_MEMBER_T_OBJECT_EX => {
            let previous = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*mut c_void>()) };
            if !value.is_null() {
                unsafe { Py_XIncRef(value) };
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<*mut c_void>(), value) };
            if !previous.is_null() {
                unsafe { Py_XDecRef(previous) };
            }
            0
        }
        PY_MEMBER_T_CHAR => {
            let Ok(py_value) = cpython_value_from_ptr(value) else {
                unsafe { PyErr_BadArgument() };
                return -1;
            };
            let Value::Str(text) = py_value else {
                unsafe { PyErr_BadArgument() };
                return -1;
            };
            if text.as_bytes().len() != 1 {
                unsafe { PyErr_BadArgument() };
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<u8>(), text.as_bytes()[0]) };
            0
        }
        PY_MEMBER_T_STRING | PY_MEMBER_T_STRING_INPLACE => {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, "readonly attribute");
            -1
        }
        PY_MEMBER_T_LONGLONG => {
            let raw = unsafe { PyLong_AsLongLong(value) };
            if raw == -1 && !unsafe { PyErr_Occurred() }.is_null() {
                return -1;
            }
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<i64>(), raw) };
            0
        }
        PY_MEMBER_T_ULONGLONG => {
            let numeric_value = match cpython_value_from_ptr(value) {
                Ok(v) => v,
                Err(err) => {
                    cpython_set_error(err);
                    return -1;
                }
            };
            let raw = if let Ok(signed) = value_to_int(numeric_value.clone()) {
                signed as u64
            } else {
                let unsigned = unsafe { PyLong_AsUnsignedLongLong(value) };
                if !unsafe { PyErr_Occurred() }.is_null() {
                    return -1;
                }
                unsigned
            };
            unsafe { std::ptr::write_unaligned(field_ptr.cast::<u64>(), raw) };
            0
        }
        _ => {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                format!("bad memberdescr type for {member_name}"),
            );
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewGetSet(
    type_obj: *mut c_void,
    getset: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if type_obj.is_null() || getset.is_null() {
            context.set_error("bad internal call");
            return std::ptr::null_mut();
        }
        if !cpython_ptr_is_type_object(type_obj) {
            context.set_error("PyDescr_NewGetSet expected type object");
            return std::ptr::null_mut();
        }
        let owner_type = type_obj.cast::<CpythonTypeObject>();
        let getset_def = getset.cast::<CpythonGetSetDef>();
        // SAFETY: getset definition pointer is non-null and extension-owned.
        let getset_name_ptr = unsafe { (*getset_def).name };
        if getset_name_ptr.is_null() {
            context.set_error("PyDescr_NewGetSet expected getset name");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_descriptor_ptr(
            std::ptr::addr_of_mut!(PyGetSetDescr_Type),
            CpythonDescriptorKind::GetSet {
                owner_type,
                getset: getset_def,
            },
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyWrapper_New(
    descriptor: *mut c_void,
    self_obj: *mut c_void,
) -> *mut c_void {
    if descriptor.is_null() || self_obj.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        // SAFETY: self_obj points to a CPython-compatible object header.
        let object_type = unsafe {
            self_obj
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type)
                .unwrap_or(std::ptr::null_mut())
        };
        if object_type.is_null() {
            context.set_error("PyWrapper_New received object without type");
            return std::ptr::null_mut();
        }
        let is_type_object = cpython_is_type_object_ptr(self_obj);
        if let Some(bound) = context.resolve_descriptor_attr_ptr(
            descriptor,
            self_obj,
            object_type.cast(),
            is_type_object,
        ) {
            return bound;
        }

        let Some(descriptor_value) = context.cpython_value_from_ptr_or_proxy(descriptor) else {
            context.set_error("PyWrapper_New received unknown descriptor");
            return std::ptr::null_mut();
        };
        let Some(self_value) = context.cpython_value_from_ptr_or_proxy(self_obj) else {
            context.set_error("PyWrapper_New received unknown self object");
            return std::ptr::null_mut();
        };
        let getter = match cpython_getattr_in_context(context, descriptor_value, "__get__") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let owner_type_value = context
            .cpython_value_from_ptr_or_proxy(object_type.cast())
            .unwrap_or(Value::None);
        let bound = match cpython_call_internal_in_context(
            context,
            getter,
            vec![self_value, owner_type_value],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        context.alloc_cpython_ptr_for_value(bound)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

unsafe extern "C" fn cpython_cfunction_tp_call(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if callable.is_null() {
            context.set_error("cfunction call received null callable");
            return std::ptr::null_mut();
        }
        let raw = callable.cast::<CpythonCFunctionCompatObject>();
        // SAFETY: `callable` is a cfunction object allocated by this runtime.
        let method_def = unsafe { (*raw).m_ml };
        if method_def.is_null() {
            context.set_error("cfunction call missing method definition");
            return std::ptr::null_mut();
        }
        // SAFETY: cfunction object layout is stable for this context.
        let self_obj = unsafe { (*raw).m_self };
        // SAFETY: cfunction object layout is stable for this context.
        let class_obj = unsafe { (*raw).m_class };
        let positional = match cpython_positional_args_from_tuple_object(args) {
            Ok(values) => values,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let keyword_args = match cpython_keyword_args_from_dict_object(kwargs) {
            Ok(values) => values,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        cpython_invoke_method_from_values(
            context,
            method_def,
            self_obj,
            class_obj,
            positional,
            keyword_args,
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_cfunction_raw_object(
    context: &mut ModuleCapiContext,
    object: *mut c_void,
    api_name: &str,
) -> Option<*mut CpythonCFunctionCompatObject> {
    if object.is_null() {
        context.set_error(format!("{api_name} received null callable"));
        return None;
    }
    // SAFETY: `object` is a potential PyObject pointer; we only inspect head fields.
    let type_ptr = unsafe {
        object
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<c_void>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null()
        || unsafe {
            PyType_IsSubtype(
                type_ptr,
                std::ptr::addr_of_mut!(PyCFunction_Type).cast::<c_void>(),
            )
        } == 0
    {
        context.set_error("bad internal call");
        return None;
    }
    Some(object.cast::<CpythonCFunctionCompatObject>())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_GetFunction(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(raw) = cpython_cfunction_raw_object(context, object, "PyCFunction_GetFunction")
        else {
            return std::ptr::null_mut();
        };
        // SAFETY: raw object + method definition were validated above.
        unsafe {
            (*raw)
                .m_ml
                .as_ref()
                .and_then(|method| method.ml_meth)
                .map(|function| function as *mut c_void)
                .unwrap_or(std::ptr::null_mut())
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_GetSelf(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(raw) = cpython_cfunction_raw_object(context, object, "PyCFunction_GetSelf") else {
            return std::ptr::null_mut();
        };
        // SAFETY: raw object was validated above.
        unsafe { (*raw).m_self }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_GetFlags(object: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(raw) = cpython_cfunction_raw_object(context, object, "PyCFunction_GetFlags")
        else {
            return -1;
        };
        // SAFETY: raw object + method definition were validated above.
        unsafe {
            (*raw)
                .m_ml
                .as_ref()
                .map(|method| method.ml_flags)
                .unwrap_or(-1)
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_Call(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    unsafe { PyObject_Call(callable, args, kwargs) }
}

unsafe extern "C" fn cpython_cfunction_tp_getattro(
    object: *mut c_void,
    name: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if object.is_null() {
            context.set_error("cfunction getattr received null object");
            return std::ptr::null_mut();
        }
        let attr_name = match context.cpython_value_from_ptr(name) {
            Some(Value::Str(text)) => text,
            _ => {
                context.set_error("cfunction getattr expected string attribute name");
                return std::ptr::null_mut();
            }
        };
        let raw = object.cast::<CpythonCFunctionCompatObject>();
        // SAFETY: `object` is expected to be a cfunction compat object.
        let method_def = unsafe { (*raw).m_ml };
        if method_def.is_null() {
            context.set_error("cfunction getattr missing method definition");
            return std::ptr::null_mut();
        }
        // SAFETY: method definition is extension-provided and pointer-stable.
        let method_name = unsafe { c_name_to_string((*method_def).ml_name) }
            .unwrap_or_else(|_| "method".to_string());
        match attr_name.as_str() {
            "__name__" | "__qualname__" => {
                context.alloc_cpython_ptr_for_value(Value::Str(method_name))
            }
            "__module__" => {
                // SAFETY: cfunction object layout is stable for this context.
                let self_obj = unsafe { (*raw).m_self };
                // SAFETY: cfunction object layout is stable for this context.
                let module_obj = unsafe { (*raw).m_module };
                let module_name = context
                    .cpython_value_from_ptr(module_obj)
                    .or_else(|| context.cpython_value_from_ptr(self_obj))
                    .and_then(|value| match value {
                        Value::Str(text) => Some(text),
                        Value::Module(module_obj) => match &*module_obj.kind() {
                            Object::Module(module_data) => Some(module_data.name.clone()),
                            _ => None,
                        },
                        _ => None,
                    })
                    .unwrap_or_else(|| "builtins".to_string());
                context.alloc_cpython_ptr_for_value(Value::Str(module_name))
            }
            "__doc__" => {
                // SAFETY: method definition is extension-provided and pointer-stable.
                let doc_ptr = unsafe { (*method_def).ml_doc };
                if doc_ptr.is_null() {
                    context.alloc_cpython_ptr_for_value(Value::None)
                } else {
                    // SAFETY: doc string is expected to be NUL-terminated by C-API contract.
                    let doc = unsafe { CStr::from_ptr(doc_ptr) }
                        .to_str()
                        .map(|text| text.to_string())
                        .unwrap_or_else(|_| String::new());
                    context.alloc_cpython_ptr_for_value(Value::Str(doc))
                }
            }
            _ => {
                context.set_error(format!(
                    "AttributeError: 'builtin_function_or_method' object has no attribute '{}'",
                    attr_name
                ));
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_New(
    start: *mut c_void,
    stop: *mut c_void,
    step: *mut c_void,
) -> *mut c_void {
    let start = if start.is_null() {
        Value::None
    } else {
        match cpython_value_from_ptr(start) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    let stop = if stop.is_null() {
        Value::None
    } else {
        match cpython_value_from_ptr(stop) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    let step = if step.is_null() {
        Value::None
    } else {
        match cpython_value_from_ptr(step) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    match cpython_call_builtin(BuiltinFunction::Slice, vec![start, stop, step]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_Unpack(
    slice: *mut c_void,
    start: *mut isize,
    stop: *mut isize,
    step: *mut isize,
) -> i32 {
    if start.is_null() || stop.is_null() || step.is_null() {
        cpython_set_error("PySlice_Unpack received null output pointer");
        return -1;
    }
    let Value::Slice(slice_value) = (match cpython_value_from_ptr(slice) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    }) else {
        cpython_set_error("PySlice_Unpack expected slice object");
        return -1;
    };
    unsafe {
        *start = slice_value.lower.unwrap_or(0) as isize;
        *stop = slice_value.upper.unwrap_or(isize::MAX as i64) as isize;
        *step = slice_value.step.unwrap_or(1) as isize;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_AdjustIndices(
    length: isize,
    start: *mut isize,
    stop: *mut isize,
    step: isize,
) -> isize {
    if start.is_null() || stop.is_null() || step == 0 {
        return 0;
    }
    // SAFETY: caller provided valid pointers.
    let mut s = unsafe { *start };
    // SAFETY: caller provided valid pointers.
    let mut e = unsafe { *stop };
    if s < 0 {
        s += length;
        if s < 0 {
            s = if step < 0 { -1 } else { 0 };
        }
    } else if s >= length {
        s = if step < 0 { length - 1 } else { length };
    }
    if e < 0 {
        e += length;
        if e < 0 {
            e = if step < 0 { -1 } else { 0 };
        }
    } else if e >= length {
        e = if step < 0 { length - 1 } else { length };
    }
    unsafe {
        *start = s;
        *stop = e;
    }
    if step < 0 {
        if e < s { (s - e - 1) / (-step) + 1 } else { 0 }
    } else if s < e {
        (e - s - 1) / step + 1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_GetIndices(
    slice: *mut c_void,
    length: isize,
    start: *mut isize,
    stop: *mut isize,
    step: *mut isize,
) -> i32 {
    if start.is_null() || stop.is_null() || step.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let Value::Slice(slice_value) = (match cpython_value_from_ptr(slice) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    }) else {
        cpython_set_error("PySlice_GetIndices expected slice object");
        return -1;
    };
    let raw_step = slice_value.step.unwrap_or(1) as isize;
    if raw_step == 0 {
        cpython_set_typed_error(unsafe { PyExc_ValueError }, "slice step cannot be zero");
        return -1;
    }
    let mut raw_start = match slice_value.lower {
        Some(value) => value as isize,
        None => {
            if raw_step < 0 {
                length.saturating_sub(1)
            } else {
                0
            }
        }
    };
    if slice_value.lower.is_some() && raw_start < 0 {
        raw_start += length;
    }
    let mut raw_stop = match slice_value.upper {
        Some(value) => value as isize,
        None => {
            if raw_step < 0 {
                -1
            } else {
                length
            }
        }
    };
    if slice_value.upper.is_some() && raw_stop < 0 {
        raw_stop += length;
    }
    if raw_stop > length || raw_start >= length {
        cpython_set_typed_error(unsafe { PyExc_ValueError }, "slice index out of range");
        return -1;
    }
    unsafe {
        *start = raw_start;
        *stop = raw_stop;
        *step = raw_step;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_GetIndicesEx(
    slice: *mut c_void,
    length: isize,
    start: *mut isize,
    stop: *mut isize,
    step: *mut isize,
    slice_length: *mut isize,
) -> i32 {
    if slice_length.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if unsafe { PySlice_Unpack(slice, start, stop, step) } < 0 {
        return -1;
    }
    // SAFETY: pointers validated by PySlice_Unpack and checked above.
    let adjusted = unsafe { PySlice_AdjustIndices(length, start, stop, *step) };
    unsafe {
        *slice_length = adjusted;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_GetObject(name: *const c_char) -> *mut c_void {
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySys_GetObject missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(sys_module) = vm.modules.get("sys") else {
            context.set_error("PySys_GetObject could not find sys module");
            return std::ptr::null_mut();
        };
        let Object::Module(data) = &*sys_module.kind() else {
            context.set_error("PySys_GetObject sys module invalid");
            return std::ptr::null_mut();
        };
        let Some(value) = data.globals.get(&name) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(value.clone())
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_sys_module_obj(context: &mut ModuleCapiContext) -> Result<ObjRef, String> {
    if context.vm.is_null() {
        return Err("missing VM context".to_string());
    }
    // SAFETY: VM pointer is valid for active C-API context.
    let vm = unsafe { &mut *context.vm };
    vm.modules
        .get("sys")
        .cloned()
        .ok_or_else(|| "could not find sys module".to_string())
}

fn cpython_sys_warnoptions_list(context: &mut ModuleCapiContext) -> Result<ObjRef, String> {
    let sys_module = cpython_sys_module_obj(context)?;
    let Object::Module(sys_data) = &mut *sys_module.kind_mut() else {
        return Err("sys module object is invalid".to_string());
    };
    if let Some(Value::List(existing)) = sys_data.globals.get("warnoptions") {
        return Ok(existing.clone());
    }
    let list_obj = if context.vm.is_null() {
        return Err("missing VM context".to_string());
    } else {
        // SAFETY: VM pointer is valid for active C-API context.
        let vm = unsafe { &mut *context.vm };
        match vm.heap.alloc_list(Vec::new()) {
            Value::List(list_obj) => list_obj,
            _ => return Err("failed to allocate warnoptions list".to_string()),
        }
    };
    sys_data
        .globals
        .insert("warnoptions".to_string(), Value::List(list_obj.clone()));
    Ok(list_obj)
}

fn cpython_sys_xoptions_dict(context: &mut ModuleCapiContext) -> Result<ObjRef, String> {
    let sys_module = cpython_sys_module_obj(context)?;
    let Object::Module(sys_data) = &mut *sys_module.kind_mut() else {
        return Err("sys module object is invalid".to_string());
    };
    if let Some(Value::Dict(existing)) = sys_data.globals.get("_xoptions") {
        return Ok(existing.clone());
    }
    let dict_obj = if context.vm.is_null() {
        return Err("missing VM context".to_string());
    } else {
        // SAFETY: VM pointer is valid for active C-API context.
        let vm = unsafe { &mut *context.vm };
        match vm.heap.alloc_dict(Vec::new()) {
            Value::Dict(dict_obj) => dict_obj,
            _ => return Err("failed to allocate _xoptions dict".to_string()),
        }
    };
    sys_data
        .globals
        .insert("_xoptions".to_string(), Value::Dict(dict_obj.clone()));
    Ok(dict_obj)
}

fn cpython_sys_add_warn_option(
    context: &mut ModuleCapiContext,
    option: String,
) -> Result<(), String> {
    let warnoptions = cpython_sys_warnoptions_list(context)?;
    let Object::List(values) = &mut *warnoptions.kind_mut() else {
        return Err("warnoptions is not a list".to_string());
    };
    values.push(Value::Str(option));
    Ok(())
}

fn cpython_sys_set_global(
    context: &mut ModuleCapiContext,
    name: &str,
    value: Value,
) -> Result<(), String> {
    let sys_module = cpython_sys_module_obj(context)?;
    {
        let Object::Module(sys_data) = &mut *sys_module.kind_mut() else {
            return Err("sys module object is invalid".to_string());
        };
        sys_data.globals.insert(name.to_string(), value.clone());
    }
    context
        .sync_module_dict_set(&sys_module, name, &value)
        .map_err(|err| format!("failed syncing sys.{name}: {err}"))?;
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_SetObject(name: *const c_char, value: *mut c_void) -> i32 {
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    with_active_cpython_context_mut(|context| {
        let sys_module = match cpython_sys_module_obj(context) {
            Ok(module) => module,
            Err(err) => {
                context.set_error(format!("PySys_SetObject {err}"));
                return -1;
            }
        };
        if value.is_null() {
            let Object::Module(sys_data) = &mut *sys_module.kind_mut() else {
                context.set_error("PySys_SetObject sys module invalid");
                return -1;
            };
            sys_data.globals.remove(&name);
            if let Some(dict_handle) = context.module_dict_handle_for_module(&sys_module)
                && let Some(slot) = context.objects.get(&dict_handle)
                && let Value::Dict(dict_obj) = &slot.value
            {
                let _ = dict_remove_value(dict_obj, &Value::Str(name.clone()));
            }
            return 0;
        }
        let Some(mapped) = context.cpython_value_from_ptr_or_proxy(value) else {
            context.set_error("PySys_SetObject received unknown value pointer");
            return -1;
        };
        let Object::Module(sys_data) = &mut *sys_module.kind_mut() else {
            context.set_error("PySys_SetObject sys module invalid");
            return -1;
        };
        sys_data.globals.insert(name.clone(), mapped.clone());
        if let Err(err) = context.sync_module_dict_set(&sys_module, &name, &mapped) {
            context.set_error(format!(
                "PySys_SetObject failed syncing module dict entry '{}': {}",
                name, err
            ));
            return -1;
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_GetXOptions() -> *mut c_void {
    with_active_cpython_context_mut(|context| match cpython_sys_xoptions_dict(context) {
        Ok(dict_obj) => context.alloc_cpython_ptr_for_value(Value::Dict(dict_obj)),
        Err(err) => {
            context.set_error(format!("PySys_GetXOptions {err}"));
            std::ptr::null_mut()
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_AddXOption(option: *const Cwchar) {
    let option = match unsafe { c_wide_name_to_string(option) } {
        Ok(option) => option,
        Err(err) => {
            cpython_set_error(err);
            return;
        }
    };
    let _ = with_active_cpython_context_mut(|context| {
        let xoptions = match cpython_sys_xoptions_dict(context) {
            Ok(dict_obj) => dict_obj,
            Err(err) => {
                context.set_error(format!("PySys_AddXOption {err}"));
                return;
            }
        };
        let (key, value) = if let Some(eq) = option.find('=') {
            (
                option[..eq].to_string(),
                Value::Str(option[eq + 1..].to_string()),
            )
        } else {
            (option, Value::Bool(true))
        };
        let _ = dict_set_value_checked(&xoptions, Value::Str(key), value).map_err(|err| {
            context.set_error(format!("PySys_AddXOption {}", err.message));
        });
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_HasWarnOptions() -> i32 {
    with_active_cpython_context_mut(|context| match cpython_sys_warnoptions_list(context) {
        Ok(list_obj) => match &*list_obj.kind() {
            Object::List(values) => i32::from(!values.is_empty()),
            _ => 0,
        },
        Err(err) => {
            context.set_error(format!("PySys_HasWarnOptions {err}"));
            0
        }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_ResetWarnOptions() {
    let _ = with_active_cpython_context_mut(|context| {
        let warnoptions = match cpython_sys_warnoptions_list(context) {
            Ok(warnoptions) => warnoptions,
            Err(err) => {
                context.set_error(format!("PySys_ResetWarnOptions {err}"));
                return;
            }
        };
        let Object::List(values) = &mut *warnoptions.kind_mut() else {
            context.set_error("PySys_ResetWarnOptions warnoptions is not a list");
            return;
        };
        values.clear();
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_AddWarnOption(option: *const Cwchar) {
    let option = match unsafe { c_wide_name_to_string(option) } {
        Ok(option) => option,
        Err(err) => {
            cpython_set_error(err);
            return;
        }
    };
    let _ = with_active_cpython_context_mut(|context| {
        if let Err(err) = cpython_sys_add_warn_option(context, option) {
            context.set_error(format!("PySys_AddWarnOption {err}"));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_AddWarnOptionUnicode(option: *const Cwchar) {
    unsafe { PySys_AddWarnOption(option) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_sys_write_stdout(text: *const c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: caller guarantees a valid NUL-terminated string.
    let line = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    print!("{line}");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_sys_write_stderr(text: *const c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: caller guarantees a valid NUL-terminated string.
    let line = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    eprint!("{line}");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_AuditTuple(event: *const c_char, _args: *mut c_void) -> i32 {
    if event.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PySys_AuditTuple requires event name",
        );
        return -1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_sys_audit_noargs(event: *const c_char) -> i32 {
    unsafe { PySys_AuditTuple(event, std::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_SetPath(path: *const Cwchar) {
    let path_text = match unsafe { c_wide_name_to_string(path) } {
        Ok(path) => path,
        Err(err) => {
            cpython_set_error(err);
            return;
        }
    };
    #[cfg(windows)]
    let delimiter = ';';
    #[cfg(not(windows))]
    let delimiter = ':';
    let entries: Vec<Value> = if path_text.is_empty() {
        Vec::new()
    } else {
        path_text
            .split(delimiter)
            .map(|entry| Value::Str(entry.to_string()))
            .collect()
    };
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySys_SetPath missing VM context");
            return;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let path_list = unsafe { (&mut *context.vm).heap.alloc_list(entries) };
        if let Err(err) = cpython_sys_set_global(context, "path", path_list) {
            context.set_error(format!("PySys_SetPath {err}"));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_SetArgvEx(argc: i32, argv: *mut *mut Cwchar, updatepath: i32) {
    if argc < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PySys_SetArgvEx argc must be >= 0",
        );
        return;
    }
    let mut argv_values: Vec<Value> = Vec::new();
    for idx in 0..(argc as usize) {
        let arg_ptr = if argv.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: caller guarantees `argv` has `argc` entries when non-null.
            unsafe { *argv.add(idx) }
        };
        let arg = if arg_ptr.is_null() {
            String::new()
        } else {
            match unsafe { c_wide_name_to_string(arg_ptr) } {
                Ok(arg) => arg,
                Err(err) => {
                    cpython_set_error(format!("PySys_SetArgvEx invalid argument: {err}"));
                    return;
                }
            }
        };
        argv_values.push(Value::Str(arg));
    }
    let argv_strings: Vec<String> = argv_values
        .iter()
        .filter_map(|value| match value {
            Value::Str(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    cpython_store_argv_wide(&argv_strings);

    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySys_SetArgvEx missing VM context");
            return;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let argv_list = vm.heap.alloc_list(argv_values.clone());
        if let Err(err) = cpython_sys_set_global(context, "argv", argv_list) {
            context.set_error(format!("PySys_SetArgvEx {err}"));
            return;
        }
        if updatepath == 0 {
            return;
        }
        let mut path_values: Vec<Value> = Vec::new();
        if let Some(Value::Str(program_path)) = argv_values.first() {
            let first_path = Path::new(program_path).parent().map_or_else(
                || "".to_string(),
                |parent| parent.to_string_lossy().to_string(),
            );
            path_values.push(Value::Str(first_path));
        }
        if let Ok(sys_module) = cpython_sys_module_obj(context)
            && let Object::Module(sys_data) = &*sys_module.kind()
            && let Some(Value::List(existing_list)) = sys_data.globals.get("path")
            && let Object::List(items) = &*existing_list.kind()
        {
            path_values.extend(items.iter().cloned());
        }
        let path_list = vm.heap.alloc_list(path_values);
        if let Err(err) = cpython_sys_set_global(context, "path", path_list) {
            context.set_error(format!("PySys_SetArgvEx {err}"));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_SetArgv(argc: i32, argv: *mut *mut Cwchar) {
    unsafe { PySys_SetArgvEx(argc, argv, 1) }
}

fn cpython_thread_lock_is_known(ptr: usize) -> bool {
    cpython_thread_lock_registry()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&ptr))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_init_thread() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_start_new_thread(
    function: Option<unsafe extern "C" fn(*mut c_void)>,
    arg: *mut c_void,
) -> c_ulong {
    let Some(function) = function else {
        return c_ulong::MAX;
    };
    let stack_size = CPYTHON_THREAD_STACK_SIZE.load(Ordering::Relaxed);
    let arg_bits = arg as usize;
    let ident = CPYTHON_THREAD_NEXT_IDENT.fetch_add(1, Ordering::Relaxed);
    let mut builder = std::thread::Builder::new();
    if stack_size > 0 {
        builder = builder.stack_size(stack_size);
    }
    match builder.spawn(move || {
        // SAFETY: C-API contract provides callable + argument pointer.
        unsafe { function(arg_bits as *mut c_void) };
    }) {
        Ok(handle) => {
            std::mem::drop(handle);
            ident as c_ulong
        }
        Err(_) => c_ulong::MAX,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_exit_thread() {
    loop {
        std::thread::park();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_get_thread_ident() -> c_ulong {
    vm_os_thread_ident() as c_ulong
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_get_thread_native_id() -> c_ulong {
    vm_os_thread_ident() as c_ulong
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_allocate_lock() -> *mut c_void {
    let raw = Box::into_raw(Box::new(CpythonThreadLock {
        state: Mutex::new(false),
        condvar: Condvar::new(),
    })) as usize;
    if let Ok(mut set) = cpython_thread_lock_registry().lock() {
        set.insert(raw);
    }
    raw as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_free_lock(lock: *mut c_void) {
    if lock.is_null() {
        return;
    }
    let ptr = lock as usize;
    let removed = cpython_thread_lock_registry()
        .lock()
        .ok()
        .is_some_and(|mut set| set.remove(&ptr));
    if removed {
        // SAFETY: pointer was produced by Box::into_raw in PyThread_allocate_lock and removed once.
        unsafe {
            drop(Box::from_raw(lock.cast::<CpythonThreadLock>()));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_acquire_lock(lock: *mut c_void, waitflag: c_int) -> c_int {
    let timeout = if waitflag != 0 { -1 } else { 0 };
    let status = unsafe { PyThread_acquire_lock_timed(lock, timeout, 0) };
    if status == 1 { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_acquire_lock_timed(
    lock: *mut c_void,
    microseconds: i64,
    _intr_flag: c_int,
) -> c_int {
    if lock.is_null() {
        return 0;
    }
    let lock_ptr = lock as usize;
    if !cpython_thread_lock_is_known(lock_ptr) {
        return 0;
    }
    // SAFETY: lock pointer validity is guarded by registry membership.
    let lock_ref = unsafe { &*lock.cast::<CpythonThreadLock>() };
    let mut state = match lock_ref.state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !*state {
        *state = true;
        return 1;
    }
    if microseconds == 0 {
        return 0;
    }
    if microseconds < 0 {
        while *state {
            state = match lock_ref.condvar.wait(state) {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
        *state = true;
        return 1;
    }
    let timeout = Duration::from_micros(microseconds as u64);
    let start = Instant::now();
    while *state {
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            return 0;
        }
        let remaining = timeout - elapsed;
        let result = lock_ref.condvar.wait_timeout(state, remaining);
        let (new_state, wait_outcome) = match result {
            Ok(value) => value,
            Err(poisoned) => poisoned.into_inner(),
        };
        state = new_state;
        if wait_outcome.timed_out() && *state {
            return 0;
        }
    }
    *state = true;
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_release_lock(lock: *mut c_void) {
    if lock.is_null() {
        return;
    }
    let lock_ptr = lock as usize;
    if !cpython_thread_lock_is_known(lock_ptr) {
        return;
    }
    // SAFETY: lock pointer validity is guarded by registry membership.
    let lock_ref = unsafe { &*lock.cast::<CpythonThreadLock>() };
    let mut state = match lock_ref.state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if *state {
        *state = false;
        lock_ref.condvar.notify_one();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_get_stacksize() -> usize {
    CPYTHON_THREAD_STACK_SIZE.load(Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_set_stacksize(size: usize) -> c_int {
    if size == 0 {
        CPYTHON_THREAD_STACK_SIZE.store(0, Ordering::Relaxed);
        return 0;
    }
    if size < 32 * 1024 {
        return -1;
    }
    CPYTHON_THREAD_STACK_SIZE.store(size, Ordering::Relaxed);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_GetInfo() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyThread_GetInfo missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let info = vm.heap.alloc_tuple(vec![
            Value::Str("pyrs".to_string()),
            Value::Str("mutex+cond".to_string()),
            Value::None,
        ]);
        context.alloc_cpython_ptr_for_value(info)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_create_key() -> c_int {
    let raw = CPYTHON_THREAD_TLS_NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    if raw > c_int::MAX as usize {
        return -1;
    }
    if let Ok(mut set) = cpython_thread_tls_key_registry().lock() {
        set.insert(raw);
    }
    raw as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_delete_key(key: c_int) {
    if key <= 0 {
        return;
    }
    let key_id = key as usize;
    if let Ok(mut set) = cpython_thread_tls_key_registry().lock() {
        set.remove(&key_id);
    }
    if let Ok(mut map) = cpython_thread_tls_values().lock() {
        map.retain(|(_, stored_key), _| *stored_key != key_id);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_set_key_value(key: c_int, value: *mut c_void) -> c_int {
    if key <= 0 {
        return -1;
    }
    let key_id = key as usize;
    let is_known = cpython_thread_tls_key_registry()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&key_id));
    if !is_known {
        return -1;
    }
    let thread_id = cpython_current_thread_ident_u64();
    if let Ok(mut map) = cpython_thread_tls_values().lock() {
        map.insert((thread_id, key_id), value as usize);
        0
    } else {
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_get_key_value(key: c_int) -> *mut c_void {
    if key <= 0 {
        return std::ptr::null_mut();
    }
    let key_id = key as usize;
    let thread_id = cpython_current_thread_ident_u64();
    cpython_thread_tls_values()
        .lock()
        .ok()
        .and_then(|map| map.get(&(thread_id, key_id)).copied())
        .unwrap_or(0) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_delete_key_value(key: c_int) {
    if key <= 0 {
        return;
    }
    let key_id = key as usize;
    let thread_id = cpython_current_thread_ident_u64();
    if let Ok(mut map) = cpython_thread_tls_values().lock() {
        map.remove(&(thread_id, key_id));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_ReInitTLS() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_alloc() -> *mut c_void {
    Box::into_raw(Box::new(CpythonThreadTss {
        initialized: 0,
        key: 0,
    })) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_free(key: *mut c_void) {
    if key.is_null() {
        return;
    }
    unsafe { PyThread_tss_delete(key) };
    // SAFETY: pointer was allocated by PyThread_tss_alloc.
    unsafe {
        drop(Box::from_raw(key.cast::<CpythonThreadTss>()));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_is_created(key: *mut c_void) -> c_int {
    if key.is_null() {
        return 0;
    }
    // SAFETY: caller provided pointer is expected to reference Py_tss_t compatible storage.
    let key_ref = unsafe { &*key.cast::<CpythonThreadTss>() };
    if key_ref.initialized != 0 { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_create(key: *mut c_void) -> c_int {
    if key.is_null() {
        return -1;
    }
    // SAFETY: caller provided pointer is expected to reference Py_tss_t compatible storage.
    let key_ref = unsafe { &mut *key.cast::<CpythonThreadTss>() };
    if key_ref.initialized != 0 {
        return 0;
    }
    let key_id = CPYTHON_THREAD_TLS_NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut set) = cpython_thread_tss_registry().lock() {
        set.insert(key_id);
    }
    key_ref.key = key_id;
    key_ref.initialized = 1;
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_delete(key: *mut c_void) {
    if key.is_null() {
        return;
    }
    // SAFETY: caller provided pointer is expected to reference Py_tss_t compatible storage.
    let key_ref = unsafe { &mut *key.cast::<CpythonThreadTss>() };
    if key_ref.initialized == 0 {
        return;
    }
    let key_id = key_ref.key;
    if let Ok(mut set) = cpython_thread_tss_registry().lock() {
        set.remove(&key_id);
    }
    if let Ok(mut map) = cpython_thread_tss_values().lock() {
        map.retain(|(_, stored_key), _| *stored_key != key_id);
    }
    key_ref.key = 0;
    key_ref.initialized = 0;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_set(key: *mut c_void, value: *mut c_void) -> c_int {
    if key.is_null() {
        return -1;
    }
    // SAFETY: caller provided pointer is expected to reference Py_tss_t compatible storage.
    let key_ref = unsafe { &*key.cast::<CpythonThreadTss>() };
    if key_ref.initialized == 0 {
        return -1;
    }
    let key_id = key_ref.key;
    let is_known = cpython_thread_tss_registry()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&key_id));
    if !is_known {
        return -1;
    }
    let thread_id = cpython_current_thread_ident_u64();
    if let Ok(mut map) = cpython_thread_tss_values().lock() {
        map.insert((thread_id, key_id), value as usize);
        0
    } else {
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_get(key: *mut c_void) -> *mut c_void {
    if key.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller provided pointer is expected to reference Py_tss_t compatible storage.
    let key_ref = unsafe { &*key.cast::<CpythonThreadTss>() };
    if key_ref.initialized == 0 {
        return std::ptr::null_mut();
    }
    let thread_id = cpython_current_thread_ident_u64();
    cpython_thread_tss_values()
        .lock()
        .ok()
        .and_then(|map| map.get(&(thread_id, key_ref.key)).copied())
        .unwrap_or(0) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetString(_exception: *mut c_void, message: *const c_char) {
    match unsafe { c_name_to_string(message) } {
        Ok(message) => {
            if std::env::var_os("PYRS_TRACE_NUMPY_DTYPE").is_some() && message.contains("data type")
            {
                eprintln!(
                    "[cpy-dtype] PyErr_SetString exc={:p} msg={} bt={:?}",
                    _exception,
                    message,
                    Backtrace::force_capture()
                );
            }
            if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some()
                && message.starts_with("cannot add indexed loop to ufunc")
            {
                let _ = with_active_cpython_context_mut(|context| {
                    if let Some(previous) = context.last_error.as_ref() {
                        eprintln!("[cpy-err-prev] {previous}");
                    } else {
                        eprintln!("[cpy-err-prev] <none>");
                    }
                });
            }
            if std::env::var_os("PYRS_TRACE_DOCSTRING_ERRORS").is_some()
                && message == "Cannot set a docstring for that object"
            {
                eprintln!(
                    "[cpy-doc-error] exc={:p} message={} bt={:?}",
                    _exception,
                    message,
                    Backtrace::force_capture()
                );
            }
            if std::env::var_os("PYRS_TRACE_NUMPY_PICKLE_FAIL").is_some()
                && message.starts_with("Unable to initialize pickling for ")
            {
                eprintln!(
                    "[numpy-pickle-fail] from-PyErr_SetString message={} bt={:?}",
                    message,
                    Backtrace::force_capture()
                );
            }
            let _ = with_active_cpython_context_mut(|context| {
                let ptype = if _exception.is_null() {
                    unsafe { PyExc_RuntimeError }
                } else {
                    _exception
                };
                context.set_error_state(ptype, std::ptr::null_mut(), std::ptr::null_mut(), message);
            })
            .map_err(|err| {
                cpython_set_error(err);
            });
        }
        Err(err) => cpython_set_error(format!("PyErr_SetString invalid message: {err}")),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_NewException(
    name: *const c_char,
    mut base: *mut c_void,
    mut dict: *mut c_void,
) -> *mut c_void {
    if name.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let name_text = match unsafe { c_name_to_string(name) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(format!("PyErr_NewException invalid name: {err}"));
            return std::ptr::null_mut();
        }
    };
    let Some((module_name, class_name)) = cpython_exception_name_parts(&name_text) else {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyErr_NewException: name must be module.class",
        );
        return std::ptr::null_mut();
    };
    if base.is_null() {
        base = unsafe { PyExc_Exception };
    }

    let mut mydict: *mut c_void = std::ptr::null_mut();
    let mut modulename_obj: *mut c_void = std::ptr::null_mut();
    let module_key = unsafe { PyUnicode_FromString(c"__module__".as_ptr()) };
    if module_key.is_null() {
        return std::ptr::null_mut();
    }

    if dict.is_null() {
        dict = unsafe { PyDict_New() };
        if dict.is_null() {
            unsafe { Py_DecRef(module_key) };
            return std::ptr::null_mut();
        }
        mydict = dict;
    }

    let mut contains_module = unsafe { PyDict_Contains(dict, module_key) };
    if contains_module < 0 {
        unsafe {
            Py_DecRef(module_key);
            Py_XDecRef(mydict);
        }
        return std::ptr::null_mut();
    }
    if contains_module == 0 {
        modulename_obj = unsafe {
            PyUnicode_FromStringAndSize(module_name.as_ptr().cast(), module_name.len() as isize)
        };
        if modulename_obj.is_null() {
            unsafe {
                Py_DecRef(module_key);
                Py_XDecRef(mydict);
            }
            return std::ptr::null_mut();
        }
        if unsafe { PyDict_SetItem(dict, module_key, modulename_obj) } != 0 {
            unsafe {
                Py_DecRef(module_key);
                Py_XDecRef(modulename_obj);
                Py_XDecRef(mydict);
            }
            return std::ptr::null_mut();
        }
        contains_module = 1;
    }
    debug_assert!(contains_module == 1);

    let result = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return Err("missing VM context for PyErr_NewException".to_string());
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let base_value = context
            .cpython_value_from_ptr_or_proxy(base)
            .ok_or_else(|| "PyErr_NewException received invalid base object".to_string())?;
        let bases = match base_value {
            Value::Tuple(_) | Value::List(_) => base_value,
            other => vm.heap.alloc_tuple(vec![other]),
        };
        let namespace = context
            .cpython_value_from_ptr_or_proxy(dict)
            .ok_or_else(|| "PyErr_NewException received invalid dict object".to_string())?;
        let class_value = match vm.call_internal(
            Value::Builtin(BuiltinFunction::Type),
            vec![Value::Str(class_name.to_string()), bases, namespace],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(value)) => value,
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                return Err(vm
                    .runtime_error_from_active_exception("PyErr_NewException failed")
                    .message);
            }
            Err(err) => return Err(err.message),
        };
        Ok(context.alloc_cpython_ptr_for_value(class_value))
    })
    .unwrap_or_else(|err| Err(err.to_string()));

    let result = match result {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    };
    unsafe {
        Py_DecRef(module_key);
        Py_XDecRef(modulename_obj);
        Py_XDecRef(mydict);
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_NewExceptionWithDoc(
    name: *const c_char,
    doc: *const c_char,
    base: *mut c_void,
    mut dict: *mut c_void,
) -> *mut c_void {
    let mut mydict: *mut c_void = std::ptr::null_mut();
    if dict.is_null() {
        dict = unsafe { PyDict_New() };
        if dict.is_null() {
            return std::ptr::null_mut();
        }
        mydict = dict;
    }

    if !doc.is_null() {
        let doc_obj = unsafe { PyUnicode_FromString(doc) };
        if doc_obj.is_null() {
            unsafe { Py_XDecRef(mydict) };
            return std::ptr::null_mut();
        }
        let status = unsafe { PyDict_SetItemString(dict, c"__doc__".as_ptr(), doc_obj) };
        unsafe { Py_DecRef(doc_obj) };
        if status != 0 {
            unsafe { Py_XDecRef(mydict) };
            return std::ptr::null_mut();
        }
    }

    let result = unsafe { PyErr_NewException(name, base, dict) };
    unsafe { Py_XDecRef(mydict) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyExceptionClass_Name(exception_class: *mut c_void) -> *const c_char {
    match with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr_or_proxy(exception_class) else {
            context.set_error("PyExceptionClass_Name received unknown object pointer");
            return std::ptr::null();
        };
        let name = match value {
            Value::ExceptionType(name) => name,
            Value::Class(class_obj) => match &*class_obj.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => {
                    context.set_error("PyExceptionClass_Name expected exception class object");
                    return std::ptr::null();
                }
            },
            _ => {
                context.set_error("PyExceptionClass_Name expected exception class");
                return std::ptr::null();
            }
        };
        context
            .scratch_c_string_ptr(&name)
            .unwrap_or(std::ptr::null())
    }) {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Occurred() -> *mut c_void {
    match with_active_cpython_context_mut(|context| {
        let ptr = context
            .current_error
            .as_ref()
            .map_or(std::ptr::null_mut(), |state| state.ptype);
        if std::env::var_os("PYRS_TRACE_PYERR_OCCURRED").is_some() && !ptr.is_null() {
            let active = ACTIVE_CPYTHON_INIT_CONTEXT.with(|cell| cell.get());
            eprintln!(
                "[cpy-err-occurred] active_ctx={:p} ctx={:p} ptype={:p} last_error={:?}",
                active, context as *mut ModuleCapiContext, ptr, context.last_error
            );
        }
        ptr
    }) {
        Ok(ptr) => ptr,
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Clear() {
    let _ = with_active_cpython_context_mut(|context| {
        if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() && context.last_error.is_some() {
            if let Some(previous) = context.last_error.as_ref() {
                eprintln!("[cpy-err-clear] clearing: {previous}");
            }
        }
        context.clear_error();
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_ExceptionMatches(_exception: *mut c_void) -> i32 {
    let occurred = unsafe { PyErr_Occurred() };
    if occurred.is_null() {
        return 0;
    }
    unsafe { PyErr_GivenExceptionMatches(occurred, _exception) }
}

fn cpython_ptr_is_type_object(ptr: *mut c_void) -> bool {
    cpython_is_type_object_ptr(ptr)
}

fn cpython_probable_c_string_ptr(ptr: *const c_char) -> bool {
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if ptr.is_null() {
        return false;
    }
    let addr = ptr as usize;
    addr >= MIN_VALID_PTR && addr % std::mem::align_of::<usize>() == 0
}

fn cpython_safe_type_name(type_ptr: *mut CpythonTypeObject) -> Option<String> {
    if type_ptr.is_null() {
        return None;
    }
    // SAFETY: caller provides a candidate type pointer; this function performs
    // conservative pointer checks before touching foreign string memory.
    unsafe {
        let ty = type_ptr.as_ref()?;
        if !cpython_probable_c_string_ptr(ty.tp_name) {
            return None;
        }
        c_name_to_string(ty.tp_name).ok()
    }
}

fn cpython_safe_object_type_name(object: *mut c_void) -> Option<String> {
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if object.is_null() {
        return None;
    }
    let addr = object as usize;
    if addr < MIN_VALID_PTR || addr % std::mem::align_of::<usize>() != 0 {
        return None;
    }
    // SAFETY: pointer passes conservative shape checks; read-only access to object head.
    unsafe {
        let head = object.cast::<CpythonObjectHead>().as_ref()?;
        cpython_safe_type_name(head.ob_type.cast::<CpythonTypeObject>())
    }
}

fn cpython_exception_type_ptr(ptr: *mut c_void) -> *mut c_void {
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    if cpython_ptr_is_type_object(ptr) {
        return ptr;
    }
    // SAFETY: pointer is inspected as a CPython object header.
    unsafe {
        ptr.cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type)
            .unwrap_or(std::ptr::null_mut())
    }
}

fn cpython_exception_class_name_from_ptr(ptr: *mut c_void) -> Option<String> {
    let type_ptr = cpython_exception_type_ptr(ptr);
    if type_ptr.is_null() || !cpython_ptr_is_type_object(type_ptr) {
        return None;
    }
    let name = cpython_safe_type_name(type_ptr.cast::<CpythonTypeObject>())?;
    if name.is_empty() || name == "type" {
        None
    } else {
        Some(name)
    }
}

fn cpython_exception_expected_name_from_ptr(ptr: *mut c_void) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    if let Some(Value::ExceptionType(name)) = cpython_exception_value_from_ptr(ptr as usize) {
        return Some(name);
    }
    cpython_exception_class_name_from_ptr(ptr)
}

fn cpython_type_inherits_exception_name(type_ptr: *mut c_void, expected_name: &str) -> bool {
    if type_ptr.is_null() || expected_name.is_empty() {
        return false;
    }
    let mut depth = 0usize;
    let mut current = type_ptr.cast::<CpythonTypeObject>();
    while !current.is_null() && depth < 128 {
        if !cpython_ptr_is_type_object(current.cast()) {
            return false;
        }
        let current_name = cpython_safe_type_name(current).unwrap_or_default();
        if current_name == expected_name {
            return true;
        }
        // SAFETY: `current` is non-null; reading `tp_base` is valid for CPython type layouts.
        current = unsafe {
            current
                .as_ref()
                .map(|ty| ty.tp_base)
                .unwrap_or(std::ptr::null_mut())
        };
        depth += 1;
    }
    false
}

fn cpython_tuple_items_for_match(tuple: *mut c_void) -> Option<Vec<*mut c_void>> {
    if tuple.is_null() {
        return None;
    }
    let tuple_type = std::ptr::addr_of_mut!(PyTuple_Type).cast::<c_void>();
    // SAFETY: pointer is inspected as CPython object header for tuple type checks.
    let ty = unsafe {
        tuple
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type)
            .unwrap_or(std::ptr::null_mut())
    };
    if ty.is_null() {
        return None;
    }
    let is_tuple = ty == tuple_type
        // SAFETY: both pointers are valid type objects for subtype checks.
        || unsafe { PyType_IsSubtype(ty, tuple_type) != 0 };
    if !is_tuple {
        return None;
    }
    // SAFETY: tuple pointer has CPython tuple layout with contiguous items.
    let len = unsafe {
        tuple
            .cast::<CpythonVarObjectHead>()
            .as_ref()
            .map(|head| head.ob_size.max(0) as usize)
            .unwrap_or(0)
    };
    // SAFETY: tuple pointer has CPython tuple layout.
    let item_ptr = unsafe { cpython_tuple_items_ptr(tuple) };
    if item_ptr.is_null() {
        return Some(Vec::new());
    }
    let mut items = Vec::with_capacity(len);
    // SAFETY: tuple stores at least `len` item pointers.
    unsafe {
        for idx in 0..len {
            items.push(*item_ptr.add(idx));
        }
    }
    Some(items)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_GivenExceptionMatches(
    given: *mut c_void,
    expected: *mut c_void,
) -> i32 {
    if given.is_null() || expected.is_null() {
        return 0;
    }
    if given == expected {
        return 1;
    }
    if let Some(items) = cpython_tuple_items_for_match(expected) {
        for item in items {
            if unsafe { PyErr_GivenExceptionMatches(given, item) } != 0 {
                return 1;
            }
        }
        return 0;
    }
    let given_type = cpython_exception_type_ptr(given);
    let expected_type = cpython_exception_type_ptr(expected);
    if given_type.is_null() || expected_type.is_null() {
        return 0;
    }
    if given_type == expected_type {
        return 1;
    }
    // SAFETY: both pointers refer to CPython-compatible type objects.
    if unsafe { PyType_IsSubtype(given_type, expected_type) } != 0 {
        return 1;
    }
    if let Some(expected_name) = cpython_exception_expected_name_from_ptr(expected)
        && cpython_type_inherits_exception_name(given_type, &expected_name)
    {
        return 1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Fetch(
    ptype: *mut *mut c_void,
    pvalue: *mut *mut c_void,
    ptraceback: *mut *mut c_void,
) {
    let state = with_active_cpython_context_mut(|context| context.fetch_error_state()).unwrap_or(
        CpythonErrorState {
            ptype: std::ptr::null_mut(),
            pvalue: std::ptr::null_mut(),
            ptraceback: std::ptr::null_mut(),
        },
    );
    if !ptype.is_null() {
        // SAFETY: caller provided writable error-type output pointer.
        unsafe { *ptype = state.ptype };
    }
    if !pvalue.is_null() {
        // SAFETY: caller provided writable error-value output pointer.
        unsafe { *pvalue = state.pvalue };
    }
    if !ptraceback.is_null() {
        // SAFETY: caller provided writable traceback output pointer.
        unsafe { *ptraceback = state.ptraceback };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Restore(
    ptype: *mut c_void,
    pvalue: *mut c_void,
    _ptraceback: *mut c_void,
) {
    let _ = with_active_cpython_context_mut(|context| {
        context.restore_error_state(CpythonErrorState {
            ptype,
            pvalue,
            ptraceback: _ptraceback,
        });
    })
    .map_err(|err| {
        cpython_set_error(err);
    });
}

fn cpython_exception_type_ptr_for_value(
    context: &mut ModuleCapiContext,
    value: &Value,
) -> Option<*mut c_void> {
    match value {
        Value::Exception(exception_obj) => {
            let attr_hint = exception_obj
                .attrs
                .borrow()
                .get(CPY_EXCEPTION_TYPE_PTR_ATTR)
                .cloned();
            if std::env::var_os("PYRS_TRACE_CPY_EXC_TYPE_HINT").is_some() {
                let map_hit = context
                    .exception_type_ptr_by_name
                    .get(&exception_obj.name)
                    .copied();
                eprintln!(
                    "[cpy-exc-type] name={} attr_hint={attr_hint:?} map_hit={map_hit:?}",
                    exception_obj.name
                );
            }
            if let Some(Value::Int(raw)) = attr_hint
                && raw > 0
            {
                return Some(raw as usize as *mut c_void);
            }
            if let Some(raw_ptr) = context.exception_type_ptr_by_name.get(&exception_obj.name) {
                return Some(*raw_ptr as *mut c_void);
            }
            if context.vm.is_null() {
                return None;
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *context.vm };
            let class = vm.alloc_synthetic_exception_class(&exception_obj.name);
            Some(context.alloc_cpython_ptr_for_value(Value::Class(class)))
        }
        Value::Instance(instance) => {
            let hint = {
                let Object::Instance(instance_data) = &*instance.kind() else {
                    return None;
                };
                instance_data
                    .attrs
                    .get(CPY_EXCEPTION_TYPE_PTR_ATTR)
                    .cloned()
            };
            if let Some(Value::Int(raw)) = hint
                && raw > 0
            {
                return Some(raw as usize as *mut c_void);
            }
            if !cpython_is_exception_instance(context, instance) {
                let instance_name = {
                    let Object::Instance(instance_data) = &*instance.kind() else {
                        return None;
                    };
                    let Object::Class(class_data) = &*instance_data.class.kind() else {
                        return None;
                    };
                    class_data.name.clone()
                };
                if let Some(raw_ptr) = context.exception_type_ptr_by_name.get(&instance_name) {
                    return Some(*raw_ptr as *mut c_void);
                }
                return None;
            }
            let Object::Instance(instance_data) = &*instance.kind() else {
                return None;
            };
            let class = instance_data.class.clone();
            Some(context.alloc_cpython_ptr_for_value(Value::Class(class)))
        }
        Value::ExceptionType(name) => {
            if context.vm.is_null() {
                return None;
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *context.vm };
            let class = vm.alloc_synthetic_exception_class(name);
            Some(context.alloc_cpython_ptr_for_value(Value::Class(class)))
        }
        Value::Class(class) => {
            Some(context.alloc_cpython_ptr_for_value(Value::Class(class.clone())))
        }
        _ => None,
    }
}

fn cpython_exception_traceback_ptr_for_value(
    context: &mut ModuleCapiContext,
    value: &Value,
) -> Option<*mut c_void> {
    match value {
        Value::Exception(exception_obj) => exception_obj
            .attrs
            .borrow()
            .get("__traceback__")
            .cloned()
            .or_else(|| exception_obj.attrs.borrow().get("exc_traceback").cloned())
            .filter(|tb| !matches!(tb, Value::None))
            .map(|tb| context.alloc_cpython_ptr_for_value(tb)),
        Value::Instance(instance) => {
            if !cpython_is_exception_instance(context, instance) {
                return None;
            }
            let Object::Instance(instance_data) = &*instance.kind() else {
                return None;
            };
            instance_data
                .attrs
                .get("__traceback__")
                .cloned()
                .or_else(|| instance_data.attrs.get("exc_traceback").cloned())
                .filter(|tb| !matches!(tb, Value::None))
                .map(|tb| context.alloc_cpython_ptr_for_value(tb))
        }
        _ => None,
    }
}

fn cpython_make_exception_instance_from_type_and_value(
    context: &mut ModuleCapiContext,
    ptype: *mut c_void,
    pvalue: Option<Value>,
) -> Option<*mut c_void> {
    if context.vm.is_null() || ptype.is_null() {
        return None;
    }
    // SAFETY: VM pointer is valid for active C-API context lifetime.
    let vm = unsafe { &mut *context.vm };
    let callable = if let Some(Value::ExceptionType(name)) =
        cpython_exception_value_from_ptr(ptype as usize)
    {
        Value::Class(vm.alloc_synthetic_exception_class(&name))
    } else {
        match context.cpython_value_from_ptr_or_proxy(ptype)? {
            Value::Class(class) => Value::Class(class),
            Value::ExceptionType(name) => Value::Class(vm.alloc_synthetic_exception_class(&name)),
            _ => return None,
        }
    };
    let args = match pvalue {
        Some(Value::Tuple(tuple_obj)) => match &*tuple_obj.kind() {
            Object::Tuple(values) => values.clone(),
            _ => Vec::new(),
        },
        Some(Value::None) | None => Vec::new(),
        Some(value) => vec![value],
    };
    match vm.call_internal(callable, args, HashMap::new()) {
        Ok(InternalCallOutcome::Value(value)) => match vm.normalize_exception_value(value) {
            Ok(value) => Some(context.alloc_cpython_ptr_for_value(value)),
            Err(_) => None,
        },
        _ => None,
    }
}

fn cpython_raised_exception_ptr_from_state(
    context: &mut ModuleCapiContext,
    state: CpythonErrorState,
) -> *mut c_void {
    if state.ptype.is_null() && state.pvalue.is_null() && state.ptraceback.is_null() {
        return std::ptr::null_mut();
    }
    let value = if !state.pvalue.is_null() {
        context.cpython_value_from_ptr_or_proxy(state.pvalue)
    } else {
        None
    };
    if let Some(value) = value.as_ref() {
        if cpython_is_exception_value(context, value) {
            return context.alloc_cpython_ptr_for_value(value.clone());
        }
    }
    if let Some(ptr) =
        cpython_make_exception_instance_from_type_and_value(context, state.ptype, value.clone())
    {
        return ptr;
    }
    if let Some(value) = value {
        return context.alloc_cpython_ptr_for_value(value);
    }
    if !state.ptype.is_null() {
        return state.ptype;
    }
    std::ptr::null_mut()
}

fn cpython_is_exception_value(context: &ModuleCapiContext, value: &Value) -> bool {
    match value {
        Value::Exception(_) | Value::ExceptionType(_) => true,
        Value::Instance(instance) => cpython_is_exception_instance(context, instance),
        _ => false,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_GetRaisedException() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let state = context.fetch_error_state();
        cpython_raised_exception_ptr_from_state(context, state)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetRaisedException(exc: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        if exc.is_null() {
            context.clear_error();
            return;
        }
        let message = context.error_message_from_ptr(exc);
        let ptype = cpython_exception_type_ptr(exc);
        if ptype.is_null() {
            context.set_error("PyErr_SetRaisedException expected exception object");
            return;
        }
        let ptraceback = context
            .cpython_value_from_ptr_or_proxy(exc)
            .as_ref()
            .and_then(|value| cpython_exception_traceback_ptr_for_value(context, value))
            .unwrap_or(std::ptr::null_mut());
        context.set_error_state(ptype, exc, ptraceback, message);
    })
    .map_err(cpython_set_error);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_GetHandledException() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        context
            .handled_exception_get()
            .map(|value| context.alloc_cpython_ptr_for_value(value))
            .unwrap_or(std::ptr::null_mut())
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetHandledException(exc: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        if exc.is_null() {
            context.handled_exception_set(None);
            return;
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(exc) else {
            context.set_error("PyErr_SetHandledException received unknown exception pointer");
            return;
        };
        if matches!(value, Value::None) {
            context.handled_exception_set(None);
            return;
        }
        let normalized = if context.vm.is_null() {
            value
        } else {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *context.vm };
            match vm.normalize_exception_value(value) {
                Ok(value) => value,
                Err(err) => {
                    context.set_error(err.message);
                    return;
                }
            }
        };
        if !cpython_is_exception_value(context, &normalized) {
            context.set_error("PyErr_SetHandledException expected exception object");
            return;
        }
        context.handled_exception_set(Some(normalized));
    })
    .map_err(cpython_set_error);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_GetExcInfo(
    p_type: *mut *mut c_void,
    p_value: *mut *mut c_void,
    p_traceback: *mut *mut c_void,
) {
    let _ = with_active_cpython_context_mut(|context| {
        let handled = context.handled_exception_get();
        if !p_type.is_null() {
            let value = handled
                .as_ref()
                .and_then(|value| cpython_exception_type_ptr_for_value(context, value))
                .unwrap_or_else(|| context.alloc_cpython_ptr_for_value(Value::None));
            // SAFETY: caller provided writable output pointer.
            unsafe { *p_type = value };
        }
        if !p_value.is_null() {
            let value = handled
                .clone()
                .map(|value| context.alloc_cpython_ptr_for_value(value))
                .unwrap_or(std::ptr::null_mut());
            // SAFETY: caller provided writable output pointer.
            unsafe { *p_value = value };
        }
        if !p_traceback.is_null() {
            let value = handled
                .as_ref()
                .and_then(|value| cpython_exception_traceback_ptr_for_value(context, value))
                .unwrap_or_else(|| context.alloc_cpython_ptr_for_value(Value::None));
            // SAFETY: caller provided writable output pointer.
            unsafe { *p_traceback = value };
        }
    })
    .map_err(cpython_set_error);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcInfo(
    p_type: *mut c_void,
    p_value: *mut c_void,
    p_traceback: *mut c_void,
) {
    unsafe { PyErr_SetHandledException(p_value) };
    // Keep CPython ownership semantics: arguments are stolen.
    unsafe {
        Py_XDecRef(p_value);
        Py_XDecRef(p_type);
        Py_XDecRef(p_traceback);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_pyerr_format_fallback(
    exception: *mut c_void,
    format: *const c_char,
) -> *mut c_void {
    let message = if format.is_null() {
        "error".to_string()
    } else {
        unsafe { CStr::from_ptr(format) }
            .to_str()
            .unwrap_or("error")
            .to_string()
    };
    if std::env::var_os("PYRS_TRACE_NUMPY_DTYPE").is_some() && message.contains("data type") {
        eprintln!(
            "[cpy-dtype] PyErr_Format exception={:p} msg={} bt={:?}",
            exception,
            message,
            Backtrace::force_capture()
        );
    }
    if std::env::var_os("PYRS_TRACE_NUMPY_PICKLE_FAIL").is_some()
        && message.starts_with("Unable to initialize pickling for ")
    {
        eprintln!(
            "[numpy-pickle-fail] from-PyErr_Format message={} bt={:?}",
            message,
            Backtrace::force_capture()
        );
    }
    if exception.is_null() {
        cpython_set_error(message);
    } else {
        cpython_set_typed_error(exception, message);
    }
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_pyerr_formatv_fallback(
    exception: *mut c_void,
    format: *const c_char,
    vargs: *mut c_void,
) -> *mut c_void {
    let _ = vargs;
    unsafe { pyrs_capi_pyerr_format_fallback(exception, format) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_NormalizeException(
    _ptype: *mut *mut c_void,
    _pvalue: *mut *mut c_void,
    _ptraceback: *mut *mut c_void,
) {
}

fn cpython_optional_filename_from_c(name: *const c_char) -> Option<String> {
    if name.is_null() {
        return None;
    }
    unsafe { c_name_to_string(name) }.ok()
}

fn cpython_optional_filename_from_object(name: *mut c_void) -> Option<String> {
    if name.is_null() {
        return None;
    }
    with_active_cpython_context_mut(|context| {
        let value = context.cpython_value_from_ptr_or_proxy(name)?;
        match value {
            Value::Str(text) => Some(text),
            Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
                _ => None,
            },
            Value::ByteArray(bytes_obj) => match &*bytes_obj.kind() {
                Object::ByteArray(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
                _ => None,
            },
            _ => None,
        }
    })
    .ok()
    .flatten()
}

fn cpython_set_os_error_message(
    exception: *mut c_void,
    code: Option<i32>,
    filename: Option<String>,
    filename2: Option<String>,
) {
    let mut message = match code {
        Some(code) => format!("system error {code}"),
        None => "system error".to_string(),
    };
    if let Some(filename) = filename {
        message.push_str(&format!(": {filename}"));
    }
    if let Some(filename2) = filename2 {
        message.push_str(&format!(" -> {filename2}"));
    }
    let exception = if exception.is_null() {
        unsafe { PyExc_OSError }
    } else {
        exception
    };
    cpython_set_typed_error(exception, message);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrno(exception: *mut c_void) -> *mut c_void {
    cpython_set_os_error_message(exception, None, None, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrnoWithFilename(
    exception: *mut c_void,
    filename: *const c_char,
) -> *mut c_void {
    let filename = cpython_optional_filename_from_c(filename);
    cpython_set_os_error_message(exception, None, filename, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrnoWithFilenameObject(
    exception: *mut c_void,
    filename: *mut c_void,
) -> *mut c_void {
    let filename = cpython_optional_filename_from_object(filename);
    cpython_set_os_error_message(exception, None, filename, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrnoWithFilenameObjects(
    exception: *mut c_void,
    filename1: *mut c_void,
    filename2: *mut c_void,
) -> *mut c_void {
    let filename1 = cpython_optional_filename_from_object(filename1);
    let filename2 = cpython_optional_filename_from_object(filename2);
    cpython_set_os_error_message(exception, None, filename1, filename2);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErr(
    exception: *mut c_void,
    ierr: i32,
) -> *mut c_void {
    cpython_set_os_error_message(exception, Some(ierr), None, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErrWithFilename(
    exception: *mut c_void,
    ierr: i32,
    filename: *const c_char,
) -> *mut c_void {
    let filename = cpython_optional_filename_from_c(filename);
    cpython_set_os_error_message(exception, Some(ierr), filename, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErrWithFilenameObject(
    exception: *mut c_void,
    ierr: i32,
    filename: *mut c_void,
) -> *mut c_void {
    let filename = cpython_optional_filename_from_object(filename);
    cpython_set_os_error_message(exception, Some(ierr), filename, None);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErrWithFilenameObjects(
    exception: *mut c_void,
    ierr: i32,
    filename1: *mut c_void,
    filename2: *mut c_void,
) -> *mut c_void {
    let filename1 = cpython_optional_filename_from_object(filename1);
    let filename2 = cpython_optional_filename_from_object(filename2);
    cpython_set_os_error_message(exception, Some(ierr), filename1, filename2);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromWindowsErr(ierr: i32) -> *mut c_void {
    unsafe { PyErr_SetExcFromWindowsErr(std::ptr::null_mut(), ierr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromWindowsErrWithFilename(
    ierr: i32,
    filename: *const c_char,
) -> *mut c_void {
    unsafe { PyErr_SetExcFromWindowsErrWithFilename(std::ptr::null_mut(), ierr, filename) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetInterrupt() {
    cpython_set_typed_error(std::ptr::null_mut(), "KeyboardInterrupt");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetInterruptEx(signum: i32) -> i32 {
    if signum <= 0 {
        return -1;
    }
    unsafe { PyErr_SetInterrupt() };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SyntaxLocation(filename: *const c_char, lineno: i32) {
    unsafe { PyErr_SyntaxLocationEx(filename, lineno, 0) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SyntaxLocationEx(
    filename: *const c_char,
    lineno: i32,
    col_offset: i32,
) {
    let filename =
        cpython_optional_filename_from_c(filename).unwrap_or_else(|| "<unknown>".to_string());
    let message = format!("invalid syntax ({filename}, line {lineno}, column {col_offset})");
    cpython_set_typed_error(std::ptr::null_mut(), message);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_ProgramText(filename: *const c_char, lineno: i32) -> *mut c_void {
    if lineno <= 0 {
        return std::ptr::null_mut();
    }
    let Some(filename) = cpython_optional_filename_from_c(filename) else {
        return std::ptr::null_mut();
    };
    let Ok(contents) = std::fs::read_to_string(&filename) else {
        return std::ptr::null_mut();
    };
    let index = (lineno - 1) as usize;
    let line = if let Some(line) = contents.split_inclusive('\n').nth(index) {
        line.to_string()
    } else if let Some(line) = contents.lines().nth(index) {
        line.to_string()
    } else {
        return std::ptr::null_mut();
    };
    cpython_new_ptr_for_value(Value::Str(line))
}

fn cpython_import_error_arg_or_none(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        cpython_new_ptr_for_value(Value::None)
    } else {
        object
    }
}

fn cpython_set_import_error_subclass_with_name_from(
    exception: *mut c_void,
    msg: *mut c_void,
    name: *mut c_void,
    path: *mut c_void,
    from_name: *mut c_void,
) -> *mut c_void {
    if exception.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "expected a subclass of ImportError",
        );
        return std::ptr::null_mut();
    }
    let is_subclass = unsafe { PyObject_IsSubclass(exception, PyExc_ImportError) };
    if is_subclass < 0 {
        return std::ptr::null_mut();
    }
    if is_subclass == 0 {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "expected a subclass of ImportError",
        );
        return std::ptr::null_mut();
    }
    if msg.is_null() {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "expected a message argument");
        return std::ptr::null_mut();
    }

    let name_obj = cpython_import_error_arg_or_none(name);
    let path_obj = cpython_import_error_arg_or_none(path);
    let from_name_obj = cpython_import_error_arg_or_none(from_name);
    if name_obj.is_null() || path_obj.is_null() || from_name_obj.is_null() {
        return std::ptr::null_mut();
    }

    let args = unsafe { PyTuple_New(1) };
    if args.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { Py_IncRef(msg) };
    if unsafe { PyTuple_SetItem(args, 0, msg) } != 0 {
        unsafe { Py_DecRef(args) };
        return std::ptr::null_mut();
    }

    let error_instance = unsafe { PyObject_CallObject(exception, args) };
    unsafe { Py_DecRef(args) };
    if error_instance.is_null() {
        return std::ptr::null_mut();
    }
    if unsafe { PyObject_SetAttrString(error_instance, c"name".as_ptr(), name_obj) } != 0
        || unsafe { PyObject_SetAttrString(error_instance, c"path".as_ptr(), path_obj) } != 0
        || unsafe { PyObject_SetAttrString(error_instance, c"name_from".as_ptr(), from_name_obj) }
            != 0
    {
        return std::ptr::null_mut();
    }

    unsafe { PyErr_SetObject(exception, error_instance) };
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetImportError(
    msg: *mut c_void,
    name: *mut c_void,
    path: *mut c_void,
) -> *mut c_void {
    cpython_set_import_error_subclass_with_name_from(
        unsafe { PyExc_ImportError },
        msg,
        name,
        path,
        std::ptr::null_mut(),
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetImportErrorSubclass(
    exception: *mut c_void,
    msg: *mut c_void,
    name: *mut c_void,
    path: *mut c_void,
) -> *mut c_void {
    cpython_set_import_error_subclass_with_name_from(
        exception,
        msg,
        name,
        path,
        std::ptr::null_mut(),
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_WarnEx(
    _category: *mut c_void,
    message: *const c_char,
    _stacklevel: isize,
) -> i32 {
    if !message.is_null()
        && let Ok(text) = unsafe { c_name_to_string(message) }
    {
        eprintln!("warning: {text}");
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_WarnExplicit(
    category: *mut c_void,
    text: *const c_char,
    filename: *const c_char,
    lineno: i32,
    module: *const c_char,
    _registry: *mut c_void,
) -> i32 {
    let text = match unsafe { c_name_to_string(text) } {
        Ok(value) => value,
        Err(_) => {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "PyErr_WarnExplicit requires non-null message",
            );
            return -1;
        }
    };
    let filename = if filename.is_null() {
        "<string>".to_string()
    } else {
        match unsafe { c_name_to_string(filename) } {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return -1;
            }
        }
    };
    let module = if module.is_null() {
        None
    } else {
        match unsafe { c_name_to_string(module) } {
            Ok(value) => Some(value),
            Err(err) => {
                cpython_set_error(err);
                return -1;
            }
        }
    };
    let mut rendered = format!("{filename}:{lineno}: {text}");
    if let Some(module) = module {
        rendered = format!("{module}: {rendered}");
    }
    let Ok(rendered) = CString::new(rendered) else {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "warning message contains interior NUL byte",
        );
        return -1;
    };
    unsafe { PyErr_WarnEx(category, rendered.as_ptr(), 1) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_WarnFormat(
    category: *mut c_void,
    stacklevel: isize,
    format: *const c_char,
) -> i32 {
    unsafe { PyErr_WarnEx(category, format, stacklevel) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_ResourceWarning(
    _source: *mut c_void,
    stack_level: isize,
    format: *const c_char,
) -> i32 {
    if format.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "PyErr_ResourceWarning requires non-null format string",
        );
        return -1;
    }
    let category = unsafe {
        if PyExc_ResourceWarning.is_null() {
            PyExc_RuntimeWarning
        } else {
            PyExc_ResourceWarning
        }
    };
    unsafe { PyErr_WarnEx(category, format, stack_level) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_WriteUnraisable(_object: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Print() {
    if let Ok(Some(message)) = with_active_cpython_context_mut(|context| context.last_error.clone())
    {
        eprintln!("error: {message}");
    }
    unsafe { PyErr_Clear() };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_PrintEx(_set_sys_last_vars: i32) {
    unsafe { PyErr_Print() };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Display(
    _exception: *mut c_void,
    value: *mut c_void,
    _traceback: *mut c_void,
) {
    let _ = with_active_cpython_context_mut(|context| {
        let message = if value.is_null() {
            "unhandled exception".to_string()
        } else {
            context.error_message_from_ptr(value)
        };
        eprintln!("error: {message}");
        context.clear_error();
    })
    .map_err(|err| cpython_set_error(err));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_DisplayException(exc: *mut c_void) {
    unsafe { PyErr_Display(std::ptr::null_mut(), exc, std::ptr::null_mut()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_FromFd(
    fd: i32,
    _name: *const c_char,
    mode: *const c_char,
    buffering: i32,
    encoding: *const c_char,
    errors: *const c_char,
    newline: *const c_char,
    closefd: i32,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyFile_FromFd missing VM context");
            return std::ptr::null_mut();
        }
        let mode_value = if mode.is_null() {
            Value::Str("r".to_string())
        } else {
            match unsafe { c_name_to_string(mode) } {
                Ok(text) => Value::Str(text),
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
        };
        let encoding_value = if encoding.is_null() {
            None
        } else {
            match unsafe { c_name_to_string(encoding) } {
                Ok(text) => Some(Value::Str(text)),
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
        };
        let errors_value = if errors.is_null() {
            None
        } else {
            match unsafe { c_name_to_string(errors) } {
                Ok(text) => Some(Value::Str(text)),
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
        };
        let newline_value = if newline.is_null() {
            None
        } else {
            match unsafe { c_name_to_string(newline) } {
                Ok(text) => Some(Value::Str(text)),
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
        };
        let mut kwargs = HashMap::new();
        kwargs.insert("mode".to_string(), mode_value);
        if buffering >= 0 {
            kwargs.insert("buffering".to_string(), Value::Int(buffering as i64));
        }
        kwargs.insert("closefd".to_string(), Value::Bool(closefd != 0));
        if let Some(value) = encoding_value {
            kwargs.insert("encoding".to_string(), value);
        }
        if let Some(value) = errors_value {
            kwargs.insert("errors".to_string(), value);
        }
        if let Some(value) = newline_value {
            kwargs.insert("newline".to_string(), value);
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_internal(
            Value::Builtin(BuiltinFunction::IoOpen),
            vec![Value::Int(fd as i64)],
            kwargs,
        ) {
            Ok(InternalCallOutcome::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("PyFile_FromFd failed")
                        .message,
                );
                std::ptr::null_mut()
            }
            Err(err) => {
                context.set_error(err.message);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_GetLine(file: *mut c_void, n: i32) -> *mut c_void {
    if file.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let readline = unsafe { PyObject_GetAttrString(file, c"readline".as_ptr()) };
    if readline.is_null() {
        return std::ptr::null_mut();
    }
    let result = if n <= 0 {
        unsafe { PyObject_CallObject(readline, std::ptr::null_mut()) }
    } else {
        let arg = unsafe { PyLong_FromLong(n as i64) };
        if arg.is_null() {
            unsafe { Py_DecRef(readline) };
            return std::ptr::null_mut();
        }
        let result = unsafe { PyObject_CallOneArg(readline, arg) };
        unsafe { Py_DecRef(arg) };
        result
    };
    unsafe { Py_DecRef(readline) };
    if result.is_null() {
        return std::ptr::null_mut();
    }
    let value = match cpython_value_from_ptr(result) {
        Ok(value) => value,
        Err(err) => {
            unsafe { Py_DecRef(result) };
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match value {
        Value::Bytes(bytes_obj) => {
            let Object::Bytes(payload) = &*bytes_obj.kind() else {
                unsafe { Py_DecRef(result) };
                cpython_set_typed_error(
                    unsafe { PyExc_TypeError },
                    "object.readline() returned non-string",
                );
                return std::ptr::null_mut();
            };
            if n < 0 {
                if payload.is_empty() {
                    unsafe { Py_DecRef(result) };
                    cpython_set_typed_error(unsafe { PyExc_EOFError }, "EOF when reading a line");
                    return std::ptr::null_mut();
                }
                if payload.last().copied() == Some(b'\n') {
                    let mut trimmed = payload.clone();
                    trimmed.pop();
                    unsafe { Py_DecRef(result) };
                    return cpython_new_bytes_ptr(trimmed);
                }
            }
            result
        }
        Value::Str(text) => {
            if n < 0 {
                if text.is_empty() {
                    unsafe { Py_DecRef(result) };
                    cpython_set_typed_error(unsafe { PyExc_EOFError }, "EOF when reading a line");
                    return std::ptr::null_mut();
                }
                if text.ends_with('\n') {
                    let mut trimmed = text;
                    let _ = trimmed.pop();
                    unsafe { Py_DecRef(result) };
                    return cpython_new_ptr_for_value(Value::Str(trimmed));
                }
            }
            result
        }
        _ => {
            unsafe { Py_DecRef(result) };
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "object.readline() returned non-string",
            );
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_WriteObject(
    value: *mut c_void,
    file: *mut c_void,
    flags: i32,
) -> i32 {
    const PY_PRINT_RAW_FLAG: i32 = 1;
    with_active_cpython_context_mut(|context| {
        if file.is_null() {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, "writeobject with NULL file");
            return -1;
        }
        let Some(file_value) = context.cpython_value_from_ptr_or_proxy(file) else {
            context.set_error("PyFile_WriteObject received unknown file pointer");
            return -1;
        };
        let Some(value) = context.cpython_value_from_ptr_or_proxy(value) else {
            context.set_error("PyFile_WriteObject received unknown value pointer");
            return -1;
        };
        if context.vm.is_null() {
            context.set_error("missing VM context for PyFile_WriteObject");
            return -1;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };

        let rendered = match vm.call_internal(
            Value::Builtin(if (flags & PY_PRINT_RAW_FLAG) != 0 {
                BuiltinFunction::Str
            } else {
                BuiltinFunction::Repr
            }),
            vec![value],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(rendered)) => rendered,
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("PyFile_WriteObject render failed")
                        .message,
                );
                return -1;
            }
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        };

        let writer = match vm.call_internal(
            Value::Builtin(BuiltinFunction::GetAttr),
            vec![file_value, Value::Str("write".to_string())],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(writer)) => writer,
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("PyFile_WriteObject missing write")
                        .message,
                );
                return -1;
            }
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        };

        match vm.call_internal(writer, vec![rendered], HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => 0,
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("PyFile_WriteObject write failed")
                        .message,
                );
                -1
            }
            Err(err) => {
                context.set_error(err.message);
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_WriteString(text: *const c_char, file: *mut c_void) -> i32 {
    if file.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                "null file for PyFile_WriteString",
            );
        }
        return -1;
    }
    if !unsafe { PyErr_Occurred() }.is_null() {
        return -1;
    }
    let value = unsafe { PyUnicode_FromString(text) };
    if value.is_null() {
        return -1;
    }
    let status = unsafe { PyFile_WriteObject(value, file, 1) };
    unsafe { Py_DecRef(value) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceBack_Here(frame: *mut c_void) -> c_int {
    if frame.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceBack_Print(tb: *mut c_void, file: *mut c_void) -> c_int {
    if file.is_null() {
        if unsafe { PyErr_Occurred() }.is_null() {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                "null file for PyTraceBack_Print",
            );
        }
        return -1;
    }
    if tb.is_null() {
        return 0;
    }
    let rendered = unsafe { PyObject_Str(tb) };
    if rendered.is_null() {
        return -1;
    }
    let text = unsafe { PyUnicode_AsUTF8(rendered) };
    if text.is_null() {
        unsafe { Py_DecRef(rendered) };
        return -1;
    }
    let status = unsafe { PyFile_WriteString(text, file) };
    unsafe { Py_DecRef(rendered) };
    status
}

fn cpython_is_exception_instance(context: &ModuleCapiContext, instance: &ObjRef) -> bool {
    if context.vm.is_null() {
        return false;
    }
    // SAFETY: VM pointer is valid for active C-API context lifetime.
    unsafe {
        (&*context.vm)
            .exception_class_name_for_instance(instance)
            .is_some()
    }
}

fn cpython_is_exception_instance_for_vm(vm: *mut Vm, instance: &ObjRef) -> bool {
    if vm.is_null() {
        return false;
    }
    // SAFETY: VM pointer is valid for active C-API context lifetime.
    unsafe { (&*vm).exception_class_name_for_instance(instance).is_some() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_GetTraceback(exception: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(exception) else {
            context.set_error("PyException_GetTraceback received unknown exception pointer");
            return std::ptr::null_mut();
        };
        let traceback = match value {
            Value::Exception(exception_obj) => {
                let attrs = exception_obj.attrs.borrow();
                attrs
                    .get("__traceback__")
                    .cloned()
                    .or_else(|| attrs.get("exc_traceback").cloned())
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance(context, &instance) {
                    context.set_error("PyException_GetTraceback expected exception object");
                    return std::ptr::null_mut();
                }
                let Object::Instance(instance_data) = &*instance.kind() else {
                    context.set_error("PyException_GetTraceback encountered invalid instance");
                    return std::ptr::null_mut();
                };
                instance_data
                    .attrs
                    .get("__traceback__")
                    .cloned()
                    .or_else(|| instance_data.attrs.get("exc_traceback").cloned())
            }
            _ => {
                context.set_error("PyException_GetTraceback expected exception object");
                return std::ptr::null_mut();
            }
        };
        match traceback {
            Some(Value::None) | None => std::ptr::null_mut(),
            Some(value) => context.alloc_cpython_ptr_for_value(value),
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_GetCause(exception: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(exception) else {
            context.set_error("PyException_GetCause received unknown exception pointer");
            return std::ptr::null_mut();
        };
        match value {
            Value::Exception(exception_obj) => match exception_obj.cause {
                Some(cause) => context.alloc_cpython_ptr_for_value(Value::Exception(cause)),
                None => std::ptr::null_mut(),
            },
            Value::Instance(instance) => {
                if !cpython_is_exception_instance(context, &instance) {
                    context.set_error("PyException_GetCause expected exception object");
                    return std::ptr::null_mut();
                }
                let Object::Instance(instance_data) = &*instance.kind() else {
                    context.set_error("PyException_GetCause encountered invalid instance");
                    return std::ptr::null_mut();
                };
                match instance_data.attrs.get("__cause__").cloned() {
                    Some(Value::None) | None => std::ptr::null_mut(),
                    Some(value) => context.alloc_cpython_ptr_for_value(value),
                }
            }
            _ => {
                context.set_error("PyException_GetCause expected exception object");
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_GetContext(exception: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(exception) else {
            context.set_error("PyException_GetContext received unknown exception pointer");
            return std::ptr::null_mut();
        };
        match value {
            Value::Exception(exception_obj) => match exception_obj.context {
                Some(context_obj) => {
                    context.alloc_cpython_ptr_for_value(Value::Exception(context_obj))
                }
                None => std::ptr::null_mut(),
            },
            Value::Instance(instance) => {
                if !cpython_is_exception_instance(context, &instance) {
                    context.set_error("PyException_GetContext expected exception object");
                    return std::ptr::null_mut();
                }
                let Object::Instance(instance_data) = &*instance.kind() else {
                    context.set_error("PyException_GetContext encountered invalid instance");
                    return std::ptr::null_mut();
                };
                match instance_data.attrs.get("__context__").cloned() {
                    Some(Value::None) | None => std::ptr::null_mut(),
                    Some(value) => context.alloc_cpython_ptr_for_value(value),
                }
            }
            _ => {
                context.set_error("PyException_GetContext expected exception object");
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_GetArgs(exception: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(exception) else {
            context.set_error("PyException_GetArgs received unknown exception pointer");
            return std::ptr::null_mut();
        };
        let args_value = match value {
            Value::Exception(exception_obj) => {
                if let Some(args) = exception_obj.attrs.borrow().get("args").cloned() {
                    args
                } else if context.vm.is_null() {
                    Value::None
                } else if let Some(message) = exception_obj.message {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    vm.heap.alloc_tuple(vec![Value::Str(message)])
                } else {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    vm.heap.alloc_tuple(Vec::new())
                }
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance(context, &instance) {
                    context.set_error("PyException_GetArgs expected exception object");
                    return std::ptr::null_mut();
                }
                let Object::Instance(instance_data) = &*instance.kind() else {
                    context.set_error("PyException_GetArgs encountered invalid instance");
                    return std::ptr::null_mut();
                };
                if let Some(args) = instance_data.attrs.get("args").cloned() {
                    args
                } else if context.vm.is_null() {
                    Value::None
                } else {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    vm.heap.alloc_tuple(Vec::new())
                }
            }
            _ => {
                context.set_error("PyException_GetArgs expected exception object");
                return std::ptr::null_mut();
            }
        };
        context.alloc_cpython_ptr_for_value(args_value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_SetArgs(exception: *mut c_void, args: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let vm_ptr = context.vm;
        let Some(handle) = context.cpython_handle_from_ptr(exception) else {
            context.set_error("PyException_SetArgs received unknown exception pointer");
            return;
        };
        let Some(args_value) = context.cpython_value_from_ptr_or_proxy(args) else {
            context.set_error("PyException_SetArgs received unknown args pointer");
            return;
        };
        let Value::Tuple(_) = args_value else {
            context.set_error("PyException_SetArgs expected tuple object");
            return;
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyException_SetArgs exception handle is not available");
            return;
        };
        match &mut slot.value {
            Value::Exception(exception_obj) => {
                exception_obj
                    .attrs
                    .borrow_mut()
                    .insert("args".to_string(), args_value);
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance_for_vm(vm_ptr, instance) {
                    context.set_error("PyException_SetArgs expected exception object");
                    return;
                }
                let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                    context.set_error("PyException_SetArgs encountered invalid instance");
                    return;
                };
                instance_data.attrs.insert("args".to_string(), args_value);
            }
            _ => {
                context.set_error("PyException_SetArgs expected exception object");
                return;
            }
        }
    })
    .map_err(|err| cpython_set_error(err));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_SetCause(exception: *mut c_void, cause: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let vm_ptr = context.vm;
        let Some(handle) = context.cpython_handle_from_ptr(exception) else {
            context.set_error("PyException_SetCause received unknown exception pointer");
            return;
        };
        let cause_value = if cause.is_null() {
            None
        } else {
            let Some(raw_value) = context.cpython_value_from_ptr_or_proxy(cause) else {
                context.set_error("PyException_SetCause received unknown cause pointer");
                return;
            };
            if vm_ptr.is_null() {
                context.set_error("PyException_SetCause missing VM context");
                return;
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *vm_ptr };
            match vm.normalize_exception_value(raw_value) {
                Ok(Value::Exception(exc)) => Some(Value::Exception(exc)),
                Ok(_) => {
                    context.set_error("PyException_SetCause expected exception cause");
                    return;
                }
                Err(err) => {
                    context.set_error(err.message);
                    return;
                }
            }
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyException_SetCause exception handle is not available");
            return;
        };
        match &mut slot.value {
            Value::Exception(exception_obj) => {
                exception_obj.cause = match cause_value.clone() {
                    Some(Value::Exception(cause_obj)) => Some(cause_obj),
                    Some(_) => {
                        context.set_error("PyException_SetCause expected exception cause");
                        return;
                    }
                    None => None,
                };
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance_for_vm(vm_ptr, instance) {
                    context.set_error("PyException_SetCause expected exception object");
                    return;
                }
                let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                    context.set_error("PyException_SetCause encountered invalid instance");
                    return;
                };
                let stored = cause_value.unwrap_or(Value::None);
                instance_data.attrs.insert("__cause__".to_string(), stored);
            }
            _ => {
                context.set_error("PyException_SetCause expected exception object");
                return;
            }
        }
    })
    .map_err(|err| cpython_set_error(err));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_SetContext(
    exception: *mut c_void,
    context_value: *mut c_void,
) {
    let _ = with_active_cpython_context_mut(|context| {
        let vm_ptr = context.vm;
        let Some(handle) = context.cpython_handle_from_ptr(exception) else {
            context.set_error("PyException_SetContext received unknown exception pointer");
            return;
        };
        let context_obj = if context_value.is_null() {
            None
        } else {
            let Some(raw_value) = context.cpython_value_from_ptr_or_proxy(context_value) else {
                context.set_error("PyException_SetContext received unknown context pointer");
                return;
            };
            if vm_ptr.is_null() {
                context.set_error("PyException_SetContext missing VM context");
                return;
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *vm_ptr };
            match vm.normalize_exception_value(raw_value) {
                Ok(Value::Exception(exc)) => Some(Value::Exception(exc)),
                Ok(_) => {
                    context.set_error("PyException_SetContext expected exception context");
                    return;
                }
                Err(err) => {
                    context.set_error(err.message);
                    return;
                }
            }
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyException_SetContext exception handle is not available");
            return;
        };
        match &mut slot.value {
            Value::Exception(exception_obj) => {
                exception_obj.context = match context_obj.clone() {
                    Some(Value::Exception(context_exception)) => Some(context_exception),
                    Some(_) => {
                        context.set_error("PyException_SetContext expected exception context");
                        return;
                    }
                    None => None,
                };
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance_for_vm(vm_ptr, instance) {
                    context.set_error("PyException_SetContext expected exception object");
                    return;
                }
                let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                    context.set_error("PyException_SetContext encountered invalid instance");
                    return;
                };
                let stored = context_obj.unwrap_or(Value::None);
                instance_data
                    .attrs
                    .insert("__context__".to_string(), stored);
            }
            _ => {
                context.set_error("PyException_SetContext expected exception object");
                return;
            }
        }
    })
    .map_err(|err| cpython_set_error(err));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_SetTraceback(exception: *mut c_void, traceback: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let vm_ptr = context.vm;
        let Some(handle) = context.cpython_handle_from_ptr(exception) else {
            context.set_error("PyException_SetTraceback received unknown exception pointer");
            return;
        };
        let traceback_value = if traceback.is_null() {
            Value::None
        } else {
            match context.cpython_value_from_ptr_or_proxy(traceback) {
                Some(value) => value,
                None => {
                    context
                        .set_error("PyException_SetTraceback received unknown traceback pointer");
                    return;
                }
            }
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyException_SetTraceback exception handle is not available");
            return;
        };
        match &mut slot.value {
            Value::Exception(exception_obj) => {
                let mut attrs = exception_obj.attrs.borrow_mut();
                attrs.insert("__traceback__".to_string(), traceback_value.clone());
                attrs.insert("exc_traceback".to_string(), traceback_value);
            }
            Value::Instance(instance) => {
                if !cpython_is_exception_instance_for_vm(vm_ptr, instance) {
                    context.set_error("PyException_SetTraceback expected exception object");
                    return;
                }
                let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
                    context.set_error("PyException_SetTraceback encountered invalid instance");
                    return;
                };
                instance_data
                    .attrs
                    .insert("__traceback__".to_string(), traceback_value.clone());
                instance_data
                    .attrs
                    .insert("exc_traceback".to_string(), traceback_value);
            }
            _ => {
                context.set_error("PyException_SetTraceback expected exception object");
                return;
            }
        }
    })
    .map_err(|err| cpython_set_error(err));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnstable_Object_IsUniquelyReferenced(_object: *mut c_void) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnstable_Object_IsUniqueReferencedTemporary(
    _object: *mut c_void,
) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GenericAlias(origin: *mut c_void, _args: *mut c_void) -> *mut c_void {
    unsafe { Py_XIncRef(origin) };
    origin
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetObject(_exception: *mut c_void, value: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let ptype = if _exception.is_null() {
            unsafe { PyExc_RuntimeError }
        } else {
            _exception
        };
        let value_obj = context.cpython_value_from_ptr_or_proxy(value);
        if std::env::var_os("PYRS_TRACE_CPY_UFUNC_ERRORS").is_some() {
            let exception_name = cpython_exception_class_name_from_ptr(ptype)
                .unwrap_or_else(|| cpython_type_name_for_object_ptr(ptype));
            if exception_name.contains("UFunc") || exception_name.contains("ufunc") {
                eprintln!(
                    "[cpy-ufunc-error] ptype={:p} name={} value_ptr={:p} value={} bt={:?}",
                    ptype,
                    exception_name,
                    value,
                    value_obj
                        .as_ref()
                        .map(cpython_value_debug_tag)
                        .unwrap_or_else(|| "<unknown>".to_string()),
                    Backtrace::force_capture()
                );
            }
        }
        if let Some(normalized) =
            cpython_make_exception_instance_from_type_and_value(context, ptype, value_obj.clone())
        {
            let message = context.error_message_from_ptr(normalized);
            context.set_error_state(ptype, normalized, std::ptr::null_mut(), message);
            return;
        }
        let message = context.error_message_from_ptr(value);
        context.set_error_state(ptype, value, std::ptr::null_mut(), message);
    })
    .map_err(|err| {
        cpython_set_error(err);
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetNone(exception: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let ptype = if exception.is_null() {
            unsafe { PyExc_RuntimeError }
        } else {
            exception
        };
        context.set_error_state(
            ptype,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            "error".to_string(),
        );
    })
    .map_err(|err| {
        cpython_set_error(err);
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_NoMemory() -> *mut c_void {
    let _ = with_active_cpython_context_mut(|context| {
        let message = "out of memory".to_string();
        let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
        context.set_error_state(
            unsafe { PyExc_MemoryError },
            pvalue,
            std::ptr::null_mut(),
            message,
        );
    })
    .map_err(|err| {
        cpython_set_error(err);
    });
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_CheckSignals() -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGILState_Ensure() -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGILState_GetThisThreadState() -> *mut c_void {
    unsafe { PyThreadState_Get() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGILState_Release(_state: i32) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_AcquireLock() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_ReleaseLock() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_AcquireThread(_state: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_ReleaseThread(_state: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_InitThreads() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_ThreadsInitialized() -> i32 {
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_CallObjectWithKeywords(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    unsafe { PyObject_Call(callable, args, kwargs) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalCode(
    code: *mut c_void,
    globals: *mut c_void,
    locals: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyEval_EvalCode missing VM context");
            return std::ptr::null_mut();
        }
        let Some(code_value) = context.cpython_value_from_ptr_or_proxy(code) else {
            context.set_error("PyEval_EvalCode received unknown code pointer");
            return std::ptr::null_mut();
        };
        let Value::Code(code_obj) = code_value else {
            context.set_error("PyEval_EvalCode expected code object");
            return std::ptr::null_mut();
        };
        if globals.is_null() {
            context.set_error("PyEval_EvalCode globals must not be NULL");
            return std::ptr::null_mut();
        }
        let Some(globals_value) = context.cpython_value_from_ptr_or_proxy(globals) else {
            context.set_error("PyEval_EvalCode received unknown globals pointer");
            return std::ptr::null_mut();
        };
        if !matches!(globals_value, Value::Dict(_) | Value::Module(_)) {
            context.set_error("PyEval_EvalCode globals must be a dict or module");
            return std::ptr::null_mut();
        }
        let locals_value = if locals.is_null() {
            globals_value.clone()
        } else {
            let Some(value) = context.cpython_value_from_ptr_or_proxy(locals) else {
                context.set_error("PyEval_EvalCode received unknown locals pointer");
                return std::ptr::null_mut();
            };
            value
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_eval(
            vec![Value::Code(code_obj), globals_value, locals_value],
            HashMap::new(),
        ) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err.message);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalCodeEx(
    code: *mut c_void,
    globals: *mut c_void,
    locals: *mut c_void,
    args: *const *mut c_void,
    argcount: i32,
    kws: *const *mut c_void,
    kwcount: i32,
    defs: *const *mut c_void,
    defcount: i32,
    kwdefs: *mut c_void,
    closure: *mut c_void,
) -> *mut c_void {
    let is_simple_call = args.is_null()
        && kws.is_null()
        && defs.is_null()
        && argcount == 0
        && kwcount == 0
        && defcount == 0
        && kwdefs.is_null()
        && closure.is_null();
    if is_simple_call {
        return unsafe { PyEval_EvalCode(code, globals, locals) };
    }
    with_active_cpython_context_mut(|context| {
        context.set_error("PyEval_EvalCodeEx extended args/kws/defs/closure are not yet supported");
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
    });
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalFrame(frame: *mut c_void) -> *mut c_void {
    if frame.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyEval_EvalFrame requires frame",
        );
        return std::ptr::null_mut();
    }
    if frame == unsafe { PyThreadState_Get() } {
        cpython_set_error("PyEval_EvalFrame current-frame evaluation is not yet supported");
        return std::ptr::null_mut();
    }
    let code = unsafe { PyFrame_GetCode(frame) };
    if code.is_null() {
        return std::ptr::null_mut();
    }
    let globals_from_frame = unsafe { PyEval_GetFrameGlobals() };
    let globals = if globals_from_frame.is_null() {
        let fallback = with_active_cpython_context_mut(|context| {
            context.alloc_cpython_ptr_for_value(Value::Module(context.module.clone()))
        })
        .unwrap_or_else(|_| std::ptr::null_mut());
        fallback
    } else {
        globals_from_frame
    };
    if globals.is_null() {
        unsafe { Py_DecRef(code) };
        return std::ptr::null_mut();
    }
    let locals = unsafe { PyEval_GetFrameLocals() };
    let locals_arg = if locals.is_null() { globals } else { locals };
    let result = unsafe { PyEval_EvalCode(code, globals, locals_arg) };
    unsafe {
        Py_DecRef(code);
        Py_DecRef(globals);
        if !locals.is_null() && locals != globals {
            Py_DecRef(locals);
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalFrameEx(frame: *mut c_void, _throwflag: i32) -> *mut c_void {
    unsafe { PyEval_EvalFrame(frame) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_SaveThread() -> *mut c_void {
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_RestoreThread(_state: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyInterpreterState_Main() -> *mut c_void {
    cpython_main_interpreter_state_ptr() as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMutex_Lock(_mutex: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMutex_Unlock(_mutex: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_strtol(
    string: *const c_char,
    endptr: *mut *mut c_char,
    base: i32,
) -> c_long {
    unsafe { strtol(string, endptr, base as c_int) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_strtoul(
    string: *const c_char,
    endptr: *mut *mut c_char,
    base: i32,
) -> c_ulong {
    unsafe { strtoul(string, endptr, base as c_int) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_string_to_double(
    string: *const c_char,
    endptr: *mut *mut c_char,
    _overflow_exception: *mut c_void,
) -> c_double {
    unsafe { strtod(string, endptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_snprintf(
    buffer: *mut c_char,
    size: usize,
    format: *const c_char,
) -> i32 {
    if buffer.is_null() || size == 0 {
        return 0;
    }
    let text = if format.is_null() {
        ""
    } else {
        // SAFETY: caller provides NUL-terminated format string.
        unsafe { CStr::from_ptr(format) }.to_str().unwrap_or("")
    };
    let bytes = text.as_bytes();
    let writable = size.saturating_sub(1).min(bytes.len());
    // SAFETY: caller provided writable output buffer with length `size`.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.cast::<u8>(), writable);
        *buffer.add(writable) = 0;
    }
    writable as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMarshal_WriteObjectToString(
    object: *mut c_void,
    _version: i32,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyMarshal_WriteObjectToString missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyMarshal_WriteObjectToString received unknown object");
            return std::ptr::null_mut();
        };
        let marshal_object = match value_to_cpython_marshal_object(&value) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(format!("PyMarshal_WriteObjectToString {err}"));
                return std::ptr::null_mut();
            }
        };
        let encoded = match marshal_dump_object(&marshal_object) {
            Ok(encoded) => encoded,
            Err(err) => {
                context.set_error(format!(
                    "PyMarshal_WriteObjectToString failed to encode object: {}",
                    err.message
                ));
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let encoded_value = unsafe { (&mut *context.vm).heap.alloc_bytes(encoded) };
        context.alloc_cpython_ptr_for_value(encoded_value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMarshal_ReadObjectFromString(
    data: *const c_char,
    len: isize,
) -> *mut c_void {
    if data.is_null() || len < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyMarshal_ReadObjectFromString requires non-null data and non-negative length",
        );
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyMarshal_ReadObjectFromString missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: caller guarantees `data` points to at least `len` bytes.
        let payload = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len as usize) };
        let decoded = match marshal_load_object(payload, true) {
            Ok(decoded) => decoded,
            Err(err) => {
                context.set_error(format!(
                    "PyMarshal_ReadObjectFromString failed to decode payload: {}",
                    err.message
                ));
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match cpython_marshal_object_to_value(&decoded, vm) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(format!("PyMarshal_ReadObjectFromString {err}"));
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}


macro_rules! define_exception_symbol {
    ($symbol:ident, $name:literal) => {
        #[unsafe(no_mangle)]
        #[used]
        pub static mut $symbol: *mut c_void = std::ptr::null_mut();
    };
}
for_each_cpython_exception_symbol!(define_exception_symbol);

const fn py_ascii_whitespace_table() -> [u8; 128] {
    let mut table = [0u8; 128];
    table[b' ' as usize] = 1;
    table[b'\t' as usize] = 1;
    table[b'\n' as usize] = 1;
    table[0x0B] = 1;
    table[0x0C] = 1;
    table[b'\r' as usize] = 1;
    table[0x1C] = 1;
    table[0x1D] = 1;
    table[0x1E] = 1;
    table[0x1F] = 1;
    table
}

#[unsafe(no_mangle)]
#[used]
pub static _Py_ascii_whitespace: [u8; 128] = py_ascii_whitespace_table();

#[unsafe(no_mangle)]
#[used]
pub static mut _Py_NoneStruct: CpythonObjectHead = CpythonObjectHead {
    ob_refcnt: -1,
    ob_type: std::ptr::null_mut(),
};
#[unsafe(no_mangle)]
#[used]
pub static mut _Py_NotImplementedStruct: CpythonObjectHead = CpythonObjectHead {
    ob_refcnt: -1,
    ob_type: std::ptr::null_mut(),
};
#[unsafe(no_mangle)]
#[used]
pub static mut _Py_EllipsisObject: CpythonObjectHead = CpythonObjectHead {
    ob_refcnt: -1,
    ob_type: std::ptr::null_mut(),
};
#[unsafe(no_mangle)]
#[used]
pub static mut _Py_FalseStruct: CpythonObjectHead = CpythonObjectHead {
    ob_refcnt: -1,
    ob_type: std::ptr::null_mut(),
};
#[unsafe(no_mangle)]
#[used]
pub static mut _Py_TrueStruct: CpythonObjectHead = CpythonObjectHead {
    ob_refcnt: -1,
    ob_type: std::ptr::null_mut(),
};
#[unsafe(no_mangle)]
#[used]
pub static PyStructSequence_UnnamedField: [u8; 14] = *b"unnamed field\0";
static PY_FILESYSTEM_DEFAULT_ENCODING: &[u8; 6] = b"utf-8\0";
static PY_FILESYSTEM_DEFAULT_ENCODE_ERRORS: &[u8; 7] = b"strict\0";
#[unsafe(no_mangle)]
#[used]
pub static mut _PyByteArray_empty_string: [c_char; 1] = [0];

#[unsafe(no_mangle)]
#[used]
pub static mut PyOS_InputHook: Option<unsafe extern "C" fn() -> c_int> = None;
#[unsafe(no_mangle)]
#[used]
pub static mut Py_FileSystemDefaultEncoding: *const c_char =
    PY_FILESYSTEM_DEFAULT_ENCODING.as_ptr().cast();
#[unsafe(no_mangle)]
#[used]
pub static mut Py_FileSystemDefaultEncodeErrors: *const c_char =
    PY_FILESYSTEM_DEFAULT_ENCODE_ERRORS.as_ptr().cast();
#[unsafe(no_mangle)]
#[used]
pub static mut Py_HasFileSystemDefaultEncoding: c_int = 1;
#[unsafe(no_mangle)]
#[used]
pub static mut Py_UTF8Mode: c_int = 1;
#[unsafe(no_mangle)]
#[used]
pub static Py_Version: c_ulong = 0x030E00F0;
#[unsafe(no_mangle)]
#[used]
pub static mut _Py_RefTotal: isize = 0;
#[unsafe(no_mangle)]
#[used]
pub static _Py_SwappedOp: [c_int; 6] = [4, 5, 2, 3, 0, 1];

const EMPTY_TYPE_FLAGS: usize = 0;
const PY_TPFLAGS_IMMUTABLETYPE: usize = 1usize << 8;
const PY_TPFLAGS_HEAPTYPE: usize = 1usize << 9;
const PY_TPFLAGS_BASETYPE: usize = 1usize << 10;
const PY_TPFLAGS_READY: usize = 1usize << 12;
const PY_TPFLAGS_LONG_SUBCLASS: usize = 1usize << 24;
const PY_TPFLAGS_LIST_SUBCLASS: usize = 1usize << 25;
const PY_TPFLAGS_TUPLE_SUBCLASS: usize = 1usize << 26;
const PY_TPFLAGS_BYTES_SUBCLASS: usize = 1usize << 27;
const PY_TPFLAGS_UNICODE_SUBCLASS: usize = 1usize << 28;
const PY_TPFLAGS_DICT_SUBCLASS: usize = 1usize << 29;
const PY_TPFLAGS_TYPE_SUBCLASS: usize = 1usize << 31;
const METH_VARARGS: c_int = 0x0001;
const METH_KEYWORDS: c_int = 0x0002;
const METH_NOARGS: c_int = 0x0004;
const METH_O: c_int = 0x0008;
const METH_FASTCALL: c_int = 0x0080;
const METH_METHOD: c_int = 0x0200;

const fn empty_type(name: *const c_char) -> CpythonTypeObject {
    CpythonTypeObject {
        ob_refcnt: 1,
        ob_type: std::ptr::null_mut(),
        ob_size: 0,
        tp_name: name,
        tp_basicsize: std::mem::size_of::<CpythonCompatObject>() as isize,
        tp_itemsize: 0,
        tp_dealloc: std::ptr::null_mut(),
        tp_vectorcall_offset: 0,
        tp_getattr: std::ptr::null_mut(),
        tp_setattr: std::ptr::null_mut(),
        tp_as_async: std::ptr::null_mut(),
        tp_repr: std::ptr::null_mut(),
        tp_as_number: std::ptr::null_mut(),
        tp_as_sequence: std::ptr::null_mut(),
        tp_as_mapping: std::ptr::null_mut(),
        tp_hash: std::ptr::null_mut(),
        tp_call: std::ptr::null_mut(),
        tp_str: std::ptr::null_mut(),
        tp_getattro: std::ptr::null_mut(),
        tp_setattro: std::ptr::null_mut(),
        tp_as_buffer: std::ptr::null_mut(),
        tp_flags: EMPTY_TYPE_FLAGS,
        tp_doc: std::ptr::null(),
        tp_traverse: std::ptr::null_mut(),
        tp_clear: std::ptr::null_mut(),
        tp_richcompare: std::ptr::null_mut(),
        tp_weaklistoffset: 0,
        tp_iter: std::ptr::null_mut(),
        tp_iternext: std::ptr::null_mut(),
        tp_methods: std::ptr::null_mut(),
        tp_members: std::ptr::null_mut(),
        tp_getset: std::ptr::null_mut(),
        tp_base: std::ptr::null_mut(),
        tp_dict: std::ptr::null_mut(),
        tp_descr_get: std::ptr::null_mut(),
        tp_descr_set: std::ptr::null_mut(),
        tp_dictoffset: 0,
        tp_init: std::ptr::null_mut(),
        tp_alloc: std::ptr::null_mut(),
        tp_new: std::ptr::null_mut(),
        tp_free: std::ptr::null_mut(),
        tp_is_gc: std::ptr::null_mut(),
        tp_bases: std::ptr::null_mut(),
        tp_mro: std::ptr::null_mut(),
        tp_cache: std::ptr::null_mut(),
        tp_subclasses: std::ptr::null_mut(),
        tp_weaklist: std::ptr::null_mut(),
        tp_del: std::ptr::null_mut(),
        tp_version_tag: 0,
        tp_finalize: std::ptr::null_mut(),
        tp_vectorcall: std::ptr::null_mut(),
        tp_watched: 0,
        tp_versions_used: 0,
    }
}

static PY_TYPE_NAME_OBJECT: &[u8; 7] = b"object\0";
static PY_TYPE_NAME_TYPE: &[u8; 5] = b"type\0";
static PY_TYPE_NAME_BOOL: &[u8; 5] = b"bool\0";
static PY_TYPE_NAME_BYTEARRAY: &[u8; 10] = b"bytearray\0";
static PY_TYPE_NAME_BYTEARRAY_ITER: &[u8; 19] = b"bytearray_iterator\0";
static PY_TYPE_NAME_BYTES: &[u8; 6] = b"bytes\0";
static PY_TYPE_NAME_BYTES_ITER: &[u8; 15] = b"bytes_iterator\0";
static PY_TYPE_NAME_CALL_ITER: &[u8; 18] = b"callable_iterator\0";
static PY_TYPE_NAME_CFUNCTION: &[u8; 27] = b"builtin_function_or_method\0";
static PY_TYPE_NAME_CAPSULE: &[u8; 8] = b"capsule\0";
static PY_TYPE_NAME_CLASSMETHOD_DESCR: &[u8; 18] = b"classmethod_descr\0";
static PY_TYPE_NAME_COMPLEX: &[u8; 8] = b"complex\0";
static PY_TYPE_NAME_DICT_PROXY: &[u8; 10] = b"dictproxy\0";
static PY_TYPE_NAME_DICT_ITEMS: &[u8; 11] = b"dict_items\0";
static PY_TYPE_NAME_DICT_ITER_ITEM: &[u8; 18] = b"dict_itemiterator\0";
static PY_TYPE_NAME_DICT_ITER_KEY: &[u8; 17] = b"dict_keyiterator\0";
static PY_TYPE_NAME_DICT_ITER_VALUE: &[u8; 19] = b"dict_valueiterator\0";
static PY_TYPE_NAME_DICT_KEYS: &[u8; 10] = b"dict_keys\0";
static PY_TYPE_NAME_DICT_REV_ITER_ITEM: &[u8; 25] = b"dict_reverseitemiterator\0";
static PY_TYPE_NAME_DICT_REV_ITER_KEY: &[u8; 24] = b"dict_reversekeyiterator\0";
static PY_TYPE_NAME_DICT_REV_ITER_VALUE: &[u8; 26] = b"dict_reversevalueiterator\0";
static PY_TYPE_NAME_DICT: &[u8; 5] = b"dict\0";
static PY_TYPE_NAME_DICT_VALUES: &[u8; 12] = b"dict_values\0";
static PY_TYPE_NAME_ELLIPSIS: &[u8; 9] = b"ellipsis\0";
static PY_TYPE_NAME_ENUM: &[u8; 5] = b"enum\0";
static PY_TYPE_NAME_FILTER: &[u8; 7] = b"filter\0";
static PY_TYPE_NAME_FLOAT: &[u8; 6] = b"float\0";
static PY_TYPE_NAME_FROZENSET: &[u8; 10] = b"frozenset\0";
static PY_TYPE_NAME_GETSET_DESCR: &[u8; 13] = b"getset_descr\0";
static PY_TYPE_NAME_GENERIC_ALIAS: &[u8; 19] = b"types.GenericAlias\0";
static PY_TYPE_NAME_LIST: &[u8; 5] = b"list\0";
static PY_TYPE_NAME_LIST_ITER: &[u8; 14] = b"list_iterator\0";
static PY_TYPE_NAME_LIST_REV_ITER: &[u8; 21] = b"list_reverseiterator\0";
static PY_TYPE_NAME_LONG: &[u8; 4] = b"int\0";
static PY_TYPE_NAME_LONG_RANGE_ITER: &[u8; 19] = b"longrange_iterator\0";
static PY_TYPE_NAME_MAP: &[u8; 4] = b"map\0";
static PY_TYPE_NAME_MEMBER_DESCR: &[u8; 13] = b"member_descr\0";
static PY_TYPE_NAME_MEMORYVIEW: &[u8; 11] = b"memoryview\0";
static PY_TYPE_NAME_METHOD: &[u8; 7] = b"method\0";
static PY_TYPE_NAME_METHOD_DESCR: &[u8; 13] = b"method_descr\0";
static PY_TYPE_NAME_MODULE_DEF: &[u8; 10] = b"moduledef\0";
static PY_TYPE_NAME_MODULE: &[u8; 7] = b"module\0";
static PY_TYPE_NAME_NONE: &[u8; 9] = b"NoneType\0";
static PY_TYPE_NAME_PROPERTY: &[u8; 9] = b"property\0";
static PY_TYPE_NAME_RANGE_ITER: &[u8; 15] = b"range_iterator\0";
static PY_TYPE_NAME_RANGE: &[u8; 6] = b"range\0";
static PY_TYPE_NAME_REVERSED: &[u8; 9] = b"reversed\0";
static PY_TYPE_NAME_SEQ_ITER: &[u8; 9] = b"iterator\0";
static PY_TYPE_NAME_SET: &[u8; 4] = b"set\0";
static PY_TYPE_NAME_SET_ITER: &[u8; 13] = b"set_iterator\0";
static PY_TYPE_NAME_SLICE: &[u8; 6] = b"slice\0";
static PY_TYPE_NAME_SUPER: &[u8; 6] = b"super\0";
static PY_TYPE_NAME_TRACEBACK: &[u8; 10] = b"traceback\0";
static PY_TYPE_NAME_TUPLE: &[u8; 6] = b"tuple\0";
static PY_TYPE_NAME_TUPLE_ITER: &[u8; 15] = b"tuple_iterator\0";
static PY_TYPE_NAME_UNICODE: &[u8; 4] = b"str\0";
static PY_TYPE_NAME_UNICODE_ITER: &[u8; 13] = b"str_iterator\0";
static PY_TYPE_NAME_WEAKREF_CALLABLE_PROXY: &[u8; 26] = b"weakref.CallableProxyType\0";
static PY_TYPE_NAME_WEAKREF_PROXY: &[u8; 18] = b"weakref.ProxyType\0";
static PY_TYPE_NAME_WEAKREF_REF: &[u8; 22] = b"weakref.ReferenceType\0";
static PY_TYPE_NAME_WRAPPER_DESCR: &[u8; 19] = b"wrapper_descriptor\0";
static PY_TYPE_NAME_ZIP: &[u8; 4] = b"zip\0";

#[unsafe(no_mangle)]
#[used]
pub static mut PyBaseObject_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_OBJECT.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyType_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_TYPE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyBool_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_BOOL.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyByteArrayIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_BYTEARRAY_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyByteArray_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_BYTEARRAY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyBytesIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_BYTES_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyBytes_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_BYTES.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyCallIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_CALL_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyCFunction_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_CFUNCTION.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyCapsule_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_CAPSULE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyClassMethodDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_CLASSMETHOD_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyComplex_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_COMPLEX.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictItems_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_ITEMS.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictIterItem_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_ITER_ITEM.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictIterKey_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_ITER_KEY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictIterValue_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_ITER_VALUE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictKeys_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_KEYS.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictProxy_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_PROXY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictRevIterItem_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_REV_ITER_ITEM.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictRevIterKey_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_REV_ITER_KEY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictRevIterValue_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_REV_ITER_VALUE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDict_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_DICT.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictValues_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_VALUES.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyEllipsis_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_ELLIPSIS.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyEnum_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_ENUM.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyFilter_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_FILTER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyFloat_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_FLOAT.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyFrozenSet_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_FROZENSET.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyGetSetDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_GETSET_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut Py_GenericAliasType: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_GENERIC_ALIAS.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyList_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_LIST.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyListIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_LIST_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyListRevIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_LIST_REV_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyLong_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_LONG.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyLongRangeIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_LONG_RANGE_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyMap_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_MAP.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyMemberDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_MEMBER_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyMemoryView_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_MEMORYVIEW.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyMethod_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_METHOD.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyMethodDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_METHOD_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyModuleDef_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_MODULE_DEF.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyModule_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_MODULE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyNone_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_NONE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyProperty_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_PROPERTY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyRangeIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_RANGE_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyRange_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_RANGE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyReversed_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_REVERSED.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySeqIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_SEQ_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySet_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_SET.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySetIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_SET_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySlice_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_SLICE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySuper_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_SUPER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyTraceBack_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_TRACEBACK.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyTuple_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_TUPLE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyTupleIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_TUPLE_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyUnicode_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_UNICODE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyUnicodeIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_UNICODE_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut _PyWeakref_CallableProxyType: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_WEAKREF_CALLABLE_PROXY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut _PyWeakref_ProxyType: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_WEAKREF_PROXY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut _PyWeakref_RefType: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_WEAKREF_REF.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyWrapperDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_WRAPPER_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyZip_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_ZIP.as_ptr().cast());

unsafe extern "C" {
    fn PyTuple_Pack(size: isize, ...) -> *mut c_void;
    fn Py_BuildValue(format: *const c_char, ...) -> *mut c_void;
    fn _Py_BuildValue_SizeT(format: *const c_char, ...) -> *mut c_void;
    fn Py_VaBuildValue(format: *const c_char, vargs: *mut c_void) -> *mut c_void;
    fn _Py_VaBuildValue_SizeT(format: *const c_char, vargs: *mut c_void) -> *mut c_void;
    fn PyUnicode_FromFormat(format: *const c_char, ...) -> *mut c_void;
    fn PyUnicode_FromFormatV(format: *const c_char, vargs: *mut c_void) -> *mut c_void;
    fn PyBytes_FromFormat(format: *const c_char, ...) -> *mut c_void;
    fn PyBytes_FromFormatV(format: *const c_char, vargs: *mut c_void) -> *mut c_void;
    fn PyObject_CallFunction(callable: *mut c_void, format: *const c_char, ...) -> *mut c_void;
    fn _PyObject_CallFunction_SizeT(
        callable: *mut c_void,
        format: *const c_char,
        ...
    ) -> *mut c_void;
    fn PyObject_CallFunctionObjArgs(callable: *mut c_void, ...) -> *mut c_void;
    fn PyObject_CallMethod(
        object: *mut c_void,
        method: *const c_char,
        format: *const c_char,
        ...
    ) -> *mut c_void;
    fn _PyObject_CallMethod_SizeT(
        object: *mut c_void,
        method: *const c_char,
        format: *const c_char,
        ...
    ) -> *mut c_void;
    fn PyEval_CallFunction(callable: *mut c_void, format: *const c_char, ...) -> *mut c_void;
    fn PyEval_CallMethod(
        object: *mut c_void,
        method: *const c_char,
        format: *const c_char,
        ...
    ) -> *mut c_void;
    fn PyObject_CallMethodObjArgs(object: *mut c_void, method: *mut c_void, ...) -> *mut c_void;
    fn PyArg_Parse(args: *mut c_void, format: *const c_char, ...) -> i32;
    fn _PyArg_Parse_SizeT(args: *mut c_void, format: *const c_char, ...) -> i32;
    fn PyArg_VaParse(args: *mut c_void, format: *const c_char, vargs: *mut c_void) -> i32;
    fn _PyArg_VaParse_SizeT(args: *mut c_void, format: *const c_char, vargs: *mut c_void) -> i32;
    fn PyArg_ValidateKeywordArguments(kwargs: *mut c_void) -> i32;
    fn PyArg_ParseTuple(args: *mut c_void, format: *const c_char, ...) -> i32;
    fn _PyArg_ParseTuple_SizeT(args: *mut c_void, format: *const c_char, ...) -> i32;
    fn PyArg_ParseTupleAndKeywords(
        args: *mut c_void,
        kwargs: *mut c_void,
        format: *const c_char,
        keywords: *mut *const c_char,
        ...
    ) -> i32;
    fn _PyArg_ParseTupleAndKeywords_SizeT(
        args: *mut c_void,
        kwargs: *mut c_void,
        format: *const c_char,
        keywords: *mut *const c_char,
        ...
    ) -> i32;
    fn PyArg_VaParseTupleAndKeywords(
        args: *mut c_void,
        kwargs: *mut c_void,
        format: *const c_char,
        keywords: *mut *const c_char,
        vargs: *mut c_void,
    ) -> i32;
    fn _PyArg_VaParseTupleAndKeywords_SizeT(
        args: *mut c_void,
        kwargs: *mut c_void,
        format: *const c_char,
        keywords: *mut *const c_char,
        vargs: *mut c_void,
    ) -> i32;
    fn PyErr_Format(exception: *mut c_void, format: *const c_char, ...) -> *mut c_void;
    fn PyErr_FormatV(
        exception: *mut c_void,
        format: *const c_char,
        vargs: *mut c_void,
    ) -> *mut c_void;
    fn PyOS_BeforeFork();
    fn PyOS_AfterFork_Parent();
    fn PyOS_AfterFork_Child();
    fn PyOS_AfterFork();
    fn PyOS_CheckStack() -> c_int;
    fn PyOS_FSPath(path: *mut c_void) -> *mut c_void;
    fn PyOS_InterruptOccurred() -> c_int;
    fn PyOS_double_to_string(
        value: c_double,
        format_code: c_char,
        precision: c_int,
        flags: c_int,
        out_type: *mut c_int,
    ) -> *mut c_char;
    fn PyOS_getsig(sig: c_int) -> *mut c_void;
    fn PyOS_setsig(sig: c_int, handler: *mut c_void) -> *mut c_void;
    fn PyOS_mystricmp(left: *const c_char, right: *const c_char) -> c_int;
    fn PyOS_mystrnicmp(left: *const c_char, right: *const c_char, size: isize) -> c_int;
    fn PyOS_vsnprintf(
        buffer: *mut c_char,
        size: usize,
        format: *const c_char,
        vargs: *mut c_void,
    ) -> c_int;
    fn PySys_WriteStdout(format: *const c_char, ...);
    fn PySys_WriteStderr(format: *const c_char, ...);
    fn PySys_FormatStdout(format: *const c_char, ...);
    fn PySys_FormatStderr(format: *const c_char, ...);
    fn PySys_Audit(event: *const c_char, format: *const c_char, ...) -> i32;
}

#[used]
static KEEP2_PYLONG_FROMSSIZE_T: unsafe extern "C" fn(isize) -> *mut c_void = PyLong_FromSsize_t;
#[used]
static KEEP2_PYLONG_FROMSIZE_T: unsafe extern "C" fn(usize) -> *mut c_void = PyLong_FromSize_t;
#[used]
static KEEP2_PYLONG_FROMINT32: unsafe extern "C" fn(i32) -> *mut c_void = PyLong_FromInt32;
#[used]
static KEEP2_PYLONG_FROMUINT32: unsafe extern "C" fn(u32) -> *mut c_void = PyLong_FromUInt32;
#[used]
static KEEP2_PYLONG_FROMINT64: unsafe extern "C" fn(i64) -> *mut c_void = PyLong_FromInt64;
#[used]
static KEEP2_PYLONG_FROMUINT64: unsafe extern "C" fn(u64) -> *mut c_void = PyLong_FromUInt64;
#[used]
static KEEP2_PYLONG_FROMUNSIGNEDLONG: unsafe extern "C" fn(u64) -> *mut c_void =
    PyLong_FromUnsignedLong;
#[used]
static KEEP2_PYLONG_FROMUNSIGNEDLONGLONG: unsafe extern "C" fn(u64) -> *mut c_void =
    PyLong_FromUnsignedLongLong;
#[used]
static KEEP2_PYLONG_FROMVOIDPTR: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyLong_FromVoidPtr;
#[used]
static KEEP2_PYLONG_FROMUNICODEOBJECT: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    PyLong_FromUnicodeObject;
#[used]
static KEEP2_PYLONG_FROMSTRING: unsafe extern "C" fn(
    *const c_char,
    *mut *mut c_char,
    i32,
) -> *mut c_void = PyLong_FromString;
#[used]
static KEEP2_PYLONG_GETINFO: unsafe extern "C" fn() -> *mut c_void = PyLong_GetInfo;
#[used]
static KEEP2_PYMODULE_GETDICT: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyModule_GetDict;
#[used]
static KEEP2_PYMODULE_NEWOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyModule_NewObject;
#[used]
static KEEP2_PYMODULE_NEW: unsafe extern "C" fn(*const c_char) -> *mut c_void = PyModule_New;
#[used]
static KEEP2_PYMODULE_GETNAMEOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyModule_GetNameObject;
#[used]
static KEEP2_PYMODULE_GETNAME: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    PyModule_GetName;
#[used]
static KEEP2_PYMODULE_GETFILENAMEOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyModule_GetFilenameObject;
#[used]
static KEEP2_PYMODULE_GETFILENAME: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    PyModule_GetFilename;
#[used]
static KEEP2_PYMODULE_SETDOCSTRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyModule_SetDocString;
#[used]
static KEEP2_PYMODULE_ADD: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> i32 =
    PyModule_Add;
#[used]
static KEEP2_PYMODULE_ADDFUNCTIONS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyModule_AddFunctions;
#[used]
static KEEP2_PYMODULE_ADDTYPE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyModule_AddType;
#[used]
static KEEP2_PYMODULE_FROMDEFANDSPEC2: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    i32,
) -> *mut c_void = PyModule_FromDefAndSpec2;
#[used]
static KEEP2_PYMODULE_EXECDEF: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyModule_ExecDef;
#[used]
static KEEP2_PYMODULE_GETDEF: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyModule_GetDef;
#[used]
static KEEP2_PYMODULE_GETSTATE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyModule_GetState;
#[used]
static KEEP2_PYSTRUCTSEQUENCE_NEWTYPE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyStructSequence_NewType;
#[used]
static KEEP2_PYSTRUCTSEQUENCE_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyStructSequence_New;
#[used]
static KEEP2_PYSTRUCTSEQUENCE_SETITEM: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) =
    PyStructSequence_SetItem;
#[used]
static KEEP2_PYSTRUCTSEQUENCE_GETITEM: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    PyStructSequence_GetItem;
#[used]
static KEEP2_PYTUPLE_NEW: unsafe extern "C" fn(isize) -> *mut c_void = PyTuple_New;
#[used]
static KEEP2_PYTUPLE_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PyTuple_Size;
#[used]
static KEEP2_PYTUPLE_GETITEM: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    PyTuple_GetItem;
#[used]
static KEEP2_PYTUPLE_SETITEM: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) -> i32 =
    PyTuple_SetItem;
#[used]
static KEEP2_PYTUPLE_GETSLICE: unsafe extern "C" fn(*mut c_void, isize, isize) -> *mut c_void =
    PyTuple_GetSlice;
#[used]
static KEEP2_PYTUPLE_PACK: unsafe extern "C" fn(isize, ...) -> *mut c_void = PyTuple_Pack;
#[used]
static KEEP2_PYOBJECT_GETITEM: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyObject_GetItem;
#[used]
static KEEP2_PYOBJECT_SETITEM: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
    PyObject_SetItem;
#[used]
static KEEP2_PYOBJECT_DELITEM: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyObject_DelItem;
#[used]
static KEEP2_PYOBJECT_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PyObject_Size;
#[used]
static KEEP2_PYOBJECT_LENGTH: unsafe extern "C" fn(*mut c_void) -> isize = PyObject_Length;
#[used]
static KEEP2_PYOBJECT_LENGTHHINT: unsafe extern "C" fn(*mut c_void, isize) -> isize =
    PyObject_LengthHint;
#[used]
static KEEP2_PYOBJECT_TYPE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_Type;
#[used]
static KEEP2__PYOBJECT_TYPE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = _PyObject_Type;
#[used]
static KEEP2_PYOBJECT_GETTYPEDATA: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyObject_GetTypeData;
#[used]
static KEEP2_PYOBJECT_HASH: unsafe extern "C" fn(*mut c_void) -> isize = PyObject_Hash;
#[used]
static KEEP2_PYOBJECT_HASHNOTIMPLEMENTED: unsafe extern "C" fn(*mut c_void) -> isize =
    PyObject_HashNotImplemented;
#[used]
static KEEP2_PYOBJECT_RICHCOMPARE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    i32,
) -> *mut c_void = PyObject_RichCompare;
#[used]
static KEEP2_PYOBJECT_RICHCOMPAREBOOL: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 =
    PyObject_RichCompareBool;
#[used]
static KEEP2_PYOBJECT_ISINSTANCE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyObject_IsInstance;
#[used]
static KEEP2_PYOBJECT_ISSUBCLASS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyObject_IsSubclass;
#[used]
static KEEP2_PYOBJECT_GETOPTIONALATTR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32 = PyObject_GetOptionalAttr;
#[used]
static KEEP2_PYSEQUENCE_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = PySequence_Check;
#[used]
static KEEP2_PYSEQUENCE_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PySequence_Size;
#[used]
static KEEP2_PYSEQUENCE_LENGTH: unsafe extern "C" fn(*mut c_void) -> isize = PySequence_Length;
#[used]
static KEEP2_PYSEQUENCE_GETITEM: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    PySequence_GetItem;
#[used]
static KEEP2_PYSEQUENCE_GETSLICE: unsafe extern "C" fn(*mut c_void, isize, isize) -> *mut c_void =
    PySequence_GetSlice;
#[used]
static KEEP2_PYSEQUENCE_SETITEM: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) -> i32 =
    PySequence_SetItem;
#[used]
static KEEP2_PYSEQUENCE_DELITEM: unsafe extern "C" fn(*mut c_void, isize) -> i32 =
    PySequence_DelItem;
#[used]
static KEEP2_PYSEQUENCE_SETSLICE: unsafe extern "C" fn(
    *mut c_void,
    isize,
    isize,
    *mut c_void,
) -> i32 = PySequence_SetSlice;
#[used]
static KEEP2_PYSEQUENCE_DELSLICE: unsafe extern "C" fn(*mut c_void, isize, isize) -> i32 =
    PySequence_DelSlice;
#[used]
static KEEP2_PYSEQUENCE_CONTAINS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PySequence_Contains;
#[used]
static KEEP2_PYSEQUENCE_IN: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 = PySequence_In;
#[used]
static KEEP2_PYSEQUENCE_TUPLE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PySequence_Tuple;
#[used]
static KEEP2_PYSEQUENCE_LIST: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PySequence_List;
#[used]
static KEEP2_PYSEQUENCE_FAST: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
    PySequence_Fast;
#[used]
static KEEP2_PYSEQUENCE_CONCAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PySequence_Concat;
#[used]
static KEEP2_PYSEQUENCE_INPLACECONCAT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PySequence_InPlaceConcat;
#[used]
static KEEP2_PYSEQUENCE_REPEAT: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    PySequence_Repeat;
#[used]
static KEEP2_PYSEQUENCE_INPLACEREPEAT: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    PySequence_InPlaceRepeat;
#[used]
static KEEP2_PYSEQUENCE_COUNT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize =
    PySequence_Count;
#[used]
static KEEP2_PYSEQUENCE_INDEX: unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize =
    PySequence_Index;
#[used]
static KEEP2_PYMAPPING_GETITEMSTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyMapping_GetItemString;
#[used]
static KEEP2_PYMAPPING_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = PyMapping_Check;
#[used]
static KEEP2_PYMAPPING_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PyMapping_Size;
#[used]
static KEEP2_PYMAPPING_LENGTH: unsafe extern "C" fn(*mut c_void) -> isize = PyMapping_Length;
#[used]
static KEEP2_PYMAPPING_KEYS: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyMapping_Keys;
#[used]
static KEEP2_PYMAPPING_ITEMS: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyMapping_Items;
#[used]
static KEEP2_PYMAPPING_VALUES: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyMapping_Values;
#[used]
static KEEP2_PYMAPPING_GETOPTIONALITEM: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32 = PyMapping_GetOptionalItem;
#[used]
static KEEP2_PYMAPPING_GETOPTIONALITEMSTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut *mut c_void,
) -> i32 = PyMapping_GetOptionalItemString;
#[used]
static KEEP2_PYMAPPING_SETITEMSTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = PyMapping_SetItemString;
#[used]
static KEEP2_PYMAPPING_HASKEYWITHERROR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyMapping_HasKeyWithError;
#[used]
static KEEP2_PYMAPPING_HASKEYSTRINGWITHERROR: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> i32 = PyMapping_HasKeyStringWithError;
#[used]
static KEEP2_PYMAPPING_HASKEY: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyMapping_HasKey;
#[used]
static KEEP2_PYMAPPING_HASKEYSTRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyMapping_HasKeyString;
#[used]
static KEEP2_PYSEQITER_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PySeqIter_New;
#[used]
static KEEP2_PYCALLITER_NEW: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyCallIter_New;
#[used]
static KEEP2_PYOBJECT_ASFILEDESCRIPTOR: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyObject_AsFileDescriptor;
#[used]
static KEEP2_PYOBJECT_CHECKBUFFER: unsafe extern "C" fn(*mut c_void) -> i32 = PyObject_CheckBuffer;
#[used]
static KEEP2_PYOBJECT_CHECKREADBUFFER: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyObject_CheckReadBuffer;
#[used]
static KEEP2_PYOBJECT_ASREADBUFFER: unsafe extern "C" fn(
    *mut c_void,
    *mut *const c_void,
    *mut isize,
) -> i32 = PyObject_AsReadBuffer;
#[used]
static KEEP2_PYOBJECT_ASWRITEBUFFER: unsafe extern "C" fn(
    *mut c_void,
    *mut *mut c_void,
    *mut isize,
) -> i32 = PyObject_AsWriteBuffer;
#[used]
static KEEP2_PYOBJECT_ASCHARBUFFER: unsafe extern "C" fn(
    *mut c_void,
    *mut *const c_char,
    *mut isize,
) -> i32 = PyObject_AsCharBuffer;
#[used]
static KEEP2_PYOBJECT_COPYDATA: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyObject_CopyData;
#[used]
static KEEP2_PYMEMORYVIEW_FROMOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyMemoryView_FromObject;
#[used]
static KEEP2_PYMEMORYVIEW_FROMMEMORY: unsafe extern "C" fn(*mut c_char, isize, i32) -> *mut c_void =
    PyMemoryView_FromMemory;
#[used]
static KEEP2_PYMEMORYVIEW_FROMBUFFER: unsafe extern "C" fn(*const CpythonBuffer) -> *mut c_void =
    PyMemoryView_FromBuffer;
#[used]
static KEEP2_PYMEMORYVIEW_GETCONTIGUOUS: unsafe extern "C" fn(
    *mut c_void,
    i32,
    c_char,
) -> *mut c_void = PyMemoryView_GetContiguous;
#[used]
static KEEP2_PYOBJECT_GETBUFFER: unsafe extern "C" fn(*mut c_void, *mut CpythonBuffer, i32) -> i32 =
    PyObject_GetBuffer;
#[used]
static KEEP2_PYBUFFER_ISCONTIGUOUS: unsafe extern "C" fn(*const CpythonBuffer, c_char) -> i32 =
    PyBuffer_IsContiguous;
#[used]
static KEEP2_PYBUFFER_GETPOINTER: unsafe extern "C" fn(
    *const CpythonBuffer,
    *const isize,
) -> *mut c_void = PyBuffer_GetPointer;
#[used]
static KEEP2_PYBUFFER_SIZEFROMFORMAT: unsafe extern "C" fn(*const c_char) -> isize =
    PyBuffer_SizeFromFormat;
#[used]
static KEEP2_PYBUFFER_FROMCONTIGUOUS: unsafe extern "C" fn(
    *const CpythonBuffer,
    *const c_void,
    isize,
    c_char,
) -> i32 = PyBuffer_FromContiguous;
#[used]
static KEEP2_PYBUFFER_TOCONTIGUOUS: unsafe extern "C" fn(
    *mut c_void,
    *const CpythonBuffer,
    isize,
    c_char,
) -> i32 = PyBuffer_ToContiguous;
#[used]
static KEEP2_PYBUFFER_FILLCONTIGUOUSSTRIDES: unsafe extern "C" fn(
    i32,
    *const isize,
    *mut isize,
    i32,
    c_char,
) = PyBuffer_FillContiguousStrides;
#[used]
static KEEP2_PYBUFFER_FILLINFO: unsafe extern "C" fn(
    *mut CpythonBuffer,
    *mut c_void,
    *mut c_void,
    isize,
    i32,
    i32,
) -> i32 = PyBuffer_FillInfo;
#[used]
static KEEP2_PYOBJECT_CALLMETHODOBJARGS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    ...
) -> *mut c_void = PyObject_CallMethodObjArgs;
#[used]
static KEEP2_PYOBJECT_PRINT: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 =
    PyObject_Print;
#[used]
static KEEP2_PYTYPE_GETFLAGS: unsafe extern "C" fn(*mut c_void) -> usize = PyType_GetFlags;
#[used]
static KEEP2_PYTYPE_ISSUBTYPE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyType_IsSubtype;
#[used]
static KEEP2_PYTYPE_READY: unsafe extern "C" fn(*mut c_void) -> i32 = PyType_Ready;
#[used]
static KEEP2_PYTYPE_GENERICALLOC: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    PyType_GenericAlloc;
#[used]
static KEEP2_PYTYPE_GENERICNEW: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyType_GenericNew;
#[used]
static KEEP2_PYTYPE_FROMMETACLASS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyType_FromMetaclass;
#[used]
static KEEP2_PYTYPE_FROMMODULEANDSPEC: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyType_FromModuleAndSpec;
#[used]
static KEEP2_PYTYPE_FROMSPECWITHBASES: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyType_FromSpecWithBases;
#[used]
static KEEP2_PYTYPE_FROMSPEC: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyType_FromSpec;
#[used]
static KEEP2_PYTYPE_GETNAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyType_GetName;
#[used]
static KEEP2_PYTYPE_GETQUALNAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyType_GetQualName;
#[used]
static KEEP2_PYTYPE_GETMODULENAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyType_GetModuleName;
#[used]
static KEEP2_PYTYPE_GETFULLYQUALIFIEDNAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyType_GetFullyQualifiedName;
#[used]
static KEEP2_PYTYPE_GETSLOT: unsafe extern "C" fn(*mut c_void, c_int) -> *mut c_void =
    PyType_GetSlot;
#[used]
static KEEP2__PYTYPE_LOOKUP: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    _PyType_Lookup;
#[used]
static KEEP2_PYTYPE_GETMODULE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyType_GetModule;
#[used]
static KEEP2_PYTYPE_GETMODULESTATE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyType_GetModuleState;
#[used]
static KEEP2_PYTYPE_GETMODULEBYDEF: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyType_GetModuleByDef;
#[used]
static KEEP2_PYTYPE_GETTYPEDATASIZE: unsafe extern "C" fn(*mut c_void) -> isize =
    PyType_GetTypeDataSize;
#[used]
static KEEP2_PYTYPE_GETBASEBYTOKEN: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> c_int = PyType_GetBaseByToken;
#[used]
static KEEP2_PYTYPE_CLEARCACHE: unsafe extern "C" fn() -> c_uint = PyType_ClearCache;
#[used]
static KEEP2_PYTYPE_MODIFIED: unsafe extern "C" fn(*mut c_void) = PyType_Modified;
#[used]
static KEEP2_PYTYPE_FREEZE: unsafe extern "C" fn(*mut c_void) -> c_int = PyType_Freeze;
#[used]
static KEEP2_PYOBJECT_MALLOC: unsafe extern "C" fn(usize) -> *mut c_void = PyObject_Malloc;
#[used]
static KEEP2_PYOBJECT_CALLOC: unsafe extern "C" fn(usize, usize) -> *mut c_void = PyObject_Calloc;
#[used]
static KEEP2_PYOBJECT_REALLOC: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void =
    PyObject_Realloc;
#[used]
static KEEP2_PYOBJECT_FREE: unsafe extern "C" fn(*mut c_void) = PyObject_Free;
#[used]
static KEEP2_PYOBJECT_GC_TRACK: unsafe extern "C" fn(*mut c_void) = PyObject_GC_Track;
#[used]
static KEEP2_PYOBJECT_GC_UNTRACK: unsafe extern "C" fn(*mut c_void) = PyObject_GC_UnTrack;
#[used]
static KEEP2_PYOBJECT_GC_ISTRACKED: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyObject_GC_IsTracked;
#[used]
static KEEP2_PYOBJECT_GC_ISFINALIZED: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyObject_GC_IsFinalized;
#[used]
static KEEP2_PYOBJECT_GC_DEL: unsafe extern "C" fn(*mut c_void) = PyObject_GC_Del;
#[used]
static KEEP2_PYGC_COLLECT: unsafe extern "C" fn() -> isize = PyGC_Collect;
#[used]
static KEEP2_PYGC_ENABLE: unsafe extern "C" fn() -> i32 = PyGC_Enable;
#[used]
static KEEP2_PYGC_DISABLE: unsafe extern "C" fn() -> i32 = PyGC_Disable;
#[used]
static KEEP2_PYGC_IS_ENABLED: unsafe extern "C" fn() -> i32 = PyGC_IsEnabled;
#[used]
static KEEP2_PYOBJECT_CLEAR_WEAKREFS: unsafe extern "C" fn(*mut c_void) = PyObject_ClearWeakRefs;
#[used]
static KEEP2_PYWEAKREF_NEWREF: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyWeakref_NewRef;
#[used]
static KEEP2_PYWEAKREF_NEWPROXY: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyWeakref_NewProxy;
#[used]
static KEEP2_PYWEAKREF_GETREF: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> c_int =
    PyWeakref_GetRef;
#[used]
static KEEP2_PYWEAKREF_GETOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyWeakref_GetObject;
#[used]
static KEEP2_PY_ADDPENDINGCALL: unsafe extern "C" fn(
    Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
    *mut c_void,
) -> c_int = Py_AddPendingCall;
#[used]
static KEEP2_PY_MAKEPENDINGCALLS: unsafe extern "C" fn() -> c_int = Py_MakePendingCalls;
#[used]
static KEEP2_PY_ATEXIT: unsafe extern "C" fn(Option<unsafe extern "C" fn()>) -> c_int = Py_AtExit;
#[used]
static KEEP2_PY_GETRECURSIONLIMIT: unsafe extern "C" fn() -> c_int = Py_GetRecursionLimit;
#[used]
static KEEP2_PY_SETRECURSIONLIMIT: unsafe extern "C" fn(c_int) = Py_SetRecursionLimit;
#[used]
static KEEP2_PY_GETVERSION: unsafe extern "C" fn() -> *const c_char = Py_GetVersion;
#[used]
static KEEP2_PY_GETBUILDINFO: unsafe extern "C" fn() -> *const c_char = Py_GetBuildInfo;
#[used]
static KEEP2_PY_GETCOMPILER: unsafe extern "C" fn() -> *const c_char = Py_GetCompiler;
#[used]
static KEEP2_PY_GETPLATFORM: unsafe extern "C" fn() -> *const c_char = Py_GetPlatform;
#[used]
static KEEP2_PY_GETCOPYRIGHT: unsafe extern "C" fn() -> *const c_char = Py_GetCopyright;
#[used]
static KEEP2_PY_GETARGCARGV: unsafe extern "C" fn(*mut c_int, *mut *mut *mut Cwchar) =
    Py_GetArgcArgv;
#[used]
static KEEP2_PY_SETPROGRAMNAME: unsafe extern "C" fn(*const Cwchar) = Py_SetProgramName;
#[used]
static KEEP2_PY_GETPROGRAMNAME: unsafe extern "C" fn() -> *mut Cwchar = Py_GetProgramName;
#[used]
static KEEP2_PY_SETPYTHONHOME: unsafe extern "C" fn(*const Cwchar) = Py_SetPythonHome;
#[used]
static KEEP2_PY_GETPYTHONHOME: unsafe extern "C" fn() -> *mut Cwchar = Py_GetPythonHome;
#[used]
static KEEP2_PY_SETPATH: unsafe extern "C" fn(*const Cwchar) = Py_SetPath;
#[used]
static KEEP2_PY_GETPATH: unsafe extern "C" fn() -> *mut Cwchar = Py_GetPath;
#[used]
static KEEP2_PY_GETPREFIX: unsafe extern "C" fn() -> *mut Cwchar = Py_GetPrefix;
#[used]
static KEEP2_PY_GETEXECPREFIX: unsafe extern "C" fn() -> *mut Cwchar = Py_GetExecPrefix;
#[used]
static KEEP2_PY_GETPROGRAMFULLPATH: unsafe extern "C" fn() -> *mut Cwchar = Py_GetProgramFullPath;
#[used]
static KEEP2_PY_ENCODELOCALE: unsafe extern "C" fn(*const Cwchar, *mut usize) -> *mut c_char =
    Py_EncodeLocale;
#[used]
static KEEP2_PY_DECODELOCALE: unsafe extern "C" fn(*const c_char, *mut usize) -> *mut Cwchar =
    Py_DecodeLocale;
#[used]
static KEEP2_PY_PACK_FULL_VERSION: unsafe extern "C" fn(c_int, c_int, c_int, c_int, c_int) -> u32 =
    Py_PACK_FULL_VERSION;
#[used]
static KEEP2_PY_PACK_VERSION: unsafe extern "C" fn(c_int, c_int) -> u32 = Py_PACK_VERSION;
#[used]
static KEEP2_PY_INITIALIZE: unsafe extern "C" fn() = Py_Initialize;
#[used]
static KEEP2_PY_INITIALIZEEX: unsafe extern "C" fn(c_int) = Py_InitializeEx;
#[used]
static KEEP2_PY_MAIN: unsafe extern "C" fn(c_int, *mut *mut Cwchar) -> c_int = Py_Main;
#[used]
static KEEP2_PY_BYTESMAIN: unsafe extern "C" fn(c_int, *mut *mut c_char) -> c_int = Py_BytesMain;
#[used]
static KEEP2_PY_COMPILESTRING: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    c_int,
) -> *mut c_void = Py_CompileString;
#[used]
static KEEP2_PY_FINALIZE: unsafe extern "C" fn() = Py_Finalize;
#[used]
static KEEP2_PY_FINALIZEEX: unsafe extern "C" fn() -> c_int = Py_FinalizeEx;
#[used]
static KEEP2_PY_EXIT: unsafe extern "C" fn(c_int) = Py_Exit;
#[used]
static KEEP2_PY_FATALERROR: unsafe extern "C" fn(*const c_char) = Py_FatalError;
#[used]
static KEEP2_PY_FATALERRORFUNC: unsafe extern "C" fn(*const c_char, *const c_char) =
    _Py_FatalErrorFunc;
#[used]
static KEEP2_PY_NEWINTERPRETER: unsafe extern "C" fn() -> *mut c_void = Py_NewInterpreter;
#[used]
static KEEP2_PY_ENDINTERPRETER: unsafe extern "C" fn(*mut c_void) = Py_EndInterpreter;
#[used]
static KEEP2_PY_ISFINALIZING: unsafe extern "C" fn() -> c_int = Py_IsFinalizing;
#[used]
static KEEP2_PY_REPRENTER: unsafe extern "C" fn(*mut c_void) -> c_int = Py_ReprEnter;
#[used]
static KEEP2_PY_REPRLEAVE: unsafe extern "C" fn(*mut c_void) = Py_ReprLeave;
#[used]
static KEEP2_PYOBJECT_INIT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyObject_Init;
#[used]
static KEEP2_PYOBJECT_INITVAR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
) -> *mut c_void = PyObject_InitVar;
#[used]
static KEEP2_PYSLICE_NEW: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PySlice_New;
#[used]
static KEEP2_PYSLICE_UNPACK: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
    *mut isize,
    *mut isize,
) -> i32 = PySlice_Unpack;
#[used]
static KEEP2_PYSLICE_ADJUSTINDICES: unsafe extern "C" fn(
    isize,
    *mut isize,
    *mut isize,
    isize,
) -> isize = PySlice_AdjustIndices;
#[used]
static KEEP2_PYSLICE_GETINDICES: unsafe extern "C" fn(
    *mut c_void,
    isize,
    *mut isize,
    *mut isize,
    *mut isize,
) -> i32 = PySlice_GetIndices;
#[used]
static KEEP2_PYSLICE_GETINDICESEX: unsafe extern "C" fn(
    *mut c_void,
    isize,
    *mut isize,
    *mut isize,
    *mut isize,
    *mut isize,
) -> i32 = PySlice_GetIndicesEx;
#[used]
static KEEP2_PYSYS_GETOBJECT: unsafe extern "C" fn(*const c_char) -> *mut c_void = PySys_GetObject;
#[used]
static KEEP2_PYSYS_SETOBJECT: unsafe extern "C" fn(*const c_char, *mut c_void) -> i32 =
    PySys_SetObject;
#[used]
static KEEP2_PYSYS_GETXOPTIONS: unsafe extern "C" fn() -> *mut c_void = PySys_GetXOptions;
#[used]
static KEEP2_PYSYS_ADDXOPTION: unsafe extern "C" fn(*const Cwchar) = PySys_AddXOption;
#[used]
static KEEP2_PYSYS_HASWARNOPTIONS: unsafe extern "C" fn() -> i32 = PySys_HasWarnOptions;
#[used]
static KEEP2_PYSYS_RESETWARNOPTIONS: unsafe extern "C" fn() = PySys_ResetWarnOptions;
#[used]
static KEEP2_PYSYS_ADDWARNOPTION: unsafe extern "C" fn(*const Cwchar) = PySys_AddWarnOption;
#[used]
static KEEP2_PYSYS_ADDWARNOPTIONUNICODE: unsafe extern "C" fn(*const Cwchar) =
    PySys_AddWarnOptionUnicode;
#[used]
static KEEP2_PYSYS_AUDITTUPLE: unsafe extern "C" fn(*const c_char, *mut c_void) -> i32 =
    PySys_AuditTuple;
#[used]
static KEEP2_PYSYS_SETARGV: unsafe extern "C" fn(i32, *mut *mut Cwchar) = PySys_SetArgv;
#[used]
static KEEP2_PYSYS_SETARGVEX: unsafe extern "C" fn(i32, *mut *mut Cwchar, i32) = PySys_SetArgvEx;
#[used]
static KEEP2_PYSYS_SETPATH: unsafe extern "C" fn(*const Cwchar) = PySys_SetPath;
#[used]
static KEEP2_PYTHREAD_INIT_THREAD: unsafe extern "C" fn() = PyThread_init_thread;
#[used]
static KEEP2_PYTHREAD_START_NEW_THREAD: unsafe extern "C" fn(
    Option<unsafe extern "C" fn(*mut c_void)>,
    *mut c_void,
) -> c_ulong = PyThread_start_new_thread;
#[used]
static KEEP2_PYTHREAD_EXIT_THREAD: unsafe extern "C" fn() = PyThread_exit_thread;
#[used]
static KEEP2_PYTHREAD_GET_THREAD_IDENT: unsafe extern "C" fn() -> c_ulong =
    PyThread_get_thread_ident;
#[used]
static KEEP2_PYTHREAD_GET_THREAD_NATIVE_ID: unsafe extern "C" fn() -> c_ulong =
    PyThread_get_thread_native_id;
#[used]
static KEEP2_PYTHREAD_ALLOCATE_LOCK: unsafe extern "C" fn() -> *mut c_void = PyThread_allocate_lock;
#[used]
static KEEP2_PYTHREAD_FREE_LOCK: unsafe extern "C" fn(*mut c_void) = PyThread_free_lock;
#[used]
static KEEP2_PYTHREAD_ACQUIRE_LOCK: unsafe extern "C" fn(*mut c_void, c_int) -> c_int =
    PyThread_acquire_lock;
#[used]
static KEEP2_PYTHREAD_ACQUIRE_LOCK_TIMED: unsafe extern "C" fn(*mut c_void, i64, c_int) -> c_int =
    PyThread_acquire_lock_timed;
#[used]
static KEEP2_PYTHREAD_RELEASE_LOCK: unsafe extern "C" fn(*mut c_void) = PyThread_release_lock;
#[used]
static KEEP2_PYTHREAD_GET_STACKSIZE: unsafe extern "C" fn() -> usize = PyThread_get_stacksize;
#[used]
static KEEP2_PYTHREAD_SET_STACKSIZE: unsafe extern "C" fn(usize) -> c_int = PyThread_set_stacksize;
#[used]
static KEEP2_PYTHREAD_GETINFO: unsafe extern "C" fn() -> *mut c_void = PyThread_GetInfo;
#[used]
static KEEP2_PYTHREAD_CREATE_KEY: unsafe extern "C" fn() -> c_int = PyThread_create_key;
#[used]
static KEEP2_PYTHREAD_DELETE_KEY: unsafe extern "C" fn(c_int) = PyThread_delete_key;
#[used]
static KEEP2_PYTHREAD_SET_KEY_VALUE: unsafe extern "C" fn(c_int, *mut c_void) -> c_int =
    PyThread_set_key_value;
#[used]
static KEEP2_PYTHREAD_GET_KEY_VALUE: unsafe extern "C" fn(c_int) -> *mut c_void =
    PyThread_get_key_value;
#[used]
static KEEP2_PYTHREAD_DELETE_KEY_VALUE: unsafe extern "C" fn(c_int) = PyThread_delete_key_value;
#[used]
static KEEP2_PYTHREAD_REINIT_TLS: unsafe extern "C" fn() = PyThread_ReInitTLS;
#[used]
static KEEP2_PYTHREAD_TSS_ALLOC: unsafe extern "C" fn() -> *mut c_void = PyThread_tss_alloc;
#[used]
static KEEP2_PYTHREAD_TSS_FREE: unsafe extern "C" fn(*mut c_void) = PyThread_tss_free;
#[used]
static KEEP2_PYTHREAD_TSS_IS_CREATED: unsafe extern "C" fn(*mut c_void) -> c_int =
    PyThread_tss_is_created;
#[used]
static KEEP2_PYTHREAD_TSS_CREATE: unsafe extern "C" fn(*mut c_void) -> c_int = PyThread_tss_create;
#[used]
static KEEP2_PYTHREAD_TSS_DELETE: unsafe extern "C" fn(*mut c_void) = PyThread_tss_delete;
#[used]
static KEEP2_PYTHREAD_TSS_SET: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
    PyThread_tss_set;
#[used]
static KEEP2_PYTHREAD_TSS_GET: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyThread_tss_get;
#[used]
static KEEP2_PYTHREADSTATE_GET: unsafe extern "C" fn() -> *mut c_void = PyThreadState_Get;
#[used]
static KEEP2_PYTHREADSTATE_GETUNCHECKED: unsafe extern "C" fn() -> *mut c_void =
    PyThreadState_GetUnchecked;
#[used]
static KEEP2_PYTHREADSTATE_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyThreadState_New;
#[used]
static KEEP2__PYTHREADSTATE_PREALLOC: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    _PyThreadState_Prealloc;
#[used]
static KEEP2__PYTHREADSTATE_INIT: unsafe extern "C" fn(*mut c_void) = _PyThreadState_Init;
#[used]
static KEEP2_PYTHREADSTATE_SWAP: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyThreadState_Swap;
#[used]
static KEEP2_PYTHREADSTATE_CLEAR: unsafe extern "C" fn(*mut c_void) = PyThreadState_Clear;
#[used]
static KEEP2_PYTHREADSTATE_DELETE: unsafe extern "C" fn(*mut c_void) = PyThreadState_Delete;
#[used]
static KEEP2_PYTHREADSTATE_DELETECURRENT: unsafe extern "C" fn() = PyThreadState_DeleteCurrent;
#[used]
static KEEP2_PYTHREADSTATE_SETASYNCEXC: unsafe extern "C" fn(u64, *mut c_void) -> i32 =
    PyThreadState_SetAsyncExc;
#[used]
static KEEP2_PYTHREADSTATE_GETFRAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyThreadState_GetFrame;
#[used]
static KEEP2_PYFRAME_NEW: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyFrame_New;
#[used]
static KEEP2_PYFRAME_GETCODE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyFrame_GetCode;
#[used]
static KEEP2_PYFRAME_GETLINENUMBER: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyFrame_GetLineNumber;
#[used]
static KEEP2_PYTHREADSTATE_GETINTERPRETER: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyThreadState_GetInterpreter;
#[used]
static KEEP2_PYTHREADSTATE_GETID: unsafe extern "C" fn(*mut c_void) -> u64 = PyThreadState_GetID;
#[used]
static KEEP2_PYTHREADSTATE_GETDICT: unsafe extern "C" fn() -> *mut c_void = PyThreadState_GetDict;
#[used]
static KEEP2_PYINTERPRETERSTATE_GET: unsafe extern "C" fn() -> *mut c_void = PyInterpreterState_Get;
#[used]
static KEEP2_PYINTERPRETERSTATE_NEW: unsafe extern "C" fn() -> *mut c_void = PyInterpreterState_New;
#[used]
static KEEP2_PYINTERPRETERSTATE_CLEAR: unsafe extern "C" fn(*mut c_void) = PyInterpreterState_Clear;
#[used]
static KEEP2_PYINTERPRETERSTATE_DELETE: unsafe extern "C" fn(*mut c_void) =
    PyInterpreterState_Delete;
#[used]
static KEEP2_PYINTERPRETERSTATE_GETID: unsafe extern "C" fn(*mut c_void) -> i64 =
    PyInterpreterState_GetID;
#[used]
static KEEP2_PYINTERPRETERSTATE_GETDICT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyInterpreterState_GetDict;
#[used]
static KEEP2_PYSTATE_ADDMODULE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyState_AddModule;
#[used]
static KEEP2__PYSTATE_ADDMODULE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> i32 = _PyState_AddModule;
#[used]
static KEEP2_PYSTATE_FINDMODULE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyState_FindModule;
#[used]
static KEEP2_PYSTATE_REMOVEMODULE: unsafe extern "C" fn(*mut c_void) -> i32 = PyState_RemoveModule;
#[used]
static KEEP2_PYTRACEMALLOC_TRACK: unsafe extern "C" fn(usize, usize, usize) -> i32 =
    PyTraceMalloc_Track;
#[used]
static KEEP2_PYTRACEMALLOC_UNTRACK: unsafe extern "C" fn(usize, usize) -> i32 =
    PyTraceMalloc_Untrack;
#[used]
static KEEP2_PY_ENTERRECURSIVECALL: unsafe extern "C" fn(*const c_char) -> i32 =
    Py_EnterRecursiveCall;
#[used]
static KEEP2_PY_LEAVERECURSIVECALL: unsafe extern "C" fn() = Py_LeaveRecursiveCall;
#[used]
static KEEP2_PY_ISINITIALIZED: unsafe extern "C" fn() -> i32 = Py_IsInitialized;
#[used]
static KEEP2_PY_GETCONSTANT: unsafe extern "C" fn(c_uint) -> *mut c_void = Py_GetConstant;
#[used]
static KEEP2_PY_GETCONSTANTBORROWED: unsafe extern "C" fn(c_uint) -> *mut c_void =
    Py_GetConstantBorrowed;
#[used]
static KEEP2_PY_IS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int = Py_Is;
#[used]
static KEEP2_PY_ISNONE: unsafe extern "C" fn(*mut c_void) -> c_int = Py_IsNone;
#[used]
static KEEP2_PY_ISTRUE: unsafe extern "C" fn(*mut c_void) -> c_int = Py_IsTrue;
#[used]
static KEEP2_PY_ISFALSE: unsafe extern "C" fn(*mut c_void) -> c_int = Py_IsFalse;
#[used]
static KEEP2_PY_NEWREF: unsafe extern "C" fn(*mut c_void) -> *mut c_void = Py_NewRef;
#[used]
static KEEP2_PY_XNEWREF: unsafe extern "C" fn(*mut c_void) -> *mut c_void = Py_XNewRef;
#[used]
static KEEP2_PY_REFCNT: unsafe extern "C" fn(*mut c_void) -> isize = Py_REFCNT;
#[used]
static KEEP2_PY_TYPE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = Py_TYPE;
#[used]
static KEEP2_PYVECTORCALL_NARGS: unsafe extern "C" fn(usize) -> usize = PyVectorcall_NARGS;
#[used]
static KEEP3_PYCONTEXTVAR_NEW: unsafe extern "C" fn(*const c_char, *mut c_void) -> *mut c_void =
    PyContextVar_New;
#[used]
static KEEP3_PYCONTEXTVAR_GET: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32 = PyContextVar_Get;
#[used]
static KEEP3_PYCONTEXTVAR_SET: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyContextVar_Set;
#[used]
static KEEP3_PYMETHOD_NEW: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyMethod_New;
#[used]
static KEEP3_PYCODE_NEWEMPTY: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    c_int,
) -> *mut c_void = PyCode_NewEmpty;
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
) -> *mut c_void = PyUnstable_Code_New;
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
) -> *mut c_void = PyUnstable_Code_NewWithPosOnlyArgs;
#[used]
static KEEP3_PYOBJECT_CALLFUNCTION: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    ...
) -> *mut c_void = PyObject_CallFunction;
#[used]
static KEEP3_PYOBJECT_CALLFUNCTIONOBJARGS: unsafe extern "C" fn(*mut c_void, ...) -> *mut c_void =
    PyObject_CallFunctionObjArgs;
#[used]
static KEEP3_PYOBJECT_CALLMETHOD: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    ...
) -> *mut c_void = PyObject_CallMethod;
#[used]
static KEEP3_PYEVAL_CALLFUNCTION: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    ...
) -> *mut c_void = PyEval_CallFunction;
#[used]
static KEEP3_PYEVAL_CALLMETHOD: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    ...
) -> *mut c_void = PyEval_CallMethod;
#[used]
static KEEP3_PYEVAL_EVALFRAME: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyEval_EvalFrame;
#[used]
static KEEP3_PYEVAL_EVALFRAMEEX: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    PyEval_EvalFrameEx;
#[used]
static KEEP3_PYARG_PARSE: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> i32 =
    PyArg_Parse;
#[used]
static KEEP3__PYARG_PARSE_SIZET: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> i32 =
    _PyArg_Parse_SizeT;
#[used]
static KEEP3_PYARG_VAPARSE: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> i32 =
    PyArg_VaParse;
#[used]
static KEEP3__PYARG_VAPARSE_SIZET: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = _PyArg_VaParse_SizeT;
#[used]
static KEEP3_PYARG_VALIDATEKEYWORDARGUMENTS: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyArg_ValidateKeywordArguments;
#[used]
static KEEP3_PYARG_PARSETUPLE: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> i32 =
    PyArg_ParseTuple;
#[used]
static KEEP3__PYARG_PARSETUPLE_SIZET: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> i32 =
    _PyArg_ParseTuple_SizeT;
#[used]
static KEEP3_PYARG_PARSETUPLEANDKEYWORDS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
    ...
) -> i32 = PyArg_ParseTupleAndKeywords;
#[used]
static KEEP3__PYARG_PARSETUPLEANDKEYWORDS_SIZET: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
    ...
) -> i32 = _PyArg_ParseTupleAndKeywords_SizeT;
#[used]
static KEEP3_PYARG_VAPARSETUPLEANDKEYWORDS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
    *mut c_void,
) -> i32 = PyArg_VaParseTupleAndKeywords;
#[used]
static KEEP3__PYARG_VAPARSETUPLEANDKEYWORDS_SIZET: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
    *mut c_void,
) -> i32 = _PyArg_VaParseTupleAndKeywords_SizeT;
#[used]
static KEEP3_PYARG_UNPACKTUPLE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    isize,
    isize,
) -> i32 = PyArg_UnpackTuple;
#[used]
static KEEP3_PY_BUILDVALUE: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void = Py_BuildValue;
#[used]
static KEEP3__PY_BUILDVALUE_SIZET: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void =
    _Py_BuildValue_SizeT;
#[used]
static KEEP3_PY_VABUILDVALUE: unsafe extern "C" fn(*const c_char, *mut c_void) -> *mut c_void =
    Py_VaBuildValue;
#[used]
static KEEP3__PY_VABUILDVALUE_SIZET: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
) -> *mut c_void = _Py_VaBuildValue_SizeT;
#[used]
static KEEP3__PYOBJECT_CALLFUNCTION_SIZET: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    ...
) -> *mut c_void = _PyObject_CallFunction_SizeT;
#[used]
static KEEP3__PYOBJECT_CALLMETHOD_SIZET: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    ...
) -> *mut c_void = _PyObject_CallMethod_SizeT;
#[used]
static KEEP3_PYVECTORCALL_CALL: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyVectorcall_Call;
#[used]
static KEEP3_PYOBJECT_VECTORCALL: unsafe extern "C" fn(
    *mut c_void,
    *const *mut c_void,
    usize,
    *mut c_void,
) -> *mut c_void = PyObject_Vectorcall;
#[used]
static KEEP3_PYOBJECT_VECTORCALLDICT: unsafe extern "C" fn(
    *mut c_void,
    *const *mut c_void,
    usize,
    *mut c_void,
) -> *mut c_void = PyObject_VectorcallDict;
#[used]
static KEEP3_PYOBJECT_VECTORCALLMETHOD: unsafe extern "C" fn(
    *mut c_void,
    *const *mut c_void,
    usize,
    *mut c_void,
) -> *mut c_void = PyObject_VectorcallMethod;
#[used]
static KEEP3_PYMUTEX_LOCK: unsafe extern "C" fn(*mut c_void) = PyMutex_Lock;
#[used]
static KEEP3_PYMUTEX_UNLOCK: unsafe extern "C" fn(*mut c_void) = PyMutex_Unlock;
#[used]
static KEEP3_PYOS_SNPRINTF: unsafe extern "C" fn(*mut c_char, usize, *const c_char) -> i32 =
    PyOS_snprintf;
#[used]
static KEEP3_PYOS_STRING_TO_DOUBLE: unsafe extern "C" fn(
    *const c_char,
    *mut *mut c_char,
    *mut c_void,
) -> c_double = PyOS_string_to_double;
#[used]
static KEEP3_PYOS_STRTOL: unsafe extern "C" fn(*const c_char, *mut *mut c_char, i32) -> c_long =
    PyOS_strtol;
#[used]
static KEEP3_PYOS_STRTOUL: unsafe extern "C" fn(*const c_char, *mut *mut c_char, i32) -> c_ulong =
    PyOS_strtoul;
#[used]
static KEEP3_PYOS_BEFOREFORK: unsafe extern "C" fn() = PyOS_BeforeFork;
#[used]
static KEEP3_PYOS_AFTERFORK_PARENT: unsafe extern "C" fn() = PyOS_AfterFork_Parent;
#[used]
static KEEP3_PYOS_AFTERFORK_CHILD: unsafe extern "C" fn() = PyOS_AfterFork_Child;
#[used]
static KEEP3_PYOS_AFTERFORK: unsafe extern "C" fn() = PyOS_AfterFork;
#[used]
static KEEP3_PYOS_CHECKSTACK: unsafe extern "C" fn() -> c_int = PyOS_CheckStack;
#[used]
static KEEP3_PYOS_FSPATH: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyOS_FSPath;
#[used]
static KEEP3_PYOS_INTERRUPTOCCURRED: unsafe extern "C" fn() -> c_int = PyOS_InterruptOccurred;
#[used]
static KEEP3_PYOS_DOUBLE_TO_STRING: unsafe extern "C" fn(
    c_double,
    c_char,
    c_int,
    c_int,
    *mut c_int,
) -> *mut c_char = PyOS_double_to_string;
#[used]
static KEEP3_PYOS_GETSIG: unsafe extern "C" fn(c_int) -> *mut c_void = PyOS_getsig;
#[used]
static KEEP3_PYOS_SETSIG: unsafe extern "C" fn(c_int, *mut c_void) -> *mut c_void = PyOS_setsig;
#[used]
static KEEP3_PYOS_MYSTRICMP: unsafe extern "C" fn(*const c_char, *const c_char) -> c_int =
    PyOS_mystricmp;
#[used]
static KEEP3_PYOS_MYSTRNICMP: unsafe extern "C" fn(*const c_char, *const c_char, isize) -> c_int =
    PyOS_mystrnicmp;
#[used]
static KEEP3_PYOS_VSNPRINTF: unsafe extern "C" fn(
    *mut c_char,
    usize,
    *const c_char,
    *mut c_void,
) -> c_int = PyOS_vsnprintf;
#[used]
static KEEP3_PYERR_EXCEPTIONMATCHES: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyErr_ExceptionMatches;
#[used]
static KEEP3_PYERR_FETCH: unsafe extern "C" fn(
    *mut *mut c_void,
    *mut *mut c_void,
    *mut *mut c_void,
) = PyErr_Fetch;
#[used]
static KEEP3_PYERR_FORMAT: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> *mut c_void =
    PyErr_Format;
#[used]
static KEEP3_PYERR_FORMATV: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> *mut c_void = PyErr_FormatV;
#[used]
static KEEP3_PYSYS_WRITESTDOUT: unsafe extern "C" fn(*const c_char, ...) = PySys_WriteStdout;
#[used]
static KEEP3_PYSYS_WRITESTDERR: unsafe extern "C" fn(*const c_char, ...) = PySys_WriteStderr;
#[used]
static KEEP3_PYSYS_FORMATSTDOUT: unsafe extern "C" fn(*const c_char, ...) = PySys_FormatStdout;
#[used]
static KEEP3_PYSYS_FORMATSTDERR: unsafe extern "C" fn(*const c_char, ...) = PySys_FormatStderr;
#[used]
static KEEP3_PYSYS_AUDIT: unsafe extern "C" fn(*const c_char, *const c_char, ...) -> i32 =
    PySys_Audit;
#[used]
static KEEP3_PYCODEC_REGISTER: unsafe extern "C" fn(*mut c_void) -> i32 = PyCodec_Register;
#[used]
static KEEP3_PYCODEC_UNREGISTER: unsafe extern "C" fn(*mut c_void) -> i32 = PyCodec_Unregister;
#[used]
static KEEP3_PYCODEC_KNOWNENCODING: unsafe extern "C" fn(*const c_char) -> i32 =
    PyCodec_KnownEncoding;
#[used]
static KEEP3_PYCODEC_ENCODE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyCodec_Encode;
#[used]
static KEEP3_PYCODEC_DECODE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyCodec_Decode;
#[used]
static KEEP3_PYCODEC_ENCODER: unsafe extern "C" fn(*const c_char) -> *mut c_void = PyCodec_Encoder;
#[used]
static KEEP3_PYCODEC_DECODER: unsafe extern "C" fn(*const c_char) -> *mut c_void = PyCodec_Decoder;
#[used]
static KEEP3_PYCODEC_INCREMENTALENCODER: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
) -> *mut c_void = PyCodec_IncrementalEncoder;
#[used]
static KEEP3_PYCODEC_INCREMENTALDECODER: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
) -> *mut c_void = PyCodec_IncrementalDecoder;
#[used]
static KEEP3_PYCODEC_STREAMREADER: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyCodec_StreamReader;
#[used]
static KEEP3_PYCODEC_STREAMWRITER: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyCodec_StreamWriter;
#[used]
static KEEP3_PYCODEC_REGISTERERROR: unsafe extern "C" fn(*const c_char, *mut c_void) -> i32 =
    PyCodec_RegisterError;
#[used]
static KEEP3_PYCODEC_LOOKUPERROR: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyCodec_LookupError;
#[used]
static KEEP3_PYCODEC_STRICTERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyCodec_StrictErrors;
#[used]
static KEEP3_PYCODEC_IGNOREERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyCodec_IgnoreErrors;
#[used]
static KEEP3_PYCODEC_REPLACEERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyCodec_ReplaceErrors;
#[used]
static KEEP3_PYCODEC_XMLCHARREFREPLACEERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyCodec_XMLCharRefReplaceErrors;
#[used]
static KEEP3_PYCODEC_BACKSLASHREPLACEERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyCodec_BackslashReplaceErrors;
#[used]
static KEEP3_PYCODEC_NAMEREPLACEERRORS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyCodec_NameReplaceErrors;
#[used]
static KEEP3_PYERR_GIVENEXCEPTIONMATCHES: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyErr_GivenExceptionMatches;
#[used]
static KEEP3_PYERR_NORMALIZEEXCEPTION: unsafe extern "C" fn(
    *mut *mut c_void,
    *mut *mut c_void,
    *mut *mut c_void,
) = PyErr_NormalizeException;
#[used]
static KEEP3_PYERR_PRINT: unsafe extern "C" fn() = PyErr_Print;
#[used]
static KEEP3_PYERR_RESTORE: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
    PyErr_Restore;
#[used]
static KEEP3_PYERR_SETFROMERRNO: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyErr_SetFromErrno;
#[used]
static KEEP3_PYERR_WARNEX: unsafe extern "C" fn(*mut c_void, *const c_char, isize) -> i32 =
    PyErr_WarnEx;
#[used]
static KEEP3_PYERR_WARNFORMAT: unsafe extern "C" fn(*mut c_void, isize, *const c_char) -> i32 =
    PyErr_WarnFormat;
#[used]
static KEEP3_PYERR_WARNEXPLICIT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    i32,
    *const c_char,
    *mut c_void,
) -> i32 = PyErr_WarnExplicit;
#[used]
static KEEP3_PYERR_RESOURCEWARNING: unsafe extern "C" fn(*mut c_void, isize, *const c_char) -> i32 =
    PyErr_ResourceWarning;
#[used]
static KEEP3_PYERR_WRITEUNRAISABLE: unsafe extern "C" fn(*mut c_void) = PyErr_WriteUnraisable;
#[used]
static KEEP3_PYTRACEBACK_HERE: unsafe extern "C" fn(*mut c_void) -> c_int = PyTraceBack_Here;
#[used]
static KEEP3_PYTRACEBACK_PRINT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
    PyTraceBack_Print;
#[used]
static KEEP3_PYEXCEPTION_GETTRACEBACK: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyException_GetTraceback;
#[used]
static KEEP3_PYEXCEPTION_GETCAUSE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyException_GetCause;
#[used]
static KEEP3_PYEXCEPTION_GETCONTEXT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyException_GetContext;
#[used]
static KEEP3_PYEXCEPTION_GETARGS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyException_GetArgs;
#[used]
static KEEP3_PYEXCEPTION_SETARGS: unsafe extern "C" fn(*mut c_void, *mut c_void) =
    PyException_SetArgs;
#[used]
static KEEP3_PYEXCEPTION_SETCAUSE: unsafe extern "C" fn(*mut c_void, *mut c_void) =
    PyException_SetCause;
#[used]
static KEEP3_PYEXCEPTION_SETCONTEXT: unsafe extern "C" fn(*mut c_void, *mut c_void) =
    PyException_SetContext;
#[used]
static KEEP3_PYEXCEPTION_SETTRACEBACK: unsafe extern "C" fn(*mut c_void, *mut c_void) =
    PyException_SetTraceback;
#[used]
static KEEP3_PYUNICODE_FROMSTRINGANDSIZE: unsafe extern "C" fn(
    *const c_char,
    isize,
) -> *mut c_void = PyUnicode_FromStringAndSize;
#[used]
static KEEP3_PYUNICODE_FROMWIDECHAR: unsafe extern "C" fn(*const Cwchar, isize) -> *mut c_void =
    PyUnicode_FromWideChar;
#[used]
static KEEP3_PYUNICODE_FROMENCODEDOBJECT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyUnicode_FromEncodedObject;
#[used]
static KEEP3_PYUNICODE_FROMKINDANDDATA: unsafe extern "C" fn(
    i32,
    *const c_void,
    isize,
) -> *mut c_void = PyUnicode_FromKindAndData;
#[used]
static KEEP3_PYUNICODE_ASUTF8: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    PyUnicode_AsUTF8;
#[used]
static KEEP3_PYUNICODE_ASUTF8ANDSIZE: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
) -> *const c_char = PyUnicode_AsUTF8AndSize;
#[used]
static KEEP3_PYUNICODE_ASUTF8STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_AsUTF8String;
#[used]
static KEEP3_PYUNICODE_ASASCIISTRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_AsASCIIString;
#[used]
static KEEP3_PYUNICODE_ASLATIN1STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_AsLatin1String;
#[used]
static KEEP3_PYUNICODE_ASMBCSSTRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_AsMBCSString;
#[used]
static KEEP3_PYUNICODE_ASCHARMAPSTRING: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyUnicode_AsCharmapString;
#[used]
static KEEP3_PYUNICODE_ASRAWUNICODEESCAPESTRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_AsRawUnicodeEscapeString;
#[used]
static KEEP3_PYUNICODE_ASUNICODEESCAPESTRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_AsUnicodeEscapeString;
#[used]
static KEEP3_PYUNICODE_ASUTF16STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_AsUTF16String;
#[used]
static KEEP3_PYUNICODE_ASUTF32STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_AsUTF32String;
#[used]
static KEEP3_PYUNICODE_ASENCODEDSTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyUnicode_AsEncodedString;
#[used]
static KEEP3_PYUNICODE_ASWIDECHARSTRING: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
) -> *mut Cwchar = PyUnicode_AsWideCharString;
#[used]
static KEEP3_PYUNICODE_ASWIDECHAR: unsafe extern "C" fn(*mut c_void, *mut Cwchar, isize) -> isize =
    PyUnicode_AsWideChar;
#[used]
static KEEP3_PYUNICODE_COMPARE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyUnicode_Compare;
#[used]
static KEEP3_PYUNICODE_COMPAREWITHASCIISTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> i32 = PyUnicode_CompareWithASCIIString;
#[used]
static KEEP3_PYUNICODE_CONCAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyUnicode_Concat;
#[used]
static KEEP3_PYUNICODE_CONTAINS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyUnicode_Contains;
#[used]
static KEEP3_PYUNICODE_FORMAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyUnicode_Format;
#[used]
static KEEP3_PYUNICODE_GETLENGTH: unsafe extern "C" fn(*mut c_void) -> isize = PyUnicode_GetLength;
#[used]
static KEEP3_PYUNICODE_INTERNFROMSTRING: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyUnicode_InternFromString;
#[used]
static KEEP3_PYUNICODE_NEW: unsafe extern "C" fn(isize, c_uint) -> *mut c_void = PyUnicode_New;
#[used]
static KEEP3_PYUNICODE_FROMFORMAT: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void =
    PyUnicode_FromFormat;
#[used]
static KEEP3_PYUNICODE_FROMFORMATV: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
) -> *mut c_void = PyUnicode_FromFormatV;
#[used]
static KEEP3_PYUNICODE_FROMOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_FromObject;
#[used]
static KEEP3_PYUNICODE_FROMORDINAL: unsafe extern "C" fn(c_int) -> *mut c_void =
    PyUnicode_FromOrdinal;
#[used]
static KEEP3_PYUNICODE_GETDEFAULTENCODING: unsafe extern "C" fn() -> *const c_char =
    PyUnicode_GetDefaultEncoding;
#[used]
static KEEP3_PYUNICODE_EQUAL: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
    PyUnicode_Equal;
#[used]
static KEEP3_PYUNICODE_EQUALTOUTF8: unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int =
    PyUnicode_EqualToUTF8;
#[used]
static KEEP3_PYUNICODE_EQUALTOUTF8ANDSIZE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    isize,
) -> c_int = PyUnicode_EqualToUTF8AndSize;
#[used]
static KEEP3_PYUNICODE_READCHAR: unsafe extern "C" fn(*mut c_void, isize) -> u32 =
    PyUnicode_ReadChar;
#[used]
static KEEP3_PYUNICODE_FIND: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
    isize,
    c_int,
) -> isize = PyUnicode_Find;
#[used]
static KEEP3_PYUNICODE_FINDCHAR: unsafe extern "C" fn(
    *mut c_void,
    u32,
    isize,
    isize,
    c_int,
) -> isize = PyUnicode_FindChar;
#[used]
static KEEP3_PYUNICODE_COUNT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
    isize,
) -> isize = PyUnicode_Count;
#[used]
static KEEP3_PYUNICODE_JOIN: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyUnicode_Join;
#[used]
static KEEP3_PYUNICODE_SPLIT: unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void =
    PyUnicode_Split;
#[used]
static KEEP3_PYUNICODE_RSPLIT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
) -> *mut c_void = PyUnicode_RSplit;
#[used]
static KEEP3_PYUNICODE_SPLITLINES: unsafe extern "C" fn(*mut c_void, c_int) -> *mut c_void =
    PyUnicode_Splitlines;
#[used]
static KEEP3_PYUNICODE_PARTITION: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyUnicode_Partition;
#[used]
static KEEP3_PYUNICODE_RPARTITION: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyUnicode_RPartition;
#[used]
static KEEP3_PYUNICODE_ISIDENTIFIER: unsafe extern "C" fn(*mut c_void) -> c_int =
    PyUnicode_IsIdentifier;
#[used]
static KEEP3_PYUNICODE_GETSIZE: unsafe extern "C" fn(*mut c_void) -> isize = PyUnicode_GetSize;
#[used]
static KEEP3_PYUNICODE_INTERNINPLACE: unsafe extern "C" fn(*mut *mut c_void) =
    PyUnicode_InternInPlace;
#[used]
static KEEP3_PYUNICODE_INTERNIMMORTAL: unsafe extern "C" fn(*mut *mut c_void) =
    PyUnicode_InternImmortal;
#[used]
static KEEP3_PYUNICODE_APPEND: unsafe extern "C" fn(*mut *mut c_void, *mut c_void) =
    PyUnicode_Append;
#[used]
static KEEP3_PYUNICODE_APPENDANDDEL: unsafe extern "C" fn(*mut *mut c_void, *mut c_void) =
    PyUnicode_AppendAndDel;
#[used]
static KEEP3_PYUNICODE_RICHCOMPARE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    c_int,
) -> *mut c_void = PyUnicode_RichCompare;
#[used]
static KEEP3_PYUNICODE_WRITECHAR: unsafe extern "C" fn(*mut c_void, isize, c_uint) -> c_int =
    PyUnicode_WriteChar;
#[used]
static KEEP3_PYUNICODE_COPYCHARACTERS: unsafe extern "C" fn(
    *mut c_void,
    isize,
    *mut c_void,
    isize,
    isize,
) -> isize = PyUnicode_CopyCharacters;
#[used]
static KEEP3_PYUNICODE_TRANSLATE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyUnicode_Translate;
#[used]
static KEEP3_PYUNICODE_RESIZE: unsafe extern "C" fn(*mut *mut c_void, isize) -> c_int =
    PyUnicode_Resize;
#[used]
static KEEP3_PYUNICODE_REPLACE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    isize,
) -> *mut c_void = PyUnicode_Replace;
#[used]
static KEEP3_PYUNICODE_SUBSTRING: unsafe extern "C" fn(*mut c_void, isize, isize) -> *mut c_void =
    PyUnicode_Substring;
#[used]
static KEEP3_PYUNICODE_TAILMATCH: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
    isize,
    i32,
) -> isize = PyUnicode_Tailmatch;
#[used]
static KEEP3_PYUNICODE_ASUCS4: unsafe extern "C" fn(*mut c_void, *mut u32, isize, i32) -> *mut u32 =
    PyUnicode_AsUCS4;
#[used]
static KEEP3_PYUNICODE_ASUCS4COPY: unsafe extern "C" fn(*mut c_void) -> *mut u32 =
    PyUnicode_AsUCS4Copy;
#[used]
static KEEP3_PYUNICODE_DECODE: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyUnicode_Decode;
#[used]
static KEEP3_PYUNICODE_DECODEASCII: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = PyUnicode_DecodeASCII;
#[used]
static KEEP3_PYUNICODE_DECODELATIN1: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = PyUnicode_DecodeLatin1;
#[used]
static KEEP3_PYUNICODE_DECODEUTF8: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = PyUnicode_DecodeUTF8;
#[used]
static KEEP3_PYUNICODE_DECODEUTF8STATEFUL: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut isize,
) -> *mut c_void = PyUnicode_DecodeUTF8Stateful;
#[used]
static KEEP3_PYUNICODE_DECODEUTF7: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = PyUnicode_DecodeUTF7;
#[used]
static KEEP3_PYUNICODE_DECODEUTF7STATEFUL: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut isize,
) -> *mut c_void = PyUnicode_DecodeUTF7Stateful;
#[used]
static KEEP3_PYUNICODE_DECODEMBCS: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = PyUnicode_DecodeMBCS;
#[used]
static KEEP3_PYUNICODE_DECODEMBCSSTATEFUL: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut isize,
) -> *mut c_void = PyUnicode_DecodeMBCSStateful;
#[used]
static KEEP3_PYUNICODE_DECODECODEPAGESTATEFUL: unsafe extern "C" fn(
    c_int,
    *const c_char,
    isize,
    *const c_char,
    *mut isize,
) -> *mut c_void = PyUnicode_DecodeCodePageStateful;
#[used]
static KEEP3_PYUNICODE_DECODECHARMAP: unsafe extern "C" fn(
    *const c_char,
    isize,
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyUnicode_DecodeCharmap;
#[used]
static KEEP3_PYUNICODE_BUILDENCODINGMAP: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_BuildEncodingMap;
#[used]
static KEEP3_PYUNICODE_DECODERAWUNICODEESCAPE: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = PyUnicode_DecodeRawUnicodeEscape;
#[used]
static KEEP3_PYUNICODE_DECODEUNICODEESCAPE: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = PyUnicode_DecodeUnicodeEscape;
#[used]
static KEEP3_PYUNICODE_DECODEUTF16: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut c_int,
) -> *mut c_void = PyUnicode_DecodeUTF16;
#[used]
static KEEP3_PYUNICODE_DECODEUTF16STATEFUL: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut c_int,
    *mut isize,
) -> *mut c_void = PyUnicode_DecodeUTF16Stateful;
#[used]
static KEEP3_PYUNICODE_DECODEUTF32: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut c_int,
) -> *mut c_void = PyUnicode_DecodeUTF32;
#[used]
static KEEP3_PYUNICODE_DECODEUTF32STATEFUL: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    *mut c_int,
    *mut isize,
) -> *mut c_void = PyUnicode_DecodeUTF32Stateful;
#[used]
static KEEP3_PYUNICODE_DECODEFSDEFAULT: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyUnicode_DecodeFSDefault;
#[used]
static KEEP3_PYUNICODE_DECODEFSDEFAULTANDSIZE: unsafe extern "C" fn(
    *const c_char,
    isize,
) -> *mut c_void = PyUnicode_DecodeFSDefaultAndSize;
#[used]
static KEEP3_PYUNICODE_DECODELOCALE: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
) -> *mut c_void = PyUnicode_DecodeLocale;
#[used]
static KEEP3_PYUNICODE_DECODELOCALEANDSIZE: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = PyUnicode_DecodeLocaleAndSize;
#[used]
static KEEP3_PYUNICODE_ENCODEFSDEFAULT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicode_EncodeFSDefault;
#[used]
static KEEP3_PYUNICODE_ENCODELOCALE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyUnicode_EncodeLocale;
#[used]
static KEEP3_PYUNICODE_ENCODECODEPAGE: unsafe extern "C" fn(
    c_int,
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyUnicode_EncodeCodePage;
#[used]
static KEEP3_PYUNICODE_ASDECODEDOBJECT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyUnicode_AsDecodedObject;
#[used]
static KEEP3_PYUNICODE_ASDECODEDUNICODE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyUnicode_AsDecodedUnicode;
#[used]
static KEEP3_PYUNICODE_ASENCODEDOBJECT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyUnicode_AsEncodedObject;
#[used]
static KEEP3_PYUNICODE_ASENCODEDUNICODE: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyUnicode_AsEncodedUnicode;
#[used]
static KEEP3_PYUNICODE_FSCONVERTER: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
    PyUnicode_FSConverter;
#[used]
static KEEP3_PYUNICODE_FSDECODER: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
    PyUnicode_FSDecoder;
#[used]
static KEEP3_PYUNICODEDECODEERROR_CREATE: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    isize,
    isize,
    isize,
    *const c_char,
) -> *mut c_void = PyUnicodeDecodeError_Create;
#[used]
static KEEP3_PYUNICODEENCODEERROR_GETENCODING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicodeEncodeError_GetEncoding;
#[used]
static KEEP3_PYUNICODEDECODEERROR_GETENCODING: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicodeDecodeError_GetEncoding;
#[used]
static KEEP3_PYUNICODEENCODEERROR_GETOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicodeEncodeError_GetObject;
#[used]
static KEEP3_PYUNICODEDECODEERROR_GETOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicodeDecodeError_GetObject;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_GETOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicodeTranslateError_GetObject;
#[used]
static KEEP3_PYUNICODEENCODEERROR_GETSTART: unsafe extern "C" fn(*mut c_void, *mut isize) -> c_int =
    PyUnicodeEncodeError_GetStart;
#[used]
static KEEP3_PYUNICODEDECODEERROR_GETSTART: unsafe extern "C" fn(*mut c_void, *mut isize) -> c_int =
    PyUnicodeDecodeError_GetStart;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_GETSTART: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
) -> c_int = PyUnicodeTranslateError_GetStart;
#[used]
static KEEP3_PYUNICODEENCODEERROR_SETSTART: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    PyUnicodeEncodeError_SetStart;
#[used]
static KEEP3_PYUNICODEDECODEERROR_SETSTART: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    PyUnicodeDecodeError_SetStart;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_SETSTART: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    PyUnicodeTranslateError_SetStart;
#[used]
static KEEP3_PYUNICODEENCODEERROR_GETEND: unsafe extern "C" fn(*mut c_void, *mut isize) -> c_int =
    PyUnicodeEncodeError_GetEnd;
#[used]
static KEEP3_PYUNICODEDECODEERROR_GETEND: unsafe extern "C" fn(*mut c_void, *mut isize) -> c_int =
    PyUnicodeDecodeError_GetEnd;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_GETEND: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
) -> c_int = PyUnicodeTranslateError_GetEnd;
#[used]
static KEEP3_PYUNICODEENCODEERROR_SETEND: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    PyUnicodeEncodeError_SetEnd;
#[used]
static KEEP3_PYUNICODEDECODEERROR_SETEND: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    PyUnicodeDecodeError_SetEnd;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_SETEND: unsafe extern "C" fn(*mut c_void, isize) -> c_int =
    PyUnicodeTranslateError_SetEnd;
#[used]
static KEEP3_PYUNICODEENCODEERROR_GETREASON: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicodeEncodeError_GetReason;
#[used]
static KEEP3_PYUNICODEDECODEERROR_GETREASON: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicodeDecodeError_GetReason;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_GETREASON: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyUnicodeTranslateError_GetReason;
#[used]
static KEEP3_PYUNICODEENCODEERROR_SETREASON: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> c_int = PyUnicodeEncodeError_SetReason;
#[used]
static KEEP3_PYUNICODEDECODEERROR_SETREASON: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> c_int = PyUnicodeDecodeError_SetReason;
#[used]
static KEEP3_PYUNICODETRANSLATEERROR_SETREASON: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> c_int = PyUnicodeTranslateError_SetReason;
#[used]
static KEEP3_PYUNSTABLE_OBJECT_ISUNIQUELYREFERENCED: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyUnstable_Object_IsUniquelyReferenced;
#[used]
static KEEP3_PYUNSTABLE_OBJECT_ISUNIQUEREFERENCEDTEMPORARY: unsafe extern "C" fn(
    *mut c_void,
) -> i32 = PyUnstable_Object_IsUniqueReferencedTemporary;
#[used]
static KEEP3_PY_GENERICALIAS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    Py_GenericAlias;
#[used]
static KEEP4__PYOBJECT_NEW: unsafe extern "C" fn(*mut CpythonTypeObject) -> *mut c_void =
    _PyObject_New;
#[used]
static KEEP4__PYOBJECT_NEWVAR: unsafe extern "C" fn(*mut CpythonTypeObject, isize) -> *mut c_void =
    _PyObject_NewVar;
#[used]
static KEEP4__PYOBJECT_GC_NEW: unsafe extern "C" fn(*mut CpythonTypeObject) -> *mut c_void =
    _PyObject_GC_New;
#[used]
static KEEP4__PY_DEALLOC: unsafe extern "C" fn(*mut c_void) = _Py_Dealloc;
#[used]
static KEEP4__PYERR_BADINTERNALCALL: unsafe extern "C" fn(*const c_char, i32) =
    _PyErr_BadInternalCall;
#[used]
static KEEP4__PY_HASHDOUBLE: unsafe extern "C" fn(*mut c_void, f64) -> isize = _Py_HashDouble;
#[used]
static KEEP4__PYUNICODE_ISWHITESPACE: unsafe extern "C" fn(u32) -> i32 = _PyUnicode_IsWhitespace;
#[used]
static KEEP4__PYUNICODE_ISALPHA: unsafe extern "C" fn(u32) -> i32 = _PyUnicode_IsAlpha;
#[used]
static KEEP4__PYUNICODE_ISDECIMALDIGIT: unsafe extern "C" fn(u32) -> i32 =
    _PyUnicode_IsDecimalDigit;
#[used]
static KEEP4__PYUNICODE_ISDIGIT: unsafe extern "C" fn(u32) -> i32 = _PyUnicode_IsDigit;
#[used]
static KEEP4__PYUNICODE_ISNUMERIC: unsafe extern "C" fn(u32) -> i32 = _PyUnicode_IsNumeric;
#[used]
static KEEP4__PYUNICODE_ISLOWERCASE: unsafe extern "C" fn(u32) -> i32 = _PyUnicode_IsLowercase;
#[used]
static KEEP4__PYUNICODE_ISUPPERCASE: unsafe extern "C" fn(u32) -> i32 = _PyUnicode_IsUppercase;
#[used]
static KEEP4__PYUNICODE_ISTITLECASE: unsafe extern "C" fn(u32) -> i32 = _PyUnicode_IsTitlecase;

#[used]
static KEEP_PYMODULEDEF_INIT: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyModuleDef_Init;
#[used]
static KEEP_PYMODULE_CREATE2: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    PyModule_Create2;
#[used]
static KEEP_PYMODULE_FROM_DEF_AND_SPEC2: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    i32,
) -> *mut c_void = PyModule_FromDefAndSpec2;
#[used]
static KEEP_PYMODULE_EXEC_DEF: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyModule_ExecDef;
#[used]
static KEEP_PYMODULE_GET_DEF: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyModule_GetDef;
#[used]
static KEEP_PYMODULE_GET_STATE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyModule_GetState;
#[used]
static KEEP_PYMODULE_ADD_OBJECT_REF: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = PyModule_AddObjectRef;
#[used]
static KEEP_PYMODULE_ADD_OBJECT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = PyModule_AddObject;
#[used]
static KEEP_PYMODULE_ADD_INT_CONSTANT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    i64,
) -> i32 = PyModule_AddIntConstant;
#[used]
static KEEP_PYMODULE_ADD_STRING_CONSTANT: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> i32 = PyModule_AddStringConstant;
#[used]
static KEEP_PYLONG_FROM_LONG: unsafe extern "C" fn(i64) -> *mut c_void = PyLong_FromLong;
#[used]
static KEEP_PYLONG_FROM_LONGLONG: unsafe extern "C" fn(i64) -> *mut c_void = PyLong_FromLongLong;
#[used]
static KEEP_PYBOOL_FROM_LONG: unsafe extern "C" fn(i64) -> *mut c_void = PyBool_FromLong;
#[used]
static KEEP_PYFLOAT_FROM_DOUBLE: unsafe extern "C" fn(f64) -> *mut c_void = PyFloat_FromDouble;
#[used]
static KEEP_PYUNICODE_FROM_STRING: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyUnicode_FromString;
#[used]
static KEEP_PYBYTES_FROM_STRING_AND_SIZE: unsafe extern "C" fn(
    *const c_char,
    isize,
) -> *mut c_void = PyBytes_FromStringAndSize;
#[used]
static KEEP_PYBYTES_FROM_OBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyBytes_FromObject;
#[used]
static KEEP_PYBYTES_FROM_FORMAT: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void =
    PyBytes_FromFormat;
#[used]
static KEEP_PYBYTES_FROM_FORMATV: unsafe extern "C" fn(*const c_char, *mut c_void) -> *mut c_void =
    PyBytes_FromFormatV;
#[used]
static KEEP_PYBYTES_CONCAT: unsafe extern "C" fn(*mut *mut c_void, *mut c_void) = PyBytes_Concat;
#[used]
static KEEP_PYBYTES_CONCAT_AND_DEL: unsafe extern "C" fn(*mut *mut c_void, *mut c_void) =
    PyBytes_ConcatAndDel;
#[used]
static KEEP_PYERR_SET_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) = PyErr_SetString;
#[used]
static KEEP_PYERR_NEW_EXCEPTION: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyErr_NewException;
#[used]
static KEEP_PYERR_NEW_EXCEPTION_WITH_DOC: unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyErr_NewExceptionWithDoc;
#[used]
static KEEP_PYEXCEPTIONCLASS_NAME: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    PyExceptionClass_Name;
#[used]
static KEEP_PYERR_OCCURRED: unsafe extern "C" fn() -> *mut c_void = PyErr_Occurred;
#[used]
static KEEP_PYERR_CLEAR: unsafe extern "C" fn() = PyErr_Clear;
#[used]
static KEEP_PYERR_BAD_ARGUMENT: unsafe extern "C" fn() -> i32 = PyErr_BadArgument;
#[used]
static KEEP_PYERR_BAD_INTERNAL_CALL: unsafe extern "C" fn() = PyErr_BadInternalCall;
#[used]
static KEEP_PYERR_PRINT_EX: unsafe extern "C" fn(i32) = PyErr_PrintEx;
#[used]
static KEEP_PYERR_DISPLAY: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
    PyErr_Display;
#[used]
static KEEP_PYERR_DISPLAY_EXCEPTION: unsafe extern "C" fn(*mut c_void) = PyErr_DisplayException;
#[used]
static KEEP_PYERR_GET_RAISED_EXCEPTION: unsafe extern "C" fn() -> *mut c_void =
    PyErr_GetRaisedException;
#[used]
static KEEP_PYERR_SET_RAISED_EXCEPTION: unsafe extern "C" fn(*mut c_void) =
    PyErr_SetRaisedException;
#[used]
static KEEP_PYERR_GET_HANDLED_EXCEPTION: unsafe extern "C" fn() -> *mut c_void =
    PyErr_GetHandledException;
#[used]
static KEEP_PYERR_SET_HANDLED_EXCEPTION: unsafe extern "C" fn(*mut c_void) =
    PyErr_SetHandledException;
#[used]
static KEEP_PYERR_GET_EXCINFO: unsafe extern "C" fn(
    *mut *mut c_void,
    *mut *mut c_void,
    *mut *mut c_void,
) = PyErr_GetExcInfo;
#[used]
static KEEP_PYERR_SET_EXCINFO: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
    PyErr_SetExcInfo;
#[used]
static KEEP_PYERR_SET_FROM_ERRNO: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyErr_SetFromErrno;
#[used]
static KEEP_PYERR_SET_FROM_ERRNO_WITH_FILENAME: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyErr_SetFromErrnoWithFilename;
#[used]
static KEEP_PYERR_SET_FROM_ERRNO_WITH_FILENAME_OBJECT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyErr_SetFromErrnoWithFilenameObject;
#[used]
static KEEP_PYERR_SET_FROM_ERRNO_WITH_FILENAME_OBJECTS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyErr_SetFromErrnoWithFilenameObjects;
#[used]
static KEEP_PYERR_SET_EXC_FROM_WINDOWS_ERR: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    PyErr_SetExcFromWindowsErr;
#[used]
static KEEP_PYERR_SET_EXC_FROM_WINDOWS_ERR_WITH_FILENAME: unsafe extern "C" fn(
    *mut c_void,
    i32,
    *const c_char,
) -> *mut c_void = PyErr_SetExcFromWindowsErrWithFilename;
#[used]
static KEEP_PYERR_SET_EXC_FROM_WINDOWS_ERR_WITH_FILENAME_OBJECT:
    unsafe extern "C" fn(*mut c_void, i32, *mut c_void) -> *mut c_void =
    PyErr_SetExcFromWindowsErrWithFilenameObject;
#[used]
static KEEP_PYERR_SET_EXC_FROM_WINDOWS_ERR_WITH_FILENAME_OBJECTS:
    unsafe extern "C" fn(*mut c_void, i32, *mut c_void, *mut c_void) -> *mut c_void =
    PyErr_SetExcFromWindowsErrWithFilenameObjects;
#[used]
static KEEP_PYERR_SET_FROM_WINDOWS_ERR: unsafe extern "C" fn(i32) -> *mut c_void =
    PyErr_SetFromWindowsErr;
#[used]
static KEEP_PYERR_SET_FROM_WINDOWS_ERR_WITH_FILENAME: unsafe extern "C" fn(
    i32,
    *const c_char,
) -> *mut c_void = PyErr_SetFromWindowsErrWithFilename;
#[used]
static KEEP_PYERR_SET_INTERRUPT: unsafe extern "C" fn() = PyErr_SetInterrupt;
#[used]
static KEEP_PYERR_SET_INTERRUPT_EX: unsafe extern "C" fn(i32) -> i32 = PyErr_SetInterruptEx;
#[used]
static KEEP_PYERR_SYNTAX_LOCATION: unsafe extern "C" fn(*const c_char, i32) = PyErr_SyntaxLocation;
#[used]
static KEEP_PYERR_SYNTAX_LOCATION_EX: unsafe extern "C" fn(*const c_char, i32, i32) =
    PyErr_SyntaxLocationEx;
#[used]
static KEEP_PYERR_PROGRAM_TEXT: unsafe extern "C" fn(*const c_char, i32) -> *mut c_void =
    PyErr_ProgramText;
#[used]
static KEEP_PYERR_SET_IMPORT_ERROR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyErr_SetImportError;
#[used]
static KEEP_PYERR_SET_IMPORT_ERROR_SUBCLASS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyErr_SetImportErrorSubclass;
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
) -> *mut c_void = PyFile_FromFd;
#[used]
static KEEP_PYFILE_GET_LINE: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void = PyFile_GetLine;
#[used]
static KEEP_PYFILE_WRITE_OBJECT: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 =
    PyFile_WriteObject;
#[used]
static KEEP_PYFILE_WRITE_STRING: unsafe extern "C" fn(*const c_char, *mut c_void) -> i32 =
    PyFile_WriteString;
#[used]
static KEEP_PY_INCREF: unsafe extern "C" fn(*mut c_void) = Py_IncRef;
#[used]
static KEEP_PY_DECREF: unsafe extern "C" fn(*mut c_void) = Py_DecRef;
#[used]
static KEEP_PY__INCREF: unsafe extern "C" fn(*mut c_void) = _Py_IncRef;
#[used]
static KEEP_PY__DECREF: unsafe extern "C" fn(*mut c_void) = _Py_DecRef;
#[used]
static KEEP_PY__SETREFCNT: unsafe extern "C" fn(*mut c_void, isize) = _Py_SetRefcnt;
#[used]
static KEEP_PY__NEGATIVEREFCOUNT: unsafe extern "C" fn(*const c_char, c_int, *mut c_void) =
    _Py_NegativeRefcount;
#[used]
static KEEP_PY__CHECKRECURSIVECALL: unsafe extern "C" fn(*const c_char) -> c_int =
    _Py_CheckRecursiveCall;
#[used]
static KEEP_PY__OBJECT_GC_NEWVAR: unsafe extern "C" fn(
    *mut CpythonTypeObject,
    isize,
) -> *mut c_void = _PyObject_GC_NewVar;
#[used]
static KEEP_PY__OBJECT_GC_RESIZE: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    _PyObject_GC_Resize;
#[used]
static KEEP_PY_XINCREF: unsafe extern "C" fn(*mut c_void) = Py_XIncRef;
#[used]
static KEEP_PY_XDECREF: unsafe extern "C" fn(*mut c_void) = Py_XDecRef;
#[used]
static KEEP_PYBYTES_FROM_STRING: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyBytes_FromString;
#[used]
static KEEP_PYBYTES_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PyBytes_Size;
#[used]
static KEEP_PYBYTES_AS_STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_char = PyBytes_AsString;
#[used]
static KEEP_PYBYTES_AS_STRING_AND_SIZE: unsafe extern "C" fn(
    *mut c_void,
    *mut *mut c_char,
    *mut isize,
) -> i32 = PyBytes_AsStringAndSize;
#[used]
static KEEP_PYBYTES_REPR: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void = PyBytes_Repr;
#[used]
static KEEP_PYBYTES_DECODE_ESCAPE: unsafe extern "C" fn(
    *const c_char,
    isize,
    *const c_char,
    isize,
    *const c_char,
) -> *mut c_void = PyBytes_DecodeEscape;
#[used]
static KEEP_PYBYTES_JOIN: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyBytes_Join;
#[used]
static KEEP_PY_BYTES_JOIN_PRIVATE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    _PyBytes_Join;
#[used]
static KEEP_PYBYTEARRAY_FROM_STRING_AND_SIZE: unsafe extern "C" fn(
    *const c_char,
    isize,
) -> *mut c_void = PyByteArray_FromStringAndSize;
#[used]
static KEEP_PYBYTEARRAY_FROM_OBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyByteArray_FromObject;
#[used]
static KEEP_PYBYTEARRAY_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PyByteArray_Size;
#[used]
static KEEP_PYBYTEARRAY_AS_STRING: unsafe extern "C" fn(*mut c_void) -> *mut c_char =
    PyByteArray_AsString;
#[used]
static KEEP_PYBYTEARRAY_RESIZE: unsafe extern "C" fn(*mut c_void, isize) -> i32 =
    PyByteArray_Resize;
#[used]
static KEEP_PYBYTEARRAY_CONCAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyByteArray_Concat;
#[used]
static KEEP_PYBUFFER_RELEASE: unsafe extern "C" fn(*mut c_void) = PyBuffer_Release;
#[used]
static KEEP_PYCALLABLE_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = PyCallable_Check;
#[used]
static KEEP_PYINDEX_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = PyIndex_Check;
#[used]
static KEEP_PYFLOAT_AS_DOUBLE: unsafe extern "C" fn(*mut c_void) -> f64 = PyFloat_AsDouble;
#[used]
static KEEP_PYFLOAT_GET_MAX: unsafe extern "C" fn() -> f64 = PyFloat_GetMax;
#[used]
static KEEP_PYFLOAT_GET_MIN: unsafe extern "C" fn() -> f64 = PyFloat_GetMin;
#[used]
static KEEP_PYFLOAT_GET_INFO: unsafe extern "C" fn() -> *mut c_void = PyFloat_GetInfo;
#[used]
static KEEP_PYFLOAT_FROM_STRING: unsafe extern "C" fn(
    *mut c_void,
    *mut *mut c_char,
) -> *mut c_void = PyFloat_FromString;
#[used]
static KEEP_PYLONG_AS_LONG: unsafe extern "C" fn(*mut c_void) -> i64 = PyLong_AsLong;
#[used]
static KEEP_PYLONG_AS_LONGLONG: unsafe extern "C" fn(*mut c_void) -> i64 = PyLong_AsLongLong;
#[used]
static KEEP_PYLONG_AS_INT: unsafe extern "C" fn(*mut c_void) -> i32 = PyLong_AsInt;
#[used]
static KEEP_PYLONG_AS_INT32: unsafe extern "C" fn(*mut c_void, *mut i32) -> i32 = PyLong_AsInt32;
#[used]
static KEEP_PYLONG_AS_INT64: unsafe extern "C" fn(*mut c_void, *mut i64) -> i32 = PyLong_AsInt64;
#[used]
static KEEP_PYLONG_AS_UINT32: unsafe extern "C" fn(*mut c_void, *mut u32) -> i32 = PyLong_AsUInt32;
#[used]
static KEEP_PYLONG_AS_UINT64: unsafe extern "C" fn(*mut c_void, *mut u64) -> i32 = PyLong_AsUInt64;
#[used]
static KEEP_PYLONG_AS_SSIZE_T: unsafe extern "C" fn(*mut c_void) -> isize = PyLong_AsSsize_t;
#[used]
static KEEP_PYLONG_AS_SIZE_T: unsafe extern "C" fn(*mut c_void) -> usize = PyLong_AsSize_t;
#[used]
static KEEP_PYLONG_AS_UNSIGNED_LONG: unsafe extern "C" fn(*mut c_void) -> u64 =
    PyLong_AsUnsignedLong;
#[used]
static KEEP_PYLONG_AS_UNSIGNED_LONGLONG: unsafe extern "C" fn(*mut c_void) -> u64 =
    PyLong_AsUnsignedLongLong;
#[used]
static KEEP_PYLONG_AS_UNSIGNED_LONG_MASK: unsafe extern "C" fn(*mut c_void) -> u64 =
    PyLong_AsUnsignedLongMask;
#[used]
static KEEP_PYLONG_AS_UNSIGNED_LONGLONG_MASK: unsafe extern "C" fn(*mut c_void) -> u64 =
    PyLong_AsUnsignedLongLongMask;
#[used]
static KEEP_PYLONG_AS_NATIVE_BYTES: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
    i32,
) -> isize = PyLong_AsNativeBytes;
#[used]
static KEEP_PYLONG_FROM_NATIVE_BYTES: unsafe extern "C" fn(
    *const c_void,
    usize,
    i32,
) -> *mut c_void = PyLong_FromNativeBytes;
#[used]
static KEEP_PYLONG_FROM_UNSIGNED_NATIVE_BYTES: unsafe extern "C" fn(
    *const c_void,
    usize,
    i32,
) -> *mut c_void = PyLong_FromUnsignedNativeBytes;
#[used]
static KEEP_PYLONG_AS_VOID_PTR: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyLong_AsVoidPtr;
#[used]
static KEEP_PYLONG_AS_LONG_AND_OVERFLOW: unsafe extern "C" fn(*mut c_void, *mut i32) -> i64 =
    PyLong_AsLongAndOverflow;
#[used]
static KEEP_PYLONG_AS_LONGLONG_AND_OVERFLOW: unsafe extern "C" fn(*mut c_void, *mut i32) -> i64 =
    PyLong_AsLongLongAndOverflow;
#[used]
static KEEP_PYLONG_AS_DOUBLE: unsafe extern "C" fn(*mut c_void) -> f64 = PyLong_AsDouble;
#[used]
static KEEP_PYLONG_FROM_DOUBLE: unsafe extern "C" fn(f64) -> *mut c_void = PyLong_FromDouble;
#[used]
static KEEP_PYCOMPLEX_FROM_DOUBLES: unsafe extern "C" fn(f64, f64) -> *mut c_void =
    PyComplex_FromDoubles;
#[used]
static KEEP_PYCOMPLEX_FROM_CCOMPLEX: unsafe extern "C" fn(CpythonComplexValue) -> *mut c_void =
    PyComplex_FromCComplex;
#[used]
static KEEP_PYCOMPLEX_AS_CCOMPLEX: unsafe extern "C" fn(*mut c_void) -> CpythonComplexValue =
    PyComplex_AsCComplex;
#[used]
static KEEP_PYCOMPLEX_REAL_AS_DOUBLE: unsafe extern "C" fn(*mut c_void) -> f64 =
    PyComplex_RealAsDouble;
#[used]
static KEEP_PYCOMPLEX_IMAG_AS_DOUBLE: unsafe extern "C" fn(*mut c_void) -> f64 =
    PyComplex_ImagAsDouble;
#[used]
static KEEP_PYIMPORT_GET_MAGIC_NUMBER: unsafe extern "C" fn() -> c_long = PyImport_GetMagicNumber;
#[used]
static KEEP_PYIMPORT_GET_MAGIC_TAG: unsafe extern "C" fn() -> *const c_char = PyImport_GetMagicTag;
#[used]
static KEEP_PYIMPORT_IMPORT_MODULE: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyImport_ImportModule;
#[used]
static KEEP_PYIMPORT_IMPORT: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyImport_Import;
#[used]
static KEEP_PYIMPORT_GET_MODULE_DICT: unsafe extern "C" fn() -> *mut c_void =
    PyImport_GetModuleDict;
#[used]
static KEEP_PYIMPORT_ADD_MODULE_REF: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyImport_AddModuleRef;
#[used]
static KEEP_PYIMPORT_ADD_MODULE_OBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyImport_AddModuleObject;
#[used]
static KEEP_PYIMPORT_ADD_MODULE: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyImport_AddModule;
#[used]
static KEEP_PYIMPORT_APPEND_INITTAB: unsafe extern "C" fn(
    *const c_char,
    Option<CpythonInittabInitFunc>,
) -> i32 = PyImport_AppendInittab;
#[used]
static KEEP_PYIMPORT_IMPORT_FROZEN_MODULE: unsafe extern "C" fn(*const c_char) -> i32 =
    PyImport_ImportFrozenModule;
#[used]
static KEEP_PYIMPORT_IMPORT_FROZEN_MODULE_OBJECT: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyImport_ImportFrozenModuleObject;
#[used]
static KEEP_PYIMPORT_EXECCODEMODULE: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
) -> *mut c_void = PyImport_ExecCodeModule;
#[used]
static KEEP_PYIMPORT_EXECCODEMODULE_EX: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyImport_ExecCodeModuleEx;
#[used]
static KEEP_PYIMPORT_EXECCODEMODULE_OBJECT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyImport_ExecCodeModuleObject;
#[used]
static KEEP_PYIMPORT_EXECCODEMODULE_WITH_PATHNAMES: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyImport_ExecCodeModuleWithPathnames;
#[used]
static KEEP_PYIMPORT_GET_MODULE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyImport_GetModule;
#[used]
static KEEP_PYIMPORT_IMPORT_MODULE_NO_BLOCK: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyImport_ImportModuleNoBlock;
#[used]
static KEEP_PYIMPORT_GET_IMPORTER: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyImport_GetImporter;
#[used]
static KEEP_PYIMPORT_IMPORT_MODULE_LEVEL_OBJECT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    i32,
) -> *mut c_void = PyImport_ImportModuleLevelObject;
#[used]
static KEEP_PYIMPORT_IMPORT_MODULE_LEVEL: unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    i32,
) -> *mut c_void = PyImport_ImportModuleLevel;
#[used]
static KEEP_PYIMPORT_RELOAD_MODULE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyImport_ReloadModule;
#[used]
static KEEP_PYEVAL_GET_FRAME: unsafe extern "C" fn() -> *mut c_void = PyEval_GetFrame;
#[used]
static KEEP_PYEVAL_GET_BUILTINS: unsafe extern "C" fn() -> *mut c_void = PyEval_GetBuiltins;
#[used]
static KEEP_PYEVAL_GET_GLOBALS: unsafe extern "C" fn() -> *mut c_void = PyEval_GetGlobals;
#[used]
static KEEP_PYEVAL_GET_LOCALS: unsafe extern "C" fn() -> *mut c_void = PyEval_GetLocals;
#[used]
static KEEP_PYEVAL_GET_FRAME_BUILTINS: unsafe extern "C" fn() -> *mut c_void =
    PyEval_GetFrameBuiltins;
#[used]
static KEEP_PYEVAL_GET_FRAME_GLOBALS: unsafe extern "C" fn() -> *mut c_void =
    PyEval_GetFrameGlobals;
#[used]
static KEEP_PYEVAL_GET_FRAME_LOCALS: unsafe extern "C" fn() -> *mut c_void = PyEval_GetFrameLocals;
#[used]
static KEEP_PYEVAL_GET_FUNC_NAME: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    PyEval_GetFuncName;
#[used]
static KEEP_PYEVAL_GET_FUNC_DESC: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    PyEval_GetFuncDesc;
#[used]
static KEEP_PYITER_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = PyIter_Check;
#[used]
static KEEP_PYITER_NEXTITEM: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> i32 =
    PyIter_NextItem;
#[used]
static KEEP_PYITER_SEND: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32 =
    PyIter_Send;
#[used]
static KEEP_PYITER_NEXT: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyIter_Next;
#[used]
static KEEP_PYCAPSULE_NEW: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    Option<unsafe extern "C" fn(*mut c_void)>,
) -> *mut c_void = PyCapsule_New;
#[used]
static KEEP_PYCAPSULE_GET_POINTER: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
    PyCapsule_GetPointer;
#[used]
static KEEP_PYCAPSULE_GET_NAME: unsafe extern "C" fn(*mut c_void) -> *const c_char =
    PyCapsule_GetName;
#[used]
static KEEP_PYCAPSULE_SET_POINTER: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyCapsule_SetPointer;
#[used]
static KEEP_PYCAPSULE_GET_DESTRUCTOR: unsafe extern "C" fn(
    *mut c_void,
) -> Option<
    unsafe extern "C" fn(*mut c_void),
> = PyCapsule_GetDestructor;
#[used]
static KEEP_PYCAPSULE_SET_DESTRUCTOR: unsafe extern "C" fn(
    *mut c_void,
    Option<unsafe extern "C" fn(*mut c_void)>,
) -> i32 = PyCapsule_SetDestructor;
#[used]
static KEEP_PYCAPSULE_SET_CONTEXT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyCapsule_SetContext;
#[used]
static KEEP_PYCAPSULE_GET_CONTEXT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyCapsule_GetContext;
#[used]
static KEEP_PYCAPSULE_SET_NAME: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyCapsule_SetName;
#[used]
static KEEP_PYCAPSULE_IS_VALID: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyCapsule_IsValid;
#[used]
static KEEP_PYCAPSULE_IMPORT: unsafe extern "C" fn(*const c_char, i32) -> *mut c_void =
    PyCapsule_Import;
#[used]
static KEEP_PYLIST_NEW: unsafe extern "C" fn(isize) -> *mut c_void = PyList_New;
#[used]
static KEEP_PYLIST_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PyList_Size;
#[used]
static KEEP_PYLIST_APPEND: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 = PyList_Append;
#[used]
static KEEP_PYLIST_GET_ITEM: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    PyList_GetItem;
#[used]
static KEEP_PYLIST_GET_ITEM_REF: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    PyList_GetItemRef;
#[used]
static KEEP_PYLIST_SET_ITEM: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) -> i32 =
    PyList_SetItem;
#[used]
static KEEP_PYLIST_INSERT: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) -> i32 =
    PyList_Insert;
#[used]
static KEEP_PYLIST_GET_SLICE: unsafe extern "C" fn(*mut c_void, isize, isize) -> *mut c_void =
    PyList_GetSlice;
#[used]
static KEEP_PYLIST_SET_SLICE: unsafe extern "C" fn(*mut c_void, isize, isize, *mut c_void) -> i32 =
    PyList_SetSlice;
#[used]
static KEEP_PYLIST_SORT: unsafe extern "C" fn(*mut c_void) -> i32 = PyList_Sort;
#[used]
static KEEP_PYLIST_REVERSE: unsafe extern "C" fn(*mut c_void) -> i32 = PyList_Reverse;
#[used]
static KEEP_PYLIST_AS_TUPLE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyList_AsTuple;
#[used]
static KEEP_PYSET_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PySet_New;
#[used]
static KEEP_PYFROZENSET_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyFrozenSet_New;
#[used]
static KEEP_PYSET_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PySet_Size;
#[used]
static KEEP_PYSET_CONTAINS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 = PySet_Contains;
#[used]
static KEEP_PYSET_ADD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 = PySet_Add;
#[used]
static KEEP_PYSET_DISCARD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 = PySet_Discard;
#[used]
static KEEP_PYSET_CLEAR: unsafe extern "C" fn(*mut c_void) -> i32 = PySet_Clear;
#[used]
static KEEP_PYSET_POP: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PySet_Pop;
#[used]
static KEEP_PYDICT_NEW: unsafe extern "C" fn() -> *mut c_void = PyDict_New;
#[used]
static KEEP_PYDICT_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PyDict_Size;
#[used]
static KEEP_PYDICT_SET_ITEM: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
    PyDict_SetItem;
#[used]
static KEEP_PYDICT_SET_DEFAULT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyDict_SetDefault;
#[used]
static KEEP_PYDICT_SET_DEFAULT_REF: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32 = PyDict_SetDefaultRef;
#[used]
static KEEP_PYDICT_GET_ITEM: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyDict_GetItem;
#[used]
static KEEP_PYDICT_GET_ITEM_WITH_ERROR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyDict_GetItemWithError;
#[used]
static KEEP__PYDICT_GET_ITEM_KNOWNHASH: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    isize,
) -> *mut c_void = _PyDict_GetItem_KnownHash;
#[used]
static KEEP_PYDICT_CONTAINS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyDict_Contains;
#[used]
static KEEP_PYDICT_SET_ITEM_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = PyDict_SetItemString;
#[used]
static KEEP_PYDICT_GET_ITEM_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyDict_GetItemString;
#[used]
static KEEP_PYDICT_GET_ITEM_REF: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32 = PyDict_GetItemRef;
#[used]
static KEEP_PYDICT_GET_ITEM_STRING_REF: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut *mut c_void,
) -> i32 = PyDict_GetItemStringRef;
#[used]
static KEEP_PYDICT_POP: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32 =
    PyDict_Pop;
#[used]
static KEEP_PYDICT_POP_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut *mut c_void,
) -> i32 = PyDict_PopString;
#[used]
static KEEP_PY_DICT_POP_PRIVATE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = _PyDict_Pop;
#[used]
static KEEP_PYDICT_DEL_ITEM: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 = PyDict_DelItem;
#[used]
static KEEP_PYDICT_DEL_ITEM_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyDict_DelItemString;
#[used]
static KEEP_PYDICT_CONTAINS_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyDict_ContainsString;
#[used]
static KEEP_PYDICT_COPY: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyDict_Copy;
#[used]
static KEEP_PYDICT_CLEAR: unsafe extern "C" fn(*mut c_void) = PyDict_Clear;
#[used]
static KEEP_PYDICT_MERGE: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 = PyDict_Merge;
#[used]
static KEEP_PYDICT_UPDATE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 = PyDict_Update;
#[used]
static KEEP_PYDICT_MERGE_FROM_SEQ2: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 =
    PyDict_MergeFromSeq2;
#[used]
static KEEP_PYDICT_KEYS: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyDict_Keys;
#[used]
static KEEP_PYDICT_VALUES: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyDict_Values;
#[used]
static KEEP_PYDICT_ITEMS: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyDict_Items;
#[used]
static KEEP_PYDICT_NEXT: unsafe extern "C" fn(
    *mut c_void,
    *mut isize,
    *mut *mut c_void,
    *mut *mut c_void,
) -> i32 = PyDict_Next;
#[used]
static KEEP_PYDICT_PROXY_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyDictProxy_New;
#[used]
static KEEP_PYOBJECT_GETATTR_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyObject_GetAttrString;
#[used]
static KEEP_PYOBJECT_GETATTR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyObject_GetAttr;
#[used]
static KEEP_PYOBJECT_GENERIC_GETATTR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyObject_GenericGetAttr;
#[used]
static KEEP_PYOBJECT_GENERIC_SETATTR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> i32 = PyObject_GenericSetAttr;
#[used]
static KEEP_PYOBJECT_GENERIC_GETDICT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyObject_GenericGetDict;
#[used]
static KEEP_PYOBJECT_GENERIC_SETDICT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> i32 = PyObject_GenericSetDict;
#[used]
static KEEP_PYOBJECT_SETATTR_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> i32 = PyObject_SetAttrString;
#[used]
static KEEP_PYOBJECT_SETATTR: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
    PyObject_SetAttr;
#[used]
static KEEP_PYOBJECT_DELATTR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyObject_DelAttr;
#[used]
static KEEP_PYOBJECT_DELATTR_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyObject_DelAttrString;
#[used]
static KEEP_PYOBJECT_DELITEM_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyObject_DelItemString;
#[used]
static KEEP_PYOBJECT_HASATTR_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyObject_HasAttrString;
#[used]
static KEEP_PYOBJECT_HASATTR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyObject_HasAttr;
#[used]
static KEEP_PYOBJECT_HASATTR_WITH_ERROR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyObject_HasAttrWithError;
#[used]
static KEEP_PYOBJECT_HASATTR_STRING_WITH_ERROR: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> i32 = PyObject_HasAttrStringWithError;
#[used]
static KEEP_PYOBJECT_GETOPTIONALATTR_STRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut *mut c_void,
) -> i32 = PyObject_GetOptionalAttrString;
#[used]
static KEEP_PYOBJECT_ISTRUE: unsafe extern "C" fn(*mut c_void) -> i32 = PyObject_IsTrue;
#[used]
static KEEP_PYOBJECT_NOT: unsafe extern "C" fn(*mut c_void) -> i32 = PyObject_Not;
#[used]
static KEEP_PYOBJECT_STR: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_Str;
#[used]
static KEEP_PYOBJECT_REPR: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_Repr;
#[used]
static KEEP_PYOBJECT_ASCII: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_ASCII;
#[used]
static KEEP_PYOBJECT_DIR: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_Dir;
#[used]
static KEEP_PYOBJECT_BYTES: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_Bytes;
#[used]
static KEEP_PYOBJECT_FORMAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyObject_Format;
#[used]
static KEEP_PYOBJECT_GETITER: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_GetIter;
#[used]
static KEEP_PYAITER_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = PyAIter_Check;
#[used]
static KEEP_PYOBJECT_GETAITER: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_GetAIter;
#[used]
static KEEP_PYOBJECT_SELFITER: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_SelfIter;
#[used]
static KEEP_PYOBJECT_CALLOBJECT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyObject_CallObject;
#[used]
static KEEP_PYOBJECT_CALL: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyObject_Call;
#[used]
static KEEP_PYOBJECT_CALL_ONEARG: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyObject_CallOneArg;
#[used]
static KEEP_PYOBJECT_CALL_NOARGS: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyObject_CallNoArgs;
#[used]
static KEEP_PYOBJECT_CALL_FINALIZER: unsafe extern "C" fn(*mut c_void) = PyObject_CallFinalizer;
#[used]
static KEEP_PYOBJECT_CALL_FINALIZER_FROM_DEALLOC: unsafe extern "C" fn(*mut c_void) -> c_int =
    PyObject_CallFinalizerFromDealloc;
#[used]
static KEEP_PYOBJECT_CLEARMANAGEDDICT: unsafe extern "C" fn(*mut c_void) =
    PyObject_ClearManagedDict;
#[used]
static KEEP_PYOBJECT_VISITMANAGEDDICT: unsafe extern "C" fn(
    *mut c_void,
    Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    *mut c_void,
) -> c_int = PyObject_VisitManagedDict;
#[used]
static KEEP_PYUNSTABLE_OBJECT_ENABLE_DEFERRED_REFCOUNT: unsafe extern "C" fn(*mut c_void) -> c_int =
    PyUnstable_Object_EnableDeferredRefcount;
#[used]
static KEEP_PYCFUNCTION_CALL: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyCFunction_Call;
#[used]
static KEEP_PYCFUNCTION_NEW: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyCFunction_New;
#[used]
static KEEP_PYCFUNCTION_NEW_EX: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyCFunction_NewEx;
#[used]
static KEEP_PYCMETHOD_NEW: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyCMethod_New;
#[used]
static KEEP_PYDESCR_NEW_METHOD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyDescr_NewMethod;
#[used]
static KEEP_PYDESCR_NEW_CLASS_METHOD: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyDescr_NewClassMethod;
#[used]
static KEEP_PYWRAPPER_NEW: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyWrapper_New;
#[used]
static KEEP_PYDESCR_NEW_MEMBER: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyDescr_NewMember;
#[used]
static KEEP_PYMEMBER_GET_ONE: unsafe extern "C" fn(*const c_char, *mut c_void) -> *mut c_void =
    PyMember_GetOne;
#[used]
static KEEP_PYMEMBER_SET_ONE: unsafe extern "C" fn(*mut c_char, *mut c_void, *mut c_void) -> c_int =
    PyMember_SetOne;
#[used]
static KEEP_PYDESCR_NEW_GETSET: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyDescr_NewGetSet;
#[used]
static KEEP_PYCFUNCTION_GET_FUNCTION: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyCFunction_GetFunction;
#[used]
static KEEP_PYCFUNCTION_GET_SELF: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyCFunction_GetSelf;
#[used]
static KEEP_PYCFUNCTION_GET_FLAGS: unsafe extern "C" fn(*mut c_void) -> i32 = PyCFunction_GetFlags;
#[used]
static KEEP_PYERR_SET_OBJECT: unsafe extern "C" fn(*mut c_void, *mut c_void) = PyErr_SetObject;
#[used]
static KEEP_PYERR_SET_NONE: unsafe extern "C" fn(*mut c_void) = PyErr_SetNone;
#[used]
static KEEP_PYERR_NOMEMORY: unsafe extern "C" fn() -> *mut c_void = PyErr_NoMemory;
#[used]
static KEEP_PYERR_CHECK_SIGNALS: unsafe extern "C" fn() -> i32 = PyErr_CheckSignals;
#[used]
static KEEP_PYGILSTATE_ENSURE: unsafe extern "C" fn() -> i32 = PyGILState_Ensure;
#[used]
static KEEP_PYGILSTATE_GET_THIS_THREAD_STATE: unsafe extern "C" fn() -> *mut c_void =
    PyGILState_GetThisThreadState;
#[used]
static KEEP_PYGILSTATE_RELEASE: unsafe extern "C" fn(i32) = PyGILState_Release;
#[used]
static KEEP_PYEVAL_ACQUIRE_LOCK: unsafe extern "C" fn() = PyEval_AcquireLock;
#[used]
static KEEP_PYEVAL_RELEASE_LOCK: unsafe extern "C" fn() = PyEval_ReleaseLock;
#[used]
static KEEP_PYEVAL_ACQUIRE_THREAD: unsafe extern "C" fn(*mut c_void) = PyEval_AcquireThread;
#[used]
static KEEP_PYEVAL_RELEASE_THREAD: unsafe extern "C" fn(*mut c_void) = PyEval_ReleaseThread;
#[used]
static KEEP_PYEVAL_INIT_THREADS: unsafe extern "C" fn() = PyEval_InitThreads;
#[used]
static KEEP_PYEVAL_THREADS_INITIALIZED: unsafe extern "C" fn() -> i32 = PyEval_ThreadsInitialized;
#[used]
static KEEP_PYEVAL_CALL_OBJECT_WITH_KEYWORDS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyEval_CallObjectWithKeywords;
#[used]
static KEEP_PYEVAL_EVAL_CODE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyEval_EvalCode;
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
) -> *mut c_void = PyEval_EvalCodeEx;
#[used]
static KEEP_PYEVAL_SAVE_THREAD: unsafe extern "C" fn() -> *mut c_void = PyEval_SaveThread;
#[used]
static KEEP_PYEVAL_RESTORE_THREAD: unsafe extern "C" fn(*mut c_void) = PyEval_RestoreThread;
#[used]
static KEEP_PYINTERPRETERSTATE_MAIN: unsafe extern "C" fn() -> *mut c_void =
    PyInterpreterState_Main;
#[used]
static KEEP_PYNUMBER_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = PyNumber_Check;
#[used]
static KEEP_PYNUMBER_ABSOLUTE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyNumber_Absolute;
#[used]
static KEEP_PYNUMBER_ADD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_Add;
#[used]
static KEEP_PYNUMBER_SUBTRACT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_Subtract;
#[used]
static KEEP_PYNUMBER_MULTIPLY: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_Multiply;
#[used]
static KEEP_PYNUMBER_TRUE_DIVIDE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_TrueDivide;
#[used]
static KEEP_PYNUMBER_FLOOR_DIVIDE: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_FloorDivide;
#[used]
static KEEP_PYNUMBER_REMAINDER: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_Remainder;
#[used]
static KEEP_PYNUMBER_DIVMOD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_Divmod;
#[used]
static KEEP_PYNUMBER_POWER: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyNumber_Power;
#[used]
static KEEP_PYNUMBER_MATRIX_MULTIPLY: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyNumber_MatrixMultiply;
#[used]
static KEEP_PYNUMBER_LSHIFT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_Lshift;
#[used]
static KEEP_PYNUMBER_RSHIFT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_Rshift;
#[used]
static KEEP_PYNUMBER_AND: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_And;
#[used]
static KEEP_PYNUMBER_OR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_Or;
#[used]
static KEEP_PYNUMBER_XOR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_Xor;
#[used]
static KEEP_PYNUMBER_INPLACE_ADD: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_InPlaceAdd;
#[used]
static KEEP_PYNUMBER_INPLACE_SUBTRACT: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyNumber_InPlaceSubtract;
#[used]
static KEEP_PYNUMBER_INPLACE_MULTIPLY: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyNumber_InPlaceMultiply;
#[used]
static KEEP_PYNUMBER_INPLACE_MATRIX_MULTIPLY: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyNumber_InPlaceMatrixMultiply;
#[used]
static KEEP_PYNUMBER_INPLACE_FLOOR_DIVIDE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyNumber_InPlaceFloorDivide;
#[used]
static KEEP_PYNUMBER_INPLACE_TRUE_DIVIDE: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyNumber_InPlaceTrueDivide;
#[used]
static KEEP_PYNUMBER_INPLACE_REMAINDER: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyNumber_InPlaceRemainder;
#[used]
static KEEP_PYNUMBER_INPLACE_POWER: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyNumber_InPlacePower;
#[used]
static KEEP_PYNUMBER_INPLACE_LSHIFT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_InPlaceLshift;
#[used]
static KEEP_PYNUMBER_INPLACE_RSHIFT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_InPlaceRshift;
#[used]
static KEEP_PYNUMBER_INPLACE_AND: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_InPlaceAnd;
#[used]
static KEEP_PYNUMBER_INPLACE_OR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_InPlaceOr;
#[used]
static KEEP_PYNUMBER_INPLACE_XOR: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyNumber_InPlaceXor;
#[used]
static KEEP_PYNUMBER_NEGATIVE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyNumber_Negative;
#[used]
static KEEP_PYNUMBER_POSITIVE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyNumber_Positive;
#[used]
static KEEP_PYNUMBER_INVERT: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyNumber_Invert;
#[used]
static KEEP_PYNUMBER_LONG: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyNumber_Long;
#[used]
static KEEP_PYNUMBER_FLOAT: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyNumber_Float;
#[used]
static KEEP_PYNUMBER_INDEX: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyNumber_Index;
#[used]
static KEEP_PYNUMBER_AS_SSIZE_T: unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize =
    PyNumber_AsSsize_t;
#[used]
static KEEP_PYNUMBER_TO_BASE: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
    PyNumber_ToBase;
#[used]
static KEEP_PYMEM_RAW_MALLOC: unsafe extern "C" fn(usize) -> *mut c_void = PyMem_RawMalloc;
#[used]
static KEEP_PYMEM_RAW_CALLOC: unsafe extern "C" fn(usize, usize) -> *mut c_void = PyMem_RawCalloc;
#[used]
static KEEP_PYMEM_RAW_REALLOC: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void =
    PyMem_RawRealloc;
#[used]
static KEEP_PYMEM_RAW_FREE: unsafe extern "C" fn(*mut c_void) = PyMem_RawFree;
#[used]
static KEEP_PYMARSHAL_READ_OBJECT_FROM_STRING: unsafe extern "C" fn(
    *const c_char,
    isize,
) -> *mut c_void = PyMarshal_ReadObjectFromString;
#[used]
static KEEP_PYMARSHAL_WRITE_OBJECT_TO_STRING: unsafe extern "C" fn(
    *mut c_void,
    i32,
) -> *mut c_void = PyMarshal_WriteObjectToString;
#[used]
static KEEP_PYMEM_MALLOC: unsafe extern "C" fn(usize) -> *mut c_void = PyMem_Malloc;
#[used]
static KEEP_PYMEM_CALLOC: unsafe extern "C" fn(usize, usize) -> *mut c_void = PyMem_Calloc;
#[used]
static KEEP_PYMEM_REALLOC: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void = PyMem_Realloc;
#[used]
static KEEP_PYMEM_FREE: unsafe extern "C" fn(*mut c_void) = PyMem_Free;

unsafe fn c_name_to_string(name: *const c_char) -> Result<String, String> {
    if name.is_null() {
        return Err("received null C string pointer".to_string());
    }
    // SAFETY: caller ensures pointer is a valid NUL-terminated C string.
    let c_name = unsafe { CStr::from_ptr(name) };
    c_name
        .to_str()
        .map(|text| text.to_string())
        .map_err(|_| "received non-utf8 C string".to_string())
}

#[cfg(windows)]
fn cpython_wide_units_to_string(code_units: &[Cwchar]) -> Result<String, String> {
    String::from_utf16(code_units).map_err(|_| "received invalid UTF-16 wide string".to_string())
}

#[cfg(not(windows))]
fn cpython_wide_units_to_string(code_units: &[Cwchar]) -> Result<String, String> {
    let mut text = String::new();
    for unit in code_units {
        if *unit < 0 {
            return Err("received invalid negative wide char value".to_string());
        }
        let Some(ch) = char::from_u32(*unit as u32) else {
            return Err("received invalid unicode scalar in wide string".to_string());
        };
        text.push(ch);
    }
    Ok(text)
}

#[cfg(windows)]
fn cpython_string_to_wide_units(text: &str) -> Vec<Cwchar> {
    text.encode_utf16().collect()
}

#[cfg(not(windows))]
fn cpython_string_to_wide_units(text: &str) -> Vec<Cwchar> {
    text.chars().map(|ch| ch as u32 as Cwchar).collect()
}

unsafe fn cpython_wide_ptr_to_string(
    value: *const Cwchar,
    len: isize,
    api_name: &str,
) -> Result<String, String> {
    if len < -1 {
        return Err(format!("{api_name} received negative length"));
    }
    if value.is_null() {
        if len == 0 {
            return Ok(String::new());
        }
        return Err(format!(
            "{api_name} received null wide string pointer with non-zero length"
        ));
    }
    let units: Vec<Cwchar> = if len < 0 {
        let mut collected = Vec::new();
        let mut cursor = value;
        loop {
            // SAFETY: caller guarantees NUL-terminated wide string for `len == -1`.
            let unit = unsafe { *cursor };
            if unit == 0 {
                break;
            }
            collected.push(unit);
            // SAFETY: advancing across caller-provided NUL-terminated wide string.
            cursor = unsafe { cursor.add(1) };
        }
        collected
    } else if len == 0 {
        Vec::new()
    } else {
        // SAFETY: caller guarantees at least `len` wide units at `value`.
        unsafe { std::slice::from_raw_parts(value, len as usize).to_vec() }
    };
    cpython_wide_units_to_string(&units)
}

unsafe fn c_wide_name_to_string(name: *const Cwchar) -> Result<String, String> {
    unsafe { cpython_wide_ptr_to_string(name, -1, "wide string decode") }
}

fn value_to_cpython_marshal_object(value: &Value) -> Result<CpythonMarshalObject, String> {
    match value {
        Value::None => Ok(CpythonMarshalObject::None),
        Value::Bool(value) => Ok(CpythonMarshalObject::Bool(*value)),
        Value::Int(value) => Ok(CpythonMarshalObject::Int(*value)),
        Value::BigInt(value) => value
            .to_i64()
            .map(CpythonMarshalObject::Int)
            .ok_or_else(|| "cannot marshal bigint values outside i64 range".to_string()),
        Value::Float(value) => Ok(CpythonMarshalObject::Float(*value)),
        Value::Complex { real, imag } => Ok(CpythonMarshalObject::Complex {
            real: *real,
            imag: *imag,
        }),
        Value::Str(value) => Ok(CpythonMarshalObject::Str(value.clone())),
        Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
            Object::Bytes(payload) => Ok(CpythonMarshalObject::Bytes(payload.clone())),
            _ => Err("invalid bytes object storage".to_string()),
        },
        Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
            Object::Tuple(items) => items
                .iter()
                .map(value_to_cpython_marshal_object)
                .collect::<Result<Vec<_>, _>>()
                .map(CpythonMarshalObject::Tuple),
            _ => Err("invalid tuple object storage".to_string()),
        },
        Value::List(list_obj) => match &*list_obj.kind() {
            Object::List(items) => items
                .iter()
                .map(value_to_cpython_marshal_object)
                .collect::<Result<Vec<_>, _>>()
                .map(CpythonMarshalObject::List),
            _ => Err("invalid list object storage".to_string()),
        },
        Value::Dict(dict_obj) => match &*dict_obj.kind() {
            Object::Dict(entries) => entries
                .iter()
                .map(|(key, value)| {
                    Ok((
                        value_to_cpython_marshal_object(key)?,
                        value_to_cpython_marshal_object(value)?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()
                .map(CpythonMarshalObject::Dict),
            _ => Err("invalid dict object storage".to_string()),
        },
        Value::Set(set_obj) => match &*set_obj.kind() {
            Object::Set(entries) => entries
                .iter()
                .map(value_to_cpython_marshal_object)
                .collect::<Result<Vec<_>, _>>()
                .map(CpythonMarshalObject::Set),
            _ => Err("invalid set object storage".to_string()),
        },
        Value::FrozenSet(set_obj) => match &*set_obj.kind() {
            Object::FrozenSet(entries) => entries
                .iter()
                .map(value_to_cpython_marshal_object)
                .collect::<Result<Vec<_>, _>>()
                .map(CpythonMarshalObject::FrozenSet),
            _ => Err("invalid frozenset object storage".to_string()),
        },
        Value::Slice(slice) => Ok(CpythonMarshalObject::Slice {
            lower: slice
                .lower
                .map(|value| Box::new(CpythonMarshalObject::Int(value))),
            upper: slice
                .upper
                .map(|value| Box::new(CpythonMarshalObject::Int(value))),
            step: slice
                .step
                .map(|value| Box::new(CpythonMarshalObject::Int(value))),
        }),
        _ => Err("marshal unsupported value type".to_string()),
    }
}

fn cpython_marshal_object_to_value(
    object: &CpythonMarshalObject,
    vm: &mut Vm,
) -> Result<Value, String> {
    match object {
        CpythonMarshalObject::Null => Ok(Value::None),
        CpythonMarshalObject::None => Ok(Value::None),
        CpythonMarshalObject::Bool(value) => Ok(Value::Bool(*value)),
        CpythonMarshalObject::Int(value) => Ok(Value::Int(*value)),
        CpythonMarshalObject::Float(value) => Ok(Value::Float(*value)),
        CpythonMarshalObject::Complex { real, imag } => Ok(Value::Complex {
            real: *real,
            imag: *imag,
        }),
        CpythonMarshalObject::Str(value) => Ok(Value::Str(value.clone())),
        CpythonMarshalObject::Bytes(bytes) => Ok(vm.heap.alloc_bytes(bytes.clone())),
        CpythonMarshalObject::Tuple(items) => items
            .iter()
            .map(|item| cpython_marshal_object_to_value(item, vm))
            .collect::<Result<Vec<_>, _>>()
            .map(|items| vm.heap.alloc_tuple(items)),
        CpythonMarshalObject::List(items) => items
            .iter()
            .map(|item| cpython_marshal_object_to_value(item, vm))
            .collect::<Result<Vec<_>, _>>()
            .map(|items| vm.heap.alloc_list(items)),
        CpythonMarshalObject::Dict(entries) => entries
            .iter()
            .map(|(key, value)| {
                Ok((
                    cpython_marshal_object_to_value(key, vm)?,
                    cpython_marshal_object_to_value(value, vm)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()
            .map(|entries| vm.heap.alloc_dict(entries)),
        CpythonMarshalObject::Set(items) => items
            .iter()
            .map(|item| cpython_marshal_object_to_value(item, vm))
            .collect::<Result<Vec<_>, _>>()
            .map(|items| vm.heap.alloc_set(items)),
        CpythonMarshalObject::FrozenSet(items) => items
            .iter()
            .map(|item| cpython_marshal_object_to_value(item, vm))
            .collect::<Result<Vec<_>, _>>()
            .map(|items| vm.heap.alloc_frozenset(items)),
        CpythonMarshalObject::Slice { lower, upper, step } => {
            let parse_int =
                |value: &Option<Box<CpythonMarshalObject>>| -> Result<Option<i64>, String> {
                    match value {
                        None => Ok(None),
                        Some(value) => match value.as_ref() {
                            CpythonMarshalObject::Int(value) => Ok(Some(*value)),
                            _ => Err("marshal slice bounds must decode to int".to_string()),
                        },
                    }
                };
            Ok(Value::Slice(Box::new(SliceValue {
                lower: parse_int(lower)?,
                upper: parse_int(upper)?,
                step: parse_int(step)?,
            })))
        }
        CpythonMarshalObject::Code(_) => {
            Err("marshal code objects are not supported in C-API decode".to_string())
        }
    }
}

unsafe fn capi_module_insert_value(
    context: &mut ModuleCapiContext,
    name: *const c_char,
    value: Value,
) -> i32 {
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        context.set_error("module context no longer points to a module");
        return -1;
    };
    module_data.globals.insert(name, value);
    0
}
