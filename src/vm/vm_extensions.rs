use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_long, c_ulong, c_void};
use std::path::{Path, PathBuf};
use std::sync::Once;

use crate::extensions::{
    CpythonExtensionInit, ExtensionEntrypoint, PYRS_CAPI_ABI_VERSION, PYRS_DYNAMIC_INIT_SYMBOL_V1,
    PYRS_EXTENSION_ABI_TAG, PYRS_EXTENSION_MANIFEST_SUFFIX, PYRS_TYPE_BOOL, PYRS_TYPE_BYTES,
    PYRS_TYPE_DICT, PYRS_TYPE_FLOAT, PYRS_TYPE_INT, PYRS_TYPE_LIST, PYRS_TYPE_NONE, PYRS_TYPE_STR,
    PYRS_TYPE_TUPLE, PyrsApiV1, PyrsBufferInfoV1, PyrsBufferInfoV2, PyrsBufferViewV1,
    PyrsCFunctionKwV1, PyrsCFunctionV1, PyrsCapsuleDestructorV1, PyrsModuleStateFinalizeV1,
    PyrsModuleStateFreeV1, PyrsObjectHandle, PyrsWritableBufferViewV1, SharedLibraryHandle,
    load_dynamic_initializer, load_dynamic_symbol, parse_extension_manifest,
    path_is_shared_library,
};
use crate::runtime::{
    BigInt, BoundMethod, BuiltinFunction, ClassObject, NativeMethodKind, NativeMethodObject,
    Object, RuntimeError, Value,
};

use super::{
    ExtensionCallableKind, GeneratorResumeOutcome, InternalCallOutcome, NativeCallResult, ObjRef,
    Vm, add_values, and_values, dict_contains_key_checked, dict_get_value, dict_remove_value,
    dict_set_value_checked, div_values, floor_div_values, invert_value, is_truthy, lshift_values,
    memoryview_bounds, mod_values, mul_values, neg_value, or_values, pos_value, pow_values,
    rshift_values, sub_values, value_to_int, xor_values,
};

struct CapiObjectSlot {
    value: Value,
    refcount: usize,
}

struct CapiCapsuleSlot {
    pointer: usize,
    context: usize,
    name: Option<CString>,
    destructor: Option<PyrsCapsuleDestructorV1>,
    exported_name: Option<String>,
    refcount: usize,
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
struct CpythonCapsuleCompatObject {
    ob_base: CpythonObjectHead,
    pointer: *mut c_void,
    name: *const c_char,
    context: *mut c_void,
    destructor: Option<PyrsCapsuleDestructorV1>,
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
pub struct CpythonComplexValue {
    real: f64,
    imag: f64,
}

const PYRS_DATETIME_CAPSULE_NAME: &str = "datetime.datetime_CAPI";

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

unsafe fn cpython_external_capsule_pointer(
    capsule: *mut c_void,
    requested_name: *const c_char,
) -> Result<Option<*mut c_void>, String> {
    if capsule.is_null() {
        return Ok(None);
    }
    // SAFETY: caller provides an object pointer from extension code; we only inspect the
    // standard object header first and bail out if it's not a capsule.
    let raw = capsule.cast::<CpythonCapsuleCompatObject>();
    let ty = unsafe { (*raw).ob_base.ob_type };
    if ty != std::ptr::addr_of_mut!(PyCapsule_Type).cast() {
        return Ok(None);
    }
    if !requested_name.is_null() {
        // SAFETY: `requested_name` is the C-API input argument.
        let requested = unsafe { CStr::from_ptr(requested_name) };
        // SAFETY: capsule name pointer is part of the external capsule object.
        let actual_ptr = unsafe { (*raw).name };
        if actual_ptr.is_null() {
            return Err("capsule name mismatch".to_string());
        }
        // SAFETY: `actual_ptr` is validated non-null above.
        let actual = unsafe { CStr::from_ptr(actual_ptr) };
        if requested.to_bytes() != actual.to_bytes() {
            return Err("capsule name mismatch".to_string());
        }
    }
    // SAFETY: `raw` points to a capsule-compatible object.
    let pointer = unsafe { (*raw).pointer };
    if pointer.is_null() {
        return Err("capsule pointer is null".to_string());
    }
    Ok(Some(pointer))
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
        Value::Bytes(_) | Value::ByteArray(_) => std::ptr::addr_of_mut!(PyBytes_Type).cast(),
        Value::MemoryView(_) => std::ptr::addr_of_mut!(PyMemoryView_Type).cast(),
        Value::Slice(_) => std::ptr::addr_of_mut!(PySlice_Type).cast(),
        Value::Class(_) => std::ptr::addr_of_mut!(PyType_Type).cast(),
        Value::Builtin(_) => std::ptr::addr_of_mut!(PyCFunction_Type).cast(),
        _ => std::ptr::addr_of_mut!(PyBaseObject_Type).cast(),
    }
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
        Value::BoundMethod(_) => "BoundMethod".to_string(),
        Value::Function(_) => "Function".to_string(),
        Value::Cell(_) => "Cell".to_string(),
        Value::Exception(err) => format!("Exception({})", err.name),
        Value::ExceptionType(name) => format!("ExceptionType({name})"),
        Value::Slice(_) => "Slice".to_string(),
        Value::Code(_) => "Code".to_string(),
        Value::Builtin(builtin) => format!("Builtin({builtin:?})"),
    }
}

fn cpython_exception_value_from_ptr(raw: usize) -> Option<Value> {
    // SAFETY: exception symbol pointers are process-global and stable.
    unsafe {
        let exception_name = if raw == PyExc_Exception as usize {
            Some("Exception")
        } else if raw == PyExc_ImportError as usize {
            Some("ImportError")
        } else if raw == PyExc_RuntimeError as usize {
            Some("RuntimeError")
        } else if raw == PyExc_TypeError as usize {
            Some("TypeError")
        } else if raw == PyExc_ValueError as usize {
            Some("ValueError")
        } else if raw == PyExc_AttributeError as usize {
            Some("AttributeError")
        } else if raw == PyExc_BufferError as usize {
            Some("BufferError")
        } else if raw == PyExc_DeprecationWarning as usize {
            Some("DeprecationWarning")
        } else if raw == PyExc_FloatingPointError as usize {
            Some("FloatingPointError")
        } else if raw == PyExc_FutureWarning as usize {
            Some("FutureWarning")
        } else if raw == PyExc_IOError as usize {
            Some("IOError")
        } else if raw == PyExc_ImportWarning as usize {
            Some("ImportWarning")
        } else if raw == PyExc_IndexError as usize {
            Some("IndexError")
        } else if raw == PyExc_KeyError as usize {
            Some("KeyError")
        } else if raw == PyExc_MemoryError as usize {
            Some("MemoryError")
        } else if raw == PyExc_NameError as usize {
            Some("NameError")
        } else if raw == PyExc_NotImplementedError as usize {
            Some("NotImplementedError")
        } else if raw == PyExc_OSError as usize {
            Some("OSError")
        } else if raw == PyExc_OverflowError as usize {
            Some("OverflowError")
        } else if raw == PyExc_RecursionError as usize {
            Some("RecursionError")
        } else if raw == PyExc_RuntimeWarning as usize {
            Some("RuntimeWarning")
        } else if raw == PyExc_SystemError as usize {
            Some("SystemError")
        } else if raw == PyExc_UnicodeDecodeError as usize {
            Some("UnicodeDecodeError")
        } else if raw == PyExc_UnicodeEncodeError as usize {
            Some("UnicodeEncodeError")
        } else if raw == PyExc_UserWarning as usize {
            Some("UserWarning")
        } else {
            None
        }?;
        Some(Value::ExceptionType(exception_name.to_string()))
    }
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

fn initialize_cpython_compat_type_objects() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        let type_ptr = std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>();
        PyType_Type.ob_type = type_ptr;
        PyType_Type.tp_call = cpython_type_tp_call as *mut c_void;
        PyType_Type.tp_alloc = PyType_GenericAlloc as *mut c_void;
        PyType_Type.tp_new = PyType_GenericNew as *mut c_void;
        PyType_Type.tp_base = std::ptr::addr_of_mut!(PyBaseObject_Type);

        let type_objects: &mut [*mut CpythonTypeObject] = &mut [
            std::ptr::addr_of_mut!(PyBaseObject_Type),
            std::ptr::addr_of_mut!(PyObject_Type),
            std::ptr::addr_of_mut!(PyBool_Type),
            std::ptr::addr_of_mut!(PyBytes_Type),
            std::ptr::addr_of_mut!(PyCFunction_Type),
            std::ptr::addr_of_mut!(PyCapsule_Type),
            std::ptr::addr_of_mut!(PyComplex_Type),
            std::ptr::addr_of_mut!(PyDictProxy_Type),
            std::ptr::addr_of_mut!(PyDict_Type),
            std::ptr::addr_of_mut!(PyFloat_Type),
            std::ptr::addr_of_mut!(PyFrozenSet_Type),
            std::ptr::addr_of_mut!(PyGetSetDescr_Type),
            std::ptr::addr_of_mut!(PyList_Type),
            std::ptr::addr_of_mut!(PyLong_Type),
            std::ptr::addr_of_mut!(PyMemberDescr_Type),
            std::ptr::addr_of_mut!(PyMemoryView_Type),
            std::ptr::addr_of_mut!(PyMethodDescr_Type),
            std::ptr::addr_of_mut!(PyNone_Type),
            std::ptr::addr_of_mut!(PySet_Type),
            std::ptr::addr_of_mut!(PySlice_Type),
            std::ptr::addr_of_mut!(PyTuple_Type),
            std::ptr::addr_of_mut!(PyUnicode_Type),
        ];
        for ty in type_objects {
            (**ty).ob_type = type_ptr;
            if (**ty).tp_base.is_null() && *ty != std::ptr::addr_of_mut!(PyBaseObject_Type) {
                (**ty).tp_base = std::ptr::addr_of_mut!(PyBaseObject_Type);
            }
        }
        PyBaseObject_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_READY;
        PyObject_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_READY;
        PyType_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_TYPE_SUBCLASS | PY_TPFLAGS_READY;

        PyLong_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_LONG_SUBCLASS | PY_TPFLAGS_READY;
        PyBool_Type.tp_flags |= PY_TPFLAGS_LONG_SUBCLASS | PY_TPFLAGS_READY;
        PyList_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_LIST_SUBCLASS | PY_TPFLAGS_READY;
        PyTuple_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_TUPLE_SUBCLASS | PY_TPFLAGS_READY;
        PyBytes_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_BYTES_SUBCLASS | PY_TPFLAGS_READY;
        PyUnicode_Type.tp_flags |=
            PY_TPFLAGS_BASETYPE | PY_TPFLAGS_UNICODE_SUBCLASS | PY_TPFLAGS_READY;
        PyDict_Type.tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_DICT_SUBCLASS | PY_TPFLAGS_READY;
        PyNone_Type.tp_flags |= PY_TPFLAGS_READY;

        _Py_NoneStruct.ob_type = std::ptr::addr_of_mut!(PyNone_Type).cast();
        _Py_NotImplementedStruct.ob_type = std::ptr::addr_of_mut!(PyBaseObject_Type).cast();
        _Py_EllipsisObject.ob_type = std::ptr::addr_of_mut!(PyBaseObject_Type).cast();
        _Py_FalseStruct.ob_type = std::ptr::addr_of_mut!(PyBool_Type).cast();
        _Py_TrueStruct.ob_type = std::ptr::addr_of_mut!(PyBool_Type).cast();

        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_Exception), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_ImportError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_RuntimeError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_TypeError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_ValueError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_AttributeError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_BufferError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_DeprecationWarning), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_FloatingPointError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_FutureWarning), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_IOError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_ImportWarning), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_IndexError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_KeyError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_MemoryError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_NameError), type_ptr);
        ensure_cpython_exception_symbol(
            std::ptr::addr_of_mut!(PyExc_NotImplementedError),
            type_ptr,
        );
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_OSError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_OverflowError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_RecursionError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_RuntimeWarning), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_SystemError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_UnicodeDecodeError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_UnicodeEncodeError), type_ptr);
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_UserWarning), type_ptr);
    });
}

