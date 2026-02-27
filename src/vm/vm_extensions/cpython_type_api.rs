use std::backtrace::Backtrace;
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::mem::align_of;

use crate::runtime::{BuiltinFunction, Object, Value};

use super::{
    _PyObject_New, _PyObject_NewVar, CpythonAsyncMethods, CpythonBufferProcs, CpythonHeapTypeInfo,
    CpythonHeapTypeObject, CpythonMappingMethods, CpythonMemberDef, CpythonMethodDef,
    CpythonModuleCompatObject, CpythonNumberMethods, CpythonObjectHead, CpythonSequenceMethods,
    CpythonTypeObject, CpythonTypeSpec, InternalCallOutcome, ModuleCapiContext,
    PY_MEMBER_RELATIVE_OFFSET, PY_TPFLAGS_BASETYPE, PY_TPFLAGS_BYTES_SUBCLASS,
    PY_TPFLAGS_DICT_SUBCLASS, PY_TPFLAGS_HEAPTYPE, PY_TPFLAGS_IMMUTABLETYPE,
    PY_TPFLAGS_LIST_SUBCLASS, PY_TPFLAGS_LONG_SUBCLASS, PY_TPFLAGS_READY,
    PY_TPFLAGS_TUPLE_SUBCLASS, PY_TPFLAGS_TYPE_SUBCLASS, PY_TPFLAGS_UNICODE_SUBCLASS,
    PY_TYPE_SLOT_AM_AITER, PY_TYPE_SLOT_AM_ANEXT, PY_TYPE_SLOT_AM_AWAIT, PY_TYPE_SLOT_AM_SEND,
    PY_TYPE_SLOT_BF_GETBUFFER, PY_TYPE_SLOT_BF_RELEASEBUFFER, PY_TYPE_SLOT_MAX,
    PY_TYPE_SLOT_MP_ASS_SUBSCRIPT, PY_TYPE_SLOT_MP_LENGTH, PY_TYPE_SLOT_MP_SUBSCRIPT,
    PY_TYPE_SLOT_NB_ABSOLUTE, PY_TYPE_SLOT_NB_ADD, PY_TYPE_SLOT_NB_AND, PY_TYPE_SLOT_NB_BOOL,
    PY_TYPE_SLOT_NB_DIVMOD, PY_TYPE_SLOT_NB_FLOAT, PY_TYPE_SLOT_NB_FLOOR_DIVIDE,
    PY_TYPE_SLOT_NB_INDEX, PY_TYPE_SLOT_NB_INPLACE_ADD, PY_TYPE_SLOT_NB_INPLACE_AND,
    PY_TYPE_SLOT_NB_INPLACE_FLOOR_DIVIDE, PY_TYPE_SLOT_NB_INPLACE_LSHIFT,
    PY_TYPE_SLOT_NB_INPLACE_MATRIX_MULTIPLY, PY_TYPE_SLOT_NB_INPLACE_MULTIPLY,
    PY_TYPE_SLOT_NB_INPLACE_OR, PY_TYPE_SLOT_NB_INPLACE_POWER, PY_TYPE_SLOT_NB_INPLACE_REMAINDER,
    PY_TYPE_SLOT_NB_INPLACE_RSHIFT, PY_TYPE_SLOT_NB_INPLACE_SUBTRACT,
    PY_TYPE_SLOT_NB_INPLACE_TRUE_DIVIDE, PY_TYPE_SLOT_NB_INPLACE_XOR, PY_TYPE_SLOT_NB_INT,
    PY_TYPE_SLOT_NB_INVERT, PY_TYPE_SLOT_NB_LSHIFT, PY_TYPE_SLOT_NB_MATRIX_MULTIPLY,
    PY_TYPE_SLOT_NB_MULTIPLY, PY_TYPE_SLOT_NB_NEGATIVE, PY_TYPE_SLOT_NB_OR,
    PY_TYPE_SLOT_NB_POSITIVE, PY_TYPE_SLOT_NB_POWER, PY_TYPE_SLOT_NB_REMAINDER,
    PY_TYPE_SLOT_NB_RSHIFT, PY_TYPE_SLOT_NB_SUBTRACT, PY_TYPE_SLOT_NB_TRUE_DIVIDE,
    PY_TYPE_SLOT_NB_XOR, PY_TYPE_SLOT_SQ_ASS_ITEM, PY_TYPE_SLOT_SQ_CONCAT,
    PY_TYPE_SLOT_SQ_CONTAINS, PY_TYPE_SLOT_SQ_INPLACE_CONCAT, PY_TYPE_SLOT_SQ_INPLACE_REPEAT,
    PY_TYPE_SLOT_SQ_ITEM, PY_TYPE_SLOT_SQ_LENGTH, PY_TYPE_SLOT_SQ_REPEAT, PY_TYPE_SLOT_TP_ALLOC,
    PY_TYPE_SLOT_TP_BASE, PY_TYPE_SLOT_TP_BASES, PY_TYPE_SLOT_TP_CALL, PY_TYPE_SLOT_TP_CLEAR,
    PY_TYPE_SLOT_TP_DEALLOC, PY_TYPE_SLOT_TP_DEL, PY_TYPE_SLOT_TP_DESCR_GET,
    PY_TYPE_SLOT_TP_DESCR_SET, PY_TYPE_SLOT_TP_DOC, PY_TYPE_SLOT_TP_FINALIZE, PY_TYPE_SLOT_TP_FREE,
    PY_TYPE_SLOT_TP_GETATTR, PY_TYPE_SLOT_TP_GETATTRO, PY_TYPE_SLOT_TP_GETSET,
    PY_TYPE_SLOT_TP_HASH, PY_TYPE_SLOT_TP_INIT, PY_TYPE_SLOT_TP_IS_GC, PY_TYPE_SLOT_TP_ITER,
    PY_TYPE_SLOT_TP_ITERNEXT, PY_TYPE_SLOT_TP_MEMBERS, PY_TYPE_SLOT_TP_METHODS,
    PY_TYPE_SLOT_TP_NEW, PY_TYPE_SLOT_TP_REPR, PY_TYPE_SLOT_TP_RICHCOMPARE,
    PY_TYPE_SLOT_TP_SETATTR, PY_TYPE_SLOT_TP_SETATTRO, PY_TYPE_SLOT_TP_STR, PY_TYPE_SLOT_TP_TOKEN,
    PY_TYPE_SLOT_TP_TRAVERSE, PY_TYPE_SLOT_TP_VECTORCALL, Py_DecRef, Py_IncRef, Py_XIncRef,
    PyBaseObject_Type, PyBool_Type, PyByteArray_Type, PyBytes_Type, PyComplex_Type,
    PyDescr_NewClassMethod, PyDescr_NewMethod, PyDict_DelItemString, PyDict_GetItemString,
    PyDict_GetItemWithError, PyDict_New, PyDict_SetItem, PyDict_SetItemString, PyDict_Type,
    PyErr_BadInternalCall, PyErr_Clear, PyErr_Occurred, PyExc_AttributeError, PyExc_MemoryError,
    PyExc_SystemError, PyExc_TypeError, PyFloat_Type, PyFrozenSet_Type, PyList_Type, PyLong_Type,
    PyMemoryView_Type, PyModule_GetState, PyObject_Free, PyObject_RichCompareBool, PyProperty_Type,
    PyRange_Type, PySet_Type, PySlice_Type, PyTuple_GetItem, PyTuple_New, PyTuple_SetItem,
    PyTuple_Size, PyTuple_Type, PyType_Type, PyUnicode_Type, c_name_to_string,
    cpython_builtin_type_name_for_ptr, cpython_builtin_type_ptr_for_class_name,
    cpython_call_internal_in_context, cpython_heap_type_registry,
    cpython_keyword_args_from_dict_object, cpython_new_ptr_for_value,
    cpython_positional_args_from_tuple_object, cpython_set_error, cpython_set_typed_error,
    cpython_value_debug_tag, cpython_value_from_ptr, cpython_value_from_ptr_or_proxy, free,
    with_active_cpython_context_mut,
};

unsafe extern "C" {
    fn calloc(nmemb: usize, size: usize) -> *mut c_void;
}

const METH_CLASS: c_int = 0x0010;
const TP_INIT_WRAPPER_NAME: &[u8; 9] = b"__init__\0";
const SPECIAL_MEMBER_VECTORCALL_OFFSET: &str = "__vectorcalloffset__";
const SPECIAL_MEMBER_DICT_OFFSET: &str = "__dictoffset__";
const SPECIAL_MEMBER_WEAKLIST_OFFSET: &str = "__weaklistoffset__";

fn align_up(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }
    let remainder = value % alignment;
    if remainder == 0 {
        value
    } else {
        value + (alignment - remainder)
    }
}

unsafe extern "C" fn cpython_type_tp_init_wrapper_method(
    self_obj: *mut c_void,
    defining_class: *mut c_void,
    args: *const *mut c_void,
    nargs: usize,
    kwnames: *mut c_void,
) -> *mut c_void {
    if self_obj.is_null() || defining_class.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_ptr = defining_class.cast::<CpythonTypeObject>();
    if type_ptr.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: defining class pointer follows PyCMethod call contract.
    let init_slot = unsafe { (*type_ptr).tp_init };
    if init_slot.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_AttributeError },
            "type object has no __init__ slot",
        );
        return std::ptr::null_mut();
    }

    let kw_count = if kwnames.is_null() {
        0usize
    } else {
        // SAFETY: kwnames must be a tuple of keyword names for METH_METHOD calls.
        let len = unsafe { PyTuple_Size(kwnames) };
        if len < 0 {
            return std::ptr::null_mut();
        }
        len as usize
    };
    if kw_count > nargs {
        cpython_set_error("METH_METHOD __init__ call received invalid nargs/kwnames");
        return std::ptr::null_mut();
    }
    let positional_count = nargs.saturating_sub(kw_count);

    // SAFETY: tuple allocation follows CPython ABI.
    let positional_tuple = unsafe { PyTuple_New(positional_count as isize) };
    if positional_tuple.is_null() {
        return std::ptr::null_mut();
    }
    for index in 0..positional_count {
        if args.is_null() {
            unsafe { Py_DecRef(positional_tuple) };
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        }
        // SAFETY: caller guarantees at least `nargs` positional+keyword entries.
        let item = unsafe { *args.add(index) };
        if item.is_null() {
            unsafe { Py_DecRef(positional_tuple) };
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        }
        // SAFETY: tuple takes ownership of inserted references.
        unsafe { Py_IncRef(item) };
        let status = unsafe { PyTuple_SetItem(positional_tuple, index as isize, item) };
        if status != 0 {
            unsafe { Py_DecRef(positional_tuple) };
            return std::ptr::null_mut();
        }
    }

    let kwargs_dict = if kw_count == 0 {
        std::ptr::null_mut()
    } else {
        // SAFETY: dict allocation follows CPython ABI.
        let dict = unsafe { PyDict_New() };
        if dict.is_null() {
            unsafe { Py_DecRef(positional_tuple) };
            return std::ptr::null_mut();
        }
        for kw_index in 0..kw_count {
            // SAFETY: kwnames is a tuple and kw_index is in range.
            let name_obj = unsafe { PyTuple_GetItem(kwnames, kw_index as isize) };
            if name_obj.is_null() {
                unsafe {
                    Py_DecRef(dict);
                    Py_DecRef(positional_tuple);
                }
                return std::ptr::null_mut();
            }
            // SAFETY: caller guarantees keyword values follow positional args in the vectorcall stack.
            let value_obj = unsafe { *args.add(positional_count + kw_index) };
            if value_obj.is_null() {
                unsafe {
                    Py_DecRef(dict);
                    Py_DecRef(positional_tuple);
                    PyErr_BadInternalCall();
                }
                return std::ptr::null_mut();
            }
            let status = unsafe { PyDict_SetItem(dict, name_obj, value_obj) };
            if status != 0 {
                unsafe {
                    Py_DecRef(dict);
                    Py_DecRef(positional_tuple);
                }
                return std::ptr::null_mut();
            }
        }
        dict
    };

    let init_fn: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int =
        // SAFETY: tp_init follows CPython initproc signature.
        unsafe { std::mem::transmute(init_slot) };
    let init_status = unsafe { init_fn(self_obj, positional_tuple, kwargs_dict) };
    unsafe {
        Py_DecRef(positional_tuple);
        if !kwargs_dict.is_null() {
            Py_DecRef(kwargs_dict);
        }
    }
    if init_status < 0 {
        return std::ptr::null_mut();
    }
    let none_ptr = std::ptr::addr_of_mut!(super::_Py_NoneStruct).cast::<c_void>();
    unsafe { Py_IncRef(none_ptr) };
    none_ptr
}

unsafe fn cpython_type_install_init_slot_wrapper(ty: *mut CpythonTypeObject) -> i32 {
    if ty.is_null() {
        return 0;
    }
    // SAFETY: caller provides a mutable, live type object pointer.
    let (tp_init, tp_dict) = unsafe { ((*ty).tp_init, (*ty).tp_dict) };
    if tp_init.is_null() || tp_dict.is_null() {
        return 0;
    }
    // SAFETY: dict pointer + static key follow CPython dict C-API contract.
    let existing =
        unsafe { PyDict_GetItemString(tp_dict, TP_INIT_WRAPPER_NAME.as_ptr().cast::<c_char>()) };
    if !existing.is_null() {
        return 0;
    }

    // SAFETY: method definition is process-lifetime metadata.
    let method_def =
        unsafe { calloc(1, std::mem::size_of::<CpythonMethodDef>()) }.cast::<CpythonMethodDef>();
    if method_def.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_MemoryError },
            "failed to allocate __init__ wrapper",
        );
        return -1;
    }
    // SAFETY: method definition storage is writable and lives for process lifetime.
    unsafe {
        method_def.write(CpythonMethodDef {
            ml_name: TP_INIT_WRAPPER_NAME.as_ptr().cast::<c_char>(),
            ml_meth: Some(std::mem::transmute::<
                unsafe extern "C" fn(
                    *mut c_void,
                    *mut c_void,
                    *const *mut c_void,
                    usize,
                    *mut c_void,
                ) -> *mut c_void,
                unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
            >(cpython_type_tp_init_wrapper_method)),
            ml_flags: super::METH_METHOD | super::METH_FASTCALL | super::METH_KEYWORDS,
            ml_doc: std::ptr::null(),
        });
    }
    // SAFETY: type pointer and method definition follow descriptor-construction contract.
    let descriptor = unsafe { PyDescr_NewMethod(ty.cast::<c_void>(), method_def.cast::<c_void>()) };
    if descriptor.is_null() {
        return -1;
    }
    // SAFETY: descriptor is a valid PyObject* and dict pointer is valid.
    let status = unsafe {
        PyDict_SetItemString(
            tp_dict,
            TP_INIT_WRAPPER_NAME.as_ptr().cast::<c_char>(),
            descriptor,
        )
    };
    unsafe { Py_DecRef(descriptor) };
    if status != 0 {
        return -1;
    }
    0
}

