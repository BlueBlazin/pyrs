//! Runtime object model (stubbed).

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;

use crate::bytecode::CodeObject;

#[derive(Debug)]
pub struct ModuleObject {
    pub name: String,
    pub globals: RefCell<HashMap<String, Value>>,
}

impl ModuleObject {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            globals: RefCell::new(HashMap::new()),
        }
    }
}

#[derive(Debug)]
pub struct FunctionObject {
    pub code: Rc<CodeObject>,
    pub module: Rc<ModuleObject>,
}

impl FunctionObject {
    pub fn new(code: Rc<CodeObject>, module: Rc<ModuleObject>) -> Self {
        Self { code, module }
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Dict(Vec<(Value, Value)>),
    Module(Rc<ModuleObject>),
    Slice {
        lower: Option<i64>,
        upper: Option<i64>,
        step: Option<i64>,
    },
    Code(Rc<CodeObject>),
    Function(Rc<FunctionObject>),
    Builtin(BuiltinFunction),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::None, Value::None) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Bool(a), Value::Int(b)) => (*a as i64) == *b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Int(a), Value::Bool(b)) => *a == (*b as i64),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Tuple(a), Value::Tuple(b)) => a == b,
            (Value::Dict(a), Value::Dict(b)) => a == b,
            (Value::Module(a), Value::Module(b)) => Rc::ptr_eq(a, b),
            (
                Value::Slice {
                    lower: a_lower,
                    upper: a_upper,
                    step: a_step,
                },
                Value::Slice {
                    lower: b_lower,
                    upper: b_upper,
                    step: b_step,
                },
            ) => a_lower == b_lower && a_upper == b_upper && a_step == b_step,
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
    Slice,
    Bool,
    Int,
    Str,
    Abs,
    Sum,
    Min,
    Max,
    All,
    Any,
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
                    Value::Tuple(values) => Ok(Value::Int(values.len() as i64)),
                    Value::Dict(values) => Ok(Value::Int(values.len() as i64)),
                    _ => Err(RuntimeError::new("len() unsupported type")),
                }
            }
            BuiltinFunction::Range => {
                if args.is_empty() || args.len() > 3 {
                    return Err(RuntimeError::new("range() expects 1-3 arguments"));
                }

                let mut nums = Vec::new();
                for arg in &args {
                    match arg {
                        Value::Int(value) => nums.push(*value),
                        Value::Bool(value) => nums.push(if *value { 1 } else { 0 }),
                        _ => return Err(RuntimeError::new("range() expects integers")),
                    }
                }

                let (start, stop, step) = match nums.len() {
                    1 => (0, nums[0], 1),
                    2 => (nums[0], nums[1], 1),
                    _ => (nums[0], nums[1], nums[2]),
                };

                if step == 0 {
                    return Err(RuntimeError::new("range() step cannot be zero"));
                }

                let mut values = Vec::new();
                let mut i = start;
                if step > 0 {
                    while i < stop {
                        values.push(Value::Int(i));
                        i += step;
                    }
                } else {
                    while i > stop {
                        values.push(Value::Int(i));
                        i += step;
                    }
                }

                Ok(Value::List(values))
            }
            BuiltinFunction::Slice => {
                if args.is_empty() || args.len() > 3 {
                    return Err(RuntimeError::new("slice() expects 1-3 arguments"));
                }

                let mut parts = Vec::with_capacity(3);
                for arg in args {
                    match arg {
                        Value::None => parts.push(None),
                        other => parts.push(Some(value_to_int(other)?)),
                    }
                }

                let (lower, upper, step) = match parts.len() {
                    1 => (None, parts[0], None),
                    2 => (parts[0], parts[1], None),
                    _ => (parts[0], parts[1], parts[2]),
                };

                Ok(Value::Slice { lower, upper, step })
            }
            BuiltinFunction::Bool => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("bool() expects one argument"));
                }
                Ok(Value::Bool(is_truthy_value(&args[0])))
            }
            BuiltinFunction::Int => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("int() expects one argument"));
                }
                match &args[0] {
                    Value::Int(value) => Ok(Value::Int(*value)),
                    Value::Bool(value) => Ok(Value::Int(if *value { 1 } else { 0 })),
                    Value::Str(value) => {
                        let trimmed = value.trim();
                        let parsed = trimmed.parse::<i64>().map_err(|_| {
                            RuntimeError::new("int() invalid literal")
                        })?;
                        Ok(Value::Int(parsed))
                    }
                    _ => Err(RuntimeError::new("int() unsupported type")),
                }
            }
            BuiltinFunction::Str => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("str() expects one argument"));
                }
                Ok(Value::Str(format_value(&args[0])))
            }
            BuiltinFunction::Abs => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("abs() expects one argument"));
                }
                match &args[0] {
                    Value::Int(value) => Ok(Value::Int(value.abs())),
                    Value::Bool(value) => Ok(Value::Int(if *value { 1 } else { 0 })),
                    _ => Err(RuntimeError::new("abs() unsupported type")),
                }
            }
            BuiltinFunction::Sum => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("sum() expects 1-2 arguments"));
                }
                let mut total = if args.len() == 2 {
                    value_to_int(args[1].clone())?
                } else {
                    0
                };

                match &args[0] {
                    Value::List(values) | Value::Tuple(values) => {
                        for value in values {
                            total += value_to_int(value.clone())?;
                        }
                    }
                    _ => return Err(RuntimeError::new("sum() expects list or tuple")),
                }

                Ok(Value::Int(total))
            }
            BuiltinFunction::Min => builtin_min_max(args, Ordering::Less),
            BuiltinFunction::Max => builtin_min_max(args, Ordering::Greater),
            BuiltinFunction::All => builtin_all_any(args, true),
            BuiltinFunction::Any => builtin_all_any(args, false),
        }
    }
}

