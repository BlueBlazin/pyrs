use std::cmp::Ordering;

use super::class_name_for_instance;
use super::containers::ensure_hashable;
use super::{
    mod_float, numeric_as_complex, numeric_as_f64, numeric_pair, python_floor_div, python_mod,
    value_to_int, NumericValue,
};
use crate::runtime::{format_value, BigInt, BuiltinFunction, Heap, Object, RuntimeError, Value};

fn int_like_to_bigint(value: &Value) -> Option<BigInt> {
    match value {
        Value::Bool(flag) => Some(BigInt::from_i64(if *flag { 1 } else { 0 })),
        Value::Int(number) => Some(BigInt::from_i64(*number)),
        Value::BigInt(number) => Some(number.clone()),
        _ => None,
    }
}

fn integer_pair(left: &Value, right: &Value) -> Option<(BigInt, BigInt)> {
    let left = int_like_to_bigint(left)?;
    let right = int_like_to_bigint(right)?;
    Some((left, right))
}

fn bigint_to_value(value: BigInt) -> Value {
    match value.to_i64() {
        Some(number) => Value::Int(number),
        None => Value::BigInt(value),
    }
}

pub(super) fn add_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let Some((left, right)) = integer_pair(&left, &right) {
        return Ok(bigint_to_value(left.add(&right)));
    }
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return match (left, right) {
            (NumericValue::Int(left), NumericValue::Int(right)) => {
                Ok(bigint_to_value(
                    BigInt::from_i64(left).add(&BigInt::from_i64(right)),
                ))
            }
            (left, right) => Ok(Value::Float(numeric_as_f64(left) + numeric_as_f64(right))),
        };
    }
    if let (Some((left_real, left_imag)), Some((right_real, right_imag))) =
        (numeric_as_complex(&left), numeric_as_complex(&right))
    {
        return Ok(Value::Complex {
            real: left_real + right_real,
            imag: left_imag + right_imag,
        });
    }

    match (left, right) {
        (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
        (Value::Bytes(a), Value::Bytes(b)) => match (&*a.kind(), &*b.kind()) {
            (Object::Bytes(left), Object::Bytes(right)) => {
                let mut result = left.clone();
                result.extend(right.clone());
                Ok(heap.alloc_bytes(result))
            }
            _ => Err(RuntimeError::new("unsupported operand type for +")),
        },
        (Value::List(a), Value::List(b)) => match (&*a.kind(), &*b.kind()) {
            (Object::List(left), Object::List(right)) => {
                let mut result = left.clone();
                result.extend(right.clone());
                Ok(heap.alloc_list(result))
            }
            _ => Err(RuntimeError::new("unsupported operand type for +")),
        },
        (Value::Tuple(a), Value::Tuple(b)) => match (&*a.kind(), &*b.kind()) {
            (Object::Tuple(left), Object::Tuple(right)) => {
                let mut result = left.clone();
                result.extend(right.clone());
                Ok(heap.alloc_tuple(result))
            }
            _ => Err(RuntimeError::new("unsupported operand type for +")),
        },
        _ => Err(RuntimeError::new("unsupported operand type for +")),
    }
}

pub(super) fn sub_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let Some((left, right)) = integer_pair(&left, &right) {
        return Ok(bigint_to_value(left.sub(&right)));
    }
    match numeric_pair(&left, &right) {
        Some((NumericValue::Int(left), NumericValue::Int(right))) => {
            Ok(bigint_to_value(
                BigInt::from_i64(left).sub(&BigInt::from_i64(right)),
            ))
        }
        Some((left, right)) => Ok(Value::Float(numeric_as_f64(left) - numeric_as_f64(right))),
        None => Err(RuntimeError::new("unsupported operand type for -")),
    }
}

pub(super) fn div_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let (left, right) = numeric_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for /"))?;
    let right_value = numeric_as_f64(right);
    if right_value == 0.0 {
        return Err(RuntimeError::new("division by zero"));
    }
    Ok(Value::Float(numeric_as_f64(left) / right_value))
}

