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
    pub closure: Vec<ObjRef>,
    pub annotations: Option<ObjRef>,
    pub dict: Option<ObjRef>,
}

impl FunctionObject {
    pub fn new(
        code: Rc<CodeObject>,
        module: ObjRef,
        defaults: Vec<Value>,
        kwonly_defaults: HashMap<String, Value>,
        closure: Vec<ObjRef>,
        annotations: Option<ObjRef>,
    ) -> Self {
        Self {
            code,
            module,
            defaults,
            kwonly_defaults,
            closure,
            annotations,
            dict: None,
        }
    }
}

#[derive(Debug)]
pub struct ClassObject {
    pub name: String,
    pub bases: Vec<ObjRef>,
    pub mro: Vec<ObjRef>,
    pub attrs: HashMap<String, Value>,
    pub slots: Option<Vec<String>>,
    pub metaclass: Option<ObjRef>,
}

impl ClassObject {
    pub fn new(name: impl Into<String>, bases: Vec<ObjRef>) -> Self {
        Self {
            name: name.into(),
            bases,
            mro: Vec::new(),
            attrs: HashMap::new(),
            slots: None,
            metaclass: None,
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
pub struct SuperObject {
    pub start_class: ObjRef,
    pub object: ObjRef,
    pub object_type: ObjRef,
}

impl SuperObject {
    pub fn new(start_class: ObjRef, object: ObjRef, object_type: ObjRef) -> Self {
        Self {
            start_class,
            object,
            object_type,
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

#[derive(Debug, Clone)]
pub struct GeneratorObject {
    pub started: bool,
    pub running: bool,
    pub closed: bool,
    pub is_coroutine: bool,
    pub is_async_generator: bool,
}

impl GeneratorObject {
    pub fn new(is_coroutine: bool, is_async_generator: bool) -> Self {
        Self {
            started: false,
            running: false,
            closed: false,
            is_coroutine,
            is_async_generator,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeMethodKind {
    GeneratorIter,
    GeneratorAwait,
    GeneratorANext,
    GeneratorNext,
    GeneratorSend,
    GeneratorThrow,
    GeneratorClose,
    DictKeys,
    DictValues,
    DictItems,
    DictUpdateMethod,
    DictSetDefault,
    DictGet,
    DictPop,
    ListAppend,
    ListExtend,
    ListInsert,
    ListRemove,
    ListCount,
    IntToBytes,
    IntBitLengthMethod,
    StrStartsWith,
    StrReplace,
    StrUpper,
    StrLower,
    StrEncode,
    StrDecode,
    BytesDecode,
    StrRemovePrefix,
    StrRemoveSuffix,
    StrFormat,
    StrIsUpper,
    StrIsSpace,
    StrJoin,
    StrSplit,
    StrLStrip,
    StrRStrip,
    StrStrip,
    SetContains,
    SetAdd,
    SetUpdate,
    RePatternSearch,
    RePatternMatch,
    RePatternFullMatch,
    RePatternSub,
    ClassRegister,
    PropertyGet,
    PropertySet,
    PropertyDelete,
    PropertyGetter,
    PropertySetter,
    PropertyDeleter,
    FunctoolsWrapsDecorator,
    FunctoolsPartialCall,
}

#[derive(Debug, Clone)]
pub struct NativeMethodObject {
    pub kind: NativeMethodKind,
}

impl NativeMethodObject {
    pub fn new(kind: NativeMethodKind) -> Self {
        Self { kind }
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
    Set(Vec<Value>),
    FrozenSet(Vec<Value>),
    Bytes(Vec<u8>),
    ByteArray(Vec<u8>),
    MemoryView(MemoryViewObject),
    Iterator(IteratorObject),
    Generator(GeneratorObject),
    Module(ModuleObject),
    Class(ClassObject),
    Instance(InstanceObject),
    Super(SuperObject),
    BoundMethod(BoundMethod),
    NativeMethod(NativeMethodObject),
    Function(FunctionObject),
    Cell(CellObject),
}

#[derive(Debug)]
pub struct CellObject {
    pub value: Option<Value>,
}

impl CellObject {
    pub fn new(value: Option<Value>) -> Self {
        Self { value }
    }
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
    Set(ObjRef),
    Bytes(ObjRef),
    ByteArray(ObjRef),
    MemoryView(ObjRef),
    Count { current: i64, step: i64 },
}

#[derive(Debug)]
pub struct MemoryViewObject {
    pub source: ObjRef,
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
    Float(u64),
    Complex(u64, u64),
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

    pub fn alloc_set(&self, values: Vec<Value>) -> Value {
        Value::Set(self.alloc(Object::Set(values)))
    }

    pub fn alloc_frozenset(&self, values: Vec<Value>) -> Value {
        Value::FrozenSet(self.alloc(Object::FrozenSet(values)))
    }

    pub fn alloc_bytes(&self, values: Vec<u8>) -> Value {
        Value::Bytes(self.alloc(Object::Bytes(values)))
    }

    pub fn alloc_bytearray(&self, values: Vec<u8>) -> Value {
        Value::ByteArray(self.alloc(Object::ByteArray(values)))
    }

    pub fn alloc_memoryview(&self, source: ObjRef) -> Value {
        Value::MemoryView(self.alloc(Object::MemoryView(MemoryViewObject { source })))
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

    pub fn alloc_super(&self, super_obj: SuperObject) -> Value {
        Value::Super(self.alloc(Object::Super(super_obj)))
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

    pub fn alloc_generator(&self, generator: GeneratorObject) -> Value {
        Value::Generator(self.alloc(Object::Generator(generator)))
    }

    pub fn alloc_native_method(&self, native: NativeMethodObject) -> ObjRef {
        self.alloc(Object::NativeMethod(native))
    }

    pub fn alloc_cell_obj(&self, value: Option<Value>) -> ObjRef {
        self.alloc(Object::Cell(CellObject::new(value)))
    }

    pub fn alloc_cell(&self, value: Option<Value>) -> Value {
        Value::Cell(self.alloc_cell_obj(value))
    }

    pub fn id_of(&self, value: &Value) -> u64 {
        match value {
            Value::None => self.id_for_immediate(ImmediateKey::None),
            Value::Bool(value) => self.id_for_immediate(ImmediateKey::Bool(*value)),
            Value::Int(value) => self.id_for_immediate(ImmediateKey::Int(*value)),
            Value::Float(value) => self.id_for_immediate(ImmediateKey::Float(value.to_bits())),
            Value::Complex { real, imag } => {
                self.id_for_immediate(ImmediateKey::Complex(real.to_bits(), imag.to_bits()))
            }
            Value::Str(value) => self.id_for_immediate(ImmediateKey::Str(value.clone())),
            Value::List(obj)
            | Value::Tuple(obj)
            | Value::Dict(obj)
            | Value::Set(obj)
            | Value::FrozenSet(obj)
            | Value::Bytes(obj)
            | Value::ByteArray(obj)
            | Value::MemoryView(obj)
            | Value::Iterator(obj)
            | Value::Generator(obj)
            | Value::Module(obj)
            | Value::Class(obj)
            | Value::Instance(obj)
            | Value::Super(obj)
            | Value::Function(obj)
            | Value::BoundMethod(obj)
            | Value::Cell(obj) => obj.id(),
            Value::Exception(exception) => self.id_for_immediate(ImmediateKey::Exception(
                exception.name.clone(),
                exception.message.clone(),
            )),
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
        | Value::Set(obj)
        | Value::FrozenSet(obj)
        | Value::Bytes(obj)
        | Value::ByteArray(obj)
        | Value::MemoryView(obj)
        | Value::Iterator(obj)
        | Value::Generator(obj)
        | Value::Module(obj)
        | Value::Class(obj)
        | Value::Instance(obj)
        | Value::Super(obj)
        | Value::Function(obj)
        | Value::BoundMethod(obj)
        | Value::Cell(obj) => {
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
        Object::Set(values) | Object::FrozenSet(values) => {
            for value in values {
                trace_value(value, stack, marked);
            }
        }
        Object::Bytes(_) | Object::ByteArray(_) => {}
        Object::MemoryView(view) => {
            stack.push(view.source.clone());
        }
        Object::Iterator(iterator) => match &iterator.kind {
            IteratorKind::List(list)
            | IteratorKind::Tuple(list)
            | IteratorKind::Dict(list)
            | IteratorKind::Set(list)
            | IteratorKind::Bytes(list)
            | IteratorKind::ByteArray(list)
            | IteratorKind::MemoryView(list) => stack.push(list.clone()),
            IteratorKind::Str(_) | IteratorKind::Count { .. } => {}
        },
        Object::Generator(_) => {}
        Object::Module(module) => {
            for value in module.globals.values() {
                trace_value(value, stack, marked);
            }
        }
        Object::Class(class) => {
            for base in &class.bases {
                stack.push(base.clone());
            }
            if let Some(meta) = &class.metaclass {
                stack.push(meta.clone());
            }
            for entry in &class.mro {
                stack.push(entry.clone());
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
        Object::Super(super_obj) => {
            stack.push(super_obj.start_class.clone());
            stack.push(super_obj.object.clone());
            stack.push(super_obj.object_type.clone());
        }
        Object::Function(func) => {
            stack.push(func.module.clone());
            for value in &func.defaults {
                trace_value(value, stack, marked);
            }
            for value in func.kwonly_defaults.values() {
                trace_value(value, stack, marked);
            }
            for cell in &func.closure {
                stack.push(cell.clone());
            }
            if let Some(annotations) = &func.annotations {
                stack.push(annotations.clone());
            }
            if let Some(dict) = &func.dict {
                stack.push(dict.clone());
            }
            for value in &func.code.constants {
                trace_value(value, stack, marked);
            }
        }
        Object::BoundMethod(method) => {
            stack.push(method.function.clone());
            stack.push(method.receiver.clone());
        }
        Object::NativeMethod(_) => {}
        Object::Cell(cell) => {
            if let Some(value) = &cell.value {
                trace_value(value, stack, marked);
            }
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
        Object::Set(values) | Object::FrozenSet(values) => {
            values.clear();
        }
        Object::Bytes(values) | Object::ByteArray(values) => {
            values.clear();
        }
        Object::MemoryView(_) => {}
        Object::Iterator(iterator) => match &mut iterator.kind {
            IteratorKind::List(_)
            | IteratorKind::Tuple(_)
            | IteratorKind::Dict(_)
            | IteratorKind::Set(_)
            | IteratorKind::Bytes(_)
            | IteratorKind::ByteArray(_)
            | IteratorKind::MemoryView(_) => {
                iterator.kind = IteratorKind::Str(String::new());
                iterator.index = 0;
            }
            IteratorKind::Str(value) => {
                value.clear();
                iterator.index = 0;
            }
            IteratorKind::Count { .. } => {
                iterator.kind = IteratorKind::Str(String::new());
                iterator.index = 0;
            }
        },
        Object::Generator(generator) => {
            generator.started = false;
            generator.running = false;
            generator.closed = true;
        }
        Object::Module(module) => {
            module.globals.clear();
        }
        Object::Class(class) => {
            class.bases.clear();
            class.mro.clear();
            class.attrs.clear();
            class.slots = None;
            class.metaclass = None;
        }
        Object::Instance(instance) => {
            instance.attrs.clear();
        }
        Object::Super(_) => {}
        Object::Function(func) => {
            func.defaults.clear();
            func.kwonly_defaults.clear();
            func.closure.clear();
            func.annotations = None;
            func.dict = None;
        }
        Object::BoundMethod(_) => {}
        Object::NativeMethod(_) => {}
        Object::Cell(cell) => {
            cell.value = None;
        }
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Complex { real: f64, imag: f64 },
    Str(String),
    List(ObjRef),
    Tuple(ObjRef),
    Dict(ObjRef),
    Set(ObjRef),
    FrozenSet(ObjRef),
    Bytes(ObjRef),
    ByteArray(ObjRef),
    MemoryView(ObjRef),
    Iterator(ObjRef),
    Generator(ObjRef),
    Module(ObjRef),
    Class(ObjRef),
    Instance(ObjRef),
    Super(ObjRef),
    BoundMethod(ObjRef),
    Function(ObjRef),
    Cell(ObjRef),
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
    pub cause: Option<Box<ExceptionObject>>,
    pub context: Option<Box<ExceptionObject>>,
    pub suppress_context: bool,
}

impl ExceptionObject {
    pub fn new(name: impl Into<String>, message: Option<String>) -> Self {
        Self {
            name: name.into(),
            message,
            cause: None,
            context: None,
            suppress_context: false,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::None, Value::None) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Bool(a), Value::Int(b)) => (*a as i64) == *b,
            (Value::Bool(a), Value::Float(b)) => (*a as i64 as f64) == *b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Int(a), Value::Bool(b)) => *a == (*b as i64),
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::Float(a), Value::Bool(b)) => *a == (*b as i64 as f64),
            (
                Value::Complex {
                    real: a_real,
                    imag: a_imag,
                },
                Value::Complex {
                    real: b_real,
                    imag: b_imag,
                },
            ) => a_real.to_bits() == b_real.to_bits() && a_imag.to_bits() == b_imag.to_bits(),
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
            (Value::Set(a), Value::Set(b)) | (Value::FrozenSet(a), Value::FrozenSet(b)) => {
                match (&*a.kind(), &*b.kind()) {
                    (Object::Set(left), Object::Set(right))
                    | (Object::FrozenSet(left), Object::FrozenSet(right)) => left == right,
                    _ => false,
                }
            }
            (Value::Bytes(a), Value::Bytes(b))
            | (Value::ByteArray(a), Value::ByteArray(b)) => match (&*a.kind(), &*b.kind()) {
                (Object::Bytes(left), Object::Bytes(right))
                | (Object::ByteArray(left), Object::ByteArray(right)) => left == right,
                _ => false,
            },
            (Value::MemoryView(a), Value::MemoryView(b)) => a.id() == b.id(),
            (Value::Iterator(a), Value::Iterator(b)) => a.id() == b.id(),
            (Value::Generator(a), Value::Generator(b)) => a.id() == b.id(),
            (Value::Module(a), Value::Module(b))
            | (Value::Class(a), Value::Class(b))
            | (Value::Instance(a), Value::Instance(b))
            | (Value::Super(a), Value::Super(b))
            | (Value::Function(a), Value::Function(b))
            | (Value::BoundMethod(a), Value::BoundMethod(b))
            | (Value::Cell(a), Value::Cell(b)) => a.id() == b.id(),
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
    Repr,
    NoOp,
    Len,
    Range,
    Slice,
    Bool,
    Int,
    IntBitLength,
    Float,
    Str,
    Ord,
    Abs,
    Sum,
    Min,
    Max,
    All,
    Any,
    Map,
    Pow,
    List,
    Tuple,
    Dict,
    DictFromKeys,
    Set,
    FrozenSet,
    Bytes,
    ByteArray,
    MemoryView,
    Complex,
    DivMod,
    Sorted,
    Enumerate,
    Zip,
    Iter,
    Next,
    AIter,
    ANext,
    Type,
    ClassMethod,
    StaticMethod,
    Property,
    ObjectNew,
    ObjectInit,
    ObjectGetAttribute,
    ObjectGetState,
    ObjectSetAttr,
    ObjectDelAttr,
    ContextVar,
    ContextVarGet,
    ContextVarSet,
    ContextCopyContext,
    ThreadRLock,
    ThreadLockEnter,
    ThreadLockExit,
    ThreadLockAcquire,
    ThreadLockRelease,
    GetAttr,
    SetAttr,
    DelAttr,
    HasAttr,
    Callable,
    IsInstance,
    IsSubclass,
    Reversed,
    Super,
    BuildClass,
    Id,
    Dir,
    Locals,
    Globals,
    SysGetFrame,
    SysGetFilesystemEncoding,
    SysGetFilesystemEncodeErrors,
    PlatformLibcVer,
    Import,
    ImportModule,
    FindSpec,
    ImportlibSourceFromCache,
    ImportlibCacheFromSource,
    RandomSeed,
    RandomRandom,
    RandomRandRange,
    RandomRandInt,
    RandomGetRandBits,
    RandomChoice,
    RandomShuffle,
    WeakRefRef,
    WeakRefProxy,
    WeakRefGetWeakRefCount,
    WeakRefGetWeakRefs,
    WeakRefRemoveDead,
    ArrayArray,
    GcCollect,
    GcEnable,
    GcDisable,
    GcIsEnabled,
    MathSqrt,
    MathCopySign,
    MathFloor,
    MathCeil,
    MathIsFinite,
    MathIsInf,
    MathIsNaN,
    TimeTime,
    TimeTimeNs,
    TimeLocalTime,
    TimeGmTime,
    TimeStrFTime,
    TimeMonotonic,
    TimeSleep,
    OsGetPid,
    OsGetCwd,
    OsListDir,
    OsFsEncode,
    OsFsDecode,
    OsRemove,
    OsWaitStatusToExitCode,
    OsPathExists,
    OsPathJoin,
    OsPathNormPath,
    OsPathNormCase,
    OsPathSplitRootEx,
    OsPathDirName,
    OsPathBaseName,
    OsPathIsDir,
    OsPathIsFile,
    OsPathSplitExt,
    OsPathAbsPath,
    OsPathExpandUser,
    OsPathRealPath,
    OsPathCommonPrefix,
    OsWaitPid,
    JsonDumps,
    JsonLoads,
    MarshalLoads,
    MarshalDumps,
    CodecsEncode,
    CodecsDecode,
    CodecsLookup,
    CodecsRegister,
    UnicodedataNormalize,
    ReSearch,
    ReMatch,
    ReFullMatch,
    ReCompile,
    ReEscape,
    OperatorAdd,
    OperatorSub,
    OperatorMul,
    OperatorMod,
    OperatorFloorDiv,
    OperatorTrueDiv,
    OperatorIndex,
    OperatorEq,
    OperatorNe,
    OperatorLt,
    OperatorLe,
    OperatorGt,
    OperatorGe,
    OperatorContains,
    OperatorGetItem,
    ItertoolsChain,
    ItertoolsCount,
    ItertoolsCycle,
    ItertoolsRepeat,
    ItertoolsBatched,
    ItertoolsPermutations,
    ItertoolsProduct,
    FunctoolsReduce,
    FunctoolsSingleDispatch,
    FunctoolsSingleDispatchMethod,
    FunctoolsSingleDispatchRegister,
    FunctoolsWraps,
    FunctoolsPartial,
    FunctoolsLruCache,
    CollectionsCounter,
    CollectionsDeque,
    CollectionsNamedTuple,
    CollectionsDefaultDict,
    TokenizeTokenizerIter,
    StructCalcSize,
    StructPack,
    StructUnpack,
    StructIterUnpack,
    StructPackInto,
    StructUnpackFrom,
    StructClearCache,
    SelectSelect,
    StringFormatterParser,
    StringFormatterFieldNameSplit,
    ImpAcquireLock,
    ImpReleaseLock,
    ImpLockHeld,
    ImpIsBuiltin,
    ImpIsFrozen,
    ImpIsFrozenPackage,
    ImpFindFrozen,
    ImpGetFrozenObject,
    ImpCreateBuiltin,
    ImpExecBuiltin,
    ImpCreateDynamic,
    ImpExecDynamic,
    ImpExtensionSuffixes,
    ImpSourceHash,
    ImpFixCoFilename,
    ImpOverrideFrozenModulesForTests,
    ImpOverrideMultiInterpExtensionsCheck,
    ImpFrozenModuleNames,
    TypingIdFunc,
    TypingTypeVar,
    TypingParamSpec,
    TypingTypeVarTuple,
    TypingTypeAliasType,
    InspectIsFunction,
    InspectIsClass,
    InspectIsModule,
    InspectIsGenerator,
    InspectIsCoroutine,
    InspectIsAwaitable,
    InspectIsAsyncGen,
    InspectStaticGetMro,
    InspectGetDunderDictOfClass,
    TypesModuleType,
    TypesMappingProxy,
    TypesMethodType,
    TypesNewClass,
    EnumConvert,
    TypeAnnotationsGet,
    IoOpen,
    IoReadText,
    IoWriteText,
    DateTimeNow,
    DateToday,
    AsyncioRun,
    AsyncioSleep,
    AsyncioCreateTask,
    AsyncioGather,
    ThreadingGetIdent,
    ThreadingCurrentThread,
    ThreadingMainThread,
    ThreadingActiveCount,
    SignalSignal,
    SignalGetSignal,
    SignalRaiseSignal,
    ColorizeCanColorize,
    ColorizeGetTheme,
    ColorizeGetColors,
    ColorizeSetTheme,
    WarningsWarn,
    WarningsWarnExplicit,
    WarningsFiltersMutated,
    AbcGetCacheToken,
    AbcInit,
    AbcRegister,
    AbcInstanceCheck,
    AbcSubclassCheck,
    AbcGetDump,
    AbcResetRegistry,
    AbcResetCaches,
    AbcAbstractMethod,
    AbcUpdateAbstractMethods,
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
            BuiltinFunction::Repr => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("repr() expects one argument"));
                }
                Ok(Value::Str(format_value(&args[0])))
            }
            BuiltinFunction::NoOp => Ok(Value::None),
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
                    Value::Set(obj) => match &*obj.kind() {
                        Object::Set(values) => Ok(Value::Int(values.len() as i64)),
                        _ => Err(RuntimeError::new("len() unsupported type")),
                    },
                    Value::FrozenSet(obj) => match &*obj.kind() {
                        Object::FrozenSet(values) => Ok(Value::Int(values.len() as i64)),
                        _ => Err(RuntimeError::new("len() unsupported type")),
                    },
                    Value::Bytes(obj) => match &*obj.kind() {
                        Object::Bytes(values) => Ok(Value::Int(values.len() as i64)),
                        _ => Err(RuntimeError::new("len() unsupported type")),
                    },
                    Value::ByteArray(obj) => match &*obj.kind() {
                        Object::ByteArray(values) => Ok(Value::Int(values.len() as i64)),
                        _ => Err(RuntimeError::new("len() unsupported type")),
                    },
                    Value::MemoryView(obj) => match &*obj.kind() {
                        Object::MemoryView(view) => match &*view.source.kind() {
                            Object::Bytes(values) | Object::ByteArray(values) => {
                                Ok(Value::Int(values.len() as i64))
                            }
                            _ => Err(RuntimeError::new("len() unsupported type")),
                        },
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
                if args.is_empty() {
                    return Ok(Value::Int(0));
                }
                if args.len() > 2 {
                    return Err(RuntimeError::new("int() expects at most two arguments"));
                }
                let parse_with_base = |text: &str, explicit_base: Option<i64>| -> Result<i64, RuntimeError> {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        return Err(RuntimeError::new("int() invalid literal"));
                    }
                    let (sign, body) = if let Some(rest) = trimmed.strip_prefix('-') {
                        (-1_i128, rest)
                    } else if let Some(rest) = trimmed.strip_prefix('+') {
                        (1_i128, rest)
                    } else {
                        (1_i128, trimmed)
                    };
                    if body.is_empty() {
                        return Err(RuntimeError::new("int() invalid literal"));
                    }

                    let mut base = explicit_base.unwrap_or(10);
                    if explicit_base.is_some() && !(base == 0 || (2..=36).contains(&base)) {
                        return Err(RuntimeError::new("int() base must be >= 2 and <= 36, or 0"));
                    }

                    let mut digits = body;
                    if base == 0 {
                        if let Some(rest) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
                            base = 16;
                            digits = rest;
                        } else if let Some(rest) = digits.strip_prefix("0o").or_else(|| digits.strip_prefix("0O")) {
                            base = 8;
                            digits = rest;
                        } else if let Some(rest) = digits.strip_prefix("0b").or_else(|| digits.strip_prefix("0B")) {
                            base = 2;
                            digits = rest;
                        } else {
                            base = 10;
                        }
                    } else if base == 16 {
                        if let Some(rest) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
                            digits = rest;
                        }
                    } else if base == 8 {
                        if let Some(rest) = digits.strip_prefix("0o").or_else(|| digits.strip_prefix("0O")) {
                            digits = rest;
                        }
                    } else if base == 2 {
                        if let Some(rest) = digits.strip_prefix("0b").or_else(|| digits.strip_prefix("0B")) {
                            digits = rest;
                        }
                    }

                    let normalized = digits.replace('_', "");
                    if normalized.is_empty() {
                        return Err(RuntimeError::new("int() invalid literal"));
                    }

                    let parsed = i128::from_str_radix(&normalized, base as u32)
                        .map_err(|_| RuntimeError::new("int() invalid literal"))?;
                    let signed = sign * parsed;
                    Ok(signed.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
                };

                let explicit_base = if args.len() == 2 {
                    Some(value_to_int(args[1].clone())?)
                } else {
                    None
                };
                if explicit_base.is_some()
                    && !matches!(args[0], Value::Str(_) | Value::Bytes(_) | Value::ByteArray(_))
                {
                    return Err(RuntimeError::new(
                        "int() can't convert non-string with explicit base",
                    ));
                }
                match &args[0] {
                    Value::Int(value) => Ok(Value::Int(*value)),
                    Value::Bool(value) => Ok(Value::Int(if *value { 1 } else { 0 })),
                    Value::Float(value) => Ok(Value::Int(*value as i64)),
                    Value::Str(value) => Ok(Value::Int(parse_with_base(value, explicit_base)?)),
                    Value::Bytes(obj) | Value::ByteArray(obj) => match &*obj.kind() {
                        Object::Bytes(bytes) | Object::ByteArray(bytes) => {
                            let text = String::from_utf8_lossy(bytes);
                            Ok(Value::Int(parse_with_base(&text, explicit_base)?))
                        }
                        _ => Err(RuntimeError::new("int() unsupported type")),
                    },
                    _ => Err(RuntimeError::new("int() unsupported type")),
                }
            }
            BuiltinFunction::IntBitLength => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("int.bit_length() expects one argument"));
                }
                let value = value_to_int(args[0].clone())?;
                Ok(Value::Int((i64::BITS - value.unsigned_abs().leading_zeros()) as i64))
            }
            BuiltinFunction::Float => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("float() expects one argument"));
                }
                match &args[0] {
                    Value::Float(value) => Ok(Value::Float(*value)),
                    Value::Int(value) => Ok(Value::Float(*value as f64)),
                    Value::Bool(value) => Ok(Value::Float(if *value { 1.0 } else { 0.0 })),
                    Value::Str(value) => {
                        let trimmed = value.trim();
                        let parsed = trimmed
                            .parse::<f64>()
                            .map_err(|_| RuntimeError::new("float() invalid literal"))?;
                        Ok(Value::Float(parsed))
                    }
                    _ => Err(RuntimeError::new("float() unsupported type")),
                }
            }
            BuiltinFunction::Str => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("str() expects one argument"));
                }
                Ok(Value::Str(format_value(&args[0])))
            }
            BuiltinFunction::Ord => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("ord() expects one argument"));
                }
                match &args[0] {
                    Value::Str(value) => {
                        let mut chars = value.chars();
                        let ch = chars
                            .next()
                            .ok_or_else(|| RuntimeError::new("ord() expected a character"))?;
                        if chars.next().is_some() {
                            return Err(RuntimeError::new("ord() expected a character"));
                        }
                        Ok(Value::Int(ch as i64))
                    }
                    Value::Bytes(obj) => match &*obj.kind() {
                        Object::Bytes(values) if values.len() == 1 => Ok(Value::Int(values[0] as i64)),
                        Object::Bytes(_) => Err(RuntimeError::new("ord() expected a character")),
                        _ => Err(RuntimeError::new("ord() unsupported type")),
                    },
                    Value::ByteArray(obj) => match &*obj.kind() {
                        Object::ByteArray(values) if values.len() == 1 => {
                            Ok(Value::Int(values[0] as i64))
                        }
                        Object::ByteArray(_) => Err(RuntimeError::new("ord() expected a character")),
                        _ => Err(RuntimeError::new("ord() unsupported type")),
                    },
                    _ => Err(RuntimeError::new("ord() expected string of length 1")),
                }
            }
            BuiltinFunction::Abs => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("abs() expects one argument"));
                }
                match &args[0] {
                    Value::Int(value) => Ok(Value::Int(value.abs())),
                    Value::Bool(value) => Ok(Value::Int(if *value { 1 } else { 0 })),
                    Value::Float(value) => Ok(Value::Float(value.abs())),
                    _ => Err(RuntimeError::new("abs() unsupported type")),
                }
            }
            BuiltinFunction::Sum => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("sum() expects 1-2 arguments"));
                }
                let mut total = if args.len() == 2 {
                    args[1].clone()
                } else {
                    Value::Int(0)
                };

                match &args[0] {
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => {
                            for value in values {
                                total = add_numeric_values(total, value.clone())?;
                            }
                        }
                        _ => return Err(RuntimeError::new("sum() expects list or tuple")),
                    },
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => {
                            for value in values {
                                total = add_numeric_values(total, value.clone())?;
                            }
                        }
                        _ => return Err(RuntimeError::new("sum() expects list or tuple")),
                    },
                    _ => return Err(RuntimeError::new("sum() expects list or tuple")),
                }

                Ok(total)
            }
            BuiltinFunction::Min => builtin_min_max(args, Ordering::Less),
            BuiltinFunction::Max => builtin_min_max(args, Ordering::Greater),
            BuiltinFunction::All => builtin_all_any(args, true),
            BuiltinFunction::Any => builtin_all_any(args, false),
            BuiltinFunction::Map => Err(RuntimeError::new("map() requires VM context")),
            BuiltinFunction::Pow => {
                if args.len() < 2 || args.len() > 3 {
                    return Err(RuntimeError::new("pow() expects 2-3 arguments"));
                }
                if args.len() == 3 {
                    let base = value_to_int(args[0].clone()).map_err(|_| {
                        RuntimeError::new(
                            "pow() 3rd argument not allowed unless all arguments are integers",
                        )
                    })?;
                    let exponent = value_to_int(args[1].clone()).map_err(|_| {
                        RuntimeError::new(
                            "pow() 3rd argument not allowed unless all arguments are integers",
                        )
                    })?;
                    if exponent < 0 {
                        return Err(RuntimeError::new(
                            "pow() 2nd argument cannot be negative when 3rd argument specified",
                        ));
                    }
                    let modu = value_to_int(args[2].clone()).map_err(|_| {
                        RuntimeError::new(
                            "pow() 3rd argument not allowed unless all arguments are integers",
                        )
                    })?;
                    if modu == 0 {
                        return Err(RuntimeError::new("pow() modulo by zero"));
                    }
                    return Ok(Value::Int(mod_pow_i64(base, exponent, modu)?));
                }
                let value = pow_numeric_values(args[0].clone(), args[1].clone())?;
                Ok(value)
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
                    Value::Str(value) => Ok(heap
                        .alloc_list(value.chars().map(|ch| Value::Str(ch.to_string())).collect())),
                    Value::Set(obj) => match &*obj.kind() {
                        Object::Set(values) => Ok(heap.alloc_list(values.clone())),
                        _ => Err(RuntimeError::new("list() unsupported type")),
                    },
                    Value::FrozenSet(obj) => match &*obj.kind() {
                        Object::FrozenSet(values) => Ok(heap.alloc_list(values.clone())),
                        _ => Err(RuntimeError::new("list() unsupported type")),
                    },
                    Value::Bytes(obj) => match &*obj.kind() {
                        Object::Bytes(values) => Ok(heap.alloc_list(
                            values.iter().map(|byte| Value::Int(*byte as i64)).collect(),
                        )),
                        _ => Err(RuntimeError::new("list() unsupported type")),
                    },
                    Value::ByteArray(obj) => match &*obj.kind() {
                        Object::ByteArray(values) => Ok(heap.alloc_list(
                            values.iter().map(|byte| Value::Int(*byte as i64)).collect(),
                        )),
                        _ => Err(RuntimeError::new("list() unsupported type")),
                    },
                    Value::MemoryView(obj) => match &*obj.kind() {
                        Object::MemoryView(view) => match &*view.source.kind() {
                            Object::Bytes(values) | Object::ByteArray(values) => Ok(heap.alloc_list(
                                values.iter().map(|byte| Value::Int(*byte as i64)).collect(),
                            )),
                            _ => Err(RuntimeError::new("list() unsupported type")),
                        },
                        _ => Err(RuntimeError::new("list() unsupported type")),
                    },
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
                    Value::Str(value) => Ok(heap
                        .alloc_tuple(value.chars().map(|ch| Value::Str(ch.to_string())).collect())),
                    Value::Set(obj) => match &*obj.kind() {
                        Object::Set(values) => Ok(heap.alloc_tuple(values.clone())),
                        _ => Err(RuntimeError::new("tuple() unsupported type")),
                    },
                    Value::FrozenSet(obj) => match &*obj.kind() {
                        Object::FrozenSet(values) => Ok(heap.alloc_tuple(values.clone())),
                        _ => Err(RuntimeError::new("tuple() unsupported type")),
                    },
                    Value::Bytes(obj) => match &*obj.kind() {
                        Object::Bytes(values) => Ok(heap.alloc_tuple(
                            values.iter().map(|byte| Value::Int(*byte as i64)).collect(),
                        )),
                        _ => Err(RuntimeError::new("tuple() unsupported type")),
                    },
                    Value::ByteArray(obj) => match &*obj.kind() {
                        Object::ByteArray(values) => Ok(heap.alloc_tuple(
                            values.iter().map(|byte| Value::Int(*byte as i64)).collect(),
                        )),
                        _ => Err(RuntimeError::new("tuple() unsupported type")),
                    },
                    Value::MemoryView(obj) => match &*obj.kind() {
                        Object::MemoryView(view) => match &*view.source.kind() {
                            Object::Bytes(values) | Object::ByteArray(values) => Ok(heap.alloc_tuple(
                                values.iter().map(|byte| Value::Int(*byte as i64)).collect(),
                            )),
                            _ => Err(RuntimeError::new("tuple() unsupported type")),
                        },
                        _ => Err(RuntimeError::new("tuple() unsupported type")),
                    },
                    _ => Err(RuntimeError::new("tuple() unsupported type")),
                }
            }
            BuiltinFunction::Dict => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("dict() expects at most one argument"));
                }
                if args.is_empty() {
                    return Ok(heap.alloc_dict(Vec::new()));
                }
                match &args[0] {
                    Value::Dict(obj) => match &*obj.kind() {
                        Object::Dict(entries) => Ok(heap.alloc_dict(entries.clone())),
                        _ => Err(RuntimeError::new("dict() unsupported type")),
                    },
                    other => {
                        let mut entries = Vec::new();
                        for item in iterable_values(other.clone())? {
                            match item {
                                Value::Tuple(pair) => match &*pair.kind() {
                                    Object::Tuple(parts) if parts.len() == 2 => {
                                        entries.push((parts[0].clone(), parts[1].clone()));
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "dict() sequence elements must be length 2",
                                        ));
                                    }
                                },
                                Value::List(pair) => match &*pair.kind() {
                                    Object::List(parts) if parts.len() == 2 => {
                                        entries.push((parts[0].clone(), parts[1].clone()));
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "dict() sequence elements must be length 2",
                                        ));
                                    }
                                },
                                _ => {
                                    return Err(RuntimeError::new(
                                        "dict() argument must be a mapping or iterable of pairs",
                                    ));
                                }
                            }
                        }
                        Ok(heap.alloc_dict(entries))
                    }
                }
            }
            BuiltinFunction::DictFromKeys => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("dict.fromkeys() expects 1-2 arguments"));
                }
                let keys = iterable_values(args[0].clone())?;
                let default = args.get(1).cloned().unwrap_or(Value::None);
                let mut entries: Vec<(Value, Value)> = Vec::new();
                for key in keys {
                    if let Some((_, value)) = entries.iter_mut().find(|(existing, _)| *existing == key)
                    {
                        *value = default.clone();
                    } else {
                        entries.push((key, default.clone()));
                    }
                }
                Ok(heap.alloc_dict(entries))
            }
            BuiltinFunction::Set => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("set() expects at most one argument"));
                }
                let values = if let Some(source) = args.into_iter().next() {
                    iterable_values(source)?
                } else {
                    Vec::new()
                };
                Ok(heap.alloc_set(dedup_values(values)))
            }
            BuiltinFunction::FrozenSet => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("frozenset() expects at most one argument"));
                }
                let values = if let Some(source) = args.into_iter().next() {
                    iterable_values(source)?
                } else {
                    Vec::new()
                };
                Ok(heap.alloc_frozenset(dedup_values(values)))
            }
            BuiltinFunction::Bytes => {
                if args.len() > 2 {
                    return Err(RuntimeError::new("bytes() expects at most 2 arguments"));
                }
                let bytes = if args.is_empty() {
                    Vec::new()
                } else if args.len() == 2 {
                    let mut it = args.into_iter();
                    let source = it.next().unwrap_or(Value::None);
                    let encoding = it.next().unwrap_or(Value::None);
                    value_to_bytes_with_encoding(source, Some(encoding))?
                } else {
                    value_to_bytes_with_encoding(args[0].clone(), None)?
                };
                Ok(heap.alloc_bytes(bytes))
            }
            BuiltinFunction::ByteArray => {
                if args.len() > 2 {
                    return Err(RuntimeError::new("bytearray() expects at most 2 arguments"));
                }
                let bytes = if args.is_empty() {
                    Vec::new()
                } else if args.len() == 2 {
                    let mut it = args.into_iter();
                    let source = it.next().unwrap_or(Value::None);
                    let encoding = it.next().unwrap_or(Value::None);
                    value_to_bytes_with_encoding(source, Some(encoding))?
                } else {
                    value_to_bytes_with_encoding(args[0].clone(), None)?
                };
                Ok(heap.alloc_bytearray(bytes))
            }
            BuiltinFunction::MemoryView => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("memoryview() expects one argument"));
                }
                let source = match &args[0] {
                    Value::Bytes(obj) | Value::ByteArray(obj) => obj.clone(),
                    _ => return Err(RuntimeError::new("memoryview() expects bytes-like object")),
                };
                Ok(heap.alloc_memoryview(source))
            }
            BuiltinFunction::Complex => {
                if args.len() > 2 {
                    return Err(RuntimeError::new("complex() expects at most 2 arguments"));
                }
                let (real, imag) = if args.is_empty() {
                    (0.0, 0.0)
                } else if args.len() == 1 {
                    value_to_complex_pair(args[0].clone())?
                } else {
                    let real = value_to_float(args[0].clone())?;
                    let imag = value_to_float(args[1].clone())?;
                    (real, imag)
                };
                Ok(Value::Complex { real, imag })
            }
            BuiltinFunction::Type => {
                if args.len() == 1 {
                    return builtin_type_of(&args[0]);
                }
                if args.len() != 3 {
                    return Err(RuntimeError::new("type() expects 1 or 3 arguments"));
                }
                let name = match &args[0] {
                    Value::Str(name) => name.clone(),
                    _ => return Err(RuntimeError::new("type() first argument must be string")),
                };
                let bases = match &args[1] {
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => values
                            .iter()
                            .map(|value| match value {
                                Value::Class(class) => Ok(class.clone()),
                                _ => Err(RuntimeError::new("type() bases must be classes")),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        _ => return Err(RuntimeError::new("type() bases must be tuple")),
                    },
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => values
                            .iter()
                            .map(|value| match value {
                                Value::Class(class) => Ok(class.clone()),
                                _ => Err(RuntimeError::new("type() bases must be classes")),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
                    },
                    _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
                };
                let attrs = match &args[2] {
                    Value::Dict(obj) => match &*obj.kind() {
                        Object::Dict(entries) => {
                            let mut out = HashMap::new();
                            for (key, value) in entries {
                                let key = match key {
                                    Value::Str(name) => name.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "type() dict keys must be strings",
                                        ));
                                    }
                                };
                                out.insert(key, value.clone());
                            }
                            out
                        }
                        _ => return Err(RuntimeError::new("type() third argument must be dict")),
                    },
                    _ => return Err(RuntimeError::new("type() third argument must be dict")),
                };
                let mut class = ClassObject::new(name.clone(), bases);
                class.attrs = attrs;
                class.metaclass = class
                    .bases
                    .iter()
                    .find_map(|base| match &*base.kind() {
                        Object::Class(class_data) => class_data.metaclass.clone(),
                        _ => None,
                    });
                class
                    .attrs
                    .entry("__name__".to_string())
                    .or_insert_with(|| Value::Str(name));
                Ok(heap.alloc_class(class))
            }
            BuiltinFunction::ClassMethod => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("classmethod() expects one argument"));
                }
                Ok(args[0].clone())
            }
            BuiltinFunction::StaticMethod => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("staticmethod() expects one argument"));
                }
                Ok(args[0].clone())
            }
            BuiltinFunction::Property => {
                if args.len() > 4 {
                    return Err(RuntimeError::new("property() expects up to four arguments"));
                }
                Ok(args.first().cloned().unwrap_or(Value::None))
            }
            BuiltinFunction::ContextVar => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new(
                        "ContextVar() expects name and optional default",
                    ));
                }
                let name = match &args[0] {
                    Value::Str(value) => value.clone(),
                    _ => return Err(RuntimeError::new("ContextVar() name must be string")),
                };
                let default = args.get(1).cloned().unwrap_or(Value::None);
                let module = match heap.alloc_module(ModuleObject::new(format!("<ContextVar {name}>")))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *module.kind_mut() {
                    module_data
                        .globals
                        .insert("__name__".to_string(), Value::Str(name));
                    module_data
                        .globals
                        .insert("__default__".to_string(), default);
                    module_data.globals.insert(
                        "get".to_string(),
                        Value::Builtin(BuiltinFunction::ContextVarGet),
                    );
                    module_data.globals.insert(
                        "set".to_string(),
                        Value::Builtin(BuiltinFunction::ContextVarSet),
                    );
                }
                Ok(Value::Module(module))
            }
            BuiltinFunction::ContextVarGet => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("ContextVar.get() expects at most one argument"));
                }
                if let Some(default) = args.into_iter().next() {
                    Ok(default)
                } else {
                    // KeyError is a LookupError subclass and matches contextvars expectations.
                    Err(RuntimeError::new("key not found"))
                }
            }
            BuiltinFunction::ContextVarSet => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("ContextVar.set() expects one argument"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::ContextCopyContext => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("copy_context() expects no arguments"));
                }
                let module = match heap.alloc_module(ModuleObject::new("<Context>")) {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *module.kind_mut() {
                    module_data
                        .globals
                        .insert("__name__".to_string(), Value::Str("Context".to_string()));
                }
                Ok(Value::Module(module))
            }
            BuiltinFunction::ThreadRLock => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("RLock() expects no arguments"));
                }
                let module = match heap.alloc_module(ModuleObject::new("<RLock>".to_string())) {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *module.kind_mut() {
                    module_data.globals.insert(
                        "__enter__".to_string(),
                        Value::Builtin(BuiltinFunction::ThreadLockEnter),
                    );
                    module_data.globals.insert(
                        "__exit__".to_string(),
                        Value::Builtin(BuiltinFunction::ThreadLockExit),
                    );
                    module_data.globals.insert(
                        "acquire".to_string(),
                        Value::Builtin(BuiltinFunction::ThreadLockAcquire),
                    );
                    module_data.globals.insert(
                        "release".to_string(),
                        Value::Builtin(BuiltinFunction::ThreadLockRelease),
                    );
                }
                Ok(Value::Module(module))
            }
            BuiltinFunction::ThreadLockEnter => Ok(Value::None),
            BuiltinFunction::ThreadLockExit => Ok(Value::Bool(false)),
            BuiltinFunction::ThreadLockAcquire => Ok(Value::Bool(true)),
            BuiltinFunction::ThreadLockRelease => Ok(Value::None),
            BuiltinFunction::FunctoolsLruCache => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("lru_cache() expects at most one argument"));
                }
                if let Some(callable) = args.into_iter().next() {
                    Ok(callable)
                } else {
                    Ok(Value::Builtin(BuiltinFunction::FunctoolsLruCache))
                }
            }
            BuiltinFunction::TokenizeTokenizerIter => {
                if args.is_empty() {
                    return Err(RuntimeError::new("TokenizerIter() expects source"));
                }
                let empty = match heap.alloc_list(Vec::new()) {
                    Value::List(obj) => obj,
                    _ => unreachable!(),
                };
                Ok(Value::Iterator(
                    heap.alloc(Object::Iterator(IteratorObject {
                        kind: IteratorKind::List(empty),
                        index: 0,
                    })),
                ))
            }
            BuiltinFunction::StructCalcSize => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("calcsize() expects one argument"));
                }
                Ok(Value::Int(0))
            }
            BuiltinFunction::StructPack => {
                if args.is_empty() {
                    return Err(RuntimeError::new("pack() expects format string"));
                }
                Ok(heap.alloc_bytes(Vec::new()))
            }
            BuiltinFunction::StructUnpack => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("unpack() expects format and buffer"));
                }
                Ok(heap.alloc_tuple(Vec::new()))
            }
            BuiltinFunction::StructIterUnpack => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("iter_unpack() expects format and buffer"));
                }
                let empty = match heap.alloc_list(Vec::new()) {
                    Value::List(obj) => obj,
                    _ => unreachable!(),
                };
                Ok(Value::Iterator(
                    heap.alloc(Object::Iterator(IteratorObject {
                        kind: IteratorKind::List(empty),
                        index: 0,
                    })),
                ))
            }
            BuiltinFunction::StructPackInto => {
                if args.len() < 3 {
                    return Err(RuntimeError::new("pack_into() expects format, buffer, offset"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::StructUnpackFrom => {
                if args.len() < 2 || args.len() > 3 {
                    return Err(RuntimeError::new(
                        "unpack_from() expects format, buffer, optional offset",
                    ));
                }
                Ok(heap.alloc_tuple(Vec::new()))
            }
            BuiltinFunction::StructClearCache => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("_clearcache() expects no arguments"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::ImpAcquireLock => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("acquire_lock() expects no arguments"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::ImpReleaseLock => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("release_lock() expects no arguments"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::ImpLockHeld => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("lock_held() expects no arguments"));
                }
                Ok(Value::Bool(false))
            }
            BuiltinFunction::ImpIsBuiltin => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("is_builtin() expects one argument"));
                }
                let name = match &args[0] {
                    Value::Str(name) => name.as_str(),
                    _ => return Err(RuntimeError::new("is_builtin() name must be string")),
                };
                Ok(Value::Bool(matches!(
                    name,
                    "sys"
                        | "builtins"
                        | "_imp"
                        | "_tokenize"
                        | "_struct"
                        | "_ast"
                        | "_typing"
                        | "_contextvars"
                )))
            }
            BuiltinFunction::ImpIsFrozen | BuiltinFunction::ImpIsFrozenPackage => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("frozen-query expects one argument"));
                }
                Ok(Value::Bool(false))
            }
            BuiltinFunction::ImpFindFrozen => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("find_frozen() expects one argument"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::ImpGetFrozenObject => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new(
                        "get_frozen_object() expects name and optional token",
                    ));
                }
                Err(RuntimeError::new("module not found"))
            }
            BuiltinFunction::ImpCreateBuiltin => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("create_builtin() expects one argument"));
                }
                let module = match heap.alloc_module(ModuleObject::new("<builtin-module>")) {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                Ok(Value::Module(module))
            }
            BuiltinFunction::ImpExecBuiltin => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("exec_builtin() expects one argument"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::ImpCreateDynamic => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("create_dynamic() expects one argument"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::ImpExecDynamic => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("exec_dynamic() expects one argument"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::ImpExtensionSuffixes => {
                if !args.is_empty() {
                    return Err(RuntimeError::new(
                        "extension_suffixes() expects no arguments",
                    ));
                }
                Ok(heap.alloc_list(Vec::new()))
            }
            BuiltinFunction::ImpSourceHash => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("source_hash() expects token and source"));
                }
                let bytes = match &args[1] {
                    Value::Bytes(obj) | Value::ByteArray(obj) => match &*obj.kind() {
                        Object::Bytes(values) | Object::ByteArray(values) => values.clone(),
                        _ => Vec::new(),
                    },
                    Value::Str(text) => text.as_bytes().to_vec(),
                    _ => Vec::new(),
                };
                let mut hash: u64 = 1469598103934665603;
                for byte in bytes {
                    hash ^= byte as u64;
                    hash = hash.wrapping_mul(1099511628211);
                }
                Ok(heap.alloc_bytes(hash.to_le_bytes().to_vec()))
            }
            BuiltinFunction::ImpFixCoFilename => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("_fix_co_filename() expects code and path"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::ImpOverrideFrozenModulesForTests
            | BuiltinFunction::ImpOverrideMultiInterpExtensionsCheck => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("override helper expects one argument"));
                }
                Ok(Value::Int(0))
            }
            BuiltinFunction::ImpFrozenModuleNames => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("_frozen_module_names() expects no arguments"));
                }
                Ok(heap.alloc_tuple(Vec::new()))
            }
            BuiltinFunction::TypingIdFunc => {
                if args.is_empty() {
                    Ok(Value::None)
                } else {
                    Ok(args[0].clone())
                }
            }
            BuiltinFunction::TypingTypeVar
            | BuiltinFunction::TypingParamSpec
            | BuiltinFunction::TypingTypeVarTuple
            | BuiltinFunction::TypingTypeAliasType => {
                if args.is_empty() {
                    return Err(RuntimeError::new("typing helper expects a name"));
                }
                let name = match &args[0] {
                    Value::Str(value) => value.clone(),
                    _ => return Err(RuntimeError::new("typing helper name must be string")),
                };
                let marker = match heap.alloc_module(ModuleObject::new(format!("<typing {name}>"))) {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *marker.kind_mut() {
                    module_data
                        .globals
                        .insert("__name__".to_string(), Value::Str(name.clone()));
                    module_data
                        .globals
                        .insert("__qualname__".to_string(), Value::Str(name));
                }
                Ok(Value::Module(marker))
            }
            BuiltinFunction::DivMod => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("divmod() expects two arguments"));
                }
                let (div, rem) = divmod_values(args[0].clone(), args[1].clone())?;
                Ok(heap.alloc_tuple(vec![div, rem]))
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
                            let all_str = result.iter().all(|value| matches!(value, Value::Str(_)));

                            if all_numeric {
                                result.sort_by(|a, b| {
                                    numeric_compare(a, b).unwrap_or(Ordering::Equal)
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
                            let all_numeric =
                                result.iter().all(|value| numeric_value(value).is_some());
                            let all_str = result.iter().all(|value| matches!(value, Value::Str(_)));

                            if all_numeric {
                                result.sort_by(|a, b| {
                                    numeric_compare(a, b).unwrap_or(Ordering::Equal)
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
            BuiltinFunction::CollectionsNamedTuple => {
                if args.len() < 2 {
                    return Err(RuntimeError::new(
                        "namedtuple() expects typename and field names",
                    ));
                }
                let type_name = match &args[0] {
                    Value::Str(name) => name.clone(),
                    _ => return Err(RuntimeError::new("namedtuple() typename must be string")),
                };
                let fields: Vec<String> = match &args[1] {
                    Value::Str(names) => names
                        .replace(',', " ")
                        .split_whitespace()
                        .map(ToOwned::to_owned)
                        .collect(),
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => values
                            .iter()
                            .map(|value| match value {
                                Value::Str(name) => Ok(name.clone()),
                                _ => Err(RuntimeError::new(
                                    "namedtuple() field names must be strings",
                                )),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        _ => {
                            return Err(RuntimeError::new(
                                "namedtuple() field names must be string/list/tuple",
                            ));
                        }
                    },
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => values
                            .iter()
                            .map(|value| match value {
                                Value::Str(name) => Ok(name.clone()),
                                _ => Err(RuntimeError::new(
                                    "namedtuple() field names must be strings",
                                )),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        _ => {
                            return Err(RuntimeError::new(
                                "namedtuple() field names must be string/list/tuple",
                            ));
                        }
                    },
                    _ => {
                        return Err(RuntimeError::new(
                            "namedtuple() field names must be string/list/tuple",
                        ));
                    }
                };

                let mut class = ClassObject::new(type_name.clone(), Vec::new());
                class
                    .attrs
                    .insert("__name__".to_string(), Value::Str(type_name));
                class.attrs.insert(
                    "_fields".to_string(),
                    heap.alloc_tuple(fields.iter().cloned().map(Value::Str).collect()),
                );
                for field in &fields {
                    let descriptor = match heap.alloc_module(ModuleObject::new(format!(
                        "__namedtuple_field_{field}"
                    ))) {
                        Value::Module(module) => {
                            if let Object::Module(module_data) = &mut *module.kind_mut() {
                                module_data.globals.insert("__doc__".to_string(), Value::None);
                            }
                            Value::Module(module)
                        }
                        _ => Value::None,
                    };
                    class.attrs.insert(field.clone(), descriptor);
                }
                Ok(heap.alloc_class(class))
            }
            BuiltinFunction::TypesMappingProxy => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("MappingProxyType() expects one argument"));
                }
                Ok(args[0].clone())
            }
            BuiltinFunction::AbcGetCacheToken => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("get_cache_token() expects no arguments"));
                }
                Ok(Value::Int(0))
            }
            BuiltinFunction::AbcInit => Ok(Value::None),
            BuiltinFunction::AbcRegister => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("_abc_register() expects two arguments"));
                }
                Ok(args[1].clone())
            }
            BuiltinFunction::AbcInstanceCheck | BuiltinFunction::AbcSubclassCheck => {
                Ok(Value::Bool(false))
            }
            BuiltinFunction::AbcGetDump => Ok(heap.alloc_tuple(vec![
                heap.alloc_set(Vec::new()),
                heap.alloc_set(Vec::new()),
                heap.alloc_set(Vec::new()),
                Value::Int(0),
            ])),
            BuiltinFunction::AbcResetRegistry | BuiltinFunction::AbcResetCaches => Ok(Value::None),
            BuiltinFunction::AbcAbstractMethod | BuiltinFunction::AbcUpdateAbstractMethods => {
                if args.len() != 1 {
                    return Err(RuntimeError::new(
                        "abc helper expects exactly one argument",
                    ));
                }
                Ok(args[0].clone())
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
                            entries.push(
                                heap.alloc_tuple(vec![
                                    Value::Int(index),
                                    Value::Str(ch.to_string()),
                                ]),
                            );
                        }
                    }
                    _ => return Err(RuntimeError::new("enumerate() expects iterable")),
                }
                Ok(heap.alloc_list(entries))
            }
            BuiltinFunction::WeakRefRef | BuiltinFunction::WeakRefProxy => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new(
                        "weakref helper expects object and optional callback",
                    ));
                }
                Ok(args[0].clone())
            }
            BuiltinFunction::WeakRefGetWeakRefCount => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("getweakrefcount() expects one argument"));
                }
                Ok(Value::Int(0))
            }
            BuiltinFunction::WeakRefGetWeakRefs => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("getweakrefs() expects one argument"));
                }
                Ok(heap.alloc_list(Vec::new()))
            }
            BuiltinFunction::WeakRefRemoveDead => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("_remove_dead_weakref() expects two arguments"));
                }
                if let Value::Dict(obj) = &args[0] {
                    if let Object::Dict(entries) = &mut *obj.kind_mut() {
                        if let Some(index) =
                            entries.iter().position(|(key, _)| *key == args[1])
                        {
                            entries.remove(index);
                        }
                    }
                }
                Ok(Value::None)
            }
            BuiltinFunction::ArrayArray => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("array.array() expects 1-2 arguments"));
                }
                let typecode = match &args[0] {
                    Value::Str(code) => code.clone(),
                    _ => return Err(RuntimeError::new("array() typecode must be a string")),
                };
                if typecode.is_empty() {
                    return Err(RuntimeError::new("array() typecode cannot be empty"));
                }
                let mut values = Vec::new();
                if let Some(initializer) = args.get(1) {
                    match initializer {
                        Value::List(obj) => match &*obj.kind() {
                            Object::List(items) => values.extend(items.clone()),
                            _ => return Err(RuntimeError::new("array() initializer must be iterable")),
                        },
                        Value::Tuple(obj) => match &*obj.kind() {
                            Object::Tuple(items) => values.extend(items.clone()),
                            _ => return Err(RuntimeError::new("array() initializer must be iterable")),
                        },
                        Value::Bytes(obj) | Value::ByteArray(obj) => match &*obj.kind() {
                            Object::Bytes(bytes) | Object::ByteArray(bytes) => {
                                values.extend(bytes.iter().map(|value| Value::Int(*value as i64)));
                            }
                            _ => return Err(RuntimeError::new("array() initializer must be iterable")),
                        },
                        Value::Str(text) => {
                            values.extend(text.chars().map(|ch| Value::Int(ch as i64)));
                        }
                        Value::None => {}
                        _ => return Err(RuntimeError::new("array() initializer must be iterable")),
                    }
                }
                Ok(heap.alloc_list(values))
            }
            BuiltinFunction::GcCollect => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("gc.collect() expects at most one argument"));
                }
                Ok(Value::Int(0))
            }
            BuiltinFunction::GcEnable | BuiltinFunction::GcDisable => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("gc helper expects no arguments"));
                }
                Ok(Value::None)
            }
            BuiltinFunction::GcIsEnabled => {
                if !args.is_empty() {
                    return Err(RuntimeError::new("gc.isenabled() expects no arguments"));
                }
                Ok(Value::Bool(true))
            }
            BuiltinFunction::GetAttr
            | BuiltinFunction::SetAttr
            | BuiltinFunction::DelAttr
            | BuiltinFunction::HasAttr
            | BuiltinFunction::Callable
            | BuiltinFunction::IsInstance
            | BuiltinFunction::IsSubclass
            | BuiltinFunction::Reversed
            | BuiltinFunction::Zip
            | BuiltinFunction::Iter
            | BuiltinFunction::Next
            | BuiltinFunction::AIter
            | BuiltinFunction::ANext
            | BuiltinFunction::Super
            | BuiltinFunction::Locals
            | BuiltinFunction::Globals
            | BuiltinFunction::SysGetFrame
            | BuiltinFunction::SysGetFilesystemEncoding
            | BuiltinFunction::SysGetFilesystemEncodeErrors
            | BuiltinFunction::PlatformLibcVer
            | BuiltinFunction::ImportlibSourceFromCache
            | BuiltinFunction::ImportlibCacheFromSource
            | BuiltinFunction::RandomSeed
            | BuiltinFunction::RandomRandom
            | BuiltinFunction::RandomRandRange
            | BuiltinFunction::RandomRandInt
            | BuiltinFunction::RandomGetRandBits
            | BuiltinFunction::RandomChoice
            | BuiltinFunction::RandomShuffle
            | BuiltinFunction::MathSqrt
            | BuiltinFunction::MathCopySign
            | BuiltinFunction::MathFloor
            | BuiltinFunction::MathCeil
            | BuiltinFunction::MathIsFinite
            | BuiltinFunction::MathIsInf
            | BuiltinFunction::MathIsNaN
            | BuiltinFunction::TimeTime
            | BuiltinFunction::TimeTimeNs
            | BuiltinFunction::TimeLocalTime
            | BuiltinFunction::TimeGmTime
            | BuiltinFunction::TimeStrFTime
            | BuiltinFunction::TimeMonotonic
            | BuiltinFunction::TimeSleep
            | BuiltinFunction::OsGetPid
            | BuiltinFunction::OsGetCwd
            | BuiltinFunction::OsListDir
            | BuiltinFunction::OsFsEncode
            | BuiltinFunction::OsFsDecode
            | BuiltinFunction::OsRemove
            | BuiltinFunction::OsWaitStatusToExitCode
            | BuiltinFunction::OsPathExists
            | BuiltinFunction::OsPathJoin
            | BuiltinFunction::OsPathNormPath
            | BuiltinFunction::OsPathNormCase
            | BuiltinFunction::OsPathSplitRootEx
            | BuiltinFunction::OsPathDirName
            | BuiltinFunction::OsPathBaseName
            | BuiltinFunction::OsPathIsDir
            | BuiltinFunction::OsPathIsFile
            | BuiltinFunction::OsPathSplitExt
            | BuiltinFunction::OsPathAbsPath
            | BuiltinFunction::OsPathExpandUser
            | BuiltinFunction::OsPathRealPath
            | BuiltinFunction::OsPathCommonPrefix
            | BuiltinFunction::OsWaitPid
            | BuiltinFunction::JsonDumps
            | BuiltinFunction::JsonLoads
            | BuiltinFunction::CodecsEncode
            | BuiltinFunction::CodecsDecode
            | BuiltinFunction::CodecsLookup
            | BuiltinFunction::CodecsRegister
            | BuiltinFunction::UnicodedataNormalize
            | BuiltinFunction::SelectSelect
            | BuiltinFunction::ReSearch
            | BuiltinFunction::ReMatch
            | BuiltinFunction::ReFullMatch
            | BuiltinFunction::ReCompile
            | BuiltinFunction::ReEscape
            | BuiltinFunction::OperatorAdd
            | BuiltinFunction::OperatorSub
            | BuiltinFunction::OperatorMul
            | BuiltinFunction::OperatorMod
            | BuiltinFunction::OperatorTrueDiv
            | BuiltinFunction::OperatorFloorDiv
            | BuiltinFunction::OperatorIndex
            | BuiltinFunction::OperatorEq
            | BuiltinFunction::OperatorNe
            | BuiltinFunction::OperatorLt
            | BuiltinFunction::OperatorLe
            | BuiltinFunction::OperatorGt
            | BuiltinFunction::OperatorGe
            | BuiltinFunction::OperatorContains
            | BuiltinFunction::OperatorGetItem
            | BuiltinFunction::ItertoolsChain
            | BuiltinFunction::ItertoolsCount
            | BuiltinFunction::ItertoolsCycle
            | BuiltinFunction::ItertoolsRepeat
            | BuiltinFunction::ItertoolsBatched
            | BuiltinFunction::ItertoolsPermutations
            | BuiltinFunction::ItertoolsProduct
            | BuiltinFunction::FunctoolsReduce
            | BuiltinFunction::FunctoolsSingleDispatch
            | BuiltinFunction::FunctoolsSingleDispatchMethod
            | BuiltinFunction::FunctoolsSingleDispatchRegister
            | BuiltinFunction::FunctoolsWraps
            | BuiltinFunction::FunctoolsPartial
            | BuiltinFunction::CollectionsCounter
            | BuiltinFunction::CollectionsDeque
            | BuiltinFunction::CollectionsDefaultDict
            | BuiltinFunction::InspectIsFunction
            | BuiltinFunction::InspectIsClass
            | BuiltinFunction::InspectIsModule
            | BuiltinFunction::InspectIsGenerator
            | BuiltinFunction::InspectIsCoroutine
            | BuiltinFunction::InspectIsAwaitable
            | BuiltinFunction::InspectIsAsyncGen
            | BuiltinFunction::InspectStaticGetMro
            | BuiltinFunction::InspectGetDunderDictOfClass
            | BuiltinFunction::TypesModuleType
            | BuiltinFunction::TypesMethodType
            | BuiltinFunction::TypesNewClass
            | BuiltinFunction::EnumConvert
            | BuiltinFunction::TypeAnnotationsGet
            | BuiltinFunction::IoOpen
            | BuiltinFunction::IoReadText
            | BuiltinFunction::IoWriteText
            | BuiltinFunction::DateTimeNow
            | BuiltinFunction::DateToday
            | BuiltinFunction::AsyncioRun
            | BuiltinFunction::AsyncioSleep
            | BuiltinFunction::AsyncioCreateTask
            | BuiltinFunction::AsyncioGather
            | BuiltinFunction::ThreadingGetIdent
            | BuiltinFunction::ThreadingCurrentThread
            | BuiltinFunction::ThreadingMainThread
            | BuiltinFunction::ThreadingActiveCount
            | BuiltinFunction::SignalSignal
            | BuiltinFunction::SignalGetSignal
            | BuiltinFunction::SignalRaiseSignal
            | BuiltinFunction::ColorizeCanColorize
            | BuiltinFunction::ColorizeGetTheme
            | BuiltinFunction::ColorizeGetColors
            | BuiltinFunction::ColorizeSetTheme
            | BuiltinFunction::WarningsWarn
            | BuiltinFunction::WarningsWarnExplicit
            | BuiltinFunction::WarningsFiltersMutated
            | BuiltinFunction::ObjectNew
            | BuiltinFunction::ObjectInit
            | BuiltinFunction::ObjectGetAttribute
            | BuiltinFunction::ObjectSetAttr
            | BuiltinFunction::ObjectDelAttr
            | BuiltinFunction::Dir
            | BuiltinFunction::ObjectGetState
            | BuiltinFunction::StringFormatterParser
            | BuiltinFunction::StringFormatterFieldNameSplit => {
                Err(RuntimeError::new("builtin requires VM context"))
            }
            BuiltinFunction::BuildClass => Err(RuntimeError::new(
                "__build_class__ is only available in the VM",
            )),
            BuiltinFunction::Import => {
                Err(RuntimeError::new("__import__ is only available in the VM"))
            }
            BuiltinFunction::ImportModule => Err(RuntimeError::new(
                "importlib.import_module() is only available in the VM",
            )),
            BuiltinFunction::FindSpec => Err(RuntimeError::new(
                "importlib.find_spec() is only available in the VM",
            )),
            BuiltinFunction::MarshalLoads => Ok(Value::None),
            BuiltinFunction::MarshalDumps => Ok(heap.alloc_bytes(Vec::new())),
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

