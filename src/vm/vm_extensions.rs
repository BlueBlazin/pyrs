use std::backtrace::Backtrace;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_long, c_ulong, c_void};
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    BigInt, BoundMethod, BuiltinFunction, ClassObject, InstanceObject, ModuleObject,
    NativeMethodKind, NativeMethodObject, Object, RuntimeError, Value,
};

use super::{
    ExtensionCallableKind, GeneratorResumeKind, GeneratorResumeOutcome, InternalCallOutcome,
    NativeCallResult, ObjRef, Vm, add_values, and_values, dict_contains_key_checked,
    dict_get_value, dict_remove_value, dict_set_value_checked, div_values, floor_div_values,
    invert_value, is_truthy, lshift_values, memoryview_bounds, mod_values, mul_values, neg_value,
    or_values, pos_value, pow_values, rshift_values, sub_values, value_to_int, xor_values,
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
const PY_MEMBER_T_INT: c_int = 1;
const PY_MEMBER_T_LONG: c_int = 2;
const PY_MEMBER_T_OBJECT: c_int = 6;
const PY_MEMBER_T_CHAR: c_int = 7;
const PY_MEMBER_T_UINT: c_int = 11;
const PY_MEMBER_T_ULONG: c_int = 12;
const PY_MEMBER_T_BOOL: c_int = 14;
const PY_MEMBER_T_OBJECT_EX: c_int = 16;
const PY_MEMBER_T_LONGLONG: c_int = 17;
const PY_MEMBER_T_ULONGLONG: c_int = 18;
const PY_MEMBER_T_PYSSIZET: c_int = 19;

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

unsafe fn cpython_external_capsule_pointer(
    _context: &ModuleCapiContext,
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
        if std::env::var_os("PYRS_TRACE_CPY_CAPSULE").is_some() {
            eprintln!(
                "[cpy-capsule] external type mismatch ptr={:p} type={:p} expected={:p}",
                capsule,
                ty,
                std::ptr::addr_of_mut!(PyCapsule_Type).cast::<c_void>()
            );
        }
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
            return Err(format!(
                "capsule name mismatch (requested='{}', actual='{}')",
                requested.to_string_lossy(),
                actual.to_string_lossy()
            ));
        }
    }
    // SAFETY: `raw` points to a capsule-compatible object.
    let pointer = unsafe { (*raw).pointer };
    if pointer.is_null() {
        return Err("capsule pointer is null".to_string());
    }
    Ok(Some(pointer))
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
        Value::Slice(_) => std::ptr::addr_of_mut!(PySlice_Type).cast(),
        Value::Class(_) => std::ptr::addr_of_mut!(PyType_Type).cast(),
        Value::Builtin(_) => std::ptr::addr_of_mut!(PyCFunction_Type).cast(),
        _ => std::ptr::addr_of_mut!(PyBaseObject_Type).cast(),
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
        } else if raw == PyExc_EOFError as usize {
            Some("EOFError")
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
        PyCFunction_Type.tp_call = cpython_cfunction_tp_call as *mut c_void;
        PyCFunction_Type.tp_getattro = cpython_cfunction_tp_getattro as *mut c_void;

        let type_objects: &mut [*mut CpythonTypeObject] = &mut [
            std::ptr::addr_of_mut!(PyBaseObject_Type),
            std::ptr::addr_of_mut!(PyBool_Type),
            std::ptr::addr_of_mut!(PyByteArray_Type),
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
        ensure_cpython_exception_symbol(std::ptr::addr_of_mut!(PyExc_EOFError), type_ptr);
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
}

struct ModuleCapiContext {
    vm: *mut Vm,
    module: ObjRef,
    run_capsule_destructors_on_drop: bool,
    strict_capsule_refcount: bool,
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
    cpython_owned_ptrs: HashSet<usize>,
    cpython_cfunction_ptr_cache: HashMap<(usize, usize, usize, usize), *mut c_void>,
    cpython_list_buffers: HashMap<PyrsObjectHandle, (*mut *mut c_void, usize)>,
    cpython_sync_in_progress: HashSet<PyrsObjectHandle>,
    module_dict_handles: HashMap<PyrsObjectHandle, ObjRef>,
    module_dict_handle_by_module_id: HashMap<u64, PyrsObjectHandle>,
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
        for raw in self.cpython_allocations.drain(..) {
            let keep_pinned = if self.vm.is_null() {
                false
            } else {
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *self.vm };
                vm.extension_pinned_cpython_allocation_set
                    .contains(&(raw as usize))
            };
            if keep_pinned {
                continue;
            }
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
            run_capsule_destructors_on_drop: true,
            strict_capsule_refcount: true,
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
            cpython_owned_ptrs: HashSet::new(),
            cpython_cfunction_ptr_cache: HashMap::new(),
            cpython_list_buffers: HashMap::new(),
            cpython_sync_in_progress: HashSet::new(),
            module_dict_handles: HashMap::new(),
            module_dict_handle_by_module_id: HashMap::new(),
        }
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
            Some(other) => format!("{other:?}"),
            None => "error".to_string(),
        }
    }

    fn set_error_state(
        &mut self,
        ptype: *mut c_void,
        pvalue: *mut c_void,
        ptraceback: *mut c_void,
        message: String,
    ) {
        self.current_error = Some(CpythonErrorState {
            ptype,
            pvalue,
            ptraceback,
        });
        self.set_error_message(message);
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
        let pvalue = self.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
        self.set_error_state(
            unsafe { PyExc_RuntimeError },
            pvalue,
            std::ptr::null_mut(),
            message,
        );
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
        let (refcount, ob_type, tuple_items, list_items, bytes_payload) =
            match self.objects.get(&handle).map(|slot| {
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
            }) {
                Some(state) => state,
                None if capsule_state.is_some() => (1, std::ptr::null_mut(), None, None, None),
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
        raw.cast()
    }

    pub(super) fn cpython_proxy_raw_ptr_from_value(value: &Value) -> Option<*mut c_void> {
        match value {
            Value::Class(class_obj) => {
                let Object::Class(class_data) = &*class_obj.kind() else {
                    return None;
                };
                if class_data.name != "__pyrs_cpython_proxy__" {
                    return None;
                }
                match class_data.attrs.get("__pyrs_cpython_proxy_ptr__") {
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
                if class_data.name != "__pyrs_cpython_proxy__" {
                    return None;
                }
                match instance_data.attrs.get("__pyrs_cpython_proxy_ptr__") {
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
                    return cfunction_ptr;
                }
            }
        }
        if let Some(raw_ptr) = Self::cpython_proxy_raw_ptr_from_value(&value) {
            return raw_ptr;
        }
        if let Value::Class(class_obj) = &value
            && let Object::Class(class_data) = &*class_obj.kind()
            && class_data.name == "__pyrs_cpython_proxy__"
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
        if let Value::Builtin(builtin) = &value
            && let Some(type_ptr) = cpython_builtin_type_ptr_for_builtin(builtin)
        {
            return type_ptr;
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
        let handle = self.alloc_object(value);
        self.alloc_cpython_ptr_for_handle(handle)
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
        let handle = self.cpython_handle_from_ptr(object)?;
        if !self.objects.contains_key(&handle) && self.capsules.contains_key(&handle) {
            return self.cpython_external_proxy_value(object);
        }
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
        let is_type_object = if object_type == expected_type {
            true
        } else if object_type.is_null() {
            false
        } else {
            // SAFETY: `object_type` is a valid type header for external objects.
            unsafe { ((*object_type).tp_flags & PY_TPFLAGS_TYPE_SUBCLASS) != 0 }
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
        // SAFETY: VM pointer is valid for the C-API context lifetime.
        let vm = unsafe { &mut *self.vm };
        let proxy_class = match vm.heap.alloc_class(ClassObject::new(
            "__pyrs_cpython_proxy__".to_string(),
            Vec::new(),
        )) {
            Value::Class(class_obj) => class_obj,
            other => return Some(other),
        };
        if is_type_object {
            if let Object::Class(class_data) = &mut *proxy_class.kind_mut() {
                class_data.attrs.insert(
                    "__pyrs_cpython_proxy_ptr__".to_string(),
                    Value::Int(object as usize as i64),
                );
            }
            return Some(Value::Class(proxy_class));
        }
        match vm.heap.alloc_instance(InstanceObject::new(proxy_class)) {
            Value::Instance(instance_obj) => {
                if let Object::Instance(instance_data) = &mut *instance_obj.kind_mut() {
                    instance_data.attrs.insert(
                        "__pyrs_cpython_proxy_ptr__".to_string(),
                        Value::Int(object as usize as i64),
                    );
                }
                Some(Value::Instance(instance_obj))
            }
            other => Some(other),
        }
    }

    fn cpython_value_from_ptr_or_proxy(&mut self, object: *mut c_void) -> Option<Value> {
        if let Some(value) = self.cpython_value_from_ptr(object) {
            return Some(value);
        }
        if object.is_null() || self.vm.is_null() {
            return None;
        }
        let proxy = self.cpython_external_proxy_value(object)?;
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
                if is_proxy_trace {
                    eprintln!(
                        "[cpy-proxy] tp_dict lookup hit current={:p} dict={:p} value_tag={}",
                        current,
                        dict_ptr,
                        cpython_value_debug_tag(&value)
                    );
                }
                return Some(self.alloc_cpython_ptr_for_value(value.clone()));
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
                                return Some(value_ptr);
                            }
                        }
                    }
                    // SAFETY: getset table entries are contiguous.
                    getset = unsafe { getset.add(1) };
                }
            }
            // SAFETY: current points to a PyTypeObject-compatible header.
            let (members_ptr, current_basicsize, current_name) = unsafe {
                (
                    (*current).tp_members.cast::<CpythonMemberDef>(),
                    (*current).tp_basicsize,
                    c_name_to_string((*current).tp_name).ok(),
                )
            };
            let should_scan_members = matches!(attr_name, "type" | "kind" | "itemsize")
                && current_name
                    .as_deref()
                    .map(|name| name.contains("dtype"))
                    .unwrap_or(false);
            if should_scan_members && !members_ptr.is_null() {
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
                            return Some(value_ptr);
                        }
                    }
                    // SAFETY: member table entries are contiguous.
                    member = unsafe { member.add(1) };
                }
            }
            // SAFETY: current points to a PyTypeObject-compatible header.
            let next = unsafe { (*current).tp_base };
            if next.is_null() || next == current {
                break;
            }
            current = next;
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
        let positional_count = args.len();
        let kw_count = kwargs.len();
        let mut stack: Vec<*mut c_void> =
            Vec::with_capacity(positional_count.saturating_add(kw_count));
        for value in args {
            let ptr = self.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                self.set_error("failed to materialize positional vectorcall argument");
                return Some(std::ptr::null_mut());
            }
            stack.push(ptr);
        }
        let mut kw_names: Vec<Value> = Vec::with_capacity(kw_count);
        for (name, value) in kwargs {
            kw_names.push(Value::Str(name.clone()));
            let ptr = self.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                self.set_error("failed to materialize keyword vectorcall argument");
                return Some(std::ptr::null_mut());
            }
            stack.push(ptr);
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *self.vm };
        let kwnames_ptr = if kw_names.is_empty() {
            std::ptr::null_mut()
        } else {
            self.alloc_cpython_ptr_for_value(vm.heap.alloc_tuple(kw_names))
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
        Some(unsafe { vectorcall(callable, args_ptr, positional_count, kwnames_ptr) })
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
            // SAFETY: `raw` points to owned capsule-compatible storage for this handle.
            unsafe {
                let raw_capsule = raw.cast::<CpythonCapsuleCompatObject>();
                (*raw_capsule).ob_base.ob_refcnt = slot.refcount.max(1) as isize;
                (*raw_capsule).ob_base.ob_type = std::ptr::addr_of_mut!(PyCapsule_Type).cast();
                (*raw_capsule).pointer = slot.pointer as *mut c_void;
                (*raw_capsule).name = slot.name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
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

fn cpython_set_typed_error(ptype: *mut c_void, message: impl Into<String>) {
    let message = message.into();
    let _ = with_active_cpython_context_mut(|context| {
        let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
        let ty = if ptype.is_null() {
            // SAFETY: exception singleton pointer is process-global.
            unsafe { PyExc_RuntimeError }
        } else {
            ptype
        };
        context.set_error_state(ty, pvalue, std::ptr::null_mut(), message);
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
        let Some(mut callable) = context.cpython_value_from_ptr(callable_ptr) else {
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
        let value_ptr = value;
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
        let value = match context.cpython_value_from_ptr_or_proxy(value) {
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
        if std::env::var_os("PYRS_TRACE_CPY_MODULE_ADD").is_some() {
            eprintln!(
                "[cpy-module-add] module={} attr={} value_tag={} value_ptr={:p}",
                module_data.name,
                attr_name,
                cpython_value_debug_tag(&value),
                value_ptr
            );
        }
        module_data.globals.insert(attr_name.clone(), value.clone());
        if let Err(err) = context.sync_module_dict_set(&module_obj, &attr_name, &value) {
            context.set_error(format!(
                "PyModule_AddObjectRef failed syncing module dict entry '{}': {}",
                attr_name, err
            ));
            return -1;
        }
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
        if let Some(existing_handle) = context.module_dict_handle_for_module(&module_obj) {
            return context.alloc_cpython_ptr_for_handle(existing_handle);
        }
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
        let dict_ptr = context.alloc_cpython_ptr_for_value(dict);
        let Some(dict_handle) = context.cpython_handle_from_ptr(dict_ptr) else {
            context.set_error("PyModule_GetDict failed to materialize dict handle");
            return std::ptr::null_mut();
        };
        context
            .module_dict_handles
            .insert(dict_handle, module_obj.clone());
        context
            .module_dict_handle_by_module_id
            .insert(module_obj.id(), dict_handle);
        dict_ptr
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
pub unsafe extern "C" fn PyLong_FromSize_t(value: usize) -> *mut c_void {
    if i64::try_from(value).is_ok() {
        return cpython_new_ptr_for_value(Value::Int(value as i64));
    }
    cpython_new_ptr_for_value(Value::BigInt(Box::new(BigInt::from_u64(value as u64))))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromInt32(value: i32) -> *mut c_void {
    unsafe { PyLong_FromLong(value as i64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUInt32(value: u32) -> *mut c_void {
    unsafe { PyLong_FromUnsignedLong(value as u64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromInt64(value: i64) -> *mut c_void {
    unsafe { PyLong_FromLongLong(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUInt64(value: u64) -> *mut c_void {
    unsafe { PyLong_FromUnsignedLongLong(value) }
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
pub unsafe extern "C" fn PyLong_FromString(
    value: *const c_char,
    pend: *mut *mut c_char,
    base: i32,
) -> *mut c_void {
    if value.is_null() {
        unsafe { PyErr_BadInternalCall() };
        if !pend.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe { *pend = std::ptr::null_mut() };
        }
        return std::ptr::null_mut();
    }
    if !(base == 0 || (2..=36).contains(&base)) {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "int() base must be >= 2 and <= 36, or 0",
        );
        if !pend.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe { *pend = value as *mut c_char };
        }
        return std::ptr::null_mut();
    }
    // SAFETY: `value` points to a NUL-terminated C string per API contract.
    let source = unsafe { CStr::from_ptr(value) };
    let source_text = source.to_string_lossy().into_owned();
    let mut args = vec![Value::Str(source_text)];
    if base != 10 {
        args.push(Value::Int(base as i64));
    }
    let parsed = match cpython_call_builtin(BuiltinFunction::Int, args) {
        Ok(parsed) => parsed,
        Err(err) => {
            cpython_set_typed_error(unsafe { PyExc_ValueError }, &err);
            if !pend.is_null() {
                // SAFETY: caller provided writable output pointer.
                unsafe { *pend = value as *mut c_char };
            }
            return std::ptr::null_mut();
        }
    };
    if !pend.is_null() {
        // SAFETY: `value` points to a NUL-terminated string; advancing by `to_bytes().len()`
        // lands on the trailing NUL, matching CPython's full-consume success path.
        unsafe { *pend = value.add(source.to_bytes().len()) as *mut c_char };
    }
    cpython_new_ptr_for_value(parsed)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_GetInfo() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyLong_GetInfo missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(sys_module) = vm.modules.get("sys").cloned() else {
            context.set_error("PyLong_GetInfo missing sys module");
            return std::ptr::null_mut();
        };
        let int_info = match &*sys_module.kind() {
            Object::Module(module_data) => module_data.globals.get("int_info").cloned(),
            _ => None,
        };
        let int_info = int_info.unwrap_or_else(|| {
            let mut synthetic = ModuleObject::new("<int_info>");
            synthetic
                .globals
                .insert("bits_per_digit".to_string(), Value::Int(30));
            synthetic
                .globals
                .insert("sizeof_digit".to_string(), Value::Int(4));
            synthetic
                .globals
                .insert("default_max_str_digits".to_string(), Value::Int(0));
            synthetic
                .globals
                .insert("str_digits_check_threshold".to_string(), Value::Int(0));
            vm.heap.alloc_module(synthetic)
        });
        context.alloc_cpython_ptr_for_value(int_info)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsNativeBytes(
    object: *mut c_void,
    buffer: *mut c_void,
    n_bytes: isize,
    flags: i32,
) -> isize {
    const PY_ASNATIVEBYTES_UNSIGNED_BUFFER: i32 = 0x4;
    const PY_ASNATIVEBYTES_REJECT_NEGATIVE: i32 = 0x8;
    const PY_ASNATIVEBYTES_ALLOW_INDEX: i32 = 0x10;

    if object.is_null() || n_bytes < 0 {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if n_bytes > 0 && buffer.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let little_endian = cpython_asnativebytes_resolve_endian(flags);
    with_active_cpython_context_mut(|context| {
        let mut value = match context.cpython_value_from_ptr_or_proxy(object) {
            Some(value) => value,
            None => {
                context.set_error("PyLong_AsNativeBytes received unknown object pointer");
                return -1;
            }
        };
        if !matches!(value, Value::Int(_) | Value::Bool(_) | Value::BigInt(_)) {
            if flags != -1 && (flags & PY_ASNATIVEBYTES_ALLOW_INDEX) != 0 {
                let indexed = unsafe { PyNumber_Index(object) };
                if indexed.is_null() {
                    return -1;
                }
                let next = match context.cpython_value_from_ptr_or_proxy(indexed) {
                    Some(value) => value,
                    None => {
                        unsafe { Py_DecRef(indexed) };
                        context.set_error("PyLong_AsNativeBytes index conversion failed");
                        return -1;
                    }
                };
                unsafe { Py_DecRef(indexed) };
                value = next;
            } else {
                context.set_error("expect int");
                return -1;
            }
        }
        let bigint = match cpython_bigint_from_value(value) {
            Ok(bigint) => bigint,
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        };
        if flags != -1 && (flags & PY_ASNATIVEBYTES_REJECT_NEGATIVE) != 0 && bigint.is_negative() {
            cpython_set_typed_error(unsafe { PyExc_ValueError }, "Cannot convert negative int");
            return -1;
        }
        let required = if !bigint.is_negative()
            && (flags == -1 || (flags & PY_ASNATIVEBYTES_UNSIGNED_BUFFER) != 0)
        {
            cpython_required_unsigned_bytes_for_bigint(&bigint)
        } else {
            cpython_required_signed_bytes_for_bigint(&bigint)
        };
        if n_bytes == 0 {
            return required as isize;
        }
        let n = n_bytes as usize;
        let encoded_le = cpython_bigint_to_twos_complement_le(&bigint, n);
        if little_endian != 0 {
            // SAFETY: caller provided writable output buffer of `n` bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(encoded_le.as_ptr(), buffer.cast::<u8>(), n);
            }
        } else {
            for (idx, byte) in encoded_le.iter().enumerate() {
                // SAFETY: caller provided writable output buffer of `n` bytes.
                unsafe {
                    *buffer.cast::<u8>().add(n - idx - 1) = *byte;
                }
            }
        }
        required as isize
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromNativeBytes(
    buffer: *const c_void,
    n_bytes: usize,
    flags: i32,
) -> *mut c_void {
    const PY_ASNATIVEBYTES_UNSIGNED_BUFFER: i32 = 0x4;
    if buffer.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let little_endian = cpython_asnativebytes_resolve_endian(flags);
    let signed = flags == -1 || (flags & PY_ASNATIVEBYTES_UNSIGNED_BUFFER) == 0;
    let raw = if n_bytes == 0 {
        &[][..]
    } else {
        // SAFETY: caller guarantees readable `n_bytes` bytes at `buffer`.
        unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n_bytes) }
    };
    let mut le = raw.to_vec();
    if little_endian == 0 {
        le.reverse();
    }
    let bigint = cpython_bigint_from_twos_complement_le(&le, signed);
    match bigint.to_i64() {
        Some(value) => cpython_new_ptr_for_value(Value::Int(value)),
        None => cpython_new_ptr_for_value(Value::BigInt(Box::new(bigint))),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUnsignedNativeBytes(
    buffer: *const c_void,
    n_bytes: usize,
    flags: i32,
) -> *mut c_void {
    if buffer.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let little_endian = cpython_asnativebytes_resolve_endian(flags);
    let raw = if n_bytes == 0 {
        &[][..]
    } else {
        // SAFETY: caller guarantees readable `n_bytes` bytes at `buffer`.
        unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n_bytes) }
    };
    let mut le = raw.to_vec();
    if little_endian == 0 {
        le.reverse();
    }
    let bigint = cpython_bigint_from_twos_complement_le(&le, false);
    match bigint.to_i64() {
        Some(value) => cpython_new_ptr_for_value(Value::Int(value)),
        None => cpython_new_ptr_for_value(Value::BigInt(Box::new(bigint))),
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
pub unsafe extern "C" fn PyBytes_FromObject(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        unsafe { _PyErr_BadInternalCall(std::ptr::null(), 0) };
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyBytes_FromObject received unknown object pointer");
            return std::ptr::null_mut();
        };
        if matches!(value, Value::Bytes(_)) {
            unsafe { Py_XIncRef(object) };
            return object;
        }
        if matches!(
            value,
            Value::Int(_) | Value::BigInt(_) | Value::Bool(_) | Value::Str(_)
        ) {
            context.set_error(format!(
                "TypeError: cannot convert '{}' object to bytes",
                cpython_type_name_for_object_ptr(object)
            ));
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            context.set_error("PyBytes_FromObject missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_builtin(BuiltinFunction::Bytes, vec![value], HashMap::new()) {
            Ok(bytes_value @ Value::Bytes(_)) => context.alloc_cpython_ptr_for_value(bytes_value),
            Ok(_) => {
                context.set_error("PyBytes_FromObject expected bytes result");
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

unsafe fn cpython_clear_pyobject_ref(slot: *mut *mut c_void) {
    if slot.is_null() {
        return;
    }
    // SAFETY: caller guarantees `slot` is a valid writable pointer location.
    let current = unsafe { *slot };
    if !current.is_null() {
        unsafe { Py_XDecRef(current) };
    }
    // SAFETY: caller guarantees `slot` is writable.
    unsafe {
        *slot = std::ptr::null_mut();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_Concat(pv: *mut *mut c_void, w: *mut c_void) {
    if pv.is_null() {
        cpython_set_error("PyBytes_Concat requires non-null output pointer");
        return;
    }
    // SAFETY: `pv` is checked non-null.
    let left_ptr = unsafe { *pv };
    if left_ptr.is_null() {
        return;
    }
    if w.is_null() {
        unsafe { cpython_clear_pyobject_ref(pv) };
        return;
    }
    let result = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyBytes_Concat missing VM context");
            return std::ptr::null_mut();
        }
        let left_value = match context.cpython_value_from_ptr(left_ptr) {
            Some(value) => value,
            None => {
                context.set_error("PyBytes_Concat received unknown left pointer");
                return std::ptr::null_mut();
            }
        };
        let right_value = match context.cpython_value_from_ptr_or_proxy(w) {
            Some(value) => value,
            None => {
                context.set_error("PyBytes_Concat received unknown right pointer");
                return std::ptr::null_mut();
            }
        };
        let left_bytes = match left_value {
            Value::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                Object::Bytes(values) => values.clone(),
                _ => {
                    context.set_error("PyBytes_Concat encountered invalid left bytes storage");
                    return std::ptr::null_mut();
                }
            },
            _ => {
                context.set_error(format!(
                    "TypeError: can't concat {} to {}",
                    cpython_type_name_for_object_ptr(w),
                    cpython_type_name_for_object_ptr(left_ptr)
                ));
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let right_bytes =
            match vm.call_builtin(BuiltinFunction::Bytes, vec![right_value], HashMap::new()) {
                Ok(Value::Bytes(bytes_obj)) => match &*bytes_obj.kind() {
                    Object::Bytes(values) => values.clone(),
                    _ => {
                        context.set_error("PyBytes_Concat encountered invalid right bytes storage");
                        return std::ptr::null_mut();
                    }
                },
                Ok(_) => {
                    context.set_error(format!(
                        "TypeError: can't concat {} to {}",
                        cpython_type_name_for_object_ptr(w),
                        cpython_type_name_for_object_ptr(left_ptr)
                    ));
                    return std::ptr::null_mut();
                }
                Err(_) => {
                    context.set_error(format!(
                        "TypeError: can't concat {} to {}",
                        cpython_type_name_for_object_ptr(w),
                        cpython_type_name_for_object_ptr(left_ptr)
                    ));
                    return std::ptr::null_mut();
                }
            };
        let mut merged = left_bytes;
        merged.extend(right_bytes);
        let merged_obj = vm.heap.alloc(Object::Bytes(merged));
        context.alloc_cpython_ptr_for_value(Value::Bytes(merged_obj))
    });
    let new_ptr = match result {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    };
    if new_ptr.is_null() {
        unsafe { cpython_clear_pyobject_ref(pv) };
        return;
    }
    unsafe {
        Py_XDecRef(left_ptr);
        *pv = new_ptr;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_ConcatAndDel(pv: *mut *mut c_void, w: *mut c_void) {
    unsafe { PyBytes_Concat(pv, w) };
    unsafe { Py_XDecRef(w) };
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
            if let Ok(true) = with_active_cpython_context_mut(|context| {
                context.owns_cpython_allocation_ptr(object)
            }) {
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
                if std::env::var_os("PYRS_TRACE_PYBYTES_CALLER_BT").is_some() {
                    let seen = PYBYTES_ASSTRING_MISMATCH_BT_COUNT.fetch_add(1, Ordering::Relaxed);
                    if seen < 8 {
                        eprintln!("[cpy-bytes] mismatch backtrace #{}:", seen + 1);
                        eprintln!("{}", Backtrace::force_capture());
                    }
                }
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
pub unsafe extern "C" fn PyByteArray_FromStringAndSize(
    bytes: *const c_char,
    size: isize,
) -> *mut c_void {
    if size < 0 {
        cpython_set_error("Negative size passed to PyByteArray_FromStringAndSize");
        return std::ptr::null_mut();
    }
    let payload = if size == 0 {
        Vec::new()
    } else if bytes.is_null() {
        vec![0; size as usize]
    } else {
        // SAFETY: caller guarantees `bytes` points to at least `size` bytes.
        unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), size as usize).to_vec() }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyByteArray_FromStringAndSize missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        context.alloc_cpython_ptr_for_value(vm.heap.alloc_bytearray(payload))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_FromObject(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::ByteArray, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_Size(object: *mut c_void) -> isize {
    let foreign_bytearray_len = |object: *mut c_void| -> Option<isize> {
        if object.is_null() {
            return None;
        }
        // SAFETY: pointer is treated as foreign PyObject; we only inspect fixed headers.
        let head = unsafe { object.cast::<CpythonVarObjectHead>().as_ref() }?;
        let ty = head.ob_base.ob_type.cast::<CpythonTypeObject>();
        if ty.is_null() {
            return None;
        }
        let is_bytearray = ty == std::ptr::addr_of_mut!(PyByteArray_Type)
            // SAFETY: type pointers are valid for subtype checks.
            || unsafe {
                PyType_IsSubtype(
                    ty.cast::<c_void>(),
                    std::ptr::addr_of_mut!(PyByteArray_Type).cast::<c_void>(),
                ) != 0
            };
        if is_bytearray {
            return Some(head.ob_size.max(0));
        }
        None
    };
    match cpython_value_from_ptr(object) {
        Ok(Value::ByteArray(bytearray_obj)) => match &*bytearray_obj.kind() {
            Object::ByteArray(values) => values.len() as isize,
            _ => {
                cpython_set_error("PyByteArray_Size encountered invalid bytearray storage");
                -1
            }
        },
        Ok(_) => {
            if let Some(len) = foreign_bytearray_len(object) {
                return len;
            }
            cpython_set_error("PyByteArray_Size expected bytearray object");
            -1
        }
        Err(err) => {
            if let Some(len) = foreign_bytearray_len(object) {
                return len;
            }
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_AsString(object: *mut c_void) -> *mut c_char {
    match cpython_value_from_ptr(object) {
        Ok(Value::ByteArray(bytearray_obj)) => {
            if let Ok(true) = with_active_cpython_context_mut(|context| {
                context.owns_cpython_allocation_ptr(object)
            }) {
                // SAFETY: owned bytearray-compatible pointers use bytes-like payload layout.
                return unsafe { cpython_bytes_data_ptr(object) };
            }
            let mut bytes_kind = bytearray_obj.kind_mut();
            match &mut *bytes_kind {
                Object::ByteArray(values) => values.as_mut_ptr().cast(),
                _ => {
                    cpython_set_error("PyByteArray_AsString encountered invalid bytearray storage");
                    std::ptr::null_mut()
                }
            }
        }
        Ok(_) => {
            cpython_set_error("PyByteArray_AsString expected bytearray object");
            std::ptr::null_mut()
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_Resize(object: *mut c_void, requested_size: isize) -> i32 {
    if requested_size < 0 {
        cpython_set_error(format!(
            "Can only resize to positive sizes, got {}",
            requested_size
        ));
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyByteArray_Resize missing VM context");
            return -1;
        }
        let Some(handle) = context.cpython_handle_from_ptr(object) else {
            context.set_error("PyByteArray_Resize received unknown object pointer");
            return -1;
        };
        let Some(value) = context.cpython_value_from_ptr(object) else {
            context.set_error("PyByteArray_Resize received unknown object pointer");
            return -1;
        };
        let Value::ByteArray(bytearray_obj) = value else {
            context.set_error("PyByteArray_Resize expected bytearray object");
            return -1;
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        if vm.heap.external_buffer_pin_count_for_source(&bytearray_obj) > 0 {
            context.set_error("BufferError: Existing exports of data: object cannot be re-sized");
            return -1;
        }
        let mut bytearray_kind = bytearray_obj.kind_mut();
        let Object::ByteArray(values) = &mut *bytearray_kind else {
            context.set_error("PyByteArray_Resize encountered invalid bytearray storage");
            return -1;
        };
        let target = requested_size as usize;
        if target >= values.len() {
            values.resize(target, 0);
        } else {
            values.truncate(target);
        }
        drop(bytearray_kind);
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_Concat(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    let mut left_view = CpythonBuffer {
        buf: std::ptr::null_mut(),
        obj: std::ptr::null_mut(),
        len: 0,
        itemsize: 0,
        readonly: 0,
        ndim: 0,
        format: std::ptr::null_mut(),
        shape: std::ptr::null_mut(),
        strides: std::ptr::null_mut(),
        suboffsets: std::ptr::null_mut(),
        internal: std::ptr::null_mut(),
    };
    let mut right_view = CpythonBuffer {
        buf: std::ptr::null_mut(),
        obj: std::ptr::null_mut(),
        len: 0,
        itemsize: 0,
        readonly: 0,
        ndim: 0,
        format: std::ptr::null_mut(),
        shape: std::ptr::null_mut(),
        strides: std::ptr::null_mut(),
        suboffsets: std::ptr::null_mut(),
        internal: std::ptr::null_mut(),
    };
    if unsafe { PyObject_GetBuffer(left, &mut left_view, 0) } != 0
        || unsafe { PyObject_GetBuffer(right, &mut right_view, 0) } != 0
    {
        if !left_view.obj.is_null() {
            unsafe { PyBuffer_Release((&mut left_view as *mut CpythonBuffer).cast()) };
        }
        if !right_view.obj.is_null() {
            unsafe { PyBuffer_Release((&mut right_view as *mut CpythonBuffer).cast()) };
        }
        cpython_set_error(format!(
            "TypeError: can't concat {} to {}",
            cpython_type_name_for_object_ptr(right),
            cpython_type_name_for_object_ptr(left)
        ));
        return std::ptr::null_mut();
    }
    if left_view.len < 0 || right_view.len < 0 {
        unsafe { PyBuffer_Release((&mut left_view as *mut CpythonBuffer).cast()) };
        unsafe { PyBuffer_Release((&mut right_view as *mut CpythonBuffer).cast()) };
        cpython_set_error("PyByteArray_Concat received invalid negative buffer size");
        return std::ptr::null_mut();
    }
    let left_len = left_view.len as usize;
    let right_len = right_view.len as usize;
    let total_len = match left_len.checked_add(right_len) {
        Some(len) => len,
        None => {
            unsafe { PyBuffer_Release((&mut left_view as *mut CpythonBuffer).cast()) };
            unsafe { PyBuffer_Release((&mut right_view as *mut CpythonBuffer).cast()) };
            cpython_set_error("out of memory");
            return std::ptr::null_mut();
        }
    };
    let mut payload = Vec::with_capacity(total_len);
    if left_len > 0 {
        // SAFETY: buffer export guarantees `buf` points to at least `len` readable bytes.
        let left_bytes =
            unsafe { std::slice::from_raw_parts(left_view.buf.cast::<u8>(), left_len) };
        payload.extend_from_slice(left_bytes);
    }
    if right_len > 0 {
        // SAFETY: buffer export guarantees `buf` points to at least `len` readable bytes.
        let right_bytes =
            unsafe { std::slice::from_raw_parts(right_view.buf.cast::<u8>(), right_len) };
        payload.extend_from_slice(right_bytes);
    }
    unsafe { PyBuffer_Release((&mut left_view as *mut CpythonBuffer).cast()) };
    unsafe { PyBuffer_Release((&mut right_view as *mut CpythonBuffer).cast()) };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyByteArray_Concat missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        context.alloc_cpython_ptr_for_value(vm.heap.alloc_bytearray(payload))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
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

fn cpython_optional_value_from_ptr(
    context: &mut ModuleCapiContext,
    object: *mut c_void,
    label: &str,
) -> Result<Value, String> {
    if object.is_null() {
        return Ok(Value::None);
    }
    context
        .cpython_value_from_ptr_or_proxy(object)
        .ok_or_else(|| format!("unknown {label} object pointer"))
}

fn cpython_module_name_from_object(
    context: &mut ModuleCapiContext,
    name: *mut c_void,
    api_name: &str,
) -> Result<String, String> {
    if name.is_null() {
        return Err(format!("{api_name} expected module name"));
    }
    match context
        .cpython_value_from_ptr_or_proxy(name)
        .ok_or_else(|| format!("{api_name} received unknown module name pointer"))?
    {
        Value::Str(name) => Ok(name),
        _ => Err(format!("{api_name} expected module name string")),
    }
}

fn cpython_import_add_module_by_name(
    context: &mut ModuleCapiContext,
    module_name: &str,
) -> Result<ObjRef, String> {
    if context.vm.is_null() {
        return Err("missing VM context for import API".to_string());
    }
    // SAFETY: VM pointer is valid for the active context lifetime.
    let vm = unsafe { &mut *context.vm };
    let module = vm.ensure_module(module_name);
    if let Some(modules_dict) = vm.sys_dict_obj("modules") {
        dict_set_value_checked(
            &modules_dict,
            Value::Str(module_name.to_string()),
            Value::Module(module.clone()),
        )
        .map_err(|err| err.message)?;
    } else {
        vm.refresh_sys_modules_dict();
    }
    Ok(module)
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
pub unsafe extern "C" fn PyImport_GetModuleDict() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyImport_GetModuleDict missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for the active context lifetime.
        let vm = unsafe { &mut *context.vm };
        vm.refresh_sys_modules_dict();
        let Some(modules_dict) = vm.sys_dict_obj("modules") else {
            context.set_error("unable to get sys.modules");
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(Value::Dict(modules_dict))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_AddModuleRef(name: *const c_char) -> *mut c_void {
    let module_name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        match cpython_import_add_module_by_name(context, &module_name) {
            Ok(module) => context.alloc_cpython_ptr_for_value(Value::Module(module)),
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
pub unsafe extern "C" fn PyImport_AddModuleObject(name: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let module_name =
            match cpython_module_name_from_object(context, name, "PyImport_AddModuleObject") {
                Ok(name) => name,
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            };
        match cpython_import_add_module_by_name(context, &module_name) {
            Ok(module) => context.alloc_cpython_ptr_for_value(Value::Module(module)),
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
pub unsafe extern "C" fn PyImport_AddModule(name: *const c_char) -> *mut c_void {
    unsafe { PyImport_AddModuleRef(name) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_GetModule(name: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let module_name = match cpython_module_name_from_object(context, name, "PyImport_GetModule")
        {
            Ok(name) => name,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        if context.vm.is_null() {
            context.set_error("PyImport_GetModule missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for the active context lifetime.
        let vm = unsafe { &mut *context.vm };
        vm.refresh_sys_modules_dict();
        let Some(modules_dict) = vm.sys_dict_obj("modules") else {
            context.set_error("unable to get sys.modules");
            return std::ptr::null_mut();
        };
        match dict_get_value(&modules_dict, &Value::Str(module_name)) {
            Some(value) => context.alloc_cpython_ptr_for_value(value),
            None => std::ptr::null_mut(),
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ImportModuleNoBlock(name: *const c_char) -> *mut c_void {
    const DEPRECATION_MESSAGE: &[u8] = b"PyImport_ImportModuleNoBlock() is deprecated and scheduled for removal in Python 3.15. Use PyImport_ImportModule() instead.\0";
    let warning_status = unsafe {
        PyErr_WarnEx(
            std::ptr::addr_of_mut!(PyExc_DeprecationWarning).cast(),
            DEPRECATION_MESSAGE.as_ptr().cast(),
            1,
        )
    };
    if warning_status != 0 {
        return std::ptr::null_mut();
    }
    unsafe { PyImport_ImportModule(name) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ImportModuleLevelObject(
    name: *mut c_void,
    globals: *mut c_void,
    locals: *mut c_void,
    fromlist: *mut c_void,
    level: i32,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyImport_ImportModuleLevelObject missing VM context");
            return std::ptr::null_mut();
        }
        let module_name = match cpython_optional_value_from_ptr(context, name, "module name") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let globals_value = match cpython_optional_value_from_ptr(context, globals, "globals") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let locals_value = match cpython_optional_value_from_ptr(context, locals, "locals") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let fromlist_value = match cpython_optional_value_from_ptr(context, fromlist, "fromlist") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for the active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let args = vec![
            module_name,
            globals_value,
            locals_value,
            fromlist_value,
            Value::Int(level as i64),
        ];
        match vm.call_internal(
            Value::Builtin(BuiltinFunction::Import),
            args,
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("import module level call failed")
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
pub unsafe extern "C" fn PyImport_ImportModuleLevel(
    name: *const c_char,
    globals: *mut c_void,
    locals: *mut c_void,
    fromlist: *mut c_void,
    level: i32,
) -> *mut c_void {
    let module_name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let name_obj = context.alloc_cpython_ptr_for_value(Value::Str(module_name));
        unsafe { PyImport_ImportModuleLevelObject(name_obj, globals, locals, fromlist, level) }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ReloadModule(module: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if module.is_null() {
            context.set_error("PyImport_ReloadModule expected module object");
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            context.set_error("PyImport_ReloadModule missing VM context");
            return std::ptr::null_mut();
        }
        let module_value = match context.cpython_value_from_ptr_or_proxy(module) {
            Some(value) => value,
            None => {
                context.set_error("PyImport_ReloadModule received unknown module pointer");
                return std::ptr::null_mut();
            }
        };
        let module_name = match &module_value {
            Value::Module(module_obj) => match &*module_obj.kind() {
                Object::Module(module_data) => module_data.name.clone(),
                _ => String::new(),
            },
            _ => {
                context.set_error("PyImport_ReloadModule expected module object");
                return std::ptr::null_mut();
            }
        };
        if module_name.is_empty() {
            context.set_error("PyImport_ReloadModule could not resolve module name");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_import_module(vec![Value::Str(module_name)], HashMap::new()) {
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
    let result = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyContextVar_New missing VM context");
            return std::ptr::null_mut();
        }
        let default = if default_value.is_null() {
            Value::None
        } else {
            context.pin_capsule_allocation_for_vm(default_value);
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
            (Value::Str("name".to_string()), Value::Str(name.clone())),
            (Value::Str("default".to_string()), default),
        ]);
        let token = Box::into_raw(Box::new(0u8));
        vm.extension_contextvar_allocations.push(token);
        vm.extension_contextvar_registry
            .insert(token as usize, dict.clone());
        token.cast()
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    });
    if std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some() {
        eprintln!(
            "[numpy-init] PyContextVar_New name={} default_ptr={:p} result={:p}",
            name, default_value, result
        );
    }
    result
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
        let trace_contextvar = std::env::var_os("PYRS_TRACE_CPY_CONTEXTVAR").is_some();
        // Prefer explicit default value if provided.
        let resolved = if !default_value.is_null() {
            context.cpython_value_from_ptr(default_value)
        } else {
            let var_value = context.cpython_value_from_ptr(var).or_else(|| {
                if context.vm.is_null() {
                    None
                } else {
                    // SAFETY: VM pointer is valid for active context lifetime.
                    let vm = unsafe { &mut *context.vm };
                    vm.extension_contextvar_registry
                        .get(&(var as usize))
                        .cloned()
                }
            });
            let Some(var_value) = var_value else {
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
            if trace_contextvar {
                eprintln!(
                    "[cpy-contextvar] get var={:p} default={:p} -> out={:p}",
                    var, default_value, ptr
                );
            }
            // SAFETY: caller provided writable out pointer.
            unsafe { *out_value = ptr };
        } else {
            if trace_contextvar {
                eprintln!(
                    "[cpy-contextvar] get var={:p} default={:p} -> out=<null>",
                    var, default_value
                );
            }
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
        let trace_contextvar = std::env::var_os("PYRS_TRACE_CPY_CONTEXTVAR").is_some();
        let var_value = context.cpython_value_from_ptr(var).or_else(|| {
            if context.vm.is_null() {
                None
            } else {
                // SAFETY: VM pointer is valid for active context lifetime.
                let vm = unsafe { &mut *context.vm };
                vm.extension_contextvar_registry
                    .get(&(var as usize))
                    .cloned()
            }
        });
        let Some(var_value) = var_value else {
            context.set_error("PyContextVar_Set received unknown var pointer");
            return None;
        };
        let Some(new_value) = context.cpython_value_from_ptr(value) else {
            context.set_error("PyContextVar_Set received unknown value pointer");
            return None;
        };
        context.pin_capsule_allocation_for_vm(value);
        let Value::Dict(dict_obj) = var_value else {
            context.set_error("PyContextVar_Set expected context-var object");
            return None;
        };
        let Object::Dict(_) = &mut *dict_obj.kind_mut() else {
            context.set_error("PyContextVar_Set context-var storage invalid");
            return None;
        };
        let _ = dict_set_value_checked(&dict_obj, Value::Str("value".to_string()), new_value);
        let token = context.alloc_cpython_ptr_for_value(Value::None);
        if trace_contextvar {
            eprintln!(
                "[cpy-contextvar] set var={:p} value={:p} -> token={:p}",
                var, value, token
            );
        }
        Some(token)
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
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return 0;
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            return 0;
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match cpython_value_is_iterator_for_capi(vm, &value) {
            Ok(true) => 1,
            Ok(false) | Err(_) => 0,
        }
    })
    .unwrap_or(0)
}

fn cpython_exception_value_attr(value: &Value) -> Option<Value> {
    match value {
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.attrs.get("value").cloned(),
            _ => None,
        },
        Value::Exception(exception) => exception.attrs.borrow().get("value").cloned(),
        _ => None,
    }
}

fn cpython_value_is_exception_name(vm: &Vm, value: &Value, expected: &str) -> bool {
    match value {
        Value::Exception(exception) => exception.name == expected,
        Value::ExceptionType(name) => name == expected,
        Value::Instance(instance) => vm
            .exception_class_name_for_instance(instance)
            .is_some_and(|name| name == expected),
        _ => false,
    }
}

fn cpython_active_exception_is(vm: &Vm, expected: &str) -> bool {
    vm.frames
        .last()
        .and_then(|frame| frame.active_exception.as_ref())
        .is_some_and(|value| cpython_value_is_exception_name(vm, value, expected))
}

fn cpython_clear_active_exception(vm: &mut Vm) {
    if let Some(frame) = vm.frames.last_mut() {
        frame.active_exception = None;
    }
}

fn cpython_value_is_stop_iteration(vm: &Vm, value: &Value) -> bool {
    cpython_value_is_exception_name(vm, value, "StopIteration")
}

fn cpython_stop_iteration_value_from_active_exception(vm: &Vm) -> Option<Value> {
    let active = vm
        .frames
        .last()
        .and_then(|frame| frame.active_exception.clone())?;
    if !cpython_value_is_stop_iteration(vm, &active) {
        return None;
    }
    Some(cpython_exception_value_attr(&active).unwrap_or(Value::None))
}

fn cpython_value_is_iterator_for_capi(vm: &mut Vm, value: &Value) -> Result<bool, RuntimeError> {
    match value {
        Value::Iterator(_) => Ok(true),
        Value::Generator(_) => vm.ensure_sync_iterator_target(value).map(|_| true),
        Value::Instance(_) => Ok(vm.lookup_bound_special_method(value, "__next__")?.is_some()),
        _ => Ok(false),
    }
}

fn cpython_iter_next_for_capi(vm: &mut Vm, iter: &Value) -> Result<Option<Value>, RuntimeError> {
    if !cpython_value_is_iterator_for_capi(vm, iter)? {
        return Err(RuntimeError::new("expected an iterator"));
    }
    match vm.next_from_iterator_value(iter)? {
        GeneratorResumeOutcome::Yield(value) => Ok(Some(value)),
        GeneratorResumeOutcome::Complete(_) => Ok(None),
        GeneratorResumeOutcome::PropagatedException => {
            if cpython_stop_iteration_value_from_active_exception(vm).is_some() {
                Ok(None)
            } else {
                Err(vm.runtime_error_from_active_exception("iteration failed"))
            }
        }
    }
}

fn cpython_iter_send_for_capi(
    vm: &mut Vm,
    iter: Value,
    arg: Value,
) -> Result<(i32, Value), RuntimeError> {
    if let Value::Generator(generator) = &iter {
        vm.ensure_sync_iterator_target(&iter)?;
        let sent = if arg == Value::None { None } else { Some(arg) };
        return match vm.resume_generator(generator, sent, None, GeneratorResumeKind::Next)? {
            GeneratorResumeOutcome::Yield(value) => Ok((1, value)),
            GeneratorResumeOutcome::Complete(value) => Ok((0, value)),
            GeneratorResumeOutcome::PropagatedException => {
                if let Some(value) = cpython_stop_iteration_value_from_active_exception(vm) {
                    Ok((0, value))
                } else {
                    Err(vm.runtime_error_from_active_exception("PyIter_Send failed"))
                }
            }
        };
    }

    if arg == Value::None && cpython_value_is_iterator_for_capi(vm, &iter)? {
        return match cpython_iter_next_for_capi(vm, &iter)? {
            Some(value) => Ok((1, value)),
            None => Ok((0, Value::None)),
        };
    }

    let send_method =
        vm.builtin_getattr(vec![iter, Value::Str("send".to_string())], HashMap::new())?;
    match vm.call_internal(send_method, vec![arg], HashMap::new()) {
        Ok(InternalCallOutcome::Value(value)) => Ok((1, value)),
        Ok(InternalCallOutcome::CallerExceptionHandled) => {
            if let Some(value) = cpython_stop_iteration_value_from_active_exception(vm) {
                Ok((0, value))
            } else {
                Err(vm.runtime_error_from_active_exception("PyIter_Send failed"))
            }
        }
        Err(err) => {
            if let Some(value) = cpython_stop_iteration_value_from_active_exception(vm) {
                Ok((0, value))
            } else {
                Err(err)
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIter_NextItem(object: *mut c_void, item: *mut *mut c_void) -> i32 {
    if item.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe { *item = std::ptr::null_mut() };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyIter_NextItem missing VM context");
            return -1;
        }
        let Some(iter) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyIter_NextItem unknown iterator pointer");
            return -1;
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        if !cpython_value_is_iterator_for_capi(vm, &iter).unwrap_or(false) {
            let message = format!(
                "expected an iterator, got '{}'",
                vm.value_type_name_for_error(&iter)
            );
            let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
            context.set_error_state(
                unsafe { PyExc_TypeError },
                pvalue,
                std::ptr::null_mut(),
                message,
            );
            return -1;
        }
        match cpython_iter_next_for_capi(vm, &iter) {
            Ok(Some(next)) => {
                unsafe { *item = context.alloc_cpython_ptr_for_value(next) };
                1
            }
            Ok(None) => 0,
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
pub unsafe extern "C" fn PyIter_Send(
    iter: *mut c_void,
    arg: *mut c_void,
    result: *mut *mut c_void,
) -> i32 {
    if arg.is_null() || result.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe { *result = std::ptr::null_mut() };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyIter_Send missing VM context");
            return -1;
        }
        let Some(iter_value) = context.cpython_value_from_ptr_or_proxy(iter) else {
            context.set_error("PyIter_Send unknown iterator pointer");
            return -1;
        };
        let Some(arg_value) = context.cpython_value_from_ptr_or_proxy(arg) else {
            context.set_error("PyIter_Send unknown argument pointer");
            return -1;
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match cpython_iter_send_for_capi(vm, iter_value, arg_value) {
            Ok((status, value)) => {
                unsafe { *result = context.alloc_cpython_ptr_for_value(value) };
                status
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
pub unsafe extern "C" fn PyIter_Next(object: *mut c_void) -> *mut c_void {
    match with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyIter_Next missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyIter_Next unknown iterator pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let next = match cpython_iter_next_for_capi(vm, &value) {
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
    destructor: Option<unsafe extern "C" fn(*mut c_void)>,
) -> *mut c_void {
    if std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some() {
        let name_text = if name.is_null() {
            "<null>".to_string()
        } else {
            // SAFETY: capsule name pointer is expected to be NUL-terminated.
            unsafe { CStr::from_ptr(name) }
                .to_str()
                .map(|text| text.to_string())
                .unwrap_or_else(|_| "<invalid>".to_string())
        };
        eprintln!(
            "[numpy-init] PyCapsule_New name={} pointer={:p}",
            name_text, pointer
        );
    }
    with_active_cpython_context_mut(|context| {
        match context.capsule_new(pointer, name, destructor) {
            Ok(handle) => context.alloc_cpython_ptr_for_handle(handle),
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
pub unsafe extern "C" fn PyCapsule_GetPointer(
    capsule: *mut c_void,
    name: *const c_char,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            match unsafe { cpython_external_capsule_pointer(context, capsule, name) } {
                Ok(Some(pointer)) => return pointer,
                Ok(None) => {
                    if std::env::var_os("PYRS_TRACE_CPY_CAPSULE").is_some() {
                        let requested_name = if name.is_null() {
                            "<null>".to_string()
                        } else {
                            unsafe { CStr::from_ptr(name) }
                                .to_str()
                                .map(|value| value.to_string())
                                .unwrap_or_else(|_| "<invalid utf8>".to_string())
                        };
                        eprintln!(
                            "[cpy-capsule] get_pointer unknown ptr={:p} requested_name={}",
                            capsule, requested_name
                        );
                    }
                    context.set_error("PyCapsule_GetPointer received unknown object pointer");
                    return std::ptr::null_mut();
                }
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
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
            match unsafe { cpython_external_capsule_pointer(context, capsule, name) } {
                Ok(Some(pointer)) => return pointer,
                Ok(None) => {}
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            }
            let proxy_ptr = context.objects.get(&handle).and_then(|slot| {
                let Value::Class(class_obj) = &slot.value else {
                    return None;
                };
                let Object::Class(class_data) = &*class_obj.kind() else {
                    return None;
                };
                match class_data.attrs.get("__pyrs_cpython_proxy_ptr__") {
                    Some(Value::Int(raw)) if *raw >= 0 => Some(*raw as usize as *mut c_void),
                    _ => None,
                }
            });
            if let Some(proxy_ptr) = proxy_ptr {
                match unsafe { cpython_external_capsule_pointer(context, proxy_ptr, name) } {
                    Ok(Some(pointer)) => return pointer,
                    Ok(None) => {}
                    Err(err) => {
                        context.set_error(err);
                        return std::ptr::null_mut();
                    }
                }
            }
            context.set_error(format!("invalid capsule handle {}", handle));
            return std::ptr::null_mut();
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
pub unsafe extern "C" fn PyCapsule_GetName(capsule: *mut c_void) -> *const c_char {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_GetName received unknown object pointer");
            return std::ptr::null();
        };
        match context.capsule_get_name_ptr(handle) {
            Ok(name) => name,
            Err(err) => {
                context.set_error(err);
                std::ptr::null()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_SetPointer(capsule: *mut c_void, pointer: *mut c_void) -> i32 {
    if pointer.is_null() {
        cpython_set_error("PyCapsule_SetPointer called with null pointer");
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_SetPointer received unknown object pointer");
            return -1;
        };
        match context.capsule_set_pointer(handle, pointer) {
            Ok(()) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
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
pub unsafe extern "C" fn PyCapsule_GetDestructor(
    capsule: *mut c_void,
) -> Option<unsafe extern "C" fn(*mut c_void)> {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_GetDestructor received unknown object pointer");
            return None;
        };
        match context.capsule_get_cpython_destructor(handle) {
            Ok(destructor) => destructor,
            Err(err) => {
                context.set_error(err);
                None
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        None
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_SetDestructor(
    capsule: *mut c_void,
    destructor: Option<unsafe extern "C" fn(*mut c_void)>,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            context.set_error("PyCapsule_SetDestructor received unknown object pointer");
            return -1;
        };
        match context.capsule_set_cpython_destructor(handle, destructor) {
            Ok(()) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
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
            Ok(()) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
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
            Ok(()) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
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
    if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_VALID").is_some() {
        eprintln!(
            "[cpy-capsule-valid] enter ptr={:p} requested_name={}",
            capsule,
            if name.is_null() {
                "<null>".to_string()
            } else {
                unsafe { CStr::from_ptr(name) }
                    .to_str()
                    .map(|text| text.to_string())
                    .unwrap_or_else(|_| "<invalid utf8>".to_string())
            }
        );
    }
    match with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(capsule) else {
            if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_VALID").is_some() {
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
                    "[cpy-capsule-valid] missing-handle ptr={:p} type={:p} type_name={} requested_name={}",
                    capsule,
                    raw_type,
                    raw_type_name,
                    if name.is_null() {
                        "<null>".to_string()
                    } else {
                        unsafe { CStr::from_ptr(name) }
                            .to_str()
                            .map(|text| text.to_string())
                            .unwrap_or_else(|_| "<invalid utf8>".to_string())
                    }
                );
            }
            return 0;
        };
        match context.capsule_is_valid(handle, name) {
            Ok(valid) => {
                if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_VALID").is_some() {
                    eprintln!(
                        "[cpy-capsule-valid] handle={} ptr={:p} valid={} requested_name={}",
                        handle,
                        capsule,
                        valid,
                        if name.is_null() {
                            "<null>".to_string()
                        } else {
                            unsafe { CStr::from_ptr(name) }
                                .to_str()
                                .map(|text| text.to_string())
                                .unwrap_or_else(|_| "<invalid utf8>".to_string())
                        }
                    );
                }
                valid
            }
            Err(err) => {
                if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_VALID").is_some() {
                    eprintln!(
                        "[cpy-capsule-valid] handle={} ptr={:p} error={}",
                        handle, capsule, err
                    );
                }
                0
            }
        }
    }) {
        Ok(value) => value,
        Err(err) => {
            if std::env::var_os("PYRS_TRACE_CPY_CAPSULE_VALID").is_some() {
                eprintln!(
                    "[cpy-capsule-valid] no-active-context ptr={:p} requested_name={} err={}",
                    capsule,
                    if name.is_null() {
                        "<null>".to_string()
                    } else {
                        unsafe { CStr::from_ptr(name) }
                            .to_str()
                            .map(|text| text.to_string())
                            .unwrap_or_else(|_| "<invalid utf8>".to_string())
                    },
                    err
                );
            }
            0
        }
    }
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
    let trace_lists = std::env::var_os("PYRS_TRACE_CPY_LIST").is_some();
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
        let ptr = context.alloc_cpython_ptr_for_value(Value::List(list));
        if trace_lists {
            eprintln!("[cpy-list-new] size={} ptr={:p}", size, ptr);
        }
        ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Size(list: *mut c_void) -> isize {
    let trace_lists = std::env::var_os("PYRS_TRACE_CPY_LIST").is_some();
    if trace_lists {
        let _ = with_active_cpython_context_mut(|context| {
            if context.owns_cpython_allocation_ptr(list) {
                // SAFETY: owned list pointers use `CpythonListCompatObject` layout.
                let raw = unsafe { list.cast::<CpythonListCompatObject>().as_ref() };
                if let Some(raw) = raw {
                    eprintln!(
                        "[cpy-list-size-raw] ptr={:p} ob_size={} ob_item={:p} allocated={}",
                        list, raw.ob_base.ob_size, raw.ob_item, raw.allocated
                    );
                }
            }
        });
    }
    match cpython_value_from_ptr(list) {
        Ok(Value::List(list_obj)) => match &*list_obj.kind() {
            Object::List(values) => {
                if trace_lists {
                    eprintln!("[cpy-list-size] ptr={:p} len={}", list, values.len());
                }
                values.len() as isize
            }
            _ => {
                if trace_lists {
                    eprintln!("[cpy-list-size] ptr={:p} invalid list storage", list);
                }
                cpython_set_error("PyList_Size encountered invalid list storage");
                -1
            }
        },
        Ok(_) => {
            if trace_lists {
                eprintln!("[cpy-list-size] ptr={:p} non-list object", list);
            }
            cpython_set_error("PyList_Size expected list object");
            -1
        }
        Err(err) => {
            if trace_lists {
                eprintln!("[cpy-list-size] ptr={:p} lookup error: {}", list, err);
            }
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Append(list: *mut c_void, item: *mut c_void) -> i32 {
    let trace_lists = std::env::var_os("PYRS_TRACE_CPY_LIST").is_some();
    with_active_cpython_context_mut(|context| {
        let item_value = match context.cpython_value_from_ptr_or_proxy(item) {
            Some(value) => value,
            None => {
                if trace_lists {
                    eprintln!(
                        "[cpy-list-append] list={:p} item={:p} unknown item pointer",
                        list, item
                    );
                }
                context.set_error("PyList_Append received unknown item pointer");
                return -1;
            }
        };
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            if trace_lists {
                eprintln!("[cpy-list-append] list={:p} unknown list pointer", list);
            }
            context.set_error("PyList_Append received unknown list pointer");
            return -1;
        };
        {
            let Some(slot) = context.objects.get_mut(&handle) else {
                if trace_lists {
                    eprintln!(
                        "[cpy-list-append] list={:p} handle={} missing slot",
                        list, handle
                    );
                }
                context.set_error("PyList_Append list handle is not available");
                return -1;
            };
            let Value::List(list_obj) = &mut slot.value else {
                if trace_lists {
                    eprintln!(
                        "[cpy-list-append] list={:p} handle={} non-list slot",
                        list, handle
                    );
                }
                context.set_error("PyList_Append expected list object");
                return -1;
            };
            let Object::List(values) = &mut *list_obj.kind_mut() else {
                if trace_lists {
                    eprintln!(
                        "[cpy-list-append] list={:p} handle={} invalid list storage",
                        list, handle
                    );
                }
                context.set_error("PyList_Append encountered invalid list storage");
                return -1;
            };
            if trace_lists {
                eprintln!(
                    "[cpy-list-append] list={:p} handle={} before_len={} item={}",
                    list,
                    handle,
                    values.len(),
                    cpython_debug_compare_value(&item_value)
                );
            }
            values.push(item_value);
            if trace_lists {
                eprintln!(
                    "[cpy-list-append] list={:p} handle={} after_len={}",
                    list,
                    handle,
                    values.len()
                );
            }
        }
        // Keep owned CPython list storage (`ob_size` / `ob_item`) synchronized for native callers
        // that access list internals directly between C-API calls.
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_GetItem(list: *mut c_void, index: isize) -> *mut c_void {
    let trace_lists = std::env::var_os("PYRS_TRACE_CPY_LIST").is_some();
    with_active_cpython_context_mut(|context| {
        if index < 0 {
            context.set_error("PyList_GetItem index out of range");
            return std::ptr::null_mut();
        }
        if context.owns_cpython_allocation_ptr(list) {
            // SAFETY: owned list pointers use `CpythonListCompatObject` layout.
            let raw = unsafe { list.cast::<CpythonListCompatObject>().as_ref() };
            let Some(raw) = raw else {
                context.set_error("PyList_GetItem received invalid list pointer");
                return std::ptr::null_mut();
            };
            let len = raw.ob_base.ob_size.max(0) as usize;
            if (index as usize) >= len || raw.ob_item.is_null() {
                context.set_error("PyList_GetItem index out of range");
                return std::ptr::null_mut();
            }
            // SAFETY: `ob_item` points to at least `len` entries.
            let item = unsafe { *raw.ob_item.add(index as usize) };
            if item.is_null() {
                context.set_error("PyList_GetItem encountered null list slot");
                return std::ptr::null_mut();
            }
            return item;
        }
        let Some(value) = context.cpython_value_from_ptr(list) else {
            if trace_lists {
                eprintln!(
                    "[cpy-list-get-item] ptr={:p} index={} unknown list pointer",
                    list, index
                );
            }
            context.set_error("PyList_GetItem received unknown list pointer");
            return std::ptr::null_mut();
        };
        let Value::List(list_obj) = value else {
            context.set_error("PyList_GetItem expected list object");
            return std::ptr::null_mut();
        };
        let Object::List(values) = &*list_obj.kind() else {
            context.set_error("PyList_GetItem encountered invalid list storage");
            return std::ptr::null_mut();
        };
        let idx = index as usize;
        if idx >= values.len() {
            context.set_error("PyList_GetItem index out of range");
            return std::ptr::null_mut();
        }
        context.alloc_cpython_ptr_for_value(values[idx].clone())
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_SetItem(list: *mut c_void, index: isize, item: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let item_handle = context.cpython_handle_from_ptr(item);
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            if let Some(item_handle) = item_handle {
                let _ = context.decref(item_handle);
            }
            context.set_error("PyList_SetItem received unknown list pointer");
            return -1;
        };
        if index < 0 {
            if let Some(item_handle) = item_handle {
                let _ = context.decref(item_handle);
            }
            context.set_error("PyList_SetItem index out of range");
            return -1;
        }
        let item_value = match context.cpython_value_from_ptr_or_proxy(item) {
            Some(value) => value,
            None => {
                if let Some(item_handle) = item_handle {
                    let _ = context.decref(item_handle);
                }
                context.set_error("PyList_SetItem received unknown item pointer");
                return -1;
            }
        };
        let mut ok = false;
        {
            let Some(slot) = context.objects.get_mut(&handle) else {
                if let Some(item_handle) = item_handle {
                    let _ = context.decref(item_handle);
                }
                context.set_error("PyList_SetItem list handle is not available");
                return -1;
            };
            let Value::List(list_obj) = &mut slot.value else {
                if let Some(item_handle) = item_handle {
                    let _ = context.decref(item_handle);
                }
                context.set_error("PyList_SetItem expected list object");
                return -1;
            };
            let Object::List(values) = &mut *list_obj.kind_mut() else {
                if let Some(item_handle) = item_handle {
                    let _ = context.decref(item_handle);
                }
                context.set_error("PyList_SetItem encountered invalid list storage");
                return -1;
            };
            let idx = index as usize;
            if idx < values.len() {
                values[idx] = item_value;
                ok = true;
            }
        }
        if let Some(item_handle) = item_handle {
            let _ = context.decref(item_handle);
        }
        if !ok {
            context.set_error("PyList_SetItem index out of range");
            return -1;
        }
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Insert(list: *mut c_void, index: isize, item: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            context.set_error("PyList_Insert received unknown list pointer");
            return -1;
        };
        let item_value = match context.cpython_value_from_ptr_or_proxy(item) {
            Some(value) => value,
            None => {
                context.set_error("PyList_Insert received unknown item pointer");
                return -1;
            }
        };
        {
            let Some(slot) = context.objects.get_mut(&handle) else {
                context.set_error("PyList_Insert list handle is not available");
                return -1;
            };
            let Value::List(list_obj) = &mut slot.value else {
                context.set_error("PyList_Insert expected list object");
                return -1;
            };
            let Object::List(values) = &mut *list_obj.kind_mut() else {
                context.set_error("PyList_Insert encountered invalid list storage");
                return -1;
            };
            let insert_at = if index <= 0 {
                0
            } else {
                (index as usize).min(values.len())
            };
            values.insert(insert_at, item_value);
        }
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_GetSlice(
    list: *mut c_void,
    low: isize,
    high: isize,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_GetSlice missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr(list) else {
            context.set_error("PyList_GetSlice received unknown list pointer");
            return std::ptr::null_mut();
        };
        let Value::List(list_obj) = value else {
            context.set_error("PyList_GetSlice expected list object");
            return std::ptr::null_mut();
        };
        let Object::List(values) = &*list_obj.kind() else {
            context.set_error("PyList_GetSlice encountered invalid list storage");
            return std::ptr::null_mut();
        };
        let len = values.len() as isize;
        let mut start = if low < 0 { low + len } else { low };
        let mut end = if high < 0 { high + len } else { high };
        start = start.clamp(0, len);
        end = end.clamp(0, len);
        let slice = if end >= start {
            values[start as usize..end as usize].to_vec()
        } else {
            Vec::new()
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let result = vm.heap.alloc(Object::List(slice));
        context.alloc_cpython_ptr_for_value(Value::List(result))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_SetSlice(
    list: *mut c_void,
    low: isize,
    high: isize,
    itemlist: *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_SetSlice missing VM context");
            return -1;
        }
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            context.set_error("PyList_SetSlice received unknown list pointer");
            return -1;
        };
        let replacement = if itemlist.is_null() {
            Vec::new()
        } else {
            let replacement_value = match context.cpython_value_from_ptr_or_proxy(itemlist) {
                Some(value) => value,
                None => {
                    context.set_error("PyList_SetSlice received unknown itemlist pointer");
                    return -1;
                }
            };
            // SAFETY: VM pointer is valid for context lifetime.
            let vm = unsafe { &mut *context.vm };
            match replacement_value {
                Value::List(list_obj) => match &*list_obj.kind() {
                    Object::List(values) => values.clone(),
                    _ => {
                        context.set_error("PyList_SetSlice encountered invalid replacement list");
                        return -1;
                    }
                },
                Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                    Object::Tuple(values) => values.clone(),
                    _ => {
                        context.set_error("PyList_SetSlice encountered invalid replacement tuple");
                        return -1;
                    }
                },
                other => match vm.collect_iterable_values(other) {
                    Ok(values) => values,
                    Err(err) => {
                        context.set_error(err.message);
                        return -1;
                    }
                },
            }
        };
        {
            let Some(slot) = context.objects.get_mut(&handle) else {
                context.set_error("PyList_SetSlice list handle is not available");
                return -1;
            };
            let Value::List(list_obj) = &mut slot.value else {
                context.set_error("PyList_SetSlice expected list object");
                return -1;
            };
            let Object::List(values) = &mut *list_obj.kind_mut() else {
                context.set_error("PyList_SetSlice encountered invalid list storage");
                return -1;
            };
            let len = values.len() as isize;
            let mut start = if low < 0 { low + len } else { low };
            let mut end = if high < 0 { high + len } else { high };
            start = start.clamp(0, len);
            end = end.clamp(0, len);
            let start = start as usize;
            let end = end.max(start as isize) as usize;
            values.splice(start..end, replacement);
        }
        context.sync_cpython_storage_from_value(handle);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Sort(list: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_Sort missing VM context");
            return -1;
        }
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            context.set_error("PyList_Sort received unknown list pointer");
            return -1;
        };
        let list_obj = {
            let Some(slot) = context.objects.get(&handle) else {
                context.set_error("PyList_Sort list handle is not available");
                return -1;
            };
            match &slot.value {
                Value::List(list_obj) => list_obj.clone(),
                _ => {
                    context.set_error("PyList_Sort expected list object");
                    return -1;
                }
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::ListSort,
            list_obj,
            Vec::new(),
            HashMap::new(),
        ) {
            Ok(NativeCallResult::Value(_)) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
            Ok(NativeCallResult::PropagatedException) => {
                let err = vm.runtime_error_from_active_exception("list.sort() failed");
                context.set_error(err.message);
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
pub unsafe extern "C" fn PyList_Reverse(list: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyList_Reverse missing VM context");
            return -1;
        }
        let Some(handle) = context.cpython_handle_from_ptr(list) else {
            context.set_error("PyList_Reverse received unknown list pointer");
            return -1;
        };
        let list_obj = {
            let Some(slot) = context.objects.get(&handle) else {
                context.set_error("PyList_Reverse list handle is not available");
                return -1;
            };
            match &slot.value {
                Value::List(list_obj) => list_obj.clone(),
                _ => {
                    context.set_error("PyList_Reverse expected list object");
                    return -1;
                }
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::ListReverse,
            list_obj,
            Vec::new(),
            HashMap::new(),
        ) {
            Ok(NativeCallResult::Value(_)) => {
                context.sync_cpython_storage_from_value(handle);
                0
            }
            Ok(NativeCallResult::PropagatedException) => {
                let err = vm.runtime_error_from_active_exception("list.reverse() failed");
                context.set_error(err.message);
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
pub unsafe extern "C" fn PyList_GetItemRef(list: *mut c_void, index: isize) -> *mut c_void {
    let trace_lists = std::env::var_os("PYRS_TRACE_CPY_LIST").is_some();
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr(list) else {
            if trace_lists {
                eprintln!(
                    "[cpy-list-get-item-ref] ptr={:p} index={} unknown list pointer",
                    list, index
                );
            }
            context.set_error("PyList_GetItemRef received unknown list pointer");
            return std::ptr::null_mut();
        };
        let Value::List(list_obj) = value else {
            if trace_lists {
                eprintln!(
                    "[cpy-list-get-item-ref] ptr={:p} index={} non-list object",
                    list, index
                );
            }
            context.set_error("PyList_GetItemRef expected list object");
            return std::ptr::null_mut();
        };
        let Object::List(values) = &*list_obj.kind() else {
            if trace_lists {
                eprintln!(
                    "[cpy-list-get-item-ref] ptr={:p} index={} invalid list storage",
                    list, index
                );
            }
            context.set_error("PyList_GetItemRef encountered invalid list storage");
            return std::ptr::null_mut();
        };
        if trace_lists {
            eprintln!(
                "[cpy-list-get-item-ref] ptr={:p} index={} len={}",
                list,
                index,
                values.len()
            );
        }
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
pub unsafe extern "C" fn PySet_New(iterable: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySet_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let value = if iterable.is_null() {
            vm.heap.alloc_set(Vec::new())
        } else {
            let source = match context.cpython_value_from_ptr_or_proxy(iterable) {
                Some(value) => value,
                None => {
                    context.set_error("PySet_New received unknown iterable pointer");
                    return std::ptr::null_mut();
                }
            };
            match vm.builtin_set(vec![source], HashMap::new()) {
                Ok(value) => value,
                Err(err) => {
                    context.set_error(err.message);
                    return std::ptr::null_mut();
                }
            }
        };
        context.alloc_cpython_ptr_for_value(value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrozenSet_New(iterable: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyFrozenSet_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let value = if iterable.is_null() {
            vm.heap.alloc_frozenset(Vec::new())
        } else {
            let source = match context.cpython_value_from_ptr_or_proxy(iterable) {
                Some(value) => value,
                None => {
                    context.set_error("PyFrozenSet_New received unknown iterable pointer");
                    return std::ptr::null_mut();
                }
            };
            match vm.builtin_frozenset(vec![source], HashMap::new()) {
                Ok(value) => value,
                Err(err) => {
                    context.set_error(err.message);
                    return std::ptr::null_mut();
                }
            }
        };
        context.alloc_cpython_ptr_for_value(value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Size(anyset: *mut c_void) -> isize {
    match cpython_value_from_ptr(anyset) {
        Ok(Value::Set(set_obj)) => match &*set_obj.kind() {
            Object::Set(values) => values.len() as isize,
            _ => {
                cpython_set_error("PySet_Size encountered invalid set storage");
                -1
            }
        },
        Ok(Value::FrozenSet(set_obj)) => match &*set_obj.kind() {
            Object::FrozenSet(values) => values.len() as isize,
            _ => {
                cpython_set_error("PySet_Size encountered invalid frozenset storage");
                -1
            }
        },
        Ok(_) => {
            cpython_set_error("PySet_Size expected set object");
            -1
        }
        Err(err) => {
            cpython_set_error(err);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Contains(anyset: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySet_Contains missing VM context");
            return -1;
        }
        let receiver = match context.cpython_value_from_ptr(anyset) {
            Some(Value::Set(set_obj)) | Some(Value::FrozenSet(set_obj)) => set_obj,
            Some(_) => {
                context.set_error("PySet_Contains expected set object");
                return -1;
            }
            None => {
                context.set_error("PySet_Contains received unknown set pointer");
                return -1;
            }
        };
        let key_value = match context.cpython_value_from_ptr_or_proxy(key) {
            Some(value) => value,
            None => {
                context.set_error("PySet_Contains received unknown key pointer");
                return -1;
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::SetContains,
            receiver,
            vec![key_value],
            HashMap::new(),
        ) {
            Ok(NativeCallResult::Value(Value::Bool(true))) => 1,
            Ok(NativeCallResult::Value(Value::Bool(false))) => 0,
            Ok(_) => {
                context.set_error("PySet_Contains returned non-boolean result");
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
pub unsafe extern "C" fn PySet_Add(set: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySet_Add missing VM context");
            return -1;
        }
        let receiver = match context.cpython_value_from_ptr(set) {
            Some(Value::Set(set_obj)) => set_obj,
            Some(_) => {
                context.set_error("PySet_Add expected set object");
                return -1;
            }
            None => {
                context.set_error("PySet_Add received unknown set pointer");
                return -1;
            }
        };
        let key_value = match context.cpython_value_from_ptr_or_proxy(key) {
            Some(value) => value,
            None => {
                context.set_error("PySet_Add received unknown key pointer");
                return -1;
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::SetAdd,
            receiver,
            vec![key_value],
            HashMap::new(),
        ) {
            Ok(_) => 0,
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
pub unsafe extern "C" fn PySet_Discard(set: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySet_Discard missing VM context");
            return -1;
        }
        let receiver = match context.cpython_value_from_ptr(set) {
            Some(Value::Set(set_obj)) => set_obj,
            Some(_) => {
                context.set_error("PySet_Discard expected set object");
                return -1;
            }
            None => {
                context.set_error("PySet_Discard received unknown set pointer");
                return -1;
            }
        };
        let key_value = match context.cpython_value_from_ptr_or_proxy(key) {
            Some(value) => value,
            None => {
                context.set_error("PySet_Discard received unknown key pointer");
                return -1;
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::SetDiscard,
            receiver,
            vec![key_value],
            HashMap::new(),
        ) {
            Ok(_) => 0,
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
pub unsafe extern "C" fn PySet_Clear(set: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(set) else {
            context.set_error("PySet_Clear received unknown set pointer");
            return -1;
        };
        let Some(slot) = context.objects.get_mut(&handle) else {
            context.set_error("PySet_Clear set handle is not available");
            return -1;
        };
        let Value::Set(set_obj) = &mut slot.value else {
            context.set_error("PySet_Clear expected set object");
            return -1;
        };
        let Object::Set(values) = &mut *set_obj.kind_mut() else {
            context.set_error("PySet_Clear encountered invalid set storage");
            return -1;
        };
        values.clear();
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Pop(set: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySet_Pop missing VM context");
            return std::ptr::null_mut();
        }
        let receiver = match context.cpython_value_from_ptr(set) {
            Some(Value::Set(set_obj)) => set_obj,
            Some(_) => {
                context.set_error("PySet_Pop expected set object");
                return std::ptr::null_mut();
            }
            None => {
                context.set_error("PySet_Pop received unknown set pointer");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.call_native_method(
            NativeMethodKind::SetPop,
            receiver,
            Vec::new(),
            HashMap::new(),
        ) {
            Ok(NativeCallResult::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(_) => {
                context.set_error("PySet_Pop returned invalid result");
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
        let module_target = context.module_dict_module_for_ptr(dict);
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
        if std::env::var_os("PYRS_TRACE_CPY_DICT").is_some() {
            eprintln!(
                "[cpy-dict-set] dict={:p} key_ptr={:p} key={} value_ptr={:p} value_tag={}",
                dict,
                key,
                cpython_debug_compare_value(&key_value),
                value,
                cpython_value_debug_tag(&item_value)
            );
        }
        match dict_set_value_checked(&dict_obj, key_value.clone(), item_value.clone()) {
            Ok(()) => {
                if let Some(module_obj) = module_target
                    && let Value::Str(name) = key_value
                    && let Object::Module(module_data) = &mut *module_obj.kind_mut()
                {
                    module_data.globals.insert(name, item_value);
                }
                0
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
        if std::env::var_os("PYRS_TRACE_CPY_DICT").is_some() {
            eprintln!(
                "[cpy-dict-get] dict={:p} key_ptr={:p} key={}",
                dict,
                key,
                cpython_debug_compare_value(&key_value)
            );
        }
        let Some(value) = dict_get_value(&dict_obj, &key_value) else {
            if std::env::var_os("PYRS_TRACE_CPY_DICT").is_some() {
                eprintln!("[cpy-dict-get] dict={:p} miss", dict);
            }
            return std::ptr::null_mut();
        };
        if std::env::var_os("PYRS_TRACE_CPY_DICT").is_some() {
            eprintln!(
                "[cpy-dict-get] dict={:p} hit value_tag={}",
                dict,
                cpython_value_debug_tag(&value)
            );
        }
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
    let key_name = unsafe { c_name_to_string(key) }.unwrap_or_else(|_| "<invalid>".to_string());
    let result = unsafe { PyDict_SetItem(dict, key_obj, value) };
    if std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some()
        && matches!(
            key_name.as_str(),
            "_ARRAY_API" | "_UFUNC_API" | "False_" | "True_"
        )
    {
        eprintln!(
            "[numpy-init] PyDict_SetItemString key={} value_ptr={:p} result={}",
            key_name, value, result
        );
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemString(
    dict: *mut c_void,
    key: *const c_char,
) -> *mut c_void {
    let key_name = unsafe { c_name_to_string(key) }.unwrap_or_else(|_| "<invalid>".to_string());
    let trace_key = matches!(
        key_name.as_str(),
        "matmul" | "logical_and" | "logical_or" | "logical_xor"
    );
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        if std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some() && trace_key {
            eprintln!(
                "[numpy-init] PyDict_GetItemString key={} key_obj=<null> dict={:p}",
                key_name, dict
            );
        }
        return std::ptr::null_mut();
    }
    let result = unsafe { PyDict_GetItem(dict, key_obj) };
    if std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some() && trace_key {
        eprintln!(
            "[numpy-init] PyDict_GetItemString key={} dict={:p} result={:p}",
            key_name, dict, result
        );
    }
    result
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
    let key_name = unsafe { c_name_to_string(key) }.unwrap_or_else(|_| "<invalid>".to_string());
    let trace_key = matches!(
        key_name.as_str(),
        "matmul" | "logical_and" | "logical_or" | "logical_xor"
    );
    let value = unsafe { PyDict_GetItemString(dict, key) };
    // SAFETY: caller provided writable pointer.
    unsafe { *out = value };
    let status = if value.is_null() { 0 } else { 1 };
    if std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some() && trace_key {
        eprintln!(
            "[numpy-init] PyDict_GetItemStringRef key={} dict={:p} value={:p} status={}",
            key_name, dict, value, status
        );
    }
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_DelItem(dict: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let module_target = context.module_dict_module_for_ptr(dict);
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
            if let Some(module_obj) = module_target
                && let Value::Str(name) = &key_value
                && let Object::Module(module_data) = &mut *module_obj.kind_mut()
            {
                module_data.globals.remove(name);
            }
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
pub unsafe extern "C" fn PyDict_Clear(dict: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let module_target = context.module_dict_module_for_ptr(dict);
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Clear received unknown dict pointer");
            return;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Clear expected dict object");
            return;
        };
        let mut dict_kind = dict_obj.kind_mut();
        let Object::Dict(values) = &mut *dict_kind else {
            context.set_error("PyDict_Clear encountered invalid dict storage");
            return;
        };
        values.clear();
        if let Some(module_obj) = module_target
            && let Object::Module(module_data) = &mut *module_obj.kind_mut()
        {
            module_data.globals.clear();
        }
    })
    .map_err(cpython_set_error);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Merge(
    dict: *mut c_void,
    other: *mut c_void,
    override_existing: i32,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        let module_target = context.module_dict_module_for_ptr(dict);
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
        let replace_existing = override_existing != 0;
        for (key, value) in source_entries {
            if !replace_existing {
                let should_skip = match dict_contains_key_checked(&dict_obj, &key) {
                    Ok(contains) => contains,
                    Err(err) => {
                        context.set_error(err.message);
                        return -1;
                    }
                };
                if should_skip {
                    continue;
                }
            }
            let module_key = match &key {
                Value::Str(name) => Some(name.clone()),
                _ => None,
            };
            let module_value = value.clone();
            if let Err(err) = dict_set_value_checked(&dict_obj, key, value) {
                context.set_error(err.message);
                return -1;
            }
            if let Some(module_obj) = module_target.as_ref()
                && let Some(name) = module_key.as_ref()
                && let Object::Module(module_data) = &mut *module_obj.kind_mut()
            {
                module_data
                    .globals
                    .insert(name.clone(), module_value.clone());
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
pub unsafe extern "C" fn PyDict_Update(dict: *mut c_void, other: *mut c_void) -> i32 {
    unsafe { PyDict_Merge(dict, other, 1) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_MergeFromSeq2(
    dict: *mut c_void,
    seq2: *mut c_void,
    override_existing: i32,
) -> i32 {
    let seq_value = match cpython_value_from_ptr(seq2) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let mapping_value = match cpython_call_builtin(BuiltinFunction::Dict, vec![seq_value]) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let mapping = cpython_new_ptr_for_value(mapping_value);
    if mapping.is_null() {
        return -1;
    }
    let status = unsafe { PyDict_Merge(dict, mapping, override_existing) };
    unsafe { Py_XDecRef(mapping) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Keys(dict: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyDict_Keys missing VM context");
            return std::ptr::null_mut();
        }
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Keys received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Keys expected dict object");
            return std::ptr::null_mut();
        };
        let entries = match &*dict_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Keys encountered invalid dict storage");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let keys = entries.into_iter().map(|(key, _)| key).collect::<Vec<_>>();
        context.alloc_cpython_ptr_for_value(Value::List(vm.heap.alloc(Object::List(keys))))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Values(dict: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyDict_Values missing VM context");
            return std::ptr::null_mut();
        }
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Values received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Values expected dict object");
            return std::ptr::null_mut();
        };
        let entries = match &*dict_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Values encountered invalid dict storage");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let values = entries
            .into_iter()
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        context.alloc_cpython_ptr_for_value(Value::List(vm.heap.alloc(Object::List(values))))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Items(dict: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyDict_Items missing VM context");
            return std::ptr::null_mut();
        }
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Items received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Items expected dict object");
            return std::ptr::null_mut();
        };
        let entries = match &*dict_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Items encountered invalid dict storage");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let mut items = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let tuple = vm.heap.alloc(Object::Tuple(vec![key, value]));
            items.push(Value::Tuple(tuple));
        }
        context.alloc_cpython_ptr_for_value(Value::List(vm.heap.alloc(Object::List(items))))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
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
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let trace_numpy_attr = std::env::var_os("PYRS_TRACE_NUMPY_INIT").is_some()
        && matches!(
            name.as_str(),
            "__array_finalize__" | "__array_ufunc__" | "__array_function__"
        );
    if !object.is_null() {
        let native_result = with_active_cpython_context_mut(|context| {
            let is_proxy_trace =
                name == "__array_finalize__" && std::env::var_os("PYRS_TRACE_CPY_PROXY_PTRS").is_some();
            let is_owned = context.owns_cpython_allocation_ptr(object);
            let is_known_compat = context.cpython_handle_from_ptr(object).is_some();
            if is_proxy_trace {
                eprintln!(
                    "[cpy-proxy] native getattr check object_ptr={:p} owned={} known_compat={}",
                    object, is_owned, is_known_compat
                );
            }
            if is_known_compat {
                return None;
            }
            // SAFETY: object pointer comes from extension code; type pointer access mirrors CPython.
            let type_ptr = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if is_proxy_trace {
                eprintln!(
                    "[cpy-proxy] native getattr type_ptr={:p}",
                    type_ptr
                );
            }
            if type_ptr.is_null() {
                return None;
            }
            if cpython_ptr_is_type_object(object)
                && let Some(result) = context.lookup_type_attr_via_tp_dict(object, &name)
            {
                if is_proxy_trace {
                    eprintln!(
                        "[cpy-proxy] native getattr tp_dict(type) hit object_ptr={:p} result_ptr={:p}",
                        object, result
                    );
                }
                return Some(result);
            }
            // SAFETY: `type_ptr` is non-null and points to a CpythonTypeObject-compatible header.
            let tp_getattro = unsafe { (*type_ptr).tp_getattro };
            if is_proxy_trace {
                eprintln!(
                    "[cpy-proxy] native getattr slots tp_getattro={:p}",
                    tp_getattro
                );
            }
            if !tp_getattro.is_null() {
                let name_ptr = context.alloc_cpython_ptr_for_value(Value::Str(name.clone()));
                if name_ptr.is_null() {
                    return Some(std::ptr::null_mut());
                }
                let getattro: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                    // SAFETY: tp_getattro follows the CPython `PyObject* (*)(PyObject*,PyObject*)` ABI.
                    unsafe { std::mem::transmute(tp_getattro) };
                return Some(unsafe { getattro(object, name_ptr) });
            }
            // SAFETY: `type_ptr` is non-null and points to a CpythonTypeObject-compatible header.
            let tp_getattr = unsafe { (*type_ptr).tp_getattr };
            if is_proxy_trace {
                eprintln!(
                    "[cpy-proxy] native getattr slots tp_getattr={:p}",
                    tp_getattr
                );
            }
            if !tp_getattr.is_null() {
                let name_cstr = match context.scratch_c_string_ptr(&name) {
                    Ok(ptr) => ptr,
                    Err(err) => {
                        context.set_error(err);
                        return Some(std::ptr::null_mut());
                    }
                };
                let getattr: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
                    // SAFETY: tp_getattr follows the CPython `char*` getattr ABI.
                    unsafe { std::mem::transmute(tp_getattr) };
                return Some(unsafe { getattr(object, name_cstr) });
            }
            if let Some(result) = context.lookup_type_attr_via_tp_dict(object, &name) {
                if is_proxy_trace {
                    eprintln!(
                        "[cpy-proxy] native getattr tp_dict hit object_ptr={:p} result_ptr={:p}",
                        object, result
                    );
                }
                return Some(result);
            }
            if is_proxy_trace {
                eprintln!("[cpy-proxy] native getattr no native path hit; falling back");
            }
            None
        })
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            Some(std::ptr::null_mut())
        });
        if let Some(result) = native_result {
            if trace_numpy_attr {
                eprintln!(
                    "[numpy-init] PyObject_GetAttrString object={:p} name={} native_result={:p}",
                    object, name, result
                );
            }
            return result;
        }
    }
    let object_value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            let (type_ptr, tp_getattro, tp_getattr, owned) =
                with_active_cpython_context_mut(|context| {
                    // SAFETY: best-effort diagnostics for unknown-pointer failures.
                    let type_ptr = unsafe {
                        object
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                            .unwrap_or(std::ptr::null_mut())
                    };
                    // SAFETY: type_ptr is either null or points to a type object header.
                    let (tp_getattro, tp_getattr) = if type_ptr.is_null() {
                        (std::ptr::null_mut(), std::ptr::null_mut())
                    } else {
                        unsafe { ((*type_ptr).tp_getattro, (*type_ptr).tp_getattr) }
                    };
                    (
                        type_ptr,
                        tp_getattro,
                        tp_getattr,
                        context.owns_cpython_allocation_ptr(object),
                    )
                })
                .unwrap_or((
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    false,
                ));
            cpython_set_error(format!(
                "{err} (PyObject_GetAttrString object={:p} attr={} owned={} type_ptr={:p} tp_getattro={:p} tp_getattr={:p})",
                object, name, owned, type_ptr, tp_getattro, tp_getattr
            ));
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
    if name == "__array_finalize__" && std::env::var_os("PYRS_TRACE_CPY_PROXY_PTRS").is_some() {
        match &object_value {
            Value::Class(class_obj) => {
                if let Object::Class(class_data) = &*class_obj.kind() {
                    eprintln!(
                        "[cpy-proxy] getattr __array_finalize__ object_ptr={:p} class={} id={} raw_ptr_attr={:?}",
                        object,
                        class_data.name,
                        class_obj.id(),
                        class_data.attrs.get("__pyrs_cpython_proxy_ptr__")
                    );
                }
            }
            other => {
                eprintln!(
                    "[cpy-proxy] getattr __array_finalize__ non-class object_ptr={:p} tag={}",
                    object,
                    cpython_value_debug_tag(other)
                );
            }
        }
    }
    match cpython_call_builtin(
        BuiltinFunction::GetAttr,
        vec![object_value, Value::Str(name.clone())],
    ) {
        Ok(value) => {
            let ptr = cpython_new_ptr_for_value(value);
            if trace_numpy_attr {
                eprintln!(
                    "[numpy-init] PyObject_GetAttrString object={:p} name={} builtin_result={:p}",
                    object, name, ptr
                );
            }
            ptr
        }
        Err(err) => {
            cpython_set_error(err.clone());
            if trace_numpy_attr {
                eprintln!(
                    "[numpy-init] PyObject_GetAttrString object={:p} name={} error={}",
                    object, name, err
                );
            }
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAttr(object: *mut c_void, name: *mut c_void) -> *mut c_void {
    if !object.is_null() {
        let native_result = with_active_cpython_context_mut(|context| {
            let is_known_compat = context.cpython_handle_from_ptr(object).is_some();
            if is_known_compat {
                return None;
            }
            // SAFETY: object pointer comes from extension code; type pointer access mirrors CPython.
            let type_ptr = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if type_ptr.is_null() {
                return None;
            }
            let attr_name =
                context
                    .cpython_value_from_ptr(name)
                    .and_then(|candidate| match candidate {
                        Value::Str(text) => Some(text),
                        _ => None,
                    });
            if cpython_ptr_is_type_object(object)
                && let Some(attr_name) = attr_name.as_ref()
                && let Some(result) = context.lookup_type_attr_via_tp_dict(object, attr_name)
            {
                return Some(result);
            }
            // SAFETY: `type_ptr` is non-null and points to a CpythonTypeObject-compatible header.
            let tp_getattro = unsafe { (*type_ptr).tp_getattro };
            if tp_getattro.is_null() {
                if let Some(attr_name) = attr_name.as_ref()
                    && let Some(result) = context.lookup_type_attr_via_tp_dict(object, attr_name)
                {
                    return Some(result);
                }
                return None;
            }
            let getattro: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                // SAFETY: tp_getattro follows the CPython `PyObject* (*)(PyObject*,PyObject*)` ABI.
                unsafe { std::mem::transmute(tp_getattro) };
            Some(unsafe { getattro(object, name) })
        })
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            Some(std::ptr::null_mut())
        });
        if let Some(result) = native_result {
            return result;
        }
    }
    let object_value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            let (type_ptr, tp_getattro, owned) = with_active_cpython_context_mut(|context| {
                // SAFETY: best-effort diagnostics for unknown-pointer failures.
                let type_ptr = unsafe {
                    object
                        .cast::<CpythonObjectHead>()
                        .as_ref()
                        .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                        .unwrap_or(std::ptr::null_mut())
                };
                let tp_getattro = if type_ptr.is_null() {
                    std::ptr::null_mut()
                } else {
                    // SAFETY: type_ptr is non-null and points to a type object header.
                    unsafe { (*type_ptr).tp_getattro }
                };
                (
                    type_ptr,
                    tp_getattro,
                    context.owns_cpython_allocation_ptr(object),
                )
            })
            .unwrap_or((std::ptr::null_mut(), std::ptr::null_mut(), false));
            cpython_set_error(format!(
                "{err} (PyObject_GetAttr object={:p} name_ptr={:p} owned={} type_ptr={:p} tp_getattro={:p})",
                object, name, owned, type_ptr, tp_getattro
            ));
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
    if !object.is_null() {
        let native_status = with_active_cpython_context_mut(|context| {
            let is_known_compat = context.cpython_handle_from_ptr(object).is_some();
            if is_known_compat {
                return None;
            }
            let attr_name = match unsafe { c_name_to_string(name) } {
                Ok(name) => name,
                Err(err) => {
                    context.set_error(err);
                    return Some(-1);
                }
            };
            // SAFETY: object pointer comes from extension code; type pointer access mirrors CPython.
            let type_ptr = unsafe {
                object
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if type_ptr.is_null() {
                return None;
            }
            // SAFETY: type pointer is non-null and follows CPython type layout.
            let tp_setattro = unsafe { (*type_ptr).tp_setattro };
            if !tp_setattro.is_null() {
                let attr_name_ptr =
                    context.alloc_cpython_ptr_for_value(Value::Str(attr_name.clone()));
                if attr_name_ptr.is_null() {
                    context.set_error("PyObject_SetAttrString failed to materialize attr name");
                    return Some(-1);
                }
                let setattro: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
                    // SAFETY: tp_setattro follows CPython setattro ABI.
                    unsafe { std::mem::transmute(tp_setattro) };
                return Some(unsafe { setattro(object, attr_name_ptr, value) });
            }
            // SAFETY: type pointer is non-null and follows CPython type layout.
            let tp_setattr = unsafe { (*type_ptr).tp_setattr };
            if !tp_setattr.is_null() {
                let c_name = match CString::new(attr_name.as_str()) {
                    Ok(name) => name,
                    Err(_) => {
                        context.set_error("attribute name contains interior NUL byte");
                        return Some(-1);
                    }
                };
                let setattr: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> i32 =
                    // SAFETY: tp_setattr follows CPython setattr ABI.
                    unsafe { std::mem::transmute(tp_setattr) };
                return Some(unsafe { setattr(object, c_name.as_ptr(), value) });
            }
            None
        })
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            Some(-1)
        });
        if let Some(status) = native_status {
            return status;
        }
    }
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
pub unsafe extern "C" fn PyObject_SetAttr(
    object: *mut c_void,
    name: *mut c_void,
    value: *mut c_void,
) -> i32 {
    if value.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe { PyObject_GenericSetAttr(object, name, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelAttr(object: *mut c_void, name: *mut c_void) -> i32 {
    unsafe { PyObject_GenericSetAttr(object, name, std::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelAttrString(object: *mut c_void, name: *const c_char) -> i32 {
    let name_text = match unsafe { c_name_to_string(name) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let name_obj = cpython_new_ptr_for_value(Value::Str(name_text));
    if name_obj.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_DelAttr(object, name_obj) };
    unsafe { Py_DecRef(name_obj) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelItemString(object: *mut c_void, key: *const c_char) -> i32 {
    let key_text = match unsafe { c_name_to_string(key) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let key_obj = cpython_new_ptr_for_value(Value::Str(key_text));
    if key_obj.is_null() {
        return -1;
    }
    let status = unsafe { PyObject_DelItem(object, key_obj) };
    unsafe { Py_DecRef(key_obj) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Type(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        cpython_set_error("PyObject_Type received null object");
        return std::ptr::null_mut();
    }
    let type_ptr = unsafe {
        object
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type)
            .unwrap_or(std::ptr::null_mut())
    };
    if type_ptr.is_null() {
        cpython_set_error("PyObject_Type encountered object without type");
        return std::ptr::null_mut();
    }
    unsafe { Py_IncRef(type_ptr.cast()) };
    type_ptr.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyObject_Type(object: *mut c_void) -> *mut c_void {
    unsafe { PyObject_Type(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetTypeData(
    object: *mut c_void,
    cls: *mut c_void,
) -> *mut c_void {
    if object.is_null() || cls.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let is_instance = unsafe { PyObject_IsInstance(object, cls) };
    if is_instance < 0 {
        return std::ptr::null_mut();
    }
    if is_instance == 0 {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "PyObject_GetTypeData called for unrelated type",
        );
        return std::ptr::null_mut();
    }
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_HasAttrString(object: *mut c_void, name: *const c_char) -> i32 {
    let status = unsafe { PyObject_HasAttrStringWithError(object, name) };
    if status < 0 {
        unsafe { PyErr_Clear() };
        0
    } else {
        status
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_HasAttr(object: *mut c_void, name: *mut c_void) -> i32 {
    let status = unsafe { PyObject_HasAttrWithError(object, name) };
    if status < 0 {
        unsafe { PyErr_Clear() };
        0
    } else {
        status
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_HasAttrWithError(object: *mut c_void, name: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_HasAttrWithError missing VM context");
            return -1;
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_HasAttrWithError received unknown object pointer");
            return -1;
        };
        let Some(name_value) = context.cpython_value_from_ptr_or_proxy(name) else {
            context.set_error("PyObject_HasAttrWithError received unknown name pointer");
            return -1;
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_getattr(vec![object_value, name_value], HashMap::new()) {
            Ok(_) => 1,
            Err(err) => {
                if cpython_active_exception_is(vm, "AttributeError")
                    || err.message.contains("has no attribute")
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
pub unsafe extern "C" fn PyObject_HasAttrStringWithError(
    object: *mut c_void,
    name: *const c_char,
) -> i32 {
    let name_text = match unsafe { c_name_to_string(name) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_HasAttrStringWithError missing VM context");
            return -1;
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_HasAttrStringWithError received unknown object pointer");
            return -1;
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_getattr(
            vec![object_value, Value::Str(name_text.clone())],
            HashMap::new(),
        ) {
            Ok(_) => 1,
            Err(err) => {
                if cpython_active_exception_is(vm, "AttributeError")
                    || err.message.contains("has no attribute")
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
pub unsafe extern "C" fn PyObject_GetOptionalAttrString(
    object: *mut c_void,
    name: *const c_char,
    result: *mut *mut c_void,
) -> i32 {
    if result.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe { *result = std::ptr::null_mut() };
    let name_text = match unsafe { c_name_to_string(name) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_GetOptionalAttrString missing VM context");
            return -1;
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_GetOptionalAttrString received unknown object pointer");
            return -1;
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_getattr(
            vec![object_value, Value::Str(name_text.clone())],
            HashMap::new(),
        ) {
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
                if cpython_active_exception_is(vm, "AttributeError")
                    || err.message.contains("has no attribute")
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
pub unsafe extern "C" fn PyObject_Repr(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::Repr, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_ASCII(object: *mut c_void) -> *mut c_void {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    match cpython_call_builtin(BuiltinFunction::Ascii, vec![value]) {
        Ok(value) => cpython_new_ptr_for_value(value),
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Dir(object: *mut c_void) -> *mut c_void {
    let args = if object.is_null() {
        Vec::new()
    } else {
        let value = match cpython_value_from_ptr(object) {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
        vec![value]
    };
    match cpython_call_builtin(BuiltinFunction::Dir, args) {
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
pub unsafe extern "C" fn PyAIter_Check(object: *mut c_void) -> i32 {
    if object.is_null() {
        return 0;
    }
    let status = unsafe { PyObject_HasAttrStringWithError(object, c"__anext__".as_ptr()) };
    if status < 0 {
        unsafe { PyErr_Clear() };
        0
    } else {
        status
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAIter(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_GetAIter missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_GetAIter received unknown object pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let aiter = match vm.builtin_getattr(vec![value, Value::Str("__aiter__".to_string())], HashMap::new()) {
            Ok(callable) => callable,
            Err(err) => {
                context.set_error(err.message);
                return std::ptr::null_mut();
            }
        };
        match vm.call_internal(aiter, Vec::new(), HashMap::new()) {
            Ok(InternalCallOutcome::Value(result)) => context.alloc_cpython_ptr_for_value(result),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(vm.runtime_error_from_active_exception("PyObject_GetAIter failed").message);
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
pub unsafe extern "C" fn PyObject_CallNoArgs(callable: *mut c_void) -> *mut c_void {
    cpython_call_object(callable, Vec::new(), HashMap::new())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMethod_New(function: *mut c_void, self_obj: *mut c_void) -> *mut c_void {
    // Minimal baseline: treat method construction as returning the callable when full
    // bound-method construction is unavailable in the compat object space.
    let _ = self_obj;
    unsafe { Py_XIncRef(function) };
    function
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
    with_active_cpython_context_mut(|context| {
        let Some(vectorcall) = (unsafe { cpython_resolve_vectorcall(callable) }) else {
            context.set_error("PyVectorcall_Call target has no vectorcall function");
            return std::ptr::null_mut();
        };
        if context.vm.is_null() {
            context.set_error("PyVectorcall_Call missing VM context");
            return std::ptr::null_mut();
        }

        let mut args_ptrs: Vec<*mut c_void> = Vec::new();
        if !tuple.is_null() {
            let Some(tuple_value) = context.cpython_value_from_ptr(tuple) else {
                context.set_error("PyVectorcall_Call received unknown args tuple");
                return std::ptr::null_mut();
            };
            let Value::Tuple(tuple_obj) = tuple_value else {
                context.set_error("PyVectorcall_Call expected tuple args");
                return std::ptr::null_mut();
            };
            let Object::Tuple(values) = &*tuple_obj.kind() else {
                context.set_error("PyVectorcall_Call tuple storage invalid");
                return std::ptr::null_mut();
            };
            for value in values {
                let ptr = context.alloc_cpython_ptr_for_value(value.clone());
                if ptr.is_null() {
                    context.set_error("PyVectorcall_Call failed to materialize positional arg");
                    return std::ptr::null_mut();
                }
                args_ptrs.push(ptr);
            }
        }

        let positional_count = args_ptrs.len();
        let mut kw_names: Vec<Value> = Vec::new();
        if !dict.is_null() {
            let Some(dict_value) = context.cpython_value_from_ptr(dict) else {
                context.set_error("PyVectorcall_Call received unknown kwargs dict");
                return std::ptr::null_mut();
            };
            let Value::Dict(dict_obj) = dict_value else {
                context.set_error("PyVectorcall_Call expected kwargs dict");
                return std::ptr::null_mut();
            };
            let entries = match &*dict_obj.kind() {
                Object::Dict(entries) => entries.clone(),
                _ => {
                    context.set_error("PyVectorcall_Call kwargs storage invalid");
                    return std::ptr::null_mut();
                }
            };
            for (key, value) in entries {
                let Value::Str(name) = key else {
                    context.set_error("PyVectorcall_Call kwargs must use str keys");
                    return std::ptr::null_mut();
                };
                kw_names.push(Value::Str(name));
                let ptr = context.alloc_cpython_ptr_for_value(value);
                if ptr.is_null() {
                    context.set_error("PyVectorcall_Call failed to materialize keyword arg");
                    return std::ptr::null_mut();
                }
                args_ptrs.push(ptr);
            }
        }

        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let has_keywords = !kw_names.is_empty();
        let kwnames_ptr = if !has_keywords {
            std::ptr::null_mut()
        } else {
            context.alloc_cpython_ptr_for_value(vm.heap.alloc_tuple(kw_names))
        };
        if has_keywords && kwnames_ptr.is_null() {
            context.set_error("PyVectorcall_Call failed to build keyword names tuple");
            return std::ptr::null_mut();
        }
        let args_ptr = if args_ptrs.is_empty() {
            std::ptr::null()
        } else {
            args_ptrs.as_ptr()
        };
        unsafe { vectorcall(callable, args_ptr, positional_count, kwnames_ptr) }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Vectorcall(
    callable: *mut c_void,
    args: *const *mut c_void,
    nargsf: usize,
    kwnames: *mut c_void,
) -> *mut c_void {
    if let Some(vectorcall) = unsafe { cpython_resolve_vectorcall(callable) } {
        return unsafe { vectorcall(callable, args, nargsf, kwnames) };
    }
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
    let trace_getitem = std::env::var_os("PYRS_TRACE_CPY_GETITEM").is_some();
    if trace_getitem {
        let key_desc = with_active_cpython_context_mut(|context| {
            context
                .cpython_value_from_ptr_or_proxy(key)
                .map(|value| cpython_debug_compare_value(&value))
                .unwrap_or_else(|| "<unknown>".to_string())
        })
        .unwrap_or_else(|_| "<no-context>".to_string());
        eprintln!(
            "[cpy-getitem] object_ptr={:p} key_ptr={:p} key={}",
            object, key, key_desc
        );
    }
    let result = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_GetItem missing VM context");
            return std::ptr::null_mut();
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_GetItem received unknown object pointer");
            return std::ptr::null_mut();
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyObject_GetItem received unknown key pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.getitem_value(object_value, key_value) {
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
    });
    if trace_getitem {
        if result.is_null() {
            eprintln!("[cpy-getitem] result=<null>");
        } else {
            let result_tag = with_active_cpython_context_mut(|context| {
                context
                    .cpython_value_from_ptr_or_proxy(result)
                    .map(|value| cpython_value_debug_tag(&value))
                    .unwrap_or_else(|| "<unknown>".to_string())
            })
            .unwrap_or_else(|_| "<no-context>".to_string());
            eprintln!("[cpy-getitem] result={:p} tag={}", result, result_tag);
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetItem(
    object: *mut c_void,
    key: *mut c_void,
    value: *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_SetItem missing VM context");
            return -1;
        }
        let object_handle = context.cpython_handle_from_ptr(object);
        let Some(target) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_SetItem received unknown object pointer");
            return -1;
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyObject_SetItem received unknown key pointer");
            return -1;
        };
        let Some(item_value) = context.cpython_value_from_ptr_or_proxy(value) else {
            context.set_error("PyObject_SetItem received unknown value pointer");
            return -1;
        };
        match &target {
            Value::Dict(dict_obj) => {
                return match dict_set_value_checked(dict_obj, key_value, item_value) {
                    Ok(()) => 0,
                    Err(err) => {
                        context.set_error(err.message);
                        -1
                    }
                };
            }
            Value::List(list_obj) => {
                if let Ok(raw_idx) = value_to_int(key_value.clone()) {
                    {
                        let mut list_kind = list_obj.kind_mut();
                        let Object::List(values) = &mut *list_kind else {
                            context.set_error("PyObject_SetItem encountered invalid list storage");
                            return -1;
                        };
                        let mut idx = raw_idx as isize;
                        if idx < 0 {
                            idx += values.len() as isize;
                        }
                        if idx < 0 || idx as usize >= values.len() {
                            context.set_error("index out of range");
                            return -1;
                        }
                        values[idx as usize] = item_value;
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
                if let Value::Slice(slice_value) = &key_value {
                    let replacement_values = {
                        // SAFETY: VM pointer is valid for context lifetime.
                        let vm = unsafe { &mut *context.vm };
                        match vm.collect_iterable_values(item_value.clone()) {
                            Ok(values) => values,
                            Err(err) => {
                                context.set_error(err.message);
                                return -1;
                            }
                        }
                    };
                    {
                        let mut list_kind = list_obj.kind_mut();
                        let Object::List(values) = &mut *list_kind else {
                            context.set_error("PyObject_SetItem encountered invalid list storage");
                            return -1;
                        };
                        let step = slice_value.step.unwrap_or(1);
                        if step == 1 {
                            let (start, stop) = cpython_slice_bounds_step_one(
                                values.len(),
                                slice_value.lower,
                                slice_value.upper,
                            );
                            values.splice(start..stop, replacement_values);
                        } else {
                            let indices = match cpython_slice_indices_for_len(
                                values.len(),
                                slice_value.lower,
                                slice_value.upper,
                                slice_value.step,
                            ) {
                                Ok(indices) => indices,
                                Err(err) => {
                                    context.set_error(err);
                                    return -1;
                                }
                            };
                            if indices.len() != replacement_values.len() {
                                context.set_error(
                                    "attempt to assign sequence of size to extended slice of different size",
                                );
                                return -1;
                            }
                            for (idx, item) in indices.into_iter().zip(replacement_values.into_iter()) {
                                values[idx] = item;
                            }
                        }
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
            }
            Value::ByteArray(bytearray_obj) => {
                if let Ok(raw_idx) = value_to_int(key_value.clone()) {
                    {
                        let mut bytes_kind = bytearray_obj.kind_mut();
                        let Object::ByteArray(values) = &mut *bytes_kind else {
                            context.set_error(
                                "PyObject_SetItem encountered invalid bytearray storage",
                            );
                            return -1;
                        };
                        let mut idx = raw_idx as isize;
                        if idx < 0 {
                            idx += values.len() as isize;
                        }
                        if idx < 0 || idx as usize >= values.len() {
                            context.set_error("index out of range");
                            return -1;
                        }
                        let byte = match value_to_int(item_value.clone()) {
                            Ok(value) => value,
                            Err(err) => {
                                context.set_error(err.message);
                                return -1;
                            }
                        };
                        if !(0..=255).contains(&byte) {
                            context.set_error("byte must be in range(0, 256)");
                            return -1;
                        }
                        values[idx as usize] = byte as u8;
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
            }
            _ => {}
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(setitem) = (match vm.lookup_bound_special_method(&target, "__setitem__") {
            Ok(method) => method,
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        }) else {
            context.set_error("object does not support item assignment");
            return -1;
        };
        match vm.call_internal(setitem, vec![key_value, item_value], HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => {
                if let Some(handle) = object_handle {
                    context.sync_cpython_storage_from_value(handle);
                }
                0
            }
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(vm.runtime_error_from_active_exception("object_set_item() failed").message);
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
pub unsafe extern "C" fn PyObject_DelItem(object: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyObject_DelItem missing VM context");
            return -1;
        }
        let object_handle = context.cpython_handle_from_ptr(object);
        let Some(target) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyObject_DelItem received unknown object pointer");
            return -1;
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyObject_DelItem received unknown key pointer");
            return -1;
        };
        match &target {
            Value::Dict(dict_obj) => {
                if dict_remove_value(dict_obj, &key_value).is_some() {
                    return 0;
                }
                context.set_error("dict key not found");
                return -1;
            }
            Value::List(list_obj) => {
                if let Ok(raw_idx) = value_to_int(key_value.clone()) {
                    {
                        let mut list_kind = list_obj.kind_mut();
                        let Object::List(values) = &mut *list_kind else {
                            context.set_error("PyObject_DelItem encountered invalid list storage");
                            return -1;
                        };
                        let mut idx = raw_idx as isize;
                        if idx < 0 {
                            idx += values.len() as isize;
                        }
                        if idx < 0 || idx as usize >= values.len() {
                            context.set_error("index out of range");
                            return -1;
                        }
                        values.remove(idx as usize);
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
                if let Value::Slice(slice_value) = &key_value {
                    {
                        let mut list_kind = list_obj.kind_mut();
                        let Object::List(values) = &mut *list_kind else {
                            context.set_error("PyObject_DelItem encountered invalid list storage");
                            return -1;
                        };
                        let step = slice_value.step.unwrap_or(1);
                        if step == 1 {
                            let (start, stop) = cpython_slice_bounds_step_one(
                                values.len(),
                                slice_value.lower,
                                slice_value.upper,
                            );
                            values.drain(start..stop);
                        } else {
                            let mut indices = match cpython_slice_indices_for_len(
                                values.len(),
                                slice_value.lower,
                                slice_value.upper,
                                slice_value.step,
                            ) {
                                Ok(indices) => indices,
                                Err(err) => {
                                    context.set_error(err);
                                    return -1;
                                }
                            };
                            indices.sort_unstable();
                            indices.dedup();
                            for idx in indices.into_iter().rev() {
                                values.remove(idx);
                            }
                        }
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
            }
            Value::ByteArray(bytearray_obj) => {
                if let Ok(raw_idx) = value_to_int(key_value.clone()) {
                    {
                        let mut bytes_kind = bytearray_obj.kind_mut();
                        let Object::ByteArray(values) = &mut *bytes_kind else {
                            context.set_error(
                                "PyObject_DelItem encountered invalid bytearray storage",
                            );
                            return -1;
                        };
                        let mut idx = raw_idx as isize;
                        if idx < 0 {
                            idx += values.len() as isize;
                        }
                        if idx < 0 || idx as usize >= values.len() {
                            context.set_error("index out of range");
                            return -1;
                        }
                        values.remove(idx as usize);
                    }
                    if let Some(handle) = object_handle {
                        context.sync_cpython_storage_from_value(handle);
                    }
                    return 0;
                }
            }
            _ => {}
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(delitem) = (match vm.lookup_bound_special_method(&target, "__delitem__") {
            Ok(method) => method,
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        }) else {
            context.set_error("object does not support item deletion");
            return -1;
        };
        match vm.call_internal(delitem, vec![key_value], HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => {
                if let Some(handle) = object_handle {
                    context.sync_cpython_storage_from_value(handle);
                }
                0
            }
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                context.set_error(
                    vm.runtime_error_from_active_exception("object_del_item() failed")
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
pub unsafe extern "C" fn PyObject_Length(object: *mut c_void) -> isize {
    unsafe { PyObject_Size(object) }
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

fn cpython_value_type_name_from_ptr(object: *mut c_void) -> String {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return "object".to_string();
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            return "object".to_string();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        vm.value_type_name_for_error(&value)
    })
    .unwrap_or_else(|_| "object".to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_HashNotImplemented(object: *mut c_void) -> isize {
    let type_name = cpython_value_type_name_from_ptr(object);
    cpython_set_typed_error(
        unsafe { PyExc_TypeError },
        format!("unhashable type: '{type_name}'"),
    );
    -1
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

fn cpython_swapped_compare_op(op: i32) -> Option<i32> {
    match op {
        0 => Some(4),
        1 => Some(5),
        2 => Some(2),
        3 => Some(3),
        4 => Some(0),
        5 => Some(1),
        _ => None,
    }
}

fn cpython_compare_op_symbol(op: i32) -> &'static str {
    match op {
        0 => "<",
        1 => "<=",
        2 => "==",
        3 => "!=",
        4 => ">",
        5 => ">=",
        _ => "?",
    }
}

fn cpython_type_name_for_object_ptr(object: *mut c_void) -> String {
    if object.is_null() {
        return "<null>".to_string();
    }
    // SAFETY: caller provides a potential PyObject pointer and we guard all nulls.
    unsafe {
        let Some(head) = object.cast::<CpythonObjectHead>().as_ref() else {
            return "<unknown>".to_string();
        };
        let ty = head.ob_type.cast::<CpythonTypeObject>();
        if ty.is_null() {
            return "<unknown>".to_string();
        }
        c_name_to_string((*ty).tp_name).unwrap_or_else(|_| "<unnamed>".to_string())
    }
}

fn cpython_is_not_implemented_ptr(value: *mut c_void) -> bool {
    if value.is_null() {
        return false;
    }
    if value == std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast::<c_void>() {
        return true;
    }
    with_active_cpython_context_mut(|context| {
        let Some(mapped) = context.cpython_value_from_ptr(value) else {
            return false;
        };
        if context.vm.is_null() {
            return false;
        }
        // SAFETY: VM pointer is valid for the C-API context lifetime.
        let vm = unsafe { &*context.vm };
        vm.builtins
            .get("NotImplemented")
            .is_some_and(|not_implemented| *not_implemented == mapped)
    })
    .unwrap_or(false)
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

fn cpython_debug_tuple_raw_ptrs(
    context: &ModuleCapiContext,
    object: *mut c_void,
) -> Option<String> {
    if object.is_null() || !context.owns_cpython_allocation_ptr(object) {
        return None;
    }
    // SAFETY: owned tuple pointers use CPython-compatible varobject header
    // followed by contiguous `PyObject*` item slots.
    unsafe {
        let head = object.cast::<CpythonVarObjectHead>();
        let len = (*head).ob_size.max(0) as usize;
        if len == 0 {
            return Some("[]".to_string());
        }
        let items_ptr = cpython_tuple_items_ptr(object);
        let mut rendered = Vec::with_capacity(len);
        for idx in 0..len {
            let item = *items_ptr.add(idx);
            rendered.push(format!("{:p}", item));
        }
        Some(format!("[{}]", rendered.join(",")))
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_RichCompare(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> *mut c_void {
    if left.is_null() || right.is_null() {
        cpython_set_error("PyObject_RichCompare received null operand");
        return std::ptr::null_mut();
    }
    let Some(slot_name) = cpython_rich_compare_slot_name(op) else {
        cpython_set_error("PyObject_RichCompare received invalid compare op");
        return std::ptr::null_mut();
    };
    let right_value = match cpython_value_from_ptr(right) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let left_value = match cpython_value_from_ptr(left) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };

    enum RichCompareAttempt {
        Missing,
        Value(*mut c_void),
        Error,
    }

    let try_call = |receiver_ptr: *mut c_void,
                    method_name: &std::ffi::CStr,
                    arg: Value|
     -> RichCompareAttempt {
        let callable = unsafe { PyObject_GetAttrString(receiver_ptr, method_name.as_ptr()) };
        if callable.is_null() {
            unsafe { PyErr_Clear() };
            return RichCompareAttempt::Missing;
        }
        let result = cpython_call_object(callable, vec![arg], HashMap::new());
        unsafe { Py_DecRef(callable) };
        if result.is_null() {
            RichCompareAttempt::Error
        } else {
            RichCompareAttempt::Value(result)
        }
    };

    match try_call(left, slot_name, right_value.clone()) {
        RichCompareAttempt::Value(result) => {
            if !cpython_is_not_implemented_ptr(result) {
                return result;
            }
            unsafe { Py_DecRef(result) };
        }
        RichCompareAttempt::Error => return std::ptr::null_mut(),
        RichCompareAttempt::Missing => {}
    }

    let swapped_op = cpython_swapped_compare_op(op).expect("valid compare op has swapped mapping");
    let swapped_slot_name =
        cpython_rich_compare_slot_name(swapped_op).expect("valid compare op has slot");
    match try_call(right, swapped_slot_name, left_value.clone()) {
        RichCompareAttempt::Value(result) => {
            if !cpython_is_not_implemented_ptr(result) {
                return result;
            }
            unsafe { Py_DecRef(result) };
        }
        RichCompareAttempt::Error => return std::ptr::null_mut(),
        RichCompareAttempt::Missing => {}
    }

    match op {
        2 => cpython_new_ptr_for_value(Value::Bool(left == right)),
        3 => cpython_new_ptr_for_value(Value::Bool(left != right)),
        _ => {
            cpython_set_error(format!(
                "TypeError: '{}' not supported between instances of '{}' and '{}'",
                cpython_compare_op_symbol(op),
                cpython_type_name_for_object_ptr(left),
                cpython_type_name_for_object_ptr(right)
            ));
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_RichCompareBool(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> i32 {
    let trace_compare_errors = std::env::var_os("PYRS_TRACE_CPY_COMPARE_ERRORS").is_some();
    if left == right {
        if op == 2 {
            return 1;
        }
        if op == 3 {
            return 0;
        }
    }
    let trace_compare = std::env::var_os("PYRS_TRACE_CPY_COMPARE").is_some();
    if trace_compare && op == 2 {
        let mut left_raw = String::new();
        let mut right_raw = String::new();
        let left_desc = with_active_cpython_context_mut(|context| {
            left_raw = cpython_debug_tuple_raw_ptrs(context, left).unwrap_or_default();
            match context.cpython_value_from_ptr(left) {
                Some(value) => cpython_debug_compare_value(&value),
                None => "ERR(unknown)".to_string(),
            }
        })
        .unwrap_or_else(|err| format!("ERR({err})"));
        let right_desc = with_active_cpython_context_mut(|context| {
            right_raw = cpython_debug_tuple_raw_ptrs(context, right).unwrap_or_default();
            match context.cpython_value_from_ptr(right) {
                Some(value) => cpython_debug_compare_value(&value),
                None => "ERR(unknown)".to_string(),
            }
        })
        .unwrap_or_else(|err| format!("ERR({err})"));
        eprintln!(
            "[cpy-cmp] eq left_ptr={:p} right_ptr={:p} left={} right={} left_raw={} right_raw={}",
            left, right, left_desc, right_desc, left_raw, right_raw
        );
    }
    let value = unsafe { PyObject_RichCompare(left, right, op) };
    if value.is_null() {
        if trace_compare_errors {
            eprintln!(
                "[cpy-cmp-err] PyObject_RichCompare returned null op={} left={:p} right={:p}",
                op, left, right
            );
        }
        if trace_compare && op == 2 {
            eprintln!("[cpy-cmp] eq result=<null>");
        }
        return -1;
    }
    if cpython_is_not_implemented_ptr(value) {
        unsafe { Py_DecRef(value) };
        return match op {
            2 => i32::from(left == right),
            3 => i32::from(left != right),
            _ => {
                cpython_set_error(format!(
                    "TypeError: '{}' not supported between instances of '{}' and '{}'",
                    cpython_compare_op_symbol(op),
                    cpython_type_name_for_object_ptr(left),
                    cpython_type_name_for_object_ptr(right)
                ));
                -1
            }
        };
    }
    let truth = unsafe { PyObject_IsTrue(value) };
    unsafe { Py_DecRef(value) };
    if trace_compare_errors && truth < 0 {
        eprintln!(
            "[cpy-cmp-err] PyObject_IsTrue failed op={} left={:p} right={:p}",
            op, left, right
        );
    }
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
            let status = unsafe { PyObject_HasAttrStringWithError(object, c"__getitem__".as_ptr()) };
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
            cpython_set_typed_error(unsafe { PyExc_TypeError }, format!("{type_name} is not a mapping"));
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
pub unsafe extern "C" fn PyObject_CheckReadBuffer(object: *mut c_void) -> i32 {
    unsafe { PyObject_CheckBuffer(object) }
}

fn cpython_legacy_bytes_buffer_slot(
    context: &mut ModuleCapiContext,
    object: *mut c_void,
    writable: bool,
) -> Result<(PyrsObjectHandle, *mut c_void, usize), String> {
    let handle = context
        .cpython_handle_from_ptr(object)
        .ok_or_else(|| "unknown object pointer".to_string())?;
    let slot = context
        .objects
        .get(&handle)
        .ok_or_else(|| "unknown object handle".to_string())?;
    let len = match &slot.value {
        Value::Bytes(obj) => {
            if writable {
                return Err("expected writable bytes-like object".to_string());
            }
            match &*obj.kind() {
                Object::Bytes(bytes) => bytes.len(),
                _ => return Err("invalid bytes storage".to_string()),
            }
        }
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(bytes) => bytes.len(),
            _ => return Err("invalid bytearray storage".to_string()),
        },
        _ => return Err("expected bytes-like object".to_string()),
    };
    context.sync_cpython_storage_from_value(handle);
    let raw_ptr = context
        .cpython_ptr_by_handle
        .get(&handle)
        .copied()
        .ok_or_else(|| "missing CPython storage pointer".to_string())?;
    Ok((handle, raw_ptr, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_AsReadBuffer(
    object: *mut c_void,
    buffer: *mut *const c_void,
    buffer_len: *mut isize,
) -> i32 {
    if buffer.is_null() || buffer_len.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe {
        *buffer = std::ptr::null();
        *buffer_len = 0;
    }
    with_active_cpython_context_mut(|context| {
        match cpython_legacy_bytes_buffer_slot(context, object, false) {
            Ok((_handle, raw_ptr, len)) => {
                // SAFETY: raw_ptr is owned CPython-compatible bytes/bytearray storage.
                let data = unsafe { cpython_bytes_data_ptr(raw_ptr) };
                unsafe {
                    *buffer = data.cast();
                    *buffer_len = len as isize;
                }
                0
            }
            Err(err) => {
                context.set_error(format!("PyObject_AsReadBuffer {err}"));
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
pub unsafe extern "C" fn PyObject_AsWriteBuffer(
    object: *mut c_void,
    buffer: *mut *mut c_void,
    buffer_len: *mut isize,
) -> i32 {
    if buffer.is_null() || buffer_len.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    unsafe {
        *buffer = std::ptr::null_mut();
        *buffer_len = 0;
    }
    with_active_cpython_context_mut(|context| {
        match cpython_legacy_bytes_buffer_slot(context, object, true) {
            Ok((_handle, raw_ptr, len)) => {
                // SAFETY: raw_ptr is owned CPython-compatible bytearray storage.
                let data = unsafe { cpython_bytes_data_ptr(raw_ptr) };
                unsafe {
                    *buffer = data.cast();
                    *buffer_len = len as isize;
                }
                0
            }
            Err(err) => {
                let message = format!("PyObject_AsWriteBuffer {err}");
                let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
                context.set_error_state(
                    unsafe { PyExc_TypeError },
                    pvalue,
                    std::ptr::null_mut(),
                    message,
                );
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
pub unsafe extern "C" fn PyObject_AsCharBuffer(
    object: *mut c_void,
    buffer: *mut *const c_char,
    buffer_len: *mut isize,
) -> i32 {
    if buffer.is_null() || buffer_len.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    let mut raw: *const c_void = std::ptr::null();
    let status = unsafe { PyObject_AsReadBuffer(object, &mut raw, buffer_len) };
    if status != 0 {
        return status;
    }
    unsafe { *buffer = raw.cast() };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CopyData(dest: *mut c_void, src: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let (src_handle, src_ptr, src_len) =
            match cpython_legacy_bytes_buffer_slot(context, src, false) {
                Ok(state) => state,
                Err(err) => {
                    context.set_error(format!("PyObject_CopyData source {err}"));
                    return -1;
                }
            };
        let (dest_handle, dest_ptr, dest_len) =
            match cpython_legacy_bytes_buffer_slot(context, dest, true) {
                Ok(state) => state,
                Err(err) => {
                    let message = format!("PyObject_CopyData destination {err}");
                    let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
                    context.set_error_state(
                        unsafe { PyExc_TypeError },
                        pvalue,
                        std::ptr::null_mut(),
                        message,
                    );
                    return -1;
                }
            };
        if src_len != dest_len {
            let message = "source and destination buffers have different lengths".to_string();
            let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
            context.set_error_state(
                unsafe { PyExc_ValueError },
                pvalue,
                std::ptr::null_mut(),
                message,
            );
            return -1;
        }
        if src_len > 0 {
            // SAFETY: pointers are owned bytes storage with at least src_len bytes each.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    cpython_bytes_data_ptr(src_ptr).cast::<u8>(),
                    cpython_bytes_data_ptr(dest_ptr).cast::<u8>(),
                    src_len,
                );
            }
        }
        context.sync_value_from_cpython_storage(src_handle, src_ptr);
        context.sync_value_from_cpython_storage(dest_handle, dest_ptr);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

fn cpython_buffer_layout_from_view(
    view: &CpythonBuffer,
) -> (
    usize,
    Option<String>,
    Option<Vec<isize>>,
    Option<Vec<isize>>,
    bool,
) {
    let itemsize = view.itemsize.max(1) as usize;
    let format = if view.format.is_null() {
        None
    } else {
        unsafe { c_name_to_string(view.format.cast_const()) }.ok()
    };
    let ndim = view.ndim.max(0) as usize;
    let shape = if ndim == 0 || view.shape.is_null() {
        None
    } else {
        Some(
            (0..ndim)
                .map(|idx| {
                    // SAFETY: `shape` is valid for `ndim` entries by C-API contract.
                    unsafe { *view.shape.add(idx) }
                })
                .collect::<Vec<_>>(),
        )
    };
    let strides = if ndim == 0 || view.strides.is_null() {
        None
    } else {
        Some(
            (0..ndim)
                .map(|idx| {
                    // SAFETY: `strides` is valid for `ndim` entries by C-API contract.
                    unsafe { *view.strides.add(idx) }
                })
                .collect::<Vec<_>>(),
        )
    };
    let contiguous =
        unsafe { PyBuffer_IsContiguous(view as *const CpythonBuffer, b'A' as c_char) } != 0;
    (itemsize, format, shape, strides, contiguous)
}

fn cpython_alloc_memoryview_with_layout(
    context: &mut ModuleCapiContext,
    bytes: Vec<u8>,
    writable: bool,
    itemsize: usize,
    format: Option<String>,
    shape: Option<Vec<isize>>,
    strides: Option<Vec<isize>>,
    contiguous: bool,
) -> *mut c_void {
    if context.vm.is_null() {
        context.set_error("memoryview allocation missing VM context");
        return std::ptr::null_mut();
    }
    // SAFETY: VM pointer is valid for context lifetime.
    let vm = unsafe { &mut *context.vm };
    let source = if writable {
        vm.heap.alloc_bytearray(bytes.clone())
    } else {
        vm.heap.alloc_bytes(bytes.clone())
    };
    let source_obj = match &source {
        Value::Bytes(obj) | Value::ByteArray(obj) => obj.clone(),
        _ => {
            context.set_error("memoryview allocation expected bytes-like source");
            return std::ptr::null_mut();
        }
    };
    let value = vm
        .heap
        .alloc_memoryview_with(source_obj, itemsize.max(1), format.clone());
    if let Value::MemoryView(view_obj) = &value
        && let Object::MemoryView(view_data) = &mut *view_obj.kind_mut()
    {
        view_data.shape = shape;
        view_data.strides = strides;
        view_data.contiguous = contiguous;
        view_data.length = Some(bytes.len());
        view_data.start = 0;
        view_data.released = false;
        view_data.format = format;
    }
    context.alloc_cpython_ptr_for_value(value)
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
pub unsafe extern "C" fn PyMemoryView_FromMemory(
    mem: *mut c_char,
    size: isize,
    flags: i32,
) -> *mut c_void {
    if mem.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyMemoryView_FromMemory(): mem must not be NULL",
        );
        return std::ptr::null_mut();
    }
    if size < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyMemoryView_FromMemory(): size must be >= 0",
        );
        return std::ptr::null_mut();
    }
    if flags != 0x0100 && flags != 0x0200 {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: caller promises `mem` points to at least `size` bytes.
    let payload = unsafe { std::slice::from_raw_parts(mem.cast::<u8>(), size as usize) }.to_vec();
    let writable = flags == 0x0200;
    with_active_cpython_context_mut(|context| {
        cpython_alloc_memoryview_with_layout(
            context,
            payload,
            writable,
            1,
            Some("B".to_string()),
            None,
            None,
            true,
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMemoryView_FromBuffer(info: *const CpythonBuffer) -> *mut c_void {
    if info.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: caller passed a valid `Py_buffer`.
    let view = unsafe { &*info };
    if view.buf.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyMemoryView_FromBuffer(): info->buf must not be NULL",
        );
        return std::ptr::null_mut();
    }
    if view.len < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyMemoryView_FromBuffer(): info->len must be >= 0",
        );
        return std::ptr::null_mut();
    }
    let len = view.len as usize;
    // SAFETY: caller promises `buf` points to at least `len` bytes.
    let payload = unsafe { std::slice::from_raw_parts(view.buf.cast::<u8>(), len) }.to_vec();
    let writable = view.readonly == 0;
    let (itemsize, format, shape, strides, contiguous) = cpython_buffer_layout_from_view(view);
    with_active_cpython_context_mut(|context| {
        cpython_alloc_memoryview_with_layout(
            context, payload, writable, itemsize, format, shape, strides, contiguous,
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMemoryView_GetContiguous(
    object: *mut c_void,
    buffertype: i32,
    order: c_char,
) -> *mut c_void {
    if buffertype != 0x0100 && buffertype != 0x0200 {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let order_char = order as u8 as char;
    if !matches!(order_char, 'C' | 'F' | 'A') {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }

    let mut view = CpythonBuffer {
        buf: std::ptr::null_mut(),
        obj: std::ptr::null_mut(),
        len: 0,
        itemsize: 0,
        readonly: 1,
        ndim: 0,
        format: std::ptr::null_mut(),
        shape: std::ptr::null_mut(),
        strides: std::ptr::null_mut(),
        suboffsets: std::ptr::null_mut(),
        internal: std::ptr::null_mut(),
    };
    if unsafe { PyObject_GetBuffer(object, &mut view, 0) } != 0 {
        return std::ptr::null_mut();
    }

    if buffertype == 0x0200 && view.readonly != 0 {
        unsafe { PyBuffer_Release((&mut view as *mut CpythonBuffer).cast()) };
        cpython_set_typed_error(
            unsafe { PyExc_BufferError },
            "underlying buffer is not writable",
        );
        return std::ptr::null_mut();
    }

    if unsafe { PyBuffer_IsContiguous(&view, order) } != 0 {
        unsafe { PyBuffer_Release((&mut view as *mut CpythonBuffer).cast()) };
        return unsafe { PyMemoryView_FromObject(object) };
    }

    if buffertype == 0x0200 {
        unsafe { PyBuffer_Release((&mut view as *mut CpythonBuffer).cast()) };
        cpython_set_typed_error(
            unsafe { PyExc_BufferError },
            "writable contiguous buffer requested for a non-contiguous object.",
        );
        return std::ptr::null_mut();
    }

    let len = view.len.max(0) as usize;
    let mut contiguous = vec![0u8; len];
    let copy_status =
        unsafe { PyBuffer_ToContiguous(contiguous.as_mut_ptr().cast(), &view, view.len, order) };
    if copy_status != 0 {
        unsafe { PyBuffer_Release((&mut view as *mut CpythonBuffer).cast()) };
        return std::ptr::null_mut();
    }

    let (itemsize, format, shape, _strides, _contiguous) = cpython_buffer_layout_from_view(&view);
    let mut strides = None;
    if let Some(shape_values) = shape.clone()
        && !shape_values.is_empty()
    {
        let mut computed = vec![0isize; shape_values.len()];
        unsafe {
            PyBuffer_FillContiguousStrides(
                shape_values.len() as i32,
                shape_values.as_ptr(),
                computed.as_mut_ptr(),
                itemsize as i32,
                if order_char == 'F' {
                    b'F' as c_char
                } else {
                    b'C' as c_char
                },
            );
        }
        strides = Some(computed);
    }
    unsafe { PyBuffer_Release((&mut view as *mut CpythonBuffer).cast()) };
    with_active_cpython_context_mut(|context| {
        cpython_alloc_memoryview_with_layout(
            context, contiguous, false, itemsize, format, shape, strides, true,
        )
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
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
        let internal = Box::into_raw(Box::new(CpythonBufferInternal { handle }));
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
                internal: internal.cast(),
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

fn cpython_buffer_is_c_contiguous(view: &CpythonBuffer) -> bool {
    if view.len == 0 {
        return true;
    }
    if view.strides.is_null() {
        return true;
    }
    if view.ndim <= 0 || view.shape.is_null() {
        return false;
    }
    let mut stride = view.itemsize;
    for dim in (0..(view.ndim as usize)).rev() {
        // SAFETY: shape/strides are validated as non-null and indexed by ndim.
        let size = unsafe { *view.shape.add(dim) };
        // SAFETY: shape/strides are validated as non-null and indexed by ndim.
        let actual = unsafe { *view.strides.add(dim) };
        if size > 1 && actual != stride {
            return false;
        }
        stride = stride.saturating_mul(size);
    }
    true
}

fn cpython_buffer_is_f_contiguous(view: &CpythonBuffer) -> bool {
    if view.len == 0 {
        return true;
    }
    if view.strides.is_null() {
        if view.ndim <= 1 {
            return true;
        }
        if view.shape.is_null() {
            return false;
        }
        let mut gt_one = 0;
        for dim in 0..(view.ndim as usize) {
            // SAFETY: shape is validated as non-null and indexed by ndim.
            if unsafe { *view.shape.add(dim) } > 1 {
                gt_one += 1;
            }
        }
        return gt_one <= 1;
    }
    if view.ndim <= 0 || view.shape.is_null() {
        return false;
    }
    let mut stride = view.itemsize;
    for dim in 0..(view.ndim as usize) {
        // SAFETY: shape/strides are validated as non-null and indexed by ndim.
        let size = unsafe { *view.shape.add(dim) };
        // SAFETY: shape/strides are validated as non-null and indexed by ndim.
        let actual = unsafe { *view.strides.add(dim) };
        if size > 1 && actual != stride {
            return false;
        }
        stride = stride.saturating_mul(size);
    }
    true
}

fn cpython_buffer_add_one_index_c(index: &mut [isize], shape: &[isize]) {
    for pos in (0..index.len()).rev() {
        if index[pos] < shape[pos].saturating_sub(1) {
            index[pos] += 1;
            break;
        }
        index[pos] = 0;
    }
}

fn cpython_buffer_add_one_index_f(index: &mut [isize], shape: &[isize]) {
    for pos in 0..index.len() {
        if index[pos] < shape[pos].saturating_sub(1) {
            index[pos] += 1;
            break;
        }
        index[pos] = 0;
    }
}

fn cpython_buffer_itemsize_from_format_char(ch: char) -> Option<isize> {
    let size = match ch {
        'x' | 'c' | 'b' | 'B' | '?' => 1,
        'h' | 'H' | 'e' => 2,
        'i' | 'I' | 'l' | 'L' | 'f' | 'n' | 'N' => 4,
        'q' | 'Q' | 'd' | 'P' => 8,
        _ => return None,
    };
    Some(size)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_IsContiguous(view: *const CpythonBuffer, order: c_char) -> i32 {
    if view.is_null() {
        return 0;
    }
    // SAFETY: caller provided a valid Py_buffer pointer.
    let view = unsafe { &*view };
    if !view.suboffsets.is_null() {
        return 0;
    }
    let order = order as u8 as char;
    let contiguous = match order {
        'C' => cpython_buffer_is_c_contiguous(view),
        'F' => cpython_buffer_is_f_contiguous(view),
        'A' => cpython_buffer_is_c_contiguous(view) || cpython_buffer_is_f_contiguous(view),
        _ => false,
    };
    if contiguous { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_GetPointer(
    view: *const CpythonBuffer,
    indices: *const isize,
) -> *mut c_void {
    if view.is_null() || indices.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller provided pointers are valid per C-API contract.
    let view = unsafe { &*view };
    let mut pointer = view.buf.cast::<u8>();
    let ndim = view.ndim.max(0) as usize;
    for dim in 0..ndim {
        let stride = if view.strides.is_null() {
            if view.shape.is_null() {
                view.itemsize
            } else {
                let mut computed = view.itemsize;
                for next in ((dim + 1)..ndim).rev() {
                    // SAFETY: shape is valid for ndim entries.
                    computed = computed.saturating_mul(unsafe { *view.shape.add(next) });
                }
                computed
            }
        } else {
            // SAFETY: strides is valid for ndim entries.
            unsafe { *view.strides.add(dim) }
        };
        // SAFETY: pointers are valid for ndim entries.
        let index = unsafe { *indices.add(dim) };
        // SAFETY: pointer arithmetic follows caller-provided buffer bounds contract.
        pointer = unsafe { pointer.offset(stride.saturating_mul(index)) };
        if !view.suboffsets.is_null() {
            // SAFETY: suboffsets is valid for ndim entries.
            let suboffset = unsafe { *view.suboffsets.add(dim) };
            if suboffset >= 0 {
                // SAFETY: pointer currently addresses a valid pointer-sized slot.
                let indirect = unsafe { *(pointer.cast::<*mut u8>()) };
                // SAFETY: pointer arithmetic follows caller-provided buffer bounds contract.
                pointer = unsafe { indirect.offset(suboffset) };
            }
        }
    }
    pointer.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_SizeFromFormat(format: *const c_char) -> isize {
    if format.is_null() {
        return 1;
    }
    let text = match unsafe { c_name_to_string(format) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let mut chars = text.chars();
    let mut first = chars.next().unwrap_or('B');
    if matches!(first, '@' | '=' | '<' | '>' | '!') {
        first = chars.next().unwrap_or('B');
    }
    match cpython_buffer_itemsize_from_format_char(first) {
        Some(size) => size,
        None => {
            cpython_set_error("PyBuffer_SizeFromFormat unsupported format");
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_FillContiguousStrides(
    ndim: i32,
    shape: *const isize,
    strides: *mut isize,
    itemsize: i32,
    fort: c_char,
) {
    if ndim <= 0 || shape.is_null() || strides.is_null() {
        return;
    }
    let mut stride = itemsize as isize;
    let fort = fort as u8 as char;
    if fort == 'F' {
        for dim in 0..(ndim as usize) {
            // SAFETY: caller provided valid shape/strides arrays.
            unsafe { *strides.add(dim) = stride };
            // SAFETY: caller provided valid shape array.
            stride = stride.saturating_mul(unsafe { *shape.add(dim) });
        }
    } else {
        for dim in (0..(ndim as usize)).rev() {
            // SAFETY: caller provided valid shape/strides arrays.
            unsafe { *strides.add(dim) = stride };
            // SAFETY: caller provided valid shape array.
            stride = stride.saturating_mul(unsafe { *shape.add(dim) });
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_FillInfo(
    view: *mut CpythonBuffer,
    object: *mut c_void,
    buf: *mut c_void,
    len: isize,
    readonly: i32,
    flags: i32,
) -> i32 {
    const PYBUF_SIMPLE: i32 = 0;
    const PYBUF_WRITABLE: i32 = 0x0001;
    const PYBUF_FORMAT: i32 = 0x0004;
    const PYBUF_ND: i32 = 0x0008;
    const PYBUF_STRIDES: i32 = 0x0010 | PYBUF_ND;
    const PYBUF_READ: i32 = 0x0100;
    const PYBUF_WRITE: i32 = 0x0200;

    if view.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_BufferError },
            "PyBuffer_FillInfo: view==NULL argument is obsolete",
        );
        return -1;
    }
    if flags != PYBUF_SIMPLE {
        if flags == PYBUF_READ || flags == PYBUF_WRITE {
            unsafe { PyErr_BadInternalCall() };
            return -1;
        }
        if (flags & PYBUF_WRITABLE) == PYBUF_WRITABLE && readonly == 1 {
            cpython_set_typed_error(unsafe { PyExc_BufferError }, "Object is not writable.");
            return -1;
        }
    }
    // SAFETY: caller passed a valid writable Py_buffer pointer.
    unsafe {
        (*view).obj = object;
        Py_XIncRef(object);
        (*view).buf = buf;
        (*view).len = len;
        (*view).readonly = readonly;
        (*view).itemsize = 1;
        (*view).format = std::ptr::null_mut();
        if (flags & PYBUF_FORMAT) == PYBUF_FORMAT {
            (*view).format = c"B".as_ptr().cast_mut();
        }
        (*view).ndim = 1;
        (*view).shape = std::ptr::null_mut();
        if (flags & PYBUF_ND) == PYBUF_ND {
            (*view).shape = std::ptr::addr_of_mut!((*view).len);
        }
        (*view).strides = std::ptr::null_mut();
        if (flags & PYBUF_STRIDES) == PYBUF_STRIDES {
            (*view).strides = std::ptr::addr_of_mut!((*view).itemsize);
        }
        (*view).suboffsets = std::ptr::null_mut();
        (*view).internal = std::ptr::null_mut();
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_FromContiguous(
    view: *const CpythonBuffer,
    buf: *const c_void,
    mut len: isize,
    fort: c_char,
) -> i32 {
    if view.is_null() || buf.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    // SAFETY: caller provided valid pointers.
    let view = unsafe { &*view };
    if len > view.len {
        len = view.len;
    }
    if len <= 0 {
        return 0;
    }
    let itemsize = view.itemsize.max(1);
    if unsafe { PyBuffer_IsContiguous(view, fort) } != 0 {
        // SAFETY: caller-provided source/destination are valid for `len` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(buf.cast::<u8>(), view.buf.cast::<u8>(), len as usize)
        };
        return 0;
    }
    if view.ndim <= 0 || view.shape.is_null() || view.strides.is_null() {
        cpython_set_error("PyBuffer_FromContiguous requires shape/strides for non-contiguous view");
        return -1;
    }
    let ndim = view.ndim as usize;
    let mut indices = vec![0isize; ndim];
    let shape: Vec<isize> = (0..ndim)
        .map(|idx| {
            // SAFETY: shape pointer is valid for `ndim` entries.
            unsafe { *view.shape.add(idx) }
        })
        .collect();
    let mut src_offset = 0usize;
    let elements = (len / itemsize).max(0) as usize;
    let src = buf.cast::<u8>();
    let use_fortran = (fort as u8 as char) == 'F';
    for _ in 0..elements {
        let dst = unsafe { PyBuffer_GetPointer(view, indices.as_ptr()) };
        if dst.is_null() {
            return -1;
        }
        // SAFETY: source and destination each have `itemsize` bytes for this element.
        unsafe {
            std::ptr::copy_nonoverlapping(src.add(src_offset), dst.cast::<u8>(), itemsize as usize)
        };
        src_offset += itemsize as usize;
        if use_fortran {
            cpython_buffer_add_one_index_f(&mut indices, &shape);
        } else {
            cpython_buffer_add_one_index_c(&mut indices, &shape);
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_ToContiguous(
    buf: *mut c_void,
    src: *const CpythonBuffer,
    len: isize,
    order: c_char,
) -> i32 {
    if buf.is_null() || src.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    // SAFETY: caller provided valid pointers.
    let src = unsafe { &*src };
    if len != src.len {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyBuffer_ToContiguous: len != view->len",
        );
        return -1;
    }
    if len <= 0 {
        return 0;
    }
    let requested_order = order as u8 as char;
    if unsafe { PyBuffer_IsContiguous(src, order) } != 0 {
        // SAFETY: destination and source are valid for `len` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(src.buf.cast::<u8>(), buf.cast::<u8>(), len as usize);
        }
        return 0;
    }
    if src.ndim <= 0 || src.shape.is_null() || src.strides.is_null() {
        cpython_set_error("PyBuffer_ToContiguous requires shape/strides for non-contiguous source");
        return -1;
    }
    let use_fortran = if requested_order == 'F' {
        true
    } else if requested_order == 'A' {
        cpython_buffer_is_f_contiguous(src) && !cpython_buffer_is_c_contiguous(src)
    } else {
        false
    };
    let ndim = src.ndim as usize;
    let shape: Vec<isize> = (0..ndim)
        .map(|idx| {
            // SAFETY: shape pointer is valid for `ndim` entries.
            unsafe { *src.shape.add(idx) }
        })
        .collect();
    let mut indices = vec![0isize; ndim];
    let itemsize = src.itemsize.max(1);
    let elements = (len / itemsize).max(0) as usize;
    let mut dst_offset = 0usize;
    for _ in 0..elements {
        let source_ptr = unsafe { PyBuffer_GetPointer(src, indices.as_ptr()) };
        if source_ptr.is_null() {
            return -1;
        }
        // SAFETY: source and destination each have `itemsize` bytes for this element.
        unsafe {
            std::ptr::copy_nonoverlapping(
                source_ptr.cast::<u8>(),
                buf.cast::<u8>().add(dst_offset),
                itemsize as usize,
            )
        };
        dst_offset += itemsize as usize;
        if use_fortran {
            cpython_buffer_add_one_index_f(&mut indices, &shape);
        } else {
            cpython_buffer_add_one_index_c(&mut indices, &shape);
        }
    }
    0
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
    let method_name = if trace_calls {
        // SAFETY: method definition pointer is valid for metadata reads.
        unsafe {
            c_name_to_string((*method_def).ml_name).unwrap_or_else(|_| "<invalid>".to_string())
        }
    } else {
        String::new()
    };
    // SAFETY: method definition layout follows CPython ABI.
    let flags = unsafe { (*method_def).ml_flags };
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
        let mut kw_names = Vec::with_capacity(kwargs.len());
        for (name, value) in &kwargs {
            kw_names.push(Value::Str(name.clone()));
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
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let kwnames_ptr = if kw_names.is_empty() {
            std::ptr::null_mut()
        } else {
            context.alloc_cpython_ptr_for_value(vm.heap.alloc_tuple(kw_names))
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
        let mut stack: Vec<*mut c_void> =
            Vec::with_capacity(args.len().saturating_add(kwargs.len()));
        for value in &args {
            let ptr = context.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                context.set_error("failed to materialize FASTCALL positional argument");
                return std::ptr::null_mut();
            }
            stack.push(ptr);
        }
        let mut kw_names = Vec::with_capacity(kwargs.len());
        for (name, value) in &kwargs {
            kw_names.push(Value::Str(name.clone()));
            let ptr = context.alloc_cpython_ptr_for_value(value.clone());
            if ptr.is_null() {
                context.set_error("failed to materialize FASTCALL keyword argument");
                return std::ptr::null_mut();
            }
            stack.push(ptr);
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let kwnames_ptr = if kw_names.is_empty() {
            std::ptr::null_mut()
        } else {
            context.alloc_cpython_ptr_for_value(vm.heap.alloc_tuple(kw_names))
        };
        if !kwargs.is_empty() && kwnames_ptr.is_null() {
            context.set_error("failed to materialize FASTCALL keyword names");
            return std::ptr::null_mut();
        }
        let call: unsafe extern "C" fn(*mut c_void, *const *mut c_void, usize, *mut c_void) -> *mut c_void =
            // SAFETY: method flags indicate FASTCALL signature.
            unsafe { std::mem::transmute(method) };
        let args_ptr = if stack.is_empty() {
            std::ptr::null()
        } else {
            stack.as_ptr()
        };
        let result = unsafe { call(self_obj, args_ptr, args.len(), kwnames_ptr) };
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
        let call: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: method flags indicate VARARGS signature.
            unsafe { std::mem::transmute(method) };
        return unsafe { call(self_obj, args_ptr) };
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
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if (subtype as usize) < MIN_VALID_PTR || (ty as usize) < MIN_VALID_PTR {
        return 0;
    }
    let target = ty.cast::<CpythonTypeObject>();
    let mut current = subtype.cast::<CpythonTypeObject>();
    while !current.is_null() {
        if (current as usize) < MIN_VALID_PTR {
            return 0;
        }
        if current == target {
            return 1;
        }
        // SAFETY: current is checked non-null.
        let next = unsafe { (*current).tp_base };
        if next == current {
            break;
        }
        current = next;
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
pub unsafe extern "C" fn PyObject_Calloc(count: usize, size: usize) -> *mut c_void {
    if count.checked_mul(size).is_none() {
        cpython_set_error("PyObject_Calloc size overflow");
        return std::ptr::null_mut();
    }
    // SAFETY: libc calloc contract; returns null on failure.
    unsafe { calloc(count, size) }
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
pub unsafe extern "C" fn PyObject_GC_IsTracked(object: *mut c_void) -> i32 {
    let value = match cpython_value_from_ptr(object) {
        Ok(value) => value,
        Err(_) => return 0,
    };
    let tracked = matches!(
        value,
        Value::List(_)
            | Value::Tuple(_)
            | Value::Dict(_)
            | Value::Set(_)
            | Value::FrozenSet(_)
            | Value::Instance(_)
            | Value::Class(_)
            | Value::Module(_)
            | Value::MemoryView(_)
    );
    i32::from(tracked)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_IsFinalized(_object: *mut c_void) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_Del(object: *mut c_void) {
    unsafe { PyObject_Free(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGC_Collect() -> isize {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyGC_Collect missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_gc_collect(Vec::new(), HashMap::new()) {
            Ok(Value::Int(value)) => value as isize,
            Ok(Value::BigInt(value)) => {
                if let Some(compact) = value.to_i64() {
                    compact.clamp(isize::MIN as i64, isize::MAX as i64) as isize
                } else if value.is_negative() {
                    isize::MIN
                } else {
                    isize::MAX
                }
            }
            Ok(_) => 0,
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
pub unsafe extern "C" fn PyGC_Enable() -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyGC_Enable missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let was_enabled = vm.gc_enabled;
        vm.gc_enabled = true;
        i32::from(was_enabled)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGC_Disable() -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyGC_Disable missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let was_enabled = vm.gc_enabled;
        vm.gc_enabled = false;
        i32::from(was_enabled)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGC_IsEnabled() -> i32 {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyGC_IsEnabled missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        i32::from(vm.gc_enabled)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
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
    let trace_dealloc = std::env::var_os("PYRS_TRACE_CPY_DEALLOC").is_some();
    let object_type_name = if trace_dealloc {
        // SAFETY: best-effort debug read for candidate PyObject*.
        let ty_ptr =
            unsafe { (*object.cast::<CpythonObjectHead>()).ob_type }.cast::<CpythonTypeObject>();
        if ty_ptr.is_null() {
            "<null>".to_string()
        } else {
            // SAFETY: best-effort type name read for tracing.
            unsafe { c_name_to_string((*ty_ptr).tp_name) }
                .unwrap_or_else(|_| "<invalid>".to_string())
        }
    } else {
        String::new()
    };
    if trace_dealloc {
        eprintln!(
            "[cpy-dealloc] object={:p} type={}",
            object, object_type_name
        );
    }
    enum DeallocAction {
        Handled,
        NoContextMatch,
    }
    let action = with_active_cpython_context_mut(|context| {
        let Some(handle) = context.cpython_handle_from_ptr(object) else {
            return DeallocAction::NoContextMatch;
        };
        let _ = context.decref(handle);
        DeallocAction::Handled
    })
    .unwrap_or(DeallocAction::NoContextMatch);
    if trace_dealloc {
        let action_label = match action {
            DeallocAction::Handled => "handled",
            DeallocAction::NoContextMatch => "none",
        };
        eprintln!("[cpy-dealloc] action={action_label} object={:p}", object);
    }
    if matches!(
        action,
        DeallocAction::Handled | DeallocAction::NoContextMatch
    ) {
        return;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyErr_BadInternalCall(_filename: *const c_char, _lineno: i32) {
    cpython_set_error("bad internal call");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_BadInternalCall() {
    unsafe { _PyErr_BadInternalCall(std::ptr::null(), 0) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_BadArgument() -> i32 {
    // SAFETY: exception singletons are process-lifetime globals.
    unsafe {
        PyErr_SetString(
            PyExc_TypeError,
            c"bad argument type for built-in operation".as_ptr(),
        )
    };
    0
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
        Ok(message) => {
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
            let _ = with_active_cpython_context_mut(|context| {
                let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
                let ptype = if _exception.is_null() {
                    unsafe { PyExc_RuntimeError }
                } else {
                    _exception
                };
                context.set_error_state(ptype, pvalue, std::ptr::null_mut(), message);
            })
            .map_err(|err| {
                cpython_set_error(err);
            });
        }
        Err(err) => cpython_set_error(format!("PyErr_SetString invalid message: {err}")),
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
    if ptr.is_null() {
        return false;
    }
    // SAFETY: pointer is inspected as a CPython object header.
    let object_type = unsafe {
        ptr.cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    let expected_type = std::ptr::addr_of_mut!(PyType_Type).cast::<CpythonTypeObject>();
    if object_type == expected_type {
        return true;
    }
    if object_type.is_null() {
        return false;
    }
    // SAFETY: `object_type` is a valid `PyTypeObject` for CPython-compatible pointers.
    unsafe { ((*object_type).tp_flags & PY_TPFLAGS_TYPE_SUBCLASS) != 0 }
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
            if context.vm.is_null() {
                return None;
            }
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *context.vm };
            let class = vm.alloc_synthetic_exception_class(&exception_obj.name);
            Some(context.alloc_cpython_ptr_for_value(Value::Class(class)))
        }
        Value::Instance(instance) => {
            if !cpython_is_exception_instance(context, instance) {
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

fn pyrs_pyerr_format_basic(format: *const c_char) -> *mut c_void {
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

fn pyrs_pyerr_formatv_basic(format: *const c_char, _vargs: *mut c_void) -> *mut c_void {
    pyrs_pyerr_format_basic(format)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_pyerr_format_fallback(
    _exception: *mut c_void,
    format: *const c_char,
) -> *mut c_void {
    pyrs_pyerr_format_basic(format)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_pyerr_formatv_fallback(
    _exception: *mut c_void,
    format: *const c_char,
    vargs: *mut c_void,
) -> *mut c_void {
    pyrs_pyerr_formatv_basic(format, vargs)
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
pub static mut PyExc_EOFError: *mut c_void = std::ptr::null_mut();
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
pub static mut PyType_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_TYPE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyBool_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_BOOL.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyByteArray_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_BYTEARRAY.as_ptr().cast());
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

unsafe extern "C" {
    fn PyTuple_Pack(size: isize, ...) -> *mut c_void;
    fn Py_BuildValue(format: *const c_char, ...) -> *mut c_void;
    fn PyObject_CallFunction(callable: *mut c_void, format: *const c_char, ...) -> *mut c_void;
    fn PyObject_CallFunctionObjArgs(callable: *mut c_void, ...) -> *mut c_void;
    fn PyObject_CallMethod(
        object: *mut c_void,
        method: *const c_char,
        format: *const c_char,
        ...
    ) -> *mut c_void;
    fn PyObject_CallMethodObjArgs(object: *mut c_void, method: *mut c_void, ...) -> *mut c_void;
    fn PyArg_ParseTuple(args: *mut c_void, format: *const c_char, ...) -> i32;
    fn PyArg_ParseTupleAndKeywords(
        args: *mut c_void,
        kwargs: *mut c_void,
        format: *const c_char,
        keywords: *mut *const c_char,
        ...
    ) -> i32;
    fn PyErr_Format(exception: *mut c_void, format: *const c_char, ...) -> *mut c_void;
    fn PyErr_FormatV(
        exception: *mut c_void,
        format: *const c_char,
        vargs: *mut c_void,
    ) -> *mut c_void;
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
static KEEP2_PYMAPPING_HASKEYSTRINGWITHERROR: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyMapping_HasKeyStringWithError;
#[used]
static KEEP2_PYMAPPING_HASKEY: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
    PyMapping_HasKey;
#[used]
static KEEP2_PYMAPPING_HASKEYSTRING: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32 =
    PyMapping_HasKeyString;
#[used]
static KEEP2_PYSEQITER_NEW: unsafe extern "C" fn(*mut c_void) -> *mut c_void = PySeqIter_New;
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
static KEEP3_PYARG_PARSETUPLE: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> i32 =
    PyArg_ParseTuple;
#[used]
static KEEP3_PYARG_PARSETUPLEANDKEYWORDS: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    *mut *const c_char,
    ...
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
static KEEP3_PYERR_FORMAT: unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> *mut c_void =
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
static KEEP_PYBYTES_FROM_OBJECT: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyBytes_FromObject;
#[used]
static KEEP_PYBYTES_CONCAT: unsafe extern "C" fn(*mut *mut c_void, *mut c_void) = PyBytes_Concat;
#[used]
static KEEP_PYBYTES_CONCAT_AND_DEL: unsafe extern "C" fn(*mut *mut c_void, *mut c_void) =
    PyBytes_ConcatAndDel;
#[used]
static KEEP_PYERR_SET_STRING: unsafe extern "C" fn(*mut c_void, *const c_char) = PyErr_SetString;
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
static KEEP_PYIMPORT_GET_MODULE: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyImport_GetModule;
#[used]
static KEEP_PYIMPORT_IMPORT_MODULE_NO_BLOCK: unsafe extern "C" fn(*const c_char) -> *mut c_void =
    PyImport_ImportModuleNoBlock;
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
static KEEP_PYEVAL_GET_BUILTINS: unsafe extern "C" fn() -> *mut c_void = PyEval_GetBuiltins;
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
static KEEP_PYOBJECT_GETAITER: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
    PyObject_GetAIter;
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
    match context.capsule_new(pointer, name, None) {
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

    fn register_cpython_module_methods_from_def(
        &mut self,
        module: &ObjRef,
        module_def: *mut CpythonModuleDef,
    ) -> Result<(), RuntimeError> {
        if module_def.is_null() {
            return Ok(());
        }
        // SAFETY: module_def points to extension-provided module definition.
        let methods_ptr = unsafe { (*module_def).m_methods };
        if std::env::var_os("PYRS_TRACE_CPY_MODULE_METHODS").is_some() {
            let module_name = match &*module.kind() {
                Object::Module(module_data) => module_data.name.clone(),
                _ => "<non-module>".to_string(),
            };
            eprintln!(
                "[cpy-module-methods] module={} module_def={:p} methods_ptr={:p}",
                module_name, module_def, methods_ptr
            );
        }
        if methods_ptr.is_null() {
            return Ok(());
        }
        let mut method = methods_ptr;
        loop {
            // SAFETY: method table is terminated by null `ml_name`.
            let method_name_ptr = unsafe { (*method).ml_name };
            if method_name_ptr.is_null() {
                break;
            }
            // SAFETY: `ml_name` is NUL-terminated by PyMethodDef contract.
            let method_name =
                unsafe { c_name_to_string(method_name_ptr) }.map_err(RuntimeError::new)?;
            if std::env::var_os("PYRS_TRACE_CPY_MODULE_METHODS").is_some() {
                // SAFETY: method points to valid PyMethodDef entry.
                let flags = unsafe { (*method).ml_flags };
                eprintln!(
                    "[cpy-module-methods] register method={} def_ptr={:p} flags={}",
                    method_name, method, flags
                );
            }
            let callable = self.register_extension_callable(
                module.clone(),
                &method_name,
                ExtensionCallableKind::CpythonMethod {
                    method_def: method as usize,
                },
            )?;
            let Object::Module(module_data) = &mut *module.kind_mut() else {
                return Err(RuntimeError::new(
                    "extension module target is not a module during method registration",
                ));
            };
            module_data.globals.insert(method_name, callable);
            // SAFETY: method table entries are contiguous.
            method = unsafe { method.add(1) };
        }
        Ok(())
    }

    pub(super) fn cpython_proxy_raw_ptr_from_value(value: &Value) -> Option<*mut c_void> {
        match value {
            Value::Class(class_obj) => {
                let Object::Class(class_data) = &*class_obj.kind() else {
                    return None;
                };
                if class_data.name != "__pyrs_cpython_proxy__" {
                    return None;
                }
                match class_data.attrs.get("__pyrs_cpython_proxy_ptr__") {
                    Some(Value::Int(raw)) if *raw >= 0 => Some(*raw as usize as *mut c_void),
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
                if class_data.name != "__pyrs_cpython_proxy__" {
                    return None;
                }
                match instance_data.attrs.get("__pyrs_cpython_proxy_ptr__") {
                    Some(Value::Int(raw)) if *raw >= 0 => Some(*raw as usize as *mut c_void),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(super) fn call_cpython_proxy_object(
        &mut self,
        proxy_value: &Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let Some(raw_ptr) = Self::cpython_proxy_raw_ptr_from_value(proxy_value) else {
            return Err(RuntimeError::new(
                "internal error: proxy call target missing raw pointer",
            ));
        };
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let previous_context = cpython_set_active_context(&mut call_ctx as *mut ModuleCapiContext);
        let result_ptr = call_ctx
            .try_native_tp_call(raw_ptr, &args, &kwargs)
            .unwrap_or(std::ptr::null_mut());
        cpython_set_active_context(previous_context);
        if result_ptr.is_null() {
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "proxy object is not callable".to_string());
            return Err(RuntimeError::new(detail));
        }
        call_ctx
            .cpython_value_from_ptr_or_proxy(result_ptr)
            .ok_or_else(|| RuntimeError::new("proxy call returned unknown object pointer"))
    }

    pub(super) fn load_cpython_proxy_attr_for_value(
        &mut self,
        proxy_value: &Value,
        attr_name: &str,
    ) -> Option<Value> {
        let raw_ptr = Self::cpython_proxy_raw_ptr_from_value(proxy_value)?;
        let c_name = CString::new(attr_name).ok()?;
        let trace_type_attr =
            attr_name == "type" && std::env::var_os("PYRS_TRACE_PROXY_TYPE_ATTR").is_some();
        if trace_type_attr {
            let (raw_type, raw_type_name) = unsafe {
                let raw_type = raw_ptr
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type)
                    .unwrap_or(std::ptr::null_mut());
                let raw_type_name = raw_type
                    .cast::<CpythonTypeObject>()
                    .as_ref()
                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string());
                (raw_type, raw_type_name)
            };
            eprintln!(
                "[cpy-proxy-attr] lookup ptr={:p} type={:p} type_name={} attr={}",
                raw_ptr, raw_type, raw_type_name, attr_name
            );
        }
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let previous_context = cpython_set_active_context(&mut call_ctx as *mut ModuleCapiContext);
        let attr_ptr = unsafe { PyObject_GetAttrString(raw_ptr, c_name.as_ptr()) };
        cpython_set_active_context(previous_context);
        if attr_ptr.is_null() {
            if trace_type_attr {
                eprintln!(
                    "[cpy-proxy-attr] lookup miss ptr={:p} attr={}",
                    raw_ptr, attr_name
                );
            }
            return None;
        }
        if trace_type_attr {
            eprintln!(
                "[cpy-proxy-attr] lookup hit ptr={:p} attr={} result_ptr={:p}",
                raw_ptr, attr_name, attr_ptr
            );
        }
        call_ctx.cpython_value_from_ptr_or_proxy(attr_ptr)
    }

    pub(super) fn load_cpython_proxy_attr(
        &mut self,
        proxy_class: &ObjRef,
        attr_name: &str,
    ) -> Option<Value> {
        self.load_cpython_proxy_attr_for_value(&Value::Class(proxy_class.clone()), attr_name)
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
                let mut arg_handles = Vec::with_capacity(args.len());
                for arg in args {
                    arg_handles.push(call_ctx.alloc_object(arg));
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
                let mut arg_handles = Vec::with_capacity(args.len());
                for arg in args {
                    arg_handles.push(call_ctx.alloc_object(arg));
                }
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
            ExtensionCallableKind::CpythonMethod { method_def } => {
                let previous_context =
                    cpython_set_active_context(&mut call_ctx as *mut ModuleCapiContext);
                let self_obj =
                    call_ctx.alloc_cpython_ptr_for_value(Value::Module(entry.module.clone()));
                let result_ptr = cpython_invoke_method_from_values(
                    &mut call_ctx,
                    method_def as *mut CpythonMethodDef,
                    self_obj,
                    std::ptr::null_mut(),
                    args,
                    kwargs,
                );
                cpython_set_active_context(previous_context);
                if result_ptr.is_null() {
                    -1
                } else if let Some(result_value) =
                    call_ctx.cpython_value_from_ptr_or_proxy(result_ptr)
                {
                    result_handle = call_ctx.alloc_object(result_value);
                    0
                } else {
                    call_ctx.set_error("CPython method call returned unknown object pointer");
                    -1
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
        if matches!(&resolved_init, ResolvedInit::Cpython { .. }) {
            module_ctx.run_capsule_destructors_on_drop = false;
            module_ctx.strict_capsule_refcount = false;
        }
        let init_result = match resolved_init {
            ResolvedInit::Pyrs {
                handle,
                initializer,
            } => {
                // Keep the extension library loaded even if init fails. Module C-API
                // teardown can still invoke extension-provided callbacks (e.g. capsule
                // destructors) while unwinding error paths.
                self.extension_libraries.push(handle);
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
                std::ptr::null_mut()
            }
            ResolvedInit::Cpython {
                handle,
                initializer,
            } => {
                // See comment above in the pyrs-v1 branch; keep shared library loaded across
                // init failures to preserve callback pointer validity during teardown.
                self.extension_libraries.push(handle);
                let previous_context =
                    cpython_set_active_context(&mut module_ctx as *mut ModuleCapiContext);
                // SAFETY: symbol was resolved with `unsafe extern "C" fn() -> *mut c_void`.
                let result = unsafe { initializer() };
                cpython_set_active_context(previous_context);
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
                        self.register_cpython_module_methods_from_def(&module, module_def)?;
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
                                        if module_ctx.last_error.is_none()
                                            && std::env::var_os("PYRS_IGNORE_SLOT_STATUS_NOERROR")
                                                .is_some()
                                        {
                                            if trace_slots {
                                                eprintln!(
                                                    "[ext-slot] module={} ignoring non-zero status={} due no last_error",
                                                    module_name, status
                                                );
                                            }
                                            // Continue slot execution in explicit probe mode.
                                            cursor = unsafe { cursor.add(1) };
                                            slot_index += 1;
                                            continue;
                                        }
                                        if module_ctx.last_error.is_none()
                                            && std::env::var_os("PYRS_TRACE_EXT_SLOT_BT").is_some()
                                        {
                                            eprintln!(
                                                "[ext-slot] module={} status={} without last_error",
                                                module_name, status
                                            );
                                            eprintln!("{}", Backtrace::force_capture());
                                        }
                                        if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
                                            eprintln!(
                                                "[ext-slot] module={} slot_exec_status={} first_error={:?} last_error={:?} current_error_ptype={:p} current_error_pvalue={:p}",
                                                module_name,
                                                status,
                                                module_ctx.first_error,
                                                module_ctx.last_error,
                                                module_ctx
                                                    .current_error
                                                    .as_ref()
                                                    .map_or(std::ptr::null_mut(), |state| state
                                                        .ptype),
                                                module_ctx
                                                    .current_error
                                                    .as_ref()
                                                    .map_or(std::ptr::null_mut(), |state| state
                                                        .pvalue)
                                            );
                                        }
                                        cpython_set_active_context(previous_context);
                                        let message = module_ctx
                                            .last_error
                                            .clone()
                                            .or_else(|| module_ctx.first_error.clone())
                                            .unwrap_or_else(|| "Py_mod_exec failed".to_string());
                                        let full_error = format!(
                                            "extension '{}' initializer '{}' Py_mod_exec failed: {}",
                                            module_name, resolved_symbol, message
                                        );
                                        if std::env::var_os("PYRS_TRACE_EXT_SLOT_MODULE_KEYS")
                                            .is_some()
                                            && let Object::Module(module_data) = &*module.kind()
                                        {
                                            let mut names: Vec<String> =
                                                module_data.globals.keys().cloned().collect();
                                            names.sort();
                                            let mut probe = Vec::new();
                                            for key in [
                                                "_ARRAY_API",
                                                "False_",
                                                "True_",
                                                "add",
                                                "matmul",
                                                "arange",
                                                "ndarray",
                                            ] {
                                                probe.push(format!(
                                                    "{}={}",
                                                    key,
                                                    module_data.globals.contains_key(key)
                                                ));
                                            }
                                            eprintln!(
                                                "[ext-slot] module={} keys={} probe=[{}] sample={:?}",
                                                module_name,
                                                names.len(),
                                                probe.join(", "),
                                                names.iter().take(24).collect::<Vec<_>>()
                                            );
                                        }
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
        if symbol_family == "cpython" {
            self.extension_initialized_names
                .insert(module_name.to_string());
        } else {
            self.extension_initialized_names.remove(module_name);
        }
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
