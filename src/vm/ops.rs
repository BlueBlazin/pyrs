use std::cmp::Ordering;

use super::class_name_for_instance;
use super::containers::{dedup_hashable_values, dict_contains_key_checked, ensure_hashable};
use super::{
    DICT_BACKING_STORAGE_ATTR, LIST_BACKING_STORAGE_ATTR, NumericValue, STR_BACKING_STORAGE_ATTR,
    mod_float, numeric_as_complex, numeric_as_f64, numeric_pair, python_floor_div, python_mod,
    value_to_int,
};
use crate::runtime::{
    BigInt, BuiltinFunction, Heap, Object, RuntimeError, Value, format_repr, format_value,
};

fn string_like_for_add(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.clone()),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get(STR_BACKING_STORAGE_ATTR) {
                    Some(Value::Str(text)) => Some(text.clone()),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

fn list_like_for_add(value: &Value) -> Option<Vec<Value>> {
    match value {
        Value::List(list) => match &*list.kind() {
            Object::List(values) => Some(values.clone()),
            _ => None,
        },
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get(LIST_BACKING_STORAGE_ATTR) {
                    Some(Value::List(list)) => match &*list.kind() {
                        Object::List(values) => Some(values.clone()),
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

fn int_like_to_bigint(value: &Value) -> Option<BigInt> {
    match value {
        Value::Bool(flag) => Some(BigInt::from_i64(if *flag { 1 } else { 0 })),
        Value::Int(number) => Some(BigInt::from_i64(*number)),
        Value::BigInt(number) => Some((**number).clone()),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get("__pyrs_int_storage__") {
                    Some(Value::Bool(flag)) => Some(BigInt::from_i64(if *flag { 1 } else { 0 })),
                    Some(Value::Int(number)) => Some(BigInt::from_i64(*number)),
                    Some(Value::BigInt(number)) => Some((**number).clone()),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

#[inline]
fn int_like_to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(flag) => Some(if *flag { 1 } else { 0 }),
        Value::Int(number) => Some(*number),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get("__pyrs_int_storage__") {
                    Some(Value::Bool(flag)) => Some(if *flag { 1 } else { 0 }),
                    Some(Value::Int(number)) => Some(*number),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

#[inline]
fn integer_i64_pair(left: &Value, right: &Value) -> Option<(i64, i64)> {
    let left = int_like_to_i64(left)?;
    let right = int_like_to_i64(right)?;
    Some((left, right))
}

fn integer_pair(left: &Value, right: &Value) -> Option<(BigInt, BigInt)> {
    let left = int_like_to_bigint(left)?;
    let right = int_like_to_bigint(right)?;
    Some((left, right))
}

fn bigint_to_value(value: BigInt) -> Value {
    match value.to_i64() {
        Some(number) => Value::Int(number),
        None => Value::BigInt(Box::new(value)),
    }
}

fn debug_value_kind(value: &Value) -> &'static str {
    match value {
        Value::None => "None",
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::BigInt(_) => "BigInt",
        Value::Float(_) => "Float",
        Value::Complex { .. } => "Complex",
        Value::Str(_) => "Str",
        Value::Bytes(_) => "Bytes",
        Value::ByteArray(_) => "ByteArray",
        Value::Tuple(_) => "Tuple",
        Value::List(_) => "List",
        Value::Dict(_) => "Dict",
        Value::Set(_) => "Set",
        Value::FrozenSet(_) => "FrozenSet",
        Value::Slice(_) => "Slice",
        Value::Iterator(_) => "Iterator",
        Value::MemoryView(_) => "MemoryView",
        Value::Code(_) => "Code",
        Value::Function(_) => "Function",
        Value::Generator(_) => "Generator",
        Value::Builtin(_) => "Builtin",
        Value::Class(_) => "Class",
        Value::Instance(_) => "Instance",
        Value::BoundMethod(_) => "BoundMethod",
        Value::Module(_) => "Module",
        Value::Exception(_) => "Exception",
        Value::ExceptionType(_) => "ExceptionType",
        Value::Super(_) => "Super",
        Value::DictKeys(_) => "DictKeys",
        Value::Cell(_) => "Cell",
    }
}

pub(super) fn add_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let (Some(left_text), Some(right_text)) =
        (string_like_for_add(&left), string_like_for_add(&right))
    {
        return Ok(Value::Str(format!("{left_text}{right_text}")));
    }
    if let (Some(mut left_values), Some(right_values)) =
        (list_like_for_add(&left), list_like_for_add(&right))
    {
        left_values.extend(right_values);
        return Ok(heap.alloc_list(left_values));
    }

    if let Some((left, right)) = integer_i64_pair(&left, &right) {
        if let Some(sum) = left.checked_add(right) {
            return Ok(Value::Int(sum));
        }
        return Ok(bigint_to_value(
            BigInt::from_i64(left).add(&BigInt::from_i64(right)),
        ));
    }
    if let Some((left, right)) = integer_pair(&left, &right) {
        return Ok(bigint_to_value(left.add(&right)));
    }
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return match (left, right) {
            (NumericValue::Int(left), NumericValue::Int(right)) => {
                if let Some(sum) = left.checked_add(right) {
                    Ok(Value::Int(sum))
                } else {
                    Ok(bigint_to_value(
                        BigInt::from_i64(left).add(&BigInt::from_i64(right)),
                    ))
                }
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
            _ => Err(RuntimeError::type_error("unsupported operand type for +")),
        },
        (Value::Bytes(a), Value::ByteArray(b)) => match (&*a.kind(), &*b.kind()) {
            (Object::Bytes(left), Object::ByteArray(right)) => {
                let mut result = left.clone();
                result.extend(right.clone());
                Ok(heap.alloc_bytes(result))
            }
            _ => Err(RuntimeError::type_error("unsupported operand type for +")),
        },
        (Value::ByteArray(a), Value::Bytes(b)) => match (&*a.kind(), &*b.kind()) {
            (Object::ByteArray(left), Object::Bytes(right)) => {
                let mut result = left.clone();
                result.extend(right.clone());
                Ok(heap.alloc_bytearray(result))
            }
            _ => Err(RuntimeError::type_error("unsupported operand type for +")),
        },
        (Value::ByteArray(a), Value::ByteArray(b)) => match (&*a.kind(), &*b.kind()) {
            (Object::ByteArray(left), Object::ByteArray(right)) => {
                let mut result = left.clone();
                result.extend(right.clone());
                Ok(heap.alloc_bytearray(result))
            }
            _ => Err(RuntimeError::type_error("unsupported operand type for +")),
        },
        (Value::Bytes(a), other) => match &*a.kind() {
            Object::Bytes(left) => {
                if let Some(right) = bytes_like_payload(&other) {
                    let mut result = left.clone();
                    result.extend(right);
                    Ok(heap.alloc_bytes(result))
                } else {
                    Err(RuntimeError::type_error("unsupported operand type for +"))
                }
            }
            _ => Err(RuntimeError::type_error("unsupported operand type for +")),
        },
        (Value::ByteArray(a), other) => match &*a.kind() {
            Object::ByteArray(left) => {
                if let Some(right) = bytes_like_payload(&other) {
                    let mut result = left.clone();
                    result.extend(right);
                    Ok(heap.alloc_bytearray(result))
                } else {
                    Err(RuntimeError::type_error("unsupported operand type for +"))
                }
            }
            _ => Err(RuntimeError::type_error("unsupported operand type for +")),
        },
        (Value::List(a), Value::List(b)) => match (&*a.kind(), &*b.kind()) {
            (Object::List(left), Object::List(right)) => {
                let mut result = left.clone();
                result.extend(right.clone());
                Ok(heap.alloc_list(result))
            }
            _ => Err(RuntimeError::type_error("unsupported operand type for +")),
        },
        (Value::Tuple(a), Value::Tuple(b)) => match (&*a.kind(), &*b.kind()) {
            (Object::Tuple(left), Object::Tuple(right)) => {
                let mut result = left.clone();
                result.extend(right.clone());
                Ok(heap.alloc_tuple(result))
            }
            _ => Err(RuntimeError::type_error("unsupported operand type for +")),
        },
        _ => Err(RuntimeError::type_error("unsupported operand type for +")),
    }
}

pub(super) fn sub_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let Value::DictKeys(_) = left
        && let (Some(left_values), Some(right_values)) =
            (as_set_values(&left), iterable_values_for_setop(&right))
    {
        let mut difference = Vec::new();
        for value in left_values {
            if !right_values.contains(&value) {
                difference.push(value);
            }
        }
        return Ok(heap.alloc_set(dedup_hashable_values(difference)?));
    }
    if let (Some(left_values), Some(right_values)) = (as_set_values(&left), as_set_values(&right)) {
        let mut difference = Vec::new();
        for value in left_values {
            if !right_values.contains(&value) {
                difference.push(value);
            }
        }
        return set_op_result(left, difference, heap);
    }
    if let Some((left, right)) = integer_i64_pair(&left, &right) {
        if let Some(difference) = left.checked_sub(right) {
            return Ok(Value::Int(difference));
        }
        return Ok(bigint_to_value(
            BigInt::from_i64(left).sub(&BigInt::from_i64(right)),
        ));
    }
    if let Some((left, right)) = integer_pair(&left, &right) {
        return Ok(bigint_to_value(left.sub(&right)));
    }
    match numeric_pair(&left, &right) {
        Some((NumericValue::Int(left), NumericValue::Int(right))) => {
            if let Some(difference) = left.checked_sub(right) {
                Ok(Value::Int(difference))
            } else {
                Ok(bigint_to_value(
                    BigInt::from_i64(left).sub(&BigInt::from_i64(right)),
                ))
            }
        }
        Some((left, right)) => Ok(Value::Float(numeric_as_f64(left) - numeric_as_f64(right))),
        None => {
            if std::env::var_os("PYRS_TRACE_SUB_OP").is_some() {
                eprintln!(
                    "[sub-op] unsupported '-' left_kind={} right_kind={} left_repr={} right_repr={}",
                    debug_value_kind(&left),
                    debug_value_kind(&right),
                    format_repr(&left),
                    format_repr(&right)
                );
            }
            Err(RuntimeError::new("unsupported operand type for -"))
        }
    }
}

pub(super) fn div_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let (left, right) = numeric_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for /"))?;
    let right_value = numeric_as_f64(right);
    if right_value == 0.0 {
        return Err(RuntimeError::zero_division_error("division by zero"));
    }
    Ok(Value::Float(numeric_as_f64(left) / right_value))
}

pub(super) fn floor_div_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let Some((left, right)) = integer_i64_pair(&left, &right) {
        return Ok(Value::Int(python_floor_div(left, right)?));
    }
    if let Some((left, right)) = integer_pair(&left, &right) {
        let (quotient, _) = left
            .div_mod_floor(&right)
            .ok_or_else(|| RuntimeError::zero_division_error("division by zero"))?;
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
                return Err(RuntimeError::zero_division_error("division by zero"));
            }
            Ok(Value::Float((numeric_as_f64(left) / right_value).floor()))
        }
    }
}

pub(super) fn mod_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let Value::Str(format) = left {
        return string_percent_format(&format, right).map(Value::Str);
    }
    if let Value::Bytes(format) = left {
        let format = match &*format.kind() {
            Object::Bytes(values) => values.clone(),
            _ => {
                return Err(RuntimeError::type_error("unsupported operand type for %"));
            }
        };
        return bytes_percent_format(&format, right).map(|value| heap.alloc_bytes(value));
    }
    if let Value::ByteArray(format) = left {
        let format = match &*format.kind() {
            Object::ByteArray(values) => values.clone(),
            _ => {
                return Err(RuntimeError::type_error("unsupported operand type for %"));
            }
        };
        return bytes_percent_format(&format, right).map(|value| heap.alloc_bytes(value));
    }
    if let Some((left, right)) = integer_i64_pair(&left, &right) {
        return Ok(Value::Int(python_mod(left, right)?));
    }
    if let Some((left, right)) = integer_pair(&left, &right) {
        let (_, remainder) = left
            .div_mod_floor(&right)
            .ok_or_else(|| RuntimeError::zero_division_error("modulo by zero"))?;
        return Ok(bigint_to_value(remainder));
    }
    let (left, right) = numeric_pair(&left, &right)
        .ok_or_else(|| RuntimeError::type_error("unsupported operand type for %"))?;
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

fn bytes_percent_format(format: &[u8], right: Value) -> Result<Vec<u8>, RuntimeError> {
    let positional_args = match &right {
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => values.clone(),
            _ => vec![right.clone()],
        },
        _ => vec![right.clone()],
    };
    let mut arg_idx = 0usize;
    let mut idx = 0usize;
    let mut out = Vec::with_capacity(format.len());
    while idx < format.len() {
        if format[idx] != b'%' {
            out.push(format[idx]);
            idx += 1;
            continue;
        }
        idx += 1;
        if idx >= format.len() {
            return Err(RuntimeError::new("incomplete format"));
        }
        if format[idx] == b'%' {
            out.push(b'%');
            idx += 1;
            continue;
        }
        while idx < format.len() && b"#0- +".contains(&format[idx]) {
            idx += 1;
        }
        if idx < format.len() && format[idx] == b'*' {
            if arg_idx >= positional_args.len() {
                return Err(RuntimeError::type_error(
                    "not enough arguments for format string",
                ));
            }
            let _ = value_to_int(positional_args[arg_idx].clone())?;
            arg_idx += 1;
            idx += 1;
        } else {
            while idx < format.len() && format[idx].is_ascii_digit() {
                idx += 1;
            }
        }
        if idx < format.len() && format[idx] == b'.' {
            idx += 1;
            if idx < format.len() && format[idx] == b'*' {
                if arg_idx >= positional_args.len() {
                    return Err(RuntimeError::type_error(
                        "not enough arguments for format string",
                    ));
                }
                let _ = value_to_int(positional_args[arg_idx].clone())?;
                arg_idx += 1;
                idx += 1;
            } else {
                while idx < format.len() && format[idx].is_ascii_digit() {
                    idx += 1;
                }
            }
        }
        if idx >= format.len() {
            return Err(RuntimeError::new("incomplete format"));
        }
        let conv = format[idx];
        idx += 1;
        if arg_idx >= positional_args.len() {
            return Err(RuntimeError::type_error(
                "not enough arguments for format string",
            ));
        }
        let value = positional_args[arg_idx].clone();
        arg_idx += 1;
        match conv {
            b'b' | b's' => out.extend(value_to_bytes_percent_value(value)?),
            _ => return Err(RuntimeError::type_error("unsupported operand type for %")),
        }
    }
    if arg_idx < positional_args.len() {
        return Err(RuntimeError::new(
            "not all arguments converted during bytes formatting",
        ));
    }
    Ok(out)
}

fn value_to_bytes_percent_value(value: Value) -> Result<Vec<u8>, RuntimeError> {
    match value {
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(bytes) => Ok(bytes.clone()),
            _ => Err(RuntimeError::type_error("unsupported operand type for %")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(bytes) => Ok(bytes.clone()),
            _ => Err(RuntimeError::type_error("unsupported operand type for %")),
        },
        _ => Err(RuntimeError::new("%b requires a bytes-like object")),
    }
}

fn string_percent_format(format: &str, right: Value) -> Result<String, RuntimeError> {
    let mut positional_args = match &right {
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => values.clone(),
            _ => vec![right.clone()],
        },
        Value::Instance(_) => {
            namedtuple_instance_percent_args(&right).unwrap_or_else(|| vec![right.clone()])
        }
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

        let mut left_align = false;
        let mut zero_pad = false;
        let mut force_sign = false;
        let mut space_sign = false;
        while idx < chars.len() && "#0- +".contains(chars[idx]) {
            match chars[idx] {
                '-' => left_align = true,
                '0' => zero_pad = true,
                '+' => force_sign = true,
                ' ' => space_sign = true,
                _ => {}
            }
            idx += 1;
        }

        let mut width: Option<usize> = None;
        if idx < chars.len() && chars[idx] == '*' {
            if mapping_key.is_some() {
                return Err(RuntimeError::new("format requires a mapping"));
            }
            if arg_idx >= positional_args.len() {
                return Err(RuntimeError::type_error(
                    "not enough arguments for format string",
                ));
            }
            let width_value = value_to_int(positional_args[arg_idx].clone())?;
            arg_idx += 1;
            idx += 1;
            if width_value < 0 {
                left_align = true;
                width = Some(width_value.unsigned_abs() as usize);
            } else {
                width = Some(width_value as usize);
            }
        } else {
            let width_start = idx;
            while idx < chars.len() && chars[idx].is_ascii_digit() {
                idx += 1;
            }
            if idx > width_start {
                width = Some(
                    chars[width_start..idx]
                        .iter()
                        .collect::<String>()
                        .parse::<usize>()
                        .map_err(|_| RuntimeError::new("invalid width in format string"))?,
                );
            }
        }

        let mut precision: Option<usize> = None;
        if idx < chars.len() && chars[idx] == '.' {
            idx += 1;
            if idx < chars.len() && chars[idx] == '*' {
                if mapping_key.is_some() {
                    return Err(RuntimeError::new("format requires a mapping"));
                }
                if arg_idx >= positional_args.len() {
                    return Err(RuntimeError::type_error(
                        "not enough arguments for format string",
                    ));
                }
                let precision_value = value_to_int(positional_args[arg_idx].clone())?;
                arg_idx += 1;
                idx += 1;
                if precision_value >= 0 {
                    precision = Some(precision_value as usize);
                }
            } else {
                let precision_start = idx;
                while idx < chars.len() && chars[idx].is_ascii_digit() {
                    idx += 1;
                }
                let precision_text = chars[precision_start..idx].iter().collect::<String>();
                precision = if precision_text.is_empty() {
                    Some(0)
                } else {
                    Some(
                        precision_text
                            .parse::<usize>()
                            .map_err(|_| RuntimeError::new("invalid precision in format string"))?,
                    )
                };
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
                return Err(RuntimeError::type_error(
                    "not enough arguments for format string",
                ));
            }
            let value = positional_args[arg_idx].clone();
            arg_idx += 1;
            value
        };

        let formatted = format_percent_value(value, conversion, precision, force_sign, space_sign)?;
        out.push_str(&apply_percent_width(
            formatted,
            width,
            left_align,
            zero_pad && !left_align,
            conversion,
        ));
    }

    if !used_mapping && arg_idx < positional_args.len() {
        return Err(RuntimeError::new(
            "not all arguments converted during string formatting",
        ));
    }
    positional_args.clear();
    Ok(out)
}

