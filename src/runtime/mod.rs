//! Runtime object model (stubbed).

use std::rc::Rc;

use crate::bytecode::CodeObject;

#[derive(Debug, Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<Value>),
    Code(Rc<CodeObject>),
    Function(Rc<CodeObject>),
    Builtin(BuiltinFunction),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::None, Value::None) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Code(a), Value::Code(b)) => Rc::ptr_eq(a, b),
            (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
            (Value::Builtin(a), Value::Builtin(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BuiltinFunction {
    Print,
    Len,
    Range,
}

impl BuiltinFunction {
    pub fn call(self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        match self {
            BuiltinFunction::Print => {
                let mut parts = Vec::new();
                for value in args {
                    parts.push(format_value(&value));
                }
                println!("{}", parts.join(" "));
                Ok(Value::None)
            }
            BuiltinFunction::Len => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("len() expects one argument"));
                }
                match &args[0] {
                    Value::Str(value) => Ok(Value::Int(value.chars().count() as i64)),
                    Value::List(values) => Ok(Value::Int(values.len() as i64)),
                    _ => Err(RuntimeError::new("len() unsupported type")),
                }
            }
            BuiltinFunction::Range => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("range() expects one argument"));
                }
                let stop = match &args[0] {
                    Value::Int(value) => *value,
                    Value::Bool(value) => if *value { 1 } else { 0 },
                    _ => return Err(RuntimeError::new("range() expects integer")),
                };
                if stop < 0 {
                    return Err(RuntimeError::new("range() negative not supported"));
                }
                let mut values = Vec::new();
                for i in 0..stop {
                    values.push(Value::Int(i));
                }
                Ok(Value::List(values))
            }
        }
    }
}

fn format_value(value: &Value) -> String {
    match value {
        Value::None => "None".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Str(value) => value.clone(),
        Value::List(values) => {
            let mut parts = Vec::new();
            for value in values {
                parts.push(format_value(value));
            }
            format!("[{}]", parts.join(", "))
        }
        Value::Code(_) => "<code>".to_string(),
        Value::Function(_) => "<function>".to_string(),
        Value::Builtin(_) => "<builtin>".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
