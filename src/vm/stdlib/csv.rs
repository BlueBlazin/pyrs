use super::super::{
    BuiltinFunction, GeneratorResumeOutcome, HashMap, InternalCallOutcome, ModuleObject,
    NativeMethodKind, ObjRef, Object, RuntimeError, Value, Vm, format_value, is_truthy,
    runtime_error_matches_exception, value_to_int,
};

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
            Err(err) if runtime_error_matches_exception(&err, "TypeError") => {
                return Err(RuntimeError::type_error("expected iterable"));
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
                    let text = self.coerce_csv_reader_text(value)?;
                    let (line_body, line_ending) = split_csv_line_ending(&text);
                    pending_record.push_str(line_body);
                    let record_state = csv_record_state(
                        &pending_record,
                        delimiter,
                        quotechar,
                        escapechar,
                        skipinitialspace,
                        quoting,
                        doublequote,
                    );
                    if record_state.trailing_escape {
                        if line_ending == "\r\n" {
                            pending_record.push('\r');
                            if record_state.in_quotes {
                                pending_record.push('\n');
                                continue;
                            }
                        } else if !line_ending.is_empty() {
                            pending_record.push_str(line_ending);
                            continue;
                        } else {
                            continue;
                        }
                    } else if record_state.in_quotes {
                        pending_record.push_str(line_ending);
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
                        return Err(RuntimeError::stop_iteration("StopIteration"));
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
            Err(err) if runtime_error_matches_exception(&err, "OSError") => {
                return Err(err);
            }
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
        let row_values = self.csv_collect_writer_row_values(row.clone())?;
        let mut fields: Vec<(String, bool, bool)> = Vec::new();
        for value in row_values {
            fields.push(self.csv_writer_field_from_value(
                value,
                delimiter,
                quotechar,
                escapechar,
                quoting,
                doublequote,
            )?);
        }

        if fields.len() == 1 && fields[0].0.is_empty() && !fields[0].1 {
            if quoting == 3 || quoting == 4 || quoting == 5 {
                return Err(RuntimeError::new(
                    "single empty field record must be quoted",
                ));
            }
            if let Some(quote) = quotechar {
                fields[0] = (format!("{quote}{quote}"), true, fields[0].2);
            }
        }
        if quoting == 3 && fields.len() == 1 && fields[0].0.is_empty() && !fields[0].1 {
            return Err(RuntimeError::new(
                "single empty field record must be quoted",
            ));
        }
        if delimiter == ' ' && skipinitialspace {
            for (text, quoted, none_field) in &mut fields {
                if text.is_empty() && !*quoted {
                    let disallow_none = *none_field && (quoting == 4 || quoting == 5);
                    if quoting == 3 || disallow_none {
                        return Err(RuntimeError::new(
                            "empty field must be quoted if delimiter is space and skipinitialspace is true",
                        ));
                    }
                    if let Some(quote) = quotechar {
                        *text = format!("{quote}{quote}");
                        *quoted = true;
                    } else {
                        return Err(RuntimeError::new(
                            "empty field must be quoted if delimiter is space and skipinitialspace is true",
                        ));
                    }
                }
            }
        }
        let mut line = fields
            .into_iter()
            .map(|(field, _, _)| field)
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
            Err(err) if runtime_error_matches_exception(&err, "TypeError") => {
                return Err(RuntimeError::type_error("expected iterable"));
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

    fn csv_collect_writer_row_values(&mut self, row: Value) -> Result<Vec<Value>, RuntimeError> {
        match self.to_iterator_value(row.clone()) {
            Ok(iterator) => {
                let mut values = Vec::new();
                loop {
                    match self.next_from_iterator_value(&iterator)? {
                        GeneratorResumeOutcome::Yield(value) => values.push(value),
                        GeneratorResumeOutcome::Complete(_) => break,
                        GeneratorResumeOutcome::PropagatedException => {
                            if self.pending_generator_exception.is_some() {
                                self.propagate_pending_generator_exception()?;
                            }
                            return Err(
                                self.runtime_error_from_active_exception("iteration failed")
                            );
                        }
                    }
                }
                Ok(values)
            }
            Err(err) if runtime_error_matches_exception(&err, "TypeError") => {
                if let Some(getitem) = self.lookup_bound_special_method(&row, "__getitem__")? {
                    let mut values = Vec::new();
                    let mut index: i64 = 0;
                    loop {
                        match self.call_internal(
                            getitem.clone(),
                            vec![Value::Int(index)],
                            HashMap::new(),
                        ) {
                            Ok(InternalCallOutcome::Value(value)) => values.push(value),
                            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                                return Err(RuntimeError::new("__getitem__() failed"));
                            }
                            Err(err)
                                if runtime_error_matches_exception(&err, "IndexError")
                                    || runtime_error_matches_exception(&err, "StopIteration") =>
                            {
                                break;
                            }
                            Err(err) => return Err(err),
                        }
                        index += 1;
                    }
                    return Ok(values);
                }
                Err(self.csv_non_iterable_error(&row))
            }
            Err(err) => Err(err),
        }
    }

    pub(in crate::vm) fn runtime_error_from_active_exception(
        &mut self,
        fallback: &str,
    ) -> RuntimeError {
        let mut active = None;
        for frame in self.frames.iter_mut().rev() {
            if let Some(value) = frame.active_exception.take() {
                active = Some(value);
                break;
            }
        }
        match active {
            Some(Value::Exception(exception)) => {
                let exception = *exception;
                let message = self.format_exception_object(&exception);
                RuntimeError {
                    message,
                    exception: Some(Box::new(exception)),
                }
            }
            Some(Value::ExceptionType(name)) => RuntimeError::with_exception(name, None),
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
    ) -> Result<(String, bool, bool), RuntimeError> {
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
        Ok((rendered, was_quoted, is_none))
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
            Err(err) if runtime_error_matches_exception(&err, "AttributeError") => Ok(None),
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
            _ => return Err(RuntimeError::with_exception("Error", Some("unknown dialect".to_string()))),
        };
        if self.csv_dialects.remove(&name).is_none() {
            return Err(RuntimeError::with_exception("Error", Some("unknown dialect".to_string())));
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
            _ => return Err(RuntimeError::with_exception("Error", Some("unknown dialect".to_string()))),
        };
        self.csv_dialects
            .get(&name)
            .cloned()
            .ok_or_else(|| RuntimeError::with_exception("Error", Some("unknown dialect".to_string())))
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
            Err(err) if runtime_error_matches_exception(&err, "AttributeError") => Ok(Value::None),
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
                .ok_or_else(|| RuntimeError::with_exception("Error", Some("unknown dialect".to_string()))),
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
        if let Some(dialect) = resolved
            && let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("delimiter".to_string())],
                HashMap::new(),
            )
        {
            return csv_char_from_value(value, "delimiter");
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
        if let Some(dialect) = resolved
            && let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("quotechar".to_string())],
                HashMap::new(),
            )
        {
            return csv_optional_char_from_value(value, "quotechar");
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
        if let Some(dialect) = resolved
            && let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("escapechar".to_string())],
                HashMap::new(),
            )
        {
            return csv_optional_char_from_value(value, "escapechar");
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
        if let Some(dialect) = resolved
            && let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("skipinitialspace".to_string())],
                HashMap::new(),
            )
        {
            return value_to_bool_flag(value, "skipinitialspace");
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
        if let Some(dialect) = resolved
            && let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("quoting".to_string())],
                HashMap::new(),
            )
        {
            return value_to_int(value);
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
        if let Some(dialect) = resolved
            && let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("doublequote".to_string())],
                HashMap::new(),
            )
        {
            return value_to_bool_flag(value, "doublequote");
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
        if let Some(dialect) = resolved
            && let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("lineterminator".to_string())],
                HashMap::new(),
            )
        {
            return match value {
                Value::Str(text) => Ok(text),
                other => Err(RuntimeError::new(format!(
                    "\"lineterminator\" must be a string, not {}",
                    csv_type_name(&other)
                ))),
            };
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
        if let Some(dialect) = resolved
            && let Ok(value) = self.builtin_getattr(
                vec![dialect, Value::Str("strict".to_string())],
                HashMap::new(),
            )
        {
            return value_to_bool_flag(value, "strict");
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
            other => {
                if let Some(str_method) = self.lookup_bound_special_method(&other, "__str__")? {
                    match self.call_internal(str_method, Vec::new(), HashMap::new())? {
                        InternalCallOutcome::Value(Value::Str(text)) => return Ok(text),
                        InternalCallOutcome::Value(_) => {
                            return Err(RuntimeError::new(
                                "__str__ returned non-string (type str expected)",
                            ));
                        }
                        InternalCallOutcome::CallerExceptionHandled => {
                            return Err(RuntimeError::new("__str__() failed"));
                        }
                    }
                }
                match self.call_builtin(BuiltinFunction::Str, vec![other], HashMap::new())? {
                    Value::Str(text) => Ok(text),
                    _ => Err(RuntimeError::new("csv conversion failed")),
                }
            }
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

fn csv_type_name(value: &Value) -> &'static str {
    match value {
        Value::None => "NoneType",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "str",
        Value::Bytes(_) => "bytes",
        Value::ByteArray(_) => "bytearray",
        Value::MemoryView(_) => "memoryview",
        Value::Complex { .. } => "complex",
        Value::List(_) => "list",
        Value::Tuple(_) => "tuple",
        Value::Dict(_) => "dict",
        Value::DictKeys(_) => "dict_keys",
        Value::Set(_) => "set",
        Value::FrozenSet(_) => "frozenset",
        Value::BigInt(_) => "int",
        Value::Module(_) => "module",
        Value::Class(_) => "type",
        Value::Instance(_) => "object",
        Value::Super(_) => "super",
        Value::BoundMethod(_) => "method",
        Value::Exception(_) => "BaseException",
        Value::ExceptionType(_) => "type",
        Value::Code(_) => "code",
        Value::Function(_) => "function",
        Value::Builtin(_) => "builtin_function_or_method",
        Value::Cell(_) => "cell",
        Value::Iterator(_) => "iterator",
        Value::Generator(_) => "generator",
        Value::Slice { .. } => "slice",
    }
}

fn csv_char_from_value(value: Value, name: &str) -> Result<char, RuntimeError> {
    match value {
        Value::Str(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => Ok(ch),
                _ => Err(RuntimeError::new(format!(
                    "\"{name}\" must be a unicode character, not a string of length {}",
                    text.chars().count()
                ))),
            }
        }
        other => Err(RuntimeError::new(format!(
            "\"{name}\" must be a unicode character, not {}",
            csv_type_name(&other)
        ))),
    }
}

