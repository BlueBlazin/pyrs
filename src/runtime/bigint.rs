use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BigInt {
    sign: i8,
    limbs: Vec<u32>,
}

impl BigInt {
    pub fn zero() -> Self {
        Self {
            sign: 0,
            limbs: Vec::new(),
        }
    }

    pub fn one() -> Self {
        Self::from_i64(1)
    }

    pub fn from_i64(value: i64) -> Self {
        if value == 0 {
            return Self::zero();
        }
        let sign = if value < 0 { -1 } else { 1 };
        let mut magnitude = value.unsigned_abs();
        let mut limbs = Vec::new();
        while magnitude != 0 {
            limbs.push((magnitude & 0xffff_ffff) as u32);
            magnitude >>= 32;
        }
        Self { sign, limbs }
    }

    pub fn from_u64(value: u64) -> Self {
        if value == 0 {
            return Self::zero();
        }
        let mut magnitude = value;
        let mut limbs = Vec::new();
        while magnitude != 0 {
            limbs.push((magnitude & 0xffff_ffff) as u32);
            magnitude >>= 32;
        }
        Self { sign: 1, limbs }
    }

    pub fn from_f64_integral(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        if value == 0.0 {
            return Some(Self::zero());
        }
        if value.fract() != 0.0 {
            return None;
        }

        let bits = value.to_bits();
        let sign = if (bits >> 63) != 0 { -1 } else { 1 };
        let exponent = ((bits >> 52) & 0x7ff) as i32;
        if exponent == 0 {
            return Some(Self::zero());
        }

        let mantissa = bits & ((1u64 << 52) - 1);
        let mut out = Self::from_u64((1u64 << 52) | mantissa);
        let shift = exponent - 1023 - 52;
        if shift >= 0 {
            out = out.shl_bits(shift as usize);
        } else {
            let rshift = (-shift) as usize;
            if out.abs_has_low_bits(rshift) {
                return None;
            }
            out = out.abs_shr_bits(rshift);
        }

        if sign < 0 {
            out = out.negated();
        }
        Some(out)
    }

    pub fn from_str_radix(text: &str, radix: u32) -> Option<Self> {
        if radix < 2 || radix > 36 {
            return None;
        }
        if text.is_empty() {
            return None;
        }

        let mut out = Self::zero();
        for ch in text.chars() {
            let digit = ch.to_digit(radix)?;
            out = out.mul_small(radix);
            out = out.add_small(digit);
        }
        Some(out)
    }

    pub fn is_zero(&self) -> bool {
        self.sign == 0
    }

    pub fn is_negative(&self) -> bool {
        self.sign < 0
    }

    pub fn negated(&self) -> Self {
        if self.is_zero() {
            return self.clone();
        }
        let mut out = self.clone();
        out.sign = -out.sign;
        out
    }

    pub fn abs(&self) -> Self {
        if self.is_negative() {
            self.negated()
        } else {
            self.clone()
        }
    }

    pub fn bit_length(&self) -> usize {
        let Some(last) = self.limbs.last() else {
            return 0;
        };
        (self.limbs.len() - 1) * 32 + (32 - last.leading_zeros() as usize)
    }

    pub fn to_i64(&self) -> Option<i64> {
        if self.is_zero() {
            return Some(0);
        }
        if self.limbs.len() > 2 {
            return None;
        }
        let low = self.limbs.first().copied().unwrap_or(0) as u64;
        let high = self.limbs.get(1).copied().unwrap_or(0) as u64;
        let magnitude = low | (high << 32);
        if self.sign > 0 {
            if magnitude <= i64::MAX as u64 {
                return Some(magnitude as i64);
            }
            return None;
        }
        let limit = 1u64 << 63;
        if magnitude == limit {
            return Some(i64::MIN);
        }
        if magnitude < limit {
            return Some(-(magnitude as i64));
        }
        None
    }

