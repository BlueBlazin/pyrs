use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize};
use std::sync::{Condvar, Mutex, Once, OnceLock};

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
    BigInt, BoundMethod, BuiltinFunction, ClassObject, InstanceObject, NativeMethodKind,
    NativeMethodObject, Object, RuntimeError, Value,
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
thread_local! {
    static CPYTHON_DESCRIPTOR_REGISTRY: RefCell<HashMap<usize, CpythonDescriptorKind>> =
        RefCell::new(HashMap::new());
}
mod callable_runtime;
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
mod cpython_unicode_api;
mod cpython_unicode_error_api;
mod cpython_unicode_error_runtime;
mod cpython_value_runtime;
mod cpython_weakref_api;
mod loader_runtime;
mod module_context_state;
mod proxy_runtime;

use self::cpython_args_runtime::{
    cpython_keyword_args_from_dict_object, cpython_positional_args_from_tuple_object,
};
use self::cpython_bigint_runtime::{
    cpython_asnativebytes_resolve_endian, cpython_bigint_from_twos_complement_le,
    cpython_bigint_from_value, cpython_bigint_low_u64, cpython_bigint_to_twos_complement_le,
    cpython_bigint_to_u64, cpython_required_signed_bytes_for_bigint,
    cpython_required_unsigned_bytes_for_bigint,
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
use self::cpython_datetime_runtime::{PYRS_DATETIME_CAPI, PYRS_DATETIME_CAPSULE_NAME};
use self::cpython_descriptor_method_api::{
    PyCFunction_Call, PyCFunction_GetFlags, PyCFunction_GetFunction, PyCFunction_GetSelf,
    PyCFunction_New, PyCFunction_NewEx, PyCMethod_New, PyDescr_NewClassMethod, PyDescr_NewGetSet,
    PyDescr_NewMember, PyDescr_NewMethod, PyMember_GetOne, PyMember_SetOne, PySlice_AdjustIndices,
    PySlice_GetIndices, PySlice_GetIndicesEx, PySlice_New, PySlice_Unpack, PyWrapper_New,
    cpython_cfunction_tp_call, cpython_cfunction_tp_getattro, cpython_invoke_method_from_values,
};
use self::cpython_dict_api::{
    _PyDict_GetItem_KnownHash, _PyDict_Pop, PyDict_Clear, PyDict_Contains, PyDict_ContainsString,
    PyDict_Copy, PyDict_DelItem, PyDict_DelItemString, PyDict_GetItem, PyDict_GetItemRef,
    PyDict_GetItemString, PyDict_GetItemStringRef, PyDict_GetItemWithError, PyDict_Items,
    PyDict_Keys, PyDict_Merge, PyDict_MergeFromSeq2, PyDict_New, PyDict_Next, PyDict_Pop,
    PyDict_PopString, PyDict_SetDefault, PyDict_SetDefaultRef, PyDict_SetItem,
    PyDict_SetItemString, PyDict_Size, PyDict_Update, PyDict_Values, PyDictProxy_New,
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
    PyObject_DelItem, PyObject_GetItem, PyObject_GetOptionalAttr, PyObject_Hash,
    PyObject_HashNotImplemented, PyObject_IsInstance, PyObject_IsSubclass, PyObject_Length,
    PyObject_LengthHint, PyObject_RichCompare, PyObject_RichCompareBool, PyObject_SetItem,
    PyObject_Size, cpython_debug_compare_value, cpython_type_name_for_object_ptr,
    cpython_value_type_name_from_ptr,
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
    _PyUnicode_IsTitlecase, _PyUnicode_IsUppercase, _PyUnicode_IsWhitespace, Py_AddPendingCall,
    Py_AtExit, Py_BytesMain, Py_CompileString, Py_DecodeLocale, Py_EncodeLocale, Py_EndInterpreter,
    Py_Exit, Py_FatalError, Py_Finalize, Py_FinalizeEx, Py_GetArgcArgv, Py_GetBuildInfo,
    Py_GetCompiler, Py_GetCopyright, Py_GetExecPrefix, Py_GetPath, Py_GetPlatform, Py_GetPrefix,
    Py_GetProgramFullPath, Py_GetProgramName, Py_GetPythonHome, Py_GetRecursionLimit,
    Py_GetVersion, Py_Initialize, Py_InitializeEx, Py_IsFinalizing, Py_Main, Py_MakePendingCalls,
    Py_NewInterpreter, Py_PACK_FULL_VERSION, Py_PACK_VERSION, Py_ReprEnter, Py_ReprLeave,
    Py_SetPath, Py_SetProgramName, Py_SetPythonHome, Py_SetRecursionLimit, PyErr_BadArgument,
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
    cpython_call_method_for_capi, cpython_call_object, cpython_codec_error_name_optional,
    cpython_codec_name_or_default, cpython_mapping_ass_subscript_slot,
    cpython_mapping_subscript_slot, cpython_sequence_item_slot, cpython_structseq_count_fields,
    cpython_try_binary_number_slot, cpython_try_richcompare_slot,
    cpython_unicode_decode_with_codec_in_context, cpython_unicode_encode_with_codec_in_context,
    cpython_unicode_text_from_value, cpython_valid_type_ptr,
};
use self::cpython_string_runtime::{
    c_name_to_string, c_wide_name_to_string, cpython_string_to_wide_units,
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
    cpython_current_thread_state_ptr, cpython_get_or_init_constant_ptr,
    cpython_get_or_init_wide_storage, cpython_heap_type_registry, cpython_init_thread_state_compat,
    cpython_interpreter_state_allocations, cpython_is_interned_unicode_ptr,
    cpython_is_known_interpreter_state_ptr, cpython_is_known_thread_state_ptr,
    cpython_lookup_interned_unicode_ptr, cpython_lookup_interned_unicode_text,
    cpython_main_interpreter_state_ptr, cpython_main_thread_state_ptr, cpython_pending_calls,
    cpython_read_sys_path_string, cpython_read_sys_string, cpython_register_interned_unicode,
    cpython_set_wide_storage, cpython_store_argv_wide, cpython_structseq_registry,
    cpython_thread_lock_registry, cpython_thread_state_allocations,
    cpython_thread_tls_key_registry, cpython_thread_tls_values, cpython_thread_tss_registry,
    cpython_thread_tss_values,
};
use self::cpython_tuple_api::{
    PyTuple_GetItem, PyTuple_GetSlice, PyTuple_New, PyTuple_SetItem, PyTuple_Size,
};
use self::cpython_type_api::{
    _PyType_Lookup, PyType_ClearCache, PyType_Freeze, PyType_FromMetaclass,
    PyType_FromModuleAndSpec, PyType_FromSpec, PyType_FromSpecWithBases, PyType_GenericAlloc,
    PyType_GenericNew, PyType_GetBaseByToken, PyType_GetFlags, PyType_GetFullyQualifiedName,
    PyType_GetModule, PyType_GetModuleByDef, PyType_GetModuleName, PyType_GetModuleState,
    PyType_GetName, PyType_GetQualName, PyType_GetSlot, PyType_GetTypeDataSize, PyType_IsSubtype,
    PyType_Modified, PyType_Ready, cpython_is_type_object_ptr, cpython_type_tp_call,
};
use self::cpython_type_exports::*;
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
struct CpythonComplexCompatObject {
    ob_base: CpythonObjectHead,
    cval: CpythonComplexValue,
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
        PyLong_Type.tp_as_number = std::ptr::addr_of_mut!(PY_LONG_NUMBER_METHODS).cast();
        PyBool_Type.tp_basicsize = 24;
        PyBool_Type.tp_itemsize = 4;
        PyBool_Type.tp_as_number = std::ptr::addr_of_mut!(PY_LONG_NUMBER_METHODS).cast();
        PyFloat_Type.tp_basicsize = 24;
        PyFloat_Type.tp_itemsize = 0;
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
    _pad_to_exc_info: [u8; 0x78 - (3 * std::mem::size_of::<*mut c_void>())],
    exc_info: *mut CpythonErrStackItemCompat,
    exc_state: CpythonErrStackItemCompat,
    _bytes: [u8; CPYTHON_THREAD_STATE_COMPAT_SIZE
        - (0x78
            + std::mem::size_of::<*mut c_void>()
            + std::mem::size_of::<CpythonErrStackItemCompat>())],
}

static mut MAIN_THREAD_STATE_STORAGE: CpythonThreadStateCompat = CpythonThreadStateCompat {
    prev: std::ptr::null_mut(),
    next: std::ptr::null_mut(),
    interp: std::ptr::null_mut(),
    _pad_to_exc_info: [0; 0x78 - (3 * std::mem::size_of::<*mut c_void>())],
    exc_info: std::ptr::null_mut(),
    exc_state: CpythonErrStackItemCompat {
        exc_value: std::ptr::null_mut(),
        previous_item: std::ptr::null_mut(),
    },
    _bytes: [0; CPYTHON_THREAD_STATE_COMPAT_SIZE
        - (0x78
            + std::mem::size_of::<*mut c_void>()
            + std::mem::size_of::<CpythonErrStackItemCompat>())],
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

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
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
            CPYTHON_DESCRIPTOR_REGISTRY.with(|registry| {
                registry.borrow_mut().remove(&(raw as usize));
            });
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
    fn is_probable_c_string_pointer(ptr: *const c_char) -> bool {
        const MIN_VALID_PTR: usize = 0x1_0000_0000;
        if ptr.is_null() {
            return false;
        }
        let addr = ptr as usize;
        addr >= MIN_VALID_PTR
    }

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
            if !Self::is_probable_c_string_pointer(tp_name) {
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
            Self::is_probable_c_string_pointer(tp_name)
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
            std::ptr::addr_of_mut!(PyClassMethodDescr_Type),
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
            complex_value,
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
                match &slot.value {
                    Value::Complex { real, imag } => Some(CpythonComplexValue {
                        real: *real,
                        imag: *imag,
                    }),
                    _ => None,
                },
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
            ),
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
        CPYTHON_DESCRIPTOR_REGISTRY.with(|registry| {
            registry.borrow_mut().insert(ptr as usize, descriptor_kind);
        });
        ptr
    }

    fn resolve_descriptor_attr_ptr(
        &mut self,
        descriptor_ptr: *mut c_void,
        object: *mut c_void,
        object_type: *mut CpythonTypeObject,
        is_type_object: bool,
    ) -> Option<*mut c_void> {
        let descriptor_key = descriptor_ptr as usize;
        let descriptor_kind = if let Some(kind) = self.cpython_descriptors.get(&descriptor_key) {
            *kind
        } else {
            let kind = CPYTHON_DESCRIPTOR_REGISTRY
                .with(|registry| registry.borrow().get(&descriptor_key).copied())?;
            self.cpython_descriptors.insert(descriptor_key, kind);
            kind
        };
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
