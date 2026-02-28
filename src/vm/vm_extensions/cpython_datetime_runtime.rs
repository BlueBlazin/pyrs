use std::ffi::{c_char, c_void};

use super::{
    CpythonCompatObject, CpythonTypeObject, EMPTY_TYPE_FLAGS, PY_TPFLAGS_BASETYPE,
    PY_TPFLAGS_READY, PyBaseObject_Type, PyType_Type, cpython_set_error,
};

const fn datetime_empty_type(name: *const c_char) -> CpythonTypeObject {
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

static PY_TYPE_NAME_DATETIME_DATE: &[u8; 14] = b"datetime.date\0";
static PY_TYPE_NAME_DATETIME_DATETIME: &[u8; 18] = b"datetime.datetime\0";
static PY_TYPE_NAME_DATETIME_TIME: &[u8; 14] = b"datetime.time\0";
static PY_TYPE_NAME_DATETIME_DELTA: &[u8; 19] = b"datetime.timedelta\0";
static PY_TYPE_NAME_DATETIME_TZINFO: &[u8; 16] = b"datetime.tzinfo\0";

pub(super) static mut PYRS_DATETIME_DATE_TYPE: CpythonTypeObject =
    datetime_empty_type(PY_TYPE_NAME_DATETIME_DATE.as_ptr().cast());
pub(super) static mut PYRS_DATETIME_DATETIME_TYPE: CpythonTypeObject =
    datetime_empty_type(PY_TYPE_NAME_DATETIME_DATETIME.as_ptr().cast());
pub(super) static mut PYRS_DATETIME_TIME_TYPE: CpythonTypeObject =
    datetime_empty_type(PY_TYPE_NAME_DATETIME_TIME.as_ptr().cast());
pub(super) static mut PYRS_DATETIME_DELTA_TYPE: CpythonTypeObject =
    datetime_empty_type(PY_TYPE_NAME_DATETIME_DELTA.as_ptr().cast());
pub(super) static mut PYRS_DATETIME_TZINFO_TYPE: CpythonTypeObject =
    datetime_empty_type(PY_TYPE_NAME_DATETIME_TZINFO.as_ptr().cast());

pub(super) fn initialize_datetime_capi_types() {
    // SAFETY: these static type objects are process-lifetime compatibility
    // descriptors for the datetime capsule and are initialized idempotently.
    unsafe {
        let type_ptr = std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>();
        let base_ptr = std::ptr::addr_of_mut!(PyBaseObject_Type);

        for ty in &mut [
            std::ptr::addr_of_mut!(PYRS_DATETIME_DATE_TYPE),
            std::ptr::addr_of_mut!(PYRS_DATETIME_DATETIME_TYPE),
            std::ptr::addr_of_mut!(PYRS_DATETIME_TIME_TYPE),
            std::ptr::addr_of_mut!(PYRS_DATETIME_DELTA_TYPE),
            std::ptr::addr_of_mut!(PYRS_DATETIME_TZINFO_TYPE),
        ] {
            (**ty).ob_type = type_ptr;
            (**ty).tp_base = base_ptr;
            (**ty).tp_flags |= PY_TPFLAGS_BASETYPE | PY_TPFLAGS_READY;
            (**ty).tp_alloc = PyType_Type.tp_alloc;
            (**ty).tp_new = PyType_Type.tp_new;
        }

        PYRS_DATETIME_DATE_TYPE.tp_basicsize = 32;
        PYRS_DATETIME_DATETIME_TYPE.tp_basicsize = 48;
        PYRS_DATETIME_TIME_TYPE.tp_basicsize = 40;
        PYRS_DATETIME_DELTA_TYPE.tp_basicsize = 40;
        PYRS_DATETIME_TZINFO_TYPE.tp_basicsize = 16;
    }
}

#[repr(C)]
pub(super) struct CpythonDateTimeCapi {
    pub(super) date_type: *mut c_void,
    pub(super) datetime_type: *mut c_void,
    pub(super) time_type: *mut c_void,
    pub(super) delta_type: *mut c_void,
    pub(super) tzinfo_type: *mut c_void,
    pub(super) timezone_utc: *mut c_void,
    pub(super) date_from_date: unsafe extern "C" fn(i32, i32, i32, *mut c_void) -> *mut c_void,
    pub(super) datetime_from_date_and_time: unsafe extern "C" fn(
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
    pub(super) time_from_time:
        unsafe extern "C" fn(i32, i32, i32, i32, *mut c_void, *mut c_void) -> *mut c_void,
    pub(super) delta_from_delta:
        unsafe extern "C" fn(i32, i32, i32, i32, *mut c_void) -> *mut c_void,
    pub(super) timezone_from_timezone:
        unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
    pub(super) datetime_from_timestamp:
        unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void,
    pub(super) date_from_timestamp: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
    pub(super) datetime_from_date_and_time_and_fold: unsafe extern "C" fn(
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
    pub(super) time_from_time_and_fold:
        unsafe extern "C" fn(i32, i32, i32, i32, *mut c_void, i32, *mut c_void) -> *mut c_void,
}

pub(super) const PYRS_DATETIME_CAPSULE_NAME: &str = "datetime.datetime_CAPI";

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

pub(super) static mut PYRS_DATETIME_CAPI: CpythonDateTimeCapi = CpythonDateTimeCapi {
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