fn csv_optional_char_from_value(value: Value, name: &str) -> Result<Option<char>, RuntimeError> {
    match value {
        Value::None => Ok(None),
        Value::Str(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => Ok(Some(ch)),
                _ => Err(RuntimeError::new(format!(
                    "\"{name}\" must be a unicode character or None, not a string of length {}",
                    text.chars().count()
                ))),
            }
        }
        other => Err(RuntimeError::new(format!(
            "\"{name}\" must be a unicode character or None, not {}",
            csv_type_name(&other)
        ))),
    }
}

fn validate_csv_parameter_consistency(
    delimiter: char,
    quotechar: Option<char>,
    escapechar: Option<char>,
    skipinitialspace: bool,
    lineterminator: &str,
    quoting: i64,
) -> Result<(), RuntimeError> {
    if !(0..=5).contains(&quoting) {
        return Err(RuntimeError::new("bad \"quoting\" value"));
    }
    if quotechar.is_none() && quoting != 3 {
        return Err(RuntimeError::new(
            "quotechar must be set if quoting enabled",
        ));
    }
    if delimiter == '\n' || delimiter == '\r' {
        return Err(RuntimeError::new("bad delimiter value"));
    }
    if quotechar.is_some_and(|ch| ch == '\n' || ch == '\r') {
        return Err(RuntimeError::new("bad quotechar value"));
    }
    if escapechar.is_some_and(|ch| ch == '\n' || ch == '\r') {
        return Err(RuntimeError::new("bad escapechar value"));
    }
    if skipinitialspace && quotechar == Some(' ') {
        return Err(RuntimeError::new("bad quotechar value"));
    }
    if skipinitialspace && escapechar == Some(' ') {
        return Err(RuntimeError::new("bad escapechar value"));
    }
    if quotechar == Some(delimiter) {
        return Err(RuntimeError::new("bad delimiter or quotechar value"));
    }
    if escapechar == Some(delimiter) {
        return Err(RuntimeError::new("bad delimiter or escapechar value"));
    }
    if let (Some(quote), Some(escape)) = (quotechar, escapechar)
        && quote == escape
    {
        return Err(RuntimeError::new("bad escapechar or quotechar value"));
    }
    if lineterminator.contains(delimiter) {
        return Err(RuntimeError::new("bad delimiter or lineterminator value"));
    }
    if quotechar.is_some_and(|ch| lineterminator.contains(ch)) {
        return Err(RuntimeError::new("bad quotechar or lineterminator value"));
    }
    if escapechar.is_some_and(|ch| lineterminator.contains(ch)) {
        return Err(RuntimeError::new("bad escapechar or lineterminator value"));
    }
    Ok(())
}

