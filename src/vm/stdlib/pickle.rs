use super::super::*;

impl Vm {
    fn object_reduce_ex_builtin_singleton_name(&self, value: &Value) -> Option<&'static str> {
        let Value::Instance(instance) = value else {
            return None;
        };
        let Some(Value::Instance(ellipsis)) = self.builtins.get("Ellipsis") else {
            return None;
        };
        if instance.id() == ellipsis.id() {
            return Some("Ellipsis");
        }
        let Some(Value::Instance(not_implemented)) = self.builtins.get("NotImplemented") else {
            return None;
        };
        if instance.id() == not_implemented.id() {
            return Some("NotImplemented");
        }
        None
    }

    fn pickle_copyreg_callable(&mut self, attr_name: &str) -> Result<Value, RuntimeError> {
        let copyreg_module = Value::Module(self.import_module_object("copyreg")?);
        self.builtin_getattr(
            vec![copyreg_module, Value::Str(attr_name.to_string())],
            HashMap::new(),
        )
    }

    fn object_reduce_ex_new_constructor_and_args(
        &mut self,
        value: &Value,
        protocol: i64,
    ) -> Result<Option<(Value, Value)>, RuntimeError> {
        let Value::Instance(_) = value else {
            return Ok(None);
        };
        let Some(class_obj) = self.class_of_value(value).map(Value::Class) else {
            return Ok(None);
        };

        if let Some(getnewargs_ex) = self.lookup_bound_special_method(value, "__getnewargs_ex__")?
        {
            let payload = match self.call_internal(getnewargs_ex, Vec::new(), HashMap::new())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("__getnewargs_ex__ callback failed"));
                }
            };
            let Value::Tuple(pair_obj) = payload else {
                return Err(RuntimeError::new(
                    "__getnewargs_ex__ should return a tuple of length 2",
                ));
            };
            let Object::Tuple(pair_values) = &*pair_obj.kind() else {
                return Err(RuntimeError::new(
                    "__getnewargs_ex__ should return a tuple of length 2",
                ));
            };
            if pair_values.len() != 2 {
                return Err(RuntimeError::new(
                    "__getnewargs_ex__ should return a tuple of length 2",
                ));
            }
            let (args_tuple, kwargs_dict) = match (&pair_values[0], &pair_values[1]) {
                (Value::Tuple(args_tuple), Value::Dict(kwargs_dict)) => {
                    (args_tuple.clone(), kwargs_dict.clone())
                }
                _ => {
                    return Err(RuntimeError::new(
                        "__getnewargs_ex__ should return (tuple, dict)",
                    ))
                }
            };

            let tuple_values = match &*args_tuple.kind() {
                Object::Tuple(values) => values.clone(),
                _ => {
                    return Err(RuntimeError::new(
                        "__getnewargs_ex__ should return (tuple, dict)",
                    ))
                }
            };
            let kwargs_entries = match &*kwargs_dict.kind() {
                Object::Dict(entries) => entries.iter().cloned().collect::<Vec<_>>(),
                _ => {
                    return Err(RuntimeError::new(
                        "__getnewargs_ex__ should return (tuple, dict)",
                    ))
                }
            };

            if protocol >= 4 {
                let constructor_args = self.heap.alloc_tuple(vec![
                    class_obj,
                    Value::Tuple(args_tuple),
                    Value::Dict(kwargs_dict),
                ]);
                return Ok(Some((
                    self.pickle_copyreg_callable("__newobj_ex__")?,
                    constructor_args,
                )));
            }

            // Protocols <4 lack NEWOBJ_EX. For int-subclass compatibility we lower
            // __getnewargs_ex__ to int(cls, *args, base?) positional form.
            if !matches!(value, Value::Instance(instance) if self.instance_backing_int(instance).is_some())
            {
                return Err(RuntimeError::new(
                    "__getnewargs_ex__ kwargs require protocol >= 4",
                ));
            }
            let mut constructor_args = Vec::with_capacity(1 + tuple_values.len() + kwargs_entries.len());
            constructor_args.push(class_obj);
            constructor_args.extend(tuple_values);
            for (key, value) in kwargs_entries {
                let Value::Str(name) = key else {
                    return Err(RuntimeError::new(
                        "__getnewargs_ex__ kwargs keys must be strings",
                    ));
                };
                if name == "base" {
                    constructor_args.push(value);
                } else {
                    return Err(RuntimeError::new(
                        "__getnewargs_ex__ kwargs require protocol >= 4",
                    ));
                }
            }
            return Ok(Some((
                Value::Builtin(BuiltinFunction::Int),
                self.heap.alloc_tuple(constructor_args),
            )));
        }

        if let Some(getnewargs) = self.lookup_bound_special_method(value, "__getnewargs__")? {
            let payload = match self.call_internal(getnewargs, Vec::new(), HashMap::new())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("__getnewargs__ callback failed"));
                }
            };
            let Value::Tuple(args_obj) = payload else {
                return Err(RuntimeError::new("__getnewargs__ should return a tuple"));
            };
            let Object::Tuple(args_values) = &*args_obj.kind() else {
                return Err(RuntimeError::new("__getnewargs__ should return a tuple"));
            };
            let mut constructor_args = Vec::with_capacity(args_values.len() + 1);
            constructor_args.push(class_obj);
            constructor_args.extend(args_values.clone());
            return Ok(Some((
                self.pickle_copyreg_callable("__newobj__")?,
                self.heap.alloc_tuple(constructor_args),
            )));
        }

        if let Value::Instance(instance) = value {
            if let Some(integer_value) = self.instance_backing_int(instance) {
                let int_value = BuiltinFunction::Int.call(&self.heap, vec![integer_value])?;
                return Ok(Some((
                    self.pickle_copyreg_callable("__newobj__")?,
                    self.heap.alloc_tuple(vec![class_obj, int_value]),
                )));
            }
        }

        Ok(None)
    }

    fn object_reduce_ex_legacy_constructor_and_args(
        &mut self,
        value: &Value,
    ) -> Result<Option<(Value, Value)>, RuntimeError> {
        let Value::Instance(instance) = value else {
            return Ok(None);
        };
        let Some(class_obj) = self.class_of_value(value).map(Value::Class) else {
            return Ok(None);
        };
        let Some(integer_value) = self.instance_backing_int(instance) else {
            return Ok(None);
        };
        let int_value = BuiltinFunction::Int.call(&self.heap, vec![integer_value])?;
        let constructor_args = self.heap.alloc_tuple(vec![
            class_obj,
            Value::Builtin(BuiltinFunction::Int),
            int_value,
        ]);
        Ok(Some((
            self.pickle_copyreg_callable("_reconstructor")?,
            constructor_args,
        )))
    }

    fn instance_has_non_object_reduce(&self, instance: &ObjRef) -> bool {
        let class = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return false,
        };
        for entry in self.class_mro_entries(&class) {
            let Object::Class(class_data) = &*entry.kind() else {
                continue;
            };
            let Some(attr) = class_data.attrs.get("__reduce__") else {
                continue;
            };
            return !matches!(
                attr,
                Value::Builtin(BuiltinFunction::ObjectReduceEx) if class_data.name == "object"
            );
        }
        false
    }

    fn object_reduce_ex_custom_reduce(
        &mut self,
        value: &Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let Value::Instance(instance) = value else {
            return Ok(None);
        };
        if !self.instance_has_non_object_reduce(instance) {
            return Ok(None);
        }
        let Some(reduce_callable) = self.lookup_bound_special_method(value, "__reduce__")? else {
            return Ok(None);
        };
        let reduced = match self.call_internal(reduce_callable, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(RuntimeError::new("__reduce__ callback failed"));
            }
        };
        if matches!(reduced, Value::Str(_)) {
            return Ok(Some(reduced));
        }
        if let Value::Tuple(obj) = &reduced {
            let tuple_len = {
                let Object::Tuple(values) = &*obj.kind() else {
                    return Err(RuntimeError::new("__reduce__ must return a tuple"));
                };
                values.len()
            };
            if !(2..=6).contains(&tuple_len) {
                return Err(RuntimeError::new(
                    "tuple returned by __reduce__ must contain 2 through 6 elements",
                ));
            }
            return Ok(Some(reduced));
        }
        Err(RuntimeError::new("__reduce__ must return a string or tuple"))
    }

    pub(in crate::vm) fn builtin_object_getstate(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "object.__getstate__() takes exactly one argument",
            ));
        }
        let Some(value) = args.first() else {
            return Ok(Value::None);
        };
        match value {
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => {
                    let entries = Self::instance_dict_entries(instance_data);
                    if entries.is_empty() {
                        Ok(Value::None)
                    } else {
                        Ok(self.heap.alloc_dict(entries))
                    }
                }
                _ => Ok(Value::None),
            },
            _ => Ok(Value::None),
        }
    }

    pub(in crate::vm) fn builtin_object_setstate(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "object.__setstate__() takes exactly two arguments",
            ));
        }
        let Value::Instance(instance) = &args[0] else {
            return Err(RuntimeError::new(
                "object.__setstate__() requires an instance receiver",
            ));
        };
        let apply_state_dict =
            |instance: &ObjRef, state: &ObjRef| -> Result<(), RuntimeError> {
                let entries: Vec<(Value, Value)> = match &*state.kind() {
                    Object::Dict(entries) => entries.iter().cloned().collect(),
                    _ => {
                        return Err(RuntimeError::new(
                            "state dictionary must be a dict object",
                        ))
                    }
                };
                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                    for (key, value) in entries {
                        let Value::Str(name) = key else {
                            return Err(RuntimeError::new(
                                "state dictionary keys must be strings",
                            ));
                        };
                        instance_data.attrs.insert(name, value);
                    }
                    Ok(())
                } else {
                    Err(RuntimeError::new(
                        "object.__setstate__() requires an instance receiver",
                    ))
                }
            };

        match &args[1] {
            Value::None => Ok(Value::None),
            Value::Dict(dict) => {
                apply_state_dict(instance, dict)?;
                Ok(Value::None)
            }
            Value::Tuple(tuple_obj) => {
                let Object::Tuple(parts) = &*tuple_obj.kind() else {
                    return Err(RuntimeError::new("invalid state payload"));
                };
                if parts.len() != 2 {
                    return Err(RuntimeError::new("invalid state payload"));
                }
                match &parts[0] {
                    Value::None => {}
                    Value::Dict(dict) => apply_state_dict(instance, dict)?,
                    _ => return Err(RuntimeError::new("invalid state payload")),
                }
                match &parts[1] {
                    Value::None => {}
                    Value::Dict(dict) => apply_state_dict(instance, dict)?,
                    _ => return Err(RuntimeError::new("invalid state payload")),
                }
                Ok(Value::None)
            }
            _ => Err(RuntimeError::new("invalid state payload")),
        }
    }

    fn reduce_ex_constructor_and_args(&self, value: &Value) -> (Value, Value) {
        match value {
            Value::Dict(dict_obj) => {
                if let Some(default_factory) = self.defaultdict_factories.get(&dict_obj.id()) {
                    let args = if matches!(default_factory, Value::None) {
                        Vec::new()
                    } else {
                        vec![default_factory.clone()]
                    };
                    return (
                        Value::Builtin(BuiltinFunction::CollectionsDefaultDict),
                        self.heap.alloc_tuple(args),
                    );
                }
                (
                    Value::Builtin(BuiltinFunction::Dict),
                    self.heap.alloc_tuple(vec![value.clone()]),
                )
            }
            Value::Bool(_)
            | Value::Int(_)
            | Value::BigInt(_)
            | Value::Float(_)
            | Value::Complex { .. }
            | Value::Str(_)
            | Value::Bytes(_)
            | Value::List(_)
            | Value::Tuple(_)
            | Value::Set(_)
            | Value::FrozenSet(_) => {
                let constructor = self
                    .class_of_value(value)
                    .map(Value::Class)
                    .unwrap_or_else(|| match value {
                        Value::Bool(_) => Value::Builtin(BuiltinFunction::Bool),
                        Value::Int(_) | Value::BigInt(_) => Value::Builtin(BuiltinFunction::Int),
                        Value::Float(_) => Value::Builtin(BuiltinFunction::Float),
                        Value::Complex { .. } => Value::Builtin(BuiltinFunction::Complex),
                        Value::Str(_) => Value::Builtin(BuiltinFunction::Str),
                        Value::Bytes(_) => Value::Builtin(BuiltinFunction::Bytes),
                        Value::List(_) => Value::Builtin(BuiltinFunction::List),
                        Value::Tuple(_) => Value::Builtin(BuiltinFunction::Tuple),
                        Value::Set(_) => Value::Builtin(BuiltinFunction::Set),
                        Value::FrozenSet(_) => Value::Builtin(BuiltinFunction::FrozenSet),
                        _ => Value::Builtin(BuiltinFunction::ObjectNew),
                    });
                (
                    constructor,
                    self.heap.alloc_tuple(vec![value.clone()]),
                )
            }
            Value::Exception(exception) => {
                let args = exception
                    .attrs
                    .borrow()
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| {
                        if let Some(message) = &exception.message {
                            self.heap.alloc_tuple(vec![Value::Str(message.clone())])
                        } else {
                            self.heap.alloc_tuple(Vec::new())
                        }
                    });
                (Value::ExceptionType(exception.name.clone()), args)
            }
            Value::ByteArray(obj) => {
                let payload = match &*obj.kind() {
                    Object::ByteArray(values) => values.iter().map(|value| *value as char).collect(),
                    _ => String::new(),
                };
                let constructor = self
                    .class_of_value(value)
                    .map(Value::Class)
                    .unwrap_or(Value::Builtin(BuiltinFunction::ByteArray));
                (
                    constructor,
                    self.heap
                        .alloc_tuple(vec![Value::Str(payload), Value::Str("latin-1".to_string())]),
                )
            }
            Value::Iterator(obj) => match &*obj.kind() {
                Object::Iterator(IteratorObject {
                    kind:
                        IteratorKind::Map {
                            func,
                            sources,
                            ..
                        },
                    ..
                }) => {
                    let mut args = Vec::with_capacity(1 + sources.len());
                    args.push(func.clone());
                    args.extend(sources.clone());
                    (
                        Value::Builtin(BuiltinFunction::Map),
                        self.heap.alloc_tuple(args),
                    )
                }
                Object::Iterator(IteratorObject {
                    kind: IteratorKind::RangeObject { start, stop, step },
                    ..
                }) => (
                    Value::Builtin(BuiltinFunction::Range),
                    self.heap.alloc_tuple(vec![
                        value_from_bigint(start.clone()),
                        value_from_bigint(stop.clone()),
                        value_from_bigint(step.clone()),
                    ]),
                ),
                _ => {
                    let constructor = self
                        .class_of_value(value)
                        .map(Value::Class)
                        .unwrap_or(Value::Builtin(BuiltinFunction::ObjectNew));
                    (constructor, self.heap.alloc_tuple(Vec::new()))
                }
            },
            _ => {
                let constructor = self
                    .class_of_value(value)
                    .map(Value::Class)
                    .unwrap_or(Value::Builtin(BuiltinFunction::ObjectNew));
                (constructor, self.heap.alloc_tuple(Vec::new()))
            }
        }
    }

    pub(in crate::vm) fn builtin_object_reduce_ex(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !(1..=2).contains(&args.len()) {
            return Err(RuntimeError::new(
                "object.__reduce_ex__() takes one or two arguments",
            ));
        }
        let value = args[0].clone();
        let mut protocol = 0;
        if args.len() == 2 {
            protocol = value_to_int(args[1].clone())?;
        }
        if let Value::Builtin(builtin) = &value {
            if matches!(
                builtin,
                BuiltinFunction::DictFromKeys
                    | BuiltinFunction::BytesMakeTrans
                    | BuiltinFunction::StrMakeTrans
            ) {
                return Err(RuntimeError::new(
                    "TypeError: cannot pickle method_descriptor objects",
                ));
            }
            return Ok(Value::Str(self.builtin_attribute_qualname(*builtin)));
        }
        if let Some(name) = self.object_reduce_ex_builtin_singleton_name(&value) {
            return Ok(Value::Str(name.to_string()));
        }
        if let Value::Instance(instance) = &value {
            if let Some(class_name) = class_name_for_instance(instance) {
                if class_name == "__csv_dialect__" {
                    return Err(RuntimeError::new("cannot pickle 'Dialect' instances"));
                }
            }
        }
        if let Some(reduced) = self.object_reduce_ex_custom_reduce(&value)? {
            return Ok(reduced);
        }

        let (constructor, constructor_args) = if protocol < 2 {
            match self.object_reduce_ex_legacy_constructor_and_args(&value)? {
                Some(pair) => pair,
                None => self.reduce_ex_constructor_and_args(&value),
            }
        } else {
            match self.object_reduce_ex_new_constructor_and_args(&value, protocol)? {
                Some(pair) => pair,
                None => self.reduce_ex_constructor_and_args(&value),
            }
        };
        let state = match value {
            Value::Instance(_) => self.builtin_object_getstate(vec![value], HashMap::new())?,
            _ => Value::None,
        };
        Ok(self
            .heap
            .alloc_tuple(vec![constructor, constructor_args, state]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{ClassObject, InstanceObject, Object};

    fn tuple_values(value: &Value) -> Vec<Value> {
        let Value::Tuple(obj) = value else {
            panic!("expected tuple value, got {value:?}");
        };
        let kind = obj.kind();
        let Object::Tuple(values) = &*kind else {
            panic!("expected tuple object");
        };
        values.clone()
    }

    fn alloc_instance_with_attrs(
        vm: &mut Vm,
        class_name: &str,
        attrs: &[(&str, Value)],
    ) -> Value {
        let class = match vm
            .heap
            .alloc_class(ClassObject::new(class_name.to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            other => panic!("expected class allocation, got {other:?}"),
        };
        let mut instance = InstanceObject::new(class);
        for (name, value) in attrs {
            instance.attrs.insert((*name).to_string(), value.clone());
        }
        vm.heap.alloc_instance(instance)
    }

    #[test]
    fn object_getstate_returns_none_for_non_instance_values() {
        let vm = Vm::new();
        let state = vm
            .builtin_object_getstate(vec![Value::Int(7)], HashMap::new())
            .expect("object.__getstate__ should succeed");
        assert_eq!(state, Value::None);
    }

    #[test]
    fn object_getstate_returns_instance_dict_payload() {
        let mut vm = Vm::new();
        let instance = alloc_instance_with_attrs(
            &mut vm,
            "Point",
            &[("x", Value::Int(4)), ("y", Value::Int(9))],
        );
        let state = vm
            .builtin_object_getstate(vec![instance], HashMap::new())
            .expect("object.__getstate__ should return state");
        let Value::Dict(dict) = state else {
            panic!("expected dict state");
        };
        let kind = dict.kind();
        let Object::Dict(entries) = &*kind else {
            panic!("expected dict object");
        };
        assert_eq!(entries.find(&Value::Str("x".to_string())), Some(&Value::Int(4)));
        assert_eq!(entries.find(&Value::Str("y".to_string())), Some(&Value::Int(9)));
    }

    #[test]
    fn object_reduce_ex_returns_tuple_for_builtin_payload() {
        let mut vm = Vm::new();
        let reduced = vm
            .builtin_object_reduce_ex(vec![Value::Int(7), Value::Int(4)], HashMap::new())
            .expect("object.__reduce_ex__ should succeed");
        let parts = tuple_values(&reduced);
        assert_eq!(parts.len(), 3);
        let constructor_args = tuple_values(&parts[1]);
        assert_eq!(constructor_args, vec![Value::Int(7)]);
        assert_eq!(parts[2], Value::None);
    }

    #[test]
    fn object_reduce_ex_bytearray_uses_latin1_constructor_args() {
        let mut vm = Vm::new();
        let payload = vm.heap.alloc_bytearray(vec![0x78, 0x79, 0x7A, 0xFF]);
        let reduced = vm
            .builtin_object_reduce_ex(vec![payload, Value::Int(0)], HashMap::new())
            .expect("object.__reduce_ex__ should succeed");
        let parts = tuple_values(&reduced);
        let constructor_args = tuple_values(&parts[1]);
        assert_eq!(
            constructor_args,
            vec![
                Value::Str("xyz\u{ff}".to_string()),
                Value::Str("latin-1".to_string())
            ]
        );
        assert_eq!(parts[2], Value::None);
    }

    #[test]
    fn object_reduce_ex_validates_arity_protocol_and_dialect_instances() {
        let mut vm = Vm::new();
        let arity_err = vm
            .builtin_object_reduce_ex(Vec::new(), HashMap::new())
            .expect_err("missing self should fail");
        assert!(
            arity_err
                .message
                .contains("object.__reduce_ex__() takes one or two arguments")
        );

        vm.builtin_object_reduce_ex(
            vec![Value::Int(1), Value::Str("bad".to_string())],
            HashMap::new(),
        )
        .expect_err("non-integer protocol should fail");

        let dialect = alloc_instance_with_attrs(&mut vm, "__csv_dialect__", &[]);
        let dialect_err = vm
            .builtin_object_reduce_ex(vec![dialect, Value::Int(4)], HashMap::new())
            .expect_err("dialect pickling should fail");
        assert!(dialect_err.message.contains("cannot pickle 'Dialect' instances"));
    }

    #[test]
    fn object_reduce_ex_returns_names_for_builtin_singletons() {
        let mut vm = Vm::new();
        let ellipsis = vm
            .builtins
            .get("Ellipsis")
            .cloned()
            .expect("Ellipsis should be installed");
        let reduced_ellipsis = vm
            .builtin_object_reduce_ex(vec![ellipsis, Value::Int(4)], HashMap::new())
            .expect("Ellipsis reduce should succeed");
        assert_eq!(reduced_ellipsis, Value::Str("Ellipsis".to_string()));

        let not_implemented = vm
            .builtins
            .get("NotImplemented")
            .cloned()
            .expect("NotImplemented should be installed");
        let reduced_not_implemented = vm
            .builtin_object_reduce_ex(vec![not_implemented, Value::Int(4)], HashMap::new())
            .expect("NotImplemented reduce should succeed");
        assert_eq!(
            reduced_not_implemented,
            Value::Str("NotImplemented".to_string())
        );
    }
}