thread_local! {
    static ACTIVE_CPYTHON_INIT_CONTEXT: Cell<*mut ModuleCapiContext> = const { Cell::new(std::ptr::null_mut()) };
}

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double;
    fn strtol(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_long;
    fn strtoul(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_ulong;
    fn Py_BuildValue(format: *const c_char, ...) -> *mut c_void;
}

struct ModuleCapiContext {
    vm: *mut Vm,
    module: ObjRef,
    next_object_handle: PyrsObjectHandle,
    objects: HashMap<PyrsObjectHandle, CapiObjectSlot>,
    capsules: HashMap<PyrsObjectHandle, CapiCapsuleSlot>,
    last_error: Option<String>,
    first_error: Option<String>,
    scratch_strings: Vec<CString>,
    scratch_isize_arrays: Vec<Vec<isize>>,
    buffer_pins: HashMap<PyrsObjectHandle, usize>,
    cpython_objects_by_ptr: HashMap<usize, PyrsObjectHandle>,
    cpython_ptr_by_handle: HashMap<PyrsObjectHandle, *mut c_void>,
    cpython_object_handles_by_id: HashMap<u64, PyrsObjectHandle>,
    cpython_allocations: Vec<*mut CpythonCompatObject>,
    cpython_list_buffers: HashMap<PyrsObjectHandle, (*mut *mut c_void, usize)>,
    cpython_sync_in_progress: HashSet<PyrsObjectHandle>,
}

impl Drop for ModuleCapiContext {
    fn drop(&mut self) {
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
        for slot in capsules.into_values() {
            if slot.exported_name.is_some() {
                continue;
            }
            if let Some(destructor) = slot.destructor {
                // SAFETY: destructor pointer was provided by extension code.
                unsafe {
                    destructor(slot.pointer as *mut c_void, slot.context as *mut c_void);
                }
            }
        }
        for raw in self.cpython_allocations.drain(..) {
            // SAFETY: pointers were allocated via C allocator in this context.
            unsafe {
                free(raw.cast());
            }
        }
        for (buffer, _) in self.cpython_list_buffers.drain().map(|(_, value)| value) {
            if !buffer.is_null() {
                // SAFETY: list item buffers were allocated through C allocator in this context.
                unsafe {
                    free(buffer.cast());
                }
            }
        }
    }
}

impl ModuleCapiContext {
    fn new(vm: *mut Vm, module: ObjRef) -> Self {
        initialize_cpython_compat_type_objects();
        Self {
            vm,
            module,
            next_object_handle: 1,
            objects: HashMap::new(),
            capsules: HashMap::new(),
            last_error: None,
            first_error: None,
            scratch_strings: Vec::new(),
            scratch_isize_arrays: Vec::new(),
            buffer_pins: HashMap::new(),
            cpython_objects_by_ptr: HashMap::new(),
            cpython_ptr_by_handle: HashMap::new(),
            cpython_object_handles_by_id: HashMap::new(),
            cpython_allocations: Vec::new(),
            cpython_list_buffers: HashMap::new(),
            cpython_sync_in_progress: HashSet::new(),
        }
    }

    #[track_caller]
    fn set_error(&mut self, message: impl Into<String>) {
        let message = message.into();
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

    fn clear_error(&mut self) {
        self.last_error = None;
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
                slot.destructor,
            )
        });
        let (refcount, ob_type, tuple_items, list_items, bytes_payload) = self
            .objects
            .get(&handle)
            .map(|slot| {
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
                )
            })
            .unwrap_or_else(|| {
                // SAFETY: static type object addresses are process-lifetime stable.
                let fallback = std::ptr::addr_of_mut!(PyBaseObject_Type).cast();
                (1, fallback, None, None, None)
            });
        let raw = if let Some((capsule_refcount, pointer, name, context, destructor)) =
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
                    destructor,
                });
            }
            raw_capsule.cast::<CpythonCompatObject>()
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
            let raw_bytes =
                unsafe { malloc(storage_bytes) }.cast::<CpythonBytesCompatObject>();
            if raw_bytes.is_null() {
                self.set_error("out of memory allocating CPython bytes compat object");
                return std::ptr::null_mut();
            }
            // SAFETY: initialize bytes header and payload; `storage_bytes` includes trailing NUL.
            unsafe {
                (*raw_bytes).ob_base = CpythonVarObjectHead {
                    ob_base: CpythonObjectHead { ob_refcnt: refcount, ob_type },
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
        raw.cast()
    }

    fn alloc_cpython_ptr_for_value(&mut self, value: Value) -> *mut c_void {
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
        let handle = self.alloc_object(value);
        self.alloc_cpython_ptr_for_handle(handle)
    }

    fn cpython_handle_from_ptr(&mut self, object: *mut c_void) -> Option<PyrsObjectHandle> {
        self.cpython_objects_by_ptr.get(&(object as usize)).copied()
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
        if let Some(exception_value) = cpython_exception_value_from_ptr(raw) {
            return Some(exception_value);
        }
        let handle = self.cpython_handle_from_ptr(object)?;
        if raw != std::ptr::addr_of!(_Py_NoneStruct) as usize
            && let Some(slot) = self.objects.get(&handle)
            && matches!(slot.value, Value::None)
        {
            let raw_type = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type)
                    .unwrap_or(std::ptr::null_mut())
            };
            let none_type = std::ptr::addr_of_mut!(PyNone_Type).cast::<c_void>();
            if !raw_type.is_null() && raw_type != none_type {
                if std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some() {
                    eprintln!(
                        "[cpy-ptr] remap stale none ptr={:p} handle={} raw_type={:p} none_type={:p}",
                        object, handle, raw_type, none_type
                    );
                }
                self.cpython_objects_by_ptr.remove(&raw);
                if self
                    .cpython_ptr_by_handle
                    .get(&handle)
                    .is_some_and(|ptr| *ptr == object)
                {
                    self.cpython_ptr_by_handle.remove(&handle);
                }
                return None;
            }
        }
        self.sync_value_from_cpython_storage(handle, object);
        let value = self.object_value(handle);
        if std::env::var_os("PYRS_TRACE_CPY_NONE_PTRS").is_some()
            && raw != std::ptr::addr_of!(_Py_NoneStruct) as usize
            && matches!(value, Some(Value::None))
        {
            eprintln!(
                "[cpy-none-ptr] ptr={:p} handle={} (non-singleton Value::None)",
                object, handle
            );
        }
        value
    }

    fn cpython_value_from_ptr_or_proxy(&mut self, object: *mut c_void) -> Option<Value> {
        if let Some(value) = self.cpython_value_from_ptr(object) {
            return Some(value);
        }
        if object.is_null() || self.vm.is_null() {
            return None;
        }
        // SAFETY: VM pointer is valid for the C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        let proxy = match vm.heap.alloc_class(ClassObject::new(
            "__pyrs_cpython_proxy__".to_string(),
            Vec::new(),
        )) {
            Value::Class(class_obj) => {
                if let Object::Class(class_data) = &mut *class_obj.kind_mut() {
                    class_data.attrs.insert(
                        "__pyrs_cpython_proxy_ptr__".to_string(),
                        Value::Int(object as usize as i64),
                    );
                }
                Value::Class(class_obj)
            }
            other => other,
        };
        let handle = self.alloc_object(proxy.clone());
        if std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some() {
            eprintln!(
                "[cpy-ptr] proxy-map handle={} external_ptr={:p}",
                handle, object
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

    fn try_native_tp_call(
        &mut self,
        callable: *mut c_void,
        args: &[Value],
        kwargs: &HashMap<String, Value>,
    ) -> Option<*mut c_void> {
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

    fn module_obj(&self, module_handle: PyrsObjectHandle) -> Result<ObjRef, String> {
        let module = self
            .object_value(module_handle)
            .ok_or_else(|| format!("invalid module handle {}", module_handle))?;
        match module {
            Value::Module(module_obj) => Ok(module_obj),
            _ => Err(format!("object handle {} is not a module", module_handle)),
        }
    }

    fn module_set_attr(
        &mut self,
        module_handle: PyrsObjectHandle,
        attr_name: &str,
        value_handle: PyrsObjectHandle,
    ) -> Result<(), String> {
        let module_obj = self.module_obj(module_handle)?;
        let value = self
            .object_value(value_handle)
            .ok_or_else(|| format!("invalid value handle {}", value_handle))?;
        let mut module_kind = module_obj.kind_mut();
        let Object::Module(module_data) = &mut *module_kind else {
            return Err(format!(
                "object handle {} has invalid module storage",
                module_handle
            ));
        };
        module_data.globals.insert(attr_name.to_string(), value);
        Ok(())
    }

    fn module_del_attr(
        &mut self,
        module_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<(), String> {
        let module_obj = self.module_obj(module_handle)?;
        let mut module_kind = module_obj.kind_mut();
        let Object::Module(module_data) = &mut *module_kind else {
            return Err(format!(
                "object handle {} has invalid module storage",
                module_handle
            ));
        };
        if module_data.globals.remove(attr_name).is_none() {
            return Err(format!("module attribute '{}' not found", attr_name));
        }
        Ok(())
    }

    fn module_has_attr(
        &mut self,
        module_handle: PyrsObjectHandle,
        attr_name: &str,
    ) -> Result<i32, String> {
        let module_obj = self.module_obj(module_handle)?;
        let module_kind = module_obj.kind();
        let Object::Module(module_data) = &*module_kind else {
            return Err(format!(
                "object handle {} has invalid module storage",
                module_handle
            ));
        };
        Ok(if module_data.globals.contains_key(attr_name) {
            1
        } else {
            0
        })
    }

    fn module_set_state(
        &mut self,
        state: *mut c_void,
        free_func: Option<PyrsModuleStateFreeV1>,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("module_set_state missing VM context".to_string());
        }
        let module_id = self.module.id();
        // SAFETY: VM pointer is valid for the context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.prune_extension_module_state_registry();
        if state.is_null() {
            if let Some(previous) = vm.extension_module_state_registry.remove(&module_id) {
                if previous.state != 0 {
                    if let Some(previous_finalize) = previous.finalize_func {
                        // SAFETY: finalize function pointer was provided by extension code.
                        unsafe {
                            previous_finalize(previous.state as *mut c_void);
                        }
                    }
                    if let Some(previous_free) = previous.free_func {
                        // SAFETY: free function pointer was provided by extension code.
                        unsafe {
                            previous_free(previous.state as *mut c_void);
                        }
                    }
                }
                if let Some(previous_finalize) = previous.finalize_func {
                    vm.extension_module_state_registry.insert(
                        module_id,
                        super::ExtensionModuleStateEntry {
                            state: 0,
                            free_func: None,
                            finalize_func: Some(previous_finalize),
                        },
                    );
                }
            }
            return Ok(());
        }
        let finalize_func = vm
            .extension_module_state_registry
            .get(&module_id)
            .and_then(|entry| entry.finalize_func);
        let entry = super::ExtensionModuleStateEntry {
            state: state as usize,
            free_func,
            finalize_func,
        };
        let previous = vm.extension_module_state_registry.insert(module_id, entry);
        if let Some(previous) = previous {
            let replaced_state = previous.state != state as usize;
            let replaced_free =
                previous.free_func.map(|func| func as usize) != free_func.map(|func| func as usize);
            if (replaced_state || replaced_free) && previous.state != 0 {
                if let Some(previous_finalize) = previous.finalize_func {
                    // SAFETY: finalize function pointer was provided by extension code.
                    unsafe {
                        previous_finalize(previous.state as *mut c_void);
                    }
                }
                if let Some(previous_free) = previous.free_func {
                    // SAFETY: free function pointer was provided by extension code.
                    unsafe {
                        previous_free(previous.state as *mut c_void);
                    }
                }
            }
        }
        Ok(())
    }

    fn module_set_finalize(
        &mut self,
        finalize_func: Option<PyrsModuleStateFinalizeV1>,
    ) -> Result<(), String> {
        if self.vm.is_null() {
            return Err("module_set_finalize missing VM context".to_string());
        }
        let module_id = self.module.id();
        // SAFETY: VM pointer is valid for the context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.prune_extension_module_state_registry();
        if let Some(entry) = vm.extension_module_state_registry.get_mut(&module_id) {
            entry.finalize_func = finalize_func;
            if entry.state == 0 && entry.free_func.is_none() && entry.finalize_func.is_none() {
                vm.extension_module_state_registry.remove(&module_id);
            }
            return Ok(());
        }
        if let Some(finalize_func) = finalize_func {
            vm.extension_module_state_registry.insert(
                module_id,
                super::ExtensionModuleStateEntry {
                    state: 0,
                    free_func: None,
                    finalize_func: Some(finalize_func),
                },
            );
        }
        Ok(())
    }

    fn module_get_state(&mut self) -> Result<*mut c_void, String> {
        if self.vm.is_null() {
            return Err("module_get_state missing VM context".to_string());
        }
        let module_id = self.module.id();
        // SAFETY: VM pointer is valid for the context lifetime.
        let vm = unsafe { &mut *self.vm };
        vm.prune_extension_module_state_registry();
        let state = vm
            .extension_module_state_registry
            .get(&module_id)
            .map_or(std::ptr::null_mut(), |entry| entry.state as *mut c_void);
        Ok(state)
    }

    fn sync_exported_capsule(
        &mut self,
        exported_name: Option<&str>,
        pointer: usize,
        context: usize,
        destructor: Option<PyrsCapsuleDestructorV1>,
        release_previous: bool,
    ) -> Result<(), String> {
        let Some(name) = exported_name else {
            return Ok(());
        };
        if self.vm.is_null() {
            return Err("capsule export missing VM context".to_string());
        }
        // SAFETY: VM pointer is valid for the context lifetime.
        let vm = unsafe { &mut *self.vm };
        let previous = vm.extension_capsule_registry.insert(
            name.to_string(),
            super::ExtensionCapsuleRegistryEntry {
                pointer,
                context,
                destructor,
            },
        );
        if release_previous && let Some(previous) = previous {
            let replaced_pointer = previous.pointer != pointer || previous.context != context;
            let replaced_destructor = previous.destructor.map(|func| func as usize)
                != destructor.map(|func| func as usize);
            if (replaced_pointer || replaced_destructor)
                && let Some(previous_destructor) = previous.destructor
            {
                // SAFETY: destructor pointer came from a previously registered capsule.
                unsafe {
                    previous_destructor(
                        previous.pointer as *mut c_void,
                        previous.context as *mut c_void,
                    );
                }
            }
        }
        Ok(())
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
            // Keep capsule handles pinned for the full C-API context lifetime.
            // NumPy and similar extensions can hand capsule pointers through C paths where
            // ownership is intentionally opaque, so eager decref-to-zero breaks valid flows.
            if slot.refcount > 1 {
                slot.refcount -= 1;
            }
            self.sync_cpython_refcount(handle);
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
                let mut values = Vec::with_capacity(item_ptrs.len());
                for (idx, item_ptr) in item_ptrs.iter().copied().enumerate() {
                    if item_ptr.is_null() {
                        if trace_raw {
                            eprintln!(
                                "[cpy-sync-tuple] handle={} tuple_ptr={:p} idx={} item_ptr=<null> value=None",
                                handle, ptr, idx
                            );
                        }
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
                            values.push(Value::None)
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
                let mut values = Vec::with_capacity(item_ptrs.len());
                for item_ptr in item_ptrs {
                    if item_ptr.is_null() {
                        values.push(Value::None);
                        continue;
                    }
                    match self.cpython_value_from_ptr_or_proxy(item_ptr) {
                        Some(value) => values.push(value),
                        None => values.push(Value::None),
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
        if let Some(slot) = self.capsules.get(&handle) {
            // SAFETY: `raw` points to owned capsule-compatible storage for this handle.
            unsafe {
                let raw_capsule = raw.cast::<CpythonCapsuleCompatObject>();
                (*raw_capsule).ob_base.ob_refcnt = slot.refcount.max(1) as isize;
                (*raw_capsule).ob_base.ob_type = std::ptr::addr_of_mut!(PyCapsule_Type).cast();
                (*raw_capsule).pointer = slot.pointer as *mut c_void;
                (*raw_capsule).name = slot.name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
                (*raw_capsule).context = slot.context as *mut c_void;
                (*raw_capsule).destructor = slot.destructor;
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
        self.cpython_allocations
            .iter()
            .any(|owned| (*owned).cast::<c_void>() == ptr)
            || self
                .cpython_list_buffers
                .values()
                .any(|(buffer, _)| !buffer.is_null() && (*buffer).cast::<c_void>() == ptr)
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
    ) -> Result<PyrsObjectHandle, String> {
        self.alloc_capsule(pointer, name)
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
                        return class_data.name == "__pyrs_cpython_proxy__";
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
                let mut list_kind = list_obj.kind_mut();
                let Object::List(values) = &mut *list_kind else {
                    return Err(format!(
                        "object handle {} has invalid list storage",
                        object_handle
                    ));
                };
                let mut idx = value_to_int(key).map_err(|err| err.message)? as isize;
                if idx < 0 {
                    idx += values.len() as isize;
                }
                if idx < 0 || idx as usize >= values.len() {
                    return Err("index out of range".to_string());
                }
                values[idx as usize] = value;
                return Ok(());
            }
            Value::ByteArray(bytearray_obj) => {
                let mut bytes_kind = bytearray_obj.kind_mut();
                let Object::ByteArray(values) = &mut *bytes_kind else {
                    return Err(format!(
                        "object handle {} has invalid bytearray storage",
                        object_handle
                    ));
                };
                let mut idx = value_to_int(key).map_err(|err| err.message)? as isize;
                if idx < 0 {
                    idx += values.len() as isize;
                }
                if idx < 0 || idx as usize >= values.len() {
                    return Err("index out of range".to_string());
                }
                let byte = value_to_int(value).map_err(|err| err.message)?;
                if !(0..=255).contains(&byte) {
                    return Err("byte must be in range(0, 256)".to_string());
                }
                values[idx as usize] = byte as u8;
                return Ok(());
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
                let mut list_kind = list_obj.kind_mut();
                let Object::List(values) = &mut *list_kind else {
                    return Err(format!(
                        "object handle {} has invalid list storage",
                        object_handle
                    ));
                };
                let mut idx = value_to_int(key).map_err(|err| err.message)? as isize;
                if idx < 0 {
                    idx += values.len() as isize;
                }
                if idx < 0 || idx as usize >= values.len() {
                    return Err("index out of range".to_string());
                }
                values.remove(idx as usize);
                return Ok(());
            }
            Value::ByteArray(bytearray_obj) => {
                let mut bytes_kind = bytearray_obj.kind_mut();
                let Object::ByteArray(values) = &mut *bytes_kind else {
                    return Err(format!(
                        "object handle {} has invalid bytearray storage",
                        object_handle
                    ));
                };
                let mut idx = value_to_int(key).map_err(|err| err.message)? as isize;
                if idx < 0 {
                    idx += values.len() as isize;
                }
                if idx < 0 || idx as usize >= values.len() {
                    return Err("index out of range".to_string());
                }
                values.remove(idx as usize);
                return Ok(());
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

fn with_active_cpython_context_mut<R>(
    f: impl FnOnce(&mut ModuleCapiContext) -> R,
) -> Result<R, String> {
    ACTIVE_CPYTHON_INIT_CONTEXT.with(|cell| {
        let ptr = cell.get();
        if ptr.is_null() {
            return Err("no active CPython extension init context".to_string());
        }
        // SAFETY: the pointer is set only while the owning `ModuleCapiContext` is alive.
        Ok(f(unsafe { &mut *ptr }))
    })
}

fn cpython_set_active_context(context: *mut ModuleCapiContext) -> *mut ModuleCapiContext {
    ACTIVE_CPYTHON_INIT_CONTEXT.with(|cell| {
        let previous = cell.get();
        cell.set(context);
        previous
    })
}

#[track_caller]
fn cpython_set_error(message: impl Into<String>) {
    let message = message.into();
    if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
        let caller = std::panic::Location::caller();
        eprintln!(
            "[cpy-err] {} (at {}:{})",
            message,
            caller.file(),
            caller.line()
        );
    }
    let _ = with_active_cpython_context_mut(|context| {
        context.set_error(message);
    });
}

fn cpython_value_from_ptr(object: *mut c_void) -> Result<Value, String> {
    if object.is_null() {
        return Err("received null PyObject pointer".to_string());
    }
    with_active_cpython_context_mut(|context| context.cpython_value_from_ptr(object))
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "unknown PyObject pointer".to_string())
}

fn cpython_new_ptr_for_value(value: Value) -> *mut c_void {
    with_active_cpython_context_mut(|context| context.alloc_cpython_ptr_for_value(value))
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            std::ptr::null_mut()
        })
}

fn cpython_new_bytes_ptr(bytes: Vec<u8>) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("missing VM context for bytes allocation");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for the active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let bytes_obj = vm.heap.alloc(Object::Bytes(bytes));
        context.alloc_cpython_ptr_for_value(Value::Bytes(bytes_obj))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

fn cpython_call_builtin(function: BuiltinFunction, args: Vec<Value>) -> Result<Value, String> {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return Err("missing VM context for builtin call".to_string());
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_internal(Value::Builtin(function), args, HashMap::new()) {
            Ok(InternalCallOutcome::Value(value)) => Ok(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => Err(vm
                .runtime_error_from_active_exception("builtin call failed")
                .message),
            Err(err) => Err(err.message),
        }
    })?
}

fn cpython_unary_numeric_op(
    object: *mut c_void,
    op: impl FnOnce(Value) -> Result<Value, RuntimeError>,
) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match op(value) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err.message);
            std::ptr::null_mut()
        }
    }
}

fn cpython_binary_numeric_op(
    left: *mut c_void,
    right: *mut c_void,
    op: impl FnOnce(Value, Value) -> Result<Value, RuntimeError>,
) -> *mut c_void {
    let left = match cpython_value_from_ptr(left) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let right = match cpython_value_from_ptr(right) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match op(left, right) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err.message);
            std::ptr::null_mut()
        }
    }
}

