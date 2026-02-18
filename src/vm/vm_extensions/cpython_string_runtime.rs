use std::ffi::{CStr, c_char};

use super::Cwchar;

pub(super) unsafe fn c_name_to_string(name: *const c_char) -> Result<String, String> {
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
pub(super) fn cpython_string_to_wide_units(text: &str) -> Vec<Cwchar> {
    text.encode_utf16().collect()
}

#[cfg(not(windows))]
pub(super) fn cpython_string_to_wide_units(text: &str) -> Vec<Cwchar> {
    text.chars().map(|ch| ch as u32 as Cwchar).collect()
}

pub(super) unsafe fn cpython_wide_ptr_to_string(
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

pub(super) unsafe fn c_wide_name_to_string(name: *const Cwchar) -> Result<String, String> {
    unsafe { cpython_wide_ptr_to_string(name, -1, "wide string decode") }
}