fn namedtuple_instance_percent_args(value: &Value) -> Option<Vec<Value>> {
    let Value::Instance(instance) = value else {
        return None;
    };
    let (class, attrs) = match &*instance.kind() {
        Object::Instance(instance_data) => {
            (instance_data.class.clone(), instance_data.attrs.clone())
        }
        _ => return None,
    };
    let field_names = match &*class.kind() {
        Object::Class(class_data) => {
            let fields = class_data.attrs.get("__pyrs_namedtuple_fields__")?;
            match fields {
                Value::Tuple(obj) => match &*obj.kind() {
                    Object::Tuple(values) => values
                        .iter()
                        .map(|value| match value {
                            Value::Str(name) => Some(name.clone()),
                            _ => None,
                        })
                        .collect::<Option<Vec<_>>>()?,
                    _ => return None,
                },
                Value::List(obj) => match &*obj.kind() {
                    Object::List(values) => values
                        .iter()
                        .map(|value| match value {
                            Value::Str(name) => Some(name.clone()),
                            _ => None,
                        })
                        .collect::<Option<Vec<_>>>()?,
                    _ => return None,
                },
                _ => return None,
            }
        }
        _ => return None,
    };

    let mut out = Vec::with_capacity(field_names.len());
    for field in field_names {
        out.push(attrs.get(&field)?.clone());
    }
    Some(out)
}