pub(super) fn floor_div_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let Some((left, right)) = integer_pair(&left, &right) {
        let (quotient, _) = left
            .div_mod_floor(&right)
            .ok_or_else(|| RuntimeError::new("division by zero"))?;
        return Ok(bigint_to_value(quotient));
    }
    let (left, right) = numeric_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for //"))?;
    match (left, right) {
        (NumericValue::Int(left), NumericValue::Int(right)) => {
            Ok(Value::Int(python_floor_div(left, right)?))
        }
        (left, right) => {
            let right_value = numeric_as_f64(right);
            if right_value == 0.0 {
                return Err(RuntimeError::new("division by zero"));
            }
            Ok(Value::Float((numeric_as_f64(left) / right_value).floor()))
        }
    }
}

pub(super) fn mod_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let Value::Str(format) = left {
        return string_percent_format(&format, right).map(Value::Str);
    }
    if let Some((left, right)) = integer_pair(&left, &right) {
        let (_, remainder) = left
            .div_mod_floor(&right)
            .ok_or_else(|| RuntimeError::new("modulo by zero"))?;
        return Ok(bigint_to_value(remainder));
    }
    let (left, right) = numeric_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for %"))?;
    match (left, right) {
        (NumericValue::Int(left), NumericValue::Int(right)) => {
            Ok(Value::Int(python_mod(left, right)?))
        }
        (left, right) => Ok(Value::Float(mod_float(
            numeric_as_f64(left),
            numeric_as_f64(right),
        )?)),
    }
}

fn string_percent_format(format: &str, right: Value) -> Result<String, RuntimeError> {
    let mut positional_args = match &right {
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => values.clone(),
            _ => vec![right.clone()],
        },
        _ => vec![right.clone()],
    };
    let mapping = match &right {
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(entries) => Some(entries.clone()),
            _ => None,
        },
        _ => None,
    };

    let chars: Vec<char> = format.chars().collect();
    let mut idx = 0usize;
    let mut arg_idx = 0usize;
    let mut used_mapping = false;
    let mut out = String::new();

    while idx < chars.len() {
        if chars[idx] != '%' {
            out.push(chars[idx]);
            idx += 1;
            continue;
        }
        idx += 1;
        if idx >= chars.len() {
            return Err(RuntimeError::new("incomplete format"));
        }
        if chars[idx] == '%' {
            out.push('%');
            idx += 1;
            continue;
        }

        let mut mapping_key: Option<String> = None;
        if chars[idx] == '(' {
            idx += 1;
            let key_start = idx;
            while idx < chars.len() && chars[idx] != ')' {
                idx += 1;
            }
            if idx >= chars.len() {
                return Err(RuntimeError::new("incomplete format key"));
            }
            mapping_key = Some(chars[key_start..idx].iter().collect());
            idx += 1;
        }

        while idx < chars.len() && "#0- +".contains(chars[idx]) {
            idx += 1;
        }
        if idx < chars.len() && chars[idx] == '*' {
            if mapping_key.is_some() {
                return Err(RuntimeError::new("format requires a mapping"));
            }
            if arg_idx >= positional_args.len() {
                return Err(RuntimeError::new("not enough arguments for format string"));
            }
            arg_idx += 1;
            idx += 1;
        } else {
            while idx < chars.len() && chars[idx].is_ascii_digit() {
                idx += 1;
            }
        }
        if idx < chars.len() && chars[idx] == '.' {
            idx += 1;
            if idx < chars.len() && chars[idx] == '*' {
                if mapping_key.is_some() {
                    return Err(RuntimeError::new("format requires a mapping"));
                }
                if arg_idx >= positional_args.len() {
                    return Err(RuntimeError::new("not enough arguments for format string"));
                }
                arg_idx += 1;
                idx += 1;
            } else {
                while idx < chars.len() && chars[idx].is_ascii_digit() {
                    idx += 1;
                }
            }
        }
        while idx < chars.len() && "hlL".contains(chars[idx]) {
            idx += 1;
        }
        if idx >= chars.len() {
            return Err(RuntimeError::new("incomplete format"));
        }
        let conversion = chars[idx];
        idx += 1;

        let value = if let Some(key) = mapping_key {
            used_mapping = true;
            let entries = mapping
                .as_ref()
                .ok_or_else(|| RuntimeError::new("format requires a mapping"))?;
            entries
                .iter()
                .find_map(|(entry_key, entry_value)| match entry_key {
                    Value::Str(name) if name == &key => Some(entry_value.clone()),
                    _ => None,
                })
                .ok_or_else(|| RuntimeError::new("format key not found"))?
        } else {
            if arg_idx >= positional_args.len() {
                return Err(RuntimeError::new("not enough arguments for format string"));
            }
            let value = positional_args[arg_idx].clone();
            arg_idx += 1;
            value
        };
        out.push_str(&format_percent_value(value, conversion)?);
    }

    if !used_mapping && arg_idx < positional_args.len() {
        return Err(RuntimeError::new(
            "not all arguments converted during string formatting",
        ));
    }

    // Keep borrow checker simple when right was a tuple by avoiding accidental future use.
    positional_args.clear();
    Ok(out)
}

