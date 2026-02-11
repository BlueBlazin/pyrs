use super::*;

impl Vm {
    fn builtin_module_binding(&self, builtin: BuiltinFunction) -> Option<(String, String)> {
        for (module_name, module) in &self.modules {
            let Object::Module(module_data) = &*module.kind() else {
                continue;
            };
            for (name, value) in &module_data.globals {
                if matches!(value, Value::Builtin(candidate) if *candidate == builtin) {
                    return Some((module_name.clone(), name.clone()));
                }
            }
        }
        None
    }

    pub(super) fn builtin_type_name(&self, builtin: BuiltinFunction) -> &'static str {
        match builtin {
            BuiltinFunction::Type => "type",
            BuiltinFunction::Ascii => "ascii",
            BuiltinFunction::Bool => "bool",
            BuiltinFunction::Int => "int",
            BuiltinFunction::Float => "float",
            BuiltinFunction::Str => "str",
            BuiltinFunction::List => "list",
            BuiltinFunction::Tuple => "tuple",
            BuiltinFunction::Dict => "dict",
            BuiltinFunction::CollectionsOrderedDict => "OrderedDict",
            BuiltinFunction::Set => "set",
            BuiltinFunction::FrozenSet => "frozenset",
            BuiltinFunction::Bytes => "bytes",
            BuiltinFunction::ByteArray => "bytearray",
            BuiltinFunction::MemoryView => "memoryview",
            BuiltinFunction::Complex => "complex",
            BuiltinFunction::Slice => "slice",
            BuiltinFunction::ClassMethod => "classmethod",
            BuiltinFunction::StaticMethod => "staticmethod",
            BuiltinFunction::Property => "property",
            BuiltinFunction::FunctoolsCachedProperty => "cached_property",
            BuiltinFunction::CodecsEncode => "encode",
            BuiltinFunction::CodecsDecode => "decode",
            BuiltinFunction::CodecsEscapeDecode => "escape_decode",
            BuiltinFunction::CodecsLookup => "lookup",
            BuiltinFunction::CodecsRegister => "register",
            BuiltinFunction::CollectionsDefaultDict => "defaultdict",
            _ => "builtin",
        }
    }

    pub(super) fn builtin_runtime_name(&self, builtin: BuiltinFunction) -> String {
        for (name, value) in &self.builtins {
            if matches!(value, Value::Builtin(candidate) if *candidate == builtin) {
                return name.clone();
            }
        }
        if let Some((_module_name, name)) = self.builtin_module_binding(builtin) {
            return name;
        }
        self.builtin_type_name(builtin).to_string()
    }

    pub(super) fn builtin_attribute_name(&self, builtin: BuiltinFunction) -> String {
        match builtin {
            BuiltinFunction::DictFromKeys => "fromkeys".to_string(),
            BuiltinFunction::SetReduce => "__reduce__".to_string(),
            BuiltinFunction::BytesMakeTrans | BuiltinFunction::StrMakeTrans => {
                "maketrans".to_string()
            }
            BuiltinFunction::JsonEncodeBaseString => "encode_basestring".to_string(),
            BuiltinFunction::JsonEncodeBaseStringAscii => "encode_basestring_ascii".to_string(),
            BuiltinFunction::JsonMakeEncoder => "make_encoder".to_string(),
            BuiltinFunction::JsonMakeEncoderCall => "_iterencode".to_string(),
            BuiltinFunction::JsonScannerMakeScanner => "make_scanner".to_string(),
            BuiltinFunction::JsonScannerPyMakeScanner => "py_make_scanner".to_string(),
            BuiltinFunction::JsonScannerScanOnce => "scan_once".to_string(),
            BuiltinFunction::JsonDecoderScanString => "scanstring".to_string(),
            BuiltinFunction::SreCompile => "compile".to_string(),
            BuiltinFunction::SreTemplate => "template".to_string(),
            BuiltinFunction::SreAsciiIsCased => "ascii_iscased".to_string(),
            BuiltinFunction::SreAsciiToLower => "ascii_tolower".to_string(),
            BuiltinFunction::SreUnicodeIsCased => "unicode_iscased".to_string(),
            BuiltinFunction::SreUnicodeToLower => "unicode_tolower".to_string(),
            BuiltinFunction::OperatorContains => "contains".to_string(),
            BuiltinFunction::FunctoolsReduce => "reduce".to_string(),
            _ => self.builtin_runtime_name(builtin),
        }
    }

    pub(super) fn builtin_attribute_qualname(&self, builtin: BuiltinFunction) -> String {
        match builtin {
            BuiltinFunction::DictFromKeys => "dict.fromkeys".to_string(),
            BuiltinFunction::SetReduce => "set.__reduce__".to_string(),
            BuiltinFunction::BytesMakeTrans => "bytearray.maketrans".to_string(),
            BuiltinFunction::StrMakeTrans => "str.maketrans".to_string(),
            BuiltinFunction::JsonEncodeBaseString => "_json.encode_basestring".to_string(),
            BuiltinFunction::JsonEncodeBaseStringAscii => {
                "_json.encode_basestring_ascii".to_string()
            }
            BuiltinFunction::JsonMakeEncoder => "_json.make_encoder".to_string(),
            BuiltinFunction::JsonMakeEncoderCall => "_json._iterencode".to_string(),
            BuiltinFunction::JsonScannerMakeScanner => "_json.make_scanner".to_string(),
            BuiltinFunction::JsonScannerPyMakeScanner => "json.scanner.py_make_scanner".to_string(),
            BuiltinFunction::JsonScannerScanOnce => "_json.scan_once".to_string(),
            BuiltinFunction::JsonDecoderScanString => "_json.scanstring".to_string(),
            BuiltinFunction::SreCompile => "_sre.compile".to_string(),
            BuiltinFunction::SreTemplate => "_sre.template".to_string(),
            BuiltinFunction::SreAsciiIsCased => "_sre.ascii_iscased".to_string(),
            BuiltinFunction::SreAsciiToLower => "_sre.ascii_tolower".to_string(),
            BuiltinFunction::SreUnicodeIsCased => "_sre.unicode_iscased".to_string(),
            BuiltinFunction::SreUnicodeToLower => "_sre.unicode_tolower".to_string(),
            BuiltinFunction::OperatorContains => "operator.contains".to_string(),
            BuiltinFunction::FunctoolsReduce => "reduce".to_string(),
            _ => self.builtin_attribute_name(builtin),
        }
    }

    pub(super) fn builtin_type_dict_entries(
        &self,
        builtin: BuiltinFunction,
    ) -> Vec<(Value, Value)> {
        let mut entries = Vec::new();
        if builtin == BuiltinFunction::Dict {
            entries.push((
                Value::Str("fromkeys".to_string()),
                Value::Builtin(BuiltinFunction::DictFromKeys),
            ));
        } else if builtin == BuiltinFunction::ByteArray {
            entries.push((
                Value::Str("maketrans".to_string()),
                Value::Builtin(BuiltinFunction::BytesMakeTrans),
            ));
        } else if builtin == BuiltinFunction::Type {
            let descriptor = match self.heap.alloc_module(ModuleObject::new(
                "__type_annotations_descriptor__".to_string(),
            )) {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *descriptor.kind_mut() {
                module_data.globals.insert(
                    "__get__".to_string(),
                    Value::Builtin(BuiltinFunction::TypeAnnotationsGet),
                );
            }
            entries.push((
                Value::Str("__annotations__".to_string()),
                Value::Module(descriptor),
            ));
        }
        entries
    }

    pub(super) fn load_attr_builtin(
        &self,
        builtin: BuiltinFunction,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let mut builtin_module_name = match builtin {
            BuiltinFunction::CsvReader
            | BuiltinFunction::CsvWriter
            | BuiltinFunction::CsvWriterRow
            | BuiltinFunction::CsvWriterRows
            | BuiltinFunction::CsvRegisterDialect
            | BuiltinFunction::CsvUnregisterDialect
            | BuiltinFunction::CsvGetDialect
            | BuiltinFunction::CsvListDialects
            | BuiltinFunction::CsvFieldSizeLimit
            | BuiltinFunction::CsvDialectValidate
            | BuiltinFunction::CsvReaderIter
            | BuiltinFunction::CsvReaderNext => "_csv",
            BuiltinFunction::PickleDump
            | BuiltinFunction::PickleDumps
            | BuiltinFunction::PickleLoad
            | BuiltinFunction::PickleLoads
            | BuiltinFunction::PickleModuleGetAttr
            | BuiltinFunction::PicklePicklerInit
            | BuiltinFunction::PicklePicklerDump
            | BuiltinFunction::PickleUnpicklerInit
            | BuiltinFunction::PickleUnpicklerLoad
            | BuiltinFunction::PickleBufferInit
            | BuiltinFunction::PickleBufferRaw
            | BuiltinFunction::PickleBufferRelease => "_pickle",
            BuiltinFunction::CopyregReconstructor
            | BuiltinFunction::CopyregNewObj
            | BuiltinFunction::CopyregNewObjEx => "copyreg",
            BuiltinFunction::JsonScannerMakeScanner
            | BuiltinFunction::JsonMakeEncoder
            | BuiltinFunction::JsonMakeEncoderCall
            | BuiltinFunction::JsonEncodeBaseString
            | BuiltinFunction::JsonEncodeBaseStringAscii
            | BuiltinFunction::JsonScannerScanOnce
            | BuiltinFunction::JsonDecoderScanString => "_json",
            BuiltinFunction::SreCompile
            | BuiltinFunction::SreTemplate
            | BuiltinFunction::SreAsciiIsCased
            | BuiltinFunction::SreAsciiToLower
            | BuiltinFunction::SreUnicodeIsCased
            | BuiltinFunction::SreUnicodeToLower => "_sre",
            BuiltinFunction::JsonScannerPyMakeScanner => "json.scanner",
            BuiltinFunction::OperatorContains => "operator",
            BuiltinFunction::FunctoolsReduce => "functools",
            BuiltinFunction::CollectionsDefaultDict => "collections",
            BuiltinFunction::CodecsEncode
            | BuiltinFunction::CodecsDecode
            | BuiltinFunction::CodecsEscapeDecode
            | BuiltinFunction::CodecsLookup
            | BuiltinFunction::CodecsRegister => "codecs",
            _ => "builtins",
        }
        .to_string();
        if builtin_module_name == "builtins" {
            let in_builtins = self
                .builtins
                .values()
                .any(|value| matches!(value, Value::Builtin(candidate) if *candidate == builtin));
            if !in_builtins {
                if let Some((module_name, _)) = self.builtin_module_binding(builtin) {
                    builtin_module_name = module_name;
                }
            }
        }
        match attr_name {
            "__dict__" => Ok(self
                .heap
                .alloc_dict(self.builtin_type_dict_entries(builtin))),
            "__name__" => Ok(Value::Str(self.builtin_attribute_name(builtin))),
            "__qualname__" => Ok(Value::Str(self.builtin_attribute_qualname(builtin))),
            "__module__" => Ok(Value::Str(builtin_module_name)),
            "__self__" => Ok(Value::Builtin(builtin)),
            "__flags__" => Ok(Value::Int(0)),
            "__new__" => Ok(Value::Builtin(builtin)),
            "__init__" if builtin == BuiltinFunction::Int => {
                Ok(Value::Builtin(BuiltinFunction::ObjectInit))
            }
            "__eq__" if builtin == BuiltinFunction::ObjectNew => {
                Ok(Value::Builtin(BuiltinFunction::OperatorEq))
            }
            "__ne__" if builtin == BuiltinFunction::ObjectNew => {
                Ok(Value::Builtin(BuiltinFunction::OperatorNe))
            }
            "__hash__" if builtin == BuiltinFunction::ObjectNew => {
                Ok(Value::Builtin(BuiltinFunction::Id))
            }
            "__getformat__" if builtin == BuiltinFunction::Float => {
                Ok(Value::Builtin(BuiltinFunction::Str))
            }
            "fromhex" if builtin == BuiltinFunction::Float => {
                Ok(Value::Builtin(BuiltinFunction::FloatFromHex))
            }
            "hex" if builtin == BuiltinFunction::Float => {
                Ok(Value::Builtin(BuiltinFunction::FloatHex))
            }
            "__repr__" | "__str__" | "__format__" if builtin == BuiltinFunction::Dict => {
                Ok(Value::Builtin(BuiltinFunction::DictTypeRepr))
            }
            "__repr__" | "__str__" | "__format__" if builtin == BuiltinFunction::List => {
                Ok(Value::Builtin(BuiltinFunction::ListTypeRepr))
            }
            "__repr__" | "__str__" | "__format__" if builtin == BuiltinFunction::Tuple => {
                Ok(Value::Builtin(BuiltinFunction::TupleTypeRepr))
            }
            "__repr__" | "__str__" | "__format__" if builtin == BuiltinFunction::Set => {
                Ok(Value::Builtin(BuiltinFunction::SetTypeRepr))
            }
            "__repr__" | "__str__" | "__format__" if builtin == BuiltinFunction::FrozenSet => {
                Ok(Value::Builtin(BuiltinFunction::FrozenSetTypeRepr))
            }
            "__repr__" | "__str__" | "__format__" if builtin == BuiltinFunction::Str => {
                Ok(Value::Builtin(BuiltinFunction::StrTypeRepr))
            }
            "__repr__" | "__str__" | "__format__" if builtin == BuiltinFunction::Bytes => {
                Ok(Value::Builtin(BuiltinFunction::BytesTypeRepr))
            }
            "__repr__" | "__str__" | "__format__" if builtin == BuiltinFunction::ByteArray => {
                Ok(Value::Builtin(BuiltinFunction::ByteArrayTypeRepr))
            }
            "__repr__" | "__str__" | "__format__"
                if builtin == BuiltinFunction::TypesMappingProxy =>
            {
                Ok(Value::Builtin(BuiltinFunction::MappingProxyTypeRepr))
            }
            "__repr__" | "__str__" | "__format__"
                if builtin == BuiltinFunction::CollectionsDefaultDict =>
            {
                Ok(Value::Builtin(
                    BuiltinFunction::CollectionsDefaultDictTypeRepr,
                ))
            }
            "__repr__" | "__str__" | "__format__"
                if builtin == BuiltinFunction::CollectionsOrderedDict =>
            {
                Ok(Value::Builtin(
                    BuiltinFunction::CollectionsOrderedDictTypeRepr,
                ))
            }
            "__repr__" | "__str__" | "__format__"
                if builtin == BuiltinFunction::CollectionsCounter =>
            {
                Ok(Value::Builtin(BuiltinFunction::CollectionsCounterTypeRepr))
            }
            "__repr__" | "__str__" | "__format__"
                if builtin == BuiltinFunction::CollectionsDeque =>
            {
                Ok(Value::Builtin(BuiltinFunction::CollectionsDequeTypeRepr))
            }
            "__repr__" | "__str__" | "__format__" => Ok(Value::Builtin(BuiltinFunction::Repr)),
            "__reduce_ex__" | "__reduce__" => {
                Ok(self.alloc_reduce_ex_bound_method(Value::Builtin(builtin)))
            }
            "bit_length" if builtin == BuiltinFunction::Int => {
                Ok(Value::Builtin(BuiltinFunction::IntBitLength))
            }
            "__add__" if builtin == BuiltinFunction::Int => {
                Ok(Value::Builtin(BuiltinFunction::OperatorAdd))
            }
            "from_bytes" if builtin == BuiltinFunction::Int => {
                Ok(Value::Builtin(BuiltinFunction::IntFromBytes))
            }
            "append" if builtin == BuiltinFunction::List => {
                Ok(Value::Builtin(BuiltinFunction::ListAppendDescriptor))
            }
            "__len__" if builtin == BuiltinFunction::List => {
                Ok(Value::Builtin(BuiltinFunction::Len))
            }
            "maketrans" if builtin == BuiltinFunction::Bytes => Ok(self
                .alloc_builtin_unbound_method(
                    "__bytes_unbound_method__",
                    Value::Builtin(BuiltinFunction::Bytes),
                    BuiltinFunction::BytesMakeTrans,
                )),
            "maketrans" if builtin == BuiltinFunction::ByteArray => Ok(self
                .alloc_builtin_unbound_method(
                    "__bytearray_unbound_method__",
                    Value::Builtin(BuiltinFunction::ByteArray),
                    BuiltinFunction::BytesMakeTrans,
                )),
            "fromkeys" if builtin == BuiltinFunction::Dict => Ok(self
                .alloc_builtin_unbound_method(
                    "__dict_unbound_method__",
                    Value::Builtin(BuiltinFunction::Dict),
                    BuiltinFunction::DictFromKeys,
                )),
            "__contains__"
                if matches!(builtin, BuiltinFunction::Set | BuiltinFunction::FrozenSet) =>
            {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__set_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(builtin));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetContains, receiver))
            }
            "maketrans" if builtin == BuiltinFunction::Str => Ok(self
                .alloc_builtin_unbound_method(
                    "__str_unbound_method__",
                    Value::Builtin(BuiltinFunction::Str),
                    BuiltinFunction::StrMakeTrans,
                )),
            "count" if builtin == BuiltinFunction::Tuple => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__tuple_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Tuple));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::TupleCount, receiver))
            }
            "count" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrCount, receiver))
            }
            "index" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrIndex, receiver))
            }
            _ => Err(RuntimeError::new(format!(
                "builtin has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn load_attr_class_builtin_base_method(
        &self,
        class: &ObjRef,
        attr_name: &str,
    ) -> Option<Value> {
        if self.class_has_builtin_tuple_base(class) && attr_name == "count" {
            let receiver = match self
                .heap
                .alloc_module(ModuleObject::new("__tuple_unbound_method__".to_string()))
            {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                module_data
                    .globals
                    .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Tuple));
            }
            return Some(self.alloc_native_bound_method(NativeMethodKind::TupleCount, receiver));
        }
        if self.class_has_builtin_str_base(class) && attr_name == "count" {
            let receiver = match self
                .heap
                .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
            {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                module_data
                    .globals
                    .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
            }
            return Some(self.alloc_native_bound_method(NativeMethodKind::StrCount, receiver));
        }
        None
    }

    pub(super) fn load_attr_list_method(
        &self,
        list: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if attr_name == "__len__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Len, list));
        }
        let kind = match attr_name {
            "append" => NativeMethodKind::ListAppend,
            "extend" => NativeMethodKind::ListExtend,
            "insert" => NativeMethodKind::ListInsert,
            "remove" => NativeMethodKind::ListRemove,
            "pop" => NativeMethodKind::ListPop,
            "count" => NativeMethodKind::ListCount,
            "index" => NativeMethodKind::ListIndex,
            "reverse" => NativeMethodKind::ListReverse,
            "sort" => NativeMethodKind::ListSort,
            _ => {
                return Err(RuntimeError::new(format!(
                    "list has no attribute '{}'",
                    attr_name
                )));
            }
        };
        Ok(self.alloc_native_bound_method(kind, list))
    }

    pub(super) fn load_attr_tuple_method(
        &self,
        tuple: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if attr_name == "__len__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Len, tuple));
        }
        let kind = match attr_name {
            "count" => NativeMethodKind::TupleCount,
            _ => {
                return Err(RuntimeError::new(format!(
                    "tuple has no attribute '{}'",
                    attr_name
                )));
            }
        };
        Ok(self.alloc_native_bound_method(kind, tuple))
    }

    pub(super) fn load_attr_int_method(
        &self,
        value: Value,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if attr_name == "__new__" {
            return Ok(Value::Builtin(BuiltinFunction::ObjectNew));
        }
        let kind = match attr_name {
            "to_bytes" => NativeMethodKind::IntToBytes,
            "bit_length" => NativeMethodKind::IntBitLengthMethod,
            _ => {
                return Err(RuntimeError::new(format!(
                    "int has no attribute '{}'",
                    attr_name
                )));
            }
        };
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__int_method__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data.globals.insert("value".to_string(), value);
        }
        Ok(self.alloc_native_bound_method(kind, receiver))
    }

    pub(super) fn load_attr_str_method(
        &self,
        text: String,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let kind = match attr_name {
            "startswith" => NativeMethodKind::StrStartsWith,
            "endswith" => NativeMethodKind::StrEndsWith,
            "replace" => NativeMethodKind::StrReplace,
            "upper" => NativeMethodKind::StrUpper,
            "lower" => NativeMethodKind::StrLower,
            "capitalize" => NativeMethodKind::StrCapitalize,
            "encode" => NativeMethodKind::StrEncode,
            "decode" => NativeMethodKind::StrDecode,
            "removeprefix" => NativeMethodKind::StrRemovePrefix,
            "removesuffix" => NativeMethodKind::StrRemoveSuffix,
            "format" => NativeMethodKind::StrFormat,
            "isupper" => NativeMethodKind::StrIsUpper,
            "islower" => NativeMethodKind::StrIsLower,
            "isascii" => NativeMethodKind::StrIsAscii,
            "isalnum" => NativeMethodKind::StrIsAlNum,
            "isdigit" => NativeMethodKind::StrIsDigit,
            "isspace" => NativeMethodKind::StrIsSpace,
            "isidentifier" => NativeMethodKind::StrIsIdentifier,
            "join" => NativeMethodKind::StrJoin,
            "split" => NativeMethodKind::StrSplit,
            "splitlines" => NativeMethodKind::StrSplitLines,
            "rsplit" => NativeMethodKind::StrRSplit,
            "partition" => NativeMethodKind::StrPartition,
            "rpartition" => NativeMethodKind::StrRPartition,
            "count" => NativeMethodKind::StrCount,
            "find" => NativeMethodKind::StrFind,
            "translate" => NativeMethodKind::StrTranslate,
            "index" => NativeMethodKind::StrIndex,
            "rfind" => NativeMethodKind::StrRFind,
            "lstrip" => NativeMethodKind::StrLStrip,
            "rstrip" => NativeMethodKind::StrRStrip,
            "strip" => NativeMethodKind::StrStrip,
            "expandtabs" => NativeMethodKind::StrExpandTabs,
            _ => {
                return Err(RuntimeError::new(format!(
                    "str has no attribute '{}'",
                    attr_name
                )));
            }
        };
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__str_method__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data
                .globals
                .insert("value".to_string(), Value::Str(text));
        }
        Ok(self.alloc_native_bound_method(kind, receiver))
    }

    pub(super) fn load_attr_bytes_method(
        &self,
        receiver_value: Value,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let kind = match attr_name {
            "decode" => NativeMethodKind::BytesDecode,
            "startswith" => NativeMethodKind::BytesStartsWith,
            "endswith" => NativeMethodKind::BytesEndsWith,
            "find" => NativeMethodKind::BytesFind,
            "translate" => NativeMethodKind::BytesTranslate,
            "join" => NativeMethodKind::BytesJoin,
            _ => {
                return Err(RuntimeError::new(format!(
                    "bytes has no attribute '{}'",
                    attr_name
                )));
            }
        };
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__bytes_method__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            match receiver_value {
                Value::Bytes(_) | Value::ByteArray(_) => {
                    module_data
                        .globals
                        .insert("value".to_string(), receiver_value);
                }
                _ => {
                    return Err(RuntimeError::new("bytes receiver is invalid"));
                }
            }
        }
        Ok(self.alloc_native_bound_method(kind, receiver))
    }

    pub(super) fn load_attr_iterator(
        &self,
        iterator: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let (type_name, range_start, range_stop, range_step, allow_reduce) = match &*iterator.kind()
        {
            Object::Iterator(state) => match &state.kind {
                IteratorKind::RangeObject { start, stop, step } => (
                    "range",
                    Some(start.clone()),
                    Some(stop.clone()),
                    Some(step.clone()),
                    true,
                ),
                IteratorKind::Map { .. } => ("map", None, None, None, true),
                IteratorKind::Range { .. } => ("range_iterator", None, None, None, false),
                IteratorKind::List(_) => ("list_iterator", None, None, None, false),
                IteratorKind::Tuple(_) => ("tuple_iterator", None, None, None, false),
                IteratorKind::Str(_) => ("str_iterator", None, None, None, false),
                IteratorKind::Dict(_) => ("dict_keyiterator", None, None, None, false),
                IteratorKind::Set(_) => ("set_iterator", None, None, None, false),
                IteratorKind::Bytes(_) => ("bytes_iterator", None, None, None, false),
                IteratorKind::ByteArray(_) => ("bytearray_iterator", None, None, None, false),
                IteratorKind::MemoryView(_) => ("memoryview_iterator", None, None, None, false),
                IteratorKind::Count { .. } => ("count", None, None, None, false),
                IteratorKind::SequenceGetItem { .. } => ("iterator", None, None, None, false),
            },
            _ => return Err(RuntimeError::new("attribute access unsupported type")),
        };
        match attr_name {
            "__reduce_ex__" | "__reduce__" if allow_reduce => {
                Ok(self.alloc_reduce_ex_bound_method(Value::Iterator(iterator)))
            }
            "start" if range_start.is_some() => Ok(value_from_bigint(
                range_start.expect("range start is present"),
            )),
            "stop" if range_stop.is_some() => Ok(value_from_bigint(
                range_stop.expect("range stop is present"),
            )),
            "step" if range_step.is_some() => Ok(value_from_bigint(
                range_step.expect("range step is present"),
            )),
            _ => Err(RuntimeError::new(format!(
                "{type_name} has no attribute '{attr_name}'"
            ))),
        }
    }

    pub(super) fn load_attr_memoryview(
        &self,
        view: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        match attr_name {
            "__enter__" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewEnter, view))
            }
            "__exit__" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewExit, view))
            }
            "toreadonly" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewToReadOnly, view))
            }
            "cast" => Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewCast, view)),
            "tolist" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewToList, view))
            }
            "release" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewRelease, view))
            }
            "tobytes" => Ok(self.alloc_builtin_bound_method(BuiltinFunction::Bytes, view)),
            "contiguous" | "c_contiguous" | "f_contiguous" => Ok(Value::Bool(true)),
            "readonly" => match &*view.kind() {
                Object::MemoryView(view_data) => bytes_like_source_is_readonly(&view_data.source)
                    .map(Value::Bool)
                    .ok_or_else(|| RuntimeError::new("memoryview receiver is invalid")),
                _ => Err(RuntimeError::new("memoryview receiver is invalid")),
            },
            "obj" => match &*view.kind() {
                Object::MemoryView(view_data) => match &*view_data.source.kind() {
                    Object::Bytes(_) => Ok(Value::Bytes(view_data.source.clone())),
                    Object::ByteArray(_) => Ok(Value::ByteArray(view_data.source.clone())),
                    Object::Instance(_) => Ok(Value::Instance(view_data.source.clone())),
                    _ => Err(RuntimeError::new("memoryview receiver is invalid")),
                },
                _ => Err(RuntimeError::new("memoryview receiver is invalid")),
            },
            "itemsize" => match &*view.kind() {
                Object::MemoryView(view_data) => Ok(Value::Int(view_data.itemsize as i64)),
                _ => Err(RuntimeError::new("memoryview receiver is invalid")),
            },
            _ => Err(RuntimeError::new(format!(
                "memoryview has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn load_attr_set_method(
        &self,
        set: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let (type_name, is_frozenset) = match &*set.kind() {
            Object::Set(_) => ("set", false),
            Object::FrozenSet(_) => ("frozenset", true),
            _ => return Err(RuntimeError::new("attribute access unsupported type")),
        };
        match attr_name {
            "__reduce__" => {
                Ok(self.alloc_builtin_bound_method(BuiltinFunction::SetReduce, set.clone()))
            }
            "__reduce_ex__" => match &*set.kind() {
                Object::Set(_) => Ok(self.alloc_reduce_ex_bound_method(Value::Set(set.clone()))),
                Object::FrozenSet(_) => {
                    Ok(self.alloc_reduce_ex_bound_method(Value::FrozenSet(set.clone())))
                }
                _ => Err(RuntimeError::new("attribute access unsupported type")),
            },
            "__contains__" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetContains, set))
            }
            "issuperset" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetIsSuperset, set))
            }
            "issubset" => Ok(self.alloc_native_bound_method(NativeMethodKind::SetIsSubset, set)),
            "isdisjoint" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetIsDisjoint, set))
            }
            "union" => Ok(self.alloc_native_bound_method(NativeMethodKind::SetUnion, set)),
            "intersection" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetIntersection, set))
            }
            "difference" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetDifference, set))
            }
            "add" if !is_frozenset => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetAdd, set))
            }
            "discard" if !is_frozenset => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetDiscard, set))
            }
            "update" if !is_frozenset => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetUpdate, set))
            }
            _ => Err(RuntimeError::new(format!(
                "{type_name} has no attribute '{attr_name}'"
            ))),
        }
    }

    pub(super) fn load_attr_dict_method(
        &self,
        dict: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if attr_name == "__contains__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::OperatorContains, dict));
        }
        if attr_name == "__reduce_ex__" || attr_name == "__reduce__" {
            return Ok(self.alloc_reduce_ex_bound_method(Value::Dict(dict)));
        }
        let kind = match attr_name {
            "keys" => NativeMethodKind::DictKeys,
            "values" => NativeMethodKind::DictValues,
            "items" => NativeMethodKind::DictItems,
            "clear" => NativeMethodKind::DictClear,
            "copy" => NativeMethodKind::DictCopy,
            "update" => NativeMethodKind::DictUpdateMethod,
            "setdefault" => NativeMethodKind::DictSetDefault,
            "get" => NativeMethodKind::DictGet,
            "__getitem__" => NativeMethodKind::DictGetItem,
            "pop" => NativeMethodKind::DictPop,
            _ => {
                return Err(RuntimeError::new(format!(
                    "dict has no attribute '{}'",
                    attr_name
                )));
            }
        };
        Ok(self.alloc_native_bound_method(kind, dict))
    }

    pub(super) fn dict_lookup_str_key(
        &self,
        dict: &ObjRef,
        key: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let string_key = Value::Str(key.to_string());
        if !matches!(&*dict.kind(), Object::Dict(_)) {
            return Err(RuntimeError::new("function attribute dict is invalid"));
        }
        Ok(dict_get_value(dict, &string_key))
    }

    pub(super) fn dict_set_str_key(
        &self,
        dict: &ObjRef,
        key: String,
        value: Value,
    ) -> Result<(), RuntimeError> {
        if !matches!(&*dict.kind(), Object::Dict(_)) {
            return Err(RuntimeError::new("function attribute dict is invalid"));
        }
        dict_set_value(dict, Value::Str(key), value);
        Ok(())
    }

    pub(super) fn dict_remove_str_key(
        &self,
        dict: &ObjRef,
        key: &str,
    ) -> Result<bool, RuntimeError> {
        if !matches!(&*dict.kind(), Object::Dict(_)) {
            return Err(RuntimeError::new("function attribute dict is invalid"));
        }
        Ok(dict_remove_value(dict, &Value::Str(key.to_string())).is_some())
    }

    pub(super) fn ensure_function_annotations(
        &mut self,
        func: &ObjRef,
    ) -> Result<ObjRef, RuntimeError> {
        let mut func_ref = func.kind_mut();
        let Object::Function(func_data) = &mut *func_ref else {
            return Err(RuntimeError::new("attribute access unsupported type"));
        };
        if let Some(obj) = &func_data.annotations {
            return Ok(obj.clone());
        }
        let dict = self.heap.alloc_dict(Vec::new());
        let obj = match dict {
            Value::Dict(obj) => obj,
            _ => unreachable!(),
        };
        func_data.annotations = Some(obj.clone());
        Ok(obj)
    }

    pub(super) fn ensure_function_dict(&mut self, func: &ObjRef) -> Result<ObjRef, RuntimeError> {
        let mut func_ref = func.kind_mut();
        let Object::Function(func_data) = &mut *func_ref else {
            return Err(RuntimeError::new("attribute access unsupported type"));
        };
        if let Some(obj) = &func_data.dict {
            return Ok(obj.clone());
        }
        let dict = self.heap.alloc_dict(Vec::new());
        let obj = match dict {
            Value::Dict(obj) => obj,
            _ => unreachable!(),
        };
        func_data.dict = Some(obj.clone());
        Ok(obj)
    }

    pub(super) fn function_module_name(&self, module: &ObjRef) -> String {
        match &*module.kind() {
            Object::Module(module_data) => module_data
                .globals
                .get("__name__")
                .and_then(|value| match value {
                    Value::Str(name) => Some(name.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| module_data.name.clone()),
            _ => "__main__".to_string(),
        }
    }

    pub(super) fn load_attr_function(
        &mut self,
        func: &ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let function_dict = {
            let func_ref = func.kind();
            let Object::Function(func_data) = &*func_ref else {
                return Err(RuntimeError::new("attribute access unsupported type"));
            };
            func_data.dict.clone()
        };
        if let Some(dict) = &function_dict {
            if let Some(value) = self.dict_lookup_str_key(dict, attr_name)? {
                return Ok(value);
            }
        }

        match attr_name {
            "__annotations__" => Ok(Value::Dict(self.ensure_function_annotations(func)?)),
            "__dict__" => Ok(Value::Dict(self.ensure_function_dict(func)?)),
            "__name__" => {
                let name = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::new("attribute access unsupported type"));
                    };
                    func_data.code.name.clone()
                };
                Ok(Value::Str(name))
            }
            "__qualname__" => {
                let qualname = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::new("attribute access unsupported type"));
                    };
                    let base_name = func_data.code.name.clone();
                    if let Some(owner_class) = &func_data.owner_class {
                        if let Object::Class(class_data) = &*owner_class.kind() {
                            let owner_qualname = class_data
                                .attrs
                                .get("__qualname__")
                                .and_then(|value| match value {
                                    Value::Str(name) => Some(name.clone()),
                                    _ => None,
                                })
                                .unwrap_or_else(|| class_data.name.clone());
                            format!("{owner_qualname}.{base_name}")
                        } else {
                            base_name
                        }
                    } else {
                        base_name
                    }
                };
                Ok(Value::Str(qualname))
            }
            "__module__" => {
                let module_name = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::new("attribute access unsupported type"));
                    };
                    self.function_module_name(&func_data.module)
                };
                Ok(Value::Str(module_name))
            }
            "__code__" => {
                let code = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::new("attribute access unsupported type"));
                    };
                    func_data.code.clone()
                };
                Ok(Value::Code(code))
            }
            "__globals__" => {
                let module = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::new("attribute access unsupported type"));
                    };
                    func_data.module.clone()
                };
                if let Object::Module(module_data) = &*module.kind() {
                    let entries = module_data
                        .globals
                        .iter()
                        .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                        .collect::<Vec<_>>();
                    Ok(self.heap.alloc_dict(entries))
                } else {
                    Ok(self.heap.alloc_dict(Vec::new()))
                }
            }
            "__doc__" => Ok(Value::None),
            "__call__" => Ok(Value::Function(func.clone())),
            "__defaults__" => {
                let defaults = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::new("attribute access unsupported type"));
                    };
                    func_data.defaults.clone()
                };
                if defaults.is_empty() {
                    Ok(Value::None)
                } else {
                    Ok(self.heap.alloc_tuple(defaults))
                }
            }
            "__kwdefaults__" => {
                let entries = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::new("attribute access unsupported type"));
                    };
                    if func_data.kwonly_defaults.is_empty() {
                        return Ok(Value::None);
                    }
                    func_data
                        .kwonly_defaults
                        .iter()
                        .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                        .collect::<Vec<_>>()
                };
                Ok(self.heap.alloc_dict(entries))
            }
            "__closure__" => {
                let closure = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::new("attribute access unsupported type"));
                    };
                    func_data.closure.clone()
                };
                if closure.is_empty() {
                    Ok(Value::None)
                } else {
                    let values = closure.into_iter().map(Value::Cell).collect::<Vec<_>>();
                    Ok(self.heap.alloc_tuple(values))
                }
            }
            _ => Err(RuntimeError::new(format!(
                "function has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn load_attr_bound_method(
        &mut self,
        method: &ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let (function, receiver) = {
            let method_ref = method.kind();
            let Object::BoundMethod(method_data) = &*method_ref else {
                return Err(RuntimeError::new("attribute access unsupported type"));
            };
            (method_data.function.clone(), method_data.receiver.clone())
        };
        enum BoundFunctionKind {
            Function,
            Module,
            Class,
            Unsupported,
        }
        let function_kind = {
            let function_ref = function.kind();
            match &*function_ref {
                Object::Function(_) => BoundFunctionKind::Function,
                Object::Module(_) => BoundFunctionKind::Module,
                Object::Class(_) => BoundFunctionKind::Class,
                _ => BoundFunctionKind::Unsupported,
            }
        };
        let as_value = |kind: &BoundFunctionKind, obj: &ObjRef| match kind {
            BoundFunctionKind::Function => Some(Value::Function(obj.clone())),
            BoundFunctionKind::Module => Some(Value::Module(obj.clone())),
            BoundFunctionKind::Class => Some(Value::Class(obj.clone())),
            BoundFunctionKind::Unsupported => None,
        };
        match attr_name {
            "__call__" => Ok(Value::BoundMethod(method.clone())),
            "__reduce_ex__" | "__reduce__" => {
                let wrapper = match self
                    .heap
                    .alloc_module(ModuleObject::new("__bound_method_reduce_ex__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
                    module_data
                        .globals
                        .insert("method".to_string(), Value::BoundMethod(method.clone()));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::BoundMethodReduceEx, wrapper))
            }
            "__self__" => self.receiver_value(&receiver),
            "__func__" => as_value(&function_kind, &function)
                .ok_or_else(|| RuntimeError::new("attribute access unsupported type")),
            "__name__" | "__qualname__" | "__module__" | "__doc__" => {
                let function_value = as_value(&function_kind, &function)
                    .ok_or_else(|| RuntimeError::new("attribute access unsupported type"))?;
                self.builtin_getattr(
                    vec![function_value, Value::Str(attr_name.to_string())],
                    HashMap::new(),
                )
            }
            _ => Err(RuntimeError::new(format!(
                "method has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn load_attr_exception_type(
        &self,
        exception_name: &str,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        match attr_name {
            "__name__" | "__qualname__" => Ok(Value::Str(exception_name.to_string())),
            "__module__" => {
                let module_name = if exception_name == "Error" {
                    "_csv"
                } else if matches!(
                    exception_name,
                    "PickleError" | "PicklingError" | "UnpicklingError"
                ) {
                    "_pickle"
                } else {
                    "builtins"
                };
                Ok(Value::Str(module_name.to_string()))
            }
            "__doc__" => Ok(Value::None),
            _ => Err(RuntimeError::new(format!(
                "exception type has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn load_attr_code(
        &self,
        code: &Rc<CodeObject>,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let mut varnames = Vec::new();
        varnames.extend(code.posonly_params.iter().cloned());
        varnames.extend(code.params.iter().cloned());
        varnames.extend(code.kwonly_params.iter().cloned());
        if let Some(vararg) = &code.vararg {
            varnames.push(vararg.clone());
        }
        if let Some(kwarg) = &code.kwarg {
            varnames.push(kwarg.clone());
        }

        let mut flags = 0x0001 | 0x0002;
        if code.vararg.is_some() {
            flags |= 0x0004;
        }
        if code.kwarg.is_some() {
            flags |= 0x0008;
        }
        if code.is_generator {
            flags |= 0x0020;
        }
        if code.is_coroutine {
            flags |= 0x0080;
        }
        if code.is_async_generator {
            flags |= 0x0200;
        }

        let first_line = code
            .locations
            .first()
            .map(|loc| loc.line as i64)
            .unwrap_or(1);

        match attr_name {
            "co_name" | "co_qualname" => Ok(Value::Str(code.name.clone())),
            "co_filename" => Ok(Value::Str(code.filename.clone())),
            "co_argcount" => Ok(Value::Int(
                (code.posonly_params.len() + code.params.len()) as i64,
            )),
            "co_posonlyargcount" => Ok(Value::Int(code.posonly_params.len() as i64)),
            "co_kwonlyargcount" => Ok(Value::Int(code.kwonly_params.len() as i64)),
            "co_nlocals" => Ok(Value::Int(varnames.len() as i64)),
            "co_stacksize" => Ok(Value::Int(0)),
            "co_flags" => Ok(Value::Int(flags)),
            "co_firstlineno" => Ok(Value::Int(first_line)),
            "co_consts" => Ok(self.heap.alloc_tuple(code.constants.clone())),
            "co_names" => Ok(self.heap.alloc_tuple(
                code.names
                    .iter()
                    .map(|name| Value::Str(name.clone()))
                    .collect::<Vec<_>>(),
            )),
            "co_varnames" => Ok(self
                .heap
                .alloc_tuple(varnames.into_iter().map(Value::Str).collect::<Vec<_>>())),
            "co_cellvars" => Ok(self.heap.alloc_tuple(
                code.cellvars
                    .iter()
                    .map(|name| Value::Str(name.clone()))
                    .collect::<Vec<_>>(),
            )),
            "co_freevars" => Ok(self.heap.alloc_tuple(
                code.freevars
                    .iter()
                    .map(|name| Value::Str(name.clone()))
                    .collect::<Vec<_>>(),
            )),
            "co_code" | "co_lnotab" | "co_exceptiontable" => Ok(self.heap.alloc_bytes(Vec::new())),
            _ => Err(RuntimeError::new(format!(
                "code has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn store_attr_function(
        &mut self,
        func: &ObjRef,
        attr_name: String,
        value: Value,
    ) -> Result<(), RuntimeError> {
        match attr_name.as_str() {
            "__annotations__" => {
                let annotations = match value {
                    Value::Dict(obj) => obj,
                    _ => return Err(RuntimeError::new("function __annotations__ must be dict")),
                };
                let mut func_ref = func.kind_mut();
                let Object::Function(func_data) = &mut *func_ref else {
                    return Err(RuntimeError::new("attribute assignment unsupported type"));
                };
                func_data.annotations = Some(annotations);
                Ok(())
            }
            "__dict__" => {
                let dict = match value {
                    Value::Dict(obj) => obj,
                    _ => return Err(RuntimeError::new("function __dict__ must be dict")),
                };
                let mut func_ref = func.kind_mut();
                let Object::Function(func_data) = &mut *func_ref else {
                    return Err(RuntimeError::new("attribute assignment unsupported type"));
                };
                func_data.dict = Some(dict);
                Ok(())
            }
            _ => {
                let dict = self.ensure_function_dict(func)?;
                self.dict_set_str_key(&dict, attr_name, value)
            }
        }
    }

    pub(super) fn store_attr_exception(
        &mut self,
        exception: &mut ExceptionObject,
        attr_name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        match attr_name {
            "__cause__" => match value {
                Value::None => {
                    exception.cause = None;
                    Ok(())
                }
                Value::Exception(cause) => {
                    exception.cause = Some(cause);
                    Ok(())
                }
                _ => Err(RuntimeError::new("__cause__ must be an exception or None")),
            },
            "__context__" => match value {
                Value::None => {
                    exception.context = None;
                    Ok(())
                }
                Value::Exception(context) => {
                    exception.context = Some(context);
                    Ok(())
                }
                _ => Err(RuntimeError::new(
                    "__context__ must be an exception or None",
                )),
            },
            "__suppress_context__" => match value {
                Value::Bool(flag) => {
                    exception.suppress_context = flag;
                    Ok(())
                }
                _ => Err(RuntimeError::new(
                    "__suppress_context__ must be set to bool",
                )),
            },
            "__traceback__" => {
                // Traceback objects are not modelled yet; accept writes for contextlib paths.
                Ok(())
            }
            _ => {
                exception
                    .attrs
                    .borrow_mut()
                    .insert(attr_name.to_string(), value);
                Ok(())
            }
        }
    }

    pub(super) fn delete_attr_exception(
        &mut self,
        exception: &ExceptionObject,
        attr_name: &str,
    ) -> Result<(), RuntimeError> {
        match attr_name {
            "__cause__" | "__context__" | "__suppress_context__" | "__traceback__" => Err(
                RuntimeError::new(format!("cannot delete exception attribute '{}'", attr_name)),
            ),
            _ => {
                if exception.attrs.borrow_mut().remove(attr_name).is_some() {
                    Ok(())
                } else {
                    Err(RuntimeError::new(format!(
                        "exception has no attribute '{}'",
                        attr_name
                    )))
                }
            }
        }
    }

    pub(super) fn delete_attr_function(
        &mut self,
        func: &ObjRef,
        attr_name: &str,
    ) -> Result<(), RuntimeError> {
        match attr_name {
            "__annotations__" => {
                let mut func_ref = func.kind_mut();
                let Object::Function(func_data) = &mut *func_ref else {
                    return Err(RuntimeError::new("attribute deletion unsupported type"));
                };
                if func_data.annotations.take().is_none() {
                    return Err(RuntimeError::new(format!(
                        "function attribute '{}' does not exist",
                        attr_name
                    )));
                }
                Ok(())
            }
            "__dict__" => Err(RuntimeError::new(
                "cannot delete function attribute '__dict__'",
            )),
            _ => {
                let dict = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::new("attribute deletion unsupported type"));
                    };
                    func_data.dict.clone()
                };
                let Some(dict) = dict else {
                    return Err(RuntimeError::new(format!(
                        "function attribute '{}' does not exist",
                        attr_name
                    )));
                };
                if self.dict_remove_str_key(&dict, attr_name)? {
                    Ok(())
                } else {
                    Err(RuntimeError::new(format!(
                        "function attribute '{}' does not exist",
                        attr_name
                    )))
                }
            }
        }
    }

    pub(super) fn bind_descriptor_method(
        &mut self,
        method: Value,
        receiver: &Value,
    ) -> Result<Option<Value>, RuntimeError> {
        match method {
            Value::Function(func) => {
                let receiver_ref = self.receiver_from_value(receiver)?;
                Ok(Some(
                    self.heap
                        .alloc_bound_method(BoundMethod::new(func, receiver_ref)),
                ))
            }
            Value::Builtin(builtin) => {
                let receiver_ref = self.receiver_from_value(receiver)?;
                Ok(Some(self.alloc_builtin_bound_method(builtin, receiver_ref)))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn lookup_bound_special_method(
        &mut self,
        receiver: &Value,
        method_name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some(class_ref) = self.class_of_value(receiver) else {
            return Ok(None);
        };
        let Some(method) = class_attr_lookup(&class_ref, method_name) else {
            return Ok(None);
        };
        self.bind_descriptor_method(method, receiver)
    }

    pub(super) fn descriptor_hooks(
        &mut self,
        descriptor: &Value,
    ) -> Result<(Option<Value>, Option<Value>, Option<Value>), RuntimeError> {
        if matches!(descriptor, Value::Function(_)) {
            return Ok((None, None, None));
        }

        if let Value::Instance(instance) = descriptor {
            if self.property_descriptor_parts(instance).is_some() {
                return Ok((
                    Some(self.alloc_native_bound_method(
                        NativeMethodKind::PropertyGet,
                        instance.clone(),
                    )),
                    Some(self.alloc_native_bound_method(
                        NativeMethodKind::PropertySet,
                        instance.clone(),
                    )),
                    Some(self.alloc_native_bound_method(
                        NativeMethodKind::PropertyDelete,
                        instance.clone(),
                    )),
                ));
            }
            if self.cached_property_descriptor_parts(instance).is_some() {
                return Ok((
                    Some(self.alloc_native_bound_method(
                        NativeMethodKind::CachedPropertyGet,
                        instance.clone(),
                    )),
                    None,
                    None,
                ));
            }
        }

        let Some(class_ref) = self.class_of_value(descriptor) else {
            return Ok((None, None, None));
        };
        let get = class_attr_lookup(&class_ref, "__get__")
            .map(|method| self.bind_descriptor_method(method, descriptor))
            .transpose()?
            .flatten();
        let set = class_attr_lookup(&class_ref, "__set__")
            .map(|method| self.bind_descriptor_method(method, descriptor))
            .transpose()?
            .flatten();
        let delete = class_attr_lookup(&class_ref, "__delete__")
            .map(|method| self.bind_descriptor_method(method, descriptor))
            .transpose()?
            .flatten();
        Ok((get, set, delete))
    }

    pub(super) fn unwrap_staticmethod_attr(&self, value: &Value) -> Option<Value> {
        let Value::Module(module) = value else {
            return None;
        };
        let Object::Module(module_data) = &*module.kind() else {
            return None;
        };
        if module_data.name != "__staticmethod__" {
            return None;
        }
        module_data.globals.get("__func__").cloned()
    }

    pub(super) fn unwrap_classmethod_attr(&self, value: &Value) -> Option<Value> {
        let Value::Module(module) = value else {
            return None;
        };
        let Object::Module(module_data) = &*module.kind() else {
            return None;
        };
        if module_data.name != "__classmethod__" {
            return None;
        }
        module_data.globals.get("__func__").cloned()
    }

    pub(super) fn bind_classmethod_attr(
        &self,
        owner_class: &ObjRef,
        value: &Value,
    ) -> Option<Value> {
        let unwrapped = self.unwrap_classmethod_attr(value)?;
        match unwrapped {
            Value::Function(func) => Some(
                self.heap
                    .alloc_bound_method(BoundMethod::new(func, owner_class.clone())),
            ),
            Value::Builtin(builtin) => {
                Some(self.alloc_builtin_bound_method(builtin, owner_class.clone()))
            }
            _ => Some(unwrapped),
        }
    }

    pub(super) fn resolve_metaclass_call_target(
        &mut self,
        class: &ObjRef,
    ) -> Result<Option<Value>, RuntimeError> {
        let metaclass = match &*class.kind() {
            Object::Class(class_data) => class_data.metaclass.clone(),
            _ => None,
        };
        let Some(metaclass) = metaclass else {
            return Ok(None);
        };
        let Some(attr) = class_attr_lookup(&metaclass, "__call__") else {
            return Ok(None);
        };
        if let Some(bound) = self.bind_classmethod_attr(class, &attr) {
            return Ok(Some(bound));
        }
        if let Some(unwrapped) = self.unwrap_staticmethod_attr(&attr) {
            return Ok(Some(unwrapped));
        }
        if let Value::Function(func) = attr.clone() {
            let bound = BoundMethod::new(func, class.clone());
            return Ok(Some(self.heap.alloc_bound_method(bound)));
        }
        let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
        if let Some(getter) = getter {
            return Ok(
                match self.call_internal(
                    getter,
                    vec![Value::Class(class.clone()), Value::Class(metaclass)],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(value) => Some(value),
                    InternalCallOutcome::CallerExceptionHandled => None,
                },
            );
        }
        Ok(Some(attr))
    }

    pub(super) fn caller_exception_handled(&self, caller_depth: usize, caller_ip: usize) -> bool {
        if self.frames.len() < caller_depth {
            return true;
        }
        self.frames
            .get(caller_depth.saturating_sub(1))
            .map(|frame| frame.ip != caller_ip)
            .unwrap_or(false)
    }

    pub(super) fn finalize_builtin_opcode_call(
        &mut self,
        caller_depth: usize,
        caller_ip: usize,
        result: Result<Value, RuntimeError>,
    ) -> Result<(), RuntimeError> {
        match result {
            Ok(value) => {
                if !self.caller_exception_handled(caller_depth, caller_ip) {
                    let caller_idx = caller_depth.saturating_sub(1);
                    self.push_value_to_caller_frame(caller_idx, value)?;
                }
                Ok(())
            }
            Err(err) => {
                if self.caller_exception_handled(caller_depth, caller_ip) {
                    Ok(())
                } else {
                    Err(err)
                }
            }
        }
    }

    pub(super) fn finalize_native_opcode_call(
        &mut self,
        caller_depth: usize,
        caller_ip: usize,
        result: Result<NativeCallResult, RuntimeError>,
    ) -> Result<(), RuntimeError> {
        match result {
            Ok(NativeCallResult::Value(value)) => {
                if !self.caller_exception_handled(caller_depth, caller_ip) {
                    let caller_idx = caller_depth.saturating_sub(1);
                    self.push_value_to_caller_frame(caller_idx, value)?;
                }
                Ok(())
            }
            Ok(NativeCallResult::PropagatedException) => {
                self.propagate_pending_generator_exception()?;
                Ok(())
            }
            Err(err) => {
                if self.caller_exception_handled(caller_depth, caller_ip) {
                    Ok(())
                } else {
                    Err(err)
                }
            }
        }
    }

    pub(super) fn call_internal(
        &mut self,
        callable: Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<InternalCallOutcome, RuntimeError> {
        let caller_depth = self.frames.len();
        if caller_depth == 0 {
            return Err(RuntimeError::new(
                "internal call requires an active execution frame",
            ));
        }
        if caller_depth as i64 >= self.recursion_limit {
            return Err(RuntimeError::new("maximum recursion depth exceeded"));
        }
        let caller_ip = self.frames.last().map(|frame| frame.ip).unwrap_or(0);

        let needs_run = match callable {
            Value::Function(func) => {
                let depth_before = self.frames.len();
                if kwargs.is_empty() {
                    let mut args = args;
                    match args.len() {
                        0 => self.push_function_call_from_obj(&func, Vec::new(), HashMap::new())?,
                        1 => {
                            let arg0 = args.pop().expect("len checked");
                            self.push_function_call_one_arg_from_obj(&func, arg0)?
                        }
                        2 => {
                            let arg1 = args.pop().expect("len checked");
                            let arg0 = args.pop().expect("len checked");
                            self.push_function_call_two_args_from_obj(&func, arg0, arg1)?
                        }
                        3 => {
                            let arg2 = args.pop().expect("len checked");
                            let arg1 = args.pop().expect("len checked");
                            let arg0 = args.pop().expect("len checked");
                            self.push_function_call_three_args_from_obj(&func, arg0, arg1, arg2)?
                        }
                        _ => {
                            self.push_function_call_from_obj(&func, args, HashMap::new())?;
                        }
                    }
                } else {
                    self.push_function_call_from_obj(&func, args, kwargs)?;
                }
                self.frames.len() > depth_before
            }
            Value::BoundMethod(method) => {
                let method_data = match &*method.kind() {
                    Object::BoundMethod(data) => data.clone(),
                    _ => return Err(RuntimeError::new("attempted to call non-function")),
                };
                match &*method_data.function.kind() {
                    Object::Function(_) => {
                        let depth_before = self.frames.len();
                        if kwargs.is_empty() {
                            let mut args = args;
                            match args.len() {
                                0 => self.push_bound_method_call_zero_args_from_obj(&method)?,
                                1 => {
                                    let arg0 = args.pop().expect("len checked");
                                    self.push_bound_method_call_one_arg_from_obj(&method, arg0)?
                                }
                                2 => {
                                    let arg1 = args.pop().expect("len checked");
                                    let arg0 = args.pop().expect("len checked");
                                    self.push_bound_method_call_two_args_from_obj(
                                        &method, arg0, arg1,
                                    )?
                                }
                                3 => {
                                    let arg2 = args.pop().expect("len checked");
                                    let arg1 = args.pop().expect("len checked");
                                    let arg0 = args.pop().expect("len checked");
                                    self.push_bound_method_call_three_args_from_obj(
                                        &method, arg0, arg1, arg2,
                                    )?
                                }
                                _ => {
                                    let mut bound_args = Vec::with_capacity(args.len() + 1);
                                    bound_args.push(self.receiver_value(&method_data.receiver)?);
                                    bound_args.extend(args);
                                    self.push_function_call_from_obj(
                                        &method_data.function,
                                        bound_args,
                                        HashMap::new(),
                                    )?;
                                }
                            }
                        } else {
                            let mut bound_args = Vec::with_capacity(args.len() + 1);
                            bound_args.push(self.receiver_value(&method_data.receiver)?);
                            bound_args.extend(args);
                            self.push_function_call_from_obj(
                                &method_data.function,
                                bound_args,
                                kwargs,
                            )?;
                        }
                        self.frames.len() > depth_before
                    }
                    Object::NativeMethod(native) => {
                        let native_call = self.call_native_method(
                            native.kind,
                            method_data.receiver.clone(),
                            args,
                            kwargs,
                        );
                        match native_call {
                            Ok(NativeCallResult::Value(result)) => {
                                return Ok(InternalCallOutcome::Value(result));
                            }
                            Ok(NativeCallResult::PropagatedException) => {
                                self.propagate_pending_generator_exception()?;
                                return Ok(InternalCallOutcome::CallerExceptionHandled);
                            }
                            Err(err) => {
                                if self.caller_exception_handled(caller_depth, caller_ip) {
                                    return Ok(InternalCallOutcome::CallerExceptionHandled);
                                }
                                return Err(err);
                            }
                        }
                    }
                    _ => return Err(RuntimeError::new("attempted to call non-function")),
                }
            }
            Value::Builtin(builtin) => {
                return match self.call_builtin(builtin, args, kwargs) {
                    Ok(result) => {
                        if self.caller_exception_handled(caller_depth, caller_ip) {
                            Ok(InternalCallOutcome::CallerExceptionHandled)
                        } else {
                            Ok(InternalCallOutcome::Value(result))
                        }
                    }
                    Err(err) => {
                        if self.caller_exception_handled(caller_depth, caller_ip) {
                            Ok(InternalCallOutcome::CallerExceptionHandled)
                        } else {
                            Err(err)
                        }
                    }
                };
            }
            Value::Instance(instance) => {
                let receiver = Value::Instance(instance.clone());
                let call_target = self
                    .lookup_bound_special_method(&receiver, "__call__")?
                    .ok_or_else(|| RuntimeError::new("attempted to call non-function"))?;
                return self.call_internal(call_target, args, kwargs);
            }
            Value::Class(class) => {
                if let Some(call_target) = self.resolve_metaclass_call_target(&class)? {
                    return self.call_internal(call_target, args, kwargs);
                }
                if let Some(message) = self.class_disallow_instantiation_message(&class) {
                    return Err(RuntimeError::new(message));
                }
                if self.class_has_builtin_type_base(&class) {
                    let class_value =
                        self.instantiate_type_derived_class(class.clone(), args, kwargs)?;
                    self.push_value(class_value);
                    false
                } else {
                    let class_value = Value::Class(class.clone());
                    let mut instance = self.alloc_instance_for_class(&class);
                    let mut used_custom_new = false;
                    if let Some(new_callable) =
                        class_attr_lookup(&class, "__new__").filter(|callable| {
                            !matches!(callable, Value::Builtin(BuiltinFunction::ObjectNew))
                        })
                    {
                        used_custom_new = true;
                        let mut new_args = Vec::with_capacity(args.len() + 1);
                        new_args.push(class_value.clone());
                        new_args.extend(args.clone());
                        match self.call_internal(new_callable, new_args, kwargs.clone())? {
                            InternalCallOutcome::Value(value) => {
                                if !self.value_is_instance_of(&value, &class_value)? {
                                    return Ok(InternalCallOutcome::Value(value));
                                }
                                let Value::Instance(created_instance) = value else {
                                    return Ok(InternalCallOutcome::Value(value));
                                };
                                instance = created_instance;
                            }
                            InternalCallOutcome::CallerExceptionHandled => {
                                return Ok(InternalCallOutcome::CallerExceptionHandled);
                            }
                        }
                    }
                    let init = class_attr_lookup(&class, "__init__");
                    if let Some(init_callable) = init {
                        if let Value::Function(init_func) = init_callable {
                            let func_data = match &*init_func.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ));
                                }
                            };
                            let mut init_args = Vec::with_capacity(args.len() + 1);
                            init_args.push(Value::Instance(instance.clone()));
                            init_args.extend(args);
                            let bindings =
                                bind_arguments(&func_data, &self.heap, init_args, kwargs)?;
                            let cells =
                                self.build_cells(&func_data.code, func_data.closure.clone());
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                                cells,
                                func_data.owner_class.clone(),
                            );
                            frame.active_exception = self
                                .frames
                                .last()
                                .and_then(|caller| caller.active_exception.clone());
                            frame.return_instance = Some(instance);
                            frame.expect_none_return = true;
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            let depth_before = self.frames.len();
                            self.frames.push(Box::new(frame));
                            self.frames.len() > depth_before
                        } else {
                            let mut init_args = Vec::with_capacity(args.len() + 1);
                            init_args.push(Value::Instance(instance.clone()));
                            init_args.extend(args);
                            match self.call_internal(init_callable, init_args, kwargs)? {
                                InternalCallOutcome::Value(Value::None) => {
                                    self.push_value(Value::Instance(instance));
                                    false
                                }
                                InternalCallOutcome::Value(_) => {
                                    return Err(RuntimeError::new("__init__() should return None"));
                                }
                                InternalCallOutcome::CallerExceptionHandled => false,
                            }
                        }
                    } else {
                        if !used_custom_new {
                            if let Some(fields) = self.class_namedtuple_fields(&class) {
                                let mut bound_values: Vec<Option<Value>> = vec![None; fields.len()];
                                if args.len() > fields.len() {
                                    return Err(RuntimeError::new(
                                        "namedtuple() argument count mismatch",
                                    ));
                                }
                                for (index, value) in args.into_iter().enumerate() {
                                    bound_values[index] = Some(value);
                                }
                                for (key, value) in kwargs {
                                    let Some(index) = fields.iter().position(|name| name == &key)
                                    else {
                                        return Err(RuntimeError::new(
                                            "namedtuple() got unexpected keyword argument",
                                        ));
                                    };
                                    if bound_values[index].is_some() {
                                        return Err(RuntimeError::new(
                                            "namedtuple() got multiple values for field",
                                        ));
                                    }
                                    bound_values[index] = Some(value);
                                }
                                for (index, name) in fields.iter().enumerate() {
                                    let Some(value) = bound_values[index].clone() else {
                                        return Err(RuntimeError::new(format!(
                                            "namedtuple() missing value for field '{}'",
                                            name
                                        )));
                                    };
                                    if let Object::Instance(instance_data) =
                                        &mut *instance.kind_mut()
                                    {
                                        instance_data.attrs.insert(name.clone(), value);
                                    } else {
                                        return Err(RuntimeError::new(
                                            "namedtuple() instance construction failed",
                                        ));
                                    }
                                }
                            } else if self.class_is_exception_class(&class) {
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        "args".to_string(),
                                        self.heap.alloc_tuple(args.clone()),
                                    );
                                    for (name, value) in kwargs {
                                        instance_data.attrs.insert(name, value);
                                    }
                                } else {
                                    return Err(RuntimeError::new(
                                        "exception instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_list_base(&class) {
                                let list_value =
                                    self.call_builtin(BuiltinFunction::List, args, kwargs)?;
                                let Value::List(_) = list_value else {
                                    return Err(RuntimeError::new(
                                        "list constructor returned non-list",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data
                                        .attrs
                                        .insert(LIST_BACKING_STORAGE_ATTR.to_string(), list_value);
                                } else {
                                    return Err(RuntimeError::new(
                                        "list instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_tuple_base(&class) {
                                let tuple_value =
                                    self.call_builtin(BuiltinFunction::Tuple, args, kwargs)?;
                                let Value::Tuple(_) = tuple_value else {
                                    return Err(RuntimeError::new(
                                        "tuple constructor returned non-tuple",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        TUPLE_BACKING_STORAGE_ATTR.to_string(),
                                        tuple_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "tuple instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_str_base(&class) {
                                let str_value =
                                    self.call_builtin(BuiltinFunction::Str, args, kwargs)?;
                                let Value::Str(_) = str_value else {
                                    return Err(RuntimeError::new(
                                        "str constructor returned non-str",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data
                                        .attrs
                                        .insert(STR_BACKING_STORAGE_ATTR.to_string(), str_value);
                                } else {
                                    return Err(RuntimeError::new(
                                        "str instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_bytes_base(&class) {
                                let bytes_value =
                                    self.call_builtin(BuiltinFunction::Bytes, args, kwargs)?;
                                let Value::Bytes(_) = bytes_value else {
                                    return Err(RuntimeError::new(
                                        "bytes constructor returned non-bytes",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        BYTES_BACKING_STORAGE_ATTR.to_string(),
                                        bytes_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "bytes instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_bytearray_base(&class) {
                                let bytearray_value =
                                    self.call_builtin(BuiltinFunction::ByteArray, args, kwargs)?;
                                let Value::ByteArray(_) = bytearray_value else {
                                    return Err(RuntimeError::new(
                                        "bytearray constructor returned non-bytearray",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        BYTES_BACKING_STORAGE_ATTR.to_string(),
                                        bytearray_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "bytearray instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_int_base(&class) {
                                let int_value = self.builtin_int(args, kwargs)?;
                                let (Value::Int(_) | Value::BigInt(_) | Value::Bool(_)) = int_value
                                else {
                                    return Err(RuntimeError::new(
                                        "int constructor returned non-int",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data
                                        .attrs
                                        .insert(INT_BACKING_STORAGE_ATTR.to_string(), int_value);
                                } else {
                                    return Err(RuntimeError::new(
                                        "int instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_float_base(&class) {
                                let float_value = self.builtin_float(args, kwargs)?;
                                let Value::Float(_) = float_value else {
                                    return Err(RuntimeError::new(
                                        "float constructor returned non-float",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        FLOAT_BACKING_STORAGE_ATTR.to_string(),
                                        float_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "float instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_complex_base(&class) {
                                let complex_value = self.builtin_complex(args, kwargs)?;
                                let Value::Complex { .. } = complex_value else {
                                    return Err(RuntimeError::new(
                                        "complex constructor returned non-complex",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        COMPLEX_BACKING_STORAGE_ATTR.to_string(),
                                        complex_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "complex instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_dict_base(&class) {
                                let dict_value =
                                    self.call_builtin(BuiltinFunction::Dict, args, kwargs)?;
                                let Value::Dict(_) = dict_value else {
                                    return Err(RuntimeError::new(
                                        "dict constructor returned non-dict",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data
                                        .attrs
                                        .insert(DICT_BACKING_STORAGE_ATTR.to_string(), dict_value);
                                } else {
                                    return Err(RuntimeError::new(
                                        "dict instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_set_base(&class) {
                                let set_value =
                                    self.call_builtin(BuiltinFunction::Set, args, kwargs)?;
                                let Value::Set(_) = set_value else {
                                    return Err(RuntimeError::new(
                                        "set constructor returned non-set",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data
                                        .attrs
                                        .insert(SET_BACKING_STORAGE_ATTR.to_string(), set_value);
                                } else {
                                    return Err(RuntimeError::new(
                                        "set instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_frozenset_base(&class) {
                                let frozenset_value =
                                    self.call_builtin(BuiltinFunction::FrozenSet, args, kwargs)?;
                                let Value::FrozenSet(_) = frozenset_value else {
                                    return Err(RuntimeError::new(
                                        "frozenset constructor returned non-frozenset",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        FROZENSET_BACKING_STORAGE_ATTR.to_string(),
                                        frozenset_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "frozenset instance construction failed",
                                    ));
                                }
                            } else if !kwargs.is_empty() || !args.is_empty() {
                                return Err(RuntimeError::new(
                                    "class constructor takes no arguments",
                                ));
                            }
                        }
                        self.push_value(Value::Instance(instance));
                        false
                    }
                }
            }
            _ => {
                return Err(RuntimeError::new("attempted to call non-function"));
            }
        };

        if !needs_run {
            let value = self.pop_value()?;
            return Ok(InternalCallOutcome::Value(value));
        }

        let previous_stop = self.run_stop_depth;
        self.run_stop_depth = Some(caller_depth);
        let run_result = self.run();
        self.run_stop_depth = previous_stop;
        run_result?;

        if self.frames.len() < caller_depth {
            return Ok(InternalCallOutcome::CallerExceptionHandled);
        }

        let caller = self
            .frames
            .get(caller_depth - 1)
            .ok_or_else(|| RuntimeError::new("caller frame missing"))?;
        if caller.ip != caller_ip {
            return Ok(InternalCallOutcome::CallerExceptionHandled);
        }

        let value = self.pop_value()?;
        Ok(InternalCallOutcome::Value(value))
    }

    pub(super) fn call_internal_preserving_caller(
        &mut self,
        callable: Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<InternalCallOutcome, RuntimeError> {
        let caller_depth = self.frames.len();
        let (caller_ip, caller_stack, caller_blocks, caller_active_exception) = self
            .frames
            .last()
            .map(|frame| {
                (
                    frame.ip,
                    frame.stack.clone(),
                    (!frame.blocks.is_empty()).then(|| frame.blocks.clone()),
                    frame.active_exception.clone(),
                )
            })
            .unwrap_or((0, Vec::new(), None, None));

        let outcome = self.call_internal(callable, args, kwargs);
        match outcome {
            Ok(InternalCallOutcome::Value(value)) => {
                self.restore_internal_call_caller_state(
                    caller_depth,
                    caller_ip,
                    &caller_stack,
                    &caller_blocks,
                    caller_active_exception.clone(),
                );
                Ok(InternalCallOutcome::Value(value))
            }
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                let active_exception = self
                    .frames
                    .last()
                    .and_then(|frame| frame.active_exception.clone());
                self.restore_internal_call_caller_state(
                    caller_depth,
                    caller_ip,
                    &caller_stack,
                    &caller_blocks,
                    active_exception,
                );
                Ok(InternalCallOutcome::CallerExceptionHandled)
            }
            Err(err) => {
                self.restore_internal_call_caller_state(
                    caller_depth,
                    caller_ip,
                    &caller_stack,
                    &caller_blocks,
                    caller_active_exception,
                );
                Err(err)
            }
        }
    }

    pub(super) fn restore_internal_call_caller_state(
        &mut self,
        caller_depth: usize,
        caller_ip: usize,
        caller_stack: &[Value],
        caller_blocks: &Option<Vec<Block>>,
        active_exception: Option<Value>,
    ) {
        if self.frames.len() == caller_depth {
            if let Some(frame) = self.frames.last_mut() {
                frame.ip = caller_ip;
                frame.stack = caller_stack.to_vec();
                if let Some(blocks) = caller_blocks {
                    frame.blocks = blocks.clone();
                } else {
                    frame.blocks.clear();
                }
                frame.active_exception = active_exception;
            }
        }
    }

    pub(super) fn load_attr_class(
        &mut self,
        class: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let (class_name, class_metaclass) = match &*class.kind() {
            Object::Class(class_data) => (class_data.name.clone(), class_data.metaclass.clone()),
            _ => ("<class>".to_string(), None),
        };
        let mut descriptor_owner: Option<ObjRef> = None;
        let attr = if let Some(attr) = class_attr_lookup(class, attr_name) {
            attr
        } else if attr_name == "__name__" {
            Value::Str(class_name.clone())
        } else if attr_name == "__qualname__" {
            Value::Str(class_name.clone())
        } else if attr_name == "__base__" {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return Err(RuntimeError::new("attribute access unsupported type"));
            };
            class_data
                .bases
                .first()
                .cloned()
                .map(Value::Class)
                .unwrap_or(Value::None)
        } else if attr_name == "__mro__" {
            let mro_values = self
                .class_mro_entries(class)
                .into_iter()
                .map(Value::Class)
                .collect::<Vec<_>>();
            self.heap.alloc_tuple(mro_values)
        } else if attr_name == "__module__" {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return Err(RuntimeError::new("attribute access unsupported type"));
            };
            class_data
                .attrs
                .get("__module__")
                .cloned()
                .unwrap_or(Value::None)
        } else if attr_name == "__dict__" {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return Err(RuntimeError::new("attribute access unsupported type"));
            };
            let entries = class_data
                .attrs
                .iter()
                .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                .collect::<Vec<_>>();
            self.heap.alloc_dict(entries)
        } else if attr_name == "register" {
            return Ok(AttrAccessOutcome::Value(self.alloc_native_bound_method(
                NativeMethodKind::ClassRegister,
                class.clone(),
            )));
        } else if attr_name == "__new__" {
            Value::Builtin(BuiltinFunction::ObjectNew)
        } else if attr_name == "__init__" {
            Value::Builtin(BuiltinFunction::ObjectInit)
        } else if attr_name == "__getstate__" {
            Value::Builtin(BuiltinFunction::ObjectGetState)
        } else if attr_name == "__doc__" {
            Value::None
        } else if attr_name == "__flags__" {
            Value::Int(PY_TPFLAGS_HEAPTYPE)
        } else if let Some(inherited) = self.load_attr_class_builtin_base_method(class, attr_name) {
            inherited
        } else if let Some(meta) = class_metaclass {
            if let Some(meta_attr) = class_attr_lookup(&meta, attr_name) {
                descriptor_owner = Some(meta);
                meta_attr
            } else {
                return Err(RuntimeError::new(format!(
                    "class '{}' has no attribute '{}'",
                    class_name, attr_name
                )));
            }
        } else {
            return Err(RuntimeError::new(format!(
                "class '{}' has no attribute '{}'",
                class_name, attr_name
            )));
        };

        if let Some(bound) = self.bind_classmethod_attr(class, &attr) {
            return Ok(AttrAccessOutcome::Value(bound));
        }

        if let Some(unwrapped) = self.unwrap_staticmethod_attr(&attr) {
            return Ok(AttrAccessOutcome::Value(unwrapped));
        }

        if descriptor_owner.is_some() {
            if let Value::Function(func) = attr.clone() {
                let bound = BoundMethod::new(func, class.clone());
                return Ok(AttrAccessOutcome::Value(
                    self.heap.alloc_bound_method(bound),
                ));
            }
        }

        let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
        if let Some(getter) = getter {
            let getter_args = if let Some(owner) = descriptor_owner {
                vec![Value::Class(class.clone()), Value::Class(owner)]
            } else {
                vec![Value::None, Value::Class(class.clone())]
            };
            return Ok(
                match self.call_internal(getter, getter_args, HashMap::new())? {
                    InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                    InternalCallOutcome::CallerExceptionHandled => {
                        AttrAccessOutcome::ExceptionHandled
                    }
                },
            );
        }

        Ok(AttrAccessOutcome::Value(attr))
    }

    pub(super) fn class_has_builtin_list_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "list",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_tuple_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => {
                    class_data.name == "tuple"
                        || matches!(
                            class_data.attrs.get("__pyrs_tuple_backed_type__"),
                            Some(Value::Bool(true))
                        )
                }
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_str_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "str",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_bytes_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "bytes",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_bytearray_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "bytearray",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_int_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "int",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_float_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "float",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_complex_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "complex",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_dict_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "dict",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_set_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "set",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_frozenset_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "frozenset",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_type_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "type",
                _ => false,
            })
    }

    pub(super) fn instantiate_type_derived_class(
        &mut self,
        metaclass: ObjRef,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 3 {
            return Err(RuntimeError::new(format!(
                "type.__new__() takes exactly 3 arguments ({} given)",
                args.len()
            )));
        }
        let name = match &args[0] {
            Value::Str(name) => name.clone(),
            _ => return Err(RuntimeError::new("type() first argument must be string")),
        };
        let base_values = match &args[1] {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
            },
            Value::List(list_obj) => match &*list_obj.kind() {
                Object::List(values) => values.clone(),
                _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
            },
            _ => return Err(RuntimeError::new("type() bases must be tuple/list")),
        };
        let mut base_classes = Vec::with_capacity(base_values.len());
        for base in base_values {
            base_classes.push(self.class_from_base_value(base)?);
        }
        let namespace = match &args[2] {
            Value::Dict(dict_obj) => match &*dict_obj.kind() {
                Object::Dict(entries) => {
                    let mut attrs = HashMap::new();
                    for (key, value) in entries {
                        let Value::Str(name) = key else {
                            return Err(RuntimeError::new("type() dict keys must be strings"));
                        };
                        attrs.insert(name.clone(), value.clone());
                    }
                    attrs
                }
                _ => return Err(RuntimeError::new("type() third argument must be dict")),
            },
            _ => return Err(RuntimeError::new("type() third argument must be dict")),
        };

        let class_module = match self.heap.alloc_module(ModuleObject::new(name)) {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *class_module.kind_mut() {
            module_data.globals = namespace;
        }
        match self.class_value_from_module(
            &class_module,
            base_classes,
            Some(Value::Class(metaclass)),
            kwargs,
        )? {
            ClassBuildOutcome::Value(value) => Ok(value),
            ClassBuildOutcome::ExceptionHandled => {
                Err(self.runtime_error_from_active_exception("metaclass call failed"))
            }
        }
    }

    pub(super) fn alloc_instance_for_class(&mut self, class: &ObjRef) -> ObjRef {
        let instance = match self.heap.alloc_instance(InstanceObject::new(class.clone())) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if self.class_has_builtin_list_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    LIST_BACKING_STORAGE_ATTR.to_string(),
                    self.heap.alloc_list(Vec::new()),
                );
            }
        } else if self.class_has_builtin_tuple_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    TUPLE_BACKING_STORAGE_ATTR.to_string(),
                    self.heap.alloc_tuple(Vec::new()),
                );
            }
        } else if self.class_has_builtin_str_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    STR_BACKING_STORAGE_ATTR.to_string(),
                    Value::Str(String::new()),
                );
            }
        }
        if self.class_has_builtin_bytes_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    BYTES_BACKING_STORAGE_ATTR.to_string(),
                    self.heap.alloc_bytes(Vec::new()),
                );
            }
        } else if self.class_has_builtin_bytearray_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    BYTES_BACKING_STORAGE_ATTR.to_string(),
                    self.heap.alloc_bytearray(Vec::new()),
                );
            }
        }
        if self.class_has_builtin_int_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(INT_BACKING_STORAGE_ATTR.to_string(), Value::Int(0));
            }
        }
        if self.class_has_builtin_float_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data
                    .attrs
                    .insert(FLOAT_BACKING_STORAGE_ATTR.to_string(), Value::Float(0.0));
            }
        }
        if self.class_has_builtin_complex_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    COMPLEX_BACKING_STORAGE_ATTR.to_string(),
                    Value::Complex {
                        real: 0.0,
                        imag: 0.0,
                    },
                );
            }
        }
        if self.class_has_builtin_dict_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    DICT_BACKING_STORAGE_ATTR.to_string(),
                    self.heap.alloc_dict(Vec::new()),
                );
            }
        }
        if self.class_has_builtin_set_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    SET_BACKING_STORAGE_ATTR.to_string(),
                    self.heap.alloc_set(Vec::new()),
                );
            }
        }
        if self.class_has_builtin_frozenset_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    FROZENSET_BACKING_STORAGE_ATTR.to_string(),
                    self.heap.alloc_frozenset(Vec::new()),
                );
            }
        }
        self.track_instance_del_candidate(class, &instance);
        instance
    }

    pub(super) fn instance_backing_list(&self, instance: &ObjRef) -> Option<ObjRef> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(LIST_BACKING_STORAGE_ATTR) {
            Some(Value::List(list)) => Some(list.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_backing_tuple(&self, instance: &ObjRef) -> Option<ObjRef> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(TUPLE_BACKING_STORAGE_ATTR) {
            Some(Value::Tuple(tuple)) => Some(tuple.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_backing_str(&self, instance: &ObjRef) -> Option<String> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(STR_BACKING_STORAGE_ATTR) {
            Some(Value::Str(text)) => Some(text.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_backing_int(&self, instance: &ObjRef) -> Option<Value> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(INT_BACKING_STORAGE_ATTR) {
            Some(Value::Int(value)) => Some(Value::Int(*value)),
            Some(Value::BigInt(value)) => Some(Value::BigInt(value.clone())),
            Some(Value::Bool(value)) => Some(Value::Bool(*value)),
            _ => None,
        }
    }

    pub(super) fn instance_backing_float(&self, instance: &ObjRef) -> Option<f64> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(FLOAT_BACKING_STORAGE_ATTR) {
            Some(Value::Float(value)) => Some(*value),
            _ => None,
        }
    }

    pub(super) fn instance_backing_complex(&self, instance: &ObjRef) -> Option<(f64, f64)> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(COMPLEX_BACKING_STORAGE_ATTR) {
            Some(Value::Complex { real, imag }) => Some((*real, *imag)),
            _ => None,
        }
    }

    pub(super) fn instance_backing_dict(&self, instance: &ObjRef) -> Option<ObjRef> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(DICT_BACKING_STORAGE_ATTR) {
            Some(Value::Dict(dict)) => Some(dict.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_backing_set(&self, instance: &ObjRef) -> Option<ObjRef> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(SET_BACKING_STORAGE_ATTR) {
            Some(Value::Set(set)) => Some(set.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_backing_frozenset(&self, instance: &ObjRef) -> Option<ObjRef> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(FROZENSET_BACKING_STORAGE_ATTR) {
            Some(Value::FrozenSet(set)) => Some(set.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_dict_entries(instance_data: &InstanceObject) -> Vec<(Value, Value)> {
        instance_data
            .attrs
            .iter()
            .filter_map(|(name, value)| match name.as_str() {
                LIST_BACKING_STORAGE_ATTR
                | TUPLE_BACKING_STORAGE_ATTR
                | STR_BACKING_STORAGE_ATTR
                | BYTES_BACKING_STORAGE_ATTR
                | INT_BACKING_STORAGE_ATTR
                | FLOAT_BACKING_STORAGE_ATTR
                | COMPLEX_BACKING_STORAGE_ATTR
                | DICT_BACKING_STORAGE_ATTR
                | SET_BACKING_STORAGE_ATTR
                | FROZENSET_BACKING_STORAGE_ATTR
                | INSTANCE_DICT_STORAGE_ATTR => None,
                _ => Some((Value::Str(name.clone()), value.clone())),
            })
            .collect()
    }

    pub(super) fn load_attr_instance(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return Err(RuntimeError::new("attribute access unsupported type")),
        };

        // CPython routes the default object-attribute path through a native slot
        // rather than an ordinary Python-level method call each access.
        // Mirror that behavior by bypassing generic bound-method invocation when
        // __getattribute__ resolves to the builtin object implementation.
        if matches!(
            class_attr_lookup(&class_ref, "__getattribute__"),
            Some(Value::Builtin(BuiltinFunction::ObjectGetAttribute))
        ) {
            return self.load_attr_instance_default(instance, attr_name, true);
        }

        let receiver = Value::Instance(instance.clone());
        if let Some(getattribute_method) =
            self.lookup_bound_special_method(&receiver, "__getattribute__")?
        {
            let getattribute_outcome = self.call_internal_preserving_caller(
                getattribute_method,
                vec![Value::Str(attr_name.to_string())],
                HashMap::new(),
            );
            match getattribute_outcome {
                Ok(InternalCallOutcome::Value(value)) => {
                    return Ok(AttrAccessOutcome::Value(value));
                }
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    if !self.active_exception_is("AttributeError") {
                        return Ok(AttrAccessOutcome::ExceptionHandled);
                    }
                    self.clear_active_exception();
                }
                Err(err) => {
                    if classify_runtime_error(&err.message) != "AttributeError" {
                        return Err(err);
                    }
                }
            }

            if let Some(getattr_method) =
                self.lookup_bound_special_method(&receiver, "__getattr__")?
            {
                return Ok(
                    match self.call_internal_preserving_caller(
                        getattr_method,
                        vec![Value::Str(attr_name.to_string())],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrAccessOutcome::ExceptionHandled
                        }
                    },
                );
            }

            let class_name = match &*instance.kind() {
                Object::Instance(instance_data) => match &*instance_data.class.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "<class>".to_string(),
                },
                _ => "<class>".to_string(),
            };
            return Err(RuntimeError::new(format!(
                "'{}' object has no attribute '{}'",
                class_name, attr_name
            )));
        }

        self.load_attr_instance_default(instance, attr_name, true)
    }

    pub(super) fn load_attr_instance_default(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
        allow_getattr_fallback: bool,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return Err(RuntimeError::new("attribute access unsupported type")),
        };

        if attr_name == "__class__" {
            return Ok(AttrAccessOutcome::Value(Value::Class(class_ref)));
        }

        if attr_name == "__dict__" {
            let has_dynamic_dict = match collect_slot_names(&class_ref) {
                Some(allowed_slots) => {
                    allowed_slots.iter().any(|name| name == "__dict__")
                        || class_inherits_dynamic_instance_dict(&class_ref)
                }
                None => true,
            };
            if !has_dynamic_dict {
                let class_name = match &*class_ref.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "<class>".to_string(),
                };
                return Err(RuntimeError::new(format!(
                    "'{}' object has no attribute '__dict__'",
                    class_name
                )));
            }
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                if let Some(Value::Dict(dict_obj)) =
                    instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
                {
                    return Ok(AttrAccessOutcome::Value(Value::Dict(dict_obj.clone())));
                }
                let dict_value = self
                    .heap
                    .alloc_dict(Self::instance_dict_entries(instance_data));
                instance_data
                    .attrs
                    .insert(INSTANCE_DICT_STORAGE_ATTR.to_string(), dict_value.clone());
                return Ok(AttrAccessOutcome::Value(dict_value));
            }
            return Err(RuntimeError::new("attribute access unsupported type"));
        }

        if let Some(attr) = self.load_attr_property_instance(instance, attr_name) {
            return Ok(AttrAccessOutcome::Value(attr));
        }
        if let Some(attr) = self.load_attr_cached_property_instance(instance, attr_name) {
            return Ok(AttrAccessOutcome::Value(attr));
        }

        let class_attr = class_attr_lookup(&class_ref, attr_name);
        if let Some(attr) = class_attr.clone() {
            let (getter, setter, deleter) = self.descriptor_hooks(&attr)?;
            if setter.is_some() || deleter.is_some() {
                if let Some(getter) = getter {
                    return Ok(
                        match self.call_internal_preserving_caller(
                            getter,
                            vec![
                                Value::Instance(instance.clone()),
                                Value::Class(class_ref.clone()),
                            ],
                            HashMap::new(),
                        )? {
                            InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                            InternalCallOutcome::CallerExceptionHandled => {
                                AttrAccessOutcome::ExceptionHandled
                            }
                        },
                    );
                }
                return Ok(AttrAccessOutcome::Value(attr));
            }
        }

        if let Object::Instance(instance_data) = &*instance.kind() {
            if let Some(attr) = instance_data.attrs.get(attr_name).cloned() {
                return Ok(AttrAccessOutcome::Value(attr));
            }
            if let Some(Value::Dict(dict_obj)) = instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
            {
                if let Some(attr) = dict_get_value(dict_obj, &Value::Str(attr_name.to_string())) {
                    return Ok(AttrAccessOutcome::Value(attr));
                }
            }
        }

        let reduce_attr = attr_name == "__reduce_ex__" || attr_name == "__reduce__";
        if let Some(backing_list) = self.instance_backing_list(instance) {
            if !reduce_attr {
                if let Ok(bound_method) = self.load_attr_list_method(backing_list, attr_name) {
                    return Ok(AttrAccessOutcome::Value(bound_method));
                }
            }
        }
        if let Some(backing_tuple) = self.instance_backing_tuple(instance) {
            if !reduce_attr {
                if let Ok(bound_method) = self.load_attr_tuple_method(backing_tuple, attr_name) {
                    return Ok(AttrAccessOutcome::Value(bound_method));
                }
            }
        }
        if let Some(backing_str) = self.instance_backing_str(instance) {
            if !reduce_attr {
                if let Ok(bound_method) = self.load_attr_str_method(backing_str, attr_name) {
                    return Ok(AttrAccessOutcome::Value(bound_method));
                }
            }
        }
        if let Some(backing_dict) = self.instance_backing_dict(instance) {
            if !reduce_attr {
                if let Ok(bound_method) = self.load_attr_dict_method(backing_dict, attr_name) {
                    return Ok(AttrAccessOutcome::Value(bound_method));
                }
            }
        }
        if let Some(backing_set) = self.instance_backing_set(instance) {
            if !reduce_attr {
                if let Ok(bound_method) = self.load_attr_set_method(backing_set, attr_name) {
                    return Ok(AttrAccessOutcome::Value(bound_method));
                }
            }
        }
        if let Some(backing_frozenset) = self.instance_backing_frozenset(instance) {
            if !reduce_attr {
                if let Ok(bound_method) = self.load_attr_set_method(backing_frozenset, attr_name) {
                    return Ok(AttrAccessOutcome::Value(bound_method));
                }
            }
        }

        if let Some(attr) = class_attr {
            if let Some(bound) = self.bind_classmethod_attr(&class_ref, &attr) {
                return Ok(AttrAccessOutcome::Value(bound));
            }

            if let Some(unwrapped) = self.unwrap_staticmethod_attr(&attr) {
                return Ok(AttrAccessOutcome::Value(unwrapped));
            }
            if let Value::Function(func) = attr.clone() {
                let bound = BoundMethod::new(func, instance.clone());
                return Ok(AttrAccessOutcome::Value(
                    self.heap.alloc_bound_method(bound),
                ));
            }
            if let Value::Builtin(builtin) = attr.clone() {
                let direct_user_defined_attr = matches!(&*class_ref.kind(), Object::Class(class_data)
                    if matches!(class_data.attrs.get("__pyrs_user_class__"), Some(Value::Bool(true)))
                        && class_data.attrs.contains_key(attr_name));
                if direct_user_defined_attr {
                    return Ok(AttrAccessOutcome::Value(Value::Builtin(builtin)));
                }
                return Ok(AttrAccessOutcome::Value(
                    self.alloc_builtin_bound_method(builtin, instance.clone()),
                ));
            }
            let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
            if let Some(getter) = getter {
                return Ok(
                    match self.call_internal_preserving_caller(
                        getter,
                        vec![
                            Value::Instance(instance.clone()),
                            Value::Class(class_ref.clone()),
                        ],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrAccessOutcome::ExceptionHandled
                        }
                    },
                );
            }
            return Ok(AttrAccessOutcome::Value(attr));
        }

        if allow_getattr_fallback {
            if let Some(getattr_method) =
                self.lookup_bound_special_method(&Value::Instance(instance.clone()), "__getattr__")?
            {
                return Ok(
                    match self.call_internal_preserving_caller(
                        getattr_method,
                        vec![Value::Str(attr_name.to_string())],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrAccessOutcome::ExceptionHandled
                        }
                    },
                );
            }
        }

        if attr_name == "__getstate__" {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::ObjectGetState,
                instance.clone(),
            )));
        }
        if attr_name == "__setstate__" {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::ObjectSetState,
                instance.clone(),
            )));
        }
        if attr_name == "__reduce_ex__" || attr_name == "__reduce__" {
            return Ok(AttrAccessOutcome::Value(
                self.alloc_reduce_ex_bound_method(Value::Instance(instance.clone())),
            ));
        }

        let class_name = match &*class_ref.kind() {
            Object::Class(class_data) => class_data.name.clone(),
            _ => "<class>".to_string(),
        };
        Err(RuntimeError::new(format!(
            "'{}' object has no attribute '{}'",
            class_name, attr_name
        )))
    }

    pub(super) fn load_attr_super(
        &mut self,
        super_ref: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let (start_class, receiver, object_type) = match &*super_ref.kind() {
            Object::Super(data) => (
                data.start_class.clone(),
                data.object.clone(),
                data.object_type.clone(),
            ),
            _ => return Err(RuntimeError::new("attribute access unsupported type")),
        };

        let receiver_value = self.receiver_value(&receiver)?;
        let owner_value = Value::Class(object_type.clone());
        let mro = self.class_mro_entries(&object_type);
        let start_idx = mro
            .iter()
            .position(|entry| entry.id() == start_class.id())
            .map(|idx| idx + 1)
            .unwrap_or(0);

        for class in mro.into_iter().skip(start_idx) {
            if let Some(attr) = class_attr_lookup_direct(&class, attr_name) {
                if let Some(bound) = self.bind_classmethod_attr(&object_type, &attr) {
                    return Ok(AttrAccessOutcome::Value(bound));
                }
                if let Some(unwrapped) = self.unwrap_staticmethod_attr(&attr) {
                    return Ok(AttrAccessOutcome::Value(unwrapped));
                }
                if let Value::Function(func) = attr.clone() {
                    let bound = BoundMethod::new(func, receiver.clone());
                    return Ok(AttrAccessOutcome::Value(
                        self.heap.alloc_bound_method(bound),
                    ));
                }
                if let Value::Builtin(builtin) = attr.clone() {
                    return Ok(AttrAccessOutcome::Value(
                        self.alloc_builtin_bound_method(builtin, receiver.clone()),
                    ));
                }
                let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
                if let Some(getter) = getter {
                    return Ok(
                        match self.call_internal(
                            getter,
                            vec![receiver_value.clone(), owner_value.clone()],
                            HashMap::new(),
                        )? {
                            InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                            InternalCallOutcome::CallerExceptionHandled => {
                                AttrAccessOutcome::ExceptionHandled
                            }
                        },
                    );
                }
                return Ok(AttrAccessOutcome::Value(attr));
            }
        }

        // Synthetic builtin base classes used by class inheritance currently
        // may not carry explicit `__new__`/`__init__` attrs in their class dict.
        // CPython still resolves these through `super(...)` in paths like
        // `super(Subclass, cls).__new__(cls, value)`.
        if attr_name == "__new__" {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::ObjectNew,
                object_type,
            )));
        }
        if attr_name == "__init__" {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::ObjectInit,
                receiver,
            )));
        }

        Err(RuntimeError::new(format!(
            "super object has no attribute '{}'",
            attr_name
        )))
    }

    pub(super) fn load_attr_module(
        &mut self,
        module: &ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let active_frame_module_dict = self
            .frames
            .iter()
            .rposition(|frame| frame.is_module && frame.module.id() == module.id())
            .map(|frame_index| self.ensure_frame_module_locals_dict(frame_index));
        let (module_name, attr, module_getattr, globals_snapshot, module_is_package) =
            match &*module.kind() {
                Object::Module(module_data) => {
                    let attr_key = Value::Str(attr_name.to_string());
                    let getattr_key = Value::Str("__getattr__".to_string());
                    let path_key = Value::Str("__path__".to_string());
                    let attr = active_frame_module_dict
                        .as_ref()
                        .and_then(|dict| dict_get_value(dict, &attr_key))
                        .or_else(|| module_data.globals.get(attr_name).cloned());
                    let module_getattr = active_frame_module_dict
                        .as_ref()
                        .and_then(|dict| dict_get_value(dict, &getattr_key))
                        .or_else(|| module_data.globals.get("__getattr__").cloned());
                    let module_name = module_data.name.clone();
                    let module_is_package = active_frame_module_dict
                        .as_ref()
                        .and_then(|dict| dict_get_value(dict, &path_key))
                        .is_some()
                        || module_data.globals.contains_key("__path__");
                    let globals_snapshot = if let Some(dict) = &active_frame_module_dict {
                        match &*dict.kind() {
                            Object::Dict(entries) => entries.to_vec(),
                            _ => Vec::new(),
                        }
                    } else {
                        module_data
                            .globals
                            .iter()
                            .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                            .collect::<Vec<_>>()
                    };
                    (
                        module_name,
                        attr,
                        module_getattr,
                        globals_snapshot,
                        module_is_package,
                    )
                }
                _ => {
                    return Err(RuntimeError::new("attribute access unsupported type"));
                }
            };
        if let Some(attr) = attr {
            return Ok(attr);
        }
        if (module_name == "__classmethod__" || module_name == "__staticmethod__")
            && (attr_name == "__reduce_ex__" || attr_name == "__reduce__")
        {
            return Ok(self.alloc_native_bound_method(
                NativeMethodKind::DescriptorReduceTypeError,
                module.clone(),
            ));
        }
        if module_name == "unittest" && attr_name == "IsolatedAsyncioTestCase" {
            if let Some(test_case) = globals_snapshot
                .iter()
                .find_map(|(name, value)| match name {
                    Value::Str(name) if name == "TestCase" || name == "Case" => Some(value.clone()),
                    _ => None,
                })
            {
                return Ok(test_case);
            }
            return Ok(Value::Class(
                self.alloc_synthetic_class("unittest.IsolatedAsyncioTestCase"),
            ));
        }
        if attr_name == "__dict__" {
            if let Some(dict) = active_frame_module_dict {
                return Ok(Value::Dict(dict));
            }
            return Ok(self.heap.alloc_dict(globals_snapshot));
        }
        if module_name == "__re_pattern__" {
            let kind = match attr_name {
                "search" => Some(NativeMethodKind::RePatternSearch),
                "match" => Some(NativeMethodKind::RePatternMatch),
                "fullmatch" => Some(NativeMethodKind::RePatternFullMatch),
                "sub" => Some(NativeMethodKind::RePatternSub),
                "findall" => Some(NativeMethodKind::Builtin(BuiltinFunction::RePatternFindAll)),
                "finditer" => Some(NativeMethodKind::Builtin(
                    BuiltinFunction::RePatternFindIter,
                )),
                _ => None,
            };
            if let Some(kind) = kind {
                return Ok(self.alloc_native_bound_method(kind, module.clone()));
            }
        }
        if module_name == "__re_match__" {
            let kind = match attr_name {
                "group" => Some(NativeMethodKind::ReMatchGroup),
                "groups" => Some(NativeMethodKind::ReMatchGroups),
                "groupdict" => Some(NativeMethodKind::ReMatchGroupDict),
                "start" => Some(NativeMethodKind::ReMatchStart),
                "end" => Some(NativeMethodKind::ReMatchEnd),
                "span" => Some(NativeMethodKind::ReMatchSpan),
                _ => None,
            };
            if let Some(kind) = kind {
                return Ok(self.alloc_native_bound_method(kind, module.clone()));
            }
        }
        if let Some(attr) = module_name.split('.').last().and_then(|suffix| {
            if suffix == attr_name {
                Some(Value::Module(module.clone()))
            } else {
                None
            }
        }) {
            return Ok(attr);
        }
        if module_is_package {
            if let Some(submodule) = self.load_submodule(module, attr_name) {
                return Ok(Value::Module(submodule));
            }
        }
        if attr_name != "__getattr__" {
            if let Some(module_getattr) = module_getattr {
                return match self.call_internal(
                    module_getattr,
                    vec![Value::Str(attr_name.to_string())],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(value) => Ok(value),
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(RuntimeError::new("module __getattr__ failed"))
                    }
                };
            }
        }
        Err(RuntimeError::new(format!(
            "module '{}' has no attribute '{}'",
            module_name, attr_name
        )))
    }

    pub(super) fn store_attr_instance(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
        value: Value,
    ) -> Result<AttrMutationOutcome, RuntimeError> {
        if let Some(setattr_method) =
            self.lookup_bound_special_method(&Value::Instance(instance.clone()), "__setattr__")?
        {
            return Ok(
                match self.call_internal(
                    setattr_method,
                    vec![Value::Str(attr_name.to_string()), value],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                    InternalCallOutcome::CallerExceptionHandled => {
                        AttrMutationOutcome::ExceptionHandled
                    }
                },
            );
        }

        self.store_attr_instance_direct(instance, attr_name, value)
    }

    fn class_is_cpickle_type(class_ref: &ObjRef, expected_name: &str) -> bool {
        let Object::Class(class_data) = &*class_ref.kind() else {
            return false;
        };
        if class_data.name != expected_name {
            return false;
        }
        matches!(
            class_data.attrs.get("__module__"),
            Some(Value::Str(module_name)) if module_name == "_pickle"
        )
    }

    fn validate_cpickle_unpickler_memo_assignment(value: &Value) -> Result<(), RuntimeError> {
        let Value::Dict(dict_obj) = value else {
            return Err(RuntimeError::new("TypeError: unpickler memo must be a dict"));
        };
        let Object::Dict(entries) = &*dict_obj.kind() else {
            return Err(RuntimeError::new("TypeError: unpickler memo must be a dict"));
        };
        for (key, _) in entries.iter() {
            let is_negative_index = match key {
                Value::Int(index) => *index < 0,
                Value::BigInt(index) => index.is_negative(),
                Value::Bool(_) => false,
                _ => {
                    return Err(RuntimeError::new(
                        "TypeError: memo keys must be integers",
                    ));
                }
            };
            if is_negative_index {
                return Err(RuntimeError::new(
                    "ValueError: memo key out of range",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn store_attr_instance_direct(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
        value: Value,
    ) -> Result<AttrMutationOutcome, RuntimeError> {
        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return Err(RuntimeError::new("attribute assignment unsupported type")),
        };
        if matches!(
            &*class_ref.kind(),
            Object::Class(class_data) if class_data.name == "__csv_dialect__"
        ) {
            return Err(RuntimeError::new("csv dialect attributes are read-only"));
        }
        if attr_name == "memo" && Self::class_is_cpickle_type(&class_ref, "Unpickler") {
            Self::validate_cpickle_unpickler_memo_assignment(&value)?;
        }

        if let Some(descriptor) = class_attr_lookup(&class_ref, attr_name) {
            let (_getter, setter, _deleter) = self.descriptor_hooks(&descriptor)?;
            if let Some(setter) = setter {
                return Ok(
                    match self.call_internal(
                        setter,
                        vec![Value::Instance(instance.clone()), value],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrMutationOutcome::ExceptionHandled
                        }
                    },
                );
            }
        }

        if let Some(allowed_slots) = collect_slot_names(&class_ref) {
            let has_dynamic_dict = allowed_slots.iter().any(|name| name == "__dict__")
                || class_inherits_dynamic_instance_dict(&class_ref);
            if has_dynamic_dict {
                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                    if let Some(Value::Dict(dict_obj)) =
                        instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
                    {
                        dict_set_value(dict_obj, Value::Str(attr_name.to_string()), value.clone());
                    }
                    instance_data.attrs.insert(attr_name.to_string(), value);
                }
                return Ok(AttrMutationOutcome::Done);
            }
            let allowed = allowed_slots.iter().any(|name| name == attr_name);
            if !allowed {
                return Err(RuntimeError::new(format!(
                    "'{}' object has no attribute '{}'",
                    class_name_for_instance(instance).unwrap_or_else(|| "object".to_string()),
                    attr_name
                )));
            }
        }

        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            if let Some(Value::Dict(dict_obj)) = instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
            {
                dict_set_value(dict_obj, Value::Str(attr_name.to_string()), value.clone());
            }
            instance_data.attrs.insert(attr_name.to_string(), value);
        }
        Ok(AttrMutationOutcome::Done)
    }

    pub(super) fn delete_attr_instance(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrMutationOutcome, RuntimeError> {
        if let Some(delattr_method) =
            self.lookup_bound_special_method(&Value::Instance(instance.clone()), "__delattr__")?
        {
            return Ok(
                match self.call_internal(
                    delattr_method,
                    vec![Value::Str(attr_name.to_string())],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                    InternalCallOutcome::CallerExceptionHandled => {
                        AttrMutationOutcome::ExceptionHandled
                    }
                },
            );
        }

        self.delete_attr_instance_direct(instance, attr_name)
    }

    pub(super) fn delete_attr_instance_direct(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrMutationOutcome, RuntimeError> {
        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return Err(RuntimeError::new("attribute deletion unsupported type")),
        };
        if matches!(
            &*class_ref.kind(),
            Object::Class(class_data) if class_data.name == "__csv_dialect__"
        ) {
            return Err(RuntimeError::new("csv dialect attributes are read-only"));
        }

        if let Some(descriptor) = class_attr_lookup(&class_ref, attr_name) {
            let (_getter, _setter, deleter) = self.descriptor_hooks(&descriptor)?;
            if let Some(deleter) = deleter {
                return Ok(
                    match self.call_internal(
                        deleter,
                        vec![Value::Instance(instance.clone())],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrMutationOutcome::ExceptionHandled
                        }
                    },
                );
            }
        }

        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            if instance_data.attrs.remove(attr_name).is_some() {
                if let Some(Value::Dict(dict_obj)) =
                    instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
                {
                    dict_remove_value(dict_obj, &Value::Str(attr_name.to_string()));
                }
                return Ok(AttrMutationOutcome::Done);
            }
            if let Some(Value::Dict(dict_obj)) = instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
            {
                if dict_remove_value(dict_obj, &Value::Str(attr_name.to_string())).is_some() {
                    return Ok(AttrMutationOutcome::Done);
                }
            }
        }

        Err(RuntimeError::new(format!(
            "attribute '{}' does not exist",
            attr_name
        )))
    }
}
