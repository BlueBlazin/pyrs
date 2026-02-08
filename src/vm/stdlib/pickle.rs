use super::super::*;

impl Vm {
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

    fn reduce_ex_constructor_and_args(&self, value: &Value) -> (Value, Value) {
        match value {
            Value::Bool(_)
            | Value::Int(_)
            | Value::BigInt(_)
            | Value::Float(_)
            | Value::Complex { .. }
            | Value::Str(_)
            | Value::Bytes(_)
            | Value::ByteArray(_)
            | Value::List(_)
            | Value::Tuple(_)
            | Value::Dict(_)
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
                        Value::ByteArray(_) => Value::Builtin(BuiltinFunction::ByteArray),
                        Value::List(_) => Value::Builtin(BuiltinFunction::List),
                        Value::Tuple(_) => Value::Builtin(BuiltinFunction::Tuple),
                        Value::Dict(_) => Value::Builtin(BuiltinFunction::Dict),
                        Value::Set(_) => Value::Builtin(BuiltinFunction::Set),
                        Value::FrozenSet(_) => Value::Builtin(BuiltinFunction::FrozenSet),
                        _ => Value::Builtin(BuiltinFunction::ObjectNew),
                    });
                (
                    constructor,
                    self.heap.alloc_tuple(vec![value.clone()]),
                )
            }
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
        if args.len() == 2 {
            let _ = value_to_int(args[1].clone())?;
        }
        if let Value::Builtin(builtin) = &value {
            return Ok(Value::Str(self.builtin_runtime_name(*builtin)));
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

        let (constructor, constructor_args) = self.reduce_ex_constructor_and_args(&value);
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
}