unsafe extern "C" fn cpython_type_mp_subscript_slot(
    object: *mut c_void,
    key: *mut c_void,
) -> *mut c_void {
    let trace_type_subscript = std::env::var_os("PYRS_TRACE_TYPE_SUBSCRIPT").is_some();
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("type mp_subscript missing VM context");
            return std::ptr::null_mut();
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("type mp_subscript received unknown object pointer");
            return std::ptr::null_mut();
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("type mp_subscript received unknown key pointer");
            return std::ptr::null_mut();
        };
        if trace_type_subscript {
            eprintln!(
                "[type-subscript] object_ptr={:p} key_ptr={:p} object={} key={}",
                object,
                key,
                cpython_value_debug_tag(&object_value),
                cpython_value_debug_tag(&key_value)
            );
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.getitem_value(object_value, key_value) {
            Ok(value) => {
                if trace_type_subscript {
                    eprintln!(
                        "[type-subscript] result={}",
                        cpython_value_debug_tag(&value)
                    );
                }
                context.alloc_cpython_ptr_for_value(value)
            }
            Err(err) => {
                if trace_type_subscript {
                    eprintln!("[type-subscript] error={}", err.message);
                }
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

pub(super) static mut PY_TYPE_MAPPING_METHODS: CpythonMappingMethods = CpythonMappingMethods {
    mp_length: std::ptr::null_mut(),
    mp_subscript: cpython_type_mp_subscript_slot as *mut c_void,
    mp_ass_subscript: std::ptr::null_mut(),
};

fn cpython_builtin_ctor_for_type_ptr(ty: *mut CpythonTypeObject) -> Option<BuiltinFunction> {
    let ptr = ty.cast::<c_void>();
    Some(if ptr == std::ptr::addr_of_mut!(PyBool_Type).cast() {
        BuiltinFunction::Bool
    } else if ptr == std::ptr::addr_of_mut!(PyLong_Type).cast() {
        BuiltinFunction::Int
    } else if ptr == std::ptr::addr_of_mut!(PyFloat_Type).cast() {
        BuiltinFunction::Float
    } else if ptr == std::ptr::addr_of_mut!(PyComplex_Type).cast() {
        BuiltinFunction::Complex
    } else if ptr == std::ptr::addr_of_mut!(PyUnicode_Type).cast() {
        BuiltinFunction::Str
    } else if ptr == std::ptr::addr_of_mut!(PyBytes_Type).cast() {
        BuiltinFunction::Bytes
    } else if ptr == std::ptr::addr_of_mut!(PyByteArray_Type).cast() {
        BuiltinFunction::ByteArray
    } else if ptr == std::ptr::addr_of_mut!(PyMemoryView_Type).cast() {
        BuiltinFunction::MemoryView
    } else if ptr == std::ptr::addr_of_mut!(PyList_Type).cast() {
        BuiltinFunction::List
    } else if ptr == std::ptr::addr_of_mut!(PyTuple_Type).cast() {
        BuiltinFunction::Tuple
    } else if ptr == std::ptr::addr_of_mut!(PyDict_Type).cast() {
        BuiltinFunction::Dict
    } else if ptr == std::ptr::addr_of_mut!(PySet_Type).cast() {
        BuiltinFunction::Set
    } else if ptr == std::ptr::addr_of_mut!(PyFrozenSet_Type).cast() {
        BuiltinFunction::FrozenSet
    } else if ptr == std::ptr::addr_of_mut!(PySlice_Type).cast() {
        BuiltinFunction::Slice
    } else if ptr == std::ptr::addr_of_mut!(PyRange_Type).cast() {
        BuiltinFunction::Range
    } else if ptr == std::ptr::addr_of_mut!(PyProperty_Type).cast() {
        BuiltinFunction::Property
    } else {
        return None;
    })
}

fn cpython_call_builtin_constructor(
    function: BuiltinFunction,
    positional: Vec<Value>,
    keywords: HashMap<String, Value>,
) -> Result<Value, String> {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return Err("missing VM context for builtin constructor".to_string());
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_internal(Value::Builtin(function), positional, keywords) {
            Ok(InternalCallOutcome::Value(value)) => Ok(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => Err(vm
                .runtime_error_from_active_exception("builtin constructor failed")
                .message),
            Err(err) => Err(err.message),
        }
    })?
}

fn cpython_build_type_from_three_arg_call(
    positional: &[Value],
    keywords: &HashMap<String, Value>,
) -> *mut c_void {
    if !keywords.is_empty() {
        cpython_set_error("TypeError: type() takes no keyword arguments");
        return std::ptr::null_mut();
    }
    let [name_value, bases_value, namespace_value] = positional else {
        cpython_set_error("TypeError: type() takes 1 or 3 arguments");
        return std::ptr::null_mut();
    };
    let Value::Str(name) = name_value else {
        cpython_set_error("TypeError: type() argument 1 must be str");
        return std::ptr::null_mut();
    };
    let namespace_entries = match namespace_value {
        Value::Dict(dict_obj) => match &*dict_obj.kind() {
            Object::Dict(entries) => entries.clone(),
            _ => {
                cpython_set_error("TypeError: type() argument 3 must be dict");
                return std::ptr::null_mut();
            }
        },
        _ => {
            cpython_set_error("TypeError: type() argument 3 must be dict");
            return std::ptr::null_mut();
        }
    };
    let owned_name = match CString::new(name.as_str()) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(format!("invalid type name: {err}"));
            return std::ptr::null_mut();
        }
    };
    let bases_ptr = cpython_new_ptr_for_value(bases_value.clone());
    if bases_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let mut spec = CpythonTypeSpec {
        name: owned_name.as_ptr(),
        basicsize: 0,
        itemsize: -1,
        flags: 0,
        slots: std::ptr::null_mut(),
    };
    let type_obj = cpython_type_from_spec_impl(
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::addr_of_mut!(spec).cast::<c_void>(),
        bases_ptr,
    );
    if std::env::var_os("PYRS_TRACE_CPY_TYPE_BUILD").is_some() {
        eprintln!(
            "[cpy-type-build] three-arg name={} bases_ptr={:p} type_obj={:p}",
            name, bases_ptr, type_obj
        );
    }
    if type_obj.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `type_obj` is a ready type object produced by `cpython_type_from_spec_impl`.
    let type_dict = unsafe { (*type_obj.cast::<CpythonTypeObject>()).tp_dict };
    if type_dict.is_null() {
        cpython_set_error("type() produced type without dictionary");
        return std::ptr::null_mut();
    }
    for (key, value) in namespace_entries {
        let Value::Str(key_name) = key else {
            cpython_set_error("TypeError: type() argument 3 contains non-string key");
            return std::ptr::null_mut();
        };
        let key_cstr = match CString::new(key_name.as_str()) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(format!("invalid class attribute name: {err}"));
                return std::ptr::null_mut();
            }
        };
        let value_ptr = cpython_new_ptr_for_value(value);
        if value_ptr.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: `type_dict` is a dict object and key/value pointers are valid PyObject*.
        let status = unsafe { PyDict_SetItemString(type_dict, key_cstr.as_ptr(), value_ptr) };
        // SAFETY: `PyDict_SetItemString` takes a borrowed reference.
        unsafe { Py_DecRef(value_ptr) };
        if status != 0 {
            return std::ptr::null_mut();
        }
    }
    type_obj
}

unsafe fn cpython_type_populate_method_descriptors(ty: *mut CpythonTypeObject) -> i32 {
    // SAFETY: caller passes a non-null type pointer.
    let mut method = unsafe { (*ty).tp_methods.cast::<CpythonMethodDef>() };
    if method.is_null() {
        return 0;
    }
    let method_align = align_of::<CpythonMethodDef>();
    if (method as usize) % method_align != 0 {
        if std::env::var_os("PYRS_TRACE_CPY_TYPE_METHODS").is_some() {
            eprintln!(
                "[cpy-type-methods] skip unaligned method table ty={:p} tp_methods={:p}",
                ty, method
            );
        }
        return 0;
    }
    loop {
        if (method as usize) % method_align != 0 {
            if std::env::var_os("PYRS_TRACE_CPY_TYPE_METHODS").is_some() {
                eprintln!(
                    "[cpy-type-methods] stop on unaligned entry ty={:p} method={:p}",
                    ty, method
                );
            }
            return 0;
        }
        // SAFETY: method table is terminated by null ml_name.
        let method_name_ptr = unsafe { (*method).ml_name };
        if method_name_ptr.is_null() {
            break;
        }
        let flags = unsafe { (*method).ml_flags };
        let descriptor = if (flags & METH_CLASS) != 0 {
            unsafe { PyDescr_NewClassMethod(ty.cast::<c_void>(), method.cast()) }
        } else {
            unsafe { PyDescr_NewMethod(ty.cast::<c_void>(), method.cast()) }
        };
        if descriptor.is_null() {
            return -1;
        }
        let status = unsafe { PyDict_SetItemString((*ty).tp_dict, method_name_ptr, descriptor) };
        unsafe { Py_DecRef(descriptor) };
        if status != 0 {
            return -1;
        }
        // SAFETY: contiguous method table entries.
        method = unsafe { method.add(1) };
    }
    0
}

pub(super) unsafe extern "C" fn cpython_type_tp_getattro(
    object: *mut c_void,
    name: *mut c_void,
) -> *mut c_void {
    if object.is_null() || name.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let attr_name = match cpython_value_from_ptr(name) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "type attribute name must be str",
            );
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let type_ptr = object.cast::<CpythonTypeObject>();
    let trace_type_getattr = std::env::var_os("PYRS_TRACE_TYPE_GETATTR").is_some();
    let trace_prepare =
        std::env::var_os("PYRS_TRACE_TYPE_PREPARE").is_some() && attr_name == "__prepare__";
    if trace_prepare {
        let object_tag = cpython_value_from_ptr(object)
            .map(|value| cpython_value_debug_tag(&value))
            .unwrap_or_else(|_| "<unresolved>".to_string());
        eprintln!(
            "[type-prepare] enter object={:p} type_object={} object_tag={}",
            object,
            super::cpython_is_type_object_ptr(object),
            object_tag
        );
    }
    if !type_ptr.is_null() {
        let type_name = cpython_type_name_from_tp_name(type_ptr);
        if trace_type_getattr
            && (type_name.contains("pybind11")
                || matches!(
                    attr_name.as_str(),
                    "_pybind11_conduit_v1_" | "__int__" | "__index__" | "__entries"
                ))
        {
            eprintln!(
                "[cpy-type-getattr] object={:p} type={} attr={}",
                object, type_name, attr_name
            );
        }
        if attr_name == "__name__" {
            return cpython_new_ptr_for_value(Value::Str(type_name));
        }
        if attr_name == "__qualname__" {
            return cpython_new_ptr_for_value(Value::Str(cpython_type_qualname_from_tp_name(
                type_ptr,
            )));
        }
        if attr_name == "__module__" {
            // CPython exposes `type.__module__` from the class dictionary when present.
            // Fall back to registry/tp_name synthesis only when the dict has no entry.
            // SAFETY: `type_ptr` is non-null and points to a type object.
            let dict_ptr = unsafe { (*type_ptr).tp_dict };
            if !dict_ptr.is_null() {
                // SAFETY: `dict_ptr` is a dictionary object for the active type.
                let module_attr =
                    unsafe { PyDict_GetItemString(dict_ptr, c"__module__".as_ptr()) };
                if !module_attr.is_null() {
                    // SAFETY: returning borrowed dict entry as new reference.
                    unsafe { Py_IncRef(module_attr) };
                    return module_attr;
                }
                // `_PyType_Lookup`-style callers must not leak incidental errors.
                if unsafe { !PyErr_Occurred().is_null() } {
                    unsafe { PyErr_Clear() };
                }
            }
            let module_name = cpython_heap_type_registry()
                .lock()
                .ok()
                .and_then(|registry| {
                    registry
                        .get(&(type_ptr as usize))
                        .map(|info| info.module_name.clone())
                })
                .unwrap_or_else(|| cpython_type_module_name_from_tp_name(type_ptr));
            return cpython_new_ptr_for_value(Value::Str(module_name));
        }
        if attr_name == "__dict__" {
            // SAFETY: `type_ptr` is non-null and points to a type object.
            let dict_ptr = unsafe { (*type_ptr).tp_dict };
            if !dict_ptr.is_null() {
                unsafe { Py_IncRef(dict_ptr) };
                return dict_ptr;
            }
        }
    }
    if let Ok(Some(attr_ptr)) = with_active_cpython_context_mut(|context| {
        context
            .lookup_type_attr_via_tp_dict(object, &attr_name)
            .or_else(|| context.lookup_type_attr_via_runtime_mro(object, &attr_name))
    }) && !attr_ptr.is_null()
    {
        if trace_prepare {
            let tag = cpython_value_from_ptr(attr_ptr)
                .map(|value| cpython_value_debug_tag(&value))
                .unwrap_or_else(|_| "<unresolved>".to_string());
            eprintln!(
                "[type-prepare] lookup-hit object={:p} attr_ptr={:p} value={}",
                object, attr_ptr, tag
            );
        }
        if attr_name == "__prepare__"
            && let Ok(Some(bound_ptr)) = with_active_cpython_context_mut(|context| {
                if context.vm.is_null() {
                    return None;
                }
                let Value::Class(class_obj) = context.cpython_value_from_ptr_or_proxy(object)?
                else {
                    return None;
                };
                // SAFETY: VM pointer is valid for active context lifetime.
                let vm = unsafe { &mut *context.vm };
                let resolved = match vm.load_attr_class(&class_obj, &attr_name) {
                    Ok(crate::vm::AttrAccessOutcome::Value(value)) => value,
                    Ok(crate::vm::AttrAccessOutcome::ExceptionHandled) => return None,
                    Err(_) => return None,
                };
                let ptr = context.alloc_cpython_ptr_for_value(resolved);
                (!ptr.is_null()).then_some(ptr)
            })
            && !bound_ptr.is_null()
        {
            if trace_prepare {
                let tag = cpython_value_from_ptr(bound_ptr)
                    .map(|value| cpython_value_debug_tag(&value))
                    .unwrap_or_else(|_| "<unresolved>".to_string());
                eprintln!(
                    "[type-prepare] explicit-bound-hit object={:p} ptr={:p} value={}",
                    object, bound_ptr, tag
                );
            }
            return bound_ptr;
        }
        // Runtime classmodel stores `classmethod`/`staticmethod` descriptors as
        // wrapper values. When they surface through raw tp_dict lookup we must
        // resolve the bound class attribute view (CPython descriptor parity)
        // instead of returning the wrapper object directly.
        if let Ok(Some(bound_ptr)) = with_active_cpython_context_mut(|context| {
            if context.vm.is_null() {
                return None;
            }
            let attr_value = context.cpython_value_from_ptr_or_proxy(attr_ptr)?;
            let needs_runtime_descriptor_binding = match attr_value {
                Value::Module(module_obj) => {
                    let Object::Module(module_data) = &*module_obj.kind() else {
                        return None;
                    };
                    module_data.name == "__classmethod__" || module_data.name == "__staticmethod__"
                }
                _ => false,
            };
            if !needs_runtime_descriptor_binding {
                return None;
            }
            let Value::Class(class_obj) = context.cpython_value_from_ptr_or_proxy(object)? else {
                return None;
            };
            // SAFETY: VM pointer is valid for active context lifetime.
            let vm = unsafe { &mut *context.vm };
            let resolved = match vm.load_attr_class(&class_obj, &attr_name) {
                Ok(crate::vm::AttrAccessOutcome::Value(value)) => value,
                Ok(crate::vm::AttrAccessOutcome::ExceptionHandled) => return None,
                Err(_) => return None,
            };
            let ptr = context.alloc_cpython_ptr_for_value(resolved);
            (!ptr.is_null()).then_some(ptr)
        }) && !bound_ptr.is_null()
        {
            if trace_type_getattr {
                eprintln!(
                    "[cpy-type-getattr] lookup-bound-hit object={:p} attr={} ptr={:p}",
                    object, attr_name, bound_ptr
                );
            }
            if trace_prepare {
                let tag = cpython_value_from_ptr(bound_ptr)
                    .map(|value| cpython_value_debug_tag(&value))
                    .unwrap_or_else(|_| "<unresolved>".to_string());
                eprintln!(
                    "[type-prepare] lookup-bound-hit object={:p} ptr={:p} value={}",
                    object, bound_ptr, tag
                );
            }
            return bound_ptr;
        }
        if trace_type_getattr {
            eprintln!(
                "[cpy-type-getattr] lookup-hit object={:p} attr={} ptr={:p}",
                object, attr_name, attr_ptr
            );
        }
        return attr_ptr;
    }
    // Type attribute lookup also consults metatype attributes/descriptors.
    if let Ok(Some(attr_ptr)) = with_active_cpython_context_mut(|context| {
        // SAFETY: `object` follows type slot dispatch contract.
        let metatype = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<c_void>())
                .unwrap_or(std::ptr::null_mut())
        };
        if metatype.is_null() {
            return None;
        }
        context
            .lookup_type_attr_via_tp_dict(metatype, &attr_name)
            .or_else(|| context.lookup_type_attr_via_runtime_mro(metatype, &attr_name))
    }) && !attr_ptr.is_null()
    {
        if trace_prepare {
            let tag = cpython_value_from_ptr(attr_ptr)
                .map(|value| cpython_value_debug_tag(&value))
                .unwrap_or_else(|_| "<unresolved>".to_string());
            eprintln!(
                "[type-prepare] metatype-hit object={:p} attr_ptr={:p} value={}",
                object, attr_ptr, tag
            );
        }
        if trace_type_getattr {
            eprintln!(
                "[cpy-type-getattr] metatype-hit object={:p} attr={} ptr={:p}",
                object, attr_name, attr_ptr
            );
        }
        // Bind metaclass descriptors against the original class object so
        // methods like ABCMeta.register receive `cls` correctly.
        if let Ok(Some(bound_ptr)) = with_active_cpython_context_mut(|context| {
            if context.vm.is_null() {
                return None;
            }
            let Value::Class(class_obj) = context.cpython_value_from_ptr_or_proxy(object)? else {
                return None;
            };
            // SAFETY: VM pointer is valid for active context lifetime.
            let vm = unsafe { &mut *context.vm };
            let resolved = match vm.load_attr_class(&class_obj, &attr_name) {
                Ok(crate::vm::AttrAccessOutcome::Value(value)) => value,
                Ok(crate::vm::AttrAccessOutcome::ExceptionHandled) => return None,
                Err(_) => return None,
            };
            Some(context.alloc_cpython_ptr_for_value(resolved))
        }) && !bound_ptr.is_null()
        {
            if trace_type_getattr {
                eprintln!(
                    "[cpy-type-getattr] metatype-bound-hit object={:p} attr={} ptr={:p}",
                    object, attr_name, bound_ptr
                );
            }
            if trace_prepare {
                let tag = cpython_value_from_ptr(bound_ptr)
                    .map(|value| cpython_value_debug_tag(&value))
                    .unwrap_or_else(|_| "<unresolved>".to_string());
                eprintln!(
                    "[type-prepare] metatype-bound-hit object={:p} ptr={:p} value={}",
                    object, bound_ptr, tag
                );
            }
            return bound_ptr;
        }
        return attr_ptr;
    }
    // Runtime classes can carry metaclass state that is richer than the raw
    // `ob_type` fallback visible through C-API compat pointers (for example
    // ABC registration surfaces used by SciPy/Cython).
    let runtime_metaclass_lookup = with_active_cpython_context_mut(|context| {
        let object_value = context.cpython_value_from_ptr_or_proxy(object)?;
        let metaclass_obj = match object_value {
            Value::Class(class_obj) => match &*class_obj.kind() {
                Object::Class(class_data) => class_data.metaclass.clone(),
                _ => None,
            },
            // Some external class objects may currently materialize as proxy
            // instances; in that case the instance class is the effective
            // metaclass for type-attribute lookup.
            Value::Instance(instance_obj) => match &*instance_obj.kind() {
                Object::Instance(instance_data) => Some(instance_data.class.clone()),
                _ => None,
            },
            _ => None,
        }?;
        let metaclass_value = Value::Class(metaclass_obj);
        let metaclass_ptr = ModuleCapiContext::cpython_proxy_raw_ptr_from_value(&metaclass_value)
            .unwrap_or_else(|| context.alloc_cpython_ptr_for_value(metaclass_value.clone()));
        if metaclass_ptr.is_null() {
            return None;
        }
        context
            .lookup_type_attr_via_tp_dict(metaclass_ptr, &attr_name)
            .or_else(|| context.lookup_type_attr_via_runtime_mro(metaclass_ptr, &attr_name))
    });
    if let Ok(Some(attr_ptr)) = runtime_metaclass_lookup
        && !attr_ptr.is_null()
    {
        if trace_prepare {
            let tag = cpython_value_from_ptr(attr_ptr)
                .map(|value| cpython_value_debug_tag(&value))
                .unwrap_or_else(|_| "<unresolved>".to_string());
            eprintln!(
                "[type-prepare] runtime-metaclass-hit object={:p} attr_ptr={:p} value={}",
                object, attr_ptr, tag
            );
        }
        if trace_type_getattr {
            eprintln!(
                "[cpy-type-getattr] runtime-metaclass-hit object={:p} attr={} ptr={:p}",
                object, attr_name, attr_ptr
            );
        }
        if let Ok(Some(bound_ptr)) = with_active_cpython_context_mut(|context| {
            if context.vm.is_null() {
                return None;
            }
            let Value::Class(class_obj) = context.cpython_value_from_ptr_or_proxy(object)? else {
                return None;
            };
            // SAFETY: VM pointer is valid for active context lifetime.
            let vm = unsafe { &mut *context.vm };
            let resolved = match vm.load_attr_class(&class_obj, &attr_name) {
                Ok(crate::vm::AttrAccessOutcome::Value(value)) => value,
                Ok(crate::vm::AttrAccessOutcome::ExceptionHandled) => return None,
                Err(_) => return None,
            };
            Some(context.alloc_cpython_ptr_for_value(resolved))
        }) && !bound_ptr.is_null()
        {
            if trace_type_getattr {
                eprintln!(
                    "[cpy-type-getattr] runtime-metaclass-bound-hit object={:p} attr={} ptr={:p}",
                    object, attr_name, bound_ptr
                );
            }
            if trace_prepare {
                let tag = cpython_value_from_ptr(bound_ptr)
                    .map(|value| cpython_value_debug_tag(&value))
                    .unwrap_or_else(|_| "<unresolved>".to_string());
                eprintln!(
                    "[type-prepare] runtime-metaclass-bound-hit object={:p} ptr={:p} value={}",
                    object, bound_ptr, tag
                );
            }
            return bound_ptr;
        }
        return attr_ptr;
    }
    // CPython `type_getattro` resolves metatype attributes such as
    // `type.__prepare__` from the metatype method surface. In pyrs, builtin
    // type objects are represented as builtin values in VM metadata; bridge
    // those attributes here so C-extension metaclass flows (for example Cython
    // class builders in pandas) observe CPython-equivalent behavior.
    if let Ok(Some(attr_ptr)) = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return None;
        }
        // For type objects created via C-API, attribute resolution can require
        // either the object itself or its metatype (`ob_type`) builtin surface.
        let object_type_ptr = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<c_void>())
                .unwrap_or(std::ptr::null_mut())
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        for candidate_ptr in [object, object_type_ptr] {
            let Some(builtin_type_name) = cpython_builtin_type_name_for_ptr(candidate_ptr) else {
                continue;
            };
            let builtin = match vm.builtins.get(builtin_type_name).cloned() {
                Some(Value::Builtin(builtin)) => builtin,
                _ => continue,
            };
            if let Ok(resolved) = vm.load_attr_builtin(builtin, &attr_name) {
                let ptr = context.alloc_cpython_ptr_for_value(resolved);
                if !ptr.is_null() {
                    return Some(ptr);
                }
            }
        }
        None
    }) && !attr_ptr.is_null()
    {
        if trace_prepare {
            let tag = cpython_value_from_ptr(attr_ptr)
                .map(|value| cpython_value_debug_tag(&value))
                .unwrap_or_else(|_| "<unresolved>".to_string());
            eprintln!(
                "[type-prepare] builtin-fallback-hit object={:p} attr_ptr={:p} value={}",
                object, attr_ptr, tag
            );
        }
        if trace_type_getattr {
            eprintln!(
                "[cpy-type-getattr] builtin-type-fallback-hit object={:p} attr={} ptr={:p}",
                object, attr_name, attr_ptr
            );
        }
        return attr_ptr;
    }
    if trace_type_getattr {
        let _ = with_active_cpython_context_mut(|context| {
            let mapped_value = context.cpython_value_from_ptr_or_proxy(object);
            let value_tag = mapped_value
                .as_ref()
                .map(cpython_value_debug_tag)
                .unwrap_or_else(|| "None".to_string());
            let metaclass_name = mapped_value.as_ref().and_then(|value| match value {
                Value::Class(class_obj) => match &*class_obj.kind() {
                    Object::Class(class_data) => {
                        class_data.metaclass.as_ref().and_then(|metaclass| {
                            match &*metaclass.kind() {
                                Object::Class(meta_data) => Some(meta_data.name.clone()),
                                _ => None,
                            }
                        })
                    }
                    _ => None,
                },
                _ => None,
            });
            eprintln!(
                "[cpy-type-getattr] runtime-metaclass-miss object={:p} attr={} value_tag={} metaclass={}",
                object,
                attr_name,
                value_tag,
                metaclass_name.unwrap_or_else(|| "<none>".to_string())
            );
        });
    }
    // SAFETY: object pointer follows type slot dispatch contract.
    let type_ptr = unsafe {
        object
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    let type_name = if type_ptr.is_null() {
        "type".to_string()
    } else {
        unsafe { c_name_to_string((*type_ptr).tp_name) }.unwrap_or_else(|_| "type".to_string())
    };
    cpython_set_typed_error(
        unsafe { PyExc_AttributeError },
        format!("type '{type_name}' has no attribute '{attr_name}'"),
    );
    if attr_name == "__getitem__"
        && type_name == "type"
        && std::env::var_os("PYRS_TRACE_TYPE_GETATTR_BT").is_some()
    {
        eprintln!(
            "[cpy-type-getattr-bt] object={:p} attr={} type={}\n{}",
            object,
            attr_name,
            type_name,
            Backtrace::capture()
        );
    }
    if trace_type_getattr {
        eprintln!(
            "[cpy-type-getattr] lookup-miss object={:p} type={} attr={}",
            object, type_name, attr_name
        );
    }
    std::ptr::null_mut()
}