    pub fn to_f64(&self) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        let mut value = 0.0;
        for limb in self.limbs.iter().rev() {
            value = value * 4_294_967_296.0 + (*limb as f64);
        }
        if self.sign < 0 {
            -value
        } else {
            value
        }
    }

    pub fn add(&self, other: &Self) -> Self {
        match (self.sign, other.sign) {
            (0, _) => other.clone(),
            (_, 0) => self.clone(),
            (1, 1) => Self::from_parts(1, Self::abs_add(&self.limbs, &other.limbs)),
            (-1, -1) => Self::from_parts(-1, Self::abs_add(&self.limbs, &other.limbs)),
            (1, -1) => self.sub(&other.negated()),
            (-1, 1) => other.sub(&self.negated()),
            _ => Self::zero(),
        }
    }

    pub fn sub(&self, other: &Self) -> Self {
        match (self.sign, other.sign) {
            (_, 0) => self.clone(),
            (0, _) => other.negated(),
            (1, -1) => Self::from_parts(1, Self::abs_add(&self.limbs, &other.limbs)),
            (-1, 1) => Self::from_parts(-1, Self::abs_add(&self.limbs, &other.limbs)),
            (1, 1) => match Self::abs_cmp_limbs(&self.limbs, &other.limbs) {
                Ordering::Greater => Self::from_parts(1, Self::abs_sub(&self.limbs, &other.limbs)),
                Ordering::Less => Self::from_parts(-1, Self::abs_sub(&other.limbs, &self.limbs)),
                Ordering::Equal => Self::zero(),
            },
            (-1, -1) => match Self::abs_cmp_limbs(&self.limbs, &other.limbs) {
                Ordering::Greater => Self::from_parts(-1, Self::abs_sub(&self.limbs, &other.limbs)),
                Ordering::Less => Self::from_parts(1, Self::abs_sub(&other.limbs, &self.limbs)),
                Ordering::Equal => Self::zero(),
            },
            _ => Self::zero(),
        }
    }

    pub fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let mut out = vec![0u32; self.limbs.len() + other.limbs.len()];
        for (i, left) in self.limbs.iter().enumerate() {
            let mut carry: u64 = 0;
            for (j, right) in other.limbs.iter().enumerate() {
                let idx = i + j;
                let sum = out[idx] as u64 + (*left as u64) * (*right as u64) + carry;
                out[idx] = sum as u32;
                carry = sum >> 32;
            }
            if carry != 0 {
                out[i + other.limbs.len()] = carry as u32;
            }
        }
        let sign = if self.sign == other.sign { 1 } else { -1 };
        Self::from_parts(sign, out)
    }

    pub fn pow_u64(&self, mut exponent: u64) -> Self {
        let mut base = self.clone();
        let mut out = Self::one();
        while exponent != 0 {
            if exponent & 1 == 1 {
                out = out.mul(&base);
            }
            exponent >>= 1;
            if exponent != 0 {
                base = base.mul(&base);
            }
        }
        out
    }

    pub fn shl_bits(&self, bits: usize) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        if bits == 0 {
            return self.clone();
        }
        let word_shift = bits / 32;
        let bit_shift = bits % 32;
        let mut out = vec![0u32; self.limbs.len() + word_shift + 1];
        let mut carry: u64 = 0;
        for (idx, limb) in self.limbs.iter().enumerate() {
            let shifted = ((*limb as u64) << bit_shift) | carry;
            out[idx + word_shift] = shifted as u32;
            carry = shifted >> 32;
        }
        if carry != 0 {
            out[self.limbs.len() + word_shift] = carry as u32;
        }
        Self::from_parts(self.sign, out)
    }

    pub fn shr_bits_arithmetic(&self, bits: usize) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        if bits == 0 {
            return self.clone();
        }
        if self.sign > 0 {
            return self.abs_shr_bits(bits);
        }

        let abs = self.abs();
        let quotient = abs.abs_shr_bits(bits);
        if abs.abs_has_low_bits(bits) {
            quotient.add(&Self::one()).negated()
        } else {
            quotient.negated()
        }
    }

    pub fn bitand(&self, other: &Self) -> Self {
        self.bitwise_op(other, |a, b| a & b)
    }

    pub fn bitor(&self, other: &Self) -> Self {
        self.bitwise_op(other, |a, b| a | b)
    }

    pub fn bitxor(&self, other: &Self) -> Self {
        self.bitwise_op(other, |a, b| a ^ b)
    }

    pub fn bitnot(&self) -> Self {
        self.negated().sub(&Self::one())
    }

    pub fn cmp_total(&self, other: &Self) -> Ordering {
        match (self.sign, other.sign) {
            (a, b) if a < b => Ordering::Less,
            (a, b) if a > b => Ordering::Greater,
            (0, 0) => Ordering::Equal,
            (1, 1) => Self::abs_cmp_limbs(&self.limbs, &other.limbs),
            (-1, -1) => Self::abs_cmp_limbs(&other.limbs, &self.limbs),
            _ => Ordering::Equal,
        }
    }

    pub fn div_mod_floor(&self, divisor: &Self) -> Option<(Self, Self)> {
        if divisor.is_zero() {
            return None;
        }
        if self.is_zero() {
            return Some((Self::zero(), Self::zero()));
        }

        let (abs_quotient, abs_remainder) = self.abs_div_mod_positive(&divisor.abs());
        let mut quotient = if self.sign == divisor.sign {
            abs_quotient
        } else {
            abs_quotient.negated()
        };
        let mut remainder = if self.sign < 0 {
            abs_remainder.negated()
        } else {
            abs_remainder
        };
        if !remainder.is_zero() && self.sign != divisor.sign {
            quotient = quotient.sub(&Self::one());
            remainder = remainder.add(divisor);
        }
        Some((quotient, remainder))
    }

    pub fn to_str_radix(&self, radix: u32) -> Option<String> {
        if !(2..=36).contains(&radix) {
            return None;
        }
        if self.is_zero() {
            return Some("0".to_string());
        }

        let mut value = self.abs();
        let mut out = Vec::new();
        while !value.is_zero() {
            let (quotient, rem) = value.div_mod_small_positive(radix);
            let ch = char::from_digit(rem, radix)?;
            out.push(ch);
            value = quotient;
        }
        if self.sign < 0 {
            out.push('-');
        }
        out.reverse();
        Some(out.into_iter().collect())
    }

    pub fn mul_small(&self, small: u32) -> Self {
        if self.is_zero() || small == 0 {
            return Self::zero();
        }
        if small == 1 {
            return self.clone();
        }

        let mut out = Vec::with_capacity(self.limbs.len() + 1);
        let mut carry: u64 = 0;
        for limb in &self.limbs {
            let value = (*limb as u64) * (small as u64) + carry;
            out.push(value as u32);
            carry = value >> 32;
        }
        if carry != 0 {
            out.push(carry as u32);
        }
        Self::from_parts(self.sign, out)
    }

    pub fn add_small(&self, small: u32) -> Self {
        if small == 0 {
            return self.clone();
        }
        if self.is_zero() {
            return Self::from_u64(small as u64);
        }
        if self.sign < 0 {
            return self.add(&Self::from_u64(small as u64));
        }

        let mut out = self.limbs.clone();
        let mut carry: u64 = small as u64;
        let mut idx = 0usize;
        while carry != 0 {
            if idx >= out.len() {
                out.push(carry as u32);
                break;
            }
            let sum = out[idx] as u64 + carry;
            out[idx] = sum as u32;
            carry = sum >> 32;
            idx += 1;
        }
        Self::from_parts(1, out)
    }

    fn abs_shr_bits(&self, bits: usize) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        let word_shift = bits / 32;
        let bit_shift = bits % 32;
        if word_shift >= self.limbs.len() {
            return Self::zero();
        }

        let mut out = Vec::with_capacity(self.limbs.len() - word_shift);
        if bit_shift == 0 {
            for idx in word_shift..self.limbs.len() {
                out.push(self.limbs[idx]);
            }
        } else {
            let mut carry: u32 = 0;
            for idx in (word_shift..self.limbs.len()).rev() {
                let limb = self.limbs[idx];
                let value = (limb >> bit_shift) | (carry << (32 - bit_shift));
                carry = limb;
                out.push(value);
            }
            out.reverse();
        }
        Self::from_parts(1, out)
    }

    fn abs_has_low_bits(&self, bits: usize) -> bool {
        if bits == 0 {
            return false;
        }
        let word_shift = bits / 32;
        let bit_shift = bits % 32;

        for idx in 0..word_shift.min(self.limbs.len()) {
            if self.limbs[idx] != 0 {
                return true;
            }
        }
        if bit_shift != 0 && word_shift < self.limbs.len() {
            let mask = (1u32 << bit_shift) - 1;
            if (self.limbs[word_shift] & mask) != 0 {
                return true;
            }
        }
        false
    }

    fn twos_complement_words(&self, words: usize) -> Vec<u32> {
        let mut out = vec![0u32; words];
        let copy_len = self.limbs.len().min(words);
        out[..copy_len].copy_from_slice(&self.limbs[..copy_len]);
        if self.sign >= 0 {
            return out;
        }

        for value in &mut out {
            *value = !*value;
        }
        let mut carry: u64 = 1;
        for value in &mut out {
            if carry == 0 {
                break;
            }
            let sum = *value as u64 + carry;
            *value = sum as u32;
            carry = sum >> 32;
        }
        out
    }

    fn from_twos_complement_words(words: &[u32]) -> Self {
        if words.is_empty() {
            return Self::zero();
        }
        let negative = (words[words.len() - 1] & 0x8000_0000) != 0;
        if !negative {
            return Self::from_parts(1, words.to_vec());
        }

        let mut mag = words.to_vec();
        let mut borrow: u64 = 1;
        for value in &mut mag {
            if borrow == 0 {
                break;
            }
            let cur = *value as u64;
            if cur >= borrow {
                *value = (cur - borrow) as u32;
                borrow = 0;
            } else {
                *value = ((1u128 << 32) + cur as u128 - borrow as u128) as u32;
                borrow = 1;
            }
        }
        for value in &mut mag {
            *value = !*value;
        }
        Self::from_parts(-1, mag)
    }

    fn bitwise_op<F>(&self, other: &Self, op: F) -> Self
    where
        F: Fn(u32, u32) -> u32,
    {
        let width_bits = self.bit_length().max(other.bit_length()) + 2;
        let words = (width_bits + 31) / 32;
        let left = self.twos_complement_words(words);
        let right = other.twos_complement_words(words);
        let mut out = Vec::with_capacity(words);
        for idx in 0..words {
            out.push(op(left[idx], right[idx]));
        }
        Self::from_twos_complement_words(&out)
    }

    fn div_mod_small_positive(&self, divisor: u32) -> (Self, u32) {
        debug_assert!(self.sign >= 0);
        debug_assert!(divisor > 0);
        if self.is_zero() {
            return (Self::zero(), 0);
        }

        let mut quotient = vec![0u32; self.limbs.len()];
        let mut rem: u64 = 0;
        for idx in (0..self.limbs.len()).rev() {
            let value = (rem << 32) | self.limbs[idx] as u64;
            quotient[idx] = (value / divisor as u64) as u32;
            rem = value % divisor as u64;
        }
        (Self::from_parts(1, quotient), rem as u32)
    }

    fn abs_div_mod_positive(&self, divisor: &Self) -> (Self, Self) {
        debug_assert!(divisor.sign > 0);
        if self.is_zero() {
            return (Self::zero(), Self::zero());
        }

        let mut remainder = self.abs();
        let divisor = divisor.abs();
        if remainder.cmp_total(&divisor) == Ordering::Less {
            return (Self::zero(), remainder);
        }

        let mut quotient = Self::zero();
        let one = Self::one();
        let divisor_bits = divisor.bit_length();

        while remainder.cmp_total(&divisor) != Ordering::Less {
            let mut shift = remainder.bit_length().saturating_sub(divisor_bits);
            let mut shifted = divisor.shl_bits(shift);
            if shifted.cmp_total(&remainder) == Ordering::Greater {
                if shift == 0 {
                    break;
                }
                shift -= 1;
                shifted = divisor.shl_bits(shift);
            }
            remainder = remainder.sub(&shifted);
            quotient = quotient.add(&one.shl_bits(shift));
        }

        (quotient, remainder)
    }

    fn abs_add(left: &[u32], right: &[u32]) -> Vec<u32> {
        let len = left.len().max(right.len());
        let mut out = Vec::with_capacity(len + 1);
        let mut carry: u64 = 0;
        for idx in 0..len {
            let lhs = left.get(idx).copied().unwrap_or(0) as u64;
            let rhs = right.get(idx).copied().unwrap_or(0) as u64;
            let sum = lhs + rhs + carry;
            out.push(sum as u32);
            carry = sum >> 32;
        }
        if carry != 0 {
            out.push(carry as u32);
        }
        out
    }

    fn abs_sub(left: &[u32], right: &[u32]) -> Vec<u32> {
        let mut out = Vec::with_capacity(left.len());
        let mut borrow: i64 = 0;
        for idx in 0..left.len() {
            let lhs = left[idx] as i64;
            let rhs = right.get(idx).copied().unwrap_or(0) as i64;
            let mut diff = lhs - rhs - borrow;
            if diff < 0 {
                diff += 1i64 << 32;
                borrow = 1;
            } else {
                borrow = 0;
            }
            out.push(diff as u32);
        }
        out
    }

    fn abs_cmp_limbs(left: &[u32], right: &[u32]) -> Ordering {
        if left.len() != right.len() {
            return left.len().cmp(&right.len());
        }
        for (lhs, rhs) in left.iter().rev().zip(right.iter().rev()) {
            if lhs != rhs {
                return lhs.cmp(rhs);
            }
        }
        Ordering::Equal
    }

    fn from_parts(sign: i8, mut limbs: Vec<u32>) -> Self {
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        if limbs.is_empty() {
            Self::zero()
        } else {
            Self {
                sign: if sign < 0 { -1 } else { 1 },
                limbs,
            }
        }
    }
}

