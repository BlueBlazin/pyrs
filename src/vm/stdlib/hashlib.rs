use super::super::{
    BuiltinFunction, HashMap, ObjRef, Object, RuntimeError, Value, Vm, bytes_like_from_value,
    is_truthy, value_to_int,
};
use blake2::{Blake2b512, Blake2s256};
use hmac::{Mac, SimpleHmac};
use md5::Md5;
use pbkdf2::pbkdf2_hmac;
use scrypt::{Params as ScryptParams, scrypt};
use sha1::Sha1;
use sha2::digest::{Digest, ExtendableOutput, Update, XofReader};
use sha2::{Sha224, Sha256, Sha384, Sha512};
use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512, Shake128, Shake256};

const HASH_KIND_ATTR: &str = "__pyrs_hash_kind__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HashKind {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
    Blake2b,
    Blake2s,
    Sha3_224,
    Sha3_256,
    Sha3_384,
    Sha3_512,
    Shake128,
    Shake256,
}

impl HashKind {
    fn module_name(self) -> &'static str {
        match self {
            Self::Md5 => "_md5",
            Self::Sha1 => "_sha1",
            Self::Sha224 | Self::Sha256 | Self::Sha384 | Self::Sha512 => "_sha2",
            Self::Blake2b | Self::Blake2s => "_blake2",
            Self::Sha3_224
            | Self::Sha3_256
            | Self::Sha3_384
            | Self::Sha3_512
            | Self::Shake128
            | Self::Shake256 => "_sha3",
        }
    }

    fn class_symbol(self) -> &'static str {
        match self {
            Self::Md5 => "MD5Type",
            Self::Sha1 => "SHA1Type",
            Self::Sha224 => "SHA224Type",
            Self::Sha256 => "SHA256Type",
            Self::Sha384 => "SHA384Type",
            Self::Sha512 => "SHA512Type",
            Self::Blake2b => "_BLAKE2bType",
            Self::Blake2s => "_BLAKE2sType",
            Self::Sha3_224 => "_SHA3_224Type",
            Self::Sha3_256 => "_SHA3_256Type",
            Self::Sha3_384 => "_SHA3_384Type",
            Self::Sha3_512 => "_SHA3_512Type",
            Self::Shake128 => "_SHAKE128Type",
            Self::Shake256 => "_SHAKE256Type",
        }
    }

    fn hash_name(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha224 => "sha224",
            Self::Sha256 => "sha256",
            Self::Sha384 => "sha384",
            Self::Sha512 => "sha512",
            Self::Blake2b => "blake2b",
            Self::Blake2s => "blake2s",
            Self::Sha3_224 => "sha3_224",
            Self::Sha3_256 => "sha3_256",
            Self::Sha3_384 => "sha3_384",
            Self::Sha3_512 => "sha3_512",
            Self::Shake128 => "shake_128",
            Self::Shake256 => "shake_256",
        }
    }

    fn digest_size(self) -> i64 {
        match self {
            Self::Md5 => 16,
            Self::Sha1 => 20,
            Self::Sha224 => 28,
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
            Self::Blake2b => 64,
            Self::Blake2s => 32,
            Self::Sha3_224 => 28,
            Self::Sha3_256 => 32,
            Self::Sha3_384 => 48,
            Self::Sha3_512 => 64,
            Self::Shake128 | Self::Shake256 => 0,
        }
    }

    fn block_size(self) -> i64 {
        match self {
            Self::Md5 | Self::Sha1 | Self::Sha224 | Self::Sha256 => 64,
            Self::Sha384 | Self::Sha512 | Self::Blake2b => 128,
            Self::Blake2s => 64,
            Self::Sha3_224 => 144,
            Self::Sha3_256 => 136,
            Self::Sha3_384 => 104,
            Self::Sha3_512 => 72,
            Self::Shake128 => 168,
            Self::Shake256 => 136,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha224 => "sha224",
            Self::Sha256 => "sha256",
            Self::Sha384 => "sha384",
            Self::Sha512 => "sha512",
            Self::Blake2b => "blake2b",
            Self::Blake2s => "blake2s",
            Self::Sha3_224 => "sha3_224",
            Self::Sha3_256 => "sha3_256",
            Self::Sha3_384 => "sha3_384",
            Self::Sha3_512 => "sha3_512",
            Self::Shake128 => "shake_128",
            Self::Shake256 => "shake_256",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "md5" => Some(Self::Md5),
            "sha1" => Some(Self::Sha1),
            "sha224" => Some(Self::Sha224),
            "sha256" => Some(Self::Sha256),
            "sha384" => Some(Self::Sha384),
            "sha512" => Some(Self::Sha512),
            "blake2b" => Some(Self::Blake2b),
            "blake2s" => Some(Self::Blake2s),
            "sha3_224" => Some(Self::Sha3_224),
            "sha3_256" => Some(Self::Sha3_256),
            "sha3_384" => Some(Self::Sha3_384),
            "sha3_512" => Some(Self::Sha3_512),
            "shake_128" => Some(Self::Shake128),
            "shake_256" => Some(Self::Shake256),
            _ => None,
        }
    }

    fn is_xof(self) -> bool {
        matches!(self, Self::Shake128 | Self::Shake256)
    }
}