pub(super) unsafe extern "C" fn cpython_type_tp_setattro(
    object: *mut c_void,
    name: *mut c_void,
    value: *mut c_void,
) -> c_int {
    if object.is_null() || name.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let attr_name = match cpython_value_from_ptr(name) {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            cpython_set_typed_error(
                unsafe { PyExc_TypeError },
                "type attribute name must be str",
            );
            return -1;
        }
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let trace_type_setattr = std::env::var_os("PYRS_TRACE_TYPE_SETATTR").is_some();
    let type_name = if object.is_null() {
        "<null>".to_string()
    } else {
        // SAFETY: object pointer follows type slot dispatch contract.
        unsafe {
            object
                .cast::<CpythonTypeObject>()
                .as_ref()
                .map(|ty| cpython_type_name_from_tp_name(ty as *const _ as *mut _))
                .unwrap_or_else(|| "<invalid>".to_string())
        }
    };
    if trace_type_setattr
        && (type_name.contains("pybind11")
            || attr_name.starts_with("__")
            || attr_name.contains("entries"))
    {
        eprintln!(
            "[cpy-type-setattr] object={:p} type={} attr={} value={:p}",
            object, type_name, attr_name, value
        );
    }
    if let Ok(Some(status)) = with_active_cpython_context_mut(|context| {
        // SAFETY: `object` is expected to be a valid type object for tp_setattro.
        let type_ptr = object.cast::<CpythonTypeObject>();
        if type_ptr.is_null() {
            return None;
        }
        // SAFETY: type pointer shape is validated by CPython slot dispatch contract.
        let dict_ptr = unsafe { (*type_ptr).tp_dict };
        if dict_ptr.is_null() {
            return None;
        }
        let key_ptr = match context.scratch_c_string_ptr(&attr_name) {
            Ok(ptr) => ptr,
            Err(err) => {
                context.set_error(err);
                return Some(-1);
            }
        };
        let result = if value.is_null() {
            // SAFETY: dict pointer + key follow PyDict C-API contract.
            unsafe { PyDict_DelItemString(dict_ptr, key_ptr) }
        } else {
            // SAFETY: dict pointer + key/value follow PyDict C-API contract.
            unsafe { PyDict_SetItemString(dict_ptr, key_ptr, value) }
        };
        if result == 0 {
            if trace_type_setattr {
                eprintln!(
                    "[cpy-type-setattr] dict-write-ok type={} attr={} delete={}",
                    type_name,
                    attr_name,
                    value.is_null()
                );
            }
            return Some(0);
        }
        if trace_type_setattr {
            eprintln!(
                "[cpy-type-setattr] dict-write-miss type={} attr={} delete={}",
                type_name,
                attr_name,
                value.is_null()
            );
        }
        None
    }) {
        return status;
    }
    cpython_set_typed_error(
        unsafe { PyExc_AttributeError },
        format!("cannot set attribute '{attr_name}' on type object"),
    );
    if trace_type_setattr {
        eprintln!(
            "[cpy-type-setattr] reject type={} attr={} value={:p}",
            type_name, attr_name, value
        );
    }
    -1
}