fn value_to_float(value: Value) -> Result<f64, RuntimeError> {
    match value {
        Value::Float(value) => Ok(value),
        Value::Int(value) => Ok(value as f64),
        Value::Bool(value) => Ok(if value { 1.0 } else { 0.0 }),
        Value::Complex { real, imag } if imag == 0.0 => Ok(real),
        Value::Str(value) => value
            .trim()
            .parse::<f64>()
            .map_err(|_| RuntimeError::new("invalid float literal")),
        _ => Err(RuntimeError::new("expected numeric value")),
    }
}

fn value_to_complex_pair(value: Value) -> Result<(f64, f64), RuntimeError> {
    match value {
        Value::Complex { real, imag } => Ok((real, imag)),
        Value::Str(text) => parse_complex_literal(&text),
        other => Ok((value_to_float(other)?, 0.0)),
    }
}

fn parse_complex_literal(text: &str) -> Result<(f64, f64), RuntimeError> {
    let trimmed = text.trim();
    if trimmed.ends_with('j') || trimmed.ends_with('J') {
        let without_j = &trimmed[..trimmed.len() - 1];
        let core = without_j.trim();
        if core.is_empty() || core == "+" {
            return Ok((0.0, 1.0));
        }
        if core == "-" {
            return Ok((0.0, -1.0));
        }

        let mut split_idx = None;
        for (idx, ch) in core.char_indices().skip(1) {
            if ch == '+' || ch == '-' {
                split_idx = Some(idx);
            }
        }
        if let Some(idx) = split_idx {
            let (real_part, imag_part) = core.split_at(idx);
            let real = real_part
                .trim()
                .parse::<f64>()
                .map_err(|_| RuntimeError::new("complex() invalid literal"))?;
            let imag = imag_part
                .trim()
                .parse::<f64>()
                .map_err(|_| RuntimeError::new("complex() invalid literal"))?;
            Ok((real, imag))
        } else {
            let imag = core
                .trim()
                .parse::<f64>()
                .map_err(|_| RuntimeError::new("complex() invalid literal"))?;
            Ok((0.0, imag))
        }
    } else {
        let real = trimmed
            .parse::<f64>()
            .map_err(|_| RuntimeError::new("complex() invalid literal"))?;
        Ok((real, 0.0))
    }
}