#[derive(Clone)]
pub(in crate::vm) enum HashState {
    Md5(Md5),
    Sha1(Sha1),
    Sha224(Sha224),
    Sha256(Sha256),
    Sha384(Sha384),
    Sha512(Sha512),
    Blake2b(Blake2b512),
    Blake2s(Blake2s256),
    Sha3_224(Sha3_224),
    Sha3_256(Sha3_256),
    Sha3_384(Sha3_384),
    Sha3_512(Sha3_512),
    Shake128(Shake128),
    Shake256(Shake256),
}

impl HashState {
    fn kind(&self) -> HashKind {
        match self {
            Self::Md5(_) => HashKind::Md5,
            Self::Sha1(_) => HashKind::Sha1,
            Self::Sha224(_) => HashKind::Sha224,
            Self::Sha256(_) => HashKind::Sha256,
            Self::Sha384(_) => HashKind::Sha384,
            Self::Sha512(_) => HashKind::Sha512,
            Self::Blake2b(_) => HashKind::Blake2b,
            Self::Blake2s(_) => HashKind::Blake2s,
            Self::Sha3_224(_) => HashKind::Sha3_224,
            Self::Sha3_256(_) => HashKind::Sha3_256,
            Self::Sha3_384(_) => HashKind::Sha3_384,
            Self::Sha3_512(_) => HashKind::Sha3_512,
            Self::Shake128(_) => HashKind::Shake128,
            Self::Shake256(_) => HashKind::Shake256,
        }
    }