pub(super) unsafe extern "C" fn cpython_type_tp_call(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    thread_local! {
        static ACTIVE_RUNTIME_TYPE_CALL_FALLBACK: std::cell::RefCell<Vec<usize>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    let trace_calls = std::env::var_os("PYRS_TRACE_CPY_CALLS").is_some();
    let trace_seed_calls = std::env::var_os("PYRS_TRACE_SEED_CALLS").is_some();
    if callable.is_null() {
        cpython_set_error("type call received null callable");
        return std::ptr::null_mut();
    }
    if callable == (&raw mut PyType_Type).cast() {
        let positional = match cpython_positional_args_from_tuple_object(args) {
            Ok(values) => values,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
        let keywords = match cpython_keyword_args_from_dict_object(kwargs) {
            Ok(values) => values,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
        if positional.len() == 1 && keywords.is_empty() {
            let ptr = cpython_new_ptr_for_value(positional[0].clone());
            if ptr.is_null() {
                return std::ptr::null_mut();
            }
            // SAFETY: object pointer was materialized by `cpython_new_ptr_for_value`.
            let ty = unsafe { (*ptr.cast::<CpythonObjectHead>()).ob_type };
            unsafe { Py_XIncRef(ty) };
            return ty;
        }
        if positional.len() != 3 {
            cpython_set_error("TypeError: type() takes 1 or 3 arguments");
            return std::ptr::null_mut();
        }
        return cpython_build_type_from_three_arg_call(&positional, &keywords);
    }
    let ty = callable.cast::<CpythonTypeObject>();
    let callable_name =
        unsafe { c_name_to_string((*ty).tp_name) }.unwrap_or_else(|_| "<unnamed>".to_string());
    // Runtime-backed heap classes currently materialize with generic/new slots.
    // Route these through VM class-call semantics so Python-level __init__/descriptor
    // initialization runs (CPython parity for pure-Python classes exposed via C-API calls).
    if let Ok(callable_value) = cpython_value_from_ptr_or_proxy(callable)
        && let Value::Class(class_obj) = &callable_value
    {
        let is_proxy_backed_class = matches!(
            &*class_obj.kind(),
            Object::Class(class_data) if super::is_cpython_proxy_class(class_data)
        );
        let use_runtime_class_call =
            unsafe { (*ty).tp_new == PyType_GenericNew as *mut c_void && (*ty).tp_init.is_null() };
        if use_runtime_class_call && !is_proxy_backed_class {
            let callable_key = callable as usize;
            let already_active = ACTIVE_RUNTIME_TYPE_CALL_FALLBACK
                .with(|active| active.borrow().contains(&callable_key));
            if already_active {
                if std::env::var_os("PYRS_TRACE_TYPE_RUNTIME_CALL_FALLBACK").is_some() {
                    eprintln!(
                        "[cpy-type-call] runtime-fallback-skip(reentry) callable={:p} name={}",
                        callable, callable_name
                    );
                }
            } else {
                struct RuntimeTypeFallbackGuard {
                    callable_key: usize,
                }
                impl Drop for RuntimeTypeFallbackGuard {
                    fn drop(&mut self) {
                        ACTIVE_RUNTIME_TYPE_CALL_FALLBACK.with(|active| {
                            let mut stack = active.borrow_mut();
                            if let Some(index) =
                                stack.iter().rposition(|entry| *entry == self.callable_key)
                            {
                                stack.remove(index);
                            }
                        });
                    }
                }
                ACTIVE_RUNTIME_TYPE_CALL_FALLBACK.with(|active| {
                    active.borrow_mut().push(callable_key);
                });
                let _guard = RuntimeTypeFallbackGuard { callable_key };
                if std::env::var_os("PYRS_TRACE_TYPE_RUNTIME_CALL_FALLBACK").is_some() {
                    eprintln!(
                        "[cpy-type-call] runtime-fallback callable={:p} name={}",
                        callable, callable_name
                    );
                }
                let positional = match cpython_positional_args_from_tuple_object(args) {
                    Ok(values) => values,
                    Err(err) => {
                        cpython_set_error(err);
                        return std::ptr::null_mut();
                    }
                };
                let keyword_pairs = match cpython_keyword_args_from_dict_object(kwargs) {
                    Ok(values) => values,
                    Err(err) => {
                        cpython_set_error(err);
                        return std::ptr::null_mut();
                    }
                };
                let runtime_result = with_active_cpython_context_mut(|context| {
                    cpython_call_internal_in_context(
                        context,
                        callable_value,
                        positional,
                        keyword_pairs
                            .into_iter()
                            .collect::<HashMap<String, Value>>(),
                    )
                });
                match runtime_result {
                    Ok(Ok(value)) => return cpython_new_ptr_for_value(value),
                    Ok(Err(err)) | Err(err) => {
                        cpython_set_error(err);
                        return std::ptr::null_mut();
                    }
                }
            }
        }
    }
    if std::env::var_os("PYRS_TRACE_NUMPY_DTYPE_ARGS").is_some()
        && callable_name.to_ascii_lowercase().contains("dtype")
    {
        let tuple_len = if args.is_null() {
            -1
        } else {
            unsafe { PyTuple_Size(args) }
        };
        eprintln!(
            "[numpy-dtype-call] callable={:p} args_ptr={:p} kwargs_ptr={:p} tuple_len={}",
            callable, args, kwargs, tuple_len
        );
        if tuple_len > 0 {
            for idx in 0..tuple_len {
                let item_ptr = unsafe { PyTuple_GetItem(args, idx) };
                let item_type_name = if item_ptr.is_null() {
                    "<null>".to_string()
                } else {
                    unsafe {
                        item_ptr
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                            .filter(|ty| !ty.is_null())
                            .and_then(|ty| c_name_to_string((*ty).tp_name).ok())
                            .unwrap_or_else(|| "<unknown>".to_string())
                    }
                };
                let item_value = with_active_cpython_context_mut(|context| {
                    context
                        .cpython_value_from_ptr_or_proxy(item_ptr)
                        .map(|value| match value {
                            Value::Str(text) => format!("Str({text})"),
                            other => cpython_value_debug_tag(&other),
                        })
                        .unwrap_or_else(|| "<unresolved>".to_string())
                })
                .unwrap_or_else(|_| "<no-context>".to_string());
                eprintln!(
                    "[numpy-dtype-call] arg[{}]={:p} type={} value={}",
                    idx, item_ptr, item_type_name, item_value
                );
            }
        }
    }
    // SAFETY: callable points to a PyTypeObject-compatible struct.
    let new_slot = unsafe { (*ty).tp_new };
    if new_slot.is_null() {
        if let Some(function) = cpython_builtin_ctor_for_type_ptr(ty) {
            let positional = match cpython_positional_args_from_tuple_object(args) {
                Ok(values) => values,
                Err(err) => {
                    cpython_set_error(err);
                    return std::ptr::null_mut();
                }
            };
            let keywords = match cpython_keyword_args_from_dict_object(kwargs) {
                Ok(values) => values,
                Err(err) => {
                    cpython_set_error(err);
                    return std::ptr::null_mut();
                }
            };
            let value = match cpython_call_builtin_constructor(function, positional, keywords) {
                Ok(value) => value,
                Err(err) => {
                    cpython_set_error(err);
                    return std::ptr::null_mut();
                }
            };
            return cpython_new_ptr_for_value(value);
        }
        cpython_set_error(format!(
            "TypeError: cannot create '{}' instances",
            callable_name
        ));
        return std::ptr::null_mut();
    }
    if trace_calls {
        // SAFETY: callable points to a PyTypeObject-compatible struct.
        let init_slot = unsafe { (*ty).tp_init };
        let trace_name = callable_name.as_str();
        eprintln!(
            "[cpy-type-call] callable={:p} name={} tp_new={:p} tp_init={:p} args_ptr={:p} kwargs_ptr={:p}",
            callable, trace_name, new_slot, init_slot, args, kwargs
        );
    }
    if trace_seed_calls {
        let callable_name =
            unsafe { c_name_to_string((*ty).tp_name) }.unwrap_or_else(|_| "<unnamed>".to_string());
        if callable_name.contains("SeedSequence")
            || callable_name.contains("BitGenerator")
            || callable_name.contains("RandomState")
            || callable_name.contains("MT19937")
        {
            let (tp_flags, tp_basicsize, tp_itemsize, tp_alloc, tp_new, tp_init) = unsafe {
                (
                    (*ty).tp_flags,
                    (*ty).tp_basicsize,
                    (*ty).tp_itemsize,
                    (*ty).tp_alloc,
                    (*ty).tp_new,
                    (*ty).tp_init,
                )
            };
            let tuple_len = if args.is_null() {
                -1
            } else {
                unsafe { PyTuple_Size(args) }
            };
            eprintln!(
                "[seed-type-call] callable={:p} name={} flags=0x{:x} basicsize={} itemsize={} tp_alloc={:p} tp_new={:p} tp_init={:p} args_ptr={:p} kwargs_ptr={:p} tuple_len={}",
                callable,
                callable_name,
                tp_flags,
                tp_basicsize,
                tp_itemsize,
                tp_alloc,
                tp_new,
                tp_init,
                args,
                kwargs,
                tuple_len
            );
            if tuple_len > 0 {
                for idx in 0..(tuple_len.min(6)) {
                    let item_ptr = unsafe { PyTuple_GetItem(args, idx) };
                    let item_type = if item_ptr.is_null() {
                        "<null>".to_string()
                    } else {
                        // SAFETY: best-effort type read for trace diagnostics.
                        unsafe {
                            item_ptr
                                .cast::<CpythonObjectHead>()
                                .as_ref()
                                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                                .filter(|ty| !ty.is_null())
                                .and_then(|ty| c_name_to_string((*ty).tp_name).ok())
                                .unwrap_or_else(|| "<unknown>".to_string())
                        }
                    };
                    eprintln!(
                        "[seed-type-call] arg[{}]={:p} type={}",
                        idx, item_ptr, item_type
                    );
                }
            }
        }
    }
    let new_fn: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
        // SAFETY: tp_new follows CPython `newfunc` signature.
        unsafe { std::mem::transmute(new_slot) };
    let object = unsafe { new_fn(callable, args, kwargs) };
    if trace_calls {
        let object_type = if object.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: object returned by tp_new is expected to be PyObject-compatible.
            unsafe { (*object.cast::<CpythonObjectHead>()).ob_type }
        };
        let object_type_name = if object_type.is_null() {
            "<null>".to_string()
        } else {
            // SAFETY: type pointer came from a freshly created object.
            unsafe {
                c_name_to_string((*object_type.cast::<CpythonTypeObject>()).tp_name)
                    .unwrap_or_else(|_| "<unnamed>".to_string())
            }
        };
        eprintln!(
            "[cpy-type-call] tp_new_result name={} object={:p} object_type={:p} object_type_name={}",
            callable_name, object, object_type, object_type_name
        );
    }
    if object.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: object returned by tp_new must be PyObject-compatible.
    let object_type = unsafe { (*object.cast::<CpythonObjectHead>()).ob_type };
    let should_init = unsafe { PyType_IsSubtype(object_type, callable) != 0 };
    if !should_init {
        return object;
    }
    if trace_seed_calls && callable_name.contains("MT19937") {
        let _ = with_active_cpython_context_mut(|context| {
            if context.vm.is_null() {
                return;
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *context.vm };
            let Some(module) = vm.modules.get("numpy.random.bit_generator").cloned() else {
                eprintln!("[seed-type-call] bit_generator module not present");
                return;
            };
            let Some(classinfo) = (match &*module.kind() {
                Object::Module(module_data) => module_data.globals.get("ISeedSequence").cloned(),
                _ => None,
            }) else {
                eprintln!("[seed-type-call] ISeedSequence missing from bit_generator");
                return;
            };
            let class_raw_ptr =
                super::ModuleCapiContext::cpython_proxy_raw_ptr_from_value(&classinfo);
            let is_proxy_class = matches!(
                &classinfo,
                Value::Class(class_obj)
                    if matches!(&*class_obj.kind(), Object::Class(class_data) if super::is_cpython_proxy_class(class_data))
            );
            let none_is_instance = vm
                .value_is_instance_of(&Value::None, &classinfo)
                .unwrap_or(false);
            let module_ptr = context.alloc_cpython_ptr_for_value(Value::Module(module.clone()));
            let i_seed_capi = if module_ptr.is_null() {
                std::ptr::null_mut()
            } else {
                let name_ptr = b"ISeedSequence\0";
                // SAFETY: module ptr + static nul-terminated attribute name.
                unsafe { super::PyObject_GetAttrString(module_ptr, name_ptr.as_ptr().cast()) }
            };
            let i_seed_capi_tag = context
                .cpython_value_from_borrowed_ptr(i_seed_capi)
                .map(|value| cpython_value_debug_tag(&value))
                .unwrap_or_else(|| "<unresolved>".to_string());
            eprintln!(
                "[seed-type-call] bit_generator.ISeedSequence={} proxy_class={} raw_ptr={:?} isinstance(None, ISeedSequence)={} capi_ptr={:p} capi_value={}",
                cpython_value_debug_tag(&classinfo),
                is_proxy_class,
                class_raw_ptr,
                none_is_instance,
                i_seed_capi,
                i_seed_capi_tag
            );
        });
    }
    let init_slot = unsafe {
        object_type
            .cast::<CpythonTypeObject>()
            .as_ref()
            .map(|object_type| object_type.tp_init)
            .unwrap_or(std::ptr::null_mut())
    };
    if init_slot.is_null() {
        return object;
    }
    let init_fn: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
        // SAFETY: tp_init follows CPython `initproc` signature.
        unsafe { std::mem::transmute(init_slot) };
    let status = unsafe { init_fn(object, args, kwargs) };
    if trace_seed_calls
        && (callable_name.contains("SeedSequence")
            || callable_name.contains("BitGenerator")
            || callable_name.contains("RandomState")
            || callable_name.contains("MT19937"))
    {
        let mut last_error = String::new();
        let _ = with_active_cpython_context_mut(|context| {
            if let Some(err) = context.last_error.as_ref() {
                last_error = err.clone();
            }
        });
        eprintln!(
            "[seed-type-call] init-status callable={} object={:p} status={} last_error={}",
            callable_name, object, status, last_error
        );
    }
    if status < 0 {
        if std::env::var_os("PYRS_TRACE_TYPE_INIT_FAILURE").is_some() {
            let mut last_error = String::new();
            let _ = with_active_cpython_context_mut(|context| {
                if let Some(err) = context.last_error.as_ref() {
                    last_error = err.clone();
                }
            });
            let object_type_name = unsafe {
                object_type
                    .cast::<CpythonTypeObject>()
                    .as_ref()
                    .map(|raw| {
                        c_name_to_string(raw.tp_name).unwrap_or_else(|_| "<unnamed>".to_string())
                    })
                    .unwrap_or_else(|| "<null>".to_string())
            };
            eprintln!(
                "[cpy-type-call] init-failed callable={} object_type={} tp_init={:p} last_error={}",
                callable_name, object_type_name, init_slot, last_error
            );
        }
        unsafe { Py_DecRef(object) };
        return std::ptr::null_mut();
    }
    if trace_calls {
        eprintln!(
            "[cpy-type-call] init complete object={:p} object_type={:p} tp_init={:p}",
            object, object_type, init_slot
        );
    }
    if trace_calls {
        let object_type_name = unsafe {
            object_type
                .cast::<CpythonTypeObject>()
                .as_ref()
                .map(|raw| {
                    c_name_to_string(raw.tp_name).unwrap_or_else(|_| "<unnamed>".to_string())
                })
                .unwrap_or_else(|| "<null>".to_string())
        };
        eprintln!(
            "[cpy-type-call] callable_type={} object_type={} should_init={}",
            callable_name, object_type_name, should_init
        );
    }
    object
}

pub(super) fn cpython_is_type_object_ptr(ptr: *mut c_void) -> bool {
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if ptr.is_null() {
        return false;
    }
    if (ptr as usize) < MIN_VALID_PTR {
        return false;
    }
    if (ptr as usize) % std::mem::align_of::<CpythonObjectHead>() != 0 {
        return false;
    }
    if !ModuleCapiContext::is_probable_type_object_without_metatype(ptr) {
        return false;
    }
    let type_type = std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>();
    if ptr == type_type {
        return true;
    }
    // SAFETY: `ptr` is assumed to reference a CPython object header.
    let object_type = unsafe {
        ptr.cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type)
            .unwrap_or(std::ptr::null_mut())
    };
    if object_type.is_null() {
        return ModuleCapiContext::is_probable_type_object_without_metatype(ptr);
    }
    if (object_type as usize) < MIN_VALID_PTR {
        return false;
    }
    if (object_type as usize) % std::mem::align_of::<CpythonObjectHead>() != 0 {
        return false;
    }
    let object_type_obj = object_type.cast::<CpythonTypeObject>();
    // SAFETY: object_type was loaded from a non-null object header above.
    let object_type_flags = unsafe {
        object_type_obj
            .as_ref()
            .map(|ty| ty.tp_flags)
            .unwrap_or_default()
    };
    if object_type == type_type || (object_type_flags & PY_TPFLAGS_TYPE_SUBCLASS) != 0 {
        return true;
    }
    // SAFETY: object_type and type_type are validated non-null pointers.
    if unsafe { PyType_IsSubtype(object_type, type_type) != 0 } {
        return true;
    }
    // Heap types created through `PyType_FromSpec*` are tracked in the registry even
    // when fast-subclass flags are still incomplete during bring-up.
    cpython_heap_type_registry()
        .lock()
        .ok()
        .is_some_and(|registry| registry.contains_key(&(ptr as usize)))
}

fn cpython_type_ptr_from_value(value: &Value) -> Option<*mut CpythonTypeObject> {
    if let Some(raw) = ModuleCapiContext::cpython_proxy_raw_ptr_from_value(value)
        && cpython_is_type_object_ptr(raw)
    {
        return Some(raw.cast::<CpythonTypeObject>());
    }
    match value {
        Value::Builtin(BuiltinFunction::Type) => Some(std::ptr::addr_of_mut!(PyType_Type)),
        Value::Class(class_obj) => {
            let Object::Class(class_data) = &*class_obj.kind() else {
                return None;
            };
            cpython_builtin_type_ptr_for_class_name(&class_data.name)
                .filter(|ptr| cpython_is_type_object_ptr(*ptr))
                .map(|ptr| ptr.cast::<CpythonTypeObject>())
        }
        _ => None,
    }
}

fn cpython_resolve_type_base_from_arg(
    bases: *mut c_void,
) -> Result<*mut CpythonTypeObject, String> {
    let default = std::ptr::addr_of_mut!(PyBaseObject_Type);
    if bases.is_null() {
        return Ok(default);
    }
    if cpython_is_type_object_ptr(bases) {
        return Ok(bases.cast::<CpythonTypeObject>());
    }
    let value = cpython_value_from_ptr(bases)?;
    if std::env::var_os("PYRS_TRACE_CPY_TYPE_BUILD").is_some() {
        eprintln!(
            "[cpy-type-build] resolve-bases ptr={:p} value={}",
            bases,
            cpython_value_debug_tag(&value)
        );
    }
    match value {
        Value::Tuple(tuple_obj) => {
            let Object::Tuple(items) = &*tuple_obj.kind() else {
                return Ok(default);
            };
            if std::env::var_os("PYRS_TRACE_CPY_TYPE_BUILD").is_some() {
                let summary = items
                    .iter()
                    .map(cpython_value_debug_tag)
                    .collect::<Vec<_>>()
                    .join(", ");
                eprintln!("[cpy-type-build] resolve-bases tuple=[{}]", summary);
            }
            for item in items {
                if let Some(base) = cpython_type_ptr_from_value(item) {
                    return Ok(base);
                }
            }
            // Fallback to raw tuple-pointer inspection for foreign extension tuples whose
            // items are not yet mapped into runtime Values.
            let raw_len = unsafe { PyTuple_Size(bases) };
            if raw_len > 0 {
                for index in 0..(raw_len as usize) {
                    let raw_item = unsafe { PyTuple_GetItem(bases, index as isize) };
                    if raw_item.is_null() {
                        continue;
                    }
                    if cpython_is_type_object_ptr(raw_item) {
                        return Ok(raw_item.cast::<CpythonTypeObject>());
                    }
                }
            }
            Ok(default)
        }
        _ => Err("bases must be a type or tuple of types".to_string()),
    }
}

fn cpython_split_type_name(full_name: &str) -> (String, String) {
    if let Some((module_name, qualname)) = full_name.rsplit_once('.')
        && !module_name.is_empty()
        && !qualname.is_empty()
    {
        return (module_name.to_string(), qualname.to_string());
    }
    ("builtins".to_string(), full_name.to_string())
}

fn cpython_type_name_from_tp_name(type_ptr: *mut CpythonTypeObject) -> String {
    // SAFETY: `type_ptr` is expected to reference a type object.
    let full = unsafe { c_name_to_string((*type_ptr).tp_name) }
        .unwrap_or_else(|_| "<unnamed>".to_string());
    full.rsplit('.').next().unwrap_or(full.as_str()).to_string()
}

fn cpython_type_is_heap_type(type_ptr: *mut CpythonTypeObject) -> bool {
    if type_ptr.is_null() {
        return false;
    }
    // SAFETY: caller ensures a non-null type pointer.
    unsafe { ((*type_ptr).tp_flags & PY_TPFLAGS_HEAPTYPE) != 0 }
}

fn cpython_type_qualname_from_tp_name(type_ptr: *mut CpythonTypeObject) -> String {
    cpython_type_name_from_tp_name(type_ptr)
}

fn cpython_type_module_name_from_tp_name(type_ptr: *mut CpythonTypeObject) -> String {
    // SAFETY: `type_ptr` is expected to reference a type object.
    let full = unsafe { c_name_to_string((*type_ptr).tp_name) }
        .unwrap_or_else(|_| "<unnamed>".to_string());
    full.rsplit_once('.')
        .map(|(module_name, _)| module_name.to_string())
        .unwrap_or_else(|| "builtins".to_string())
}

#[derive(Default)]
struct CpythonTypeSlotScratch {
    async_allocated: bool,
    number_allocated: bool,
    sequence_allocated: bool,
    mapping_allocated: bool,
    buffer_allocated: bool,
}

fn cpython_clone_or_alloc_slot_table<T>(
    current: *mut c_void,
    allocated: &mut bool,
    table_name: &str,
) -> Result<*mut T, String> {
    if current.is_null() {
        let raw = unsafe { calloc(1, std::mem::size_of::<T>()) }.cast::<T>();
        if raw.is_null() {
            return Err(format!("failed to allocate {table_name} table"));
        }
        let _ = with_active_cpython_context_mut(|context| {
            context.register_aux_allocation(raw.cast::<c_void>());
        });
        *allocated = true;
        return Ok(raw);
    }
    if *allocated {
        return Ok(current.cast::<T>());
    }
    let raw = unsafe { calloc(1, std::mem::size_of::<T>()) }.cast::<T>();
    if raw.is_null() {
        return Err(format!("failed to allocate {table_name} table"));
    }
    // SAFETY: source and destination are valid pointers to `T`.
    unsafe { std::ptr::copy_nonoverlapping(current.cast::<T>(), raw, 1) };
    let _ = with_active_cpython_context_mut(|context| {
        context.register_aux_allocation(raw.cast::<c_void>());
    });
    *allocated = true;
    Ok(raw)
}

fn cpython_ensure_async_methods<'a>(
    ty: &'a mut CpythonTypeObject,
    scratch: &mut CpythonTypeSlotScratch,
) -> Result<&'a mut CpythonAsyncMethods, String> {
    let raw = cpython_clone_or_alloc_slot_table::<CpythonAsyncMethods>(
        ty.tp_as_async,
        &mut scratch.async_allocated,
        "tp_as_async",
    )?;
    ty.tp_as_async = raw.cast::<c_void>();
    // SAFETY: `raw` points to a valid allocated table.
    Ok(unsafe { &mut *raw })
}

fn cpython_ensure_number_methods<'a>(
    ty: &'a mut CpythonTypeObject,
    scratch: &mut CpythonTypeSlotScratch,
) -> Result<&'a mut CpythonNumberMethods, String> {
    let raw = cpython_clone_or_alloc_slot_table::<CpythonNumberMethods>(
        ty.tp_as_number,
        &mut scratch.number_allocated,
        "tp_as_number",
    )?;
    ty.tp_as_number = raw.cast::<c_void>();
    // SAFETY: `raw` points to a valid allocated table.
    Ok(unsafe { &mut *raw })
}

fn cpython_ensure_sequence_methods<'a>(
    ty: &'a mut CpythonTypeObject,
    scratch: &mut CpythonTypeSlotScratch,
) -> Result<&'a mut CpythonSequenceMethods, String> {
    let raw = cpython_clone_or_alloc_slot_table::<CpythonSequenceMethods>(
        ty.tp_as_sequence,
        &mut scratch.sequence_allocated,
        "tp_as_sequence",
    )?;
    ty.tp_as_sequence = raw.cast::<c_void>();
    // SAFETY: `raw` points to a valid allocated table.
    Ok(unsafe { &mut *raw })
}

fn cpython_ensure_mapping_methods<'a>(
    ty: &'a mut CpythonTypeObject,
    scratch: &mut CpythonTypeSlotScratch,
) -> Result<&'a mut CpythonMappingMethods, String> {
    let raw = cpython_clone_or_alloc_slot_table::<CpythonMappingMethods>(
        ty.tp_as_mapping,
        &mut scratch.mapping_allocated,
        "tp_as_mapping",
    )?;
    ty.tp_as_mapping = raw.cast::<c_void>();
    // SAFETY: `raw` points to a valid allocated table.
    Ok(unsafe { &mut *raw })
}

fn cpython_ensure_buffer_procs<'a>(
    ty: &'a mut CpythonTypeObject,
    scratch: &mut CpythonTypeSlotScratch,
) -> Result<&'a mut CpythonBufferProcs, String> {
    let raw = cpython_clone_or_alloc_slot_table::<CpythonBufferProcs>(
        ty.tp_as_buffer,
        &mut scratch.buffer_allocated,
        "tp_as_buffer",
    )?;
    ty.tp_as_buffer = raw.cast::<c_void>();
    // SAFETY: `raw` points to a valid allocated table.
    Ok(unsafe { &mut *raw })
}

