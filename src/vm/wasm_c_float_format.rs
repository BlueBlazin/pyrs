use crate::runtime::RuntimeError;

fn ensure_alternate_decimal(mut text: String) -> String {
    if let Some(exp_pos) = text.find(['e', 'E']) {
        if !text[..exp_pos].contains('.') {
            text.insert(exp_pos, '.');
        }
        return text;
    }
    if !text.contains('.') {
        text.push('.');
    }
    text
}

fn strip_trailing_fraction_zeros(text: &str) -> String {
    let (mantissa, exponent_suffix) = if let Some(pos) = text.find(['e', 'E']) {
        (&text[..pos], &text[pos..])
    } else {
        (text, "")
    };
    if let Some(dot) = mantissa.find('.') {
        let mut head = mantissa[..dot].to_string();
        let mut tail = mantissa[dot + 1..].to_string();
        while tail.ends_with('0') {
            tail.pop();
        }
        if !tail.is_empty() {
            head.push('.');
            head.push_str(&tail);
        }
        head.push_str(exponent_suffix);
        head
    } else {
        text.to_string()
    }
}

fn with_sign_prefix(value: f64, sign_flag: Option<char>, mut body: String) -> String {
    if value.is_sign_negative() {
        let mut out = String::with_capacity(body.len() + 1);
        out.push('-');
        out.push_str(&body);
        return out;
    }
    match sign_flag {
        Some('+') => {
            let mut out = String::with_capacity(body.len() + 1);
            out.push('+');
            out.push_str(&body);
            out
        }
        Some(' ') => {
            let mut out = String::with_capacity(body.len() + 1);
            out.push(' ');
            out.push_str(&body);
            out
        }
        _ => {
            if body.starts_with('+') {
                body.remove(0);
            }
            body
        }
    }
}

fn parse_c_float_pattern(
    pattern: &str,
) -> Result<(Option<char>, bool, Option<usize>, char), RuntimeError> {
    let chars: Vec<char> = pattern.chars().collect();
    if chars.first() != Some(&'%') || chars.len() < 2 {
        return Err(RuntimeError::value_error("invalid format string"));
    }
    let mut idx = 1usize;
    let mut sign_flag = None;
    let mut alternate = false;
    if idx < chars.len() && matches!(chars[idx], '+' | ' ') {
        sign_flag = Some(chars[idx]);
        idx += 1;
    }
    if idx < chars.len() && chars[idx] == '#' {
        alternate = true;
        idx += 1;
    }
    let mut precision = None;
    if idx < chars.len() && chars[idx] == '.' {
        idx += 1;
        let start = idx;
        while idx < chars.len() && chars[idx].is_ascii_digit() {
            idx += 1;
        }
        if start == idx {
            return Err(RuntimeError::value_error("invalid format string"));
        }
        let text: String = chars[start..idx].iter().collect();
        precision = Some(
            text.parse::<usize>()
                .map_err(|_| RuntimeError::value_error("invalid format string"))?,
        );
    }
    if idx + 1 != chars.len() {
        return Err(RuntimeError::value_error("invalid format string"));
    }
    let conversion = chars[idx];
    if !matches!(conversion, 'e' | 'E' | 'f' | 'F' | 'g' | 'G') {
        return Err(RuntimeError::value_error("invalid format string"));
    }
    Ok((sign_flag, alternate, precision, conversion))
}

fn format_nonfinite(value: f64, sign_flag: Option<char>, uppercase: bool) -> String {
    let token = if value.is_nan() {
        if uppercase { "NAN" } else { "nan" }
    } else if uppercase {
        "INF"
    } else {
        "inf"
    };
    with_sign_prefix(value, sign_flag, token.to_string())
}

fn format_with_exponent(value: f64, precision: usize, alternate: bool, uppercase: bool) -> String {
    let mut text = format!("{:.*e}", precision, value.abs());
    if uppercase {
        text = text.replace('e', "E");
    }
    if !alternate {
        text = strip_trailing_fraction_zeros(&text);
    } else {
        text = ensure_alternate_decimal(text);
    }
    text
}

fn format_with_fixed(value: f64, precision: usize, alternate: bool) -> String {
    let mut text = format!("{:.*}", precision, value.abs());
    if !alternate {
        text = strip_trailing_fraction_zeros(&text);
    } else {
        text = ensure_alternate_decimal(text);
    }
    text
}

pub(super) fn format_float_with_c_pattern(
    pattern: &str,
    value: f64,
) -> Result<String, RuntimeError> {
    let (sign_flag, alternate, precision, conversion) = parse_c_float_pattern(pattern)?;
    if !value.is_finite() {
        return Ok(format_nonfinite(
            value,
            sign_flag,
            conversion.is_ascii_uppercase(),
        ));
    }

    let mut body = match conversion {
        'f' | 'F' => {
            let p = precision.unwrap_or(6);
            format_with_fixed(value, p, alternate)
        }
        'e' | 'E' => {
            let p = precision.unwrap_or(6);
            format_with_exponent(value, p, alternate, conversion == 'E')
        }
        'g' | 'G' => {
            let p = precision.unwrap_or(6).max(1);
            let exponent = if value == 0.0 {
                0
            } else {
                value.abs().log10().floor() as i32
            };
            let use_exp = exponent < -4 || exponent >= p as i32;
            if use_exp {
                format_with_exponent(value, p - 1, alternate, conversion == 'G')
            } else {
                let fractional_digits = if exponent >= 0 {
                    p.saturating_sub(exponent as usize + 1)
                } else {
                    p + (-exponent as usize) - 1
                };
                format_with_fixed(value, fractional_digits, alternate)
            }
        }
        _ => return Err(RuntimeError::value_error("invalid format string")),
    };

    if conversion.is_ascii_uppercase() {
        body = body.to_uppercase();
    }
    Ok(with_sign_prefix(value, sign_flag, body))
}
