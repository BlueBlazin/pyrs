//! Runtime object model (stubbed).

pub mod bigint;
mod dict_backend;

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Index;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use crate::bytecode::CodeObject;
pub use bigint::BigInt;
use dict_backend::DictBackend;

#[derive(Debug)]
pub struct ModuleObject {
    pub name: String,
    pub globals: HashMap<String, Value>,
    pub globals_version: u64,
}

impl ModuleObject {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            globals: HashMap::new(),
            globals_version: 1,
        }
    }

    pub fn touch_globals_version(&mut self) {
        self.globals_version = self.globals_version.wrapping_add(1);
        if self.globals_version == 0 {
            self.globals_version = 1;
        }
    }
}

#[derive(Debug, Clone)]
pub struct FunctionObject {
    pub code: Rc<CodeObject>,
    pub module: ObjRef,
    pub defaults: Vec<Value>,
    pub kwonly_defaults: HashMap<String, Value>,
    pub plain_positional_call_arity: Option<usize>,
    pub call_cache_epoch: u64,
    pub closure: Vec<ObjRef>,
    pub annotations: Option<ObjRef>,
    pub owner_class: Option<ObjRef>,
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
        let plain_positional_call_arity = if defaults.is_empty() && kwonly_defaults.is_empty() {
            code.plain_positional_arity
        } else {
            None
        };
        Self {
            code,
            module,
            defaults,
            kwonly_defaults,
            plain_positional_call_arity,
            call_cache_epoch: 1,
            closure,
            annotations,
            owner_class: None,
            dict: None,
        }
    }

    pub fn refresh_plain_positional_call_arity(&mut self) {
        self.plain_positional_call_arity =
            if self.defaults.is_empty() && self.kwonly_defaults.is_empty() {
                self.code.plain_positional_arity
            } else {
                None
            };
        self.touch_call_cache_epoch();
    }

    pub fn touch_call_cache_epoch(&mut self) {
        self.call_cache_epoch = self.call_cache_epoch.wrapping_add(1);
        if self.call_cache_epoch == 0 {
            self.call_cache_epoch = 1;
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

#[derive(Debug, Clone)]
pub struct DictKeysView {
    pub dict: ObjRef,
}

impl DictKeysView {
    pub fn new(dict: ObjRef) -> Self {
        Self { dict }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeMethodKind {
    Builtin(BuiltinFunction),
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
    DictClear,
    DictUpdateMethod,
    DictSetDefault,
    DictGet,
    DictGetItem,
    DictPop,
    DictCopy,
    ListAppend,
    ListExtend,
    ListInsert,
    ListRemove,
    ListPop,
    ListCount,
    TupleCount,
    ListIndex,
    ListReverse,
    ListSort,
    IntToBytes,
    IntBitLengthMethod,
    IntIndexMethod,
    StrStartsWith,
    StrEndsWith,
    StrReplace,
    StrUpper,
    StrLower,
    StrCapitalize,
    StrEncode,
    StrDecode,
    BytesDecode,
    BytesStartsWith,
    BytesEndsWith,
    BytesCount,
    BytesFind,
    BytesTranslate,
    BytesJoin,
    ByteArrayExtend,
    ByteArrayClear,
    ByteArrayResize,
    MemoryViewEnter,
    MemoryViewExit,
    MemoryViewToReadOnly,
    MemoryViewCast,
    MemoryViewToList,
    MemoryViewRelease,
    StrRemovePrefix,
    StrRemoveSuffix,
    StrFormat,
    StrIsUpper,
    StrIsLower,
    StrIsAscii,
    StrIsAlNum,
    StrIsDigit,
    StrIsSpace,
    StrIsIdentifier,
    StrJoin,
    StrSplit,
    StrSplitLines,
    StrRSplit,
    StrPartition,
    StrRPartition,
    StrCount,
    StrFind,
    StrTranslate,
    StrIndex,
    StrRFind,
    StrLStrip,
    StrRStrip,
    StrStrip,
    StrExpandTabs,
    SetContains,
    SetAdd,
    SetDiscard,
    SetUpdate,
    SetUnion,
    SetIntersection,
    SetDifference,
    SetIsSuperset,
    SetIsSubset,
    SetIsDisjoint,
    RePatternSearch,
    RePatternMatch,
    RePatternFullMatch,
    RePatternSub,
    ReMatchGroup,
    ReMatchGroups,
    ReMatchGroupDict,
    ReMatchStart,
    ReMatchEnd,
    ReMatchSpan,
    ExceptionWithTraceback,
    ExceptionAddNote,
    DescriptorReduceTypeError,
    ObjectReduceExBound,
    BoundMethodReduceEx,
    ComplexReduceEx,
    ClassRegister,
    PropertyGet,
    PropertySet,
    PropertyDelete,
    PropertyGetter,
    PropertySetter,
    PropertyDeleter,
    CachedPropertyGet,
    OperatorItemGetterCall,
    OperatorAttrGetterCall,
    OperatorMethodCallerCall,
    FunctoolsWrapsDecorator,
    FunctoolsPartialCall,
    FunctoolsCmpToKeyCall,
    CodecsIncrementalEncoderFactoryCall,
    CodecsIncrementalDecoderFactoryCall,
    CodecsIncrementalEncoderEncode,
    CodecsIncrementalEncoderReset,
    CodecsIncrementalEncoderGetState,
    CodecsIncrementalEncoderSetState,
    CodecsIncrementalDecoderDecode,
    CodecsIncrementalDecoderReset,
    CodecsIncrementalDecoderGetState,
    CodecsIncrementalDecoderSetState,
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

fn float_key_equal(left: f64, right: f64) -> bool {
    if left == right {
        return true;
    }
    left.is_nan() && right.is_nan() && left.to_bits() == right.to_bits()
}

fn frozenset_key_equal(left: &ObjRef, right: &ObjRef) -> bool {
    let Object::FrozenSet(left_values) = &*left.kind() else {
        return false;
    };
    let Object::FrozenSet(right_values) = &*right.kind() else {
        return false;
    };
    if left_values.len() != right_values.len() {
        return false;
    }
    let mut matched = vec![false; right_values.len()];
    for left_value in left_values {
        let mut found = false;
        for (index, right_value) in right_values.iter().enumerate() {
            if matched[index] {
                continue;
            }
            if value_key_equal(left_value, right_value) {
                matched[index] = true;
                found = true;
                break;
            }
        }
        if !found {
            return false;
        }
    }
    true
}

pub(super) fn value_key_equal(left: &Value, right: &Value) -> bool {
    if left == right {
        return true;
    }
    match (left, right) {
        (Value::Float(left), Value::Float(right)) => float_key_equal(*left, *right),
        (
            Value::Complex {
                real: left_real,
                imag: left_imag,
            },
            Value::Complex {
                real: right_real,
                imag: right_imag,
            },
        ) => float_key_equal(*left_real, *right_real) && float_key_equal(*left_imag, *right_imag),
        (Value::Tuple(left_obj), Value::Tuple(right_obj)) => {
            let Object::Tuple(left_values) = &*left_obj.kind() else {
                return false;
            };
            let Object::Tuple(right_values) = &*right_obj.kind() else {
                return false;
            };
            if left_values.len() != right_values.len() {
                return false;
            }
            left_values
                .iter()
                .zip(right_values.iter())
                .all(|(left_value, right_value)| value_key_equal(left_value, right_value))
        }
        (Value::FrozenSet(left_obj), Value::FrozenSet(right_obj)) => {
            frozenset_key_equal(left_obj, right_obj)
        }
        _ => false,
    }
}

pub(crate) fn value_lookup_hash(value: &Value) -> Option<u64> {
    value_hash_key(value)
}

#[derive(Debug, Clone)]
enum IndexBucket {
    One(usize),
    Many(Vec<usize>),
}

impl IndexBucket {
    fn new(index: usize) -> Self {
        Self::One(index)
    }

    fn push(&mut self, index: usize) {
        match self {
            Self::One(existing) => {
                let first = *existing;
                *self = Self::Many(vec![first, index]);
            }
            Self::Many(indices) => indices.push(index),
        }
    }

    fn find_index_with<F>(&self, mut predicate: F) -> Option<usize>
    where
        F: FnMut(usize) -> bool,
    {
        match self {
            Self::One(index) => predicate(*index).then_some(*index),
            Self::Many(indices) => indices.iter().copied().find(|index| predicate(*index)),
        }
    }

    fn remove_index(&mut self, index: usize) {
        match self {
            Self::One(existing) => {
                if *existing == index {
                    *self = Self::Many(Vec::new());
                }
            }
            Self::Many(indices) => {
                if let Some(position) = indices.iter().position(|existing| *existing == index) {
                    indices.swap_remove(position);
                }
            }
        }
    }

    fn replace_index(&mut self, old_index: usize, new_index: usize) {
        match self {
            Self::One(index) => {
                if *index == old_index {
                    *index = new_index;
                }
            }
            Self::Many(indices) => {
                if let Some(position) = indices.iter().position(|existing| *existing == old_index) {
                    indices[position] = new_index;
                }
            }
        }
    }

    #[cfg(test)]
    fn adjust_indices_after_remove(&mut self, removed_index: usize) {
        match self {
            Self::One(index) => {
                if *index > removed_index {
                    *index -= 1;
                }
            }
            Self::Many(indices) => {
                for index in indices {
                    if *index > removed_index {
                        *index -= 1;
                    }
                }
            }
        }
    }

    fn normalize(&mut self) {
        if let Self::Many(indices) = self {
            if indices.len() == 1 {
                *self = Self::One(indices[0]);
            }
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Self::Many(indices) if indices.is_empty())
    }
}

#[derive(Debug, Clone)]
pub struct DictObject {
    backend: DictBackend,
}

impl DictObject {
    pub fn new(entries: Vec<(Value, Value)>) -> Self {
        Self {
            backend: DictBackend::new(entries),
        }
    }

    pub fn len(&self) -> usize {
        self.backend.len()
    }

    pub fn is_empty(&self) -> bool {
        self.backend.is_empty()
    }

    pub fn clear(&mut self) {
        self.backend.clear();
    }

    pub fn iter(&self) -> std::slice::Iter<'_, (Value, Value)> {
        self.backend.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, (Value, Value)> {
        self.backend.iter_mut()
    }

    pub fn to_vec(&self) -> Vec<(Value, Value)> {
        self.backend.to_vec()
    }

    pub fn push(&mut self, pair: (Value, Value)) {
        self.insert(pair.0, pair.1);
    }

    pub fn remove(&mut self, index: usize) -> (Value, Value) {
        self.backend.remove(index)
    }

    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&(Value, Value)) -> bool,
    {
        self.backend.retain(|entry| f(entry));
    }

    pub fn find(&self, key: &Value) -> Option<&Value> {
        self.backend.find(key)
    }

    pub fn find_with_hash(&self, key: &Value, hash: u64) -> Option<&Value> {
        self.backend.find_with_hash(key, hash)
    }

    pub fn contains_key(&self, key: &Value) -> bool {
        self.backend.contains_key(key)
    }

    pub fn contains_key_with_hash(&self, key: &Value, hash: u64) -> bool {
        self.backend.contains_key_with_hash(key, hash)
    }

    pub fn insert(&mut self, key: Value, value: Value) {
        self.backend.insert(key, value);
    }

    pub fn remove_key(&mut self, key: &Value) -> Option<(Value, Value)> {
        self.backend.remove_key(key)
    }

    pub fn remove_key_with_hash(&mut self, key: &Value, hash: u64) -> Option<(Value, Value)> {
        self.backend.remove_key_with_hash(key, hash)
    }
}

impl PartialEq for DictObject {
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        self.iter().all(|(key, value)| {
            other
                .find(key)
                .is_some_and(|other_value| other_value == value)
        })
    }
}

impl Eq for DictObject {}

impl<'a> IntoIterator for &'a DictObject {
    type Item = &'a (Value, Value);
    type IntoIter = std::slice::Iter<'a, (Value, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        self.backend.iter()
    }
}

impl<'a> IntoIterator for &'a mut DictObject {
    type Item = &'a mut (Value, Value);
    type IntoIter = std::slice::IterMut<'a, (Value, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        self.backend.iter_mut()
    }
}

impl IntoIterator for DictObject {
    type Item = (Value, Value);
    type IntoIter = std::vec::IntoIter<(Value, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        self.backend.into_entries().into_iter()
    }
}

impl Index<usize> for DictObject {
    type Output = (Value, Value);

    fn index(&self, index: usize) -> &Self::Output {
        self.backend.entry_at(index)
    }
}

#[derive(Debug, Clone)]
pub struct SetObject {
    values: Vec<Value>,
    index: HashMap<u64, IndexBucket>,
}

impl SetObject {
    pub fn new(values: Vec<Value>) -> Self {
        let mut out = Self {
            values: Vec::new(),
            index: HashMap::new(),
        };
        for value in values {
            out.insert(value);
        }
        out
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.index.clear();
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Value> {
        self.values.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Value> {
        self.values.iter_mut()
    }

    pub fn to_vec(&self) -> Vec<Value> {
        self.values.clone()
    }

    pub fn push(&mut self, value: Value) {
        self.insert(value);
    }

    pub fn remove(&mut self, index: usize) -> Value {
        let last_index = self.values.len().saturating_sub(1);
        let removed = self.values.swap_remove(index);
        if let Some(hash) = value_lookup_hash(&removed) {
            self.remove_hash_index(hash, index);
        }
        if index < self.values.len() {
            let moved = &self.values[index];
            if let Some(moved_hash) = value_lookup_hash(moved) {
                if let Some(bucket) = self.index.get_mut(&moved_hash) {
                    bucket.replace_index(last_index, index);
                }
            }
        }
        removed
    }

    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&Value) -> bool,
    {
        self.values.retain(|value| f(value));
        self.rebuild_index();
    }

    pub fn contains(&self, value: &Value) -> bool {
        if let Some(hash) = value_lookup_hash(value) {
            if let Some(bucket) = self.index.get(&hash) {
                return bucket
                    .find_index_with(|index| value_key_equal(&self.values[index], value))
                    .is_some();
            }
            return false;
        }
        self.values.iter().any(|item| value_key_equal(item, value))
    }

    pub fn insert(&mut self, value: Value) -> bool {
        if self.find_index(&value).is_some() {
            return false;
        }
        let index = self.values.len();
        self.values.push(value);
        if let Some(hash) = value_lookup_hash(&self.values[index]) {
            self.index
                .entry(hash)
                .and_modify(|bucket| bucket.push(index))
                .or_insert_with(|| IndexBucket::new(index));
        }
        true
    }

    pub fn remove_value(&mut self, value: &Value) -> bool {
        let Some(index) = self.find_index(value) else {
            return false;
        };
        self.remove(index);
        true
    }

    fn find_index(&self, value: &Value) -> Option<usize> {
        if let Some(hash) = value_lookup_hash(value) {
            if let Some(bucket) = self.index.get(&hash) {
                if let Some(index) =
                    bucket.find_index_with(|index| value_key_equal(&self.values[index], value))
                {
                    return Some(index);
                }
                return None;
            }
        }
        self.values
            .iter()
            .position(|item| value_key_equal(item, value))
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for (index, value) in self.values.iter().enumerate() {
            if let Some(hash) = value_lookup_hash(value) {
                self.index
                    .entry(hash)
                    .and_modify(|bucket| bucket.push(index))
                    .or_insert_with(|| IndexBucket::new(index));
            }
        }
    }

    fn remove_hash_index(&mut self, hash: u64, entry_index: usize) {
        let mut bucket_is_empty = false;
        if let Some(bucket) = self.index.get_mut(&hash) {
            bucket.remove_index(entry_index);
            bucket.normalize();
            bucket_is_empty = bucket.is_empty();
        }
        if bucket_is_empty {
            self.index.remove(&hash);
        }
    }
}

impl PartialEq for SetObject {
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        self.values.iter().all(|value| other.contains(value))
    }
}

impl Eq for SetObject {}

impl<'a> IntoIterator for &'a SetObject {
    type Item = &'a Value;
    type IntoIter = std::slice::Iter<'a, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

impl<'a> IntoIterator for &'a mut SetObject {
    type Item = &'a mut Value;
    type IntoIter = std::slice::IterMut<'a, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut()
    }
}

impl IntoIterator for SetObject {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl Index<usize> for SetObject {
    type Output = Value;

    fn index(&self, index: usize) -> &Self::Output {
        &self.values[index]
    }
}

#[inline]
fn hash_mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value
}

#[inline]
fn fast_numeric_hash_bits(value: &Value) -> Option<u64> {
    match value {
        Value::Bool(boolean) => Some((*boolean as i64 as f64).to_bits()),
        Value::Int(integer) => Some((*integer as f64).to_bits()),
        Value::BigInt(integer) => {
            if let Some(small) = integer.to_i64() {
                Some((small as f64).to_bits())
            } else {
                let as_float = integer.to_f64();
                if as_float.is_finite()
                    && BigInt::from_f64_integral(as_float)
                        .as_ref()
                        .is_some_and(|converted| converted == integer.as_ref())
                {
                    Some(if as_float == 0.0 { 0.0 } else { as_float }.to_bits())
                } else {
                    None
                }
            }
        }
        Value::Float(float) => Some(if *float == 0.0 { 0.0 } else { *float }.to_bits()),
        _ => None,
    }
}

#[inline]
fn fast_value_hash_key(value: &Value) -> Option<u64> {
    match value {
        Value::None => Some(hash_mix64(0x00)),
        Value::Bool(_) | Value::Int(_) | Value::BigInt(_) | Value::Float(_) => {
            let bits = fast_numeric_hash_bits(value)?;
            Some(hash_mix64((1u64 << 56) ^ bits))
        }
        _ => None,
    }
}

fn value_hash_key(value: &Value) -> Option<u64> {
    if let Some(hash) = fast_value_hash_key(value) {
        return Some(hash);
    }
    let mut hasher = DefaultHasher::new();
    match value {
        Value::BigInt(integer) => {
            1u8.hash(&mut hasher);
            let as_float = integer.to_f64();
            if as_float.is_finite()
                && BigInt::from_f64_integral(as_float)
                    .as_ref()
                    .is_some_and(|converted| converted == integer.as_ref())
            {
                let normalized = if as_float == 0.0 { 0.0 } else { as_float };
                normalized.to_bits().hash(&mut hasher);
            } else {
                0xffu8.hash(&mut hasher);
                integer.hash(&mut hasher);
            }
        }
        Value::Complex { real, imag } => {
            2u8.hash(&mut hasher);
            let real = if *real == 0.0 { 0.0 } else { *real };
            let imag = if *imag == 0.0 { 0.0 } else { *imag };
            real.to_bits().hash(&mut hasher);
            imag.to_bits().hash(&mut hasher);
        }
        Value::Str(text) => {
            3u8.hash(&mut hasher);
            text.hash(&mut hasher);
        }
        Value::Tuple(tuple) => {
            4u8.hash(&mut hasher);
            let Object::Tuple(values) = &*tuple.kind() else {
                return None;
            };
            for value in values {
                value_hash_key(value)?.hash(&mut hasher);
            }
        }
        Value::FrozenSet(set) => {
            5u8.hash(&mut hasher);
            let Object::FrozenSet(values) = &*set.kind() else {
                return None;
            };
            let mut folded: u64 = 0;
            for value in values {
                folded ^= value_hash_key(value)?;
            }
            folded.hash(&mut hasher);
            values.len().hash(&mut hasher);
        }
        Value::Bytes(bytes) => {
            6u8.hash(&mut hasher);
            let Object::Bytes(values) = &*bytes.kind() else {
                return None;
            };
            values.hash(&mut hasher);
        }
        Value::Exception(exception) => {
            7u8.hash(&mut hasher);
            exception.hash(&mut hasher);
        }
        Value::ExceptionType(name) => {
            8u8.hash(&mut hasher);
            name.hash(&mut hasher);
        }
        Value::Builtin(builtin) => {
            9u8.hash(&mut hasher);
            builtin.hash(&mut hasher);
        }
        Value::Code(code) => {
            10u8.hash(&mut hasher);
            Rc::as_ptr(code).hash(&mut hasher);
        }
        Value::Module(obj)
        | Value::Class(obj)
        | Value::Instance(obj)
        | Value::Super(obj)
        | Value::Function(obj)
        | Value::BoundMethod(obj)
        | Value::Iterator(obj)
        | Value::Generator(obj)
        | Value::Cell(obj) => {
            11u8.hash(&mut hasher);
            obj.id().hash(&mut hasher);
        }
        Value::List(_)
        | Value::Dict(_)
        | Value::DictKeys(_)
        | Value::Set(_)
        | Value::ByteArray(_)
        | Value::MemoryView(_)
        | Value::Slice(_) => return None,
        Value::None | Value::Bool(_) | Value::Int(_) | Value::Float(_) => unreachable!(),
    }
    Some(hasher.finish())
}

#[derive(Debug)]
pub enum Object {
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Dict(DictObject),
    Set(SetObject),
    FrozenSet(SetObject),
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
    DictKeysView(DictKeysView),
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
    Cycle {
        values: Vec<Value>,
    },
    Count {
        current: i64,
        step: i64,
    },
    RangeObject {
        start: BigInt,
        stop: BigInt,
        step: BigInt,
    },
    Map {
        values: Vec<Value>,
        func: Value,
        iterators: Vec<Value>,
        sources: Vec<Value>,
        exhausted: bool,
    },
    Range {
        current: BigInt,
        stop: BigInt,
        step: BigInt,
    },
    SequenceGetItem {
        target: Value,
        getitem: Value,
    },
}

#[derive(Debug)]
pub struct MemoryViewObject {
    pub source: ObjRef,
    pub itemsize: usize,
    pub format: Option<String>,
    pub export_owner: Option<ObjRef>,
    pub released: bool,
    pub start: usize,
    pub length: Option<usize>,
}

#[derive(Debug)]
pub struct Heap {
    next_id: Cell<u64>,
    registry: RefCell<Vec<Weak<Obj>>>,
    small_int_ids: RefCell<Vec<u64>>,
    immediate_ids: RefCell<HashMap<ImmediateKey, u64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ImmediateKey {
    None,
    Bool(bool),
    Int(i64),
    BigInt(BigInt),
    Float(u64),
    Complex(u64, u64),
    Str(String),
    Code(u64),
    Exception(u64),
    ExceptionType(String),
    Slice(Option<i64>, Option<i64>, Option<i64>),
    Builtin(BuiltinFunction),
}

const SMALL_INT_MIN: i64 = -5;
const SMALL_INT_MAX: i64 = 256;
const SMALL_INT_COUNT: usize = (SMALL_INT_MAX - SMALL_INT_MIN + 1) as usize;

static NEXT_EXCEPTION_OBJECT_ID: AtomicU64 = AtomicU64::new(1);

fn next_exception_object_id() -> u64 {
    NEXT_EXCEPTION_OBJECT_ID.fetch_add(1, AtomicOrdering::Relaxed)
}

impl Heap {
    pub fn new() -> Self {
        Self {
            next_id: Cell::new(1),
            registry: RefCell::new(Vec::new()),
            small_int_ids: RefCell::new(vec![0; SMALL_INT_COUNT]),
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
        Value::Dict(self.alloc(Object::Dict(DictObject::new(values))))
    }

    pub fn alloc_dict_keys_view(&self, dict: ObjRef) -> Value {
        Value::DictKeys(self.alloc(Object::DictKeysView(DictKeysView::new(dict))))
    }

    pub fn alloc_set(&self, values: Vec<Value>) -> Value {
        Value::Set(self.alloc(Object::Set(SetObject::new(values))))
    }

    pub fn alloc_frozenset(&self, values: Vec<Value>) -> Value {
        Value::FrozenSet(self.alloc(Object::FrozenSet(SetObject::new(values))))
    }

    pub fn alloc_bytes(&self, values: Vec<u8>) -> Value {
        Value::Bytes(self.alloc(Object::Bytes(values)))
    }

    pub fn alloc_bytearray(&self, values: Vec<u8>) -> Value {
        Value::ByteArray(self.alloc(Object::ByteArray(values)))
    }

    pub fn alloc_memoryview(&self, source: ObjRef) -> Value {
        Value::MemoryView(self.alloc(Object::MemoryView(MemoryViewObject {
            source,
            itemsize: 1,
            format: None,
            export_owner: None,
            released: false,
            start: 0,
            length: None,
        })))
    }

    pub fn alloc_memoryview_with(
        &self,
        source: ObjRef,
        itemsize: usize,
        format: Option<String>,
    ) -> Value {
        Value::MemoryView(self.alloc(Object::MemoryView(MemoryViewObject {
            source,
            itemsize,
            format,
            export_owner: None,
            released: false,
            start: 0,
            length: None,
        })))
    }

    pub fn count_live_memoryview_exports_for_owner(&self, owner: &ObjRef) -> usize {
        let mut count = 0usize;
        for weak in self.registry.borrow().iter() {
            let Some(obj) = weak.upgrade() else {
                continue;
            };
            let Object::MemoryView(view) = &*obj.kind.borrow() else {
                continue;
            };
            if view.released {
                continue;
            }
            if let Some(export_owner) = &view.export_owner {
                if export_owner.id() == owner.id() {
                    count += 1;
                }
            }
        }
        count
    }

    pub fn count_live_memoryview_exports_for_source(&self, source: &ObjRef) -> usize {
        let mut count = 0usize;
        for weak in self.registry.borrow().iter() {
            let Some(obj) = weak.upgrade() else {
                continue;
            };
            let Object::MemoryView(view) = &*obj.kind.borrow() else {
                continue;
            };
            if view.released || view.export_owner.is_none() {
                continue;
            }
            if view.source.id() == source.id() {
                count += 1;
            }
        }
        count
    }

    pub fn count_live_memoryviews_for_source(&self, source: &ObjRef) -> usize {
        let mut count = 0usize;
        for weak in self.registry.borrow().iter() {
            let Some(obj) = weak.upgrade() else {
                continue;
            };
            let Object::MemoryView(view) = &*obj.kind.borrow() else {
                continue;
            };
            if view.released {
                continue;
            }
            if view.source.id() == source.id() {
                count += 1;
            }
        }
        count
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
            Value::Int(value) => {
                if *value >= SMALL_INT_MIN && *value <= SMALL_INT_MAX {
                    self.id_for_small_int(*value)
                } else {
                    self.id_for_immediate(ImmediateKey::Int(*value))
                }
            }
            Value::BigInt(value) => self.id_for_immediate(ImmediateKey::BigInt((**value).clone())),
            Value::Float(value) => self.id_for_immediate(ImmediateKey::Float(value.to_bits())),
            Value::Complex { real, imag } => {
                self.id_for_immediate(ImmediateKey::Complex(real.to_bits(), imag.to_bits()))
            }
            Value::Str(value) => self.id_for_immediate(ImmediateKey::Str(value.clone())),
            Value::List(obj)
            | Value::Tuple(obj)
            | Value::Dict(obj)
            | Value::DictKeys(obj)
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
            Value::Exception(exception) => {
                self.id_for_immediate(ImmediateKey::Exception(exception.object_id))
            }
            Value::ExceptionType(name) => {
                self.id_for_immediate(ImmediateKey::ExceptionType(name.clone()))
            }
            Value::Slice(slice) => {
                self.id_for_immediate(ImmediateKey::Slice(slice.lower, slice.upper, slice.step))
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

    fn id_for_small_int(&self, value: i64) -> u64 {
        let index = (value - SMALL_INT_MIN) as usize;
        let mut ids = self.small_int_ids.borrow_mut();
        let id = ids[index];
        if id != 0 {
            return id;
        }
        let id = self.next_id();
        ids[index] = id;
        id
    }

    pub fn collect_cycles(&self, roots: &[Value]) {
        let marked = self.reachable_object_ids(roots);

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

    pub fn unreachable_objects(&self, roots: &[Value]) -> Vec<ObjRef> {
        let marked = self.reachable_object_ids(roots);
        let mut registry = self.registry.borrow_mut();
        registry.retain(|weak| weak.strong_count() > 0);
        let mut out = Vec::new();
        for weak in registry.iter() {
            if let Some(obj) = weak.upgrade() {
                let obj_ref = ObjRef::from_rc(obj);
                if !marked.contains_key(&obj_ref.id()) {
                    out.push(obj_ref);
                }
            }
        }
        out
    }

    pub fn live_objects_count(&self) -> usize {
        self.registry
            .borrow()
            .iter()
            .filter(|weak| weak.strong_count() > 0)
            .count()
    }

    pub fn find_object_by_id(&self, id: u64) -> Option<ObjRef> {
        let mut registry = self.registry.borrow_mut();
        registry.retain(|weak| weak.strong_count() > 0);
        for weak in registry.iter() {
            if let Some(obj) = weak.upgrade() {
                let obj_ref = ObjRef::from_rc(obj);
                if obj_ref.id() == id {
                    return Some(obj_ref);
                }
            }
        }
        None
    }

    fn reachable_object_ids(&self, roots: &[Value]) -> HashMap<u64, bool> {
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
        marked
    }
}

fn trace_value(value: &Value, stack: &mut Vec<ObjRef>, marked: &mut HashMap<u64, bool>) {
    match value {
        Value::List(obj)
        | Value::Tuple(obj)
        | Value::Dict(obj)
        | Value::DictKeys(obj)
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
        Object::DictKeysView(view) => {
            stack.push(view.dict.clone());
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
            IteratorKind::Cycle { values } => {
                for value in values {
                    trace_value(value, stack, marked);
                }
            }
            IteratorKind::Map {
                values,
                func,
                iterators,
                sources,
                ..
            } => {
                for value in values {
                    trace_value(value, stack, marked);
                }
                trace_value(func, stack, marked);
                for iterator in iterators {
                    trace_value(iterator, stack, marked);
                }
                for source in sources {
                    trace_value(source, stack, marked);
                }
            }
            IteratorKind::SequenceGetItem { target, getitem } => {
                trace_value(target, stack, marked);
                trace_value(getitem, stack, marked);
            }
            IteratorKind::Str(_)
            | IteratorKind::Count { .. }
            | IteratorKind::RangeObject { .. }
            | IteratorKind::Range { .. } => {}
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
    let mut kind = obj.kind_mut();
    let replacement = match &mut *kind {
        Object::List(values) | Object::Tuple(values) => {
            values.clear();
            None
        }
        Object::Dict(entries) => {
            entries.clear();
            None
        }
        Object::DictKeysView(_) => Some(Object::Bytes(Vec::new())),
        Object::Set(values) | Object::FrozenSet(values) => {
            values.clear();
            None
        }
        Object::Bytes(values) | Object::ByteArray(values) => {
            values.clear();
            None
        }
        Object::MemoryView(_) => Some(Object::Bytes(Vec::new())),
        Object::Iterator(iterator) => {
            match &mut iterator.kind {
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
                IteratorKind::Cycle { values } => {
                    values.clear();
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
                IteratorKind::Map {
                    values,
                    iterators,
                    sources,
                    ..
                } => {
                    values.clear();
                    iterators.clear();
                    sources.clear();
                    iterator.kind = IteratorKind::Str(String::new());
                    iterator.index = 0;
                }
                IteratorKind::RangeObject { .. } | IteratorKind::Range { .. } => {
                    iterator.kind = IteratorKind::Str(String::new());
                    iterator.index = 0;
                }
                IteratorKind::SequenceGetItem { .. } => {
                    iterator.kind = IteratorKind::Str(String::new());
                    iterator.index = 0;
                }
            }
            None
        }
        Object::Generator(generator) => {
            generator.started = false;
            generator.running = false;
            generator.closed = true;
            None
        }
        Object::Module(module) => {
            module.globals.clear();
            None
        }
        Object::Class(class) => {
            class.bases.clear();
            class.mro.clear();
            class.attrs.clear();
            class.slots = None;
            class.metaclass = None;
            None
        }
        Object::Instance(instance) => {
            instance.attrs.clear();
            None
        }
        Object::Super(_) => Some(Object::Bytes(Vec::new())),
        Object::Function(func) => {
            func.defaults.clear();
            func.kwonly_defaults.clear();
            func.refresh_plain_positional_call_arity();
            func.closure.clear();
            func.annotations = None;
            func.dict = None;
            None
        }
        Object::BoundMethod(_) => Some(Object::Bytes(Vec::new())),
        Object::NativeMethod(_) => None,
        Object::Cell(cell) => {
            cell.value = None;
            None
        }
    };

    if let Some(replacement) = replacement {
        *kind = replacement;
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    BigInt(Box<BigInt>),
    Float(f64),
    Complex { real: f64, imag: f64 },
    Str(String),
    List(ObjRef),
    Tuple(ObjRef),
    Dict(ObjRef),
    DictKeys(ObjRef),
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
    Exception(Box<ExceptionObject>),
    ExceptionType(String),
    Slice(Box<SliceValue>),
    Code(Rc<CodeObject>),
    Builtin(BuiltinFunction),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SliceValue {
    pub lower: Option<i64>,
    pub upper: Option<i64>,
    pub step: Option<i64>,
}

impl SliceValue {
    pub fn new(lower: Option<i64>, upper: Option<i64>, step: Option<i64>) -> Self {
        Self { lower, upper, step }
    }
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
                Object::Dict(values) => Some(values.to_vec()),
                _ => None,
            },
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExceptionObject {
    pub object_id: u64,
    pub name: String,
    pub message: Option<String>,
    pub notes: Vec<String>,
    pub exceptions: Vec<ExceptionObject>,
    pub cause: Option<Box<ExceptionObject>>,
    pub context: Option<Box<ExceptionObject>>,
    pub suppress_context: bool,
    pub attrs: Rc<RefCell<HashMap<String, Value>>>,
}

impl ExceptionObject {
    pub fn new(name: impl Into<String>, message: Option<String>) -> Self {
        Self {
            object_id: next_exception_object_id(),
            name: name.into(),
            message,
            notes: Vec::new(),
            exceptions: Vec::new(),
            cause: None,
            context: None,
            suppress_context: false,
            attrs: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    pub fn with_members(
        name: impl Into<String>,
        message: Option<String>,
        members: Vec<ExceptionObject>,
    ) -> Self {
        Self {
            object_id: next_exception_object_id(),
            name: name.into(),
            message,
            notes: Vec::new(),
            exceptions: members,
            cause: None,
            context: None,
            suppress_context: false,
            attrs: Rc::new(RefCell::new(HashMap::new())),
        }
    }
}

impl PartialEq for ExceptionObject {
    fn eq(&self, other: &Self) -> bool {
        self.object_id == other.object_id
    }
}

impl Eq for ExceptionObject {}

impl Hash for ExceptionObject {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.object_id.hash(state);
    }
}

fn instance_bytes_storage(instance: &ObjRef) -> Option<Vec<u8>> {
    let Object::Instance(instance_data) = &*instance.kind() else {
        return None;
    };
    let storage = instance_data.attrs.get("__pyrs_bytes_storage__")?;
    match storage {
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => Some(values.clone()),
            _ => None,
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => Some(values.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn bytearray_payload(obj: &ObjRef) -> Option<Vec<u8>> {
    match &*obj.kind() {
        Object::ByteArray(values) => Some(values.clone()),
        _ => None,
    }
}

fn bytes_payload(obj: &ObjRef) -> Option<Vec<u8>> {
    match &*obj.kind() {
        Object::Bytes(values) => Some(values.clone()),
        _ => None,
    }
}

fn memoryview_payload(obj: &ObjRef) -> Option<Vec<u8>> {
    match &*obj.kind() {
        Object::MemoryView(view) => with_bytes_like_source(&view.source, |values| {
            let (start, end) = memoryview_bounds(view.start, view.length, values.len());
            values[start..end].to_vec()
        }),
        _ => None,
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::None, Value::None) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Bool(a), Value::Int(b)) => (*a as i64) == *b,
            (Value::Bool(a), Value::BigInt(b)) => BigInt::from_i64(*a as i64) == **b,
            (Value::Bool(a), Value::Float(b)) => (*a as i64 as f64) == *b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Int(a), Value::Bool(b)) => *a == (*b as i64),
            (Value::Int(a), Value::BigInt(b)) => BigInt::from_i64(*a) == **b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::BigInt(a), Value::BigInt(b)) => a == b,
            (Value::BigInt(a), Value::Bool(b)) => **a == BigInt::from_i64(*b as i64),
            (Value::BigInt(a), Value::Int(b)) => **a == BigInt::from_i64(*b),
            (Value::BigInt(a), Value::Float(b)) => BigInt::from_f64_integral(*b)
                .as_ref()
                .is_some_and(|converted| converted == a.as_ref()),
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::Float(a), Value::BigInt(b)) => BigInt::from_f64_integral(*a)
                .as_ref()
                .is_some_and(|converted| converted == b.as_ref()),
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
            (Value::Set(a), Value::Set(b))
            | (Value::Set(a), Value::FrozenSet(b))
            | (Value::FrozenSet(a), Value::Set(b))
            | (Value::FrozenSet(a), Value::FrozenSet(b)) => match (&*a.kind(), &*b.kind()) {
                (
                    Object::Set(left) | Object::FrozenSet(left),
                    Object::Set(right) | Object::FrozenSet(right),
                ) => left == right,
                _ => false,
            },
            (Value::Bytes(a), Value::Bytes(b)) | (Value::ByteArray(a), Value::ByteArray(b)) => {
                match (&*a.kind(), &*b.kind()) {
                    (Object::Bytes(left), Object::Bytes(right))
                    | (Object::ByteArray(left), Object::ByteArray(right)) => left == right,
                    _ => false,
                }
            }
            (Value::Bytes(a), Value::ByteArray(b)) | (Value::ByteArray(b), Value::Bytes(a)) => {
                match (&*a.kind(), &*b.kind()) {
                    (Object::Bytes(left), Object::ByteArray(right)) => left == right,
                    _ => false,
                }
            }
            (Value::MemoryView(a), Value::MemoryView(b)) => {
                if let (Some(left), Some(right)) = (memoryview_payload(a), memoryview_payload(b)) {
                    left == right
                } else {
                    a.id() == b.id()
                }
            }
            (Value::MemoryView(view), Value::Bytes(obj))
            | (Value::Bytes(obj), Value::MemoryView(view)) => {
                if let (Some(left), Some(right)) = (memoryview_payload(view), bytes_payload(obj)) {
                    left == right
                } else {
                    false
                }
            }
            (Value::MemoryView(view), Value::ByteArray(obj))
            | (Value::ByteArray(obj), Value::MemoryView(view)) => {
                if let (Some(left), Some(right)) =
                    (memoryview_payload(view), bytearray_payload(obj))
                {
                    left == right
                } else {
                    false
                }
            }
            (Value::MemoryView(view), Value::Instance(instance))
            | (Value::Instance(instance), Value::MemoryView(view)) => {
                if let (Some(left), Some(right)) =
                    (memoryview_payload(view), instance_bytes_storage(instance))
                {
                    left == right
                } else {
                    false
                }
            }
            (Value::Iterator(a), Value::Iterator(b)) => a.id() == b.id(),
            (Value::Generator(a), Value::Generator(b)) => a.id() == b.id(),
            (Value::Module(a), Value::Module(b))
            | (Value::Class(a), Value::Class(b))
            | (Value::Super(a), Value::Super(b))
            | (Value::Function(a), Value::Function(b))
            | (Value::BoundMethod(a), Value::BoundMethod(b))
            | (Value::Cell(a), Value::Cell(b)) => a.id() == b.id(),
            (Value::Instance(a), Value::Instance(b)) => {
                if let (Some(left), Some(right)) =
                    (instance_bytes_storage(a), instance_bytes_storage(b))
                {
                    left == right
                } else {
                    a.id() == b.id()
                }
            }
            (Value::Instance(instance), Value::ByteArray(obj))
            | (Value::ByteArray(obj), Value::Instance(instance)) => {
                if let (Some(left), Some(right)) =
                    (instance_bytes_storage(instance), bytearray_payload(obj))
                {
                    left == right
                } else {
                    false
                }
            }
            (Value::Instance(instance), Value::Bytes(obj))
            | (Value::Bytes(obj), Value::Instance(instance)) => {
                if let (Some(left), Some(right)) =
                    (instance_bytes_storage(instance), bytes_payload(obj))
                {
                    left == right
                } else {
                    false
                }
            }
            (Value::Exception(a), Value::Exception(b)) => a == b,
            (Value::ExceptionType(a), Value::ExceptionType(b)) => a == b,
            (Value::Slice(a), Value::Slice(b)) => {
                a.lower == b.lower && a.upper == b.upper && a.step == b.step
            }
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
    Ascii,
    DictTypeRepr,
    ListTypeRepr,
    TupleTypeRepr,
    SetTypeRepr,
    FrozenSetTypeRepr,
    StrTypeRepr,
    BytesTypeRepr,
    ByteArrayTypeRepr,
    MappingProxyTypeRepr,
    SimpleNamespaceTypeRepr,
    NoOp,
    Len,
    Range,
    Slice,
    Bool,
    Int,
    IntBitLength,
    IntFromBytes,
    Float,
    FloatFromHex,
    FloatHex,
    Str,
    StrMakeTrans,
    BytesMakeTrans,
    Compile,
    Ord,
    Chr,
    Bin,
    Oct,
    Hex,
    Abs,
    Sum,
    Min,
    Max,
    All,
    Any,
    Map,
    Filter,
    Pow,
    Round,
    Format,
    List,
    ListAppendDescriptor,
    Tuple,
    Dict,
    DictFromKeys,
    Set,
    SetReduce,
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
    ObjectSetState,
    ObjectReduce,
    ObjectReduceEx,
    ObjectSetAttr,
    ObjectDelAttr,
    ContextVar,
    ContextVarGet,
    ContextVarSet,
    ContextCopyContext,
    ThreadRLock,
    ThreadStartNewThread,
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
    SysException,
    SysExcInfo,
    SysExit,
    SysGetFilesystemEncoding,
    SysGetFilesystemEncodeErrors,
    SysGetRefCount,
    SysGetRecursionLimit,
    SysSetRecursionLimit,
    SysStdoutWrite,
    SysStdoutBufferWrite,
    SysStdoutFlush,
    SysStderrWrite,
    SysStderrBufferWrite,
    SysStderrFlush,
    SysStdinWrite,
    SysStdinFlush,
    SysStreamIsATty,
    PlatformLibcVer,
    PlatformWin32IsIot,
    Import,
    Exec,
    ImportModule,
    FindSpec,
    ImportlibInvalidateCaches,
    ImportlibSourceFromCache,
    ImportlibCacheFromSource,
    ImportlibSpecFromFileLocation,
    FrozenImportlibSpecFromLoader,
    FrozenImportlibVerboseMessage,
    FrozenImportlibExternalPathJoin,
    FrozenImportlibExternalPathSplit,
    FrozenImportlibExternalPathStat,
    FrozenImportlibExternalUnpackUint16,
    FrozenImportlibExternalUnpackUint32,
    FrozenImportlibExternalUnpackUint64,
    OpcodeStackEffect,
    OpcodeHasArg,
    OpcodeHasConst,
    OpcodeHasName,
    OpcodeHasJump,
    OpcodeHasFree,
    OpcodeHasLocal,
    OpcodeHasExc,
    OpcodeGetExecutor,
    RandomSeed,
    RandomRandom,
    RandomRandRange,
    RandomRandInt,
    RandomGetRandBits,
    RandomChoice,
    RandomChoices,
    RandomShuffle,
    DecimalGetContext,
    DecimalSetContext,
    DecimalLocalContext,
    WeakRefRef,
    WeakRefProxy,
    WeakRefFinalize,
    WeakRefFinalizeDetach,
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
    MathLdExp,
    MathHypot,
    MathFAbs,
    MathExp,
    MathErfc,
    MathLog,
    MathFSum,
    MathSumProd,
    MathCos,
    MathSin,
    MathTan,
    MathCosh,
    MathAsin,
    MathAtan,
    MathAcos,
    MathIsClose,
    TimeTime,
    TimeTimeNs,
    TimeLocalTime,
    TimeGmTime,
    TimeStrFTime,
    TimeMonotonic,
    TimeSleep,
    OsGetPid,
    OsGetCwd,
    OsGetEnv,
    OsGetTerminalSize,
    OsTerminalSize,
    OsOpen,
    OsPipe,
    OsRead,
    OsReadInto,
    OsWrite,
    OsDup,
    OsLSeek,
    OsFTruncate,
    OsClose,
    OsKill,
    OsIsATty,
    OsSetInheritable,
    OsGetInheritable,
    OsURandom,
    OsStat,
    OsLStat,
    OsMkdir,
    OsChmod,
    OsRmdir,
    OsUTime,
    OsScandir,
    OsScandirIter,
    OsScandirNext,
    OsScandirEnter,
    OsScandirExit,
    OsScandirClose,
    OsDirEntryIsDir,
    OsDirEntryIsFile,
    OsDirEntryIsSymlink,
    OsWalk,
    OsWIfStopped,
    OsWStopSig,
    OsWIfSignaled,
    OsWTermSig,
    OsWIfExited,
    OsWExitStatus,
    OsListDir,
    OsAccess,
    OsFspath,
    OsFsEncode,
    OsFsDecode,
    OsRemove,
    OsWaitStatusToExitCode,
    OsPathExists,
    OsPathJoin,
    OsPathNormPath,
    OsPathNormCase,
    OsPathSplitRootEx,
    OsPathSplit,
    OsPathDirName,
    OsPathBaseName,
    OsPathIsAbs,
    OsPathIsDir,
    OsPathIsFile,
    OsPathIsLink,
    OsPathIsJunction,
    OsPathSplitExt,
    OsPathAbsPath,
    OsPathExpandUser,
    OsPathRealPath,
    OsPathRelPath,
    OsPathCommonPrefix,
    OsWaitPid,
    PosixSubprocessForkExec,
    SubprocessPopenInit,
    SubprocessPopenCommunicate,
    SubprocessPopenWait,
    SubprocessPopenKill,
    SubprocessPopenPoll,
    SubprocessPopenEnter,
    SubprocessPopenExit,
    SubprocessCleanup,
    SubprocessCheckCall,
    JsonDumps,
    JsonLoads,
    JsonEncodeBaseString,
    JsonEncodeBaseStringAscii,
    JsonMakeEncoder,
    JsonMakeEncoderCall,
    PickleDump,
    PickleDumps,
    PickleLoad,
    PickleLoads,
    PickleModuleGetAttr,
    PicklePicklerInit,
    PicklePicklerDump,
    PicklePicklerClearMemo,
    PicklePicklerPersistentId,
    PickleUnpicklerInit,
    PickleUnpicklerLoad,
    PickleUnpicklerPersistentLoad,
    PickleBufferInit,
    PickleBufferRaw,
    PickleBufferRelease,
    CopyregReconstructor,
    CopyregNewObj,
    CopyregNewObjEx,
    JsonScannerMakeScanner,
    JsonScannerPyMakeScanner,
    JsonScannerScanOnce,
    JsonDecoderScanString,
    MarshalLoads,
    MarshalDumps,
    PyLongIntToDecimalString,
    PyLongIntDivMod,
    PyLongIntFromString,
    PyLongComputePowers,
    PyLongDecStrToIntInner,
    CodecsEncode,
    CodecsDecode,
    CodecsEscapeDecode,
    CodecsLookup,
    CodecsRegister,
    CodecsGetIncrementalEncoder,
    CodecsGetIncrementalDecoder,
    CodecsIncrementalEncoderInit,
    CodecsIncrementalEncoderEncode,
    CodecsIncrementalEncoderReset,
    CodecsIncrementalEncoderGetState,
    CodecsIncrementalEncoderSetState,
    CodecsIncrementalDecoderInit,
    CodecsIncrementalDecoderDecode,
    CodecsIncrementalDecoderReset,
    CodecsIncrementalDecoderGetState,
    CodecsIncrementalDecoderSetState,
    UnicodedataNormalize,
    ReSearch,
    ReMatch,
    ReFullMatch,
    ReCompile,
    ReEscape,
    SreCompile,
    SreTemplate,
    SreAsciiIsCased,
    SreAsciiToLower,
    SreUnicodeIsCased,
    SreUnicodeToLower,
    RePatternFindAll,
    RePatternFindIter,
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
    OperatorItemGetter,
    OperatorAttrGetter,
    OperatorMethodCaller,
    ItertoolsChain,
    ItertoolsAccumulate,
    ItertoolsCombinations,
    ItertoolsCombinationsWithReplacement,
    ItertoolsCompress,
    ItertoolsCount,
    ItertoolsCycle,
    ItertoolsDropWhile,
    ItertoolsFilterFalse,
    ItertoolsGroupBy,
    ItertoolsISlice,
    ItertoolsPairwise,
    ItertoolsRepeat,
    ItertoolsStarMap,
    ItertoolsTakeWhile,
    ItertoolsTee,
    ItertoolsZipLongest,
    ItertoolsBatched,
    ItertoolsPermutations,
    ItertoolsProduct,
    FunctoolsReduce,
    FunctoolsSingleDispatch,
    FunctoolsSingleDispatchMethod,
    FunctoolsSingleDispatchRegister,
    FunctoolsWraps,
    FunctoolsPartial,
    FunctoolsCmpToKey,
    FunctoolsCachedProperty,
    FunctoolsLruCache,
    CollectionsCounter,
    CollectionsDeque,
    CollectionsOrderedDict,
    CollectionsChainMapInit,
    CollectionsChainMapNewChild,
    CollectionsChainMapRepr,
    CollectionsChainMapItems,
    CollectionsChainMapGet,
    CollectionsChainMapGetItem,
    CollectionsChainMapSetItem,
    CollectionsChainMapDelItem,
    CollectionsOrderedDictTypeRepr,
    CollectionsDefaultDictTypeRepr,
    CollectionsCounterTypeRepr,
    CollectionsDequeTypeRepr,
    CollectionsUserDictTypeRepr,
    CollectionsUserListTypeRepr,
    CollectionsUserStringTypeRepr,
    CollectionsNamedTuple,
    CollectionsNamedTupleMake,
    CollectionsDefaultDict,
    TokenizeTokenizerIter,
    StructCalcSize,
    StructPack,
    StructUnpack,
    StructIterUnpack,
    StructPackInto,
    StructUnpackFrom,
    StructClearCache,
    StructClassInit,
    StructClassPack,
    StructClassUnpack,
    StructClassIterUnpack,
    StructClassPackInto,
    StructClassUnpackFrom,
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
    InspectIsMethod,
    InspectIsRoutine,
    InspectIsMethodDescriptor,
    InspectIsMethodWrapper,
    InspectIsTraceback,
    InspectIsFrame,
    InspectIsCode,
    InspectUnwrap,
    InspectSignature,
    InspectGetModule,
    InspectGetFile,
    InspectGetSourceFile,
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
    TestInternalCapiGetRecursionDepth,
    DataclassesField,
    DataclassesIsDataclass,
    DataclassesFields,
    DataclassesAsDict,
    DataclassesAsTuple,
    DataclassesReplace,
    DataclassesMakeDataclass,
    IoOpen,
    IoReadText,
    IoWriteText,
    IoTextEncoding,
    IoTextIOWrapperInit,
    IoFileInit,
    IoFileRead,
    IoFileReadLine,
    IoFileReadInto,
    IoFileReadLines,
    IoFileWrite,
    IoFileWriteLines,
    IoFileTruncate,
    IoFileSeek,
    IoFileTell,
    IoFileClose,
    IoFileFlush,
    IoFileIter,
    IoFileNext,
    IoFileEnter,
    IoFileExit,
    IoFileFileno,
    IoFileDetach,
    IoFileReadable,
    IoFileWritable,
    IoFileSeekable,
    IoBufferedInit,
    IoBufferedRead,
    IoBufferedRead1,
    IoBufferedReadLine,
    IoBufferedWrite,
    IoBufferedFlush,
    IoBufferedClose,
    IoBufferedDetach,
    IoBufferedFileno,
    IoBufferedSeek,
    IoBufferedTell,
    IoBufferedTruncate,
    IoBufferedReadInto,
    IoBufferedReadInto1,
    IoBufferedPeek,
    IoBufferedReadable,
    IoBufferedWritable,
    IoBufferedSeekable,
    IoBufferedRWPairInit,
    IoBufferedRWPairRead,
    IoBufferedRWPairReadLine,
    IoBufferedRWPairRead1,
    IoBufferedRWPairReadInto,
    IoBufferedRWPairReadInto1,
    IoBufferedRWPairWrite,
    IoBufferedRWPairFlush,
    IoBufferedRWPairClose,
    IoBufferedRWPairReadable,
    IoBufferedRWPairWritable,
    IoBufferedRWPairSeekable,
    IoBufferedRWPairDetach,
    IoBufferedRWPairPeek,
    IoRawRead,
    IoRawReadAll,
    IoBaseReadLine,
    IoBaseReadLines,
    IoBaseWriteLines,
    IoBaseEnter,
    IoBaseExit,
    IoBaseIter,
    IoBaseNext,
    IoBaseClose,
    IoBaseFlush,
    IoBaseDel,
    StringIOInit,
    StringIOWrite,
    StringIORead,
    StringIOReadLine,
    StringIOReadLines,
    StringIOGetValue,
    StringIOGetState,
    StringIOSetState,
    StringIOSeek,
    StringIOTell,
    StringIOWriteLines,
    StringIOTruncate,
    StringIODetach,
    StringIOIter,
    StringIONext,
    StringIOEnter,
    StringIOExit,
    StringIOClose,
    StringIOFlush,
    StringIOIsAtty,
    StringIOFileno,
    StringIOReadable,
    StringIOWritable,
    StringIOSeekable,
    BytesIOInit,
    BytesIOWrite,
    BytesIOWriteLines,
    BytesIOTruncate,
    BytesIORead,
    BytesIORead1,
    BytesIOReadLine,
    BytesIOReadLines,
    BytesIOReadInto,
    BytesIOGetValue,
    BytesIOGetBuffer,
    BytesIOGetState,
    BytesIOSetState,
    BytesIODetach,
    BytesIOSeek,
    BytesIOTell,
    BytesIOIter,
    BytesIONext,
    BytesIOEnter,
    BytesIOExit,
    BytesIOClose,
    BytesIOFlush,
    BytesIOIsAtty,
    BytesIOFileno,
    BytesIOReadable,
    BytesIOWritable,
    BytesIOSeekable,
    DateTimeNow,
    DateToday,
    DateInit,
    AsyncioRun,
    AsyncioSleep,
    AsyncioCreateTask,
    AsyncioGather,
    ThreadingExcepthook,
    ThreadingGetIdent,
    ThreadingCurrentThread,
    ThreadingMainThread,
    ThreadingActiveCount,
    ThreadClassInit,
    ThreadClassStart,
    ThreadClassJoin,
    ThreadClassIsAlive,
    ThreadEventInit,
    ThreadEventClear,
    ThreadEventIsSet,
    ThreadEventSet,
    ThreadEventWait,
    ThreadConditionInit,
    ThreadConditionAcquire,
    ThreadConditionNotify,
    ThreadConditionNotifyAll,
    ThreadConditionRelease,
    ThreadConditionWait,
    ThreadSemaphoreInit,
    ThreadSemaphoreAcquire,
    ThreadSemaphoreRelease,
    ThreadBoundedSemaphoreInit,
    ThreadBarrierInit,
    ThreadBarrierAbort,
    ThreadBarrierReset,
    ThreadBarrierWait,
    SignalSignal,
    SignalGetSignal,
    SignalRaiseSignal,
    SocketGetHostName,
    SocketGetHostByName,
    SocketGetAddrInfo,
    SocketFromFd,
    SocketGetDefaultTimeout,
    SocketSetDefaultTimeout,
    SocketNtoHs,
    SocketNtoHl,
    SocketHtoNs,
    SocketHtoNl,
    SocketObjectInit,
    SocketObjectClose,
    SocketObjectDetach,
    SocketObjectFileno,
    UuidClassInit,
    UuidGetNode,
    Uuid1,
    Uuid3,
    Uuid4,
    Uuid5,
    Uuid6,
    Uuid7,
    Uuid8,
    BinasciiCrc32,
    CsvReader,
    CsvWriter,
    CsvWriterRow,
    CsvWriterRows,
    CsvRegisterDialect,
    CsvUnregisterDialect,
    CsvGetDialect,
    CsvListDialects,
    CsvFieldSizeLimit,
    CsvDialectValidate,
    CsvReaderIter,
    CsvReaderNext,
    CollectionsCountElements,
    AtexitRegister,
    AtexitUnregister,
    AtexitRunExitFuncs,
    AtexitClear,
    ColorizeCanColorize,
    ColorizeGetTheme,
    ColorizeGetColors,
    ColorizeSetTheme,
    ColorizeDecolor,
    WarningsWarn,
    WarningsWarnExplicit,
    WarningsFiltersMutated,
    WarningsAcquireLock,
    WarningsReleaseLock,
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
                Ok(Value::Str(format_repr(&args[0])))
            }
            BuiltinFunction::Ascii => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("ascii() expects one argument"));
                }
                Ok(Value::Str(format_ascii(&args[0])))
            }
            BuiltinFunction::DictTypeRepr
            | BuiltinFunction::ListTypeRepr
            | BuiltinFunction::TupleTypeRepr
            | BuiltinFunction::SetTypeRepr
            | BuiltinFunction::FrozenSetTypeRepr
            | BuiltinFunction::StrTypeRepr
            | BuiltinFunction::BytesTypeRepr
            | BuiltinFunction::ByteArrayTypeRepr
            | BuiltinFunction::MappingProxyTypeRepr
            | BuiltinFunction::SimpleNamespaceTypeRepr
            | BuiltinFunction::CollectionsOrderedDictTypeRepr
            | BuiltinFunction::CollectionsDefaultDictTypeRepr
            | BuiltinFunction::CollectionsCounterTypeRepr
            | BuiltinFunction::CollectionsDequeTypeRepr
            | BuiltinFunction::CollectionsUserDictTypeRepr
            | BuiltinFunction::CollectionsUserListTypeRepr
            | BuiltinFunction::CollectionsUserStringTypeRepr => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("__repr__() expects one argument"));
                }
                Ok(Value::Str(format_repr(&args[0])))
            }
            BuiltinFunction::NoOp => Ok(Value::None),
            BuiltinFunction::Format => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::new("format() expects 1-2 arguments"));
                }
                let value = args[0].clone();
                if args.len() == 2 {
                    match &args[1] {
                        Value::Str(spec) if spec.is_empty() => {}
                        Value::Str(_) => {
                            return Err(RuntimeError::new(
                                "format() with non-empty spec is not available in runtime-only call path",
                            ));
                        }
                        _ => return Err(RuntimeError::new("format() argument 2 must be str")),
                    }
                }
                Ok(Value::Str(format_value(&value)))
            }
            BuiltinFunction::StringIOInit
            | BuiltinFunction::StringIOWrite
            | BuiltinFunction::StringIORead
            | BuiltinFunction::StringIOReadLine
            | BuiltinFunction::StringIOReadLines
            | BuiltinFunction::StringIOGetValue
            | BuiltinFunction::StringIOGetState
            | BuiltinFunction::StringIOSetState
            | BuiltinFunction::StringIOSeek
            | BuiltinFunction::StringIOTell
            | BuiltinFunction::StringIOWriteLines
            | BuiltinFunction::StringIOTruncate
            | BuiltinFunction::StringIODetach
            | BuiltinFunction::StringIOIter
            | BuiltinFunction::StringIONext
            | BuiltinFunction::StringIOEnter
            | BuiltinFunction::StringIOExit
            | BuiltinFunction::StringIOClose
            | BuiltinFunction::StringIOFlush
            | BuiltinFunction::StringIOIsAtty
            | BuiltinFunction::StringIOFileno
            | BuiltinFunction::StringIOReadable
            | BuiltinFunction::StringIOWritable
            | BuiltinFunction::StringIOSeekable
            | BuiltinFunction::BytesIOInit
            | BuiltinFunction::BytesIOWrite
            | BuiltinFunction::BytesIORead
            | BuiltinFunction::BytesIORead1
            | BuiltinFunction::BytesIOReadLine
            | BuiltinFunction::BytesIOReadLines
            | BuiltinFunction::BytesIOReadInto
            | BuiltinFunction::BytesIOGetValue
            | BuiltinFunction::BytesIOGetBuffer
            | BuiltinFunction::BytesIOGetState
            | BuiltinFunction::BytesIOSetState
            | BuiltinFunction::BytesIODetach
            | BuiltinFunction::BytesIOSeek
            | BuiltinFunction::BytesIOTell
            | BuiltinFunction::BytesIOIter
            | BuiltinFunction::BytesIONext
            | BuiltinFunction::BytesIOEnter
            | BuiltinFunction::BytesIOExit
            | BuiltinFunction::BytesIOClose
            | BuiltinFunction::BytesIOFlush
            | BuiltinFunction::BytesIOIsAtty
            | BuiltinFunction::BytesIOFileno
            | BuiltinFunction::BytesIOReadable
            | BuiltinFunction::BytesIOWritable
            | BuiltinFunction::BytesIOSeekable
            | BuiltinFunction::IoFileInit
            | BuiltinFunction::IoFileReadInto
            | BuiltinFunction::IoFileWriteLines
            | BuiltinFunction::IoFileTruncate
            | BuiltinFunction::IoBaseReadLine
            | BuiltinFunction::IoBaseReadLines
            | BuiltinFunction::IoBaseWriteLines
            | BuiltinFunction::IoBaseEnter
            | BuiltinFunction::IoBaseExit
            | BuiltinFunction::IoBaseIter
            | BuiltinFunction::IoBaseNext
            | BuiltinFunction::IoBaseClose
            | BuiltinFunction::IoBaseFlush
            | BuiltinFunction::IoBaseDel
            | BuiltinFunction::BytesIOWriteLines
            | BuiltinFunction::BytesIOTruncate
            | BuiltinFunction::IoFileDetach
            | BuiltinFunction::IoBufferedInit
            | BuiltinFunction::IoBufferedRead
            | BuiltinFunction::IoBufferedRead1
            | BuiltinFunction::IoBufferedReadLine
            | BuiltinFunction::IoBufferedWrite
            | BuiltinFunction::IoBufferedFlush
            | BuiltinFunction::IoBufferedClose
            | BuiltinFunction::IoBufferedDetach
            | BuiltinFunction::IoBufferedFileno
            | BuiltinFunction::IoBufferedSeek
            | BuiltinFunction::IoBufferedTell
            | BuiltinFunction::IoBufferedTruncate
            | BuiltinFunction::IoBufferedReadInto
            | BuiltinFunction::IoBufferedReadInto1
            | BuiltinFunction::IoBufferedPeek
            | BuiltinFunction::IoBufferedReadable
            | BuiltinFunction::IoBufferedWritable
            | BuiltinFunction::IoBufferedSeekable
            | BuiltinFunction::IoBufferedRWPairInit
            | BuiltinFunction::IoBufferedRWPairRead
            | BuiltinFunction::IoBufferedRWPairReadLine
            | BuiltinFunction::IoBufferedRWPairRead1
            | BuiltinFunction::IoBufferedRWPairReadInto
            | BuiltinFunction::IoBufferedRWPairReadInto1
            | BuiltinFunction::IoBufferedRWPairWrite
            | BuiltinFunction::IoBufferedRWPairFlush
            | BuiltinFunction::IoBufferedRWPairClose
            | BuiltinFunction::IoBufferedRWPairReadable
            | BuiltinFunction::IoBufferedRWPairWritable
            | BuiltinFunction::IoBufferedRWPairSeekable
            | BuiltinFunction::IoBufferedRWPairDetach
            | BuiltinFunction::IoBufferedRWPairPeek
            | BuiltinFunction::IoRawRead
            | BuiltinFunction::IoRawReadAll
            | BuiltinFunction::RePatternFindAll
            | BuiltinFunction::RePatternFindIter
            | BuiltinFunction::SreCompile
            | BuiltinFunction::SreTemplate
            | BuiltinFunction::SreAsciiIsCased
            | BuiltinFunction::SreAsciiToLower
            | BuiltinFunction::SreUnicodeIsCased
            | BuiltinFunction::SreUnicodeToLower
            | BuiltinFunction::CollectionsChainMapInit
            | BuiltinFunction::CollectionsChainMapNewChild
            | BuiltinFunction::CollectionsChainMapRepr
            | BuiltinFunction::CollectionsChainMapItems
            | BuiltinFunction::CollectionsChainMapGet
            | BuiltinFunction::CollectionsChainMapGetItem
            | BuiltinFunction::CollectionsChainMapSetItem
            | BuiltinFunction::CollectionsChainMapDelItem
            | BuiltinFunction::CsvReaderIter
            | BuiltinFunction::CsvReaderNext
            | BuiltinFunction::PickleDump
            | BuiltinFunction::PickleDumps
            | BuiltinFunction::PickleLoad
            | BuiltinFunction::PickleLoads
            | BuiltinFunction::PickleModuleGetAttr
            | BuiltinFunction::PicklePicklerInit
            | BuiltinFunction::PicklePicklerDump
            | BuiltinFunction::PicklePicklerClearMemo
            | BuiltinFunction::PicklePicklerPersistentId
            | BuiltinFunction::PickleUnpicklerInit
            | BuiltinFunction::PickleUnpicklerLoad
            | BuiltinFunction::PickleUnpicklerPersistentLoad
            | BuiltinFunction::PickleBufferInit
            | BuiltinFunction::PickleBufferRaw
            | BuiltinFunction::PickleBufferRelease
            | BuiltinFunction::CopyregReconstructor
            | BuiltinFunction::CopyregNewObj
            | BuiltinFunction::CopyregNewObjEx => Err(RuntimeError::new(
                "StringIO/BytesIO builtin not available in runtime-only call path",
            )),
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
                    Value::DictKeys(obj) => match &*obj.kind() {
                        Object::DictKeysView(view) => match &*view.dict.kind() {
                            Object::Dict(values) => Ok(Value::Int(values.len() as i64)),
                            _ => Err(RuntimeError::new("len() unsupported type")),
                        },
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
                        Object::MemoryView(view) => {
                            let itemsize = view.itemsize.max(1);
                            with_bytes_like_source(&view.source, |values| {
                                let (start, end) =
                                    memoryview_bounds(view.start, view.length, values.len());
                                Ok(Value::Int((end.saturating_sub(start) / itemsize) as i64))
                            })
                            .unwrap_or_else(|| Err(RuntimeError::new("len() unsupported type")))
                        }
                        _ => Err(RuntimeError::new("len() unsupported type")),
                    },
                    Value::Module(obj) => match &*obj.kind() {
                        Object::Module(module_data) if module_data.name == "__array__" => {
                            match module_data.globals.get("values") {
                                Some(Value::List(values)) => match &*values.kind() {
                                    Object::List(items) => {
                                        let itemsize = match module_data.globals.get("itemsize") {
                                            Some(Value::Int(value)) if *value > 0 => {
                                                *value as usize
                                            }
                                            _ => 1usize,
                                        };
                                        let from_bytes_initializer = matches!(
                                            module_data.globals.get("__pyrs_array_frombytes__"),
                                            Some(Value::Bool(true))
                                        );
                                        let logical_len = if from_bytes_initializer && itemsize > 1
                                        {
                                            items.len() / itemsize
                                        } else {
                                            items.len()
                                        };
                                        Ok(Value::Int(logical_len as i64))
                                    }
                                    _ => Err(RuntimeError::new("len() unsupported type")),
                                },
                                _ => Err(RuntimeError::new("len() unsupported type")),
                            }
                        }
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

                Ok(Value::Slice(Box::new(SliceValue::new(lower, upper, step))))
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
                let parse_with_base = |text: &str,
                                       explicit_base: Option<i64>|
                 -> Result<Value, RuntimeError> {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        return Err(RuntimeError::new("invalid literal for int()"));
                    }
                    let (is_negative, body) = if let Some(rest) = trimmed.strip_prefix('-') {
                        (true, rest)
                    } else if let Some(rest) = trimmed.strip_prefix('+') {
                        (false, rest)
                    } else {
                        (false, trimmed)
                    };
                    if body.is_empty() {
                        return Err(RuntimeError::new("invalid literal for int()"));
                    }

                    let mut base = explicit_base.unwrap_or(10);
                    if explicit_base.is_some() && !(base == 0 || (2..=36).contains(&base)) {
                        return Err(RuntimeError::new("int() base must be >= 2 and <= 36, or 0"));
                    }

                    let mut digits = body;
                    let mut saw_prefix = false;
                    if base == 0 {
                        if let Some(rest) = digits
                            .strip_prefix("0x")
                            .or_else(|| digits.strip_prefix("0X"))
                        {
                            base = 16;
                            digits = rest;
                            saw_prefix = true;
                        } else if let Some(rest) = digits
                            .strip_prefix("0o")
                            .or_else(|| digits.strip_prefix("0O"))
                        {
                            base = 8;
                            digits = rest;
                            saw_prefix = true;
                        } else if let Some(rest) = digits
                            .strip_prefix("0b")
                            .or_else(|| digits.strip_prefix("0B"))
                        {
                            base = 2;
                            digits = rest;
                            saw_prefix = true;
                        } else {
                            base = 10;
                        }
                    } else if base == 16 {
                        if let Some(rest) = digits
                            .strip_prefix("0x")
                            .or_else(|| digits.strip_prefix("0X"))
                        {
                            digits = rest;
                            saw_prefix = true;
                        }
                    } else if base == 8 {
                        if let Some(rest) = digits
                            .strip_prefix("0o")
                            .or_else(|| digits.strip_prefix("0O"))
                        {
                            digits = rest;
                            saw_prefix = true;
                        }
                    } else if base == 2 {
                        if let Some(rest) = digits
                            .strip_prefix("0b")
                            .or_else(|| digits.strip_prefix("0B"))
                        {
                            digits = rest;
                            saw_prefix = true;
                        }
                    }

                    let normalized = normalize_int_digits_for_base(digits, base as u32, saw_prefix)
                        .ok_or_else(|| RuntimeError::new("invalid literal for int()"))?;
                    if explicit_base == Some(0)
                        && !saw_prefix
                        && normalized.len() > 1
                        && normalized.starts_with('0')
                        && normalized.chars().any(|ch| ch != '0')
                    {
                        return Err(RuntimeError::new("invalid literal for int()"));
                    }

                    let mut parsed = BigInt::from_str_radix(&normalized, base as u32)
                        .ok_or_else(|| RuntimeError::new("invalid literal for int()"))?;
                    if is_negative {
                        parsed = parsed.negated();
                    }
                    Ok(match parsed.to_i64() {
                        Some(value) => Value::Int(value),
                        None => Value::BigInt(Box::new(parsed)),
                    })
                };

                let explicit_base = if args.len() == 2 {
                    Some(value_to_int(args[1].clone())?)
                } else {
                    None
                };
                if explicit_base.is_some()
                    && !matches!(
                        args[0],
                        Value::Str(_) | Value::Bytes(_) | Value::ByteArray(_)
                    )
                {
                    return Err(RuntimeError::new(
                        "int() can't convert non-string with explicit base",
                    ));
                }
                match &args[0] {
                    Value::Int(value) => Ok(Value::Int(*value)),
                    Value::BigInt(value) => Ok(Value::BigInt(value.clone())),
                    Value::Bool(value) => Ok(Value::Int(if *value { 1 } else { 0 })),
                    Value::Float(value) => {
                        if value.is_nan() {
                            return Err(RuntimeError::new("cannot convert float NaN to integer"));
                        }
                        if value.is_infinite() {
                            return Err(RuntimeError::new(
                                "cannot convert float infinity to integer",
                            ));
                        }
                        let truncated = value.trunc();
                        let bigint = BigInt::from_f64_integral(truncated)
                            .ok_or_else(|| RuntimeError::new("invalid literal for int()"))?;
                        Ok(match bigint.to_i64() {
                            Some(value) => Value::Int(value),
                            None => Value::BigInt(Box::new(bigint)),
                        })
                    }
                    Value::Str(value) => parse_with_base(value, explicit_base),
                    Value::Bytes(obj) | Value::ByteArray(obj) => match &*obj.kind() {
                        Object::Bytes(bytes) | Object::ByteArray(bytes) => {
                            let text = std::str::from_utf8(bytes)
                                .map_err(|_| RuntimeError::new("invalid literal for int()"))?;
                            parse_with_base(text, explicit_base)
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
                let bits = match &args[0] {
                    Value::Int(value) => (i64::BITS - value.unsigned_abs().leading_zeros()) as i64,
                    Value::Bool(value) => {
                        if *value {
                            1
                        } else {
                            0
                        }
                    }
                    Value::BigInt(value) => value.bit_length() as i64,
                    _ => return Err(RuntimeError::new("expected integer")),
                };
                Ok(Value::Int(bits))
            }
            BuiltinFunction::IntFromBytes => {
                Err(RuntimeError::new("int.from_bytes() requires VM context"))
            }
            BuiltinFunction::Float => {
                if args.is_empty() {
                    return Ok(Value::Float(0.0));
                }
                if args.len() != 1 {
                    return Err(RuntimeError::new("float() expects at most one argument"));
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
                    Value::Bytes(obj) => match &*obj.kind() {
                        Object::Bytes(values) => {
                            let text = std::str::from_utf8(values)
                                .map_err(|_| RuntimeError::new("float() invalid literal"))?;
                            let parsed = text
                                .trim()
                                .parse::<f64>()
                                .map_err(|_| RuntimeError::new("float() invalid literal"))?;
                            Ok(Value::Float(parsed))
                        }
                        _ => Err(RuntimeError::new("float() unsupported type")),
                    },
                    Value::ByteArray(obj) => match &*obj.kind() {
                        Object::ByteArray(values) => {
                            let text = std::str::from_utf8(values)
                                .map_err(|_| RuntimeError::new("float() invalid literal"))?;
                            let parsed = text
                                .trim()
                                .parse::<f64>()
                                .map_err(|_| RuntimeError::new("float() invalid literal"))?;
                            Ok(Value::Float(parsed))
                        }
                        _ => Err(RuntimeError::new("float() unsupported type")),
                    },
                    _ => Err(RuntimeError::new("float() unsupported type")),
                }
            }
            BuiltinFunction::Str => {
                if args.is_empty() {
                    return Ok(Value::Str(String::new()));
                }
                if args.len() != 1 {
                    return Err(RuntimeError::new("str() expects at most one argument"));
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
                        Object::Bytes(values) if values.len() == 1 => {
                            Ok(Value::Int(values[0] as i64))
                        }
                        Object::Bytes(_) => Err(RuntimeError::new("ord() expected a character")),
                        _ => Err(RuntimeError::new("ord() unsupported type")),
                    },
                    Value::ByteArray(obj) => match &*obj.kind() {
                        Object::ByteArray(values) if values.len() == 1 => {
                            Ok(Value::Int(values[0] as i64))
                        }
                        Object::ByteArray(_) => {
                            Err(RuntimeError::new("ord() expected a character"))
                        }
                        _ => Err(RuntimeError::new("ord() unsupported type")),
                    },
                    _ => Err(RuntimeError::new("ord() expected string of length 1")),
                }
            }
            BuiltinFunction::Chr => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("chr() expects one argument"));
                }
                let codepoint = match &args[0] {
                    Value::Int(value) => *value,
                    Value::BigInt(value) => value
                        .to_i64()
                        .ok_or_else(|| RuntimeError::new("chr() arg not in range(0x110000)"))?,
                    Value::Bool(value) => i64::from(*value),
                    _ => return Err(RuntimeError::new("chr() argument must be int")),
                };
                if !(0..=0x10FFFF).contains(&codepoint) {
                    return Err(RuntimeError::new("chr() arg not in range(0x110000)"));
                }
                let ch = char::from_u32(codepoint as u32)
                    .ok_or_else(|| RuntimeError::new("chr() arg not in range(0x110000)"))?;
                Ok(Value::Str(ch.to_string()))
            }
            BuiltinFunction::Bin => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("bin() expects one argument"));
                }
                let rendered = int_to_prefixed_base_string(&args[0], 2, "0b")
                    .ok_or_else(|| RuntimeError::new("bin() argument must be an integer"))?;
                Ok(Value::Str(rendered))
            }
            BuiltinFunction::Oct => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("oct() expects one argument"));
                }
                let rendered = int_to_prefixed_base_string(&args[0], 8, "0o")
                    .ok_or_else(|| RuntimeError::new("oct() argument must be an integer"))?;
                Ok(Value::Str(rendered))
            }
            BuiltinFunction::Hex => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("hex() expects one argument"));
                }
                let rendered = int_to_prefixed_base_string(&args[0], 16, "0x")
                    .ok_or_else(|| RuntimeError::new("hex() argument must be an integer"))?;
                Ok(Value::Str(rendered))
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
            BuiltinFunction::Filter => Err(RuntimeError::new("filter() requires VM context")),
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
            BuiltinFunction::Round => Err(RuntimeError::new("round() requires VM context")),
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
                        Object::Set(values) => Ok(heap.alloc_list(values.to_vec())),
                        _ => Err(RuntimeError::new("list() unsupported type")),
                    },
                    Value::FrozenSet(obj) => match &*obj.kind() {
                        Object::FrozenSet(values) => Ok(heap.alloc_list(values.to_vec())),
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
                        Object::MemoryView(view) => {
                            with_bytes_like_source(&view.source, |values| {
                                let (start, end) =
                                    memoryview_bounds(view.start, view.length, values.len());
                                Ok(heap.alloc_list(
                                    values[start..end]
                                        .iter()
                                        .map(|byte| Value::Int(*byte as i64))
                                        .collect(),
                                ))
                            })
                            .unwrap_or_else(|| Err(RuntimeError::new("list() unsupported type")))
                        }
                        _ => Err(RuntimeError::new("list() unsupported type")),
                    },
                    _ => Err(RuntimeError::new("list() unsupported type")),
                }
            }
            BuiltinFunction::ListAppendDescriptor => {
                if args.len() != 2 {
                    return Err(RuntimeError::new("descriptor 'append' expects 2 arguments"));
                }
                let target = args[0].clone();
                let value = args[1].clone();
                match target {
                    Value::List(obj) => {
                        let mut list = obj.kind_mut();
                        let Object::List(values) = &mut *list else {
                            return Err(RuntimeError::new("append() receiver must be list"));
                        };
                        values.push(value);
                        Ok(Value::None)
                    }
                    _ => Err(RuntimeError::new("append() receiver must be list")),
                }
            }
            BuiltinFunction::Tuple => {
                let source = match args.len() {
                    0 => None,
                    1 => Some(args[0].clone()),
                    2 => match &args[0] {
                        Value::Class(_) | Value::Builtin(BuiltinFunction::Tuple) => {
                            Some(args[1].clone())
                        }
                        _ => return Err(RuntimeError::new("tuple() expects at most one argument")),
                    },
                    _ => return Err(RuntimeError::new("tuple() expects at most one argument")),
                };
                if source.is_none() {
                    return Ok(heap.alloc_tuple(Vec::new()));
                }
                match source.expect("checked is_some") {
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
                        Object::Set(values) => Ok(heap.alloc_tuple(values.to_vec())),
                        _ => Err(RuntimeError::new("tuple() unsupported type")),
                    },
                    Value::FrozenSet(obj) => match &*obj.kind() {
                        Object::FrozenSet(values) => Ok(heap.alloc_tuple(values.to_vec())),
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
                        Object::MemoryView(view) => {
                            with_bytes_like_source(&view.source, |values| {
                                let (start, end) =
                                    memoryview_bounds(view.start, view.length, values.len());
                                Ok(heap.alloc_tuple(
                                    values[start..end]
                                        .iter()
                                        .map(|byte| Value::Int(*byte as i64))
                                        .collect(),
                                ))
                            })
                            .unwrap_or_else(|| Err(RuntimeError::new("tuple() unsupported type")))
                        }
                        _ => Err(RuntimeError::new("tuple() unsupported type")),
                    },
                    _ => Err(RuntimeError::new("tuple() unsupported type")),
                }
            }
            BuiltinFunction::Dict | BuiltinFunction::CollectionsOrderedDict => {
                if args.len() > 1 {
                    return Err(RuntimeError::new("dict() expects at most one argument"));
                }
                if args.is_empty() {
                    return Ok(heap.alloc_dict(Vec::new()));
                }
                match &args[0] {
                    Value::Dict(obj) => match &*obj.kind() {
                        Object::Dict(entries) => Ok(heap.alloc_dict(entries.to_vec())),
                        _ => Err(RuntimeError::new("dict() unsupported type")),
                    },
                    other => {
                        let mut entries = Vec::new();
                        for item in iterable_values(other.clone())? {
                            match item {
                                Value::Tuple(pair) => match &*pair.kind() {
                                    Object::Tuple(parts) if parts.len() == 2 => {
                                        ensure_hashable_key(&parts[0])?;
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
                                        ensure_hashable_key(&parts[0])?;
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
                    ensure_hashable_key(&key)?;
                    if let Some((_, value)) =
                        entries.iter_mut().find(|(existing, _)| *existing == key)
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
                Ok(heap.alloc_set(dedup_values(values)?))
            }
            BuiltinFunction::FrozenSet => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "frozenset() expects at most one argument",
                    ));
                }
                let values = if let Some(source) = args.into_iter().next() {
                    iterable_values(source)?
                } else {
                    Vec::new()
                };
                Ok(heap.alloc_frozenset(dedup_values(values)?))
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
                    Value::MemoryView(obj) => match &*obj.kind() {
                        Object::MemoryView(view) => view.source.clone(),
                        _ => {
                            return Err(RuntimeError::new(
                                "memoryview() expects bytes-like object",
                            ));
                        }
                    },
                    Value::Instance(obj) => match &*obj.kind() {
                        Object::Instance(instance_data) => {
                            let is_picklebuffer = matches!(
                                &*instance_data.class.kind(),
                                Object::Class(class_data)
                                    if class_data.name == "PickleBuffer"
                            );
                            if is_picklebuffer
                                && matches!(
                                    instance_data.attrs.get("__pyrs_picklebuffer_released__"),
                                    Some(Value::Bool(true))
                                )
                            {
                                return Err(RuntimeError::new(
                                    "ValueError: operation forbidden on released PickleBuffer object",
                                ));
                            }
                            if is_picklebuffer {
                                if let Some(source) = instance_data
                                    .attrs
                                    .get("__pyrs_picklebuffer_source__")
                                    .or_else(|| instance_data.attrs.get("__pyrs_bytes_storage__"))
                                {
                                    match source {
                                        Value::Bytes(source)
                                        | Value::ByteArray(source)
                                        | Value::Instance(source) => source.clone(),
                                        Value::MemoryView(view) => match &*view.kind() {
                                            Object::MemoryView(view_data) => {
                                                view_data.source.clone()
                                            }
                                            _ => {
                                                return Err(RuntimeError::new(
                                                    "memoryview() expects bytes-like object",
                                                ));
                                            }
                                        },
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "memoryview() expects bytes-like object",
                                            ));
                                        }
                                    }
                                } else {
                                    return Err(RuntimeError::new(
                                        "memoryview() expects bytes-like object",
                                    ));
                                }
                            } else {
                                match instance_data.attrs.get("__pyrs_bytes_storage__") {
                                    Some(Value::Bytes(_)) | Some(Value::ByteArray(_)) => {
                                        obj.clone()
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "memoryview() expects bytes-like object",
                                        ));
                                    }
                                }
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "memoryview() expects bytes-like object",
                            ));
                        }
                    },
                    Value::Module(obj) => match &*obj.kind() {
                        Object::Module(module_data) if module_data.name == "__array__" => {
                            obj.clone()
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "memoryview() expects bytes-like object",
                            ));
                        }
                    },
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
                class.metaclass = class.bases.iter().find_map(|base| match &*base.kind() {
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
                let wrapped = match heap.alloc_module(ModuleObject::new("__classmethod__")) {
                    Value::Module(module) => module,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *wrapped.kind_mut() {
                    module_data
                        .globals
                        .insert("__func__".to_string(), args[0].clone());
                }
                Ok(Value::Module(wrapped))
            }
            BuiltinFunction::StaticMethod => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("staticmethod() expects one argument"));
                }
                let wrapped = match heap.alloc_module(ModuleObject::new("__staticmethod__")) {
                    Value::Module(module) => module,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *wrapped.kind_mut() {
                    module_data
                        .globals
                        .insert("__func__".to_string(), args[0].clone());
                }
                Ok(Value::Module(wrapped))
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
                let module =
                    match heap.alloc_module(ModuleObject::new(format!("<ContextVar {name}>"))) {
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
                    return Err(RuntimeError::new(
                        "ContextVar.get() expects at most one argument",
                    ));
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
                    return Err(RuntimeError::new(
                        "lru_cache() expects at most one argument",
                    ));
                }
                if let Some(callable) = args.into_iter().next() {
                    match callable {
                        Value::Function(_)
                        | Value::Builtin(_)
                        | Value::BoundMethod(_)
                        | Value::Class(_) => Ok(callable),
                        _ => Ok(Value::Builtin(BuiltinFunction::FunctoolsLruCache)),
                    }
                } else {
                    Ok(Value::Builtin(BuiltinFunction::FunctoolsLruCache))
                }
            }
            BuiltinFunction::FunctoolsCachedProperty => {
                Err(RuntimeError::new("cached_property() requires VM context"))
            }
            BuiltinFunction::TokenizeTokenizerIter => {
                if args.is_empty() {
                    return Err(RuntimeError::new("TokenizerIter() expects source"));
                }
                let empty = match heap.alloc_list(Vec::new()) {
                    Value::List(obj) => obj,
                    _ => unreachable!(),
                };
                Ok(Value::Iterator(heap.alloc(Object::Iterator(
                    IteratorObject {
                        kind: IteratorKind::List(empty),
                        index: 0,
                    },
                ))))
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
                Ok(Value::Iterator(heap.alloc(Object::Iterator(
                    IteratorObject {
                        kind: IteratorKind::List(empty),
                        index: 0,
                    },
                ))))
            }
            BuiltinFunction::StructPackInto => {
                if args.len() < 3 {
                    return Err(RuntimeError::new(
                        "pack_into() expects format, buffer, offset",
                    ));
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
            BuiltinFunction::StructClassInit
            | BuiltinFunction::StructClassPack
            | BuiltinFunction::StructClassUnpack
            | BuiltinFunction::StructClassIterUnpack
            | BuiltinFunction::StructClassPackInto
            | BuiltinFunction::StructClassUnpackFrom => Err(RuntimeError::new(
                "struct.Struct methods require VM context",
            )),
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
                    return Err(RuntimeError::new(
                        "_fix_co_filename() expects code and path",
                    ));
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
                    return Err(RuntimeError::new(
                        "_frozen_module_names() expects no arguments",
                    ));
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
                let marker = match heap.alloc_module(ModuleObject::new(format!("<typing {name}>")))
                {
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
                let field_tuple =
                    heap.alloc_tuple(fields.iter().cloned().map(Value::Str).collect());
                class
                    .attrs
                    .insert("_fields".to_string(), field_tuple.clone());
                class
                    .attrs
                    .insert("__pyrs_namedtuple_fields__".to_string(), field_tuple);
                for field in &fields {
                    let descriptor = match heap
                        .alloc_module(ModuleObject::new(format!("__namedtuple_field_{field}")))
                    {
                        Value::Module(module) => {
                            if let Object::Module(module_data) = &mut *module.kind_mut() {
                                module_data
                                    .globals
                                    .insert("__doc__".to_string(), Value::None);
                            }
                            Value::Module(module)
                        }
                        _ => Value::None,
                    };
                    class.attrs.insert(field.clone(), descriptor);
                }
                let class_value = heap.alloc_class(class);
                let make_wrapper = match heap.alloc_module(ModuleObject::new("__classmethod__")) {
                    Value::Module(module) => module,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *make_wrapper.kind_mut() {
                    module_data.globals.insert(
                        "__func__".to_string(),
                        Value::Builtin(BuiltinFunction::CollectionsNamedTupleMake),
                    );
                }
                if let Value::Class(class_ref) = &class_value {
                    if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
                        class_data
                            .attrs
                            .insert("_make".to_string(), Value::Module(make_wrapper));
                    }
                }
                Ok(class_value)
            }
            BuiltinFunction::CollectionsNamedTupleMake => {
                if args.len() != 2 {
                    return Err(RuntimeError::new(
                        "namedtuple._make() expects class and iterable",
                    ));
                }
                let class = match &args[0] {
                    Value::Class(class) => class.clone(),
                    _ => {
                        return Err(RuntimeError::new(
                            "namedtuple._make() requires class receiver",
                        ));
                    }
                };
                let fields = match &*class.kind() {
                    Object::Class(class_data) => {
                        match class_data.attrs.get("__pyrs_namedtuple_fields__") {
                            Some(Value::Tuple(fields_obj)) => match &*fields_obj.kind() {
                                Object::Tuple(values) => values
                                    .iter()
                                    .map(|value| match value {
                                        Value::Str(name) => Some(name.clone()),
                                        _ => None,
                                    })
                                    .collect::<Option<Vec<_>>>(),
                                _ => None,
                            },
                            Some(Value::List(fields_obj)) => match &*fields_obj.kind() {
                                Object::List(values) => values
                                    .iter()
                                    .map(|value| match value {
                                        Value::Str(name) => Some(name.clone()),
                                        _ => None,
                                    })
                                    .collect::<Option<Vec<_>>>(),
                                _ => None,
                            },
                            _ => None,
                        }
                    }
                    _ => None,
                }
                .ok_or_else(|| RuntimeError::new("namedtuple._make() requires namedtuple class"))?;
                let values = match &args[1] {
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => values.clone(),
                        _ => return Err(RuntimeError::new("namedtuple._make() expects iterable")),
                    },
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => values.clone(),
                        _ => return Err(RuntimeError::new("namedtuple._make() expects iterable")),
                    },
                    _ => return Err(RuntimeError::new("namedtuple._make() expects iterable")),
                };
                if values.len() != fields.len() {
                    return Err(RuntimeError::new(format!(
                        "Expected {} arguments, got {}",
                        fields.len(),
                        values.len()
                    )));
                }
                let instance = match heap.alloc_instance(InstanceObject::new(class.clone())) {
                    Value::Instance(instance) => instance,
                    _ => unreachable!(),
                };
                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                    for (field, value) in fields.into_iter().zip(values.into_iter()) {
                        instance_data.attrs.insert(field, value);
                    }
                }
                Ok(Value::Instance(instance))
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
                    return Err(RuntimeError::new("abc helper expects exactly one argument"));
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
            BuiltinFunction::WeakRefFinalize => {
                if args.len() < 2 {
                    return Err(RuntimeError::new("finalize() expects object and callback"));
                }
                let object = args[0].clone();
                let callback = args[1].clone();
                let callback_args = heap.alloc_tuple(args.into_iter().skip(2).collect());
                let callback_kwargs = heap.alloc_dict(Vec::new());
                let finalizer = match heap
                    .alloc_module(ModuleObject::new("__weakref_finalize__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *finalizer.kind_mut() {
                    module_data
                        .globals
                        .insert("__pyrs_weakref_finalize__".to_string(), Value::Bool(true));
                    module_data
                        .globals
                        .insert("alive".to_string(), Value::Bool(true));
                    module_data.globals.insert("_obj".to_string(), object);
                    module_data.globals.insert("_func".to_string(), callback);
                    module_data
                        .globals
                        .insert("_args".to_string(), callback_args);
                    module_data
                        .globals
                        .insert("_kwargs".to_string(), callback_kwargs);
                }
                let native = heap.alloc_native_method(NativeMethodObject::new(
                    NativeMethodKind::Builtin(BuiltinFunction::WeakRefFinalizeDetach),
                ));
                let detach = heap.alloc_bound_method(BoundMethod::new(native, finalizer.clone()));
                if let Object::Module(module_data) = &mut *finalizer.kind_mut() {
                    module_data.globals.insert("detach".to_string(), detach);
                }
                Ok(Value::Module(finalizer))
            }
            BuiltinFunction::WeakRefFinalizeDetach => {
                if args.len() != 1 {
                    return Err(RuntimeError::new("finalize.detach() expects no arguments"));
                }
                let finalizer = match &args[0] {
                    Value::Module(obj) => obj.clone(),
                    _ => return Err(RuntimeError::new("invalid finalize receiver")),
                };
                let mut obj = Value::None;
                let mut func = Value::None;
                let mut cb_args = Value::None;
                let mut cb_kwargs = Value::None;
                let mut alive = false;
                if let Object::Module(module_data) = &mut *finalizer.kind_mut() {
                    alive = matches!(module_data.globals.get("alive"), Some(Value::Bool(true)));
                    if alive {
                        module_data
                            .globals
                            .insert("alive".to_string(), Value::Bool(false));
                        obj = module_data
                            .globals
                            .insert("_obj".to_string(), Value::None)
                            .unwrap_or(Value::None);
                        func = module_data
                            .globals
                            .get("_func")
                            .cloned()
                            .unwrap_or(Value::None);
                        cb_args = module_data
                            .globals
                            .get("_args")
                            .cloned()
                            .unwrap_or_else(|| heap.alloc_tuple(Vec::new()));
                        cb_kwargs = module_data
                            .globals
                            .get("_kwargs")
                            .cloned()
                            .unwrap_or_else(|| heap.alloc_dict(Vec::new()));
                    }
                }
                if !alive || matches!(obj, Value::None) {
                    return Ok(Value::None);
                }
                Ok(heap.alloc_tuple(vec![obj, func, cb_args, cb_kwargs]))
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
                    return Err(RuntimeError::new(
                        "_remove_dead_weakref() expects two arguments",
                    ));
                }
                if let Value::Dict(obj) = &args[0] {
                    if let Object::Dict(entries) = &mut *obj.kind_mut() {
                        if let Some(index) = entries.iter().position(|(key, _)| *key == args[1]) {
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
                let itemsize = match typecode.chars().next().expect("checked empty") {
                    'b' | 'B' => 1,
                    'u' | 'h' | 'H' => 2,
                    'i' | 'I' | 'l' | 'L' | 'f' | 'w' => 4,
                    'q' | 'Q' | 'd' => 8,
                    _ => 1,
                };
                let is_wide_char = typecode.starts_with('w');
                let mut values = Vec::new();
                let mut from_bytes_initializer = false;
                if let Some(initializer) = args.get(1) {
                    match initializer {
                        Value::List(obj) => match &*obj.kind() {
                            Object::List(items) => values.extend(items.clone()),
                            _ => {
                                return Err(RuntimeError::new(
                                    "array() initializer must be iterable",
                                ));
                            }
                        },
                        Value::Tuple(obj) => match &*obj.kind() {
                            Object::Tuple(items) => values.extend(items.clone()),
                            _ => {
                                return Err(RuntimeError::new(
                                    "array() initializer must be iterable",
                                ));
                            }
                        },
                        Value::Bytes(obj) | Value::ByteArray(obj) => match &*obj.kind() {
                            Object::Bytes(bytes) | Object::ByteArray(bytes) => {
                                from_bytes_initializer = true;
                                values.extend(bytes.iter().map(|value| Value::Int(*value as i64)));
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "array() initializer must be iterable",
                                ));
                            }
                        },
                        Value::Str(text) => {
                            if is_wide_char {
                                values.extend(text.chars().map(|ch| Value::Str(ch.to_string())));
                            } else {
                                values.extend(text.chars().map(|ch| Value::Int(ch as i64)));
                            }
                        }
                        Value::None => {}
                        _ => return Err(RuntimeError::new("array() initializer must be iterable")),
                    }
                }
                let values = heap.alloc_list(values);
                let module = match heap.alloc_module(ModuleObject::new("__array__")) {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *module.kind_mut() {
                    module_data
                        .globals
                        .insert("typecode".to_string(), Value::Str(typecode));
                    module_data
                        .globals
                        .insert("itemsize".to_string(), Value::Int(itemsize));
                    module_data.globals.insert(
                        "__pyrs_array_frombytes__".to_string(),
                        Value::Bool(from_bytes_initializer),
                    );
                    module_data.globals.insert("values".to_string(), values);
                }
                Ok(Value::Module(module))
            }
            BuiltinFunction::GcCollect => {
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "gc.collect() expects at most one argument",
                    ));
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
            | BuiltinFunction::FloatFromHex
            | BuiltinFunction::FloatHex
            | BuiltinFunction::StrMakeTrans
            | BuiltinFunction::BytesMakeTrans
            | BuiltinFunction::Compile
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
            | BuiltinFunction::Exec
            | BuiltinFunction::SysGetFrame
            | BuiltinFunction::SysException
            | BuiltinFunction::SysExcInfo
            | BuiltinFunction::SysExit
            | BuiltinFunction::SysGetFilesystemEncoding
            | BuiltinFunction::SysGetFilesystemEncodeErrors
            | BuiltinFunction::SysGetRefCount
            | BuiltinFunction::SysGetRecursionLimit
            | BuiltinFunction::SysSetRecursionLimit
            | BuiltinFunction::SysStdoutWrite
            | BuiltinFunction::SysStdoutBufferWrite
            | BuiltinFunction::SysStdoutFlush
            | BuiltinFunction::SysStderrWrite
            | BuiltinFunction::SysStderrBufferWrite
            | BuiltinFunction::SysStderrFlush
            | BuiltinFunction::SysStdinWrite
            | BuiltinFunction::SysStdinFlush
            | BuiltinFunction::SysStreamIsATty
            | BuiltinFunction::PlatformLibcVer
            | BuiltinFunction::PlatformWin32IsIot
            | BuiltinFunction::ImportlibInvalidateCaches
            | BuiltinFunction::ImportlibSourceFromCache
            | BuiltinFunction::ImportlibCacheFromSource
            | BuiltinFunction::ImportlibSpecFromFileLocation
            | BuiltinFunction::FrozenImportlibSpecFromLoader
            | BuiltinFunction::FrozenImportlibVerboseMessage
            | BuiltinFunction::FrozenImportlibExternalPathJoin
            | BuiltinFunction::FrozenImportlibExternalPathSplit
            | BuiltinFunction::FrozenImportlibExternalPathStat
            | BuiltinFunction::FrozenImportlibExternalUnpackUint16
            | BuiltinFunction::FrozenImportlibExternalUnpackUint32
            | BuiltinFunction::FrozenImportlibExternalUnpackUint64
            | BuiltinFunction::OpcodeStackEffect
            | BuiltinFunction::OpcodeHasArg
            | BuiltinFunction::OpcodeHasConst
            | BuiltinFunction::OpcodeHasName
            | BuiltinFunction::OpcodeHasJump
            | BuiltinFunction::OpcodeHasFree
            | BuiltinFunction::OpcodeHasLocal
            | BuiltinFunction::OpcodeHasExc
            | BuiltinFunction::OpcodeGetExecutor
            | BuiltinFunction::RandomSeed
            | BuiltinFunction::RandomRandom
            | BuiltinFunction::RandomRandRange
            | BuiltinFunction::RandomRandInt
            | BuiltinFunction::RandomGetRandBits
            | BuiltinFunction::RandomChoice
            | BuiltinFunction::RandomChoices
            | BuiltinFunction::RandomShuffle
            | BuiltinFunction::DecimalGetContext
            | BuiltinFunction::DecimalSetContext
            | BuiltinFunction::DecimalLocalContext
            | BuiltinFunction::MathSqrt
            | BuiltinFunction::MathCopySign
            | BuiltinFunction::MathFloor
            | BuiltinFunction::MathCeil
            | BuiltinFunction::MathIsFinite
            | BuiltinFunction::MathIsInf
            | BuiltinFunction::MathIsNaN
            | BuiltinFunction::MathLdExp
            | BuiltinFunction::MathHypot
            | BuiltinFunction::MathFAbs
            | BuiltinFunction::MathExp
            | BuiltinFunction::MathErfc
            | BuiltinFunction::MathLog
            | BuiltinFunction::MathFSum
            | BuiltinFunction::MathSumProd
            | BuiltinFunction::MathCos
            | BuiltinFunction::MathSin
            | BuiltinFunction::MathTan
            | BuiltinFunction::MathCosh
            | BuiltinFunction::MathAsin
            | BuiltinFunction::MathAtan
            | BuiltinFunction::MathAcos
            | BuiltinFunction::MathIsClose
            | BuiltinFunction::TimeTime
            | BuiltinFunction::TimeTimeNs
            | BuiltinFunction::TimeLocalTime
            | BuiltinFunction::TimeGmTime
            | BuiltinFunction::TimeStrFTime
            | BuiltinFunction::TimeMonotonic
            | BuiltinFunction::TimeSleep
            | BuiltinFunction::OsGetPid
            | BuiltinFunction::OsGetCwd
            | BuiltinFunction::OsGetEnv
            | BuiltinFunction::OsGetTerminalSize
            | BuiltinFunction::OsTerminalSize
            | BuiltinFunction::OsOpen
            | BuiltinFunction::OsPipe
            | BuiltinFunction::OsRead
            | BuiltinFunction::OsReadInto
            | BuiltinFunction::OsWrite
            | BuiltinFunction::OsDup
            | BuiltinFunction::OsLSeek
            | BuiltinFunction::OsFTruncate
            | BuiltinFunction::OsClose
            | BuiltinFunction::OsKill
            | BuiltinFunction::OsIsATty
            | BuiltinFunction::OsSetInheritable
            | BuiltinFunction::OsGetInheritable
            | BuiltinFunction::OsURandom
            | BuiltinFunction::OsStat
            | BuiltinFunction::OsLStat
            | BuiltinFunction::OsMkdir
            | BuiltinFunction::OsChmod
            | BuiltinFunction::OsRmdir
            | BuiltinFunction::OsUTime
            | BuiltinFunction::OsScandir
            | BuiltinFunction::OsScandirIter
            | BuiltinFunction::OsScandirNext
            | BuiltinFunction::OsScandirEnter
            | BuiltinFunction::OsScandirExit
            | BuiltinFunction::OsScandirClose
            | BuiltinFunction::OsDirEntryIsDir
            | BuiltinFunction::OsDirEntryIsFile
            | BuiltinFunction::OsDirEntryIsSymlink
            | BuiltinFunction::OsWalk
            | BuiltinFunction::OsWIfStopped
            | BuiltinFunction::OsWStopSig
            | BuiltinFunction::OsWIfSignaled
            | BuiltinFunction::OsWTermSig
            | BuiltinFunction::OsWIfExited
            | BuiltinFunction::OsWExitStatus
            | BuiltinFunction::OsListDir
            | BuiltinFunction::OsAccess
            | BuiltinFunction::OsFspath
            | BuiltinFunction::OsFsEncode
            | BuiltinFunction::OsFsDecode
            | BuiltinFunction::OsRemove
            | BuiltinFunction::OsWaitStatusToExitCode
            | BuiltinFunction::OsPathExists
            | BuiltinFunction::OsPathJoin
            | BuiltinFunction::OsPathNormPath
            | BuiltinFunction::OsPathNormCase
            | BuiltinFunction::OsPathSplitRootEx
            | BuiltinFunction::OsPathSplit
            | BuiltinFunction::OsPathDirName
            | BuiltinFunction::OsPathBaseName
            | BuiltinFunction::OsPathIsAbs
            | BuiltinFunction::OsPathIsDir
            | BuiltinFunction::OsPathIsFile
            | BuiltinFunction::OsPathIsLink
            | BuiltinFunction::OsPathIsJunction
            | BuiltinFunction::OsPathSplitExt
            | BuiltinFunction::OsPathAbsPath
            | BuiltinFunction::OsPathExpandUser
            | BuiltinFunction::OsPathRealPath
            | BuiltinFunction::OsPathRelPath
            | BuiltinFunction::OsPathCommonPrefix
            | BuiltinFunction::OsWaitPid
            | BuiltinFunction::PosixSubprocessForkExec
            | BuiltinFunction::SubprocessPopenInit
            | BuiltinFunction::SubprocessPopenCommunicate
            | BuiltinFunction::SubprocessPopenWait
            | BuiltinFunction::SubprocessPopenKill
            | BuiltinFunction::SubprocessPopenPoll
            | BuiltinFunction::SubprocessPopenEnter
            | BuiltinFunction::SubprocessPopenExit
            | BuiltinFunction::SubprocessCleanup
            | BuiltinFunction::SubprocessCheckCall
            | BuiltinFunction::JsonDumps
            | BuiltinFunction::JsonLoads
            | BuiltinFunction::JsonEncodeBaseString
            | BuiltinFunction::JsonEncodeBaseStringAscii
            | BuiltinFunction::JsonMakeEncoder
            | BuiltinFunction::JsonMakeEncoderCall
            | BuiltinFunction::JsonScannerMakeScanner
            | BuiltinFunction::JsonScannerPyMakeScanner
            | BuiltinFunction::JsonScannerScanOnce
            | BuiltinFunction::JsonDecoderScanString
            | BuiltinFunction::PyLongIntToDecimalString
            | BuiltinFunction::PyLongIntDivMod
            | BuiltinFunction::PyLongIntFromString
            | BuiltinFunction::PyLongComputePowers
            | BuiltinFunction::PyLongDecStrToIntInner
            | BuiltinFunction::CodecsEncode
            | BuiltinFunction::CodecsDecode
            | BuiltinFunction::CodecsEscapeDecode
            | BuiltinFunction::CodecsLookup
            | BuiltinFunction::CodecsRegister
            | BuiltinFunction::CodecsGetIncrementalEncoder
            | BuiltinFunction::CodecsGetIncrementalDecoder
            | BuiltinFunction::CodecsIncrementalEncoderInit
            | BuiltinFunction::CodecsIncrementalEncoderEncode
            | BuiltinFunction::CodecsIncrementalEncoderReset
            | BuiltinFunction::CodecsIncrementalEncoderGetState
            | BuiltinFunction::CodecsIncrementalEncoderSetState
            | BuiltinFunction::CodecsIncrementalDecoderInit
            | BuiltinFunction::CodecsIncrementalDecoderDecode
            | BuiltinFunction::CodecsIncrementalDecoderReset
            | BuiltinFunction::CodecsIncrementalDecoderGetState
            | BuiltinFunction::CodecsIncrementalDecoderSetState
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
            | BuiltinFunction::OperatorItemGetter
            | BuiltinFunction::OperatorAttrGetter
            | BuiltinFunction::OperatorMethodCaller
            | BuiltinFunction::ItertoolsChain
            | BuiltinFunction::ItertoolsAccumulate
            | BuiltinFunction::ItertoolsCombinations
            | BuiltinFunction::ItertoolsCombinationsWithReplacement
            | BuiltinFunction::ItertoolsCompress
            | BuiltinFunction::ItertoolsCount
            | BuiltinFunction::ItertoolsCycle
            | BuiltinFunction::ItertoolsDropWhile
            | BuiltinFunction::ItertoolsFilterFalse
            | BuiltinFunction::ItertoolsGroupBy
            | BuiltinFunction::ItertoolsISlice
            | BuiltinFunction::ItertoolsPairwise
            | BuiltinFunction::ItertoolsRepeat
            | BuiltinFunction::ItertoolsStarMap
            | BuiltinFunction::ItertoolsTakeWhile
            | BuiltinFunction::ItertoolsTee
            | BuiltinFunction::ItertoolsZipLongest
            | BuiltinFunction::ItertoolsBatched
            | BuiltinFunction::ItertoolsPermutations
            | BuiltinFunction::ItertoolsProduct
            | BuiltinFunction::FunctoolsReduce
            | BuiltinFunction::FunctoolsSingleDispatch
            | BuiltinFunction::FunctoolsSingleDispatchMethod
            | BuiltinFunction::FunctoolsSingleDispatchRegister
            | BuiltinFunction::FunctoolsWraps
            | BuiltinFunction::FunctoolsPartial
            | BuiltinFunction::FunctoolsCmpToKey
            | BuiltinFunction::CollectionsCounter
            | BuiltinFunction::CollectionsDeque
            | BuiltinFunction::CollectionsDefaultDict
            | BuiltinFunction::InspectIsFunction
            | BuiltinFunction::InspectIsMethod
            | BuiltinFunction::InspectIsRoutine
            | BuiltinFunction::InspectIsMethodDescriptor
            | BuiltinFunction::InspectIsMethodWrapper
            | BuiltinFunction::InspectIsTraceback
            | BuiltinFunction::InspectIsFrame
            | BuiltinFunction::InspectIsCode
            | BuiltinFunction::InspectUnwrap
            | BuiltinFunction::InspectSignature
            | BuiltinFunction::InspectGetModule
            | BuiltinFunction::InspectGetFile
            | BuiltinFunction::InspectGetSourceFile
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
            | BuiltinFunction::TestInternalCapiGetRecursionDepth
            | BuiltinFunction::DataclassesField
            | BuiltinFunction::DataclassesIsDataclass
            | BuiltinFunction::DataclassesFields
            | BuiltinFunction::DataclassesAsDict
            | BuiltinFunction::DataclassesAsTuple
            | BuiltinFunction::DataclassesReplace
            | BuiltinFunction::DataclassesMakeDataclass
            | BuiltinFunction::IoOpen
            | BuiltinFunction::IoReadText
            | BuiltinFunction::IoWriteText
            | BuiltinFunction::IoTextEncoding
            | BuiltinFunction::IoTextIOWrapperInit
            | BuiltinFunction::IoFileRead
            | BuiltinFunction::IoFileReadLine
            | BuiltinFunction::IoFileReadLines
            | BuiltinFunction::IoFileWrite
            | BuiltinFunction::IoFileSeek
            | BuiltinFunction::IoFileTell
            | BuiltinFunction::IoFileClose
            | BuiltinFunction::IoFileFlush
            | BuiltinFunction::IoFileIter
            | BuiltinFunction::IoFileNext
            | BuiltinFunction::IoFileEnter
            | BuiltinFunction::IoFileExit
            | BuiltinFunction::IoFileFileno
            | BuiltinFunction::IoFileReadable
            | BuiltinFunction::IoFileWritable
            | BuiltinFunction::IoFileSeekable
            | BuiltinFunction::DateTimeNow
            | BuiltinFunction::DateToday
            | BuiltinFunction::DateInit
            | BuiltinFunction::AsyncioRun
            | BuiltinFunction::AsyncioSleep
            | BuiltinFunction::AsyncioCreateTask
            | BuiltinFunction::AsyncioGather
            | BuiltinFunction::ThreadingExcepthook
            | BuiltinFunction::ThreadingGetIdent
            | BuiltinFunction::ThreadStartNewThread
            | BuiltinFunction::ThreadingCurrentThread
            | BuiltinFunction::ThreadingMainThread
            | BuiltinFunction::ThreadingActiveCount
            | BuiltinFunction::ThreadClassInit
            | BuiltinFunction::ThreadClassStart
            | BuiltinFunction::ThreadClassJoin
            | BuiltinFunction::ThreadClassIsAlive
            | BuiltinFunction::ThreadEventInit
            | BuiltinFunction::ThreadEventClear
            | BuiltinFunction::ThreadEventIsSet
            | BuiltinFunction::ThreadEventSet
            | BuiltinFunction::ThreadEventWait
            | BuiltinFunction::ThreadConditionInit
            | BuiltinFunction::ThreadConditionAcquire
            | BuiltinFunction::ThreadConditionNotify
            | BuiltinFunction::ThreadConditionNotifyAll
            | BuiltinFunction::ThreadConditionRelease
            | BuiltinFunction::ThreadConditionWait
            | BuiltinFunction::ThreadSemaphoreInit
            | BuiltinFunction::ThreadSemaphoreAcquire
            | BuiltinFunction::ThreadSemaphoreRelease
            | BuiltinFunction::ThreadBoundedSemaphoreInit
            | BuiltinFunction::ThreadBarrierInit
            | BuiltinFunction::ThreadBarrierAbort
            | BuiltinFunction::ThreadBarrierReset
            | BuiltinFunction::ThreadBarrierWait
            | BuiltinFunction::SignalSignal
            | BuiltinFunction::SignalGetSignal
            | BuiltinFunction::SignalRaiseSignal
            | BuiltinFunction::SocketGetHostName
            | BuiltinFunction::SocketGetHostByName
            | BuiltinFunction::SocketGetAddrInfo
            | BuiltinFunction::SocketFromFd
            | BuiltinFunction::SocketGetDefaultTimeout
            | BuiltinFunction::SocketSetDefaultTimeout
            | BuiltinFunction::SocketNtoHs
            | BuiltinFunction::SocketNtoHl
            | BuiltinFunction::SocketHtoNs
            | BuiltinFunction::SocketHtoNl
            | BuiltinFunction::SocketObjectInit
            | BuiltinFunction::SocketObjectClose
            | BuiltinFunction::SocketObjectDetach
            | BuiltinFunction::SocketObjectFileno
            | BuiltinFunction::UuidClassInit
            | BuiltinFunction::UuidGetNode
            | BuiltinFunction::Uuid1
            | BuiltinFunction::Uuid3
            | BuiltinFunction::Uuid4
            | BuiltinFunction::Uuid5
            | BuiltinFunction::Uuid6
            | BuiltinFunction::Uuid7
            | BuiltinFunction::Uuid8
            | BuiltinFunction::BinasciiCrc32
            | BuiltinFunction::CsvReader
            | BuiltinFunction::CsvWriter
            | BuiltinFunction::CsvWriterRow
            | BuiltinFunction::CsvWriterRows
            | BuiltinFunction::CsvRegisterDialect
            | BuiltinFunction::CsvUnregisterDialect
            | BuiltinFunction::CsvGetDialect
            | BuiltinFunction::CsvListDialects
            | BuiltinFunction::CsvFieldSizeLimit
            | BuiltinFunction::CsvDialectValidate
            | BuiltinFunction::CollectionsCountElements
            | BuiltinFunction::AtexitRegister
            | BuiltinFunction::AtexitUnregister
            | BuiltinFunction::AtexitRunExitFuncs
            | BuiltinFunction::AtexitClear
            | BuiltinFunction::ColorizeCanColorize
            | BuiltinFunction::ColorizeGetTheme
            | BuiltinFunction::ColorizeGetColors
            | BuiltinFunction::ColorizeSetTheme
            | BuiltinFunction::ColorizeDecolor
            | BuiltinFunction::WarningsWarn
            | BuiltinFunction::WarningsWarnExplicit
            | BuiltinFunction::WarningsFiltersMutated
            | BuiltinFunction::WarningsAcquireLock
            | BuiltinFunction::WarningsReleaseLock
            | BuiltinFunction::ObjectNew
            | BuiltinFunction::ObjectInit
            | BuiltinFunction::ObjectGetAttribute
            | BuiltinFunction::ObjectSetAttr
            | BuiltinFunction::ObjectDelAttr
            | BuiltinFunction::Dir
            | BuiltinFunction::ObjectGetState
            | BuiltinFunction::ObjectSetState
            | BuiltinFunction::ObjectReduce
            | BuiltinFunction::ObjectReduceEx
            | BuiltinFunction::SetReduce
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
        Value::BigInt(value) => value
            .to_i64()
            .ok_or_else(|| RuntimeError::new("integer too large to convert")),
        Value::Bool(value) => Ok(if value { 1 } else { 0 }),
        _ => Err(RuntimeError::new("expected integer")),
    }
}

