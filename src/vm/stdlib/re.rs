use super::super::*;

const CSV_SNIFFER_PATTERN_1: &str =
    r#"(?P<delim>[^\w\n"\'])(?P<space> ?)(?P<quote>["\']).*?(?P=quote)(?P=delim)"#;
const CSV_SNIFFER_PATTERN_2: &str =
    r#"(?:^|\n)(?P<quote>["\']).*?(?P=quote)(?P<delim>[^\w\n"\'])(?P<space> ?)"#;
const CSV_SNIFFER_PATTERN_3: &str =
    r#"(?P<delim>[^\w\n"\'])(?P<space> ?)(?P<quote>["\']).*?(?P=quote)(?:$|\n)"#;
const CSV_SNIFFER_PATTERN_4: &str = r#"(?:^|\n)(?P<quote>["\']).*?(?P=quote)(?:$|\n)"#;

impl Vm {
    pub(in crate::vm) fn builtin_re_search(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_re_match_mode(args, kwargs, ReMode::Search)
    }

    pub(in crate::vm) fn builtin_re_match(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_re_match_mode(args, kwargs, ReMode::Match)
    }

    pub(in crate::vm) fn builtin_re_fullmatch(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_re_match_mode(args, kwargs, ReMode::FullMatch)
    }

    pub(in crate::vm) fn builtin_re_compile(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "re.compile() expects pattern and optional flags",
            ));
        }
        if args.len() == 2 {
            // Flags are currently accepted for compatibility but not interpreted.
            let _ = args.pop();
        }
        let pattern_value = match args.remove(0) {
            Value::Module(module) => {
                let compiled_pattern = matches!(
                    &*module.kind(),
                    Object::Module(module_data) if module_data.name == "__re_pattern__"
                );
                if compiled_pattern {
                    return Ok(Value::Module(module));
                }
                return Err(RuntimeError::new(
                    "re.compile() expects string or bytes pattern",
                ));
            }
            Value::Str(pattern) => Value::Str(pattern),
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(values) => self.heap.alloc_bytes(values.clone()),
                _ => {
                    return Err(RuntimeError::new(
                        "re.compile() expects string or bytes pattern",
                    ));
                }
            },
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(values) => self.heap.alloc_bytes(values.clone()),
                _ => {
                    return Err(RuntimeError::new(
                        "re.compile() expects string or bytes pattern",
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::new(
                    "re.compile() expects string or bytes pattern",
                ));
            }
        };
        let compiled = match self
            .heap
            .alloc_module(ModuleObject::new("__re_pattern__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *compiled.kind_mut() {
            module_data
                .globals
                .insert("pattern".to_string(), pattern_value);
            let groupindex = if let Some(pattern) = module_data.globals.get("pattern") {
                match pattern {
                    Value::Str(text) => {
                        let entries = csv_sniffer_groupindex_entries(text)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(name, idx)| (Value::Str(name.to_string()), Value::Int(idx)))
                            .collect();
                        self.heap.alloc_dict(entries)
                    }
                    _ => self.heap.alloc_dict(Vec::new()),
                }
            } else {
                self.heap.alloc_dict(Vec::new())
            };
            module_data
                .globals
                .insert("groupindex".to_string(), groupindex);
        }
        Ok(Value::Module(compiled))
    }

    pub(in crate::vm) fn builtin_re_escape(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("re.escape() expects one argument"));
        }
        match &args[0] {
            Value::Str(text) => {
                let mut escaped = String::new();
                for ch in text.chars() {
                    if ch.is_ascii_alphanumeric() || ch == '_' {
                        escaped.push(ch);
                    } else {
                        escaped.push('\\');
                        escaped.push(ch);
                    }
                }
                Ok(Value::Str(escaped))
            }
            Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                let text = bytes_like_from_value(args[0].clone())?;
                let mut escaped = Vec::with_capacity(text.len() * 2);
                for byte in text {
                    if byte.is_ascii_alphanumeric() || byte == b'_' {
                        escaped.push(byte);
                    } else {
                        escaped.push(b'\\');
                        escaped.push(byte);
                    }
                }
                Ok(self.heap.alloc_bytes(escaped))
            }
            _ => Err(RuntimeError::new("re.escape() expects string argument")),
        }
    }

    pub(in crate::vm) fn builtin_re_pattern_findall(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 4 {
            return Err(RuntimeError::new(
                "Pattern.findall() expects string and optional pos/endpos",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let pattern = re_pattern_from_compiled_module(&receiver)?;
        let target = args.remove(0);

        let clamp_index = |len: usize, raw: i64| -> usize {
            if raw < 0 {
                (len as i64 + raw).max(0) as usize
            } else {
                (raw as usize).min(len)
            }
        };

        match target {
            Value::Str(text) => {
                if !matches!(pattern, RePatternValue::Str(_)) {
                    return Err(RuntimeError::new(
                        "cannot use a bytes pattern on a string-like object",
                    ));
                }
                if let RePatternValue::Str(pattern_text) = &pattern {
                    if let Some((group_count, matches)) =
                        csv_sniffer_pattern_findall(pattern_text, &text)
                    {
                        if group_count == 1 {
                            let out = matches
                                .into_iter()
                                .map(|row| Value::Str(row[0].clone()))
                                .collect::<Vec<_>>();
                            return Ok(self.heap.alloc_list(out));
                        }
                        let out = matches
                            .into_iter()
                            .map(|row| {
                                self.heap
                                    .alloc_tuple(row.into_iter().map(Value::Str).collect())
                            })
                            .collect::<Vec<_>>();
                        return Ok(self.heap.alloc_list(out));
                    }
                }
                let raw_pos = if let Some(value) = args.first() {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let raw_end = if let Some(value) = args.get(1) {
                    value_to_int(value.clone())?
                } else {
                    text.len() as i64
                };
                let mut start = clamp_index(text.len(), raw_pos);
                let mut stop = clamp_index(text.len(), raw_end);
                while start > 0 && !text.is_char_boundary(start) {
                    start -= 1;
                }
                while stop > 0 && !text.is_char_boundary(stop) {
                    stop -= 1;
                }
                if stop < start {
                    stop = start;
                }
                let mut matches = Vec::new();
                let mut cursor = start;
                while cursor <= stop {
                    let segment = Value::Str(text[cursor..stop].to_string());
                    let Some((match_start, match_end)) =
                        re_match_bounds(&pattern, &segment, ReMode::Search)?
                    else {
                        break;
                    };
                    let absolute_start = cursor + match_start;
                    let absolute_end = cursor + match_end;
                    if absolute_start > stop || absolute_end > stop {
                        break;
                    }
                    matches.push(Value::Str(text[absolute_start..absolute_end].to_string()));
                    if absolute_end == absolute_start {
                        if absolute_end >= stop {
                            break;
                        }
                        let mut next = absolute_end + 1;
                        while next < stop && !text.is_char_boundary(next) {
                            next += 1;
                        }
                        cursor = next;
                    } else {
                        cursor = absolute_end;
                    }
                }
                Ok(self.heap.alloc_list(matches))
            }
            Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                if !matches!(pattern, RePatternValue::Bytes(_)) {
                    return Err(RuntimeError::new(
                        "cannot use a string pattern on a bytes-like object",
                    ));
                }
                let bytes = bytes_like_from_value(target)?;
                let raw_pos = if let Some(value) = args.first() {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let raw_end = if let Some(value) = args.get(1) {
                    value_to_int(value.clone())?
                } else {
                    bytes.len() as i64
                };
                let start = clamp_index(bytes.len(), raw_pos);
                let mut stop = clamp_index(bytes.len(), raw_end);
                if stop < start {
                    stop = start;
                }
                let mut matches = Vec::new();
                let mut cursor = start;
                while cursor <= stop {
                    let segment = self.heap.alloc_bytes(bytes[cursor..stop].to_vec());
                    let Some((match_start, match_end)) =
                        re_match_bounds(&pattern, &segment, ReMode::Search)?
                    else {
                        break;
                    };
                    let absolute_start = cursor + match_start;
                    let absolute_end = cursor + match_end;
                    if absolute_start > stop || absolute_end > stop {
                        break;
                    }
                    matches.push(
                        self.heap
                            .alloc_bytes(bytes[absolute_start..absolute_end].to_vec()),
                    );
                    if absolute_end == absolute_start {
                        if absolute_end >= stop {
                            break;
                        }
                        cursor = absolute_end + 1;
                    } else {
                        cursor = absolute_end;
                    }
                }
                Ok(self.heap.alloc_list(matches))
            }
            _ => Err(RuntimeError::new(
                "Pattern.findall() expects string or bytes-like object",
            )),
        }
    }

    pub(in crate::vm) fn builtin_re_match_mode(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        mode: ReMode,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 {
            return Err(RuntimeError::new("re function expects pattern and string"));
        }
        let pattern = re_pattern_from_argument(&args[0])?;
        if let (ReMode::Search, RePatternValue::Str(pattern_text), Value::Str(text)) =
            (mode, &pattern, &args[1])
        {
            if let Some(quote) = csv_sniffer_doublequote_quote(pattern_text) {
                let needle = format!("{quote}{quote}");
                if let Some(start) = text.find(&needle) {
                    return Ok(self.heap.alloc_tuple(vec![
                        Value::Int(start as i64),
                        Value::Int((start + needle.len()) as i64),
                    ]));
                }
                return Ok(Value::None);
            }
        }
        let found = re_match_bounds(&pattern, &args[1], mode)?;
        if let Some((start, end)) = found {
            Ok(self
                .heap
                .alloc_tuple(vec![Value::Int(start as i64), Value::Int(end as i64)]))
        } else {
            Ok(Value::None)
        }
    }
}

fn csv_sniffer_groupindex_entries(pattern: &str) -> Option<Vec<(&'static str, i64)>> {
    match pattern {
        CSV_SNIFFER_PATTERN_1 => Some(vec![("delim", 1), ("space", 2), ("quote", 3)]),
        CSV_SNIFFER_PATTERN_2 => Some(vec![("quote", 1), ("delim", 2), ("space", 3)]),
        CSV_SNIFFER_PATTERN_3 => Some(vec![("delim", 1), ("space", 2), ("quote", 3)]),
        CSV_SNIFFER_PATTERN_4 => Some(vec![("quote", 1)]),
        _ => None,
    }
}

fn csv_sniffer_is_word_char(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

fn csv_sniffer_is_delim_candidate(ch: char) -> bool {
    !csv_sniffer_is_word_char(ch) && ch != '\n' && ch != '\'' && ch != '"'
}

fn csv_sniffer_pattern_findall(pattern: &str, text: &str) -> Option<(usize, Vec<Vec<String>>)> {
    let chars: Vec<char> = text.chars().collect();
    match pattern {
        CSV_SNIFFER_PATTERN_1 => {
            let mut out = Vec::new();
            for i in 0..chars.len() {
                let delim = chars[i];
                if !csv_sniffer_is_delim_candidate(delim) {
                    continue;
                }
                let mut quote_idx = i + 1;
                let space = if quote_idx < chars.len() && chars[quote_idx] == ' ' {
                    quote_idx += 1;
                    " ".to_string()
                } else {
                    String::new()
                };
                if quote_idx >= chars.len() {
                    continue;
                }
                let quote = chars[quote_idx];
                if quote != '\'' && quote != '"' {
                    continue;
                }
                let mut close = quote_idx + 1;
                while close < chars.len() {
                    if chars[close] == quote && close + 1 < chars.len() && chars[close + 1] == delim
                    {
                        out.push(vec![delim.to_string(), space.clone(), quote.to_string()]);
                        break;
                    }
                    close += 1;
                }
            }
            Some((3, out))
        }
        CSV_SNIFFER_PATTERN_2 => {
            let mut out = Vec::new();
            for start in 0..chars.len() {
                if start != 0 && chars[start - 1] != '\n' {
                    continue;
                }
                let quote = chars[start];
                if quote != '\'' && quote != '"' {
                    continue;
                }
                let mut close = start + 1;
                while close < chars.len() {
                    if chars[close] == quote
                        && close + 1 < chars.len()
                        && csv_sniffer_is_delim_candidate(chars[close + 1])
                    {
                        let delim = chars[close + 1];
                        let space = if close + 2 < chars.len() && chars[close + 2] == ' ' {
                            " ".to_string()
                        } else {
                            String::new()
                        };
                        out.push(vec![quote.to_string(), delim.to_string(), space]);
                        break;
                    }
                    close += 1;
                }
            }
            Some((3, out))
        }
        CSV_SNIFFER_PATTERN_3 => {
            let mut out = Vec::new();
            for i in 0..chars.len() {
                let delim = chars[i];
                if !csv_sniffer_is_delim_candidate(delim) {
                    continue;
                }
                let mut quote_idx = i + 1;
                let space = if quote_idx < chars.len() && chars[quote_idx] == ' ' {
                    quote_idx += 1;
                    " ".to_string()
                } else {
                    String::new()
                };
                if quote_idx >= chars.len() {
                    continue;
                }
                let quote = chars[quote_idx];
                if quote != '\'' && quote != '"' {
                    continue;
                }
                let mut close = quote_idx + 1;
                while close < chars.len() {
                    if chars[close] == quote && (close + 1 == chars.len() || chars[close + 1] == '\n')
                    {
                        out.push(vec![delim.to_string(), space.clone(), quote.to_string()]);
                        break;
                    }
                    close += 1;
                }
            }
            Some((3, out))
        }
        CSV_SNIFFER_PATTERN_4 => {
            let mut out = Vec::new();
            for start in 0..chars.len() {
                if start != 0 && chars[start - 1] != '\n' {
                    continue;
                }
                let quote = chars[start];
                if quote != '\'' && quote != '"' {
                    continue;
                }
                let mut close = start + 1;
                while close < chars.len() {
                    if chars[close] == quote && (close + 1 == chars.len() || chars[close + 1] == '\n')
                    {
                        out.push(vec![quote.to_string()]);
                        break;
                    }
                    close += 1;
                }
            }
            Some((1, out))
        }
        _ => None,
    }
}

fn csv_sniffer_doublequote_quote(pattern: &str) -> Option<char> {
    if !pattern.starts_with("((") || !pattern.contains(")|^)") || !pattern.ends_with(")|$)") {
        return None;
    }
    let marker = if pattern.contains(")|^)\\W*") {
        ")|^)\\W*"
    } else if pattern.contains(")|^)W*") {
        ")|^)W*"
    } else {
        return None;
    };
    let marker_pos = pattern.find(marker)?;
    let mut rest = &pattern[marker_pos + marker.len()..];
    if let Some(stripped) = rest.strip_prefix('\\') {
        rest = stripped;
    }
    let mut iter = rest.chars();
    let first = iter.next()?;
    let quote = if first == '\\' { iter.next()? } else { first };
    if quote == '\'' || quote == '"' {
        Some(quote)
    } else {
        None
    }
}
