use super::super::{
    ExceptionObject, HashMap, InstanceObject, InternalCallOutcome, ObjRef, Object, RuntimeError,
    Value, Vm, is_truthy,
};

#[derive(Clone)]
pub(in crate::vm) struct ExpatParserState {
    buffer: String,
    reparse_deferral_enabled: bool,
}

#[derive(Debug)]
struct ExpatSyntaxError {
    message: String,
    code: i64,
    lineno: i64,
    offset: i64,
}

enum ExpatParseFailure {
    Syntax(ExpatSyntaxError),
    Callback(RuntimeError),
}

impl Vm {
    fn pyexpat_parser_state_mut(
        &mut self,
        parser: &ObjRef,
    ) -> Result<&mut ExpatParserState, RuntimeError> {
        self.expat_parsers
            .get_mut(&parser.id())
            .ok_or_else(|| RuntimeError::type_error("invalid pyexpat parser"))
    }

    fn pyexpat_set_error_position(&mut self, parser: &ObjRef, lineno: i64, offset: i64) {
        if let Object::Instance(instance_data) = &mut *parser.kind_mut() {
            instance_data
                .attrs
                .insert("ErrorLineNumber".to_string(), Value::Int(lineno));
            instance_data
                .attrs
                .insert("ErrorColumnNumber".to_string(), Value::Int(offset));
        }
    }

    fn pyexpat_raise_error(
        &mut self,
        parser: &ObjRef,
        syntax_error: ExpatSyntaxError,
    ) -> Result<RuntimeError, RuntimeError> {
        self.pyexpat_set_error_position(parser, syntax_error.lineno, syntax_error.offset);
        let exception = ExceptionObject::new("ExpatError".to_string(), Some(syntax_error.message));
        exception
            .attrs
            .borrow_mut()
            .insert("code".to_string(), Value::Int(syntax_error.code));
        exception
            .attrs
            .borrow_mut()
            .insert("lineno".to_string(), Value::Int(syntax_error.lineno));
        exception
            .attrs
            .borrow_mut()
            .insert("offset".to_string(), Value::Int(syntax_error.offset));
        self.raise_exception(Value::Exception(Box::new(exception)))?;
        Ok(self.runtime_error_from_active_exception("pyexpat parser error"))
    }

    fn pyexpat_error_position(text: &str, byte_index: usize) -> (i64, i64) {
        let safe_index = byte_index.min(text.len());
        let head = &text[..safe_index];
        let lineno = head.chars().filter(|ch| *ch == '\n').count() as i64 + 1;
        let offset = match head.rfind('\n') {
            Some(last_nl) => head[last_nl + 1..].chars().count() as i64,
            None => head.chars().count() as i64,
        };
        (lineno, offset)
    }

    fn pyexpat_syntax_error(text: &str, byte_index: usize, message: &str) -> ExpatSyntaxError {
        let (lineno, offset) = Self::pyexpat_error_position(text, byte_index);
        let code = match message {
            "unclosed element" => 3,
            _ => 1,
        };
        ExpatSyntaxError {
            message: message.to_string(),
            code,
            lineno,
            offset,
        }
    }

    fn pyexpat_parser_attr(parser: &ObjRef, name: &str) -> Option<Value> {
        match &*parser.kind() {
            Object::Instance(instance_data) => instance_data.attrs.get(name).cloned(),
            _ => None,
        }
    }