fn format_percent_value(value: Value, conversion: char) -> Result<String, RuntimeError> {
    match conversion {
        's' => Ok(match value {
            Value::Str(text) => text,
            other => format_value(&other),
        }),
        'r' | 'a' => Ok(format_value(&value)),
        'd' | 'i' | 'u' => {
            let integer =
                int_like_to_bigint(&value).ok_or_else(|| RuntimeError::new("expected integer"))?;
            Ok(integer.to_string())
        }
        'x' => {
            let integer =
                int_like_to_bigint(&value).ok_or_else(|| RuntimeError::new("expected integer"))?;
            integer
                .to_str_radix(16)
                .ok_or_else(|| RuntimeError::new("unsupported operand type for hex formatting"))
        }
        'X' => {
            let integer =
                int_like_to_bigint(&value).ok_or_else(|| RuntimeError::new("expected integer"))?;
            let value = integer
                .to_str_radix(16)
                .ok_or_else(|| RuntimeError::new("unsupported operand type for hex formatting"))?;
            Ok(value.to_ascii_uppercase())
        }
        'o' => {
            let integer =
                int_like_to_bigint(&value).ok_or_else(|| RuntimeError::new("expected integer"))?;
            integer
                .to_str_radix(8)
                .ok_or_else(|| RuntimeError::new("unsupported operand type for octal formatting"))
        }
        'c' => match value {
            Value::Int(code) => {
                let code =
                    u32::try_from(code).map_err(|_| RuntimeError::new("%c arg not in range"))?;
                let ch =
                    char::from_u32(code).ok_or_else(|| RuntimeError::new("%c arg not in range"))?;
                Ok(ch.to_string())
            }
            Value::Str(text) => {
                let mut chars = text.chars();
                let ch = chars
                    .next()
                    .ok_or_else(|| RuntimeError::new("%c requires int or char"))?;
                if chars.next().is_some() {
                    return Err(RuntimeError::new("%c requires int or char"));
                }
                Ok(ch.to_string())
            }
            _ => Err(RuntimeError::new("%c requires int or char")),
        },
        _ => Err(RuntimeError::new("unsupported format character")),
    }
}

pub(super) fn pow_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let Some((left_big, right_big)) = integer_pair(&left, &right) {
        if right_big.is_negative() {
            let left = left_big.to_f64();
            let right = right_big.to_f64();
            if left == 0.0 {
                return Err(RuntimeError::new("division by zero"));
            }
            return Ok(Value::Float(left.powf(right)));
        }
        let exponent = right_big
            .to_i64()
            .ok_or_else(|| RuntimeError::new("exponent too large"))?;
        return Ok(bigint_to_value(left_big.pow_u64(exponent as u64)));
    }

    let (left, right) = numeric_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for **"))?;
    match (left, right) {
        (NumericValue::Int(left), NumericValue::Int(right)) if right >= 0 => {
            if let Some(value) = left.checked_pow(right as u32) {
                Ok(Value::Int(value))
            } else {
                Ok(Value::Float((left as f64).powf(right as f64)))
            }
        }
        (left, right) => {
            let left = numeric_as_f64(left);
            let right = numeric_as_f64(right);
            if left == 0.0 && right < 0.0 {
                return Err(RuntimeError::new("division by zero"));
            }
            Ok(Value::Float(left.powf(right)))
        }
    }
}

