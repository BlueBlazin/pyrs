use super::super::*;

impl Vm {
    pub(in crate::vm) fn builtin_json_dumps(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("dumps() expects one argument"));
        }
        let text = json_serialize_value(&args[0])?;
        Ok(Value::Str(text))
    }

    pub(in crate::vm) fn builtin_json_loads(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("loads() expects one argument"));
        }
        let text = match &args[0] {
            Value::Str(text) => text.clone(),
            _ => return Err(RuntimeError::new("loads() expects a string")),
        };
        let node = parse_json_node(&text)?;
        Ok(json_node_to_value(node, &self.heap))
    }
}