fn dedup_values(values: Vec<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    for value in values {
        if !out.iter().any(|existing| *existing == value) {
            out.push(value);
        }
    }
    out
}

fn iterable_values(source: Value) -> Result<Vec<Value>, RuntimeError> {
    match source {
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => Ok(values.clone()),
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => Ok(values.clone()),
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => Ok(values.clone()),
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => Ok(values.clone()),
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(values) => Ok(values.iter().map(|(key, _)| key.clone()).collect()),
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => Ok(values.iter().map(|byte| Value::Int(*byte as i64)).collect()),
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => {
                Ok(values.iter().map(|byte| Value::Int(*byte as i64)).collect())
            }
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => match &*view.source.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => {
                    Ok(values.iter().map(|byte| Value::Int(*byte as i64)).collect())
                }
                _ => Err(RuntimeError::new("expected iterable")),
            },
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::Str(value) => Ok(value.chars().map(|ch| Value::Str(ch.to_string())).collect()),
        _ => Err(RuntimeError::new("expected iterable")),
    }
}

fn value_to_bytes_with_encoding(
    value: Value,
    encoding: Option<Value>,
) -> Result<Vec<u8>, RuntimeError> {
    let encoding_name = match encoding {
        Some(Value::Str(name)) => name.to_ascii_lowercase(),
        Some(_) => return Err(RuntimeError::new("encoding must be string")),
        None => "utf-8".to_string(),
    };

    if !matches!(encoding_name.as_str(), "utf-8" | "utf8" | "ascii" | "latin-1" | "latin1") {
        return Err(RuntimeError::new("unsupported encoding"));
    }

    match value {
        Value::None => Ok(Vec::new()),
        Value::Int(size) => {
            if size < 0 {
                return Err(RuntimeError::new("negative count"));
            }
            Ok(vec![0; size as usize])
        }
        Value::Str(text) => {
            if matches!(encoding_name.as_str(), "ascii") && !text.is_ascii() {
                return Err(RuntimeError::new("ascii codec can't encode character"));
            }
            if matches!(encoding_name.as_str(), "latin-1" | "latin1") {
                let mut out = Vec::with_capacity(text.len());
                for ch in text.chars() {
                    let code = ch as u32;
                    if code > 0xFF {
                        return Err(RuntimeError::new("latin-1 codec can't encode character"));
                    }
                    out.push(code as u8);
                }
                Ok(out)
            } else {
                Ok(text.into_bytes())
            }
        }
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => Ok(values.clone()),
            _ => Err(RuntimeError::new("bytes() unsupported type")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => Ok(values.clone()),
            _ => Err(RuntimeError::new("bytes() unsupported type")),
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => match &*view.source.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => Ok(values.clone()),
                _ => Err(RuntimeError::new("bytes() unsupported type")),
            },
            _ => Err(RuntimeError::new("bytes() unsupported type")),
        },
        other => {
            let mut out = Vec::new();
            for item in iterable_values(other)? {
                let value = value_to_int(item)?;
                if !(0..=255).contains(&value) {
                    return Err(RuntimeError::new("bytes must be in range(0, 256)"));
                }
                out.push(value as u8);
            }
            Ok(out)
        }
    }
}