    fn new(kind: HashKind) -> Self {
        match kind {
            HashKind::Md5 => Self::Md5(Md5::new()),
            HashKind::Sha1 => Self::Sha1(Sha1::new()),
            HashKind::Sha224 => Self::Sha224(Sha224::new()),
            HashKind::Sha256 => Self::Sha256(Sha256::new()),
            HashKind::Sha384 => Self::Sha384(Sha384::new()),
            HashKind::Sha512 => Self::Sha512(Sha512::new()),
            HashKind::Blake2b => Self::Blake2b(Blake2b512::new()),
            HashKind::Blake2s => Self::Blake2s(Blake2s256::new()),
            HashKind::Sha3_224 => Self::Sha3_224(Sha3_224::new()),
            HashKind::Sha3_256 => Self::Sha3_256(Sha3_256::new()),
            HashKind::Sha3_384 => Self::Sha3_384(Sha3_384::new()),
            HashKind::Sha3_512 => Self::Sha3_512(Sha3_512::new()),
            HashKind::Shake128 => Self::Shake128(Shake128::default()),
            HashKind::Shake256 => Self::Shake256(Shake256::default()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Md5(state) => Update::update(state, data),
            Self::Sha1(state) => Update::update(state, data),
            Self::Sha224(state) => Update::update(state, data),
            Self::Sha256(state) => Update::update(state, data),
            Self::Sha384(state) => Update::update(state, data),
            Self::Sha512(state) => Update::update(state, data),
            Self::Blake2b(state) => Update::update(state, data),
            Self::Blake2s(state) => Update::update(state, data),
            Self::Sha3_224(state) => Update::update(state, data),
            Self::Sha3_256(state) => Update::update(state, data),
            Self::Sha3_384(state) => Update::update(state, data),
            Self::Sha3_512(state) => Update::update(state, data),
            Self::Shake128(state) => Update::update(state, data),
            Self::Shake256(state) => Update::update(state, data),
        }
    }

    fn digest_bytes(&self, len: Option<usize>) -> Result<Vec<u8>, RuntimeError> {
        match self {
            Self::Md5(state) => Ok(state.clone().finalize().to_vec()),
            Self::Sha1(state) => Ok(state.clone().finalize().to_vec()),
            Self::Sha224(state) => Ok(state.clone().finalize().to_vec()),
            Self::Sha256(state) => Ok(state.clone().finalize().to_vec()),
            Self::Sha384(state) => Ok(state.clone().finalize().to_vec()),
            Self::Sha512(state) => Ok(state.clone().finalize().to_vec()),
            Self::Blake2b(state) => Ok(state.clone().finalize().to_vec()),
            Self::Blake2s(state) => Ok(state.clone().finalize().to_vec()),
            Self::Sha3_224(state) => Ok(state.clone().finalize().to_vec()),
            Self::Sha3_256(state) => Ok(state.clone().finalize().to_vec()),
            Self::Sha3_384(state) => Ok(state.clone().finalize().to_vec()),
            Self::Sha3_512(state) => Ok(state.clone().finalize().to_vec()),
            Self::Shake128(state) => {
                let requested = len.ok_or_else(|| {
                    RuntimeError::new(
                        "TypeError: digest() missing required argument 'length' (pos 1)",
                    )
                })?;
                let mut out = vec![0u8; requested];
                let mut reader = state.clone().finalize_xof();
                reader.read(&mut out);
                Ok(out)
            }
            Self::Shake256(state) => {
                let requested = len.ok_or_else(|| {
                    RuntimeError::new(
                        "TypeError: digest() missing required argument 'length' (pos 1)",
                    )
                })?;
                let mut out = vec![0u8; requested];
                let mut reader = state.clone().finalize_xof();
                reader.read(&mut out);
                Ok(out)
            }
        }
    }
}

#[derive(Clone)]
pub(in crate::vm) enum HmacState {
    Md5(SimpleHmac<Md5>),
    Sha1(SimpleHmac<Sha1>),
    Sha224(SimpleHmac<Sha224>),
    Sha256(SimpleHmac<Sha256>),
    Sha384(SimpleHmac<Sha384>),
    Sha512(SimpleHmac<Sha512>),
    Blake2b(SimpleHmac<Blake2b512>),
    Blake2s(SimpleHmac<Blake2s256>),
    Sha3_224(SimpleHmac<Sha3_224>),
    Sha3_256(SimpleHmac<Sha3_256>),
    Sha3_384(SimpleHmac<Sha3_384>),
    Sha3_512(SimpleHmac<Sha3_512>),
}

impl HmacState {
    fn kind(&self) -> HashKind {
        match self {
            Self::Md5(_) => HashKind::Md5,
            Self::Sha1(_) => HashKind::Sha1,
            Self::Sha224(_) => HashKind::Sha224,
            Self::Sha256(_) => HashKind::Sha256,
            Self::Sha384(_) => HashKind::Sha384,
            Self::Sha512(_) => HashKind::Sha512,
            Self::Blake2b(_) => HashKind::Blake2b,
            Self::Blake2s(_) => HashKind::Blake2s,
            Self::Sha3_224(_) => HashKind::Sha3_224,
            Self::Sha3_256(_) => HashKind::Sha3_256,
            Self::Sha3_384(_) => HashKind::Sha3_384,
            Self::Sha3_512(_) => HashKind::Sha3_512,
        }
    }

    fn new(kind: HashKind, key: &[u8]) -> Result<Self, RuntimeError> {
        let map_err = |_| RuntimeError::new("ValueError: invalid hmac key");
        match kind {
            HashKind::Md5 => SimpleHmac::<Md5>::new_from_slice(key)
                .map(Self::Md5)
                .map_err(map_err),
            HashKind::Sha1 => SimpleHmac::<Sha1>::new_from_slice(key)
                .map(Self::Sha1)
                .map_err(map_err),
            HashKind::Sha224 => SimpleHmac::<Sha224>::new_from_slice(key)
                .map(Self::Sha224)
                .map_err(map_err),
            HashKind::Sha256 => SimpleHmac::<Sha256>::new_from_slice(key)
                .map(Self::Sha256)
                .map_err(map_err),
            HashKind::Sha384 => SimpleHmac::<Sha384>::new_from_slice(key)
                .map(Self::Sha384)
                .map_err(map_err),
            HashKind::Sha512 => SimpleHmac::<Sha512>::new_from_slice(key)
                .map(Self::Sha512)
                .map_err(map_err),
            HashKind::Blake2b => SimpleHmac::<Blake2b512>::new_from_slice(key)
                .map(Self::Blake2b)
                .map_err(map_err),
            HashKind::Blake2s => SimpleHmac::<Blake2s256>::new_from_slice(key)
                .map(Self::Blake2s)
                .map_err(map_err),
            HashKind::Sha3_224 => SimpleHmac::<Sha3_224>::new_from_slice(key)
                .map(Self::Sha3_224)
                .map_err(map_err),
            HashKind::Sha3_256 => SimpleHmac::<Sha3_256>::new_from_slice(key)
                .map(Self::Sha3_256)
                .map_err(map_err),
            HashKind::Sha3_384 => SimpleHmac::<Sha3_384>::new_from_slice(key)
                .map(Self::Sha3_384)
                .map_err(map_err),
            HashKind::Sha3_512 => SimpleHmac::<Sha3_512>::new_from_slice(key)
                .map(Self::Sha3_512)
                .map_err(map_err),
            HashKind::Shake128 | HashKind::Shake256 => {
                Err(RuntimeError::value_error("no reason supplied"))
            }
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Md5(state) => Mac::update(state, data),
            Self::Sha1(state) => Mac::update(state, data),
            Self::Sha224(state) => Mac::update(state, data),
            Self::Sha256(state) => Mac::update(state, data),
            Self::Sha384(state) => Mac::update(state, data),
            Self::Sha512(state) => Mac::update(state, data),
            Self::Blake2b(state) => Mac::update(state, data),
            Self::Blake2s(state) => Mac::update(state, data),
            Self::Sha3_224(state) => Mac::update(state, data),
            Self::Sha3_256(state) => Mac::update(state, data),
            Self::Sha3_384(state) => Mac::update(state, data),
            Self::Sha3_512(state) => Mac::update(state, data),
        }
    }

