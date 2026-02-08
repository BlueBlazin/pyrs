use super::super::*;

impl Vm {
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
                    let entries = instance_data
                        .attrs
                        .iter()
                        .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                        .collect::<Vec<_>>();
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
        if let Value::Instance(instance) = &value {
            if let Some(class_name) = class_name_for_instance(instance) {
                if class_name == "__csv_dialect__" {
                    return Err(RuntimeError::new("cannot pickle 'Dialect' instances"));
                }
            }
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