#[derive(Clone, Copy)]
enum NumericValue {
    Int(i64),
    Float(f64),
}

fn numeric_value(value: &Value) -> Option<NumericValue> {
    match value {
        Value::Int(value) => Some(NumericValue::Int(*value)),
        Value::Bool(value) => Some(NumericValue::Int(if *value { 1 } else { 0 })),
        Value::Float(value) => Some(NumericValue::Float(*value)),
        _ => None,
    }
}

fn numeric_compare(left: &Value, right: &Value) -> Option<Ordering> {
    match (numeric_value(left)?, numeric_value(right)?) {
        (NumericValue::Int(left), NumericValue::Int(right)) => Some(left.cmp(&right)),
        (left, right) => {
            let left = match left {
                NumericValue::Int(value) => value as f64,
                NumericValue::Float(value) => value,
            };
            let right = match right {
                NumericValue::Int(value) => value as f64,
                NumericValue::Float(value) => value,
            };
            Some(left.total_cmp(&right))
        }
    }
}

fn add_numeric_values(left: Value, right: Value) -> Result<Value, RuntimeError> {
    match (numeric_value(&left), numeric_value(&right)) {
        (Some(NumericValue::Int(left)), Some(NumericValue::Int(right))) => {
            let value = left
                .checked_add(right)
                .ok_or_else(|| RuntimeError::new("integer overflow"))?;
            Ok(Value::Int(value))
        }
        (Some(left), Some(right)) => {
            let left = match left {
                NumericValue::Int(value) => value as f64,
                NumericValue::Float(value) => value,
            };
            let right = match right {
                NumericValue::Int(value) => value as f64,
                NumericValue::Float(value) => value,
            };
            Ok(Value::Float(left + right))
        }
        _ => Err(RuntimeError::new("unsupported operand type")),
    }
}

