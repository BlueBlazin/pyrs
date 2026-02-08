use super::super::*;

impl Vm {
    pub(in crate::vm) fn builtin_csv_reader(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("reader() expects iterable argument"));
        }
        let source = args.remove(0);
        let dialect_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("dialect")
        };
        let delimiter = self.extract_csv_delimiter(dialect_arg.clone(), &mut kwargs)?;
        let quotechar = self.extract_csv_quotechar(dialect_arg.clone(), &mut kwargs)?;
        let escapechar = self.extract_csv_escapechar(dialect_arg.clone(), &mut kwargs)?;
        let skipinitialspace =
            self.extract_csv_skipinitialspace(dialect_arg.clone(), &mut kwargs)?;
        let mut quoting = self.extract_csv_quoting(dialect_arg.clone(), &mut kwargs)?;
        let doublequote = self.extract_csv_doublequote(dialect_arg.clone(), &mut kwargs)?;
        let lineterminator = self.extract_csv_lineterminator(dialect_arg.clone(), &mut kwargs)?;
        let strict = self.extract_csv_strict(dialect_arg, &mut kwargs)?;
        if quotechar.is_none() && quoting == 0 {
            quoting = 3;
        }
        validate_csv_parameter_consistency(
            delimiter,
            quotechar,
            escapechar,
            skipinitialspace,
            &lineterminator,
            quoting,
        )?;
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "reader() got unexpected keyword arguments",
            ));
        }
        let iterator = match self.to_iterator_value(source) {
            Ok(iterator) => iterator,
            Err(err) if classify_runtime_error(&err.message) == "TypeError" => {
                return Err(RuntimeError::new("expected iterable"));
            }
            Err(err) => return Err(err),
        };
        let reader_class = self.csv_reader_class();
        let reader = self.alloc_instance_for_class(&reader_class);
        if let Object::Instance(instance_data) = &mut *reader.kind_mut() {
            instance_data.attrs.insert("_iter".to_string(), iterator);
            instance_data
                .attrs
                .insert("_pending_record".to_string(), Value::Str(String::new()));
            instance_data
                .attrs
                .insert("_physical_line_num".to_string(), Value::Int(0));
            instance_data
                .attrs
                .insert("line_num".to_string(), Value::Int(0));
            instance_data.attrs.insert(
                "__csv_delimiter__".to_string(),
                Value::Str(delimiter.to_string()),
            );
            instance_data.attrs.insert(
                "__csv_quotechar__".to_string(),
                match quotechar {
                    Some(ch) => Value::Str(ch.to_string()),
                    None => Value::None,
                },
            );
            instance_data.attrs.insert(
                "__csv_escapechar__".to_string(),
                match escapechar {
                    Some(ch) => Value::Str(ch.to_string()),
                    None => Value::None,
                },
            );
            instance_data
                .attrs
                .insert("__csv_quoting__".to_string(), Value::Int(quoting));
            instance_data
                .attrs
                .insert("__csv_doublequote__".to_string(), Value::Bool(doublequote));
            instance_data.attrs.insert(
                "__csv_skipinitialspace__".to_string(),
                Value::Bool(skipinitialspace),
            );
            instance_data
                .attrs
                .insert("__csv_strict__".to_string(), Value::Bool(strict));
            instance_data.attrs.insert(
                "__csv_field_limit__".to_string(),
                Value::Int(self.csv_field_size_limit),
            );
            let dialect = self.build_csv_dialect_module(
                delimiter,
                quotechar,
                escapechar,
                doublequote,
                skipinitialspace,
                lineterminator,
                quoting,
                strict,
            );
            instance_data.attrs.insert("dialect".to_string(), dialect);
        }
        Ok(Value::Instance(reader))
    }

    pub(in crate::vm) fn csv_reader_class(&mut self) -> ObjRef {
        let class = self.alloc_synthetic_class("__csv_reader__");
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data.attrs.insert(
                "__iter__".to_string(),
                Value::Builtin(BuiltinFunction::CsvReaderIter),
            );
            class_data.attrs.insert(
                "__next__".to_string(),
                Value::Builtin(BuiltinFunction::CsvReaderNext),
            );
        }
        class
    }

    pub(in crate::vm) fn build_csv_dialect_module(
        &mut self,
        delimiter: char,
        quotechar: Option<char>,
        escapechar: Option<char>,
        doublequote: bool,
        skipinitialspace: bool,
        lineterminator: String,
        quoting: i64,
        strict: bool,
    ) -> Value {
        let class = self.alloc_synthetic_class("__csv_dialect__");
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data.attrs.insert(
                "__reduce_ex__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectReduceEx),
            );
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_csv".to_string()));
        }
        let dialect = self.alloc_instance_for_class(&class);
        if let Object::Instance(instance_data) = &mut *dialect.kind_mut() {
            instance_data
                .attrs
                .insert("delimiter".to_string(), Value::Str(delimiter.to_string()));
            instance_data.attrs.insert(
                "quotechar".to_string(),
                match quotechar {
                    Some(ch) => Value::Str(ch.to_string()),
                    None => Value::None,
                },
            );
            instance_data.attrs.insert(
                "escapechar".to_string(),
                match escapechar {
                    Some(ch) => Value::Str(ch.to_string()),
                    None => Value::None,
                },
            );
            instance_data
                .attrs
                .insert("doublequote".to_string(), Value::Bool(doublequote));
            instance_data.attrs.insert(
                "skipinitialspace".to_string(),
                Value::Bool(skipinitialspace),
            );
            instance_data
                .attrs
                .insert("lineterminator".to_string(), Value::Str(lineterminator));
            instance_data
                .attrs
                .insert("quoting".to_string(), Value::Int(quoting));
            instance_data
                .attrs
                .insert("strict".to_string(), Value::Bool(strict));
        }
        Value::Instance(dialect)
    }

    pub(in crate::vm) fn builtin_csv_reader_iter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "csv.reader.__iter__ expects no arguments",
            ));
        }
        Ok(args.remove(0))
    }

    pub(in crate::vm) fn builtin_csv_reader_next(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "csv.reader.__next__ expects no arguments",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let (
            iterator,
            mut pending_record,
            mut physical_line_num,
            delimiter,
            quotechar,
            escapechar,
            quoting,
            doublequote,
            skipinitialspace,
            strict,
            field_limit,
        ) = match &*receiver.kind() {
            Object::Instance(instance_data) => {
                let iterator = instance_data
                    .attrs
                    .get("_iter")
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("csv.reader missing iterator"))?;
                let pending_record = match instance_data.attrs.get("_pending_record") {
                    Some(Value::Str(value)) => value.clone(),
                    _ => String::new(),
                };
                let physical_line_num = match instance_data.attrs.get("_physical_line_num") {
                    Some(Value::Int(value)) => *value,
                    _ => 0,
                };
                let delimiter = instance_data
                    .attrs
                    .get("__csv_delimiter__")
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("csv.reader missing delimiter"))
                    .and_then(|value| csv_char_from_value(value, "delimiter"))?;
                let quotechar = instance_data
                    .attrs
                    .get("__csv_quotechar__")
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("csv.reader missing quotechar"))
                    .and_then(|value| csv_optional_char_from_value(value, "quotechar"))?;
                let escapechar = instance_data
                    .attrs
                    .get("__csv_escapechar__")
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("csv.reader missing escapechar"))
                    .and_then(|value| csv_optional_char_from_value(value, "escapechar"))?;
                let quoting = instance_data
                    .attrs
                    .get("__csv_quoting__")
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("csv.reader missing quoting"))
                    .and_then(value_to_int)?;
                let doublequote = instance_data
                    .attrs
                    .get("__csv_doublequote__")
                    .cloned()
                    .map(|value| is_truthy(&value))
                    .unwrap_or(true);
                let skipinitialspace = instance_data
                    .attrs
                    .get("__csv_skipinitialspace__")
                    .cloned()
                    .map(|value| is_truthy(&value))
                    .unwrap_or(false);
                let strict = instance_data
                    .attrs
                    .get("__csv_strict__")
                    .cloned()
                    .map(|value| is_truthy(&value))
                    .unwrap_or(false);
                let field_limit = match instance_data.attrs.get("__csv_field_limit__") {
                    Some(Value::Int(limit)) if *limit >= 0 => Some(*limit as usize),
                    _ => None,
                };
                (
                    iterator,
                    pending_record,
                    physical_line_num,
                    delimiter,
                    quotechar,
                    escapechar,
                    quoting,
                    doublequote,
                    skipinitialspace,
                    strict,
                    field_limit,
                )
            }
            _ => return Err(RuntimeError::new("csv.reader.__next__ receiver invalid")),
        };

        loop {
            match self.next_from_iterator_value(&iterator)? {
                GeneratorResumeOutcome::Yield(value) => {
                    physical_line_num += 1;
                    let mut text = self.coerce_csv_reader_text(value)?;
                    if text.ends_with('\n') {
                        text.pop();
                        if text.ends_with('\r') {
                            text.pop();
                        }
                    } else if text.ends_with('\r') {
                        text.pop();
                    }
                    if !pending_record.is_empty() {
                        pending_record.push('\n');
                    }
                    pending_record.push_str(&text);
                    if csv_record_needs_more_data(
                        &pending_record,
                        delimiter,
                        quotechar,
                        escapechar,
                        skipinitialspace,
                        quoting,
                        doublequote,
                    ) {
                        continue;
                    }
                    let fields = parse_csv_row_simple(
                        &pending_record,
                        delimiter,
                        quotechar,
                        escapechar,
                        skipinitialspace,
                        quoting,
                        doublequote,
                        strict,
                        field_limit,
                    )?
                    .into_iter()
                    .map(|field| csv_reader_value_for_field(field, quoting))
                    .collect::<Result<Vec<_>, _>>()?;
                    pending_record.clear();
                    if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
                        instance_data.attrs.insert(
                            "_pending_record".to_string(),
                            Value::Str(pending_record.clone()),
                        );
                        instance_data.attrs.insert(
                            "_physical_line_num".to_string(),
                            Value::Int(physical_line_num),
                        );
                        instance_data
                            .attrs
                            .insert("line_num".to_string(), Value::Int(physical_line_num));
                    }
                    return Ok(self.heap.alloc_list(fields));
                }
                GeneratorResumeOutcome::Complete(_) => {
                    if pending_record.is_empty() {
                        if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
                            instance_data
                                .attrs
                                .insert("_pending_record".to_string(), Value::Str(String::new()));
                            instance_data.attrs.insert(
                                "_physical_line_num".to_string(),
                                Value::Int(physical_line_num),
                            );
                            instance_data
                                .attrs
                                .insert("line_num".to_string(), Value::Int(physical_line_num));
                        }
                        return Err(RuntimeError::new("StopIteration"));
                    }
                    let fields = parse_csv_row_simple(
                        &pending_record,
                        delimiter,
                        quotechar,
                        escapechar,
                        skipinitialspace,
                        quoting,
                        doublequote,
                        strict,
                        field_limit,
                    )?
                    .into_iter()
                    .map(|field| csv_reader_value_for_field(field, quoting))
                    .collect::<Result<Vec<_>, _>>()?;
                    if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
                        instance_data
                            .attrs
                            .insert("_pending_record".to_string(), Value::Str(String::new()));
                        instance_data.attrs.insert(
                            "_physical_line_num".to_string(),
                            Value::Int(physical_line_num),
                        );
                        instance_data
                            .attrs
                            .insert("line_num".to_string(), Value::Int(physical_line_num));
                    }
                    return Ok(self.heap.alloc_list(fields));
                }
                GeneratorResumeOutcome::PropagatedException => {
                    if self.pending_generator_exception.is_some() {
                        self.propagate_pending_generator_exception()?;
                    }
                    if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
                        instance_data.attrs.insert(
                            "_pending_record".to_string(),
                            Value::Str(pending_record.clone()),
                        );
                        instance_data.attrs.insert(
                            "_physical_line_num".to_string(),
                            Value::Int(physical_line_num),
                        );
                        instance_data
                            .attrs
                            .insert("line_num".to_string(), Value::Int(physical_line_num));
                    }
                    return Err(self.runtime_error_from_active_exception("iteration failed"));
                }
            }
        }
    }

    pub(in crate::vm) fn builtin_csv_writer(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("writer() expects file-like argument"));
        }
        let target = args.remove(0);
        let write_callable = match self.builtin_getattr(
            vec![target.clone(), Value::Str("write".to_string())],
            HashMap::new(),
        ) {
            Ok(value) => value,
            Err(err) if classify_runtime_error(&err.message) == "OSError" => return Err(err),
            Err(_) => {
                return Err(RuntimeError::new(
                    "writer() argument must have a write method",
                ));
            }
        };
        if !self.is_callable_value(&write_callable) {
            return Err(RuntimeError::new(
                "writer() argument must have a write method",
            ));
        }
        let dialect_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("dialect")
        };
        let delimiter = self.extract_csv_delimiter(dialect_arg.clone(), &mut kwargs)?;
        let quotechar = self.extract_csv_quotechar(dialect_arg.clone(), &mut kwargs)?;
        let escapechar = self.extract_csv_escapechar(dialect_arg.clone(), &mut kwargs)?;
        let skipinitialspace =
            self.extract_csv_skipinitialspace(dialect_arg.clone(), &mut kwargs)?;
        let mut quoting = self.extract_csv_quoting(dialect_arg.clone(), &mut kwargs)?;
        let doublequote = self.extract_csv_doublequote(dialect_arg.clone(), &mut kwargs)?;
        let lineterminator = self.extract_csv_lineterminator(dialect_arg.clone(), &mut kwargs)?;
        let strict = self.extract_csv_strict(dialect_arg, &mut kwargs)?;
        if quotechar.is_none() && quoting == 0 {
            quoting = 3;
        }
        validate_csv_parameter_consistency(
            delimiter,
            quotechar,
            escapechar,
            skipinitialspace,
            &lineterminator,
            quoting,
        )?;
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "writer() got unexpected keyword arguments",
            ));
        }

        let writer = match self.heap.alloc_module(ModuleObject::new("__csv_writer__")) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(writer_data) = &mut *writer.kind_mut() {
            writer_data
                .globals
                .insert("__csv_target__".to_string(), target.clone());
            writer_data.globals.insert(
                "__csv_delimiter__".to_string(),
                Value::Str(delimiter.to_string()),
            );
            writer_data.globals.insert(
                "__csv_quotechar__".to_string(),
                match quotechar {
                    Some(ch) => Value::Str(ch.to_string()),
                    None => Value::None,
                },
            );
            writer_data.globals.insert(
                "__csv_escapechar__".to_string(),
                match escapechar {
                    Some(ch) => Value::Str(ch.to_string()),
                    None => Value::None,
                },
            );
            writer_data
                .globals
                .insert("__csv_quoting__".to_string(), Value::Int(quoting));
            writer_data
                .globals
                .insert("__csv_doublequote__".to_string(), Value::Bool(doublequote));
            writer_data.globals.insert(
                "__csv_lineterminator__".to_string(),
                Value::Str(lineterminator.clone()),
            );
            writer_data.globals.insert(
                "__csv_skipinitialspace__".to_string(),
                Value::Bool(skipinitialspace),
            );
            writer_data
                .globals
                .insert("__csv_strict__".to_string(), Value::Bool(strict));
            writer_data.globals.insert(
                "writerow".to_string(),
                self.alloc_native_bound_method(
                    NativeMethodKind::Builtin(BuiltinFunction::CsvWriterRow),
                    writer.clone(),
                ),
            );
            writer_data.globals.insert(
                "writerows".to_string(),
                self.alloc_native_bound_method(
                    NativeMethodKind::Builtin(BuiltinFunction::CsvWriterRows),
                    writer.clone(),
                ),
            );
            let dialect = self.build_csv_dialect_module(
                delimiter,
                quotechar,
                escapechar,
                doublequote,
                skipinitialspace,
                lineterminator,
                quoting,
                strict,
            );
            writer_data.globals.insert("dialect".to_string(), dialect);
        }
        Ok(Value::Module(writer))
    }

    pub(in crate::vm) fn builtin_csv_writerow(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("writerow() expects one row argument"));
        }
        let writer = match args.remove(0) {
            Value::Module(module) => module,
            _ => return Err(RuntimeError::new("writerow() receiver must be csv writer")),
        };
        let row = args.remove(0);
        let (
            target,
            delimiter,
            quotechar,
            escapechar,
            quoting,
            doublequote,
            lineterminator,
            skipinitialspace,
        ) = self.extract_csv_writer_state(&writer)?;
        let iterator = match self.to_iterator_value(row.clone()) {
            Ok(iterator) => iterator,
            Err(err) if classify_runtime_error(&err.message) == "TypeError" => {
                return Err(self.csv_non_iterable_error(&row));
            }
            Err(err) => return Err(err),
        };
        let mut fields: Vec<(String, bool)> = Vec::new();
        loop {
            match self.next_from_iterator_value(&iterator)? {
                GeneratorResumeOutcome::Yield(value) => {
                    fields.push(self.csv_writer_field_from_value(
                        value,
                        delimiter,
                        quotechar,
                        escapechar,
                        quoting,
                        doublequote,
                    )?);
                }
                GeneratorResumeOutcome::Complete(_) => break,
                GeneratorResumeOutcome::PropagatedException => {
                    if self.pending_generator_exception.is_some() {
                        self.propagate_pending_generator_exception()?;
                    }
                    return Err(self.runtime_error_from_active_exception("iteration failed"));
                }
            }
        }

        if fields.len() == 1 && fields[0].0.is_empty() && !fields[0].1 {
            if quoting == 3 || quoting == 4 || quoting == 5 {
                return Err(RuntimeError::new(
                    "single empty field record must be quoted",
                ));
            }
            if let Some(quote) = quotechar {
                fields[0] = (format!("{quote}{quote}"), true);
            }
        }
        if quoting == 3 && fields.len() == 1 && fields[0].0.is_empty() && !fields[0].1 {
            return Err(RuntimeError::new(
                "single empty field record must be quoted",
            ));
        }
        if delimiter == ' ' && skipinitialspace {
            for (text, quoted) in &fields {
                if text.is_empty() && !quoted {
                    return Err(RuntimeError::new(
                        "empty field must be quoted if delimiter is space and skipinitialspace is true",
                    ));
                }
            }
        }
        let mut line = fields
            .into_iter()
            .map(|(field, _)| field)
            .collect::<Vec<_>>()
            .join(&delimiter.to_string());
        line.push_str(&lineterminator);
        let writer_callable = self.builtin_getattr(
            vec![target, Value::Str("write".to_string())],
            HashMap::new(),
        )?;
        match self.call_internal(
            writer_callable,
            vec![Value::Str(line.clone())],
            HashMap::new(),
        )? {
            InternalCallOutcome::Value(_) => Ok(Value::Int(line.len() as i64)),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("writerow() write failed"))
            }
        }
    }

    pub(in crate::vm) fn builtin_csv_writerows(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("writerows() expects iterable argument"));
        }
        let writer = args.remove(0);
        let rows = args.remove(0);
        let iterator = match self.to_iterator_value(rows.clone()) {
            Ok(iterator) => iterator,
            Err(err) if classify_runtime_error(&err.message) == "TypeError" => {
                return Err(RuntimeError::new("expected iterable"));
            }
            Err(err) => return Err(err),
        };
        loop {
            match self.next_from_iterator_value(&iterator)? {
                GeneratorResumeOutcome::Yield(row) => {
                    self.builtin_csv_writerow(vec![writer.clone(), row], HashMap::new())?;
                }
                GeneratorResumeOutcome::Complete(_) => break,
                GeneratorResumeOutcome::PropagatedException => {
                    if self.pending_generator_exception.is_some() {
                        self.propagate_pending_generator_exception()?;
                    }
                    return Err(self.runtime_error_from_active_exception("iteration failed"));
                }
            }
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn runtime_error_from_active_exception(
        &mut self,
        fallback: &str,
    ) -> RuntimeError {
        let active = self
            .frames
            .last_mut()
            .and_then(|frame| frame.active_exception.take());
        match active {
            Some(value) => RuntimeError::new(format_value(&value)),
            None => RuntimeError::new(fallback),
        }
    }

    pub(in crate::vm) fn iteration_error_from_state(
        &mut self,
        fallback: &str,
    ) -> Result<RuntimeError, RuntimeError> {
        if self.pending_generator_exception.is_some() {
            self.propagate_pending_generator_exception()?;
        }
        Ok(self.runtime_error_from_active_exception(fallback))
    }

    pub(in crate::vm) fn csv_non_iterable_error(&self, value: &Value) -> RuntimeError {
        RuntimeError::new(format!("iterable expected, not {}", csv_type_name(value)))
    }

    pub(in crate::vm) fn csv_writer_field_from_value(
        &mut self,
        value: Value,
        delimiter: char,
        quotechar: Option<char>,
        escapechar: Option<char>,
        quoting: i64,
        doublequote: bool,
    ) -> Result<(String, bool), RuntimeError> {
        let is_none = matches!(&value, Value::None);
        let is_string = matches!(&value, Value::Str(_));
        let is_numeric = matches!(
            &value,
            Value::Int(_)
                | Value::BigInt(_)
                | Value::Float(_)
                | Value::Bool(_)
                | Value::Complex { .. }
        );
        let text = self.coerce_csv_writer_field_text(value)?;
        let force_quote = match quoting {
            1 => true,        // QUOTE_ALL
            2 => !is_numeric, // QUOTE_NONNUMERIC
            4 => is_string,   // QUOTE_STRINGS
            5 => !is_none,    // QUOTE_NOTNULL
            _ => false,
        };
        let rendered = quote_csv_field(
            &text,
            delimiter,
            quotechar,
            escapechar,
            quoting,
            doublequote,
            force_quote,
        )?;
        let was_quoted = force_quote || rendered != text;
        Ok((rendered, was_quoted))
    }

    pub(in crate::vm) fn builtin_csv_register_dialect(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("register_dialect() expects name"));
        }
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("register_dialect() name must be str")),
        };
        if args.len() > 1 {
            return Err(RuntimeError::new(
                "register_dialect() expects at most one dialect argument",
            ));
        }
        let base_dialect = args.first().cloned();
        let dialect = self.build_csv_registered_dialect(base_dialect, &mut kwargs)?;
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "register_dialect() got unexpected keyword argument",
            ));
        }
        self.csv_dialects.insert(name, dialect);
        Ok(Value::None)
    }

    pub(in crate::vm) fn csv_dialect_attr(
        &mut self,
        dialect: Option<&Value>,
        name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some(dialect) = dialect else {
            return Ok(None);
        };
        match self.builtin_getattr(
            vec![dialect.clone(), Value::Str(name.to_string())],
            HashMap::new(),
        ) {
            Ok(value) => Ok(Some(value)),
            Err(err) if classify_runtime_error(&err.message) == "AttributeError" => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(in crate::vm) fn build_csv_registered_dialect(
        &mut self,
        base_dialect: Option<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let base_ref = base_dialect.as_ref();
        let delimiter_value = kwargs
            .remove("delimiter")
            .or(self.csv_dialect_attr(base_ref, "delimiter")?)
            .unwrap_or_else(|| Value::Str(",".to_string()));
        let quotechar_value = kwargs
            .remove("quotechar")
            .or(self.csv_dialect_attr(base_ref, "quotechar")?)
            .unwrap_or_else(|| Value::Str("\"".to_string()));
        let escapechar_value = kwargs
            .remove("escapechar")
            .or(self.csv_dialect_attr(base_ref, "escapechar")?)
            .unwrap_or(Value::None);
        let doublequote_value = kwargs
            .remove("doublequote")
            .or(self.csv_dialect_attr(base_ref, "doublequote")?)
            .unwrap_or(Value::Bool(true));
        let skipinitialspace_value = kwargs
            .remove("skipinitialspace")
            .or(self.csv_dialect_attr(base_ref, "skipinitialspace")?)
            .unwrap_or(Value::Bool(false));
        let lineterminator_value = kwargs
            .remove("lineterminator")
            .or(self.csv_dialect_attr(base_ref, "lineterminator")?)
            .unwrap_or_else(|| Value::Str("\r\n".to_string()));
        let quoting_value = kwargs
            .remove("quoting")
            .or(self.csv_dialect_attr(base_ref, "quoting")?)
            .unwrap_or(Value::Int(0));
        let strict_value = kwargs
            .remove("strict")
            .or(self.csv_dialect_attr(base_ref, "strict")?)
            .unwrap_or(Value::Bool(false));

        let delimiter = csv_char_from_value(delimiter_value, "delimiter")?;
        let quotechar = csv_optional_char_from_value(quotechar_value, "quotechar")?;
        let escapechar = csv_optional_char_from_value(escapechar_value, "escapechar")?;
        let doublequote = value_to_bool_flag(doublequote_value, "doublequote")?;
        let skipinitialspace = value_to_bool_flag(skipinitialspace_value, "skipinitialspace")?;
        let lineterminator = match lineterminator_value {
            Value::Str(text) => text,
            other => {
                return Err(RuntimeError::new(format!(
                    "\"lineterminator\" must be a string, not {}",
                    csv_type_name(&other)
                )));
            }
        };
        let mut quoting = value_to_int(quoting_value)?;
        let strict = value_to_bool_flag(strict_value, "strict")?;
        if quotechar.is_none() && quoting == 0 {
            quoting = 3;
        }
        validate_csv_parameter_consistency(
            delimiter,
            quotechar,
            escapechar,
            skipinitialspace,
            &lineterminator,
            quoting,
        )?;

        Ok(self.build_csv_dialect_module(
            delimiter,
            quotechar,
            escapechar,
            doublequote,
            skipinitialspace,
            lineterminator,
            quoting,
            strict,
        ))
    }

    pub(in crate::vm) fn builtin_csv_unregister_dialect(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "unregister_dialect() expects one argument",
            ));
        }
        let name = match &args[0] {
            Value::Str(name) => name.clone(),
            _ => return Err(RuntimeError::new("unknown dialect")),
        };
        if self.csv_dialects.remove(&name).is_none() {
            return Err(RuntimeError::new("unknown dialect"));
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_csv_get_dialect(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("get_dialect() expects one argument"));
        }
        let name = match &args[0] {
            Value::Str(name) => name.clone(),
            _ => return Err(RuntimeError::new("unknown dialect")),
        };
        self.csv_dialects
            .get(&name)
            .cloned()
            .ok_or_else(|| RuntimeError::new("unknown dialect"))
    }

    pub(in crate::vm) fn builtin_csv_list_dialects(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("list_dialects() expects no arguments"));
        }
        let mut names: Vec<String> = self.csv_dialects.keys().cloned().collect();
        names.sort();
        Ok(self
            .heap
            .alloc_list(names.into_iter().map(Value::Str).collect()))
    }

    pub(in crate::vm) fn builtin_csv_field_size_limit(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new(
                "field_size_limit() expects at most one argument",
            ));
        }
        let previous = self.csv_field_size_limit;
        if let Some(value) = args.into_iter().next() {
            self.csv_field_size_limit = value_to_int(value)?;
        }
        Ok(Value::Int(previous))
    }

    pub(in crate::vm) fn builtin_csv_dialect_validate(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("Dialect() expects one argument"));
        }
        let dialect = args.into_iter().next().expect("checked len");
        let load_attr = |vm: &mut Vm, name: &str| {
            vm.builtin_getattr(
                vec![dialect.clone(), Value::Str(name.to_string())],
                HashMap::new(),
            )
        };
        let load_attr_or_none = |vm: &mut Vm, name: &str| match load_attr(vm, name) {
            Ok(value) => Ok(value),
            Err(err) if classify_runtime_error(&err.message) == "AttributeError" => Ok(Value::None),
            Err(err) => Err(err),
        };
        let delimiter = csv_char_from_value(load_attr_or_none(self, "delimiter")?, "delimiter")?;
        let quotechar =
            csv_optional_char_from_value(load_attr_or_none(self, "quotechar")?, "quotechar")?;
        let escapechar =
            csv_optional_char_from_value(load_attr_or_none(self, "escapechar")?, "escapechar")?;
        let doublequote = match load_attr_or_none(self, "doublequote")? {
            Value::None => true,
            value => value_to_bool_flag(value, "doublequote")?,
        };
        let skipinitialspace = match load_attr_or_none(self, "skipinitialspace")? {
            Value::None => false,
            value => value_to_bool_flag(value, "skipinitialspace")?,
        };
        let lineterminator = match load_attr_or_none(self, "lineterminator")? {
            Value::Str(text) => text,
            other => {
                return Err(RuntimeError::new(format!(
                    "\"lineterminator\" must be a string, not {}",
                    csv_type_name(&other)
                )));
            }
        };
        let mut quoting = value_to_int(load_attr_or_none(self, "quoting")?)?;
        let _strict = match load_attr_or_none(self, "strict")? {
            Value::None => false,
            value => value_to_bool_flag(value, "strict")?,
        };
        let _ = doublequote;
        if quotechar.is_none() && quoting == 0 {
            quoting = 3;
        }
        validate_csv_parameter_consistency(
            delimiter,
            quotechar,
            escapechar,
            skipinitialspace,
            &lineterminator,
            quoting,
        )?;
        Ok(dialect)
    }

    pub(in crate::vm) fn resolve_csv_dialect(
        &self,
        dialect: Option<Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        match dialect {
            Some(Value::Str(name)) => self
                .csv_dialects
                .get(&name)
                .cloned()
                .map(Some)
                .ok_or_else(|| RuntimeError::new("unknown dialect")),
            other => Ok(other),
        }
    }

    pub(in crate::vm) fn extract_csv_delimiter(
        &mut self,
        dialect: Option<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<char, RuntimeError> {
        if let Some(value) = kwargs.remove("delimiter") {
            return csv_char_from_value(value, "delimiter");
        }
        let resolved = self.resolve_csv_dialect(dialect)?;
        if let Some(dialect) = resolved {
            if let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("delimiter".to_string())],
                HashMap::new(),
            ) {
                return csv_char_from_value(value, "delimiter");
            }
        }
        Ok(',')
    }

    pub(in crate::vm) fn extract_csv_quotechar(
        &mut self,
        dialect: Option<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<Option<char>, RuntimeError> {
        if let Some(value) = kwargs.remove("quotechar") {
            return csv_optional_char_from_value(value, "quotechar");
        }
        let resolved = self.resolve_csv_dialect(dialect)?;
        if let Some(dialect) = resolved {
            if let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("quotechar".to_string())],
                HashMap::new(),
            ) {
                return csv_optional_char_from_value(value, "quotechar");
            }
        }
        Ok(Some('"'))
    }

    pub(in crate::vm) fn extract_csv_escapechar(
        &mut self,
        dialect: Option<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<Option<char>, RuntimeError> {
        if let Some(value) = kwargs.remove("escapechar") {
            return csv_optional_char_from_value(value, "escapechar");
        }
        let resolved = self.resolve_csv_dialect(dialect)?;
        if let Some(dialect) = resolved {
            if let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("escapechar".to_string())],
                HashMap::new(),
            ) {
                return csv_optional_char_from_value(value, "escapechar");
            }
        }
        Ok(None)
    }

    pub(in crate::vm) fn extract_csv_skipinitialspace(
        &mut self,
        dialect: Option<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<bool, RuntimeError> {
        if let Some(value) = kwargs.remove("skipinitialspace") {
            return value_to_bool_flag(value, "skipinitialspace");
        }
        let resolved = self.resolve_csv_dialect(dialect)?;
        if let Some(dialect) = resolved {
            if let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("skipinitialspace".to_string())],
                HashMap::new(),
            ) {
                return value_to_bool_flag(value, "skipinitialspace");
            }
        }
        Ok(false)
    }

    pub(in crate::vm) fn extract_csv_quoting(
        &mut self,
        dialect: Option<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<i64, RuntimeError> {
        if let Some(value) = kwargs.remove("quoting") {
            return value_to_int(value);
        }
        let resolved = self.resolve_csv_dialect(dialect)?;
        if let Some(dialect) = resolved {
            if let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("quoting".to_string())],
                HashMap::new(),
            ) {
                return value_to_int(value);
            }
        }
        Ok(0)
    }

    pub(in crate::vm) fn extract_csv_doublequote(
        &mut self,
        dialect: Option<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<bool, RuntimeError> {
        if let Some(value) = kwargs.remove("doublequote") {
            return value_to_bool_flag(value, "doublequote");
        }
        let resolved = self.resolve_csv_dialect(dialect)?;
        if let Some(dialect) = resolved {
            if let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("doublequote".to_string())],
                HashMap::new(),
            ) {
                return value_to_bool_flag(value, "doublequote");
            }
        }
        Ok(true)
    }

    pub(in crate::vm) fn extract_csv_lineterminator(
        &mut self,
        dialect: Option<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<String, RuntimeError> {
        if let Some(value) = kwargs.remove("lineterminator") {
            return match value {
                Value::Str(text) => Ok(text),
                other => Err(RuntimeError::new(format!(
                    "\"lineterminator\" must be a string, not {}",
                    csv_type_name(&other)
                ))),
            };
        }
        let resolved = self.resolve_csv_dialect(dialect)?;
        if let Some(dialect) = resolved {
            if let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("lineterminator".to_string())],
                HashMap::new(),
            ) {
                return match value {
                    Value::Str(text) => Ok(text),
                    other => Err(RuntimeError::new(format!(
                        "\"lineterminator\" must be a string, not {}",
                        csv_type_name(&other)
                    ))),
                };
            }
        }
        Ok("\r\n".to_string())
    }

    pub(in crate::vm) fn extract_csv_strict(
        &mut self,
        dialect: Option<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<bool, RuntimeError> {
        if let Some(value) = kwargs.remove("strict") {
            return value_to_bool_flag(value, "strict");
        }
        let resolved = self.resolve_csv_dialect(dialect)?;
        if let Some(dialect) = resolved {
            if let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("strict".to_string())],
                HashMap::new(),
            ) {
                return value_to_bool_flag(value, "strict");
            }
        }
        Ok(false)
    }

    pub(in crate::vm) fn extract_csv_writer_state(
        &self,
        writer: &ObjRef,
    ) -> Result<
        (
            Value,
            char,
            Option<char>,
            Option<char>,
            i64,
            bool,
            String,
            bool,
        ),
        RuntimeError,
    > {
        let Object::Module(writer_data) = &*writer.kind() else {
            return Err(RuntimeError::new("writer object is invalid"));
        };
        let target = writer_data
            .globals
            .get("__csv_target__")
            .cloned()
            .ok_or_else(|| RuntimeError::new("writer target is missing"))?;
        let delimiter = writer_data
            .globals
            .get("__csv_delimiter__")
            .cloned()
            .ok_or_else(|| RuntimeError::new("writer delimiter is missing"))
            .and_then(|value| csv_char_from_value(value, "delimiter"))?;
        let quotechar = writer_data
            .globals
            .get("__csv_quotechar__")
            .cloned()
            .ok_or_else(|| RuntimeError::new("writer quotechar is missing"))
            .and_then(|value| csv_optional_char_from_value(value, "quotechar"))?;
        let escapechar = writer_data
            .globals
            .get("__csv_escapechar__")
            .cloned()
            .ok_or_else(|| RuntimeError::new("writer escapechar is missing"))
            .and_then(|value| csv_optional_char_from_value(value, "escapechar"))?;
        let quoting = writer_data
            .globals
            .get("__csv_quoting__")
            .cloned()
            .ok_or_else(|| RuntimeError::new("writer quoting is missing"))
            .and_then(value_to_int)?;
        let doublequote = writer_data
            .globals
            .get("__csv_doublequote__")
            .cloned()
            .map(|value| is_truthy(&value))
            .unwrap_or(true);
        let skipinitialspace = writer_data
            .globals
            .get("__csv_skipinitialspace__")
            .cloned()
            .map(|value| is_truthy(&value))
            .unwrap_or(false);
        let lineterminator = match writer_data.globals.get("__csv_lineterminator__") {
            Some(Value::Str(value)) => value.clone(),
            _ => return Err(RuntimeError::new("writer lineterminator is missing")),
        };
        Ok((
            target,
            delimiter,
            quotechar,
            escapechar,
            quoting,
            doublequote,
            lineterminator,
            skipinitialspace,
        ))
    }

    pub(in crate::vm) fn coerce_csv_writer_field_text(
        &mut self,
        value: Value,
    ) -> Result<String, RuntimeError> {
        match value {
            Value::Str(text) => Ok(text),
            Value::None => Ok(String::new()),
            other => match self.call_builtin(BuiltinFunction::Str, vec![other], HashMap::new())? {
                Value::Str(text) => Ok(text),
                _ => Err(RuntimeError::new("csv conversion failed")),
            },
        }
    }

    pub(in crate::vm) fn coerce_csv_reader_text(
        &mut self,
        value: Value,
    ) -> Result<String, RuntimeError> {
        match value {
            Value::Str(text) => Ok(text),
            Value::Bytes(_) | Value::ByteArray(_) | Value::MemoryView(_) => Err(RuntimeError::new(
                "iterator should return strings, not bytes (the file should be opened in text mode)",
            )),
            other => Err(RuntimeError::new(format!(
                "iterator should return strings, not {}",
                csv_type_name(&other)
            ))),
        }
    }
}
