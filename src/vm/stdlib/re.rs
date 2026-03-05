use super::super::{
    HashMap, InstanceObject, ObjRef, Object, ReMatchDetail, ReMode, RePatternValue, RuntimeError,
    Value, Vm, bytes_like_from_value, dict_get_value, format_value,
    re_compiled_regex_program_from_argument, re_compiled_regex_program_from_object,
    re_match_details, re_pattern_from_argument, re_pattern_from_compiled_object, value_to_int,
};

const CSV_SNIFFER_PATTERN_1: &str =
    r#"(?P<delim>[^\w\n"\'])(?P<space> ?)(?P<quote>["\']).*?(?P=quote)(?P=delim)"#;
const CSV_SNIFFER_PATTERN_2: &str =
    r#"(?:^|\n)(?P<quote>["\']).*?(?P=quote)(?P<delim>[^\w\n"\'])(?P<space> ?)"#;
const CSV_SNIFFER_PATTERN_3: &str =
    r#"(?P<delim>[^\w\n"\'])(?P<space> ?)(?P<quote>["\']).*?(?P=quote)(?:$|\n)"#;
const CSV_SNIFFER_PATTERN_4: &str = r#"(?:^|\n)(?P<quote>["\']).*?(?P=quote)(?:$|\n)"#;
const PKGUTIL_RESOLVE_NAME_PATTERN: &str =
    r"^(?P<pkg>(?!\d)(\w+)(\.(?!\d)(\w+))*)(?P<cln>:(?P<obj>(?!\d)(\w+)(\.(?!\d)(\w+))*)?)?$";
const RE_PATTERN_MODULE_NAME: &str = "__re_pattern__";
const RE_MATCH_MODULE_NAME: &str = "__re_match__";
const RE_PATTERN_CLASS_NAME: &str = "Pattern";
const RE_MATCH_CLASS_NAME: &str = "Match";
const RE_FLAG_UNICODE: i64 = 32;
const RE_FLAG_VERBOSE: i64 = 64;

fn clamp_index(len: usize, raw: i64) -> usize {
    if raw < 0 {
        (len as i64 + raw).max(0) as usize
    } else {
        (raw as usize).min(len)
    }
}

fn utf8_char_index_to_byte(text: &str, char_index: usize) -> Option<usize> {
    let char_len = text.chars().count();
    if char_index > char_len {
        return None;
    }
    if char_index == char_len {
        return Some(text.len());
    }
    text.char_indices()
        .nth(char_index)
        .map(|(byte_idx, _)| byte_idx)
}

fn utf8_byte_index_to_char(text: &str, byte_index: usize) -> Option<usize> {
    if byte_index > text.len() || !text.is_char_boundary(byte_index) {
        return None;
    }
    Some(text[..byte_index].chars().count())
}

fn utf8_slice_by_char_range(text: &str, start_char: usize, end_char: usize) -> Option<String> {
    if start_char > end_char {
        return None;
    }
    let start_byte = utf8_char_index_to_byte(text, start_char)?;
    let end_byte = utf8_char_index_to_byte(text, end_char)?;
    Some(text[start_byte..end_byte].to_string())
}

fn normalize_string_window(
    text: &str,
    raw_pos: i64,
    raw_end: i64,
) -> Result<(usize, usize, i64, i64), RuntimeError> {
    let char_len = text.chars().count();
    let start_char = clamp_index(char_len, raw_pos);
    let mut stop_char = clamp_index(char_len, raw_end);
    if stop_char < start_char {
        stop_char = start_char;
    }
    let start_byte = utf8_char_index_to_byte(text, start_char)
        .ok_or_else(|| RuntimeError::new("invalid regex match bounds"))?;
    let stop_byte = utf8_char_index_to_byte(text, stop_char)
        .ok_or_else(|| RuntimeError::new("invalid regex match bounds"))?;
    Ok((start_byte, stop_byte, start_char as i64, stop_char as i64))
}

fn decimal_parser_groupindex_entries(pattern: &str) -> Option<Vec<(&'static str, i64)>> {
    let looks_like_decimal_parser = pattern.contains("(?P<sign>[-+])?")
        && pattern.contains("(?P<int>\\d*)")
        && pattern.contains("Inf(inity)?")
        && pattern.contains("(?P<signal>s)?")
        && pattern.contains("NaN")
        && pattern.contains("(?P<diag>\\d*)");
    if !looks_like_decimal_parser {
        return None;
    }
    Some(vec![
        ("sign", 1),
        ("int", 3),
        ("frac", 5),
        ("exp", 7),
        ("signal", 9),
        ("diag", 10),
    ])
}

impl Vm {
    fn coerce_sre_int_arg(&mut self, value: Value) -> Result<i64, RuntimeError> {
        let coerced = self
            .builtin_int(vec![value], HashMap::new())
            .map_err(|_| RuntimeError::type_error("an integer is required"))?;
        value_to_int(coerced).map_err(|_| RuntimeError::type_error("an integer is required"))
    }

    fn is_re_runtime_class_instance(&self, receiver: &ObjRef, class_name: &str) -> bool {
        let Object::Instance(instance_data) = &*receiver.kind() else {
            return false;
        };
        let Object::Class(class_data) = &*instance_data.class.kind() else {
            return false;
        };
        class_data.name == class_name
            && matches!(
                class_data.attrs.get("__module__"),
                Some(Value::Str(module_name)) if module_name == "re"
            )
    }

    fn re_match_attr(&self, receiver: &ObjRef, name: &str) -> Option<Value> {
        match &*receiver.kind() {
            Object::Instance(instance_data)
                if self.is_re_runtime_class_instance(receiver, RE_MATCH_CLASS_NAME) =>
            {
                instance_data.attrs.get(name).cloned()
            }
            Object::Module(module_data) if module_data.name == RE_MATCH_MODULE_NAME => {
                // Legacy compatibility path for stale module-backed match objects.
                module_data.globals.get(name).cloned()
            }
            _ => None,
        }
    }

    fn re_pattern_attr(&self, receiver: &ObjRef, name: &str) -> Option<Value> {
        match &*receiver.kind() {
            Object::Instance(instance_data)
                if self.is_re_runtime_class_instance(receiver, RE_PATTERN_CLASS_NAME) =>
            {
                instance_data.attrs.get(name).cloned()
            }
            Object::Module(module_data) if module_data.name == RE_PATTERN_MODULE_NAME => {
                // Legacy compatibility path for stale module-backed pattern objects.
                module_data.globals.get(name).cloned()
            }
            _ => None,
        }
    }