fn value_to_bool_flag(value: Value, name: &str) -> Result<bool, RuntimeError> {
    match value {
        Value::Bool(flag) => Ok(flag),
        Value::Int(number) => Ok(number != 0),
        Value::BigInt(number) => Ok(!number.is_zero()),
        _ => Err(RuntimeError::new(format!("{name} must be bool"))),
    }
}

#[derive(Debug)]
struct CsvParsedField {
    value: String,
    quoted: bool,
}

fn csv_reader_value_for_field(field: CsvParsedField, quoting: i64) -> Result<Value, RuntimeError> {
    match quoting {
        2 if !field.quoted && !field.value.is_empty() => {
            Ok(Value::Float(parse_csv_reader_float(&field.value)?))
        }
        4 if !field.quoted && field.value.is_empty() => Ok(Value::None),
        4 if !field.quoted => Ok(Value::Float(parse_csv_reader_float(&field.value)?)),
        5 if !field.quoted && field.value.is_empty() => Ok(Value::None),
        _ => Ok(Value::Str(field.value)),
    }
}

fn parse_csv_reader_float(text: &str) -> Result<f64, RuntimeError> {
    text.trim()
        .parse::<f64>()
        .map_err(|_| RuntimeError::new("could not convert string to float"))
}

fn split_csv_line_ending(text: &str) -> (&str, &str) {
    if let Some(stripped) = text.strip_suffix("\r\n") {
        return (stripped, "\r\n");
    }
    if let Some(stripped) = text.strip_suffix('\n') {
        return (stripped, "\n");
    }
    if let Some(stripped) = text.strip_suffix('\r') {
        return (stripped, "\r");
    }
    (text, "")
}

