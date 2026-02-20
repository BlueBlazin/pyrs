use super::super::{
    HashMap, ObjRef, Object, RuntimeError, Value, Vm, bytes_like_from_value, is_truthy,
};
use md5::Md5;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};

const HASH_KIND_ATTR: &str = "__pyrs_hash_kind__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HashKind {
    Md5,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl HashKind {
    fn module_name(self) -> &'static str {
        match self {
            Self::Md5 => "_md5",
            Self::Sha224 | Self::Sha256 | Self::Sha384 | Self::Sha512 => "_sha2",
        }
    }

    fn class_symbol(self) -> &'static str {
        match self {
            Self::Md5 => "MD5Type",
            Self::Sha224 => "SHA224Type",
            Self::Sha256 => "SHA256Type",
            Self::Sha384 => "SHA384Type",
            Self::Sha512 => "SHA512Type",
        }
    }

    fn hash_name(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha224 => "sha224",
            Self::Sha256 => "sha256",
            Self::Sha384 => "sha384",
            Self::Sha512 => "sha512",
        }
    }

    fn digest_size(self) -> i64 {
        match self {
            Self::Md5 => 16,
            Self::Sha224 => 28,
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }

    fn block_size(self) -> i64 {
        match self {
            Self::Md5 | Self::Sha224 | Self::Sha256 => 64,
            Self::Sha384 | Self::Sha512 => 128,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha224 => "sha224",
            Self::Sha256 => "sha256",
            Self::Sha384 => "sha384",
            Self::Sha512 => "sha512",
        }
    }
}

#[derive(Clone)]
pub(in crate::vm) enum HashState {
    Md5(Md5),
    Sha224(Sha224),
    Sha256(Sha256),
    Sha384(Sha384),
    Sha512(Sha512),
}

impl HashState {
    fn kind(&self) -> HashKind {
        match self {
            Self::Md5(_) => HashKind::Md5,
            Self::Sha224(_) => HashKind::Sha224,
            Self::Sha256(_) => HashKind::Sha256,
            Self::Sha384(_) => HashKind::Sha384,
            Self::Sha512(_) => HashKind::Sha512,
        }
    }

    fn new(kind: HashKind) -> Self {
        match kind {
            HashKind::Md5 => Self::Md5(Md5::new()),
            HashKind::Sha224 => Self::Sha224(Sha224::new()),
            HashKind::Sha256 => Self::Sha256(Sha256::new()),
            HashKind::Sha384 => Self::Sha384(Sha384::new()),
            HashKind::Sha512 => Self::Sha512(Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Md5(state) => state.update(data),
            Self::Sha224(state) => state.update(data),
            Self::Sha256(state) => state.update(data),
            Self::Sha384(state) => state.update(data),
            Self::Sha512(state) => state.update(data),
        }
    }

    fn digest_bytes(&self) -> Vec<u8> {
        match self {
            Self::Md5(state) => state.clone().finalize().to_vec(),
            Self::Sha224(state) => state.clone().finalize().to_vec(),
            Self::Sha256(state) => state.clone().finalize().to_vec(),
            Self::Sha384(state) => state.clone().finalize().to_vec(),
            Self::Sha512(state) => state.clone().finalize().to_vec(),
        }
    }
}

impl Vm {
    fn hash_method_owner_name(&self, receiver: &ObjRef) -> String {
        let Object::Instance(instance_data) = &*receiver.kind() else {
            return "hash".to_string();
        };
        let Object::Class(class_data) = &*instance_data.class.kind() else {
            return "hash".to_string();
        };
        class_data.name.clone()
    }

    fn hash_payload_from_value(&self, value: Value) -> Result<Vec<u8>, RuntimeError> {
        match value {
            Value::Str(_) => Err(RuntimeError::new(
                "TypeError: Strings must be encoded before hashing",
            )),
            other => bytes_like_from_value(other)
                .map_err(|_| RuntimeError::type_error("object supporting the buffer API required")),
        }
    }