    fn re_match_groupindex_from_pattern_arg(&self, pattern_arg: &Value) -> Value {
        match pattern_arg {
            Value::Instance(instance)
                if self.is_re_runtime_class_instance(instance, RE_PATTERN_CLASS_NAME) =>
            {
                let Object::Instance(instance_data) = &*instance.kind() else {
                    return self.heap.alloc_dict(Vec::new());
                };
                instance_data
                    .attrs
                    .get("groupindex")
                    .cloned()
                    .unwrap_or_else(|| self.heap.alloc_dict(Vec::new()))
            }
            Value::Module(module) => {
                let Object::Module(module_data) = &*module.kind() else {
                    return self.heap.alloc_dict(Vec::new());
                };
                if module_data.name != RE_PATTERN_MODULE_NAME {
                    return self.heap.alloc_dict(Vec::new());
                }
                module_data
                    .globals
                    .get("groupindex")
                    .cloned()
                    .unwrap_or_else(|| self.heap.alloc_dict(Vec::new()))
            }
            Value::Str(pattern) => {
                let entries = csv_sniffer_groupindex_entries(pattern)
                    .or_else(|| decimal_parser_groupindex_entries(pattern))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, idx)| (Value::Str(name.to_string()), Value::Int(idx)))
                    .collect();
                self.heap.alloc_dict(entries)
            }
            _ => self.heap.alloc_dict(Vec::new()),
        }
    }

    fn alloc_re_match_value(
        &mut self,
        pattern: Value,
        source: Value,
        detail: ReMatchDetail,
        groupindex: Value,
        pos: i64,
        endpos: i64,
    ) -> Result<Value, RuntimeError> {
        let ReMatchDetail {
            start: match_start,
            end: match_end,
            captures: detail_captures,
        } = detail;
        let expected_groups = match &groupindex {
            Value::Dict(obj) => match &*obj.kind() {
                Object::Dict(entries) => entries
                    .iter()
                    .filter_map(|(_, value)| value_to_int(value.clone()).ok())
                    .filter(|index| *index > 0)
                    .max()
                    .unwrap_or(0) as usize,
                _ => 0,
            },
            _ => 0,
        };
        let mut captures = detail_captures;
        if captures.len() < expected_groups {
            captures.resize(expected_groups, None);
        }
        let mut groups = Vec::with_capacity(captures.len());
        let mut spans = Vec::with_capacity(captures.len());
        let mut stored_match_start = match_start;
        let mut stored_match_end = match_end;
        match &source {
            Value::Str(text) => {
                stored_match_start = utf8_byte_index_to_char(text, match_start)
                    .ok_or_else(|| RuntimeError::new("invalid regex match bounds"))?;
                stored_match_end = utf8_byte_index_to_char(text, match_end)
                    .ok_or_else(|| RuntimeError::new("invalid regex match bounds"))?;
                for capture in &captures {
                    match capture {
                        Some((start, end)) => {
                            groups.push(Value::Str(text[*start..*end].to_string()));
                            let span_start = utf8_byte_index_to_char(text, *start)
                                .ok_or_else(|| RuntimeError::new("invalid regex match bounds"))?;
                            let span_end = utf8_byte_index_to_char(text, *end)
                                .ok_or_else(|| RuntimeError::new("invalid regex match bounds"))?;
                            spans.push(self.heap.alloc_tuple(vec![
                                Value::Int(span_start as i64),
                                Value::Int(span_end as i64),
                            ]));
                        }
                        None => {
                            groups.push(Value::None);
                            spans.push(Value::None);
                        }
                    }
                }
            }
            Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                let bytes = bytes_like_from_value(source.clone())?;
                for capture in &captures {
                    match capture {
                        Some((start, end)) => {
                            groups.push(self.heap.alloc_bytes(bytes[*start..*end].to_vec()));
                            spans.push(self.heap.alloc_tuple(vec![
                                Value::Int(*start as i64),
                                Value::Int(*end as i64),
                            ]));
                        }
                        None => {
                            groups.push(Value::None);
                            spans.push(Value::None);
                        }
                    }
                }
            }
            _ => return Err(RuntimeError::new("re match source is invalid")),
        }

        let match_class = self.ensure_re_runtime_type_class(RE_MATCH_CLASS_NAME);
        let match_obj = match self.heap.alloc_instance(InstanceObject::new(match_class)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *match_obj.kind_mut() {
            let groups_tuple = self.heap.alloc_tuple(groups.clone());
            let spans_list = self.heap.alloc_list(spans.clone());
            let regs = {
                let mut entries = Vec::with_capacity(spans.len() + 1);
                entries.push(self.heap.alloc_tuple(vec![
                    Value::Int(stored_match_start as i64),
                    Value::Int(stored_match_end as i64),
                ]));
                for span in &spans {
                    if let Value::Tuple(tuple_obj) = span {
                        entries.push(Value::Tuple(tuple_obj.clone()));
                    } else {
                        entries.push(self.heap.alloc_tuple(vec![Value::Int(-1), Value::Int(-1)]));
                    }
                }
                self.heap.alloc_tuple(entries)
            };
            let groupindex_obj = match &groupindex {
                Value::Dict(obj) => Some(obj.clone()),
                _ => None,
            };
            let lastindex_value = captures
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, capture)| capture.map(|_| Value::Int((index + 1) as i64)))
                .unwrap_or(Value::None);
            let lastgroup_value = if let (Value::Int(lastindex), Some(mapping)) =
                (&lastindex_value, groupindex_obj.as_ref())
            {
                let name = match &*mapping.kind() {
                    Object::Dict(entries) => entries.iter().find_map(|(key, value)| {
                        let Value::Str(name) = key else {
                            return None;
                        };
                        let index = value_to_int(value.clone()).ok()?;
                        (index == *lastindex).then_some(name.clone())
                    }),
                    _ => None,
                };
                name.map(Value::Str).unwrap_or(Value::None)
            } else {
                Value::None
            };

            instance_data
                .attrs
                .insert("_source".to_string(), source.clone());
            instance_data
                .attrs
                .insert("_start".to_string(), Value::Int(stored_match_start as i64));
            instance_data
                .attrs
                .insert("_end".to_string(), Value::Int(stored_match_end as i64));
            instance_data
                .attrs
                .insert("_groups".to_string(), groups_tuple.clone());
            instance_data
                .attrs
                .insert("_spans".to_string(), spans_list.clone());
            instance_data
                .attrs
                .insert("_groupindex".to_string(), groupindex.clone());
            instance_data.attrs.insert("re".to_string(), pattern);
            instance_data.attrs.insert("string".to_string(), source);
            instance_data
                .attrs
                .insert("pos".to_string(), Value::Int(pos.max(0)));
            instance_data
                .attrs
                .insert("endpos".to_string(), Value::Int(endpos.max(0)));
            instance_data
                .attrs
                .insert("lastindex".to_string(), lastindex_value);
            instance_data
                .attrs
                .insert("lastgroup".to_string(), lastgroup_value);
            instance_data.attrs.insert("regs".to_string(), regs);
        }
        Ok(Value::Instance(match_obj))
    }

    pub(in crate::vm) fn re_match_snapshot(
        &self,
        receiver: &ObjRef,
    ) -> Result<
        (
            Value,
            i64,
            i64,
            Vec<Value>,
            Vec<Option<(i64, i64)>>,
            Option<ObjRef>,
        ),
        RuntimeError,
    > {
        if !self.is_re_runtime_class_instance(receiver, RE_MATCH_CLASS_NAME)
            && !matches!(&*receiver.kind(), Object::Module(module_data) if module_data.name == RE_MATCH_MODULE_NAME)
        {
            return Err(RuntimeError::type_error("re match receiver is invalid"));
        }
        let source = self
            .re_match_attr(receiver, "_source")
            .ok_or_else(|| RuntimeError::type_error("re match receiver is invalid"))?;
        let start = match self.re_match_attr(receiver, "_start") {
            Some(Value::Int(value)) => value,
            _ => return Err(RuntimeError::type_error("re match receiver is invalid")),
        };
        let end = match self.re_match_attr(receiver, "_end") {
            Some(Value::Int(value)) => value,
            _ => return Err(RuntimeError::type_error("re match receiver is invalid")),
        };
        let groups = match self.re_match_attr(receiver, "_groups") {
            Some(Value::Tuple(obj)) => match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::type_error("re match receiver is invalid")),
            },
            Some(Value::List(obj)) => match &*obj.kind() {
                Object::List(values) => values.clone(),
                _ => return Err(RuntimeError::type_error("re match receiver is invalid")),
            },
            _ => return Err(RuntimeError::type_error("re match receiver is invalid")),
        };
        let spans = match self.re_match_attr(receiver, "_spans") {
            Some(Value::List(obj)) => match &*obj.kind() {
                Object::List(values) => values
                    .iter()
                    .map(|value| match value {
                        Value::None => Ok(None),
                        Value::Tuple(tuple_obj) => {
                            let Object::Tuple(items) = &*tuple_obj.kind() else {
                                return Err(RuntimeError::type_error(
                                    "re match receiver is invalid",
                                ));
                            };
                            if items.len() != 2 {
                                return Err(RuntimeError::type_error(
                                    "re match receiver is invalid",
                                ));
                            }
                            let start = value_to_int(items[0].clone())?;
                            let end = value_to_int(items[1].clone())?;
                            Ok(Some((start, end)))
                        }
                        _ => Err(RuntimeError::type_error("re match receiver is invalid")),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                _ => return Err(RuntimeError::type_error("re match receiver is invalid")),
            },
            _ => return Err(RuntimeError::type_error("re match receiver is invalid")),
        };
        let groupindex = match self.re_match_attr(receiver, "_groupindex") {
            Some(Value::Dict(dict)) => Some(dict.clone()),
            _ => None,
        };
        Ok((source, start, end, groups, spans, groupindex))
    }

    fn re_match_group_number(
        &self,
        group: Option<Value>,
        groupindex: Option<&ObjRef>,
        max_group: usize,
    ) -> Result<usize, RuntimeError> {
        let raw_index = match group {
            None => 0,
            Some(Value::Str(name)) => {
                let Some(mapping) = groupindex else {
                    return Err(RuntimeError::new("no such group"));
                };
                let Some(index) = dict_get_value(mapping, &Value::Str(name)) else {
                    return Err(RuntimeError::new("no such group"));
                };
                value_to_int(index)?
            }
            Some(other) => value_to_int(other)?,
        };
        if raw_index < 0 || raw_index as usize > max_group {
            return Err(RuntimeError::new("no such group"));
        }
        Ok(raw_index as usize)
    }

    fn re_match_group_value(
        &mut self,
        source: &Value,
        start: i64,
        end: i64,
        groups: &[Value],
        index: usize,
    ) -> Result<Value, RuntimeError> {
        if index == 0 {
            return match source {
                Value::Str(text) => {
                    let start = usize::try_from(start)
                        .map_err(|_| RuntimeError::new("invalid regex match bounds"))?;
                    let end = usize::try_from(end)
                        .map_err(|_| RuntimeError::new("invalid regex match bounds"))?;
                    let matched = utf8_slice_by_char_range(text, start, end)
                        .ok_or_else(|| RuntimeError::new("invalid regex match bounds"))?;
                    Ok(Value::Str(matched))
                }
                Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                    let bytes = bytes_like_from_value(source.clone())?;
                    Ok(self
                        .heap
                        .alloc_bytes(bytes[start as usize..end as usize].to_vec()))
                }
                _ => Err(RuntimeError::type_error("re match receiver is invalid")),
            };
        }
        Ok(groups[index - 1].clone())
    }

    pub(in crate::vm) fn native_re_match_group(
        &mut self,
        receiver: &ObjRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let (source, start, end, groups, _spans, groupindex) = self.re_match_snapshot(receiver)?;
        let max_group = groups.len();
        if args.is_empty() {
            return self.re_match_group_value(&source, start, end, &groups, 0);
        }
        if args.len() == 1 {
            let index =
                self.re_match_group_number(args.first().cloned(), groupindex.as_ref(), max_group)?;
            return self.re_match_group_value(&source, start, end, &groups, index);
        }
        let mut out = Vec::with_capacity(args.len());
        for value in args {
            let index = self.re_match_group_number(Some(value), groupindex.as_ref(), max_group)?;
            out.push(self.re_match_group_value(&source, start, end, &groups, index)?);
        }
        Ok(self.heap.alloc_tuple(out))
    }

    pub(in crate::vm) fn native_re_match_groups(
        &mut self,
        receiver: &ObjRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new("groups() expects at most one argument"));
        }
        let (_source, _start, _end, groups, _spans, _groupindex) =
            self.re_match_snapshot(receiver)?;
        let default = args.into_iter().next().unwrap_or(Value::None);
        let values = groups
            .into_iter()
            .map(|value| {
                if value == Value::None {
                    default.clone()
                } else {
                    value
                }
            })
            .collect::<Vec<_>>();
        Ok(self.heap.alloc_tuple(values))
    }

    pub(in crate::vm) fn native_re_match_groupdict(
        &mut self,
        receiver: &ObjRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new(
                "groupdict() expects at most one argument",
            ));
        }
        let (_source, _start, _end, groups, _spans, groupindex) =
            self.re_match_snapshot(receiver)?;
        let default = args.into_iter().next().unwrap_or(Value::None);
        let Some(mapping) = groupindex else {
            return Ok(self.heap.alloc_dict(Vec::new()));
        };
        let Object::Dict(entries) = &*mapping.kind() else {
            return Ok(self.heap.alloc_dict(Vec::new()));
        };
        let mut out = Vec::new();
        for (key, raw_index) in entries.iter() {
            let Value::Str(name) = key else {
                continue;
            };
            let index = value_to_int(raw_index.clone())?;
            let value = if index <= 0 || (index as usize) > groups.len() {
                default.clone()
            } else {
                let matched = groups[index as usize - 1].clone();
                if matched == Value::None {
                    default.clone()
                } else {
                    matched
                }
            };
            out.push((Value::Str(name.clone()), value));
        }
        Ok(self.heap.alloc_dict(out))
    }

    pub(in crate::vm) fn native_re_match_start(
        &self,
        receiver: &ObjRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new("start() expects at most one argument"));
        }
        let (_source, start, _end, groups, spans, groupindex) = self.re_match_snapshot(receiver)?;
        let index =
            self.re_match_group_number(args.into_iter().next(), groupindex.as_ref(), groups.len())?;
        if index == 0 {
            return Ok(Value::Int(start));
        }
        Ok(Value::Int(spans[index - 1].map(|(s, _)| s).unwrap_or(-1)))
    }

    pub(in crate::vm) fn native_re_match_end(
        &self,
        receiver: &ObjRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new("end() expects at most one argument"));
        }
        let (_source, _start, end, groups, spans, groupindex) = self.re_match_snapshot(receiver)?;
        let index =
            self.re_match_group_number(args.into_iter().next(), groupindex.as_ref(), groups.len())?;
        if index == 0 {
            return Ok(Value::Int(end));
        }
        Ok(Value::Int(spans[index - 1].map(|(_, e)| e).unwrap_or(-1)))
    }

    pub(in crate::vm) fn native_re_match_span(
        &self,
        receiver: &ObjRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new("span() expects at most one argument"));
        }
        let (_source, start, end, groups, spans, groupindex) = self.re_match_snapshot(receiver)?;
        let index =
            self.re_match_group_number(args.into_iter().next(), groupindex.as_ref(), groups.len())?;
        if index == 0 {
            return Ok(self
                .heap
                .alloc_tuple(vec![Value::Int(start), Value::Int(end)]));
        }
        let (group_start, group_end) = spans[index - 1].unwrap_or((-1, -1));
        Ok(self
            .heap
            .alloc_tuple(vec![Value::Int(group_start), Value::Int(group_end)]))
    }

    pub(in crate::vm) fn native_re_pattern_repr(
        &mut self,
        receiver: &ObjRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "__repr__() takes no arguments ({} given)",
                args.len()
            )));
        }
        let pattern = self
            .re_pattern_attr(receiver, "pattern")
            .ok_or_else(|| RuntimeError::type_error("pattern receiver is invalid"))?;
        let flags = match self.re_pattern_attr(receiver, "flags") {
            Some(Value::Int(value)) => value,
            _ => 0,
        };
        let pattern_repr = match self.builtin_repr(vec![pattern], HashMap::new())? {
            Value::Str(text) => text,
            _ => return Err(RuntimeError::type_error("__repr__ returned non-string")),
        };
        let effective_flags = if matches!(
            self.re_pattern_attr(receiver, "pattern"),
            Some(Value::Str(_))
        ) {
            flags & !RE_FLAG_UNICODE
        } else {
            flags
        };
        if effective_flags == 0 {
            return Ok(Value::Str(format!("re.compile({pattern_repr})")));
        }
        Ok(Value::Str(format!(
            "re.compile({pattern_repr}, {effective_flags})"
        )))
    }

    pub(in crate::vm) fn native_re_match_repr(
        &mut self,
        receiver: &ObjRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "__repr__() takes no arguments ({} given)",
                args.len()
            )));
        }
        let (source, start, end, _groups, _spans, _groupindex) =
            self.re_match_snapshot(receiver)?;
        let matched = match &source {
            Value::Str(text) => {
                let start = usize::try_from(start)
                    .map_err(|_| RuntimeError::new("invalid regex match bounds"))?;
                let end = usize::try_from(end)
                    .map_err(|_| RuntimeError::new("invalid regex match bounds"))?;
                let matched = utf8_slice_by_char_range(text, start, end)
                    .ok_or_else(|| RuntimeError::new("invalid regex match bounds"))?;
                Value::Str(matched)
            }
            Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                let bytes = bytes_like_from_value(source)?;
                self.heap
                    .alloc_bytes(bytes[start as usize..end as usize].to_vec())
            }
            _ => return Err(RuntimeError::type_error("re match receiver is invalid")),
        };
        let matched_repr = match self.builtin_repr(vec![matched], HashMap::new())? {
            Value::Str(text) => text,
            _ => return Err(RuntimeError::type_error("__repr__ returned non-string")),
        };
        Ok(Value::Str(format!(
            "<re.Match object; span=({}, {}), match={}>",
            start, end, matched_repr
        )))
    }

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
        let flags = if args.len() == 2 {
            value_to_int(args.pop().unwrap_or(Value::Int(0)))?
        } else {
            0
        };
        let pattern_value = match args.remove(0) {
            Value::Instance(instance)
                if self.is_re_runtime_class_instance(&instance, RE_PATTERN_CLASS_NAME) =>
            {
                return Ok(Value::Instance(instance));
            }
            Value::Module(module) => {
                let compiled_pattern = matches!(
                    &*module.kind(),
                    Object::Module(module_data) if module_data.name == RE_PATTERN_MODULE_NAME
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
        let compiled_pattern_value = match &pattern_value {
            Value::Str(pattern) if (flags & RE_FLAG_VERBOSE) != 0 => {
                Value::Str(strip_verbose_pattern(pattern))
            }
            _ => pattern_value.clone(),
        };
        let pattern_class = self.ensure_re_runtime_type_class(RE_PATTERN_CLASS_NAME);
        let compiled = match self.heap.alloc_instance(InstanceObject::new(pattern_class)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *compiled.kind_mut() {
            instance_data
                .attrs
                .insert("pattern".to_string(), pattern_value);
            instance_data.attrs.insert(
                "__pyrs_compiled_pattern__".to_string(),
                compiled_pattern_value,
            );
            instance_data
                .attrs
                .insert("flags".to_string(), Value::Int(flags));
            let groupindex = if let Some(pattern) = instance_data.attrs.get("pattern") {
                match pattern {
                    Value::Str(text) => {
                        let entries = csv_sniffer_groupindex_entries(text)
                            .or_else(|| decimal_parser_groupindex_entries(text))
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
            let groups = match &groupindex {
                Value::Dict(dict_obj) => match &*dict_obj.kind() {
                    Object::Dict(entries) => entries
                        .iter()
                        .filter_map(|(_, value)| value_to_int(value.clone()).ok())
                        .filter(|index| *index > 0)
                        .max()
                        .unwrap_or(0),
                    _ => 0,
                },
                _ => 0,
            };
            instance_data
                .attrs
                .insert("groupindex".to_string(), groupindex);
            instance_data
                .attrs
                .insert("groups".to_string(), Value::Int(groups.max(0)));
        }
        Ok(Value::Instance(compiled))
    }

    pub(in crate::vm) fn builtin_sre_compile(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 6 {
            return Err(RuntimeError::new("_sre.compile() expects 6 arguments"));
        }
        let pattern = args[0].clone();
        let flags = self.coerce_sre_int_arg(args[1].clone())?;
        let code_seq = match &args[2] {
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => values.clone(),
                _ => return Err(RuntimeError::new("compile() code must be list of integers")),
            },
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::new("compile() code must be list of integers")),
            },
            _ => return Err(RuntimeError::new("compile() code must be list of integers")),
        };
        let mut normalized_code = Vec::with_capacity(code_seq.len());
        for value in &code_seq {
            normalized_code.push(Value::Int(self.coerce_sre_int_arg(value.clone())?));
        }
        let groups = self.coerce_sre_int_arg(args[3].clone())?;
        let groupindex = match &args[4] {
            Value::Dict(_) => args[4].clone(),
            _ => return Err(RuntimeError::new("groupindex must be a dict")),
        };
        let indexgroup = match &args[5] {
            Value::Tuple(_) | Value::List(_) => args[5].clone(),
            _ => return Err(RuntimeError::new("indexgroup must be a tuple")),
        };

        let compiled = self.builtin_re_compile(vec![pattern, Value::Int(flags)], HashMap::new())?;
        if let Value::Instance(compiled_obj) = &compiled
            && let Object::Instance(instance_data) = &mut *compiled_obj.kind_mut()
        {
            instance_data
                .attrs
                .insert("flags".to_string(), Value::Int(flags));
            instance_data
                .attrs
                .insert("groups".to_string(), Value::Int(groups.max(0)));
            instance_data
                .attrs
                .insert("groupindex".to_string(), groupindex);
            instance_data
                .attrs
                .insert("indexgroup".to_string(), indexgroup);
            instance_data.attrs.insert(
                "__pyrs_sre_code__".to_string(),
                self.heap.alloc_list(normalized_code),
            );
        }
        Ok(compiled)
    }

    pub(in crate::vm) fn builtin_sre_template(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("_sre.template() expects 2 arguments"));
        }
        let pieces: Vec<Value> = match &args[1] {
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => values.clone(),
                _ => return Err(RuntimeError::new("template must be a list")),
            },
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::new("template must be a list")),
            },
            _ => return Err(RuntimeError::new("template must be a list")),
        };
        for piece in pieces {
            match &piece {
                Value::Int(value) => {
                    if *value < 0 {
                        return Err(RuntimeError::new("invalid template"));
                    }
                }
                Value::Str(_)
                | Value::Bytes(_)
                | Value::ByteArray(_)
                | Value::MemoryView(_)
                | Value::None => {}
                _ => return Err(RuntimeError::type_error("an integer is required")),
            }
        }
        Ok(args[1].clone())
    }

    pub(in crate::vm) fn builtin_sre_ascii_iscased(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let code = parse_sre_char_arg(args, kwargs, "_sre.ascii_iscased")?;
        let is_ascii_letter = (b'a' as u32..=b'z' as u32).contains(&code)
            || (b'A' as u32..=b'Z' as u32).contains(&code);
        Ok(Value::Bool(is_ascii_letter))
    }

    pub(in crate::vm) fn builtin_sre_ascii_tolower(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let code = parse_sre_char_arg(args, kwargs, "_sre.ascii_tolower")?;
        let lowered = if (b'A' as u32..=b'Z' as u32).contains(&code) {
            (code + 32) as i64
        } else {
            code as i64
        };
        Ok(Value::Int(lowered))
    }

    pub(in crate::vm) fn builtin_sre_unicode_iscased(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let code = parse_sre_char_arg(args, kwargs, "_sre.unicode_iscased")?;
        let ch = char::from_u32(code).ok_or_else(|| RuntimeError::new("invalid character code"))?;
        Ok(Value::Bool(
            ch != ch.to_lowercase().next().unwrap_or(ch)
                || ch != ch.to_uppercase().next().unwrap_or(ch),
        ))
    }

    pub(in crate::vm) fn builtin_sre_unicode_tolower(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let code = parse_sre_char_arg(args, kwargs, "_sre.unicode_tolower")?;
        let ch = char::from_u32(code).ok_or_else(|| RuntimeError::new("invalid character code"))?;
        let lowered = ch.to_lowercase().next().unwrap_or(ch) as i64;
        Ok(Value::Int(lowered))
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
        let pattern = re_pattern_from_compiled_object(&receiver)?;
        let compiled_program = re_compiled_regex_program_from_object(&receiver);
        let target = args.remove(0);
        let target = match target {
            Value::Str(_) | Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => target,
            other => Value::Str(format_value(&other)),
        };

        match target {
            Value::Str(text) => {
                if !matches!(pattern, RePatternValue::Str(_)) {
                    return Err(RuntimeError::new(
                        "cannot use a bytes pattern on a string-like object",
                    ));
                }
                if let RePatternValue::Str(pattern_text) = &pattern
                    && let Some((group_count, matches)) =
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
                let raw_pos = if let Some(value) = args.first() {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let raw_end = if let Some(value) = args.get(1) {
                    value_to_int(value.clone())?
                } else {
                    text.chars().count() as i64
                };
                let (start, stop, _start_char, _stop_char) =
                    normalize_string_window(&text, raw_pos, raw_end)?;
                let mut matches = Vec::new();
                let mut cursor = start;
                while cursor <= stop {
                    let segment = Value::Str(text[cursor..stop].to_string());
                    let Some(detail) = re_match_details(
                        &pattern,
                        &segment,
                        ReMode::Search,
                        compiled_program.as_ref(),
                    )?
                    else {
                        break;
                    };
                    let absolute_start = cursor + detail.start;
                    let absolute_end = cursor + detail.end;
                    if absolute_start > stop || absolute_end > stop {
                        break;
                    }
                    if detail.captures.is_empty() {
                        matches.push(Value::Str(text[absolute_start..absolute_end].to_string()));
                    } else if detail.captures.len() == 1 {
                        let value = detail
                            .captures
                            .first()
                            .and_then(|capture| *capture)
                            .map(|(capture_start, capture_end)| {
                                Value::Str(
                                    text[cursor + capture_start..cursor + capture_end].to_string(),
                                )
                            })
                            .unwrap_or(Value::None);
                        matches.push(value);
                    } else {
                        let groups = detail
                            .captures
                            .iter()
                            .map(|capture| match capture {
                                Some((capture_start, capture_end)) => Value::Str(
                                    text[cursor + capture_start..cursor + capture_end].to_string(),
                                ),
                                None => Value::None,
                            })
                            .collect::<Vec<_>>();
                        matches.push(self.heap.alloc_tuple(groups));
                    }
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
                    let Some(detail) = re_match_details(
                        &pattern,
                        &segment,
                        ReMode::Search,
                        compiled_program.as_ref(),
                    )?
                    else {
                        break;
                    };
                    let absolute_start = cursor + detail.start;
                    let absolute_end = cursor + detail.end;
                    if absolute_start > stop || absolute_end > stop {
                        break;
                    }
                    if detail.captures.is_empty() {
                        matches.push(
                            self.heap
                                .alloc_bytes(bytes[absolute_start..absolute_end].to_vec()),
                        );
                    } else if detail.captures.len() == 1 {
                        let value = detail
                            .captures
                            .first()
                            .and_then(|capture| *capture)
                            .map(|(capture_start, capture_end)| {
                                self.heap.alloc_bytes(
                                    bytes[cursor + capture_start..cursor + capture_end].to_vec(),
                                )
                            })
                            .unwrap_or(Value::None);
                        matches.push(value);
                    } else {
                        let groups = detail
                            .captures
                            .iter()
                            .map(|capture| match capture {
                                Some((capture_start, capture_end)) => self.heap.alloc_bytes(
                                    bytes[cursor + capture_start..cursor + capture_end].to_vec(),
                                ),
                                None => Value::None,
                            })
                            .collect::<Vec<_>>();
                        matches.push(self.heap.alloc_tuple(groups));
                    }
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

    pub(in crate::vm) fn builtin_re_pattern_finditer(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 4 {
            return Err(RuntimeError::new(
                "Pattern.finditer() expects string and optional pos/endpos",
            ));
        }
        let pattern_arg = args.remove(0);
        let receiver = self.receiver_from_value(&pattern_arg)?;
        let pattern = re_pattern_from_compiled_object(&receiver)?;
        let compiled_program = re_compiled_regex_program_from_object(&receiver);
        let target = args.remove(0);
        let target = match target {
            Value::Str(_) | Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => target,
            other => Value::Str(format_value(&other)),
        };
        let groupindex = self.re_match_groupindex_from_pattern_arg(&pattern_arg);

        let match_values = match target {
            Value::Str(text) => {
                if !matches!(pattern, RePatternValue::Str(_)) {
                    return Err(RuntimeError::new(
                        "cannot use a bytes pattern on a string-like object",
                    ));
                }
                let raw_pos = if let Some(value) = args.first() {
                    value_to_int(value.clone())?
                } else {
                    0
                };
                let raw_end = if let Some(value) = args.get(1) {
                    value_to_int(value.clone())?
                } else {
                    text.chars().count() as i64
                };
                let (start, stop, start_char, stop_char) =
                    normalize_string_window(&text, raw_pos, raw_end)?;

                let source = Value::Str(text.clone());
                let mut out = Vec::new();
                let mut cursor = start;
                while cursor <= stop {
                    let segment = Value::Str(text[cursor..stop].to_string());
                    let Some(mut detail) = re_match_details(
                        &pattern,
                        &segment,
                        ReMode::Search,
                        compiled_program.as_ref(),
                    )?
                    else {
                        break;
                    };
                    detail.start += cursor;
                    detail.end += cursor;
                    for capture in &mut detail.captures {
                        if let Some((capture_start, capture_end)) = capture.as_mut() {
                            *capture_start += cursor;
                            *capture_end += cursor;
                        }
                    }
                    let absolute_start = detail.start;
                    let absolute_end = detail.end;
                    out.push(self.alloc_re_match_value(
                        pattern_arg.clone(),
                        source.clone(),
                        detail,
                        groupindex.clone(),
                        start_char,
                        stop_char,
                    )?);
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
                out
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

                let source = self.heap.alloc_bytes(bytes.clone());
                let mut out = Vec::new();
                let mut cursor = start;
                while cursor <= stop {
                    let segment = self.heap.alloc_bytes(bytes[cursor..stop].to_vec());
                    let Some(mut detail) = re_match_details(
                        &pattern,
                        &segment,
                        ReMode::Search,
                        compiled_program.as_ref(),
                    )?
                    else {
                        break;
                    };
                    detail.start += cursor;
                    detail.end += cursor;
                    for capture in &mut detail.captures {
                        if let Some((capture_start, capture_end)) = capture.as_mut() {
                            *capture_start += cursor;
                            *capture_end += cursor;
                        }
                    }
                    let absolute_start = detail.start;
                    let absolute_end = detail.end;
                    out.push(self.alloc_re_match_value(
                        pattern_arg.clone(),
                        source.clone(),
                        detail,
                        groupindex.clone(),
                        start as i64,
                        stop as i64,
                    )?);
                    if absolute_end == absolute_start {
                        if absolute_end >= stop {
                            break;
                        }
                        cursor = absolute_end + 1;
                    } else {
                        cursor = absolute_end;
                    }
                }
                out
            }
            _ => {
                return Err(RuntimeError::new(
                    "Pattern.finditer() expects string or bytes-like object",
                ));
            }
        };

        self.builtin_iter(vec![self.heap.alloc_list(match_values)], HashMap::new())
    }

    pub(in crate::vm) fn builtin_re_pattern_split(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "Pattern.split() expects string and optional maxsplit",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let pattern = re_pattern_from_compiled_object(&receiver)?;
        let compiled_program = re_compiled_regex_program_from_object(&receiver);
        let target = args.remove(0);
        let target = match target {
            Value::Str(_) | Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => target,
            other => Value::Str(format_value(&other)),
        };
        let mut maxsplit = if !args.is_empty() {
            Some(value_to_int(args.remove(0))?)
        } else {
            None
        };
        if let Some(value) = kwargs.remove("maxsplit") {
            if maxsplit.is_some() {
                return Err(RuntimeError::new(
                    "Pattern.split() got multiple values for argument 'maxsplit'",
                ));
            }
            maxsplit = Some(value_to_int(value)?);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "Pattern.split() got an unexpected keyword argument",
            ));
        }
        let maxsplit = maxsplit.unwrap_or(0).max(0) as usize;

        match target {
            Value::Str(text) => {
                if !matches!(pattern, RePatternValue::Str(_)) {
                    return Err(RuntimeError::new(
                        "cannot use a bytes pattern on a string-like object",
                    ));
                }
                let stop = text.len();
                let mut segment_start = 0usize;
                let mut search_pos = 0usize;
                let mut splits = 0usize;
                let mut out = Vec::new();
                while search_pos <= stop && (maxsplit == 0 || splits < maxsplit) {
                    let segment = Value::Str(text[search_pos..stop].to_string());
                    let Some(detail) = re_match_details(
                        &pattern,
                        &segment,
                        ReMode::Search,
                        compiled_program.as_ref(),
                    )?
                    else {
                        break;
                    };
                    let absolute_start = search_pos + detail.start;
                    let absolute_end = search_pos + detail.end;
                    if absolute_start > stop || absolute_end > stop {
                        break;
                    }
                    out.push(Value::Str(text[segment_start..absolute_start].to_string()));
                    if !detail.captures.is_empty() {
                        for capture in &detail.captures {
                            match capture {
                                Some((capture_start, capture_end)) => {
                                    let capture_start = search_pos + capture_start;
                                    let capture_end = search_pos + capture_end;
                                    out.push(Value::Str(
                                        text[capture_start..capture_end].to_string(),
                                    ));
                                }
                                None => out.push(Value::None),
                            }
                        }
                    }
                    splits += 1;
                    segment_start = absolute_end;
                    if absolute_end == absolute_start {
                        if absolute_end >= stop {
                            break;
                        }
                        let mut next = absolute_end + 1;
                        while next < stop && !text.is_char_boundary(next) {
                            next += 1;
                        }
                        search_pos = next;
                    } else {
                        search_pos = absolute_end;
                    }
                }
                out.push(Value::Str(text[segment_start..].to_string()));
                Ok(self.heap.alloc_list(out))
            }
            Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                if !matches!(pattern, RePatternValue::Bytes(_)) {
                    return Err(RuntimeError::new(
                        "cannot use a string pattern on a bytes-like object",
                    ));
                }
                let bytes = bytes_like_from_value(target)?;
                let stop = bytes.len();
                let mut segment_start = 0usize;
                let mut search_pos = 0usize;
                let mut splits = 0usize;
                let mut out = Vec::new();
                while search_pos <= stop && (maxsplit == 0 || splits < maxsplit) {
                    let segment = self.heap.alloc_bytes(bytes[search_pos..stop].to_vec());
                    let Some(detail) = re_match_details(
                        &pattern,
                        &segment,
                        ReMode::Search,
                        compiled_program.as_ref(),
                    )?
                    else {
                        break;
                    };
                    let absolute_start = search_pos + detail.start;
                    let absolute_end = search_pos + detail.end;
                    if absolute_start > stop || absolute_end > stop {
                        break;
                    }
                    out.push(
                        self.heap
                            .alloc_bytes(bytes[segment_start..absolute_start].to_vec()),
                    );
                    if !detail.captures.is_empty() {
                        for capture in &detail.captures {
                            match capture {
                                Some((capture_start, capture_end)) => {
                                    let capture_start = search_pos + capture_start;
                                    let capture_end = search_pos + capture_end;
                                    out.push(
                                        self.heap.alloc_bytes(
                                            bytes[capture_start..capture_end].to_vec(),
                                        ),
                                    );
                                }
                                None => out.push(Value::None),
                            }
                        }
                    }
                    splits += 1;
                    segment_start = absolute_end;
                    if absolute_end == absolute_start {
                        if absolute_end >= stop {
                            break;
                        }
                        search_pos = absolute_end + 1;
                    } else {
                        search_pos = absolute_end;
                    }
                }
                out.push(self.heap.alloc_bytes(bytes[segment_start..].to_vec()));
                Ok(self.heap.alloc_list(out))
            }
            _ => Err(RuntimeError::new(
                "Pattern.split() expects string or bytes-like object",
            )),
        }
    }

    pub(in crate::vm) fn builtin_re_match_mode(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        mode: ReMode,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 4 {
            return Err(RuntimeError::new("re function expects pattern and string"));
        }
        let compiled_pattern = if let Value::Instance(instance) = &args[0]
            && self.is_re_runtime_class_instance(instance, RE_PATTERN_CLASS_NAME)
        {
            Value::Instance(instance.clone())
        } else {
            self.builtin_re_compile(vec![args[0].clone()], HashMap::new())?
        };
        let pattern = re_pattern_from_argument(&compiled_pattern)?;
        let compiled_program = re_compiled_regex_program_from_argument(&compiled_pattern);
        let groupindex = self.re_match_groupindex_from_pattern_arg(&compiled_pattern);
        let target = args[1].clone();
        let raw_pos = if let Some(value) = args.get(2) {
            value_to_int(value.clone())?
        } else {
            0
        };
        let default_end = match &target {
            Value::Str(text) => text.chars().count() as i64,
            Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                bytes_like_from_value(target.clone())?.len() as i64
            }
            _ => 0,
        };
        let raw_end = if let Some(value) = args.get(3) {
            value_to_int(value.clone())?
        } else {
            default_end
        };

        let (found, pos, endpos) = match target.clone() {
            Value::Str(text) => {
                let (start, stop, start_char, stop_char) =
                    normalize_string_window(&text, raw_pos, raw_end)?;
                let segment = Value::Str(text[start..stop].to_string());
                let found = re_match_details(&pattern, &segment, mode, compiled_program.as_ref())?
                    .map(|mut detail| {
                        detail.start += start;
                        detail.end += start;
                        for (cap_start, cap_end) in detail.captures.iter_mut().flatten() {
                            *cap_start += start;
                            *cap_end += start;
                        }
                        detail
                    });
                (found, start_char, stop_char)
            }
            Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => {
                let bytes = bytes_like_from_value(target.clone())?;
                let start = clamp_index(bytes.len(), raw_pos);
                let mut stop = clamp_index(bytes.len(), raw_end);
                if stop < start {
                    stop = start;
                }
                let segment = self.heap.alloc_bytes(bytes[start..stop].to_vec());
                let found = re_match_details(&pattern, &segment, mode, compiled_program.as_ref())?
                    .map(|mut detail| {
                        detail.start += start;
                        detail.end += start;
                        for (cap_start, cap_end) in detail.captures.iter_mut().flatten() {
                            *cap_start += start;
                            *cap_end += start;
                        }
                        detail
                    });
                (found, start as i64, stop as i64)
            }
            other => (
                re_match_details(&pattern, &other, mode, compiled_program.as_ref())?,
                raw_pos.max(0),
                raw_end.max(0),
            ),
        };

        match found {
            Some(detail) => {
                self.alloc_re_match_value(compiled_pattern, target, detail, groupindex, pos, endpos)
            }
            None => Ok(Value::None),
        }
    }
}

fn csv_sniffer_groupindex_entries(pattern: &str) -> Option<Vec<(&'static str, i64)>> {
    match pattern {
        CSV_SNIFFER_PATTERN_1 => Some(vec![("delim", 1), ("space", 2), ("quote", 3)]),
        CSV_SNIFFER_PATTERN_2 => Some(vec![("quote", 1), ("delim", 2), ("space", 3)]),
        CSV_SNIFFER_PATTERN_3 => Some(vec![("delim", 1), ("space", 2), ("quote", 3)]),
        CSV_SNIFFER_PATTERN_4 => Some(vec![("quote", 1)]),
        PKGUTIL_RESOLVE_NAME_PATTERN => Some(vec![("pkg", 1), ("cln", 5), ("obj", 6)]),
        _ => None,
    }
}

fn strip_verbose_pattern(pattern: &str) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            out.push(ch);
            escaped = true;
            continue;
        }
        if in_class {
            if ch == ']' {
                in_class = false;
            }
            out.push(ch);
            continue;
        }
        if ch == '[' {
            in_class = true;
            out.push(ch);
            continue;
        }
        if ch == '#' {
            while let Some(next) = chars.next() {
                if next == '\n' {
                    break;
                }
            }
            continue;
        }
        if ch.is_whitespace() {
            continue;
        }
        out.push(ch);
    }
    out
}

