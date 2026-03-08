use std::ffi::{CStr, c_char};

use super::Cwchar;

pub(super) unsafe fn c_name_to_bytes(name: *const c_char) -> Result<Vec<u8>, String> {
    const MAX_C_STRING_BYTES: usize = 1 << 20; // 1 MiB safety cap for native C strings.
    const MIN_VALID_PTR: usize = super::MIN_VALID_PTR_THRESHOLD;
    const SCAN_LEN: usize = MAX_C_STRING_BYTES + 1;
    if name.is_null() {
        return Err("received null C string pointer".to_string());
    }
    let base = name as usize;
    if base < MIN_VALID_PTR {
        return Err("received invalid C string pointer".to_string());
    }
    if base.checked_add(SCAN_LEN).is_none() {
        return Err("received invalid C string pointer range".to_string());
    }
    // SAFETY: caller provides at least a readable C string; cap scan length to avoid
    // runaway reads from malformed pointers/doc strings.
    let raw_bytes = unsafe { std::slice::from_raw_parts(name.cast::<u8>(), SCAN_LEN) };
    let Some(end) = raw_bytes.iter().position(|byte| *byte == 0) else {
        return Err("received unterminated C string (exceeds 1 MiB)".to_string());
    };
    let with_nul = &raw_bytes[..=end];
    let c_name = CStr::from_bytes_with_nul(with_nul)
        .map_err(|_| "received invalid C string bytes".to_string())?;
    Ok(c_name.to_bytes().to_vec())
}

pub(super) unsafe fn c_name_to_string(name: *const c_char) -> Result<String, String> {
    let bytes = unsafe { c_name_to_bytes(name) }?;
    std::str::from_utf8(&bytes)
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