    fn pyexpat_call_handler(
        &mut self,
        parser: &ObjRef,
        attr_name: &str,
        args: Vec<Value>,
    ) -> Result<(), RuntimeError> {
        let Some(handler) = Self::pyexpat_parser_attr(parser, attr_name) else {
            return Ok(());
        };
        if matches!(handler, Value::None) {
            return Ok(());
        }
        match self.call_internal(handler, args, HashMap::new()) {
            Ok(InternalCallOutcome::Value(_)) => Ok(()),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                Err(self.runtime_error_from_active_exception("pyexpat handler failed"))
            }
            Err(err) => Err(err),
        }
    }

    fn pyexpat_emit_character_data(
        &mut self,
        parser: &ObjRef,
        data: &str,
    ) -> Result<(), RuntimeError> {
        if data.is_empty() {
            return Ok(());
        }
        self.pyexpat_call_handler(
            parser,
            "CharacterDataHandler",
            vec![Value::Str(data.to_string())],
        )
    }

    fn pyexpat_emit_start(
        &mut self,
        parser: &ObjRef,
        tag: String,
        attrs: Vec<(String, String)>,
    ) -> Result<(), RuntimeError> {
        let ordered = Self::pyexpat_parser_attr(parser, "ordered_attributes")
            .map(|value| is_truthy(&value))
            .unwrap_or(false);
        let attrs_value = if ordered {
            let mut ordered_values = Vec::with_capacity(attrs.len() * 2);
            for (key, value) in attrs {
                ordered_values.push(Value::Str(key));
                ordered_values.push(Value::Str(value));
            }
            self.heap.alloc_list(ordered_values)
        } else {
            let dict_entries = attrs
                .into_iter()
                .map(|(key, value)| (Value::Str(key), Value::Str(value)))
                .collect();
            self.heap.alloc_dict(dict_entries)
        };
        self.pyexpat_call_handler(
            parser,
            "StartElementHandler",
            vec![Value::Str(tag), attrs_value],
        )
    }

    fn pyexpat_emit_end(&mut self, parser: &ObjRef, tag: String) -> Result<(), RuntimeError> {
        self.pyexpat_call_handler(parser, "EndElementHandler", vec![Value::Str(tag)])
    }

    fn pyexpat_emit_comment(&mut self, parser: &ObjRef, text: String) -> Result<(), RuntimeError> {
        self.pyexpat_call_handler(parser, "CommentHandler", vec![Value::Str(text)])
    }

    fn pyexpat_emit_pi(
        &mut self,
        parser: &ObjRef,
        target: String,
        data: String,
    ) -> Result<(), RuntimeError> {
        self.pyexpat_call_handler(
            parser,
            "ProcessingInstructionHandler",
            vec![Value::Str(target), Value::Str(data)],
        )
    }

    fn pyexpat_emit_default(&mut self, parser: &ObjRef, text: String) -> Result<(), RuntimeError> {
        self.pyexpat_call_handler(parser, "DefaultHandlerExpand", vec![Value::Str(text)])
    }

    fn pyexpat_parse_name(input: &str, index: usize) -> (String, usize) {
        let mut end = index;
        while end < input.len() {
            let byte = input.as_bytes()[end];
            if byte.is_ascii_whitespace() || byte == b'=' || byte == b'/' || byte == b'>' {
                break;
            }
            end += 1;
        }
        if end == index {
            return (String::new(), index);
        }
        (input[index..end].to_string(), end)
    }

    fn pyexpat_parse_start_tag(
        segment: &str,
        source_text: &str,
        error_index: usize,
    ) -> Result<(String, Vec<(String, String)>), ExpatSyntaxError> {
        let mut index = 0usize;
        while index < segment.len() && segment.as_bytes()[index].is_ascii_whitespace() {
            index += 1;
        }
        let (tag, next_index) = Self::pyexpat_parse_name(segment, index);
        if tag.is_empty() {
            return Err(Self::pyexpat_syntax_error(
                source_text,
                error_index,
                "missing element name",
            ));
        }
        index = next_index;
        let mut attrs = Vec::new();
        while index < segment.len() {
            while index < segment.len() && segment.as_bytes()[index].is_ascii_whitespace() {
                index += 1;
            }
            if index >= segment.len() {
                break;
            }
            let (name, name_end) = Self::pyexpat_parse_name(segment, index);
            if name.is_empty() {
                return Err(Self::pyexpat_syntax_error(
                    source_text,
                    error_index + index,
                    "invalid attribute name",
                ));
            }
            index = name_end;
            while index < segment.len() && segment.as_bytes()[index].is_ascii_whitespace() {
                index += 1;
            }
            if index >= segment.len() || segment.as_bytes()[index] != b'=' {
                return Err(Self::pyexpat_syntax_error(
                    source_text,
                    error_index + index,
                    "expected '=' after attribute name",
                ));
            }
            index += 1;
            while index < segment.len() && segment.as_bytes()[index].is_ascii_whitespace() {
                index += 1;
            }
            if index >= segment.len() {
                return Err(Self::pyexpat_syntax_error(
                    source_text,
                    error_index + index,
                    "missing attribute value",
                ));
            }
            let quote = segment.as_bytes()[index];
            if quote != b'"' && quote != b'\'' {
                return Err(Self::pyexpat_syntax_error(
                    source_text,
                    error_index + index,
                    "attribute values must be quoted",
                ));
            }
            index += 1;
            let value_start = index;
            while index < segment.len() && segment.as_bytes()[index] != quote {
                index += 1;
            }
            if index >= segment.len() {
                return Err(Self::pyexpat_syntax_error(
                    source_text,
                    error_index + value_start,
                    "unterminated attribute value",
                ));
            }
            let value = segment[value_start..index].to_string();
            index += 1;
            attrs.push((name, value));
        }
        Ok((tag, attrs))
    }

    fn pyexpat_parse_document(
        &mut self,
        parser: &ObjRef,
        text: &str,
    ) -> Result<(), ExpatParseFailure> {
        let mut index = 0usize;
        let mut stack: Vec<String> = Vec::new();

        while index < text.len() {
            if text.as_bytes()[index] != b'<' {
                let next = text[index..]
                    .find('<')
                    .map(|offset| index + offset)
                    .unwrap_or(text.len());
                if let Err(err) = self.pyexpat_emit_character_data(parser, &text[index..next]) {
                    return Err(ExpatParseFailure::Callback(err));
                }
                index = next;
                continue;
            }

            if text[index..].starts_with("<!--") {
                let Some(end_rel) = text[index + 4..].find("-->") else {
                    return Err(ExpatParseFailure::Syntax(Self::pyexpat_syntax_error(
                        text,
                        index,
                        "unclosed comment",
                    )));
                };
                let end = index + 4 + end_rel;
                if let Err(err) =
                    self.pyexpat_emit_comment(parser, text[index + 4..end].to_string())
                {
                    return Err(ExpatParseFailure::Callback(err));
                }
                index = end + 3;
                continue;
            }

            if text[index..].starts_with("<?") {
                let Some(end_rel) = text[index + 2..].find("?>") else {
                    return Err(ExpatParseFailure::Syntax(Self::pyexpat_syntax_error(
                        text,
                        index,
                        "unclosed processing instruction",
                    )));
                };
                let end = index + 2 + end_rel;
                let body = text[index + 2..end].trim();
                if !body.is_empty() {
                    let mut parts = body.splitn(2, char::is_whitespace);
                    let target = parts.next().unwrap_or_default().to_string();
                    let data = parts.next().unwrap_or("").trim().to_string();
                    if let Err(err) = self.pyexpat_emit_pi(parser, target, data) {
                        return Err(ExpatParseFailure::Callback(err));
                    }
                }
                index = end + 2;
                continue;
            }

            if text[index..].starts_with("<![CDATA[") {
                let Some(end_rel) = text[index + 9..].find("]]>") else {
                    return Err(ExpatParseFailure::Syntax(Self::pyexpat_syntax_error(
                        text,
                        index,
                        "unclosed CDATA section",
                    )));
                };
                let end = index + 9 + end_rel;
                if let Err(err) = self.pyexpat_emit_character_data(parser, &text[index + 9..end]) {
                    return Err(ExpatParseFailure::Callback(err));
                }
                index = end + 3;
                continue;
            }

            if text[index..].starts_with("<!") {
                let Some(end_rel) = text[index + 2..].find('>') else {
                    return Err(ExpatParseFailure::Syntax(Self::pyexpat_syntax_error(
                        text,
                        index,
                        "unclosed declaration",
                    )));
                };
                let end = index + 2 + end_rel;
                if let Err(err) = self.pyexpat_emit_default(parser, text[index..=end].to_string()) {
                    return Err(ExpatParseFailure::Callback(err));
                }
                index = end + 1;
                continue;
            }

            if text[index..].starts_with("</") {
                let Some(end_rel) = text[index + 2..].find('>') else {
                    return Err(ExpatParseFailure::Syntax(Self::pyexpat_syntax_error(
                        text,
                        index,
                        "unclosed end tag",
                    )));
                };
                let end = index + 2 + end_rel;
                let tag = text[index + 2..end].trim();
                if tag.is_empty() {
                    return Err(ExpatParseFailure::Syntax(Self::pyexpat_syntax_error(
                        text,
                        index,
                        "missing end tag name",
                    )));
                }
                if let Some(expected) = stack.pop() {
                    if expected != tag {
                        return Err(ExpatParseFailure::Syntax(Self::pyexpat_syntax_error(
                            text,
                            index,
                            "mismatched end tag",
                        )));
                    }
                } else {
                    return Err(ExpatParseFailure::Syntax(Self::pyexpat_syntax_error(
                        text,
                        index,
                        "unexpected end tag",
                    )));
                }
                if let Err(err) = self.pyexpat_emit_end(parser, tag.to_string()) {
                    return Err(ExpatParseFailure::Callback(err));
                }
                index = end + 1;
                continue;
            }

            let Some(end_rel) = text[index + 1..].find('>') else {
                return Err(ExpatParseFailure::Syntax(Self::pyexpat_syntax_error(
                    text,
                    index,
                    "unclosed start tag",
                )));
            };
            let end = index + 1 + end_rel;
            let mut segment = text[index + 1..end].trim().to_string();
            let self_closing = segment.ends_with('/');
            if self_closing {
                segment.pop();
                segment = segment.trim_end().to_string();
            }
            let (tag, attrs) = match Self::pyexpat_parse_start_tag(&segment, text, index + 1) {
                Ok(parsed) => parsed,
                Err(err) => return Err(ExpatParseFailure::Syntax(err)),
            };
            if let Err(err) = self.pyexpat_emit_start(parser, tag.clone(), attrs) {
                return Err(ExpatParseFailure::Callback(err));
            }
            if self_closing {
                if let Err(err) = self.pyexpat_emit_end(parser, tag) {
                    return Err(ExpatParseFailure::Callback(err));
                }
            } else {
                stack.push(tag);
            }
            index = end + 1;
        }

        if !stack.is_empty() {
            return Err(ExpatParseFailure::Syntax(Self::pyexpat_syntax_error(
                text,
                text.len(),
                "unclosed element",
            )));
        }
        Ok(())
    }

    fn pyexpat_parser_class(&self) -> Option<ObjRef> {
        let module = self.modules.get("pyexpat")?.clone();
        let module_kind = module.kind();
        let Object::Module(module_data) = &*module_kind else {
            return None;
        };
        match module_data.globals.get("xmlparser") {
            Some(Value::Class(class)) => Some(class.clone()),
            _ => None,
        }
    }

    pub(in crate::vm) fn builtin_pyexpat_parser_create(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new(format!(
                "TypeError: ParserCreate() takes at most 2 positional arguments ({} given)",
                args.len()
            )));
        }
        let encoding = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("encoding")
        };
        let namespace_separator = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("namespace_separator")
        };
        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: ParserCreate() got an unexpected keyword argument '{key}'",
            )));
        }

        let class = self
            .pyexpat_parser_class()
            .ok_or_else(|| RuntimeError::type_error("pyexpat parser class unavailable"))?;
        let instance = match self.heap.alloc_instance(InstanceObject::new(class)) {
            Value::Instance(instance) => instance,
            _ => unreachable!(),
        };
        let encoding_value = match encoding {
            Some(Value::Str(value)) => Value::Str(value),
            Some(Value::None) | None => Value::None,
            Some(_) => return Err(RuntimeError::type_error("encoding must be str or None")),
        };
        let ns_value = match namespace_separator {
            Some(Value::Str(value)) => Value::Str(value),
            Some(Value::None) | None => Value::None,
            Some(_) => {
                return Err(RuntimeError::new(
                    "TypeError: namespace_separator must be str or None",
                ));
            }
        };
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data
                .attrs
                .insert("encoding".to_string(), encoding_value);
            instance_data
                .attrs
                .insert("namespace_separator".to_string(), ns_value);
            instance_data
                .attrs
                .insert("buffer_text".to_string(), Value::Int(0));
            instance_data
                .attrs
                .insert("ordered_attributes".to_string(), Value::Int(0));
            instance_data
                .attrs
                .insert("ErrorLineNumber".to_string(), Value::Int(1));
            instance_data
                .attrs
                .insert("ErrorColumnNumber".to_string(), Value::Int(0));
            for handler_name in [
                "StartElementHandler",
                "EndElementHandler",
                "StartNamespaceDeclHandler",
                "EndNamespaceDeclHandler",
                "CharacterDataHandler",
                "CommentHandler",
                "ProcessingInstructionHandler",
                "DefaultHandlerExpand",
            ] {
                instance_data
                    .attrs
                    .insert(handler_name.to_string(), Value::None);
            }
        }
        self.expat_parsers.insert(
            instance.id(),
            ExpatParserState {
                buffer: String::new(),
                reparse_deferral_enabled: true,
            },
        );
        Ok(Value::Instance(instance))
    }

    pub(in crate::vm) fn builtin_pyexpat_parser_parse(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new(
                "TypeError: Parse() expects data and optional isfinal",
            ));
        }
        let parser = self.take_bound_instance_arg(&mut args, "pyexpat.Parse")?;
        let data = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs
                .remove("data")
                .ok_or_else(|| RuntimeError::type_error("Parse() missing data argument"))?
        };
        let isfinal = if !args.is_empty() {
            is_truthy(&args.remove(0))
        } else {
            kwargs
                .remove("isfinal")
                .map(|value| is_truthy(&value))
                .unwrap_or(false)
        };
        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: Parse() got an unexpected keyword argument '{key}'",
            )));
        }
        let chunk = match data {
            Value::Str(text) => text,
            Value::Bytes(bytes) => match &*bytes.kind() {
                Object::Bytes(payload) => String::from_utf8(payload.clone())
                    .map_err(|_| RuntimeError::unicode_decode_error("invalid UTF-8"))?,
                _ => return Err(RuntimeError::type_error("invalid bytes value")),
            },
            Value::ByteArray(bytes) => match &*bytes.kind() {
                Object::ByteArray(payload) => String::from_utf8(payload.clone())
                    .map_err(|_| RuntimeError::unicode_decode_error("invalid UTF-8"))?,
                _ => return Err(RuntimeError::type_error("invalid bytearray value")),
            },
            _ => {
                return Err(RuntimeError::new(
                    "TypeError: Parse() data must be str or bytes",
                ));
            }
        };

        {
            let state = self.pyexpat_parser_state_mut(&parser)?;
            if !chunk.is_empty() {
                state.buffer.push_str(&chunk);
            }
            if !isfinal {
                return Ok(Value::Int(1));
            }
        }

        let text = {
            let state = self.pyexpat_parser_state_mut(&parser)?;
            let text = state.buffer.clone();
            state.buffer.clear();
            text
        };
        if text.is_empty() {
            return Ok(Value::Int(1));
        }
        match self.pyexpat_parse_document(&parser, &text) {
            Ok(()) => Ok(Value::Int(1)),
            Err(ExpatParseFailure::Callback(err)) => Err(err),
            Err(ExpatParseFailure::Syntax(err)) => Err(self.pyexpat_raise_error(&parser, err)?),
        }
    }

    pub(in crate::vm) fn builtin_pyexpat_parser_get_reparse_deferral_enabled(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "TypeError: GetReparseDeferralEnabled() expects no arguments",
            ));
        }
        let parser =
            self.take_bound_instance_arg(&mut args, "pyexpat.GetReparseDeferralEnabled")?;
        let state = self.pyexpat_parser_state_mut(&parser)?;
        Ok(Value::Bool(state.reparse_deferral_enabled))
    }

    pub(in crate::vm) fn builtin_pyexpat_parser_set_reparse_deferral_enabled(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "TypeError: SetReparseDeferralEnabled() expects one argument",
            ));
        }
        let parser =
            self.take_bound_instance_arg(&mut args, "pyexpat.SetReparseDeferralEnabled")?;
        let enabled = is_truthy(&args.remove(0));
        let state = self.pyexpat_parser_state_mut(&parser)?;
        state.reparse_deferral_enabled = enabled;
        Ok(Value::None)
    }
}
