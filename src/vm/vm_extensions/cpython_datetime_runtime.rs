use std::ffi::c_void;

use super::cpython_set_error;

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