fn format_percent_value(
    value: Value,
    conversion: char,
    precision: Option<usize>,
    force_sign: bool,
    space_sign: bool,
) -> Result<String, RuntimeError> {
    match conversion {
        's' => {
            let raw = match value {
                Value::Str(text) => text,
                other => format_value(&other),
            };
            Ok(match precision {
                Some(limit) => raw.chars().take(limit).collect(),
                None => raw,
            })
        }
        'r' | 'a' => {
            let raw = format_repr(&value);
            Ok(match precision {
                Some(limit) => raw.chars().take(limit).collect(),
                None => raw,
            })
        }
        'd' | 'i' | 'u' => {
            let integer = int_like_to_bigint(&value)
                .ok_or_else(|| RuntimeError::type_error("expected integer"))?;
            let mut text = integer.to_string();
            if !text.starts_with('-') {
                if force_sign {
                    text.insert(0, '+');
                } else if space_sign {
                    text.insert(0, ' ');
                }
            }
            Ok(text)
        }
        'f' | 'F' => {
            let number = match value {
                Value::Bool(flag) => {
                    if flag {
                        1.0
                    } else {
                        0.0
                    }
                }
                Value::Int(number) => number as f64,
                Value::BigInt(number) => number.to_f64(),
                Value::Float(number) => number,
                _ => return Err(RuntimeError::new("must be real number")),
            };
            let precision = precision.unwrap_or(6);
            let mut text = format!("{number:.precision$}");
            if conversion == 'F' {
                text = text.to_ascii_uppercase();
            }
            if !text.starts_with('-') {
                if force_sign {
                    text.insert(0, '+');
                } else if space_sign {
                    text.insert(0, ' ');
                }
            }
            Ok(text)
        }
        'x' => {
            let integer = int_like_to_bigint(&value)
                .ok_or_else(|| RuntimeError::type_error("expected integer"))?;
            integer
                .to_str_radix(16)
                .ok_or_else(|| RuntimeError::new("unsupported operand type for hex formatting"))
        }
        'X' => {
            let integer = int_like_to_bigint(&value)
                .ok_or_else(|| RuntimeError::type_error("expected integer"))?;
            let value = integer
                .to_str_radix(16)
                .ok_or_else(|| RuntimeError::new("unsupported operand type for hex formatting"))?;
            Ok(value.to_ascii_uppercase())
        }
        'o' => {
            let integer = int_like_to_bigint(&value)
                .ok_or_else(|| RuntimeError::type_error("expected integer"))?;
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

fn apply_percent_width(
    text: String,
    width: Option<usize>,
    left_align: bool,
    zero_pad: bool,
    conversion: char,
) -> String {
    let Some(width) = width else {
        return text;
    };
    let len = text.chars().count();
    if len >= width {
        return text;
    }
    let pad_count = width - len;
    let pad_char = if zero_pad && is_numeric_percent_conversion(conversion) {
        '0'
    } else {
        ' '
    };
    if left_align {
        let mut out = text;
        out.extend(std::iter::repeat_n(' ', pad_count));
        out
    } else if pad_char == '0'
        && (text.starts_with('-') || text.starts_with('+') || text.starts_with(' '))
    {
        let mut chars = text.chars();
        let sign = chars.next().unwrap_or('+');
        let rest = chars.collect::<String>();
        let mut out = String::new();
        out.push(sign);
        out.extend(std::iter::repeat_n('0', pad_count));
        out.push_str(&rest);
        out
    } else {
        let mut out = String::new();
        out.extend(std::iter::repeat_n(pad_char, pad_count));
        out.push_str(&text);
        out
    }
}

fn is_numeric_percent_conversion(conversion: char) -> bool {
    matches!(conversion, 'd' | 'i' | 'u' | 'x' | 'X' | 'o' | 'f' | 'F')
}

pub(super) fn pow_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    if let Some((left_big, right_big)) = integer_pair(&left, &right) {
        if right_big.is_negative() {
            let left = left_big.to_f64();
            let right = right_big.to_f64();
            if left == 0.0 {
                return Err(RuntimeError::zero_division_error("division by zero"));
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
                return Err(RuntimeError::zero_division_error("division by zero"));
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
                .ok_or_else(|| RuntimeError::overflow_error("integer overflow"))?;
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
        Value::BigInt(value) => Ok(bigint_to_value(*value)),
        Value::Bool(value) => Ok(Value::Int(if value { 1 } else { 0 })),
        Value::Float(value) => Ok(Value::Float(value)),
        _ => Err(RuntimeError::type_error("unsupported operand type for +")),
    }
}

pub(super) fn invert_value(value: Value) -> Result<Value, RuntimeError> {
    let value = int_like_to_bigint(&value)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for ~"))?;
    Ok(bigint_to_value(value.bitnot()))
}

pub(super) fn and_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
        return Ok(Value::Bool(*left & *right));
    }
    if let Value::DictKeys(_) = left
        && let (Some(left_values), Some(right_values)) =
            (as_set_values(&left), iterable_values_for_setop(&right))
    {
        let mut intersection = Vec::new();
        for value in left_values {
            if right_values.contains(&value) {
                intersection.push(value);
            }
        }
        return Ok(heap.alloc_set(dedup_hashable_values(intersection)?));
    }
    if let (Some(left_values), Some(right_values)) = (as_set_values(&left), as_set_values(&right)) {
        let mut intersection = Vec::new();
        for value in left_values {
            if right_values.contains(&value) {
                intersection.push(value);
            }
        }
        return set_op_result(left, intersection, heap);
    }
    let (left, right) = integer_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for &"))?;
    Ok(bigint_to_value(left.bitand(&right)))
}

pub(super) fn xor_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
        return Ok(Value::Bool(*left ^ *right));
    }
    if let Value::DictKeys(_) = left
        && let (Some(left_values), Some(right_values)) =
            (as_set_values(&left), iterable_values_for_setop(&right))
    {
        let mut out = Vec::new();
        for value in &left_values {
            if !right_values.iter().any(|candidate| candidate == value) {
                out.push(value.clone());
            }
        }
        for value in &right_values {
            if !left_values.iter().any(|candidate| candidate == value) {
                out.push(value.clone());
            }
        }
        return Ok(heap.alloc_set(dedup_hashable_values(out)?));
    }
    if let (Some(left_values), Some(right_values)) = (as_set_values(&left), as_set_values(&right)) {
        let mut out = Vec::new();
        for value in &left_values {
            if !right_values.iter().any(|candidate| candidate == value) {
                out.push(value.clone());
            }
        }
        for value in &right_values {
            if !left_values.iter().any(|candidate| candidate == value) {
                out.push(value.clone());
            }
        }
        return set_op_result(left, out, heap);
    }
    let (left, right) = integer_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for ^"))?;
    Ok(bigint_to_value(left.bitxor(&right)))
}

