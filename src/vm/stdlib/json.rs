use super::super::*;

#[derive(Clone)]
struct JsonDumpsOptions {
    skipkeys: bool,
    ensure_ascii: bool,
    allow_nan: bool,
    sort_keys: bool,
    item_separator: String,
    key_separator: String,
}

impl Default for JsonDumpsOptions {
    fn default() -> Self {
        Self {
            skipkeys: false,
            ensure_ascii: true,
            allow_nan: true,
            sort_keys: false,
            item_separator: ", ".to_string(),
            key_separator: ": ".to_string(),
        }
    }
}

impl Vm {
    pub(in crate::vm) fn builtin_json_dumps(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new("dumps() expects one argument"));
        }

        let mut options = JsonDumpsOptions::default();

        if let Some(skipkeys) = kwargs.remove("skipkeys") {
            options.skipkeys = is_truthy(&skipkeys);
        }
        if let Some(ensure_ascii) = kwargs.remove("ensure_ascii") {
            options.ensure_ascii = is_truthy(&ensure_ascii);
        }
        if let Some(check_circular) = kwargs.remove("check_circular") {
            if !is_truthy(&check_circular) {
                return Err(RuntimeError::new(
                    "dumps() check_circular=False is not supported yet",
                ));
            }
        }
        if let Some(allow_nan) = kwargs.remove("allow_nan") {
            options.allow_nan = is_truthy(&allow_nan);
        }
        if let Some(indent) = kwargs.remove("indent") {
            if !matches!(indent, Value::None) {
                return Err(RuntimeError::new("dumps() indent is not supported yet"));
            }
        }
        if let Some(sort_keys) = kwargs.remove("sort_keys") {
            options.sort_keys = is_truthy(&sort_keys);
        }
        if let Some(separators) = kwargs.remove("separators") {
            let (item_sep, key_sep) = parse_json_separators(separators)?;
            options.item_separator = item_sep;
            options.key_separator = key_sep;
        }

        if let Some(cls) = kwargs.remove("cls") {
            if !matches!(cls, Value::None) {
                return Err(RuntimeError::new("dumps() cls is not supported yet"));
            }
        }
        let default = kwargs.remove("default");
        if let Some(default_callable) = &default {
            if matches!(default_callable, Value::None) {
                // Treat default=None as not set.
            } else if !self.is_callable_value(default_callable) {
                return Err(RuntimeError::new("dumps() default must be callable"));
            }
        }

        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "dumps() got an unexpected keyword argument",
            ));
        }

        let default_ref = default
            .as_ref()
            .and_then(|value| if matches!(value, Value::None) { None } else { Some(value) });
        let text = json_serialize_value(self, &args[0], &options, default_ref)?;
        Ok(Value::Str(text))
    }

    pub(in crate::vm) fn builtin_json_loads(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new("loads() expects one argument"));
        }

        if let Some(strict) = kwargs.remove("strict") {
            if !is_truthy(&strict) {
                return Err(RuntimeError::new("loads() strict=False is not supported yet"));
            }
        }
        for key in [
            "cls",
            "object_hook",
            "object_pairs_hook",
            "parse_float",
            "parse_int",
            "parse_constant",
        ] {
            if let Some(value) = kwargs.remove(key) {
                if !matches!(value, Value::None) {
                    return Err(RuntimeError::new(format!(
                        "loads() {} is not supported yet",
                        key
                    )));
                }
            }
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "loads() got an unexpected keyword argument",
            ));
        }

        let text = json_source_text(&args[0])?;
        let node = parse_json_node(&text)?;
        Ok(json_node_to_value(node, &self.heap))
    }

    pub(in crate::vm) fn builtin_json_scanner_make_scanner(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("make_scanner() expects context"));
        }
        Ok(Value::Builtin(BuiltinFunction::JsonScannerScanOnce))
    }

    pub(in crate::vm) fn builtin_json_scanner_scan_once(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("scan_once() expects string and index"));
        }
        let source = match &args[0] {
            Value::Str(text) => text.clone(),
            _ => return Err(RuntimeError::new("scan_once() expects string and index")),
        };
        let idx = value_to_int(args[1].clone())?;
        if idx < 0 {
            return Err(RuntimeError::new("StopIteration: 0"));
        }
        let idx = idx as usize;
        let (node, end) = parse_json_node_from_index(&source, idx)
            .map_err(|_| RuntimeError::new(format!("StopIteration: {idx}")))?;
        let value = json_node_to_value(node, &self.heap);
        Ok(self.heap.alloc_tuple(vec![value, Value::Int(end as i64)]))
    }

    pub(in crate::vm) fn builtin_json_decoder_scanstring(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "scanstring() expects string, end, optional strict",
            ));
        }
        let source = match args.remove(0) {
            Value::Str(text) => text,
            _ => return Err(RuntimeError::new("scanstring() expects string input")),
        };
        let end = value_to_int(args.remove(0))?;
        let _strict = if !args.is_empty() {
            is_truthy(&args.remove(0))
        } else {
            true
        };
        if end <= 0 || end as usize > source.len() {
            return Err(RuntimeError::new("scanstring() end index out of range"));
        }
        let start = end as usize - 1;
        let (node, parsed_end) = parse_json_node_from_index(&source, start)?;
        let JsonNode::String(text) = node else {
            return Err(RuntimeError::new("scanstring() expected string token"));
        };
        Ok(self
            .heap
            .alloc_tuple(vec![Value::Str(text), Value::Int(parsed_end as i64)]))
    }
}