fn builtin_all_any(args: Vec<Value>, expect_all: bool) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new("all/any expects one argument"));
    }
    match &args[0] {
        Value::List(values) | Value::Tuple(values) => {
            let mut result = expect_all;
            for value in values {
                let truthy = is_truthy_value(value);
                if expect_all {
                    if !truthy {
                        result = false;
                        break;
                    }
                } else if truthy {
                    result = true;
                    break;
                }
            }
            Ok(Value::Bool(result))
        }
        _ => Err(RuntimeError::new("all/any expects list or tuple")),
    }
}

fn builtin_min_max(args: Vec<Value>, preferred: Ordering) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("min/max expects at least one argument"));
    }

    let mut values: Vec<Value> = if args.len() == 1 {
        match &args[0] {
            Value::List(values) | Value::Tuple(values) => values.clone(),
            _ => return Err(RuntimeError::new("min/max expects list or tuple")),
        }
    } else {
        args
    };

    if values.is_empty() {
        return Err(RuntimeError::new("min/max arg is an empty sequence"));
    }

    let mut best = values.swap_remove(0);
    for value in values {
        let ordering = compare_values(&value, &best)?;
        if ordering == preferred {
            best = value;
        }
    }
    Ok(best)
}

fn value_to_int(value: Value) -> Result<i64, RuntimeError> {
    match value {
        Value::Int(value) => Ok(value),
        Value::Bool(value) => Ok(if value { 1 } else { 0 }),
        _ => Err(RuntimeError::new("expected integer")),
    }
}

fn numeric_value(value: &Value) -> Option<i64> {
    match value {
        Value::Int(value) => Some(*value),
        Value::Bool(value) => Some(if *value { 1 } else { 0 }),
        _ => None,
    }
}

fn compare_values(left: &Value, right: &Value) -> Result<Ordering, RuntimeError> {
    if let (Some(left), Some(right)) = (numeric_value(left), numeric_value(right)) {
        return Ok(left.cmp(&right));
    }

    match (left, right) {
        (Value::Str(a), Value::Str(b)) => Ok(a.cmp(b)),
        _ => Err(RuntimeError::new("min/max unsupported type")),
    }
}

fn format_value(value: &Value) -> String {
    match value {
        Value::None => "None".to_string(),
        Value::Bool(value) => {
            if *value {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Value::Int(value) => value.to_string(),
        Value::Str(value) => value.clone(),
        Value::List(values) => {
            let mut parts = Vec::new();
            for value in values {
                parts.push(format_value(value));
            }
            format!("[{}]", parts.join(", "))
        }
        Value::Tuple(values) => {
            let mut parts = Vec::new();
            for value in values {
                parts.push(format_value(value));
            }
            if parts.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            }
        }
        Value::Dict(values) => {
            let mut parts = Vec::new();
            for (key, value) in values {
                parts.push(format!("{}: {}", format_value(key), format_value(value)));
            }
            format!("{{{}}}", parts.join(", "))
        }
        Value::Module(module) => format!("<module {}>", module.name),
        Value::Slice { lower, upper, step } => {
            let lower = lower.map_or("None".to_string(), |value| value.to_string());
            let upper = upper.map_or("None".to_string(), |value| value.to_string());
            let step = step.map_or("None".to_string(), |value| value.to_string());
            format!("slice({lower}, {upper}, {step})")
        }
        Value::Code(_) => "<code>".to_string(),
        Value::Function(_) => "<function>".to_string(),
        Value::Builtin(_) => "<builtin>".to_string(),
    }
}

fn is_truthy_value(value: &Value) -> bool {
    match value {
        Value::None => false,
        Value::Bool(value) => *value,
        Value::Int(value) => *value != 0,
        Value::Str(value) => !value.is_empty(),
        Value::List(values) => !values.is_empty(),
        Value::Tuple(values) => !values.is_empty(),
        Value::Dict(values) => !values.is_empty(),
        Value::Slice { .. } => true,
        Value::Module(_) | Value::Code(_) | Value::Function(_) | Value::Builtin(_) => true,
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