pub(super) fn neg_value(value: Value) -> Result<Value, RuntimeError> {
    match value {
        Value::Int(value) => {
            let value = value
                .checked_neg()
                .ok_or_else(|| RuntimeError::new("integer overflow"))?;
            Ok(Value::Int(value))
        }
        Value::BigInt(value) => Ok(bigint_to_value(value.negated())),
        Value::Bool(value) => Ok(Value::Int(if value { -1 } else { 0 })),
        Value::Float(value) => Ok(Value::Float(-value)),
        _ => Err(RuntimeError::new("unsupported operand type for -")),
    }
}

pub(super) fn pos_value(value: Value) -> Result<Value, RuntimeError> {
    match value {
        Value::Int(value) => Ok(Value::Int(value)),
        Value::BigInt(value) => Ok(bigint_to_value(value)),
        Value::Bool(value) => Ok(Value::Int(if value { 1 } else { 0 })),
        Value::Float(value) => Ok(Value::Float(value)),
        _ => Err(RuntimeError::new("unsupported operand type for +")),
    }
}

pub(super) fn invert_value(value: Value) -> Result<Value, RuntimeError> {
    let value = int_like_to_bigint(&value)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for ~"))?;
    Ok(bigint_to_value(value.bitnot()))
}

pub(super) fn and_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
        return Ok(Value::Bool(*left & *right));
    }
    let (left, right) =
        integer_pair(&left, &right).ok_or_else(|| RuntimeError::new("unsupported operand type for &"))?;
    Ok(bigint_to_value(left.bitand(&right)))
}

pub(super) fn xor_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
        return Ok(Value::Bool(*left ^ *right));
    }
    let (left, right) =
        integer_pair(&left, &right).ok_or_else(|| RuntimeError::new("unsupported operand type for ^"))?;
    Ok(bigint_to_value(left.bitxor(&right)))
}

pub(super) fn or_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
        return Ok(Value::Bool(*left | *right));
    }
    if let (Some(mut merged), Some(right_values)) = (as_set_values(&left), as_set_values(&right)) {
        for value in right_values {
            if !merged.iter().any(|existing| *existing == value) {
                merged.push(value);
            }
        }
        return match left {
            Value::Set(_) => Ok(heap.alloc_set(merged)),
            Value::FrozenSet(_) => Ok(heap.alloc_frozenset(merged)),
            _ => unreachable!(),
        };
    }
    if let (Value::Dict(left_dict), Value::Dict(right_dict)) = (&left, &right) {
        let mut merged = match &*left_dict.kind() {
            Object::Dict(entries) => entries.to_vec(),
            _ => Vec::new(),
        };
        if let Object::Dict(entries) = &*right_dict.kind() {
            for (key, value) in entries {
                if let Some((_, stored)) = merged.iter_mut().find(|(existing, _)| *existing == *key)
                {
                    *stored = value.clone();
                } else {
                    merged.push((key.clone(), value.clone()));
                }
            }
        }
        return Ok(heap.alloc_dict(merged));
    }
    if is_type_union_operand(&left) && is_type_union_operand(&right) {
        let mut members = Vec::new();
        append_type_union_members(left, &mut members);
        append_type_union_members(right, &mut members);
        return Ok(heap.alloc_tuple(members));
    }
    if (matches!(left, Value::None) && !matches!(right, Value::Int(_) | Value::Bool(_) | Value::BigInt(_)))
        || (matches!(right, Value::None)
            && !matches!(left, Value::Int(_) | Value::Bool(_) | Value::BigInt(_)))
    {
        let mut members = Vec::new();
        append_type_union_members(left, &mut members);
        append_type_union_members(right, &mut members);
        return Ok(heap.alloc_tuple(members));
    }
    let (left, right) =
        integer_pair(&left, &right).ok_or_else(|| RuntimeError::new("unsupported operand type for |"))?;
    Ok(bigint_to_value(left.bitor(&right)))
}