struct CsvRecordState {
    in_quotes: bool,
    trailing_escape: bool,
}

fn csv_record_state(
    row: &str,
    delimiter: char,
    quotechar: Option<char>,
    escapechar: Option<char>,
    skipinitialspace: bool,
    quoting: i64,
    doublequote: bool,
) -> CsvRecordState {
    if row.is_empty() {
        return CsvRecordState {
            in_quotes: false,
            trailing_escape: false,
        };
    }
    let active_quotechar = if quoting == 3 { None } else { quotechar };
    let mut in_quotes = false;
    let mut at_field_start = true;
    let mut trailing_escape = false;
    let mut chars = row.chars().peekable();
    while let Some(ch) = chars.next() {
        trailing_escape = false;
        if let Some(escape) = escapechar
            && ch == escape
        {
            if chars.peek().is_some() {
                chars.next();
                at_field_start = false;
                continue;
            }
            trailing_escape = true;
            break;
        }
        if skipinitialspace && !in_quotes && at_field_start && ch == ' ' {
            continue;
        }
        if let Some(quote) = active_quotechar
            && ch == quote
        {
            if in_quotes {
                if doublequote && chars.peek().is_some_and(|next| *next == quote) {
                    chars.next();
                    at_field_start = false;
                } else {
                    in_quotes = false;
                }
            } else if at_field_start {
                in_quotes = true;
                at_field_start = false;
            } else {
                at_field_start = false;
            }
            continue;
        }
        if ch == delimiter && !in_quotes {
            at_field_start = true;
            continue;
        }
        at_field_start = false;
    }

    CsvRecordState {
        in_quotes,
        trailing_escape,
    }
}