fn pow_numeric_values(base: Value, exponent: Value) -> Result<Value, RuntimeError> {
    match (numeric_value(&base), numeric_value(&exponent)) {
        (Some(NumericValue::Int(base)), Some(NumericValue::Int(exp))) if exp >= 0 => {
            let value = base
                .checked_pow(exp as u32)
                .ok_or_else(|| RuntimeError::new("integer overflow"))?;
            Ok(Value::Int(value))
        }
        (Some(base), Some(exponent)) => {
            let base = match base {
                NumericValue::Int(value) => value as f64,
                NumericValue::Float(value) => value,
            };
            let exponent = match exponent {
                NumericValue::Int(value) => value as f64,
                NumericValue::Float(value) => value,
            };
            if base == 0.0 && exponent < 0.0 {
                return Err(RuntimeError::new("division by zero"));
            }
            Ok(Value::Float(base.powf(exponent)))
        }
        _ => Err(RuntimeError::new("unsupported operand type")),
    }
}

fn mod_pow_i64(base: i64, exponent: i64, modulo: i64) -> Result<i64, RuntimeError> {
    if modulo == 0 {
        return Err(RuntimeError::new("pow() modulo by zero"));
    }
    let modulus = modulo as i128;
    let mut acc = 1_i128.rem_euclid(modulus);
    let mut factor = (base as i128).rem_euclid(modulus);
    let mut exp = exponent as u64;
    while exp > 0 {
        if (exp & 1) == 1 {
            acc = (acc * factor).rem_euclid(modulus);
        }
        exp >>= 1;
        if exp > 0 {
            factor = (factor * factor).rem_euclid(modulus);
        }
    }
    i64::try_from(acc).map_err(|_| RuntimeError::new("integer overflow"))
}