    fn hash_constructor_payload(
        &self,
        args: &mut Vec<Value>,
        kwargs: &mut HashMap<String, Value>,
        constructor_name: &str,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new(format!(
                "TypeError: {constructor_name}() takes at most 1 positional argument ({} given)",
                args.len()
            )));
        }

        let positional = if args.is_empty() {
            None
        } else {
            Some(args.remove(0))
        };
        let kw_data = kwargs.remove("data");
        if positional.is_some() && kw_data.is_some() {
            return Err(RuntimeError::new(format!(
                "TypeError: argument for {constructor_name}() given by name ('data') and position (1)"
            )));
        }
        let mut data_arg = positional.or(kw_data);
        let string_arg = kwargs.remove("string");
        if let Some(value) = kwargs.remove("usedforsecurity") {
            let _ = is_truthy(&value);
        }
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(RuntimeError::new(format!(
                "TypeError: {constructor_name}() got an unexpected keyword argument '{unexpected}'"
            )));
        }
        if data_arg.is_some() && string_arg.is_some() {
            return Err(RuntimeError::new(
                "TypeError: 'data' and 'string' are mutually exclusive and support for 'string' keyword parameter is slated for removal in a future version.",
            ));
        }
        if data_arg.is_none() {
            data_arg = string_arg;
        }
        match data_arg {
            Some(value) => Ok(Some(self.hash_payload_from_value(value)?)),
            None => Ok(None),
        }
    }

    fn hash_class_from_kind(&self, kind: HashKind) -> Option<ObjRef> {
        let module = self.modules.get(kind.module_name())?;
        let Object::Module(module_data) = &*module.kind() else {
            return None;
        };
        match module_data.globals.get(kind.class_symbol()) {
            Some(Value::Class(class)) => Some(class.clone()),
            _ => None,
        }
    }

    fn hash_init_instance_attrs(&mut self, instance: &ObjRef, kind: HashKind) {
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data.attrs.insert(
                HASH_KIND_ATTR.to_string(),
                Value::Str(kind.tag().to_string()),
            );
            instance_data
                .attrs
                .insert("name".to_string(), Value::Str(kind.hash_name().to_string()));
            instance_data
                .attrs
                .insert("digest_size".to_string(), Value::Int(kind.digest_size()));
            instance_data
                .attrs
                .insert("block_size".to_string(), Value::Int(kind.block_size()));
        }
    }

    fn hash_new_instance(
        &mut self,
        kind: HashKind,
        payload: Option<Vec<u8>>,
    ) -> Result<Value, RuntimeError> {
        let class = self.hash_class_from_kind(kind).ok_or_else(|| {
            RuntimeError::new(format!(
                "RuntimeError: {} backend type '{}' is unavailable",
                kind.module_name(),
                kind.class_symbol()
            ))
        })?;
        let instance = self.alloc_instance_for_class(&class);
        self.hash_init_instance_attrs(&instance, kind);
        let mut state = HashState::new(kind);
        if let Some(data) = payload {
            state.update(&data);
        }
        self.hash_states.insert(instance.id(), state);
        Ok(Value::Instance(instance))
    }

    fn hash_constructor(
        &mut self,
        kind: HashKind,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
        constructor_name: &str,
    ) -> Result<Value, RuntimeError> {
        let payload = self.hash_constructor_payload(&mut args, &mut kwargs, constructor_name)?;
        self.hash_new_instance(kind, payload)
    }

    fn hash_receiver_from_args<'a>(
        &'a self,
        args: &'a [Value],
        method_name: &str,
    ) -> Result<(&'a ObjRef, String), RuntimeError> {
        let Some(Value::Instance(receiver)) = args.first() else {
            return Err(RuntimeError::new(format!(
                "TypeError: {method_name}() requires a hash object"
            )));
        };
        if !self.hash_states.contains_key(&receiver.id()) {
            return Err(RuntimeError::new(format!(
                "TypeError: {method_name}() requires a hash object"
            )));
        }
        Ok((receiver, self.hash_method_owner_name(receiver)))
    }

    pub(in crate::vm) fn builtin_hashlib_md5(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Md5, args, kwargs, "md5")
    }

    pub(in crate::vm) fn builtin_hashlib_sha224(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Sha224, args, kwargs, "sha224")
    }

    pub(in crate::vm) fn builtin_hashlib_sha256(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Sha256, args, kwargs, "sha256")
    }

    pub(in crate::vm) fn builtin_hashlib_sha384(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Sha384, args, kwargs, "sha384")
    }

    pub(in crate::vm) fn builtin_hashlib_sha512(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Sha512, args, kwargs, "sha512")
    }

    pub(in crate::vm) fn builtin_hashlib_hash_update(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (receiver, owner_name) = self.hash_receiver_from_args(&args, "update")?;
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(format!(
                "TypeError: {owner_name}.update() takes no keyword arguments"
            )));
        }
        let provided = args.len().saturating_sub(1);
        if args.len() != 2 {
            return Err(RuntimeError::new(format!(
                "TypeError: {owner_name}.update() takes exactly one argument ({provided} given)"
            )));
        }
        let payload = self.hash_payload_from_value(args[1].clone())?;
        let Some(state) = self.hash_states.get_mut(&receiver.id()) else {
            return Err(RuntimeError::new(
                "TypeError: update() requires a hash object",
            ));
        };
        state.update(&payload);
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_hashlib_hash_digest(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (receiver, owner_name) = self.hash_receiver_from_args(&args, "digest")?;
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(format!(
                "TypeError: {owner_name}.digest() takes no keyword arguments"
            )));
        }
        let provided = args.len().saturating_sub(1);
        if args.len() != 1 {
            return Err(RuntimeError::new(format!(
                "TypeError: {owner_name}.digest() takes no arguments ({provided} given)"
            )));
        }
        let digest = self
            .hash_states
            .get(&receiver.id())
            .ok_or_else(|| RuntimeError::type_error("digest() requires a hash object"))?
            .digest_bytes();
        Ok(self.heap.alloc_bytes(digest))
    }

    pub(in crate::vm) fn builtin_hashlib_hash_hexdigest(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (receiver, owner_name) = self.hash_receiver_from_args(&args, "hexdigest")?;
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(format!(
                "TypeError: {owner_name}.hexdigest() takes no keyword arguments"
            )));
        }
        let provided = args.len().saturating_sub(1);
        if args.len() != 1 {
            return Err(RuntimeError::new(format!(
                "TypeError: {owner_name}.hexdigest() takes no arguments ({provided} given)"
            )));
        }
        let digest = self
            .hash_states
            .get(&receiver.id())
            .ok_or_else(|| RuntimeError::type_error("hexdigest() requires a hash object"))?
            .digest_bytes();
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            out.push_str(&format!("{byte:02x}"));
        }
        Ok(Value::Str(out))
    }

    pub(in crate::vm) fn builtin_hashlib_hash_copy(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (receiver, _owner_name) = self.hash_receiver_from_args(&args, "copy")?;
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: copy() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error("copy() takes no arguments"));
        }
        let state = self
            .hash_states
            .get(&receiver.id())
            .ok_or_else(|| RuntimeError::type_error("copy() requires a hash object"))?
            .clone();
        let kind = state.kind();
        let class = self.hash_class_from_kind(kind).ok_or_else(|| {
            RuntimeError::new(format!(
                "RuntimeError: {} backend type '{}' is unavailable",
                kind.module_name(),
                kind.class_symbol()
            ))
        })?;
        let new_instance = self.alloc_instance_for_class(&class);
        self.hash_init_instance_attrs(&new_instance, kind);
        self.hash_states.insert(new_instance.id(), state);
        Ok(Value::Instance(new_instance))
    }
}