fn json_source_text(value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::Str(text) => Ok(text.clone()),
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(bytes) => String::from_utf8(bytes.clone())
                .map_err(|_| RuntimeError::new("loads() bytes must be valid UTF-8")),
            _ => Err(RuntimeError::new("loads() expects str, bytes, or bytearray")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(bytes) => String::from_utf8(bytes.clone())
                .map_err(|_| RuntimeError::new("loads() bytearray must be valid UTF-8")),
            _ => Err(RuntimeError::new("loads() expects str, bytes, or bytearray")),
        },
        _ => Err(RuntimeError::new("loads() expects str, bytes, or bytearray")),
    }
}

fn parse_json_separators(value: Value) -> Result<(String, String), RuntimeError> {
    let values = match value {
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => values.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "dumps() separators must be a 2-item tuple",
                ))
            }
        },
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => values.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "dumps() separators must be a 2-item tuple",
                ))
            }
        },
        _ => {
            return Err(RuntimeError::new(
                "dumps() separators must be a 2-item tuple",
            ))
        }
    };

    if values.len() != 2 {
        return Err(RuntimeError::new(
            "dumps() separators must be a 2-item tuple",
        ));
    }

    let item_sep = match &values[0] {
        Value::Str(text) => text.clone(),
        _ => return Err(RuntimeError::new("dumps() separators items must be strings")),
    };
    let key_sep = match &values[1] {
        Value::Str(text) => text.clone(),
        _ => return Err(RuntimeError::new("dumps() separators items must be strings")),
    };
    Ok((item_sep, key_sep))
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::None => "NoneType",
        Value::Bool(_) => "bool",
        Value::Int(_) | Value::BigInt(_) => "int",
        Value::Float(_) => "float",
        Value::Complex { .. } => "complex",
        Value::Str(_) => "str",
        Value::List(_) => "list",
        Value::Tuple(_) => "tuple",
        Value::Dict(_) => "dict",
        Value::Set(_) => "set",
        Value::FrozenSet(_) => "frozenset",
        Value::Bytes(_) => "bytes",
        Value::ByteArray(_) => "bytearray",
        Value::MemoryView(_) => "memoryview",
        Value::Iterator(_) => "iterator",
        Value::Generator(_) => "generator",
        Value::Module(_) => "module",
        Value::Class(_) => "type",
        Value::Instance(_) => "object",
        Value::Super(_) => "super",
        Value::BoundMethod(_) => "method",
        Value::Exception(_) | Value::ExceptionType(_) => "exception",
        Value::Code(_) => "code",
        Value::Function(_) | Value::Builtin(_) => "function",
        Value::Cell(_) => "cell",
        Value::Slice { .. } => "slice",
        Value::DictKeys(_) => "dict_keys",
    }
}