fn cpython_apply_type_slot(
    ty: &mut CpythonTypeObject,
    slot: c_int,
    pfunc: *mut c_void,
    token: &mut usize,
    scratch: &mut CpythonTypeSlotScratch,
) -> Result<(), String> {
    match slot {
        PY_TYPE_SLOT_BF_GETBUFFER => {
            let table = cpython_ensure_buffer_procs(ty, scratch)?;
            table.bf_getbuffer = if pfunc.is_null() {
                None
            } else {
                Some(unsafe { std::mem::transmute(pfunc) })
            };
        }
        PY_TYPE_SLOT_BF_RELEASEBUFFER => {
            let table = cpython_ensure_buffer_procs(ty, scratch)?;
            table.bf_releasebuffer = if pfunc.is_null() {
                None
            } else {
                Some(unsafe { std::mem::transmute(pfunc) })
            };
        }
        PY_TYPE_SLOT_MP_ASS_SUBSCRIPT => {
            cpython_ensure_mapping_methods(ty, scratch)?.mp_ass_subscript = pfunc;
        }
        PY_TYPE_SLOT_MP_LENGTH => {
            cpython_ensure_mapping_methods(ty, scratch)?.mp_length = pfunc;
        }
        PY_TYPE_SLOT_MP_SUBSCRIPT => {
            cpython_ensure_mapping_methods(ty, scratch)?.mp_subscript = pfunc;
        }
        PY_TYPE_SLOT_NB_ABSOLUTE => cpython_ensure_number_methods(ty, scratch)?.nb_absolute = pfunc,
        PY_TYPE_SLOT_NB_ADD => cpython_ensure_number_methods(ty, scratch)?.nb_add = pfunc,
        PY_TYPE_SLOT_NB_AND => cpython_ensure_number_methods(ty, scratch)?.nb_and = pfunc,
        PY_TYPE_SLOT_NB_BOOL => cpython_ensure_number_methods(ty, scratch)?.nb_bool = pfunc,
        PY_TYPE_SLOT_NB_DIVMOD => cpython_ensure_number_methods(ty, scratch)?.nb_divmod = pfunc,
        PY_TYPE_SLOT_NB_FLOAT => cpython_ensure_number_methods(ty, scratch)?.nb_float = pfunc,
        PY_TYPE_SLOT_NB_FLOOR_DIVIDE => {
            cpython_ensure_number_methods(ty, scratch)?.nb_floor_divide = pfunc;
        }
        PY_TYPE_SLOT_NB_INDEX => {
            cpython_ensure_number_methods(ty, scratch)?.nb_index = if pfunc.is_null() {
                None
            } else {
                Some(unsafe { std::mem::transmute(pfunc) })
            };
        }
        PY_TYPE_SLOT_NB_INPLACE_ADD => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_add = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_AND => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_and = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_FLOOR_DIVIDE => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_floor_divide = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_LSHIFT => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_lshift = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_MATRIX_MULTIPLY => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_matrix_multiply = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_MULTIPLY => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_multiply = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_OR => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_or = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_POWER => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_power = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_REMAINDER => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_remainder = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_RSHIFT => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_rshift = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_SUBTRACT => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_subtract = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_TRUE_DIVIDE => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_true_divide = pfunc;
        }
        PY_TYPE_SLOT_NB_INPLACE_XOR => {
            cpython_ensure_number_methods(ty, scratch)?.nb_inplace_xor = pfunc;
        }
        PY_TYPE_SLOT_NB_INT => {
            cpython_ensure_number_methods(ty, scratch)?.nb_int = if pfunc.is_null() {
                None
            } else {
                Some(unsafe { std::mem::transmute(pfunc) })
            };
        }
        PY_TYPE_SLOT_NB_INVERT => cpython_ensure_number_methods(ty, scratch)?.nb_invert = pfunc,
        PY_TYPE_SLOT_NB_LSHIFT => cpython_ensure_number_methods(ty, scratch)?.nb_lshift = pfunc,
        PY_TYPE_SLOT_NB_MATRIX_MULTIPLY => {
            cpython_ensure_number_methods(ty, scratch)?.nb_matrix_multiply = pfunc;
        }
        PY_TYPE_SLOT_NB_MULTIPLY => {
            cpython_ensure_number_methods(ty, scratch)?.nb_multiply = pfunc;
        }
        PY_TYPE_SLOT_NB_NEGATIVE => {
            cpython_ensure_number_methods(ty, scratch)?.nb_negative = pfunc;
        }
        PY_TYPE_SLOT_NB_OR => cpython_ensure_number_methods(ty, scratch)?.nb_or = pfunc,
        PY_TYPE_SLOT_NB_POSITIVE => {
            cpython_ensure_number_methods(ty, scratch)?.nb_positive = pfunc;
        }
        PY_TYPE_SLOT_NB_POWER => cpython_ensure_number_methods(ty, scratch)?.nb_power = pfunc,
        PY_TYPE_SLOT_NB_REMAINDER => {
            cpython_ensure_number_methods(ty, scratch)?.nb_remainder = pfunc;
        }
        PY_TYPE_SLOT_NB_RSHIFT => cpython_ensure_number_methods(ty, scratch)?.nb_rshift = pfunc,
        PY_TYPE_SLOT_NB_SUBTRACT => {
            cpython_ensure_number_methods(ty, scratch)?.nb_subtract = pfunc;
        }
        PY_TYPE_SLOT_NB_TRUE_DIVIDE => {
            cpython_ensure_number_methods(ty, scratch)?.nb_true_divide = pfunc;
        }
        PY_TYPE_SLOT_NB_XOR => cpython_ensure_number_methods(ty, scratch)?.nb_xor = pfunc,
        PY_TYPE_SLOT_SQ_ASS_ITEM => {
            cpython_ensure_sequence_methods(ty, scratch)?.sq_ass_item = pfunc;
        }
        PY_TYPE_SLOT_SQ_CONCAT => cpython_ensure_sequence_methods(ty, scratch)?.sq_concat = pfunc,
        PY_TYPE_SLOT_SQ_CONTAINS => {
            cpython_ensure_sequence_methods(ty, scratch)?.sq_contains = pfunc;
        }
        PY_TYPE_SLOT_SQ_INPLACE_CONCAT => {
            cpython_ensure_sequence_methods(ty, scratch)?.sq_inplace_concat = pfunc;
        }
        PY_TYPE_SLOT_SQ_INPLACE_REPEAT => {
            cpython_ensure_sequence_methods(ty, scratch)?.sq_inplace_repeat = pfunc;
        }
        PY_TYPE_SLOT_SQ_ITEM => cpython_ensure_sequence_methods(ty, scratch)?.sq_item = pfunc,
        PY_TYPE_SLOT_SQ_LENGTH => cpython_ensure_sequence_methods(ty, scratch)?.sq_length = pfunc,
        PY_TYPE_SLOT_SQ_REPEAT => cpython_ensure_sequence_methods(ty, scratch)?.sq_repeat = pfunc,
        PY_TYPE_SLOT_TP_ALLOC => ty.tp_alloc = pfunc,
        PY_TYPE_SLOT_TP_BASE => ty.tp_base = pfunc.cast::<CpythonTypeObject>(),
        PY_TYPE_SLOT_TP_BASES => ty.tp_bases = pfunc,
        PY_TYPE_SLOT_TP_CALL => ty.tp_call = pfunc,
        PY_TYPE_SLOT_TP_CLEAR => ty.tp_clear = pfunc,
        PY_TYPE_SLOT_TP_DEALLOC => ty.tp_dealloc = pfunc,
        PY_TYPE_SLOT_TP_DEL => ty.tp_del = pfunc,
        PY_TYPE_SLOT_TP_DESCR_GET => ty.tp_descr_get = pfunc,
        PY_TYPE_SLOT_TP_DESCR_SET => ty.tp_descr_set = pfunc,
        PY_TYPE_SLOT_TP_DOC => ty.tp_doc = pfunc.cast::<c_char>(),
        PY_TYPE_SLOT_TP_GETATTR => ty.tp_getattr = pfunc,
        PY_TYPE_SLOT_TP_GETATTRO => ty.tp_getattro = pfunc,
        PY_TYPE_SLOT_TP_HASH => ty.tp_hash = pfunc,
        PY_TYPE_SLOT_TP_INIT => ty.tp_init = pfunc,
        PY_TYPE_SLOT_TP_IS_GC => ty.tp_is_gc = pfunc,
        PY_TYPE_SLOT_TP_ITER => ty.tp_iter = pfunc,
        PY_TYPE_SLOT_TP_ITERNEXT => ty.tp_iternext = pfunc,
        PY_TYPE_SLOT_TP_METHODS => ty.tp_methods = pfunc,
        PY_TYPE_SLOT_TP_NEW => ty.tp_new = pfunc,
        PY_TYPE_SLOT_TP_REPR => ty.tp_repr = pfunc,
        PY_TYPE_SLOT_TP_RICHCOMPARE => ty.tp_richcompare = pfunc,
        PY_TYPE_SLOT_TP_SETATTR => ty.tp_setattr = pfunc,
        PY_TYPE_SLOT_TP_SETATTRO => ty.tp_setattro = pfunc,
        PY_TYPE_SLOT_TP_STR => ty.tp_str = pfunc,
        PY_TYPE_SLOT_TP_TRAVERSE => ty.tp_traverse = pfunc,
        PY_TYPE_SLOT_TP_MEMBERS => ty.tp_members = pfunc,
        PY_TYPE_SLOT_TP_GETSET => ty.tp_getset = pfunc,
        PY_TYPE_SLOT_TP_FREE => ty.tp_free = pfunc,
        PY_TYPE_SLOT_AM_AWAIT => cpython_ensure_async_methods(ty, scratch)?.am_await = pfunc,
        PY_TYPE_SLOT_AM_AITER => cpython_ensure_async_methods(ty, scratch)?.am_aiter = pfunc,
        PY_TYPE_SLOT_AM_ANEXT => cpython_ensure_async_methods(ty, scratch)?.am_anext = pfunc,
        PY_TYPE_SLOT_TP_FINALIZE => ty.tp_finalize = pfunc,
        PY_TYPE_SLOT_AM_SEND => cpython_ensure_async_methods(ty, scratch)?.am_send = pfunc,
        PY_TYPE_SLOT_TP_VECTORCALL => ty.tp_vectorcall = pfunc,
        PY_TYPE_SLOT_TP_TOKEN => *token = pfunc as usize,
        _ => return Err(format!("invalid type slot id {slot}")),
    }
    Ok(())
}

fn cpython_type_slot_from_type_object(
    type_ptr: *mut CpythonTypeObject,
    slot: c_int,
) -> *mut c_void {
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller provides a valid type pointer.
    let ty = unsafe { &*type_ptr };
    let async_methods = ty.tp_as_async.cast::<CpythonAsyncMethods>();
    let number_methods = ty.tp_as_number.cast::<CpythonNumberMethods>();
    let sequence_methods = ty.tp_as_sequence.cast::<CpythonSequenceMethods>();
    let mapping_methods = ty.tp_as_mapping.cast::<CpythonMappingMethods>();
    let buffer_methods = ty.tp_as_buffer.cast::<CpythonBufferProcs>();
    match slot {
        PY_TYPE_SLOT_BF_GETBUFFER => {
            if buffer_methods.is_null() {
                std::ptr::null_mut()
            } else {
                // SAFETY: non-null buffer table pointer.
                unsafe {
                    (*buffer_methods)
                        .bf_getbuffer
                        .map(|func| func as *mut c_void)
                        .unwrap_or(std::ptr::null_mut())
                }
            }
        }
        PY_TYPE_SLOT_BF_RELEASEBUFFER => {
            if buffer_methods.is_null() {
                std::ptr::null_mut()
            } else {
                // SAFETY: non-null buffer table pointer.
                unsafe {
                    (*buffer_methods)
                        .bf_releasebuffer
                        .map(|func| func as *mut c_void)
                        .unwrap_or(std::ptr::null_mut())
                }
            }
        }
        PY_TYPE_SLOT_MP_ASS_SUBSCRIPT => {
            if mapping_methods.is_null() {
                std::ptr::null_mut()
            } else {
                // SAFETY: non-null mapping table pointer.
                unsafe { (*mapping_methods).mp_ass_subscript }
            }
        }
        PY_TYPE_SLOT_MP_LENGTH => {
            if mapping_methods.is_null() {
                std::ptr::null_mut()
            } else {
                // SAFETY: non-null mapping table pointer.
                unsafe { (*mapping_methods).mp_length }
            }
        }
        PY_TYPE_SLOT_MP_SUBSCRIPT => {
            if mapping_methods.is_null() {
                std::ptr::null_mut()
            } else {
                // SAFETY: non-null mapping table pointer.
                unsafe { (*mapping_methods).mp_subscript }
            }
        }
        PY_TYPE_SLOT_NB_ABSOLUTE => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_absolute }
            }
        }
        PY_TYPE_SLOT_NB_ADD => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_add }
            }
        }
        PY_TYPE_SLOT_NB_AND => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_and }
            }
        }
        PY_TYPE_SLOT_NB_BOOL => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_bool }
            }
        }
        PY_TYPE_SLOT_NB_DIVMOD => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_divmod }
            }
        }
        PY_TYPE_SLOT_NB_FLOAT => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_float }
            }
        }
        PY_TYPE_SLOT_NB_FLOOR_DIVIDE => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_floor_divide }
            }
        }
        PY_TYPE_SLOT_NB_INDEX => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe {
                    (*number_methods)
                        .nb_index
                        .map(|func| func as *mut c_void)
                        .unwrap_or(std::ptr::null_mut())
                }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_ADD => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_add }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_AND => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_and }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_FLOOR_DIVIDE => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_floor_divide }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_LSHIFT => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_lshift }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_MATRIX_MULTIPLY => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_matrix_multiply }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_MULTIPLY => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_multiply }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_OR => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_or }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_POWER => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_power }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_REMAINDER => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_remainder }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_RSHIFT => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_rshift }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_SUBTRACT => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_subtract }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_TRUE_DIVIDE => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_true_divide }
            }
        }
        PY_TYPE_SLOT_NB_INPLACE_XOR => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_inplace_xor }
            }
        }
        PY_TYPE_SLOT_NB_INT => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe {
                    (*number_methods)
                        .nb_int
                        .map(|func| func as *mut c_void)
                        .unwrap_or(std::ptr::null_mut())
                }
            }
        }
        PY_TYPE_SLOT_NB_INVERT => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_invert }
            }
        }
        PY_TYPE_SLOT_NB_LSHIFT => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_lshift }
            }
        }
        PY_TYPE_SLOT_NB_MATRIX_MULTIPLY => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_matrix_multiply }
            }
        }
        PY_TYPE_SLOT_NB_MULTIPLY => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_multiply }
            }
        }
        PY_TYPE_SLOT_NB_NEGATIVE => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_negative }
            }
        }
        PY_TYPE_SLOT_NB_OR => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_or }
            }
        }
        PY_TYPE_SLOT_NB_POSITIVE => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_positive }
            }
        }
        PY_TYPE_SLOT_NB_POWER => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_power }
            }
        }
        PY_TYPE_SLOT_NB_REMAINDER => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_remainder }
            }
        }
        PY_TYPE_SLOT_NB_RSHIFT => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_rshift }
            }
        }
        PY_TYPE_SLOT_NB_SUBTRACT => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_subtract }
            }
        }
        PY_TYPE_SLOT_NB_TRUE_DIVIDE => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_true_divide }
            }
        }
        PY_TYPE_SLOT_NB_XOR => {
            if number_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*number_methods).nb_xor }
            }
        }
        PY_TYPE_SLOT_SQ_ASS_ITEM => {
            if sequence_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*sequence_methods).sq_ass_item }
            }
        }
        PY_TYPE_SLOT_SQ_CONCAT => {
            if sequence_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*sequence_methods).sq_concat }
            }
        }
        PY_TYPE_SLOT_SQ_CONTAINS => {
            if sequence_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*sequence_methods).sq_contains }
            }
        }
        PY_TYPE_SLOT_SQ_INPLACE_CONCAT => {
            if sequence_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*sequence_methods).sq_inplace_concat }
            }
        }
        PY_TYPE_SLOT_SQ_INPLACE_REPEAT => {
            if sequence_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*sequence_methods).sq_inplace_repeat }
            }
        }
        PY_TYPE_SLOT_SQ_ITEM => {
            if sequence_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*sequence_methods).sq_item }
            }
        }
        PY_TYPE_SLOT_SQ_LENGTH => {
            if sequence_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*sequence_methods).sq_length }
            }
        }
        PY_TYPE_SLOT_SQ_REPEAT => {
            if sequence_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*sequence_methods).sq_repeat }
            }
        }
        PY_TYPE_SLOT_TP_ALLOC => ty.tp_alloc,
        PY_TYPE_SLOT_TP_BASE => ty.tp_base.cast::<c_void>(),
        PY_TYPE_SLOT_TP_BASES => ty.tp_bases,
        PY_TYPE_SLOT_TP_CALL => ty.tp_call,
        PY_TYPE_SLOT_TP_CLEAR => ty.tp_clear,
        PY_TYPE_SLOT_TP_DEALLOC => ty.tp_dealloc,
        PY_TYPE_SLOT_TP_DEL => ty.tp_del,
        PY_TYPE_SLOT_TP_DESCR_GET => ty.tp_descr_get,
        PY_TYPE_SLOT_TP_DESCR_SET => ty.tp_descr_set,
        PY_TYPE_SLOT_TP_DOC => ty.tp_doc as *mut c_void,
        PY_TYPE_SLOT_TP_GETATTR => ty.tp_getattr,
        PY_TYPE_SLOT_TP_GETATTRO => ty.tp_getattro,
        PY_TYPE_SLOT_TP_HASH => ty.tp_hash,
        PY_TYPE_SLOT_TP_INIT => ty.tp_init,
        PY_TYPE_SLOT_TP_IS_GC => ty.tp_is_gc,
        PY_TYPE_SLOT_TP_ITER => ty.tp_iter,
        PY_TYPE_SLOT_TP_ITERNEXT => ty.tp_iternext,
        PY_TYPE_SLOT_TP_METHODS => ty.tp_methods,
        PY_TYPE_SLOT_TP_NEW => ty.tp_new,
        PY_TYPE_SLOT_TP_REPR => ty.tp_repr,
        PY_TYPE_SLOT_TP_RICHCOMPARE => ty.tp_richcompare,
        PY_TYPE_SLOT_TP_SETATTR => ty.tp_setattr,
        PY_TYPE_SLOT_TP_SETATTRO => ty.tp_setattro,
        PY_TYPE_SLOT_TP_STR => ty.tp_str,
        PY_TYPE_SLOT_TP_TRAVERSE => ty.tp_traverse,
        PY_TYPE_SLOT_TP_MEMBERS => ty.tp_members,
        PY_TYPE_SLOT_TP_GETSET => ty.tp_getset,
        PY_TYPE_SLOT_TP_FREE => ty.tp_free,
        PY_TYPE_SLOT_AM_AWAIT => {
            if async_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*async_methods).am_await }
            }
        }
        PY_TYPE_SLOT_AM_AITER => {
            if async_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*async_methods).am_aiter }
            }
        }
        PY_TYPE_SLOT_AM_ANEXT => {
            if async_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*async_methods).am_anext }
            }
        }
        PY_TYPE_SLOT_TP_FINALIZE => ty.tp_finalize,
        PY_TYPE_SLOT_AM_SEND => {
            if async_methods.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { (*async_methods).am_send }
            }
        }
        PY_TYPE_SLOT_TP_VECTORCALL => ty.tp_vectorcall,
        _ => std::ptr::null_mut(),
    }
}