fn parse_csv_row_simple(
    row: &str,
    delimiter: char,
    quotechar: Option<char>,
    escapechar: Option<char>,
    skipinitialspace: bool,
    quoting: i64,
    doublequote: bool,
    strict: bool,
    field_limit: Option<usize>,
) -> Result<Vec<CsvParsedField>, RuntimeError> {
    if row.is_empty() {
        return Ok(Vec::new());
    }
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut field_quoted = false;
    let mut in_quotes = false;
    let mut just_closed_quote = false;
    let active_quotechar = if quoting == 3 { None } else { quotechar };
    let mut chars = row.chars().peekable();
    while let Some(ch) = chars.next() {
        if !in_quotes && just_closed_quote {
            if ch == delimiter {
                fields.push(CsvParsedField {
                    value: std::mem::take(&mut current),
                    quoted: field_quoted,
                });
                field_quoted = false;
                just_closed_quote = false;
                if skipinitialspace {
                    while chars.peek().is_some_and(|next| *next == ' ') {
                        chars.next();
                    }
                }
                continue;
            }
            if strict {
                return Err(RuntimeError::new("',' expected after '\"'"));
            }
            just_closed_quote = false;
        }
        if let Some(quote) = active_quotechar
            && ch == quote
        {
            if in_quotes {
                if doublequote && chars.peek().is_some_and(|next| *next == quote) {
                    current.push(quote);
                    chars.next();
                } else {
                    in_quotes = false;
                    just_closed_quote = true;
                }
            } else if current.is_empty() {
                field_quoted = true;
                in_quotes = true;
                just_closed_quote = false;
            } else {
                if strict {
                    return Err(RuntimeError::new("',' expected after '\"'"));
                }
                current.push(quote);
                just_closed_quote = false;
            }
            continue;
        }
        if let Some(escape) = escapechar
            && ch == escape
        {
            if let Some(next) = chars.next() {
                current.push(next);
            } else if strict {
                return Err(RuntimeError::new("unexpected end of data"));
            } else if in_quotes || current.is_empty() {
                current.push('\n');
            } else {
                current.push(escape);
            }
            continue;
        }
        if skipinitialspace && !in_quotes && current.is_empty() && ch == ' ' {
            continue;
        }
        if (ch == '\n' || ch == '\r') && !in_quotes {
            return Err(RuntimeError::new(
                "new-line character seen in unquoted field - do you need to open the file with newline=''?",
            ));
        }
        if ch == delimiter && !in_quotes {
            fields.push(CsvParsedField {
                value: std::mem::take(&mut current),
                quoted: field_quoted,
            });
            field_quoted = false;
            just_closed_quote = false;
            if skipinitialspace {
                while chars.peek().is_some_and(|next| *next == ' ') {
                    chars.next();
                }
            }
            continue;
        }
        current.push(ch);
        just_closed_quote = false;
        if let Some(limit) = field_limit
            && current.chars().count() > limit
        {
            return Err(RuntimeError::new("field larger than field limit"));
        }
    }
    if in_quotes && strict {
        return Err(RuntimeError::new("unexpected end of data"));
    }
    fields.push(CsvParsedField {
        value: current,
        quoted: field_quoted,
    });
    Ok(fields)
}