fn json_escape_string(text: &str, ensure_ascii: bool) -> String {
    let mut out = String::new();
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if ensure_ascii && !c.is_ascii() => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units).iter() {
                    out.push_str(&format!("\\u{:04x}", unit));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_serialize_value(
    vm: &mut Vm,
    value: &Value,
    options: &JsonDumpsOptions,
    default: Option<&Value>,
) -> Result<String, RuntimeError> {
    match value {
        Value::None => Ok("null".to_string()),
        Value::Bool(value) => Ok(if *value {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        Value::Int(value) => Ok(value.to_string()),
        Value::BigInt(value) => Ok(value.to_string()),
        Value::Float(value) => {
            if value.is_finite() {
                Ok(value.to_string())
            } else if options.allow_nan {
                if value.is_nan() {
                    Ok("NaN".to_string())
                } else if value.is_sign_negative() {
                    Ok("-Infinity".to_string())
                } else {
                    Ok("Infinity".to_string())
                }
            } else {
                Err(RuntimeError::new(
                    "Out of range float values are not JSON compliant",
                ))
            }
        }
        Value::Str(value) => Ok(json_escape_string(value, options.ensure_ascii)),
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => {
                let mut parts = Vec::with_capacity(values.len());
                for value in values {
                    parts.push(json_serialize_value(vm, value, options, default)?);
                }
                Ok(format!("[{}]", parts.join(&options.item_separator)))
            }
            _ => Err(RuntimeError::new("json unsupported type")),
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => {
                let mut parts = Vec::with_capacity(values.len());
                for value in values {
                    parts.push(json_serialize_value(vm, value, options, default)?);
                }
                Ok(format!("[{}]", parts.join(&options.item_separator)))
            }
            _ => Err(RuntimeError::new("json unsupported type")),
        },
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(entries) => {
                let mut mapped: Vec<(String, &Value)> = Vec::new();
                mapped.reserve(entries.len());
                for (key, value) in entries {
                    match key {
                        Value::Str(text) => mapped.push((text.clone(), value)),
                        _ if options.skipkeys => {}
                        _ => {
                            return Err(RuntimeError::new(
                                "keys must be str, int, float, bool or None, not unsupported type",
                            ))
                        }
                    }
                }
                if options.sort_keys {
                    mapped.sort_by(|left, right| left.0.cmp(&right.0));
                }
                let mut parts = Vec::with_capacity(mapped.len());
                for (key, value) in mapped {
                    let encoded_key = json_escape_string(&key, options.ensure_ascii);
                    let encoded_value = json_serialize_value(vm, value, options, default)?;
                    parts.push(format!(
                        "{}{sep}{}",
                        encoded_key,
                        encoded_value,
                        sep = options.key_separator
                    ));
                }
                Ok(format!("{{{}}}", parts.join(&options.item_separator)))
            }
            _ => Err(RuntimeError::new("json unsupported type")),
        },
        _ => {
            if let Some(default_callable) = default {
                match vm.call_internal(default_callable.clone(), vec![value.clone()], HashMap::new())? {
                    InternalCallOutcome::Value(converted) => {
                        json_serialize_value(vm, &converted, options, default)
                    }
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(RuntimeError::new("dumps() default callback failed"))
                    }
                }
            } else {
                Err(RuntimeError::new(format!(
                    "Object of type {} is not JSON serializable",
                    json_type_name(value)
                )))
            }
        }
    }
}

#[derive(Debug)]
enum JsonNode {
    Null,
    Bool(bool),
    Int(i64),
    BigInt(BigInt),
    Float(f64),
    String(String),
    Array(Vec<JsonNode>),
    Object(Vec<(String, JsonNode)>),
}