fn divmod_values(left: Value, right: Value) -> Result<(Value, Value), RuntimeError> {
    match (numeric_value(&left), numeric_value(&right)) {
        (Some(NumericValue::Int(left)), Some(NumericValue::Int(right))) => {
            if right == 0 {
                return Err(RuntimeError::new("divmod() division by zero"));
            }
            let div = left.div_euclid(right);
            let rem = left.rem_euclid(right);
            Ok((Value::Int(div), Value::Int(rem)))
        }
        (Some(left), Some(right)) => {
            let left = match left {
                NumericValue::Int(value) => value as f64,
                NumericValue::Float(value) => value,
            };
            let right = match right {
                NumericValue::Int(value) => value as f64,
                NumericValue::Float(value) => value,
            };
            if right == 0.0 {
                return Err(RuntimeError::new("divmod() division by zero"));
            }
            let div = (left / right).floor();
            let mut rem = left - div * right;
            if rem == 0.0 {
                rem = 0.0f64.copysign(right);
            }
            Ok((Value::Float(div), Value::Float(rem)))
        }
        _ => Err(RuntimeError::new("unsupported operand type")),
    }
}

fn compare_values(left: &Value, right: &Value) -> Result<Ordering, RuntimeError> {
    if let Some(ordering) = numeric_compare(left, right) {
        return Ok(ordering);
    }

    match (left, right) {
        (Value::Str(a), Value::Str(b)) => Ok(a.cmp(b)),
        _ => Err(RuntimeError::new("min/max unsupported type")),
    }
}