pub(super) fn or_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
        return Ok(Value::Bool(*left | *right));
    }
    if let Value::DictKeys(_) = left
        && let (Some(mut merged), Some(right_values)) =
            (as_set_values(&left), iterable_values_for_setop(&right))
    {
        for value in right_values {
            if !merged.contains(&value) {
                merged.push(value);
            }
        }
        return Ok(heap.alloc_set(dedup_hashable_values(merged)?));
    }
    if let (Some(mut merged), Some(right_values)) = (as_set_values(&left), as_set_values(&right)) {
        for value in right_values {
            if !merged.contains(&value) {
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
    if (matches!(left, Value::None)
        && !matches!(right, Value::Int(_) | Value::Bool(_) | Value::BigInt(_)))
        || (matches!(right, Value::None)
            && !matches!(left, Value::Int(_) | Value::Bool(_) | Value::BigInt(_)))
    {
        let mut members = Vec::new();
        append_type_union_members(left, &mut members);
        append_type_union_members(right, &mut members);
        return Ok(heap.alloc_tuple(members));
    }
    let (left, right) = match integer_pair(&left, &right) {
        Some(pair) => pair,
        None => {
            if std::env::var_os("PYRS_TRACE_TYPE_UNION").is_some() {
                eprintln!(
                    "[type-union] unsupported | left={} right={}",
                    format_repr(&left),
                    format_repr(&right),
                );
            }
            return Err(RuntimeError::new("unsupported operand type for |"));
        }
    };
    Ok(bigint_to_value(left.bitor(&right)))
}

fn is_type_union_operand(value: &Value) -> bool {
    match value {
        Value::None | Value::Class(_) | Value::ExceptionType(_) => true,
        Value::Instance(instance) => is_type_union_instance(instance),
        Value::Builtin(
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
            | BuiltinFunction::Range
            | BuiltinFunction::Slice
            | BuiltinFunction::Complex
            | BuiltinFunction::ClassMethod
            | BuiltinFunction::StaticMethod
            | BuiltinFunction::Property,
        ) => true,
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => values.iter().all(is_type_union_operand),
            _ => false,
        },
        _ => false,
    }
}

fn is_type_union_instance(instance: &crate::runtime::ObjRef) -> bool {
    let Some(class_name) = class_name_for_instance(instance) else {
        return false;
    };
    if matches!(
        class_name.as_str(),
        "GenericAlias" | "UnionType" | "TypeVar" | "TypeVarTuple" | "ParamSpec" | "TypeAliasType"
    ) {
        return true;
    }

    let module_name = {
        let instance_kind = instance.kind();
        match &*instance_kind {
            Object::Instance(instance_data) => {
                let class_kind = instance_data.class.kind();
                match &*class_kind {
                    Object::Class(class_data) => match class_data.attrs.get("__module__") {
                        Some(Value::Str(module_name)) => Some(module_name.clone()),
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        }
    };

    if matches!(module_name.as_deref(), Some("typing" | "_typing" | "types")) {
        if class_name.contains("GenericAlias") || class_name.contains("SpecialForm") {
            return true;
        }
        if matches!(
            class_name.as_str(),
            "Union"
                | "_SpecialForm"
                | "_TypedCacheSpecialForm"
                | "_AnyMeta"
                | "_TupleType"
                | "_TypingEllipsis"
        ) {
            return true;
        }
    }

    false
}

fn append_type_union_members(value: Value, members: &mut Vec<Value>) {
    if let Value::Tuple(obj) = &value
        && let Object::Tuple(values) = &*obj.kind()
        && values.iter().all(is_type_union_operand)
    {
        for member in values.iter().cloned() {
            if !members.contains(&member) {
                members.push(member);
            }
        }
        return;
    }
    if !members.contains(&value) {
        members.push(value);
    }
}

pub(super) fn lshift_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    let (left, right) = integer_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for <<"))?;
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
    let (left, right) = integer_pair(&left, &right)
        .ok_or_else(|| RuntimeError::new("unsupported operand type for >>"))?;
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
    if let Some((left, right)) = integer_i64_pair(&left, &right) {
        return Ok(left.cmp(&right));
    }
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
    if let (Some(left_values), Some(right_values)) = (
        namedtuple_instance_percent_args(&left),
        namedtuple_instance_percent_args(&right),
    ) {
        return compare_sequence_order(&left_values, &right_values);
    }

    match (left, right) {
        (Value::Str(a), Value::Str(b)) => Ok(a.cmp(&b)),
        (Value::Tuple(left), Value::Tuple(right)) => match (&*left.kind(), &*right.kind()) {
            (Object::Tuple(left), Object::Tuple(right)) => compare_sequence_order(left, right),
            _ => Err(RuntimeError::type_error(
                "unsupported operand type for comparison",
            )),
        },
        (Value::List(left), Value::List(right)) => match (&*left.kind(), &*right.kind()) {
            (Object::List(left), Object::List(right)) => compare_sequence_order(left, right),
            _ => Err(RuntimeError::type_error(
                "unsupported operand type for comparison",
            )),
        },
        _ => Err(RuntimeError::type_error(
            "unsupported operand type for comparison",
        )),
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
        Value::DictKeys(obj) => match &*obj.kind() {
            Object::DictKeysView(view) => match &*view.dict.kind() {
                Object::Dict(values) => Some(values.iter().map(|(key, _)| key.clone()).collect()),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn iterable_values_for_setop(value: &Value) -> Option<Vec<Value>> {
    match value {
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => Some(values.clone()),
            _ => None,
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => Some(values.clone()),
            _ => None,
        },
        Value::Str(value) => Some(value.chars().map(|ch| Value::Str(ch.to_string())).collect()),
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(values) => Some(values.iter().map(|(key, _)| key.clone()).collect()),
            _ => None,
        },
        Value::Set(_) | Value::FrozenSet(_) | Value::DictKeys(_) => as_set_values(value),
        _ => None,
    }
}

fn set_op_result(left: Value, values: Vec<Value>, heap: &Heap) -> Result<Value, RuntimeError> {
    let values = dedup_hashable_values(values)?;
    match left {
        Value::Set(_) => Ok(heap.alloc_set(values)),
        Value::FrozenSet(_) => Ok(heap.alloc_frozenset(values)),
        _ => unreachable!(),
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
            _ => Err(RuntimeError::type_error("unsupported operand type for in")),
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => Ok(values.iter().any(|value| value == left)),
            _ => Err(RuntimeError::type_error("unsupported operand type for in")),
        },
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(_) => dict_contains_key_checked(obj, left),
            _ => Err(RuntimeError::type_error("unsupported operand type for in")),
        },
        Value::DictKeys(obj) => match &*obj.kind() {
            Object::DictKeysView(view) => match &*view.dict.kind() {
                Object::Dict(_) => dict_contains_key_checked(&view.dict, left),
                _ => Err(RuntimeError::type_error("unsupported operand type for in")),
            },
            _ => Err(RuntimeError::type_error("unsupported operand type for in")),
        },
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => {
                ensure_hashable(left)?;
                Ok(values.contains(left))
            }
            _ => Err(RuntimeError::type_error("unsupported operand type for in")),
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => {
                ensure_hashable(left)?;
                Ok(values.contains(left))
            }
            _ => Err(RuntimeError::type_error("unsupported operand type for in")),
        },
        Value::Str(haystack) => match left {
            Value::Str(needle) => Ok(haystack.contains(needle)),
            _ => Err(RuntimeError::new("in expects string on left")),
        },
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => compare_bytes_like_membership(left, values),
            _ => Err(RuntimeError::type_error("unsupported operand type for in")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => compare_bytes_like_membership(left, values),
            _ => Err(RuntimeError::type_error("unsupported operand type for in")),
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => match &*view.source.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => {
                    compare_bytes_like_membership(left, values)
                }
                _ => Err(RuntimeError::type_error("unsupported operand type for in")),
            },
            _ => Err(RuntimeError::type_error("unsupported operand type for in")),
        },
        Value::Instance(obj) => match &*obj.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get(DICT_BACKING_STORAGE_ATTR) {
                    Some(Value::Dict(storage)) => dict_contains_key_checked(storage, left),
                    _ => Err(RuntimeError::type_error("unsupported operand type for in")),
                }
            }
            _ => Err(RuntimeError::type_error("unsupported operand type for in")),
        },
        _ => Err(RuntimeError::type_error("unsupported operand type for in")),
    }
}

