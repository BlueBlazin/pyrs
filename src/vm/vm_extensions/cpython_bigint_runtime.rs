use crate::runtime::{BigInt, Value};

pub(super) fn cpython_bigint_from_value(value: Value) -> Result<BigInt, String> {
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

fn cpython_bigint_to_unsigned_le_bytes(value: &BigInt) -> Vec<u8> {
    let abs = value.abs();
    abs.to_abs_le_bytes()
}

pub(super) fn cpython_bigint_to_twos_complement_le(value: &BigInt, n_bytes: usize) -> Vec<u8> {
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

pub(super) fn cpython_bigint_from_twos_complement_le(bytes: &[u8], signed: bool) -> BigInt {
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

pub(super) fn cpython_required_signed_bytes_for_bigint(value: &BigInt) -> usize {
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

pub(super) fn cpython_required_unsigned_bytes_for_bigint(value: &BigInt) -> usize {
    if value.is_zero() {
        1
    } else {
        value.bit_length().div_ceil(8).max(1)
    }
}

pub(super) fn cpython_asnativebytes_resolve_endian(flags: i32) -> i32 {
    if flags == -1 || (flags & 0x2) != 0 {
        if cfg!(target_endian = "little") { 1 } else { 0 }
    } else {
        flags & 0x1
    }
}