impl std::fmt::Display for BigInt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_zero() {
            return f.write_str("0");
        }
        let mut value = self.abs();
        let mut chunks = Vec::new();
        while !value.is_zero() {
            let (quotient, rem) = value.div_mod_small_positive(1_000_000_000);
            chunks.push(rem);
            value = quotient;
        }
        if self.sign < 0 {
            f.write_str("-")?;
        }
        if let Some(last) = chunks.pop() {
            write!(f, "{last}")?;
            while let Some(chunk) = chunks.pop() {
                write!(f, "{chunk:09}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::BigInt;

    #[test]
    fn parses_and_formats_large_decimal() {
        let text = "340282366920938463463374607431768211455";
        let value = BigInt::from_str_radix(text, 10).expect("parse");
        assert_eq!(value.to_string(), text);
    }

    #[test]
    fn pow_and_shifts_for_large_values() {
        let two = BigInt::from_i64(2);
        let value = two.pow_u64(128);
        assert_eq!(value.bit_length(), 129);
        let shifted = value.shr_bits_arithmetic(64);
        assert_eq!(shifted.to_string(), "18446744073709551616");
    }

    #[test]
    fn signed_bitwise_matches_python_style_examples() {
        let a = BigInt::from_i64(-5);
        let b = BigInt::from_i64(3);
        assert_eq!(a.bitand(&b).to_string(), "3");
        assert_eq!(a.bitor(&b).to_string(), "-5");
        assert_eq!(a.bitxor(&b).to_string(), "-8");
        assert_eq!(a.bitnot().to_string(), "4");
    }
}