fn cpython_binary_numeric_op_with_heap(
    left: *mut c_void,
    right: *mut c_void,
    op: impl FnOnce(Value, Value, &crate::runtime::Heap) -> Result<Value, RuntimeError>,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("missing VM context for numeric operation");
            return std::ptr::null_mut();
        }
        let Some(left) = context.cpython_value_from_ptr(left) else {
            context.set_error("unknown left operand pointer");
            return std::ptr::null_mut();
        };
        let Some(right) = context.cpython_value_from_ptr(right) else {
            context.set_error("unknown right operand pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match op(left, right, &vm.heap) {
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
        let Some(callable) = context.cpython_value_from_ptr(callable_ptr) else {
            context.set_error("unknown callable object pointer");
            return std::ptr::null_mut();
        };
        if std::env::var_os("PYRS_TRACE_CPY_API").is_some() {
            eprintln!(
                "[cpy-api] cpython_call_object ptr={:p} callable={}",
                callable_ptr,
                cpython_value_debug_tag(&callable)
            );
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_internal(callable, args, kwargs) {
            Ok(InternalCallOutcome::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("object call failed")
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

fn cpython_positional_args_from_tuple_object(args: *mut c_void) -> Result<Vec<Value>, String> {
    if args.is_null() {
        return Ok(Vec::new());
    }
    let value = cpython_value_from_ptr(args)?;
    match value {
        Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
            Object::Tuple(values) => Ok(values.clone()),
            _ => Err("invalid tuple storage".to_string()),
        },
        _ => Err("expected tuple for positional arguments".to_string()),
    }
}

fn cpython_keyword_args_from_dict_object(
    kwargs: *mut c_void,
) -> Result<HashMap<String, Value>, String> {
    if kwargs.is_null() {
        return Ok(HashMap::new());
    }
    let value = cpython_value_from_ptr(kwargs)?;
    let Value::Dict(dict_obj) = value else {
        return Err("expected dict for keyword arguments".to_string());
    };
    let Object::Dict(entries) = &*dict_obj.kind() else {
        return Err("invalid kwargs dict storage".to_string());
    };
    let mut out = HashMap::new();
    for (key, value) in entries.iter() {
        let Value::Str(name) = key else {
            return Err("keyword argument names must be str".to_string());
        };
        out.insert(name.clone(), value.clone());
    }
    Ok(out)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModuleDef_Init(module: *mut c_void) -> *mut c_void {
    module
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_Create2(module: *mut c_void, _apiver: i32) -> *mut c_void {
    if module.is_null() {
        cpython_set_error("PyModule_Create2 received null module definition");
        return std::ptr::null_mut();
    }
    let module = module.cast::<CpythonModuleDef>();
    let result = with_active_cpython_context_mut(|context| {
        if !unsafe { (*module).m_name.is_null() } {
            let name_result = unsafe { c_name_to_string((*module).m_name) };
            if let Err(err) = name_result {
                context.set_error(format!("PyModule_Create2 invalid module name: {err}"));
                return std::ptr::null_mut();
            }
        }
        context.alloc_cpython_ptr_for_value(Value::Module(context.module.clone()))
    });
    match result {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddObjectRef(
    module: *mut c_void,
    name: *const c_char,
    value: *mut c_void,
) -> i32 {
    match with_active_cpython_context_mut(|context| {
        let attr_name = match unsafe { c_name_to_string(name) } {
            Ok(name) => name,
            Err(err) => {
                context.set_error(format!("PyModule_AddObjectRef invalid name: {err}"));
                return -1;
            }
        };
        let module_obj = match context.cpython_module_obj_from_ptr(module) {
            Ok(module_obj) => module_obj,
            Err(err) => {
                context.set_error(format!("PyModule_AddObjectRef invalid module: {err}"));
                return -1;
            }
        };
        let value = match context.cpython_value_from_ptr(value) {
            Some(value) => value,
            None => {
                context.set_error("PyModule_AddObjectRef invalid value object");
                return -1;
            }
        };
        let Object::Module(module_data) = &mut *module_obj.kind_mut() else {
            context.set_error("PyModule_AddObjectRef module no longer valid");
            return -1;
        };
        module_data.globals.insert(attr_name, value);
        0
    }) {
        Ok(status) => status,
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddObject(
    module: *mut c_void,
    name: *const c_char,
    value: *mut c_void,
) -> i32 {
    let status = unsafe { PyModule_AddObjectRef(module, name, value) };
    if status != 0 || value.is_null() {
        return status;
    }
    let _ = with_active_cpython_context_mut(|context| {
        if let Some(handle) = context.cpython_handle_from_ptr(value) {
            let _ = context.decref(handle);
        }
    });
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddIntConstant(
    module: *mut c_void,
    name: *const c_char,
    value: i64,
) -> i32 {
    let object = unsafe { PyLong_FromLongLong(value) };
    if object.is_null() {
        return -1;
    }
    unsafe { PyModule_AddObject(module, name, object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddStringConstant(
    module: *mut c_void,
    name: *const c_char,
    value: *const c_char,
) -> i32 {
    let object = unsafe { PyUnicode_FromString(value) };
    if object.is_null() {
        return -1;
    }
    unsafe { PyModule_AddObject(module, name, object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetDict(module: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyModule_GetDict missing VM context");
            return std::ptr::null_mut();
        }
        let module_obj = match context.cpython_module_obj_from_ptr(module) {
            Ok(module_obj) => module_obj,
            Err(err) => {
                context.set_error(format!("PyModule_GetDict invalid module: {err}"));
                return std::ptr::null_mut();
            }
        };
        let globals = match &*module_obj.kind() {
            Object::Module(data) => data.globals.clone(),
            _ => {
                context.set_error("PyModule_GetDict module pointer is not a module");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let dict = vm.heap.alloc_dict(
            globals
                .into_iter()
                .map(|(name, value)| (Value::Str(name), value))
                .collect(),
        );
        context.alloc_cpython_ptr_for_value(dict)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromLong(value: i64) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        context.alloc_cpython_ptr_for_value(Value::Int(value))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromLongLong(value: i64) -> *mut c_void {
    unsafe { PyLong_FromLong(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromSsize_t(value: isize) -> *mut c_void {
    unsafe { PyLong_FromLongLong(value as i64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUnsignedLong(value: u64) -> *mut c_void {
    if i64::try_from(value).is_ok() {
        return cpython_new_ptr_for_value(Value::Int(value as i64));
    }
    cpython_new_ptr_for_value(Value::BigInt(Box::new(BigInt::from_u64(value))))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUnsignedLongLong(value: u64) -> *mut c_void {
    unsafe { PyLong_FromUnsignedLong(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromVoidPtr(value: *mut c_void) -> *mut c_void {
    unsafe { PyLong_FromUnsignedLongLong(value as usize as u64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUnicodeObject(object: *mut c_void, base: i32) -> *mut c_void {
    let Value::Str(text) = (match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    }) else {
        cpython_set_error("PyLong_FromUnicodeObject expects str input");
        return std::ptr::null_mut();
    };
    let parsed_base = if base == 0 {
        10
    } else if (2..=36).contains(&base) {
        base as u32
    } else {
        cpython_set_error("PyLong_FromUnicodeObject received invalid base");
        return std::ptr::null_mut();
    };
    let trimmed = text.trim();
    match BigInt::from_str_radix(trimmed, parsed_base) {
        Some(bigint) => {
            if let Some(i) = bigint.to_i64() {
                cpython_new_ptr_for_value(Value::Int(i))
            } else {
                cpython_new_ptr_for_value(Value::BigInt(Box::new(bigint)))
            }
        }
        None => {
            cpython_set_error("PyLong_FromUnicodeObject failed to parse integer");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBool_FromLong(value: i64) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        context.alloc_cpython_ptr_for_value(Value::Bool(value != 0))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_FromDouble(value: f64) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        context.alloc_cpython_ptr_for_value(Value::Float(value))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_FromString(
    object: *mut c_void,
    _endptr: *mut *mut c_char,
) -> *mut c_void {
    match cpython_value_from_ptr(object) {
        Ok(Value::Str(text)) => match text.parse::<f64>() {
            Ok(value) => cpython_new_ptr_for_value(Value::Float(value)),
            Err(_) => {
                cpython_set_error("PyFloat_FromString failed to parse float");
                std::ptr::null_mut()
            }
        },
        Ok(_) => {
            cpython_set_error("PyFloat_FromString expects str object");
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
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
    unsafe { PyUnicode_FromString(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromFormat(format: *const c_char) -> *mut c_void {
    unsafe { PyUnicode_FromString(format) }
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
pub unsafe extern "C" fn PyBytes_FromStringAndSize(
    value: *const c_char,
    len: isize,
) -> *mut c_void {
    if len < 0 {
        cpython_set_error("PyBytes_FromStringAndSize received negative length");
        return std::ptr::null_mut();
    }
    if value.is_null() && len != 0 {
        cpython_set_error("PyBytes_FromStringAndSize received null pointer with non-zero length");
        return std::ptr::null_mut();
    }
    let bytes = if len == 0 {
        Vec::new()
    } else {
        // SAFETY: caller guarantees `value` points to at least `len` bytes.
        unsafe { std::slice::from_raw_parts(value.cast::<u8>(), len as usize).to_vec() }
    };
    cpython_new_bytes_ptr(bytes)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_FromString(value: *const c_char) -> *mut c_void {
    if value.is_null() {
        cpython_set_error("PyBytes_FromString received null pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: pointer must be NUL-terminated C string.
    let bytes = unsafe { CStr::from_ptr(value).to_bytes().to_vec() };
    cpython_new_bytes_ptr(bytes)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_Size(object: *mut c_void) -> isize {
    let foreign_bytes_len = |object: *mut c_void| -> Option<isize> {
        if object.is_null() {
            return None;
        }
        // SAFETY: pointer is treated as foreign PyObject; we only inspect fixed headers.
        let head = unsafe { object.cast::<CpythonVarObjectHead>().as_ref() }?;
        let ty = head.ob_base.ob_type.cast::<CpythonTypeObject>();
        if ty.is_null() {
            return None;
        }
        let is_bytes = ty == std::ptr::addr_of_mut!(PyBytes_Type)
            // SAFETY: type pointers are valid for subtype checks.
            || unsafe {
                PyType_IsSubtype(
                    ty.cast::<c_void>(),
                    std::ptr::addr_of_mut!(PyBytes_Type).cast::<c_void>(),
                ) != 0
            };
        if is_bytes {
            return Some(head.ob_size.max(0));
        }
        None
    };
    match cpython_value_from_ptr(object) {
        Ok(Value::Bytes(bytes_obj)) | Ok(Value::ByteArray(bytes_obj)) => match &*bytes_obj.kind() {
            Object::Bytes(values) | Object::ByteArray(values) => values.len() as isize,
            _ => {
                cpython_set_error("PyBytes_Size encountered invalid bytes storage");
                -1
            }
        },
        Ok(_) => {
            if let Some(len) = foreign_bytes_len(object) {
                return len;
            }
            cpython_set_error("PyBytes_Size expected bytes-compatible object");
            -1
        }
        Err(err) => {
            if let Some(len) = foreign_bytes_len(object) {
                return len;
            }
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_AsString(object: *mut c_void) -> *mut c_char {
    let foreign_bytes_payload = |object: *mut c_void| -> Option<*mut c_char> {
        if object.is_null() {
            return None;
        }
        // SAFETY: pointer is treated as foreign PyObject; we only inspect fixed headers.
        let head = unsafe { object.cast::<CpythonVarObjectHead>().as_ref() }?;
        let ty = head.ob_base.ob_type.cast::<CpythonTypeObject>();
        if ty.is_null() {
            return None;
        }
        let is_bytes = ty == std::ptr::addr_of_mut!(PyBytes_Type)
            // SAFETY: type pointers are valid for subtype checks.
            || unsafe {
                PyType_IsSubtype(
                    ty.cast::<c_void>(),
                    std::ptr::addr_of_mut!(PyBytes_Type).cast::<c_void>(),
                ) != 0
            };
        if !is_bytes {
            return None;
        }
        // CPython bytes layout: PyObject_VAR_HEAD + ob_shash + ob_sval[...].
        Some(unsafe {
            object
                .cast::<u8>()
                .add(std::mem::size_of::<CpythonVarObjectHead>() + std::mem::size_of::<isize>())
                .cast::<c_char>()
        })
    };
    match cpython_value_from_ptr(object) {
        Ok(Value::Bytes(bytes_obj)) | Ok(Value::ByteArray(bytes_obj)) => {
            if let Ok(true) =
                with_active_cpython_context_mut(|context| context.owns_cpython_allocation_ptr(object))
            {
                // SAFETY: owned bytes-compatible pointers use CPython bytes layout.
                return unsafe { cpython_bytes_data_ptr(object) };
            }
            match &*bytes_obj.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => {
                    values.as_ptr().cast_mut().cast()
                }
                _ => {
                    cpython_set_error("PyBytes_AsString encountered invalid bytes storage");
                    std::ptr::null_mut()
                }
            }
        }
        Ok(_) => {
            if let Some(ptr) = foreign_bytes_payload(object) {
                return ptr;
            }
            if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() && !object.is_null() {
                // SAFETY: candidate object pointer for diagnostics only.
                let ty = unsafe { (*object.cast::<CpythonObjectHead>()).ob_type };
                let ty_name = unsafe {
                    ty.cast::<CpythonTypeObject>()
                        .as_ref()
                        .and_then(|raw| c_name_to_string(raw.tp_name).ok())
                        .unwrap_or_else(|| "<unknown>".to_string())
                };
                eprintln!(
                    "[cpy-bytes] as_string mismatch object={:p} type={:p} type_name={}",
                    object, ty, ty_name
                );
            }
            cpython_set_error("PyBytes_AsString expected bytes object");
            std::ptr::null_mut()
        }
        Err(err) => {
            if let Some(ptr) = foreign_bytes_payload(object) {
                return ptr;
            }
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_AsStringAndSize(
    object: *mut c_void,
    out_buffer: *mut *mut c_char,
    out_len: *mut isize,
) -> i32 {
    if out_buffer.is_null() || out_len.is_null() {
        cpython_set_error("PyBytes_AsStringAndSize requires non-null out pointers");
        return -1;
    }
    let ptr = unsafe { PyBytes_AsString(object) };
    if ptr.is_null() {
        return -1;
    }
    let len = unsafe { PyBytes_Size(object) };
    if len < 0 {
        return -1;
    }
    // SAFETY: caller provided valid pointers.
    unsafe {
        *out_buffer = ptr;
        *out_len = len;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_Release(_view: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCallable_Check(object: *mut c_void) -> i32 {
    match with_active_cpython_context_mut(|context| {
        let value = context.cpython_value_from_ptr(object);
        if context.vm.is_null() {
            context.set_error("PyCallable_Check missing VM context");
            return -1;
        }
        let Some(value) = value else {
            context.set_error("PyCallable_Check received unknown object pointer");
            return -1;
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        if vm.is_callable_value(&value) { 1 } else { 0 }
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
    match cpython_value_from_ptr(object) {
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
            cpython_set_error("PyFloat_AsDouble expected float-compatible object");
            -1.0
        }
        Err(err) => {
            cpython_set_error(err);
            -1.0
        }
    }
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
            if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
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
    let value = unsafe { PyLong_AsLongLong(object) };
    if value < 0 {
        cpython_set_error("PyLong_AsUnsignedLong requires non-negative integer");
        return u64::MAX;
    }
    value as u64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLongLong(object: *mut c_void) -> u64 {
    unsafe { PyLong_AsUnsignedLong(object) }
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
    match cpython_value_from_ptr(object) {
        Ok(Value::Complex { real, imag }) => CpythonComplexValue { real, imag },
        Ok(Value::Float(real)) => CpythonComplexValue { real, imag: 0.0 },
        Ok(Value::Int(real)) => CpythonComplexValue {
            real: real as f64,
            imag: 0.0,
        },
        Ok(_) => {
            cpython_set_error("PyComplex_AsCComplex expected complex-compatible object");
            CpythonComplexValue {
                real: -1.0,
                imag: 0.0,
            }
        }
        Err(err) => {
            cpython_set_error(err);
            CpythonComplexValue {
                real: -1.0,
                imag: 0.0,
            }
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
pub unsafe extern "C" fn PyNumber_Check(object: *mut c_void) -> i32 {
    match cpython_value_from_ptr(object) {
        Ok(
            Value::Bool(_)
            | Value::Int(_)
            | Value::BigInt(_)
            | Value::Float(_)
            | Value::Complex { .. },
        ) => 1,
        Ok(_) => 0,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Absolute(object: *mut c_void) -> *mut c_void {
    cpython_unary_numeric_op(object, |value| match value {
        Value::Complex { real, imag } => Ok(Value::Float((real * real + imag * imag).sqrt())),
        Value::Int(value) => Ok(Value::Int(value.saturating_abs())),
        Value::Bool(value) => Ok(Value::Int(if value { 1 } else { 0 })),
        Value::Float(value) => Ok(Value::Float(value.abs())),
        Value::BigInt(value) => {
            if value.is_negative() {
                neg_value(Value::BigInt(value))
            } else {
                Ok(Value::BigInt(value))
            }
        }
        other => Err(RuntimeError::new(format!(
            "bad operand type for abs(): {:?}",
            other
        ))),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Add(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op_with_heap(left, right, add_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Subtract(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op_with_heap(left, right, sub_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Multiply(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op_with_heap(left, right, mul_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_TrueDivide(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op(left, right, div_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_FloorDivide(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    cpython_binary_numeric_op(left, right, floor_div_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Remainder(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op_with_heap(left, right, mod_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Divmod(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    let quotient = cpython_binary_numeric_op(left, right, floor_div_values);
    if quotient.is_null() {
        return std::ptr::null_mut();
    }
    let remainder = cpython_binary_numeric_op_with_heap(left, right, mod_values);
    if remainder.is_null() {
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyNumber_Divmod missing VM context");
            return std::ptr::null_mut();
        }
        let Some(q) = context.cpython_value_from_ptr(quotient) else {
            context.set_error("PyNumber_Divmod missing quotient value");
            return std::ptr::null_mut();
        };
        let Some(r) = context.cpython_value_from_ptr(remainder) else {
            context.set_error("PyNumber_Divmod missing remainder value");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let tuple = vm.heap.alloc(Object::Tuple(vec![q, r]));
        context.alloc_cpython_ptr_for_value(Value::Tuple(tuple))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Power(
    left: *mut c_void,
    right: *mut c_void,
    _modulo: *mut c_void,
) -> *mut c_void {
    cpython_binary_numeric_op(left, right, pow_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Lshift(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op(left, right, lshift_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Rshift(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op(left, right, rshift_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_And(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op_with_heap(left, right, and_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Or(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op_with_heap(left, right, or_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Xor(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    cpython_binary_numeric_op_with_heap(left, right, xor_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Negative(object: *mut c_void) -> *mut c_void {
    cpython_unary_numeric_op(object, neg_value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Positive(object: *mut c_void) -> *mut c_void {
    cpython_unary_numeric_op(object, pos_value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Invert(object: *mut c_void) -> *mut c_void {
    cpython_unary_numeric_op(object, invert_value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Long(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if let Some(value) = context.cpython_value_from_ptr(object) {
            return match value_to_int(value) {
                Ok(value) => context.alloc_cpython_ptr_for_value(Value::Int(value)),
                Err(err) => {
                    context.set_error(err.message);
                    std::ptr::null_mut()
                }
            };
        }
        if object.is_null() {
            context.set_error("PyNumber_Long expected object");
            return std::ptr::null_mut();
        }
        // SAFETY: `object` is a foreign PyObject* from extension code.
        let type_ptr = unsafe {
            object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if type_ptr.is_null() {
            context.set_error("unknown PyObject pointer");
            return std::ptr::null_mut();
        }
        // SAFETY: `type_ptr` is non-null and points to a type object.
        let number_methods = unsafe {
            (*type_ptr)
                .tp_as_number
                .cast::<CpythonNumberMethods>()
                .as_ref()
        };
        let Some(number_methods) = number_methods else {
            context.set_error("unknown PyObject pointer");
            return std::ptr::null_mut();
        };
        let converter = number_methods.nb_int.or(number_methods.nb_index);
        let Some(converter) = converter else {
            context.set_error("PyNumber_Long requires int-compatible object");
            return std::ptr::null_mut();
        };
        // SAFETY: `converter` is a valid nb_int/nb_index slot for this object type.
        unsafe { converter(object) }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Float(object: *mut c_void) -> *mut c_void {
    let value = unsafe { PyFloat_AsDouble(object) };
    if value == -1.0 && !unsafe { PyErr_Occurred() }.is_null() {
        return std::ptr::null_mut();
    }
    cpython_new_ptr_for_value(Value::Float(value))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Index(object: *mut c_void) -> *mut c_void {
    unsafe { PyNumber_Long(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_AsSsize_t(object: *mut c_void, _exc: *mut c_void) -> isize {
    unsafe { PyLong_AsSsize_t(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ImportModule(name: *const c_char) -> *mut c_void {
    match unsafe { c_name_to_string(name) } {
        Ok(module_name) => {
            if std::env::var_os("PYRS_TRACE_CPY_API").is_some() {
                eprintln!("[cpy-api] PyImport_ImportModule name={module_name}");
            }
            with_active_cpython_context_mut(|context| match context.module_import(&module_name) {
                Ok(handle) => context.alloc_cpython_ptr_for_handle(handle),
                Err(err) => {
                    context.set_error(err);
                    std::ptr::null_mut()
                }
            })
            .unwrap_or_else(|err| {
                cpython_set_error(err);
                std::ptr::null_mut()
            })
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_Import(name: *mut c_void) -> *mut c_void {
    let module_name = match cpython_value_from_ptr(name) {
        Ok(Value::Str(name)) => name,
        Ok(_) => {
            cpython_set_error("PyImport_Import expects module name string");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let c_name = match CString::new(module_name) {
        Ok(name) => name,
        Err(_) => {
            cpython_set_error("PyImport_Import received module name with NUL byte");
            return std::ptr::null_mut();
        }
    };
    unsafe { PyImport_ImportModule(c_name.as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyContextVar_New(
    name: *const c_char,
    default_value: *mut c_void,
) -> *mut c_void {
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyContextVar_New missing VM context");
            return std::ptr::null_mut();
        }
        let default = if default_value.is_null() {
            Value::None
        } else {
            match context.cpython_value_from_ptr_or_proxy(default_value) {
                Some(value) => value,
                None => {
                    context.set_error(format!(
                        "PyContextVar_New received unknown default pointer {:p}",
                        default_value
                    ));
                    return std::ptr::null_mut();
                }
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let dict = vm.heap.alloc_dict(vec![
            (Value::Str("name".to_string()), Value::Str(name)),
            (Value::Str("default".to_string()), default),
        ]);
        context.alloc_cpython_ptr_for_value(dict)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyContextVar_Get(
    var: *mut c_void,
    default_value: *mut c_void,
    out_value: *mut *mut c_void,
) -> i32 {
    if out_value.is_null() {
        cpython_set_error("PyContextVar_Get requires non-null output pointer");
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        // Prefer explicit default value if provided.
        let resolved = if !default_value.is_null() {
            context.cpython_value_from_ptr(default_value)
        } else {
            let Some(var_value) = context.cpython_value_from_ptr(var) else {
                context.set_error("PyContextVar_Get received unknown var pointer");
                return -1;
            };
            match var_value {
                Value::Dict(dict_obj) => {
                    dict_get_value(&dict_obj, &Value::Str("default".to_string()))
                }
                _ => None,
            }
        };
        if let Some(value) = resolved {
            let ptr = context.alloc_cpython_ptr_for_value(value);
            // SAFETY: caller provided writable out pointer.
            unsafe { *out_value = ptr };
        } else {
            // SAFETY: caller provided writable out pointer.
            unsafe { *out_value = std::ptr::null_mut() };
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyContextVar_Set(var: *mut c_void, value: *mut c_void) -> *mut c_void {
    let Some(value) = with_active_cpython_context_mut(|context| {
        let Some(var_value) = context.cpython_value_from_ptr(var) else {
            context.set_error("PyContextVar_Set received unknown var pointer");
            return None;
        };
        let Some(new_value) = context.cpython_value_from_ptr(value) else {
            context.set_error("PyContextVar_Set received unknown value pointer");
            return None;
        };
        let Value::Dict(dict_obj) = var_value else {
            context.set_error("PyContextVar_Set expected context-var object");
            return None;
        };
        let Object::Dict(_) = &mut *dict_obj.kind_mut() else {
            context.set_error("PyContextVar_Set context-var storage invalid");
            return None;
        };
        let _ = dict_set_value_checked(&dict_obj, Value::Str("value".to_string()), new_value);
        Some(context.alloc_cpython_ptr_for_value(Value::None))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        None
    }) else {
        return std::ptr::null_mut();
    };
    value
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_GetBuiltins() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyEval_GetBuiltins missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(module) = vm.modules.get("builtins") else {
            context.set_error("PyEval_GetBuiltins missing builtins module");
            return std::ptr::null_mut();
        };
        let globals = match &*module.kind() {
            Object::Module(data) => data.globals.clone(),
            _ => {
                context.set_error("PyEval_GetBuiltins invalid builtins module object");
                return std::ptr::null_mut();
            }
        };
        let entries: Vec<(Value, Value)> = globals
            .into_iter()
            .map(|(name, value)| (Value::Str(name), value))
            .collect();
        let dict = vm.heap.alloc_dict(entries);
        context.alloc_cpython_ptr_for_value(dict)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIter_Check(object: *mut c_void) -> i32 {
    match cpython_value_from_ptr(object) {
        Ok(Value::Iterator(_) | Value::Generator(_)) => 1,
        Ok(_) => 0,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIter_Next(object: *mut c_void) -> *mut c_void {
    match with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyIter_Next missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyIter_Next unknown iterator pointer");
            return std::ptr::null_mut();
        };
        let iterator_ref = match value {
            Value::Iterator(iterator) => iterator,
            _ => {
                context.set_error("PyIter_Next expected iterator object");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let next = match vm.iterator_next_value(&iterator_ref) {
            Ok(Some(next)) => next,
            Ok(None) => return std::ptr::null_mut(),
            Err(err) => {
                context.set_error(err.message);
                return std::ptr::null_mut();
            }
        };
        context.alloc_cpython_ptr_for_value(next)
    }) {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_New(
    pointer: *mut c_void,
    name: *const c_char,
    _destructor: Option<unsafe extern "C" fn(*mut c_void)>,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| match context.capsule_new(pointer, name) {
        Ok(handle) => context.alloc_cpython_ptr_for_handle(handle),
        Err(err) => {
            context.set_error(err);
            std::ptr::null_mut()
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_GetPointer(
    capsule: *mut c_void,
    name: *const c_char,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_GetPointer received unknown object pointer");
            return std::ptr::null_mut();
        };
        if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some()
            && !context.capsules.contains_key(&handle)
        {
            let tag = context
                .objects
                .get(&handle)
                .map(|slot| cpython_value_debug_tag(&slot.value))
                .unwrap_or_else(|| "<missing>".to_string());
            let requested_name = if name.is_null() {
                "<null>".to_string()
            } else {
                // SAFETY: caller provides a NUL-terminated capsule name.
                unsafe { CStr::from_ptr(name) }
                    .to_str()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|_| "<invalid utf8>".to_string())
            };
            let raw_type = if capsule.is_null() {
                std::ptr::null_mut()
            } else {
                // SAFETY: `capsule` is a candidate PyObject*.
                unsafe { (*capsule.cast::<CpythonObjectHead>()).ob_type }
            };
            let raw_type_name = unsafe {
                raw_type
                    .cast::<CpythonTypeObject>()
                    .as_ref()
                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string())
            };
            eprintln!(
                "[cpy-capsule] get_pointer ptr={:p} handle={} name={} non_capsule_tag={} raw_type={:p} raw_type_name={} expected_capsule_type={:p}",
                capsule,
                handle,
                requested_name,
                tag,
                raw_type,
                raw_type_name,
                std::ptr::addr_of_mut!(PyCapsule_Type).cast::<c_void>()
            );
        }
        if !context.capsules.contains_key(&handle) {
            match unsafe { cpython_external_capsule_pointer(capsule, name) } {
                Ok(Some(pointer)) => return pointer,
                Ok(None) => {}
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
        }
        match context.capsule_get_pointer(handle, name) {
            Ok(pointer) => pointer,
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
pub unsafe extern "C" fn PyCapsule_SetContext(
    capsule: *mut c_void,
    context_value: *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_SetContext received unknown object pointer");
            return -1;
        };
        match context.capsule_set_context(handle, context_value) {
            Ok(()) => 0,
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyCapsule_GetContext(capsule: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_GetContext received unknown object pointer");
            return std::ptr::null_mut();
        };
        match context.capsule_get_context(handle) {
            Ok(ctx) => ctx,
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
pub unsafe extern "C" fn PyCapsule_SetName(capsule: *mut c_void, name: *const c_char) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_SetName received unknown object pointer");
            return -1;
        };
        match context.capsule_set_name(handle, name) {
            Ok(()) => 0,
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyCapsule_IsValid(capsule: *mut c_void, name: *const c_char) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            return 0;
        };
        match context.capsule_is_valid(handle, name) {
            Ok(valid) => valid,
            Err(_) => 0,
        }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_Import(name: *const c_char, no_block: i32) -> *mut c_void {
    with_active_cpython_context_mut(|context| match context.capsule_import(name, no_block) {
        Ok(pointer) => pointer,
        Err(err) => {
            context.set_error(err);
            std::ptr::null_mut()
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_New(size: isize) -> *mut c_void {
    if size < 0 {
        cpython_set_error("PyList_New requires non-negative size");
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let list = vm
            .heap
            .alloc(Object::List(vec![Value::None; size as usize]));
        context.alloc_cpython_ptr_for_value(Value::List(list))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Size(list: *mut c_void) -> isize {
    match cpython_value_from_ptr(list) {
        Ok(Value::List(list_obj)) => match &*list_obj.kind() {
            Object::List(values) => values.len() as isize,
            _ => {
                cpython_set_error("PyList_Size encountered invalid list storage");
                -1
            }
        },
        Ok(_) => {
            cpython_set_error("PyList_Size expected list object");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Append(list: *mut c_void, item: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let item_value = match context.cpython_value_from_ptr(item) {
            Some(value) => value,
            None => {
                context.set_error("PyList_Append received unknown item pointer");
                return -1;
            }
        };
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            context.set_error("PyList_Append received unknown list pointer");
            return -1;
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PyList_Append list handle is not available");
            return -1;
        };
        let Value::List(list_obj) = &mut slot.value else {
            context.set_error("PyList_Append expected list object");
            return -1;
        };
        let Object::List(values) = &mut *list_obj.kind_mut() else {
            context.set_error("PyList_Append encountered invalid list storage");
            return -1;
        };
        values.push(item_value);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_GetItemRef(list: *mut c_void, index: isize) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(list) else {
            context.set_error("PyList_GetItemRef received unknown list pointer");
            return std::ptr::null_mut();
        };
        let Value::List(list_obj) = value else {
            context.set_error("PyList_GetItemRef expected list object");
            return std::ptr::null_mut();
        };
        let Object::List(values) = &*list_obj.kind() else {
            context.set_error("PyList_GetItemRef encountered invalid list storage");
            return std::ptr::null_mut();
        };
        let idx = if index < 0 {
            values.len() as isize + index
        } else {
            index
        };
        if idx < 0 || idx as usize >= values.len() {
            context.set_error("PyList_GetItemRef index out of range");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(values[idx as usize].clone())
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_AsTuple(list: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_AsTuple missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr(list) else {
            context.set_error("PyList_AsTuple received unknown list pointer");
            return std::ptr::null_mut();
        };
        let Value::List(list_obj) = value else {
            context.set_error("PyList_AsTuple expected list object");
            return std::ptr::null_mut();
        };
        let Object::List(values) = &*list_obj.kind() else {
            context.set_error("PyList_AsTuple encountered invalid list storage");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let tuple = vm.heap.alloc(Object::Tuple(values.clone()));
        context.alloc_cpython_ptr_for_value(Value::Tuple(tuple))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_New(size: isize) -> *mut c_void {
    if size < 0 {
        cpython_set_error("PyTuple_New requires non-negative size");
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyTuple_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let tuple = vm
            .heap
            .alloc(Object::Tuple(vec![Value::None; size as usize]));
        let ptr = context.alloc_cpython_ptr_for_value(Value::Tuple(tuple));
        if std::env::var_os("PYRS_TRACE_CPY_TUPLE").is_some() {
            eprintln!("[cpy-tuple] new size={} ptr={:p}", size, ptr);
        }
        ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_Size(tuple: *mut c_void) -> isize {
    if tuple.is_null() {
        cpython_set_error("PyTuple_Size expected tuple object");
        return -1;
    }
    if let Ok(Some(size)) = with_active_cpython_context_mut(|context| {
        if context.owns_cpython_allocation_ptr(tuple) {
            // SAFETY: owned tuple pointers use CPython-compatible varobject header.
            let size = unsafe { (*tuple.cast::<CpythonVarObjectHead>()).ob_size };
            return Some(size.max(0));
        }
        None
    }) {
        return size;
    }
    match cpython_value_from_ptr(tuple) {
        Ok(Value::Tuple(tuple_obj)) => match &*tuple_obj.kind() {
            Object::Tuple(values) => values.len() as isize,
            _ => {
                cpython_set_error("PyTuple_Size encountered invalid tuple storage");
                -1
            }
        },
        Ok(_) => {
            cpython_set_error("PyTuple_Size expected tuple object");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_GetItem(tuple: *mut c_void, index: isize) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.owns_cpython_allocation_ptr(tuple) {
            // SAFETY: owned tuple pointers use CPython-compatible varobject header
            // followed by contiguous `PyObject*` item slots.
            unsafe {
                let head = tuple.cast::<CpythonVarObjectHead>();
                let len = (*head).ob_size.max(0) as usize;
                let idx = if index < 0 {
                    len as isize + index
                } else {
                    index
                };
                if idx < 0 || idx as usize >= len {
                    context.set_error("PyTuple_GetItem index out of range");
                    return std::ptr::null_mut();
                }
                let items_ptr = cpython_tuple_items_ptr(tuple);
                return *items_ptr.add(idx as usize);
            }
        }
        let Some(value) = context.cpython_value_from_ptr(tuple) else {
            context.set_error("PyTuple_GetItem received unknown tuple pointer");
            return std::ptr::null_mut();
        };
        let Value::Tuple(tuple_obj) = value else {
            context.set_error("PyTuple_GetItem expected tuple object");
            return std::ptr::null_mut();
        };
        let Object::Tuple(values) = &*tuple_obj.kind() else {
            context.set_error("PyTuple_GetItem encountered invalid tuple storage");
            return std::ptr::null_mut();
        };
        let idx = if index < 0 {
            values.len() as isize + index
        } else {
            index
        };
        if idx < 0 || idx as usize >= values.len() {
            context.set_error("PyTuple_GetItem index out of range");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(values[idx as usize].clone())
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_SetItem(
    tuple: *mut c_void,
    index: isize,
    item: *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        let item_handle = context.cpython_handle_from_ptr(item);
        let Some(handle) = context.cpython_handle_from_ptr(tuple) else {
            context.set_error("PyTuple_SetItem received unknown tuple pointer");
            return -1;
        };
        let item_value = match context.cpython_value_from_ptr_or_proxy(item) {
            Some(value) => value,
            None => {
                context.set_error("PyTuple_SetItem received unknown item pointer");
                return -1;
            }
        };
        if std::env::var_os("PYRS_TRACE_CPY_TUPLE_SET").is_some() {
            eprintln!(
                "[cpy-tuple-set] tuple={:p} idx={} item_ptr={:p} item={}",
                tuple,
                index,
                item,
                cpython_debug_compare_value(&item_value)
            );
        }
        let mut status = 0;
        {
            let Some(slot) = context.objects.get_mut(&handle) else {
                context.set_error("PyTuple_SetItem tuple handle is not available");
                return -1;
            };
            let Value::Tuple(tuple_obj) = &mut slot.value else {
                context.set_error("PyTuple_SetItem expected tuple object");
                return -1;
            };
            let Object::Tuple(values) = &mut *tuple_obj.kind_mut() else {
                context.set_error("PyTuple_SetItem encountered invalid tuple storage");
                return -1;
            };
            let idx = if index < 0 {
                values.len() as isize + index
            } else {
                index
            };
            if idx < 0 || idx as usize >= values.len() {
                status = -1;
            } else {
                values[idx as usize] = item_value;
            }
        }
        if status != 0 {
            context.set_error("PyTuple_SetItem index out of range");
            return -1;
        }
        if context.owns_cpython_allocation_ptr(tuple) {
            // SAFETY: owned tuple pointers use CPython-compatible varobject header
            // followed by contiguous `PyObject*` item slots.
            unsafe {
                let head = tuple.cast::<CpythonVarObjectHead>();
                let capacity = (*head).ob_size.max(0) as usize;
                let idx = if index < 0 {
                    capacity as isize + index
                } else {
                    index
                };
                if idx >= 0 && (idx as usize) < capacity {
                    let items_ptr = cpython_tuple_items_ptr(tuple);
                    *items_ptr.add(idx as usize) = item;
                }
            }
        }
        if let Some(item_handle) = item_handle {
            let _ = context.decref(item_handle);
        }
        if std::env::var_os("PYRS_TRACE_CPY_TUPLE").is_some() {
            eprintln!(
                "[cpy-tuple] set ptr={:p} index={} item={:p}",
                tuple, index, item
            );
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_GetSlice(
    tuple: *mut c_void,
    low: isize,
    high: isize,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyTuple_GetSlice missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr(tuple) else {
            context.set_error("PyTuple_GetSlice received unknown tuple pointer");
            return std::ptr::null_mut();
        };
        let Value::Tuple(tuple_obj) = value else {
            context.set_error("PyTuple_GetSlice expected tuple object");
            return std::ptr::null_mut();
        };
        let Object::Tuple(values) = &*tuple_obj.kind() else {
            context.set_error("PyTuple_GetSlice encountered invalid tuple storage");
            return std::ptr::null_mut();
        };
        let len = values.len() as isize;
        let start = low.clamp(0, len) as usize;
        let end = high.clamp(0, len) as usize;
        let slice = if end >= start {
            values[start..end].to_vec()
        } else {
            Vec::new()
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let result = vm.heap.alloc(Object::Tuple(slice));
        context.alloc_cpython_ptr_for_value(Value::Tuple(result))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[allow(non_snake_case)]
pub unsafe extern "C" fn PyTuple_Pack(size: isize) -> *mut c_void {
    if size < 0 {
        cpython_set_error("PyTuple_Pack requires non-negative size");
        return std::ptr::null_mut();
    }
    unsafe { PyTuple_New(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_New() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyDict_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let dict = vm.heap.alloc_dict(Vec::new());
        context.alloc_cpython_ptr_for_value(dict)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Size(dict: *mut c_void) -> isize {
    match cpython_value_from_ptr(dict) {
        Ok(Value::Dict(dict_obj)) => match &*dict_obj.kind() {
            Object::Dict(values) => values.len() as isize,
            _ => {
                cpython_set_error("PyDict_Size encountered invalid dict storage");
                -1
            }
        },
        Ok(_) => {
            cpython_set_error("PyDict_Size expected dict object");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_SetItem(
    dict: *mut c_void,
    key: *mut c_void,
    value: *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        if value.is_null() && std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
            eprintln!(
                "[cpy-err] PyDict_SetItem null value pointer dict={:p} key={:p}",
                dict, key
            );
            eprintln!("{:?}", std::backtrace::Backtrace::capture());
        }
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_SetItem received unknown dict pointer");
            return -1;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_SetItem expected dict object");
            return -1;
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyDict_SetItem received unknown key pointer");
            return -1;
        };
        let Some(item_value) = context.cpython_value_from_ptr_or_proxy(value) else {
            context.set_error("PyDict_SetItem received unknown value pointer");
            return -1;
        };
        match dict_set_value_checked(&dict_obj, key_value, item_value) {
            Ok(()) => 0,
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
pub unsafe extern "C" fn PyDict_GetItem(dict: *mut c_void, key: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_GetItem received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_GetItem expected dict object");
            return std::ptr::null_mut();
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyDict_GetItem received unknown key pointer");
            return std::ptr::null_mut();
        };
        let Some(value) = dict_get_value(&dict_obj, &key_value) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemWithError(
    dict: *mut c_void,
    key: *mut c_void,
) -> *mut c_void {
    unsafe { PyDict_GetItem(dict, key) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Contains(dict: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Contains received unknown dict pointer");
            return -1;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Contains expected dict object");
            return -1;
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyDict_Contains received unknown key pointer");
            return -1;
        };
        match dict_contains_key_checked(&dict_obj, &key_value) {
            Ok(true) => 1,
            Ok(false) => 0,
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
pub unsafe extern "C" fn PyDict_SetItemString(
    dict: *mut c_void,
    key: *const c_char,
    value: *mut c_void,
) -> i32 {
    if value.is_null() && std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
        let key_name = unsafe { c_name_to_string(key) }.unwrap_or_else(|_| "<invalid>".to_string());
        eprintln!("[cpy-err] PyDict_SetItemString null value key={key_name}");
    }
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        return -1;
    }
    unsafe { PyDict_SetItem(dict, key_obj, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemString(
    dict: *mut c_void,
    key: *const c_char,
) -> *mut c_void {
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { PyDict_GetItem(dict, key_obj) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemRef(
    dict: *mut c_void,
    key: *mut c_void,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() {
        cpython_set_error("PyDict_GetItemRef requires non-null out pointer");
        return -1;
    }
    let value = unsafe { PyDict_GetItem(dict, key) };
    // SAFETY: caller provided writable pointer.
    unsafe { *out = value };
    if value.is_null() { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemStringRef(
    dict: *mut c_void,
    key: *const c_char,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() {
        cpython_set_error("PyDict_GetItemStringRef requires non-null out pointer");
        return -1;
    }
    let value = unsafe { PyDict_GetItemString(dict, key) };
    // SAFETY: caller provided writable pointer.
    unsafe { *out = value };
    if value.is_null() { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_DelItem(dict: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_DelItem received unknown dict pointer");
            return -1;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_DelItem expected dict object");
            return -1;
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyDict_DelItem received unknown key pointer");
            return -1;
        };
        if dict_remove_value(&dict_obj, &key_value).is_some() {
            0
        } else {
            context.set_error("PyDict_DelItem key not found");
            -1
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_DelItemString(dict: *mut c_void, key: *const c_char) -> i32 {
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        return -1;
    }
    unsafe { PyDict_DelItem(dict, key_obj) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_ContainsString(dict: *mut c_void, key: *const c_char) -> i32 {
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        return -1;
    }
    unsafe { PyDict_Contains(dict, key_obj) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Copy(dict: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyDict_Copy missing VM context");
            return std::ptr::null_mut();
        }
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Copy received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Copy expected dict object");
            return std::ptr::null_mut();
        };
        let entries = match &*dict_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Copy encountered invalid dict storage");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let copied = vm.heap.alloc_dict(entries);
        context.alloc_cpython_ptr_for_value(copied)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Merge(
    dict: *mut c_void,
    other: *mut c_void,
    _override: i32,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Merge received unknown dict pointer");
            return -1;
        };
        let Some(source) = context.cpython_value_from_ptr(other) else {
            context.set_error("PyDict_Merge received unknown source pointer");
            return -1;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Merge expected target dict");
            return -1;
        };
        let Value::Dict(source_obj) = source else {
            context.set_error("PyDict_Merge expected source dict");
            return -1;
        };
        let source_entries = match &*source_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Merge encountered invalid source dict storage");
                return -1;
            }
        };
        for (key, value) in source_entries {
            if let Err(err) = dict_set_value_checked(&dict_obj, key, value) {
                context.set_error(err.message);
                return -1;
            }
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Next(
    dict: *mut c_void,
    position: *mut isize,
    out_key: *mut *mut c_void,
    out_value: *mut *mut c_void,
) -> i32 {
    if position.is_null() {
        cpython_set_error("PyDict_Next requires non-null position pointer");
        return 0;
    }
    with_active_cpython_context_mut(|context| {
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Next received unknown dict pointer");
            return 0;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Next expected dict object");
            return 0;
        };
        let entries = match &*dict_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Next encountered invalid dict storage");
                return 0;
            }
        };
        // SAFETY: caller-provided pointer is writable.
        let idx = unsafe { *position };
        if idx < 0 || idx as usize >= entries.len() {
            return 0;
        }
        let (key, value) = entries[idx as usize].clone();
        if !out_key.is_null() {
            // SAFETY: caller-provided pointer is writable.
            unsafe { *out_key = context.alloc_cpython_ptr_for_value(key) };
        }
        if !out_value.is_null() {
            // SAFETY: caller-provided pointer is writable.
            unsafe { *out_value = context.alloc_cpython_ptr_for_value(value) };
        }
        // SAFETY: caller-provided pointer is writable.
        unsafe { *position = idx + 1 };
        1
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDictProxy_New(dict: *mut c_void) -> *mut c_void {
    unsafe { PyDict_Copy(dict) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAttrString(
    object: *mut c_void,
    name: *const c_char,
) -> *mut c_void {
    let object_value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    if std::env::var_os("PYRS_TRACE_CPY_API").is_some() {
        let tag = cpython_value_debug_tag(&object_value);
        eprintln!(
            "[cpy-api] PyObject_GetAttrString object_ptr={:p} object={} attr={}",
            object, tag, name
        );
    }
    match cpython_call_builtin(
        BuiltinFunction::GetAttr,
        vec![object_value, Value::Str(name)],
    ) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAttr(object: *mut c_void, name: *mut c_void) -> *mut c_void {
    let object_value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let name_value = match cpython_value_from_ptr(name) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::GetAttr, vec![object_value, name_value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericGetAttr(
    object: *mut c_void,
    name: *mut c_void,
) -> *mut c_void {
    unsafe { PyObject_GetAttr(object, name) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericSetAttr(
    object: *mut c_void,
    name: *mut c_void,
    value: *mut c_void,
) -> i32 {
    let object_value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let name_value = match cpython_value_from_ptr(name) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };

    let result = if value.is_null() {
        cpython_call_builtin(BuiltinFunction::DelAttr, vec![object_value, name_value])
    } else {
        let attr_value = match cpython_value_from_ptr(value) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return -1;
            }
        };
        cpython_call_builtin(
            BuiltinFunction::SetAttr,
            vec![object_value, name_value, attr_value],
        )
    };
    match result {
        Ok(_) => 0,
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericGetDict(
    object: *mut c_void,
    _context: *mut c_void,
) -> *mut c_void {
    unsafe { PyObject_GetAttrString(object, c"__dict__".as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericSetDict(
    object: *mut c_void,
    value: *mut c_void,
    _context: *mut c_void,
) -> i32 {
    if value.is_null() {
        cpython_set_error("PyObject_GenericSetDict does not support deleting __dict__");
        return -1;
    }
    unsafe { PyObject_SetAttrString(object, c"__dict__".as_ptr(), value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetAttrString(
    object: *mut c_void,
    name: *const c_char,
    value: *mut c_void,
) -> i32 {
    let object_value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
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
    match cpython_call_builtin(
        BuiltinFunction::SetAttr,
        vec![object_value, Value::Str(name), value],
    ) {
        Ok(_) => 0,
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_HasAttrString(object: *mut c_void, name: *const c_char) -> i32 {
    let value = unsafe { PyObject_GetAttrString(object, name) };
    if value.is_null() {
        unsafe { PyErr_Clear() };
        0
    } else {
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsTrue(object: *mut c_void) -> i32 {
    match with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_IsTrue missing VM context");
            return -1;
        }
        let Some(value) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyObject_IsTrue received unknown object pointer");
            return -1;
        };
        if is_truthy(&value) { 1 } else { 0 }
    }) {
        Ok(result) => result,
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Not(object: *mut c_void) -> i32 {
    let truthy = unsafe { PyObject_IsTrue(object) };
    if truthy < 0 {
        -1
    } else if truthy == 0 {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Str(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::Str, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Bytes(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::Bytes, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Format(
    object: *mut c_void,
    format_spec: *mut c_void,
) -> *mut c_void {
    let object = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let spec = if format_spec.is_null() {
        Value::Str(String::new())
    } else {
        match cpython_value_from_ptr(format_spec) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    match cpython_call_builtin(BuiltinFunction::Format, vec![object, spec]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetIter(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::Iter, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SelfIter(object: *mut c_void) -> *mut c_void {
    unsafe { Py_XIncRef(object) };
    object
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallObject(
    callable: *mut c_void,
    args: *mut c_void,
) -> *mut c_void {
    let args = match cpython_positional_args_from_tuple_object(args) {
        Ok(args) => args,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_call_object(callable, args, HashMap::new())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Call(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    let args = match cpython_positional_args_from_tuple_object(args) {
        Ok(args) => args,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let kwargs = match cpython_keyword_args_from_dict_object(kwargs) {
        Ok(kwargs) => kwargs,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_call_object(callable, args, kwargs)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallOneArg(
    callable: *mut c_void,
    arg: *mut c_void,
) -> *mut c_void {
    let arg = match cpython_value_from_ptr(arg) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    cpython_call_object(callable, vec![arg], HashMap::new())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMethod_New(function: *mut c_void, self_obj: *mut c_void) -> *mut c_void {
    // Minimal baseline: treat method construction as returning the callable when full
    // bound-method construction is unavailable in the compat object space.
    let _ = self_obj;
    unsafe { Py_XIncRef(function) };
    function
}

#[allow(non_snake_case)]
pub unsafe extern "C" fn PyObject_CallFunction(
    callable: *mut c_void,
    format: *const c_char,
) -> *mut c_void {
    let format_text = if format.is_null() {
        String::new()
    } else {
        match unsafe { c_name_to_string(format) } {
            Ok(text) => text,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    if format_text.is_empty() || format_text == "()" {
        return unsafe { PyObject_CallObject(callable, std::ptr::null_mut()) };
    }
    cpython_set_error(format!(
        "PyObject_CallFunction variadic format parsing is not implemented for '{}'",
        format_text
    ));
    std::ptr::null_mut()
}

#[allow(non_snake_case)]
pub unsafe extern "C" fn PyObject_CallFunctionObjArgs(
    callable: *mut c_void,
    arg0: *mut c_void,
) -> *mut c_void {
    if arg0.is_null() {
        return unsafe { PyObject_CallObject(callable, std::ptr::null_mut()) };
    }
    unsafe { PyObject_CallOneArg(callable, arg0) }
}

#[allow(non_snake_case)]
pub unsafe extern "C" fn PyObject_CallMethod(
    object: *mut c_void,
    method: *const c_char,
    _format: *const c_char,
) -> *mut c_void {
    let callable = unsafe { PyObject_GetAttrString(object, method) };
    if callable.is_null() {
        return std::ptr::null_mut();
    }
    let result = unsafe { PyObject_CallObject(callable, std::ptr::null_mut()) };
    unsafe { Py_DecRef(callable) };
    result
}

#[allow(non_snake_case)]
pub unsafe extern "C" fn PyArg_ParseTuple(_args: *mut c_void, _format: *const c_char) -> i32 {
    cpython_set_error("PyArg_ParseTuple variadic output parsing is not implemented");
    0
}

#[allow(non_snake_case)]
pub unsafe extern "C" fn PyArg_ParseTupleAndKeywords(
    _args: *mut c_void,
    _kwargs: *mut c_void,
    _format: *const c_char,
    _keywords: *mut *const c_char,
) -> i32 {
    cpython_set_error("PyArg_ParseTupleAndKeywords variadic output parsing is not implemented");
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyArg_VaParseTupleAndKeywords(
    _args: *mut c_void,
    _kwargs: *mut c_void,
    _format: *const c_char,
    _keywords: *mut *const c_char,
    _vargs: *mut c_void,
) -> i32 {
    cpython_set_error("PyArg_VaParseTupleAndKeywords is not implemented");
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyArg_UnpackTuple(
    args: *mut c_void,
    _name: *const c_char,
    min: isize,
    max: isize,
) -> i32 {
    let argc = unsafe { PyTuple_Size(args) };
    if argc < 0 {
        return 0;
    }
    if argc < min || argc > max {
        cpython_set_error("PyArg_UnpackTuple argument count mismatch");
        return 0;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyVectorcall_Call(
    callable: *mut c_void,
    tuple: *mut c_void,
    dict: *mut c_void,
) -> *mut c_void {
    unsafe { PyObject_Call(callable, tuple, dict) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Vectorcall(
    callable: *mut c_void,
    args: *const *mut c_void,
    nargsf: usize,
    kwnames: *mut c_void,
) -> *mut c_void {
    let positional_count = nargsf & (usize::MAX >> 1);
    let kw_count = if kwnames.is_null() {
        0usize
    } else {
        unsafe { PyTuple_Size(kwnames) }.max(0) as usize
    };
    let total_count = positional_count.saturating_add(kw_count);
    let mut values = Vec::with_capacity(total_count);
    if total_count > 0 {
        if args.is_null() {
            cpython_set_error("PyObject_Vectorcall received null args with non-zero nargsf");
            return std::ptr::null_mut();
        }
        for idx in 0..total_count {
            // SAFETY: caller promises args has at least total_count entries.
            let ptr = unsafe { *args.add(idx) };
            let value = match cpython_value_from_ptr(ptr) {
                Ok(value) => value,
                Err(err) => {
                    cpython_set_error(err);
                    return std::ptr::null_mut();
                }
            };
            values.push(value);
        }
    }
    let mut kwargs = HashMap::new();
    if kw_count > 0 {
        let kw_tuple = match cpython_value_from_ptr(kwnames) {
            Ok(Value::Tuple(tuple_obj)) => tuple_obj,
            Ok(_) => {
                cpython_set_error("PyObject_Vectorcall expected tuple keyword names");
                return std::ptr::null_mut();
            }
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
        let Object::Tuple(names) = &*kw_tuple.kind() else {
            cpython_set_error("PyObject_Vectorcall keyword tuple storage invalid");
            return std::ptr::null_mut();
        };
        if names.len() != kw_count {
            cpython_set_error("PyObject_Vectorcall keyword tuple length mismatch");
            return std::ptr::null_mut();
        }
        for (offset, name_value) in names.iter().enumerate() {
            let Value::Str(name) = name_value else {
                cpython_set_error("PyObject_Vectorcall keyword names must be str");
                return std::ptr::null_mut();
            };
            let value_index = positional_count + offset;
            let Some(value) = values.get(value_index) else {
                cpython_set_error("PyObject_Vectorcall keyword value missing");
                return std::ptr::null_mut();
            };
            kwargs.insert(name.clone(), value.clone());
        }
        values.truncate(positional_count);
    }
    cpython_call_object(callable, values, kwargs)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_VectorcallMethod(
    name: *mut c_void,
    args: *const *mut c_void,
    nargsf: usize,
    kwnames: *mut c_void,
) -> *mut c_void {
    if args.is_null() || nargsf == 0 {
        cpython_set_error("PyObject_VectorcallMethod requires self arg");
        return std::ptr::null_mut();
    }
    // SAFETY: caller guarantees at least one arg pointer.
    let self_obj = unsafe { *args };
    let method = unsafe { PyObject_GetAttr(self_obj, name) };
    if method.is_null() {
        return std::ptr::null_mut();
    }
    let remaining = nargsf.saturating_sub(1);
    let result = unsafe { PyObject_Vectorcall(method, args.add(1), remaining, kwnames) };
    unsafe { Py_DecRef(method) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetItem(object: *mut c_void, key: *mut c_void) -> *mut c_void {
    let callable = unsafe { PyObject_GetAttrString(object, c"__getitem__".as_ptr()) };
    if callable.is_null() {
        return std::ptr::null_mut();
    }
    let key = match cpython_value_from_ptr(key) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            unsafe { Py_DecRef(callable) };
            return std::ptr::null_mut();
        }
    };
    let result = cpython_call_object(callable, vec![key], HashMap::new());
    unsafe { Py_DecRef(callable) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetItem(
    object: *mut c_void,
    key: *mut c_void,
    value: *mut c_void,
) -> i32 {
    let callable = unsafe { PyObject_GetAttrString(object, c"__setitem__".as_ptr()) };
    if callable.is_null() {
        return -1;
    }
    let key = match cpython_value_from_ptr(key) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            unsafe { Py_DecRef(callable) };
            return -1;
        }
    };
    let value = match cpython_value_from_ptr(value) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            unsafe { Py_DecRef(callable) };
            return -1;
        }
    };
    let result = cpython_call_object(callable, vec![key, value], HashMap::new());
    unsafe { Py_DecRef(callable) };
    if result.is_null() {
        -1
    } else {
        unsafe { Py_DecRef(result) };
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Size(object: *mut c_void) -> isize {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    match cpython_call_builtin(BuiltinFunction::Len, vec![value]) {
        Ok(Value::Int(size)) => size as isize,
        Ok(Value::BigInt(big)) => big.to_i64().unwrap_or(-1) as isize,
        Ok(_) => {
            cpython_set_error("PyObject_Size expected integer len() result");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_LengthHint(object: *mut c_void, default: isize) -> isize {
    let size = unsafe { PyObject_Size(object) };
    if size < 0 {
        unsafe { PyErr_Clear() };
        default
    } else {
        size
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Hash(object: *mut c_void) -> isize {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    match cpython_call_builtin(BuiltinFunction::Hash, vec![value]) {
        Ok(Value::Int(hash)) => hash as isize,
        Ok(Value::BigInt(hash)) => hash.to_i64().unwrap_or(-1) as isize,
        Ok(_) => {
            cpython_set_error("PyObject_Hash expected integer hash() result");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

fn cpython_rich_compare_slot_name(op: i32) -> Option<&'static std::ffi::CStr> {
    match op {
        0 => Some(c"__lt__"),
        1 => Some(c"__le__"),
        2 => Some(c"__eq__"),
        3 => Some(c"__ne__"),
        4 => Some(c"__gt__"),
        5 => Some(c"__ge__"),
        _ => None,
    }
}

fn cpython_debug_compare_value(value: &Value) -> String {
    match value {
        Value::Tuple(tuple_obj) => {
            if let Object::Tuple(values) = &*tuple_obj.kind() {
                let mut rendered = Vec::with_capacity(values.len());
                for item in values {
                    rendered.push(match item {
                        Value::Class(obj) => format!("Class#{}", obj.id()),
                        Value::Tuple(obj) => format!("Tuple#{}", obj.id()),
                        Value::Int(v) => format!("Int({v})"),
                        Value::Str(text) => format!("Str({text})"),
                        other => format!("{other:?}"),
                    });
                }
                format!("Tuple#{}({})", tuple_obj.id(), rendered.join(","))
            } else {
                format!("Tuple#{}(<invalid>)", tuple_obj.id())
            }
        }
        Value::Class(obj) => format!("Class#{}", obj.id()),
        Value::List(obj) => format!("List#{}", obj.id()),
        Value::Int(v) => format!("Int({v})"),
        Value::Str(text) => format!("Str({text})"),
        other => format!("{other:?}"),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_RichCompare(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> *mut c_void {
    let Some(slot_name) = cpython_rich_compare_slot_name(op) else {
        cpython_set_error("PyObject_RichCompare received invalid compare op");
        return std::ptr::null_mut();
    };
    let callable = unsafe { PyObject_GetAttrString(left, slot_name.as_ptr()) };
    if callable.is_null() {
        return std::ptr::null_mut();
    }
    let right = match cpython_value_from_ptr(right) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            unsafe { Py_DecRef(callable) };
            return std::ptr::null_mut();
        }
    };
    let result = cpython_call_object(callable, vec![right], HashMap::new());
    unsafe { Py_DecRef(callable) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_RichCompareBool(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> i32 {
    let trace_compare = std::env::var_os("PYRS_TRACE_CPY_COMPARE").is_some();
    if trace_compare && op == 2 {
        let left_desc = match cpython_value_from_ptr(left) {
            Ok(value) => cpython_debug_compare_value(&value),
            Err(err) => format!("ERR({err})"),
        };
        let right_desc = match cpython_value_from_ptr(right) {
            Ok(value) => cpython_debug_compare_value(&value),
            Err(err) => format!("ERR({err})"),
        };
        eprintln!(
            "[cpy-cmp] eq left_ptr={:p} right_ptr={:p} left={} right={}",
            left, right, left_desc, right_desc
        );
    }
    let value = unsafe { PyObject_RichCompare(left, right, op) };
    if value.is_null() {
        if trace_compare && op == 2 {
            eprintln!("[cpy-cmp] eq result=<null>");
        }
        return -1;
    }
    let truth = unsafe { PyObject_IsTrue(value) };
    unsafe { Py_DecRef(value) };
    if trace_compare && op == 2 {
        eprintln!("[cpy-cmp] eq truth={truth}");
    }
    truth
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsInstance(object: *mut c_void, class: *mut c_void) -> i32 {
    let object = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let class = match cpython_value_from_ptr(class) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    match cpython_call_builtin(BuiltinFunction::IsInstance, vec![object, class]) {
        Ok(value) => {
            if is_truthy(&value) {
                1
            } else {
                0
            }
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsSubclass(subclass: *mut c_void, class: *mut c_void) -> i32 {
    let subclass = match cpython_value_from_ptr(subclass) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let class = match cpython_value_from_ptr(class) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    match cpython_call_builtin(BuiltinFunction::IsSubclass, vec![subclass, class]) {
        Ok(value) => {
            if is_truthy(&value) {
                1
            } else {
                0
            }
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetOptionalAttr(
    object: *mut c_void,
    name: *mut c_void,
    result: *mut *mut c_void,
) -> i32 {
    if result.is_null() {
        cpython_set_error("PyObject_GetOptionalAttr requires non-null result pointer");
        return -1;
    }
    let value = unsafe { PyObject_GetAttr(object, name) };
    if value.is_null() {
        unsafe {
            *result = std::ptr::null_mut();
            PyErr_Clear();
        }
        0
    } else {
        unsafe { *result = value };
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Check(object: *mut c_void) -> i32 {
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
pub unsafe extern "C" fn PySequence_GetItem(object: *mut c_void, index: isize) -> *mut c_void {
    let index = unsafe { PyLong_FromSsize_t(index) };
    if index.is_null() {
        return std::ptr::null_mut();
    }
    let value = unsafe { PyObject_GetItem(object, index) };
    unsafe { Py_DecRef(index) };
    value
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
pub unsafe extern "C" fn PyMapping_GetItemString(
    mapping: *mut c_void,
    key: *const c_char,
) -> *mut c_void {
    let key = unsafe { PyUnicode_FromString(key) };
    if key.is_null() {
        return std::ptr::null_mut();
    }
    let result = unsafe { PyObject_GetItem(mapping, key) };
    unsafe { Py_DecRef(key) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySeqIter_New(object: *mut c_void) -> *mut c_void {
    unsafe { PyObject_GetIter(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_AsFileDescriptor(object: *mut c_void) -> i32 {
    unsafe { PyLong_AsLong(object) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CheckBuffer(object: *mut c_void) -> i32 {
    match cpython_value_from_ptr(object) {
        Ok(Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_)) => 1,
        Ok(_) => 0,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMemoryView_FromObject(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::MemoryView, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetBuffer(
    object: *mut c_void,
    view: *mut CpythonBuffer,
    _flags: i32,
) -> i32 {
    if view.is_null() {
        cpython_set_error("PyObject_GetBuffer received null view");
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(object) else {
            context.set_error("PyObject_GetBuffer received unknown object pointer");
            return -1;
        };
        let info = match context.object_get_buffer_info_v2(handle) {
            Ok(info) => info,
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        };
        // SAFETY: caller passed a valid writable Py_buffer pointer.
        unsafe {
            *view = CpythonBuffer {
                buf: info.data.cast_mut().cast(),
                obj: object,
                len: info.len as isize,
                itemsize: info.itemsize as isize,
                readonly: info.readonly,
                ndim: info.ndim as i32,
                format: info.format.cast_mut(),
                shape: info.shape.cast_mut(),
                strides: info.strides.cast_mut(),
                suboffsets: std::ptr::null_mut(),
                internal: std::ptr::null_mut(),
            };
            Py_XIncRef(object);
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Print(
    object: *mut c_void,
    _file: *mut c_void,
    _flags: i32,
) -> i32 {
    let rendered = unsafe { PyObject_Str(object) };
    if rendered.is_null() {
        return -1;
    }
    let text = match cpython_value_from_ptr(rendered) {
        Ok(Value::Str(text)) => text,
        Ok(other) => format!("{other:?}"),
        Err(_) => "<unprintable>".to_string(),
    };
    println!("{text}");
    unsafe { Py_DecRef(rendered) };
    0
}

unsafe extern "C" fn cpython_type_tp_call(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    let trace_calls = std::env::var_os("PYRS_TRACE_CPY_CALLS").is_some();
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
    }
    let ty = callable.cast::<CpythonTypeObject>();
    // SAFETY: callable points to a PyTypeObject-compatible struct.
    let new_slot = unsafe { (*ty).tp_new };
    if new_slot.is_null() {
        let type_name =
            unsafe { c_name_to_string((*ty).tp_name) }.unwrap_or_else(|_| "<unnamed>".to_string());
        cpython_set_error(format!("TypeError: cannot create '{type_name}' instances"));
        return std::ptr::null_mut();
    }
    if trace_calls {
        // SAFETY: callable points to a PyTypeObject-compatible struct.
        let init_slot = unsafe { (*ty).tp_init };
        eprintln!(
            "[cpy-type-call] callable={:p} tp_new={:p} tp_init={:p} args_ptr={:p} kwargs_ptr={:p}",
            callable, new_slot, init_slot, args, kwargs
        );
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
        eprintln!(
            "[cpy-type-call] tp_new_result object={:p} object_type={:p}",
            object, object_type
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
    if status < 0 {
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
        let callable_type_name =
            unsafe { c_name_to_string((*ty).tp_name) }.unwrap_or_else(|_| "<unnamed>".to_string());
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
            callable_type_name, object_type_name, should_init
        );
    }
    object
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
    if subtype.is_null() || ty.is_null() {
        return 0;
    }
    let target = ty.cast::<CpythonTypeObject>();
    let mut current = subtype.cast::<CpythonTypeObject>();
    while !current.is_null() {
        if current == target {
            return 1;
        }
        // SAFETY: current is checked non-null.
        current = unsafe { (*current).tp_base };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_Ready(ty: *mut c_void) -> i32 {
    if ty.is_null() {
        cpython_set_error("PyType_Ready received null type");
        return -1;
    }
    // SAFETY: caller provided non-null type pointer.
    let ty = ty.cast::<CpythonTypeObject>();
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
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GenericAlloc(subtype: *mut c_void, nitems: isize) -> *mut c_void {
    if subtype.is_null() {
        cpython_set_error("PyType_GenericAlloc received null subtype");
        return std::ptr::null_mut();
    }
    let ty = subtype.cast::<CpythonTypeObject>();
    // SAFETY: subtype is checked non-null.
    let itemsize = unsafe { (*ty).tp_itemsize };
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Malloc(size: usize) -> *mut c_void {
    unsafe { PyMem_Malloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe { PyMem_Realloc(ptr, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Free(ptr: *mut c_void) {
    unsafe { PyMem_Free(ptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_Track(_object: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_UnTrack(_object: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_Del(object: *mut c_void) {
    unsafe { PyObject_Free(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_ClearWeakRefs(_object: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Init(object: *mut c_void, ty: *mut c_void) -> *mut c_void {
    if object.is_null() || ty.is_null() {
        cpython_set_error("PyObject_Init received null object/type");
        return std::ptr::null_mut();
    }
    // SAFETY: caller guarantees object points to writable PyObject-compatible memory.
    unsafe {
        let head = object.cast::<CpythonObjectHead>();
        (*head).ob_refcnt = 1;
        (*head).ob_type = ty;
    }
    object
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_InitVar(
    object: *mut c_void,
    ty: *mut c_void,
    size: isize,
) -> *mut c_void {
    if object.is_null() || ty.is_null() {
        cpython_set_error("PyObject_InitVar received null object/type");
        return std::ptr::null_mut();
    }
    // SAFETY: caller guarantees object points to writable PyVarObject-compatible memory.
    unsafe {
        let head = object.cast::<CpythonVarObjectHead>();
        (*head).ob_base.ob_refcnt = 1;
        (*head).ob_base.ob_type = ty;
        (*head).ob_size = size;
    }
    object
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_New(ty: *mut CpythonTypeObject) -> *mut c_void {
    let basicsize = if ty.is_null() {
        std::mem::size_of::<CpythonObjectHead>()
    } else {
        // SAFETY: caller provided a type pointer.
        let size = unsafe { (*ty).tp_basicsize };
        if size <= 0 {
            std::mem::size_of::<CpythonObjectHead>()
        } else {
            size as usize
        }
    };
    let raw = unsafe { PyObject_Malloc(basicsize) }.cast::<u8>();
    if raw.is_null() {
        unsafe { PyErr_NoMemory() };
        return std::ptr::null_mut();
    }
    // SAFETY: newly allocated buffer has at least basicsize bytes.
    unsafe {
        std::ptr::write_bytes(raw, 0, basicsize);
        let head = raw.cast::<CpythonObjectHead>();
        (*head).ob_refcnt = 1;
        (*head).ob_type = ty.cast();
    }
    raw.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_NewVar(
    ty: *mut CpythonTypeObject,
    nitems: isize,
) -> *mut c_void {
    let base = if ty.is_null() {
        std::mem::size_of::<CpythonVarObjectHead>()
    } else {
        let size = unsafe { (*ty).tp_basicsize };
        if size <= 0 {
            std::mem::size_of::<CpythonVarObjectHead>()
        } else {
            size as usize
        }
    };
    let item_size = if ty.is_null() {
        0usize
    } else {
        unsafe { (*ty).tp_itemsize.max(0) as usize }
    };
    let extra = if nitems <= 0 {
        0usize
    } else {
        item_size.saturating_mul(nitems as usize)
    };
    let total = base.saturating_add(extra);
    let raw = unsafe { PyObject_Malloc(total) }.cast::<u8>();
    if raw.is_null() {
        unsafe { PyErr_NoMemory() };
        return std::ptr::null_mut();
    }
    // SAFETY: newly allocated buffer has at least total bytes.
    unsafe {
        std::ptr::write_bytes(raw, 0, total);
        let head = raw.cast::<CpythonVarObjectHead>();
        (*head).ob_base.ob_refcnt = 1;
        (*head).ob_base.ob_type = ty.cast();
        (*head).ob_size = nitems;
    }
    raw.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_GC_New(ty: *mut CpythonTypeObject) -> *mut c_void {
    unsafe { _PyObject_New(ty) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_Dealloc(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    unsafe { PyObject_Free(object) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyErr_BadInternalCall(_filename: *const c_char, _lineno: i32) {
    cpython_set_error("bad internal call");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_HashDouble(_inst: *mut c_void, value: f64) -> isize {
    if value.is_nan() {
        return 0;
    }
    let bits = value.to_bits() as i64;
    bits as isize
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsWhitespace(ch: u32) -> i32 {
    char::from_u32(ch)
        .map(|value| i32::from(value.is_whitespace()))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsAlpha(ch: u32) -> i32 {
    char::from_u32(ch)
        .map(|value| i32::from(value.is_alphabetic()))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsDecimalDigit(ch: u32) -> i32 {
    char::from_u32(ch)
        .map(|value| i32::from(value.is_ascii_digit()))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsDigit(ch: u32) -> i32 {
    unsafe { _PyUnicode_IsDecimalDigit(ch) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsNumeric(ch: u32) -> i32 {
    unsafe { _PyUnicode_IsDecimalDigit(ch) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsLowercase(ch: u32) -> i32 {
    char::from_u32(ch)
        .map(|value| i32::from(value.is_lowercase()))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsUppercase(ch: u32) -> i32 {
    char::from_u32(ch)
        .map(|value| i32::from(value.is_uppercase()))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsTitlecase(ch: u32) -> i32 {
    // Rust stdlib does not expose titlecase directly; use uppercase heuristic.
    unsafe { _PyUnicode_IsUppercase(ch) }
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_Get() -> *mut c_void {
    1usize as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceMalloc_Track(_domain: usize, _ptr: usize, _size: usize) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceMalloc_Untrack(_domain: usize, _ptr: usize) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_EnterRecursiveCall(_where: *const c_char) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_LeaveRecursiveCall() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IsInitialized() -> i32 {
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetString(_exception: *mut c_void, message: *const c_char) {
    match unsafe { c_name_to_string(message) } {
        Ok(message) => cpython_set_error(message),
        Err(err) => cpython_set_error(format!("PyErr_SetString invalid message: {err}")),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Occurred() -> *mut c_void {
    match with_active_cpython_context_mut(|context| context.last_error.is_some()) {
        Ok(true) => 1usize as *mut c_void,
        Ok(false) => std::ptr::null_mut(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Clear() {
    let _ = with_active_cpython_context_mut(|context| {
        context.clear_error();
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_ExceptionMatches(_exception: *mut c_void) -> i32 {
    if unsafe { PyErr_Occurred() }.is_null() {
        0
    } else {
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_GivenExceptionMatches(
    given: *mut c_void,
    expected: *mut c_void,
) -> i32 {
    if given.is_null() || expected.is_null() {
        return 0;
    }
    if given == expected { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Fetch(
    ptype: *mut *mut c_void,
    pvalue: *mut *mut c_void,
    ptraceback: *mut *mut c_void,
) {
    if !ptype.is_null() {
        unsafe { *ptype = std::ptr::null_mut() };
    }
    if !pvalue.is_null() {
        unsafe { *pvalue = std::ptr::null_mut() };
    }
    if !ptraceback.is_null() {
        unsafe { *ptraceback = std::ptr::null_mut() };
    }
    unsafe { PyErr_Clear() };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Restore(
    ptype: *mut c_void,
    pvalue: *mut c_void,
    _ptraceback: *mut c_void,
) {
    if pvalue.is_null() {
        if ptype.is_null() {
            unsafe { PyErr_Clear() };
        } else {
            cpython_set_error("PyErr_Restore called with null value");
        }
        return;
    }
    unsafe { PyErr_SetObject(ptype, pvalue) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_Format(
    _exception: *mut c_void,
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
    cpython_set_error(message);
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_FormatV(
    exception: *mut c_void,
    format: *const c_char,
    _vargs: *mut c_void,
) -> *mut c_void {
    unsafe { PyErr_Format(exception, format) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_NormalizeException(
    _ptype: *mut *mut c_void,
    _pvalue: *mut *mut c_void,
    _ptraceback: *mut *mut c_void,
) {
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrno(exception: *mut c_void) -> *mut c_void {
    unsafe { PyErr_SetString(exception, c"system error".as_ptr()) };
    std::ptr::null_mut()
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
pub unsafe extern "C" fn PyErr_WarnFormat(
    category: *mut c_void,
    stacklevel: isize,
    format: *const c_char,
) -> i32 {
    unsafe { PyErr_WarnEx(category, format, stacklevel) }
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
pub unsafe extern "C" fn PyException_SetCause(_exception: *mut c_void, _cause: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_SetContext(_exception: *mut c_void, _context: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyException_SetTraceback(
    _exception: *mut c_void,
    _traceback: *mut c_void,
) {
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
    match cpython_value_from_ptr(value) {
        Ok(Value::Str(message)) => cpython_set_error(message),
        Ok(other) => cpython_set_error(format!("{other:?}")),
        Err(err) => cpython_set_error(err),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetNone(exception: *mut c_void) {
    let _ = exception;
    cpython_set_error("error");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_NoMemory() -> *mut c_void {
    cpython_set_error("out of memory");
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
pub unsafe extern "C" fn PyGILState_Release(_state: i32) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_SaveThread() -> *mut c_void {
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_RestoreThread(_state: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyInterpreterState_Main() -> *mut c_void {
    1usize as *mut c_void
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
pub unsafe extern "C" fn PyMem_RawMalloc(size: usize) -> *mut c_void {
    // SAFETY: forwarded directly to C allocator.
    unsafe { malloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawCalloc(count: usize, size: usize) -> *mut c_void {
    // SAFETY: forwarded directly to C allocator.
    unsafe { calloc(count, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawRealloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    // SAFETY: forwarded directly to C allocator.
    unsafe { realloc(ptr, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawFree(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    if let Ok(true) =
        with_active_cpython_context_mut(|context| context.owns_cpython_allocation_ptr(ptr))
    {
        if std::env::var_os("PYRS_TRACE_CPY_PTRS").is_some() {
            eprintln!("[cpy-ptr] suppress free for compat ptr={:p}", ptr);
        }
        return;
    }
    // SAFETY: forwarded directly to C allocator.
    unsafe { free(ptr) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Malloc(size: usize) -> *mut c_void {
    unsafe { PyMem_RawMalloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Calloc(count: usize, size: usize) -> *mut c_void {
    unsafe { PyMem_RawCalloc(count, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe { PyMem_RawRealloc(ptr, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Free(ptr: *mut c_void) {
    unsafe { PyMem_RawFree(ptr) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IncRef(object: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        if let Some(handle) = context.cpython_handle_from_ptr(object) {
            let _ = context.incref(handle);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_DecRef(object: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        if let Some(handle) = context.cpython_handle_from_ptr(object) {
            let _ = context.decref(handle);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_XIncRef(object: *mut c_void) {
    if !object.is_null() {
        unsafe { Py_IncRef(object) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_XDecRef(object: *mut c_void) {
    if !object.is_null() {
        unsafe { Py_DecRef(object) };
    }
}

#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_Exception: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_ImportError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_RuntimeError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_TypeError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_ValueError: *mut c_void = std::ptr::null_mut();

#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_AttributeError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_BufferError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_DeprecationWarning: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_FloatingPointError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_FutureWarning: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_IOError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_ImportWarning: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_IndexError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_KeyError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_MemoryError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_NameError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_NotImplementedError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_OSError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_OverflowError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_RecursionError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_RuntimeWarning: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_SystemError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_UnicodeDecodeError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_UnicodeEncodeError: *mut c_void = std::ptr::null_mut();
#[unsafe(no_mangle)]
#[used]
pub static mut PyExc_UserWarning: *mut c_void = std::ptr::null_mut();

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
    ob_refcnt: 1,
    ob_type: std::ptr::null_mut(),
};
#[unsafe(no_mangle)]
#[used]
pub static mut _Py_NotImplementedStruct: CpythonObjectHead = CpythonObjectHead {
    ob_refcnt: 1,
    ob_type: std::ptr::null_mut(),
};
#[unsafe(no_mangle)]
#[used]
pub static mut _Py_EllipsisObject: CpythonObjectHead = CpythonObjectHead {
    ob_refcnt: 1,
    ob_type: std::ptr::null_mut(),
};
#[unsafe(no_mangle)]
#[used]
pub static mut _Py_FalseStruct: CpythonObjectHead = CpythonObjectHead {
    ob_refcnt: 1,
    ob_type: std::ptr::null_mut(),
};
#[unsafe(no_mangle)]
#[used]
pub static mut _Py_TrueStruct: CpythonObjectHead = CpythonObjectHead {
    ob_refcnt: 1,
    ob_type: std::ptr::null_mut(),
};

const EMPTY_TYPE_FLAGS: usize = 0;
const PY_TPFLAGS_BASETYPE: usize = 1usize << 10;
const PY_TPFLAGS_READY: usize = 1usize << 12;
const PY_TPFLAGS_LONG_SUBCLASS: usize = 1usize << 24;
const PY_TPFLAGS_LIST_SUBCLASS: usize = 1usize << 25;
const PY_TPFLAGS_TUPLE_SUBCLASS: usize = 1usize << 26;
const PY_TPFLAGS_BYTES_SUBCLASS: usize = 1usize << 27;
const PY_TPFLAGS_UNICODE_SUBCLASS: usize = 1usize << 28;
const PY_TPFLAGS_DICT_SUBCLASS: usize = 1usize << 29;
const PY_TPFLAGS_TYPE_SUBCLASS: usize = 1usize << 31;

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
static PY_TYPE_NAME_BYTES: &[u8; 6] = b"bytes\0";
static PY_TYPE_NAME_CFUNCTION: &[u8; 27] = b"builtin_function_or_method\0";
static PY_TYPE_NAME_CAPSULE: &[u8; 8] = b"capsule\0";
static PY_TYPE_NAME_COMPLEX: &[u8; 8] = b"complex\0";
static PY_TYPE_NAME_DICT_PROXY: &[u8; 10] = b"dictproxy\0";
static PY_TYPE_NAME_DICT: &[u8; 5] = b"dict\0";
static PY_TYPE_NAME_FLOAT: &[u8; 6] = b"float\0";
static PY_TYPE_NAME_FROZENSET: &[u8; 10] = b"frozenset\0";
static PY_TYPE_NAME_GETSET_DESCR: &[u8; 13] = b"getset_descr\0";
static PY_TYPE_NAME_LIST: &[u8; 5] = b"list\0";
static PY_TYPE_NAME_LONG: &[u8; 4] = b"int\0";
static PY_TYPE_NAME_MEMBER_DESCR: &[u8; 13] = b"member_descr\0";
static PY_TYPE_NAME_MEMORYVIEW: &[u8; 11] = b"memoryview\0";
static PY_TYPE_NAME_METHOD_DESCR: &[u8; 13] = b"method_descr\0";
static PY_TYPE_NAME_NONE: &[u8; 9] = b"NoneType\0";
static PY_TYPE_NAME_SET: &[u8; 4] = b"set\0";
static PY_TYPE_NAME_SLICE: &[u8; 6] = b"slice\0";
static PY_TYPE_NAME_TUPLE: &[u8; 6] = b"tuple\0";
static PY_TYPE_NAME_UNICODE: &[u8; 4] = b"str\0";

#[unsafe(no_mangle)]
#[used]
pub static mut PyBaseObject_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_OBJECT.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyObject_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_OBJECT.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyType_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_TYPE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyBool_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_BOOL.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyBytes_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_BYTES.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyCFunction_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_CFUNCTION.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyCapsule_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_CAPSULE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyComplex_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_COMPLEX.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictProxy_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_PROXY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDict_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_DICT.as_ptr().cast());
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
pub static mut PyList_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_LIST.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyLong_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_LONG.as_ptr().cast());
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
pub static mut PyMethodDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_METHOD_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyNone_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_NONE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySet_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_SET.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySlice_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_SLICE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyTuple_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_TUPLE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyUnicode_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_UNICODE.as_ptr().cast());

#[used]
static KEEP2_PYLONG_FROMSSIZE_T: unsafe extern "C" fn(isize) -> *mut c_void = PyLong_FromSsize_t;
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
static KEEP2_PYMODULE_GETDICT: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyModule_GetDict;
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
static KEEP2_PYTUPLE_PACK: unsafe extern "C" fn(isize) -> *mut c_void = PyTuple_Pack;
#[used]
static KEEP2_PYOBJECT_GETITEM: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyObject_GetItem;
#[used]
static KEEP2_PYOBJECT_SETITEM: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
    PyObject_SetItem;
#[used]
static KEEP2_PYOBJECT_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PyObject_Size;
#[used]
static KEEP2_PYOBJECT_LENGTHHINT: unsafe extern "C" fn(*mut c_void, isize) -> isize =
    PyObject_LengthHint;
#[used]
static KEEP2_PYOBJECT_HASH: unsafe extern "C" fn(*mut c_void) -> isize = PyObject_Hash;
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
static KEEP2_PYSEQUENCE_GETITEM: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    PySequence_GetItem;
#[used]
static KEEP2_PYSEQUENCE_CONTAINS: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PySequence_Contains;
#[used]
static KEEP2_PYSEQUENCE_TUPLE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PySequence_Tuple;
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
static KEEP2_PYMAPPING_GETITEMSTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyMapping_GetItemString;
#[used]
static KEEP2_PYSEQITER_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PySeqIter_New;
#[used]
static KEEP2_PYOBJECT_ASFILEDESCRIPTOR: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyObject_AsFileDescriptor;
#[used]
static KEEP2_PYOBJECT_CHECKBUFFER: unsafe extern "C" fn(*mut c_void) -> i32 = PyObject_CheckBuffer;
#[used]
static KEEP2_PYMEMORYVIEW_FROMOBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyMemoryView_FromObject;
#[used]
static KEEP2_PYOBJECT_GETBUFFER: unsafe extern "C" fn(*mut c_void, *mut CpythonBuffer, i32) -> i32 =
    PyObject_GetBuffer;
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
static KEEP2_PYOBJECT_MALLOC: unsafe extern "C" fn(usize) -> *mut c_void = PyObject_Malloc;
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
static KEEP2_PYOBJECT_GC_DEL: unsafe extern "C" fn(*mut c_void) = PyObject_GC_Del;
#[used]
static KEEP2_PYOBJECT_CLEAR_WEAKREFS: unsafe extern "C" fn(*mut c_void) = PyObject_ClearWeakRefs;
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
static KEEP2_PYSYS_GETOBJECT: unsafe extern "C" fn(*const c_char) -> *mut c_void = PySys_GetObject;
#[used]
static KEEP2_PYTHREADSTATE_GET: unsafe extern "C" fn() -> *mut c_void = PyThreadState_Get;
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
static KEEP3_PYOBJECT_CALLFUNCTION: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
) -> *mut c_void = PyObject_CallFunction;
#[used]
static KEEP3_PYOBJECT_CALLFUNCTIONOBJARGS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyObject_CallFunctionObjArgs;
#[used]
static KEEP3_PYOBJECT_CALLMETHOD: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyObject_CallMethod;
#[used]
static KEEP3_PYARG_PARSETUPLE: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyArg_ParseTuple;
#[used]
static KEEP3_PYARG_PARSETUPLEANDKEYWORDS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
) -> i32 = PyArg_ParseTupleAndKeywords;
#[used]
static KEEP3_PYARG_VAPARSETUPLEANDKEYWORDS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
    *mut c_void,
) -> i32 = PyArg_VaParseTupleAndKeywords;
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
static KEEP3_PYERR_EXCEPTIONMATCHES: unsafe extern "C" fn(*mut c_void) -> i32 =
    PyErr_ExceptionMatches;
#[used]
static KEEP3_PYERR_FETCH: unsafe extern "C" fn(
    *mut *mut c_void,
    *mut *mut c_void,
    *mut *mut c_void,
) = PyErr_Fetch;
#[used]
static KEEP3_PYERR_FORMAT: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
    PyErr_Format;
#[used]
static KEEP3_PYERR_FORMATV: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
) -> *mut c_void = PyErr_FormatV;
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
static KEEP3_PYERR_WRITEUNRAISABLE: unsafe extern "C" fn(*mut c_void) = PyErr_WriteUnraisable;
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
static KEEP3_PYUNICODE_ASENCODEDSTRING: unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
) -> *mut c_void = PyUnicode_AsEncodedString;
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
static KEEP3_PYUNICODE_FROMFORMAT: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyUnicode_FromFormat;
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
static KEEP_PYERR_SET_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) = PyErr_SetString;
#[used]
static KEEP_PYERR_OCCURRED: unsafe extern "C" fn() -> *mut c_void = PyErr_Occurred;
#[used]
static KEEP_PYERR_CLEAR: unsafe extern "C" fn() = PyErr_Clear;
#[used]
static KEEP_PY_INCREF: unsafe extern "C" fn(*mut c_void) = Py_IncRef;
#[used]
static KEEP_PY_DECREF: unsafe extern "C" fn(*mut c_void) = Py_DecRef;
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
static KEEP_PYBUFFER_RELEASE: unsafe extern "C" fn(*mut c_void) = PyBuffer_Release;
#[used]
static KEEP_PYCALLABLE_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = PyCallable_Check;
#[used]
static KEEP_PYINDEX_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = PyIndex_Check;
#[used]
static KEEP_PYFLOAT_AS_DOUBLE: unsafe extern "C" fn(*mut c_void) -> f64 = PyFloat_AsDouble;
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
static KEEP_PYLONG_AS_SSIZE_T: unsafe extern "C" fn(*mut c_void) -> isize = PyLong_AsSsize_t;
#[used]
static KEEP_PYLONG_AS_UNSIGNED_LONG: unsafe extern "C" fn(*mut c_void) -> u64 =
    PyLong_AsUnsignedLong;
#[used]
static KEEP_PYLONG_AS_UNSIGNED_LONGLONG: unsafe extern "C" fn(*mut c_void) -> u64 =
    PyLong_AsUnsignedLongLong;
#[used]
static KEEP_PYLONG_AS_VOID_PTR: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyLong_AsVoidPtr;
#[used]
static KEEP_PYLONG_AS_LONG_AND_OVERFLOW: unsafe extern "C" fn(*mut c_void, *mut i32) -> i64 =
    PyLong_AsLongAndOverflow;
#[used]
static KEEP_PYLONG_AS_LONGLONG_AND_OVERFLOW: unsafe extern "C" fn(*mut c_void, *mut i32) -> i64 =
    PyLong_AsLongLongAndOverflow;
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
static KEEP_PYIMPORT_IMPORT_MODULE: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyImport_ImportModule;
#[used]
static KEEP_PYIMPORT_IMPORT: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyImport_Import;
#[used]
static KEEP_PYEVAL_GET_BUILTINS: unsafe extern "C" fn() -> *mut c_void = PyEval_GetBuiltins;
#[used]
static KEEP_PYITER_CHECK: unsafe extern "C" fn(*mut c_void) -> i32 = PyIter_Check;
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
static KEEP_PYLIST_GET_ITEM_REF: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
    PyList_GetItemRef;
#[used]
static KEEP_PYLIST_AS_TUPLE: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyList_AsTuple;
#[used]
static KEEP_PYDICT_NEW: unsafe extern "C" fn() -> *mut c_void = PyDict_New;
#[used]
static KEEP_PYDICT_SIZE: unsafe extern "C" fn(*mut c_void) -> isize = PyDict_Size;
#[used]
static KEEP_PYDICT_SET_ITEM: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
    PyDict_SetItem;
#[used]
static KEEP_PYDICT_GET_ITEM: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyDict_GetItem;
#[used]
static KEEP_PYDICT_GET_ITEM_WITH_ERROR: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
) -> *mut c_void = PyDict_GetItemWithError;
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
static KEEP_PYDICT_MERGE: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32 = PyDict_Merge;
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
static KEEP_PYOBJECT_HASATTR_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyObject_HasAttrString;
#[used]
static KEEP_PYOBJECT_ISTRUE: unsafe extern "C" fn(*mut c_void) -> i32 = PyObject_IsTrue;
#[used]
static KEEP_PYOBJECT_NOT: unsafe extern "C" fn(*mut c_void) -> i32 = PyObject_Not;
#[used]
static KEEP_PYOBJECT_STR: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_Str;
#[used]
static KEEP_PYOBJECT_BYTES: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_Bytes;
#[used]
static KEEP_PYOBJECT_FORMAT: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
    PyObject_Format;
#[used]
static KEEP_PYOBJECT_GETITER: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PyObject_GetIter;
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
static KEEP_PYGILSTATE_RELEASE: unsafe extern "C" fn(i32) = PyGILState_Release;
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
static KEEP_PYMEM_RAW_MALLOC: unsafe extern "C" fn(usize) -> *mut c_void = PyMem_RawMalloc;
#[used]
static KEEP_PYMEM_RAW_CALLOC: unsafe extern "C" fn(usize, usize) -> *mut c_void = PyMem_RawCalloc;
#[used]
static KEEP_PYMEM_RAW_REALLOC: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void =
    PyMem_RawRealloc;
#[used]
static KEEP_PYMEM_RAW_FREE: unsafe extern "C" fn(*mut c_void) = PyMem_RawFree;
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

unsafe extern "C" fn capi_api_has_capability(module_ctx: *mut c_void, name: *const c_char) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    let supported = matches!(
        name.as_str(),
        "module_add_function"
            | "module_add_function_kw"
            | "module_get_object"
            | "module_import"
            | "module_get_attr"
            | "module_set_state"
            | "module_get_state"
            | "module_set_finalize"
            | "module_set_attr"
            | "module_del_attr"
            | "module_has_attr"
            | "object_new_none"
            | "object_new_float"
            | "object_new_bytes"
            | "object_new_bytearray"
            | "object_new_memoryview"
            | "object_new_tuple"
            | "object_new_list"
            | "object_new_dict"
            | "object_len"
            | "object_get_item"
            | "object_set_item"
            | "object_del_item"
            | "object_contains"
            | "object_dict_keys"
            | "object_dict_items"
            | "object_get_buffer"
            | "object_get_writable_buffer"
            | "object_get_buffer_info"
            | "object_get_buffer_info_v2"
            | "object_release_buffer"
            | "capsule_new"
            | "capsule_get_pointer"
            | "capsule_set_pointer"
            | "capsule_get_name"
            | "capsule_set_context"
            | "capsule_get_context"
            | "capsule_set_destructor"
            | "capsule_get_destructor"
            | "capsule_set_name"
            | "capsule_is_valid"
            | "capsule_export"
            | "capsule_import"
            | "object_sequence_len"
            | "object_sequence_get_item"
            | "object_get_iter"
            | "object_iter_next"
            | "object_list_append"
            | "object_list_set_item"
            | "object_dict_len"
            | "object_dict_set_item"
            | "object_dict_get_item"
            | "object_dict_contains"
            | "object_dict_del_item"
            | "object_get_attr"
            | "object_set_attr"
            | "object_del_attr"
            | "object_has_attr"
            | "object_is_instance"
            | "object_is_subclass"
            | "object_call_noargs"
            | "object_call_onearg"
            | "object_call"
            | "error_get_message"
            | "error_state"
            | "extension_symbol_metadata"
    );
    if supported { 1 } else { 0 }
}

unsafe extern "C" fn capi_module_set_int(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: i64,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, Value::Int(value)) }
}

unsafe extern "C" fn capi_module_set_bool(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: i32,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, Value::Bool(value != 0)) }
}

unsafe extern "C" fn capi_module_set_string(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let value = match unsafe { c_name_to_string(value) } {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    unsafe { capi_module_insert_value(context, name, Value::Str(value)) }
}

unsafe extern "C" fn capi_module_add_function(
    module_ctx: *mut c_void,
    name: *const c_char,
    callback: Option<PyrsCFunctionV1>,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let Some(callback) = callback else {
        context.set_error("module_add_function requires a non-null callback");
        return -1;
    };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    if context.vm.is_null() {
        context.set_error("module_add_function missing VM context");
        return -1;
    }
    // SAFETY: VM pointer is set by `exec_extension_module` and valid during init callback.
    let vm = unsafe { &mut *context.vm };
    let callable = match vm.register_extension_callable(
        context.module.clone(),
        &name,
        ExtensionCallableKind::Positional(callback),
    ) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err.message);
            return -1;
        }
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        context.set_error("module context no longer points to a module");
        return -1;
    };
    module_data.globals.insert(name, callable);
    0
}

unsafe extern "C" fn capi_module_add_function_kw(
    module_ctx: *mut c_void,
    name: *const c_char,
    callback: Option<PyrsCFunctionKwV1>,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let Some(callback) = callback else {
        context.set_error("module_add_function_kw requires a non-null callback");
        return -1;
    };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    if context.vm.is_null() {
        context.set_error("module_add_function_kw missing VM context");
        return -1;
    }
    // SAFETY: VM pointer is set by `exec_extension_module` and valid during init callback.
    let vm = unsafe { &mut *context.vm };
    let callable = match vm.register_extension_callable(
        context.module.clone(),
        &name,
        ExtensionCallableKind::WithKeywords(callback),
    ) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err.message);
            return -1;
        }
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        context.set_error("module context no longer points to a module");
        return -1;
    };
    module_data.globals.insert(name, callable);
    0
}

unsafe extern "C" fn capi_object_new_int(module_ctx: *mut c_void, value: i64) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Int(value))
}

unsafe extern "C" fn capi_object_new_none(module_ctx: *mut c_void) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::None)
}

unsafe extern "C" fn capi_object_new_bool(module_ctx: *mut c_void, value: i32) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Bool(value != 0))
}

unsafe extern "C" fn capi_object_new_float(
    module_ctx: *mut c_void,
    value: f64,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Float(value))
}

unsafe extern "C" fn capi_object_new_bytes(
    module_ctx: *mut c_void,
    data: *const u8,
    len: usize,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if data.is_null() && len != 0 {
        context.set_error("object_new_bytes received null data pointer with non-zero len");
        return 0;
    }
    let bytes = if len == 0 {
        Vec::new()
    } else {
        // SAFETY: caller-provided pointer/len pair is assumed valid for read.
        unsafe { std::slice::from_raw_parts(data, len) }.to_vec()
    };
    if context.vm.is_null() {
        context.set_error("object_new_bytes missing VM context");
        return 0;
    }
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_bytes(bytes))
}

unsafe extern "C" fn capi_object_new_bytearray(
    module_ctx: *mut c_void,
    data: *const u8,
    len: usize,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if data.is_null() && len != 0 {
        context.set_error("object_new_bytearray received null data pointer with non-zero len");
        return 0;
    }
    let bytes = if len == 0 {
        Vec::new()
    } else {
        // SAFETY: caller-provided pointer/len pair is assumed valid for read.
        unsafe { std::slice::from_raw_parts(data, len) }.to_vec()
    };
    if context.vm.is_null() {
        context.set_error("object_new_bytearray missing VM context");
        return 0;
    }
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_bytearray(bytes))
}

unsafe extern "C" fn capi_object_new_memoryview(
    module_ctx: *mut c_void,
    source_handle: PyrsObjectHandle,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if context.vm.is_null() {
        context.set_error("object_new_memoryview missing VM context");
        return 0;
    }
    let Some(source_value) = context.object_value(source_handle) else {
        context.set_error(format!("invalid object handle {}", source_handle));
        return 0;
    };
    let source = match source_value {
        Value::Bytes(obj) | Value::ByteArray(obj) => obj,
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => view.source.clone(),
            _ => {
                context.set_error(format!(
                    "object handle {} has invalid memoryview storage",
                    source_handle
                ));
                return 0;
            }
        },
        _ => {
            context.set_error(format!(
                "object handle {} does not support memoryview construction",
                source_handle
            ));
            return 0;
        }
    };
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_memoryview(source))
}

unsafe extern "C" fn capi_object_new_tuple(
    module_ctx: *mut c_void,
    len: usize,
    items: *const PyrsObjectHandle,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if len != 0 && items.is_null() {
        context.set_error("object_new_tuple received null items pointer with non-zero len");
        return 0;
    }
    if context.vm.is_null() {
        context.set_error("object_new_tuple missing VM context");
        return 0;
    }
    let mut values = Vec::with_capacity(len);
    for idx in 0..len {
        // SAFETY: caller-provided pointer/len pair is assumed valid for read.
        let handle = unsafe { *items.add(idx) };
        let Some(value) = context.object_value(handle) else {
            context.set_error(format!(
                "object_new_tuple received invalid item handle {} at index {}",
                handle, idx
            ));
            return 0;
        };
        values.push(value);
    }
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_tuple(values))
}

unsafe extern "C" fn capi_object_new_list(
    module_ctx: *mut c_void,
    len: usize,
    items: *const PyrsObjectHandle,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if len != 0 && items.is_null() {
        context.set_error("object_new_list received null items pointer with non-zero len");
        return 0;
    }
    if context.vm.is_null() {
        context.set_error("object_new_list missing VM context");
        return 0;
    }
    let mut values = Vec::with_capacity(len);
    for idx in 0..len {
        // SAFETY: caller-provided pointer/len pair is assumed valid for read.
        let handle = unsafe { *items.add(idx) };
        let Some(value) = context.object_value(handle) else {
            context.set_error(format!(
                "object_new_list received invalid item handle {} at index {}",
                handle, idx
            ));
            return 0;
        };
        values.push(value);
    }
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_list(values))
}

unsafe extern "C" fn capi_object_new_dict(module_ctx: *mut c_void) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    if context.vm.is_null() {
        context.set_error("object_new_dict missing VM context");
        return 0;
    }
    // SAFETY: VM pointer is set by extension entrypoint dispatch and valid here.
    let vm = unsafe { &mut *context.vm };
    context.alloc_object(vm.heap.alloc_dict(Vec::new()))
}

unsafe extern "C" fn capi_object_new_string(
    module_ctx: *mut c_void,
    value: *const c_char,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    let value = match unsafe { c_name_to_string(value) } {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            return 0;
        }
    };
    context.alloc_object(Value::Str(value))
}

unsafe extern "C" fn capi_object_incref(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.incref(handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_decref(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.decref(handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_module_set_object(
    module_ctx: *mut c_void,
    name: *const c_char,
    handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let Some(value) = context.object_value(handle) else {
        context.set_error(format!("invalid object handle {}", handle));
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, value) }
}

unsafe extern "C" fn capi_module_get_object(
    module_ctx: *mut c_void,
    name: *const c_char,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("module_get_object received null output pointer");
        return -1;
    }
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_get_object(&name) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_module_import(
    module_ctx: *mut c_void,
    module_name: *const c_char,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("module_import received null output pointer");
        return -1;
    }
    let module_name = match unsafe { c_name_to_string(module_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_import(&module_name) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_module_get_attr(
    module_ctx: *mut c_void,
    module_handle: PyrsObjectHandle,
    attr_name: *const c_char,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("module_get_attr received null output pointer");
        return -1;
    }
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_get_attr(module_handle, &attr_name) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_module_set_state(
    module_ctx: *mut c_void,
    state: *mut c_void,
    free_func: Option<PyrsModuleStateFreeV1>,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.module_set_state(state, free_func) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_module_get_state(module_ctx: *mut c_void) -> *mut c_void {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null_mut();
    };
    match context.module_get_state() {
        Ok(state) => state,
        Err(err) => {
            context.set_error(err);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn capi_module_set_finalize(
    module_ctx: *mut c_void,
    finalize_func: Option<PyrsModuleStateFinalizeV1>,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.module_set_finalize(finalize_func) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_module_set_attr(
    module_ctx: *mut c_void,
    module_handle: PyrsObjectHandle,
    attr_name: *const c_char,
    value_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_set_attr(module_handle, &attr_name, value_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_module_del_attr(
    module_ctx: *mut c_void,
    module_handle: PyrsObjectHandle,
    attr_name: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_del_attr(module_handle, &attr_name) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_module_has_attr(
    module_ctx: *mut c_void,
    module_handle: PyrsObjectHandle,
    attr_name: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.module_has_attr(module_handle, &attr_name) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_type(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    match context.object_type(handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            0
        }
    }
}

unsafe extern "C" fn capi_object_is_instance(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    classinfo_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_is_instance(object_handle, classinfo_handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_is_subclass(
    module_ctx: *mut c_void,
    class_handle: PyrsObjectHandle,
    classinfo_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_is_subclass(class_handle, classinfo_handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_int(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out: *mut i64,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out.is_null() {
        context.set_error("object_get_int received null out pointer");
        return -1;
    }
    match context.object_get_int(handle) {
        Ok(value) => {
            // SAFETY: caller provided non-null out pointer.
            unsafe {
                *out = value;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_float(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out: *mut f64,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out.is_null() {
        context.set_error("object_get_float received null out pointer");
        return -1;
    }
    match context.object_get_float(handle) {
        Ok(value) => {
            // SAFETY: caller provided non-null out pointer.
            unsafe {
                *out = value;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_bool(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out: *mut i32,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out.is_null() {
        context.set_error("object_get_bool received null out pointer");
        return -1;
    }
    match context.object_get_bool(handle) {
        Ok(value) => {
            // SAFETY: caller provided non-null out pointer.
            unsafe {
                *out = value;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_bytes(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out_data: *mut *const u8,
    out_len: *mut usize,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_data.is_null() || out_len.is_null() {
        context.set_error("object_get_bytes received null output pointer");
        return -1;
    }
    match context.object_get_bytes_parts(handle) {
        Ok((data_ptr, len)) => {
            // SAFETY: caller provided non-null out pointers.
            unsafe {
                *out_data = data_ptr;
                *out_len = len;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_len(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out_len: *mut usize,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_len.is_null() {
        context.set_error("object_len received null output pointer");
        return -1;
    }
    match context.object_len(handle) {
        Ok(len) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_len = len;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_item(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_get_item received null output pointer");
        return -1;
    }
    match context.object_get_item(object_handle, key_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_set_item(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
    value_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_set_item(object_handle, key_handle, value_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_del_item(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_del_item(object_handle, key_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_contains(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    needle_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_contains(object_handle, needle_handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_dict_keys(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_dict_keys received null output pointer");
        return -1;
    }
    match context.object_dict_keys(dict_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_dict_items(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_dict_items received null output pointer");
        return -1;
    }
    match context.object_dict_items(dict_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_buffer(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    out_view: *mut PyrsBufferViewV1,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_view.is_null() {
        context.set_error("object_get_buffer received null output pointer");
        return -1;
    }
    match context.object_get_buffer(object_handle) {
        Ok(view) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_view = view;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_writable_buffer(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    out_view: *mut PyrsWritableBufferViewV1,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_view.is_null() {
        context.set_error("object_get_writable_buffer received null output pointer");
        return -1;
    }
    match context.object_get_writable_buffer(object_handle) {
        Ok(view) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_view = view;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_buffer_info(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    out_info: *mut PyrsBufferInfoV1,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_info.is_null() {
        context.set_error("object_get_buffer_info received null output pointer");
        return -1;
    }
    match context.object_get_buffer_info(object_handle) {
        Ok(info) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_info = info;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_buffer_info_v2(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    out_info: *mut PyrsBufferInfoV2,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_info.is_null() {
        context.set_error("object_get_buffer_info_v2 received null output pointer");
        return -1;
    }
    match context.object_get_buffer_info_v2(object_handle) {
        Ok(info) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_info = info;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_release_buffer(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_release_buffer(object_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_capsule_new(
    module_ctx: *mut c_void,
    pointer: *mut c_void,
    name: *const c_char,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    match context.capsule_new(pointer, name) {
        Ok(handle) => handle,
        Err(err) => {
            context.set_error(err);
            0
        }
    }
}

unsafe extern "C" fn capi_capsule_get_pointer(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    name: *const c_char,
) -> *mut c_void {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null_mut();
    };
    match context.capsule_get_pointer(capsule_handle, name) {
        Ok(ptr) => ptr,
        Err(err) => {
            context.set_error(err);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn capi_capsule_set_pointer(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    pointer: *mut c_void,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.capsule_set_pointer(capsule_handle, pointer) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_capsule_get_name(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
) -> *const c_char {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null();
    };
    match context.capsule_get_name_ptr(capsule_handle) {
        Ok(name_ptr) => name_ptr,
        Err(err) => {
            context.set_error(err);
            std::ptr::null()
        }
    }
}

unsafe extern "C" fn capi_capsule_set_context(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    context: *mut c_void,
) -> i32 {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context_obj.capsule_set_context(capsule_handle, context) {
        Ok(()) => 0,
        Err(err) => {
            context_obj.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_capsule_get_context(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
) -> *mut c_void {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null_mut();
    };
    match context_obj.capsule_get_context(capsule_handle) {
        Ok(ptr) => ptr,
        Err(err) => {
            context_obj.set_error(err);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn capi_capsule_set_destructor(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    destructor: Option<PyrsCapsuleDestructorV1>,
) -> i32 {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context_obj.capsule_set_destructor(capsule_handle, destructor) {
        Ok(()) => 0,
        Err(err) => {
            context_obj.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_capsule_get_destructor(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
) -> Option<PyrsCapsuleDestructorV1> {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return None;
    };
    match context_obj.capsule_get_destructor(capsule_handle) {
        Ok(destructor) => destructor,
        Err(err) => {
            context_obj.set_error(err);
            None
        }
    }
}

unsafe extern "C" fn capi_capsule_set_name(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    name: *const c_char,
) -> i32 {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context_obj.capsule_set_name(capsule_handle, name) {
        Ok(()) => 0,
        Err(err) => {
            context_obj.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_capsule_is_valid(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
    name: *const c_char,
) -> i32 {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context_obj.capsule_is_valid(capsule_handle, name) {
        Ok(value) => value,
        Err(err) => {
            context_obj.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_capsule_export(
    module_ctx: *mut c_void,
    capsule_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context_obj.capsule_export(capsule_handle) {
        Ok(()) => 0,
        Err(err) => {
            context_obj.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_capsule_import(
    module_ctx: *mut c_void,
    name: *const c_char,
    no_block: i32,
) -> *mut c_void {
    let Some(context_obj) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null_mut();
    };
    match context_obj.capsule_import(name, no_block) {
        Ok(ptr) => ptr,
        Err(err) => {
            context_obj.set_error(err);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn capi_object_sequence_len(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out_len: *mut usize,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_len.is_null() {
        context.set_error("object_sequence_len received null output pointer");
        return -1;
    }
    match context.object_sequence_len(handle) {
        Ok(len) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_len = len;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_sequence_get_item(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    index: usize,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_sequence_get_item received null output pointer");
        return -1;
    }
    match context.object_sequence_get_item(handle, index) {
        Ok(value) => {
            let item_handle = context.alloc_object(value);
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = item_handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_iter(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_get_iter received null output pointer");
        return -1;
    }
    match context.object_get_iter(handle) {
        Ok(iterator_handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = iterator_handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_iter_next(
    module_ctx: *mut c_void,
    iter_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_iter_next received null output pointer");
        return -1;
    }
    match context.object_iter_next(iter_handle) {
        Ok(Some(item_handle)) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = item_handle;
            }
            1
        }
        Ok(None) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_list_append(
    module_ctx: *mut c_void,
    list_handle: PyrsObjectHandle,
    item_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_list_append(list_handle, item_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_list_set_item(
    module_ctx: *mut c_void,
    list_handle: PyrsObjectHandle,
    index: usize,
    item_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_list_set_item(list_handle, index, item_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_dict_len(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out_len: *mut usize,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_len.is_null() {
        context.set_error("object_dict_len received null output pointer");
        return -1;
    }
    match context.object_dict_len(handle) {
        Ok(len) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_len = len;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_dict_set_item(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
    value_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_dict_set_item(dict_handle, key_handle, value_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_dict_get_item(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_dict_get_item received null output pointer");
        return -1;
    }
    match context.object_dict_get_item(dict_handle, key_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_dict_contains(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_dict_contains(dict_handle, key_handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_dict_del_item(
    module_ctx: *mut c_void,
    dict_handle: PyrsObjectHandle,
    key_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.object_dict_del_item(dict_handle, key_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_attr(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    attr_name: *const c_char,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_get_attr received null output pointer");
        return -1;
    }
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.object_get_attr(object_handle, &attr_name) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_set_attr(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    attr_name: *const c_char,
    value_handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.object_set_attr(object_handle, &attr_name, value_handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_del_attr(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    attr_name: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.object_del_attr(object_handle, &attr_name) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_has_attr(
    module_ctx: *mut c_void,
    object_handle: PyrsObjectHandle,
    attr_name: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let attr_name = match unsafe { c_name_to_string(attr_name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    match context.object_has_attr(object_handle, &attr_name) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_call_noargs(
    module_ctx: *mut c_void,
    callable_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_call_noargs received null output pointer");
        return -1;
    }
    match context.object_call_noargs(callable_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_call_onearg(
    module_ctx: *mut c_void,
    callable_handle: PyrsObjectHandle,
    arg_handle: PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_call_onearg received null output pointer");
        return -1;
    }
    match context.object_call_onearg(callable_handle, arg_handle) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_call(
    module_ctx: *mut c_void,
    callable_handle: PyrsObjectHandle,
    argc: usize,
    argv: *const PyrsObjectHandle,
    kwargc: usize,
    kwarg_names: *const *const c_char,
    kwarg_values: *const PyrsObjectHandle,
    out_handle: *mut PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out_handle.is_null() {
        context.set_error("object_call received null output pointer");
        return -1;
    }
    if argc > 0 && argv.is_null() {
        context.set_error("object_call received null argv pointer");
        return -1;
    }
    if kwargc > 0 && (kwarg_names.is_null() || kwarg_values.is_null()) {
        context.set_error("object_call received null keyword payload");
        return -1;
    }
    let arg_handles = if argc == 0 {
        &[][..]
    } else {
        // SAFETY: validated above; caller guarantees array length by `argc`.
        unsafe { std::slice::from_raw_parts(argv, argc) }
    };
    let mut kwarg_handles = Vec::with_capacity(kwargc);
    if kwargc > 0 {
        // SAFETY: validated above; caller guarantees array lengths by `kwargc`.
        let kw_names = unsafe { std::slice::from_raw_parts(kwarg_names, kwargc) };
        // SAFETY: validated above; caller guarantees array lengths by `kwargc`.
        let kw_values = unsafe { std::slice::from_raw_parts(kwarg_values, kwargc) };
        for idx in 0..kwargc {
            let name_ptr = kw_names[idx];
            let name = match unsafe { c_name_to_string(name_ptr) } {
                Ok(name) => name,
                Err(err) => {
                    context.set_error(format!(
                        "object_call invalid keyword name at index {idx}: {err}"
                    ));
                    return -1;
                }
            };
            kwarg_handles.push((name, kw_values[idx]));
        }
    }
    match context.object_call(callable_handle, arg_handles, &kwarg_handles) {
        Ok(handle) => {
            // SAFETY: caller provided non-null output pointer.
            unsafe {
                *out_handle = handle;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_string(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
) -> *const c_char {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null();
    };
    match context.object_get_string_ptr(handle) {
        Ok(ptr) => ptr,
        Err(err) => {
            context.set_error(err);
            std::ptr::null()
        }
    }
}

unsafe extern "C" fn capi_error_set(module_ctx: *mut c_void, message: *const c_char) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match unsafe { c_name_to_string(message) } {
        Ok(message) => {
            context.set_error(message);
            -1
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_error_get_message(module_ctx: *mut c_void) -> *const c_char {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null();
    };
    context.error_get_message_ptr()
}

unsafe extern "C" fn capi_error_clear(module_ctx: *mut c_void) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    context.clear_error();
    0
}

unsafe extern "C" fn capi_error_occurred(module_ctx: *mut c_void) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 1;
    };
    if context.last_error.is_some() { 1 } else { 0 }
}

enum ExtensionExecutionPlan {
    HelloExt,
    Dynamic {
        library_path: PathBuf,
        symbol: String,
    },
}

impl Vm {
    fn ensure_builtin_datetime_capi_capsule(&mut self) {
        if self
            .extension_capsule_registry
            .contains_key(PYRS_DATETIME_CAPSULE_NAME)
        {
            return;
        }
        // SAFETY: static capsule storage and exported type/singleton symbols live for
        // process lifetime; registry stores raw pointers as opaque capsule payloads.
        unsafe {
            PYRS_DATETIME_CAPI.date_type = std::ptr::addr_of_mut!(PyType_Type).cast();
            PYRS_DATETIME_CAPI.datetime_type = std::ptr::addr_of_mut!(PyType_Type).cast();
            PYRS_DATETIME_CAPI.time_type = std::ptr::addr_of_mut!(PyType_Type).cast();
            PYRS_DATETIME_CAPI.delta_type = std::ptr::addr_of_mut!(PyType_Type).cast();
            PYRS_DATETIME_CAPI.tzinfo_type = std::ptr::addr_of_mut!(PyType_Type).cast();
            PYRS_DATETIME_CAPI.timezone_utc = std::ptr::addr_of_mut!(_Py_NoneStruct).cast();
            self.extension_capsule_registry.insert(
                PYRS_DATETIME_CAPSULE_NAME.to_string(),
                super::ExtensionCapsuleRegistryEntry {
                    pointer: std::ptr::addr_of_mut!(PYRS_DATETIME_CAPI) as usize,
                    context: 0,
                    destructor: None,
                },
            );
        }
    }

    fn prune_extension_module_state_registry(&mut self) {
        let live_module_ids: std::collections::HashSet<u64> =
            self.modules.values().map(|module| module.id()).collect();
        let stale_ids: Vec<u64> = self
            .extension_module_state_registry
            .keys()
            .copied()
            .filter(|id| !live_module_ids.contains(id))
            .collect();
        for stale_id in stale_ids {
            if let Some(entry) = self.extension_module_state_registry.remove(&stale_id) {
                if entry.state != 0 {
                    if let Some(finalize_func) = entry.finalize_func {
                        // SAFETY: finalize function pointer was provided by extension code.
                        unsafe {
                            finalize_func(entry.state as *mut c_void);
                        }
                    }
                    if let Some(free_func) = entry.free_func {
                        // SAFETY: free function pointer was provided by extension code.
                        unsafe {
                            free_func(entry.state as *mut c_void);
                        }
                    }
                }
            }
        }
    }

    fn cpython_init_symbol_for_module(module_name: &str) -> String {
        let leaf = module_name
            .rsplit('.')
            .next()
            .unwrap_or(module_name)
            .replace('-', "_");
        format!("PyInit_{leaf}")
    }

    fn capi_api_v1(&self) -> PyrsApiV1 {
        PyrsApiV1 {
            abi_version: PYRS_CAPI_ABI_VERSION,
            api_has_capability: capi_api_has_capability,
            module_set_int: capi_module_set_int,
            module_set_bool: capi_module_set_bool,
            module_set_string: capi_module_set_string,
            module_add_function: capi_module_add_function,
            module_add_function_kw: capi_module_add_function_kw,
            object_new_int: capi_object_new_int,
            object_new_none: capi_object_new_none,
            object_new_bool: capi_object_new_bool,
            object_new_float: capi_object_new_float,
            object_new_bytes: capi_object_new_bytes,
            object_new_bytearray: capi_object_new_bytearray,
            object_new_memoryview: capi_object_new_memoryview,
            object_new_tuple: capi_object_new_tuple,
            object_new_list: capi_object_new_list,
            object_new_dict: capi_object_new_dict,
            object_new_string: capi_object_new_string,
            object_incref: capi_object_incref,
            object_decref: capi_object_decref,
            module_set_object: capi_module_set_object,
            module_get_object: capi_module_get_object,
            module_import: capi_module_import,
            module_get_attr: capi_module_get_attr,
            module_set_state: capi_module_set_state,
            module_get_state: capi_module_get_state,
            module_set_finalize: capi_module_set_finalize,
            object_type: capi_object_type,
            object_is_instance: capi_object_is_instance,
            object_is_subclass: capi_object_is_subclass,
            object_get_int: capi_object_get_int,
            object_get_float: capi_object_get_float,
            object_get_bool: capi_object_get_bool,
            object_get_bytes: capi_object_get_bytes,
            object_len: capi_object_len,
            object_get_item: capi_object_get_item,
            object_sequence_len: capi_object_sequence_len,
            object_sequence_get_item: capi_object_sequence_get_item,
            object_get_iter: capi_object_get_iter,
            object_iter_next: capi_object_iter_next,
            object_list_append: capi_object_list_append,
            object_list_set_item: capi_object_list_set_item,
            object_dict_len: capi_object_dict_len,
            object_dict_set_item: capi_object_dict_set_item,
            object_dict_get_item: capi_object_dict_get_item,
            object_dict_contains: capi_object_dict_contains,
            object_dict_del_item: capi_object_dict_del_item,
            object_get_attr: capi_object_get_attr,
            object_set_attr: capi_object_set_attr,
            object_del_attr: capi_object_del_attr,
            object_has_attr: capi_object_has_attr,
            object_call_noargs: capi_object_call_noargs,
            object_call_onearg: capi_object_call_onearg,
            object_call: capi_object_call,
            object_get_string: capi_object_get_string,
            error_set: capi_error_set,
            error_get_message: capi_error_get_message,
            error_clear: capi_error_clear,
            error_occurred: capi_error_occurred,
            module_set_attr: capi_module_set_attr,
            module_del_attr: capi_module_del_attr,
            module_has_attr: capi_module_has_attr,
            object_set_item: capi_object_set_item,
            object_del_item: capi_object_del_item,
            object_contains: capi_object_contains,
            object_dict_keys: capi_object_dict_keys,
            object_dict_items: capi_object_dict_items,
            object_get_buffer: capi_object_get_buffer,
            object_get_writable_buffer: capi_object_get_writable_buffer,
            object_release_buffer: capi_object_release_buffer,
            capsule_new: capi_capsule_new,
            capsule_get_pointer: capi_capsule_get_pointer,
            capsule_set_pointer: capi_capsule_set_pointer,
            capsule_get_name: capi_capsule_get_name,
            capsule_set_context: capi_capsule_set_context,
            capsule_get_context: capi_capsule_get_context,
            capsule_set_destructor: capi_capsule_set_destructor,
            capsule_get_destructor: capi_capsule_get_destructor,
            capsule_set_name: capi_capsule_set_name,
            capsule_is_valid: capi_capsule_is_valid,
            capsule_export: capi_capsule_export,
            capsule_import: capi_capsule_import,
            object_get_buffer_info: capi_object_get_buffer_info,
            object_get_buffer_info_v2: capi_object_get_buffer_info_v2,
        }
    }

    pub(super) fn register_extension_callable(
        &mut self,
        module: ObjRef,
        name: &str,
        kind: ExtensionCallableKind,
    ) -> Result<Value, RuntimeError> {
        let id = self.next_extension_callable_id;
        self.next_extension_callable_id = self.next_extension_callable_id.wrapping_add(1);
        if self.next_extension_callable_id == 0 {
            self.next_extension_callable_id = 1;
        }
        self.extension_callable_registry.insert(
            id,
            super::ExtensionCallableEntry {
                module: module.clone(),
                name: name.to_string(),
                kind,
            },
        );

        let native = self.heap.alloc_native_method(NativeMethodObject::new(
            NativeMethodKind::ExtensionFunctionCall(id),
        ));
        let bound = match self
            .heap
            .alloc_bound_method(BoundMethod::new(native, module))
        {
            Value::BoundMethod(obj) => obj,
            _ => unreachable!(),
        };
        Ok(Value::BoundMethod(bound))
    }

    pub(super) fn call_extension_callable(
        &mut self,
        function_id: u64,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<NativeCallResult, RuntimeError> {
        let Some(entry) = self.extension_callable_registry.get(&function_id).cloned() else {
            return Err(RuntimeError::new(format!(
                "unknown extension callable id {}",
                function_id
            )));
        };
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, entry.module.clone());
        let mut arg_handles = Vec::with_capacity(args.len());
        for arg in args {
            arg_handles.push(call_ctx.alloc_object(arg));
        }
        let api = self.capi_api_v1();
        let mut result_handle: PyrsObjectHandle = 0;
        let status = match entry.kind {
            ExtensionCallableKind::Positional(callback) => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(format!(
                        "extension function '{}.{}' does not accept keyword arguments",
                        match &*entry.module.kind() {
                            Object::Module(module_data) => module_data.name.clone(),
                            _ => "<extension>".to_string(),
                        },
                        entry.name
                    )));
                }
                // SAFETY: callback pointer comes from extension registration and the API/context
                // pointers remain valid for the duration of this call.
                unsafe {
                    callback(
                        &api as *const PyrsApiV1,
                        (&mut call_ctx as *mut ModuleCapiContext).cast(),
                        arg_handles.len(),
                        arg_handles.as_ptr(),
                        &mut result_handle as *mut PyrsObjectHandle,
                    )
                }
            }
            ExtensionCallableKind::WithKeywords(callback) => {
                let mut kw_name_storage = Vec::with_capacity(kwargs.len());
                let mut kw_name_ptrs = Vec::with_capacity(kwargs.len());
                let mut kw_value_handles = Vec::with_capacity(kwargs.len());
                for (name, value) in kwargs {
                    let c_name = CString::new(name.as_str()).map_err(|_| {
                        RuntimeError::new("extension call keyword name contains interior NUL byte")
                    })?;
                    kw_name_storage.push(c_name);
                    let ptr = kw_name_storage
                        .last()
                        .map(|name| name.as_ptr())
                        .unwrap_or(std::ptr::null());
                    kw_name_ptrs.push(ptr);
                    kw_value_handles.push(call_ctx.alloc_object(value));
                }
                // SAFETY: callback pointer comes from extension registration and the API/context
                // pointers remain valid for the duration of this call. Keyword C strings and
                // value handles remain alive for the callback duration.
                unsafe {
                    callback(
                        &api as *const PyrsApiV1,
                        (&mut call_ctx as *mut ModuleCapiContext).cast(),
                        arg_handles.len(),
                        arg_handles.as_ptr(),
                        kw_name_ptrs.len(),
                        kw_name_ptrs.as_ptr(),
                        kw_value_handles.as_ptr(),
                        &mut result_handle as *mut PyrsObjectHandle,
                    )
                }
            }
        };
        if status != 0 {
            let detail = call_ctx
                .last_error
                .as_deref()
                .map(|text| format!(": {text}"))
                .unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "extension function '{}.{}' failed with status {}{}",
                match &*entry.module.kind() {
                    Object::Module(module_data) => module_data.name.clone(),
                    _ => "<extension>".to_string(),
                },
                entry.name,
                status,
                detail
            )));
        }
        if result_handle == 0 {
            return Err(RuntimeError::new(format!(
                "extension function '{}.{}' returned null handle",
                match &*entry.module.kind() {
                    Object::Module(module_data) => module_data.name.clone(),
                    _ => "<extension>".to_string(),
                },
                entry.name
            )));
        }
        let Some(result) = call_ctx.object_value(result_handle) else {
            return Err(RuntimeError::new(format!(
                "extension function '{}.{}' returned unknown handle {}",
                match &*entry.module.kind() {
                    Object::Module(module_data) => module_data.name.clone(),
                    _ => "<extension>".to_string(),
                },
                entry.name,
                result_handle
            )));
        };
        Ok(NativeCallResult::Value(result))
    }

    fn set_extension_metadata(
        &mut self,
        module: &ObjRef,
        abi_tag: &str,
        entrypoint: &str,
        origin: &Path,
    ) -> Result<(), RuntimeError> {
        let Object::Module(module_data) = &mut *module.kind_mut() else {
            return Err(RuntimeError::new("extension load target is not a module"));
        };
        module_data
            .globals
            .insert("__pyrs_extension__".to_string(), Value::Bool(true));
        module_data.globals.insert(
            "__pyrs_extension_abi__".to_string(),
            Value::Str(abi_tag.to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_entrypoint__".to_string(),
            Value::Str(entrypoint.to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_origin__".to_string(),
            Value::Str(origin.to_string_lossy().to_string()),
        );
        module_data.globals.insert(
            "__pyrs_capi_abi_version__".to_string(),
            Value::Int(PYRS_CAPI_ABI_VERSION as i64),
        );
        Ok(())
    }

    fn execute_dynamic_extension(
        &mut self,
        module: &ObjRef,
        module_name: &str,
        library_path: &Path,
        symbol: &str,
    ) -> Result<(), RuntimeError> {
        let trace_slots = std::env::var_os("PYRS_TRACE_EXT_SLOTS").is_some();
        if trace_slots {
            eprintln!(
                "[ext-load] module={} begin initialized={} in_progress={}",
                module_name,
                self.extension_initialized_names.contains(module_name),
                self.extension_init_in_progress.contains(module_name)
            );
        }
        if self.extension_init_in_progress.contains(module_name) {
            if trace_slots {
                eprintln!("[ext-load] module={} skip=init_in_progress", module_name);
            }
            return Ok(());
        }
        if self.extension_initialized_names.contains(module_name) {
            if trace_slots {
                eprintln!("[ext-load] module={} skip=already_initialized", module_name);
            }
            if let Some(existing) = self.modules.get(module_name).cloned()
                && existing.id() != module.id()
                && let Object::Module(existing_data) = &*existing.kind()
                && let Object::Module(current_data) = &mut *module.kind_mut()
            {
                current_data.globals = existing_data.globals.clone();
            }
            return Ok(());
        }
        if let Some(message) = self.extension_init_failures.get(module_name).cloned() {
            return Err(RuntimeError::new(message));
        }
        if let Object::Module(module_data) = &*module.kind()
            && matches!(
                module_data.globals.get("__pyrs_extension_initialized__"),
                Some(Value::Bool(true))
            )
        {
            if trace_slots {
                eprintln!(
                    "[ext-load] module={} skip=module_flag_initialized",
                    module_name
                );
            }
            return Ok(());
        }
        if let Object::Module(module_data) = &*module.kind()
            && let Some(Value::Str(message)) =
                module_data.globals.get("__pyrs_extension_init_error__")
        {
            return Err(RuntimeError::new(message.clone()));
        }
        self.extension_init_in_progress
            .insert(module_name.to_string());
        let _init_scope_guard = ExtensionInitScopeGuard::new(self, module_name);

        enum ResolvedInit {
            Pyrs {
                handle: SharedLibraryHandle,
                initializer: crate::extensions::PyrsExtensionInitV1,
            },
            Cpython {
                handle: SharedLibraryHandle,
                initializer: CpythonExtensionInit,
            },
        }

        let (resolved_symbol, resolved_init): (String, ResolvedInit) = if symbol
            .starts_with("PyInit_")
        {
            let (handle, init) = load_dynamic_symbol::<CpythonExtensionInit>(library_path, symbol)
                .map_err(RuntimeError::new)?;
            (
                symbol.to_string(),
                ResolvedInit::Cpython {
                    handle,
                    initializer: init,
                },
            )
        } else {
            match load_dynamic_initializer(library_path, symbol) {
                Ok((handle, init)) => (
                    symbol.to_string(),
                    ResolvedInit::Pyrs {
                        handle,
                        initializer: init,
                    },
                ),
                Err(pyrs_err) if symbol == PYRS_DYNAMIC_INIT_SYMBOL_V1 => {
                    let cpython_symbol = Self::cpython_init_symbol_for_module(module_name);
                    match load_dynamic_symbol::<CpythonExtensionInit>(library_path, &cpython_symbol)
                    {
                        Ok((handle, init)) => (
                            cpython_symbol,
                            ResolvedInit::Cpython {
                                handle,
                                initializer: init,
                            },
                        ),
                        Err(cpython_err) => {
                            return Err(RuntimeError::new(format!(
                                "{pyrs_err}; fallback '{}' also failed: {cpython_err}",
                                cpython_symbol
                            )));
                        }
                    }
                }
                Err(err) => return Err(RuntimeError::new(err)),
            }
        };

        let mut module_ctx = ModuleCapiContext::new(self as *mut Vm, module.clone());
        let init_result = match resolved_init {
            ResolvedInit::Pyrs {
                handle,
                initializer,
            } => {
                let api = self.capi_api_v1();
                // SAFETY: initializer is resolved from the shared object symbol with expected signature;
                // pointers are valid for the duration of the call.
                let status = unsafe {
                    initializer(
                        &api as *const PyrsApiV1,
                        (&mut module_ctx as *mut ModuleCapiContext).cast(),
                    )
                };
                if status != 0 {
                    let message = module_ctx
                        .last_error
                        .as_deref()
                        .map(|text| format!(": {text}"))
                        .unwrap_or_default();
                    return Err(RuntimeError::new(format!(
                        "extension '{}' initializer '{}' failed with status {}{}",
                        module_name, resolved_symbol, status, message
                    )));
                }
                if let Some(message) = module_ctx.last_error.as_deref() {
                    return Err(RuntimeError::new(format!(
                        "extension '{}' initializer '{}' reported error despite success: {}",
                        module_name, resolved_symbol, message
                    )));
                }
                self.extension_libraries.push(handle);
                std::ptr::null_mut()
            }
            ResolvedInit::Cpython {
                handle,
                initializer,
            } => {
                let previous_context =
                    cpython_set_active_context(&mut module_ctx as *mut ModuleCapiContext);
                // SAFETY: symbol was resolved with `unsafe extern "C" fn() -> *mut c_void`.
                let result = unsafe { initializer() };
                cpython_set_active_context(previous_context);
                self.extension_libraries.push(handle);
                result
            }
        };

        if resolved_symbol.starts_with("PyInit_") {
            if init_result.is_null() {
                let message = module_ctx
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "extension returned null module".to_string());
                return Err(RuntimeError::new(format!(
                    "extension '{}' initializer '{}' failed: {}",
                    module_name, resolved_symbol, message
                )));
            }
            let returned = if let Some(value) = module_ctx.cpython_value_from_ptr(init_result) {
                value
            } else {
                let previous_context =
                    cpython_set_active_context(&mut module_ctx as *mut ModuleCapiContext);
                // CPython multi-phase extensions return `PyModuleDef*` from `PyInit_*`.
                // Our import path already created the target module object, so use that
                // module as the execution target and drive slot execution from `m_slots`.
                let mut module_ptr =
                    module_ctx.alloc_cpython_ptr_for_value(Value::Module(module.clone()));
                if !module_ptr.is_null() {
                    let module_def = init_result.cast::<CpythonModuleDef>();
                    if !module_def.is_null() {
                        // SAFETY: module_def points to extension-provided PyModuleDef layout.
                        let slots_ptr = unsafe { (*module_def).m_slots };
                        if !slots_ptr.is_null() {
                            let mut slot_index = 0usize;
                            let mut cursor = slots_ptr.cast::<CpythonModuleDefSlot>();
                            let module_spec_ptr = match &*module.kind() {
                                Object::Module(module_data) => module_data
                                    .globals
                                    .get("__spec__")
                                    .cloned()
                                    .map(|spec| module_ctx.alloc_cpython_ptr_for_value(spec))
                                    .unwrap_or(std::ptr::null_mut()),
                                _ => std::ptr::null_mut(),
                            };
                            loop {
                                // SAFETY: slots array is terminated by {0, NULL}.
                                let slot = unsafe { (*cursor).slot };
                                // SAFETY: slots array is terminated by {0, NULL}.
                                let value = unsafe { (*cursor).value };
                                if slot == 0 {
                                    break;
                                }
                                if trace_slots {
                                    eprintln!(
                                        "[ext-slot] module={} symbol={} index={} slot={} value={:p}",
                                        module_name, resolved_symbol, slot_index, slot, value
                                    );
                                }
                                if slot == 1 && !value.is_null() {
                                    // Py_mod_create(module_spec, module_def) -> module object.
                                    let create: unsafe extern "C" fn(
                                        *mut c_void,
                                        *mut c_void,
                                    )
                                        -> *mut c_void = unsafe { std::mem::transmute(value) };
                                    let created = unsafe { create(module_spec_ptr, init_result) };
                                    if !created.is_null() {
                                        module_ptr = created;
                                    }
                                } else if slot == 2 && !value.is_null() {
                                    // Py_mod_exec(module) -> int status.
                                    let exec: unsafe extern "C" fn(*mut c_void) -> i32 =
                                        unsafe { std::mem::transmute(value) };
                                    let status = unsafe { exec(module_ptr) };
                                    if status != 0 {
                                        if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
                                            eprintln!(
                                                "[ext-slot] module={} slot_exec_status={} first_error={:?} last_error={:?}",
                                                module_name,
                                                status,
                                                module_ctx.first_error,
                                                module_ctx.last_error
                                            );
                                        }
                                        cpython_set_active_context(previous_context);
                                        let message = module_ctx
                                            .first_error
                                            .clone()
                                            .or_else(|| module_ctx.last_error.clone())
                                            .unwrap_or_else(|| "Py_mod_exec failed".to_string());
                                        let full_error = format!(
                                            "extension '{}' initializer '{}' Py_mod_exec failed: {}",
                                            module_name, resolved_symbol, message
                                        );
                                        if let Object::Module(module_data) = &mut *module.kind_mut()
                                        {
                                            module_data.globals.insert(
                                                "__pyrs_extension_init_error__".to_string(),
                                                Value::Str(full_error.clone()),
                                            );
                                        }
                                        self.extension_init_failures
                                            .insert(module_name.to_string(), full_error.clone());
                                        if trace_slots {
                                            eprintln!(
                                                "[ext-load] module={} slot_exec_error={}",
                                                module_name, message
                                            );
                                        }
                                        return Err(RuntimeError::new(full_error));
                                    }
                                }
                                // SAFETY: move to next slot entry.
                                cursor = unsafe { cursor.add(1) };
                                slot_index += 1;
                            }
                        }
                    }
                }
                cpython_set_active_context(previous_context);
                if module_ptr.is_null() {
                    if trace_slots {
                        eprintln!("[ext-load] module={} unknown_module_ptr", module_name);
                    }
                    return Err(RuntimeError::new(format!(
                        "extension '{}' initializer '{}' returned unknown PyObject pointer",
                        module_name, resolved_symbol
                    )));
                }
                module_ctx
                    .cpython_value_from_ptr(module_ptr)
                    .ok_or_else(|| {
                        RuntimeError::new(format!(
                            "extension '{}' initializer '{}' returned unknown PyObject pointer",
                            module_name, resolved_symbol
                        ))
                    })?
            };
            let Value::Module(returned_module) = returned else {
                if trace_slots {
                    eprintln!("[ext-load] module={} non_module_return", module_name);
                }
                return Err(RuntimeError::new(format!(
                    "extension '{}' initializer '{}' did not return a module object",
                    module_name, resolved_symbol
                )));
            };
            if returned_module.id() != module.id() {
                if trace_slots {
                    eprintln!(
                        "[ext-load] module={} unexpected_module_instance returned_id={} expected_id={}",
                        module_name,
                        returned_module.id(),
                        module.id()
                    );
                }
                return Err(RuntimeError::new(format!(
                    "extension '{}' initializer '{}' returned unexpected module instance",
                    module_name, resolved_symbol
                )));
            }
        }

        let Object::Module(module_data) = &mut *module.kind_mut() else {
            return Err(RuntimeError::new(format!(
                "module '{}' invalid after extension init",
                module_name
            )));
        };
        module_data.globals.insert(
            "__pyrs_extension_library__".to_string(),
            Value::Str(library_path.to_string_lossy().to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_symbol__".to_string(),
            Value::Str(resolved_symbol.clone()),
        );
        module_data.globals.insert(
            "__pyrs_extension_entrypoint__".to_string(),
            Value::Str(format!("dynamic:{resolved_symbol}")),
        );
        let symbol_family = if resolved_symbol == PYRS_DYNAMIC_INIT_SYMBOL_V1 {
            "pyrs-v1"
        } else if resolved_symbol.starts_with("PyInit_") {
            "cpython"
        } else {
            "custom"
        };
        module_data.globals.insert(
            "__pyrs_extension_expected_symbol__".to_string(),
            Value::Str(resolved_symbol),
        );
        module_data.globals.insert(
            "__pyrs_extension_symbol_family__".to_string(),
            Value::Str(symbol_family.to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_initialized__".to_string(),
            Value::Bool(true),
        );
        module_data.globals.remove("__pyrs_extension_init_error__");
        self.extension_init_failures.remove(module_name);
        self.extension_initialized_names
            .insert(module_name.to_string());
        if trace_slots {
            eprintln!("[ext-load] module={} done", module_name);
        }
        Ok(())
    }

    pub(super) fn exec_extension_module(
        &mut self,
        module: &ObjRef,
        name: &str,
        source_path: &Path,
    ) -> Result<(), RuntimeError> {
        let (abi_tag, entrypoint_name, plan) = if source_path
            .to_string_lossy()
            .ends_with(PYRS_EXTENSION_MANIFEST_SUFFIX)
        {
            let manifest =
                parse_extension_manifest(source_path, name).map_err(RuntimeError::new)?;
            let entrypoint_name = manifest.entrypoint.as_str();
            let plan = match manifest.entrypoint {
                ExtensionEntrypoint::HelloExt => ExtensionExecutionPlan::HelloExt,
                ExtensionEntrypoint::DynamicSymbol(ref symbol) => {
                    let library_path =
                        manifest.resolve_library_path(source_path).ok_or_else(|| {
                            RuntimeError::new(format!(
                                "extension manifest '{}' missing dynamic library path",
                                source_path.display()
                            ))
                        })?;
                    ExtensionExecutionPlan::Dynamic {
                        library_path,
                        symbol: symbol.clone(),
                    }
                }
            };
            (manifest.abi_tag, entrypoint_name, plan)
        } else if path_is_shared_library(source_path) {
            (
                PYRS_EXTENSION_ABI_TAG.to_string(),
                format!("dynamic:{PYRS_DYNAMIC_INIT_SYMBOL_V1}"),
                ExtensionExecutionPlan::Dynamic {
                    library_path: source_path.to_path_buf(),
                    symbol: PYRS_DYNAMIC_INIT_SYMBOL_V1.to_string(),
                },
            )
        } else {
            return Err(RuntimeError::new(format!(
                "unsupported extension module source '{}'",
                source_path.display()
            )));
        };

        self.set_extension_metadata(module, &abi_tag, &entrypoint_name, source_path)?;

        match plan {
            ExtensionExecutionPlan::HelloExt => {
                let Object::Module(module_data) = &mut *module.kind_mut() else {
                    return Err(RuntimeError::new(format!(
                        "module '{}' extension load target is invalid",
                        name
                    )));
                };
                module_data
                    .globals
                    .insert("EXTENSION_LOADED".to_string(), Value::Bool(true));
                module_data.globals.insert(
                    "ENTRYPOINT".to_string(),
                    Value::Str("hello_ext".to_string()),
                );
                module_data.globals.insert(
                    "MESSAGE".to_string(),
                    Value::Str("hello from hello_ext".to_string()),
                );
                Ok(())
            }
            ExtensionExecutionPlan::Dynamic {
                library_path,
                symbol,
            } => self.execute_dynamic_extension(module, name, &library_path, &symbol),
        }
    }
}