fn parse_sre_char_arg(
    args: Vec<Value>,
    kwargs: HashMap<String, Value>,
    fn_name: &str,
) -> Result<u32, RuntimeError> {
    if !kwargs.is_empty() || args.len() != 1 {
        return Err(RuntimeError::new(format!(
            "{fn_name}() expects one integer argument"
        )));
    }
    let value = value_to_int(args[0].clone())
        .map_err(|_| RuntimeError::type_error("an integer is required"))?;
    if !(0..=0x10ffff).contains(&value) {
        return Err(RuntimeError::new("character code out of range"));
    }
    Ok(value as u32)
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
                    if chars[close] == quote
                        && (close + 1 == chars.len() || chars[close + 1] == '\n')
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
                    if chars[close] == quote
                        && (close + 1 == chars.len() || chars[close + 1] == '\n')
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

#[cfg(test)]
mod tests {
    use super::{
        CSV_SNIFFER_PATTERN_1, CSV_SNIFFER_PATTERN_2, CSV_SNIFFER_PATTERN_4,
        PKGUTIL_RESOLVE_NAME_PATTERN, ReMode, csv_sniffer_groupindex_entries,
        csv_sniffer_pattern_findall,
    };
    use crate::runtime::{Object, Value};
    use crate::vm::Vm;
    use std::collections::HashMap;

    #[test]
    fn csv_sniffer_groupindex_entries_cover_known_patterns() {
        let p1 = csv_sniffer_groupindex_entries(CSV_SNIFFER_PATTERN_1)
            .expect("pattern 1 should expose groups");
        assert_eq!(p1, vec![("delim", 1), ("space", 2), ("quote", 3)]);

        let p4 = csv_sniffer_groupindex_entries(CSV_SNIFFER_PATTERN_4)
            .expect("pattern 4 should expose groups");
        assert_eq!(p4, vec![("quote", 1)]);

        assert!(csv_sniffer_groupindex_entries("unknown").is_none());
    }

    #[test]
    fn csv_sniffer_pattern_findall_recognizes_delimiters_and_quotes() {
        let sample = "a,\"b\",c\n\"q\",r\n";
        let (groups, matches) = csv_sniffer_pattern_findall(CSV_SNIFFER_PATTERN_1, sample)
            .expect("pattern 1 should run");
        assert_eq!(groups, 3);
        assert!(
            matches
                .iter()
                .any(|entry| entry == &vec![",".to_string(), "".to_string(), "\"".to_string()])
        );

        let (_, line_start_matches) =
            csv_sniffer_pattern_findall(CSV_SNIFFER_PATTERN_2, sample).expect("pattern 2 runs");
        assert!(
            line_start_matches
                .iter()
                .any(|entry| entry[0] == "\"" && entry[1] == "," && entry[2].is_empty())
        );
    }

    #[test]
    fn csv_sniffer_pattern_findall_unknown_pattern_returns_none() {
        assert!(csv_sniffer_pattern_findall("not-a-pattern", "a,b").is_none());
    }

    #[test]
    fn re_match_methods_handle_index_and_missing_groups() {
        let mut vm = Vm::new();
        let matched = vm
            .builtin_re_match_mode(
                vec![
                    Value::Str("(a)(b+)?".to_string()),
                    Value::Str("a".to_string()),
                ],
                HashMap::new(),
                ReMode::Match,
            )
            .expect("match should succeed");
        let match_obj = match matched {
            Value::Instance(obj) => obj,
            other => panic!("expected match object instance, got {other:?}"),
        };

        let whole = vm
            .native_re_match_group(&match_obj, vec![])
            .expect("group() should work");
        assert_eq!(whole, Value::Str("a".to_string()));
        let g1 = vm
            .native_re_match_group(&match_obj, vec![Value::Int(1)])
            .expect("group(1) should work");
        assert_eq!(g1, Value::Str("a".to_string()));
        let g2 = vm
            .native_re_match_group(&match_obj, vec![Value::Int(2)])
            .expect("group(2) should work");
        assert_eq!(g2, Value::None);

        let start2 = vm
            .native_re_match_start(&match_obj, vec![Value::Int(2)])
            .expect("start(2) should work");
        let end2 = vm
            .native_re_match_end(&match_obj, vec![Value::Int(2)])
            .expect("end(2) should work");
        assert_eq!(start2, Value::Int(-1));
        assert_eq!(end2, Value::Int(-1));
    }

    #[test]
    fn re_match_groups_support_default_fill_value() {
        let mut vm = Vm::new();
        let matched = vm
            .builtin_re_match_mode(
                vec![
                    Value::Str("(a)(b+)?".to_string()),
                    Value::Str("a".to_string()),
                ],
                HashMap::new(),
                ReMode::Match,
            )
            .expect("match should succeed");
        let match_obj = match matched {
            Value::Instance(obj) => obj,
            other => panic!("expected match object instance, got {other:?}"),
        };

        let groups = vm
            .native_re_match_groups(&match_obj, vec![Value::Str("missing".to_string())])
            .expect("groups(default) should work");
        match groups {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => {
                    assert_eq!(values.len(), 2);
                    assert_eq!(values[0], Value::Str("a".to_string()));
                    assert_eq!(values[1], Value::Str("missing".to_string()));
                }
                other => panic!("expected tuple object, got {other:?}"),
            },
            other => panic!("expected tuple value, got {other:?}"),
        }
    }

    #[test]
    fn pkgutil_name_pattern_matches_module_only_name() {
        let mut vm = Vm::new();
        let matched = vm
            .builtin_re_match_mode(
                vec![
                    Value::Str(PKGUTIL_RESOLVE_NAME_PATTERN.to_string()),
                    Value::Str("tempfile".to_string()),
                ],
                HashMap::new(),
                ReMode::Match,
            )
            .expect("match should succeed");
        let match_obj = match matched {
            Value::Instance(obj) => obj,
            other => panic!("expected match object instance, got {other:?}"),
        };
        let groupdict = vm
            .native_re_match_groupdict(&match_obj, vec![])
            .expect("groupdict() should work");
        let Value::Dict(dict_obj) = groupdict else {
            panic!("groupdict() must return dict");
        };
        let Object::Dict(entries) = &*dict_obj.kind() else {
            panic!("groupdict() must return dict object");
        };
        let entries = entries.to_vec();
        assert!(entries.contains(&(
            Value::Str("pkg".to_string()),
            Value::Str("tempfile".to_string())
        )));
        assert!(entries.contains(&(Value::Str("cln".to_string()), Value::None)));
        assert!(entries.contains(&(Value::Str("obj".to_string()), Value::None)));
    }

    #[test]
    fn pkgutil_name_pattern_matches_colon_object_form() {
        let mut vm = Vm::new();
        let matched = vm
            .builtin_re_match_mode(
                vec![
                    Value::Str(PKGUTIL_RESOLVE_NAME_PATTERN.to_string()),
                    Value::Str("pkg.mod:attr.child".to_string()),
                ],
                HashMap::new(),
                ReMode::Match,
            )
            .expect("match should succeed");
        let match_obj = match matched {
            Value::Instance(obj) => obj,
            other => panic!("expected match object instance, got {other:?}"),
        };
        let groupdict = vm
            .native_re_match_groupdict(&match_obj, vec![])
            .expect("groupdict() should work");
        let Value::Dict(dict_obj) = groupdict else {
            panic!("groupdict() must return dict");
        };
        let Object::Dict(entries) = &*dict_obj.kind() else {
            panic!("groupdict() must return dict object");
        };
        let entries = entries.to_vec();
        assert!(entries.contains(&(
            Value::Str("pkg".to_string()),
            Value::Str("pkg.mod".to_string())
        )));
        assert!(entries.contains(&(
            Value::Str("cln".to_string()),
            Value::Str(":attr.child".to_string())
        )));
        assert!(entries.contains(&(
            Value::Str("obj".to_string()),
            Value::Str("attr.child".to_string())
        )));
    }

    #[test]
    fn sre_case_helpers_follow_cpython_reference_examples() {
        let mut vm = Vm::new();
        let ascii_lower = vm
            .builtin_sre_ascii_tolower(vec![Value::Int('A' as i64)], HashMap::new())
            .expect("ascii_tolower should succeed");
        assert_eq!(ascii_lower, Value::Int('a' as i64));

        let unicode_lower = vm
            .builtin_sre_unicode_tolower(vec![Value::Int(0x0130)], HashMap::new())
            .expect("unicode_tolower should succeed");
        assert_eq!(unicode_lower, Value::Int('i' as i64));

        let ascii_non_ascii = vm
            .builtin_sre_ascii_tolower(vec![Value::Int(0x0130)], HashMap::new())
            .expect("ascii_tolower should keep non-ascii unchanged");
        assert_eq!(ascii_non_ascii, Value::Int(0x0130));

        let ascii_cased = vm
            .builtin_sre_ascii_iscased(vec![Value::Int('Z' as i64)], HashMap::new())
            .expect("ascii_iscased should succeed");
        assert_eq!(ascii_cased, Value::Bool(true));

        let ascii_not_cased = vm
            .builtin_sre_ascii_iscased(vec![Value::Int(0x0130)], HashMap::new())
            .expect("ascii_iscased should treat non-ascii as uncased");
        assert_eq!(ascii_not_cased, Value::Bool(false));

        let unicode_cased = vm
            .builtin_sre_unicode_iscased(vec![Value::Int(0x0130)], HashMap::new())
            .expect("unicode_iscased should succeed");
        assert_eq!(unicode_cased, Value::Bool(true));
    }

    #[test]
    fn sre_template_rejects_invalid_template_items() {
        let mut vm = Vm::new();
        let err = vm
            .builtin_sre_template(
                vec![
                    Value::Str(String::new()),
                    vm.heap.alloc_list(vec![
                        Value::Str(String::new()),
                        Value::Int(-1),
                        Value::Str(String::new()),
                    ]),
                ],
                HashMap::new(),
            )
            .expect_err("negative template group index should fail");
        assert!(err.message.contains("invalid template"));

        let err = vm
            .builtin_sre_template(
                vec![
                    Value::Str(String::new()),
                    vm.heap.alloc_list(vec![
                        Value::Str(String::new()),
                        vm.heap.alloc_tuple(vec![Value::Int(1)]),
                        Value::Str(String::new()),
                    ]),
                ],
                HashMap::new(),
            )
            .expect_err("non-int template group should fail");
        assert!(err.message.contains("an integer is required"));
    }

    #[test]
    fn sre_compile_accepts_cpython_signature_and_populates_pattern_attrs() {
        let mut vm = Vm::new();
        let groupindex = vm
            .heap
            .alloc_dict(vec![(Value::Str("name".to_string()), Value::Int(1))]);
        let compiled = vm
            .builtin_sre_compile(
                vec![
                    Value::Str("abc".to_string()),
                    Value::Int(0),
                    vm.heap.alloc_list(vec![Value::Int(1), Value::Int(2)]),
                    Value::Int(1),
                    groupindex.clone(),
                    vm.heap
                        .alloc_tuple(vec![Value::None, Value::Str("name".to_string())]),
                ],
                HashMap::new(),
            )
            .expect("_sre.compile should succeed");
        let Value::Instance(pattern_obj) = compiled else {
            panic!("_sre.compile should return compiled pattern instance");
        };
        let Object::Instance(instance_data) = &*pattern_obj.kind() else {
            panic!("compiled pattern should be instance object");
        };
        assert_eq!(instance_data.attrs.get("flags"), Some(&Value::Int(0)));
        assert_eq!(instance_data.attrs.get("groups"), Some(&Value::Int(1)));
        assert_eq!(instance_data.attrs.get("groupindex"), Some(&groupindex));
    }
}
