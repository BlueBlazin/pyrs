use super::super::*;

const SSL_NID_SERVER_AUTH: i64 = 129;
const SSL_NID_CLIENT_AUTH: i64 = 130;

fn ssl_oid_record_from_oid(oid: &str) -> (i64, String, String, String) {
    match oid {
        "1.3.6.1.5.5.7.3.1" => (
            SSL_NID_SERVER_AUTH,
            "serverAuth".to_string(),
            "TLS Web Server Authentication".to_string(),
            oid.to_string(),
        ),
        "1.3.6.1.5.5.7.3.2" => (
            SSL_NID_CLIENT_AUTH,
            "clientAuth".to_string(),
            "TLS Web Client Authentication".to_string(),
            oid.to_string(),
        ),
        _ => (0, oid.to_string(), oid.to_string(), oid.to_string()),
    }
}

fn ssl_oid_record_from_name(name: &str) -> (i64, String, String, String) {
    match name {
        "serverAuth" | "TLS Web Server Authentication" => (
            SSL_NID_SERVER_AUTH,
            "serverAuth".to_string(),
            "TLS Web Server Authentication".to_string(),
            "1.3.6.1.5.5.7.3.1".to_string(),
        ),
        "clientAuth" | "TLS Web Client Authentication" => (
            SSL_NID_CLIENT_AUTH,
            "clientAuth".to_string(),
            "TLS Web Client Authentication".to_string(),
            "1.3.6.1.5.5.7.3.2".to_string(),
        ),
        _ => (0, name.to_string(), name.to_string(), name.to_string()),
    }
}