    fn digest_bytes(&self) -> Vec<u8> {
        match self {
            Self::Md5(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Sha1(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Sha224(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Sha256(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Sha384(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Sha512(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Blake2b(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Blake2s(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Sha3_224(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Sha3_256(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Sha3_384(state) => state.clone().finalize().into_bytes().to_vec(),
            Self::Sha3_512(state) => state.clone().finalize().into_bytes().to_vec(),
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
            Value::Bytes(_)
            | Value::ByteArray(_)
            | Value::MemoryView(_)
            | Value::Instance(_)
            | Value::Module(_) => bytes_like_from_value(value)
                .map_err(|_| RuntimeError::type_error("object supporting the buffer API required")),
            _ => Err(RuntimeError::type_error(
                "object supporting the buffer API required",
            )),
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

    fn hash_kind_from_value(&self, value: Value) -> Result<HashKind, RuntimeError> {
        let Value::Str(name) = value else {
            return Err(RuntimeError::new("TypeError: hash name must be a string"));
        };
        HashKind::from_name(&name)
            .ok_or_else(|| RuntimeError::new(format!("ValueError: unsupported hash type {name}")))
    }

    fn hash_kind_from_builtin_constructor(&self, builtin: BuiltinFunction) -> Option<HashKind> {
        match builtin {
            BuiltinFunction::HashlibMd5 => Some(HashKind::Md5),
            BuiltinFunction::HashlibSha1 => Some(HashKind::Sha1),
            BuiltinFunction::HashlibSha224 => Some(HashKind::Sha224),
            BuiltinFunction::HashlibSha256 => Some(HashKind::Sha256),
            BuiltinFunction::HashlibSha384 => Some(HashKind::Sha384),
            BuiltinFunction::HashlibSha512 => Some(HashKind::Sha512),
            BuiltinFunction::HashlibBlake2b => Some(HashKind::Blake2b),
            BuiltinFunction::HashlibBlake2s => Some(HashKind::Blake2s),
            BuiltinFunction::HashlibSha3_224 => Some(HashKind::Sha3_224),
            BuiltinFunction::HashlibSha3_256 => Some(HashKind::Sha3_256),
            BuiltinFunction::HashlibSha3_384 => Some(HashKind::Sha3_384),
            BuiltinFunction::HashlibSha3_512 => Some(HashKind::Sha3_512),
            BuiltinFunction::HashlibShake128 => Some(HashKind::Shake128),
            BuiltinFunction::HashlibShake256 => Some(HashKind::Shake256),
            _ => None,
        }
    }

    fn unsupported_digestmod_error(&self, digestmod: &Value) -> RuntimeError {
        let detail = match digestmod {
            Value::None => "None".to_string(),
            Value::Str(name) => name.clone(),
            other => format!("<{} object>", self.value_type_name_for_error(other)),
        };
        RuntimeError::with_exception(
            "UnsupportedDigestmodError",
            Some(format!("Unsupported digestmod {detail}")),
        )
    }

    fn hash_kind_from_digestmod_value(&self, digestmod: Value) -> Result<HashKind, RuntimeError> {
        let kind = match &digestmod {
            Value::Str(name) => HashKind::from_name(name)
                .ok_or_else(|| self.unsupported_digestmod_error(&digestmod))?,
            Value::Builtin(builtin) => self
                .hash_kind_from_builtin_constructor(*builtin)
                .ok_or_else(|| self.unsupported_digestmod_error(&digestmod))?,
            _ => return Err(self.unsupported_digestmod_error(&digestmod)),
        };
        if kind.is_xof() {
            return Err(RuntimeError::value_error("no reason supplied"));
        }
        Ok(kind)
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

    fn hmac_class(&self) -> Option<ObjRef> {
        if let Some(module) = self.modules.get("_hashlib")
            && let Object::Module(module_data) = &*module.kind()
            && let Some(Value::Class(class)) = module_data.globals.get("HMAC")
        {
            return Some(class.clone());
        }
        self.synthetic_builtin_classes
            .get("__hashlib_hmac_type__")
            .cloned()
    }

    fn hmac_init_instance_attrs(&mut self, instance: &ObjRef, kind: HashKind) {
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data.attrs.insert(
                "name".to_string(),
                Value::Str(format!("hmac-{}", kind.hash_name())),
            );
            instance_data
                .attrs
                .insert("digest_size".to_string(), Value::Int(kind.digest_size()));
            instance_data
                .attrs
                .insert("block_size".to_string(), Value::Int(kind.block_size()));
        }
    }

    fn hmac_new_instance(
        &mut self,
        key: Vec<u8>,
        msg: Option<Vec<u8>>,
        digestmod: Value,
    ) -> Result<Value, RuntimeError> {
        let class = self
            .hmac_class()
            .ok_or_else(|| RuntimeError::runtime_error("HMAC backend type is unavailable"))?;
        let kind = self.hash_kind_from_digestmod_value(digestmod)?;
        let mut state = HmacState::new(kind, &key)?;
        if let Some(msg) = msg {
            state.update(&msg);
        }
        let instance = self.alloc_instance_for_class(&class);
        self.hmac_init_instance_attrs(&instance, kind);
        self.hmac_states.insert(instance.id(), state);
        Ok(Value::Instance(instance))
    }

    fn hmac_receiver_from_args<'a>(
        &'a self,
        args: &'a [Value],
        method_name: &str,
    ) -> Result<(&'a ObjRef, String), RuntimeError> {
        let Some(Value::Instance(receiver)) = args.first() else {
            return Err(RuntimeError::new(format!(
                "TypeError: {method_name}() requires a HMAC object"
            )));
        };
        if !self.hmac_states.contains_key(&receiver.id()) {
            return Err(RuntimeError::new(format!(
                "TypeError: {method_name}() requires a HMAC object"
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

    pub(in crate::vm) fn builtin_hashlib_sha1(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Sha1, args, kwargs, "sha1")
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

    pub(in crate::vm) fn builtin_hashlib_blake2b(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Blake2b, args, kwargs, "blake2b")
    }

    pub(in crate::vm) fn builtin_hashlib_blake2s(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Blake2s, args, kwargs, "blake2s")
    }

    pub(in crate::vm) fn builtin_hashlib_sha3_224(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Sha3_224, args, kwargs, "sha3_224")
    }

    pub(in crate::vm) fn builtin_hashlib_sha3_256(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Sha3_256, args, kwargs, "sha3_256")
    }

    pub(in crate::vm) fn builtin_hashlib_sha3_384(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Sha3_384, args, kwargs, "sha3_384")
    }

    pub(in crate::vm) fn builtin_hashlib_sha3_512(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Sha3_512, args, kwargs, "sha3_512")
    }

    pub(in crate::vm) fn builtin_hashlib_shake128(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Shake128, args, kwargs, "shake_128")
    }

    pub(in crate::vm) fn builtin_hashlib_shake256(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.hash_constructor(HashKind::Shake256, args, kwargs, "shake_256")
    }

    pub(in crate::vm) fn builtin_hashlib_new(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: new() missing required argument 'name' (pos 1)",
            ));
        }
        let kind = self.hash_kind_from_value(args.remove(0))?;
        self.hash_constructor(kind, args, kwargs, "new")
    }

    pub(in crate::vm) fn builtin_hashlib_pbkdf2_hmac(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() && !kwargs.contains_key("hash_name") {
            return Err(RuntimeError::new(
                "TypeError: pbkdf2_hmac() missing required argument 'hash_name' (pos 1)",
            ));
        }
        let hash_name = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs
                .remove("hash_name")
                .ok_or_else(|| RuntimeError::new("TypeError: missing hash_name"))?
        };
        if !args.is_empty() && kwargs.contains_key("hash_name") {
            return Err(RuntimeError::new(
                "TypeError: pbkdf2_hmac() got multiple values for argument 'hash_name'",
            ));
        }
        let kind = self.hash_kind_from_value(hash_name)?;
        if kind.is_xof() {
            return Err(RuntimeError::new(format!(
                "ValueError: unsupported hash type {}",
                kind.hash_name()
            )));
        }
        let password = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("password").ok_or_else(|| {
                RuntimeError::new(
                    "TypeError: pbkdf2_hmac() missing required argument 'password' (pos 2)",
                )
            })?
        };
        let salt = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("salt").ok_or_else(|| {
                RuntimeError::new(
                    "TypeError: pbkdf2_hmac() missing required argument 'salt' (pos 3)",
                )
            })?
        };
        let iterations = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("iterations").ok_or_else(|| {
                RuntimeError::new(
                    "TypeError: pbkdf2_hmac() missing required argument 'iterations' (pos 4)",
                )
            })?
        };
        let dklen = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("dklen")
        };
        if !args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: pbkdf2_hmac() takes at most 5 positional arguments",
            ));
        }
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(RuntimeError::new(format!(
                "TypeError: pbkdf2_hmac() got an unexpected keyword argument '{unexpected}'"
            )));
        }
        let password_bytes = self.hash_payload_from_value(password)?;
        let salt_bytes = self.hash_payload_from_value(salt)?;
        let rounds = value_to_int(iterations)?;
        if rounds <= 0 {
            return Err(RuntimeError::new(
                "ValueError: iteration value must be greater than 0.",
            ));
        }
        let out_len = if let Some(value) = dklen {
            let len = value_to_int(value)?;
            if len <= 0 {
                return Err(RuntimeError::new(
                    "ValueError: key length must be greater than 0.",
                ));
            }
            len as usize
        } else {
            kind.digest_size() as usize
        };
        let mut out = vec![0u8; out_len];
        match kind {
            HashKind::Md5 => {
                pbkdf2_hmac::<Md5>(&password_bytes, &salt_bytes, rounds as u32, &mut out)
            }
            HashKind::Sha1 => {
                pbkdf2_hmac::<Sha1>(&password_bytes, &salt_bytes, rounds as u32, &mut out)
            }
            HashKind::Sha224 => {
                pbkdf2_hmac::<Sha224>(&password_bytes, &salt_bytes, rounds as u32, &mut out)
            }
            HashKind::Sha256 => {
                pbkdf2_hmac::<Sha256>(&password_bytes, &salt_bytes, rounds as u32, &mut out)
            }
            HashKind::Sha384 => {
                pbkdf2_hmac::<Sha384>(&password_bytes, &salt_bytes, rounds as u32, &mut out)
            }
            HashKind::Sha512 => {
                pbkdf2_hmac::<Sha512>(&password_bytes, &salt_bytes, rounds as u32, &mut out)
            }
            HashKind::Sha3_224 => {
                pbkdf2_hmac::<Sha3_224>(&password_bytes, &salt_bytes, rounds as u32, &mut out)
            }
            HashKind::Sha3_256 => {
                pbkdf2_hmac::<Sha3_256>(&password_bytes, &salt_bytes, rounds as u32, &mut out)
            }
            HashKind::Sha3_384 => {
                pbkdf2_hmac::<Sha3_384>(&password_bytes, &salt_bytes, rounds as u32, &mut out)
            }
            HashKind::Sha3_512 => {
                pbkdf2_hmac::<Sha3_512>(&password_bytes, &salt_bytes, rounds as u32, &mut out)
            }
            HashKind::Blake2b | HashKind::Blake2s | HashKind::Shake128 | HashKind::Shake256 => {
                return Err(RuntimeError::new(format!(
                    "ValueError: unsupported hash type {}",
                    kind.hash_name()
                )));
            }
        }
        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_hashlib_scrypt(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: scrypt() missing required argument 'password' (pos 1)",
            ));
        }
        if args.len() > 1 {
            return Err(RuntimeError::new(
                "TypeError: scrypt() takes exactly one positional argument",
            ));
        }
        let password = self.hash_payload_from_value(args.remove(0))?;
        let salt = kwargs
            .remove("salt")
            .ok_or_else(|| RuntimeError::new("TypeError: scrypt() missing required keyword 'salt'"))
            .and_then(|value| self.hash_payload_from_value(value))?;
        let n = kwargs
            .remove("n")
            .ok_or_else(|| RuntimeError::new("TypeError: scrypt() missing required keyword 'n'"))
            .and_then(value_to_int)?;
        let r = kwargs
            .remove("r")
            .ok_or_else(|| RuntimeError::new("TypeError: scrypt() missing required keyword 'r'"))
            .and_then(value_to_int)?;
        let p = kwargs
            .remove("p")
            .ok_or_else(|| RuntimeError::new("TypeError: scrypt() missing required keyword 'p'"))
            .and_then(value_to_int)?;
        let _maxmem = kwargs
            .remove("maxmem")
            .map(value_to_int)
            .transpose()?
            .unwrap_or(0);
        let dklen = kwargs
            .remove("dklen")
            .map(value_to_int)
            .transpose()?
            .unwrap_or(64);
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(RuntimeError::new(format!(
                "TypeError: scrypt() got an unexpected keyword argument '{unexpected}'"
            )));
        }
        if n <= 1 || (n & (n - 1)) != 0 {
            return Err(RuntimeError::new(
                "ValueError: n must be a power of 2 greater than 1.",
            ));
        }
        if r <= 0 || p <= 0 {
            return Err(RuntimeError::new(
                "ValueError: r and p must be positive integers.",
            ));
        }
        if dklen <= 0 {
            return Err(RuntimeError::new(
                "ValueError: key length must be greater than 0.",
            ));
        }
        let log_n = (n as u64).trailing_zeros() as u8;
        let params = ScryptParams::new(log_n, r as u32, p as u32, dklen as usize)
            .map_err(|err| RuntimeError::new(format!("ValueError: {err}")))?;
        let mut out = vec![0u8; dklen as usize];
        scrypt(&password, &salt, &params, &mut out)
            .map_err(|err| RuntimeError::new(format!("ValueError: {err}")))?;
        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_hashlib_hmac_new(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 3 {
            return Err(RuntimeError::new(format!(
                "TypeError: hmac_new() takes at most 3 arguments ({} given)",
                args.len()
            )));
        }
        if !args.is_empty() && kwargs.contains_key("key") {
            return Err(RuntimeError::new(
                "TypeError: argument for hmac_new() given by name ('key') and position (1)",
            ));
        }
        if args.len() > 1 && kwargs.contains_key("msg") {
            return Err(RuntimeError::new(
                "TypeError: argument for hmac_new() given by name ('msg') and position (2)",
            ));
        }
        if args.len() > 2 && kwargs.contains_key("digestmod") {
            return Err(RuntimeError::new(
                "TypeError: argument for hmac_new() given by name ('digestmod') and position (3)",
            ));
        }

        let key = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("key").ok_or_else(|| {
                RuntimeError::new("TypeError: hmac_new() missing required argument 'key' (pos 1)")
            })?
        };
        let msg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("msg")
        };
        let digestmod = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("digestmod")
        };
        if !args.is_empty() {
            return Err(RuntimeError::new(format!(
                "TypeError: hmac_new() takes at most 3 arguments ({} given)",
                args.len() + 3
            )));
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(format!(
                "TypeError: hmac_new() takes at most 3 arguments ({} given)",
                kwargs.len() + 3
            )));
        }
        let digestmod = digestmod.ok_or_else(|| {
            RuntimeError::new("TypeError: Missing required parameter 'digestmod'.")
        })?;
        let key = self.hash_payload_from_value(key)?;
        let msg = match msg {
            None | Some(Value::None) => None,
            Some(value) => Some(self.hash_payload_from_value(value)?),
        };
        self.hmac_new_instance(key, msg, digestmod)
    }

    pub(in crate::vm) fn builtin_hashlib_hmac_digest(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 3 {
            return Err(RuntimeError::new(format!(
                "TypeError: hmac_digest() takes at most 3 arguments ({} given)",
                args.len()
            )));
        }
        if !args.is_empty() && kwargs.contains_key("key") {
            return Err(RuntimeError::new(
                "TypeError: argument for hmac_digest() given by name ('key') and position (1)",
            ));
        }
        if args.len() > 1 && kwargs.contains_key("msg") {
            return Err(RuntimeError::new(
                "TypeError: argument for hmac_digest() given by name ('msg') and position (2)",
            ));
        }
        if args.len() > 2 && kwargs.contains_key("digest") {
            return Err(RuntimeError::new(
                "TypeError: argument for hmac_digest() given by name ('digest') and position (3)",
            ));
        }
        let key = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("key").ok_or_else(|| {
                RuntimeError::new(
                    "TypeError: hmac_digest() missing required argument 'key' (pos 1)",
                )
            })?
        };
        let msg = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("msg").ok_or_else(|| {
                RuntimeError::new(
                    "TypeError: hmac_digest() missing required argument 'msg' (pos 2)",
                )
            })?
        };
        let digest = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("digest").ok_or_else(|| {
                RuntimeError::new(
                    "TypeError: hmac_digest() missing required argument 'digest' (pos 3)",
                )
            })?
        };
        if !args.is_empty() {
            return Err(RuntimeError::new(format!(
                "TypeError: hmac_digest() takes at most 3 arguments ({} given)",
                args.len() + 3
            )));
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(format!(
                "TypeError: hmac_digest() takes at most 3 arguments ({} given)",
                kwargs.len() + 3
            )));
        }
        let key = self.hash_payload_from_value(key)?;
        let msg = self.hash_payload_from_value(msg)?;
        let kind = self.hash_kind_from_digestmod_value(digest)?;
        let mut state = HmacState::new(kind, &key)?;
        state.update(&msg);
        Ok(self.heap.alloc_bytes(state.digest_bytes()))
    }

    pub(in crate::vm) fn builtin_hashlib_hmac_update(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (receiver, owner_name) = self.hmac_receiver_from_args(&args, "update")?;
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
        let Some(state) = self.hmac_states.get_mut(&receiver.id()) else {
            return Err(RuntimeError::new(
                "TypeError: update() requires a HMAC object",
            ));
        };
        state.update(&payload);
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_hashlib_hmac_obj_digest(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (receiver, owner_name) = self.hmac_receiver_from_args(&args, "digest")?;
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
        let state = self
            .hmac_states
            .get(&receiver.id())
            .ok_or_else(|| RuntimeError::type_error("digest() requires a HMAC object"))?;
        Ok(self.heap.alloc_bytes(state.digest_bytes()))
    }

    pub(in crate::vm) fn builtin_hashlib_hmac_obj_hexdigest(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (receiver, owner_name) = self.hmac_receiver_from_args(&args, "hexdigest")?;
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
        let state = self
            .hmac_states
            .get(&receiver.id())
            .ok_or_else(|| RuntimeError::type_error("hexdigest() requires a HMAC object"))?;
        let digest = state.digest_bytes();
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            out.push_str(&format!("{byte:02x}"));
        }
        Ok(Value::Str(out))
    }

    pub(in crate::vm) fn builtin_hashlib_hmac_copy(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (receiver, _owner_name) = self.hmac_receiver_from_args(&args, "copy")?;
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: copy() takes no keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error("copy() takes no arguments"));
        }
        let state = self
            .hmac_states
            .get(&receiver.id())
            .ok_or_else(|| RuntimeError::type_error("copy() requires a HMAC object"))?
            .clone();
        let class = self
            .hmac_class()
            .ok_or_else(|| RuntimeError::runtime_error("HMAC backend type is unavailable"))?;
        let kind = state.kind();
        let new_instance = self.alloc_instance_for_class(&class);
        self.hmac_init_instance_attrs(&new_instance, kind);
        self.hmac_states.insert(new_instance.id(), state);
        Ok(Value::Instance(new_instance))
    }

    pub(in crate::vm) fn builtin_hashlib_hmac_repr(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (receiver, owner_name) = self.hmac_receiver_from_args(&args, "__repr__")?;
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(format!(
                "TypeError: {owner_name}.__repr__() takes no keyword arguments"
            )));
        }
        if args.len() != 1 {
            let provided = args.len().saturating_sub(1);
            return Err(RuntimeError::new(format!(
                "TypeError: {owner_name}.__repr__() takes no arguments ({provided} given)"
            )));
        }
        let state = self
            .hmac_states
            .get(&receiver.id())
            .ok_or_else(|| RuntimeError::type_error("__repr__() requires a HMAC object"))?;
        Ok(Value::Str(format!(
            "<{} HMAC object @ 0x{:x}>",
            state.kind().hash_name(),
            receiver.id()
        )))
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
        let state = self
            .hash_states
            .get(&receiver.id())
            .ok_or_else(|| RuntimeError::type_error("digest() requires a hash object"))?;
        let len = if state.kind().is_xof() {
            if args.len() != 2 {
                return Err(RuntimeError::new(format!(
                    "TypeError: {owner_name}.digest() missing required argument 'length' (pos 1)"
                )));
            }
            let requested = value_to_int(args[1].clone())?;
            if requested < 0 {
                return Err(RuntimeError::new(
                    "ValueError: digest length must be non-negative",
                ));
            }
            Some(requested as usize)
        } else {
            let provided = args.len().saturating_sub(1);
            if args.len() != 1 {
                return Err(RuntimeError::new(format!(
                    "TypeError: {owner_name}.digest() takes no arguments ({provided} given)"
                )));
            }
            None
        };
        let digest = state.digest_bytes(len)?;
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
        let state = self
            .hash_states
            .get(&receiver.id())
            .ok_or_else(|| RuntimeError::type_error("hexdigest() requires a hash object"))?;
        let len = if state.kind().is_xof() {
            if args.len() != 2 {
                return Err(RuntimeError::new(format!(
                    "TypeError: {owner_name}.hexdigest() missing required argument 'length' (pos 1)"
                )));
            }
            let requested = value_to_int(args[1].clone())?;
            if requested < 0 {
                return Err(RuntimeError::new(
                    "ValueError: digest length must be non-negative",
                ));
            }
            Some(requested as usize)
        } else {
            let provided = args.len().saturating_sub(1);
            if args.len() != 1 {
                return Err(RuntimeError::new(format!(
                    "TypeError: {owner_name}.hexdigest() takes no arguments ({provided} given)"
                )));
            }
            None
        };
        let digest = state.digest_bytes(len)?;
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
