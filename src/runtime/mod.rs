//! Runtime object model (stubbed).

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::bytecode::CodeObject;

#[derive(Debug)]
pub struct ModuleObject {
    pub name: String,
    pub globals: HashMap<String, Value>,
}

impl ModuleObject {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            globals: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FunctionObject {
    pub code: Rc<CodeObject>,
    pub module: ObjRef,
    pub defaults: Vec<Value>,
    pub kwonly_defaults: HashMap<String, Value>,
}

impl FunctionObject {
    pub fn new(
        code: Rc<CodeObject>,
        module: ObjRef,
        defaults: Vec<Value>,
        kwonly_defaults: HashMap<String, Value>,
    ) -> Self {
        Self {
            code,
            module,
            defaults,
            kwonly_defaults,
        }
    }
}

#[derive(Debug)]
pub struct ClassObject {
    pub name: String,
    pub bases: Vec<ObjRef>,
    pub attrs: HashMap<String, Value>,
}

impl ClassObject {
    pub fn new(name: impl Into<String>, bases: Vec<ObjRef>) -> Self {
        Self {
            name: name.into(),
            bases,
            attrs: HashMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct InstanceObject {
    pub class: ObjRef,
    pub attrs: HashMap<String, Value>,
}

impl InstanceObject {
    pub fn new(class: ObjRef) -> Self {
        Self {
            class,
            attrs: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BoundMethod {
    pub function: ObjRef,
    pub receiver: ObjRef,
}

impl BoundMethod {
    pub fn new(function: ObjRef, receiver: ObjRef) -> Self {
        Self { function, receiver }
    }
}

#[derive(Debug)]
pub struct Obj {
    id: u64,
    kind: RefCell<Object>,
}

#[derive(Debug, Clone)]
pub struct ObjRef(Rc<Obj>);

impl ObjRef {
    pub fn id(&self) -> u64 {
        self.0.id
    }

    pub fn kind(&self) -> Ref<'_, Object> {
        self.0.kind.borrow()
    }

    pub fn kind_mut(&self) -> RefMut<'_, Object> {
        self.0.kind.borrow_mut()
    }

    pub fn strong_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }

    pub fn downgrade(&self) -> Weak<Obj> {
        Rc::downgrade(&self.0)
    }

    pub fn from_rc(rc: Rc<Obj>) -> Self {
        Self(rc)
    }
}

#[derive(Debug)]
pub enum Object {
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Dict(Vec<(Value, Value)>),
    Iterator(IteratorObject),
    Module(ModuleObject),
    Class(ClassObject),
    Instance(InstanceObject),
    BoundMethod(BoundMethod),
    Function(FunctionObject),
}

#[derive(Debug, Clone)]
pub struct IteratorObject {
    pub kind: IteratorKind,
    pub index: usize,
}

#[derive(Debug, Clone)]
pub enum IteratorKind {
    List(ObjRef),
    Tuple(ObjRef),
    Str(String),
    Dict(ObjRef),
}

#[derive(Debug)]
pub struct Heap {
    next_id: Cell<u64>,
    registry: RefCell<Vec<Weak<Obj>>>,
    immediate_ids: RefCell<HashMap<ImmediateKey, u64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ImmediateKey {
    None,
    Bool(bool),
    Int(i64),
    Str(String),
    Code(u64),
    Exception(String, Option<String>),
    ExceptionType(String),
    Slice(Option<i64>, Option<i64>, Option<i64>),
    Builtin(BuiltinFunction),
}

impl Heap {
    pub fn new() -> Self {
        Self {
            next_id: Cell::new(1),
            registry: RefCell::new(Vec::new()),
            immediate_ids: RefCell::new(HashMap::new()),
        }
    }

    fn next_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    pub fn alloc(&self, kind: Object) -> ObjRef {
        let id = self.next_id();
        let obj = Rc::new(Obj {
            id,
            kind: RefCell::new(kind),
        });
        self.registry.borrow_mut().push(Rc::downgrade(&obj));
        ObjRef(obj)
    }

    pub fn alloc_list(&self, values: Vec<Value>) -> Value {
        Value::List(self.alloc(Object::List(values)))
    }

    pub fn alloc_tuple(&self, values: Vec<Value>) -> Value {
        Value::Tuple(self.alloc(Object::Tuple(values)))
    }

    pub fn alloc_dict(&self, values: Vec<(Value, Value)>) -> Value {
        Value::Dict(self.alloc(Object::Dict(values)))
    }

    pub fn alloc_module(&self, module: ModuleObject) -> Value {
        Value::Module(self.alloc(Object::Module(module)))
    }

    pub fn alloc_class(&self, class: ClassObject) -> Value {
        Value::Class(self.alloc(Object::Class(class)))
    }

    pub fn alloc_instance(&self, instance: InstanceObject) -> Value {
        Value::Instance(self.alloc(Object::Instance(instance)))
    }

    pub fn alloc_function(&self, function: FunctionObject) -> Value {
        Value::Function(self.alloc(Object::Function(function)))
    }

    pub fn alloc_bound_method(&self, method: BoundMethod) -> Value {
        Value::BoundMethod(self.alloc(Object::BoundMethod(method)))
    }

    pub fn alloc_iterator(&self, iterator: IteratorObject) -> Value {
        Value::Iterator(self.alloc(Object::Iterator(iterator)))
    }

    pub fn id_of(&self, value: &Value) -> u64 {
        match value {
            Value::None => self.id_for_immediate(ImmediateKey::None),
            Value::Bool(value) => self.id_for_immediate(ImmediateKey::Bool(*value)),
            Value::Int(value) => self.id_for_immediate(ImmediateKey::Int(*value)),
            Value::Str(value) => self.id_for_immediate(ImmediateKey::Str(value.clone())),
            Value::List(obj)
            | Value::Tuple(obj)
            | Value::Dict(obj)
            | Value::Iterator(obj)
            | Value::Module(obj)
            | Value::Class(obj)
            | Value::Instance(obj)
            | Value::Function(obj)
            | Value::BoundMethod(obj) => obj.id(),
            Value::Exception(exception) => {
                self.id_for_immediate(ImmediateKey::Exception(
                    exception.name.clone(),
                    exception.message.clone(),
                ))
            }
            Value::ExceptionType(name) => {
                self.id_for_immediate(ImmediateKey::ExceptionType(name.clone()))
            }
            Value::Slice { lower, upper, step } => {
                self.id_for_immediate(ImmediateKey::Slice(*lower, *upper, *step))
            }
            Value::Builtin(builtin) => self.id_for_immediate(ImmediateKey::Builtin(*builtin)),
            Value::Code(code) => {
                let addr = Rc::as_ptr(code) as usize as u64;
                self.id_for_immediate(ImmediateKey::Code(addr))
            }
        }
    }

    fn id_for_immediate(&self, key: ImmediateKey) -> u64 {
        let mut map = self.immediate_ids.borrow_mut();
        if let Some(id) = map.get(&key) {
            return *id;
        }
        let id = self.next_id();
        map.insert(key, id);
        id
    }

    pub fn collect_cycles(&self, roots: &[Value]) {
        let mut marked = HashMap::new();
        let mut stack: Vec<ObjRef> = Vec::new();

        for value in roots {
            trace_value(value, &mut stack, &mut marked);
        }

        while let Some(obj) = stack.pop() {
            let id = obj.id();
            if marked.insert(id, true).is_some() {
                continue;
            }
            trace_object(&obj, &mut stack, &mut marked);
        }

        let mut registry = self.registry.borrow_mut();
        registry.retain(|weak| weak.strong_count() > 0);
        for weak in registry.iter() {
            if let Some(obj) = weak.upgrade() {
                let obj_ref = ObjRef::from_rc(obj);
                if !marked.contains_key(&obj_ref.id()) {
                    clear_object_refs(&obj_ref);
                }
            }
        }
    }

    pub fn live_objects_count(&self) -> usize {
        self.registry
            .borrow()
            .iter()
            .filter(|weak| weak.strong_count() > 0)
            .count()
    }
}

fn trace_value(value: &Value, stack: &mut Vec<ObjRef>, marked: &mut HashMap<u64, bool>) {
    match value {
        Value::List(obj)
        | Value::Tuple(obj)
        | Value::Dict(obj)
        | Value::Iterator(obj)
        | Value::Module(obj)
        | Value::Class(obj)
        | Value::Instance(obj)
        | Value::Function(obj)
        | Value::BoundMethod(obj) => {
            let id = obj.id();
            if marked.contains_key(&id) {
                return;
            }
            stack.push(obj.clone());
        }
        _ => {}
    }
}

fn trace_object(obj: &ObjRef, stack: &mut Vec<ObjRef>, marked: &mut HashMap<u64, bool>) {
    match &*obj.kind() {
        Object::List(values) | Object::Tuple(values) => {
            for value in values {
                trace_value(value, stack, marked);
            }
        }
        Object::Dict(entries) => {
            for (key, value) in entries {
                for item in [key, value] {
                    trace_value(item, stack, marked);
                }
            }
        }
        Object::Iterator(iterator) => match &iterator.kind {
            IteratorKind::List(list)
            | IteratorKind::Tuple(list)
            | IteratorKind::Dict(list) => {
                stack.push(list.clone());
            }
            IteratorKind::Str(_) => {}
        },
        Object::Module(module) => {
            for value in module.globals.values() {
                trace_value(value, stack, marked);
            }
        }
        Object::Class(class) => {
            for base in &class.bases {
                stack.push(base.clone());
            }
            for value in class.attrs.values() {
                trace_value(value, stack, marked);
            }
        }
        Object::Instance(instance) => {
            stack.push(instance.class.clone());
            for value in instance.attrs.values() {
                trace_value(value, stack, marked);
            }
        }
        Object::Function(func) => {
            stack.push(func.module.clone());
            for value in &func.defaults {
                trace_value(value, stack, marked);
            }
            for value in func.kwonly_defaults.values() {
                trace_value(value, stack, marked);
            }
            for value in &func.code.constants {
                trace_value(value, stack, marked);
            }
        }
        Object::BoundMethod(method) => {
            stack.push(method.function.clone());
            stack.push(method.receiver.clone());
        }
    }
}

fn clear_object_refs(obj: &ObjRef) {
    match &mut *obj.kind_mut() {
        Object::List(values) | Object::Tuple(values) => {
            values.clear();
        }
        Object::Dict(entries) => {
            entries.clear();
        }
        Object::Iterator(iterator) => match &mut iterator.kind {
            IteratorKind::List(_)
            | IteratorKind::Tuple(_)
            | IteratorKind::Dict(_) => {
                iterator.kind = IteratorKind::Str(String::new());
                iterator.index = 0;
            }
            IteratorKind::Str(value) => {
                value.clear();
                iterator.index = 0;
            }
        },
        Object::Module(module) => {
            module.globals.clear();
        }
        Object::Class(class) => {
            class.bases.clear();
            class.attrs.clear();
        }
        Object::Instance(instance) => {
            instance.attrs.clear();
        }
        Object::Function(func) => {
            func.defaults.clear();
            func.kwonly_defaults.clear();
        }
        Object::BoundMethod(_) => {}
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Str(String),
    List(ObjRef),
    Tuple(ObjRef),
    Dict(ObjRef),
    Iterator(ObjRef),
    Module(ObjRef),
    Class(ObjRef),
    Instance(ObjRef),
    BoundMethod(ObjRef),
    Function(ObjRef),
    Exception(ExceptionObject),
    ExceptionType(String),
    Slice {
        lower: Option<i64>,
        upper: Option<i64>,
        step: Option<i64>,
    },
    Code(Rc<CodeObject>),
    Builtin(BuiltinFunction),
}

impl Value {
    pub fn as_list(&self) -> Option<Vec<Value>> {
        match self {
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => Some(values.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn as_tuple(&self) -> Option<Vec<Value>> {
        match self {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => Some(values.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<Vec<(Value, Value)>> {
        match self {
            Value::Dict(obj) => match &*obj.kind() {
                Object::Dict(values) => Some(values.clone()),
                _ => None,
            },
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionObject {
    pub name: String,
    pub message: Option<String>,
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
            (Value::List(a), Value::List(b)) => match (&*a.kind(), &*b.kind()) {
                (Object::List(left), Object::List(right)) => left == right,
                _ => false,
            },
            (Value::Tuple(a), Value::Tuple(b)) => match (&*a.kind(), &*b.kind()) {
                (Object::Tuple(left), Object::Tuple(right)) => left == right,
                _ => false,
            },
            (Value::Dict(a), Value::Dict(b)) => match (&*a.kind(), &*b.kind()) {
                (Object::Dict(left), Object::Dict(right)) => left == right,
                _ => false,
            },
            (Value::Iterator(a), Value::Iterator(b)) => a.id() == b.id(),
            (Value::Module(a), Value::Module(b))
            | (Value::Class(a), Value::Class(b))
            | (Value::Instance(a), Value::Instance(b))
            | (Value::Function(a), Value::Function(b))
            | (Value::BoundMethod(a), Value::BoundMethod(b)) => a.id() == b.id(),
            (Value::Exception(a), Value::Exception(b)) => a == b,
            (Value::ExceptionType(a), Value::ExceptionType(b)) => a == b,
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
            (Value::Builtin(a), Value::Builtin(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
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
    Pow,
    List,
    Tuple,
    DivMod,
    Sorted,
    Enumerate,
    BuildClass,
    Id,
}

impl BuiltinFunction {
    pub fn call(self, heap: &Heap, args: Vec<Value>) -> Result<Value, RuntimeError> {
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
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => Ok(Value::Int(values.len() as i64)),
                        _ => Err(RuntimeError::new("len() unsupported type")),
                    },
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => Ok(Value::Int(values.len() as i64)),
                        _ => Err(RuntimeError::new("len() unsupported type")),
                    },
                    Value::Dict(obj) => match &*obj.kind() {
                        Object::Dict(values) => Ok(Value::Int(values.len() as i64)),
                        _ => Err(RuntimeError::new("len() unsupported type")),
                    },
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

                Ok(heap.alloc_list(values))
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
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => {
                            for value in values {
                                total += value_to_int(value.clone())?;
                            }
                        }
                        _ => return Err(RuntimeError::new("sum() expects list or tuple")),
                    },
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => {
                            for value in values {
                                total += value_to_int(value.clone())?;
                            }
                        }
                        _ => return Err(RuntimeError::new("sum() expects list or tuple")),
                    },
                    _ => return Err(RuntimeError::new("sum() expects list or tuple")),
                }

                Ok(Value::Int(total))
            }
            BuiltinFunction::Min => builtin_min_max(args, Ordering::Less),
            BuiltinFunction::Max => builtin_min_max(args, Ordering::Greater),
            BuiltinFunction::All => builtin_all_any(args, true),
            BuiltinFunction::Any => builtin_all_any(args, false),
            BuiltinFunction::Pow => {
                if args.len() < 2 || args.len() > 3 {
                    return Err(RuntimeError::new("pow() expects 2-3 arguments"));
                }
                let base = value_to_int(args[0].clone())?;
                let exp = value_to_int(args[1].clone())?;
                if exp < 0 {
                    return Err(RuntimeError::new("pow() negative exponent unsupported"));
                }
                let mut value = base.pow(exp as u32);
                if args.len() == 3 {
                    let modu = value_to_int(args[2].clone())?;
                    if modu == 0 {
                        return Err(RuntimeError::new("pow() modulo by zero"));
                    }
                    value = value.rem_euclid(modu);
                }
                Ok(Value::Int(value))
            }
            BuiltinFunction::List => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("list() expects at most one argument"));
                }
                if args.is_empty() {
                    return Ok(heap.alloc_list(Vec::new()));
                }
                match &args[0] {
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => Ok(heap.alloc_list(values.clone())),
                        _ => Err(RuntimeError::new("list() unsupported type")),
                    },
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => Ok(heap.alloc_list(values.clone())),
                        _ => Err(RuntimeError::new("list() unsupported type")),
                    },
                    Value::Str(value) => Ok(heap.alloc_list(
                        value
                            .chars()
                            .map(|ch| Value::Str(ch.to_string()))
                            .collect(),
                    )),
                    _ => Err(RuntimeError::new("list() unsupported type")),
                }
            }
            BuiltinFunction::Tuple => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("tuple() expects at most one argument"));
                }
                if args.is_empty() {
                    return Ok(heap.alloc_tuple(Vec::new()));
                }
                match &args[0] {
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => Ok(heap.alloc_tuple(values.clone())),
                        _ => Err(RuntimeError::new("tuple() unsupported type")),
                    },
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => Ok(heap.alloc_tuple(values.clone())),
                        _ => Err(RuntimeError::new("tuple() unsupported type")),
                    },
                    Value::Str(value) => Ok(heap.alloc_tuple(
                        value
                            .chars()
                            .map(|ch| Value::Str(ch.to_string()))
                            .collect(),
                    )),
                    _ => Err(RuntimeError::new("tuple() unsupported type")),
                }
            }
            BuiltinFunction::DivMod => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("divmod() expects two arguments"));
                }
                let left = value_to_int(args[0].clone())?;
                let right = value_to_int(args[1].clone())?;
                if right == 0 {
                    return Err(RuntimeError::new("divmod() division by zero"));
                }
                let div = left.div_euclid(right);
                let rem = left.rem_euclid(right);
                Ok(heap.alloc_tuple(vec![Value::Int(div), Value::Int(rem)]))
            }
            BuiltinFunction::Sorted => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("sorted() expects one argument"));
                }
                match &args[0] {
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => {
                            let mut result = values.clone();
                            let all_numeric =
                                result.iter().all(|value| numeric_value(value).is_some());
                            let all_str =
                                result.iter().all(|value| matches!(value, Value::Str(_)));

                            if all_numeric {
                                result.sort_by(|a, b| {
                                    let left = numeric_value(a).unwrap();
                                    let right = numeric_value(b).unwrap();
                                    left.cmp(&right)
                                });
                            } else if all_str {
                                result.sort_by(|a, b| match (a, b) {
                                    (Value::Str(a), Value::Str(b)) => a.cmp(b),
                                    _ => Ordering::Equal,
                                });
                            } else {
                                return Err(RuntimeError::new(
                                    "sorted() expects list/tuple of comparable values",
                                ));
                            }

                            Ok(heap.alloc_list(result))
                        }
                        _ => Err(RuntimeError::new("sorted() expects list or tuple")),
                    },
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => {
                            let mut result = values.clone();
                        let all_numeric = result.iter().all(|value| numeric_value(value).is_some());
                        let all_str = result.iter().all(|value| matches!(value, Value::Str(_)));

                        if all_numeric {
                            result.sort_by(|a, b| {
                                let left = numeric_value(a).unwrap();
                                let right = numeric_value(b).unwrap();
                                left.cmp(&right)
                            });
                        } else if all_str {
                            result.sort_by(|a, b| match (a, b) {
                                (Value::Str(a), Value::Str(b)) => a.cmp(b),
                                _ => Ordering::Equal,
                            });
                        } else {
                            return Err(RuntimeError::new(
                                "sorted() expects list/tuple of comparable values",
                            ));
                        }

                            Ok(heap.alloc_list(result))
                        }
                        _ => Err(RuntimeError::new("sorted() expects list or tuple")),
                    },
                    _ => Err(RuntimeError::new("sorted() expects list or tuple")),
                }
            }
            BuiltinFunction::Enumerate => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("enumerate() expects 1-2 arguments"));
                }
                let start = if args.len() == 2 {
                    value_to_int(args[1].clone())?
                } else {
                    0
                };
                let mut entries = Vec::new();
                match &args[0] {
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => {
                            for (idx, value) in values.iter().cloned().enumerate() {
                                let index = start + idx as i64;
                                entries.push(heap.alloc_tuple(vec![Value::Int(index), value]));
                            }
                        }
                        _ => return Err(RuntimeError::new("enumerate() expects iterable")),
                    },
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => {
                            for (idx, value) in values.iter().cloned().enumerate() {
                                let index = start + idx as i64;
                                entries.push(heap.alloc_tuple(vec![Value::Int(index), value]));
                            }
                        }
                        _ => return Err(RuntimeError::new("enumerate() expects iterable")),
                    },
                    Value::Str(value) => {
                        for (idx, ch) in value.chars().enumerate() {
                            let index = start + idx as i64;
                            entries.push(heap.alloc_tuple(vec![
                                Value::Int(index),
                                Value::Str(ch.to_string()),
                            ]));
                        }
                    }
                    _ => return Err(RuntimeError::new("enumerate() expects iterable")),
                }
                Ok(heap.alloc_list(entries))
            }
            BuiltinFunction::BuildClass => Err(RuntimeError::new(
                "__build_class__ is only available in the VM",
            )),
            BuiltinFunction::Id => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("id() expects one argument"));
                }
                let id = heap.id_of(&args[0]);
                Ok(Value::Int(id as i64))
            }
        }
    }
}

