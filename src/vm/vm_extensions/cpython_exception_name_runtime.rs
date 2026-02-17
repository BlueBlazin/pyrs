pub(in crate::vm::vm_extensions) fn cpython_exception_name_from_runtime_message(
    message: &str,
) -> Option<String> {
    let candidate = if message.starts_with("Traceback (most recent call last):") {
        message
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
            .unwrap_or(message.trim())
    } else {
        message.trim()
    };
    if candidate.is_empty() {
        return None;
    }
    let prefix = candidate
        .split_once(':')
        .map(|(name, _)| name)
        .unwrap_or(candidate);
    if prefix
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && (prefix.ends_with("Error")
            || prefix.ends_with("Warning")
            || prefix == "Exception"
            || prefix == "BaseException")
    {
        Some(prefix.to_string())
    } else {
        None
    }
}

pub(in crate::vm::vm_extensions) fn cpython_exception_name_parts(
    name: &str,
) -> Option<(&str, &str)> {
    let dot = name.rfind('.')?;
    if dot == 0 || dot + 1 >= name.len() {
        return None;
    }
    Some((&name[..dot], &name[dot + 1..]))
}
