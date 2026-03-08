//! CPython C-API compatibility layer and extension-runtime substrate.
//!
//! This module translates raw CPython-facing pointers/calls into pyrs runtime
//! objects while tracking ownership/lifetime through `ModuleCapiContext` and
//! VM-global C-API registries.
#![cfg_attr(target_arch = "wasm32", allow(unused_imports, dead_code))]

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::atomic::{AtomicI32, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, Once, OnceLock};

use self::cpython_context_runtime::ActiveCpythonContextGuard;
use super::capi_registry::{BorrowedRef, CapiPtrProvenance, CapiRefKind, OwnedRef, StolenRef};
use super::{
    BYTES_BACKING_STORAGE_ATTR, ExtensionCallableKind, GeneratorResumeOutcome, InternalCallOutcome,
    NativeCallResult, ObjRef, STR_BACKING_STORAGE_ATTR, Vm, add_values, dict_contains_key_checked,
    dict_get_value, dict_remove_value, dict_set_value_checked, exception_type_is_subclass,
    is_truthy, memoryview_bounds, value_to_int, vm_current_thread_ident,
};
use crate::extensions::{
    PYRS_CAPI_ABI_VERSION, PYRS_TYPE_BOOL, PYRS_TYPE_BYTES, PYRS_TYPE_DICT, PYRS_TYPE_FLOAT,
    PYRS_TYPE_INT, PYRS_TYPE_LIST, PYRS_TYPE_NONE, PYRS_TYPE_STR, PYRS_TYPE_TUPLE, PyrsApiV1,
    PyrsBufferInfoV1, PyrsBufferInfoV2, PyrsBufferViewV1, PyrsCFunctionKwV1, PyrsCFunctionV1,
    PyrsCapsuleDestructorV1, PyrsModuleStateFinalizeV1, PyrsModuleStateFreeV1, PyrsObjectHandle,
    PyrsWritableBufferViewV1,
};
use crate::runtime::{
    BigInt, BoundMethod, BuiltinFunction, ClassObject, DictViewKind, InstanceObject,
    NativeMethodKind, NativeMethodObject, ObjDropHook, Object, RuntimeError,
    TestCapiStringParseKind, Value, register_obj_drop_hook, value_lookup_hash,
};
use crate::vm::ExtensionModuleStateEntry;

#[cfg(windows)]
type Cwchar = u16;
#[cfg(not(windows))]
type Cwchar = i32;

const CPY_PROXY_CLASS_NAME: &str = "__pyrs_cpython_proxy__";
const CPY_PROXY_PTR_ATTR: &str = "__pyrs_cpython_proxy_ptr__";
const CPY_PROXY_MARKER_ATTR: &str = "__pyrs_cpython_proxy_marker__";
const CPY_EXCEPTION_TYPE_PTR_ATTR: &str = "__pyrs_cpython_exception_type_ptr__";
const CPY_FRAME_MODULE_NAME: &str = "__pyframe__";
const CPY_RICHCMP_LT: i32 = 0;
const CPY_RICHCMP_LE: i32 = 1;
const CPY_RICHCMP_EQ: i32 = 2;
const CPY_RICHCMP_NE: i32 = 3;
const CPY_RICHCMP_GT: i32 = 4;
const CPY_RICHCMP_GE: i32 = 5;
pub(crate) const MIN_VALID_PTR_THRESHOLD: usize = if usize::BITS > 32 {
    0x1_0000_0000_u64 as usize
} else {
    0x1_0000_u64 as usize
};
static TRACE_NUMPY_TYPEDICT_PTR: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    static CPYTHON_DESCRIPTOR_REGISTRY: RefCell<HashMap<usize, CpythonDescriptorKind>> =
        RefCell::new(HashMap::new());
}
mod callable_runtime;
mod capi_perf_runtime;
mod capi_v1;
mod cpython_args_runtime;
mod cpython_bigint_runtime;
mod cpython_bytes_api;
mod cpython_call_runtime;
mod cpython_capsule_api;
mod cpython_codec_api;
mod cpython_codec_runtime;
mod cpython_context_runtime;
mod cpython_contextvar_api;
mod cpython_datetime_runtime;
mod cpython_descriptor_method_api;
mod cpython_dict_api;
mod cpython_error_numeric_api;
mod cpython_eval_api;
mod cpython_eval_os_marshal_api;
mod cpython_exception_file_api;
mod cpython_exception_name_runtime;
mod cpython_gc_alloc_api;
mod cpython_import_api;
mod cpython_import_runtime;
mod cpython_iter_api;
#[cfg(not(target_arch = "wasm32"))]
mod cpython_keepalive_exports;
mod cpython_list_api;
mod cpython_long_float_api;
mod cpython_marshal_runtime;
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
mod cpython_sequence_mapping_api;
mod cpython_set_api;
mod cpython_slot_runtime;
mod cpython_string_runtime;
mod cpython_sys_thread_api;
mod cpython_thread_interp_api;
mod cpython_thread_runtime;
mod cpython_tuple_api;
mod cpython_type_api;
mod cpython_type_exports;
mod cpython_type_layout;
mod cpython_unicode_api;
mod cpython_unicode_error_api;
mod cpython_unicode_error_runtime;
mod cpython_value_runtime;
mod cpython_weakref_api;
mod loader_runtime;
mod module_context_state;
mod proxy_runtime;

pub(crate) use self::capi_perf_runtime::capi_perf_snapshot;
use self::capi_perf_runtime::{
    capi_perf_inc_handle_from_ptr_calls, capi_perf_inc_handle_from_ptr_hits,
    capi_perf_inc_py_decref_calls, capi_perf_inc_py_decref_handle_hits,
    capi_perf_inc_py_incref_calls, capi_perf_inc_py_incref_handle_hits,
    capi_perf_inc_richcompare_bool_calls, capi_perf_inc_richcompare_calls,
    capi_perf_inc_richcompare_dunder_attr_missing,
    capi_perf_inc_richcompare_dunder_callable_invocations,
    capi_perf_inc_richcompare_dunder_calls_external, capi_perf_inc_richcompare_dunder_calls_owned,
    capi_perf_inc_richcompare_dunder_fallback_attempts, capi_perf_inc_richcompare_slot_attempts,
    capi_perf_inc_value_from_ptr_calls,
};
use self::cpython_args_runtime::{
    cpython_keyword_args_from_dict_object, cpython_positional_args_from_tuple_object,
};
use self::cpython_bigint_runtime::{
    cpython_asnativebytes_resolve_endian, cpython_bigint_from_twos_complement_le,
    cpython_bigint_from_value, cpython_bigint_to_twos_complement_le,
    cpython_required_signed_bytes_for_bigint, cpython_required_unsigned_bytes_for_bigint,
};
use self::cpython_bytes_api::{
    _PyBytes_Join, _PyBytes_Resize, PyByteArray_AsString, PyByteArray_Concat,
    PyByteArray_FromObject, PyByteArray_FromStringAndSize, PyByteArray_Resize, PyByteArray_Size,
    PyBytes_AsString, PyBytes_AsStringAndSize, PyBytes_Concat, PyBytes_ConcatAndDel,
    PyBytes_DecodeEscape, PyBytes_FromObject, PyBytes_FromString, PyBytes_FromStringAndSize,
    PyBytes_Join, PyBytes_Repr, PyBytes_Resize, PyBytes_Size,
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
    cpython_new_bytes_ptr, cpython_new_ptr_for_value, cpython_set_error, cpython_set_typed_error,
    cpython_trace_flag_enabled, cpython_trace_numpy_reduce_enabled, cpython_value_from_ptr,
    cpython_value_from_ptr_or_proxy, with_active_cpython_context_mut,
};
use self::cpython_contextvar_api::{
    PyContextVar_Get, PyContextVar_New, PyContextVar_Reset, PyContextVar_Set,
};
use self::cpython_datetime_runtime::{
    PYRS_DATETIME_CAPI, PYRS_DATETIME_CAPSULE_NAME, PYRS_DATETIME_DATE_TYPE,
    PYRS_DATETIME_DATETIME_TYPE, PYRS_DATETIME_DELTA_TYPE, PYRS_DATETIME_TIME_TYPE,
    PYRS_DATETIME_TZINFO_TYPE, initialize_datetime_capi_types,
};
use self::cpython_descriptor_method_api::{
    _PyClassMethod_New, _PyStaticMethod_New, PyCFunction_Call, PyCFunction_GetFlags,
    PyCFunction_GetFunction, PyCFunction_GetSelf, PyCFunction_New, PyCFunction_NewEx,
    PyCMethod_New, PyClassMethod_New, PyDescr_NewClassMethod, PyDescr_NewGetSet, PyDescr_NewMember,
    PyDescr_NewMethod, PyMember_GetOne, PyMember_SetOne, PySlice_AdjustIndices, PySlice_GetIndices,
    PySlice_GetIndicesEx, PySlice_New, PySlice_Unpack, PyStaticMethod_New, PyWrapper_New,
    cpython_cfunction_tp_call, cpython_cfunction_tp_descr_get, cpython_cfunction_tp_getattro,
    cpython_function_tp_descr_get, cpython_invoke_method_from_values,
    cpython_method_descriptor_tp_call, cpython_method_descriptor_tp_descr_get,
    cpython_method_tp_call,
};
use self::cpython_dict_api::{
    _PyDict_GetItem_KnownHash, _PyDict_NewPresized, _PyDict_Pop, PY_DICT_MAPPING_METHODS,
    PyDict_Clear, PyDict_Contains, PyDict_ContainsString, PyDict_Copy, PyDict_DelItem,
    PyDict_DelItemString, PyDict_GetItem, PyDict_GetItemRef, PyDict_GetItemString,
    PyDict_GetItemStringRef, PyDict_GetItemWithError, PyDict_Items, PyDict_Keys, PyDict_Merge,
    PyDict_MergeFromSeq2, PyDict_New, PyDict_Next, PyDict_Pop, PyDict_PopString, PyDict_SetDefault,
    PyDict_SetDefaultRef, PyDict_SetItem, PyDict_SetItemString, PyDict_Size, PyDict_Update,
    PyDict_Values, PyDictProxy_New,
};
use self::cpython_error_numeric_api::{
    Py_GenericAlias, PyComplex_AsCComplex, PyComplex_FromCComplex, PyComplex_FromDoubles,
    PyComplex_ImagAsDouble, PyComplex_RealAsDouble, PyErr_Clear, PyErr_Display,
    PyErr_DisplayException, PyErr_ExceptionMatches, PyErr_Fetch, PyErr_GetExcInfo,
    PyErr_GetHandledException, PyErr_GetRaisedException, PyErr_GivenExceptionMatches,
    PyErr_NewException, PyErr_NewExceptionWithDoc, PyErr_NoMemory, PyErr_NormalizeException,
    PyErr_Occurred, PyErr_Print, PyErr_PrintEx, PyErr_ProgramText, PyErr_ResourceWarning,
    PyErr_Restore, PyErr_SetExcFromWindowsErr, PyErr_SetExcFromWindowsErrWithFilename,
    PyErr_SetExcFromWindowsErrWithFilenameObject, PyErr_SetExcFromWindowsErrWithFilenameObjects,
    PyErr_SetExcInfo, PyErr_SetFromErrno, PyErr_SetFromErrnoWithFilename,
    PyErr_SetFromErrnoWithFilenameObject, PyErr_SetFromErrnoWithFilenameObjects,
    PyErr_SetFromWindowsErr, PyErr_SetFromWindowsErrWithFilename, PyErr_SetHandledException,
    PyErr_SetImportError, PyErr_SetImportErrorSubclass, PyErr_SetInterrupt, PyErr_SetInterruptEx,
    PyErr_SetNone, PyErr_SetObject, PyErr_SetRaisedException, PyErr_SetString,
    PyErr_SyntaxLocation, PyErr_SyntaxLocationEx, PyErr_WarnEx, PyErr_WarnExplicit,
    PyErr_WarnFormat, PyErr_WriteUnraisable, PyExceptionClass_Name, PyFloat_AsDouble,
    PyFloat_GetInfo, PyFloat_GetMax, PyFloat_GetMin, PyLong_AsDouble, PyLong_AsInt, PyLong_AsInt32,
    PyLong_AsInt64, PyLong_AsLong, PyLong_AsLongAndOverflow, PyLong_AsLongLong,
    PyLong_AsLongLongAndOverflow, PyLong_AsSize_t, PyLong_AsSsize_t, PyLong_AsUInt32,
    PyLong_AsUInt64, PyLong_AsUnsignedLong, PyLong_AsUnsignedLongLong,
    PyLong_AsUnsignedLongLongMask, PyLong_AsUnsignedLongMask, PyLong_AsVoidPtr, PyLong_FromDouble,
    PyStructSequence_GetItem, PyStructSequence_New, PyStructSequence_NewType,
    PyStructSequence_SetItem, PyUnstable_Object_IsUniqueReferencedTemporary,
    PyUnstable_Object_IsUniquelyReferenced, cpython_exception_class_name_from_ptr,
    cpython_exception_traceback_ptr_for_value, cpython_exception_type_ptr,
    cpython_exception_type_ptr_for_value, cpython_ptr_is_type_object,
    cpython_safe_object_type_name,
};
use self::cpython_eval_api::{
    PyEval_GetBuiltins, PyEval_GetFrame, PyEval_GetFrameBuiltins, PyEval_GetFrameGlobals,
    PyEval_GetFrameLocals, PyEval_GetFuncDesc, PyEval_GetFuncName, PyEval_GetGlobals,
    PyEval_GetLocals,
};
use self::cpython_eval_os_marshal_api::{
    PyErr_CheckSignals, PyEval_AcquireLock, PyEval_AcquireThread, PyEval_CallObjectWithKeywords,
    PyEval_EvalCode, PyEval_EvalCodeEx, PyEval_EvalFrame, PyEval_EvalFrameEx, PyEval_InitThreads,
    PyEval_ReleaseLock, PyEval_ReleaseThread, PyEval_RestoreThread, PyEval_SaveThread,
    PyEval_ThreadsInitialized, PyGILState_Ensure, PyGILState_GetThisThreadState,
    PyGILState_Release, PyInterpreterState_Main, PyMarshal_ReadObjectFromString,
    PyMarshal_WriteObjectToString, PyMutex_Lock, PyMutex_Unlock, PyOS_snprintf,
    PyOS_string_to_double, PyOS_strtol, PyOS_strtoul,
};
use self::cpython_exception_file_api::{
    PyException_GetArgs, PyException_GetCause, PyException_GetContext, PyException_GetTraceback,
    PyException_SetArgs, PyException_SetCause, PyException_SetContext, PyException_SetTraceback,
    PyFile_FromFd, PyFile_GetLine, PyFile_WriteObject, PyFile_WriteString, PyTraceBack_Here,
    PyTraceBack_Print, cpython_is_exception_instance,
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
    PY_LIST_MAPPING_METHODS, PY_LIST_SEQUENCE_METHODS, PyList_Append, PyList_AsTuple,
    PyList_GetItem, PyList_GetItemRef, PyList_GetSlice, PyList_Insert, PyList_New, PyList_Reverse,
    PyList_SetItem, PyList_SetSlice, PyList_Size, PyList_Sort,
};
use self::cpython_long_float_api::{
    _PyLong_Copy, PyBool_FromLong, PyFloat_FromDouble, PyFloat_FromString, PyLong_AsNativeBytes,
    PyLong_FromInt32, PyLong_FromInt64, PyLong_FromLong, PyLong_FromLongLong,
    PyLong_FromNativeBytes, PyLong_FromSize_t, PyLong_FromSsize_t, PyLong_FromString,
    PyLong_FromUInt32, PyLong_FromUInt64, PyLong_FromUnicodeObject, PyLong_FromUnsignedLong,
    PyLong_FromUnsignedLongLong, PyLong_FromUnsignedNativeBytes, PyLong_FromVoidPtr,
    PyLong_GetInfo,
};
use self::cpython_marshal_runtime::{
    cpython_marshal_object_to_value, value_to_cpython_marshal_object,
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
    PyObject_DelItem, PyObject_GenericHash, PyObject_GetItem, PyObject_GetOptionalAttr,
    PyObject_Hash, PyObject_HashNotImplemented, PyObject_IsInstance, PyObject_IsSubclass,
    PyObject_Length, PyObject_LengthHint, PyObject_RichCompare, PyObject_RichCompareBool,
    PyObject_SetItem, PyObject_Size, cpython_debug_compare_value, cpython_tuple_richcompare_slot,
    cpython_type_name_for_object_ptr, cpython_value_type_name_from_ptr,
};
use self::cpython_object_lifecycle_api::{
    _Py_Dealloc, _PyObject_GC_New, _PyObject_New, _PyObject_NewVar, PyObject_Init, PyObject_InitVar,
};
pub use self::cpython_refcount_api::Py_DecRef;
use self::cpython_refcount_api::{
    _Py_CheckRecursiveCall, _Py_DecRef, _Py_IncRef, _Py_NegativeRefcount, _Py_SetRefcnt,
    _PyObject_GC_NewVar, _PyObject_GC_Resize, Py_IncRef, Py_XDecRef, Py_XIncRef,
};
use self::cpython_runtime_misc_api::{
    _Py_FatalErrorFunc, _Py_HashDouble, _PyErr_BadInternalCall, _PyUnicode_IsAlpha,
    _PyUnicode_IsDecimalDigit, _PyUnicode_IsDigit, _PyUnicode_IsLowercase, _PyUnicode_IsNumeric,
    _PyUnicode_IsTitlecase, _PyUnicode_IsUppercase, _PyUnicode_IsWhitespace,
    _PyUnicode_ToDecimalDigit, Py_AddPendingCall, Py_AtExit, Py_BytesMain, Py_CompileString,
    Py_DecodeLocale, Py_EncodeLocale, Py_EndInterpreter, Py_Exit, Py_FatalError, Py_Finalize,
    Py_FinalizeEx, Py_GetArgcArgv, Py_GetBuildInfo, Py_GetCompiler, Py_GetCopyright,
    Py_GetExecPrefix, Py_GetPath, Py_GetPlatform, Py_GetPrefix, Py_GetProgramFullPath,
    Py_GetProgramName, Py_GetPythonHome, Py_GetRecursionLimit, Py_GetVersion, Py_Initialize,
    Py_InitializeEx, Py_IsFinalizing, Py_Main, Py_MakePendingCalls, Py_NewInterpreter,
    Py_PACK_FULL_VERSION, Py_PACK_VERSION, Py_ReprEnter, Py_ReprLeave, Py_SetPath,
    Py_SetProgramName, Py_SetPythonHome, Py_SetRecursionLimit, PyErr_BadArgument,
    PyErr_BadInternalCall,
};
use self::cpython_sequence_mapping_api::{
    PyCallIter_New, PyMapping_Check, PyMapping_GetItemString, PyMapping_GetOptionalItem,
    PyMapping_GetOptionalItemString, PyMapping_HasKey, PyMapping_HasKeyString,
    PyMapping_HasKeyStringWithError, PyMapping_HasKeyWithError, PyMapping_Items, PyMapping_Keys,
    PyMapping_Length, PyMapping_SetItemString, PyMapping_Size, PyMapping_Values, PySeqIter_New,
    PySequence_Check, PySequence_Concat, PySequence_Contains, PySequence_Count, PySequence_DelItem,
    PySequence_DelSlice, PySequence_Fast, PySequence_GetItem, PySequence_GetSlice, PySequence_In,
    PySequence_InPlaceConcat, PySequence_InPlaceRepeat, PySequence_Index, PySequence_Length,
    PySequence_List, PySequence_Repeat, PySequence_SetItem, PySequence_SetSlice, PySequence_Size,
    PySequence_Tuple, cpython_slice_bounds_step_one, cpython_slice_indices_for_len,
};
use self::cpython_set_api::{
    PyFrozenSet_New, PySet_Add, PySet_Clear, PySet_Contains, PySet_Discard, PySet_New, PySet_Pop,
    PySet_Size,
};
use self::cpython_slot_runtime::{
    PY_RUNTIME_MAPPING_METHODS, cpython_call_method_for_capi, cpython_call_object,
    cpython_codec_error_name_optional, cpython_codec_name_or_default,
    cpython_mapping_ass_subscript_slot, cpython_mapping_subscript_slot, cpython_sequence_item_slot,
    cpython_structseq_count_fields, cpython_try_binary_number_slot, cpython_try_richcompare_slot,
    cpython_unicode_decode_with_codec_in_context, cpython_unicode_encode_with_codec_in_context,
    cpython_unicode_text_from_value, cpython_valid_type_ptr,
};
use self::cpython_string_runtime::{
    c_name_to_bytes, c_name_to_string, c_wide_name_to_string, cpython_string_to_wide_units,
    cpython_wide_ptr_to_string,
};
use self::cpython_sys_thread_api::{
    PySys_AddWarnOption, PySys_AddWarnOptionUnicode, PySys_AddXOption, PySys_AuditTuple,
    PySys_GetObject, PySys_GetXOptions, PySys_HasWarnOptions, PySys_ResetWarnOptions,
    PySys_SetArgv, PySys_SetArgvEx, PySys_SetObject, PySys_SetPath, PyThread_GetInfo,
    PyThread_ReInitTLS, PyThread_acquire_lock, PyThread_acquire_lock_timed, PyThread_allocate_lock,
    PyThread_create_key, PyThread_delete_key, PyThread_delete_key_value, PyThread_exit_thread,
    PyThread_free_lock, PyThread_get_key_value, PyThread_get_stacksize, PyThread_get_thread_ident,
    PyThread_get_thread_native_id, PyThread_init_thread, PyThread_release_lock,
    PyThread_set_key_value, PyThread_set_stacksize, PyThread_start_new_thread, PyThread_tss_alloc,
    PyThread_tss_create, PyThread_tss_delete, PyThread_tss_free, PyThread_tss_get,
    PyThread_tss_is_created, PyThread_tss_set, cpython_sys_module_obj,
};
use self::cpython_thread_interp_api::{
    _PyState_AddModule, _PyThreadState_Init, _PyThreadState_Prealloc, Py_EnterRecursiveCall,
    Py_GetConstant, Py_GetConstantBorrowed, Py_Is, Py_IsFalse, Py_IsInitialized, Py_IsNone,
    Py_IsTrue, Py_LeaveRecursiveCall, Py_NewRef, Py_REFCNT, Py_TYPE, Py_XNewRef, PyFrame_GetCode,
    PyFrame_GetLineNumber, PyFrame_New, PyInterpreterState_Clear, PyInterpreterState_Delete,
    PyInterpreterState_Get, PyInterpreterState_GetDict, PyInterpreterState_GetID,
    PyInterpreterState_New, PyState_AddModule, PyState_FindModule, PyState_RemoveModule,
    PyThreadState_Clear, PyThreadState_Delete, PyThreadState_DeleteCurrent, PyThreadState_Get,
    PyThreadState_GetDict, PyThreadState_GetFrame, PyThreadState_GetID,
    PyThreadState_GetInterpreter, PyThreadState_GetUnchecked, PyThreadState_New,
    PyThreadState_SetAsyncExc, PyThreadState_Swap, PyTraceMalloc_Track, PyTraceMalloc_Untrack,
    PyVectorcall_NARGS,
};
use self::cpython_thread_runtime::{
    cpython_atexit_callbacks, cpython_collect_sys_argv, cpython_current_thread_ident_u64,
    cpython_current_thread_state_ptr, cpython_current_thread_state_ptr_unchecked,
    cpython_get_or_init_constant_ptr, cpython_get_or_init_wide_storage,
    cpython_gil_acquire_for_current_thread, cpython_gil_current_thread_holds,
    cpython_gil_release_for_current_thread, cpython_gilstate_visible_thread_state_ptr,
    cpython_heap_type_registry, cpython_init_thread_state_compat,
    cpython_interpreter_state_allocations, cpython_is_interned_unicode_ptr,
    cpython_is_known_interpreter_state_ptr, cpython_is_known_thread_state_ptr,
    cpython_lookup_interned_unicode_ptr, cpython_lookup_interned_unicode_text,
    cpython_main_interpreter_state_ptr, cpython_main_thread_state_ptr,
    cpython_mark_pending_interrupt, cpython_mark_thread_runtime_initialized, cpython_pending_calls,
    cpython_read_sys_path_string, cpython_read_sys_string, cpython_register_interned_unicode,
    cpython_set_current_thread_state_ptr, cpython_set_wide_storage, cpython_store_argv_wide,
    cpython_structseq_registry, cpython_take_pending_interrupt_signum,
    cpython_thread_lock_registry, cpython_thread_runtime_initialized,
    cpython_thread_state_allocations, cpython_thread_tls_key_registry, cpython_thread_tls_values,
    cpython_thread_tss_registry, cpython_thread_tss_values, cpython_tracemalloc_traces,
};
use self::cpython_tuple_api::{
    PyTuple_GetItem, PyTuple_GetSlice, PyTuple_New, PyTuple_SetItem, PyTuple_Size,
};
use self::cpython_type_api::{
    _PyType_Lookup, PY_TYPE_MAPPING_METHODS, PyType_ClearCache, PyType_Freeze,
    PyType_FromMetaclass, PyType_FromModuleAndSpec, PyType_FromSpec, PyType_FromSpecWithBases,
    PyType_GenericAlloc, PyType_GenericNew, PyType_GetBaseByToken, PyType_GetFlags,
    PyType_GetFullyQualifiedName, PyType_GetModule, PyType_GetModuleByDef, PyType_GetModuleName,
    PyType_GetModuleState, PyType_GetName, PyType_GetQualName, PyType_GetSlot,
    PyType_GetTypeDataSize, PyType_IsSubtype, PyType_Modified, PyType_Ready,
    cpython_is_type_object_ptr, cpython_type_tp_call, cpython_type_tp_getattro,
    cpython_type_tp_setattro,
};
use self::cpython_type_exports::*;
use self::cpython_type_layout::*;
use self::cpython_unicode_api::{
    PyBuffer_Release, PyCallable_Check, PyIndex_Check, PyUnicode_Append, PyUnicode_AppendAndDel,
    PyUnicode_AsASCIIString, PyUnicode_AsCharmapString, PyUnicode_AsDecodedObject,
    PyUnicode_AsDecodedUnicode, PyUnicode_AsEncodedObject, PyUnicode_AsEncodedString,
    PyUnicode_AsEncodedUnicode, PyUnicode_AsLatin1String, PyUnicode_AsMBCSString,
    PyUnicode_AsRawUnicodeEscapeString, PyUnicode_AsUCS4, PyUnicode_AsUCS4Copy, PyUnicode_AsUTF8,
    PyUnicode_AsUTF8AndSize, PyUnicode_AsUTF8String, PyUnicode_AsUTF16String,
    PyUnicode_AsUTF32String, PyUnicode_AsUnicodeEscapeString, PyUnicode_AsWideChar,
    PyUnicode_AsWideCharString, PyUnicode_BuildEncodingMap, PyUnicode_Compare,
    PyUnicode_CompareWithASCIIString, PyUnicode_Concat, PyUnicode_Contains,
    PyUnicode_CopyCharacters, PyUnicode_Count, PyUnicode_Decode, PyUnicode_DecodeASCII,
    PyUnicode_DecodeCharmap, PyUnicode_DecodeCodePageStateful, PyUnicode_DecodeFSDefault,
    PyUnicode_DecodeFSDefaultAndSize, PyUnicode_DecodeLatin1, PyUnicode_DecodeLocale,
    PyUnicode_DecodeLocaleAndSize, PyUnicode_DecodeMBCS, PyUnicode_DecodeMBCSStateful,
    PyUnicode_DecodeRawUnicodeEscape, PyUnicode_DecodeUTF7, PyUnicode_DecodeUTF7Stateful,
    PyUnicode_DecodeUTF8, PyUnicode_DecodeUTF8Stateful, PyUnicode_DecodeUTF16,
    PyUnicode_DecodeUTF16Stateful, PyUnicode_DecodeUTF32, PyUnicode_DecodeUTF32Stateful,
    PyUnicode_DecodeUnicodeEscape, PyUnicode_EncodeCodePage, PyUnicode_EncodeFSDefault,
    PyUnicode_EncodeLocale, PyUnicode_Equal, PyUnicode_EqualToUTF8, PyUnicode_EqualToUTF8AndSize,
    PyUnicode_FSConverter, PyUnicode_FSDecoder, PyUnicode_Find, PyUnicode_FindChar,
    PyUnicode_Format, PyUnicode_FromEncodedObject, PyUnicode_FromKindAndData, PyUnicode_FromObject,
    PyUnicode_FromOrdinal, PyUnicode_FromString, PyUnicode_FromStringAndSize,
    PyUnicode_FromWideChar, PyUnicode_GetDefaultEncoding, PyUnicode_GetLength, PyUnicode_GetSize,
    PyUnicode_InternFromString, PyUnicode_InternImmortal, PyUnicode_InternInPlace,
    PyUnicode_IsIdentifier, PyUnicode_Join, PyUnicode_New, PyUnicode_Partition,
    PyUnicode_RPartition, PyUnicode_RSplit, PyUnicode_ReadChar, PyUnicode_Replace,
    PyUnicode_Resize, PyUnicode_RichCompare, PyUnicode_Split, PyUnicode_Splitlines,
    PyUnicode_Substring, PyUnicode_Tailmatch, PyUnicode_Translate, PyUnicode_WriteChar,
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
use self::cpython_value_runtime::{
    cpython_builtin_type_name_for_ptr, cpython_builtin_type_ptr_for_builtin,
    cpython_builtin_type_ptr_for_class_name, cpython_debug_ufunc_attr_summary,
    cpython_debug_ufunc_exception_summary, cpython_objref_from_value, cpython_type_for_value,
    cpython_value_debug_tag,
};
use self::cpython_weakref_api::{
    PyObject_ClearWeakRefs, PyWeakref_GetObject, PyWeakref_GetRef, PyWeakref_NewProxy,
    PyWeakref_NewRef,
};

thread_local! {
    static PROXY_REFRESH_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static CPY_PROXY_GET_ITER_ACTIVE: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static CPY_PROXY_ATTR_LOOKUP_ACTIVE: RefCell<Vec<(usize, String, bool)>> = const { RefCell::new(Vec::new()) };
    static CPY_PROXY_ATTR_LOOKUP_DEPTH: Cell<usize> = const { Cell::new(0) };
    static CPY_PTR_MAP_DEPTH: Cell<usize> = const { Cell::new(0) };
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
        const MAX_PROXY_ATTR_LOOKUP_DEPTH: usize = 256;
        let depth_allowed = CPY_PROXY_ATTR_LOOKUP_DEPTH.with(|depth| {
            let next = depth.get().saturating_add(1);
            depth.set(next);
            if super::env_var_present_cached("PYRS_TRACE_PROXY_ATTR_DEPTH") && next >= 8 {
                eprintln!(
                    "[proxy-attr-depth] depth={} ptr={:p} attr={} is_type={}",
                    next, raw_ptr as *mut c_void, attr_name, is_type_object
                );
            }
            next <= MAX_PROXY_ATTR_LOOKUP_DEPTH
        });
        if !depth_allowed {
            CPY_PROXY_ATTR_LOOKUP_DEPTH.with(|depth| {
                let current = depth.get();
                if current > 0 {
                    depth.set(current - 1);
                }
            });
            return None;
        }
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
            CPY_PROXY_ATTR_LOOKUP_DEPTH.with(|depth| {
                let current = depth.get();
                if current > 0 {
                    depth.set(current - 1);
                }
            });
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
        CPY_PROXY_ATTR_LOOKUP_DEPTH.with(|depth| {
            let current = depth.get();
            if current > 0 {
                depth.set(current - 1);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CpythonProxyPtrOwnership {
    ExternalBorrowed,
    ExternalOwnedRef,
    OwnedCompat,
}

impl CpythonProxyPtrOwnership {
    fn drop_provenance(self) -> usize {
        match self {
            Self::ExternalBorrowed => 0,
            Self::ExternalOwnedRef => 1,
            Self::OwnedCompat => 2,
        }
    }

    fn from_drop_provenance(provenance: usize) -> Option<Self> {
        match provenance {
            0 => Some(Self::ExternalBorrowed),
            1 => Some(Self::ExternalOwnedRef),
            2 => Some(Self::OwnedCompat),
            _ => None,
        }
    }

    fn is_owned_compat(self) -> bool {
        matches!(self, Self::OwnedCompat)
    }
}

unsafe fn cpython_proxy_instance_drop_hook(
    _heap_instance_id: u64,
    object_id: u64,
    vm_ptr: usize,
    raw_ptr: usize,
    provenance: usize,
) {
    if super::env_var_present_cached("PYRS_TRACE_CPY_PROXY_DROP") {
        let raw_type_name = unsafe {
            (raw_ptr as *mut c_void)
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .filter(|type_ptr| !type_ptr.is_null())
                .and_then(|type_ptr| c_name_to_string((*type_ptr).tp_name).ok())
                .unwrap_or_else(|| "<unknown>".to_string())
        };
        eprintln!(
            "[cpy-proxy-drop] hook-fire object_id={} raw_ptr={:p} provenance={} raw_type={}",
            object_id, raw_ptr as *mut c_void, provenance, raw_type_name
        );
    }
    let vm_ptr = vm_ptr as *mut Vm;
    if vm_ptr.is_null() {
        return;
    }
    let Some(ownership) = CpythonProxyPtrOwnership::from_drop_provenance(provenance) else {
        return;
    };
    unsafe {
        (&mut *vm_ptr).release_cpython_proxy_instance_from_drop(
            raw_ptr as *mut c_void,
            object_id,
            ownership,
        );
    }
}

impl Vm {
    fn register_cpython_proxy_instance_drop_hook(
        &mut self,
        proxy: &Value,
        raw_ptr: *mut c_void,
        ownership: CpythonProxyPtrOwnership,
    ) {
        let Value::Instance(instance_obj) = proxy else {
            return;
        };
        if super::env_var_present_cached("PYRS_TRACE_CPY_PROXY_DROP") {
            let class_name = match &*instance_obj.kind() {
                Object::Instance(instance_data) => match &*instance_data.class.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "<non-class>".to_string(),
                },
                _ => "<non-instance>".to_string(),
            };
            let raw_type_name = unsafe {
                raw_ptr
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .filter(|type_ptr| !type_ptr.is_null())
                    .and_then(|type_ptr| c_name_to_string((*type_ptr).tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string())
            };
            eprintln!(
                "[cpy-proxy-drop] register object_id={} class={} raw_ptr={:p} ownership={:?} raw_type={}",
                instance_obj.id(),
                class_name,
                raw_ptr,
                ownership,
                raw_type_name
            );
        }
        register_obj_drop_hook(
            instance_obj.heap_instance_id(),
            instance_obj.id(),
            ObjDropHook {
                callback: cpython_proxy_instance_drop_hook,
                arg0: self as *mut Vm as usize,
                arg1: raw_ptr as usize,
                arg2: ownership.drop_provenance(),
            },
        );
    }

    fn release_cpython_proxy_instance_from_drop(
        &mut self,
        raw_ptr: *mut c_void,
        object_id: u64,
        ownership: CpythonProxyPtrOwnership,
    ) {
        if raw_ptr.is_null() || self.is_finalizing {
            return;
        }
        self.extension_cpython_ptr_value_remove(raw_ptr as usize);
        if self
            .extension_cpython_ptr_by_object_id
            .get(&object_id)
            .copied()
            == Some(raw_ptr as usize)
        {
            self.extension_cpython_ptr_by_object_id.remove(&object_id);
        }
        if ownership.is_owned_compat() {
            self.capi_unpin_owned_ptr(raw_ptr as usize);
        } else {
            self.capi_registry_unpin_external(raw_ptr as usize);
        }
        // SAFETY: `raw_ptr` remains live for the duration of the refcount release path.
        let current_refcount =
            unsafe { (*raw_ptr.cast::<CpythonObjectHead>()).ob_refcnt.max(0) as usize };
        if super::env_var_present_cached("PYRS_TRACE_CPY_PROXY_DROP") {
            let raw_type_name = unsafe {
                raw_ptr
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .filter(|type_ptr| !type_ptr.is_null())
                    .and_then(|type_ptr| c_name_to_string((*type_ptr).tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string())
            };
            eprintln!(
                "[cpy-proxy-drop] release object_id={} raw_ptr={:p} ownership={:?} refcnt={} raw_type={}",
                object_id, raw_ptr, ownership, current_refcount, raw_type_name
            );
        }
        if current_refcount == 0 {
            return;
        }
        let mut release_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        release_ctx.suppress_vm_proxy_persistence = true;
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(release_ctx));
        let _ = release_ctx.cpython_value_from_ptr_or_proxy(raw_ptr);
        if let Some(handle) = release_ctx.cpython_handle_from_ptr(raw_ptr) {
            release_ctx.set_object_refcount(handle, current_refcount);
            release_ctx.sync_cpython_header_refcount(handle);
        }
        // SAFETY: the temporary context above is active for the duration of the decref.
        unsafe {
            Py_DecRef(raw_ptr);
        }
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
/// Mirror of CPython's thread-local `(type, value, traceback)` error tuple.
///
/// Pointers are context-relative; they must be interpreted only while the
/// owning `ModuleCapiContext` is active.
struct CpythonErrorState {
    ptype: *mut c_void,
    pvalue: *mut c_void,
    ptraceback: *mut c_void,
}

#[derive(Clone)]
struct CpythonExceptionCompatState {
    args: Option<Value>,
    notes: Option<Value>,
    traceback: Option<Value>,
    suppress_context: bool,
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
struct CpythonByteArrayCompatObject {
    ob_base: CpythonVarObjectHead,
    ob_alloc: isize,
    ob_bytes: *mut c_char,
    ob_start: *mut c_char,
    ob_exports: isize,
}

#[repr(C)]
struct CpythonAsciiUnicodeCompatObject {
    ob_base: CpythonObjectHead,
    length: isize,
    hash: isize,
    state: u32,
}

#[repr(C)]
struct CpythonCompactUnicodeCompatObject {
    ob_base: CpythonAsciiUnicodeCompatObject,
    utf8_length: isize,
    utf8: *mut c_char,
}

#[repr(C)]
struct CpythonFloatCompatObject {
    ob_base: CpythonObjectHead,
    ob_fval: f64,
}

#[repr(C)]
struct CpythonComplexCompatObject {
    ob_base: CpythonObjectHead,
    cval: CpythonComplexValue,
}

#[repr(C)]
struct CpythonBaseExceptionCompatObject {
    ob_base: CpythonObjectHead,
    dict: *mut c_void,
    args: *mut c_void,
    notes: *mut c_void,
    traceback: *mut c_void,
    context: *mut c_void,
    cause: *mut c_void,
    suppress_context: c_char,
    _padding: [u8; 7],
}

#[repr(C)]
struct CpythonFrameCompatObject {
    ob_base: CpythonVarObjectHead,
    f_back: *mut c_void,
    f_trace: *mut c_void,
    f_lineno: c_int,
    _padding: c_int,
    f_code: *mut c_void,
    f_globals: *mut c_void,
    f_locals: *mut c_void,
}

#[repr(C)]
struct CpythonModuleCompatObject {
    ob_base: CpythonObjectHead,
    md_dict: *mut c_void,
    md_def: *mut CpythonModuleDef,
    md_state: *mut c_void,
    md_weaklist: *mut c_void,
    md_name: *mut c_void,
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
    m_weakreflist: *mut c_void,
    vectorcall: *mut c_void,
}

#[repr(C)]
struct CpythonCMethodCompatObject {
    function: CpythonCFunctionCompatObject,
    mm_class: *mut c_void,
}

#[repr(C)]
struct CpythonMethodCompatObject {
    ob_base: CpythonObjectHead,
    im_func: *mut c_void,
    im_self: *mut c_void,
    im_weakreflist: *mut c_void,
    vectorcall: *mut c_void,
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

const CPY_SLOT_LT_NAME: &[u8] = b"__lt__\0";
const CPY_SLOT_LE_NAME: &[u8] = b"__le__\0";
const CPY_SLOT_EQ_NAME: &[u8] = b"__eq__\0";
const CPY_SLOT_NE_NAME: &[u8] = b"__ne__\0";
const CPY_SLOT_GT_NAME: &[u8] = b"__gt__\0";
const CPY_SLOT_GE_NAME: &[u8] = b"__ge__\0";
const CPY_SLOT_REPR_NAME: &[u8] = b"__repr__\0";
const CPY_SLOT_STR_NAME: &[u8] = b"__str__\0";
const CPY_SLOT_BOOL_NAME: &[u8] = b"__bool__\0";
const CPY_SLOT_INT_NAME: &[u8] = b"__int__\0";
const CPY_SLOT_FLOAT_NAME: &[u8] = b"__float__\0";
const CPY_SLOT_INDEX_NAME: &[u8] = b"__index__\0";
const CPY_SLOT_GETITEM_NAME: &[u8] = b"__getitem__\0";
const CPY_SLOT_LEN_NAME: &[u8] = b"__len__\0";
const CPY_SLOT_ITER_NAME: &[u8] = b"__iter__\0";
const CPY_SLOT_SETITEM_NAME: &[u8] = b"__setitem__\0";
const CPY_SLOT_GET_NAME: &[u8] = b"__get__\0";
const CPY_SLOT_SET_NAME: &[u8] = b"__set__\0";
const CPY_SLOT_DELETE_NAME: &[u8] = b"__delete__\0";
const CPY_SLOT_INIT_NAME: &[u8] = b"__init__\0";
const CPY_LONG_BIT_LENGTH_NAME: &[u8] = b"bit_length\0";
const CPY_FLOAT_AS_INTEGER_RATIO_NAME: &[u8] = b"as_integer_ratio\0";

static mut CPY_SLOT_LT_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_LT_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_lt),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_LE_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_LE_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_le),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_EQ_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_EQ_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_eq),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_NE_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_NE_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_ne),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_GT_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_GT_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_gt),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_GE_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_GE_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_ge),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_REPR_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_REPR_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_repr),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_STR_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_STR_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_str),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_BOOL_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_BOOL_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_bool),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_INT_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_INT_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_int),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_FLOAT_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_FLOAT_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_float),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_INDEX_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_INDEX_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_index),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_GETITEM_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_GETITEM_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_getitem),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_LEN_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_LEN_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_len),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_ITER_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_ITER_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_iter),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_SETITEM_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_SETITEM_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_setitem),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_GET_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_GET_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_get),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_SET_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_SET_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_set),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_DELETE_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_DELETE_NAME.as_ptr().cast::<c_char>(),
    ml_meth: Some(cpython_slot_dunder_delete),
    ml_flags: METH_VARARGS,
    ml_doc: std::ptr::null(),
};
static mut CPY_SLOT_INIT_METHOD_DEF: CpythonMethodDef = CpythonMethodDef {
    ml_name: CPY_SLOT_INIT_NAME.as_ptr().cast::<c_char>(),
    ml_meth: None,
    ml_flags: METH_VARARGS | METH_KEYWORDS,
    ml_doc: std::ptr::null(),
};

static mut PY_LONG_TYPE_METHOD_DEFS: [CpythonMethodDef; 2] = [
    CpythonMethodDef {
        ml_name: CPY_LONG_BIT_LENGTH_NAME.as_ptr().cast::<c_char>(),
        ml_meth: Some(cpython_long_bit_length_noargs_method),
        ml_flags: METH_NOARGS,
        ml_doc: std::ptr::null(),
    },
    CpythonMethodDef {
        ml_name: std::ptr::null(),
        ml_meth: None,
        ml_flags: 0,
        ml_doc: std::ptr::null(),
    },
];

static mut PY_FLOAT_TYPE_METHOD_DEFS: [CpythonMethodDef; 2] = [
    CpythonMethodDef {
        ml_name: CPY_FLOAT_AS_INTEGER_RATIO_NAME.as_ptr().cast::<c_char>(),
        ml_meth: Some(cpython_float_as_integer_ratio_noargs_method),
        ml_flags: METH_NOARGS,
        ml_doc: std::ptr::null(),
    },
    CpythonMethodDef {
        ml_name: std::ptr::null(),
        ml_meth: None,
        ml_flags: 0,
        ml_doc: std::ptr::null(),
    },
];

#[repr(C)]
struct CpythonDescrCompatObject {
    ob_base: CpythonObjectHead,
    d_type: *mut CpythonTypeObject,
    d_name: *mut c_void,
    d_qualname: *mut c_void,
}

#[repr(C)]
struct CpythonMethodDescrCompatObject {
    d_common: CpythonDescrCompatObject,
    d_method: *mut CpythonMethodDef,
    vectorcall: *mut c_void,
}

#[repr(C)]
struct CpythonMemberDescrCompatObject {
    d_common: CpythonDescrCompatObject,
    d_member: *mut CpythonMemberDef,
}

#[repr(C)]
struct CpythonGetSetDescrCompatObject {
    d_common: CpythonDescrCompatObject,
    d_getset: *mut CpythonGetSetDef,
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
fn cpython_unicode_state(kind: u32, compact: bool, ascii: bool) -> u32 {
    const INTERNED_NOT_INTERNED: u32 = 0;
    const INTERNED_SHIFT: u32 = 0;
    const KIND_SHIFT: u32 = 2;
    const COMPACT_SHIFT: u32 = 5;
    const ASCII_SHIFT: u32 = 6;
    (INTERNED_NOT_INTERNED << INTERNED_SHIFT)
        | ((kind & 0b111) << KIND_SHIFT)
        | ((compact as u32) << COMPACT_SHIFT)
        | ((ascii as u32) << ASCII_SHIFT)
}

#[inline]
fn cpython_unicode_precomputed_hash(text: &str) -> isize {
    let hash_bits = value_lookup_hash(&Value::Str(text.to_string())).unwrap_or(0);
    let hash = hash_bits as i64;
    if hash == -1 { -2isize } else { hash as isize }
}

#[inline]
fn cpython_long_storage_bytes(ndigits: usize) -> usize {
    std::mem::size_of::<CpythonObjectHead>()
        .saturating_add(std::mem::size_of::<usize>())
        .saturating_add(ndigits.max(1).saturating_mul(std::mem::size_of::<u32>()))
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

#[inline]
unsafe fn cpython_long_digits_ptr(object: *mut c_void) -> *mut u32 {
    // SAFETY: caller guarantees `object` points to long-compatible storage.
    unsafe {
        object
            .cast::<u8>()
            .add(std::mem::size_of::<CpythonObjectHead>() + std::mem::size_of::<usize>())
            .cast::<u32>()
    }
}

#[inline]
unsafe fn cpython_long_lv_tag_ptr(object: *mut c_void) -> *mut usize {
    // SAFETY: caller guarantees `object` points to long-compatible storage.
    unsafe {
        object
            .cast::<u8>()
            .add(std::mem::size_of::<CpythonObjectHead>())
            .cast::<usize>()
    }
}

type CpythonVectorcallFn =
    unsafe extern "C" fn(*mut c_void, *const *mut c_void, usize, *mut c_void) -> *mut c_void;

unsafe fn cpython_resolve_vectorcall(callable: *mut c_void) -> Option<CpythonVectorcallFn> {
    const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
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
    let trace_vectorcall_resolve =
        super::env_var_present_cached("PYRS_TRACE_CPY_VECTORCALL_RESOLVE");
    // SAFETY: `type_ptr` is non-null and points to a type object header.
    let mut raw = unsafe { (*type_ptr).tp_vectorcall };
    // SAFETY: `type_ptr` is valid for metadata reads.
    let offset = unsafe { (*type_ptr).tp_vectorcall_offset };
    let mut slot_raw = std::ptr::null_mut();
    if raw.is_null() && offset > 0 {
        // SAFETY: CPython stores vectorcall function pointer at object+offset.
        let slot_ptr = unsafe { callable.cast::<u8>().add(offset as usize) }.cast::<*mut c_void>();
        // SAFETY: slot address computed from object + valid offset.
        slot_raw = unsafe { *slot_ptr };
        raw = slot_raw;
    }
    if trace_vectorcall_resolve {
        // SAFETY: `type_ptr` is valid for name access.
        let type_name = unsafe {
            c_name_to_string((*type_ptr).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
        };
        if type_name.contains("cython_function_or_method") || type_name == "method" {
            eprintln!(
                "[cpy-vectorcall-resolve] callable={:p} type={:p}({}) tp_vectorcall={:p} tp_vectorcall_offset={} slot_raw={:p} resolved={:p}",
                callable,
                type_ptr,
                type_name,
                unsafe { (*type_ptr).tp_vectorcall },
                offset,
                slot_raw,
                raw
            );
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
    const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
    if object.is_null() {
        return None;
    }
    let object_addr = object as usize;
    if object_addr == usize::MAX
        || object_addr < MIN_VALID_PTR
        || object_addr % std::mem::align_of::<CpythonObjectHead>() != 0
    {
        return None;
    }
    // SAFETY: caller provides a foreign PyObject*.
    let head = unsafe { object.cast::<CpythonObjectHead>().as_ref() }?;
    let type_ptr = head.ob_type.cast::<CpythonTypeObject>();
    if type_ptr.is_null() {
        return None;
    }
    let type_addr = type_ptr as usize;
    if type_addr < MIN_VALID_PTR || type_addr % std::mem::align_of::<CpythonTypeObject>() != 0 {
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
    // CPython 3.14 long layout:
    // - low 2 bits: sign (0 positive, 1 zero, 2 negative)
    // - bit 2: compact flag
    // - upper bits: ndigits for non-compact values
    // See Include/cpython/longintrepr.h.
    let raw = object.cast::<CpythonForeignLongObject>();
    // SAFETY: layout matches CPython long object memory representation.
    let lv_tag = unsafe { (*raw).long_value.lv_tag };
    // SAFETY: layout matches CPython long object memory representation.
    let digits = unsafe { (*raw).long_value.ob_digit.as_ptr() };
    cpython_foreign_long_payload_to_i64(lv_tag, digits)
}

unsafe fn cpython_foreign_long_to_u64(object: *mut c_void) -> Option<u64> {
    const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
    if object.is_null() {
        return None;
    }
    let object_addr = object as usize;
    if object_addr == usize::MAX
        || object_addr < MIN_VALID_PTR
        || object_addr % std::mem::align_of::<CpythonObjectHead>() != 0
    {
        return None;
    }
    // SAFETY: caller provides a foreign PyObject*.
    let head = unsafe { object.cast::<CpythonObjectHead>().as_ref() }?;
    let type_ptr = head.ob_type.cast::<CpythonTypeObject>();
    if type_ptr.is_null() {
        return None;
    }
    let type_addr = type_ptr as usize;
    if type_addr < MIN_VALID_PTR || type_addr % std::mem::align_of::<CpythonTypeObject>() != 0 {
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
    // CPython 3.14 long layout:
    // - low 2 bits: sign (0 positive, 1 zero, 2 negative)
    // - bit 2: compact flag
    // - upper bits: ndigits for non-compact values
    // See Include/cpython/longintrepr.h.
    let raw = object.cast::<CpythonForeignLongObject>();
    // SAFETY: layout matches CPython long object memory representation.
    let lv_tag = unsafe { (*raw).long_value.lv_tag };
    // SAFETY: layout matches CPython long object memory representation.
    let digits = unsafe { (*raw).long_value.ob_digit.as_ptr() };
    cpython_foreign_long_payload_to_u64(lv_tag, digits)
}

fn cpython_foreign_long_payload_to_i64(lv_tag: usize, digits: *const u32) -> Option<i64> {
    const PY_LONG_SIGN_MASK: usize = 0x3;
    const PY_LONG_NON_SIZE_BITS: usize = 3;
    if lv_tag < (2usize << PY_LONG_NON_SIZE_BITS) {
        let sign_bits = lv_tag & PY_LONG_SIGN_MASK;
        let compact_digits = lv_tag >> PY_LONG_NON_SIZE_BITS;
        if compact_digits == 0 {
            return (sign_bits == 1).then_some(0);
        }
        let sign = 1i128 - (sign_bits as i128);
        if sign == 0 || sign < -1 || sign > 1 {
            return None;
        }
        // SAFETY: compact longs expose payload in `ob_digit[0]`.
        let compact = unsafe { *digits } as i128;
        let value = sign.checked_mul(compact)?;
        return i64::try_from(value).ok();
    }
    let sign_bits = lv_tag & PY_LONG_SIGN_MASK;
    if sign_bits == 1 {
        return Some(0);
    }
    if sign_bits > 2 {
        return None;
    }
    let sign = if sign_bits == 2 { -1i128 } else { 1i128 };
    let ndigits = lv_tag >> PY_LONG_NON_SIZE_BITS;
    if ndigits == 0 {
        return Some(0);
    }
    let mut acc: i128 = 0;
    for idx in 0..ndigits {
        // SAFETY: caller passes CPython long digit storage.
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

fn cpython_foreign_long_payload_to_u64(lv_tag: usize, digits: *const u32) -> Option<u64> {
    const PY_LONG_SIGN_MASK: usize = 0x3;
    const PY_LONG_NON_SIZE_BITS: usize = 3;
    if lv_tag < (2usize << PY_LONG_NON_SIZE_BITS) {
        let sign_bits = lv_tag & PY_LONG_SIGN_MASK;
        let compact_digits = lv_tag >> PY_LONG_NON_SIZE_BITS;
        if compact_digits == 0 {
            return (sign_bits == 1).then_some(0);
        }
        let sign = 1isize - (sign_bits as isize);
        if sign <= 0 {
            return (sign == 0).then_some(0);
        }
        // SAFETY: compact longs expose payload in `ob_digit[0]`.
        let compact = unsafe { *digits } as u64;
        return Some(compact);
    }
    let sign_bits = lv_tag & PY_LONG_SIGN_MASK;
    if sign_bits == 1 {
        return Some(0);
    }
    if sign_bits > 2 || sign_bits == 2 {
        return None;
    }
    let ndigits = lv_tag >> PY_LONG_NON_SIZE_BITS;
    if ndigits == 0 {
        return Some(0);
    }
    let mut acc: u128 = 0;
    for idx in 0..ndigits {
        // SAFETY: caller passes CPython long digit storage.
        let digit = unsafe { *digits.add(idx) } as u128;
        let shift = 30usize.saturating_mul(idx);
        if shift >= 128 {
            return None;
        }
        acc = acc.checked_add(digit.checked_shl(shift as u32)?)?;
    }
    u64::try_from(acc).ok()
}

fn cpython_long_digits_from_u64(mut magnitude: u64) -> Vec<u32> {
    const PY_LONG_SHIFT: u32 = 30;
    const PY_LONG_MASK: u64 = (1u64 << PY_LONG_SHIFT) - 1;
    let mut digits = Vec::new();
    while magnitude != 0 {
        digits.push((magnitude & PY_LONG_MASK) as u32);
        magnitude >>= PY_LONG_SHIFT;
    }
    digits
}

fn cpython_long_digits_from_abs_le_bytes(bytes: &[u8]) -> Vec<u32> {
    const PY_LONG_BASE: u64 = 1u64 << 30;
    if bytes.is_empty() || bytes.iter().all(|byte| *byte == 0) {
        return Vec::new();
    }
    let mut limbs32 = Vec::with_capacity(bytes.len().div_ceil(4));
    for chunk in bytes.chunks(4) {
        let mut limb = 0u32;
        for (idx, byte) in chunk.iter().enumerate() {
            limb |= (*byte as u32) << (idx * 8);
        }
        limbs32.push(limb);
    }
    while limbs32.last().is_some_and(|limb| *limb == 0) {
        limbs32.pop();
    }
    let mut digits30 = Vec::new();
    while !limbs32.is_empty() {
        let mut rem = 0u64;
        for idx in (0..limbs32.len()).rev() {
            let wide = (rem << 32) | (limbs32[idx] as u64);
            limbs32[idx] = (wide / PY_LONG_BASE) as u32;
            rem = wide % PY_LONG_BASE;
        }
        digits30.push(rem as u32);
        while limbs32.last().is_some_and(|limb| *limb == 0) {
            limbs32.pop();
        }
    }
    digits30
}

fn cpython_long_payload_from_value(value: &Value) -> Option<(usize, Vec<u32>)> {
    const PY_LONG_SIGN_ZERO: usize = 1;
    const PY_LONG_SIGN_NEGATIVE: usize = 2;
    const PY_LONG_NON_SIZE_BITS: usize = 3;
    let (sign_bits, mut digits) = match value {
        Value::Int(raw) => {
            if *raw == 0 {
                (PY_LONG_SIGN_ZERO, Vec::new())
            } else {
                let sign_bits = if *raw < 0 { PY_LONG_SIGN_NEGATIVE } else { 0 };
                (sign_bits, cpython_long_digits_from_u64(raw.unsigned_abs()))
            }
        }
        Value::BigInt(raw) => {
            if raw.is_zero() {
                (PY_LONG_SIGN_ZERO, Vec::new())
            } else {
                let sign_bits = if raw.is_negative() {
                    PY_LONG_SIGN_NEGATIVE
                } else {
                    0
                };
                (
                    sign_bits,
                    cpython_long_digits_from_abs_le_bytes(&raw.to_abs_le_bytes()),
                )
            }
        }
        _ => return None,
    };
    while digits.last().is_some_and(|digit| *digit == 0) {
        digits.pop();
    }
    if digits.is_empty() {
        return Some((PY_LONG_SIGN_ZERO, Vec::new()));
    }
    let ndigits = digits.len();
    Some(((ndigits << PY_LONG_NON_SIZE_BITS) | sign_bits, digits))
}

#[cfg(test)]
mod long_payload_tests {
    use super::{cpython_foreign_long_payload_to_i64, cpython_foreign_long_payload_to_u64};

    #[test]
    fn compact_long_payload_decodes_zero_positive_and_negative() {
        // CPython 3.14 compact tags:
        // zero: sign=1, ndigits=0 -> 0x5
        // +8:   sign=0, ndigits=1 -> 0xc
        // -7:   sign=2, ndigits=1 -> 0xe
        let zero_digits = [0xFFFF_FFFFu32];
        assert_eq!(
            cpython_foreign_long_payload_to_i64(0x5, zero_digits.as_ptr()),
            Some(0)
        );
        assert_eq!(
            cpython_foreign_long_payload_to_u64(0x5, zero_digits.as_ptr()),
            Some(0)
        );

        let positive_digits = [8u32];
        assert_eq!(
            cpython_foreign_long_payload_to_i64(0xC, positive_digits.as_ptr()),
            Some(8)
        );
        assert_eq!(
            cpython_foreign_long_payload_to_u64(0xC, positive_digits.as_ptr()),
            Some(8)
        );

        let negative_digits = [7u32];
        assert_eq!(
            cpython_foreign_long_payload_to_i64(0xE, negative_digits.as_ptr()),
            Some(-7)
        );
        assert_eq!(
            cpython_foreign_long_payload_to_u64(0xE, negative_digits.as_ptr()),
            None
        );
    }

    #[test]
    fn compact_long_payload_rejects_invalid_zero_ndigits_tags() {
        let digits = [0xFFFF_FFFFu32];
        assert_eq!(
            cpython_foreign_long_payload_to_i64(0x0, digits.as_ptr()),
            None
        );
        assert_eq!(
            cpython_foreign_long_payload_to_u64(0x0, digits.as_ptr()),
            None
        );
    }

    #[test]
    fn non_compact_long_payload_decodes_multi_digit_values() {
        // ndigits=2, sign=0 => 0x10 => (1 << 30)
        let digits = [0u32, 1u32];
        assert_eq!(
            cpython_foreign_long_payload_to_i64(0x10, digits.as_ptr()),
            Some(1_i64 << 30)
        );
        assert_eq!(
            cpython_foreign_long_payload_to_u64(0x10, digits.as_ptr()),
            Some(1_u64 << 30)
        );

        // ndigits=2, sign=2 => 0x12 => -(1 << 30)
        assert_eq!(
            cpython_foreign_long_payload_to_i64(0x12, digits.as_ptr()),
            Some(-(1_i64 << 30))
        );
        assert_eq!(
            cpython_foreign_long_payload_to_u64(0x12, digits.as_ptr()),
            None
        );
    }

    #[test]
    fn non_compact_long_payload_rejects_large_overflowing_shift() {
        // ndigits=5, sign=0 => 0x28; idx=4 would shift by 120 (still valid),
        // set idx=5 by encoding ndigits=6 to cross i64/u64-safe bounds.
        let digits = [1u32, 1u32, 1u32, 1u32, 1u32, 1u32];
        let lv_tag = 6usize << 3;
        assert_eq!(
            cpython_foreign_long_payload_to_i64(lv_tag, digits.as_ptr()),
            None
        );
        assert_eq!(
            cpython_foreign_long_payload_to_u64(lv_tag, digits.as_ptr()),
            None
        );
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
                let symbol_storage = std::ptr::addr_of_mut!($symbol) as usize;
                if raw == symbol_storage || (symbol != 0 && raw == symbol) {
                    return Some(Value::ExceptionType($name.to_string()));
                }
            }
        };
    }
    for_each_cpython_exception_symbol!(match_exception_symbol);
    None
}

pub(super) fn cpython_exception_ptr_for_name(name: &str) -> Option<*mut c_void> {
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

fn cpython_builtin_exception_parent_name(name: &str) -> Option<&'static str> {
    match name {
        "BaseException" => None,
        "Exception" => Some("BaseException"),
        "BaseExceptionGroup" => Some("BaseException"),
        "GeneratorExit" => Some("BaseException"),
        "KeyboardInterrupt" => Some("BaseException"),
        "SystemExit" => Some("BaseException"),
        "StopIteration" => Some("Exception"),
        "StopAsyncIteration" => Some("Exception"),
        "ArithmeticError" => Some("Exception"),
        "OverflowError" => Some("ArithmeticError"),
        "FloatingPointError" => Some("ArithmeticError"),
        "ZeroDivisionError" => Some("ArithmeticError"),
        "AssertionError" => Some("Exception"),
        "AttributeError" => Some("Exception"),
        "BufferError" => Some("Exception"),
        "EOFError" => Some("Exception"),
        "ImportError" => Some("Exception"),
        "ModuleNotFoundError" => Some("ImportError"),
        "LookupError" => Some("Exception"),
        "IndexError" => Some("LookupError"),
        "KeyError" => Some("LookupError"),
        "MemoryError" => Some("Exception"),
        "NameError" => Some("Exception"),
        "UnboundLocalError" => Some("NameError"),
        "OSError" => Some("Exception"),
        "BlockingIOError" => Some("OSError"),
        "BrokenPipeError" => Some("ConnectionError"),
        "ChildProcessError" => Some("OSError"),
        "ConnectionError" => Some("OSError"),
        "ConnectionAbortedError" => Some("ConnectionError"),
        "ConnectionRefusedError" => Some("ConnectionError"),
        "ConnectionResetError" => Some("ConnectionError"),
        "FileExistsError" => Some("OSError"),
        "FileNotFoundError" => Some("OSError"),
        "InterruptedError" => Some("OSError"),
        "IsADirectoryError" => Some("OSError"),
        "NotADirectoryError" => Some("OSError"),
        "PermissionError" => Some("OSError"),
        "ProcessLookupError" => Some("OSError"),
        "TimeoutError" => Some("OSError"),
        "ReferenceError" => Some("Exception"),
        "RuntimeError" => Some("Exception"),
        "NotImplementedError" => Some("RuntimeError"),
        "RecursionError" => Some("RuntimeError"),
        "SyntaxError" => Some("Exception"),
        "IndentationError" => Some("SyntaxError"),
        "TabError" => Some("IndentationError"),
        "SystemError" => Some("Exception"),
        "TypeError" => Some("Exception"),
        "ValueError" => Some("Exception"),
        "UnicodeError" => Some("ValueError"),
        "UnicodeDecodeError" => Some("UnicodeError"),
        "UnicodeEncodeError" => Some("UnicodeError"),
        "UnicodeTranslateError" => Some("UnicodeError"),
        "Warning" => Some("Exception"),
        "DeprecationWarning" => Some("Warning"),
        "PendingDeprecationWarning" => Some("Warning"),
        "RuntimeWarning" => Some("Warning"),
        "SyntaxWarning" => Some("Warning"),
        "UserWarning" => Some("Warning"),
        "FutureWarning" => Some("Warning"),
        "ImportWarning" => Some("Warning"),
        "UnicodeWarning" => Some("Warning"),
        "BytesWarning" => Some("Warning"),
        "ResourceWarning" => Some("Warning"),
        "EncodingWarning" => Some("Warning"),
        "EnvironmentError" => Some("OSError"),
        "IOError" => Some("OSError"),
        "WindowsError" => Some("OSError"),
        _ => Some("Exception"),
    }
}

unsafe fn ensure_cpython_exception_symbol(
    slot: *mut *mut c_void,
    type_ptr: *mut c_void,
    name: &str,
) {
    // SAFETY: caller passes valid pointer to static exception symbol slot.
    let raw = if unsafe { (*slot).is_null() } {
        // SAFETY: allocate and initialize stable type-object storage for exception export symbols.
        let allocated =
            unsafe { malloc(std::mem::size_of::<CpythonTypeObject>()) }.cast::<CpythonTypeObject>();
        if allocated.is_null() {
            return;
        }
        // SAFETY: `allocated` points to writable memory with full CpythonTypeObject size.
        unsafe {
            std::ptr::write_bytes(
                allocated.cast::<u8>(),
                0,
                std::mem::size_of::<CpythonTypeObject>(),
            );
        }
        // SAFETY: name is static ascii and NUL-free.
        let tp_name = CString::new(name)
            .ok()
            .map(|text| text.into_raw() as *const c_char)
            .unwrap_or(c"<exception>".as_ptr());
        // SAFETY: initialized storage for exception-type symbol metadata.
        unsafe {
            (*allocated).ob_refcnt = 1;
            (*allocated).ob_type = type_ptr;
            (*allocated).ob_size = 0;
            (*allocated).tp_name = tp_name;
            (*allocated).tp_basicsize = std::mem::size_of::<CpythonObjectHead>() as isize;
            (*allocated).tp_itemsize = 0;
            (*allocated).tp_flags =
                PY_TPFLAGS_BASETYPE | PY_TPFLAGS_READY | PY_TPFLAGS_BASE_EXC_SUBCLASS;
            *slot = allocated.cast::<c_void>();
        }
        allocated
    } else {
        // SAFETY: existing slot points to a stable exception-type object.
        unsafe { (*slot).cast::<CpythonTypeObject>() }
    };

    let default_base = std::ptr::addr_of_mut!(PyBaseObject_Type);
    let base_ptr = cpython_builtin_exception_parent_name(name)
        .and_then(cpython_exception_ptr_for_name)
        .map(|ptr| ptr.cast::<CpythonTypeObject>())
        .unwrap_or(default_base);

    // SAFETY: `raw` points to initialized exception-type storage.
    unsafe {
        (*raw).tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_READY | PY_TPFLAGS_BASE_EXC_SUBCLASS;
        (*raw).tp_base = base_ptr;
        if (*raw).tp_getattro.is_null() {
            (*raw).tp_getattro = PyObject_GenericGetAttr as *mut c_void;
        }
        if (*raw).tp_setattro.is_null() {
            (*raw).tp_setattro = PyObject_GenericSetAttr as *mut c_void;
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

fn cpython_slot_richcompare_method_def(attr_name: &str) -> Option<*mut CpythonMethodDef> {
    match attr_name {
        "__lt__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_LT_METHOD_DEF)),
        "__le__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_LE_METHOD_DEF)),
        "__eq__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_EQ_METHOD_DEF)),
        "__ne__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_NE_METHOD_DEF)),
        "__gt__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_GT_METHOD_DEF)),
        "__ge__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_GE_METHOD_DEF)),
        _ => None,
    }
}

fn cpython_slot_repr_str_method_def(attr_name: &str) -> Option<*mut CpythonMethodDef> {
    match attr_name {
        "__repr__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_REPR_METHOD_DEF)),
        "__str__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_STR_METHOD_DEF)),
        _ => None,
    }
}

fn cpython_slot_unary_method_def(attr_name: &str) -> Option<*mut CpythonMethodDef> {
    match attr_name {
        "__bool__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_BOOL_METHOD_DEF)),
        "__int__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_INT_METHOD_DEF)),
        "__float__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_FLOAT_METHOD_DEF)),
        "__index__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_INDEX_METHOD_DEF)),
        _ => None,
    }
}

fn cpython_slot_getitem_method_def(attr_name: &str) -> Option<*mut CpythonMethodDef> {
    (attr_name == "__getitem__").then_some(std::ptr::addr_of_mut!(CPY_SLOT_GETITEM_METHOD_DEF))
}

fn cpython_slot_len_iter_setitem_method_def(attr_name: &str) -> Option<*mut CpythonMethodDef> {
    match attr_name {
        "__len__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_LEN_METHOD_DEF)),
        "__iter__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_ITER_METHOD_DEF)),
        "__setitem__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_SETITEM_METHOD_DEF)),
        _ => None,
    }
}

fn cpython_slot_descriptor_method_def(attr_name: &str) -> Option<*mut CpythonMethodDef> {
    match attr_name {
        "__get__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_GET_METHOD_DEF)),
        "__set__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_SET_METHOD_DEF)),
        "__delete__" => Some(std::ptr::addr_of_mut!(CPY_SLOT_DELETE_METHOD_DEF)),
        _ => None,
    }
}

unsafe fn cpython_find_nb_bool_slot(type_ptr: *mut CpythonTypeObject) -> *mut c_void {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is read-only slot metadata from type hierarchy walk.
        let as_number = unsafe { (*current).tp_as_number.cast::<CpythonNumberMethods>() };
        if !as_number.is_null() {
            // SAFETY: number slot table is read-only.
            let slot = unsafe { (*as_number).nb_bool };
            if !slot.is_null() {
                return slot;
            }
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    std::ptr::null_mut()
}

unsafe fn cpython_find_nb_int_or_index_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void> {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is read-only slot metadata from type hierarchy walk.
        let as_number = unsafe { (*current).tp_as_number.cast::<CpythonNumberMethods>() };
        if !as_number.is_null() {
            // SAFETY: number slot table is read-only.
            if let Some(slot) = unsafe { (*as_number).nb_int } {
                return Some(slot);
            }
            // SAFETY: number slot table is read-only.
            if let Some(slot) = unsafe { (*as_number).nb_index } {
                return Some(slot);
            }
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    None
}

unsafe fn cpython_find_nb_float_slot(type_ptr: *mut CpythonTypeObject) -> *mut c_void {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is read-only slot metadata from type hierarchy walk.
        let as_number = unsafe { (*current).tp_as_number.cast::<CpythonNumberMethods>() };
        if !as_number.is_null() {
            // SAFETY: number slot table is read-only.
            let slot = unsafe { (*as_number).nb_float };
            if !slot.is_null() {
                return slot;
            }
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    std::ptr::null_mut()
}

unsafe fn cpython_find_nb_index_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void> {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is read-only slot metadata from type hierarchy walk.
        let as_number = unsafe { (*current).tp_as_number.cast::<CpythonNumberMethods>() };
        if !as_number.is_null() {
            // SAFETY: number slot table is read-only.
            if let Some(slot) = unsafe { (*as_number).nb_index } {
                return Some(slot);
            }
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    None
}

unsafe fn cpython_find_getitem_mapping_slot(type_ptr: *mut CpythonTypeObject) -> *mut c_void {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is read-only slot metadata from type hierarchy walk.
        let as_mapping = unsafe { (*current).tp_as_mapping.cast::<CpythonMappingMethods>() };
        if !as_mapping.is_null() {
            // SAFETY: mapping table is read-only.
            let slot = unsafe { (*as_mapping).mp_subscript };
            if !slot.is_null() {
                return slot;
            }
        }
        // SAFETY: `current` is read-only slot metadata from type hierarchy walk.
        let as_sequence = unsafe { (*current).tp_as_sequence.cast::<CpythonSequenceMethods>() };
        if !as_sequence.is_null() {
            // SAFETY: sequence table is read-only.
            let slot = unsafe { (*as_sequence).sq_item };
            if !slot.is_null() {
                return slot;
            }
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    std::ptr::null_mut()
}

unsafe fn cpython_find_len_slot(type_ptr: *mut CpythonTypeObject) -> *mut c_void {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is read-only slot metadata from type hierarchy walk.
        let as_mapping = unsafe { (*current).tp_as_mapping.cast::<CpythonMappingMethods>() };
        if !as_mapping.is_null() {
            // SAFETY: mapping table is read-only.
            let slot = unsafe { (*as_mapping).mp_length };
            if !slot.is_null() {
                return slot;
            }
        }
        // SAFETY: `current` is read-only slot metadata from type hierarchy walk.
        let as_sequence = unsafe { (*current).tp_as_sequence.cast::<CpythonSequenceMethods>() };
        if !as_sequence.is_null() {
            // SAFETY: sequence table is read-only.
            let slot = unsafe { (*as_sequence).sq_length };
            if !slot.is_null() {
                return slot;
            }
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    std::ptr::null_mut()
}

unsafe fn cpython_find_iter_slot(type_ptr: *mut CpythonTypeObject) -> *mut c_void {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: type slot metadata is read-only.
        let slot = unsafe { (*current).tp_iter };
        if !slot.is_null() {
            return slot;
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    std::ptr::null_mut()
}

unsafe fn cpython_find_repr_slot(type_ptr: *mut CpythonTypeObject) -> *mut c_void {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: type slot metadata is read-only.
        let slot = unsafe { (*current).tp_repr };
        if !slot.is_null() {
            return slot;
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    std::ptr::null_mut()
}

unsafe fn cpython_find_str_slot(type_ptr: *mut CpythonTypeObject) -> *mut c_void {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: type slot metadata is read-only.
        let slot = unsafe { (*current).tp_str };
        if !slot.is_null() {
            return slot;
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    std::ptr::null_mut()
}

unsafe fn cpython_find_setitem_slot(
    type_ptr: *mut CpythonTypeObject,
) -> (
    /* mapping */ *mut c_void,
    /* sequence */ *mut c_void,
) {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is read-only slot metadata from type hierarchy walk.
        let as_mapping = unsafe { (*current).tp_as_mapping.cast::<CpythonMappingMethods>() };
        if !as_mapping.is_null() {
            // SAFETY: mapping table is read-only.
            let mapping_slot = unsafe { (*as_mapping).mp_ass_subscript };
            if !mapping_slot.is_null() {
                return (mapping_slot, std::ptr::null_mut());
            }
        }
        // SAFETY: `current` is read-only slot metadata from type hierarchy walk.
        let as_sequence = unsafe { (*current).tp_as_sequence.cast::<CpythonSequenceMethods>() };
        if !as_sequence.is_null() {
            // SAFETY: sequence table is read-only.
            let sequence_slot = unsafe { (*as_sequence).sq_ass_item };
            if !sequence_slot.is_null() {
                return (std::ptr::null_mut(), sequence_slot);
            }
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    (std::ptr::null_mut(), std::ptr::null_mut())
}

unsafe fn cpython_find_descr_get_slot(type_ptr: *mut CpythonTypeObject) -> *mut c_void {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is a type pointer from bounded hierarchy walk.
        let slot = unsafe { (*current).tp_descr_get };
        if !slot.is_null() {
            return slot;
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    std::ptr::null_mut()
}

unsafe fn cpython_find_descr_set_slot(type_ptr: *mut CpythonTypeObject) -> *mut c_void {
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is a type pointer from bounded hierarchy walk.
        let slot = unsafe { (*current).tp_descr_set };
        if !slot.is_null() {
            return slot;
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    std::ptr::null_mut()
}

unsafe fn cpython_slot_unary_available(type_ptr: *mut CpythonTypeObject, attr_name: &str) -> bool {
    match attr_name {
        "__bool__" => unsafe { !cpython_find_nb_bool_slot(type_ptr).is_null() },
        "__int__" => unsafe { cpython_find_nb_int_or_index_slot(type_ptr).is_some() },
        "__float__" => unsafe { !cpython_find_nb_float_slot(type_ptr).is_null() },
        "__index__" => unsafe { cpython_find_nb_index_slot(type_ptr).is_some() },
        _ => false,
    }
}

unsafe fn cpython_slot_repr_str_available(
    type_ptr: *mut CpythonTypeObject,
    attr_name: &str,
) -> bool {
    match attr_name {
        "__repr__" => unsafe { !cpython_find_repr_slot(type_ptr).is_null() },
        "__str__" => unsafe { !cpython_find_str_slot(type_ptr).is_null() },
        _ => false,
    }
}

unsafe fn cpython_slot_getitem_available(type_ptr: *mut CpythonTypeObject) -> bool {
    // SAFETY: helper performs bounded type hierarchy walk.
    unsafe { !cpython_find_getitem_mapping_slot(type_ptr).is_null() }
}

unsafe fn cpython_slot_len_iter_setitem_available(
    type_ptr: *mut CpythonTypeObject,
    attr_name: &str,
) -> bool {
    match attr_name {
        "__len__" => {
            // SAFETY: helper performs bounded type hierarchy walk.
            unsafe { !cpython_find_len_slot(type_ptr).is_null() }
        }
        "__iter__" => {
            // SAFETY: helper performs bounded type hierarchy walk.
            unsafe { !cpython_find_iter_slot(type_ptr).is_null() }
        }
        "__setitem__" => {
            // SAFETY: helper performs bounded type hierarchy walk.
            let (mapping_slot, sequence_slot) = unsafe { cpython_find_setitem_slot(type_ptr) };
            !mapping_slot.is_null() || !sequence_slot.is_null()
        }
        _ => false,
    }
}

unsafe fn cpython_slot_descriptor_available(
    type_ptr: *mut CpythonTypeObject,
    attr_name: &str,
) -> bool {
    match attr_name {
        "__get__" => {
            // SAFETY: helper performs bounded type hierarchy walk.
            unsafe { !cpython_find_descr_get_slot(type_ptr).is_null() }
        }
        "__set__" | "__delete__" => {
            // SAFETY: helper performs bounded type hierarchy walk.
            unsafe { !cpython_find_descr_set_slot(type_ptr).is_null() }
        }
        _ => false,
    }
}

fn cpython_slot_init_method_def() -> *mut CpythonMethodDef {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let init_callable: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *mut c_void,
        ) -> *mut c_void = cpython_slot_dunder_init;
        // SAFETY: slot-init wrapper follows the METH_VARARGS|METH_KEYWORDS ABI.
        unsafe {
            CPY_SLOT_INIT_METHOD_DEF.ml_meth = Some(std::mem::transmute::<
                unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void,
                unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
            >(init_callable));
        }
    });
    std::ptr::addr_of_mut!(CPY_SLOT_INIT_METHOD_DEF)
}

unsafe extern "C" fn cpython_slot_dunder_repr(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    // SAFETY: wrapper validates tuple arity and returns either `self_obj` or arg0.
    let target = unsafe { cpython_slot_unary_target(self_obj, args, "__repr__") };
    if target.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `target` is a valid candidate object pointer.
    let type_ptr = unsafe {
        target
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: helper performs bounded type hierarchy walk.
    let slot = unsafe { cpython_find_repr_slot(type_ptr) };
    if slot.is_null() {
        cpython_set_error("TypeError: descriptor '__repr__' is unavailable for this object");
        return std::ptr::null_mut();
    }
    // SAFETY: `tp_repr` follows unary reprfunc ABI.
    let repr_fn: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
        unsafe { std::mem::transmute(slot) };
    // SAFETY: slot invocation follows CPython ABI.
    unsafe { repr_fn(target) }
}

unsafe extern "C" fn cpython_slot_dunder_str(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    // SAFETY: wrapper validates tuple arity and returns either `self_obj` or arg0.
    let target = unsafe { cpython_slot_unary_target(self_obj, args, "__str__") };
    if target.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `target` is a valid candidate object pointer.
    let type_ptr = unsafe {
        target
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: helper performs bounded type hierarchy walk.
    let slot = unsafe { cpython_find_str_slot(type_ptr) };
    if slot.is_null() {
        cpython_set_error("TypeError: descriptor '__str__' is unavailable for this object");
        return std::ptr::null_mut();
    }
    // SAFETY: `tp_str` follows unary reprfunc ABI.
    let str_fn: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
        unsafe { std::mem::transmute(slot) };
    // SAFETY: slot invocation follows CPython ABI.
    unsafe { str_fn(target) }
}

unsafe fn cpython_slot_richcompare_dunder_call(
    self_obj: *mut c_void,
    args: *mut c_void,
    op: i32,
    opname: &str,
) -> *mut c_void {
    if args.is_null() {
        cpython_set_error(format!(
            "TypeError: descriptor '{opname}' requires an argument tuple"
        ));
        return std::ptr::null_mut();
    }
    // SAFETY: `args` is expected to be a tuple pointer for METH_VARARGS call flow.
    let argc = unsafe { PyTuple_Size(args) };
    if argc < 0 {
        return std::ptr::null_mut();
    }
    let (left, right) = if argc == 2 {
        // SAFETY: arg tuple size validated above.
        let left = unsafe { PyTuple_GetItem(args, 0) };
        // SAFETY: arg tuple size validated above.
        let right = unsafe { PyTuple_GetItem(args, 1) };
        (left, right)
    } else if argc == 1 && !self_obj.is_null() {
        // SAFETY: arg tuple size validated above.
        let right = unsafe { PyTuple_GetItem(args, 0) };
        (self_obj, right)
    } else {
        let expected = if self_obj.is_null() { 2 } else { 1 };
        cpython_set_error(format!(
            "TypeError: descriptor '{opname}' expected {expected} arguments, got {argc}"
        ));
        return std::ptr::null_mut();
    };
    if left.is_null() || right.is_null() {
        return std::ptr::null_mut();
    }
    if let Some(result) = cpython_try_richcompare_slot(left, right, op) {
        return result;
    }
    let not_implemented = std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast::<c_void>();
    // SAFETY: singleton is process-lifetime stable.
    unsafe { Py_IncRef(not_implemented) };
    not_implemented
}

unsafe fn cpython_slot_unary_target(
    self_obj: *mut c_void,
    args: *mut c_void,
    opname: &str,
) -> *mut c_void {
    if args.is_null() {
        cpython_set_error(format!(
            "TypeError: descriptor '{opname}' requires an argument tuple"
        ));
        return std::ptr::null_mut();
    }
    // SAFETY: `args` is expected to be a tuple pointer for METH_VARARGS call flow.
    let argc = unsafe { PyTuple_Size(args) };
    if argc < 0 {
        return std::ptr::null_mut();
    }
    if !self_obj.is_null() {
        if argc != 0 {
            cpython_set_error(format!(
                "TypeError: descriptor '{opname}' expected 0 arguments, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        return self_obj;
    }
    if argc != 1 {
        cpython_set_error(format!(
            "TypeError: descriptor '{opname}' expected 1 arguments, got {argc}"
        ));
        return std::ptr::null_mut();
    }
    // SAFETY: arg tuple size validated above.
    unsafe { PyTuple_GetItem(args, 0) }
}

unsafe extern "C" fn cpython_slot_dunder_bool(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    // SAFETY: wrapper validates tuple arity and returns either `self_obj` or arg0.
    let target = unsafe { cpython_slot_unary_target(self_obj, args, "__bool__") };
    if target.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `target` is a valid candidate object pointer.
    let type_ptr = unsafe {
        target
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: bounded type hierarchy walk for `nb_bool`.
    let slot = unsafe { cpython_find_nb_bool_slot(type_ptr) };
    if slot.is_null() {
        cpython_set_error("TypeError: descriptor '__bool__' is unavailable for this object");
        return std::ptr::null_mut();
    }
    // SAFETY: `nb_bool` follows `inquiry` ABI.
    let bool_fn: unsafe extern "C" fn(*mut c_void) -> i32 = unsafe { std::mem::transmute(slot) };
    // SAFETY: slot ABI invocation.
    let result = unsafe { bool_fn(target) };
    if result < 0 {
        return std::ptr::null_mut();
    }
    cpython_new_ptr_for_value(Value::Bool(result != 0))
}

unsafe extern "C" fn cpython_slot_dunder_int(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    // SAFETY: wrapper validates tuple arity and returns either `self_obj` or arg0.
    let target = unsafe { cpython_slot_unary_target(self_obj, args, "__int__") };
    if target.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `target` is a valid candidate object pointer.
    let type_ptr = unsafe {
        target
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: bounded type hierarchy walk for `nb_int`/`nb_index`.
    if let Some(slot_fn) = unsafe { cpython_find_nb_int_or_index_slot(type_ptr) } {
        // SAFETY: slot ABI invocation.
        return unsafe { slot_fn(target) };
    }
    cpython_set_error("TypeError: descriptor '__int__' is unavailable for this object");
    std::ptr::null_mut()
}

unsafe extern "C" fn cpython_slot_dunder_float(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    // SAFETY: wrapper validates tuple arity and returns either `self_obj` or arg0.
    let target = unsafe { cpython_slot_unary_target(self_obj, args, "__float__") };
    if target.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `target` is a valid candidate object pointer.
    let type_ptr = unsafe {
        target
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: bounded type hierarchy walk for `nb_float`.
    let slot = unsafe { cpython_find_nb_float_slot(type_ptr) };
    if slot.is_null() {
        cpython_set_error("TypeError: descriptor '__float__' is unavailable for this object");
        return std::ptr::null_mut();
    }
    // SAFETY: `nb_float` follows unaryfunc ABI.
    let float_fn: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
        unsafe { std::mem::transmute(slot) };
    // SAFETY: slot ABI invocation.
    unsafe { float_fn(target) }
}

unsafe extern "C" fn cpython_slot_dunder_index(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    // SAFETY: wrapper validates tuple arity and returns either `self_obj` or arg0.
    let target = unsafe { cpython_slot_unary_target(self_obj, args, "__index__") };
    if target.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `target` is a valid candidate object pointer.
    let type_ptr = unsafe {
        target
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: bounded type hierarchy walk for `nb_index`.
    if let Some(index_fn) = unsafe { cpython_find_nb_index_slot(type_ptr) } {
        // SAFETY: slot ABI invocation.
        return unsafe { index_fn(target) };
    }
    cpython_set_error("TypeError: descriptor '__index__' is unavailable for this object");
    std::ptr::null_mut()
}

unsafe extern "C" fn cpython_slot_dunder_getitem(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    if args.is_null() {
        cpython_set_error("TypeError: descriptor '__getitem__' requires an argument tuple");
        return std::ptr::null_mut();
    }
    // SAFETY: `args` is expected to be a tuple pointer for METH_VARARGS call flow.
    let argc = unsafe { PyTuple_Size(args) };
    if argc < 0 {
        return std::ptr::null_mut();
    }
    let (target, key) = if !self_obj.is_null() {
        if argc != 1 {
            cpython_set_error(format!(
                "TypeError: descriptor '__getitem__' expected 1 arguments, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: arg tuple size validated above.
        let key = unsafe { PyTuple_GetItem(args, 0) };
        (self_obj, key)
    } else {
        if argc != 2 {
            cpython_set_error(format!(
                "TypeError: descriptor '__getitem__' expected 2 arguments, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: arg tuple size validated above.
        let target = unsafe { PyTuple_GetItem(args, 0) };
        // SAFETY: arg tuple size validated above.
        let key = unsafe { PyTuple_GetItem(args, 1) };
        (target, key)
    };
    if target.is_null() || key.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `target` is a valid candidate object pointer.
    let type_ptr = unsafe {
        target
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is a type pointer from bounded hierarchy walk.
        let as_mapping = unsafe { (*current).tp_as_mapping.cast::<CpythonMappingMethods>() };
        if !as_mapping.is_null() {
            // SAFETY: mapping table is readable in this branch.
            let slot = unsafe { (*as_mapping).mp_subscript };
            if !slot.is_null() {
                // SAFETY: `mp_subscript` follows binaryfunc ABI.
                let getitem_fn: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                    unsafe { std::mem::transmute(slot) };
                // SAFETY: slot ABI invocation.
                return unsafe { getitem_fn(target, key) };
            }
        }
        // SAFETY: `current` is a type pointer from bounded hierarchy walk.
        let as_sequence = unsafe { (*current).tp_as_sequence.cast::<CpythonSequenceMethods>() };
        if !as_sequence.is_null() {
            // SAFETY: sequence table is readable in this branch.
            let slot = unsafe { (*as_sequence).sq_item };
            if !slot.is_null() {
                // SAFETY: converts index object to Py_ssize_t per CPython ABI.
                let index = unsafe { PyLong_AsSsize_t(key) };
                // SAFETY: checks active C error indicator after conversion.
                if index == -1 && unsafe { !PyErr_Occurred().is_null() } {
                    return std::ptr::null_mut();
                }
                // SAFETY: `sq_item` follows `ssizeargfunc` ABI.
                let sq_item_fn: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
                    unsafe { std::mem::transmute(slot) };
                // SAFETY: slot ABI invocation.
                return unsafe { sq_item_fn(target, index) };
            }
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    cpython_set_error("TypeError: descriptor '__getitem__' is unavailable for this object");
    std::ptr::null_mut()
}

unsafe extern "C" fn cpython_compat_descriptor_tp_descr_get(
    descriptor: *mut c_void,
    object: *mut c_void,
    _owner: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if descriptor.is_null() {
            context.set_error("descriptor __get__ received null descriptor");
            return std::ptr::null_mut();
        }
        if object.is_null() {
            unsafe { Py_XIncRef(descriptor) };
            return descriptor;
        }
        let object_type = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        context
            .resolve_descriptor_attr_ptr(descriptor, object, object_type, false)
            .unwrap_or_else(|| {
                context.set_error("descriptor metadata is unavailable");
                std::ptr::null_mut()
            })
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

unsafe extern "C" fn cpython_compat_descriptor_tp_descr_set(
    descriptor: *mut c_void,
    object: *mut c_void,
    value: *mut c_void,
) -> c_int {
    with_active_cpython_context_mut(|context| {
        if descriptor.is_null() || object.is_null() {
            context.set_error("descriptor __set__ received null receiver");
            return -1;
        }
        let object_type = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        context
            .set_descriptor_attr_ptr(descriptor, object, object_type, value)
            .unwrap_or_else(|| {
                context.set_error("descriptor metadata is unavailable");
                -1
            })
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

unsafe extern "C" fn cpython_slot_dunder_get(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    if args.is_null() {
        cpython_set_error("TypeError: descriptor '__get__' requires an argument tuple");
        return std::ptr::null_mut();
    }
    // SAFETY: `args` is expected to be a tuple pointer for METH_VARARGS call flow.
    let argc = unsafe { PyTuple_Size(args) };
    if argc < 0 {
        return std::ptr::null_mut();
    }
    let (descriptor, instance, owner) = if !self_obj.is_null() {
        if argc != 1 && argc != 2 {
            cpython_set_error(format!(
                "TypeError: descriptor '__get__' expected 1 or 2 arguments, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: arg tuple size validated above.
        let instance = unsafe { PyTuple_GetItem(args, 0) };
        let owner = if argc == 2 {
            // SAFETY: arg tuple size validated above.
            unsafe { PyTuple_GetItem(args, 1) }
        } else {
            std::ptr::null_mut()
        };
        (self_obj, instance, owner)
    } else {
        if argc != 2 && argc != 3 {
            cpython_set_error(format!(
                "TypeError: descriptor '__get__' expected 2 or 3 arguments, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: arg tuple size validated above.
        let descriptor = unsafe { PyTuple_GetItem(args, 0) };
        // SAFETY: arg tuple size validated above.
        let instance = unsafe { PyTuple_GetItem(args, 1) };
        let owner = if argc == 3 {
            // SAFETY: arg tuple size validated above.
            unsafe { PyTuple_GetItem(args, 2) }
        } else {
            std::ptr::null_mut()
        };
        (descriptor, instance, owner)
    };
    if descriptor.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: descriptor is expected to be a valid object pointer.
    let descriptor_type = unsafe {
        descriptor
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if descriptor_type.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: helper performs bounded type hierarchy walk.
    let slot = unsafe { cpython_find_descr_get_slot(descriptor_type) };
    if slot.is_null() {
        cpython_set_error("TypeError: descriptor '__get__' is unavailable for this object");
        return std::ptr::null_mut();
    }
    let descriptor_get: unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
    ) -> *mut c_void =
        // SAFETY: `tp_descr_get` follows descriptor-get ABI.
        unsafe { std::mem::transmute(slot) };
    let self_ptr =
        if instance.is_null() || instance == std::ptr::addr_of_mut!(_Py_NoneStruct).cast() {
            std::ptr::null_mut()
        } else {
            instance
        };
    // SAFETY: descriptor slot invocation follows CPython ABI.
    unsafe { descriptor_get(descriptor, self_ptr, owner) }
}

unsafe extern "C" fn cpython_slot_dunder_set(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    if args.is_null() {
        cpython_set_error("TypeError: descriptor '__set__' requires an argument tuple");
        return std::ptr::null_mut();
    }
    // SAFETY: `args` is expected to be a tuple pointer for METH_VARARGS call flow.
    let argc = unsafe { PyTuple_Size(args) };
    if argc < 0 {
        return std::ptr::null_mut();
    }
    let (descriptor, instance, value) = if !self_obj.is_null() {
        if argc != 2 {
            cpython_set_error(format!(
                "TypeError: descriptor '__set__' expected 2 arguments, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: arg tuple size validated above.
        let instance = unsafe { PyTuple_GetItem(args, 0) };
        // SAFETY: arg tuple size validated above.
        let value = unsafe { PyTuple_GetItem(args, 1) };
        (self_obj, instance, value)
    } else {
        if argc != 3 {
            cpython_set_error(format!(
                "TypeError: descriptor '__set__' expected 3 arguments, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: arg tuple size validated above.
        let descriptor = unsafe { PyTuple_GetItem(args, 0) };
        // SAFETY: arg tuple size validated above.
        let instance = unsafe { PyTuple_GetItem(args, 1) };
        // SAFETY: arg tuple size validated above.
        let value = unsafe { PyTuple_GetItem(args, 2) };
        (descriptor, instance, value)
    };
    if descriptor.is_null() || instance.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: descriptor is expected to be a valid object pointer.
    let descriptor_type = unsafe {
        descriptor
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if descriptor_type.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: helper performs bounded type hierarchy walk.
    let slot = unsafe { cpython_find_descr_set_slot(descriptor_type) };
    if slot.is_null() {
        cpython_set_error("TypeError: descriptor '__set__' is unavailable for this object");
        return std::ptr::null_mut();
    }
    let descriptor_set: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int =
        // SAFETY: `tp_descr_set` follows descriptor-set ABI.
        unsafe { std::mem::transmute(slot) };
    // SAFETY: descriptor slot invocation follows CPython ABI.
    let status = unsafe { descriptor_set(descriptor, instance, value) };
    if status < 0 {
        return std::ptr::null_mut();
    }
    let none_ptr = std::ptr::addr_of_mut!(_Py_NoneStruct).cast::<c_void>();
    // SAFETY: singleton reference increment for Python-visible return value.
    unsafe { Py_IncRef(none_ptr) };
    none_ptr
}

unsafe extern "C" fn cpython_slot_dunder_delete(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    if args.is_null() {
        cpython_set_error("TypeError: descriptor '__delete__' requires an argument tuple");
        return std::ptr::null_mut();
    }
    // SAFETY: `args` is expected to be a tuple pointer for METH_VARARGS call flow.
    let argc = unsafe { PyTuple_Size(args) };
    if argc < 0 {
        return std::ptr::null_mut();
    }
    let (descriptor, instance) = if !self_obj.is_null() {
        if argc != 1 {
            cpython_set_error(format!(
                "TypeError: descriptor '__delete__' expected 1 argument, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: arg tuple size validated above.
        let instance = unsafe { PyTuple_GetItem(args, 0) };
        (self_obj, instance)
    } else {
        if argc != 2 {
            cpython_set_error(format!(
                "TypeError: descriptor '__delete__' expected 2 arguments, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: arg tuple size validated above.
        let descriptor = unsafe { PyTuple_GetItem(args, 0) };
        // SAFETY: arg tuple size validated above.
        let instance = unsafe { PyTuple_GetItem(args, 1) };
        (descriptor, instance)
    };
    if descriptor.is_null() || instance.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: descriptor is expected to be a valid object pointer.
    let descriptor_type = unsafe {
        descriptor
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if descriptor_type.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: helper performs bounded type hierarchy walk.
    let slot = unsafe { cpython_find_descr_set_slot(descriptor_type) };
    if slot.is_null() {
        cpython_set_error("TypeError: descriptor '__delete__' is unavailable for this object");
        return std::ptr::null_mut();
    }
    let descriptor_set: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int =
        // SAFETY: `tp_descr_set` follows descriptor-set ABI.
        unsafe { std::mem::transmute(slot) };
    // SAFETY: descriptor slot invocation follows CPython ABI; delete passes null value.
    let status = unsafe { descriptor_set(descriptor, instance, std::ptr::null_mut()) };
    if status < 0 {
        return std::ptr::null_mut();
    }
    let none_ptr = std::ptr::addr_of_mut!(_Py_NoneStruct).cast::<c_void>();
    // SAFETY: singleton reference increment for Python-visible return value.
    unsafe { Py_IncRef(none_ptr) };
    none_ptr
}

unsafe extern "C" fn cpython_slot_dunder_len(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    // SAFETY: wrapper validates tuple arity and returns either `self_obj` or arg0.
    let target = unsafe { cpython_slot_unary_target(self_obj, args, "__len__") };
    if target.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `target` is a valid candidate object pointer.
    let type_ptr = unsafe {
        target
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is a type pointer from bounded hierarchy walk.
        let as_mapping = unsafe { (*current).tp_as_mapping.cast::<CpythonMappingMethods>() };
        if !as_mapping.is_null() {
            // SAFETY: mapping table is read-only in this branch.
            let slot = unsafe { (*as_mapping).mp_length };
            if !slot.is_null() {
                // SAFETY: `mp_length` follows lenfunc ABI.
                let len_fn: unsafe extern "C" fn(*mut c_void) -> isize =
                    unsafe { std::mem::transmute(slot) };
                // SAFETY: slot ABI invocation.
                let length = unsafe { len_fn(target) };
                if length < 0 {
                    // SAFETY: inspect active error state after slot call.
                    if unsafe { PyErr_Occurred() }.is_null() {
                        cpython_set_error("SystemError: __len__ returned negative without error");
                    }
                    return std::ptr::null_mut();
                }
                // SAFETY: stable exported helper for Py_ssize_t -> int object.
                return unsafe { PyLong_FromSsize_t(length) };
            }
        }
        // SAFETY: `current` is a type pointer from bounded hierarchy walk.
        let as_sequence = unsafe { (*current).tp_as_sequence.cast::<CpythonSequenceMethods>() };
        if !as_sequence.is_null() {
            // SAFETY: sequence table is read-only in this branch.
            let slot = unsafe { (*as_sequence).sq_length };
            if !slot.is_null() {
                // SAFETY: `sq_length` follows lenfunc ABI.
                let len_fn: unsafe extern "C" fn(*mut c_void) -> isize =
                    unsafe { std::mem::transmute(slot) };
                // SAFETY: slot ABI invocation.
                let length = unsafe { len_fn(target) };
                if length < 0 {
                    // SAFETY: inspect active error state after slot call.
                    if unsafe { PyErr_Occurred() }.is_null() {
                        cpython_set_error("SystemError: __len__ returned negative without error");
                    }
                    return std::ptr::null_mut();
                }
                // SAFETY: stable exported helper for Py_ssize_t -> int object.
                return unsafe { PyLong_FromSsize_t(length) };
            }
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    cpython_set_error("TypeError: descriptor '__len__' is unavailable for this object");
    std::ptr::null_mut()
}

unsafe extern "C" fn cpython_slot_dunder_iter(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    // SAFETY: wrapper validates tuple arity and returns either `self_obj` or arg0.
    let target = unsafe { cpython_slot_unary_target(self_obj, args, "__iter__") };
    if target.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `target` is a valid candidate object pointer.
    let type_ptr = unsafe {
        target
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: bounded type hierarchy walk for `tp_iter`.
    let slot = unsafe { cpython_find_iter_slot(type_ptr) };
    if slot.is_null() {
        cpython_set_error("TypeError: descriptor '__iter__' is unavailable for this object");
        return std::ptr::null_mut();
    }
    // SAFETY: `tp_iter` follows unaryfunc ABI.
    let iter_fn: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
        unsafe { std::mem::transmute(slot) };
    // SAFETY: slot ABI invocation.
    unsafe { iter_fn(target) }
}

unsafe extern "C" fn cpython_slot_dunder_setitem(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    if args.is_null() {
        cpython_set_error("TypeError: descriptor '__setitem__' requires an argument tuple");
        return std::ptr::null_mut();
    }
    // SAFETY: `args` is expected to be a tuple pointer for METH_VARARGS call flow.
    let argc = unsafe { PyTuple_Size(args) };
    if argc < 0 {
        return std::ptr::null_mut();
    }
    let (target, key, value) = if !self_obj.is_null() {
        if argc != 2 {
            cpython_set_error(format!(
                "TypeError: descriptor '__setitem__' expected 2 arguments, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: arg tuple size validated above.
        let key = unsafe { PyTuple_GetItem(args, 0) };
        // SAFETY: arg tuple size validated above.
        let value = unsafe { PyTuple_GetItem(args, 1) };
        (self_obj, key, value)
    } else {
        if argc != 3 {
            cpython_set_error(format!(
                "TypeError: descriptor '__setitem__' expected 3 arguments, got {argc}"
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: arg tuple size validated above.
        let target = unsafe { PyTuple_GetItem(args, 0) };
        // SAFETY: arg tuple size validated above.
        let key = unsafe { PyTuple_GetItem(args, 1) };
        // SAFETY: arg tuple size validated above.
        let value = unsafe { PyTuple_GetItem(args, 2) };
        (target, key, value)
    };
    if target.is_null() || key.is_null() || value.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `target` is a valid candidate object pointer.
    let type_ptr = unsafe {
        target
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }

    let mut current = type_ptr;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // SAFETY: `current` is a type pointer from bounded hierarchy walk.
        let as_mapping = unsafe { (*current).tp_as_mapping.cast::<CpythonMappingMethods>() };
        if !as_mapping.is_null() {
            // SAFETY: mapping table is readable in this branch.
            let slot = unsafe { (*as_mapping).mp_ass_subscript };
            if !slot.is_null() {
                // SAFETY: `mp_ass_subscript` follows objobjargproc ABI.
                let setitem_fn: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
                    unsafe { std::mem::transmute(slot) };
                // SAFETY: slot ABI invocation.
                let status = unsafe { setitem_fn(target, key, value) };
                if status == 0 {
                    return cpython_new_ptr_for_value(Value::None);
                }
                return std::ptr::null_mut();
            }
        }
        // SAFETY: `current` is a type pointer from bounded hierarchy walk.
        let as_sequence = unsafe { (*current).tp_as_sequence.cast::<CpythonSequenceMethods>() };
        if !as_sequence.is_null() {
            // SAFETY: sequence table is readable in this branch.
            let slot = unsafe { (*as_sequence).sq_ass_item };
            if !slot.is_null() {
                // SAFETY: converts index object to Py_ssize_t per CPython ABI.
                let index = unsafe { PyLong_AsSsize_t(key) };
                // SAFETY: checks active C error indicator after conversion.
                if index == -1 && unsafe { !PyErr_Occurred().is_null() } {
                    return std::ptr::null_mut();
                }
                // SAFETY: `sq_ass_item` follows ssizeobjargproc ABI.
                let setitem_fn: unsafe extern "C" fn(*mut c_void, isize, *mut c_void) -> i32 =
                    unsafe { std::mem::transmute(slot) };
                // SAFETY: slot ABI invocation.
                let status = unsafe { setitem_fn(target, index, value) };
                if status == 0 {
                    return cpython_new_ptr_for_value(Value::None);
                }
                return std::ptr::null_mut();
            }
        }
        // SAFETY: type hierarchy link read-only.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    cpython_set_error("TypeError: descriptor '__setitem__' is unavailable for this object");
    std::ptr::null_mut()
}

unsafe extern "C" fn cpython_slot_dunder_lt(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    unsafe { cpython_slot_richcompare_dunder_call(self_obj, args, CPY_RICHCMP_LT, "__lt__") }
}

unsafe extern "C" fn cpython_slot_dunder_le(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    unsafe { cpython_slot_richcompare_dunder_call(self_obj, args, CPY_RICHCMP_LE, "__le__") }
}

unsafe extern "C" fn cpython_slot_dunder_eq(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    unsafe { cpython_slot_richcompare_dunder_call(self_obj, args, CPY_RICHCMP_EQ, "__eq__") }
}

unsafe extern "C" fn cpython_slot_dunder_ne(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    unsafe { cpython_slot_richcompare_dunder_call(self_obj, args, CPY_RICHCMP_NE, "__ne__") }
}

unsafe extern "C" fn cpython_slot_dunder_gt(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    unsafe { cpython_slot_richcompare_dunder_call(self_obj, args, CPY_RICHCMP_GT, "__gt__") }
}

unsafe extern "C" fn cpython_slot_dunder_ge(
    self_obj: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    unsafe { cpython_slot_richcompare_dunder_call(self_obj, args, CPY_RICHCMP_GE, "__ge__") }
}

unsafe extern "C" fn cpython_slot_dunder_init(
    self_obj: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    if self_obj.is_null() {
        cpython_set_error("TypeError: descriptor '__init__' received null self");
        return std::ptr::null_mut();
    }
    let args_tuple = if args.is_null() {
        // SAFETY: tuple allocation follows CPython ABI.
        let tuple = unsafe { PyTuple_New(0) };
        if tuple.is_null() {
            return std::ptr::null_mut();
        }
        tuple
    } else {
        args
    };
    let needs_args_decref = args.is_null();
    let receiver_is_type_object = cpython_is_type_object_ptr(self_obj);
    let (target, init_args) = if receiver_is_type_object {
        // SAFETY: tuple pointer is either provided by the caller or created above.
        let argc = unsafe { PyTuple_Size(args_tuple) };
        if argc < 1 {
            if needs_args_decref {
                // SAFETY: tuple was allocated in this function.
                unsafe { Py_DecRef(args_tuple) };
            }
            cpython_set_error("TypeError: descriptor '__init__' requires an instance argument");
            return std::ptr::null_mut();
        }
        // SAFETY: tuple size validated above.
        let instance = unsafe { PyTuple_GetItem(args_tuple, 0) };
        if instance.is_null() {
            if needs_args_decref {
                // SAFETY: tuple was allocated in this function.
                unsafe { Py_DecRef(args_tuple) };
            }
            return std::ptr::null_mut();
        }
        let tail_len = (argc - 1) as usize;
        // SAFETY: tuple allocation follows CPython ABI.
        let tail_tuple = unsafe { PyTuple_New(tail_len as isize) };
        if tail_tuple.is_null() {
            if needs_args_decref {
                // SAFETY: tuple was allocated in this function.
                unsafe { Py_DecRef(args_tuple) };
            }
            return std::ptr::null_mut();
        }
        for index in 0..tail_len {
            // SAFETY: bounds checked by `tail_len`.
            let value = unsafe { PyTuple_GetItem(args_tuple, (index + 1) as isize) };
            if value.is_null() {
                if needs_args_decref {
                    // SAFETY: tuple was allocated in this function.
                    unsafe { Py_DecRef(args_tuple) };
                }
                // SAFETY: tuple was allocated in this function.
                unsafe { Py_DecRef(tail_tuple) };
                return std::ptr::null_mut();
            }
            // SAFETY: destination tuple steals a reference.
            unsafe { Py_XIncRef(value) };
            if unsafe { PyTuple_SetItem(tail_tuple, index as isize, value) } != 0 {
                if needs_args_decref {
                    // SAFETY: tuple was allocated in this function.
                    unsafe { Py_DecRef(args_tuple) };
                }
                // SAFETY: tuple was allocated in this function.
                unsafe { Py_DecRef(tail_tuple) };
                return std::ptr::null_mut();
            }
        }
        (instance, tail_tuple)
    } else {
        (self_obj, args_tuple)
    };

    const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
    let target_type = if receiver_is_type_object {
        self_obj.cast::<CpythonTypeObject>()
    } else if (target as usize) < MIN_VALID_PTR
        || (target as usize) % std::mem::align_of::<CpythonObjectHead>() != 0
    {
        std::ptr::null_mut()
    } else {
        // SAFETY: `target` pointer shape was validated above.
        unsafe {
            target
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        }
    };
    if target_type.is_null() {
        if needs_args_decref {
            // SAFETY: tuple was allocated in this function.
            unsafe { Py_DecRef(args_tuple) };
        }
        if init_args != args_tuple {
            // SAFETY: tuple was allocated in this function.
            unsafe { Py_DecRef(init_args) };
        }
        cpython_set_error("TypeError: descriptor '__init__' received target without type");
        return std::ptr::null_mut();
    }
    // SAFETY: target type pointer is non-null above.
    let init_slot = unsafe { (*target_type).tp_init };
    if init_slot.is_null() {
        if needs_args_decref {
            // SAFETY: tuple was allocated in this function.
            unsafe { Py_DecRef(args_tuple) };
        }
        if init_args != args_tuple {
            // SAFETY: tuple was allocated in this function.
            unsafe { Py_DecRef(init_args) };
        }
        // SAFETY: type pointer is valid for metadata lookup.
        let type_name = unsafe { c_name_to_string((*target_type).tp_name) }
            .unwrap_or_else(|_| "<unknown>".to_string());
        cpython_set_error(format!(
            "TypeError: descriptor '__init__' for '{type_name}' has no initializer"
        ));
        return std::ptr::null_mut();
    }
    let init_fn: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int =
        // SAFETY: slot pointer follows CPython `initproc` ABI.
        unsafe { std::mem::transmute(init_slot) };
    let status = unsafe { init_fn(target, init_args, kwargs) };
    if needs_args_decref {
        // SAFETY: tuple was allocated in this function.
        unsafe { Py_DecRef(args_tuple) };
    }
    if init_args != args_tuple {
        // SAFETY: tuple was allocated in this function.
        unsafe { Py_DecRef(init_args) };
    }
    if status < 0 {
        return std::ptr::null_mut();
    }
    let none_ptr = std::ptr::addr_of_mut!(_Py_NoneStruct).cast::<c_void>();
    // SAFETY: None singleton is process-global and stable.
    unsafe { Py_IncRef(none_ptr) };
    none_ptr
}

fn cpython_call_noargs_attr_on_self(self_obj: *mut c_void, attr_name: &str) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if self_obj.is_null() {
            context.set_error("null self pointer for no-args method dispatch");
            return std::ptr::null_mut();
        }
        let Some(target) = context.cpython_value_from_ptr_or_proxy(self_obj) else {
            context.set_error("unknown self pointer for no-args method dispatch");
            return std::ptr::null_mut();
        };
        let callable = match cpython_getattr_in_context(context, target, attr_name) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let result =
            match cpython_call_internal_in_context(context, callable, Vec::new(), HashMap::new()) {
                Ok(value) => value,
                Err(err) => {
                    context.set_error_from_runtime_error(err);
                    return std::ptr::null_mut();
                }
            };
        context.alloc_cpython_ptr_for_value(result)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

unsafe extern "C" fn cpython_long_bit_length_noargs_method(
    self_obj: *mut c_void,
    _noargs: *mut c_void,
) -> *mut c_void {
    cpython_call_noargs_attr_on_self(self_obj, "bit_length")
}

unsafe extern "C" fn cpython_float_as_integer_ratio_noargs_method(
    self_obj: *mut c_void,
    _noargs: *mut c_void,
) -> *mut c_void {
    cpython_call_noargs_attr_on_self(self_obj, "as_integer_ratio")
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

unsafe extern "C" fn cpython_long_nb_add_slot(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    cpython_numeric_runtime::cpython_binary_numeric_op_with_heap(left, right, add_values)
}

fn initialize_cpython_compat_type_objects() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        let type_ptr = std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>();
        PyType_Type.ob_type = type_ptr;
        PyType_Type.tp_call = cpython_type_tp_call as *mut c_void;
        PyType_Type.tp_alloc = PyType_GenericAlloc as *mut c_void;
        PyType_Type.tp_new = PyType_GenericNew as *mut c_void;
        PyType_Type.tp_getattro = cpython_type_tp_getattro as *mut c_void;
        PyType_Type.tp_setattro = cpython_type_tp_setattro as *mut c_void;
        PyType_Type.tp_as_mapping = std::ptr::addr_of_mut!(PY_TYPE_MAPPING_METHODS).cast();
        PyType_Type.tp_base = std::ptr::addr_of_mut!(PyBaseObject_Type);
        PyBaseObject_Type.tp_alloc = PyType_GenericAlloc as *mut c_void;
        PyBaseObject_Type.tp_new = PyType_GenericNew as *mut c_void;
        PyBaseObject_Type.tp_getattro = PyObject_GenericGetAttr as *mut c_void;
        PyBaseObject_Type.tp_setattro = PyObject_GenericSetAttr as *mut c_void;
        PyBaseObject_Type.tp_repr = cpython_object_tp_repr as *mut c_void;
        PyBaseObject_Type.tp_str = cpython_object_tp_str as *mut c_void;
        PyCFunction_Type.tp_call = cpython_cfunction_tp_call as *mut c_void;
        PyCFunction_Type.tp_getattro = cpython_cfunction_tp_getattro as *mut c_void;
        PyCFunction_Type.tp_descr_get = cpython_cfunction_tp_descr_get as *mut c_void;
        PyFunction_Type.tp_descr_get = cpython_function_tp_descr_get as *mut c_void;
        PyMethodDescr_Type.tp_call = cpython_method_descriptor_tp_call as *mut c_void;
        PyMethodDescr_Type.tp_descr_get = cpython_method_descriptor_tp_descr_get as *mut c_void;
        PyClassMethodDescr_Type.tp_call = cpython_method_descriptor_tp_call as *mut c_void;
        PyClassMethodDescr_Type.tp_descr_get =
            cpython_method_descriptor_tp_descr_get as *mut c_void;
        PyMemberDescr_Type.tp_getattro = PyObject_GenericGetAttr as *mut c_void;
        PyMemberDescr_Type.tp_descr_get = cpython_compat_descriptor_tp_descr_get as *mut c_void;
        PyMemberDescr_Type.tp_descr_set = cpython_compat_descriptor_tp_descr_set as *mut c_void;
        PyGetSetDescr_Type.tp_getattro = PyObject_GenericGetAttr as *mut c_void;
        PyGetSetDescr_Type.tp_descr_get = cpython_compat_descriptor_tp_descr_get as *mut c_void;
        PyGetSetDescr_Type.tp_descr_set = cpython_compat_descriptor_tp_descr_set as *mut c_void;
        PyMethod_Type.tp_call = cpython_method_tp_call as *mut c_void;
        PyFloat_Type.tp_new = cpython_float_tp_new as *mut c_void;
        PyUnicode_Type.tp_richcompare = PyUnicode_RichCompare as *mut c_void;
        PyTuple_Type.tp_richcompare = cpython_tuple_richcompare_slot as *mut c_void;

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
            std::ptr::addr_of_mut!(PyCoro_Type),
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
            std::ptr::addr_of_mut!(PyFrame_Type),
            std::ptr::addr_of_mut!(PyFloat_Type),
            std::ptr::addr_of_mut!(PyFrozenSet_Type),
            std::ptr::addr_of_mut!(PyFunction_Type),
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
        PyLong_Type.tp_as_number = std::ptr::addr_of_mut!(PY_LONG_NUMBER_METHODS).cast();
        PyLong_Type.tp_methods = std::ptr::addr_of_mut!(PY_LONG_TYPE_METHOD_DEFS).cast();
        PyBool_Type.tp_basicsize = 24;
        PyBool_Type.tp_itemsize = 4;
        PyBool_Type.tp_as_number = std::ptr::addr_of_mut!(PY_LONG_NUMBER_METHODS).cast();
        PyFloat_Type.tp_basicsize = 24;
        PyFloat_Type.tp_itemsize = 0;
        PyFloat_Type.tp_as_number = std::ptr::addr_of_mut!(PY_FLOAT_NUMBER_METHODS).cast();
        PyFloat_Type.tp_methods = std::ptr::addr_of_mut!(PY_FLOAT_TYPE_METHOD_DEFS).cast();
        PyFrame_Type.tp_basicsize = std::mem::size_of::<CpythonFrameCompatObject>() as isize;
        PyFrame_Type.tp_itemsize = 0;
        PyComplex_Type.tp_basicsize = std::mem::size_of::<CpythonComplexCompatObject>() as isize;
        PyComplex_Type.tp_itemsize = 0;
        PyBytes_Type.tp_basicsize = 33;
        PyBytes_Type.tp_itemsize = 1;
        PyUnicode_Type.tp_basicsize = 64;
        PyUnicode_Type.tp_itemsize = 0;
        PyTuple_Type.tp_basicsize = 32;
        PyTuple_Type.tp_itemsize = 8;
        PyList_Type.tp_basicsize = 40;
        PyList_Type.tp_itemsize = 0;
        PyList_Type.tp_as_sequence = std::ptr::addr_of_mut!(PY_LIST_SEQUENCE_METHODS).cast();
        PyList_Type.tp_as_mapping = std::ptr::addr_of_mut!(PY_LIST_MAPPING_METHODS).cast();
        PyDict_Type.tp_basicsize = 48;
        PyDict_Type.tp_itemsize = 0;
        PyDict_Type.tp_as_mapping = std::ptr::addr_of_mut!(PY_DICT_MAPPING_METHODS).cast();
        PySet_Type.tp_basicsize = 200;
        PySet_Type.tp_itemsize = 0;
        PySlice_Type.tp_basicsize = 40;
        PySlice_Type.tp_itemsize = 0;
        PyModule_Type.tp_basicsize = 56;
        PyModule_Type.tp_itemsize = 0;
        PyFunction_Type.tp_basicsize = 136;
        PyFunction_Type.tp_itemsize = 0;
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

        // Iterator-like builtin types must expose tp_iter/tp_iternext so native
        // extension code (including Cython-generated loops) can drive iteration
        // through slots without falling back to Python-level wrappers.
        let iter_self = PyObject_SelfIter as *mut c_void;
        let iter_next = PyIter_Next as *mut c_void;
        let iterator_types: &mut [*mut CpythonTypeObject] = &mut [
            std::ptr::addr_of_mut!(PyByteArrayIter_Type),
            std::ptr::addr_of_mut!(PyBytesIter_Type),
            std::ptr::addr_of_mut!(PyCallIter_Type),
            std::ptr::addr_of_mut!(PyDictIterItem_Type),
            std::ptr::addr_of_mut!(PyDictIterKey_Type),
            std::ptr::addr_of_mut!(PyDictIterValue_Type),
            std::ptr::addr_of_mut!(PyDictRevIterItem_Type),
            std::ptr::addr_of_mut!(PyDictRevIterKey_Type),
            std::ptr::addr_of_mut!(PyDictRevIterValue_Type),
            std::ptr::addr_of_mut!(PyFilter_Type),
            std::ptr::addr_of_mut!(PyGen_Type),
            std::ptr::addr_of_mut!(PyListIter_Type),
            std::ptr::addr_of_mut!(PyListRevIter_Type),
            std::ptr::addr_of_mut!(PyLongRangeIter_Type),
            std::ptr::addr_of_mut!(PyMap_Type),
            std::ptr::addr_of_mut!(PyRangeIter_Type),
            std::ptr::addr_of_mut!(PyReversed_Type),
            std::ptr::addr_of_mut!(PySeqIter_Type),
            std::ptr::addr_of_mut!(PySetIter_Type),
            std::ptr::addr_of_mut!(PyTupleIter_Type),
            std::ptr::addr_of_mut!(PyUnicodeIter_Type),
            std::ptr::addr_of_mut!(PyZip_Type),
        ];
        for ty in iterator_types {
            (**ty).tp_iter = iter_self;
            (**ty).tp_iternext = iter_next;
        }

        _Py_NoneStruct.ob_type = std::ptr::addr_of_mut!(PyNone_Type).cast();
        _Py_NotImplementedStruct.ob_type = std::ptr::addr_of_mut!(PyBaseObject_Type).cast();
        _Py_EllipsisObject.ob_type = std::ptr::addr_of_mut!(PyEllipsis_Type).cast();
        _Py_FalseStruct.ob_type = std::ptr::addr_of_mut!(PyBool_Type).cast();
        _Py_TrueStruct.ob_type = std::ptr::addr_of_mut!(PyBool_Type).cast();

        macro_rules! init_exception_symbol {
            ($symbol:ident, $name:literal) => {
                ensure_cpython_exception_symbol(std::ptr::addr_of_mut!($symbol), type_ptr, $name);
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

pub(super) fn cpython_active_context_is_set() -> bool {
    ACTIVE_CPYTHON_INIT_CONTEXT.with(|cell| !cell.get().is_null())
}

pub(super) fn cpython_clear_thread_error_indicator() {
    let state_ptr =
        CURRENT_THREAD_STATE_PTR.load(Ordering::SeqCst) as *mut CpythonThreadStateCompat;
    if state_ptr.is_null() {
        return;
    }
    // SAFETY: thread-state pointer comes from the runtime registry and is writable for the active
    // thread.
    unsafe {
        let state = &mut *state_ptr;
        state.exc_info = std::ptr::addr_of_mut!(state.exc_state);
        state.current_exception = std::ptr::null_mut();
        state.exc_state.exc_value = std::ptr::null_mut();
        state.exc_state.previous_item = std::ptr::null_mut();
    }
}

const CPYTHON_THREAD_STATE_COMPAT_SIZE: usize = 4096;

#[repr(C)]
struct CpythonErrStackItemCompat {
    exc_value: *mut c_void,
    previous_item: *mut CpythonErrStackItemCompat,
}

#[repr(C, align(16))]
struct CpythonThreadStateCompat {
    prev: *mut c_void,
    next: *mut c_void,
    interp: *mut c_void,
    // CPython 3.14: `current_exception` sits at offset 112.
    _pad_to_current_exception: [u8; 112 - (3 * std::mem::size_of::<*mut c_void>())],
    current_exception: *mut c_void,
    // CPython 3.14: `exc_info` sits at offset 120.
    exc_info: *mut CpythonErrStackItemCompat,
    // CPython 3.14: `exc_state` sits at offset 256.
    _pad_to_exc_state: [u8; 256 - (112 + (2 * std::mem::size_of::<*mut c_void>()))],
    exc_state: CpythonErrStackItemCompat,
    _bytes: [u8; CPYTHON_THREAD_STATE_COMPAT_SIZE
        - (256 + std::mem::size_of::<CpythonErrStackItemCompat>())],
}

static mut MAIN_THREAD_STATE_STORAGE: CpythonThreadStateCompat = CpythonThreadStateCompat {
    prev: std::ptr::null_mut(),
    next: std::ptr::null_mut(),
    interp: std::ptr::null_mut(),
    _pad_to_current_exception: [0; 112 - (3 * std::mem::size_of::<*mut c_void>())],
    current_exception: std::ptr::null_mut(),
    exc_info: std::ptr::null_mut(),
    _pad_to_exc_state: [0; 256 - (112 + (2 * std::mem::size_of::<*mut c_void>()))],
    exc_state: CpythonErrStackItemCompat {
        exc_value: std::ptr::null_mut(),
        previous_item: std::ptr::null_mut(),
    },
    _bytes: [0; CPYTHON_THREAD_STATE_COMPAT_SIZE
        - (256 + std::mem::size_of::<CpythonErrStackItemCompat>())],
};
static CURRENT_THREAD_STATE_PTR: AtomicUsize = AtomicUsize::new(0);
static CPYTHON_THREAD_RUNTIME_INITIALIZED: AtomicUsize = AtomicUsize::new(1);
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
static CPYTHON_PENDING_INTERRUPT_SIGNUM: AtomicI32 = AtomicI32::new(0);
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
static CPYTHON_TRACEMALLOC_TRACES: OnceLock<Mutex<HashMap<(usize, usize), usize>>> =
    OnceLock::new();
static CPYTHON_STRUCTSEQ_TYPE_REGISTRY: OnceLock<Mutex<HashMap<usize, CpythonStructSeqTypeInfo>>> =
    OnceLock::new();
static CPYTHON_HEAP_TYPE_REGISTRY: OnceLock<Mutex<HashMap<usize, CpythonHeapTypeInfo>>> =
    OnceLock::new();
static CPYTHON_INTERNED_UNICODE_REGISTRY: OnceLock<Mutex<CpythonInternedUnicodeRegistry>> =
    OnceLock::new();
static CPYTHON_STABLE_UTF8_REGISTRY: OnceLock<Mutex<HashMap<String, Box<[u8]>>>> = OnceLock::new();

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

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

/// Per-extension-call bridge context between raw C-API pointers and pyrs values.
///
/// The context owns temporary compat allocations and pointer/handle maps created
/// during extension calls. On drop it either frees or promotes allocations into
/// VM-pinned ownership depending on observed escape/pin state.
struct ModuleCapiContext {
    vm: *mut Vm,
    module: ObjRef,
    run_capsule_destructors_on_drop: bool,
    strict_capsule_refcount: bool,
    keep_cpython_allocations_on_drop: bool,
    suppress_vm_proxy_persistence: bool,
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
    buffer_internal_handles: HashMap<usize, PyrsObjectHandle>,
    cpython_objects_by_ptr: HashMap<usize, PyrsObjectHandle>,
    cpython_ptr_by_handle: HashMap<PyrsObjectHandle, *mut c_void>,
    cpython_object_handles_by_id: HashMap<u64, PyrsObjectHandle>,
    cpython_allocations: Vec<*mut CpythonCompatObject>,
    cpython_aux_allocations: Vec<*mut c_void>,
    cpython_known_type_ptrs: HashSet<usize>,
    cpython_descriptors: HashMap<usize, CpythonDescriptorKind>,
    cpython_cfunction_ptr_cache: HashMap<(usize, usize, usize, usize), *mut c_void>,
    cpython_builtin_cfunction_ptr_cache: HashMap<BuiltinFunction, *mut c_void>,
    cpython_builtin_method_defs: HashMap<BuiltinFunction, *mut CpythonMethodDef>,
    cpython_builtin_by_method_def: HashMap<usize, BuiltinFunction>,
    cpython_list_buffers: HashMap<PyrsObjectHandle, (*mut *mut c_void, usize)>,
    cpython_bytearray_buffers: HashMap<PyrsObjectHandle, (*mut c_char, usize)>,
    cpython_list_items_cache_by_handle: HashMap<PyrsObjectHandle, Vec<usize>>,
    cpython_tuple_items_cache_by_handle: HashMap<PyrsObjectHandle, Vec<usize>>,
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
    /// Tear down context-owned C-API allocations.
    ///
    /// This drop path is responsible for:
    /// - clearing thread-state exception pointers that still point into this context,
    /// - transferring escaped allocations into VM pinned ownership when required,
    /// - freeing non-escaped compat/aux allocations exactly once.
    fn drop(&mut self) {
        self.clear_thread_state_error_if_owned_by_context();
        self.codec_error_handlers.clear();
        for (internal_ptr, _) in self.buffer_internal_handles.drain() {
            if internal_ptr != 0 {
                // SAFETY: internal pointers are only inserted via `register_buffer_internal`
                // from `Box<CpythonBufferInternal>` allocations owned by this context.
                unsafe {
                    drop(Box::from_raw(internal_ptr as *mut CpythonBufferInternal));
                }
            }
        }
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
                    vm.capi_owned_ptr_is_pinned(ptr as usize)
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
        let mut seen_cpython_allocations: HashSet<usize> = HashSet::new();
        for raw in drained_cpython_allocations {
            const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
            let raw_addr = raw as usize;
            if !seen_cpython_allocations.insert(raw_addr) {
                if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                    eprintln!(
                        "[pin-free] context-skip duplicate compat ptr={:p}",
                        raw.cast::<c_void>()
                    );
                }
                continue;
            }
            if raw_addr < MIN_VALID_PTR || raw_addr % std::mem::align_of::<CpythonObjectHead>() != 0
            {
                if super::env_var_present_cached("PYRS_TRACE_CPY_DROP") {
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
                if vm.capi_pin_owned_ptr(raw as usize) {
                    vm.capi_registry_mark_alive(raw as usize);
                    if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                        eprintln!(
                            "[pin-free] pin-insert ptr={:p} reason=keep_cpython_allocations_on_drop",
                            raw.cast::<c_void>()
                        );
                    }
                }
                if let Some(handle) = self.cpython_objects_by_ptr.get(&(raw as usize)).copied() {
                    escaped_handles.insert(handle);
                    self.persist_escaped_ptr_value(vm, handle, raw as usize);
                }
                preserve_aux_allocations = true;
                continue;
            }
            let identity_wrapper_handle = self
                .cpython_objects_by_ptr
                .get(&(raw as usize))
                .copied()
                .filter(|handle| {
                    self.objects
                        .get(handle)
                        .is_some_and(|slot| self.should_keep_identity_cpython_wrapper(&slot.value))
                });
            let mut keep_pinned = false;
            let interned_unicode = cpython_is_interned_unicode_ptr(raw.cast::<c_void>());
            if !self.vm.is_null() {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *self.vm };
                keep_pinned = vm.capi_owned_ptr_is_pinned(raw as usize);
                if !keep_pinned && let Some(handle) = identity_wrapper_handle {
                    if vm.capi_pin_owned_ptr(raw as usize) {
                        vm.capi_registry_mark_alive(raw as usize);
                        if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                            eprintln!(
                                "[pin-free] pin-insert ptr={:p} reason=identity_wrapper",
                                raw.cast::<c_void>()
                            );
                        }
                    }
                    escaped_handles.insert(handle);
                    self.persist_escaped_ptr_value(vm, handle, raw as usize);
                    keep_pinned = true;
                }
                if !keep_pinned {
                    if interned_unicode {
                        if vm.capi_pin_owned_ptr(raw as usize) {
                            vm.capi_registry_mark_alive(raw as usize);
                            if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                                eprintln!(
                                    "[pin-free] pin-insert ptr={:p} reason=interned_unicode",
                                    raw.cast::<c_void>()
                                );
                            }
                        }
                        if let Some(handle) =
                            self.cpython_objects_by_ptr.get(&(raw as usize)).copied()
                        {
                            escaped_handles.insert(handle);
                            self.persist_escaped_ptr_value(vm, handle, raw as usize);
                        }
                        preserve_aux_allocations = true;
                        keep_pinned = true;
                    }
                }
                if !keep_pinned {
                    // If external/native code retained this object via Py_INCREF, transfer
                    // ownership from this call context into the VM-level pinned registry.
                    // SAFETY: `raw` points to a valid compat object header.
                    let refcount = unsafe { (*raw.cast::<CpythonObjectHead>()).ob_refcnt };
                    if refcount > 1 {
                        // SAFETY: writable compat object header.
                        unsafe {
                            (*raw.cast::<CpythonObjectHead>()).ob_refcnt = refcount - 1;
                        }
                        if vm.capi_pin_owned_ptr(raw as usize) {
                            vm.capi_registry_mark_alive(raw as usize);
                            if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                                eprintln!(
                                    "[pin-free] pin-insert ptr={:p} reason=refcount_escape",
                                    raw.cast::<c_void>()
                                );
                            }
                        }
                        if let Some(handle) =
                            self.cpython_objects_by_ptr.get(&(raw as usize)).copied()
                        {
                            escaped_handles.insert(handle);
                            self.persist_escaped_ptr_value(vm, handle, raw as usize);
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
            if !self.capi_owned_ptr_prepare_for_free(raw.cast()) {
                continue;
            }
            if !self.vm.is_null() {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *self.vm };
                if let Some(value) = vm.extension_cpython_ptr_value_remove(raw as usize)
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
            CPYTHON_DESCRIPTOR_REGISTRY.with(|registry| {
                registry.borrow_mut().remove(&(raw as usize));
            });
            self.cpython_known_type_ptrs.remove(&(raw as usize));
            self.cpython_descriptors.remove(&(raw as usize));
            // SAFETY: pointers were allocated via C allocator in this context.
            unsafe {
                free(raw.cast());
            }
            self.capi_owned_ptr_mark_freed(raw.cast(), "context-free compat");
        }
        let drained_list_buffers: Vec<(PyrsObjectHandle, (*mut *mut c_void, usize))> =
            self.cpython_list_buffers.drain().collect();
        let mut seen_list_buffers: HashSet<usize> = HashSet::new();
        for (handle, (buffer, _)) in drained_list_buffers {
            if buffer.is_null() {
                continue;
            }
            if !seen_list_buffers.insert(buffer as usize) {
                if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                    eprintln!(
                        "[pin-free] context-skip duplicate list-buffer ptr={:p}",
                        buffer.cast::<c_void>()
                    );
                }
                continue;
            }
            self.capi_registry_register_owned_ptr(buffer.cast(), None);
            let keep_pinned =
                if self.keep_cpython_allocations_on_drop || escaped_handles.contains(&handle) {
                    true
                } else if self.vm.is_null() {
                    false
                } else {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    vm.capi_owned_ptr_is_pinned(buffer as usize)
                };
            if keep_pinned {
                if !self.vm.is_null() {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    if vm.capi_pin_owned_ptr(buffer as usize) {
                        vm.capi_registry_mark_alive(buffer as usize);
                        if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                            eprintln!(
                                "[pin-free] pin-insert ptr={:p} reason=list_buffer_keep",
                                buffer.cast::<c_void>()
                            );
                        }
                    }
                }
                continue;
            }
            if !self.capi_owned_ptr_prepare_for_free(buffer.cast()) {
                continue;
            }
            // SAFETY: list item buffers were allocated through C allocator in this context.
            unsafe {
                free(buffer.cast());
            }
            self.capi_owned_ptr_mark_freed(buffer.cast(), "context-free list-buffer");
        }
        let drained_bytearray_buffers: Vec<(PyrsObjectHandle, (*mut c_char, usize))> =
            self.cpython_bytearray_buffers.drain().collect();
        let mut seen_bytearray_buffers: HashSet<usize> = HashSet::new();
        for (handle, (buffer, _)) in drained_bytearray_buffers {
            if buffer.is_null() {
                continue;
            }
            if !seen_bytearray_buffers.insert(buffer as usize) {
                if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                    eprintln!(
                        "[pin-free] context-skip duplicate bytearray-buffer ptr={:p}",
                        buffer
                    );
                }
                continue;
            }
            self.capi_registry_register_owned_ptr(buffer.cast(), None);
            let keep_pinned =
                if self.keep_cpython_allocations_on_drop || escaped_handles.contains(&handle) {
                    true
                } else if self.vm.is_null() {
                    false
                } else {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    vm.capi_owned_ptr_is_pinned(buffer as usize)
                };
            if keep_pinned {
                if !self.vm.is_null() {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    if vm.capi_pin_owned_ptr(buffer as usize) {
                        vm.capi_registry_mark_alive(buffer as usize);
                        if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                            eprintln!(
                                "[pin-free] pin-insert ptr={:p} reason=bytearray_buffer_keep",
                                buffer
                            );
                        }
                    }
                }
                continue;
            }
            if !self.capi_owned_ptr_prepare_for_free(buffer.cast()) {
                continue;
            }
            // SAFETY: bytearray payload buffers were allocated through C allocator.
            unsafe {
                free(buffer.cast());
            }
            self.capi_owned_ptr_mark_freed(buffer.cast(), "context-free bytearray-buffer");
        }
        let drained_aux_allocations: Vec<*mut c_void> =
            self.cpython_aux_allocations.drain(..).collect();
        let mut seen_aux_allocations: HashSet<usize> = HashSet::new();
        for raw in drained_aux_allocations {
            if raw.is_null() {
                continue;
            }
            if !seen_aux_allocations.insert(raw as usize) {
                if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                    eprintln!("[pin-free] context-skip duplicate aux ptr={:p}", raw);
                }
                continue;
            }
            self.capi_registry_register_owned_ptr(raw, None);
            let keep_pinned = if self.keep_cpython_allocations_on_drop || preserve_aux_allocations {
                true
            } else if self.vm.is_null() {
                false
            } else {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.capi_owned_ptr_is_pinned(raw as usize)
            };
            if keep_pinned {
                if !self.vm.is_null() {
                    // SAFETY: VM pointer is valid for context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    if vm.capi_pin_owned_ptr(raw as usize) {
                        vm.capi_registry_mark_alive(raw as usize);
                        if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                            eprintln!("[pin-free] pin-insert ptr={:p} reason=aux_keep", raw);
                        }
                    }
                }
                continue;
            }
            if !self.capi_owned_ptr_prepare_for_free(raw) {
                continue;
            }
            // SAFETY: auxiliary raw buffers were allocated via C allocator in this context.
            unsafe {
                free(raw);
            }
            self.capi_owned_ptr_mark_freed(raw, "context-free aux");
        }
    }
}

impl ModuleCapiContext {
    fn builtin_type_ptrs() -> [*mut c_void; 28] {
        [
            std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyBaseObject_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyLong_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyBool_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyFloat_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyComplex_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyByteArray_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyBytes_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyUnicode_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyTuple_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyList_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyDict_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PySet_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyFrozenSet_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PySlice_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyModule_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyMethod_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyCapsule_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyCFunction_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyMethodDescr_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyClassMethodDescr_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyGetSetDescr_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyMemberDescr_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyWrapperDescr_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyProperty_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyMemoryView_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PySuper_Type).cast::<c_void>(),
            std::ptr::addr_of_mut!(PyNone_Type).cast::<c_void>(),
        ]
    }

    fn is_probable_c_string_pointer(ptr: *const c_char) -> bool {
        const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
        if ptr.is_null() {
            return false;
        }
        let addr = ptr as usize;
        addr >= MIN_VALID_PTR
    }

    fn is_probable_type_object_without_metatype(object: *mut c_void) -> bool {
        const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
        if object.is_null() {
            return false;
        }
        let object_addr = object as usize;
        if object_addr < MIN_VALID_PTR
            || object_addr % std::mem::align_of::<CpythonTypeObject>() != 0
        {
            return false;
        }
        if Self::builtin_type_ptrs().contains(&object) {
            return true;
        }
        // SAFETY: type pointer is validated above.
        let Some(head) = (unsafe { object.cast::<CpythonObjectHead>().as_ref() }) else {
            return false;
        };
        if head.ob_refcnt == 0 {
            return false;
        }
        let metatype = head.ob_type.cast::<CpythonTypeObject>();
        if metatype.is_null() {
            return false;
        }
        let metatype_addr = metatype as usize;
        if metatype_addr < MIN_VALID_PTR
            || metatype_addr % std::mem::align_of::<CpythonTypeObject>() != 0
        {
            return false;
        }
        let py_type_type = std::ptr::addr_of_mut!(PyType_Type);
        let metatype_is_type_family = if metatype == py_type_type {
            true
        } else {
            // SAFETY: pointers are plausibility-checked above and only queried
            // via CPython subtype relation helper.
            unsafe {
                PyType_IsSubtype(metatype.cast::<c_void>(), py_type_type.cast::<c_void>()) != 0
            }
        };
        if !metatype_is_type_family {
            return false;
        }
        // SAFETY: `object` is now validated as an instance of `type` (or a
        // metatype subtype), so reading `tp_name` is structure-safe.
        let Some(type_obj) = (unsafe { object.cast::<CpythonTypeObject>().as_ref() }) else {
            return false;
        };
        if !Self::is_probable_c_string_pointer(type_obj.tp_name) {
            return false;
        }
        // SAFETY: metatype pointer is plausibility-checked above.
        unsafe {
            metatype
                .as_ref()
                .map(|meta| Self::is_probable_c_string_pointer(meta.tp_name))
                .unwrap_or(false)
        }
    }

    fn is_probable_type_object_ptr(object: *mut c_void) -> bool {
        const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
        if object.is_null() {
            return false;
        }
        if !Self::is_probable_type_object_without_metatype(object) {
            return false;
        }
        // SAFETY: best-effort probe of object header only.
        let object_type = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        let expected_type = std::ptr::addr_of_mut!(PyType_Type).cast::<CpythonTypeObject>();
        if object_type.is_null() {
            return true;
        }
        if object_type == expected_type {
            return true;
        }
        let type_addr = object_type as usize;
        if type_addr < MIN_VALID_PTR || type_addr % std::mem::align_of::<usize>() != 0 {
            return false;
        }
        if !Self::is_probable_type_object_without_metatype(object_type.cast::<c_void>()) {
            return false;
        }
        // SAFETY: `object_type` is non-null in this branch and used only for
        // type-subclass/probe checks.
        unsafe {
            ((*object_type).tp_flags & PY_TPFLAGS_TYPE_SUBCLASS) != 0
                || PyType_IsSubtype(object_type.cast::<c_void>(), expected_type.cast::<c_void>())
                    != 0
        }
    }

    fn is_probable_external_cpython_object_ptr(object: *mut c_void) -> bool {
        const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
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
            let tp_name = (*type_ptr).tp_name;
            if !Self::is_probable_c_string_pointer(tp_name) {
                return false;
            }
            if type_head.ob_refcnt == 0 {
                // Some transient Cython heap metatypes can be observed with refcnt=0 during
                // module init while still carrying valid type metadata and a real metatype.
                // Keep the gate strict by requiring a valid metatype chain to `type`.
                let metatype_ptr = type_head.ob_type.cast::<CpythonTypeObject>();
                if metatype_ptr.is_null() {
                    return false;
                }
                let metatype_addr = metatype_ptr as usize;
                if metatype_addr < MIN_VALID_PTR
                    || metatype_addr % std::mem::align_of::<usize>() != 0
                {
                    return false;
                }
                let metatype_tp_name = (*metatype_ptr).tp_name;
                if !Self::is_probable_c_string_pointer(metatype_tp_name) {
                    return false;
                }
                let py_type = std::ptr::addr_of_mut!(PyType_Type).cast::<CpythonTypeObject>();
                if metatype_ptr == py_type {
                    return true;
                }
                return ((*metatype_ptr).tp_flags & PY_TPFLAGS_TYPE_SUBCLASS) != 0
                    || PyType_IsSubtype(metatype_ptr.cast::<c_void>(), py_type.cast::<c_void>())
                        != 0;
            }
            true
        }
    }

    fn pin_owned_child_pointer_for_vm(&mut self, vm: &mut Vm, child_ptr: *mut c_void) {
        if child_ptr.is_null() || !self.owns_cpython_allocation_ptr(child_ptr) {
            return;
        }
        let mut newly_pinned = false;
        if vm.capi_pin_owned_ptr(child_ptr as usize) {
            vm.capi_registry_mark_alive(child_ptr as usize);
            newly_pinned = true;
        }
        if let Some(handle) = self
            .cpython_objects_by_ptr
            .get(&(child_ptr as usize))
            .copied()
            && let Some(value) = self.object_value(handle)
        {
            vm.extension_cpython_ptr_value_set(child_ptr as usize, &value);
            if let Some(object_id) = Self::identity_object_id(&value) {
                vm.extension_cpython_ptr_by_object_id
                    .insert(object_id, child_ptr as usize);
            }
        }
        if newly_pinned {
            self.pin_container_children_for_vm(vm, child_ptr.cast::<CpythonCompatObject>());
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
            // Keep the dynamic type object alive whenever an escaped compat object is pinned.
            // Without this, objects that survive across active C-API contexts can end up with a
            // dangling `ob_type` pointer once the owning context frees heap-type allocations.
            self.pin_owned_child_pointer_for_vm(vm, head_ref.ob_type);
            if cpython_is_type_object_ptr(raw.cast()) {
                let type_ptr = raw.cast::<CpythonTypeObject>();
                self.pin_owned_child_pointer_for_vm(vm, (*type_ptr).tp_base.cast::<c_void>());
                self.pin_owned_child_pointer_for_vm(vm, (*type_ptr).tp_dict);
                self.pin_owned_child_pointer_for_vm(vm, (*type_ptr).tp_bases);
                self.pin_owned_child_pointer_for_vm(vm, (*type_ptr).tp_mro);
                if ((*type_ptr).tp_flags & PY_TPFLAGS_HEAPTYPE) != 0 {
                    let heap_type = type_ptr.cast::<CpythonHeapTypeObject>();
                    self.pin_owned_child_pointer_for_vm(vm, (*heap_type).ht_module);
                }
            }
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
                return;
            }
            if head_ref.ob_type == std::ptr::addr_of_mut!(PyByteArray_Type).cast() {
                let bytearray_ptr = raw.cast::<CpythonByteArrayCompatObject>();
                self.pin_owned_child_pointer_for_vm(vm, (*bytearray_ptr).ob_bytes.cast());
            }
        }
    }

    /// Create a fresh C-API bridge context for one extension/module operation.
    fn new(vm: *mut Vm, module: ObjRef) -> Self {
        initialize_cpython_compat_type_objects();
        let mut known_type_ptrs = HashSet::new();
        for ptr in Self::builtin_type_ptrs() {
            known_type_ptrs.insert(ptr as usize);
        }
        Self {
            vm,
            module,
            run_capsule_destructors_on_drop: true,
            strict_capsule_refcount: true,
            keep_cpython_allocations_on_drop: false,
            suppress_vm_proxy_persistence: false,
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
            buffer_internal_handles: HashMap::new(),
            cpython_objects_by_ptr: HashMap::new(),
            cpython_ptr_by_handle: HashMap::new(),
            cpython_object_handles_by_id: HashMap::new(),
            cpython_allocations: Vec::new(),
            cpython_aux_allocations: Vec::new(),
            cpython_known_type_ptrs: known_type_ptrs,
            cpython_descriptors: HashMap::new(),
            cpython_cfunction_ptr_cache: HashMap::new(),
            cpython_builtin_cfunction_ptr_cache: HashMap::new(),
            cpython_builtin_method_defs: HashMap::new(),
            cpython_builtin_by_method_def: HashMap::new(),
            cpython_list_buffers: HashMap::new(),
            cpython_bytearray_buffers: HashMap::new(),
            cpython_list_items_cache_by_handle: HashMap::new(),
            cpython_tuple_items_cache_by_handle: HashMap::new(),
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
            Some(Value::Exception(err)) => err.message.unwrap_or(err.name),
            Some(Value::Instance(instance)) if self.vm.is_null() => "error".to_string(),
            Some(Value::Instance(instance)) => {
                // SAFETY: VM pointer is valid while CPython init context is active.
                let vm = unsafe { &mut *self.vm };
                if !cpython_is_exception_instance(self, &instance) {
                    if let Ok(text) = vm.call_builtin(
                        BuiltinFunction::Str,
                        vec![Value::Instance(instance.clone())],
                        HashMap::new(),
                    ) && let Value::Str(message) = text
                        && !message.is_empty()
                    {
                        return message;
                    }
                    return format!(
                        "{} object",
                        vm.value_type_name_for_error(&Value::Instance(instance))
                    );
                }
                vm.exception_message_for_instance(&instance)
                    .unwrap_or_else(|| {
                        vm.exception_class_name_for_instance(&instance)
                            .unwrap_or_else(|| "error".to_string())
                    })
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

    fn active_thread_state_ptr(&self) -> *mut CpythonThreadStateCompat {
        let raw = cpython_current_thread_state_ptr();
        if raw == 0 || !cpython_is_known_thread_state_ptr(raw) {
            return std::ptr::null_mut();
        }
        raw as *mut CpythonThreadStateCompat
    }

    fn clear_thread_state_error_if_owned_by_context(&self) {
        let state_ptr = self.active_thread_state_ptr();
        if state_ptr.is_null() {
            return;
        }
        // SAFETY: thread-state pointer comes from runtime registry and is writable for active
        // thread.
        let current_exception = unsafe { (*state_ptr).current_exception };
        if current_exception.is_null() {
            return;
        }
        let owned_compat = self.owns_cpython_allocation_ptr(current_exception);
        let owned_aux = self.cpython_aux_allocations.contains(&current_exception);
        let owned_list_buffer = self
            .cpython_list_buffers
            .values()
            .any(|(buffer, _)| buffer.cast::<c_void>() == current_exception);
        let owned_context_alloc = self
            .cpython_allocations
            .iter()
            .any(|raw| raw.cast::<c_void>() == current_exception);
        if !(owned_compat || owned_aux || owned_list_buffer || owned_context_alloc) {
            return;
        }
        // SAFETY: thread-state pointer is active/writable; clear stale exception indicator that
        // points into this context before owned allocations are released.
        unsafe {
            (*state_ptr).current_exception = std::ptr::null_mut();
            if !(*state_ptr).exc_info.is_null() {
                (*(*state_ptr).exc_info).exc_value = std::ptr::null_mut();
            }
            (*state_ptr).exc_state.exc_value = std::ptr::null_mut();
            (*state_ptr).exc_state.previous_item = std::ptr::null_mut();
        }
    }

    fn value_ptr_is_exception_instance(&mut self, value_ptr: *mut c_void) -> bool {
        if value_ptr.is_null() {
            return false;
        }
        match self.cpython_value_from_ptr_or_proxy(value_ptr) {
            Some(Value::Exception(_)) => true,
            Some(Value::Instance(instance_obj)) => {
                cpython_is_exception_instance(self, &instance_obj)
            }
            _ => false,
        }
    }

    fn sync_thread_state_exception_view_from_current_error(&mut self) {
        let state_ptr = self.active_thread_state_ptr();
        if state_ptr.is_null() {
            return;
        }
        // SAFETY: thread-state pointer comes from the runtime registry and is writable for the
        // active thread.
        unsafe {
            let state = &mut *state_ptr;
            state.exc_info = std::ptr::addr_of_mut!(state.exc_state);

            if let Some(current) = self.current_error {
                let mut thread_exception = std::ptr::null_mut();
                if !current.pvalue.is_null() {
                    let registry_known_live = if self.vm.is_null() {
                        false
                    } else {
                        // SAFETY: VM pointer is valid for active C-API context lifetime.
                        let vm = &*self.vm;
                        vm.capi_registry_contains_alive(current.pvalue as usize)
                    };
                    let owned_ptr = self.owns_cpython_allocation_ptr(current.pvalue);
                    let pointer_is_known = owned_ptr || registry_known_live;
                    let mapped_value = if pointer_is_known {
                        self.cpython_value_from_ptr(current.pvalue)
                            .or_else(|| self.cpython_value_from_ptr_or_proxy(current.pvalue))
                    } else {
                        None
                    };
                    let mapped_exception_instance =
                        mapped_value.as_ref().is_some_and(|value| match value {
                            Value::Exception(_) => true,
                            Value::Instance(instance_obj) => {
                                cpython_is_exception_instance(self, &instance_obj)
                            }
                            _ => false,
                        });
                    if super::env_var_present_cached("PYRS_TRACE_DEFAULT_RNG_ERRFLOW") {
                        let mapped_value_tag = mapped_value
                            .as_ref()
                            .map(cpython_value_debug_tag)
                            .unwrap_or_else(|| "<unmapped>".to_string());
                        eprintln!(
                            "[tstate-exc-sync] pvalue={:p} ptype={:p} mapped={} mapped_exc={} owned={} registry_live={} pointer_is_known={}",
                            current.pvalue,
                            current.ptype,
                            mapped_value_tag,
                            mapped_exception_instance,
                            owned_ptr,
                            registry_known_live,
                            pointer_is_known
                        );
                    }
                    if pointer_is_known && mapped_exception_instance {
                        thread_exception = current.pvalue;
                    } else {
                        if super::env_var_present_cached("PYRS_TRACE_DEFAULT_RNG_ERRFLOW") {
                            eprintln!(
                                "[tstate-exc-sync] dropping non-exception pvalue={:p} mapped_exception_instance={} owned={} registry_live={} pointer_is_known={} ptype={:p}",
                                current.pvalue,
                                mapped_exception_instance,
                                owned_ptr,
                                registry_known_live,
                                pointer_is_known,
                                current.ptype
                            );
                        }
                    }
                }
                state.current_exception = thread_exception;
                state.exc_state.exc_value = thread_exception;
                state.exc_state.previous_item = std::ptr::null_mut();
                return;
            }

            state.current_exception = std::ptr::null_mut();
            state.exc_state.exc_value = std::ptr::null_mut();
            state.exc_state.previous_item = std::ptr::null_mut();
        }
    }

    fn sync_current_error_from_thread_state(&mut self) {
        let state_ptr = self.active_thread_state_ptr();
        if state_ptr.is_null() {
            return;
        }
        // SAFETY: thread-state pointer comes from the runtime registry and is readable for the
        // active thread.
        let current_exception = unsafe { (*state_ptr).current_exception };
        if current_exception.is_null() {
            self.current_error = None;
            return;
        }

        let registry_known_live = if self.vm.is_null() {
            false
        } else {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &*self.vm };
            vm.capi_registry_contains_alive(current_exception as usize)
        };
        let owned_ptr = self.owns_cpython_allocation_ptr(current_exception);
        let pointer_is_known = registry_known_live || owned_ptr;
        if !pointer_is_known {
            // SAFETY: thread-state pointer is writable for active thread.
            unsafe {
                (*state_ptr).current_exception = std::ptr::null_mut();
                if !(*state_ptr).exc_info.is_null() {
                    (*(*state_ptr).exc_info).exc_value = std::ptr::null_mut();
                }
                (*state_ptr).exc_state.exc_value = std::ptr::null_mut();
                (*state_ptr).exc_state.previous_item = std::ptr::null_mut();
            }
            self.current_error = None;
            return;
        }

        let Some(exception_value) = self
            .cpython_value_from_ptr(current_exception)
            .or_else(|| self.cpython_value_from_ptr_or_proxy(current_exception))
        else {
            // SAFETY: thread-state pointer is writable for active thread.
            unsafe {
                (*state_ptr).current_exception = std::ptr::null_mut();
                if !(*state_ptr).exc_info.is_null() {
                    (*(*state_ptr).exc_info).exc_value = std::ptr::null_mut();
                }
                (*state_ptr).exc_state.exc_value = std::ptr::null_mut();
                (*state_ptr).exc_state.previous_item = std::ptr::null_mut();
            }
            self.current_error = None;
            return;
        };
        let value_is_exception = match &exception_value {
            Value::Exception(_) | Value::ExceptionType(_) => true,
            Value::Instance(instance_obj) => cpython_is_exception_instance(self, instance_obj),
            _ => false,
        };
        if !value_is_exception {
            // SAFETY: thread-state pointer is writable for active thread.
            unsafe {
                (*state_ptr).current_exception = std::ptr::null_mut();
                if !(*state_ptr).exc_info.is_null() {
                    (*(*state_ptr).exc_info).exc_value = std::ptr::null_mut();
                }
                (*state_ptr).exc_state.exc_value = std::ptr::null_mut();
                (*state_ptr).exc_state.previous_item = std::ptr::null_mut();
            }
            self.current_error = None;
            return;
        }

        let ptype = cpython_exception_type_ptr_for_value(self, &exception_value)
            .unwrap_or_else(|| cpython_exception_type_ptr(current_exception));
        let ptraceback = cpython_exception_traceback_ptr_for_value(self, &exception_value)
            .unwrap_or(std::ptr::null_mut());
        let next_state = CpythonErrorState {
            ptype,
            pvalue: current_exception,
            ptraceback,
        };
        if self.current_error.as_ref().is_some_and(|state| {
            state.ptype == next_state.ptype
                && state.pvalue == next_state.pvalue
                && state.ptraceback == next_state.ptraceback
        }) {
            return;
        }
        let message = self.error_message_from_ptr(current_exception);
        self.current_error = Some(next_state);
        self.set_error_message(message);
    }

    #[track_caller]
    /// Store CPython-style error state and synchronize thread-state views.
    ///
    /// `ptype` is only derived from `pvalue` when `pvalue` is exception-like;
    /// message payload objects must not replace the declared exception type.
    fn set_error_state(
        &mut self,
        ptype: *mut c_void,
        pvalue: *mut c_void,
        ptraceback: *mut c_void,
        message: String,
    ) {
        let mut ptype = ptype;
        let pvalue = pvalue;
        if super::env_var_present_cached("PYRS_TRACE_NONE_ERROR_TYPE") {
            let none_ptr = (&raw mut _Py_NoneStruct).cast::<c_void>();
            if ptype == none_ptr {
                let caller = std::panic::Location::caller();
                eprintln!(
                    "[cpy-err-none-type] set_error_state caller={}:{} pvalue={:p} msg={}",
                    caller.file(),
                    caller.line(),
                    pvalue,
                    message
                );
            }
        }
        if super::env_var_present_cached("PYRS_TRACE_CPY_SET_ERROR_STATE") {
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
        if super::env_var_present_cached("PYRS_TRACE_CPY_UFUNC_ERRORS")
            && message.contains("_UFunc")
        {
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
        // Only derive `ptype` from `pvalue` when `pvalue` is exception-like.
        // For APIs like PyErr_SetString/PyErr_SetObject(exc, "msg"), `pvalue`
        // is typically a message object and must not replace `ptype`.
        if !pvalue.is_null()
            && let Some(value) = self.cpython_value_from_ptr_or_proxy(pvalue)
        {
            let value_is_exception_like = match &value {
                Value::Exception(_) | Value::ExceptionType(_) => true,
                Value::Instance(instance_obj) => cpython_is_exception_instance(self, instance_obj),
                _ => false,
            };
            if value_is_exception_like {
                if let Some(derived) = cpython_exception_type_ptr_for_value(self, &value) {
                    ptype = derived;
                } else {
                    let derived = cpython_exception_type_ptr(pvalue);
                    if !derived.is_null() {
                        ptype = derived;
                    }
                }
            }
        }
        if !ptype.is_null() && self.value_ptr_is_exception_instance(ptype) {
            let derived = cpython_exception_type_ptr(ptype);
            if !derived.is_null() {
                ptype = derived;
            }
        }
        self.current_error = Some(CpythonErrorState {
            ptype,
            pvalue,
            ptraceback,
        });
        self.sync_thread_state_exception_view_from_current_error();
        self.set_error_message(message);
    }

    fn set_error_from_runtime_error(&mut self, err: RuntimeError) {
        let RuntimeError { message, exception } = err;
        let exception = exception.or_else(|| RuntimeError::new(message.clone()).exception);
        if let Some(exception_obj) = exception {
            let exception_name = exception_obj.name.clone();
            let exception_class_value = exception_obj
                .attrs
                .borrow()
                .get("__class__")
                .cloned()
                .and_then(|value| match value {
                    Value::Class(class) => Some(Value::Class(class)),
                    Value::ExceptionType(name) => Some(Value::ExceptionType(name)),
                    _ => None,
                });
            let ptype = exception_class_value
                .map(|value| self.alloc_cpython_ptr_for_value(value))
                .filter(|ptr| !ptr.is_null())
                .or_else(|| cpython_exception_ptr_for_name(&exception_name))
                .unwrap_or(std::ptr::null_mut());
            let pvalue = self.alloc_cpython_ptr_for_value(Value::Exception(exception_obj));
            if !pvalue.is_null() {
                let resolved_ptype = if ptype.is_null() {
                    cpython_exception_type_ptr(pvalue).cast()
                } else {
                    ptype
                };
                self.set_error_state(
                    if resolved_ptype.is_null() {
                        unsafe { PyExc_RuntimeError }
                    } else {
                        resolved_ptype
                    },
                    pvalue,
                    std::ptr::null_mut(),
                    message,
                );
                return;
            }
        }
        self.set_error(message);
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
        if super::env_var_present_cached("PYRS_TRACE_CPY_ERRORS") {
            let caller = std::panic::Location::caller();
            eprintln!(
                "[cpy-err] {} (at {}:{})",
                message,
                caller.file(),
                caller.line()
            );
        }
        if super::env_var_present_cached("PYRS_TRACE_PYARROW_ERROR") && message.contains("pyarrow")
        {
            let stack = if self.vm.is_null() {
                "<no-vm>".to_string()
            } else {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &*self.vm };
                vm.frames
                    .iter()
                    .rev()
                    .take(10)
                    .map(|frame| format!("{}@{}", frame.code.name, frame.code.filename))
                    .collect::<Vec<_>>()
                    .join(" <- ")
            };
            eprintln!(
                "[pyarrow-error] message={} ptype={:p} stack={}",
                message,
                self.current_error
                    .as_ref()
                    .map_or(std::ptr::null_mut(), |state| state.ptype),
                stack
            );
        }
        self.last_error = Some(message);
    }

    #[track_caller]
    fn set_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        let inferred_ptype = message
            .split_once(':')
            .and_then(|(head, _)| cpython_exception_ptr_for_name(head.trim()))
            .unwrap_or(unsafe { PyExc_RuntimeError });
        self.current_error = Some(CpythonErrorState {
            ptype: inferred_ptype,
            pvalue: std::ptr::null_mut(),
            ptraceback: std::ptr::null_mut(),
        });
        self.sync_thread_state_exception_view_from_current_error();
        self.set_error_message(message);
    }

    fn clear_error(&mut self) {
        self.current_error = None;
        self.sync_thread_state_exception_view_from_current_error();
        self.last_error = None;
        self.first_error = None;
    }

    fn runtime_error_from_current_error_state(&mut self, fallback: &str) -> Option<RuntimeError> {
        let state = self.current_error?;
        let mut ptype = state.ptype;
        let pvalue = state.pvalue;
        if ptype.is_null() && !pvalue.is_null() {
            let derived = cpython_exception_type_ptr(pvalue);
            if !derived.is_null() {
                ptype = derived;
            }
        }
        let message = if !pvalue.is_null() {
            self.error_message_from_ptr(pvalue)
        } else {
            self.last_error
                .clone()
                .or_else(|| self.first_error.clone())
                .unwrap_or_else(|| fallback.to_string())
        };
        if let Some(exception_name) = cpython_exception_class_name_from_ptr(ptype) {
            if message.is_empty() {
                return Some(RuntimeError::with_exception(exception_name, None));
            }
            return Some(RuntimeError::with_exception(exception_name, Some(message)));
        }
        if message.is_empty() {
            None
        } else {
            Some(RuntimeError::new(message))
        }
    }

    fn fetch_error_state(&mut self) -> CpythonErrorState {
        // Preserve errors set through context-local setters that may not have a
        // materialized exception instance pointer in thread-state storage.
        if self.current_error.is_none() {
            self.sync_current_error_from_thread_state();
        }
        let state = self.current_error.take().unwrap_or(CpythonErrorState {
            ptype: std::ptr::null_mut(),
            pvalue: std::ptr::null_mut(),
            ptraceback: std::ptr::null_mut(),
        });
        self.sync_thread_state_exception_view_from_current_error();
        self.last_error = None;
        self.first_error = None;
        state
    }

    fn restore_error_state(&mut self, state: CpythonErrorState) {
        if state.ptype.is_null() && state.pvalue.is_null() && state.ptraceback.is_null() {
            self.clear_error();
            return;
        }
        if super::env_var_present_cached("PYRS_TRACE_NONE_ERROR_TYPE") {
            let none_ptr = (&raw mut _Py_NoneStruct).cast::<c_void>();
            if state.ptype == none_ptr {
                eprintln!(
                    "[cpy-err-none-type] restore_error_state pvalue={:p} ptraceback={:p}",
                    state.pvalue, state.ptraceback
                );
            }
        }
        let message = self.error_message_from_ptr(state.pvalue);
        self.current_error = Some(state);
        self.sync_thread_state_exception_view_from_current_error();
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

    fn set_object_refcount(&mut self, handle: PyrsObjectHandle, refcount: usize) {
        if let Some(slot) = self.objects.get_mut(&handle) {
            slot.refcount = refcount;
        }
    }

    fn register_owned_compat_allocation(
        &mut self,
        handle: PyrsObjectHandle,
        raw: *mut CpythonCompatObject,
    ) {
        if raw.is_null() {
            return;
        }
        if super::env_var_present_cached("PYRS_TRACE_CPY_PTRS") {
            let tag = self
                .object_value(handle)
                .map(|value| cpython_value_debug_tag(&value))
                .unwrap_or_else(|| "<missing>".to_string());
            eprintln!(
                "[cpy-ptr] alloc handle={} ptr={:p} tag={}",
                handle,
                raw.cast::<c_void>(),
                tag
            );
        }
        if let Some(previous) = self.cpython_objects_by_ptr.insert(raw as usize, handle)
            && super::env_var_present_cached("PYRS_TRACE_CPY_PTRS")
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
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            let mut object_id = None;
            if let Some(value) = self.object_value(handle) {
                vm.extension_cpython_ptr_value_set(raw as usize, &value);
                if let Some(id) = Self::identity_object_id(&value) {
                    object_id = Some(id);
                    vm.extension_cpython_ptr_by_object_id
                        .insert(id, raw as usize);
                }
            }
            vm.capi_registry_register_ptr(raw as usize, CapiPtrProvenance::OwnedCompat, object_id);
        }
    }

    fn capi_registry_register_owned_ptr(&mut self, ptr: *mut c_void, object_id: Option<u64>) {
        if ptr.is_null() || self.vm.is_null() {
            return;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.capi_registry_register_ptr(ptr as usize, CapiPtrProvenance::OwnedCompat, object_id);
    }

    fn capi_registry_register_external_ptr(&mut self, ptr: *mut c_void, object_id: Option<u64>) {
        if ptr.is_null() || self.vm.is_null() {
            return;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.capi_registry_register_ptr(ptr as usize, CapiPtrProvenance::ExternalRef, object_id);
    }

    fn capi_registry_mark_pending_free_ptr(&mut self, ptr: *mut c_void) {
        if ptr.is_null() || self.vm.is_null() {
            return;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.capi_registry_mark_pending_free(ptr as usize);
    }

    fn capi_registry_should_free_now_ptr(&mut self, ptr: *mut c_void) -> bool {
        if ptr.is_null() || self.vm.is_null() {
            return true;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.capi_registry_should_free_now(ptr as usize)
    }

    fn capi_registry_mark_freed_ptr(&mut self, ptr: *mut c_void) {
        if ptr.is_null() || self.vm.is_null() {
            return;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.capi_registry_mark_freed(ptr as usize);
    }

    fn capi_owned_ptr_prepare_for_free(&mut self, ptr: *mut c_void) -> bool {
        if ptr.is_null() {
            return false;
        }
        self.capi_registry_mark_pending_free_ptr(ptr);
        if self.capi_registry_should_free_now_ptr(ptr) {
            return true;
        }
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            if vm.capi_pin_owned_ptr(ptr as usize) {
                vm.capi_registry_mark_alive(ptr as usize);
            }
        }
        false
    }

    fn capi_owned_ptr_mark_freed(&mut self, ptr: *mut c_void, trace_label: &str) {
        if ptr.is_null() {
            return;
        }
        self.clear_thread_state_exception_if_matches(ptr);
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for context lifetime.
            let vm = unsafe { &mut *self.vm };
            let was_pinned = vm.capi_unpin_owned_ptr(ptr as usize);
            vm.extension_pinned_capsule_names.remove(&(ptr as usize));
            vm.capi_registry_mark_freed(ptr as usize);
            if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                eprintln!(
                    "[pin-free] {} ptr={:p} was_pinned={}",
                    trace_label, ptr, was_pinned
                );
            }
        }
        self.capi_registry_mark_freed_ptr(ptr);
    }

    fn clear_thread_state_exception_if_matches(&mut self, ptr: *mut c_void) {
        if ptr.is_null() {
            return;
        }
        let state_ptr = self.active_thread_state_ptr();
        if state_ptr.is_null() {
            return;
        }
        // SAFETY: thread-state pointer comes from runtime registry and is writable for active thread.
        unsafe {
            let state = &mut *state_ptr;
            let mut cleared = false;
            if state.current_exception == ptr {
                state.current_exception = std::ptr::null_mut();
                cleared = true;
            }
            if !state.exc_info.is_null() && (*state.exc_info).exc_value == ptr {
                (*state.exc_info).exc_value = std::ptr::null_mut();
                cleared = true;
            }
            if state.exc_state.exc_value == ptr {
                state.exc_state.exc_value = std::ptr::null_mut();
                cleared = true;
            }
            if cleared {
                self.current_error = None;
            }
        }
    }

    fn capi_registry_pin_external_once_ptr(&mut self, ptr: *mut c_void) -> bool {
        if ptr.is_null() || self.vm.is_null() {
            return false;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.capi_registry_pin_external_once(ptr as usize)
    }

    fn class_special_method_mro_value(
        &self,
        class_attrs: &HashMap<String, Value>,
        class_bases: &[ObjRef],
        method_name: &str,
    ) -> Option<Value> {
        if let Some(value) = class_attrs.get(method_name) {
            return Some(value.clone());
        }
        let mut seen = HashSet::new();
        let mut pending = class_bases.to_vec();
        while let Some(base_obj) = pending.pop() {
            if !seen.insert(base_obj.id()) {
                continue;
            }
            let base_kind = base_obj.kind();
            let Object::Class(base_data) = &*base_kind else {
                continue;
            };
            if let Some(value) = base_data.attrs.get(method_name) {
                return Some(value.clone());
            }
            for base in base_data.bases.iter().rev() {
                pending.push(base.clone());
            }
        }
        None
    }

    fn class_supports_mapping_subscript_slot(
        &self,
        class_attrs: &HashMap<String, Value>,
        class_bases: &[ObjRef],
    ) -> bool {
        self.class_special_method_mro_value(class_attrs, class_bases, "__getitem__")
            .is_some_and(|value| !matches!(value, Value::None))
    }

    fn alloc_cpython_ptr_for_handle(&mut self, handle: PyrsObjectHandle) -> *mut c_void {
        if let Some(existing) = self.cpython_ptr_by_handle.get(&handle).copied() {
            if super::env_var_present_cached("PYRS_TRACE_CPY_PTRS") {
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
            mut ob_type,
            long_payload,
            str_payload,
            tuple_items,
            list_items,
            dict_len,
            bytes_payload,
            bytearray_payload,
            module_state,
            class_state,
            instance_class,
            bound_method_state,
            float_value,
            complex_value,
            exception_state,
        ) = match self.objects.get(&handle).map(|slot| {
            (
                slot.refcount.max(1) as isize,
                cpython_type_for_value(&slot.value),
                cpython_long_payload_from_value(&slot.value),
                match &slot.value {
                    Value::Str(text) => Some(text.clone()),
                    _ => None,
                },
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
                    _ => None,
                },
                match &slot.value {
                    Value::ByteArray(bytes_obj) => match &*bytes_obj.kind() {
                        Object::ByteArray(values) => Some(values.clone()),
                        _ => None,
                    },
                    _ => None,
                },
                match &slot.value {
                    Value::Module(module_obj) => Some(module_obj.clone()),
                    _ => None,
                },
                match &slot.value {
                    Value::Class(class_obj) => match &*class_obj.kind() {
                        Object::Class(class_data) => Some((
                            class_obj.clone(),
                            class_data.name.clone(),
                            class_data.attrs.clone(),
                            class_data.metaclass.clone(),
                            class_data.bases.clone(),
                        )),
                        _ => None,
                    },
                    _ => None,
                },
                match &slot.value {
                    Value::Instance(instance_obj) => match &*instance_obj.kind() {
                        Object::Instance(instance_data) => Some(instance_data.class.clone()),
                        _ => None,
                    },
                    _ => None,
                },
                match &slot.value {
                    Value::BoundMethod(bound_obj) => match &*bound_obj.kind() {
                        Object::BoundMethod(bound_method) => {
                            Some((bound_method.function.clone(), bound_method.receiver.clone()))
                        }
                        _ => None,
                    },
                    _ => None,
                },
                match &slot.value {
                    Value::Float(value) => Some(*value),
                    _ => None,
                },
                match &slot.value {
                    Value::Complex { real, imag } => Some(CpythonComplexValue {
                        real: *real,
                        imag: *imag,
                    }),
                    _ => None,
                },
                self.exception_compat_state_from_value(&slot.value),
            )
        }) {
            Some(state) => state,
            None if capsule_state.is_some() => (
                1,
                std::ptr::null_mut(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            None => {
                self.set_error(format!("invalid object handle {handle}"));
                return std::ptr::null_mut();
            }
        };
        if let Some(instance_class) = instance_class {
            let class_ptr = self.alloc_cpython_ptr_for_value(Value::Class(instance_class));
            if !class_ptr.is_null() {
                ob_type = class_ptr;
            }
        }
        let mut already_registered = false;
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
        } else if let Some((lv_tag, digits)) = long_payload.as_ref() {
            let storage_bytes = cpython_long_storage_bytes(digits.len());
            // SAFETY: allocate storage for CPython long-compatible header + digits.
            let raw_long = unsafe { malloc(storage_bytes) };
            if raw_long.is_null() {
                self.set_error("out of memory allocating CPython long compat object");
                return std::ptr::null_mut();
            }
            // SAFETY: `raw_long` is writable long-compatible storage.
            unsafe {
                let head = raw_long.cast::<CpythonObjectHead>();
                (*head).ob_refcnt = refcount;
                (*head).ob_type = ob_type;
                *cpython_long_lv_tag_ptr(raw_long) = *lv_tag;
                let digits_ptr = cpython_long_digits_ptr(raw_long);
                if digits.is_empty() {
                    *digits_ptr = 0;
                } else {
                    std::ptr::copy_nonoverlapping(digits.as_ptr(), digits_ptr, digits.len());
                }
            }
            raw_long.cast::<CpythonCompatObject>()
        } else if let Some(text) = str_payload.as_ref() {
            let precomputed_hash = cpython_unicode_precomputed_hash(text);
            if text.is_ascii() {
                let storage_bytes = std::mem::size_of::<CpythonAsciiUnicodeCompatObject>()
                    .saturating_add(text.len())
                    .saturating_add(1);
                // SAFETY: allocate storage for compact ASCII unicode header + payload.
                let raw_unicode =
                    unsafe { malloc(storage_bytes) }.cast::<CpythonAsciiUnicodeCompatObject>();
                if raw_unicode.is_null() {
                    self.set_error("out of memory allocating CPython unicode compat object");
                    return std::ptr::null_mut();
                }
                // SAFETY: initialize compact ASCII unicode header and payload.
                unsafe {
                    raw_unicode.write(CpythonAsciiUnicodeCompatObject {
                        ob_base: CpythonObjectHead {
                            ob_refcnt: refcount,
                            ob_type,
                        },
                        length: text.len() as isize,
                        hash: precomputed_hash,
                        state: cpython_unicode_state(1, true, true),
                    });
                    let data = raw_unicode
                        .cast::<u8>()
                        .add(std::mem::size_of::<CpythonAsciiUnicodeCompatObject>());
                    if !text.is_empty() {
                        std::ptr::copy_nonoverlapping(text.as_ptr(), data, text.len());
                    }
                    *data.add(text.len()) = 0;
                }
                raw_unicode.cast::<CpythonCompatObject>()
            } else {
                let codepoints = text.chars().map(|ch| ch as u32).collect::<Vec<_>>();
                let storage_bytes = std::mem::size_of::<CpythonCompactUnicodeCompatObject>()
                    .saturating_add(
                        codepoints
                            .len()
                            .saturating_add(1)
                            .saturating_mul(std::mem::size_of::<u32>()),
                    );
                // SAFETY: allocate storage for compact non-ASCII unicode header + UCS4 payload.
                let raw_unicode =
                    unsafe { malloc(storage_bytes) }.cast::<CpythonCompactUnicodeCompatObject>();
                if raw_unicode.is_null() {
                    self.set_error("out of memory allocating CPython unicode compat object");
                    return std::ptr::null_mut();
                }
                // SAFETY: initialize compact unicode header and canonical payload.
                unsafe {
                    raw_unicode.write(CpythonCompactUnicodeCompatObject {
                        ob_base: CpythonAsciiUnicodeCompatObject {
                            ob_base: CpythonObjectHead {
                                ob_refcnt: refcount,
                                ob_type,
                            },
                            length: codepoints.len() as isize,
                            hash: precomputed_hash,
                            state: cpython_unicode_state(4, true, false),
                        },
                        utf8_length: 0,
                        utf8: std::ptr::null_mut(),
                    });
                    let data = raw_unicode
                        .cast::<u8>()
                        .add(std::mem::size_of::<CpythonCompactUnicodeCompatObject>())
                        .cast::<u32>();
                    if !codepoints.is_empty() {
                        std::ptr::copy_nonoverlapping(codepoints.as_ptr(), data, codepoints.len());
                    }
                    *data.add(codepoints.len()) = 0;
                }
                raw_unicode.cast::<CpythonCompatObject>()
            }
        } else if let Some(value) = complex_value {
            // SAFETY: allocate storage for CPython complex-compatible header.
            let raw_complex = unsafe { malloc(std::mem::size_of::<CpythonComplexCompatObject>()) }
                .cast::<CpythonComplexCompatObject>();
            if raw_complex.is_null() {
                self.set_error("out of memory allocating CPython complex compat object");
                return std::ptr::null_mut();
            }
            // SAFETY: initialize complex header and payload fields.
            unsafe {
                raw_complex.write(CpythonComplexCompatObject {
                    ob_base: CpythonObjectHead {
                        ob_refcnt: refcount,
                        ob_type,
                    },
                    cval: value,
                });
            }
            raw_complex.cast::<CpythonCompatObject>()
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
        } else if let Some(exception_state) = exception_state.as_ref() {
            // SAFETY: allocate storage for CPython base-exception-compatible header.
            let raw_exception =
                unsafe { malloc(std::mem::size_of::<CpythonBaseExceptionCompatObject>()) }
                    .cast::<CpythonBaseExceptionCompatObject>();
            if raw_exception.is_null() {
                self.set_error("out of memory allocating CPython exception compat object");
                return std::ptr::null_mut();
            }
            let args_ptr = self.exception_args_tuple_ptr_from_state(exception_state.args.clone());
            let notes_ptr =
                self.exception_optional_ptr_from_state_value(exception_state.notes.clone());
            let traceback_ptr =
                self.exception_optional_ptr_from_state_value(exception_state.traceback.clone());
            // SAFETY: initialize base-exception header and payload fields.
            unsafe {
                raw_exception.write(CpythonBaseExceptionCompatObject {
                    ob_base: CpythonObjectHead {
                        ob_refcnt: refcount,
                        ob_type,
                    },
                    dict: std::ptr::null_mut(),
                    args: args_ptr,
                    notes: notes_ptr,
                    traceback: traceback_ptr,
                    context: std::ptr::null_mut(),
                    cause: std::ptr::null_mut(),
                    suppress_context: if exception_state.suppress_context {
                        1
                    } else {
                        0
                    },
                    _padding: [0; 7],
                });
            }
            raw_exception.cast::<CpythonCompatObject>()
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
            self.capi_registry_register_owned_ptr(keys_stub, None);
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
            if !buffer_ptr.is_null() {
                self.capi_registry_register_owned_ptr(buffer_ptr.cast(), None);
            }
            raw_list.cast::<CpythonCompatObject>()
        } else if let Some(bytes) = bytearray_payload.as_ref() {
            let mut capacity = bytes.len().saturating_add(1);
            if capacity == 0 {
                capacity = 1;
            }
            // SAFETY: allocate storage for CPython bytearray-compatible header.
            let raw_bytearray =
                unsafe { malloc(std::mem::size_of::<CpythonByteArrayCompatObject>()) }
                    .cast::<CpythonByteArrayCompatObject>();
            if raw_bytearray.is_null() {
                self.set_error("out of memory allocating CPython bytearray compat object");
                return std::ptr::null_mut();
            }
            // SAFETY: allocate mutable bytearray payload buffer.
            let buffer = unsafe { malloc(capacity) }.cast::<c_char>();
            if buffer.is_null() {
                // SAFETY: `raw_bytearray` was allocated above and is owned here.
                unsafe {
                    free(raw_bytearray.cast());
                }
                self.set_error("out of memory allocating CPython bytearray payload");
                return std::ptr::null_mut();
            }
            // SAFETY: initialize bytearray header and payload.
            unsafe {
                raw_bytearray.write(CpythonByteArrayCompatObject {
                    ob_base: CpythonVarObjectHead {
                        ob_base: CpythonObjectHead {
                            ob_refcnt: refcount,
                            ob_type,
                        },
                        ob_size: bytes.len() as isize,
                    },
                    ob_alloc: capacity as isize,
                    ob_bytes: buffer,
                    ob_start: buffer,
                    ob_exports: 0,
                });
                if !bytes.is_empty() {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.cast::<u8>(), bytes.len());
                }
                *buffer.add(bytes.len()) = 0;
            }
            self.cpython_bytearray_buffers
                .insert(handle, (buffer, capacity));
            self.capi_registry_register_owned_ptr(buffer.cast(), None);
            raw_bytearray.cast::<CpythonCompatObject>()
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
        } else if let Some(module_obj) = module_state.as_ref() {
            // SAFETY: allocate storage for CPython module-compatible header.
            let raw_module = unsafe { malloc(std::mem::size_of::<CpythonModuleCompatObject>()) }
                .cast::<CpythonModuleCompatObject>();
            if raw_module.is_null() {
                self.set_error("out of memory allocating CPython module compat object");
                return std::ptr::null_mut();
            }
            // SAFETY: initialize a minimal module header before recursive field sync so
            // self-referential module dict entries can resolve this handle without
            // re-entering allocation infinitely.
            unsafe {
                raw_module.write(CpythonModuleCompatObject {
                    ob_base: CpythonObjectHead {
                        ob_refcnt: refcount.max(1),
                        ob_type: std::ptr::addr_of_mut!(PyModule_Type).cast(),
                    },
                    md_dict: std::ptr::null_mut(),
                    md_def: std::ptr::null_mut(),
                    md_state: std::ptr::null_mut(),
                    md_weaklist: std::ptr::null_mut(),
                    md_name: std::ptr::null_mut(),
                });
            }
            self.register_owned_compat_allocation(handle, raw_module.cast());
            already_registered = true;
            self.sync_module_compat_from_value(raw_module, module_obj, refcount);
            raw_module.cast::<CpythonCompatObject>()
        } else if let Some((class_obj, class_name, class_attrs, class_metaclass, class_bases)) =
            class_state
        {
            // SAFETY: allocate storage for CPython type-compatible header.
            let raw_type = unsafe { malloc(std::mem::size_of::<CpythonTypeObject>()) }
                .cast::<CpythonTypeObject>();
            if raw_type.is_null() {
                self.set_error("out of memory allocating CPython type compat object");
                return std::ptr::null_mut();
            }
            let name_ptr = match self.alloc_owned_c_string_for_capi(&class_name) {
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
            let ob_type_ptr = class_metaclass
                .and_then(|meta| {
                    let ptr = self.alloc_cpython_ptr_for_value(Value::Class(meta));
                    (!ptr.is_null()).then_some(ptr)
                })
                .unwrap_or_else(|| std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>());
            let tp_base_ptr = class_bases
                .first()
                .and_then(|base| {
                    let ptr = self.alloc_cpython_ptr_for_value(Value::Class(base.clone()));
                    (!ptr.is_null()).then_some(ptr.cast::<CpythonTypeObject>())
                })
                .unwrap_or_else(|| std::ptr::addr_of_mut!(PyBaseObject_Type));
            let needs_native_base_inheritance = class_bases.iter().any(|base| {
                Self::cpython_proxy_raw_ptr_from_value(&Value::Class(base.clone())).is_some()
            });
            let native_base_inheritance_enabled = needs_native_base_inheritance
                && !super::env_var_present_cached("PYRS_DISABLE_NATIVE_BASE_READY");
            let supports_mapping_subscript =
                self.class_supports_mapping_subscript_slot(&class_attrs, &class_bases);
            let exception_subclass_flag = if self.vm.is_null() {
                0
            } else {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &mut *self.vm };
                if vm.exception_inherits(&class_name, "BaseException") {
                    PY_TPFLAGS_BASE_EXC_SUBCLASS
                } else {
                    0
                }
            };
            let tp_basicsize = {
                let inherited_base = if tp_base_ptr.is_null() {
                    std::mem::size_of::<CpythonObjectHead>() as isize
                } else {
                    // SAFETY: `tp_base_ptr` is either a static builtin type or a
                    // compat type object materialized in this context.
                    unsafe { (*tp_base_ptr).tp_basicsize }
                };
                // For heap classes, never under-allocate compared to the base
                // type. This is critical for extension-backed subclasses such as
                // `numpy.ma.MaskedArray` (base: `numpy.ndarray`).
                let default =
                    inherited_base.max(std::mem::size_of::<CpythonCompatObject>() as isize);
                let module_name = class_attrs.get("__module__").and_then(|value| match value {
                    Value::Str(name) => Some(name.as_str()),
                    _ => None,
                });
                match (module_name, class_name.as_str()) {
                    (Some("datetime"), "date") => 32,
                    (Some("datetime"), "datetime") => 48,
                    (Some("datetime"), "time") => 40,
                    (Some("datetime"), "timedelta") => 40,
                    (Some("datetime"), "tzinfo") => 16,
                    (Some("datetime"), "timezone") => 32,
                    _ => default,
                }
            };
            let type_subclass_flag = if tp_base_ptr == std::ptr::addr_of_mut!(PyType_Type) {
                PY_TPFLAGS_TYPE_SUBCLASS
            } else {
                0
            };
            // SAFETY: `raw_type` points to writable CpythonTypeObject storage.
            unsafe {
                raw_type.write(CpythonTypeObject {
                    ob_refcnt: refcount,
                    ob_type: ob_type_ptr,
                    ob_size: 0,
                    tp_name: name_ptr,
                    tp_basicsize,
                    tp_itemsize: 0,
                    tp_dealloc: std::ptr::null_mut(),
                    tp_vectorcall_offset: 0,
                    tp_getattr: std::ptr::null_mut(),
                    tp_setattr: std::ptr::null_mut(),
                    tp_as_async: std::ptr::null_mut(),
                    tp_repr: std::ptr::null_mut(),
                    tp_as_number: std::ptr::null_mut(),
                    tp_as_sequence: std::ptr::null_mut(),
                    tp_as_mapping: if supports_mapping_subscript {
                        std::ptr::addr_of_mut!(PY_RUNTIME_MAPPING_METHODS).cast::<c_void>()
                    } else {
                        std::ptr::null_mut()
                    },
                    tp_hash: std::ptr::null_mut(),
                    // Class objects are instances of `type`; constructor-call behavior is
                    // driven by `type.tp_call` on the metatype, not by overriding this slot
                    // on each class object.
                    tp_call: std::ptr::null_mut(),
                    tp_str: std::ptr::null_mut(),
                    // Native-proxy subclasses inherit slot behavior from C bases through
                    // PyType_Ready; regular runtime classes keep the generic object protocol.
                    tp_getattro: if native_base_inheritance_enabled {
                        std::ptr::null_mut()
                    } else {
                        PyObject_GenericGetAttr as *mut c_void
                    },
                    tp_setattro: if native_base_inheritance_enabled {
                        std::ptr::null_mut()
                    } else {
                        PyObject_GenericSetAttr as *mut c_void
                    },
                    tp_as_buffer: std::ptr::null_mut(),
                    tp_flags: PY_TPFLAGS_HEAPTYPE
                        | PY_TPFLAGS_BASETYPE
                        | type_subclass_flag
                        | exception_subclass_flag
                        | if native_base_inheritance_enabled {
                            0
                        } else {
                            PY_TPFLAGS_READY
                        },
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
                    tp_alloc: if native_base_inheritance_enabled {
                        std::ptr::null_mut()
                    } else {
                        PyType_GenericAlloc as *mut c_void
                    },
                    tp_new: if native_base_inheritance_enabled {
                        std::ptr::null_mut()
                    } else {
                        PyType_GenericNew as *mut c_void
                    },
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
                (*raw_type).tp_base = tp_base_ptr;
            }
            if native_base_inheritance_enabled
                && unsafe { PyType_Ready(raw_type.cast::<c_void>()) } != 0
            {
                // SAFETY: `raw_type` is uniquely owned in this failure path.
                unsafe {
                    free(raw_type.cast());
                }
                return std::ptr::null_mut();
            }
            if super::env_var_present_cached("PYRS_TRACE_TYPED_CACHE_SUBSCRIPT")
                && class_name.contains("TypedCache")
            {
                eprintln!(
                    "[typed-cache-subscript] class name={} id={} mapped_getitem_slot={}",
                    class_name,
                    class_obj.id(),
                    supports_mapping_subscript
                );
            }
            raw_type.cast::<CpythonCompatObject>()
        } else if let Some((function_obj, receiver_obj)) = bound_method_state.as_ref() {
            let function_ptr =
                if let Some(function_value) = Self::value_from_objref_for_capi(function_obj) {
                    self.alloc_cpython_ptr_for_value(function_value)
                } else {
                    std::ptr::addr_of_mut!(_Py_NoneStruct).cast::<c_void>()
                };
            if function_ptr.is_null() {
                self.set_error("failed to materialize bound-method function pointer");
                return std::ptr::null_mut();
            }
            let receiver_ptr =
                if let Some(receiver_value) = Self::value_from_objref_for_capi(receiver_obj) {
                    self.alloc_cpython_ptr_for_value(receiver_value)
                } else {
                    std::ptr::addr_of_mut!(_Py_NoneStruct).cast::<c_void>()
                };
            if receiver_ptr.is_null() {
                self.set_error("failed to materialize bound-method receiver pointer");
                return std::ptr::null_mut();
            }
            // SAFETY: allocate storage for CPython method-compatible header.
            let raw_method = unsafe { malloc(std::mem::size_of::<CpythonMethodCompatObject>()) }
                .cast::<CpythonMethodCompatObject>();
            if raw_method.is_null() {
                self.set_error("out of memory allocating CPython method compat object");
                return std::ptr::null_mut();
            }
            // SAFETY: initialize method header and payload fields.
            unsafe {
                raw_method.write(CpythonMethodCompatObject {
                    ob_base: CpythonObjectHead {
                        ob_refcnt: refcount,
                        ob_type: std::ptr::addr_of_mut!(PyMethod_Type).cast(),
                    },
                    im_func: function_ptr,
                    im_self: receiver_ptr,
                    im_weakreflist: std::ptr::null_mut(),
                    vectorcall: std::ptr::null_mut(),
                });
            }
            if super::env_var_present_cached("PYRS_TRACE_BOUND_METHOD_PTR") {
                eprintln!(
                    "[bound-method-ptr] alloc handle={} method_ptr={:p} im_func={:p} im_self={:p}",
                    handle,
                    raw_method.cast::<c_void>(),
                    function_ptr,
                    receiver_ptr
                );
            }
            raw_method.cast::<CpythonCompatObject>()
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
        if !already_registered {
            self.register_owned_compat_allocation(handle, raw);
        }
        raw.cast()
    }

    fn value_from_objref_for_capi(obj: &ObjRef) -> Option<Value> {
        match &*obj.kind() {
            Object::List(_) => Some(Value::List(obj.clone())),
            Object::Tuple(_) => Some(Value::Tuple(obj.clone())),
            Object::Dict(_) => Some(Value::Dict(obj.clone())),
            Object::Set(_) => Some(Value::Set(obj.clone())),
            Object::FrozenSet(_) => Some(Value::FrozenSet(obj.clone())),
            Object::Bytes(_) => Some(Value::Bytes(obj.clone())),
            Object::ByteArray(_) => Some(Value::ByteArray(obj.clone())),
            Object::MemoryView(_) => Some(Value::MemoryView(obj.clone())),
            Object::Iterator(_) => Some(Value::Iterator(obj.clone())),
            Object::Generator(_) => Some(Value::Generator(obj.clone())),
            Object::Module(_) => Some(Value::Module(obj.clone())),
            Object::Class(_) => Some(Value::Class(obj.clone())),
            Object::Instance(_) => Some(Value::Instance(obj.clone())),
            Object::Super(_) => Some(Value::Super(obj.clone())),
            Object::BoundMethod(_) => Some(Value::BoundMethod(obj.clone())),
            Object::Function(_) => Some(Value::Function(obj.clone())),
            Object::Cell(_) => Some(Value::Cell(obj.clone())),
            Object::DictView(view) => Some(match view.kind {
                DictViewKind::Keys => Value::DictKeys(obj.clone()),
                DictViewKind::Values => Value::DictValues(obj.clone()),
                DictViewKind::Items => Value::DictItems(obj.clone()),
            }),
            Object::NativeMethod(_) => None,
        }
    }

    pub(super) fn cpython_proxy_raw_ptr_from_value(value: &Value) -> Option<*mut c_void> {
        match value {
            Value::Class(class_obj) => {
                let Object::Class(class_data) = &*class_obj.kind() else {
                    return None;
                };
                match class_data.attrs.get(CPY_PROXY_PTR_ATTR) {
                    Some(Value::Int(raw_ptr)) if *raw_ptr >= 0 => {
                        Some(*raw_ptr as usize as *mut c_void)
                    }
                    _ => {
                        if super::env_var_present_cached("PYRS_TRACE_PROXY_PTR_MISS") {
                            let mut keys = class_data.attrs.keys().cloned().collect::<Vec<_>>();
                            keys.sort();
                            eprintln!(
                                "[proxy-ptr-miss] kind=class class={} attrs={keys:?}",
                                class_data.name
                            );
                        }
                        None
                    }
                }
            }
            Value::Instance(instance_obj) => {
                let Object::Instance(instance_data) = &*instance_obj.kind() else {
                    return None;
                };
                match instance_data.attrs.get(CPY_PROXY_PTR_ATTR) {
                    Some(Value::Int(raw_ptr)) if *raw_ptr >= 0 => {
                        Some(*raw_ptr as usize as *mut c_void)
                    }
                    _ => {
                        if super::env_var_present_cached("PYRS_TRACE_PROXY_PTR_MISS") {
                            let class_name = match &*instance_data.class.kind() {
                                Object::Class(class_data) => class_data.name.clone(),
                                _ => "<non-class>".to_string(),
                            };
                            let mut keys = instance_data.attrs.keys().cloned().collect::<Vec<_>>();
                            keys.sort();
                            eprintln!(
                                "[proxy-ptr-miss] kind=instance class={} attrs={keys:?}",
                                class_name
                            );
                        }
                        None
                    }
                }
            }
            _ => None,
        }
    }

    fn value_is_exception_instance_like(&self, value: &Value) -> bool {
        match value {
            Value::Exception(_) => true,
            Value::Instance(instance_obj) => {
                if self.vm.is_null() {
                    return false;
                }
                let Object::Instance(instance_data) = &*instance_obj.kind() else {
                    return false;
                };
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &*self.vm };
                vm.class_is_exception_class(&instance_data.class)
            }
            _ => false,
        }
    }

    fn exception_compat_state_from_value(
        &self,
        value: &Value,
    ) -> Option<CpythonExceptionCompatState> {
        match value {
            Value::Exception(exception_obj) => {
                let attrs = exception_obj.attrs.borrow();
                let args = attrs
                    .get("args")
                    .cloned()
                    .or_else(|| exception_obj.message.clone().map(Value::Str));
                let notes = attrs.get("__notes__").cloned();
                let traceback = attrs
                    .get("__traceback__")
                    .cloned()
                    .or_else(|| attrs.get("exc_traceback").cloned());
                let suppress_context = exception_obj.suppress_context
                    || matches!(attrs.get("__suppress_context__"), Some(Value::Bool(true)));
                Some(CpythonExceptionCompatState {
                    args,
                    notes,
                    traceback,
                    suppress_context,
                })
            }
            Value::Instance(instance_obj) => {
                if self.vm.is_null() {
                    return None;
                }
                let Object::Instance(instance_data) = &*instance_obj.kind() else {
                    return None;
                };
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &*self.vm };
                if !vm.class_is_exception_class(&instance_data.class) {
                    return None;
                }
                let args = instance_data.attrs.get("args").cloned();
                let notes = instance_data.attrs.get("__notes__").cloned();
                let traceback = instance_data
                    .attrs
                    .get("__traceback__")
                    .cloned()
                    .or_else(|| instance_data.attrs.get("exc_traceback").cloned());
                let suppress_context = matches!(
                    instance_data.attrs.get("__suppress_context__"),
                    Some(Value::Bool(true))
                );
                Some(CpythonExceptionCompatState {
                    args,
                    notes,
                    traceback,
                    suppress_context,
                })
            }
            _ => None,
        }
    }

    fn exception_args_tuple_ptr_from_state(&mut self, args: Option<Value>) -> *mut c_void {
        let tuple_value = match args {
            Some(value @ Value::Tuple(_)) => value,
            Some(Value::None) | None => {
                if self.vm.is_null() {
                    return std::ptr::null_mut();
                }
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.heap.alloc_tuple(Vec::new())
            }
            Some(value) => {
                if self.vm.is_null() {
                    return std::ptr::null_mut();
                }
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.heap.alloc_tuple(vec![value])
            }
        };
        self.alloc_cpython_ptr_for_value(tuple_value)
    }

    fn exception_optional_ptr_from_state_value(&mut self, value: Option<Value>) -> *mut c_void {
        match value {
            Some(Value::None) | None => std::ptr::null_mut(),
            Some(value) => self.alloc_cpython_ptr_for_value(value),
        }
    }

    /// Convert a runtime `Value` into a CPython-facing pointer.
    ///
    /// Returns existing pinned/proxy pointers when possible; otherwise allocates
    /// context-owned compat storage and records it for drop-time lifecycle handling.
    fn alloc_cpython_ptr_for_value(&mut self, value: Value) -> *mut c_void {
        let trace_bound_ptr = super::env_var_present_cached("PYRS_TRACE_BOUND_METHOD_PTR");
        let is_bound_method = matches!(value, Value::BoundMethod(_));
        if let Value::Module(module_obj) = &value
            && let Object::Module(module_data) = &*module_obj.kind()
            && (module_data.name == "__classmethod__" || module_data.name == "__staticmethod__")
            && let Some(wrapped) = module_data.globals.get("__func__").cloned()
        {
            return self.alloc_cpython_ptr_for_value(wrapped);
        }
        if let Value::ExceptionType(name) = &value {
            if let Some(ptr) = cpython_exception_ptr_for_name(name) {
                return ptr;
            }
            if !self.vm.is_null() {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &mut *self.vm };
                let class = vm.alloc_synthetic_exception_class(name);
                return self.alloc_cpython_ptr_for_value(Value::Class(class));
            }
        }
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
                if super::env_var_present_cached("PYRS_TRACE_CPY_CFUNCTION_WRAP") {
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
                if trace_bound_ptr && is_bound_method {
                    eprintln!(
                        "[bound-method-ptr] object_id={} cached_ptr={:p} pinned={} has_value={}",
                        object_id,
                        raw_ptr as *mut c_void,
                        vm.capi_owned_ptr_is_pinned(raw_ptr),
                        vm.extension_cpython_ptr_contains_live(raw_ptr)
                    );
                }
                if vm.capi_owned_ptr_is_pinned(raw_ptr)
                    && vm.extension_cpython_ptr_contains_live(raw_ptr)
                {
                    vm.extension_cpython_ptr_value_set(raw_ptr, &value);
                    if trace_bound_ptr && is_bound_method {
                        eprintln!(
                            "[bound-method-ptr] reuse object_id={} ptr={:p}",
                            object_id, raw_ptr as *mut c_void
                        );
                    }
                    return raw_ptr as *mut c_void;
                }
                vm.extension_cpython_ptr_by_object_id.remove(&object_id);
            }
        }
        if let Value::Class(class_obj) = &value
            && let Object::Class(class_data) = &*class_obj.kind()
            && is_cpython_proxy_class(class_data)
        {
            if super::env_var_present_cached("PYRS_TRACE_CPY_PROXY_PTRS") {
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
        capi_perf_inc_handle_from_ptr_calls();
        if let Some(handle) = self.cpython_objects_by_ptr.get(&(object as usize)).copied() {
            capi_perf_inc_handle_from_ptr_hits();
            return Some(handle);
        }
        if !self.owns_cpython_allocation_ptr(object) {
            return None;
        }
        None
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
        capi_perf_inc_value_from_ptr_calls();
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
            if self.cpython_handle_requires_storage_sync(handle) {
                self.sync_value_from_cpython_storage(handle, object);
            }
            if !self.owns_cpython_allocation_ptr(object) {
                self.refresh_external_proxy_instance_type(handle, object);
            }
            if let Some(value) = self.object_value(handle) {
                if super::env_var_present_cached("PYRS_TRACE_METHOD_MAP_SOURCE")
                    && matches!(value, Value::BoundMethod(_))
                {
                    let type_name = cpython_type_name_for_object_ptr(object);
                    eprintln!(
                        "[method-map] source=handle ptr={:p} handle={} type={} value={}",
                        object,
                        handle,
                        type_name,
                        cpython_value_debug_tag(&value)
                    );
                }
                if Self::value_requires_external_mapping_stale_check(&value)
                    && self.external_proxy_mapping_is_stale(&value, object)
                {
                    if super::env_var_present_cached("PYRS_TRACE_CPY_PROXY_PTRS") {
                        eprintln!(
                            "[cpy-proxy] stale mapping reset object_ptr={:p} value_tag={}",
                            object,
                            cpython_value_debug_tag(&value)
                        );
                    }
                    self.cpython_objects_by_ptr.remove(&(object as usize));
                    if self
                        .cpython_ptr_by_handle
                        .get(&handle)
                        .is_some_and(|ptr| *ptr == object)
                    {
                        self.cpython_ptr_by_handle.remove(&handle);
                    }
                    if !self.vm.is_null() {
                        // SAFETY: VM pointer is valid for active C-API context lifetime.
                        let vm = unsafe { &mut *self.vm };
                        vm.extension_cpython_ptr_value_remove(object as usize);
                        if let Some(object_id) = Self::identity_object_id(&value)
                            && vm
                                .extension_cpython_ptr_by_object_id
                                .get(&object_id)
                                .is_some_and(|ptr| *ptr == object as usize)
                        {
                            vm.extension_cpython_ptr_by_object_id.remove(&object_id);
                        }
                    }
                    return self.cpython_value_from_ptr_or_proxy(object);
                }
                return Some(value);
            }
            return None;
        }
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            if let Some(value) = vm.extension_cpython_ptr_value(raw) {
                if super::env_var_present_cached("PYRS_TRACE_METHOD_MAP_SOURCE")
                    && matches!(value, Value::BoundMethod(_))
                {
                    let type_name = cpython_type_name_for_object_ptr(object);
                    eprintln!(
                        "[method-map] source=vm-cache ptr={:p} type={} value={}",
                        object,
                        type_name,
                        cpython_value_debug_tag(&value)
                    );
                }
                if Self::value_requires_external_mapping_stale_check(&value)
                    && self.external_proxy_mapping_is_stale(&value, object)
                {
                    if super::env_var_present_cached("PYRS_TRACE_CPY_PROXY_PTRS") {
                        eprintln!(
                            "[cpy-proxy] stale vm-cache reset object_ptr={:p} value_tag={}",
                            object,
                            cpython_value_debug_tag(&value)
                        );
                    }
                    vm.extension_cpython_ptr_value_remove(raw);
                    if let Some(object_id) = Self::identity_object_id(&value)
                        && vm
                            .extension_cpython_ptr_by_object_id
                            .get(&object_id)
                            .is_some_and(|ptr| *ptr == raw)
                    {
                        vm.extension_cpython_ptr_by_object_id.remove(&object_id);
                    }
                } else {
                    return Some(value);
                }
            }
        }
        if let Some(text) = cpython_lookup_interned_unicode_text(object) {
            return Some(Value::Str(text));
        }
        if !self.owns_cpython_allocation_ptr(object) {
            // Decode foreign PyLongObject payloads lazily, after owned/runtime-cached
            // pointer lookups, so we do not reinterpret pyrs-owned int layouts.
            if let Some(value) = unsafe { cpython_foreign_long_to_i64(object) } {
                return Some(Value::Int(value));
            }
            if let Some(value) = unsafe { cpython_foreign_long_to_u64(object) } {
                if let Ok(signed) = i64::try_from(value) {
                    return Some(Value::Int(signed));
                }
                return Some(Value::BigInt(Box::new(BigInt::from_u64(value))));
            }
        }
        None
    }

    fn cpython_value_from_borrowed_ptr(&mut self, object: *mut c_void) -> Option<Value> {
        let borrowed = BorrowedRef::from_ptr(object)?;
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.capi_registry_record_ref_kind(borrowed.ptr(), CapiRefKind::Borrowed);
        }
        self.cpython_value_from_ptr_or_proxy_with_external_ref_kind(
            object,
            CpythonProxyPtrOwnership::ExternalBorrowed,
        )
    }

    fn cpython_value_from_owned_ptr(&mut self, object: *mut c_void) -> Option<Value> {
        let owned = OwnedRef::from_ptr(object)?;
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.capi_registry_record_ref_kind(owned.ptr(), CapiRefKind::Owned);
        }
        self.cpython_value_from_ptr_or_proxy_with_external_ref_kind(
            object,
            CpythonProxyPtrOwnership::ExternalOwnedRef,
        )
    }

    fn cpython_value_from_stolen_ptr(&mut self, object: *mut c_void) -> Option<Value> {
        let stolen = StolenRef::from_ptr(object)?;
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.capi_registry_record_ref_kind(stolen.ptr(), CapiRefKind::Stolen);
        }
        self.cpython_value_from_ptr_or_proxy_with_external_ref_kind(
            object,
            CpythonProxyPtrOwnership::ExternalOwnedRef,
        )
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
        if super::env_var_present_cached("PYRS_TRACE_PROXY_CLASS_SOURCE") {
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

    fn external_proxy_mapping_is_stale(&self, value: &Value, object: *mut c_void) -> bool {
        if object.is_null() || self.vm.is_null() || self.owns_cpython_allocation_ptr(object) {
            return false;
        }
        // SAFETY: best-effort header probe for external PyObject*.
        let current_type_ptr = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<c_void>())
                .unwrap_or(std::ptr::null_mut())
        };
        if current_type_ptr.is_null() {
            return false;
        }
        match value {
            Value::Instance(instance_obj) => {
                let Object::Instance(instance_data) = &*instance_obj.kind() else {
                    return false;
                };
                let expected_type_ptr = Self::cpython_proxy_raw_ptr_from_value(&Value::Class(
                    instance_data.class.clone(),
                ));
                expected_type_ptr.is_some_and(|expected| expected != current_type_ptr)
            }
            Value::Class(class_obj) => {
                let expected_type_ptr =
                    Self::cpython_proxy_raw_ptr_from_value(&Value::Class(class_obj.clone()));
                expected_type_ptr.is_some_and(|expected| expected != object)
            }
            Value::BoundMethod(_) => {
                let expected_type_ptr = std::ptr::addr_of_mut!(PyMethod_Type).cast::<c_void>();
                if current_type_ptr != expected_type_ptr {
                    return true;
                }
                let Some((current_func, current_self)) =
                    Self::bound_method_payload_ptrs_from_raw_ptr(object)
                else {
                    return true;
                };
                let expected_pair = self.bound_method_payload_ptrs_from_value(value);
                match expected_pair {
                    Some((expected_func, expected_self)) => {
                        current_func != expected_func || current_self != expected_self
                    }
                    None => true,
                }
            }
            Value::Function(_) => {
                let expected_type_ptr = std::ptr::addr_of_mut!(PyFunction_Type).cast::<c_void>();
                current_type_ptr != expected_type_ptr
            }
            _ => false,
        }
    }

    fn cpython_existing_ptr_for_value(&self, value: &Value) -> Option<*mut c_void> {
        if let Some(raw) = Self::cpython_proxy_raw_ptr_from_value(value) {
            return Some(raw);
        }
        let object_id = Self::identity_object_id(value)?;
        if let Some(handle) = self.cpython_object_handles_by_id.get(&object_id)
            && let Some(ptr) = self.cpython_ptr_by_handle.get(handle).copied()
        {
            return Some(ptr);
        }
        if self.vm.is_null() {
            return None;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &*self.vm };
        vm.extension_cpython_ptr_by_object_id
            .get(&object_id)
            .copied()
            .map(|raw| raw as *mut c_void)
    }

    fn bound_method_payload_ptrs_from_value(
        &self,
        value: &Value,
    ) -> Option<(*mut c_void, *mut c_void)> {
        let Value::BoundMethod(bound_obj) = value else {
            return None;
        };
        let Object::BoundMethod(bound_data) = &*bound_obj.kind() else {
            return None;
        };
        let function_value = Self::value_from_objref_for_capi(&bound_data.function)?;
        let receiver_value = Self::value_from_objref_for_capi(&bound_data.receiver)?;
        let function_ptr = self.cpython_existing_ptr_for_value(&function_value)?;
        let receiver_ptr = self.cpython_existing_ptr_for_value(&receiver_value)?;
        Some((function_ptr, receiver_ptr))
    }

    fn bound_method_payload_ptrs_from_raw_ptr(
        object: *mut c_void,
    ) -> Option<(*mut c_void, *mut c_void)> {
        // SAFETY: caller validates `object` as a candidate method object pointer.
        let method = unsafe { object.cast::<CpythonMethodCompatObject>().as_ref() }?;
        Some((method.im_func, method.im_self))
    }

    fn value_requires_external_mapping_stale_check(value: &Value) -> bool {
        matches!(
            value,
            Value::Instance(_) | Value::Class(_) | Value::BoundMethod(_) | Value::Function(_)
        )
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
        let is_type_object =
            self.is_known_type_ptr(object) || Self::is_probable_type_object_ptr(object);
        if super::env_var_present_cached("PYRS_TRACE_CPY_PROXY_PTRS") {
            let object_type_name = unsafe {
                object_type
                    .as_ref()
                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string())
            };
            if is_type_object {
                // SAFETY: object_type indicates `object` has PyTypeObject layout.
                let (tp_name, tp_dict, tp_base) = unsafe {
                    let ty = object.cast::<CpythonTypeObject>();
                    let tp_name =
                        c_name_to_string((*ty).tp_name).unwrap_or_else(|_| "<invalid>".to_string());
                    (tp_name, (*ty).tp_dict, (*ty).tp_base)
                };
                eprintln!(
                    "[cpy-proxy] create proxy type-object ptr={:p} object_type={:p} object_type_name={} expected_type={:p} tp_name={} tp_dict={:p} tp_base={:p}",
                    object, object_type, object_type_name, expected_type, tp_name, tp_dict, tp_base
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
                if super::env_var_present_cached("PYRS_TRACE_PROXY_CLASS_SOURCE") {
                    let class_name = match &*type_proxy_class.kind() {
                        Object::Class(class_data) => class_data.name.clone(),
                        _ => "<non-class>".to_string(),
                    };
                    let source = if is_proxy_type_class {
                        "type-proxy"
                    } else {
                        "runtime-class"
                    };
                    eprintln!(
                        "[cpy-proxy] instance uses {} class={} object_ptr={:p} type_ptr={:p}",
                        source, class_name, object, object_type
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
            } else if super::env_var_present_cached("PYRS_TRACE_PROXY_CLASS_SOURCE") {
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
                        Value::DictValues(_) => "DictValues",
                        Value::DictItems(_) => "DictItems",
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
        if super::env_var_present_cached("PYRS_TRACE_PROXY_CLASS_SOURCE") && !is_type_object {
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
            let heap_type_name = cpython_heap_type_registry()
                .lock()
                .ok()
                .and_then(|registry| {
                    registry.get(&(object as usize)).map(|info| {
                        let module = if info.module_name.is_empty()
                            || info.module_name == "builtins"
                            || info.module_name == "__main__"
                        {
                            None
                        } else {
                            Some(info.module_name.clone())
                        };
                        (info.qualname.clone(), module)
                    })
                });
            if let Some((name, module)) = heap_type_name {
                if super::env_var_present_cached("PYRS_TRACE_CPY_OWNED_TYPES") {
                    eprintln!(
                        "[cpy-owned-type] proxy-name source=heap-registry ptr={:p} name={} module={}",
                        object,
                        name,
                        module.as_deref().unwrap_or("<none>")
                    );
                }
                (name, module)
            } else if self.is_known_type_ptr(object) {
                // SAFETY: known type pointers are registered from PyType_Ready / static exports.
                let type_name =
                    unsafe { c_name_to_string((*object.cast::<CpythonTypeObject>()).tp_name).ok() };
                if let Some(type_name) = type_name {
                    if super::env_var_present_cached("PYRS_TRACE_CPY_OWNED_TYPES") {
                        eprintln!(
                            "[cpy-owned-type] proxy-name source=known-tp_name ptr={:p} tp_name={}",
                            object, type_name
                        );
                    }
                    match type_name.rsplit_once('.') {
                        Some((module, name)) => (name.to_string(), Some(module.to_string())),
                        None => (type_name, None),
                    }
                } else {
                    (
                        format!("{CPY_PROXY_CLASS_NAME}_type_{:x}", object as usize),
                        None,
                    )
                }
            } else {
                (
                    format!("{CPY_PROXY_CLASS_NAME}_type_{:x}", object as usize),
                    None,
                )
            }
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
        let proxy_metaclass_value = if is_type_object {
            let metatype_ptr = object_type.cast::<c_void>();
            if metatype_ptr.is_null() || metatype_ptr == object {
                None
            } else {
                self.cpython_value_from_ptr_or_proxy(metatype_ptr)
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
        let proxy_metaclass =
            proxy_metaclass_value.and_then(|value| vm.class_from_base_value(value).ok());
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
            if let Some(metaclass_obj) = &proxy_metaclass {
                class_data.metaclass = Some(metaclass_obj.clone());
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
            let seeded_value = Value::Class(proxy_class.clone());
            vm.extension_cpython_ptr_value_set(object as usize, &seeded_value);
            if let Some(object_id) = Self::identity_object_id(&seeded_value) {
                vm.extension_cpython_ptr_by_object_id
                    .insert(object_id, object as usize);
            }
            self.populate_proxy_class_layout_attrs_from_type_object(&proxy_class, object);
            self.populate_proxy_class_attrs_from_type_dict(&proxy_class, object);
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
        self.cpython_value_from_ptr_or_proxy_with_external_ref_kind(
            object,
            CpythonProxyPtrOwnership::ExternalBorrowed,
        )
    }

    #[track_caller]
    fn cpython_value_from_ptr_or_proxy_with_external_ref_kind(
        &mut self,
        object: *mut c_void,
        external_ref_kind: CpythonProxyPtrOwnership,
    ) -> Option<Value> {
        let depth = CPY_PTR_MAP_DEPTH.with(|depth| {
            let next = depth.get().saturating_add(1);
            depth.set(next);
            next
        });
        struct CpyPtrMapDepthGuard;
        impl Drop for CpyPtrMapDepthGuard {
            fn drop(&mut self) {
                CPY_PTR_MAP_DEPTH.with(|depth| {
                    depth.set(depth.get().saturating_sub(1));
                });
            }
        }
        let _depth_guard = CpyPtrMapDepthGuard;
        if super::env_var_present_cached("PYRS_DEBUG_CPY_PTR_DEPTH") && depth > 256 {
            panic!(
                "cpython_value_from_ptr_or_proxy recursion depth exceeded: depth={depth} ptr={object:p}"
            );
        }
        if let Some(value) = self.cpython_value_from_ptr(object) {
            if !self.owns_cpython_allocation_ptr(object)
                && matches!(
                    external_ref_kind,
                    CpythonProxyPtrOwnership::ExternalOwnedRef
                )
            {
                unsafe {
                    Py_DecRef(object);
                }
            }
            return Some(value);
        }
        if object.is_null() || self.vm.is_null() {
            return None;
        }
        let mut owns_allocation = self.owns_cpython_allocation_ptr(object);
        let mut registry_known_live = {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &*self.vm };
            vm.capi_registry_contains_live_or_pending(object as usize)
        };
        if !registry_known_live {
            let discovered_owned = self
                .cpython_allocations
                .iter()
                .any(|allocation| allocation.cast::<c_void>() == object)
                || self.cpython_aux_allocations.contains(&object)
                || self
                    .cpython_list_buffers
                    .values()
                    .any(|(buffer, _)| buffer.cast::<c_void>() == object);
            if discovered_owned {
                self.capi_registry_register_owned_ptr(object, None);
                registry_known_live = true;
                owns_allocation = true;
                if super::env_var_present_cached("PYRS_TRACE_CPY_UNKNOWN_PTR") {
                    eprintln!(
                        "[cpy-proxy-registry-heal] ptr={:p} discovered_owned=true",
                        object
                    );
                }
            }
        }
        let probable_external = Self::is_probable_external_cpython_object_ptr(object);
        if !probable_external && !registry_known_live {
            // Some extension-created heap metatypes can fail the conservative
            // pointer-probability gate (for example, transient Cython metatypes)
            // while still being valid type objects. Allow proxy materialization
            // for those type-like pointers so C-API tuple/dict setters can carry
            // them through initialization paths.
            let likely_type_object = unsafe {
                const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
                let object_addr = object as usize;
                if object_addr < MIN_VALID_PTR
                    || object_addr % std::mem::align_of::<CpythonObjectHead>() != 0
                {
                    false
                } else {
                    object
                        .cast::<CpythonObjectHead>()
                        .as_ref()
                        .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                        .filter(|type_ptr| !type_ptr.is_null())
                        .map(|type_ptr| {
                            let type_addr = type_ptr as usize;
                            if type_addr < MIN_VALID_PTR
                                || type_addr % std::mem::align_of::<CpythonTypeObject>() != 0
                            {
                                return false;
                            }
                            let py_type =
                                std::ptr::addr_of_mut!(PyType_Type).cast::<CpythonTypeObject>();
                            type_ptr == py_type
                                || ((*type_ptr).tp_flags & PY_TPFLAGS_TYPE_SUBCLASS) != 0
                                || PyType_IsSubtype(
                                    type_ptr.cast::<c_void>(),
                                    py_type.cast::<c_void>(),
                                ) != 0
                        })
                        .unwrap_or(false)
                }
            };
            if likely_type_object && let Some(proxy) = self.cpython_external_proxy_value(object) {
                let ownership = if owns_allocation {
                    CpythonProxyPtrOwnership::OwnedCompat
                } else {
                    external_ref_kind
                };
                let proxy = self.cache_cpython_proxy_value_for_ptr(object, proxy, ownership, true);
                if super::env_var_present_cached("PYRS_TRACE_CPY_UNKNOWN_PTR") {
                    eprintln!(
                        "[cpy-proxy-type-fallback] ptr={:p} owns={} probable=false type_like={}",
                        object, owns_allocation, likely_type_object
                    );
                }
                return Some(proxy);
            }
            if super::env_var_present_cached("PYRS_TRACE_CPY_UNKNOWN_PTR") {
                let stack = if self.vm.is_null() {
                    "<no-vm>".to_string()
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
                    "[cpy-proxy-reject] ptr={:p} owns={} probable=false registry_known_live=false stack={}",
                    object, owns_allocation, stack
                );
            }
            return None;
        }
        let proxy = self.cpython_external_proxy_value(object)?;
        let ownership = if owns_allocation {
            CpythonProxyPtrOwnership::OwnedCompat
        } else {
            external_ref_kind
        };
        Some(self.cache_cpython_proxy_value_for_ptr(
            object,
            proxy,
            ownership,
            registry_known_live && !probable_external,
        ))
    }

    fn cache_cpython_proxy_value_for_ptr(
        &mut self,
        object: *mut c_void,
        proxy: Value,
        ownership: CpythonProxyPtrOwnership,
        force_trace_fallback: bool,
    ) -> Value {
        let proxy_object_id = Self::identity_object_id(&proxy);
        if !self.suppress_vm_proxy_persistence {
            match ownership {
                CpythonProxyPtrOwnership::ExternalBorrowed
                | CpythonProxyPtrOwnership::ExternalOwnedRef => {
                    self.capi_registry_register_external_ptr(object, proxy_object_id);
                    let inserted = self.capi_registry_pin_external_once_ptr(object);
                    if super::env_var_present_cached("PYRS_TRACE_CPY_PIN") || force_trace_fallback {
                        eprintln!(
                            "[cpy-pin] ptr={:p} branch=external ownership={:?} inserted={}",
                            object, ownership, inserted
                        );
                    }
                    if inserted {
                        if matches!(ownership, CpythonProxyPtrOwnership::ExternalBorrowed) {
                            // SAFETY: borrowed external proxies need one VM-owned incref so the
                            // runtime wrapper can hold them alive across C-API context teardown.
                            unsafe {
                                Py_IncRef(object);
                            }
                        }
                    } else if matches!(ownership, CpythonProxyPtrOwnership::ExternalOwnedRef) {
                        // SAFETY: an existing live proxy already owns the canonical external
                        // reference for this pointer; consume the extra owned ref now.
                        unsafe {
                            Py_DecRef(object);
                        }
                    }
                }
                CpythonProxyPtrOwnership::OwnedCompat => {
                    self.capi_registry_register_owned_ptr(object, proxy_object_id);
                    self.pin_owned_cpython_allocation_for_vm(object);
                }
            }
            if !self.vm.is_null() {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.extension_cpython_ptr_value_cache_if_absent(object as usize, &proxy);
                vm.register_cpython_proxy_instance_drop_hook(&proxy, object, ownership);
            }
        }
        let handle = self.alloc_object(proxy.clone());
        if super::env_var_present_cached("PYRS_TRACE_CPY_PTRS") {
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
            && super::env_var_present_cached("PYRS_TRACE_CPY_PTRS")
        {
            eprintln!(
                "[cpy-ptr] overwrite external ptr={:p} previous_handle={} new_handle={}",
                object, previous, handle
            );
        }
        self.cpython_ptr_by_handle.insert(handle, object);
        proxy
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

    fn ensure_module_dict_handle(&mut self, module: &ObjRef) -> Result<PyrsObjectHandle, String> {
        if let Some(existing) = self.module_dict_handle_for_module(module) {
            return Ok(existing);
        }
        if self.vm.is_null() {
            return Err("missing VM context".to_string());
        }
        let globals = match &*module.kind() {
            Object::Module(module_data) => module_data.globals.clone(),
            _ => return Err("module pointer is not a module".to_string()),
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *self.vm };
        let dict = vm.heap.alloc_dict(
            globals
                .into_iter()
                .map(|(name, value)| (Value::Str(name), value))
                .collect(),
        );
        let dict_ptr = self.alloc_cpython_ptr_for_value(dict);
        let Some(dict_handle) = self.cpython_handle_from_ptr(dict_ptr) else {
            return Err("failed to materialize module dict handle".to_string());
        };
        self.module_dict_handles.insert(dict_handle, module.clone());
        self.module_dict_handle_by_module_id
            .insert(module.id(), dict_handle);
        Ok(dict_handle)
    }

    fn extension_module_def_ptr_for_module(&mut self, module: &ObjRef) -> *mut CpythonModuleDef {
        if self.vm.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.extension_module_def_registry
            .get(&module.id())
            .copied()
            .map_or(std::ptr::null_mut(), |ptr| ptr as *mut CpythonModuleDef)
    }

    fn extension_module_state_ptr_for_module(&mut self, module: &ObjRef) -> *mut c_void {
        if self.vm.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.extension_module_state_registry
            .get(&module.id())
            .map_or(std::ptr::null_mut(), |entry| entry.state as *mut c_void)
    }

    fn sync_module_compat_from_value(
        &mut self,
        raw_module: *mut CpythonModuleCompatObject,
        module: &ObjRef,
        refcount: isize,
    ) {
        if raw_module.is_null() {
            return;
        }
        let dict_ptr = self
            .ensure_module_dict_handle(module)
            .ok()
            .map(|handle| self.alloc_cpython_ptr_for_handle(handle))
            .unwrap_or(std::ptr::null_mut());
        let module_name = match &*module.kind() {
            Object::Module(module_data) => module_data.name.clone(),
            _ => "<module>".to_string(),
        };
        let name_ptr = self.alloc_cpython_ptr_for_value(Value::Str(module_name));
        let def_ptr = self.extension_module_def_ptr_for_module(module);
        let state_ptr = self.extension_module_state_ptr_for_module(module);
        // SAFETY: `raw_module` points to writable module-compatible storage.
        unsafe {
            (*raw_module).ob_base.ob_refcnt = refcount.max(1);
            (*raw_module).ob_base.ob_type = std::ptr::addr_of_mut!(PyModule_Type).cast();
            (*raw_module).md_dict = dict_ptr;
            (*raw_module).md_def = def_ptr;
            (*raw_module).md_state = state_ptr;
            (*raw_module).md_weaklist = std::ptr::null_mut();
            (*raw_module).md_name = name_ptr;
        }
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

    fn sync_module_dict_del(&mut self, module: &ObjRef, key: &str) -> Result<(), String> {
        let Some(dict_handle) = self.module_dict_handle_for_module(module) else {
            return Ok(());
        };
        let Some(slot) = self.objects.get(&dict_handle) else {
            return Ok(());
        };
        let Value::Dict(dict_obj) = &slot.value else {
            return Ok(());
        };
        let _ = dict_remove_value(dict_obj, &Value::Str(key.to_string()));
        Ok(())
    }

    fn alloc_aux_buffer(&mut self, size: usize) -> *mut c_void {
        // SAFETY: allocates raw storage managed by this context.
        let raw = unsafe { malloc(size) };
        if raw.is_null() {
            self.set_error("out of memory allocating CPython auxiliary buffer");
            return std::ptr::null_mut();
        }
        self.cpython_aux_allocations.push(raw);
        self.capi_registry_register_owned_ptr(raw, None);
        raw
    }

    pub(super) fn register_aux_allocation(&mut self, raw: *mut c_void) {
        if raw.is_null() {
            return;
        }
        self.cpython_aux_allocations.push(raw);
        self.capi_registry_register_owned_ptr(raw, None);
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
        let invalid_sentinel = usize::MAX as *mut c_void;
        if self_ptr == invalid_sentinel
            || module_ptr == invalid_sentinel
            || class_ptr == invalid_sentinel
        {
            let method_name = if method_def.is_null() {
                "<null>".to_string()
            } else {
                // SAFETY: method definition pointer is extension-owned metadata.
                unsafe { c_name_to_string((*method_def).ml_name) }
                    .unwrap_or_else(|_| "<invalid>".to_string())
            };
            self.set_error(format!(
                "invalid cfunction binding sentinel for method '{}': self={:p} module={:p} class={:p}",
                method_name, self_ptr, module_ptr, class_ptr
            ));
            return std::ptr::null_mut();
        }
        let cache_key = (
            method_def as usize,
            self_ptr as usize,
            module_ptr as usize,
            class_ptr as usize,
        );
        if let Some(existing) = self.cpython_cfunction_ptr_cache.get(&cache_key).copied() {
            return existing;
        }
        let method_flags = if method_def.is_null() {
            0
        } else {
            // SAFETY: method definition pointer is validated by callers.
            unsafe { (*method_def).ml_flags }
        };
        let is_cmethod = (method_flags & METH_METHOD) != 0;
        if !is_cmethod && !class_ptr.is_null() {
            let method_name = if method_def.is_null() {
                "<null>".to_string()
            } else {
                // SAFETY: method definition pointer is extension-owned metadata.
                unsafe { c_name_to_string((*method_def).ml_name) }
                    .unwrap_or_else(|_| "<invalid>".to_string())
            };
            self.set_error(format!(
                "non-METH_METHOD cfunction '{}' received class binding",
                method_name
            ));
            return std::ptr::null_mut();
        }
        // SAFETY: allocates C-compatible storage for cfunction/cmethod object payload.
        let (raw_ptr, compat_ptr) = if is_cmethod {
            let raw_method = unsafe { malloc(std::mem::size_of::<CpythonCMethodCompatObject>()) }
                .cast::<CpythonCMethodCompatObject>();
            if raw_method.is_null() {
                self.set_error("out of memory allocating CPython cmethod object");
                return std::ptr::null_mut();
            }
            // SAFETY: `raw_method` points to writable cmethod storage.
            unsafe {
                raw_method.write(CpythonCMethodCompatObject {
                    function: CpythonCFunctionCompatObject {
                        ob_base: CpythonObjectHead {
                            ob_refcnt: 1,
                            ob_type: std::ptr::addr_of_mut!(PyCFunction_Type).cast(),
                        },
                        m_ml: method_def,
                        m_self: self_ptr,
                        m_module: module_ptr,
                        m_weakreflist: std::ptr::null_mut(),
                        vectorcall: std::ptr::null_mut(),
                    },
                    mm_class: class_ptr,
                });
            }
            (
                raw_method.cast::<c_void>(),
                raw_method.cast::<CpythonCompatObject>(),
            )
        } else {
            let raw_function =
                unsafe { malloc(std::mem::size_of::<CpythonCFunctionCompatObject>()) }
                    .cast::<CpythonCFunctionCompatObject>();
            if raw_function.is_null() {
                self.set_error("out of memory allocating CPython cfunction object");
                return std::ptr::null_mut();
            }
            // SAFETY: `raw_function` points to writable cfunction storage.
            unsafe {
                raw_function.write(CpythonCFunctionCompatObject {
                    ob_base: CpythonObjectHead {
                        ob_refcnt: 1,
                        ob_type: std::ptr::addr_of_mut!(PyCFunction_Type).cast(),
                    },
                    m_ml: method_def,
                    m_self: self_ptr,
                    m_module: module_ptr,
                    m_weakreflist: std::ptr::null_mut(),
                    vectorcall: std::ptr::null_mut(),
                });
            }
            (
                raw_function.cast::<c_void>(),
                raw_function.cast::<CpythonCompatObject>(),
            )
        };
        if raw_ptr.is_null() {
            return std::ptr::null_mut();
        }
        let ptr = raw_ptr;
        self.cpython_allocations.push(compat_ptr);
        self.capi_registry_register_owned_ptr(ptr, None);
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
        let (owner_type, name_ptr) = match descriptor_kind {
            CpythonDescriptorKind::Method {
                owner_type,
                method_def,
                ..
            } => {
                let method_name = if method_def.is_null() {
                    std::ptr::null()
                } else {
                    // SAFETY: method metadata pointer is extension-owned.
                    unsafe { (*method_def).ml_name }
                };
                (owner_type, method_name)
            }
            CpythonDescriptorKind::GetSet { owner_type, getset } => {
                let getset_name = if getset.is_null() {
                    std::ptr::null()
                } else {
                    // SAFETY: getset metadata pointer is extension-owned.
                    unsafe { (*getset).name }
                };
                (owner_type, getset_name)
            }
            CpythonDescriptorKind::Member { owner_type, member } => {
                let member_name = if member.is_null() {
                    std::ptr::null()
                } else {
                    // SAFETY: member metadata pointer is extension-owned.
                    unsafe { (*member).name }
                };
                (owner_type, member_name)
            }
        };
        let name_object_ptr = if name_ptr.is_null() {
            std::ptr::null_mut()
        } else {
            let name_value = unsafe { c_name_to_string(name_ptr) }
                .ok()
                .map(Value::Str)
                .unwrap_or(Value::None);
            self.alloc_cpython_ptr_for_value(name_value)
        };
        let descriptor_ptr = match descriptor_kind {
            CpythonDescriptorKind::Method { method_def, .. } => {
                // SAFETY: allocates C-compatible storage for method-descriptor payload.
                let raw = unsafe { malloc(std::mem::size_of::<CpythonMethodDescrCompatObject>()) }
                    .cast::<CpythonMethodDescrCompatObject>();
                if raw.is_null() {
                    self.set_error("out of memory allocating CPython method descriptor");
                    return std::ptr::null_mut();
                }
                // SAFETY: raw points to writable method-descriptor storage.
                unsafe {
                    raw.write(CpythonMethodDescrCompatObject {
                        d_common: CpythonDescrCompatObject {
                            ob_base: CpythonObjectHead {
                                ob_refcnt: 1,
                                ob_type: descriptor_type.cast(),
                            },
                            d_type: owner_type,
                            d_name: name_object_ptr,
                            d_qualname: std::ptr::null_mut(),
                        },
                        d_method: method_def,
                        vectorcall: std::ptr::null_mut(),
                    });
                }
                let raw_object = raw.cast::<CpythonCompatObject>();
                self.cpython_allocations.push(raw_object);
                self.capi_registry_register_owned_ptr(raw_object.cast(), None);
                raw.cast::<c_void>()
            }
            CpythonDescriptorKind::GetSet { getset, .. } => {
                // SAFETY: allocates C-compatible storage for getset-descriptor payload.
                let raw = unsafe { malloc(std::mem::size_of::<CpythonGetSetDescrCompatObject>()) }
                    .cast::<CpythonGetSetDescrCompatObject>();
                if raw.is_null() {
                    self.set_error("out of memory allocating CPython getset descriptor");
                    return std::ptr::null_mut();
                }
                // SAFETY: raw points to writable getset-descriptor storage.
                unsafe {
                    raw.write(CpythonGetSetDescrCompatObject {
                        d_common: CpythonDescrCompatObject {
                            ob_base: CpythonObjectHead {
                                ob_refcnt: 1,
                                ob_type: descriptor_type.cast(),
                            },
                            d_type: owner_type,
                            d_name: name_object_ptr,
                            d_qualname: std::ptr::null_mut(),
                        },
                        d_getset: getset,
                    });
                }
                let raw_object = raw.cast::<CpythonCompatObject>();
                self.cpython_allocations.push(raw_object);
                self.capi_registry_register_owned_ptr(raw_object.cast(), None);
                raw.cast::<c_void>()
            }
            CpythonDescriptorKind::Member { member, .. } => {
                // SAFETY: allocates C-compatible storage for member-descriptor payload.
                let raw = unsafe { malloc(std::mem::size_of::<CpythonMemberDescrCompatObject>()) }
                    .cast::<CpythonMemberDescrCompatObject>();
                if raw.is_null() {
                    self.set_error("out of memory allocating CPython member descriptor");
                    return std::ptr::null_mut();
                }
                // SAFETY: raw points to writable member-descriptor storage.
                unsafe {
                    raw.write(CpythonMemberDescrCompatObject {
                        d_common: CpythonDescrCompatObject {
                            ob_base: CpythonObjectHead {
                                ob_refcnt: 1,
                                ob_type: descriptor_type.cast(),
                            },
                            d_type: owner_type,
                            d_name: name_object_ptr,
                            d_qualname: std::ptr::null_mut(),
                        },
                        d_member: member,
                    });
                }
                let raw_object = raw.cast::<CpythonCompatObject>();
                self.cpython_allocations.push(raw_object);
                self.capi_registry_register_owned_ptr(raw_object.cast(), None);
                raw.cast::<c_void>()
            }
        };
        self.capi_registry_register_owned_ptr(descriptor_ptr, None);
        self.cpython_descriptors
            .insert(descriptor_ptr as usize, descriptor_kind);
        CPYTHON_DESCRIPTOR_REGISTRY.with(|registry| {
            registry
                .borrow_mut()
                .insert(descriptor_ptr as usize, descriptor_kind);
        });
        descriptor_ptr
    }

    fn resolve_descriptor_attr_ptr(
        &mut self,
        descriptor_ptr: *mut c_void,
        object: *mut c_void,
        object_type: *mut CpythonTypeObject,
        is_type_object: bool,
    ) -> Option<*mut c_void> {
        let descriptor_kind = self.descriptor_kind_for_ptr(descriptor_ptr)?;
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

    fn descriptor_kind_for_ptr(
        &mut self,
        descriptor_ptr: *mut c_void,
    ) -> Option<CpythonDescriptorKind> {
        let descriptor_key = descriptor_ptr as usize;
        if let Some(kind) = self.cpython_descriptors.get(&descriptor_key) {
            return Some(*kind);
        }
        let kind = CPYTHON_DESCRIPTOR_REGISTRY
            .with(|registry| registry.borrow().get(&descriptor_key).copied())?;
        self.cpython_descriptors.insert(descriptor_key, kind);
        Some(kind)
    }

    fn set_descriptor_attr_ptr(
        &mut self,
        descriptor_ptr: *mut c_void,
        object: *mut c_void,
        object_type: *mut CpythonTypeObject,
        value: *mut c_void,
    ) -> Option<c_int> {
        let descriptor_kind = self.descriptor_kind_for_ptr(descriptor_ptr)?;
        match descriptor_kind {
            CpythonDescriptorKind::Method { .. } => {
                self.set_error("TypeError: descriptor is not writable");
                Some(-1)
            }
            CpythonDescriptorKind::GetSet { owner_type, getset } => {
                if owner_type.is_null() || getset.is_null() {
                    self.set_error("descriptor getset metadata is invalid");
                    return Some(-1);
                }
                if object_type.is_null()
                    || unsafe { PyType_IsSubtype(object_type.cast(), owner_type.cast()) } == 0
                {
                    self.set_error("descriptor getset requires subtype instance");
                    return Some(-1);
                }
                let setter = unsafe { (*getset).set };
                if setter.is_null() {
                    self.set_error("AttributeError: attribute is not writable");
                    return Some(-1);
                }
                let set: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int =
                    unsafe { std::mem::transmute(setter) };
                let closure = unsafe { (*getset).closure };
                Some(unsafe { set(object, value, closure) })
            }
            CpythonDescriptorKind::Member { owner_type, member } => {
                if owner_type.is_null() || member.is_null() {
                    self.set_error("descriptor member metadata is invalid");
                    return Some(-1);
                }
                if object_type.is_null()
                    || unsafe { PyType_IsSubtype(object_type.cast(), owner_type.cast()) } == 0
                {
                    self.set_error("descriptor member requires subtype instance");
                    return Some(-1);
                }
                Some(unsafe {
                    PyMember_SetOne(object.cast::<c_char>(), member.cast::<c_void>(), value)
                })
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
            PY_MEMBER_T_BYTE => {
                // SAFETY: BYTE members store a signed 8-bit integer.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<i8>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Int(raw as i64)))
            }
            PY_MEMBER_T_UBYTE => {
                // SAFETY: UBYTE members store an unsigned 8-bit integer.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u8>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Int(raw as i64)))
            }
            PY_MEMBER_T_SHORT => {
                // SAFETY: SHORT members store a signed 16-bit integer.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<i16>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Int(raw as i64)))
            }
            PY_MEMBER_T_USHORT => {
                // SAFETY: USHORT members store an unsigned 16-bit integer.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<u16>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Int(raw as i64)))
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
            PY_MEMBER_T_FLOAT => {
                // SAFETY: FLOAT members store an f32 payload.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<f32>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Float(raw as f64)))
            }
            PY_MEMBER_T_DOUBLE => {
                // SAFETY: DOUBLE members store an f64 payload.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<f64>()) };
                Some(self.alloc_cpython_ptr_for_value(Value::Float(raw)))
            }
            PY_MEMBER_T_STRING => {
                // SAFETY: STRING members store a `const char*`.
                let raw = unsafe { std::ptr::read_unaligned(field_ptr.cast::<*const c_char>()) };
                if raw.is_null() {
                    Some(self.alloc_cpython_ptr_for_value(Value::None))
                } else {
                    let text = unsafe { CStr::from_ptr(raw) }.to_str().ok()?.to_string();
                    Some(self.alloc_cpython_ptr_for_value(Value::Str(text)))
                }
            }
            PY_MEMBER_T_STRING_INPLACE => {
                // SAFETY: STRING_INPLACE members are stored inline as a NUL-terminated buffer.
                let text = unsafe { CStr::from_ptr(field_ptr.cast::<c_char>()) }
                    .to_str()
                    .ok()?
                    .to_string();
                Some(self.alloc_cpython_ptr_for_value(Value::Str(text)))
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

    fn cpython_slot_table_ptr_is_valid<T>(ptr: *const T) -> bool {
        const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
        !ptr.is_null()
            && (ptr as usize) >= MIN_VALID_PTR
            && (ptr as usize) % std::mem::align_of::<T>() == 0
    }

    fn external_mapping_get_item_string(
        &mut self,
        mapping_ptr: *mut c_void,
        key: &str,
    ) -> *mut c_void {
        const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
        if mapping_ptr.is_null()
            || (mapping_ptr as usize) < MIN_VALID_PTR
            || (mapping_ptr as usize) % std::mem::align_of::<CpythonObjectHead>() != 0
        {
            return std::ptr::null_mut();
        }
        if self.owns_cpython_allocation_ptr(mapping_ptr) {
            return std::ptr::null_mut();
        }
        // External `tp_dict` pointers are usually not registry-tracked in our runtime.
        // Require structural pointer/type validation below, but do not require prior
        // registry enrollment; otherwise class-attribute lookups miss valid foreign
        // entries (for example `numpy.generic.__format__`).
        let key_c_name = match CString::new(key) {
            Ok(name) => name,
            Err(_) => return std::ptr::null_mut(),
        };
        let key_ptr = unsafe { PyUnicode_FromString(key_c_name.as_ptr()) };
        if key_ptr.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: best-effort foreign object header probe.
        let type_ptr = unsafe {
            mapping_ptr
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if type_ptr.is_null()
            || (type_ptr as usize) < MIN_VALID_PTR
            || (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0
        {
            return std::ptr::null_mut();
        }
        // SAFETY: `type_ptr` points at candidate PyTypeObject layout.
        let mapping = unsafe { (*type_ptr).tp_as_mapping.cast::<CpythonMappingMethods>() };
        if mapping.is_null()
            || (mapping as usize) < MIN_VALID_PTR
            || (mapping as usize) % std::mem::align_of::<CpythonMappingMethods>() != 0
        {
            return std::ptr::null_mut();
        }
        // SAFETY: mapping table follows CPython ABI for external objects.
        let mp_subscript = unsafe { (*mapping).mp_subscript };
        if mp_subscript.is_null() {
            return std::ptr::null_mut();
        }
        let subscript: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: `mp_subscript` follows `binaryfunc` ABI for mapping subscripts.
            unsafe { std::mem::transmute(mp_subscript) };
        // SAFETY: external mapping slot call with validated mapping pointer + key object.
        let value_ptr = unsafe { subscript(mapping_ptr, key_ptr) };
        // SAFETY: `key_ptr` is a temporary strong reference created above.
        unsafe { Py_DecRef(key_ptr) };
        if value_ptr.is_null() {
            // PyDict_GetItem-style probes suppress lookup exceptions.
            unsafe { PyErr_Clear() };
        }
        value_ptr
    }

    fn bind_generic_descriptor_attr_ptr(
        &mut self,
        descriptor_ptr: *mut c_void,
        object: *mut c_void,
        object_type: *mut c_void,
        is_type_object: bool,
    ) -> Option<*mut c_void> {
        if descriptor_ptr.is_null() {
            return None;
        }
        // SAFETY: caller provides a candidate descriptor pointer.
        let descriptor_type = unsafe {
            descriptor_ptr
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if !Self::cpython_slot_table_ptr_is_valid(descriptor_type) {
            return None;
        }
        // SAFETY: descriptor type layout validated above.
        let descriptor_get = unsafe { (*descriptor_type).tp_descr_get };
        if descriptor_get.is_null() {
            return None;
        }
        let descriptor_get: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *mut c_void,
        ) -> *mut c_void =
            // SAFETY: `tp_descr_get` follows the descriptor-get ABI.
            unsafe { std::mem::transmute(descriptor_get) };
        let self_ptr = if is_type_object {
            std::ptr::null_mut()
        } else {
            object
        };
        let owner_ptr = if is_type_object { object } else { object_type };
        // SAFETY: mirrors CPython descriptor dispatch semantics.
        let bound = unsafe { descriptor_get(descriptor_ptr, self_ptr, owner_ptr) };
        if !bound.is_null() {
            return Some(bound);
        }
        if unsafe { !PyErr_Occurred().is_null() } {
            return Some(std::ptr::null_mut());
        }
        None
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
        let expected_type = std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>();
        // Use object-level type-object detection (not type-flag checks on `ob_type` alone):
        // instances have `ob_type` pointing at a type object, but are not type objects.
        let is_type_object = cpython_is_type_object_ptr(object)
            || self.is_known_type_ptr(object)
            || Self::is_probable_type_object_ptr(object);
        let trace_type_attr =
            attr_name == "type" && super::env_var_present_cached("PYRS_TRACE_PROXY_TYPE_ATTR");
        let trace_attr_max =
            attr_name == "max" && super::env_var_present_cached("PYRS_TRACE_ATTR_MAX");
        let trace_lookup_branch = super::env_var_present_cached("PYRS_TRACE_PROXY_LOOKUP_BRANCH");
        let trace_repr_lookup = super::env_var_present_cached("PYRS_TRACE_PROXY_REPR_LOOKUP")
            && matches!(attr_name, "__repr__" | "__str__");
        let is_proxy_trace = attr_name == "__array_finalize__"
            && super::env_var_present_cached("PYRS_TRACE_CPY_PROXY_PTRS");
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
        if trace_attr_max {
            eprintln!(
                "[cpy-lookup-max] object={:p} object_type={:p} is_type_object={} start_current={:p}",
                object, object_type, is_type_object, current
            );
        }
        let key = Value::Str(attr_name.to_string());
        for _ in 0..64 {
            if current.is_null() {
                break;
            }
            if !Self::cpython_slot_table_ptr_is_valid::<CpythonTypeObject>(current) {
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
                // SAFETY: current points to a type object header.
                let members_ptr = unsafe { (*current).tp_members };
                eprintln!(
                    "[cpy-proxy-attr] scan type={:p} name={} dict={:p} methods={:p} getset={:p} members={:p} base={:p}",
                    current, type_name, dict_ptr, methods_ptr, getset_ptr, members_ptr, base_ptr
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
            if !dict_ptr.is_null() {
                // Probe through the C-API mapping surface first. This works for both
                // external tp_dict pointers and compat-owned dict objects whose runtime
                // mapping was not materialized yet.
                let external_value_ptr = self.external_mapping_get_item_string(dict_ptr, attr_name);
                if !external_value_ptr.is_null() {
                    let mut descriptor_rejected_for_slot_fallback = false;
                    let trace_generate_state_lookup = attr_name == "generate_state"
                        && super::env_var_present_cached("PYRS_TRACE_GETATTR_GENERATE_STATE");
                    // SAFETY: best-effort descriptor probe on tp_dict entry.
                    let descriptor_type = unsafe {
                        external_value_ptr
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                            .unwrap_or(std::ptr::null_mut())
                    };
                    if trace_generate_state_lookup {
                        let descriptor_type_name = if descriptor_type.is_null() {
                            "<null>".to_string()
                        } else {
                            // SAFETY: descriptor type pointer is checked above.
                            unsafe {
                                c_name_to_string((*descriptor_type).tp_name)
                                    .unwrap_or_else(|_| "<invalid>".to_string())
                            }
                        };
                        eprintln!(
                            "[generate-state-lookup] object={:p} current={:p} dict={:p} value_ptr={:p} descriptor_type={:p} descriptor_type_name={}",
                            object,
                            current,
                            dict_ptr,
                            external_value_ptr,
                            descriptor_type,
                            descriptor_type_name
                        );
                    }
                    if !descriptor_type.is_null() {
                        // SAFETY: descriptor type pointer was loaded from object header above.
                        let descriptor_get = unsafe { (*descriptor_type).tp_descr_get };
                        if trace_attr_max {
                            let descriptor_type_name = unsafe {
                                c_name_to_string((*descriptor_type).tp_name)
                                    .unwrap_or_else(|_| "<invalid>".to_string())
                            };
                            eprintln!(
                                "[cpy-lookup-max] descriptor value_ptr={:p} descriptor_type={:p} descriptor_type_name={} descriptor_get={:p}",
                                external_value_ptr,
                                descriptor_type,
                                descriptor_type_name,
                                descriptor_get
                            );
                        }
                        if trace_generate_state_lookup {
                            eprintln!(
                                "[generate-state-lookup] descriptor_get={:p}",
                                descriptor_get
                            );
                        }
                        if !descriptor_get.is_null() {
                            let descriptor_get: unsafe extern "C" fn(
                                *mut c_void,
                                *mut c_void,
                                *mut c_void,
                            ) -> *mut c_void =
                                // SAFETY: descriptor getter follows CPython descriptor ABI.
                                unsafe { std::mem::transmute(descriptor_get) };
                            let owner_ptr = if is_type_object {
                                object
                            } else {
                                object_type.cast::<c_void>()
                            };
                            let self_ptr = if is_type_object {
                                std::ptr::null_mut()
                            } else {
                                object
                            };
                            if trace_attr_max {
                                eprintln!(
                                    "[cpy-lookup-max] descriptor bind self_ptr={:p} owner_ptr={:p}",
                                    self_ptr, owner_ptr
                                );
                            }
                            // SAFETY: descriptor access mirrors CPython descriptor invocation.
                            let bound =
                                unsafe { descriptor_get(external_value_ptr, self_ptr, owner_ptr) };
                            if trace_attr_max {
                                eprintln!(
                                    "[cpy-lookup-max] descriptor result={:p} pyerr={:p}",
                                    bound,
                                    unsafe { PyErr_Occurred() }
                                );
                            }
                            if trace_generate_state_lookup {
                                eprintln!(
                                    "[generate-state-lookup] descriptor_get_result={:p} pyerr={:p} self_ptr={:p} owner_ptr={:p}",
                                    bound,
                                    unsafe { PyErr_Occurred() },
                                    self_ptr,
                                    owner_ptr
                                );
                            }
                            if !bound.is_null() {
                                if trace_lookup_branch {
                                    eprintln!(
                                        "[proxy-lookup-branch] attr={} branch=dict_external_descriptor object={:p} current={:p} value_ptr={:p}",
                                        attr_name, object, current, bound
                                    );
                                }
                                return Some(bound);
                            }
                            // If descriptor binding raised an exception, propagate the failure
                            // instead of silently returning the unbound descriptor object.
                            if unsafe { !PyErr_Occurred().is_null() } {
                                let allow_slot_fallback = !is_type_object
                                    && (cpython_slot_richcompare_method_def(attr_name).is_some()
                                        || cpython_slot_unary_method_def(attr_name).is_some()
                                        || cpython_slot_getitem_method_def(attr_name).is_some()
                                        || cpython_slot_len_iter_setitem_method_def(attr_name)
                                            .is_some()
                                        || attr_name == "__init__");
                                if allow_slot_fallback {
                                    unsafe { PyErr_Clear() };
                                    descriptor_rejected_for_slot_fallback = true;
                                } else {
                                    return Some(std::ptr::null_mut());
                                }
                            }
                        }
                    }
                    if !descriptor_rejected_for_slot_fallback {
                        if !is_type_object {
                            if let Some(descriptor_value) =
                                self.cpython_value_from_borrowed_ptr(external_value_ptr)
                            {
                                if trace_attr_max {
                                    eprintln!(
                                        "[cpy-lookup-max] runtime descriptor fallback descriptor_tag={}",
                                        cpython_value_debug_tag(&descriptor_value)
                                    );
                                }
                                match cpython_getattr_in_context(
                                    self,
                                    descriptor_value.clone(),
                                    "__get__",
                                ) {
                                    Ok(getter) => {
                                        if let Some(instance_value) =
                                            self.cpython_value_from_ptr_or_proxy(object)
                                        {
                                            let owner_value = self
                                                .cpython_value_from_ptr_or_proxy(
                                                    object_type.cast::<c_void>(),
                                                )
                                                .unwrap_or(Value::None);
                                            match cpython_call_internal_in_context(
                                                self,
                                                getter,
                                                vec![instance_value, owner_value],
                                                HashMap::new(),
                                            ) {
                                                Ok(bound_value) => {
                                                    let bound_ptr = self
                                                        .alloc_cpython_ptr_for_value(bound_value);
                                                    if !bound_ptr.is_null() {
                                                        if trace_attr_max {
                                                            eprintln!(
                                                                "[cpy-lookup-max] runtime descriptor __get__ fallback bound_ptr={:p}",
                                                                bound_ptr
                                                            );
                                                        }
                                                        if trace_lookup_branch {
                                                            eprintln!(
                                                                "[proxy-lookup-branch] attr={} branch=dict_external_runtime_descriptor object={:p} current={:p} value_ptr={:p}",
                                                                attr_name,
                                                                object,
                                                                current,
                                                                bound_ptr
                                                            );
                                                        }
                                                        return Some(bound_ptr);
                                                    }
                                                }
                                                Err(err) => {
                                                    if trace_attr_max {
                                                        eprintln!(
                                                            "[cpy-lookup-max] runtime descriptor __get__ call failed err={}",
                                                            err.message
                                                        );
                                                    }
                                                }
                                            }
                                        } else if trace_attr_max {
                                            eprintln!(
                                                "[cpy-lookup-max] runtime descriptor fallback missing instance mapping"
                                            );
                                        }
                                    }
                                    Err(err) => {
                                        if trace_attr_max {
                                            eprintln!(
                                                "[cpy-lookup-max] runtime descriptor missing __get__ err={}",
                                                err
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        if trace_attr_max {
                            eprintln!(
                                "[cpy-lookup-max] returning external value without descriptor binding ptr={:p}",
                                external_value_ptr
                            );
                        }
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=dict_external_value object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, external_value_ptr
                            );
                        }
                        return Some(external_value_ptr);
                    }
                } else if trace_repr_lookup {
                    eprintln!(
                        "[proxy-repr-lookup] attr={} dict_external_miss object={:p} current={:p}",
                        attr_name, object, current
                    );
                }
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
                if let Some(raw_descriptor_ptr) = Self::cpython_proxy_raw_ptr_from_value(&value)
                    && let Some(bound_ptr) = self.bind_generic_descriptor_attr_ptr(
                        raw_descriptor_ptr,
                        object,
                        object_type.cast(),
                        is_type_object,
                    )
                {
                    if trace_lookup_branch {
                        eprintln!(
                            "[proxy-lookup-branch] attr={} branch=dict_generic_descriptor object={:p} current={:p} value_ptr={:p}",
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
                if super::env_var_present_cached("PYRS_TRACE_PROXY_ATTR_CALL") {
                    eprintln!(
                        "[proxy-attr-map] source=tp_dict_value target={:p} attr={} value_tag={}",
                        object,
                        attr_name,
                        cpython_value_debug_tag(&value)
                    );
                }
                let value_ptr = self.alloc_cpython_ptr_for_value(value.clone());
                if !value_ptr.is_null()
                    && !is_type_object
                    && let Some(bound_ptr) = self.bind_generic_descriptor_attr_ptr(
                        value_ptr,
                        object,
                        object_type.cast(),
                        false,
                    )
                {
                    if trace_lookup_branch {
                        eprintln!(
                            "[proxy-lookup-branch] attr={} branch=dict_value_generic_descriptor object={:p} current={:p} value_ptr={:p}",
                            attr_name, object, current, bound_ptr
                        );
                    }
                    return Some(bound_ptr);
                }
                if trace_lookup_branch {
                    eprintln!(
                        "[proxy-lookup-branch] attr={} branch=dict_value object={:p} current={:p} value_ptr={:p}",
                        attr_name, object, current, value_ptr
                    );
                }
                return Some(value_ptr);
            } else if is_proxy_trace && !dict_ptr.is_null() {
                eprintln!(
                    "[cpy-proxy] tp_dict lookup miss current={:p} dict_ptr={:p}",
                    current, dict_ptr
                );
            }
            if !is_type_object {
                // SAFETY: current points to a PyTypeObject-compatible header.
                let methods_ptr = unsafe { (*current).tp_methods }.cast::<CpythonMethodDef>();
                if Self::cpython_slot_table_ptr_is_valid(methods_ptr) {
                    let mut method = methods_ptr;
                    let mut traced_methods = 0usize;
                    for _ in 0..4096 {
                        if !Self::cpython_slot_table_ptr_is_valid(method) {
                            break;
                        }
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
                if Self::cpython_slot_table_ptr_is_valid(getset_ptr) {
                    let mut getset = getset_ptr;
                    let mut traced_getsets = 0usize;
                    for _ in 0..4096 {
                        if !Self::cpython_slot_table_ptr_is_valid(getset) {
                            break;
                        }
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
                if Self::cpython_slot_table_ptr_is_valid(members_ptr) {
                    let mut member = members_ptr;
                    let mut traced_members = 0usize;
                    for _ in 0..4096 {
                        if !Self::cpython_slot_table_ptr_is_valid(member) {
                            break;
                        }
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
                if let Some(method_def) = cpython_slot_richcompare_method_def(attr_name)
                    && unsafe { !(*current).tp_richcompare.is_null() }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=tp_richcompare_bound_slot_wrapper object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_unary_method_def(attr_name)
                    && unsafe { cpython_slot_unary_available(current, attr_name) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=tp_unary_bound_slot_wrapper object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_getitem_method_def(attr_name)
                    && unsafe { cpython_slot_getitem_available(current) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=tp_getitem_bound_slot_wrapper object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_len_iter_setitem_method_def(attr_name)
                    && unsafe { cpython_slot_len_iter_setitem_available(current, attr_name) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            let branch = match attr_name {
                                "__len__" => "tp_len_bound_slot_wrapper",
                                "__iter__" => "tp_iter_bound_slot_wrapper",
                                "__setitem__" => "tp_setitem_bound_slot_wrapper",
                                _ => "tp_misc_bound_slot_wrapper",
                            };
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch={} object={:p} current={:p} value_ptr={:p}",
                                attr_name, branch, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_repr_str_method_def(attr_name)
                    && unsafe { cpython_slot_repr_str_available(current, attr_name) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            let branch = match attr_name {
                                "__repr__" => "tp_repr_bound_slot_wrapper",
                                "__str__" => "tp_str_bound_slot_wrapper",
                                _ => "tp_repr_str_bound_slot_wrapper",
                            };
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch={} object={:p} current={:p} value_ptr={:p}",
                                attr_name, branch, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_descriptor_method_def(attr_name)
                    && unsafe { cpython_slot_descriptor_available(current, attr_name) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=tp_descriptor_bound_slot_wrapper object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
            }
            if is_type_object {
                // For class-level attribute lookup, materialize descriptor
                // objects instead of invoking instance getters/setters.
                let methods_ptr = unsafe { (*current).tp_methods }.cast::<CpythonMethodDef>();
                if Self::cpython_slot_table_ptr_is_valid(methods_ptr) {
                    let mut method = methods_ptr;
                    for _ in 0..4096 {
                        if !Self::cpython_slot_table_ptr_is_valid(method) {
                            break;
                        }
                        let method_name_ptr = unsafe { (*method).ml_name };
                        if method_name_ptr.is_null() {
                            break;
                        }
                        let method_name = unsafe { CStr::from_ptr(method_name_ptr) }
                            .to_str()
                            .ok()
                            .unwrap_or("");
                        if method_name == attr_name {
                            let descriptor = unsafe {
                                PyDescr_NewMethod(current.cast::<c_void>(), method.cast::<c_void>())
                            };
                            if !descriptor.is_null() {
                                if trace_lookup_branch {
                                    eprintln!(
                                        "[proxy-lookup-branch] attr={} branch=tp_methods_descriptor object={:p} current={:p} value_ptr={:p}",
                                        attr_name, object, current, descriptor
                                    );
                                }
                                return Some(descriptor);
                            }
                        }
                        method = unsafe { method.add(1) };
                    }
                }
                let getset_ptr = unsafe { (*current).tp_getset }.cast::<CpythonGetSetDef>();
                if Self::cpython_slot_table_ptr_is_valid(getset_ptr) {
                    let mut getset = getset_ptr;
                    for _ in 0..4096 {
                        if !Self::cpython_slot_table_ptr_is_valid(getset) {
                            break;
                        }
                        let getset_name_ptr = unsafe { (*getset).name };
                        if getset_name_ptr.is_null() {
                            break;
                        }
                        let getset_name = unsafe { CStr::from_ptr(getset_name_ptr) }
                            .to_str()
                            .ok()
                            .unwrap_or("");
                        if getset_name == attr_name {
                            let descriptor = unsafe {
                                PyDescr_NewGetSet(current.cast::<c_void>(), getset.cast::<c_void>())
                            };
                            if !descriptor.is_null() {
                                if trace_lookup_branch {
                                    eprintln!(
                                        "[proxy-lookup-branch] attr={} branch=tp_getset_descriptor object={:p} current={:p} value_ptr={:p}",
                                        attr_name, object, current, descriptor
                                    );
                                }
                                return Some(descriptor);
                            }
                        }
                        getset = unsafe { getset.add(1) };
                    }
                }
                let members_ptr = unsafe { (*current).tp_members }.cast::<CpythonMemberDef>();
                if Self::cpython_slot_table_ptr_is_valid(members_ptr) {
                    let mut member = members_ptr;
                    for _ in 0..4096 {
                        if !Self::cpython_slot_table_ptr_is_valid(member) {
                            break;
                        }
                        let member_name_ptr = unsafe { (*member).name };
                        if member_name_ptr.is_null() {
                            break;
                        }
                        let member_name = unsafe { CStr::from_ptr(member_name_ptr) }
                            .to_str()
                            .ok()
                            .unwrap_or("");
                        if member_name == attr_name {
                            let descriptor = unsafe {
                                PyDescr_NewMember(current.cast::<c_void>(), member.cast::<c_void>())
                            };
                            if !descriptor.is_null() {
                                if trace_lookup_branch {
                                    eprintln!(
                                        "[proxy-lookup-branch] attr={} branch=tp_members_descriptor object={:p} current={:p} value_ptr={:p}",
                                        attr_name, object, current, descriptor
                                    );
                                }
                                return Some(descriptor);
                            }
                        }
                        member = unsafe { member.add(1) };
                    }
                }
                if let Some(method_def) = cpython_slot_richcompare_method_def(attr_name)
                    && unsafe { !(*current).tp_richcompare.is_null() }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        object,
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=tp_richcompare_slot_wrapper object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_repr_str_method_def(attr_name)
                    && unsafe { cpython_slot_repr_str_available(current, attr_name) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            let branch = match attr_name {
                                "__repr__" => "tp_repr_slot_wrapper",
                                "__str__" => "tp_str_slot_wrapper",
                                _ => "tp_repr_str_slot_wrapper",
                            };
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch={} object={:p} current={:p} value_ptr={:p}",
                                attr_name, branch, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
            }
            if attr_name == "__init__" && unsafe { !(*current).tp_init.is_null() } {
                let method_def = cpython_slot_init_method_def();
                let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                    method_def,
                    object,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                if !callable_ptr.is_null() {
                    if trace_lookup_branch {
                        eprintln!(
                            "[proxy-lookup-branch] attr={} branch=tp_init_slot_wrapper object={:p} current={:p} value_ptr={:p}",
                            attr_name, object, current, callable_ptr
                        );
                    }
                    return Some(callable_ptr);
                }
            }
            if !is_type_object {
                if let Some(method_def) = cpython_slot_richcompare_method_def(attr_name)
                    && unsafe { !(*current).tp_richcompare.is_null() }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=tp_richcompare_bound_slot_wrapper_fallback object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_unary_method_def(attr_name)
                    && unsafe { cpython_slot_unary_available(current, attr_name) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=tp_unary_bound_slot_wrapper_fallback object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_getitem_method_def(attr_name)
                    && unsafe { cpython_slot_getitem_available(current) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=tp_getitem_bound_slot_wrapper_fallback object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_len_iter_setitem_method_def(attr_name)
                    && unsafe { cpython_slot_len_iter_setitem_available(current, attr_name) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            let branch = match attr_name {
                                "__len__" => "tp_len_bound_slot_wrapper_fallback",
                                "__iter__" => "tp_iter_bound_slot_wrapper_fallback",
                                "__setitem__" => "tp_setitem_bound_slot_wrapper_fallback",
                                _ => "tp_misc_bound_slot_wrapper_fallback",
                            };
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch={} object={:p} current={:p} value_ptr={:p}",
                                attr_name, branch, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_repr_str_method_def(attr_name)
                    && unsafe { cpython_slot_repr_str_available(current, attr_name) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            let branch = match attr_name {
                                "__repr__" => "tp_repr_bound_slot_wrapper_fallback",
                                "__str__" => "tp_str_bound_slot_wrapper_fallback",
                                _ => "tp_repr_str_bound_slot_wrapper_fallback",
                            };
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch={} object={:p} current={:p} value_ptr={:p}",
                                attr_name, branch, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
            } else {
                if let Some(method_def) = cpython_slot_richcompare_method_def(attr_name)
                    && unsafe { !(*current).tp_richcompare.is_null() }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        object,
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch=tp_richcompare_slot_wrapper_fallback object={:p} current={:p} value_ptr={:p}",
                                attr_name, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
                if let Some(method_def) = cpython_slot_repr_str_method_def(attr_name)
                    && unsafe { cpython_slot_repr_str_available(current, attr_name) }
                {
                    let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                        method_def,
                        object,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if !callable_ptr.is_null() {
                        if trace_lookup_branch {
                            let branch = match attr_name {
                                "__repr__" => "tp_repr_slot_wrapper_fallback",
                                "__str__" => "tp_str_slot_wrapper_fallback",
                                _ => "tp_repr_str_slot_wrapper_fallback",
                            };
                            eprintln!(
                                "[proxy-lookup-branch] attr={} branch={} object={:p} current={:p} value_ptr={:p}",
                                attr_name, branch, object, current, callable_ptr
                            );
                        }
                        return Some(callable_ptr);
                    }
                }
            }
            if attr_name == "__init__" && unsafe { !(*current).tp_init.is_null() } {
                let method_def = cpython_slot_init_method_def();
                let callable_ptr = self.alloc_cpython_method_cfunction_ptr(
                    method_def,
                    object,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                if !callable_ptr.is_null() {
                    if trace_lookup_branch {
                        eprintln!(
                            "[proxy-lookup-branch] attr={} branch=tp_init_slot_wrapper_fallback object={:p} current={:p} value_ptr={:p}",
                            attr_name, object, current, callable_ptr
                        );
                    }
                    return Some(callable_ptr);
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
        fn walk_recursive(class: &ObjRef, out: &mut Vec<ObjRef>, seen: &mut HashSet<u64>) {
            let class_kind = class.kind();
            let class_data = match &*class_kind {
                Object::Class(class_data) => class_data,
                _ => return,
            };
            if !class_data.mro.is_empty() {
                for entry in &class_data.mro {
                    if seen.insert(entry.id()) {
                        out.push(entry.clone());
                    }
                }
                return;
            }
            if !seen.insert(class.id()) {
                return;
            }
            out.push(class.clone());
            for base in &class_data.bases {
                walk_recursive(base, out, seen);
            }
        }

        let mut out = Vec::new();
        let mut seen: HashSet<u64> = HashSet::new();
        walk_recursive(class, &mut out, &mut seen);
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
        let trace_vectorcall = super::env_var_present_cached("PYRS_TRACE_CPY_VECTORCALL");
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
        self.sync_owned_compat_storage_from_raw();
        Some(result)
    }

    fn try_native_tp_call(
        &mut self,
        callable: *mut c_void,
        args: &[Value],
        kwargs: &HashMap<String, Value>,
    ) -> Option<*mut c_void> {
        const MIN_VALID_PTR: usize = MIN_VALID_PTR_THRESHOLD;
        let trace_calls = super::env_var_present_cached("PYRS_TRACE_CPY_CALLS");
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
        if super::env_var_present_cached("PYRS_TRACE_CPY_NONE_CALL")
            && callable == std::ptr::addr_of_mut!(_Py_NoneStruct).cast::<c_void>()
        {
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
                "[cpy-none-call] args=[{}] kwargs=[{}]",
                arg_summary, kw_summary
            );
            if super::env_var_present_cached("PYRS_TRACE_CPY_NONE_CALL_BT") {
                eprintln!(
                    "[cpy-none-call-bt]\n{:?}",
                    std::backtrace::Backtrace::force_capture()
                );
            }
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
        let trace_seed_calls = if super::env_var_present_cached("PYRS_TRACE_NUMPY_SEED_CALLS") {
            let callable_name = if cpython_is_type_object_ptr(callable) {
                // SAFETY: type-object pointer shape is validated by `cpython_is_type_object_ptr`.
                unsafe {
                    c_name_to_string((*callable.cast::<CpythonTypeObject>()).tp_name)
                        .unwrap_or_else(|_| "<invalid>".to_string())
                }
            } else {
                // SAFETY: `type_ptr` is validated above.
                unsafe {
                    c_name_to_string((*type_ptr).tp_name)
                        .unwrap_or_else(|_| "<invalid>".to_string())
                }
            };
            callable_name.contains("SeedSequence")
                || callable_name.contains("BitGenerator")
                || callable_name.contains("MT19937")
                || callable_name.contains("RandomState")
        } else {
            false
        };
        let trace_numpy_ufunc_call = super::env_var_present_cached("PYRS_TRACE_NUMPY_UFUNC_CALL");
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
            let allow_owned_type_call = self.cpython_known_type_ptrs.contains(&(callable as usize))
                || self
                    .cpython_value_from_ptr(callable)
                    .is_some_and(|value| matches!(value, Value::Class(_)));
            if !allow_owned_type_call {
                if trace_calls {
                    let mapped = self
                        .cpython_value_from_ptr(callable)
                        .map(|value| cpython_value_debug_tag(&value))
                        .unwrap_or_else(|| "<none>".to_string());
                    eprintln!(
                        "[cpy-call] skip native callable={:p} reason=owned-compat-type-object mapped={}",
                        callable, mapped
                    );
                }
                return None;
            }
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
            let type_name = unsafe {
                c_name_to_string((*type_ptr).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
            };
            eprintln!(
                "[cpy-call] native callable={:p} type={:p}({}) tp_call={:p} args={} kwargs={}",
                callable,
                type_ptr,
                type_name,
                tp_call_raw,
                args.len(),
                kwargs.len()
            );
        }
        let result = unsafe { call(callable, args_ptr, kwargs_ptr) };
        self.sync_owned_compat_storage_from_raw();
        if result.is_null() && unsafe { PyErr_Occurred() }.is_null() {
            let type_name = unsafe {
                c_name_to_string((*type_ptr).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
            };
            self.set_error(format!(
                "SystemError: NULL result without error in native tp_call ({type_name})"
            ));
        }
        if trace_seed_calls {
            let none_ptr = std::ptr::addr_of_mut!(_Py_NoneStruct).cast::<c_void>();
            let type_name = unsafe {
                c_name_to_string((*type_ptr).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
            };
            eprintln!(
                "[cpy-seed-call] callable={:p} type={} args={} kwargs={} result={:p} is_none_result={}",
                callable,
                type_name,
                args.len(),
                kwargs.len(),
                result,
                result == none_ptr
            );
        }
        Some(result)
    }

    fn identity_object_id(value: &Value) -> Option<u64> {
        match value {
            Value::List(obj)
            | Value::Tuple(obj)
            | Value::Dict(obj)
            | Value::DictKeys(obj)
            | Value::DictValues(obj)
            | Value::DictItems(obj)
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

    fn should_keep_identity_cpython_wrapper(&self, value: &Value) -> bool {
        matches!(
            value,
            Value::Function(_) | Value::BoundMethod(_) | Value::Module(_)
        ) || matches!(
            value,
            Value::Instance(instance_obj) if self.instance_class_defines_call(instance_obj)
        )
    }

    fn instance_class_defines_call(&self, instance_obj: &ObjRef) -> bool {
        let Object::Instance(instance_data) = &*instance_obj.kind() else {
            return false;
        };
        let Object::Class(class_data) = &*instance_data.class.kind() else {
            return false;
        };
        if class_data.attrs.contains_key("__call__") {
            return true;
        }
        class_data.mro.iter().any(|base| {
            matches!(
                &*base.kind(),
                Object::Class(base_data) if base_data.attrs.contains_key("__call__")
            )
        })
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

    fn persist_escaped_ptr_value(&self, vm: &mut Vm, handle: PyrsObjectHandle, raw_ptr: usize) {
        let Some(slot) = self.objects.get(&handle) else {
            return;
        };
        if let Some(object_id) = Self::identity_object_id(&slot.value) {
            vm.extension_cpython_ptr_by_object_id
                .insert(object_id, raw_ptr);
        }
        // `Value::Code` carries `Rc<CodeObject>` references that are frame-scoped and
        // should not be published into cross-context pointer caches.
        if !matches!(slot.value, Value::Code(_)) {
            vm.extension_cpython_ptr_value_set(raw_ptr, &slot.value);
            vm.register_cpython_proxy_instance_drop_hook(
                &slot.value,
                raw_ptr as *mut c_void,
                CpythonProxyPtrOwnership::OwnedCompat,
            );
        }
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
        let value = match vm.import_module_value_sync(module_name) {
            Ok(value) => value,
            Err(err) => {
                // Preserve the active exception type for C-API consumers that
                // branch on ImportError subclasses (for example optional
                // imports in Cython extension module init paths).
                let detail = vm.runtime_error_from_active_exception(&err.message).message;
                let message = if detail.is_empty() {
                    err.message
                } else {
                    detail
                };
                return Err(message);
            }
        };
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
            self.sync_cpython_header_refcount(handle);
            return Ok(());
        }
        if let Some(slot) = self.capsules.get_mut(&handle) {
            slot.refcount = slot.refcount.saturating_add(1);
            self.sync_cpython_header_refcount(handle);
            return Ok(());
        }
        Err(format!("invalid object handle {}", handle))
    }

    fn is_frame_module_value(value: &Value) -> bool {
        matches!(
            value,
            Value::Module(module_obj)
                if matches!(&*module_obj.kind(), Object::Module(module_data) if module_data.name == CPY_FRAME_MODULE_NAME)
        )
    }

    fn release_frame_object_handle(&mut self, handle: PyrsObjectHandle) {
        let Some(slot) = self.objects.remove(&handle) else {
            return;
        };
        if let Some(object_id) = Self::identity_object_id(&slot.value)
            && self.cpython_object_handles_by_id.get(&object_id).copied() == Some(handle)
        {
            self.cpython_object_handles_by_id.remove(&object_id);
        }
        self.cpython_sync_in_progress.remove(&handle);
        self.cpython_list_items_cache_by_handle.remove(&handle);
        self.cpython_tuple_items_cache_by_handle.remove(&handle);
        self.module_dict_handles.remove(&handle);
        if self.thread_state_dict_handle == Some(handle) {
            self.thread_state_dict_handle = None;
        }
        if self.interpreter_state_dict_handle == Some(handle) {
            self.interpreter_state_dict_handle = None;
        }
        if let Some(ptr) = self.cpython_ptr_by_handle.remove(&handle) {
            let ptr_addr = ptr as usize;
            self.cpython_objects_by_ptr.remove(&ptr_addr);
            if let Some(index) = self
                .cpython_allocations
                .iter()
                .position(|owned| owned.cast::<c_void>() == ptr)
            {
                self.cpython_allocations.swap_remove(index);
            }
            self.cpython_known_type_ptrs.remove(&ptr_addr);
            self.cpython_descriptors.remove(&ptr_addr);
            if let Some((buffer, _)) = self.cpython_list_buffers.remove(&handle)
                && !buffer.is_null()
            {
                if self.capi_owned_ptr_prepare_for_free(buffer.cast()) {
                    // SAFETY: list buffer pointer was allocated through C allocator.
                    unsafe {
                        free(buffer.cast());
                    }
                    self.capi_owned_ptr_mark_freed(buffer.cast(), "frame-release list-buffer");
                }
            }
            // SAFETY: frame pointer was created by PyFrame_New and points to frame-compatible
            // storage with referenced object pointers that need balanced decref.
            let mut frame_freed = false;
            unsafe {
                let raw_frame = ptr.cast::<CpythonFrameCompatObject>();
                let back = (*raw_frame).f_back;
                let trace = (*raw_frame).f_trace;
                let code = (*raw_frame).f_code;
                let globals = (*raw_frame).f_globals;
                let locals = (*raw_frame).f_locals;
                (*raw_frame).f_back = std::ptr::null_mut();
                (*raw_frame).f_trace = std::ptr::null_mut();
                (*raw_frame).f_code = std::ptr::null_mut();
                (*raw_frame).f_globals = std::ptr::null_mut();
                (*raw_frame).f_locals = std::ptr::null_mut();
                Py_XDecRef(back);
                Py_XDecRef(trace);
                Py_XDecRef(code);
                Py_XDecRef(globals);
                Py_XDecRef(locals);
                if self.capi_owned_ptr_prepare_for_free(ptr) {
                    free(ptr);
                    self.capi_owned_ptr_mark_freed(ptr, "frame-release frame");
                    frame_freed = true;
                }
            }
            if frame_freed && !self.vm.is_null() {
                // SAFETY: VM pointer is valid for active C-API context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.extension_cpython_ptr_value_remove(ptr_addr);
                if let Some(object_id) = Self::identity_object_id(&slot.value)
                    && vm
                        .extension_cpython_ptr_by_object_id
                        .get(&object_id)
                        .copied()
                        == Some(ptr_addr)
                {
                    vm.extension_cpython_ptr_by_object_id.remove(&object_id);
                }
            }
        }
    }

    fn forget_released_object_handle(
        &mut self,
        handle: PyrsObjectHandle,
    ) -> Option<(CapiObjectSlot, *mut c_void)> {
        let slot = self.objects.remove(&handle)?;
        if let Some(object_id) = Self::identity_object_id(&slot.value)
            && self.cpython_object_handles_by_id.get(&object_id).copied() == Some(handle)
        {
            self.cpython_object_handles_by_id.remove(&object_id);
        }
        self.cpython_sync_in_progress.remove(&handle);
        self.cpython_list_items_cache_by_handle.remove(&handle);
        self.cpython_tuple_items_cache_by_handle.remove(&handle);
        self.module_dict_handles.remove(&handle);
        if self.thread_state_dict_handle == Some(handle) {
            self.thread_state_dict_handle = None;
        }
        if self.interpreter_state_dict_handle == Some(handle) {
            self.interpreter_state_dict_handle = None;
        }
        let ptr = self
            .cpython_ptr_by_handle
            .remove(&handle)
            .unwrap_or(std::ptr::null_mut());
        if !ptr.is_null() {
            let ptr_addr = ptr as usize;
            self.cpython_objects_by_ptr.remove(&ptr_addr);
            if let Some(index) = self
                .cpython_allocations
                .iter()
                .position(|owned| owned.cast::<c_void>() == ptr)
            {
                self.cpython_allocations.swap_remove(index);
            }
            self.cpython_known_type_ptrs.remove(&ptr_addr);
            self.cpython_descriptors.remove(&ptr_addr);
            if let Some((buffer, _)) = self.cpython_list_buffers.remove(&handle)
                && !buffer.is_null()
                && self.capi_owned_ptr_prepare_for_free(buffer.cast())
            {
                unsafe {
                    free(buffer.cast());
                }
                self.capi_owned_ptr_mark_freed(buffer.cast(), "released-object list-buffer");
            }
            if let Some((buffer, _)) = self.cpython_bytearray_buffers.remove(&handle)
                && !buffer.is_null()
                && self.capi_owned_ptr_prepare_for_free(buffer.cast())
            {
                unsafe {
                    free(buffer.cast());
                }
                self.capi_owned_ptr_mark_freed(buffer.cast(), "released-object bytearray-buffer");
            }
            if !self.vm.is_null() {
                let vm = unsafe { &mut *self.vm };
                vm.extension_cpython_ptr_value_remove(ptr_addr);
                if let Some(object_id) = Self::identity_object_id(&slot.value)
                    && vm
                        .extension_cpython_ptr_by_object_id
                        .get(&object_id)
                        .copied()
                        == Some(ptr_addr)
                {
                    vm.extension_cpython_ptr_by_object_id.remove(&object_id);
                }
            }
        }
        Some((slot, ptr))
    }

    fn release_object_handle_after_zero_ref(
        &mut self,
        handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let ptr = self
            .cpython_ptr_by_handle
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("invalid object handle {}", handle))?;
        if ptr.is_null() {
            let _ = self.forget_released_object_handle(handle);
            return Ok(());
        }
        let header_refcount = unsafe { ptr.cast::<CpythonObjectHead>().as_ref() }
            .map(|head| head.ob_refcnt.max(0) as usize)
            .unwrap_or(0);
        if header_refcount > 0 {
            self.set_object_refcount(handle, header_refcount);
            self.sync_cpython_header_refcount(handle);
            return Ok(());
        }
        if !self.vm.is_null() {
            let vm = unsafe { &mut *self.vm };
            if !vm.capi_registry_is_gc_finalized(ptr as usize) {
                unsafe {
                    PyObject_CallFinalizerFromDealloc(ptr);
                }
                let _ = vm.capi_registry_set_gc_finalized(ptr as usize, true);
            }
        }
        if self.owns_cpython_allocation_ptr(ptr) {
            self.sync_cpython_storage_from_value(handle);
        }
        let resurrected = unsafe {
            ptr.cast::<CpythonObjectHead>()
                .as_ref()
                .is_some_and(|head| head.ob_refcnt > 0)
        };
        if resurrected {
            let resurrected_refcount = unsafe {
                ptr.cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_refcnt.max(0) as usize)
                    .unwrap_or(1)
            };
            self.set_object_refcount(handle, resurrected_refcount);
            self.sync_cpython_header_refcount(handle);
            if !self.vm.is_null() {
                let vm = unsafe { &mut *self.vm };
                vm.capi_registry_mark_alive(ptr as usize);
            }
            return Ok(());
        }

        let type_ptr = unsafe {
            ptr.cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if !type_ptr.is_null() {
            let tp_dealloc = unsafe { (*type_ptr).tp_dealloc };
            if !tp_dealloc.is_null() {
                let dealloc: unsafe extern "C" fn(*mut c_void) =
                    unsafe { std::mem::transmute(tp_dealloc) };
                unsafe {
                    dealloc(ptr);
                }
            } else {
                let tp_free = unsafe { (*type_ptr).tp_free };
                if !tp_free.is_null() {
                    let free_fn: unsafe extern "C" fn(*mut c_void) =
                        unsafe { std::mem::transmute(tp_free) };
                    unsafe {
                        free_fn(ptr);
                    }
                } else {
                    unsafe {
                        free(ptr);
                    }
                }
                if unsafe { ((*type_ptr).tp_flags & PY_TPFLAGS_HEAPTYPE) != 0 } {
                    unsafe {
                        Py_DecRef(type_ptr.cast::<c_void>());
                    }
                }
            }
        } else {
            unsafe {
                free(ptr);
            }
        }

        let _ = self.forget_released_object_handle(handle);
        if self.owns_cpython_allocation_ptr(ptr) {
            self.capi_owned_ptr_mark_freed(ptr, "released-object compat");
        }
        Ok(())
    }

    fn decref(&mut self, handle: PyrsObjectHandle) -> Result<(), String> {
        let is_frame = self
            .objects
            .get(&handle)
            .is_some_and(|slot| Self::is_frame_module_value(&slot.value));
        if is_frame {
            let mut should_release = false;
            if let Some(slot) = self.objects.get_mut(&handle) {
                if slot.refcount > 0 {
                    slot.refcount -= 1;
                }
                should_release = slot.refcount == 0;
            }
            if should_release {
                self.release_frame_object_handle(handle);
            } else {
                self.sync_cpython_header_refcount(handle);
            }
            return Ok(());
        }
        if let Some(slot) = self.objects.get_mut(&handle) {
            if slot.refcount > 0 {
                slot.refcount -= 1;
            }
            let should_release = slot.refcount == 0;
            if !should_release {
                self.sync_cpython_header_refcount(handle);
                return Ok(());
            }
        }
        if self.objects.contains_key(&handle) {
            return self.release_object_handle_after_zero_ref(handle);
        }
        if let Some(slot) = self.capsules.get_mut(&handle) {
            if !self.strict_capsule_refcount {
                if slot.refcount > 1 {
                    slot.refcount -= 1;
                }
                self.sync_cpython_header_refcount(handle);
                return Ok(());
            }
            if slot.refcount == 0 {
                self.capsules.remove(&handle);
                return Ok(());
            }
            slot.refcount -= 1;
            if slot.refcount > 0 {
                self.sync_cpython_header_refcount(handle);
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
            ByteArray(Vec<u8>),
            Str(String),
            Exception {
                args: *mut c_void,
                notes: *mut c_void,
                traceback: *mut c_void,
                suppress_context: bool,
            },
            Module {
                module: ObjRef,
                md_dict: *mut c_void,
                md_def: *mut CpythonModuleDef,
                md_state: *mut c_void,
            },
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
                Value::ByteArray(_) => {
                    // SAFETY: `ptr` is an owned bytearray-compatible allocation for this handle.
                    let bytes = unsafe {
                        let raw = ptr.cast::<CpythonByteArrayCompatObject>();
                        let len = (*raw).ob_base.ob_size.max(0) as usize;
                        let data = (*raw).ob_start;
                        if data.is_null() || len == 0 {
                            Vec::new()
                        } else {
                            std::slice::from_raw_parts(data.cast::<u8>(), len).to_vec()
                        }
                    };
                    Some(SyncPayload::ByteArray(bytes))
                }
                Value::Str(_) => self
                    .unicode_text_from_raw_storage(ptr)
                    .map(SyncPayload::Str),
                Value::Exception(_) => {
                    // SAFETY: `ptr` is an owned base-exception-compatible allocation for this handle.
                    let raw_exception =
                        unsafe { ptr.cast::<CpythonBaseExceptionCompatObject>().as_ref() };
                    raw_exception.map(|raw_exception| SyncPayload::Exception {
                        args: raw_exception.args,
                        notes: raw_exception.notes,
                        traceback: raw_exception.traceback,
                        suppress_context: raw_exception.suppress_context != 0,
                    })
                }
                Value::Instance(_) if self.value_is_exception_instance_like(&slot.value) => {
                    // SAFETY: `ptr` is an owned base-exception-compatible allocation for this handle.
                    let raw_exception =
                        unsafe { ptr.cast::<CpythonBaseExceptionCompatObject>().as_ref() };
                    raw_exception.map(|raw_exception| SyncPayload::Exception {
                        args: raw_exception.args,
                        notes: raw_exception.notes,
                        traceback: raw_exception.traceback,
                        suppress_context: raw_exception.suppress_context != 0,
                    })
                }
                Value::Module(module_obj) => {
                    // SAFETY: `ptr` is an owned module-compatible allocation for this handle.
                    let module_raw = unsafe { ptr.cast::<CpythonModuleCompatObject>().as_ref() };
                    module_raw.map(|module_raw| SyncPayload::Module {
                        module: module_obj.clone(),
                        md_dict: module_raw.md_dict,
                        md_def: module_raw.md_def,
                        md_state: module_raw.md_state,
                    })
                }
                _ => None,
            }
        } else {
            None
        };

        match payload {
            Some(SyncPayload::Tuple(item_ptrs)) => {
                let raw_items: Vec<usize> = item_ptrs.iter().map(|ptr| *ptr as usize).collect();
                if self
                    .cpython_tuple_items_cache_by_handle
                    .get(&handle)
                    .is_some_and(|cached| *cached == raw_items)
                {
                    self.cpython_sync_in_progress.remove(&handle);
                    return;
                }
                self.cpython_tuple_items_cache_by_handle
                    .insert(handle, raw_items);
                let trace_raw = super::env_var_present_cached("PYRS_TRACE_CPY_TUPLE_RAW");
                let mut values = Vec::with_capacity(item_ptrs.len());
                let mut fallback_indices = Vec::new();
                for (idx, item_ptr) in item_ptrs.iter().copied().enumerate() {
                    if item_ptr.is_null() {
                        if trace_raw {
                            eprintln!(
                                "[cpy-sync-tuple] handle={} tuple_ptr={:p} idx={} item_ptr=<null> value=None",
                                handle, ptr, idx
                            );
                        }
                        fallback_indices.push(idx);
                        values.push(Value::None);
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
                            fallback_indices.push(idx);
                            values.push(Value::None)
                        }
                    }
                }
                if !fallback_indices.is_empty()
                    && let Some(existing_values) =
                        self.objects
                            .get(&handle)
                            .and_then(|slot| match &slot.value {
                                Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                                    Object::Tuple(items) => Some(items.clone()),
                                    _ => None,
                                },
                                _ => None,
                            })
                {
                    for idx in fallback_indices {
                        if let Some(existing) = existing_values.get(idx) {
                            values[idx] = existing.clone();
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
                let raw_items: Vec<usize> = item_ptrs.iter().map(|ptr| *ptr as usize).collect();
                if self
                    .cpython_list_items_cache_by_handle
                    .get(&handle)
                    .is_some_and(|cached| *cached == raw_items)
                {
                    self.cpython_sync_in_progress.remove(&handle);
                    return;
                }
                self.cpython_list_items_cache_by_handle
                    .insert(handle, raw_items);
                let mut values = Vec::with_capacity(item_ptrs.len());
                let mut fallback_indices = Vec::new();
                for (idx, item_ptr) in item_ptrs.into_iter().enumerate() {
                    if item_ptr.is_null() {
                        fallback_indices.push(idx);
                        values.push(Value::None);
                        continue;
                    }
                    match self.cpython_value_from_ptr_or_proxy(item_ptr) {
                        Some(value) => values.push(value),
                        None => {
                            fallback_indices.push(idx);
                            values.push(Value::None)
                        }
                    }
                }
                if !fallback_indices.is_empty()
                    && let Some(existing_values) =
                        self.objects
                            .get(&handle)
                            .and_then(|slot| match &slot.value {
                                Value::List(list_obj) => match &*list_obj.kind() {
                                    Object::List(items) => Some(items.clone()),
                                    _ => None,
                                },
                                _ => None,
                            })
                {
                    for idx in fallback_indices {
                        if let Some(existing) = existing_values.get(idx) {
                            values[idx] = existing.clone();
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
            Some(SyncPayload::ByteArray(bytes)) => {
                if let Some(slot) = self.objects.get_mut(&handle) {
                    if let Value::ByteArray(bytes_obj) = &mut slot.value
                        && let Object::ByteArray(values) = &mut *bytes_obj.kind_mut()
                    {
                        *values = bytes;
                    }
                }
            }
            Some(SyncPayload::Str(text)) => {
                if let Some(slot) = self.objects.get_mut(&handle)
                    && matches!(slot.value, Value::Str(_))
                {
                    slot.value = Value::Str(text);
                }
            }
            Some(SyncPayload::Exception {
                args,
                notes,
                traceback,
                suppress_context,
            }) => {
                let args_value = if args.is_null() {
                    None
                } else {
                    self.cpython_value_from_ptr_or_proxy(args)
                };
                let notes_value = if notes.is_null() {
                    None
                } else {
                    self.cpython_value_from_ptr_or_proxy(notes)
                };
                let traceback_value = if traceback.is_null() {
                    None
                } else {
                    self.cpython_value_from_ptr_or_proxy(traceback)
                };
                if let Some(slot) = self.objects.get_mut(&handle) {
                    match &mut slot.value {
                        Value::Exception(exception_obj) => {
                            let mut attrs = exception_obj.attrs.borrow_mut();
                            if let Some(args_value) = args_value.clone() {
                                attrs.insert("args".to_string(), args_value);
                            }
                            if let Some(traceback_value) = traceback_value.clone() {
                                attrs.insert("__traceback__".to_string(), traceback_value.clone());
                                attrs.insert("exc_traceback".to_string(), traceback_value);
                            }
                            if let Some(notes_value) = notes_value {
                                attrs.insert("__notes__".to_string(), notes_value);
                            }
                            attrs.insert(
                                "__suppress_context__".to_string(),
                                Value::Bool(suppress_context),
                            );
                            exception_obj.suppress_context = suppress_context;
                        }
                        Value::Instance(instance_obj) => {
                            if let Object::Instance(instance_data) = &mut *instance_obj.kind_mut() {
                                if let Some(args_value) = args_value.clone() {
                                    instance_data.attrs.insert("args".to_string(), args_value);
                                }
                                if let Some(traceback_value) = traceback_value.clone() {
                                    instance_data.attrs.insert(
                                        "__traceback__".to_string(),
                                        traceback_value.clone(),
                                    );
                                    instance_data
                                        .attrs
                                        .insert("exc_traceback".to_string(), traceback_value);
                                }
                                if let Some(notes_value) = notes_value {
                                    instance_data
                                        .attrs
                                        .insert("__notes__".to_string(), notes_value);
                                }
                                instance_data.attrs.insert(
                                    "__suppress_context__".to_string(),
                                    Value::Bool(suppress_context),
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some(SyncPayload::Module {
                module,
                md_dict,
                md_def,
                md_state,
            }) => {
                let mut synced_globals = None;
                if !md_dict.is_null()
                    && let Some(dict_handle) = self.cpython_handle_from_ptr(md_dict)
                {
                    self.module_dict_handles.insert(dict_handle, module.clone());
                    self.module_dict_handle_by_module_id
                        .insert(module.id(), dict_handle);
                    if let Some(slot) = self.objects.get(&dict_handle)
                        && let Value::Dict(dict_obj) = &slot.value
                        && let Object::Dict(entries) = &*dict_obj.kind()
                    {
                        let mut globals = HashMap::new();
                        for (key, value) in entries {
                            if let Value::Str(name) = key {
                                globals.insert(name.clone(), value.clone());
                            }
                        }
                        synced_globals = Some(globals);
                    }
                }
                if let Some(globals) = synced_globals
                    && let Object::Module(module_data) = &mut *module.kind_mut()
                {
                    // Merge CPython dict-backed updates without dropping attributes
                    // seeded through module-method registration or bootstrap metadata.
                    for (name, value) in globals {
                        module_data.globals.insert(name, value);
                    }
                }
                if !self.vm.is_null() {
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    let vm = unsafe { &mut *self.vm };
                    if !md_def.is_null() {
                        vm.extension_module_def_registry
                            .insert(module.id(), md_def as usize);
                    }
                    if !md_state.is_null() {
                        let entry = vm
                            .extension_module_state_registry
                            .entry(module.id())
                            .or_insert(ExtensionModuleStateEntry {
                                state: 0,
                                free_func: None,
                                finalize_func: None,
                            });
                        entry.state = md_state as usize;
                    }
                }
            }
            None => {}
        }

        self.cpython_sync_in_progress.remove(&handle);
    }

    fn cpython_handle_requires_storage_sync(&self, handle: PyrsObjectHandle) -> bool {
        let Some(slot) = self.objects.get(&handle) else {
            return false;
        };
        match &slot.value {
            Value::Tuple(_)
            | Value::List(_)
            | Value::ByteArray(_)
            | Value::Str(_)
            | Value::Module(_)
            | Value::Exception(_) => true,
            Value::Instance(_) => self.value_is_exception_instance_like(&slot.value),
            _ => false,
        }
    }

    fn sync_unicode_raw_storage_from_text(&mut self, ptr: *mut c_void, text: &str) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: caller guarantees `ptr` references owned unicode-compatible storage.
        let Some(ascii) = (unsafe { ptr.cast::<CpythonAsciiUnicodeCompatObject>().as_mut() })
        else {
            return;
        };
        let length = ascii.length.max(0) as usize;
        let state = ascii.state;
        let kind = (state >> 2) & 0b111;
        let compact = ((state >> 5) & 1) != 0;
        let is_ascii = ((state >> 6) & 1) != 0;
        if !compact {
            return;
        }
        let codepoints = text.chars().map(|ch| ch as u32).collect::<Vec<_>>();
        if codepoints.len() != length {
            return;
        }
        let max_char = codepoints.iter().copied().max().unwrap_or(0);
        if is_ascii && max_char > 0x7f {
            return;
        }
        // SAFETY: compact unicode objects store data immediately after the header.
        let data_ptr = unsafe {
            if is_ascii {
                ptr.cast::<u8>()
                    .add(std::mem::size_of::<CpythonAsciiUnicodeCompatObject>())
            } else {
                ptr.cast::<u8>()
                    .add(std::mem::size_of::<CpythonCompactUnicodeCompatObject>())
            }
        };
        match kind {
            1 => {
                if max_char > 0xff {
                    return;
                }
                // SAFETY: kind=1 stores one byte per codepoint plus NUL.
                unsafe {
                    for (idx, ch) in codepoints.iter().enumerate() {
                        *data_ptr.add(idx) = *ch as u8;
                    }
                    *data_ptr.add(codepoints.len()) = 0;
                }
            }
            2 => {
                if max_char > 0xffff {
                    return;
                }
                let data = data_ptr.cast::<u16>();
                // SAFETY: kind=2 stores one u16 per codepoint plus NUL.
                unsafe {
                    for (idx, ch) in codepoints.iter().enumerate() {
                        *data.add(idx) = *ch as u16;
                    }
                    *data.add(codepoints.len()) = 0;
                }
            }
            4 => {
                let data = data_ptr.cast::<u32>();
                // SAFETY: kind=4 stores one u32 per codepoint plus NUL.
                unsafe {
                    for (idx, ch) in codepoints.iter().enumerate() {
                        *data.add(idx) = *ch;
                    }
                    *data.add(codepoints.len()) = 0;
                }
            }
            _ => return,
        }
        ascii.hash = cpython_unicode_precomputed_hash(text);
        if !is_ascii {
            // SAFETY: non-ASCII compact strings use this extended header layout.
            if let Some(compact_unicode) =
                unsafe { ptr.cast::<CpythonCompactUnicodeCompatObject>().as_mut() }
            {
                compact_unicode.utf8_length = 0;
                compact_unicode.utf8 = std::ptr::null_mut();
            }
        }
    }

    fn unicode_text_from_raw_storage(&self, ptr: *mut c_void) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        // SAFETY: caller guarantees `ptr` references owned unicode-compatible storage.
        let ascii = unsafe { ptr.cast::<CpythonAsciiUnicodeCompatObject>().as_ref()? };
        let length = ascii.length.max(0) as usize;
        let state = ascii.state;
        let kind = (state >> 2) & 0b111;
        let compact = ((state >> 5) & 1) != 0;
        let is_ascii = ((state >> 6) & 1) != 0;
        if !compact {
            return None;
        }
        // SAFETY: compact unicode objects store data immediately after the header.
        let data_ptr = unsafe {
            if is_ascii {
                ptr.cast::<u8>()
                    .add(std::mem::size_of::<CpythonAsciiUnicodeCompatObject>())
            } else {
                ptr.cast::<u8>()
                    .add(std::mem::size_of::<CpythonCompactUnicodeCompatObject>())
            }
        };
        match kind {
            1 => {
                // SAFETY: kind=1 stores one byte per codepoint.
                let bytes = unsafe { std::slice::from_raw_parts(data_ptr, length) };
                Some(bytes.iter().map(|b| char::from(*b)).collect())
            }
            2 => {
                // SAFETY: kind=2 stores one u16 per codepoint.
                let units = unsafe { std::slice::from_raw_parts(data_ptr.cast::<u16>(), length) };
                Some(
                    units
                        .iter()
                        .filter_map(|unit| char::from_u32(*unit as u32))
                        .collect(),
                )
            }
            4 => {
                // SAFETY: kind=4 stores one u32 per codepoint.
                let units = unsafe { std::slice::from_raw_parts(data_ptr.cast::<u32>(), length) };
                Some(
                    units
                        .iter()
                        .filter_map(|unit| char::from_u32(*unit))
                        .collect(),
                )
            }
            _ => None,
        }
    }

    fn sync_cpython_header_refcount(&mut self, handle: PyrsObjectHandle) {
        let Some(ptr) = self.cpython_ptr_by_handle.get(&handle).copied() else {
            return;
        };
        if !self.is_owned_compat_ptr(ptr) {
            return;
        }
        let raw = ptr.cast::<CpythonCompatObject>();
        if let Some(slot) = self.capsules.get(&handle) {
            // SAFETY: `raw` points to owned capsule-compatible storage for this handle.
            unsafe {
                let raw_capsule = raw.cast::<CpythonCapsuleCompatObject>();
                (*raw_capsule).ob_base.ob_refcnt = slot.refcount.max(1) as isize;
            }
            return;
        }
        if let Some(slot) = self.objects.get(&handle) {
            if matches!(slot.value, Value::Module(_)) {
                // SAFETY: `raw` points to owned module-compatible storage for this handle.
                unsafe {
                    let raw_module = raw.cast::<CpythonModuleCompatObject>();
                    (*raw_module).ob_base.ob_refcnt = slot.refcount.max(1) as isize;
                }
                return;
            }
            // SAFETY: `raw` points to owned allocation with object header-compatible layout.
            unsafe {
                (*raw).ob_base.ob_base.ob_refcnt = slot.refcount.max(1) as isize;
            }
        }
    }

    fn sync_cpython_storage_from_value(&mut self, handle: PyrsObjectHandle) {
        self.sync_cpython_storage_inner(handle, false);
    }

    fn sync_owned_compat_storage_from_raw(&mut self) {
        let handles = self
            .cpython_ptr_by_handle
            .iter()
            .map(|(handle, ptr)| (*handle, *ptr))
            .collect::<Vec<_>>();
        for (handle, ptr) in handles {
            if self.owns_cpython_allocation_ptr(ptr)
                && self.cpython_handle_requires_storage_sync(handle)
            {
                self.sync_value_from_cpython_storage(handle, ptr);
            }
        }
    }

    fn sync_cpython_storage_inner(&mut self, handle: PyrsObjectHandle, pull_from_raw: bool) {
        let Some(ptr) = self.cpython_ptr_by_handle.get(&handle).copied() else {
            return;
        };
        if !self.is_owned_compat_ptr(ptr) {
            return;
        }
        let raw = ptr.cast::<CpythonCompatObject>();
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
        let module_sync = if let Value::Module(module_obj) = &slot.value {
            Some((module_obj.clone(), slot.refcount as isize))
        } else {
            None
        };
        if let Some((module_obj, refcount)) = module_sync {
            // SAFETY: `raw` points to owned module-compatible storage for this handle.
            let raw_module = raw.cast::<CpythonModuleCompatObject>();
            self.sync_module_compat_from_value(raw_module, &module_obj, refcount);
            return;
        }
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
        let long_payload = cpython_long_payload_from_value(&slot.value);
        let bytes_payload = match &slot.value {
            Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(values) => Some(values.clone()),
                _ => None,
            },
            _ => None,
        };
        let bytearray_payload = match &slot.value {
            Value::ByteArray(bytes_obj) => match &*bytes_obj.kind() {
                Object::ByteArray(values) => Some(values.clone()),
                _ => None,
            },
            _ => None,
        };
        let exception_state = self.exception_compat_state_from_value(&slot.value);
        let unicode_text = match &slot.value {
            Value::Str(text) => Some(text.clone()),
            _ => None,
        };
        // SAFETY: `raw` is owned allocation with `CpythonCompatObject` layout.
        unsafe {
            (*raw).ob_base.ob_base.ob_refcnt = slot.refcount.max(1) as isize;
            (*raw).ob_base.ob_base.ob_type = cpython_type_for_value(&slot.value);
            if let Some((lv_tag, digits)) = long_payload.as_ref() {
                let raw_long = raw.cast::<c_void>();
                *cpython_long_lv_tag_ptr(raw_long) = *lv_tag;
                let digits_ptr = cpython_long_digits_ptr(raw_long);
                if digits.is_empty() {
                    *digits_ptr = 0;
                } else {
                    std::ptr::copy_nonoverlapping(digits.as_ptr(), digits_ptr, digits.len());
                }
                return;
            }
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
            if let Some(bytes) = bytearray_payload.as_ref() {
                let raw_bytearray = raw.cast::<CpythonByteArrayCompatObject>();
                let (mut buffer_ptr, mut capacity) = self
                    .cpython_bytearray_buffers
                    .get(&handle)
                    .copied()
                    .unwrap_or((
                        (*raw_bytearray).ob_bytes,
                        (*raw_bytearray).ob_alloc.max(0) as usize,
                    ));
                if !self.cpython_bytearray_buffers.contains_key(&handle) && !buffer_ptr.is_null() {
                    self.cpython_bytearray_buffers
                        .insert(handle, (buffer_ptr, capacity.max(1)));
                    self.capi_registry_register_owned_ptr(buffer_ptr.cast(), None);
                }
                let required = bytes.len().saturating_add(1).max(1);
                if capacity < required {
                    let previous_ptr = buffer_ptr;
                    let previous_was_pinned = if previous_ptr.is_null() || self.vm.is_null() {
                        false
                    } else {
                        // SAFETY: VM pointer is valid for active C-API context lifetime.
                        let vm = &mut *self.vm;
                        vm.capi_owned_ptr_is_pinned(previous_ptr as usize)
                    };
                    let grown = if previous_ptr.is_null() {
                        // SAFETY: allocate mutable bytearray payload storage.
                        malloc(required).cast::<c_char>()
                    } else {
                        // SAFETY: grow mutable bytearray payload storage in place when possible.
                        realloc(previous_ptr.cast(), required).cast::<c_char>()
                    };
                    if grown.is_null() {
                        self.set_error("out of memory resizing CPython bytearray payload");
                        return;
                    }
                    buffer_ptr = grown;
                    capacity = required;
                    if !previous_ptr.is_null() && previous_ptr != buffer_ptr {
                        self.capi_registry_mark_freed_ptr(previous_ptr.cast());
                        if !self.vm.is_null() {
                            // SAFETY: VM pointer is valid for active C-API context lifetime.
                            let vm = &mut *self.vm;
                            if previous_was_pinned {
                                vm.capi_unpin_owned_ptr(previous_ptr as usize);
                            }
                            vm.extension_pinned_capsule_names
                                .remove(&(previous_ptr as usize));
                            vm.capi_registry_mark_freed(previous_ptr as usize);
                        }
                    }
                    if previous_ptr != buffer_ptr {
                        self.capi_registry_register_owned_ptr(buffer_ptr.cast(), None);
                        if previous_was_pinned && !self.vm.is_null() {
                            // SAFETY: VM pointer is valid for active C-API context lifetime.
                            let vm = &mut *self.vm;
                            if vm.capi_pin_owned_ptr(buffer_ptr as usize) {
                                vm.capi_registry_mark_alive(buffer_ptr as usize);
                            }
                        }
                    }
                    self.cpython_bytearray_buffers
                        .insert(handle, (buffer_ptr, capacity));
                } else if buffer_ptr.is_null() {
                    // SAFETY: allocate mutable bytearray payload storage.
                    let allocated = malloc(required).cast::<c_char>();
                    if allocated.is_null() {
                        self.set_error("out of memory allocating CPython bytearray payload");
                        return;
                    }
                    buffer_ptr = allocated;
                    capacity = required;
                    self.capi_registry_register_owned_ptr(buffer_ptr.cast(), None);
                    self.cpython_bytearray_buffers
                        .insert(handle, (buffer_ptr, capacity));
                }
                if !bytes.is_empty() {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        buffer_ptr.cast::<u8>(),
                        bytes.len(),
                    );
                }
                *buffer_ptr.add(bytes.len()) = 0;
                (*raw_bytearray).ob_base.ob_size = bytes.len() as isize;
                (*raw_bytearray).ob_alloc = capacity as isize;
                (*raw_bytearray).ob_bytes = buffer_ptr;
                (*raw_bytearray).ob_start = buffer_ptr;
                return;
            }
            if let Some(text) = unicode_text.as_ref() {
                self.sync_unicode_raw_storage_from_text(raw.cast(), text);
                return;
            }
            if let Some(exception_state) = exception_state.as_ref() {
                let raw_exception = raw.cast::<CpythonBaseExceptionCompatObject>();
                (*raw_exception).dict = std::ptr::null_mut();
                (*raw_exception).args =
                    self.exception_args_tuple_ptr_from_state(exception_state.args.clone());
                (*raw_exception).notes =
                    self.exception_optional_ptr_from_state_value(exception_state.notes.clone());
                (*raw_exception).traceback =
                    self.exception_optional_ptr_from_state_value(exception_state.traceback.clone());
                (*raw_exception).context = std::ptr::null_mut();
                (*raw_exception).cause = std::ptr::null_mut();
                (*raw_exception).suppress_context = if exception_state.suppress_context {
                    1
                } else {
                    0
                };
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
                    let previous_was_pinned = if previous_ptr.is_null() || self.vm.is_null() {
                        false
                    } else {
                        // SAFETY: VM pointer is valid for active C-API context lifetime.
                        let vm = &mut *self.vm;
                        vm.capi_owned_ptr_is_pinned(previous_ptr as usize)
                    };
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
                    if !previous_ptr.is_null() && previous_ptr != buffer_ptr {
                        self.capi_registry_mark_freed_ptr(previous_ptr.cast());
                        if !self.vm.is_null() {
                            // SAFETY: VM pointer is valid for active C-API context lifetime.
                            let vm = &mut *self.vm;
                            if previous_was_pinned {
                                vm.capi_unpin_owned_ptr(previous_ptr as usize);
                            }
                            vm.extension_pinned_capsule_names
                                .remove(&(previous_ptr as usize));
                            vm.capi_registry_mark_freed(previous_ptr as usize);
                        }
                    }
                    if !buffer_ptr.is_null() {
                        self.capi_registry_register_owned_ptr(buffer_ptr.cast(), None);
                        if previous_ptr != buffer_ptr && previous_was_pinned && !self.vm.is_null() {
                            // SAFETY: VM pointer is valid for active C-API context lifetime.
                            let vm = &mut *self.vm;
                            if vm.capi_pin_owned_ptr(buffer_ptr as usize) {
                                vm.capi_registry_mark_alive(buffer_ptr as usize);
                            }
                        }
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

    fn populate_proxy_class_attrs_from_type_dict(
        &mut self,
        proxy_class: &ObjRef,
        type_ptr: *mut c_void,
    ) {
        if type_ptr.is_null() {
            return;
        }
        let trace = super::env_var_present_cached("PYRS_TRACE_PROXY_CLASS_SOURCE");
        // SAFETY: caller gates `type_ptr` to type-object creation flow.
        let type_dict_ptr = unsafe { (*type_ptr.cast::<CpythonTypeObject>()).tp_dict };
        if type_dict_ptr.is_null() {
            if trace {
                eprintln!(
                    "[cpy-proxy] populate class attrs skip type_ptr={:p} reason=null-tp_dict",
                    type_ptr
                );
            }
            return;
        }
        let Some(Value::Dict(dict_obj)) = self.cpython_value_from_ptr(type_dict_ptr) else {
            if trace {
                eprintln!(
                    "[cpy-proxy] populate class attrs skip type_ptr={:p} dict_ptr={:p} reason=non-runtime-dict",
                    type_ptr, type_dict_ptr
                );
            }
            return;
        };
        let Object::Dict(entries) = &*dict_obj.kind() else {
            if trace {
                eprintln!(
                    "[cpy-proxy] populate class attrs skip type_ptr={:p} dict_ptr={:p} reason=invalid-dict-storage",
                    type_ptr, type_dict_ptr
                );
            }
            return;
        };
        let mut updates = Vec::new();
        let mut has_init = false;
        for (key, value) in entries.iter() {
            let Value::Str(name) = key else {
                continue;
            };
            if name == "__init__" {
                has_init = true;
            }
            if matches!(
                name.as_str(),
                CPY_PROXY_MARKER_ATTR
                    | CPY_PROXY_PTR_ATTR
                    | "__name__"
                    | "__qualname__"
                    | "__module__"
                    | "__bases__"
                    | "__mro__"
            ) {
                continue;
            }
            updates.push((name.clone(), value.clone()));
        }
        if updates.is_empty() {
            if trace {
                eprintln!(
                    "[cpy-proxy] populate class attrs skip type_ptr={:p} dict_ptr={:p} reason=no-copyable-entries has_init={}",
                    type_ptr, type_dict_ptr, has_init
                );
            }
            return;
        }
        if let Object::Class(class_data) = &mut *proxy_class.kind_mut() {
            for (name, value) in updates {
                class_data.attrs.insert(name, value);
            }
            if trace {
                eprintln!(
                    "[cpy-proxy] populate class attrs from tp_dict type_ptr={:p} class={} attrs={} has_init={}",
                    type_ptr,
                    class_data.name,
                    class_data.attrs.len(),
                    has_init
                );
            }
        }
    }

    fn populate_proxy_class_layout_attrs_from_type_object(
        &mut self,
        proxy_class: &ObjRef,
        type_ptr: *mut c_void,
    ) {
        if type_ptr.is_null() {
            return;
        }
        // SAFETY: caller only invokes this in type-object proxy construction paths.
        let type_ref = unsafe { type_ptr.cast::<CpythonTypeObject>().as_ref() };
        let Some(type_ref) = type_ref else {
            return;
        };
        let flags_value = match i64::try_from(type_ref.tp_flags) {
            Ok(value) => Value::Int(value),
            Err(_) => Value::BigInt(Box::new(BigInt::from_u64(type_ref.tp_flags as u64))),
        };
        if let Object::Class(class_data) = &mut *proxy_class.kind_mut() {
            class_data.attrs.insert(
                "__dictoffset__".to_string(),
                Value::Int(type_ref.tp_dictoffset as i64),
            );
            class_data.attrs.insert(
                "__weakrefoffset__".to_string(),
                Value::Int(type_ref.tp_weaklistoffset as i64),
            );
            class_data.attrs.insert(
                "__basicsize__".to_string(),
                Value::Int(type_ref.tp_basicsize as i64),
            );
            class_data.attrs.insert(
                "__itemsize__".to_string(),
                Value::Int(type_ref.tp_itemsize as i64),
            );
            class_data
                .attrs
                .insert("__flags__".to_string(), flags_value);
        }
    }

    fn owns_cpython_allocation_ptr(&self, ptr: *mut c_void) -> bool {
        if ptr.is_null() {
            return false;
        }
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &*self.vm };
            return vm.capi_ptr_is_owned_compat(ptr as usize);
        }
        self.is_owned_compat_ptr(ptr)
            || self.cpython_aux_allocations.contains(&ptr)
            || self
                .cpython_list_buffers
                .values()
                .any(|(buffer, _)| buffer.cast::<c_void>() == ptr)
    }

    fn is_owned_compat_ptr(&self, ptr: *mut c_void) -> bool {
        if ptr.is_null() {
            return false;
        }
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &*self.vm };
            return vm.capi_ptr_is_owned_compat(ptr as usize);
        }
        self.cpython_allocations
            .iter()
            .any(|allocation| allocation.cast::<c_void>() == ptr)
    }

    fn refresh_owned_type_proxy_name(&mut self, ptr: *mut c_void) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: caller only invokes this for pointers registered as owned type objects.
        let Ok(type_name) =
            (unsafe { c_name_to_string((*ptr.cast::<CpythonTypeObject>()).tp_name) })
        else {
            return;
        };
        let (name, module_name) = match type_name.rsplit_once('.') {
            Some((module, short)) => (short.to_string(), Some(module.to_string())),
            None => (type_name, None),
        };
        let Some(handle) = self.cpython_objects_by_ptr.get(&(ptr as usize)).copied() else {
            if super::env_var_present_cached("PYRS_TRACE_CPY_OWNED_TYPES") {
                eprintln!(
                    "[cpy-owned-type] refresh-name ptr={:p} status=no-handle",
                    ptr
                );
            }
            return;
        };
        let updated_class = {
            let Some(slot) = self.objects.get_mut(&handle) else {
                if super::env_var_present_cached("PYRS_TRACE_CPY_OWNED_TYPES") {
                    eprintln!(
                        "[cpy-owned-type] refresh-name ptr={:p} handle={} status=missing-slot",
                        ptr, handle
                    );
                }
                return;
            };
            let Value::Class(class_obj) = &mut slot.value else {
                if super::env_var_present_cached("PYRS_TRACE_CPY_OWNED_TYPES") {
                    eprintln!(
                        "[cpy-owned-type] refresh-name ptr={:p} handle={} status=non-class",
                        ptr, handle
                    );
                }
                return;
            };
            if let Object::Class(class_data) = &mut *class_obj.kind_mut() {
                class_data.name = name.clone();
                class_data
                    .attrs
                    .insert("__name__".to_string(), Value::Str(name.clone()));
                class_data
                    .attrs
                    .insert("__qualname__".to_string(), Value::Str(name.clone()));
                if let Some(module_name) = module_name.clone() {
                    class_data
                        .attrs
                        .insert("__module__".to_string(), Value::Str(module_name));
                }
            }
            if super::env_var_present_cached("PYRS_TRACE_CPY_OWNED_TYPES") {
                eprintln!(
                    "[cpy-owned-type] refresh-name ptr={:p} handle={} status=updated name={} module={}",
                    ptr,
                    handle,
                    name,
                    module_name.as_deref().unwrap_or("<none>")
                );
            }
            class_obj.clone()
        };
        if !self.vm.is_null() {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.extension_cpython_ptr_value_set(ptr as usize, &Value::Class(updated_class));
        }
    }

    pub(super) fn register_owned_type_ptr(&mut self, ptr: *mut c_void) {
        if ptr.is_null() {
            return;
        }
        let inserted = self.cpython_known_type_ptrs.insert(ptr as usize);
        self.capi_registry_register_owned_ptr(ptr, None);
        if inserted {
            self.refresh_owned_type_proxy_name(ptr);
        }
        if inserted && super::env_var_present_cached("PYRS_TRACE_CPY_OWNED_TYPES") {
            eprintln!("[cpy-owned-type] register ptr={:p}", ptr);
        }
    }

    pub(super) fn register_known_type_ptr(&mut self, ptr: *mut c_void) {
        if ptr.is_null() {
            return;
        }
        self.cpython_known_type_ptrs.insert(ptr as usize);
    }

    pub(super) fn is_known_type_ptr(&self, ptr: *mut c_void) -> bool {
        if ptr.is_null() {
            return false;
        }
        if Self::builtin_type_ptrs().contains(&ptr) {
            return true;
        }
        self.cpython_known_type_ptrs.contains(&(ptr as usize))
    }

    fn pin_owned_cpython_allocation_for_vm(&mut self, ptr: *mut c_void) {
        if ptr.is_null() || self.vm.is_null() || !self.owns_cpython_allocation_ptr(ptr) {
            return;
        }
        self.capi_registry_register_owned_ptr(ptr, None);
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        if vm.capi_pin_owned_ptr(ptr as usize) {
            vm.capi_registry_mark_alive(ptr as usize);
            if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                eprintln!(
                    "[pin-free] pin-insert ptr={:p} reason=pin_owned_allocation_for_vm",
                    ptr
                );
            }
        }
    }

    fn pin_capsule_allocation_for_vm(&mut self, ptr: *mut c_void) {
        if ptr.is_null() || self.vm.is_null() {
            return;
        }
        if !self.owns_cpython_allocation_ptr(ptr) {
            return;
        }
        self.capi_registry_register_owned_ptr(ptr, None);
        let Some(handle) = self.cpython_objects_by_ptr.get(&(ptr as usize)).copied() else {
            return;
        };
        let Some(slot) = self.capsules.get(&handle) else {
            return;
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        if vm.capi_pin_owned_ptr(ptr as usize) {
            vm.capi_registry_mark_alive(ptr as usize);
            if super::env_var_present_cached("PYRS_TRACE_PIN_FREE") {
                eprintln!(
                    "[pin-free] pin-insert ptr={:p} reason=pin_capsule_allocation_for_vm",
                    ptr
                );
            }
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
        let trace_capsule_import = super::env_var_present_cached("PYRS_TRACE_CPY_CAPSULE_IMPORT");
        if self.vm.is_null() {
            return Err("capsule_import missing VM context".to_string());
        }
        {
            // SAFETY: VM pointer is valid for the context lifetime.
            let vm = unsafe { &mut *self.vm };
            if requested_name == PYRS_DATETIME_CAPSULE_NAME {
                vm.ensure_builtin_datetime_capi_capsule();
            }
            if let Some(entry) = vm.extension_capsule_registry.get(requested_name) {
                if trace_capsule_import {
                    eprintln!(
                        "[capsule-import] registry-hit name={} ptr=0x{:x}",
                        requested_name, entry.pointer
                    );
                }
                return Ok(entry.pointer as *mut c_void);
            }
        }
        if let Some((pointer, context, destructor)) = self.capsules.values().find_map(|slot| {
            let slot_name = slot.name.as_ref()?.to_str().ok()?;
            if slot_name == requested_name {
                Some((slot.pointer, slot.context, slot.destructor))
            } else {
                None
            }
        }) {
            // SAFETY: VM pointer is valid for the context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.extension_capsule_registry.insert(
                requested_name.to_string(),
                crate::vm::ExtensionCapsuleRegistryEntry {
                    pointer,
                    context,
                    destructor,
                },
            );
            if trace_capsule_import {
                eprintln!(
                    "[capsule-import] context-capsule-hit name={} ptr=0x{:x}",
                    requested_name, pointer
                );
            }
            return Ok(pointer as *mut c_void);
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
        let mut object = {
            // SAFETY: VM pointer is valid for the context lifetime.
            let vm = unsafe { &mut *self.vm };
            vm.import_module_value_sync(module_name).map_err(|_| {
                if trace_capsule_import {
                    eprintln!(
                        "[capsule-import] import-fail module={} requested={}",
                        module_name, requested_name
                    );
                }
                format!(
                    "PyCapsule_Import could not import module \"{}\"",
                    module_name
                )
            })?
        };
        for part in parts {
            object = {
                // SAFETY: VM pointer is valid for the context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.builtin_getattr(vec![object, Value::Str(part.to_string())], HashMap::new())
                    .map_err(|_| {
                        if trace_capsule_import {
                            eprintln!(
                                "[capsule-import] getattr-fail module={} attr={} requested={}",
                                module_name, part, requested_name
                            );
                        }
                        format!("PyCapsule_Import \"{}\" is not valid", requested_name)
                    })?
            };
        }
        if trace_capsule_import {
            eprintln!(
                "[capsule-import] resolved-object name={} tag={}",
                requested_name,
                cpython_value_debug_tag(&object)
            );
        }
        if let Some(capsule_ptr) = Self::cpython_proxy_raw_ptr_from_value(&object)
            && let Some(capsule_handle) = self.cpython_handle_from_ptr(capsule_ptr)
            && self.capsules.contains_key(&capsule_handle)
        {
            if trace_capsule_import {
                eprintln!(
                    "[capsule-import] proxy-hit name={} raw_ptr={:p} handle={}",
                    requested_name, capsule_ptr, capsule_handle
                );
            }
            let pointer = self.capsule_get_pointer(capsule_handle, name)?;
            if let Some(slot) = self.capsules.get(&capsule_handle) {
                // SAFETY: VM pointer is valid for the context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.extension_capsule_registry.insert(
                    requested_name.to_string(),
                    crate::vm::ExtensionCapsuleRegistryEntry {
                        pointer: slot.pointer,
                        context: slot.context,
                        destructor: slot.destructor,
                    },
                );
                if trace_capsule_import {
                    eprintln!(
                        "[capsule-import] registry-insert name={} pointer=0x{:x}",
                        requested_name, slot.pointer
                    );
                }
            }
            return Ok(pointer);
        }
        if trace_capsule_import {
            let proxy_ptr =
                Self::cpython_proxy_raw_ptr_from_value(&object).unwrap_or(std::ptr::null_mut());
            let handle = if proxy_ptr.is_null() {
                None
            } else {
                self.cpython_handle_from_ptr(proxy_ptr)
            };
            eprintln!(
                "[capsule-import] miss name={} object_tag={} proxy_ptr={:p} handle={:?}",
                requested_name,
                cpython_value_debug_tag(&object),
                proxy_ptr,
                handle
            );
        }
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
                    let owned_raw = self
                        .cpython_ptr_by_handle
                        .get(&object_handle)
                        .copied()
                        .filter(|ptr| self.owns_cpython_allocation_ptr(*ptr));
                    if let Some(raw_ptr) = owned_raw {
                        // SAFETY: `raw_ptr` is an owned bytearray-compatible allocation for this handle.
                        let (raw_data, raw_len) = unsafe {
                            let raw = raw_ptr.cast::<CpythonByteArrayCompatObject>();
                            let len = (*raw).ob_base.ob_size.max(0) as usize;
                            ((*raw).ob_start.cast::<u8>(), len)
                        };
                        if !raw_data.is_null() || raw_len == 0 {
                            return Ok(BufferInfoSnapshot {
                                data: raw_data.cast_const(),
                                len: raw_len,
                                readonly: false,
                                itemsize: 1,
                                shape,
                                strides,
                                contiguous: true,
                                format_text: "B".to_string(),
                            });
                        }
                    }
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
        if let Some(ptr) = self.cpython_ptr_by_handle.get(&object_handle).copied()
            && self.owns_cpython_allocation_ptr(ptr)
        {
            self.sync_value_from_cpython_storage(object_handle, ptr);
        }
        self.decref(object_handle)
    }

    fn register_buffer_internal(&mut self, object_handle: PyrsObjectHandle) -> *mut c_void {
        let internal = Box::into_raw(Box::new(CpythonBufferInternal {
            handle: object_handle,
        }));
        self.buffer_internal_handles
            .insert(internal as usize, object_handle);
        internal.cast()
    }

    fn take_owned_buffer_internal_handle(
        &mut self,
        internal: *mut c_void,
    ) -> Option<PyrsObjectHandle> {
        if internal.is_null() {
            return None;
        }
        let key = internal as usize;
        self.buffer_internal_handles.remove(&key)?;
        // SAFETY: this pointer key was created by `register_buffer_internal` in this context.
        let internal = unsafe { Box::from_raw(internal.cast::<CpythonBufferInternal>()) };
        Some(internal.handle)
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

fn cpython_stable_utf8_registry() -> &'static Mutex<HashMap<String, Box<[u8]>>> {
    CPYTHON_STABLE_UTF8_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn cpython_stable_utf8_ptr(text: &str) -> Result<*const c_char, String> {
    let mut registry = cpython_stable_utf8_registry()
        .lock()
        .map_err(|_| "failed to lock stable UTF-8 registry".to_string())?;
    if let Some(bytes) = registry.get(text) {
        return Ok(bytes.as_ptr().cast::<c_char>());
    }
    let mut bytes = Vec::with_capacity(text.len().saturating_add(1));
    bytes.extend_from_slice(text.as_bytes());
    bytes.push(0);
    let boxed = bytes.into_boxed_slice();
    let ptr = boxed.as_ptr().cast::<c_char>();
    registry.insert(text.to_string(), boxed);
    Ok(ptr)
}

unsafe fn capi_context_mut<'a>(module_ctx: *mut c_void) -> Option<&'a mut ModuleCapiContext> {
    if module_ctx.is_null() {
        return None;
    }
    // SAFETY: caller guarantees `module_ctx` points to a valid `ModuleCapiContext`.
    Some(unsafe { &mut *(module_ctx as *mut ModuleCapiContext) })
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
const PY_TPFLAGS_BASE_EXC_SUBCLASS: usize = 1usize << 30;
const PY_TPFLAGS_TYPE_SUBCLASS: usize = 1usize << 31;
const METH_VARARGS: c_int = 0x0001;
const METH_KEYWORDS: c_int = 0x0002;
const METH_NOARGS: c_int = 0x0004;
const METH_O: c_int = 0x0008;
const METH_FASTCALL: c_int = 0x0080;
const METH_METHOD: c_int = 0x0200;

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
    fn pyrs_testcapi_get_args(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_get_kwargs(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_empty(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_tuple(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_c(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_C(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_s(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_s_star(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_s_hash(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_z(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_z_star(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_z_hash(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_y(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_y_star(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_y_hash(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_es(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_et(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_es_hash(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_et_hash(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_w_star(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_getargs_w_star_opt(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_gh_99240_clear_args(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_parse_tuple_and_keywords(
        args: *mut c_void,
        kwargs: *mut c_void,
    ) -> *mut c_void;
    fn pyrs_testcapi_argparsing(args: *mut c_void, kwargs: *mut c_void) -> *mut c_void;
    fn pyrs_testcapi_init_heaptype_types(module: *mut c_void) -> i32;
    fn pyrs_testcapi_init_structmember_types(module: *mut c_void) -> i32;
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

impl Vm {
    fn call_testcapi_capi_helper(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        helper: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
        failure_label: &str,
    ) -> Result<Value, RuntimeError> {
        let args_tuple = self.heap.alloc_tuple(args);
        let kwargs_value = if kwargs.is_empty() {
            None
        } else {
            Some(
                self.heap.alloc_dict(
                    kwargs
                        .into_iter()
                        .map(|(name, value)| (Value::Str(name), value))
                        .collect(),
                ),
            )
        };
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let args_ptr = call_ctx.alloc_cpython_ptr_for_value(args_tuple);
        if args_ptr.is_null() {
            return Err(RuntimeError::new(format!(
                "{failure_label}: failed to materialize args tuple",
            )));
        }
        let kwargs_ptr = match kwargs_value {
            Some(kwargs_value) => {
                let ptr = call_ctx.alloc_cpython_ptr_for_value(kwargs_value);
                if ptr.is_null() {
                    return Err(RuntimeError::new(format!(
                        "{failure_label}: failed to materialize kwargs dict",
                    )));
                }
                ptr
            }
            None => std::ptr::null_mut(),
        };
        let result_ptr = unsafe { helper(args_ptr, kwargs_ptr) };
        call_ctx.sync_owned_compat_storage_from_raw();
        if result_ptr.is_null() {
            if let Some(err) = call_ctx.runtime_error_from_current_error_state(failure_label) {
                return Err(err);
            }
            return Err(RuntimeError::new(failure_label));
        }
        call_ctx
            .cpython_value_from_owned_ptr(result_ptr)
            .ok_or_else(|| RuntimeError::new(format!("{failure_label}: unknown result pointer")))
    }

    pub(in crate::vm) fn builtin_testcapi_get_args_via_capi(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.call_testcapi_capi_helper(args, kwargs, pyrs_testcapi_get_args, "get_args failed")
    }

    pub(in crate::vm) fn builtin_testcapi_get_kwargs_via_capi(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.call_testcapi_capi_helper(args, kwargs, pyrs_testcapi_get_kwargs, "get_kwargs failed")
    }

    pub(in crate::vm) fn builtin_testcapi_getargs_empty_via_capi(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.call_testcapi_capi_helper(
            args,
            kwargs,
            pyrs_testcapi_getargs_empty,
            "getargs_empty failed",
        )
    }

    pub(in crate::vm) fn builtin_testcapi_getargs_tuple_via_capi(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.call_testcapi_capi_helper(
            args,
            kwargs,
            pyrs_testcapi_getargs_tuple,
            "getargs_tuple failed",
        )
    }

    pub(in crate::vm) fn builtin_testcapi_getargs_string_via_capi(
        &mut self,
        kind: TestCapiStringParseKind,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        type TestCapiHelper = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
        let (helper, label) = match kind {
            TestCapiStringParseKind::LowerC => (
                pyrs_testcapi_getargs_c as TestCapiHelper,
                "getargs_c failed",
            ),
            TestCapiStringParseKind::UpperC => (
                pyrs_testcapi_getargs_C as TestCapiHelper,
                "getargs_C failed",
            ),
            TestCapiStringParseKind::LowerS => (
                pyrs_testcapi_getargs_s as TestCapiHelper,
                "getargs_s failed",
            ),
            TestCapiStringParseKind::LowerSStar => (
                pyrs_testcapi_getargs_s_star as TestCapiHelper,
                "getargs_s_star failed",
            ),
            TestCapiStringParseKind::LowerSHash => (
                pyrs_testcapi_getargs_s_hash as TestCapiHelper,
                "getargs_s_hash failed",
            ),
            TestCapiStringParseKind::LowerZ => (
                pyrs_testcapi_getargs_z as TestCapiHelper,
                "getargs_z failed",
            ),
            TestCapiStringParseKind::LowerZStar => (
                pyrs_testcapi_getargs_z_star as TestCapiHelper,
                "getargs_z_star failed",
            ),
            TestCapiStringParseKind::LowerZHash => (
                pyrs_testcapi_getargs_z_hash as TestCapiHelper,
                "getargs_z_hash failed",
            ),
            TestCapiStringParseKind::LowerY => (
                pyrs_testcapi_getargs_y as TestCapiHelper,
                "getargs_y failed",
            ),
            TestCapiStringParseKind::LowerYStar => (
                pyrs_testcapi_getargs_y_star as TestCapiHelper,
                "getargs_y_star failed",
            ),
            TestCapiStringParseKind::LowerYHash => (
                pyrs_testcapi_getargs_y_hash as TestCapiHelper,
                "getargs_y_hash failed",
            ),
            TestCapiStringParseKind::LowerEs => (
                pyrs_testcapi_getargs_es as TestCapiHelper,
                "getargs_es failed",
            ),
            TestCapiStringParseKind::LowerEt => (
                pyrs_testcapi_getargs_et as TestCapiHelper,
                "getargs_et failed",
            ),
            TestCapiStringParseKind::LowerEsHash => (
                pyrs_testcapi_getargs_es_hash as TestCapiHelper,
                "getargs_es_hash failed",
            ),
            TestCapiStringParseKind::LowerEtHash => (
                pyrs_testcapi_getargs_et_hash as TestCapiHelper,
                "getargs_et_hash failed",
            ),
            TestCapiStringParseKind::WStar => (
                pyrs_testcapi_getargs_w_star as TestCapiHelper,
                "getargs_w_star failed",
            ),
            TestCapiStringParseKind::WStarOpt => (
                pyrs_testcapi_getargs_w_star_opt as TestCapiHelper,
                "getargs_w_star_opt failed",
            ),
            TestCapiStringParseKind::Gh99240ClearArgs => (
                pyrs_testcapi_gh_99240_clear_args as TestCapiHelper,
                "gh_99240_clear_args failed",
            ),
        };
        self.call_testcapi_capi_helper(args, kwargs, helper, label)
    }

    pub(in crate::vm) fn builtin_testcapi_parse_tuple_and_keywords_via_capi(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.call_testcapi_capi_helper(
            args,
            kwargs,
            pyrs_testcapi_parse_tuple_and_keywords,
            "parse_tuple_and_keywords failed",
        )
    }

    pub(in crate::vm) fn builtin_testcapi_argparsing_via_capi(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.call_testcapi_capi_helper(args, kwargs, pyrs_testcapi_argparsing, "argparsing failed")
    }

    pub(in crate::vm) fn init_testcapi_structmember_types_via_capi(
        &mut self,
        module: ObjRef,
    ) -> Result<(), RuntimeError> {
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let module_ptr = call_ctx.alloc_cpython_ptr_for_value(Value::Module(module));
        if module_ptr.is_null() {
            return Err(RuntimeError::new(
                "testcapi structmember init failed: failed to materialize module",
            ));
        }
        let status = unsafe { pyrs_testcapi_init_structmember_types(module_ptr) };
        call_ctx.sync_owned_compat_storage_from_raw();
        if status == 0 {
            return Ok(());
        }
        if let Some(err) =
            call_ctx.runtime_error_from_current_error_state("testcapi structmember init failed")
        {
            return Err(err);
        }
        Err(RuntimeError::new("testcapi structmember init failed"))
    }

    pub(in crate::vm) fn init_testcapi_heaptype_types_via_capi(
        &mut self,
        module: ObjRef,
    ) -> Result<(), RuntimeError> {
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let module_ptr = call_ctx.alloc_cpython_ptr_for_value(Value::Module(module));
        if module_ptr.is_null() {
            return Err(RuntimeError::new(
                "testcapi heaptype init failed: failed to materialize module",
            ));
        }
        let status = unsafe { pyrs_testcapi_init_heaptype_types(module_ptr) };
        call_ctx.sync_owned_compat_storage_from_raw();
        if status == 0 {
            return Ok(());
        }
        if let Some(err) =
            call_ctx.runtime_error_from_current_error_state("testcapi heaptype init failed")
        {
            return Err(err);
        }
        Err(RuntimeError::new("testcapi heaptype init failed"))
    }
}