fn builtin_type_of(value: &Value) -> Result<Value, RuntimeError> {
    let ty = match value {
        Value::None => Value::Str("NoneType".to_string()),
        Value::Bool(_) => Value::Builtin(BuiltinFunction::Bool),
        Value::Int(_) => Value::Builtin(BuiltinFunction::Int),
        Value::Float(_) => Value::Builtin(BuiltinFunction::Float),
        Value::Complex { .. } => Value::Builtin(BuiltinFunction::Complex),
        Value::Str(_) => Value::Builtin(BuiltinFunction::Str),
        Value::List(_) => Value::Builtin(BuiltinFunction::List),
        Value::Tuple(_) => Value::Builtin(BuiltinFunction::Tuple),
        Value::Dict(_) => Value::Builtin(BuiltinFunction::Dict),
        Value::Set(_) => Value::Builtin(BuiltinFunction::Set),
        Value::FrozenSet(_) => Value::Builtin(BuiltinFunction::FrozenSet),
        Value::Bytes(_) => Value::Builtin(BuiltinFunction::Bytes),
        Value::ByteArray(_) => Value::Builtin(BuiltinFunction::ByteArray),
        Value::MemoryView(_) => Value::Builtin(BuiltinFunction::MemoryView),
        Value::Iterator(_) => Value::Str("iterator".to_string()),
        Value::Generator(obj) => match &*obj.kind() {
            Object::Generator(generator) if generator.is_async_generator => {
                Value::Str("async_generator".to_string())
            }
            Object::Generator(generator) if generator.is_coroutine => {
                Value::Str("coroutine".to_string())
            }
            Object::Generator(_) => Value::Str("generator".to_string()),
            _ => Value::Str("generator".to_string()),
        },
        Value::Module(_) => Value::Str("module".to_string()),
        Value::Class(class) => Value::Class(class.clone()),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(obj) => Value::Class(obj.class.clone()),
            _ => Value::Str("object".to_string()),
        },
        Value::Super(_) => Value::Str("super".to_string()),
        Value::BoundMethod(_) => Value::Str("method".to_string()),
        Value::Function(_) => Value::Str("function".to_string()),
        Value::Cell(_) => Value::Str("cell".to_string()),
        Value::Exception(_) => Value::ExceptionType("BaseException".to_string()),
        Value::ExceptionType(_) => Value::Str("type".to_string()),
        Value::Slice { .. } => Value::Builtin(BuiltinFunction::Slice),
        Value::Code(_) => Value::Str("code".to_string()),
        Value::Builtin(_) => Value::Builtin(BuiltinFunction::Type),
    };
    Ok(ty)
}