fn quote_csv_field(
    field: &str,
    delimiter: char,
    quotechar: Option<char>,
    escapechar: Option<char>,
    quoting: i64,
    doublequote: bool,
    force_quote: bool,
) -> Result<String, RuntimeError> {
    let contains_quote = quotechar.is_some_and(|quote| field.contains(quote));
    let needs_quote = field.contains(delimiter)
        || field.contains('\n')
        || field.contains('\r')
        || (contains_quote && (doublequote || escapechar.is_none()));

    if quoting == 3 {
        let mut out = String::new();
        for ch in field.chars() {
            let must_escape = ch == delimiter
                || ch == '\n'
                || ch == '\r'
                || quotechar.is_some_and(|quote| ch == quote)
                || escapechar.is_some_and(|escape| ch == escape);
            if must_escape {
                let Some(escape) = escapechar else {
                    return Err(RuntimeError::new("need to escape, but no escapechar set"));
                };
                out.push(escape);
            }
            out.push(ch);
        }
        return Ok(out);
    }

    if !force_quote && !needs_quote {
        if let Some(escape) = escapechar {
            let mut escaped = String::new();
            let mut changed = false;
            for ch in field.chars() {
                let must_escape_quote = quotechar.is_some_and(|quote| ch == quote) && !doublequote;
                let must_escape_escape = ch == escape;
                if must_escape_quote || must_escape_escape {
                    escaped.push(escape);
                    changed = true;
                }
                escaped.push(ch);
            }
            if changed {
                return Ok(escaped);
            }
        } else if contains_quote && !doublequote {
            return Err(RuntimeError::new("need to escape quotechar"));
        }
        return Ok(field.to_string());
    }

    let Some(quotechar) = quotechar else {
        let mut out = String::new();
        for ch in field.chars() {
            let must_escape = ch == delimiter || ch == '\n' || ch == '\r';
            if must_escape {
                let Some(escape) = escapechar else {
                    return Err(RuntimeError::new("need to escape, but no escapechar set"));
                };
                out.push(escape);
            }
            out.push(ch);
        }
        return Ok(out);
    };

    let mut escaped = String::new();
    for ch in field.chars() {
        if ch == quotechar {
            if doublequote {
                escaped.push(quotechar);
                escaped.push(quotechar);
            } else if let Some(escape) = escapechar {
                escaped.push(escape);
                escaped.push(quotechar);
            } else {
                return Err(RuntimeError::new("need to escape quotechar"));
            }
        } else if escapechar.is_some_and(|escape| ch == escape) {
            let escape = escapechar.expect("checked some");
            escaped.push(escape);
            escaped.push(ch);
        } else {
            escaped.push(ch);
        }
    }
    Ok(format!("{quotechar}{escaped}{quotechar}"))
}

#[cfg(test)]
mod tests {
    use super::{
        csv_char_from_value, csv_optional_char_from_value, csv_record_state, parse_csv_row_simple,
        quote_csv_field, split_csv_line_ending, validate_csv_parameter_consistency,
    };
    use crate::runtime::Value;

    #[test]
    fn csv_char_parsers_validate_lengths_and_types() {
        assert_eq!(
            csv_char_from_value(Value::Str(",".to_string()), "delimiter").expect("single char"),
            ','
        );
        assert_eq!(
            csv_optional_char_from_value(Value::None, "quotechar").expect("none is valid"),
            None
        );
        assert_eq!(
            csv_optional_char_from_value(Value::Str("\"".to_string()), "quotechar")
                .expect("single quotechar"),
            Some('"')
        );

        let err = csv_char_from_value(Value::Str("::".to_string()), "delimiter")
            .expect_err("multi-char should fail");
        assert!(err.message.contains("string of length 2"));

        let err = csv_optional_char_from_value(Value::Int(1), "quotechar")
            .expect_err("non-string should fail");
        assert!(err.message.contains("unicode character or None"));
    }