fn cpython_module_name_and_def_for_type_creation(
    module_ptr: *mut c_void,
    fallback_module_name: String,
) -> Result<(String, usize), String> {
    if module_ptr.is_null() {
        return Ok((fallback_module_name, 0));
    }
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr_or_proxy(module_ptr) else {
            return Err("invalid module object".to_string());
        };
        let Value::Module(module_obj) = value else {
            return Err("module argument must be a module object".to_string());
        };
        let module_name = match &*module_obj.kind() {
            Object::Module(module_data) => module_data.name.clone(),
            _ => fallback_module_name,
        };
        let mut module_def_ptr = 0usize;
        if !context.vm.is_null() {
            // SAFETY: VM pointer is valid for active context lifetime.
            let vm = unsafe { &mut *context.vm };
            module_def_ptr = vm
                .extension_module_def_registry
                .get(&module_obj.id())
                .copied()
                .unwrap_or(0);
        }
        Ok((module_name, module_def_ptr))
    })
    .unwrap_or_else(|err| Err(err.to_string()))
}

fn cpython_type_from_spec_impl(
    metaclass: *mut c_void,
    module: *mut c_void,
    spec: *mut c_void,
    bases: *mut c_void,
) -> *mut c_void {
    if spec.is_null() {
        cpython_set_typed_error(unsafe { PyExc_SystemError }, "type spec cannot be NULL");
        return std::ptr::null_mut();
    }
    // SAFETY: pointer is validated non-null above.
    let spec = unsafe { &*(spec.cast::<CpythonTypeSpec>()) };
    if spec.name.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "type spec name cannot be NULL",
        );
        return std::ptr::null_mut();
    }
    let full_name = match unsafe { c_name_to_string(spec.name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(format!("invalid type spec name: {err}"));
            return std::ptr::null_mut();
        }
    };
    let metaclass_ptr = if metaclass.is_null() {
        std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>()
    } else if cpython_is_type_object_ptr(metaclass) {
        metaclass
    } else {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "metaclass must be a type object",
        );
        return std::ptr::null_mut();
    };
    let base = match cpython_resolve_type_base_from_arg(bases) {
        Ok(base) => base,
        Err(err) => {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, err);
            return std::ptr::null_mut();
        }
    };
    if std::env::var_os("PYRS_TRACE_CPY_TYPE_BUILD").is_some()
        && (full_name.contains("cython_function_or_method")
            || full_name.contains("_common_types_metatype"))
    {
        // SAFETY: base is a resolved type pointer.
        let base_name = unsafe { c_name_to_string((*base).tp_name) }
            .unwrap_or_else(|_| "<invalid>".to_string());
        eprintln!(
            "[cpy-type-build] from-spec name={} metaclass={:p} bases_arg={:p} resolved_base={:p} base_name={}",
            full_name, metaclass_ptr, bases, base, base_name
        );
    }
    let (default_module_name, qualname) = cpython_split_type_name(&full_name);
    let (module_name, module_def_ptr) =
        match cpython_module_name_and_def_for_type_creation(module, default_module_name) {
            Ok(parts) => parts,
            Err(err) => {
                cpython_set_typed_error(unsafe { PyExc_TypeError }, err);
                return std::ptr::null_mut();
            }
        };
    let owned_name = match CString::new(full_name.clone()) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(format!("invalid type spec name: {err}"));
            return std::ptr::null_mut();
        }
    };
    // SAFETY: base pointer is validated and can be copied by value as a template.
    let mut type_value = unsafe { std::ptr::read(base) };
    if std::env::var_os("PYRS_TRACE_EXT_SLOTS").is_some() {
        // SAFETY: `base` is validated type pointer for type creation.
        let base_name = unsafe { c_name_to_string((*base).tp_name) }
            .unwrap_or_else(|_| "<unnamed>".to_string());
        // SAFETY: `base` is validated type pointer for type creation.
        let base_size = unsafe { (*base).tp_basicsize };
        eprintln!(
            "[cpy-type-spec] name={} spec_basicsize={} spec_itemsize={} base={} base_basicsize={}",
            full_name, spec.basicsize, spec.itemsize, base_name, base_size
        );
    }
    type_value.ob_refcnt = 1;
    type_value.ob_type = metaclass_ptr;
    type_value.ob_size = 0;
    type_value.tp_name = owned_name.as_ptr();
    let mut type_data_offset = 0isize;
    if spec.basicsize > 0 {
        type_value.tp_basicsize = spec.basicsize as isize;
    } else if spec.basicsize < 0 {
        // CPython allows negative basicsize for "extends base" layouts used with
        // Py_RELATIVE_OFFSET member definitions.
        // Align with pointer width so member offsets stay ABI-compatible.
        let base_size = unsafe { (*base).tp_basicsize.max(0) as usize };
        let extension_size = (-spec.basicsize) as usize;
        let alignment = align_of::<usize>();
        let aligned_base = align_up(base_size, alignment);
        let aligned_extension = align_up(extension_size, alignment);
        type_data_offset = aligned_base as isize;
        type_value.tp_basicsize = (aligned_base + aligned_extension) as isize;
    }
    if spec.itemsize >= 0 {
        type_value.tp_itemsize = spec.itemsize as isize;
    }
    type_value.tp_base = base;
    type_value.tp_dict = std::ptr::null_mut();
    type_value.tp_flags |= PY_TPFLAGS_HEAPTYPE | PY_TPFLAGS_BASETYPE;
    type_value.tp_flags |= spec.flags as usize;
    type_value.tp_flags &= !PY_TPFLAGS_READY;

    let mut token = 0usize;
    let mut slot_scratch = CpythonTypeSlotScratch::default();
    let mut slot_map: HashMap<c_int, usize> = HashMap::new();
    if !spec.slots.is_null() {
        let mut slot = spec.slots;
        let mut guard = 0usize;
        while guard < 8192 {
            // SAFETY: slots is a contiguous slot table terminated by slot==0.
            let slot_entry = unsafe { &*slot };
            if slot_entry.slot == 0 {
                break;
            }
            let slot_pfunc =
                if slot_entry.slot == PY_TYPE_SLOT_TP_TOKEN && slot_entry.pfunc.is_null() {
                    spec as *const CpythonTypeSpec as *mut c_void
                } else {
                    slot_entry.pfunc
                };
            slot_map.insert(slot_entry.slot, slot_pfunc as usize);
            if let Err(err) = cpython_apply_type_slot(
                &mut type_value,
                slot_entry.slot,
                slot_pfunc,
                &mut token,
                &mut slot_scratch,
            ) {
                cpython_set_typed_error(unsafe { PyExc_SystemError }, err);
                return std::ptr::null_mut();
            }
            // SAFETY: move to next entry in contiguous table.
            slot = unsafe { slot.add(1) };
            guard += 1;
        }
        if guard == 8192 {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                "type slot table is not terminated",
            );
            return std::ptr::null_mut();
        }
    }

    if !type_value.tp_members.is_null() {
        let mut member = type_value.tp_members.cast::<CpythonMemberDef>();
        let mut guard = 0usize;
        while guard < 8192 {
            // SAFETY: tp_members is expected to point at a NULL-terminated PyMemberDef array.
            let member_def = unsafe { &*member };
            if member_def.name.is_null() {
                break;
            }
            let member_name = unsafe { c_name_to_string(member_def.name) }
                .unwrap_or_else(|_| "<invalid>".to_string());
            let mut resolved_offset = member_def.offset;
            if (member_def.flags & PY_MEMBER_RELATIVE_OFFSET) != 0 {
                if spec.basicsize >= 0 {
                    cpython_set_typed_error(
                        unsafe { PyExc_SystemError },
                        "With Py_RELATIVE_OFFSET, basicsize must be negative.",
                    );
                    return std::ptr::null_mut();
                }
                resolved_offset += type_data_offset;
            }
            match member_name.as_str() {
                SPECIAL_MEMBER_VECTORCALL_OFFSET => {
                    type_value.tp_vectorcall_offset = resolved_offset
                }
                SPECIAL_MEMBER_DICT_OFFSET => type_value.tp_dictoffset = resolved_offset,
                SPECIAL_MEMBER_WEAKLIST_OFFSET => type_value.tp_weaklistoffset = resolved_offset,
                _ => {}
            }
            // SAFETY: move to next member definition in contiguous table.
            member = unsafe { member.add(1) };
            guard += 1;
        }
        if guard == 8192 {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                "type members table is not terminated",
            );
            return std::ptr::null_mut();
        }
    }

    let type_data_size = (type_value.tp_basicsize - unsafe { (*base).tp_basicsize }).max(0);
    let heap_type_size = unsafe { PyType_Type.tp_basicsize.max(0) as usize }
        .max(std::mem::size_of::<CpythonTypeObject>());
    // SAFETY: allocate heap-type storage large enough for CPython heap-type layout expectations.
    let type_ptr = unsafe { calloc(1, heap_type_size) }.cast::<CpythonTypeObject>();
    if type_ptr.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_MemoryError },
            "failed to allocate heap type object",
        );
        return std::ptr::null_mut();
    }
    // SAFETY: destination points to writable heap block with room for CpythonTypeObject header.
    unsafe {
        std::ptr::write(type_ptr, type_value);
    }
    // Populate CPython heap-type extension fields consumed by inline/internal helpers
    // (for example _PyType_GetModuleState and token-based lookups in extension code).
    // SAFETY: heap_type_size is based on PyType_Type.tp_basicsize, which is initialized to
    // CPython-compatible PyHeapTypeObject size in this runtime.
    unsafe {
        let heap_type = type_ptr.cast::<CpythonHeapTypeObject>();
        (*heap_type).ht_module = if module.is_null() {
            std::ptr::null_mut()
        } else {
            Py_IncRef(module);
            module
        };
        (*heap_type).ht_token = if token == 0 {
            std::ptr::null_mut()
        } else {
            token as *mut c_void
        };
    }
    if unsafe { PyType_Ready(type_ptr.cast::<c_void>()) } != 0 {
        // SAFETY: type pointer allocated above and not published on failure.
        unsafe {
            free(type_ptr.cast());
        }
        return std::ptr::null_mut();
    }
    let _ = with_active_cpython_context_mut(|context| {
        context.register_owned_type_ptr(type_ptr.cast::<c_void>());
    });
    let type_key = type_ptr as usize;
    match cpython_heap_type_registry().lock() {
        Ok(mut registry) => {
            registry.insert(
                type_key,
                CpythonHeapTypeInfo {
                    _owned_name: owned_name,
                    qualname,
                    module_name,
                    module_ptr: module as usize,
                    module_def_ptr,
                    token,
                    type_data_size,
                    slots: slot_map,
                },
            );
        }
        Err(_) => {
            std::mem::forget(owned_name);
        }
    }
    type_ptr.cast::<c_void>()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromMetaclass(
    metaclass: *mut c_void,
    module: *mut c_void,
    spec: *mut c_void,
    bases: *mut c_void,
) -> *mut c_void {
    cpython_type_from_spec_impl(metaclass, module, spec, bases)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromModuleAndSpec(
    module: *mut c_void,
    spec: *mut c_void,
    bases: *mut c_void,
) -> *mut c_void {
    cpython_type_from_spec_impl(std::ptr::null_mut(), module, spec, bases)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromSpecWithBases(
    spec: *mut c_void,
    bases: *mut c_void,
) -> *mut c_void {
    cpython_type_from_spec_impl(std::ptr::null_mut(), std::ptr::null_mut(), spec, bases)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromSpec(spec: *mut c_void) -> *mut c_void {
    cpython_type_from_spec_impl(
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        spec,
        std::ptr::null_mut(),
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetName(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    cpython_new_ptr_for_value(Value::Str(cpython_type_name_from_tp_name(
        ty.cast::<CpythonTypeObject>(),
    )))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetQualName(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_key = ty as usize;
    let qualname = cpython_heap_type_registry()
        .lock()
        .ok()
        .and_then(|registry| registry.get(&type_key).map(|info| info.qualname.clone()))
        .unwrap_or_else(|| cpython_type_qualname_from_tp_name(ty.cast::<CpythonTypeObject>()));
    cpython_new_ptr_for_value(Value::Str(qualname))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModuleName(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_key = ty as usize;
    let module_name = cpython_heap_type_registry()
        .lock()
        .ok()
        .and_then(|registry| registry.get(&type_key).map(|info| info.module_name.clone()))
        .unwrap_or_else(|| cpython_type_module_name_from_tp_name(ty.cast::<CpythonTypeObject>()));
    cpython_new_ptr_for_value(Value::Str(module_name))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFullyQualifiedName(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_ptr = ty.cast::<CpythonTypeObject>();
    if !cpython_type_is_heap_type(type_ptr) {
        let full = unsafe { c_name_to_string((*type_ptr).tp_name) }
            .unwrap_or_else(|_| "<unnamed>".to_string());
        return cpython_new_ptr_for_value(Value::Str(full));
    }
    let type_key = ty as usize;
    let (module_name, qualname) = cpython_heap_type_registry()
        .lock()
        .ok()
        .and_then(|registry| {
            registry.get(&type_key).map(|info| {
                let module = info.module_name.clone();
                let qual = info.qualname.clone();
                (module, qual)
            })
        })
        .unwrap_or_else(|| {
            (
                cpython_type_module_name_from_tp_name(ty.cast::<CpythonTypeObject>()),
                cpython_type_qualname_from_tp_name(ty.cast::<CpythonTypeObject>()),
            )
        });
    let full_name =
        if module_name.is_empty() || module_name == "builtins" || module_name == "__main__" {
            qualname
        } else {
            format!("{module_name}.{qualname}")
        };
    cpython_new_ptr_for_value(Value::Str(full_name))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetSlot(ty: *mut c_void, slot: c_int) -> *mut c_void {
    let trace_slots = std::env::var_os("PYRS_TRACE_TYPE_SLOT_GET").is_some();
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if !(1..=PY_TYPE_SLOT_MAX).contains(&slot) {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_ptr = ty.cast::<CpythonTypeObject>();
    let type_key = type_ptr as usize;
    let result = if let Ok(registry) = cpython_heap_type_registry().lock()
        && let Some(info) = registry.get(&type_key)
    {
        if slot == PY_TYPE_SLOT_TP_TOKEN {
            info.token as *mut c_void
        } else if let Some(raw) = info.slots.get(&slot)
            && *raw != 0
        {
            *raw as *mut c_void
        } else {
            // Mirror CPython: inherited/default slots are discoverable even when
            // they were not explicitly provided in the original PyType_Spec.
            cpython_type_slot_from_type_object(type_ptr, slot)
        }
    } else if slot == PY_TYPE_SLOT_TP_TOKEN {
        std::ptr::null_mut()
    } else {
        cpython_type_slot_from_type_object(type_ptr, slot)
    };
    if trace_slots && result.is_null() {
        let type_name = unsafe { c_name_to_string((*type_ptr).tp_name) }
            .unwrap_or_else(|_| "<unnamed>".to_string());
        eprintln!(
            "[type-slot-get] null type={:p} name={} slot={}",
            ty, type_name, slot
        );
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyType_Lookup(ty: *mut c_void, name: *mut c_void) -> *mut c_void {
    if ty.is_null() || name.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        fn descriptor_or_attr_ptr_for_runtime_value(
            context: &mut ModuleCapiContext,
            owner_type: *mut CpythonTypeObject,
            value: Value,
        ) -> *mut c_void {
            if let Value::Builtin(builtin) = &value {
                let method_def = context.ensure_builtin_method_def(*builtin);
                if !method_def.is_null() {
                    // SAFETY: owner type + method def pointers are C-ABI metadata for descriptors.
                    let descriptor = unsafe {
                        PyDescr_NewMethod(owner_type.cast::<c_void>(), method_def.cast::<c_void>())
                    };
                    if !descriptor.is_null() {
                        return descriptor;
                    }
                    if std::env::var_os("PYRS_TRACE_TYPE_LOOKUP_EXIT").is_some() {
                        let error = context
                            .last_error
                            .clone()
                            .unwrap_or_else(|| "<none>".to_string());
                        eprintln!(
                            "[type-lookup] descriptor-create-failed owner={:p} method_def={:p} builtin={:?} error={}",
                            owner_type,
                            method_def,
                            builtin,
                            error
                        );
                    }
                }
            }
            if let Some(raw_ptr) = ModuleCapiContext::cpython_proxy_raw_ptr_from_value(&value) {
                return raw_ptr;
            }
            context.alloc_cpython_ptr_for_value(value)
        }

        fn lookup_runtime_mro_with_attr_name(
            context: &mut ModuleCapiContext,
            ty: *mut c_void,
            attr_name: &str,
        ) -> *mut c_void {
            let mut current = ty.cast::<CpythonTypeObject>();
            for _ in 0..64 {
                if current.is_null() {
                    break;
                }
                if let Some(Value::Class(class_obj)) =
                    context.cpython_value_from_ptr_or_proxy(current.cast::<c_void>())
                    && let Object::Class(class_data) = &*class_obj.kind()
                    && let Some(attr_value) = class_data.attrs.get(attr_name).cloned()
                {
                    return descriptor_or_attr_ptr_for_runtime_value(context, current, attr_value);
                }
                // SAFETY: `current` points to a type object; `tp_base` is read-only metadata.
                let next = unsafe { (*current).tp_base };
                if next.is_null() || next == current {
                    break;
                }
                current = next;
            }
            std::ptr::null_mut()
        }

        fn lookup_tp_dict_chain_with_name_object(
            ty: *mut c_void,
            name: *mut c_void,
        ) -> *mut c_void {
            let mut current = ty.cast::<CpythonTypeObject>();
            for _ in 0..64 {
                if current.is_null() {
                    break;
                }
                // SAFETY: `current` is a candidate type object pointer; dictionary lookup helpers
                // perform their own validation and suppress lookup misses.
                let dict_ptr = unsafe { (*current).tp_dict };
                if !dict_ptr.is_null() {
                    // SAFETY: dictionary/key pointers are forwarded to CPython-compatible API.
                    let value = unsafe { PyDict_GetItemWithError(dict_ptr, name) };
                    if !value.is_null() {
                        return value;
                    }
                    // _PyType_Lookup must not leak incidental lookup errors.
                    if unsafe { !PyErr_Occurred().is_null() } {
                        unsafe { PyErr_Clear() };
                    }
                }
                // SAFETY: `tp_base` is read-only metadata for candidate type objects.
                let next = unsafe { (*current).tp_base };
                if next.is_null() || next == current {
                    break;
                }
                current = next;
            }
            std::ptr::null_mut()
        }

        fn lookup_runtime_mro_with_name_object(
            context: &mut ModuleCapiContext,
            ty: *mut c_void,
            name: *mut c_void,
        ) -> *mut c_void {
            let mut current = ty.cast::<CpythonTypeObject>();
            for _ in 0..64 {
                if current.is_null() {
                    break;
                }
                if let Some(Value::Class(class_obj)) =
                    context.cpython_value_from_ptr_or_proxy(current.cast::<c_void>())
                    && let Object::Class(class_data) = &*class_obj.kind()
                {
                    for (attr_key, attr_value) in &class_data.attrs {
                        let key_ptr = context.alloc_cpython_ptr_for_value(Value::Str(attr_key.clone()));
                        if key_ptr.is_null() {
                            continue;
                        }
                        // SAFETY: both pointers are live for the active context.
                        let matches = unsafe { PyObject_RichCompareBool(name, key_ptr, 2) };
                        if matches == 1 {
                            return descriptor_or_attr_ptr_for_runtime_value(
                                context,
                                current,
                                attr_value.clone(),
                            );
                        }
                        if matches < 0 {
                            // _PyType_Lookup must not leak comparison errors.
                            unsafe { PyErr_Clear() };
                        }
                    }
                }
                // SAFETY: `current` points to a type object; `tp_base` is read-only metadata.
                let next = unsafe { (*current).tp_base };
                if next.is_null() || next == current {
                    break;
                }
                current = next;
            }
            std::ptr::null_mut()
        }

        let name_value = context.cpython_value_from_ptr_or_proxy(name);
        let trace_exit_lookup = std::env::var_os("PYRS_TRACE_TYPE_LOOKUP_EXIT").is_some();
        if trace_exit_lookup {
            let type_name = unsafe {
                let type_ptr = ty.cast::<CpythonTypeObject>();
                if type_ptr.is_null() {
                    "<unnamed>".to_string()
                } else {
                    c_name_to_string((*type_ptr).tp_name).unwrap_or_else(|_| "<unnamed>".to_string())
                }
            };
            let type_tag = context
                .cpython_value_from_ptr_or_proxy(ty)
                .map(|value| cpython_value_debug_tag(&value))
                .unwrap_or_else(|| "<unmapped>".to_string());
            let name_desc = match &name_value {
                Some(Value::Str(text)) => text.clone(),
                Some(other) => format!("<{}>", cpython_value_debug_tag(other)),
                None => format!("<unmapped:{name:p}>"),
            };
            if name_desc == "__exit__" || name_desc.starts_with("<unmapped") {
                let (tp_dict, tp_base, tp_getattro, tp_flags, tp_basicsize) = unsafe {
                    let type_ptr = ty.cast::<CpythonTypeObject>();
                    if type_ptr.is_null() {
                        (
                            std::ptr::null_mut(),
                            std::ptr::null_mut::<CpythonTypeObject>(),
                            std::ptr::null_mut(),
                            0usize,
                            0isize,
                        )
                    } else {
                        (
                            (*type_ptr).tp_dict,
                            (*type_ptr).tp_base,
                            (*type_ptr).tp_getattro,
                            (*type_ptr).tp_flags,
                            (*type_ptr).tp_basicsize,
                        )
                    }
                };
                let (registry_alive, registry_live_or_pending, registry_freed) =
                    if context.vm.is_null() {
                        (false, false, false)
                    } else {
                        // SAFETY: VM pointer is valid for the active C-API context.
                        let vm = unsafe { &mut *context.vm };
                        (
                            vm.capi_registry_contains_alive(ty as usize),
                            vm.capi_registry_contains_live_or_pending(ty as usize),
                            vm.capi_registry_is_freed(ty as usize),
                        )
                    };
                eprintln!(
                    "[type-lookup] enter type={:p} name_ptr={:p} type_name={} type_tag={} name={} tp_dict={:p} tp_base={:p} tp_getattro={:p} tp_flags=0x{:x} tp_basicsize={} registry_alive={} registry_live_or_pending={} registry_freed={}",
                    ty,
                    name,
                    type_name,
                    type_tag,
                    name_desc,
                    tp_dict,
                    tp_base,
                    tp_getattro,
                    tp_flags,
                    tp_basicsize,
                    registry_alive,
                    registry_live_or_pending,
                    registry_freed
                );
            }
        }
        // _PyType_Lookup() must not leave a new error behind when lookup misses.
        let saved_current_error = context.current_error;
        let saved_last_error = context.last_error.clone();
        let saved_first_error = context.first_error.clone();
        let dict_chain_result = lookup_tp_dict_chain_with_name_object(ty, name);
        let result = if !dict_chain_result.is_null() {
            dict_chain_result
        } else {
            match &name_value {
            Some(Value::Str(attr_name)) => context
                .lookup_type_attr_via_tp_dict(ty, &attr_name)
                .or_else(|| {
                    let runtime_attr = lookup_runtime_mro_with_attr_name(context, ty, attr_name);
                    (!runtime_attr.is_null()).then_some(runtime_attr)
                })
                .unwrap_or(std::ptr::null_mut()),
            Some(_) => {
                unsafe { PyErr_BadInternalCall() };
                std::ptr::null_mut()
            }
            None => lookup_runtime_mro_with_name_object(context, ty, name),
            }
        };
        context.current_error = saved_current_error;
        context.last_error = saved_last_error;
        context.first_error = saved_first_error;
        if trace_exit_lookup {
            let name_desc = match &name_value {
                Some(Value::Str(text)) => text.clone(),
                Some(other) => format!("<{}>", cpython_value_debug_tag(other)),
                None => format!("<unmapped:{name:p}>"),
            };
            if name_desc == "__exit__" || name_desc.starts_with("<unmapped") {
                let (result_type_name, result_descr_get, result_tag) = if result.is_null() {
                    ("<null>".to_string(), std::ptr::null_mut(), "<none>".to_string())
                } else {
                    unsafe {
                        let result_type = result
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                            .unwrap_or(std::ptr::null_mut());
                        let type_name = if result_type.is_null() {
                            "<unknown>".to_string()
                        } else {
                            c_name_to_string((*result_type).tp_name)
                                .unwrap_or_else(|_| "<invalid>".to_string())
                        };
                        let descr_get = if result_type.is_null() {
                            std::ptr::null_mut()
                        } else {
                            (*result_type).tp_descr_get
                        };
                        let tag = context
                            .cpython_value_from_ptr_or_proxy(result)
                            .map(|value| cpython_value_debug_tag(&value))
                            .unwrap_or_else(|| "<unmapped>".to_string());
                        (type_name, descr_get, tag)
                    }
                };
                eprintln!(
                    "[type-lookup] result name={} result={:p} result_type={} result_descr_get={:p} result_tag={}",
                    name_desc, result, result_type_name, result_descr_get, result_tag
                );
            }
        }
        result
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModule(ty: *mut c_void) -> *mut c_void {
    let trace = std::env::var_os("PYRS_TRACE_TYPE_MODULE").is_some();
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_ptr = ty.cast::<CpythonTypeObject>();
    let type_key = type_ptr as usize;
    if trace {
        let type_name = unsafe { c_name_to_string((*type_ptr).tp_name) }
            .unwrap_or_else(|_| "<unnamed>".to_string());
        eprintln!("[cpy-type-module] enter ty={:p} type={}", ty, type_name);
    }
    let Some((module_ptr, type_name)) =
        cpython_heap_type_registry()
            .lock()
            .ok()
            .and_then(|registry| {
                registry
                    .get(&type_key)
                    .map(|info| (info.module_ptr, cpython_type_name_from_tp_name(type_ptr)))
            })
    else {
        let type_name = unsafe { c_name_to_string((*type_ptr).tp_name) }
            .unwrap_or_else(|_| "<unnamed>".to_string());
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            format!("PyType_GetModule: Type '{type_name}' is not a heap type"),
        );
        if trace {
            eprintln!("[cpy-type-module] miss ty={:p} (not heap type)", ty);
        }
        return std::ptr::null_mut();
    };
    if module_ptr == 0 {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            format!(
                "PyType_GetModule: Type '{}' has no associated module",
                type_name
            ),
        );
        if trace {
            eprintln!("[cpy-type-module] no-module ty={:p} type={}", ty, type_name);
        }
        return std::ptr::null_mut();
    }
    if trace {
        eprintln!(
            "[cpy-type-module] hit ty={:p} type={} module_ptr={:p}",
            ty, type_name, module_ptr as *mut c_void
        );
    }
    module_ptr as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModuleState(ty: *mut c_void) -> *mut c_void {
    let module = unsafe { PyType_GetModule(ty) };
    if module.is_null() {
        if std::env::var_os("PYRS_TRACE_TYPE_MODULE").is_some() {
            eprintln!("[cpy-type-module] state miss ty={:p} module=<null>", ty);
        }
        return std::ptr::null_mut();
    }
    let state = unsafe { PyModule_GetState(module) };
    if std::env::var_os("PYRS_TRACE_TYPE_MODULE").is_some() {
        eprintln!(
            "[cpy-type-module] state hit ty={:p} module={:p} state={:p}",
            ty, module, state
        );
    }
    state
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModuleByDef(
    ty: *mut c_void,
    module_def: *mut c_void,
) -> *mut c_void {
    if ty.is_null() || module_def.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if !cpython_is_type_object_ptr(ty) {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "expected type object");
        return std::ptr::null_mut();
    }
    let requested_module_def = module_def as usize;
    let trace_moddef = std::env::var_os("PYRS_TRACE_CPY_TYPE_MODDEF").is_some();
    let trace_module_ptr = |label: &str, module_ptr: *mut c_void| {
        if !trace_moddef || module_ptr.is_null() {
            return;
        }
        // SAFETY: diagnostics only; pointer may not actually be module layout, so guard output.
        let (md_def, md_state) = unsafe {
            let module = module_ptr.cast::<CpythonModuleCompatObject>();
            (
                module
                    .as_ref()
                    .map(|raw| raw.md_def)
                    .unwrap_or(std::ptr::null_mut()),
                module
                    .as_ref()
                    .map(|raw| raw.md_state)
                    .unwrap_or(std::ptr::null_mut()),
            )
        };
        eprintln!(
            "[cpy-type-moddef] {} module_ptr={:p} md_def={:p} md_state={:p}",
            label, module_ptr, md_def, md_state
        );
    };
    if trace_moddef {
        let type_name = unsafe { c_name_to_string((*ty.cast::<CpythonTypeObject>()).tp_name) }
            .unwrap_or_else(|_| "<unnamed>".to_string());
        eprintln!(
            "[cpy-type-moddef] enter type={} ty={:p} requested_def={:p}",
            type_name, ty, module_def
        );
    }
    if let Ok(Some(active_module_ptr)) = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return None;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(bound_def) = vm
            .extension_module_def_registry
            .get(&context.module.id())
            .copied()
        else {
            return None;
        };
        if bound_def != requested_module_def {
            return None;
        }
        Some(context.alloc_cpython_ptr_for_value(Value::Module(context.module.clone())))
    }) && !active_module_ptr.is_null()
    {
        if trace_moddef {
            eprintln!(
                "[cpy-type-moddef] resolve source=active_context module_ptr={:p}",
                active_module_ptr
            );
        }
        trace_module_ptr("active_context_state", active_module_ptr);
        return active_module_ptr;
    }
    let mut current = ty.cast::<CpythonTypeObject>();
    while !current.is_null() {
        let key = current as usize;
        if let Ok(registry) = cpython_heap_type_registry().lock()
            && let Some(info) = registry.get(&key)
        {
            if info.module_ptr != 0
                && info.module_def_ptr != 0
                && info.module_def_ptr == requested_module_def
            {
                if trace_moddef {
                    eprintln!(
                        "[cpy-type-moddef] resolve source=heap_registry_direct current={:p} module_ptr={:p} module_def={:p}",
                        current, info.module_ptr as *mut c_void, info.module_def_ptr as *mut c_void
                    );
                }
                trace_module_ptr("heap_registry_direct_state", info.module_ptr as *mut c_void);
                return info.module_ptr as *mut c_void;
            }
            if info.module_ptr != 0 {
                let module_ptr = info.module_ptr as *mut c_void;
                let resolved_module_def = with_active_cpython_context_mut(|context| {
                    if context.vm.is_null() {
                        return None;
                    }
                    let module_value = context.cpython_value_from_ptr_or_proxy(module_ptr)?;
                    let Value::Module(module_obj) = module_value else {
                        return None;
                    };
                    // SAFETY: VM pointer is valid for active C-API context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    vm.extension_module_def_registry
                        .get(&module_obj.id())
                        .copied()
                })
                .ok()
                .flatten();
                if resolved_module_def == Some(requested_module_def) {
                    if trace_moddef {
                        eprintln!(
                            "[cpy-type-moddef] resolve source=heap_registry_resolve current={:p} module_ptr={:p} resolved_def={:p}",
                            current, module_ptr, requested_module_def as *mut c_void
                        );
                    }
                    trace_module_ptr("heap_registry_resolve_state", module_ptr);
                    return module_ptr;
                }
            }
        }
        // SAFETY: current pointer is valid in MRO walk.
        current = unsafe { (*current).tp_base };
    }
    if let Ok(Some(any_bound_module_ptr)) = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return None;
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let module_id =
            vm.extension_module_def_registry
                .iter()
                .find_map(|(module_id, def_ptr)| {
                    (*def_ptr == requested_module_def).then_some(*module_id)
                })?;
        let module_obj = vm
            .modules
            .values()
            .find(|module| module.id() == module_id)
            .cloned()?;
        Some(context.alloc_cpython_ptr_for_value(Value::Module(module_obj)))
    }) && !any_bound_module_ptr.is_null()
    {
        if trace_moddef {
            eprintln!(
                "[cpy-type-moddef] resolve source=vm_registry_scan module_ptr={:p}",
                any_bound_module_ptr
            );
        }
        trace_module_ptr("vm_registry_scan_state", any_bound_module_ptr);
        return any_bound_module_ptr;
    }
    // SAFETY: `ty` is non-null and expected to be a type object.
    let type_name = unsafe { c_name_to_string((*ty.cast::<CpythonTypeObject>()).tp_name) }
        .unwrap_or_else(|_| "<unnamed>".to_string());
    cpython_set_typed_error(
        unsafe { PyExc_TypeError },
        format!("PyType_GetModuleByDef: No superclass of '{type_name}' has the given module"),
    );
    if trace_moddef {
        eprintln!(
            "[cpy-type-moddef] resolve source=none type={} requested_def={:p}",
            type_name, module_def
        );
    }
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetTypeDataSize(ty: *mut c_void) -> isize {
    if ty.is_null() {
        return 0;
    }
    let key = ty as usize;
    if let Ok(registry) = cpython_heap_type_registry().lock()
        && let Some(info) = registry.get(&key)
    {
        return info.type_data_size.max(0);
    }
    // SAFETY: caller provided type pointer.
    let ty = ty.cast::<CpythonTypeObject>();
    // SAFETY: pointer validated non-null above.
    let base = unsafe { (*ty).tp_base };
    if base.is_null() {
        return 0;
    }
    // SAFETY: type and base pointers are valid for read.
    let delta = unsafe { (*ty).tp_basicsize - (*base).tp_basicsize };
    delta.max(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetBaseByToken(
    ty: *mut c_void,
    token: *mut c_void,
    result: *mut *mut c_void,
) -> c_int {
    let trace_token = std::env::var_os("PYRS_TRACE_TYPE_TOKEN").is_some();
    if !result.is_null() {
        // SAFETY: output pointer is caller-provided and writable.
        unsafe { *result = std::ptr::null_mut() };
    }
    if token.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyType_GetBaseByToken called with token=NULL",
        );
        return -1;
    }
    if ty.is_null() || !cpython_is_type_object_ptr(ty) {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "expected a type object");
        return -1;
    }
    let token_key = token as usize;
    let mut current = ty.cast::<CpythonTypeObject>();
    if trace_token {
        let type_name = unsafe { c_name_to_string((*current).tp_name) }
            .unwrap_or_else(|_| "<unnamed>".to_string());
        eprintln!(
            "[cpy-type-token] enter ty={:p} type={} token={:p}",
            ty, type_name, token
        );
    }
    while !current.is_null() {
        let key = current as usize;
        if let Ok(registry) = cpython_heap_type_registry().lock()
            && let Some(info) = registry.get(&key)
            && info.token == token_key
        {
            if !result.is_null() {
                // SAFETY: output pointer is caller-provided and writable.
                unsafe { *result = current.cast::<c_void>() };
                // SAFETY: result pointer holds a type object pointer that must be returned as new reference.
                unsafe { Py_IncRef(*result) };
            }
            if trace_token {
                let type_name = unsafe { c_name_to_string((*current).tp_name) }
                    .unwrap_or_else(|_| "<unnamed>".to_string());
                eprintln!(
                    "[cpy-type-token] match current={:p} type={} token={:p}",
                    current, type_name, token
                );
            }
            return 1;
        }
        if trace_token {
            let type_name = unsafe { c_name_to_string((*current).tp_name) }
                .unwrap_or_else(|_| "<unnamed>".to_string());
            let token_info = cpython_heap_type_registry()
                .lock()
                .ok()
                .and_then(|registry| registry.get(&(current as usize)).map(|info| info.token))
                .unwrap_or(0);
            eprintln!(
                "[cpy-type-token] miss current={:p} type={} token={:p} entry_token={:p}",
                current, type_name, token, token_info as *mut c_void
            );
        }
        // SAFETY: current pointer is valid in MRO walk.
        current = unsafe { (*current).tp_base };
    }
    if trace_token {
        eprintln!("[cpy-type-token] no-match ty={:p} token={:p}", ty, token);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_ClearCache() -> c_uint {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return 0;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        vm.clear_all_type_caches()
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_Modified(ty: *mut c_void) {
    if ty.is_null() {
        return;
    }
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        let Some(type_value) = context.cpython_value_from_ptr_or_proxy(ty) else {
            // Unknown type pointer: conservatively invalidate all type caches.
            // SAFETY: VM pointer is valid for active context lifetime.
            let vm = unsafe { &mut *context.vm };
            let _ = vm.clear_all_type_caches();
            return;
        };
        let Value::Class(class_obj) = type_value else {
            // Non-runtime class mapping: fall back to global invalidation.
            // SAFETY: VM pointer is valid for active context lifetime.
            let vm = unsafe { &mut *context.vm };
            let _ = vm.clear_all_type_caches();
            return;
        };
        context.populate_proxy_class_attrs_from_type_dict(&class_obj, ty);
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let _ = vm.invalidate_type_cache_for_class_id(class_obj.id());
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_Freeze(ty: *mut c_void) -> c_int {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if !cpython_is_type_object_ptr(ty) {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "expected type object");
        return -1;
    }
    // SAFETY: pointer is validated above.
    unsafe {
        (*ty.cast::<CpythonTypeObject>()).tp_flags |= PY_TPFLAGS_IMMUTABLETYPE;
    }
    unsafe { PyType_Modified(ty) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFlags(ty: *mut c_void) -> usize {
    if ty.is_null() {
        return 0;
    }
    // SAFETY: caller provided a type pointer.
    unsafe { ty.cast::<CpythonTypeObject>().as_ref() }
        .map(|ty| ty.tp_flags)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_IsSubtype(subtype: *mut c_void, ty: *mut c_void) -> i32 {
    let trace = std::env::var_os("PYRS_TRACE_TYPE_SUBTYPE").is_some();
    let trace_iseed = std::env::var_os("PYRS_TRACE_ISEED_SUBTYPE").is_some();
    let trace_tzinfo = std::env::var_os("PYRS_TRACE_TZINFO_SUBTYPE").is_some();
    let type_name_for = |ptr: *mut c_void| -> String {
        if ptr.is_null() {
            return "<null>".to_string();
        }
        // SAFETY: best-effort tracing only.
        unsafe {
            ptr.cast::<CpythonTypeObject>()
                .as_ref()
                .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                .unwrap_or_else(|| "<unknown>".to_string())
        }
    };
    if trace_iseed {
        let subtype_name = type_name_for(subtype);
        let target_name = type_name_for(ty);
        if subtype_name.contains("NoneType")
            || target_name.contains("ISeedSequence")
            || target_name.contains("SeedSequence")
        {
            eprintln!(
                "[iseed-subtype] call subtype={:p}({}) target={:p}({})",
                subtype, subtype_name, ty, target_name
            );
        }
    }
    if trace_tzinfo {
        let subtype_name = type_name_for(subtype);
        let target_name = type_name_for(ty);
        if subtype_name.contains("tz")
            || target_name.contains("tz")
            || subtype_name.contains("datetime.")
            || target_name.contains("datetime.")
        {
            eprintln!(
                "[tz-subtype] call subtype={:p}({}) target={:p}({})",
                subtype, subtype_name, ty, target_name
            );
        }
    }
    if subtype.is_null() || ty.is_null() {
        if trace {
            eprintln!(
                "[type-subtype] early-null subtype={:p} ty={:p}",
                subtype, ty
            );
        }
        return 0;
    }
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    const TYPE_ALIGN: usize = std::mem::align_of::<CpythonTypeObject>();
    if (subtype as usize) < MIN_VALID_PTR || (ty as usize) < MIN_VALID_PTR {
        if trace {
            eprintln!("[type-subtype] below-min subtype={:p} ty={:p}", subtype, ty);
        }
        return 0;
    }
    if (subtype as usize) % TYPE_ALIGN != 0 || (ty as usize) % TYPE_ALIGN != 0 {
        if trace {
            eprintln!("[type-subtype] unaligned subtype={:p} ty={:p}", subtype, ty);
        }
        return 0;
    }
    if trace {
        eprintln!(
            "[type-subtype] start subtype={:p}({}) target={:p}({})",
            subtype,
            type_name_for(subtype),
            ty,
            type_name_for(ty)
        );
    }
    let target = ty.cast::<CpythonTypeObject>();
    let mut current = subtype.cast::<CpythonTypeObject>();
    let mut guard = 0usize;
    while !current.is_null() {
        if guard > 8192 {
            if trace {
                eprintln!(
                    "[type-subtype] guard-hit subtype={:p} target={:p}",
                    subtype, ty
                );
            }
            return 0;
        }
        if (current as usize) < MIN_VALID_PTR || (current as usize) % TYPE_ALIGN != 0 {
            if trace {
                eprintln!(
                    "[type-subtype] invalid-current current={:p} guard={}",
                    current, guard
                );
                if std::env::var_os("PYRS_TRACE_TYPE_SUBTYPE_BT").is_some() {
                    eprintln!(
                        "[type-subtype] invalid-current bt={}",
                        std::backtrace::Backtrace::force_capture()
                    );
                }
            }
            return 0;
        }
        if current == target {
            if trace {
                eprintln!(
                    "[type-subtype] match current={:p}({}) guard={}",
                    current.cast::<c_void>(),
                    type_name_for(current.cast()),
                    guard
                );
            }
            if trace_tzinfo {
                eprintln!(
                    "[tz-subtype] match current={:p}({}) target={:p}({}) guard={}",
                    current.cast::<c_void>(),
                    type_name_for(current.cast()),
                    ty,
                    type_name_for(ty),
                    guard
                );
            }
            return 1;
        }
        // SAFETY: current is checked non-null.
        let next = unsafe { (*current).tp_base };
        let _ = with_active_cpython_context_mut(|context| {
            context.register_known_type_ptr(current.cast::<c_void>());
        });
        if next == current {
            break;
        }
        if next.is_null() {
            break;
        }
        if (next as usize) < MIN_VALID_PTR || (next as usize) % TYPE_ALIGN != 0 {
            if trace {
                eprintln!(
                    "[type-subtype] invalid-next next={:p} current={:p} guard={}",
                    next, current, guard
                );
                if std::env::var_os("PYRS_TRACE_TYPE_SUBTYPE_BT").is_some() {
                    eprintln!(
                        "[type-subtype] invalid-next bt={}",
                        std::backtrace::Backtrace::force_capture()
                    );
                }
            }
            return 0;
        }
        if next == target {
            if trace {
                eprintln!(
                    "[type-subtype] match-next next={:p}({}) guard={}",
                    next.cast::<c_void>(),
                    type_name_for(next.cast()),
                    guard
                );
            }
            if trace_tzinfo {
                eprintln!(
                    "[tz-subtype] match-next next={:p}({}) target={:p}({}) guard={}",
                    next.cast::<c_void>(),
                    type_name_for(next.cast()),
                    ty,
                    type_name_for(ty),
                    guard
                );
            }
            return 1;
        }
        current = next;
        guard += 1;
    }
    if trace {
        eprintln!(
            "[type-subtype] no-match subtype={:p}({}) target={:p}({})",
            subtype,
            type_name_for(subtype),
            ty,
            type_name_for(ty)
        );
    }
    if trace_tzinfo {
        let subtype_name = type_name_for(subtype);
        let target_name = type_name_for(ty);
        if subtype_name.contains("tz")
            || target_name.contains("tz")
            || subtype_name.contains("datetime.")
            || target_name.contains("datetime.")
        {
            eprintln!(
                "[tz-subtype] no-match subtype={:p}({}) target={:p}({})",
                subtype, subtype_name, ty, target_name
            );
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_Ready(ty: *mut c_void) -> i32 {
    if ty.is_null() {
        cpython_set_error("PyType_Ready received null type");
        return -1;
    }
    let ty = ty.cast::<CpythonTypeObject>();
    let _ = with_active_cpython_context_mut(|context| {
        context.register_known_type_ptr(ty.cast::<c_void>());
    });
    // SAFETY: `ty` is valid for mutation during type ready.
    unsafe {
        if (*ty).ob_type.is_null() {
            (*ty).ob_type = (&raw mut PyType_Type).cast();
        }
        if (*ty).tp_base.is_null()
            && ty != (&raw mut PyBaseObject_Type)
            && ty != (&raw mut PyType_Type)
        {
            (*ty).tp_base = &raw mut PyBaseObject_Type;
        }
        let base = (*ty).tp_base;
        if (*ty).tp_basicsize <= 0 {
            if !base.is_null() && (*base).tp_basicsize > 0 {
                (*ty).tp_basicsize = (*base).tp_basicsize;
            } else {
                (*ty).tp_basicsize = std::mem::size_of::<CpythonObjectHead>() as isize;
            }
        }
        if (*ty).tp_call.is_null() && !base.is_null() {
            (*ty).tp_call = (*base).tp_call;
        }
        if (*ty).tp_init.is_null() && !base.is_null() {
            (*ty).tp_init = (*base).tp_init;
        }
        if (*ty).tp_alloc.is_null() && !base.is_null() {
            (*ty).tp_alloc = (*base).tp_alloc;
        }
        if (*ty).tp_new.is_null() && !base.is_null() {
            (*ty).tp_new = (*base).tp_new;
        }
        if (*ty).tp_free.is_null() && !base.is_null() {
            (*ty).tp_free = (*base).tp_free;
        }
        if (*ty).tp_getattro.is_null() && !base.is_null() {
            (*ty).tp_getattro = (*base).tp_getattro;
        }
        if (*ty).tp_setattro.is_null() && !base.is_null() {
            (*ty).tp_setattro = (*base).tp_setattro;
        }
        if (*ty).tp_repr.is_null() && !base.is_null() {
            (*ty).tp_repr = (*base).tp_repr;
        }
        if (*ty).tp_str.is_null() && !base.is_null() {
            (*ty).tp_str = (*base).tp_str;
        }
        if (*ty).tp_basicsize <= 0 {
            (*ty).tp_basicsize = std::mem::size_of::<CpythonObjectHead>() as isize;
        }
        if (*ty).tp_alloc.is_null() {
            (*ty).tp_alloc = PyType_GenericAlloc as *mut c_void;
        }
        if (*ty).tp_free.is_null() {
            (*ty).tp_free = PyObject_Free as *mut c_void;
        }
        if (*ty).tp_new.is_null() {
            (*ty).tp_new = PyType_GenericNew as *mut c_void;
        }
        if (*ty).tp_dict.is_null() {
            let dict_ptr = PyDict_New();
            if dict_ptr.is_null() {
                return -1;
            }
            (*ty).tp_dict = dict_ptr;
        }
        if (*ty).tp_bases.is_null() {
            let base_obj = if !base.is_null() {
                base.cast::<c_void>()
            } else {
                std::ptr::null_mut()
            };
            let bases_len = usize::from(!base_obj.is_null());
            let bases_tuple = PyTuple_New(bases_len as isize);
            if bases_tuple.is_null() {
                return -1;
            }
            if !base_obj.is_null() {
                Py_XIncRef(base_obj);
                if PyTuple_SetItem(bases_tuple, 0, base_obj) != 0 {
                    Py_DecRef(bases_tuple);
                    return -1;
                }
            }
            (*ty).tp_bases = bases_tuple;
        }
        if (*ty).tp_mro.is_null() {
            let type_obj = ty.cast::<c_void>();
            let mut base_entries: Vec<*mut c_void> = Vec::new();
            if !base.is_null() {
                let base_mro = (*base).tp_mro;
                if !base_mro.is_null() {
                    let base_mro_len = PyTuple_Size(base_mro).max(0) as usize;
                    if base_mro_len > 0 {
                        for index in 0..base_mro_len {
                            let entry = PyTuple_GetItem(base_mro, index as isize);
                            if !entry.is_null() {
                                base_entries.push(entry);
                            }
                        }
                    }
                }
                if base_entries.is_empty() {
                    base_entries.push(base.cast::<c_void>());
                }
            }
            let mro_tuple = PyTuple_New((1 + base_entries.len()) as isize);
            if mro_tuple.is_null() {
                return -1;
            }
            Py_XIncRef(type_obj);
            if PyTuple_SetItem(mro_tuple, 0, type_obj) != 0 {
                Py_DecRef(mro_tuple);
                return -1;
            }
            for (index, entry) in base_entries.into_iter().enumerate() {
                Py_XIncRef(entry);
                if PyTuple_SetItem(mro_tuple, (index + 1) as isize, entry) != 0 {
                    Py_DecRef(mro_tuple);
                    return -1;
                }
            }
            (*ty).tp_mro = mro_tuple;
        }
        if cpython_type_populate_method_descriptors(ty) != 0 {
            return -1;
        }
        if cpython_type_install_init_slot_wrapper(ty) != 0 {
            return -1;
        }
        if !base.is_null() {
            let inherited_subclass_bits = (*base).tp_flags
                & (PY_TPFLAGS_LONG_SUBCLASS
                    | PY_TPFLAGS_LIST_SUBCLASS
                    | PY_TPFLAGS_TUPLE_SUBCLASS
                    | PY_TPFLAGS_BYTES_SUBCLASS
                    | PY_TPFLAGS_UNICODE_SUBCLASS
                    | PY_TPFLAGS_DICT_SUBCLASS
                    | PY_TPFLAGS_TYPE_SUBCLASS);
            (*ty).tp_flags |= inherited_subclass_bits;
        }
        (*ty).tp_flags |= PY_TPFLAGS_READY;
    }
    let _ = with_active_cpython_context_mut(|context| {
        context.register_known_type_ptr(ty.cast::<c_void>());
    });
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GenericAlloc(subtype: *mut c_void, nitems: isize) -> *mut c_void {
    if subtype.is_null() {
        cpython_set_error("PyType_GenericAlloc received null subtype");
        return std::ptr::null_mut();
    }
    let ty = subtype.cast::<CpythonTypeObject>();
    let trace_type_alloc = std::env::var_os("PYRS_TRACE_TYPE_ALLOC").is_some();
    let (tp_name, tp_basicsize, tp_itemsize) = unsafe {
        (
            c_name_to_string((*ty).tp_name).unwrap_or_else(|_| "<unnamed>".to_string()),
            (*ty).tp_basicsize,
            (*ty).tp_itemsize,
        )
    };
    if trace_type_alloc {
        eprintln!(
            "[type-alloc] subtype={:p} name={} basicsize={} itemsize={} nitems={}",
            subtype, tp_name, tp_basicsize, tp_itemsize, nitems
        );
    }
    // SAFETY: subtype is checked non-null.
    let itemsize = tp_itemsize;
    if itemsize > 0 || nitems > 0 {
        unsafe { _PyObject_NewVar(ty, nitems.max(0)) }
    } else {
        unsafe { _PyObject_New(ty) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GenericNew(
    subtype: *mut c_void,
    _args: *mut c_void,
    _kwargs: *mut c_void,
) -> *mut c_void {
    if subtype.is_null() {
        cpython_set_error("PyType_GenericNew received null subtype");
        return std::ptr::null_mut();
    }
    let ty = subtype.cast::<CpythonTypeObject>();
    // SAFETY: subtype is checked non-null.
    let alloc = unsafe { (*ty).tp_alloc };
    if alloc.is_null() {
        return unsafe { PyType_GenericAlloc(subtype, 0) };
    }
    let alloc_fn: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
        // SAFETY: tp_alloc slot follows CPython allocfunc signature.
        unsafe { std::mem::transmute(alloc) };
    unsafe { alloc_fn(subtype, 0) }
}