struct JsonParser<'a> {
    source: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source: source.as_bytes(),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<JsonNode, RuntimeError> {
        self.skip_ws();
        let value = self.parse_value()?;
        self.skip_ws();
        if self.pos != self.source.len() {
            return Err(RuntimeError::new("invalid JSON trailing data"));
        }
        Ok(value)
    }

    fn parse_value(&mut self) -> Result<JsonNode, RuntimeError> {
        self.skip_ws();
        let byte = self
            .peek()
            .ok_or_else(|| RuntimeError::new("unexpected end of JSON"))?;
        match byte {
            b'n' => self.parse_literal(b"null", JsonNode::Null),
            b't' => self.parse_literal(b"true", JsonNode::Bool(true)),
            b'f' => self.parse_literal(b"false", JsonNode::Bool(false)),
            b'"' => self.parse_string().map(JsonNode::String),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(RuntimeError::new("invalid JSON value")),
        }
    }

    fn parse_literal(&mut self, text: &[u8], node: JsonNode) -> Result<JsonNode, RuntimeError> {
        if self.source.get(self.pos..self.pos + text.len()) == Some(text) {
            self.pos += text.len();
            Ok(node)
        } else {
            Err(RuntimeError::new("invalid JSON literal"))
        }
    }

    fn parse_string(&mut self) -> Result<String, RuntimeError> {
        self.expect(b'"')?;
        let mut out = String::new();
        while let Some(byte) = self.next() {
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self
                        .next()
                        .ok_or_else(|| RuntimeError::new("invalid JSON escape"))?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex_u16()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                self.expect(b'\\')?;
                                self.expect(b'u')?;
                                let low = self.parse_hex_u16()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(RuntimeError::new("invalid unicode escape"));
                                }
                                let scalar = 0x10000
                                    + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                let ch = char::from_u32(scalar)
                                    .ok_or_else(|| RuntimeError::new("invalid unicode escape"))?;
                                out.push(ch);
                            } else if (0xDC00..=0xDFFF).contains(&code) {
                                return Err(RuntimeError::new("invalid unicode escape"));
                            } else {
                                let ch = char::from_u32(code as u32)
                                    .ok_or_else(|| RuntimeError::new("invalid unicode escape"))?;
                                out.push(ch);
                            }
                        }
                        _ => return Err(RuntimeError::new("invalid JSON escape")),
                    }
                }
                b if b < 0x80 => out.push(b as char),
                b => self.push_utf8_char(b, &mut out)?,
            }
        }
        Err(RuntimeError::new("unterminated JSON string"))
    }

    fn push_utf8_char(&mut self, first: u8, out: &mut String) -> Result<(), RuntimeError> {
        let width = if first >> 5 == 0b110 {
            2
        } else if first >> 4 == 0b1110 {
            3
        } else if first >> 3 == 0b11110 {
            4
        } else {
            return Err(RuntimeError::new("invalid UTF-8 in JSON string"));
        };
        let mut bytes = vec![first];
        for _ in 1..width {
            let next = self
                .next()
                .ok_or_else(|| RuntimeError::new("invalid UTF-8 in JSON string"))?;
            if (next & 0b1100_0000) != 0b1000_0000 {
                return Err(RuntimeError::new("invalid UTF-8 in JSON string"));
            }
            bytes.push(next);
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| RuntimeError::new("invalid UTF-8 in JSON string"))?;
        let ch = text
            .chars()
            .next()
            .ok_or_else(|| RuntimeError::new("invalid UTF-8 in JSON string"))?;
        out.push(ch);
        Ok(())
    }

    fn parse_hex_u16(&mut self) -> Result<u16, RuntimeError> {
        let mut value: u16 = 0;
        for _ in 0..4 {
            let byte = self
                .next()
                .ok_or_else(|| RuntimeError::new("invalid unicode escape"))?;
            value <<= 4;
            value |= match byte {
                b'0'..=b'9' => (byte - b'0') as u16,
                b'a'..=b'f' => (byte - b'a' + 10) as u16,
                b'A'..=b'F' => (byte - b'A' + 10) as u16,
                _ => return Err(RuntimeError::new("invalid unicode escape")),
            };
        }
        Ok(value)
    }

    fn parse_array(&mut self) -> Result<JsonNode, RuntimeError> {
        self.expect(b'[')?;
        self.skip_ws();
        let mut values = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonNode::Array(values));
        }
        loop {
            values.push(self.parse_value()?);
            self.skip_ws();
            match self.next() {
                Some(b',') => {
                    self.skip_ws();
                }
                Some(b']') => break,
                _ => return Err(RuntimeError::new("invalid JSON array")),
            }
        }
        Ok(JsonNode::Array(values))
    }

    fn parse_object(&mut self) -> Result<JsonNode, RuntimeError> {
        self.expect(b'{')?;
        self.skip_ws();
        let mut values = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonNode::Object(values));
        }
        loop {
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.parse_value()?;
            values.push((key, value));
            self.skip_ws();
            match self.next() {
                Some(b',') => {
                    self.skip_ws();
                }
                Some(b'}') => break,
                _ => return Err(RuntimeError::new("invalid JSON object")),
            }
        }
        Ok(JsonNode::Object(values))
    }

    fn parse_number(&mut self) -> Result<JsonNode, RuntimeError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.source[start..self.pos])
            .map_err(|_| RuntimeError::new("invalid JSON number"))?;
        if text.contains('.') || text.contains('e') || text.contains('E') {
            let value = text
                .parse::<f64>()
                .map_err(|_| RuntimeError::new("invalid JSON number"))?;
            Ok(JsonNode::Float(value))
        } else if let Ok(value) = text.parse::<i64>() {
            Ok(JsonNode::Int(value))
        } else {
            let (negative, digits) = match text.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, text),
            };
            let mut value = BigInt::from_str_radix(digits, 10)
                .ok_or_else(|| RuntimeError::new("invalid JSON number"))?;
            if negative {
                value = value.negated();
            }
            Ok(JsonNode::BigInt(value))
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), RuntimeError> {
        match self.next() {
            Some(found) if found == byte => Ok(()),
            _ => Err(RuntimeError::new("invalid JSON syntax")),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.pos += 1;
        Some(value)
    }
}