fn is_type_union_operand(value: &Value) -> bool {
    match value {
        Value::None | Value::Class(_) | Value::ExceptionType(_) => true,
        Value::Instance(instance) => class_name_for_instance(instance)
            .map(|name| {
                matches!(
                    name.as_str(),
                    "GenericAlias"
                        | "UnionType"
                        | "TypeVar"
                        | "TypeVarTuple"
                        | "ParamSpec"
                        | "TypeAliasType"
                )
            })
            .unwrap_or(false),
        Value::Builtin(builtin)
            if matches!(
                builtin,
                BuiltinFunction::Type
                    | BuiltinFunction::Bool
                    | BuiltinFunction::Int
                    | BuiltinFunction::Float
                    | BuiltinFunction::Str
                    | BuiltinFunction::List
                    | BuiltinFunction::Tuple
                    | BuiltinFunction::Dict
                    | BuiltinFunction::Set
                    | BuiltinFunction::FrozenSet
                    | BuiltinFunction::Bytes
                    | BuiltinFunction::ByteArray
                    | BuiltinFunction::MemoryView
                    | BuiltinFunction::Complex
                    | BuiltinFunction::ClassMethod
                    | BuiltinFunction::StaticMethod
                    | BuiltinFunction::Property
            ) =>
        {
            true
        }
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => values.iter().all(is_type_union_operand),
            _ => false,
        },
        _ => false,
    }
}

fn append_type_union_members(value: Value, members: &mut Vec<Value>) {
    if let Value::Tuple(obj) = &value {
        if let Object::Tuple(values) = &*obj.kind() {
            if values.iter().all(is_type_union_operand) {
                for member in values.iter().cloned() {
                    if !members.iter().any(|existing| *existing == member) {
                        members.push(member);
                    }
                }
                return;
            }
        }
    }
    if !members.iter().any(|existing| *existing == value) {
        members.push(value);
    }
}

pub(super) fn lshift_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let (left, right) =
        integer_pair(&left, &right).ok_or_else(|| RuntimeError::new("unsupported operand type for <<"))?;
    if right.is_negative() {
        return Err(RuntimeError::new("negative shift count"));
    }
    let shift = right
        .to_i64()
        .ok_or_else(|| RuntimeError::new("shift count too large"))?;
    let shift = usize::try_from(shift).map_err(|_| RuntimeError::new("shift count too large"))?;
    Ok(bigint_to_value(left.shl_bits(shift)))
}

pub(super) fn rshift_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let (left, right) =
        integer_pair(&left, &right).ok_or_else(|| RuntimeError::new("unsupported operand type for >>"))?;
    if right.is_negative() {
        return Err(RuntimeError::new("negative shift count"));
    }
    let shift = right
        .to_i64()
        .ok_or_else(|| RuntimeError::new("shift count too large"))?;
    let shift = usize::try_from(shift).map_err(|_| RuntimeError::new("shift count too large"))?;
    Ok(bigint_to_value(left.shr_bits_arithmetic(shift)))
}

pub(super) fn matmul_values(_left: Value, _right: Value) -> Result<Value, RuntimeError> {
    Err(RuntimeError::new("unsupported operand type for @"))
}

pub(super) fn ordering_from_cmp_value(value: Value) -> Result<Ordering, RuntimeError> {
    let numeric = match value {
        Value::Int(value) => value as f64,
        Value::BigInt(value) => value.to_f64(),
        Value::Bool(value) => {
            if value {
                1.0
            } else {
                0.0
            }
        }
        Value::Float(value) => value,
        _ => {
            return Err(RuntimeError::new(
                "cmp_to_key comparator must return a number",
            ));
        }
    };
    if numeric.is_nan() || numeric == 0.0 {
        Ok(Ordering::Equal)
    } else if numeric < 0.0 {
        Ok(Ordering::Less)
    } else {
        Ok(Ordering::Greater)
    }
}