    #[test]
    fn validate_csv_parameter_consistency_rejects_invalid_combinations() {
        let err = validate_csv_parameter_consistency(',', None, None, false, "\n", 0)
            .expect_err("QUOTE_MINIMAL with no quotechar should fail");
        assert!(err.message.contains("quotechar must be set"));

        let err = validate_csv_parameter_consistency(',', Some('"'), Some('"'), false, "\n", 0)
            .expect_err("quotechar and escapechar cannot match");
        assert!(err.message.contains("bad escapechar or quotechar value"));

        let err = validate_csv_parameter_consistency(',', Some('"'), Some('\\'), false, ",", 0)
            .expect_err("delimiter in lineterminator should fail");
        assert!(
            err.message
                .contains("bad delimiter or lineterminator value")
        );

        let err = validate_csv_parameter_consistency(',', Some('"'), None, false, "\"", 0)
            .expect_err("quotechar in lineterminator should fail");
        assert!(
            err.message
                .contains("bad quotechar or lineterminator value")
        );

        let err = validate_csv_parameter_consistency(',', Some('"'), Some('\\'), false, "\\", 0)
            .expect_err("escapechar in lineterminator should fail");
        assert!(
            err.message
                .contains("bad escapechar or lineterminator value")
        );

        assert!(
            validate_csv_parameter_consistency(',', Some('"'), Some('\\'), false, "", 0).is_ok()
        );
        assert!(
            validate_csv_parameter_consistency(',', Some('"'), Some('\\'), false, "\n", 0).is_ok()
        );
    }

    #[test]
    fn split_csv_line_ending_detects_common_endings() {
        assert_eq!(split_csv_line_ending("a,b\r\n"), ("a,b", "\r\n"));
        assert_eq!(split_csv_line_ending("a,b\n"), ("a,b", "\n"));
        assert_eq!(split_csv_line_ending("a,b\r"), ("a,b", "\r"));
        assert_eq!(split_csv_line_ending("a,b"), ("a,b", ""));
    }

    #[test]
    fn csv_record_state_tracks_quote_and_escape_state() {
        let state = csv_record_state("a,\"b", ',', Some('"'), None, false, 0, true);
        assert!(state.in_quotes);
        assert!(!state.trailing_escape);

        let state = csv_record_state("a,\\", ',', Some('"'), Some('\\'), false, 0, true);
        assert!(!state.in_quotes);
        assert!(state.trailing_escape);
    }

    #[test]
    fn parse_csv_row_simple_honors_strict_mode_and_field_limit() {
        let fields = parse_csv_row_simple(
            "a,\"b,c\",d",
            ',',
            Some('"'),
            None,
            false,
            0,
            true,
            true,
            None,
        )
        .expect("valid quoted row");
        let values: Vec<String> = fields.into_iter().map(|field| field.value).collect();
        assert_eq!(
            values,
            vec!["a".to_string(), "b,c".to_string(), "d".to_string()]
        );

        let err = parse_csv_row_simple("a,\"", ',', Some('"'), None, false, 0, true, true, None)
            .expect_err("strict unterminated quote should fail");
        assert!(err.message.contains("unexpected end of data"));

        let err =
            parse_csv_row_simple("abcd", ',', Some('"'), None, false, 0, true, false, Some(3))
                .expect_err("field limit should fail");
        assert!(err.message.contains("field larger than field limit"));
    }

    #[test]
    fn quote_csv_field_handles_minimal_and_quote_none_modes() {
        let quoted =
            quote_csv_field("a,b", ',', Some('"'), None, 0, true, false).expect("minimal quote");
        assert_eq!(quoted, "\"a,b\"");

        let quote_none = quote_csv_field("a,b", ',', None, Some('\\'), 3, false, false)
            .expect("quote none with escapechar");
        assert_eq!(quote_none, "a\\,b");

        let err = quote_csv_field("a,b", ',', None, None, 3, false, false)
            .expect_err("quote none without escapechar should fail");
        assert!(err.message.contains("need to escape"));
    }
}