fn format_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        if value.is_sign_negative() {
            return "-inf".to_string();
        }
        return "inf".to_string();
    }
    let mut text = value.to_string();
    if !text.contains('.') && !text.contains('e') && !text.contains('E') {
        text.push_str(".0");
    }
    text
}

fn format_bytes(values: &[u8], mutable: bool) -> String {
    let mut out = String::new();
    if mutable {
        out.push_str("bytearray(");
    }
    out.push('b');
    out.push('\'');
    for byte in values {
        match *byte {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            b'\'' => out.push_str("\\'"),
            32..=126 => out.push(*byte as char),
            _ => out.push_str(&format!("\\x{:02x}", byte)),
        }
    }
    out.push('\'');
    if mutable {
        out.push(')');
    }
    out
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
        Value::Float(value) => format_float(*value),
        Value::Complex { real, imag } => {
            if *real == 0.0 {
                format!("{}j", format_float(*imag))
            } else {
                format!("({}+{}j)", format_float(*real), format_float(*imag))
            }
        }
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
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => {
                if values.is_empty() {
                    "set()".to_string()
                } else {
                    let mut parts = Vec::new();
                    for value in values {
                        parts.push(format_value(value));
                    }
                    format!("{{{}}}", parts.join(", "))
                }
            }
            _ => "<set>".to_string(),
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => {
                let mut parts = Vec::new();
                for value in values {
                    parts.push(format_value(value));
                }
                format!("frozenset({{{}}})", parts.join(", "))
            }
            _ => "<frozenset>".to_string(),
        },
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => format_bytes(values, false),
            _ => "<bytes>".to_string(),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => format_bytes(values, true),
            _ => "<bytearray>".to_string(),
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => format!("<memory at 0x{:x}>", view.source.id()),
            _ => "<memoryview>".to_string(),
        },
        Value::Iterator(_) => "<iterator>".to_string(),
        Value::Generator(obj) => match &*obj.kind() {
            Object::Generator(generator) if generator.is_async_generator => {
                "<async_generator>".to_string()
            }
            Object::Generator(generator) if generator.is_coroutine => "<coroutine>".to_string(),
            Object::Generator(_) => "<generator>".to_string(),
            _ => "<generator>".to_string(),
        },
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
        Value::Super(_) => "<super>".to_string(),
        Value::BoundMethod(obj) => match &*obj.kind() {
            Object::BoundMethod(method) => match &*method.function.kind() {
                Object::Function(func) => format!("<bound method {}>", func.code.name),
                Object::NativeMethod(native) => match native.kind {
                    NativeMethodKind::GeneratorIter => "<bound method __iter__>".to_string(),
                    NativeMethodKind::GeneratorAwait => "<bound method __await__>".to_string(),
                    NativeMethodKind::GeneratorANext => "<bound method __anext__>".to_string(),
                    NativeMethodKind::GeneratorNext => "<bound method __next__>".to_string(),
                    NativeMethodKind::GeneratorSend => "<bound method send>".to_string(),
                    NativeMethodKind::GeneratorThrow => "<bound method throw>".to_string(),
                    NativeMethodKind::GeneratorClose => "<bound method close>".to_string(),
                    NativeMethodKind::DictKeys => "<bound method dict.keys>".to_string(),
                    NativeMethodKind::DictValues => "<bound method dict.values>".to_string(),
                    NativeMethodKind::DictItems => "<bound method dict.items>".to_string(),
                    NativeMethodKind::DictUpdateMethod => "<bound method dict.update>".to_string(),
                    NativeMethodKind::DictSetDefault => {
                        "<bound method dict.setdefault>".to_string()
                    }
                    NativeMethodKind::DictGet => "<bound method dict.get>".to_string(),
                    NativeMethodKind::DictPop => "<bound method dict.pop>".to_string(),
                    NativeMethodKind::ListAppend => "<bound method list.append>".to_string(),
                    NativeMethodKind::ListExtend => "<bound method list.extend>".to_string(),
                    NativeMethodKind::ListInsert => "<bound method list.insert>".to_string(),
                    NativeMethodKind::ListRemove => "<bound method list.remove>".to_string(),
                    NativeMethodKind::ListCount => "<bound method list.count>".to_string(),
                    NativeMethodKind::IntToBytes => "<bound method int.to_bytes>".to_string(),
                    NativeMethodKind::IntBitLengthMethod => {
                        "<bound method int.bit_length>".to_string()
                    }
                    NativeMethodKind::StrStartsWith => "<bound method str.startswith>".to_string(),
                    NativeMethodKind::StrReplace => "<bound method str.replace>".to_string(),
                    NativeMethodKind::StrUpper => "<bound method str.upper>".to_string(),
                    NativeMethodKind::StrLower => "<bound method str.lower>".to_string(),
                    NativeMethodKind::StrEncode => "<bound method str.encode>".to_string(),
                    NativeMethodKind::StrDecode => "<bound method str.decode>".to_string(),
                    NativeMethodKind::BytesDecode => "<bound method bytes.decode>".to_string(),
                    NativeMethodKind::StrRemovePrefix => {
                        "<bound method str.removeprefix>".to_string()
                    }
                    NativeMethodKind::StrRemoveSuffix => {
                        "<bound method str.removesuffix>".to_string()
                    }
                    NativeMethodKind::StrFormat => "<bound method str.format>".to_string(),
                    NativeMethodKind::StrIsUpper => "<bound method str.isupper>".to_string(),
                    NativeMethodKind::StrIsSpace => "<bound method str.isspace>".to_string(),
                    NativeMethodKind::StrJoin => "<bound method str.join>".to_string(),
                    NativeMethodKind::StrSplit => "<bound method str.split>".to_string(),
                    NativeMethodKind::StrLStrip => "<bound method str.lstrip>".to_string(),
                    NativeMethodKind::StrRStrip => "<bound method str.rstrip>".to_string(),
                    NativeMethodKind::StrStrip => "<bound method str.strip>".to_string(),
                    NativeMethodKind::SetContains => "<bound method __contains__>".to_string(),
                    NativeMethodKind::SetAdd => "<bound method set.add>".to_string(),
                    NativeMethodKind::SetUpdate => "<bound method set.update>".to_string(),
                    NativeMethodKind::RePatternSearch => "<bound method Pattern.search>".to_string(),
                    NativeMethodKind::RePatternMatch => "<bound method Pattern.match>".to_string(),
                    NativeMethodKind::RePatternFullMatch => {
                        "<bound method Pattern.fullmatch>".to_string()
                    }
                    NativeMethodKind::RePatternSub => "<bound method Pattern.sub>".to_string(),
                    NativeMethodKind::ClassRegister => "<bound method register>".to_string(),
                    NativeMethodKind::PropertyGet => "<bound method property.__get__>".to_string(),
                    NativeMethodKind::PropertySet => "<bound method property.__set__>".to_string(),
                    NativeMethodKind::PropertyDelete => {
                        "<bound method property.__delete__>".to_string()
                    }
                    NativeMethodKind::PropertyGetter => "<bound method property.getter>".to_string(),
                    NativeMethodKind::PropertySetter => "<bound method property.setter>".to_string(),
                    NativeMethodKind::PropertyDeleter => {
                        "<bound method property.deleter>".to_string()
                    }
                    NativeMethodKind::FunctoolsWrapsDecorator => {
                        "<bound method functools.wraps-decorator>".to_string()
                    }
                    NativeMethodKind::FunctoolsPartialCall => {
                        "<bound method functools.partial-call>".to_string()
                    }
                },
                _ => "<bound method ?>".to_string(),
            },
            _ => "<bound method ?>".to_string(),
        },
        Value::Cell(_) => "<cell>".to_string(),
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
        Value::Float(value) => *value != 0.0,
        Value::Complex { real, imag } => *real != 0.0 || *imag != 0.0,
        Value::Str(value) => !value.is_empty(),
        Value::Cell(_) => true,
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
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => !values.is_empty(),
            _ => true,
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => !values.is_empty(),
            _ => true,
        },
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => !values.is_empty(),
            _ => true,
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => !values.is_empty(),
            _ => true,
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => match &*view.source.kind() {
                Object::Bytes(values) | Object::ByteArray(values) => !values.is_empty(),
                _ => true,
            },
            _ => true,
        },
        Value::Iterator(_) => true,
        Value::Generator(_) => true,
        Value::Slice { .. } => true,
        Value::Module(_)
        | Value::Class(_)
        | Value::Instance(_)
        | Value::Super(_)
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