pub(super) fn compare_order(left: Value, right: Value) -> Result<Ordering, RuntimeError> {
    if let Some((left, right)) = integer_pair(&left, &right) {
        return Ok(left.cmp_total(&right));
    }
    match (&left, &right) {
        (Value::BigInt(left), Value::Float(right)) => {
            return Ok(left.to_f64().total_cmp(right));
        }
        (Value::Float(left), Value::BigInt(right)) => {
            return Ok(left.total_cmp(&right.to_f64()));
        }
        _ => {}
    }
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return Ok(match (left, right) {
            (NumericValue::Int(left), NumericValue::Int(right)) => left.cmp(&right),
            (left, right) => numeric_as_f64(left).total_cmp(&numeric_as_f64(right)),
        });
    }

    match (left, right) {
        (Value::Str(a), Value::Str(b)) => Ok(a.cmp(&b)),
        (Value::Tuple(left), Value::Tuple(right)) => match (&*left.kind(), &*right.kind()) {
            (Object::Tuple(left), Object::Tuple(right)) => compare_sequence_order(left, right),
            _ => Err(RuntimeError::new("unsupported operand type for comparison")),
        },
        (Value::List(left), Value::List(right)) => match (&*left.kind(), &*right.kind()) {
            (Object::List(left), Object::List(right)) => compare_sequence_order(left, right),
            _ => Err(RuntimeError::new("unsupported operand type for comparison")),
        },
        _ => Err(RuntimeError::new("unsupported operand type for comparison")),
    }
}

fn compare_sequence_order(left: &[Value], right: &[Value]) -> Result<Ordering, RuntimeError> {
    for (left_item, right_item) in left.iter().zip(right.iter()) {
        let ordering = compare_order(left_item.clone(), right_item.clone())?;
        if ordering != Ordering::Equal {
            return Ok(ordering);
        }
    }
    Ok(left.len().cmp(&right.len()))
}

fn as_set_values(value: &Value) -> Option<Vec<Value>> {
    match value {
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => Some(values.to_vec()),
            _ => None,
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => Some(values.to_vec()),
            _ => None,
        },
        _ => None,
    }
}

fn set_is_subset(left: &[Value], right: &[Value]) -> bool {
    left.iter()
        .all(|needle| right.iter().any(|value| value == needle))
}

pub(super) fn compare_lt(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let (Some(left_values), Some(right_values)) = (as_set_values(&left), as_set_values(&right)) {
        let is_subset = set_is_subset(&left_values, &right_values);
        let is_equal = left_values.len() == right_values.len() && is_subset;
        return Ok(Value::Bool(is_subset && !is_equal));
    }
    Ok(Value::Bool(compare_order(left, right)? == Ordering::Less))
}

pub(super) fn compare_le(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let (Some(left_values), Some(right_values)) = (as_set_values(&left), as_set_values(&right)) {
        return Ok(Value::Bool(set_is_subset(&left_values, &right_values)));
    }
    Ok(Value::Bool(
        compare_order(left, right)? != Ordering::Greater,
    ))
}

pub(super) fn compare_gt(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let (Some(left_values), Some(right_values)) = (as_set_values(&left), as_set_values(&right)) {
        let is_superset = set_is_subset(&right_values, &left_values);
        let is_equal = left_values.len() == right_values.len() && is_superset;
        return Ok(Value::Bool(is_superset && !is_equal));
    }
    Ok(Value::Bool(
        compare_order(left, right)? == Ordering::Greater,
    ))
}

pub(super) fn compare_ge(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let (Some(left_values), Some(right_values)) = (as_set_values(&left), as_set_values(&right)) {
        return Ok(Value::Bool(set_is_subset(&right_values, &left_values)));
    }
    Ok(Value::Bool(compare_order(left, right)? != Ordering::Less))
}