fn compare_bytes_like_membership(left: &Value, haystack: &[u8]) -> Result<bool, RuntimeError> {
    if let Value::Int(_) | Value::Bool(_) | Value::BigInt(_) = left {
        let needle = value_to_int(left.clone())?;
        if !(0..=255).contains(&needle) {
            return Ok(false);
        }
        return Ok(haystack.iter().any(|value| *value as i64 == needle));
    }
    let needle = bytes_like_payload(left)
        .ok_or_else(|| RuntimeError::new("a bytes-like object is required"))?;
    Ok(bytes_contains(haystack, &needle))
}

fn bytes_like_payload(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => Some(values.clone()),
            _ => None,
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => Some(values.clone()),
            _ => None,
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => match &*view.source.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => Some(values.clone()),
                _ => None,
            },
            _ => None,
        },
        Value::Instance(obj) => match &*obj.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get("__pyrs_bytes_storage__") {
                    Some(Value::Bytes(storage)) => match &*storage.kind() {
                        Object::Bytes(values) => Some(values.clone()),
                        _ => None,
                    },
                    Some(Value::ByteArray(storage)) => match &*storage.kind() {
                        Object::ByteArray(values) => Some(values.clone()),
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        },
        Value::Module(obj) => match &*obj.kind() {
            Object::Module(module_data) if module_data.name == "__array__" => {
                let values = module_data.globals.get("values")?;
                let Value::List(list_obj) = values else {
                    return None;
                };
                let Object::List(items) = &*list_obj.kind() else {
                    return None;
                };
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    let value = value_to_int(item.clone()).ok()?;
                    if !(0..=255).contains(&value) {
                        return None;
                    }
                    out.push(value as u8);
                }
                Some(out)
            }
            _ => None,
        },
        _ => None,
    }
}