impl Vm {
    fn ssl_init_context_attrs(
        &mut self,
        instance: &ObjRef,
        protocol: i64,
        verify_mode: i64,
        check_hostname: bool,
        verify_flags: i64,
    ) {
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data
                .attrs
                .insert("protocol".to_string(), Value::Int(protocol));
            instance_data
                .attrs
                .insert("verify_mode".to_string(), Value::Int(verify_mode));
            instance_data
                .attrs
                .insert("verify_flags".to_string(), Value::Int(verify_flags));
            instance_data
                .attrs
                .insert("options".to_string(), Value::Int(0));
            instance_data
                .attrs
                .insert("check_hostname".to_string(), Value::Bool(check_hostname));
        }
    }

    pub(in crate::vm) fn builtin_ssl_txt2obj(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: txt2obj() missing required argument 'txt'",
            ));
        }
        if args.len() > 2 {
            return Err(RuntimeError::new(format!(
                "TypeError: txt2obj() takes at most 2 positional arguments ({} given)",
                args.len()
            )));
        }
        let txt = match args.remove(0) {
            Value::Str(s) => s,
            _ => {
                return Err(RuntimeError::new(
                    "TypeError: txt2obj() argument 'txt' must be str",
                ));
            }
        };
        let name_flag = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("name").unwrap_or(Value::Bool(false))
        };
        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: txt2obj() got an unexpected keyword argument '{key}'",
            )));
        }
        let from_name = is_truthy(&name_flag);
        let (nid, short_name, long_name, oid) = if from_name {
            ssl_oid_record_from_name(&txt)
        } else {
            ssl_oid_record_from_oid(&txt)
        };
        Ok(self.heap.alloc_tuple(vec![
            Value::Int(nid),
            Value::Str(short_name),
            Value::Str(long_name),
            Value::Str(oid),
        ]))
    }

    pub(in crate::vm) fn builtin_ssl_nid2obj(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: nid2obj() does not accept keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "TypeError: nid2obj() takes exactly one argument",
            ));
        }
        let nid = match args.remove(0) {
            Value::Int(v) => v,
            Value::Bool(v) => {
                if v {
                    1
                } else {
                    0
                }
            }
            _ => {
                return Err(RuntimeError::new(
                    "TypeError: nid2obj() argument must be int",
                ));
            }
        };
        let (short_name, long_name, oid) = match nid {
            SSL_NID_SERVER_AUTH => (
                "serverAuth",
                "TLS Web Server Authentication",
                "1.3.6.1.5.5.7.3.1",
            ),
            SSL_NID_CLIENT_AUTH => (
                "clientAuth",
                "TLS Web Client Authentication",
                "1.3.6.1.5.5.7.3.2",
            ),
            _ => return Err(RuntimeError::new("ValueError: unknown NID")),
        };
        Ok(self.heap.alloc_tuple(vec![
            Value::Int(nid),
            Value::Str(short_name.to_string()),
            Value::Str(long_name.to_string()),
            Value::Str(oid.to_string()),
        ]))
    }

    pub(in crate::vm) fn builtin_ssl_rand_status(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: RAND_status() takes no arguments",
            ));
        }
        Ok(Value::Bool(true))
    }

    pub(in crate::vm) fn builtin_ssl_rand_add(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: RAND_add() does not accept keyword arguments",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::new(
                "TypeError: RAND_add() takes exactly two arguments",
            ));
        }
        let _ = bytes_like_from_value(args.remove(0))
            .map_err(|_| RuntimeError::new("TypeError: a bytes-like object is required"))?;
        match args.remove(0) {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) => Ok(Value::None),
            _ => Err(RuntimeError::new(
                "TypeError: entropy argument must be a number",
            )),
        }
    }

    pub(in crate::vm) fn builtin_ssl_rand_bytes(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: RAND_bytes() does not accept keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "TypeError: RAND_bytes() takes exactly one argument",
            ));
        }
        let n = match args.remove(0) {
            Value::Int(v) => v,
            Value::Bool(v) => {
                if v {
                    1
                } else {
                    0
                }
            }
            _ => {
                return Err(RuntimeError::new(
                    "TypeError: RAND_bytes() argument must be int",
                ));
            }
        };
        if n < 0 {
            return Err(RuntimeError::new("ValueError: num must be non-negative"));
        }
        let mut out = vec![0u8; n as usize];
        for byte in &mut out {
            *byte = (self.random.next_u32() & 0xff) as u8;
        }
        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_ssl_rand_egd(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: RAND_egd() does not accept keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "TypeError: RAND_egd() takes exactly one argument",
            ));
        }
        match args.remove(0) {
            Value::Str(_) => Ok(Value::Int(0)),
            _ => Err(RuntimeError::new(
                "TypeError: RAND_egd() argument must be str",
            )),
        }
    }

    pub(in crate::vm) fn builtin_ssl_context_new(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: _SSLContext.__new__ does not accept keyword arguments",
            ));
        }
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: _SSLContext.__new__ missing cls argument",
            ));
        }
        let cls = match args.remove(0) {
            Value::Class(class) => class,
            _ => {
                return Err(RuntimeError::new(
                    "TypeError: _SSLContext.__new__ requires a class",
                ));
            }
        };
        let protocol = match args.first() {
            Some(Value::Int(v)) => *v,
            Some(Value::Bool(v)) => {
                if *v {
                    1
                } else {
                    0
                }
            }
            Some(_) => {
                return Err(RuntimeError::new(
                    "TypeError: protocol must be int",
                ));
            }
            None => 2,
        };
        let instance = self.alloc_instance_for_class(&cls);
        self.ssl_init_context_attrs(&instance, protocol, 0, false, 0);
        Ok(Value::Instance(instance))
    }

    pub(in crate::vm) fn builtin_ssl_context_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: SSLContext.__init__ does not accept keyword arguments",
            ));
        }
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: SSLContext.__init__ missing self argument",
            ));
        }
        let instance = match args.remove(0) {
            Value::Instance(obj) => obj,
            _ => return Err(RuntimeError::new("TypeError: invalid SSLContext object")),
        };
        let protocol = match args.first() {
            Some(Value::Int(v)) => *v,
            Some(Value::Bool(v)) => {
                if *v {
                    1
                } else {
                    0
                }
            }
            Some(_) => return Err(RuntimeError::new("TypeError: protocol must be int")),
            None => 2,
        };
        self.ssl_init_context_attrs(&instance, protocol, 0, false, 0);
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_ssl_create_default_context(
        &mut self,
        _args: Vec<Value>,
        _kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let module = self
            .modules
            .get("ssl")
            .ok_or_else(|| RuntimeError::new("module 'ssl' not found"))?
            .clone();
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("invalid ssl module object"));
        };
        let class = match module_data.globals.get("SSLContext") {
            Some(Value::Class(class)) => class.clone(),
            _ => return Err(RuntimeError::new("ssl.SSLContext is not available")),
        };
        let instance = self.alloc_instance_for_class(&class);
        self.ssl_init_context_attrs(
            &instance,
            16,
            2,
            true,
            0x80000 | 32,
        );
        Ok(Value::Instance(instance))
    }

    pub(in crate::vm) fn ssl_module_constants(&self) -> Vec<(&'static str, Value)> {
        vec![
            ("OPENSSL_VERSION_NUMBER", Value::Int(0x3000_0000)),
            (
                "OPENSSL_VERSION_INFO",
                self.heap
                    .alloc_tuple(vec![Value::Int(3), Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(0)]),
            ),
            (
                "OPENSSL_VERSION",
                Value::Str("OpenSSL 3.0.0 (pyrs shim)".to_string()),
            ),
            (
                "_OPENSSL_API_VERSION",
                self.heap
                    .alloc_tuple(vec![Value::Int(3), Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(0)]),
            ),
            ("_DEFAULT_CIPHERS", Value::Str("DEFAULT:@SECLEVEL=2".to_string())),
            ("HAS_SNI", Value::Bool(false)),
            ("HAS_ECDH", Value::Bool(false)),
            ("HAS_NPN", Value::Bool(false)),
            ("HAS_ALPN", Value::Bool(false)),
            ("HAS_SSLv2", Value::Bool(false)),
            ("HAS_SSLv3", Value::Bool(false)),
            ("HAS_TLSv1", Value::Bool(true)),
            ("HAS_TLSv1_1", Value::Bool(true)),
            ("HAS_TLSv1_2", Value::Bool(true)),
            ("HAS_TLSv1_3", Value::Bool(true)),
            ("HAS_PSK", Value::Bool(false)),
            ("HAS_PHA", Value::Bool(false)),
            ("PROTOCOL_SSLv23", Value::Int(2)),
            ("PROTOCOL_TLS", Value::Int(2)),
            ("PROTOCOL_TLS_CLIENT", Value::Int(16)),
            ("PROTOCOL_TLS_SERVER", Value::Int(17)),
            ("PROTOCOL_TLSv1", Value::Int(3)),
            ("PROTOCOL_TLSv1_1", Value::Int(4)),
            ("PROTOCOL_TLSv1_2", Value::Int(5)),
            ("PROTO_MINIMUM_SUPPORTED", Value::Int(-2)),
            ("PROTO_SSLv3", Value::Int(0)),
            ("PROTO_TLSv1", Value::Int(1)),
            ("PROTO_TLSv1_1", Value::Int(2)),
            ("PROTO_TLSv1_2", Value::Int(3)),
            ("PROTO_TLSv1_3", Value::Int(4)),
            ("PROTO_MAXIMUM_SUPPORTED", Value::Int(-1)),
            ("CERT_NONE", Value::Int(0)),
            ("CERT_OPTIONAL", Value::Int(1)),
            ("CERT_REQUIRED", Value::Int(2)),
            ("VERIFY_DEFAULT", Value::Int(0)),
            ("VERIFY_X509_STRICT", Value::Int(32)),
            ("VERIFY_X509_PARTIAL_CHAIN", Value::Int(0x80000)),
            ("OP_NO_SSLv2", Value::Int(0x0100_0000)),
            ("OP_NO_SSLv3", Value::Int(0x0200_0000)),
            ("OP_NO_COMPRESSION", Value::Int(0x0002_0000)),
            ("SSL_ERROR_ZERO_RETURN", Value::Int(6)),
            ("SSL_ERROR_WANT_READ", Value::Int(2)),
            ("SSL_ERROR_WANT_WRITE", Value::Int(3)),
            ("SSL_ERROR_WANT_X509_LOOKUP", Value::Int(4)),
            ("SSL_ERROR_SYSCALL", Value::Int(5)),
            ("SSL_ERROR_SSL", Value::Int(1)),
            ("SSL_ERROR_WANT_CONNECT", Value::Int(7)),
            ("SSL_ERROR_EOF", Value::Int(8)),
            ("SSL_ERROR_INVALID_ERROR_CODE", Value::Int(9)),
            ("ALERT_DESCRIPTION_CLOSE_NOTIFY", Value::Int(0)),
            ("ALERT_DESCRIPTION_UNEXPECTED_MESSAGE", Value::Int(10)),
            ("ALERT_DESCRIPTION_BAD_RECORD_MAC", Value::Int(20)),
            ("ALERT_DESCRIPTION_HANDSHAKE_FAILURE", Value::Int(40)),
            ("ALERT_DESCRIPTION_PROTOCOL_VERSION", Value::Int(70)),
            ("ALERT_DESCRIPTION_INTERNAL_ERROR", Value::Int(80)),
            ("ALERT_DESCRIPTION_UNRECOGNIZED_NAME", Value::Int(112)),
        ]
    }
}