fn builtin_all_any(args: Vec<Value>, expect_all: bool) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new("all/any expects one argument"));
    }
    match &args[0] {
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => {
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
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => {
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
        },
        _ => Err(RuntimeError::new("all/any expects list or tuple")),
    }
}

fn builtin_min_max(args: Vec<Value>, preferred: Ordering) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("min/max expects at least one argument"));
    }

    let mut values: Vec<Value> = if args.len() == 1 {
        match &args[0] {
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => values.clone(),
                _ => return Err(RuntimeError::new("min/max expects list or tuple")),
            },
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::new("min/max expects list or tuple")),
            },
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

pub fn format_value(value: &Value) -> String {
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
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => {
                let mut parts = Vec::new();
                for value in values {
                    parts.push(format_value(value));
                }
                format!("[{}]", parts.join(", "))
            }
            _ => "<list>".to_string(),
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => {
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
            _ => "<tuple>".to_string(),
        },
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(values) => {
                let mut parts = Vec::new();
                for (key, value) in values {
                    parts.push(format!("{}: {}", format_value(key), format_value(value)));
                }
                format!("{{{}}}", parts.join(", "))
            }
            _ => "<dict>".to_string(),
        },
        Value::Iterator(_) => "<iterator>".to_string(),
        Value::Module(obj) => match &*obj.kind() {
            Object::Module(module) => format!("<module {}>", module.name),
            _ => "<module ?>".to_string(),
        },
        Value::Class(obj) => match &*obj.kind() {
            Object::Class(class) => format!("<class {}>", class.name),
            _ => "<class ?>".to_string(),
        },
        Value::Instance(obj) => match &*obj.kind() {
            Object::Instance(instance) => match &*instance.class.kind() {
                Object::Class(class) => format!("<{} instance>", class.name),
                _ => "<instance ?>".to_string(),
            },
            _ => "<instance ?>".to_string(),
        },
        Value::BoundMethod(obj) => match &*obj.kind() {
            Object::BoundMethod(method) => match &*method.function.kind() {
                Object::Function(func) => format!("<bound method {}>", func.code.name),
                _ => "<bound method ?>".to_string(),
            },
            _ => "<bound method ?>".to_string(),
        },
        Value::Exception(exception) => match &exception.message {
            Some(message) if !message.is_empty() => format!("{}: {}", exception.name, message),
            _ => exception.name.clone(),
        },
        Value::ExceptionType(name) => format!("<class '{}'>", name),
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
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => !values.is_empty(),
            _ => true,
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => !values.is_empty(),
            _ => true,
        },
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(values) => !values.is_empty(),
            _ => true,
        },
        Value::Iterator(_) => true,
        Value::Slice { .. } => true,
        Value::Module(_)
        | Value::Class(_)
        | Value::Instance(_)
        | Value::BoundMethod(_)
        | Value::Exception(_)
        | Value::ExceptionType(_)
        | Value::Code(_)
        | Value::Function(_)
        | Value::Builtin(_) => true,
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