fn parse_json_node(source: &str) -> Result<JsonNode, RuntimeError> {
    JsonParser::new(source).parse()
}

fn parse_json_node_from_index(source: &str, idx: usize) -> Result<(JsonNode, usize), RuntimeError> {
    if idx > source.len() {
        return Err(RuntimeError::new("unexpected end of JSON"));
    }
    let mut parser = JsonParser::new(source);
    parser.pos = idx;
    let node = parser.parse_value()?;
    Ok((node, parser.pos))
}

fn json_node_to_value(node: JsonNode, heap: &Heap) -> Value {
    match node {
        JsonNode::Null => Value::None,
        JsonNode::Bool(value) => Value::Bool(value),
        JsonNode::Int(value) => Value::Int(value),
        JsonNode::BigInt(value) => Value::BigInt(value),
        JsonNode::Float(value) => Value::Float(value),
        JsonNode::String(value) => Value::Str(value),
        JsonNode::Array(values) => heap.alloc_list(
            values
                .into_iter()
                .map(|value| json_node_to_value(value, heap))
                .collect(),
        ),
        JsonNode::Object(entries) => heap.alloc_dict(
            entries
                .into_iter()
                .map(|(key, value)| (Value::Str(key), json_node_to_value(value, heap)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escape_string_ensure_ascii_uses_lowercase_hex() {
        let escaped = json_escape_string("A☺😊", true);
        assert_eq!(escaped, "\"A\\u263a\\ud83d\\ude0a\"");
    }

    #[test]
    fn json_source_text_accepts_utf8_bytes_and_bytearray() {
        let heap = Heap::new();
        let bytes = heap.alloc_bytes(br#"{"ok":1}"#.to_vec());
        let bytearray = heap.alloc_bytearray(br#"{"ok":2}"#.to_vec());

        assert_eq!(json_source_text(&bytes).expect("bytes should decode"), "{\"ok\":1}");
        assert_eq!(
            json_source_text(&bytearray).expect("bytearray should decode"),
            "{\"ok\":2}"
        );
    }

    #[test]
    fn json_source_text_rejects_invalid_utf8_bytes() {
        let heap = Heap::new();
        let bytes = heap.alloc_bytes(vec![0xFF, 0xFE, 0x00]);

        let err = json_source_text(&bytes).expect_err("invalid bytes should fail");
        assert!(err.message.contains("valid UTF-8"));
    }

    #[test]
    fn parse_json_separators_requires_two_strings() {
        let ok = parse_json_separators(Value::Tuple(
            Heap::new().alloc(Object::Tuple(vec![
                Value::Str(",".to_string()),
                Value::Str(":".to_string()),
            ])),
        ))
        .expect("tuple separators should parse");
        assert_eq!(ok, (",".to_string(), ":".to_string()));

        let err = parse_json_separators(Value::Tuple(
            Heap::new().alloc(Object::Tuple(vec![Value::Str(",".to_string())])),
        ))
        .expect_err("one element should fail");
        assert!(err.message.contains("2-item tuple"));

        let err = parse_json_separators(Value::Tuple(Heap::new().alloc(Object::Tuple(vec![
            Value::Int(1),
            Value::Str(":".to_string()),
        ]))))
        .expect_err("non-string separator should fail");
        assert!(err.message.contains("must be strings"));
    }

    #[test]
    fn parse_json_node_from_index_respects_offsets() {
        let source = "  [1,2] {\"a\":3}";
        let (array_node, array_end) = parse_json_node_from_index(source, 2).expect("array parse");
        let (object_node, object_end) =
            parse_json_node_from_index(source, 8).expect("object parse");

        match array_node {
            JsonNode::Array(values) => assert_eq!(values.len(), 2),
            other => panic!("expected array node, got {other:?}"),
        }
        assert_eq!(array_end, 7);

        match object_node {
            JsonNode::Object(values) => {
                assert_eq!(values.len(), 1);
                assert_eq!(values[0].0, "a");
            }
            other => panic!("expected object node, got {other:?}"),
        }
        assert_eq!(object_end, 15);
    }

    #[test]
    fn parse_json_node_decodes_surrogate_pair_escape() {
        let node = parse_json_node("\"\\ud83d\\ude0a\"").expect("surrogate pair should decode");
        match node {
            JsonNode::String(value) => assert_eq!(value, "😊"),
            other => panic!("expected string node, got {other:?}"),
        }
    }
}