pub(super) fn compare_in(left: &Value, right: &Value) -> Result<bool, RuntimeError> {
    match right {
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => Ok(values.iter().any(|value| value == left)),
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => Ok(values.iter().any(|value| value == left)),
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(entries) => {
                ensure_hashable(left)?;
                Ok(entries.contains_key(left))
            }
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => {
                ensure_hashable(left)?;
                Ok(values.contains(left))
            }
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => {
                ensure_hashable(left)?;
                Ok(values.contains(left))
            }
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::Str(haystack) => match left {
            Value::Str(needle) => Ok(haystack.contains(needle)),
            _ => Err(RuntimeError::new("in expects string on left")),
        },
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => {
                let needle = value_to_int(left.clone())?;
                if !(0..=255).contains(&needle) {
                    return Ok(false);
                }
                Ok(values.iter().any(|value| *value as i64 == needle))
            }
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => {
                let needle = value_to_int(left.clone())?;
                if !(0..=255).contains(&needle) {
                    return Ok(false);
                }
                Ok(values.iter().any(|value| *value as i64 == needle))
            }
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => match &*view.source.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => {
                    let needle = value_to_int(left.clone())?;
                    if !(0..=255).contains(&needle) {
                        return Ok(false);
                    }
                    Ok(values.iter().any(|value| *value as i64 == needle))
                }
                _ => Err(RuntimeError::new("unsupported operand type for in")),
            },
            _ => Err(RuntimeError::new("unsupported operand type for in")),
        },
        _ => Err(RuntimeError::new("unsupported operand type for in")),
    }
}

pub(super) fn mul_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let Some((left, right)) = integer_pair(&left, &right) {
        return Ok(bigint_to_value(left.mul(&right)));
    }
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return match (left, right) {
            (NumericValue::Int(left), NumericValue::Int(right)) => {
                Ok(bigint_to_value(
                    BigInt::from_i64(left).mul(&BigInt::from_i64(right)),
                ))
            }
            (left, right) => Ok(Value::Float(numeric_as_f64(left) * numeric_as_f64(right))),
        };
    }

    match (left, right) {
        (Value::Str(s), other) | (other, Value::Str(s)) => {
            let count = value_to_int(other)?;
            if count <= 0 {
                return Ok(Value::Str(String::new()));
            }
            Ok(Value::Str(s.repeat(count as usize)))
        }
        (Value::List(obj), other) | (other, Value::List(obj)) => {
            let count = value_to_int(other)?;
            if count <= 0 {
                return Ok(heap.alloc_list(Vec::new()));
            }
            let values = match &*obj.kind() {
                Object::List(values) => values.clone(),
                _ => return Err(RuntimeError::new("unsupported operand type for *")),
            };
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(heap.alloc_list(result))
        }
        (Value::Tuple(obj), other) | (other, Value::Tuple(obj)) => {
            let count = value_to_int(other)?;
            if count <= 0 {
                return Ok(heap.alloc_tuple(Vec::new()));
            }
            let values = match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::new("unsupported operand type for *")),
            };
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(heap.alloc_tuple(result))
        }
        (Value::Bytes(obj), other) | (other, Value::Bytes(obj)) => {
            let count = value_to_int(other)?;
            if count <= 0 {
                return Ok(heap.alloc_bytes(Vec::new()));
            }
            let values = match &*obj.kind() {
                Object::Bytes(values) => values.clone(),
                _ => return Err(RuntimeError::new("unsupported operand type for *")),
            };
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(heap.alloc_bytes(result))
        }
        (Value::ByteArray(obj), other) | (other, Value::ByteArray(obj)) => {
            let count = value_to_int(other)?;
            if count <= 0 {
                return Ok(heap.alloc_bytearray(Vec::new()));
            }
            let values = match &*obj.kind() {
                Object::ByteArray(values) => values.clone(),
                _ => return Err(RuntimeError::new("unsupported operand type for *")),
            };
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(heap.alloc_bytearray(result))
        }
        _ => Err(RuntimeError::new("unsupported operand type for *")),
    }
}