fn value_to_float(value: Value) -> Result<f64, RuntimeError> {
    match value {
        Value::Float(value) => Ok(value),
        Value::Int(value) => Ok(value as f64),
        Value::BigInt(value) => Ok(value.to_f64()),
        Value::Bool(value) => Ok(if value { 1.0 } else { 0.0 }),
        Value::Complex { real, imag } if imag == 0.0 => Ok(real),
        Value::Str(value) => value
            .trim()
            .parse::<f64>()
            .map_err(|_| RuntimeError::new("invalid float literal")),
        _ => Err(RuntimeError::new("expected numeric value")),
    }
}

fn int_to_prefixed_base_string(value: &Value, radix: u32, prefix: &str) -> Option<String> {
    match value {
        Value::Bool(flag) => {
            if *flag {
                Some(format!("{prefix}1"))
            } else {
                Some(format!("{prefix}0"))
            }
        }
        Value::Int(number) => {
            let negative = *number < 0;
            let magnitude = if negative {
                ((*number as i128).wrapping_neg()) as u128
            } else {
                *number as u128
            };
            let digits = match radix {
                2 => format!("{magnitude:b}"),
                8 => format!("{magnitude:o}"),
                16 => format!("{magnitude:x}"),
                _ => return None,
            };
            if negative {
                Some(format!("-{prefix}{digits}"))
            } else {
                Some(format!("{prefix}{digits}"))
            }
        }
        Value::BigInt(number) => {
            let text = number.to_str_radix(radix)?;
            if let Some(rest) = text.strip_prefix('-') {
                Some(format!("-{prefix}{rest}"))
            } else {
                Some(format!("{prefix}{text}"))
            }
        }
        _ => None,
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

fn dedup_values(values: Vec<Value>) -> Result<Vec<Value>, RuntimeError> {
    let mut out = Vec::new();
    for value in values {
        ensure_hashable_key(&value)?;
        if !out.iter().any(|existing| *existing == value) {
            out.push(value);
        }
    }
    Ok(out)
}

fn normalize_int_digits_for_base(
    digits: &str,
    radix: u32,
    allow_prefix_underscore: bool,
) -> Option<String> {
    if !(2..=36).contains(&radix) {
        return None;
    }
    if digits.is_empty() {
        return None;
    }

    let mut chars = digits.chars().peekable();
    if allow_prefix_underscore && matches!(chars.peek(), Some('_')) {
        chars.next();
    }

    let mut out = String::with_capacity(digits.len());
    let mut saw_digit = false;
    let mut prev_underscore = false;
    for ch in chars {
        if ch == '_' {
            if !saw_digit || prev_underscore {
                return None;
            }
            prev_underscore = true;
            continue;
        }
        if ch.to_digit(radix).is_none() {
            return None;
        }
        saw_digit = true;
        prev_underscore = false;
        out.push(ch);
    }
    if !saw_digit || prev_underscore {
        return None;
    }
    Some(out)
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
            Object::Set(values) => Ok(values.to_vec()),
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::FrozenSet(obj) => match &*obj.kind() {
            Object::FrozenSet(values) => Ok(values.to_vec()),
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::Dict(obj) => match &*obj.kind() {
            Object::Dict(values) => Ok(values.iter().map(|(key, _)| key.clone()).collect()),
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::DictKeys(obj) => match &*obj.kind() {
            Object::DictKeysView(view) => match &*view.dict.kind() {
                Object::Dict(values) => Ok(values.iter().map(|(key, _)| key.clone()).collect()),
                _ => Err(RuntimeError::new("expected iterable")),
            },
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(values) => {
                Ok(values.iter().map(|byte| Value::Int(*byte as i64)).collect())
            }
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(values) => {
                Ok(values.iter().map(|byte| Value::Int(*byte as i64)).collect())
            }
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => with_bytes_like_source(&view.source, |values| {
                let (start, end) = memoryview_bounds(view.start, view.length, values.len());
                Ok(values[start..end]
                    .iter()
                    .map(|byte| Value::Int(*byte as i64))
                    .collect())
            })
            .unwrap_or_else(|| Err(RuntimeError::new("expected iterable"))),
            _ => Err(RuntimeError::new("expected iterable")),
        },
        Value::Iterator(obj) => {
            let mut obj_kind = obj.kind_mut();
            let Object::Iterator(iterator) = &mut *obj_kind else {
                return Err(RuntimeError::new("expected iterable"));
            };
            match &mut iterator.kind {
                IteratorKind::List(list_obj) => match &*list_obj.kind() {
                    Object::List(values) => {
                        let start = iterator.index.min(values.len());
                        let out = values[start..].to_vec();
                        iterator.index = values.len();
                        Ok(out)
                    }
                    _ => Err(RuntimeError::new("expected iterable")),
                },
                IteratorKind::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                    Object::Tuple(values) => {
                        let start = iterator.index.min(values.len());
                        let out = values[start..].to_vec();
                        iterator.index = values.len();
                        Ok(out)
                    }
                    _ => Err(RuntimeError::new("expected iterable")),
                },
                IteratorKind::Str(text) => {
                    let chars = text.chars().collect::<Vec<_>>();
                    let start = iterator.index.min(chars.len());
                    let out = chars[start..]
                        .iter()
                        .map(|ch| Value::Str(ch.to_string()))
                        .collect::<Vec<_>>();
                    iterator.index = chars.len();
                    Ok(out)
                }
                IteratorKind::Dict(dict_obj) => match &*dict_obj.kind() {
                    Object::Dict(values) => {
                        let start = iterator.index.min(values.len());
                        let out = values
                            .iter()
                            .skip(start)
                            .map(|(key, _)| key.clone())
                            .collect::<Vec<_>>();
                        iterator.index = values.len();
                        Ok(out)
                    }
                    _ => Err(RuntimeError::new("expected iterable")),
                },
                IteratorKind::Set(set_obj) => match &*set_obj.kind() {
                    Object::Set(values) | Object::FrozenSet(values) => {
                        let all = values.to_vec();
                        let start = iterator.index.min(all.len());
                        let out = all.into_iter().skip(start).collect::<Vec<_>>();
                        iterator.index = start.saturating_add(out.len());
                        Ok(out)
                    }
                    _ => Err(RuntimeError::new("expected iterable")),
                },
                IteratorKind::Bytes(bytes_obj) => match &*bytes_obj.kind() {
                    Object::Bytes(values) => {
                        let start = iterator.index.min(values.len());
                        let out = values[start..]
                            .iter()
                            .map(|byte| Value::Int(*byte as i64))
                            .collect::<Vec<_>>();
                        iterator.index = values.len();
                        Ok(out)
                    }
                    _ => Err(RuntimeError::new("expected iterable")),
                },
                IteratorKind::ByteArray(bytearray_obj) => match &*bytearray_obj.kind() {
                    Object::ByteArray(values) => {
                        let start = iterator.index.min(values.len());
                        let out = values[start..]
                            .iter()
                            .map(|byte| Value::Int(*byte as i64))
                            .collect::<Vec<_>>();
                        iterator.index = values.len();
                        Ok(out)
                    }
                    _ => Err(RuntimeError::new("expected iterable")),
                },
                IteratorKind::MemoryView(memory_obj) => match &*memory_obj.kind() {
                    Object::MemoryView(view) => with_bytes_like_source(&view.source, |values| {
                        let (view_start, view_end) =
                            memoryview_bounds(view.start, view.length, values.len());
                        let view_len = view_end.saturating_sub(view_start);
                        let start = iterator.index.min(view_len);
                        let out = values[view_start + start..view_end]
                            .iter()
                            .map(|byte| Value::Int(*byte as i64))
                            .collect::<Vec<_>>();
                        iterator.index = view_len;
                        Ok(out)
                    })
                    .unwrap_or_else(|| Err(RuntimeError::new("expected iterable"))),
                    _ => Err(RuntimeError::new("expected iterable")),
                },
                IteratorKind::Map { values, .. } => {
                    let start = iterator.index.min(values.len());
                    let out = values[start..].to_vec();
                    iterator.index = values.len();
                    Ok(out)
                }
                IteratorKind::Range {
                    current,
                    stop,
                    step,
                } => {
                    let mut out = Vec::new();
                    let mut cursor = current.clone();
                    if step.is_zero() {
                        return Err(RuntimeError::new("range() arg 3 must not be zero"));
                    }
                    if !step.is_negative() {
                        while cursor.cmp_total(stop) == Ordering::Less {
                            out.push(match cursor.to_i64() {
                                Some(number) => Value::Int(number),
                                None => Value::BigInt(Box::new(cursor.clone())),
                            });
                            cursor = cursor.add(step);
                        }
                    } else {
                        while cursor.cmp_total(stop) == Ordering::Greater {
                            out.push(match cursor.to_i64() {
                                Some(number) => Value::Int(number),
                                None => Value::BigInt(Box::new(cursor.clone())),
                            });
                            cursor = cursor.add(step);
                        }
                    }
                    *current = cursor;
                    iterator.index = iterator.index.saturating_add(out.len());
                    Ok(out)
                }
                IteratorKind::RangeObject { start, stop, step } => {
                    if step.is_zero() {
                        return Err(RuntimeError::new("range() arg 3 must not be zero"));
                    }
                    let mut cursor = start.clone();
                    for _ in 0..iterator.index {
                        cursor = cursor.add(step);
                    }
                    let mut out = Vec::new();
                    if !step.is_negative() {
                        while cursor.cmp_total(stop) == Ordering::Less {
                            out.push(match cursor.to_i64() {
                                Some(number) => Value::Int(number),
                                None => Value::BigInt(Box::new(cursor.clone())),
                            });
                            cursor = cursor.add(step);
                        }
                    } else {
                        while cursor.cmp_total(stop) == Ordering::Greater {
                            out.push(match cursor.to_i64() {
                                Some(number) => Value::Int(number),
                                None => Value::BigInt(Box::new(cursor.clone())),
                            });
                            cursor = cursor.add(step);
                        }
                    }
                    iterator.index = iterator.index.saturating_add(out.len());
                    Ok(out)
                }
                IteratorKind::SequenceGetItem { .. } => Err(RuntimeError::new("expected iterable")),
                IteratorKind::Count { .. } | IteratorKind::Cycle { .. } => {
                    Err(RuntimeError::new("expected iterable"))
                }
            }
        }
        Value::Str(value) => Ok(value.chars().map(|ch| Value::Str(ch.to_string())).collect()),
        _ => Err(RuntimeError::new("expected iterable")),
    }
}

fn ensure_hashable_key(value: &Value) -> Result<(), RuntimeError> {
    if value_hash_key(value).is_some() {
        Ok(())
    } else {
        Err(RuntimeError::new(format!(
            "unhashable type: '{}'",
            value_type_name(value)
        )))
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::None => "NoneType",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::BigInt(_) => "int",
        Value::Float(_) => "float",
        Value::Complex { .. } => "complex",
        Value::Str(_) => "str",
        Value::List(_) => "list",
        Value::Tuple(_) => "tuple",
        Value::Dict(_) => "dict",
        Value::DictKeys(_) => "dict_keys",
        Value::Set(_) => "set",
        Value::FrozenSet(_) => "frozenset",
        Value::Bytes(_) => "bytes",
        Value::ByteArray(_) => "bytearray",
        Value::MemoryView(_) => "memoryview",
        Value::Iterator(_) => "iterator",
        Value::Generator(_) => "generator",
        Value::Module(_) => "module",
        Value::Class(_) => "type",
        Value::Instance(_) => "object",
        Value::Super(_) => "super",
        Value::Function(_) => "function",
        Value::BoundMethod(_) => "method",
        Value::Exception(_) => "exception",
        Value::ExceptionType(_) => "exceptiontype",
        Value::Slice(_) => "slice",
        Value::Code(_) => "code",
        Value::Builtin(_) => "builtin_function_or_method",
        Value::Cell(_) => "cell",
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

    if !matches!(
        encoding_name.as_str(),
        "utf-8" | "utf8" | "ascii" | "latin-1" | "latin1"
    ) {
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
        Value::Instance(obj) => match &*obj.kind() {
            Object::Instance(instance_data) => {
                if matches!(
                    &*instance_data.class.kind(),
                    Object::Class(class_data)
                        if class_data.name == "PickleBuffer"
                            && matches!(
                                instance_data.attrs.get("__pyrs_picklebuffer_released__"),
                                Some(Value::Bool(true))
                            )
                ) {
                    return Err(RuntimeError::new(
                        "ValueError: operation forbidden on released PickleBuffer object",
                    ));
                }
                match instance_data.attrs.get("__pyrs_bytes_storage__") {
                    Some(Value::Bytes(storage)) => match &*storage.kind() {
                        Object::Bytes(values) => Ok(values.clone()),
                        _ => Err(RuntimeError::new("bytes() unsupported type")),
                    },
                    Some(Value::ByteArray(storage)) => match &*storage.kind() {
                        Object::ByteArray(values) => Ok(values.clone()),
                        _ => Err(RuntimeError::new("bytes() unsupported type")),
                    },
                    _ => Err(RuntimeError::new("bytes() unsupported type")),
                }
            }
            _ => Err(RuntimeError::new("bytes() unsupported type")),
        },
        Value::MemoryView(obj) => match &*obj.kind() {
            Object::MemoryView(view) => with_bytes_like_source(&view.source, |values| {
                let (start, end) = memoryview_bounds(view.start, view.length, values.len());
                values[start..end].to_vec()
            })
            .ok_or_else(|| RuntimeError::new("bytes() unsupported type")),
            _ => Err(RuntimeError::new("bytes() unsupported type")),
        },
        Value::Module(obj) => match &*obj.kind() {
            Object::Module(module_data) if module_data.name == "__array__" => {
                let Some(Value::List(values_obj)) = module_data.globals.get("values") else {
                    return Err(RuntimeError::new("bytes() unsupported type"));
                };
                let Object::List(values) = &*values_obj.kind() else {
                    return Err(RuntimeError::new("bytes() unsupported type"));
                };
                let mut out = Vec::with_capacity(values.len());
                for item in values {
                    let value = value_to_int(item.clone())?;
                    if !(0..=255).contains(&value) {
                        return Err(RuntimeError::new("bytes must be in range(0, 256)"));
                    }
                    out.push(value as u8);
                }
                Ok(out)
            }
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

fn with_bytes_like_source<R>(source: &ObjRef, map: impl FnOnce(&[u8]) -> R) -> Option<R> {
    match &*source.kind() {
        Object::Bytes(values) | Object::ByteArray(values) => Some(map(values)),
        Object::Instance(instance_data) => {
            match instance_data.attrs.get("__pyrs_bytes_storage__") {
                Some(Value::Bytes(storage)) => match &*storage.kind() {
                    Object::Bytes(values) => Some(map(values)),
                    _ => None,
                },
                Some(Value::ByteArray(storage)) => match &*storage.kind() {
                    Object::ByteArray(values) => Some(map(values)),
                    _ => None,
                },
                _ => None,
            }
        }
        Object::Module(module_data) if module_data.name == "__array__" => {
            let values = module_data.globals.get("values")?;
            let Value::List(values_obj) = values else {
                return None;
            };
            let Object::List(items) = &*values_obj.kind() else {
                return None;
            };
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let value = value_to_int(item.clone()).ok()?;
                if !(0..=255).contains(&value) {
                    return None;
                }
                out.push(value as u8);
            }
            Some(map(&out))
        }
        _ => None,
    }
}

fn memoryview_bounds(start: usize, length: Option<usize>, source_len: usize) -> (usize, usize) {
    let start = start.min(source_len);
    let end = match length {
        Some(length) => start.saturating_add(length).min(source_len),
        None => source_len,
    };
    (start, end)
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

fn int_like_bigint(value: &Value) -> Option<BigInt> {
    match value {
        Value::Int(value) => Some(BigInt::from_i64(*value)),
        Value::Bool(value) => Some(BigInt::from_i64(if *value { 1 } else { 0 })),
        Value::BigInt(value) => Some((**value).clone()),
        _ => None,
    }
}

fn bigint_to_value(value: BigInt) -> Value {
    match value.to_i64() {
        Some(number) => Value::Int(number),
        None => Value::BigInt(Box::new(value)),
    }
}

fn divmod_values(left: Value, right: Value) -> Result<(Value, Value), RuntimeError> {
    if let (Some(left), Some(right)) = (int_like_bigint(&left), int_like_bigint(&right)) {
        let (quotient, remainder) = left
            .div_mod_floor(&right)
            .ok_or_else(|| RuntimeError::new("divmod() division by zero"))?;
        return Ok((bigint_to_value(quotient), bigint_to_value(remainder)));
    }
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
        Value::BigInt(_) => Value::Builtin(BuiltinFunction::Int),
        Value::Float(_) => Value::Builtin(BuiltinFunction::Float),
        Value::Complex { .. } => Value::Builtin(BuiltinFunction::Complex),
        Value::Str(_) => Value::Builtin(BuiltinFunction::Str),
        Value::List(_) => Value::Builtin(BuiltinFunction::List),
        Value::Tuple(_) => Value::Builtin(BuiltinFunction::Tuple),
        Value::Dict(_) => Value::Builtin(BuiltinFunction::Dict),
        Value::DictKeys(_) => Value::Str("dict_keys".to_string()),
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
        Value::Module(obj) => match &*obj.kind() {
            Object::Module(module_data) => match module_data.name.as_str() {
                "__staticmethod__" => Value::Builtin(BuiltinFunction::StaticMethod),
                "__classmethod__" => Value::Builtin(BuiltinFunction::ClassMethod),
                _ => Value::Builtin(BuiltinFunction::TypesModuleType),
            },
            _ => Value::Builtin(BuiltinFunction::TypesModuleType),
        },
        Value::Class(_) => Value::Builtin(BuiltinFunction::Type),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(obj) => Value::Class(obj.class.clone()),
            _ => Value::Str("object".to_string()),
        },
        Value::Super(_) => Value::Str("super".to_string()),
        Value::BoundMethod(_) => Value::Str("method".to_string()),
        Value::Function(_) => Value::Str("function".to_string()),
        Value::Cell(_) => Value::Str("cell".to_string()),
        Value::Exception(exception) => Value::ExceptionType(exception.name.clone()),
        Value::ExceptionType(_) => Value::Builtin(BuiltinFunction::Type),
        Value::Slice(_) => Value::Builtin(BuiltinFunction::Slice),
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

fn format_complex_component(value: f64) -> String {
    if value.is_finite()
        && value.fract() == 0.0
        && value >= i64::MIN as f64
        && value <= i64::MAX as f64
    {
        return (value as i64).to_string();
    }
    format_float(value)
}

fn format_complex(real: f64, imag: f64) -> String {
    if real == 0.0 {
        return format!("{}j", format_complex_component(imag));
    }
    let sign = if imag.is_sign_negative() { "-" } else { "+" };
    let imag_abs = if imag.is_sign_negative() { -imag } else { imag };
    format!(
        "({}{}{}j)",
        format_complex_component(real),
        sign,
        format_complex_component(imag_abs)
    )
}

fn format_bytes(values: &[u8], mutable: bool) -> String {
    let mut out = String::new();
    if mutable {
        out.push_str("bytearray(");
    }
    let use_double_quotes = values.contains(&b'\'') && !values.contains(&b'"');
    let quote = if use_double_quotes { '"' } else { '\'' };
    out.push('b');
    out.push(quote);
    for byte in values {
        match *byte {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            b'\'' if quote == '\'' => out.push_str("\\'"),
            b'"' if quote == '"' => out.push_str("\\\""),
            32..=126 => out.push(*byte as char),
            _ => out.push_str(&format!("\\x{:02x}", byte)),
        }
    }
    out.push(quote);
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
        Value::BigInt(value) => value.to_string(),
        Value::Float(value) => format_float(*value),
        Value::Complex { real, imag } => format_complex(*real, *imag),
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
        Value::DictKeys(obj) => match &*obj.kind() {
            Object::DictKeysView(view) => match &*view.dict.kind() {
                Object::Dict(values) => {
                    let mut parts = Vec::new();
                    for (key, _) in values {
                        parts.push(format_repr(key));
                    }
                    format!("dict_keys([{}])", parts.join(", "))
                }
                _ => "dict_keys([])".to_string(),
            },
            _ => "<dict_keys>".to_string(),
        },
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => {
                if values.is_empty() {
                    "set()".to_string()
                } else {
                    let mut parts = Vec::new();
                    for value in values {
                        parts.push(format_repr(value));
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
                    parts.push(format_repr(value));
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
                    NativeMethodKind::Builtin(builtin) => {
                        format!("<bound method {:?}>", builtin)
                    }
                    NativeMethodKind::GeneratorAwait => "<bound method __await__>".to_string(),
                    NativeMethodKind::GeneratorANext => "<bound method __anext__>".to_string(),
                    NativeMethodKind::GeneratorNext => "<bound method __next__>".to_string(),
                    NativeMethodKind::GeneratorSend => "<bound method send>".to_string(),
                    NativeMethodKind::GeneratorThrow => "<bound method throw>".to_string(),
                    NativeMethodKind::GeneratorClose => "<bound method close>".to_string(),
                    NativeMethodKind::DictKeys => "<bound method dict.keys>".to_string(),
                    NativeMethodKind::DictValues => "<bound method dict.values>".to_string(),
                    NativeMethodKind::DictItems => "<bound method dict.items>".to_string(),
                    NativeMethodKind::DictClear => "<bound method dict.clear>".to_string(),
                    NativeMethodKind::DictUpdateMethod => "<bound method dict.update>".to_string(),
                    NativeMethodKind::DictSetDefault => {
                        "<bound method dict.setdefault>".to_string()
                    }
                    NativeMethodKind::DictGet => "<bound method dict.get>".to_string(),
                    NativeMethodKind::DictGetItem => "<bound method dict.__getitem__>".to_string(),
                    NativeMethodKind::DictPop => "<bound method dict.pop>".to_string(),
                    NativeMethodKind::DictCopy => "<bound method dict.copy>".to_string(),
                    NativeMethodKind::ListAppend => "<bound method list.append>".to_string(),
                    NativeMethodKind::ListExtend => "<bound method list.extend>".to_string(),
                    NativeMethodKind::ListInsert => "<bound method list.insert>".to_string(),
                    NativeMethodKind::ListRemove => "<bound method list.remove>".to_string(),
                    NativeMethodKind::ListPop => "<bound method list.pop>".to_string(),
                    NativeMethodKind::ListCount => "<bound method list.count>".to_string(),
                    NativeMethodKind::TupleCount => "<bound method tuple.count>".to_string(),
                    NativeMethodKind::ListIndex => "<bound method list.index>".to_string(),
                    NativeMethodKind::ListReverse => "<bound method list.reverse>".to_string(),
                    NativeMethodKind::ListSort => "<bound method list.sort>".to_string(),
                    NativeMethodKind::IntToBytes => "<bound method int.to_bytes>".to_string(),
                    NativeMethodKind::IntBitLengthMethod => {
                        "<bound method int.bit_length>".to_string()
                    }
                    NativeMethodKind::IntIndexMethod => "<bound method int.__index__>".to_string(),
                    NativeMethodKind::StrStartsWith => "<bound method str.startswith>".to_string(),
                    NativeMethodKind::StrEndsWith => "<bound method str.endswith>".to_string(),
                    NativeMethodKind::StrReplace => "<bound method str.replace>".to_string(),
                    NativeMethodKind::StrUpper => "<bound method str.upper>".to_string(),
                    NativeMethodKind::StrLower => "<bound method str.lower>".to_string(),
                    NativeMethodKind::StrCapitalize => "<bound method str.capitalize>".to_string(),
                    NativeMethodKind::StrEncode => "<bound method str.encode>".to_string(),
                    NativeMethodKind::StrDecode => "<bound method str.decode>".to_string(),
                    NativeMethodKind::BytesDecode => "<bound method bytes.decode>".to_string(),
                    NativeMethodKind::BytesStartsWith => {
                        "<bound method bytes.startswith>".to_string()
                    }
                    NativeMethodKind::BytesEndsWith => "<bound method bytes.endswith>".to_string(),
                    NativeMethodKind::BytesCount => "<bound method bytes.count>".to_string(),
                    NativeMethodKind::BytesFind => "<bound method bytes.find>".to_string(),
                    NativeMethodKind::BytesTranslate => {
                        "<bound method bytes.translate>".to_string()
                    }
                    NativeMethodKind::BytesJoin => "<bound method bytes.join>".to_string(),
                    NativeMethodKind::ByteArrayExtend => {
                        "<bound method bytearray.extend>".to_string()
                    }
                    NativeMethodKind::ByteArrayClear => {
                        "<bound method bytearray.clear>".to_string()
                    }
                    NativeMethodKind::ByteArrayResize => {
                        "<bound method bytearray.resize>".to_string()
                    }
                    NativeMethodKind::MemoryViewEnter => {
                        "<bound method memoryview.__enter__>".to_string()
                    }
                    NativeMethodKind::MemoryViewExit => {
                        "<bound method memoryview.__exit__>".to_string()
                    }
                    NativeMethodKind::MemoryViewToReadOnly => {
                        "<bound method memoryview.toreadonly>".to_string()
                    }
                    NativeMethodKind::MemoryViewCast => {
                        "<bound method memoryview.cast>".to_string()
                    }
                    NativeMethodKind::MemoryViewToList => {
                        "<bound method memoryview.tolist>".to_string()
                    }
                    NativeMethodKind::MemoryViewRelease => {
                        "<bound method memoryview.release>".to_string()
                    }
                    NativeMethodKind::StrRemovePrefix => {
                        "<bound method str.removeprefix>".to_string()
                    }
                    NativeMethodKind::StrRemoveSuffix => {
                        "<bound method str.removesuffix>".to_string()
                    }
                    NativeMethodKind::StrFormat => "<bound method str.format>".to_string(),
                    NativeMethodKind::StrIsUpper => "<bound method str.isupper>".to_string(),
                    NativeMethodKind::StrIsLower => "<bound method str.islower>".to_string(),
                    NativeMethodKind::StrIsAscii => "<bound method str.isascii>".to_string(),
                    NativeMethodKind::StrIsAlNum => "<bound method str.isalnum>".to_string(),
                    NativeMethodKind::StrIsDigit => "<bound method str.isdigit>".to_string(),
                    NativeMethodKind::StrIsSpace => "<bound method str.isspace>".to_string(),
                    NativeMethodKind::StrIsIdentifier => {
                        "<bound method str.isidentifier>".to_string()
                    }
                    NativeMethodKind::StrJoin => "<bound method str.join>".to_string(),
                    NativeMethodKind::StrSplit => "<bound method str.split>".to_string(),
                    NativeMethodKind::StrSplitLines => "<bound method str.splitlines>".to_string(),
                    NativeMethodKind::StrRSplit => "<bound method str.rsplit>".to_string(),
                    NativeMethodKind::StrPartition => "<bound method str.partition>".to_string(),
                    NativeMethodKind::StrRPartition => "<bound method str.rpartition>".to_string(),
                    NativeMethodKind::StrCount => "<bound method str.count>".to_string(),
                    NativeMethodKind::StrFind => "<bound method str.find>".to_string(),
                    NativeMethodKind::StrTranslate => "<bound method str.translate>".to_string(),
                    NativeMethodKind::StrIndex => "<bound method str.index>".to_string(),
                    NativeMethodKind::StrRFind => "<bound method str.rfind>".to_string(),
                    NativeMethodKind::StrLStrip => "<bound method str.lstrip>".to_string(),
                    NativeMethodKind::StrRStrip => "<bound method str.rstrip>".to_string(),
                    NativeMethodKind::StrStrip => "<bound method str.strip>".to_string(),
                    NativeMethodKind::StrExpandTabs => "<bound method str.expandtabs>".to_string(),
                    NativeMethodKind::SetContains => "<bound method __contains__>".to_string(),
                    NativeMethodKind::SetAdd => "<bound method set.add>".to_string(),
                    NativeMethodKind::SetDiscard => "<bound method set.discard>".to_string(),
                    NativeMethodKind::SetUpdate => "<bound method set.update>".to_string(),
                    NativeMethodKind::SetUnion => "<bound method set.union>".to_string(),
                    NativeMethodKind::SetIntersection => {
                        "<bound method set.intersection>".to_string()
                    }
                    NativeMethodKind::SetDifference => "<bound method set.difference>".to_string(),
                    NativeMethodKind::SetIsSuperset => "<bound method set.issuperset>".to_string(),
                    NativeMethodKind::SetIsSubset => "<bound method set.issubset>".to_string(),
                    NativeMethodKind::SetIsDisjoint => "<bound method set.isdisjoint>".to_string(),
                    NativeMethodKind::RePatternSearch => {
                        "<bound method Pattern.search>".to_string()
                    }
                    NativeMethodKind::RePatternMatch => "<bound method Pattern.match>".to_string(),
                    NativeMethodKind::RePatternFullMatch => {
                        "<bound method Pattern.fullmatch>".to_string()
                    }
                    NativeMethodKind::RePatternSub => "<bound method Pattern.sub>".to_string(),
                    NativeMethodKind::ReMatchGroup => "<bound method Match.group>".to_string(),
                    NativeMethodKind::ReMatchGroups => "<bound method Match.groups>".to_string(),
                    NativeMethodKind::ReMatchGroupDict => {
                        "<bound method Match.groupdict>".to_string()
                    }
                    NativeMethodKind::ReMatchStart => "<bound method Match.start>".to_string(),
                    NativeMethodKind::ReMatchEnd => "<bound method Match.end>".to_string(),
                    NativeMethodKind::ReMatchSpan => "<bound method Match.span>".to_string(),
                    NativeMethodKind::ExceptionWithTraceback => {
                        "<bound method BaseException.with_traceback>".to_string()
                    }
                    NativeMethodKind::ExceptionAddNote => {
                        "<bound method BaseException.add_note>".to_string()
                    }
                    NativeMethodKind::DescriptorReduceTypeError => {
                        "<bound method descriptor.__reduce_ex__>".to_string()
                    }
                    NativeMethodKind::ObjectReduceExBound => {
                        "<bound method object.__reduce_ex__>".to_string()
                    }
                    NativeMethodKind::BoundMethodReduceEx => {
                        "<bound method method.__reduce_ex__>".to_string()
                    }
                    NativeMethodKind::ComplexReduceEx => {
                        "<bound method complex.__reduce_ex__>".to_string()
                    }
                    NativeMethodKind::ClassRegister => "<bound method register>".to_string(),
                    NativeMethodKind::PropertyGet => "<bound method property.__get__>".to_string(),
                    NativeMethodKind::PropertySet => "<bound method property.__set__>".to_string(),
                    NativeMethodKind::PropertyDelete => {
                        "<bound method property.__delete__>".to_string()
                    }
                    NativeMethodKind::PropertyGetter => {
                        "<bound method property.getter>".to_string()
                    }
                    NativeMethodKind::PropertySetter => {
                        "<bound method property.setter>".to_string()
                    }
                    NativeMethodKind::PropertyDeleter => {
                        "<bound method property.deleter>".to_string()
                    }
                    NativeMethodKind::CachedPropertyGet => {
                        "<bound method cached_property.__get__>".to_string()
                    }
                    NativeMethodKind::OperatorItemGetterCall => {
                        "<bound method operator.itemgetter-call>".to_string()
                    }
                    NativeMethodKind::OperatorAttrGetterCall => {
                        "<bound method operator.attrgetter-call>".to_string()
                    }
                    NativeMethodKind::OperatorMethodCallerCall => {
                        "<bound method operator.methodcaller-call>".to_string()
                    }
                    NativeMethodKind::FunctoolsWrapsDecorator => {
                        "<bound method functools.wraps-decorator>".to_string()
                    }
                    NativeMethodKind::FunctoolsPartialCall => {
                        "<bound method functools.partial-call>".to_string()
                    }
                    NativeMethodKind::FunctoolsCmpToKeyCall => {
                        "<bound method functools.cmp_to_key-call>".to_string()
                    }
                    NativeMethodKind::CodecsIncrementalEncoderFactoryCall => {
                        "<bound method codecs.incrementalencoder-factory-call>".to_string()
                    }
                    NativeMethodKind::CodecsIncrementalDecoderFactoryCall => {
                        "<bound method codecs.incrementaldecoder-factory-call>".to_string()
                    }
                    NativeMethodKind::CodecsIncrementalEncoderEncode => {
                        "<bound method codecs.incrementalencoder.encode>".to_string()
                    }
                    NativeMethodKind::CodecsIncrementalEncoderReset => {
                        "<bound method codecs.incrementalencoder.reset>".to_string()
                    }
                    NativeMethodKind::CodecsIncrementalEncoderGetState => {
                        "<bound method codecs.incrementalencoder.getstate>".to_string()
                    }
                    NativeMethodKind::CodecsIncrementalEncoderSetState => {
                        "<bound method codecs.incrementalencoder.setstate>".to_string()
                    }
                    NativeMethodKind::CodecsIncrementalDecoderDecode => {
                        "<bound method codecs.incrementaldecoder.decode>".to_string()
                    }
                    NativeMethodKind::CodecsIncrementalDecoderReset => {
                        "<bound method codecs.incrementaldecoder.reset>".to_string()
                    }
                    NativeMethodKind::CodecsIncrementalDecoderGetState => {
                        "<bound method codecs.incrementaldecoder.getstate>".to_string()
                    }
                    NativeMethodKind::CodecsIncrementalDecoderSetState => {
                        "<bound method codecs.incrementaldecoder.setstate>".to_string()
                    }
                },
                _ => "<bound method ?>".to_string(),
            },
            _ => "<bound method ?>".to_string(),
        },
        Value::Cell(_) => "<cell>".to_string(),
        Value::Exception(exception) => exception.message.clone().unwrap_or_default(),
        Value::ExceptionType(name) => format!("<class '{}'>", name),
        Value::Slice(slice) => {
            let lower = slice
                .lower
                .map_or("None".to_string(), |value| value.to_string());
            let upper = slice
                .upper
                .map_or("None".to_string(), |value| value.to_string());
            let step = slice
                .step
                .map_or("None".to_string(), |value| value.to_string());
            format!("slice({lower}, {upper}, {step})")
        }
        Value::Code(_) => "<code>".to_string(),
        Value::Function(_) => "<function>".to_string(),
        Value::Builtin(_) => "<builtin>".to_string(),
    }
}

fn format_repr_string(value: &str) -> String {
    let mut out = String::new();
    out.push('\'');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => append_python_char_escape(&mut out, c),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn append_python_char_escape(out: &mut String, ch: char) {
    let code = ch as u32;
    if code <= 0xFF {
        out.push_str(&format!("\\x{code:02x}"));
    } else if code <= 0xFFFF {
        out.push_str(&format!("\\u{code:04x}"));
    } else {
        out.push_str(&format!("\\U{code:08x}"));
    }
}

pub fn format_repr(value: &Value) -> String {
    match value {
        Value::Str(value) => format_repr_string(value),
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => {
                let mut parts = Vec::new();
                for value in values {
                    parts.push(format_repr(value));
                }
                format!("[{}]", parts.join(", "))
            }
            _ => "<list>".to_string(),
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => {
                let mut parts = Vec::new();
                for value in values {
                    parts.push(format_repr(value));
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
                    parts.push(format!("{}: {}", format_repr(key), format_repr(value)));
                }
                format!("{{{}}}", parts.join(", "))
            }
            _ => "<dict>".to_string(),
        },
        Value::DictKeys(obj) => match &*obj.kind() {
            Object::DictKeysView(view) => match &*view.dict.kind() {
                Object::Dict(values) => {
                    let mut parts = Vec::new();
                    for (key, _) in values {
                        parts.push(format_repr(key));
                    }
                    format!("dict_keys([{}])", parts.join(", "))
                }
                _ => "dict_keys([])".to_string(),
            },
            _ => "<dict_keys>".to_string(),
        },
        Value::Set(obj) => match &*obj.kind() {
            Object::Set(values) => {
                if values.is_empty() {
                    "set()".to_string()
                } else {
                    let mut parts = Vec::new();
                    for value in values {
                        parts.push(format_repr(value));
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
                    parts.push(format_repr(value));
                }
                format!("frozenset({{{}}})", parts.join(", "))
            }
            _ => "<frozenset>".to_string(),
        },
        Value::Instance(obj) => match &*obj.kind() {
            Object::Instance(instance_data) => {
                if let Object::Class(class_data) = &*instance_data.class.kind() {
                    if class_data.name == "StackObject" {
                        if let Some(Value::Str(name)) = instance_data.attrs.get("name") {
                            return name.clone();
                        }
                    }
                }
                format_value(value)
            }
            _ => format_value(value),
        },
        _ => format_value(value),
    }
}

pub fn format_ascii(value: &Value) -> String {
    let repr = format_repr(value);
    let mut out = String::with_capacity(repr.len());
    for ch in repr.chars() {
        if ch.is_ascii() {
            out.push(ch);
            continue;
        }
        append_python_char_escape(&mut out, ch);
    }
    out
}

fn is_truthy_value(value: &Value) -> bool {
    match value {
        Value::None => false,
        Value::Bool(value) => *value,
        Value::Int(value) => *value != 0,
        Value::BigInt(value) => !value.is_zero(),
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
        Value::DictKeys(obj) => match &*obj.kind() {
            Object::DictKeysView(view) => match &*view.dict.kind() {
                Object::Dict(values) => !values.is_empty(),
                _ => true,
            },
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
            Object::MemoryView(view) => {
                with_bytes_like_source(&view.source, |values| !values.is_empty()).unwrap_or(true)
            }
            _ => true,
        },
        Value::Iterator(_) => true,
        Value::Generator(_) => true,
        Value::Slice(_) => true,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc_collects_self_referential_dict_keys_view() {
        let heap = Heap::new();
        let view = heap.alloc(Object::DictKeysView(DictKeysView::new(
            heap.alloc(Object::List(Vec::new())),
        )));
        if let Object::DictKeysView(data) = &mut *view.kind_mut() {
            data.dict = view.clone();
        }
        drop(view);
        heap.collect_cycles(&[]);
        assert_eq!(heap.live_objects_count(), 0);
    }

    #[test]
    fn gc_collects_self_referential_memory_view() {
        let heap = Heap::new();
        let view = heap.alloc(Object::MemoryView(MemoryViewObject {
            source: heap.alloc(Object::ByteArray(vec![])),
            itemsize: 1,
            format: None,
            export_owner: None,
            released: false,
            start: 0,
            length: None,
        }));
        if let Object::MemoryView(data) = &mut *view.kind_mut() {
            data.source = view.clone();
        }
        drop(view);
        heap.collect_cycles(&[]);
        assert_eq!(heap.live_objects_count(), 0);
    }

    #[test]
    fn gc_collects_self_referential_super_object() {
        let heap = Heap::new();
        let super_obj = heap.alloc(Object::Super(SuperObject::new(
            heap.alloc(Object::List(Vec::new())),
            heap.alloc(Object::Tuple(Vec::new())),
            heap.alloc(Object::Dict(DictObject::new(Vec::new()))),
        )));
        if let Object::Super(data) = &mut *super_obj.kind_mut() {
            data.start_class = super_obj.clone();
            data.object = super_obj.clone();
            data.object_type = super_obj.clone();
        }
        drop(super_obj);
        heap.collect_cycles(&[]);
        assert_eq!(heap.live_objects_count(), 0);
    }

    #[test]
    fn gc_collects_self_referential_bound_method() {
        let heap = Heap::new();
        let method = heap.alloc(Object::BoundMethod(BoundMethod::new(
            heap.alloc(Object::Class(ClassObject::new("Placeholder", Vec::new()))),
            heap.alloc(Object::Instance(InstanceObject::new(
                heap.alloc(Object::Class(ClassObject::new("Receiver", Vec::new()))),
            ))),
        )));
        if let Object::BoundMethod(data) = &mut *method.kind_mut() {
            data.function = method.clone();
            data.receiver = method.clone();
        }
        drop(method);
        heap.collect_cycles(&[]);
        assert_eq!(heap.live_objects_count(), 0);
    }

    #[test]
    fn gc_collects_self_referential_instance_with_rooted_class() {
        let heap = Heap::new();
        let class = heap.alloc(Object::Class(ClassObject::new("Carrier", Vec::new())));
        let instance = heap.alloc(Object::Instance(InstanceObject::new(class.clone())));
        let instance_id = instance.id();
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data
                .attrs
                .insert("self".to_string(), Value::Instance(instance.clone()));
        }

        heap.collect_cycles(&[Value::Class(class)]);
        drop(instance);

        assert!(
            heap.find_object_by_id(instance_id).is_none(),
            "self-referential instance should be collectable",
        );
    }

    #[test]
    fn gc_preserves_rooted_self_referential_list() {
        let heap = Heap::new();
        let list = heap.alloc(Object::List(Vec::new()));
        if let Object::List(values) = &mut *list.kind_mut() {
            values.push(Value::List(list.clone()));
        }

        heap.collect_cycles(&[Value::List(list.clone())]);

        match &*list.kind() {
            Object::List(values) => {
                assert_eq!(values.len(), 1);
                match &values[0] {
                    Value::List(inner) => assert_eq!(inner.id(), list.id()),
                    other => panic!("expected rooted list reference, got {other:?}"),
                }
            }
            other => panic!("expected list object after rooted gc, got {other:?}"),
        }
    }

    #[test]
    fn gc_clears_unrooted_mutual_list_dict_cycle() {
        let heap = Heap::new();
        let list = heap.alloc(Object::List(Vec::new()));
        let dict = heap.alloc(Object::Dict(DictObject::new(Vec::new())));

        if let Object::List(values) = &mut *list.kind_mut() {
            values.push(Value::Dict(dict.clone()));
        }
        if let Object::Dict(entries) = &mut *dict.kind_mut() {
            entries.insert(Value::Str("list".to_string()), Value::List(list.clone()));
        }

        heap.collect_cycles(&[]);

        match &*list.kind() {
            Object::List(values) => assert!(values.is_empty(), "list cycle should be cleared"),
            other => panic!("expected list object, got {other:?}"),
        }
        match &*dict.kind() {
            Object::Dict(entries) => assert_eq!(entries.len(), 0, "dict cycle should be cleared"),
            other => panic!("expected dict object, got {other:?}"),
        }
    }

    #[test]
    fn immediate_ids_are_stable_for_equal_values() {
        let heap = Heap::new();
        let int_a = heap.id_of(&Value::Int(42));
        let int_b = heap.id_of(&Value::Int(42));
        let str_a = heap.id_of(&Value::Str("same".to_string()));
        let str_b = heap.id_of(&Value::Str("same".to_string()));
        let bool_true = heap.id_of(&Value::Bool(true));
        let bool_false = heap.id_of(&Value::Bool(false));

        assert_eq!(int_a, int_b);
        assert_eq!(str_a, str_b);
        assert_ne!(bool_true, bool_false);
        assert_ne!(int_a, str_a);
    }

    #[test]
    fn small_int_ids_are_stable_on_cpython_range() {
        let heap = Heap::new();
        let low_a = heap.id_of(&Value::Int(-5));
        let low_b = heap.id_of(&Value::Int(-5));
        let high_a = heap.id_of(&Value::Int(256));
        let high_b = heap.id_of(&Value::Int(256));
        let outside_a = heap.id_of(&Value::Int(257));
        let outside_b = heap.id_of(&Value::Int(257));

        assert_eq!(low_a, low_b);
        assert_eq!(high_a, high_b);
        assert_eq!(outside_a, outside_b);
        assert_ne!(low_a, high_a);
        assert_ne!(high_a, outside_a);
    }

    #[test]
    fn dict_object_remove_and_reinsert_keeps_lookup_consistent() {
        let mut dict = DictObject::new(vec![
            (Value::Str("a".to_string()), Value::Int(1)),
            (Value::Str("b".to_string()), Value::Int(2)),
            (Value::Str("c".to_string()), Value::Int(3)),
        ]);
        let removed = dict.remove_key(&Value::Str("b".to_string()));
        assert_eq!(removed, Some((Value::Str("b".to_string()), Value::Int(2))));
        assert_eq!(
            dict.find(&Value::Str("a".to_string())),
            Some(&Value::Int(1))
        );
        assert_eq!(
            dict.find(&Value::Str("c".to_string())),
            Some(&Value::Int(3))
        );
        assert!(!dict.contains_key(&Value::Str("b".to_string())));

        dict.insert(Value::Str("d".to_string()), Value::Int(4));
        dict.insert(Value::Str("a".to_string()), Value::Int(10));
        assert_eq!(
            dict.find(&Value::Str("d".to_string())),
            Some(&Value::Int(4))
        );
        assert_eq!(
            dict.find(&Value::Str("a".to_string())),
            Some(&Value::Int(10))
        );
    }

    #[test]
    fn set_object_remove_and_insert_keeps_membership_consistent() {
        let mut set = SetObject::new(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        assert!(set.contains(&Value::Int(2)));
        assert!(set.remove_value(&Value::Int(2)));
        assert!(!set.contains(&Value::Int(2)));
        assert!(!set.remove_value(&Value::Int(2)));
        assert!(set.insert(Value::Int(4)));
        assert!(set.contains(&Value::Int(4)));
    }

    #[test]
    fn index_bucket_push_remove_and_normalize() {
        let mut bucket = IndexBucket::new(3);
        assert_eq!(bucket.find_index_with(|idx| idx == 3), Some(3));
        bucket.push(7);
        assert_eq!(bucket.find_index_with(|idx| idx == 7), Some(7));
        bucket.remove_index(3);
        bucket.normalize();
        assert_eq!(bucket.find_index_with(|idx| idx == 3), None);
        assert_eq!(bucket.find_index_with(|idx| idx == 7), Some(7));
        bucket.remove_index(7);
        bucket.normalize();
        assert!(bucket.is_empty());
    }

    #[test]
    fn index_bucket_replace_and_adjust_indices() {
        let mut bucket = IndexBucket::new(8);
        bucket.push(10);
        bucket.push(12);
        bucket.replace_index(12, 5);
        assert_eq!(bucket.find_index_with(|idx| idx == 5), Some(5));
        bucket.adjust_indices_after_remove(7);
        assert_eq!(bucket.find_index_with(|idx| idx == 8), None);
        assert_eq!(bucket.find_index_with(|idx| idx == 7), Some(7));
        assert_eq!(bucket.find_index_with(|idx| idx == 5), Some(5));
    }

    #[test]
    fn format_repr_escapes_control_characters_and_quotes() {
        let value = Value::Str("line1\nline2\t'quoted'".to_string());
        let repr = format_repr(&value);
        assert_eq!(repr, "'line1\\nline2\\t\\'quoted\\''");
    }

    #[test]
    fn truthiness_matches_collection_and_memoryview_content() {
        let heap = Heap::new();
        let empty_list = heap.alloc_list(Vec::new());
        let non_empty_list = heap.alloc_list(vec![Value::Int(1)]);
        let empty_bytes = heap.alloc_bytes(Vec::new());
        let non_empty_bytes = heap.alloc_bytes(vec![1]);
        let empty_memory = match empty_bytes {
            Value::Bytes(obj) => heap.alloc_memoryview(obj),
            other => panic!("expected bytes value, got {other:?}"),
        };
        let non_empty_memory = match non_empty_bytes {
            Value::Bytes(obj) => heap.alloc_memoryview(obj),
            other => panic!("expected bytes value, got {other:?}"),
        };

        assert!(!is_truthy_value(&empty_list));
        assert!(is_truthy_value(&non_empty_list));
        assert!(!is_truthy_value(&empty_memory));
        assert!(is_truthy_value(&non_empty_memory));
    }
}