fn bytes_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

pub(super) fn mul_values(left: Value, right: Value, heap: &Heap) -> Result<Value, RuntimeError> {
    let repeat_count = |value: Value| {
        value_to_int(value).map_err(|_| RuntimeError::type_error("unsupported operand type for *"))
    };
    if let Some((left, right)) = integer_i64_pair(&left, &right) {
        if let Some(product) = left.checked_mul(right) {
            return Ok(Value::Int(product));
        }
        return Ok(bigint_to_value(
            BigInt::from_i64(left).mul(&BigInt::from_i64(right)),
        ));
    }
    if let Some((left, right)) = integer_pair(&left, &right) {
        return Ok(bigint_to_value(left.mul(&right)));
    }
    if let Some((left, right)) = numeric_pair(&left, &right) {
        return match (left, right) {
            (NumericValue::Int(left), NumericValue::Int(right)) => {
                if let Some(product) = left.checked_mul(right) {
                    Ok(Value::Int(product))
                } else {
                    Ok(bigint_to_value(
                        BigInt::from_i64(left).mul(&BigInt::from_i64(right)),
                    ))
                }
            }
            (left, right) => Ok(Value::Float(numeric_as_f64(left) * numeric_as_f64(right))),
        };
    }

    match (left, right) {
        (Value::Str(s), other) | (other, Value::Str(s)) => {
            let count = repeat_count(other)?;
            if count <= 0 {
                return Ok(Value::Str(String::new()));
            }
            Ok(Value::Str(s.repeat(count as usize)))
        }
        (Value::List(obj), other) | (other, Value::List(obj)) => {
            let count = repeat_count(other)?;
            if count <= 0 {
                return Ok(heap.alloc_list(Vec::new()));
            }
            let values = match &*obj.kind() {
                Object::List(values) => values.clone(),
                _ => return Err(RuntimeError::type_error("unsupported operand type for *")),
            };
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(heap.alloc_list(result))
        }
        (Value::Tuple(obj), other) | (other, Value::Tuple(obj)) => {
            let count = repeat_count(other)?;
            if count <= 0 {
                return Ok(heap.alloc_tuple(Vec::new()));
            }
            let values = match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::type_error("unsupported operand type for *")),
            };
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(heap.alloc_tuple(result))
        }
        (Value::Bytes(obj), other) | (other, Value::Bytes(obj)) => {
            let count = repeat_count(other)?;
            if count <= 0 {
                return Ok(heap.alloc_bytes(Vec::new()));
            }
            let values = match &*obj.kind() {
                Object::Bytes(values) => values.clone(),
                _ => return Err(RuntimeError::type_error("unsupported operand type for *")),
            };
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(heap.alloc_bytes(result))
        }
        (Value::ByteArray(obj), other) | (other, Value::ByteArray(obj)) => {
            let count = repeat_count(other)?;
            if count <= 0 {
                return Ok(heap.alloc_bytearray(Vec::new()));
            }
            let values = match &*obj.kind() {
                Object::ByteArray(values) => values.clone(),
                _ => return Err(RuntimeError::type_error("unsupported operand type for *")),
            };
            let mut result = Vec::new();
            for _ in 0..count {
                result.extend(values.clone());
            }
            Ok(heap.alloc_bytearray(result))
        }
        _ => Err(RuntimeError::type_error("unsupported operand type for *")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        and_values, compare_in, compare_order, floor_div_values, lshift_values, mod_values,
        mul_values, or_values, ordering_from_cmp_value, rshift_values, xor_values,
    };
    use crate::runtime::{BigInt, Heap, Object, Value};
    use std::cmp::Ordering;

    #[test]
    fn shifts_reject_negative_shift_count() {
        let lshift_err =
            lshift_values(Value::Int(1), Value::Int(-1)).expect_err("negative shift should fail");
        assert!(lshift_err.message.contains("negative shift count"));

        let rshift_err =
            rshift_values(Value::Int(8), Value::Int(-1)).expect_err("negative shift should fail");
        assert!(rshift_err.message.contains("negative shift count"));
    }

    #[test]
    fn compare_in_rejects_unhashable_dict_key_lookup() {
        let heap = Heap::new();
        let dict = heap.alloc_dict(vec![(Value::Str("k".to_string()), Value::Int(1))]);
        let needle = heap.alloc_list(vec![Value::Int(1)]);

        let err = compare_in(&needle, &dict)
            .expect_err("unhashable key should fail dictionary membership");
        assert!(err.message.contains("unhashable type: 'list'"));
    }

    #[test]
    fn compare_in_for_bytes_like_values_obeys_byte_range() {
        let heap = Heap::new();
        let bytes = heap.alloc_bytes(vec![1, 2, 3]);
        let bytearray = heap.alloc_bytearray(vec![4, 5, 6]);
        let memoryview = match &bytearray {
            Value::ByteArray(obj) => heap.alloc_memoryview(obj.clone()),
            _ => unreachable!(),
        };

        assert!(compare_in(&Value::Int(2), &bytes).expect("valid membership"));
        assert!(!compare_in(&Value::Int(300), &bytes).expect("out-of-range should be false"));
        assert!(compare_in(&Value::Int(5), &bytearray).expect("bytearray membership"));
        assert!(compare_in(&Value::Int(6), &memoryview).expect("memoryview membership"));
    }

    #[test]
    fn string_percent_c_enforces_range_and_type() {
        let heap = Heap::new();
        let range_err = mod_values(Value::Str("%c".to_string()), Value::Int(0x110000), &heap)
            .expect_err("out of range codepoint should fail");
        assert!(range_err.message.contains("%c arg not in range"));

        let type_err = mod_values(
            Value::Str("%c".to_string()),
            heap.alloc_list(vec![Value::Int(1)]),
            &heap,
        )
        .expect_err("non-int/char argument should fail");
        assert!(type_err.message.contains("%c requires int or char"));
    }

    #[test]
    fn mul_values_handles_zero_and_negative_repeat_counts() {
        let heap = Heap::new();
        let repeated = mul_values(Value::Str("ab".to_string()), Value::Int(0), &heap)
            .expect("zero repeat should succeed");
        assert_eq!(repeated, Value::Str(String::new()));

        let list = heap.alloc_list(vec![Value::Int(1), Value::Int(2)]);
        let repeated_list =
            mul_values(list, Value::Int(-3), &heap).expect("negative repeat should succeed");
        match repeated_list {
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => assert!(values.is_empty()),
                other => panic!("expected list object, got {other:?}"),
            },
            other => panic!("expected list value, got {other:?}"),
        }
    }

    #[test]
    fn floor_div_and_mod_follow_python_sign_rules() {
        let heap = Heap::new();
        let quotient = floor_div_values(Value::Int(-7), Value::Int(3)).expect("floor div works");
        let remainder = mod_values(Value::Int(-7), Value::Int(3), &heap).expect("mod works");
        assert_eq!(quotient, Value::Int(-3));
        assert_eq!(remainder, Value::Int(2));

        let big = Value::BigInt(Box::new(
            BigInt::from_str_radix("100000000000000000000", 10).unwrap(),
        ));
        let big_q = floor_div_values(big.clone(), Value::Int(7)).expect("big floor div works");
        let big_r = mod_values(big, Value::Int(7), &heap).expect("big mod works");
        assert_eq!(
            big_q,
            Value::BigInt(Box::new(
                BigInt::from_str_radix("14285714285714285714", 10).unwrap(),
            ))
        );
        assert_eq!(big_r, Value::Int(2));
    }

    #[test]
    fn set_bitwise_ops_return_expected_members() {
        let heap = Heap::new();
        let left = heap.alloc_set(vec![Value::Int(1), Value::Int(2)]);
        let right = heap.alloc_set(vec![Value::Int(2), Value::Int(3)]);

        let intersection = and_values(left.clone(), right.clone(), &heap).expect("set and works");
        let union = or_values(left.clone(), right.clone(), &heap).expect("set or works");
        let sym_diff = xor_values(left, right, &heap).expect("set xor works");

        assert!(matches!(
            intersection,
            Value::Set(obj)
                if matches!(&*obj.kind(), Object::Set(values)
                    if values.contains(&Value::Int(2)) && values.len() == 1)
        ));
        assert!(matches!(
            union,
            Value::Set(obj)
                if matches!(&*obj.kind(), Object::Set(values)
                    if values.contains(&Value::Int(1))
                        && values.contains(&Value::Int(2))
                        && values.contains(&Value::Int(3))
                        && values.len() == 3)
        ));
        assert!(matches!(
            sym_diff,
            Value::Set(obj)
                if matches!(&*obj.kind(), Object::Set(values)
                    if values.contains(&Value::Int(1))
                        && values.contains(&Value::Int(3))
                        && values.len() == 2)
        ));
    }

    #[test]
    fn ordering_from_cmp_value_accepts_int_like_and_rejects_other_types() {
        assert_eq!(
            ordering_from_cmp_value(Value::Int(-1)).expect("int should map to ordering"),
            Ordering::Less
        );
        assert_eq!(
            ordering_from_cmp_value(Value::Bool(false)).expect("bool false should map to equal"),
            Ordering::Equal
        );
        assert_eq!(
            ordering_from_cmp_value(Value::Bool(true)).expect("bool true should map to greater"),
            Ordering::Greater
        );
        let err = ordering_from_cmp_value(Value::Str("nope".to_string()))
            .expect_err("non-int-like cmp value should fail");
        assert!(
            err.message
                .contains("cmp_to_key comparator must return a number")
        );
    }

    #[test]
    fn compare_order_for_sequences_is_lexicographic() {
        let heap = Heap::new();
        let left = heap.alloc_list(vec![Value::Int(1), Value::Int(2)]);
        let right = heap.alloc_list(vec![Value::Int(1), Value::Int(3)]);
        let ordering = compare_order(left, right).expect("list compare should work");
        assert_eq!(ordering, Ordering::Less);
    }
}
