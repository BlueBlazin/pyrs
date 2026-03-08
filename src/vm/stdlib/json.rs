//! Native JSON helper implementations used by stdlib `json` accelerator hooks.
//!
//! The goal here is CPython-compatible behavior for core `dumps`/`loads` paths,
//! while explicitly rejecting unsupported option families with typed errors.

use super::super::{
    BigInt, BuiltinFunction, ExceptionObject, HashMap, HashSet, Heap, InstanceObject,
    InternalCallOutcome, ModuleObject, ObjRef, Object, RuntimeError, Value, Vm, format_repr,
    is_truthy, runtime_get_int_max_str_digits, value_to_int,
};
use crate::unicode::{internal_char_from_codepoint, surrogate_code_unit_from_internal_char};

#[derive(Clone)]
/// Normalized option set for `json.dumps`-style serialization.
struct JsonDumpsOptions {
    skipkeys: bool,
    ensure_ascii: bool,
    allow_nan: bool,
    sort_keys: bool,
    item_separator: String,
    key_separator: String,
    indent: Option<String>,
}

impl Default for JsonDumpsOptions {
    fn default() -> Self {
        Self {
            skipkeys: false,
            ensure_ascii: true,
            allow_nan: true,
            sort_keys: false,
            item_separator: ", ".to_string(),
            key_separator: ": ".to_string(),
            indent: None,
        }
    }
}

const JSON_SCANNER_CLASS_ATTR: &str = "__pyrs_scanner_class__";
const JSON_SCANNER_PARSE_OBJECT_ATTR: &str = "__pyrs_json_parse_object__";
const JSON_SCANNER_PARSE_ARRAY_ATTR: &str = "__pyrs_json_parse_array__";
const JSON_SCANNER_PARSE_STRING_ATTR: &str = "__pyrs_json_parse_string__";
const JSON_SCANNER_RECURSIVE_SCANNER_ATTR: &str = "__pyrs_json_scanner__";
const JSON_SCANNER_MEMO_ATTR: &str = "__pyrs_json_memo__";

#[derive(Clone)]
struct JsonScannerState {
    strict: bool,
    object_hook: Value,
    object_pairs_hook: Value,
    parse_float: Value,
    parse_int: Value,
    parse_constant: Value,
    parse_object: Value,
    parse_array: Value,
    parse_string: Value,
}

impl Vm {
    pub(in crate::vm) fn builtin_json_dumps(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new("dumps() expects one argument"));
        }

        let mut options = JsonDumpsOptions::default();

        if let Some(skipkeys) = kwargs.remove("skipkeys") {
            options.skipkeys = self.truthy_from_value(&skipkeys)?;
        }
        if let Some(ensure_ascii) = kwargs.remove("ensure_ascii") {
            options.ensure_ascii = self.truthy_from_value(&ensure_ascii)?;
        }
        if let Some(check_circular) = kwargs.remove("check_circular") {
            let check_circular = self.truthy_from_value(&check_circular)?;
            if !check_circular {
                return Err(RuntimeError::new(
                    "dumps() check_circular=False is not supported yet",
                ));
            }
        }
        if let Some(allow_nan) = kwargs.remove("allow_nan") {
            options.allow_nan = self.truthy_from_value(&allow_nan)?;
        }
        if let Some(indent) = kwargs.remove("indent") {
            options.indent = parse_json_indent(indent)?;
            if options.indent.is_some() {
                options.item_separator = ",".to_string();
            }
        }
        if let Some(sort_keys) = kwargs.remove("sort_keys") {
            options.sort_keys = self.truthy_from_value(&sort_keys)?;
        }
        if let Some(separators) = kwargs.remove("separators") {
            let (item_sep, key_sep) = parse_json_separators(separators)?;
            options.item_separator = item_sep;
            options.key_separator = key_sep;
        }

        if let Some(cls) = kwargs.remove("cls")
            && !matches!(cls, Value::None)
        {
            return Err(RuntimeError::new("dumps() cls is not supported yet"));
        }
        let default = kwargs.remove("default");
        if let Some(default_callable) = &default {
            if matches!(default_callable, Value::None) {
                // Treat default=None as not set.
            } else if !self.is_callable_value(default_callable) {
                return Err(RuntimeError::new("dumps() default must be callable"));
            }
        }

        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "dumps() got an unexpected keyword argument",
            ));
        }

        let default_ref = default.as_ref().and_then(|value| {
            if matches!(value, Value::None) {
                None
            } else {
                Some(value)
            }
        });
        let text = json_serialize_value(self, &args[0], &options, default_ref)?;
        Ok(Value::Str(text))
    }

    pub(in crate::vm) fn builtin_json_loads(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new("loads() expects one argument"));
        }

        if let Some(strict) = kwargs.remove("strict") {
            let strict = self.truthy_from_value(&strict)?;
            if !strict {
                return Err(RuntimeError::new(
                    "loads() strict=False is not supported yet",
                ));
            }
        }
        for key in [
            "cls",
            "object_hook",
            "object_pairs_hook",
            "parse_float",
            "parse_int",
            "parse_constant",
        ] {
            if let Some(value) = kwargs.remove(key)
                && !matches!(value, Value::None)
            {
                return Err(RuntimeError::new(format!(
                    "loads() {} is not supported yet",
                    key
                )));
            }
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "loads() got an unexpected keyword argument",
            ));
        }

        let text = json_source_text(&args[0])?;
        let depth_limit = json_parse_depth_limit(self);
        let node = parse_json_node_with_limit(&text, depth_limit).map_err(|err| {
            if json_parse_error_is_recursion_limit(&err) {
                self.recursion_limit_error()
            } else {
                err
            }
        })?;
        Ok(json_node_to_value(node, &self.heap))
    }

    pub(in crate::vm) fn builtin_json_encode_basestring(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "encode_basestring() expects one string argument",
            ));
        }
        let Value::Str(text) = &args[0] else {
            return Err(RuntimeError::new("encode_basestring() expects str"));
        };
        Ok(Value::Str(json_escape_string(text, false)))
    }

    pub(in crate::vm) fn builtin_json_encode_basestring_ascii(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "encode_basestring_ascii() expects one string argument",
            ));
        }
        let Value::Str(text) = &args[0] else {
            return Err(RuntimeError::new("encode_basestring_ascii() expects str"));
        };
        Ok(Value::Str(json_escape_string(text, true)))
    }

    pub(in crate::vm) fn builtin_json_make_encoder(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let arg_names = [
            "markers",
            "default",
            "encoder",
            "indent",
            "key_separator",
            "item_separator",
            "sort_keys",
            "skipkeys",
            "allow_nan",
        ];
        if args.len() > arg_names.len() {
            return Err(RuntimeError::type_error(
                "make_encoder() expects 9 positional arguments",
            ));
        }
        let mut values: Vec<Option<Value>> = vec![None; arg_names.len()];
        for (idx, value) in args.drain(..).enumerate() {
            values[idx] = Some(value);
        }
        for (idx, name) in arg_names.iter().enumerate() {
            if let Some(value) = kwargs.remove(*name) {
                if values[idx].is_some() {
                    return Err(RuntimeError::type_error(format!(
                        "make_encoder() got multiple values for argument '{}'",
                        name
                    )));
                }
                values[idx] = Some(value);
            }
        }
        if let Some(unexpected) = kwargs.keys().next().cloned() {
            return Err(RuntimeError::type_error(format!(
                "make_encoder() got an unexpected keyword argument '{}'",
                unexpected
            )));
        }
        if values.iter().any(|value| value.is_none()) {
            return Err(RuntimeError::type_error(
                "make_encoder() expects 9 arguments",
            ));
        }
        let markers = values[0].take().expect("validated marker argument");
        if !matches!(markers, Value::None | Value::Dict(_)) {
            return Err(RuntimeError::type_error(format!(
                "make_encoder() argument 1 must be dict or None, not {}",
                self.value_type_name_for_error(&markers)
            )));
        }
        let default_callable = values[1].take().expect("validated default argument");
        let encoder_callable = values[2].take().expect("validated encoder argument");
        let indent_value = values[3].take().expect("validated indent argument");
        let key_separator_value = values[4].take().expect("validated key separator argument");
        let item_separator_value = values[5].take().expect("validated item separator argument");
        let sort_keys_value = values[6].take().expect("validated sort_keys argument");
        let skipkeys_value = values[7].take().expect("validated skipkeys argument");
        let allow_nan_value = values[8].take().expect("validated allow_nan argument");

        let key_separator = match &key_separator_value {
            Value::Str(value) => value.clone(),
            _ => {
                return Err(RuntimeError::type_error(format!(
                    "make_encoder() argument 5 must be str, not {}",
                    self.value_type_name_for_error(&key_separator_value)
                )));
            }
        };
        let item_separator = match &item_separator_value {
            Value::Str(value) => value.clone(),
            _ => {
                return Err(RuntimeError::type_error(format!(
                    "make_encoder() argument 6 must be str, not {}",
                    self.value_type_name_for_error(&item_separator_value)
                )));
            }
        };
        let ensure_ascii = matches!(
            encoder_callable,
            Value::Builtin(BuiltinFunction::JsonEncodeBaseStringAscii)
        );
        let skipkeys = self.truthy_from_value(&skipkeys_value)?;
        let allow_nan = self.truthy_from_value(&allow_nan_value)?;
        let sort_keys = self.truthy_from_value(&sort_keys_value)?;

        let wrapper = match self
            .heap
            .alloc_module(ModuleObject::new("__json_make_encoder__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
            module_data
                .globals
                .insert("skipkeys".to_string(), Value::Bool(skipkeys));
            module_data
                .globals
                .insert("ensure_ascii".to_string(), Value::Bool(ensure_ascii));
            module_data
                .globals
                .insert("allow_nan".to_string(), Value::Bool(allow_nan));
            module_data
                .globals
                .insert("sort_keys".to_string(), Value::Bool(sort_keys));
            module_data
                .globals
                .insert("item_separator".to_string(), Value::Str(item_separator));
            module_data
                .globals
                .insert("key_separator".to_string(), Value::Str(key_separator));
            module_data
                .globals
                .insert("default".to_string(), default_callable);
            module_data
                .globals
                .insert("encoder".to_string(), encoder_callable);
            module_data.globals.insert("markers".to_string(), markers);
            module_data
                .globals
                .insert("indent".to_string(), indent_value);
        }
        Ok(self.alloc_builtin_bound_method(BuiltinFunction::JsonMakeEncoderCall, wrapper))
    }

    pub(in crate::vm) fn builtin_json_make_encoder_call(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::type_error(
                "_iterencode() expects receiver, object, and _current_indent_level",
            ));
        }
        let current_indent_level = if args.len() == 3 {
            if kwargs.contains_key("_current_indent_level") {
                return Err(RuntimeError::type_error(
                    "_iterencode() got multiple values for argument '_current_indent_level'",
                ));
            }
            value_to_int(args.remove(2))?
        } else if let Some(value) = kwargs.remove("_current_indent_level") {
            value_to_int(value)?
        } else {
            return Err(RuntimeError::type_error(
                "_iterencode() missing required argument '_current_indent_level'",
            ));
        };
        if let Some(unexpected) = kwargs.keys().next().cloned() {
            return Err(RuntimeError::type_error(format!(
                "_iterencode() got an unexpected keyword argument '{}'",
                unexpected
            )));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let value = args.remove(0);

        let (
            skipkeys,
            ensure_ascii,
            allow_nan,
            sort_keys,
            item_separator,
            key_separator,
            default_callable,
            encoder_callable,
            indent,
        ) = match &*receiver.kind() {
            Object::Module(module_data) => {
                let bool_setting = |name: &str| {
                    module_data
                        .globals
                        .get(name)
                        .map(is_truthy)
                        .unwrap_or(false)
                };
                let string_setting = |name: &str, fallback: &str| {
                    module_data
                        .globals
                        .get(name)
                        .and_then(|value| match value {
                            Value::Str(text) => Some(text.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| fallback.to_string())
                };
                (
                    bool_setting("skipkeys"),
                    bool_setting("ensure_ascii"),
                    bool_setting("allow_nan"),
                    bool_setting("sort_keys"),
                    string_setting("item_separator", ", "),
                    string_setting("key_separator", ": "),
                    module_data.globals.get("default").cloned(),
                    module_data
                        .globals
                        .get("encoder")
                        .cloned()
                        .unwrap_or(Value::None),
                    module_data
                        .globals
                        .get("indent")
                        .cloned()
                        .unwrap_or(Value::None),
                )
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "_iterencode() receiver is invalid",
                ));
            }
        };

        self.json_validate_encoder_for_value(&encoder_callable, &value)?;

        let mut dumps_kwargs = HashMap::new();
        dumps_kwargs.insert("skipkeys".to_string(), Value::Bool(skipkeys));
        dumps_kwargs.insert("ensure_ascii".to_string(), Value::Bool(ensure_ascii));
        dumps_kwargs.insert("allow_nan".to_string(), Value::Bool(allow_nan));
        dumps_kwargs.insert("sort_keys".to_string(), Value::Bool(sort_keys));
        dumps_kwargs.insert(
            "separators".to_string(),
            self.heap
                .alloc_tuple(vec![Value::Str(item_separator), Value::Str(key_separator)]),
        );
        dumps_kwargs.insert("indent".to_string(), indent.clone());
        if let Some(default) = default_callable {
            dumps_kwargs.insert("default".to_string(), default);
        }
        let mut rendered = match self.builtin_json_dumps(vec![value], dumps_kwargs)? {
            Value::Str(text) => text,
            other => {
                return Err(RuntimeError::type_error(format!(
                    "_iterencode() expected string output, got {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        if current_indent_level > 0
            && let Value::Str(indent_unit) = &indent
            && !indent_unit.is_empty()
        {
            rendered =
                json_with_extra_indent(&rendered, indent_unit, current_indent_level as usize);
        }
        Ok(self.heap.alloc_list(vec![Value::Str(rendered)]))
    }

    fn json_validate_encoder_for_value(
        &mut self,
        encoder: &Value,
        value: &Value,
    ) -> Result<(), RuntimeError> {
        let mut probe_strings = Vec::new();
        match value {
            Value::Str(text) => probe_strings.push(text.clone()),
            Value::List(obj) => {
                if let Object::List(items) = &*obj.kind() {
                    for item in items {
                        if let Value::Str(text) = item {
                            probe_strings.push(text.clone());
                        }
                    }
                }
            }
            Value::Tuple(obj) => {
                if let Object::Tuple(items) = &*obj.kind() {
                    for item in items {
                        if let Value::Str(text) = item {
                            probe_strings.push(text.clone());
                        }
                    }
                }
            }
            Value::Dict(obj) => {
                if let Object::Dict(items) = &*obj.kind() {
                    for (key, nested) in items.iter() {
                        if let Value::Str(text) = key {
                            probe_strings.push(text.clone());
                        }
                        if let Value::Str(text) = nested {
                            probe_strings.push(text.clone());
                        }
                    }
                }
            }
            _ => {}
        }
        for text in probe_strings {
            let encoded = match self.call_internal(
                encoder.clone(),
                vec![Value::Str(text)],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception("encoder() failed"));
                }
            };
            if !matches!(encoded, Value::Str(_)) {
                return Err(RuntimeError::type_error(format!(
                    "encoder() must return a string, not {}",
                    self.value_type_name_for_error(&encoded)
                )));
            }
        }
        Ok(())
    }

    pub(in crate::vm) fn builtin_json_scanner_make_scanner(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::type_error(
                "make_scanner() expects one context argument",
            ));
        }
        let context = if let Some(context) = args.pop() {
            if kwargs.contains_key("context") {
                return Err(RuntimeError::type_error(
                    "make_scanner() got multiple values for argument 'context'",
                ));
            }
            context
        } else if let Some(context) = kwargs.remove("context") {
            context
        } else {
            return Err(RuntimeError::type_error(
                "make_scanner() missing required argument 'context'",
            ));
        };
        if let Some(unexpected) = kwargs.keys().next().cloned() {
            return Err(RuntimeError::type_error(format!(
                "make_scanner() got an unexpected keyword argument '{}'",
                unexpected
            )));
        }

        let strict_value = self.builtin_getattr(
            vec![context.clone(), Value::Str("strict".to_string())],
            HashMap::new(),
        )?;
        let strict = self.truthy_from_value(&strict_value)?;
        let object_hook = self.builtin_getattr(
            vec![context.clone(), Value::Str("object_hook".to_string())],
            HashMap::new(),
        )?;
        let object_pairs_hook = self.builtin_getattr(
            vec![
                context.clone(),
                Value::Str("object_pairs_hook".to_string()),
            ],
            HashMap::new(),
        )?;
        let parse_float = self.builtin_getattr(
            vec![context.clone(), Value::Str("parse_float".to_string())],
            HashMap::new(),
        )?;
        let parse_int = self.builtin_getattr(
            vec![context.clone(), Value::Str("parse_int".to_string())],
            HashMap::new(),
        )?;
        let parse_constant = self.builtin_getattr(
            vec![
                context.clone(),
                Value::Str("parse_constant".to_string()),
            ],
            HashMap::new(),
        )?;
        let parse_object = self.builtin_getattr(
            vec![context.clone(), Value::Str("parse_object".to_string())],
            HashMap::new(),
        )?;
        let parse_array = self.builtin_getattr(
            vec![context.clone(), Value::Str("parse_array".to_string())],
            HashMap::new(),
        )?;
        let parse_string = self.builtin_getattr(
            vec![context, Value::Str("parse_string".to_string())],
            HashMap::new(),
        )?;

        let scanner_class = self.json_scanner_runtime_class()?;
        let scanner = match self.heap.alloc_instance(InstanceObject::new(scanner_class)) {
            Value::Instance(instance) => instance,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *scanner.kind_mut() {
            instance_data
                .attrs
                .insert("strict".to_string(), Value::Bool(strict));
            instance_data
                .attrs
                .insert("object_hook".to_string(), object_hook.clone());
            instance_data
                .attrs
                .insert("object_pairs_hook".to_string(), object_pairs_hook.clone());
            instance_data
                .attrs
                .insert("parse_float".to_string(), parse_float.clone());
            instance_data
                .attrs
                .insert("parse_int".to_string(), parse_int.clone());
            instance_data
                .attrs
                .insert("parse_constant".to_string(), parse_constant.clone());
            instance_data
                .attrs
                .insert(JSON_SCANNER_PARSE_OBJECT_ATTR.to_string(), parse_object);
            instance_data
                .attrs
                .insert(JSON_SCANNER_PARSE_ARRAY_ATTR.to_string(), parse_array);
            instance_data
                .attrs
                .insert(JSON_SCANNER_PARSE_STRING_ATTR.to_string(), parse_string);
        }
        Ok(Value::Instance(scanner))
    }

    pub(in crate::vm) fn builtin_json_scanner_call(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let receiver = self.receiver_from_value(
            args.first()
                .ok_or_else(|| RuntimeError::type_error("scan_once() missing receiver"))?,
        )?;
        args.remove(0);
        let (source, idx) = self.json_scanner_parse_call_args(args, kwargs)?;
        let memo = self.heap.alloc_dict(Vec::new());
        let recursive_state = match self
            .heap
            .alloc_module(ModuleObject::new("__json_scanner_scan_once__".to_string()))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *recursive_state.kind_mut() {
            module_data.globals.insert(
                JSON_SCANNER_RECURSIVE_SCANNER_ATTR.to_string(),
                Value::Instance(receiver.clone()),
            );
            module_data
                .globals
                .insert(JSON_SCANNER_MEMO_ATTR.to_string(), memo.clone());
        }
        let recursive_scan_once =
            self.alloc_builtin_bound_method(BuiltinFunction::JsonScannerScanOnce, recursive_state);
        self.json_scanner_scan_once_impl(&receiver, memo, recursive_scan_once, source, idx)
    }

    pub(in crate::vm) fn builtin_json_scanner_scan_once(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let receiver = self.receiver_from_value(
            args.first()
                .ok_or_else(|| RuntimeError::type_error("scan_once() missing receiver"))?,
        )?;
        args.remove(0);
        let (source, idx) = self.json_scanner_parse_call_args(args, kwargs)?;
        let (scanner, memo) = match &*receiver.kind() {
            Object::Module(module_data) => (
                module_data
                    .globals
                    .get(JSON_SCANNER_RECURSIVE_SCANNER_ATTR)
                    .cloned()
                    .ok_or_else(|| RuntimeError::runtime_error("json scanner state missing"))?,
                module_data
                    .globals
                    .get(JSON_SCANNER_MEMO_ATTR)
                    .cloned()
                    .ok_or_else(|| RuntimeError::runtime_error("json scanner memo missing"))?,
            ),
            _ => {
                return Err(RuntimeError::runtime_error(
                    "json scanner recursive receiver is invalid",
                ));
            }
        };
        let Value::Instance(scanner) = scanner else {
            return Err(RuntimeError::runtime_error(
                "json scanner recursive receiver is invalid",
            ));
        };
        let recursive_scan_once =
            self.alloc_builtin_bound_method(BuiltinFunction::JsonScannerScanOnce, receiver);
        self.json_scanner_scan_once_impl(&scanner, memo, recursive_scan_once, source, idx)
    }

    fn json_scanner_runtime_class(&self) -> Result<ObjRef, RuntimeError> {
        let module = self
            .modules
            .get("_json")
            .cloned()
            .ok_or_else(|| RuntimeError::runtime_error("_json module is unavailable"))?;
        match &*module.kind() {
            Object::Module(module_data) => match module_data.globals.get(JSON_SCANNER_CLASS_ATTR) {
                Some(Value::Class(class)) => Ok(class.clone()),
                _ => Err(RuntimeError::runtime_error(
                    "_json scanner type is unavailable",
                )),
            },
            _ => Err(RuntimeError::runtime_error("_json module is invalid")),
        }
    }

    fn json_scanner_parse_call_args(
        &self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<(String, i64), RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::type_error(
                "scan_once() expects string and index",
            ));
        }
        let source = if !args.is_empty() {
            if kwargs.contains_key("string") {
                return Err(RuntimeError::type_error(
                    "scan_once() got multiple values for argument 'string'",
                ));
            }
            args.remove(0)
        } else if let Some(source) = kwargs.remove("string") {
            source
        } else {
            return Err(RuntimeError::type_error(
                "scan_once() missing required argument 'string'",
            ));
        };
        let idx = if !args.is_empty() {
            if kwargs.contains_key("idx") {
                return Err(RuntimeError::type_error(
                    "scan_once() got multiple values for argument 'idx'",
                ));
            }
            args.remove(0)
        } else if let Some(idx) = kwargs.remove("idx") {
            idx
        } else {
            return Err(RuntimeError::type_error(
                "scan_once() missing required argument 'idx'",
            ));
        };
        if let Some(unexpected) = kwargs.keys().next().cloned() {
            return Err(RuntimeError::type_error(format!(
                "scan_once() got an unexpected keyword argument '{}'",
                unexpected
            )));
        }
        let Value::Str(source) = source else {
            return Err(RuntimeError::type_error(format!(
                "first argument must be a string, not {}",
                self.value_type_name_for_error(&source)
            )));
        };
        Ok((source, value_to_int(idx)?))
    }

    fn json_scanner_state(&self, scanner: &ObjRef) -> Result<JsonScannerState, RuntimeError> {
        let Object::Instance(instance_data) = &*scanner.kind() else {
            return Err(RuntimeError::type_error(
                "json scanner receiver is invalid",
            ));
        };
        let strict = match instance_data.attrs.get("strict") {
            Some(Value::Bool(strict)) => *strict,
            _ => {
                return Err(RuntimeError::runtime_error(
                    "json scanner strict flag is missing",
                ));
            }
        };
        let get_attr = |name: &str| {
            instance_data
                .attrs
                .get(name)
                .cloned()
                .ok_or_else(|| RuntimeError::runtime_error(format!("json scanner attr '{}' missing", name)))
        };
        Ok(JsonScannerState {
            strict,
            object_hook: get_attr("object_hook")?,
            object_pairs_hook: get_attr("object_pairs_hook")?,
            parse_float: get_attr("parse_float")?,
            parse_int: get_attr("parse_int")?,
            parse_constant: get_attr("parse_constant")?,
            parse_object: get_attr(JSON_SCANNER_PARSE_OBJECT_ATTR)?,
            parse_array: get_attr(JSON_SCANNER_PARSE_ARRAY_ATTR)?,
            parse_string: get_attr(JSON_SCANNER_PARSE_STRING_ATTR)?,
        })
    }

    fn json_scanner_call_callback(
        &mut self,
        callable: Value,
        args: Vec<Value>,
        fallback: &str,
    ) -> Result<Value, RuntimeError> {
        match self.call_internal(callable, args, HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception(fallback))
            }
        }
    }

    fn json_scanner_scan_once_impl(
        &mut self,
        scanner: &ObjRef,
        memo: Value,
        recursive_scan_once: Value,
        source: String,
        idx: i64,
    ) -> Result<Value, RuntimeError> {
        if idx < 0 {
            return Err(RuntimeError::value_error("idx cannot be negative"));
        }
        let idx = idx as usize;
        let Some(start_byte) = utf8_char_index_to_byte(&source, idx) else {
            return Err(self.stop_iteration_runtime_error(Value::Int(idx as i64)));
        };
        let state = self.json_scanner_state(scanner)?;
        let bytes = source.as_bytes();
        let Some(&first) = bytes.get(start_byte) else {
            return Err(self.stop_iteration_runtime_error(Value::Int(idx as i64)));
        };

        let tuple_result = match first {
            b'"' => self.json_scanner_call_callback(
                state.parse_string,
                vec![
                    Value::Str(source),
                    Value::Int(idx as i64 + 1),
                    Value::Bool(state.strict),
                ],
                "json scanner string parse failed",
            )?,
            b'{' => self.json_scanner_call_callback(
                state.parse_object,
                vec![
                    self.heap
                        .alloc_tuple(vec![Value::Str(source), Value::Int(idx as i64 + 1)]),
                    Value::Bool(state.strict),
                    recursive_scan_once,
                    state.object_hook,
                    state.object_pairs_hook,
                    memo,
                ],
                "json scanner object parse failed",
            )?,
            b'[' => self.json_scanner_call_callback(
                state.parse_array,
                vec![
                    self.heap
                        .alloc_tuple(vec![Value::Str(source), Value::Int(idx as i64 + 1)]),
                    recursive_scan_once,
                ],
                "json scanner array parse failed",
            )?,
            b'n' if bytes.get(start_byte..start_byte + 4) == Some(b"null") => {
                self.heap.alloc_tuple(vec![Value::None, Value::Int(idx as i64 + 4)])
            }
            b't' if bytes.get(start_byte..start_byte + 4) == Some(b"true") => {
                self.heap.alloc_tuple(vec![Value::Bool(true), Value::Int(idx as i64 + 4)])
            }
            b'f' if bytes.get(start_byte..start_byte + 5) == Some(b"false") => {
                self.heap.alloc_tuple(vec![Value::Bool(false), Value::Int(idx as i64 + 5)])
            }
            b'N' if bytes.get(start_byte..start_byte + 3) == Some(b"NaN") => {
                let value = self.json_scanner_call_callback(
                    state.parse_constant,
                    vec![Value::Str("NaN".to_string())],
                    "json scanner parse_constant failed",
                )?;
                self.heap
                    .alloc_tuple(vec![value, Value::Int(idx as i64 + 3)])
            }
            b'I' if bytes.get(start_byte..start_byte + 8) == Some(b"Infinity") => {
                let value = self.json_scanner_call_callback(
                    state.parse_constant,
                    vec![Value::Str("Infinity".to_string())],
                    "json scanner parse_constant failed",
                )?;
                self.heap
                    .alloc_tuple(vec![value, Value::Int(idx as i64 + 8)])
            }
            b'-' if bytes.get(start_byte..start_byte + 9) == Some(b"-Infinity") => {
                let value = self.json_scanner_call_callback(
                    state.parse_constant,
                    vec![Value::Str("-Infinity".to_string())],
                    "json scanner parse_constant failed",
                )?;
                self.heap
                    .alloc_tuple(vec![value, Value::Int(idx as i64 + 9)])
            }
            b'-' | b'0'..=b'9' => {
                let Some(end_byte) = json_scan_number_end_byte(&source, start_byte) else {
                    return Err(self.stop_iteration_runtime_error(Value::Int(idx as i64)));
                };
                let number_text = source[start_byte..end_byte].to_string();
                let end_char = utf8_byte_index_to_char(&source, end_byte)
                    .ok_or_else(|| RuntimeError::runtime_error("json scanner index is invalid"))?;
                let value = if number_text
                    .bytes()
                    .any(|byte| matches!(byte, b'.' | b'e' | b'E'))
                {
                    self.json_scanner_call_callback(
                        state.parse_float,
                        vec![Value::Str(number_text)],
                        "json scanner parse_float failed",
                    )?
                } else {
                    self.json_scanner_call_callback(
                        state.parse_int,
                        vec![Value::Str(number_text)],
                        "json scanner parse_int failed",
                    )?
                };
                self.heap
                    .alloc_tuple(vec![value, Value::Int(end_char as i64)])
            }
            _ => return Err(self.stop_iteration_runtime_error(Value::Int(idx as i64))),
        };

        Ok(tuple_result)
    }

    pub(in crate::vm) fn builtin_json_decoder_scanstring(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() >= 3 {
            let strict = self.truthy_from_value(&args[2])?;
            args[2] = Value::Bool(strict);
        }
        if let Some(strict_kw) = kwargs.get("strict").cloned() {
            let strict = self.truthy_from_value(&strict_kw)?;
            kwargs.insert("strict".to_string(), Value::Bool(strict));
        }
        let caller_depth = self.frames.len();
        if let Ok(decoder_module) = self.import_module_object("json.decoder")
            && let Ok(decoder_module) = self.return_imported_module(decoder_module, caller_depth)
            && let Object::Module(module_data) = &*decoder_module.kind()
            && let Some(py_scanstring) = module_data.globals.get("py_scanstring").cloned()
        {
            let recursive_builtin = matches!(
                py_scanstring,
                Value::Builtin(BuiltinFunction::JsonDecoderScanString)
            );
            if !recursive_builtin {
                match self.call_internal(py_scanstring, args.clone(), kwargs.clone())? {
                    InternalCallOutcome::Value(value) => return Ok(value),
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(RuntimeError::runtime_error(
                            "scanstring() callback did not return a value",
                        ));
                    }
                }
            }
        }
        self.builtin_json_decoder_scanstring_native(args, kwargs)
    }

    fn builtin_json_decoder_scanstring_native(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "scanstring() expects string, index, and optional strict",
            ));
        }
        let strict_from_kwargs = kwargs.remove("strict");
        let mut strict = strict_from_kwargs
            .as_ref()
            .map(|value| self.truthy_from_value(value))
            .transpose()?
            .unwrap_or(true);
        if args.len() == 3 {
            if strict_from_kwargs.is_some() {
                return Err(RuntimeError::new(
                    "scanstring() got multiple values for argument 'strict'",
                ));
            }
            strict = self.truthy_from_value(&args[2])?;
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "scanstring() got an unexpected keyword argument",
            ));
        }

        let source = match &args[0] {
            Value::Str(text) => text.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "scanstring() expects string source and integer index",
                ));
            }
        };
        let end_char = value_to_int(args.remove(1))?;
        if end_char < 1 {
            return Err(RuntimeError::new(
                "scanstring() expects index after opening quote",
            ));
        }
        let end_char = end_char as usize;
        let start_quote_char = end_char - 1;
        let start_quote_byte = utf8_char_index_to_byte(&source, start_quote_char)
            .ok_or_else(|| RuntimeError::new("scanstring() start index is out of range"))?;
        let depth_limit = json_parse_depth_limit(self);
        let (node, end_byte) =
            parse_json_node_from_index_with_limit(&source, start_quote_byte, depth_limit).map_err(
                |err| {
                    if json_parse_error_is_recursion_limit(&err) {
                        self.recursion_limit_error()
                    } else {
                        err
                    }
                },
            )?;
        let value = match node {
            JsonNode::String(text) => text,
            _ => {
                return Err(RuntimeError::new(
                    "scanstring() did not parse a JSON string",
                ));
            }
        };
        if strict
            && json_string_has_unescaped_ascii_control_char(&source, start_quote_byte, end_byte)
        {
            return Err(RuntimeError::new(
                "Invalid control character in JSON string",
            ));
        }
        let end_char = utf8_byte_index_to_char(&source, end_byte)
            .ok_or_else(|| RuntimeError::new("scanstring() end index is out of range"))?;
        Ok(self
            .heap
            .alloc_tuple(vec![Value::Str(value), Value::Int(end_char as i64)]))
    }
}

fn json_source_text(value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::Str(text) => Ok(text.clone()),
        Value::Bytes(obj) => match &*obj.kind() {
            Object::Bytes(bytes) => String::from_utf8(bytes.clone())
                .map_err(|_| RuntimeError::new("loads() bytes must be valid UTF-8")),
            _ => Err(RuntimeError::new(
                "loads() expects str, bytes, or bytearray",
            )),
        },
        Value::ByteArray(obj) => match &*obj.kind() {
            Object::ByteArray(bytes) => String::from_utf8(bytes.clone())
                .map_err(|_| RuntimeError::new("loads() bytearray must be valid UTF-8")),
            _ => Err(RuntimeError::new(
                "loads() expects str, bytes, or bytearray",
            )),
        },
        _ => Err(RuntimeError::new(
            "loads() expects str, bytes, or bytearray",
        )),
    }
}

fn json_with_extra_indent(rendered: &str, indent_unit: &str, extra_levels: usize) -> String {
    if extra_levels == 0 || rendered.is_empty() {
        return rendered.to_string();
    }
    let prefix = indent_unit.repeat(extra_levels);
    let mut out =
        String::with_capacity(rendered.len() + prefix.len() * rendered.matches('\n').count());
    for (idx, line) in rendered.split('\n').enumerate() {
        if idx > 0 {
            out.push('\n');
            out.push_str(&prefix);
        }
        out.push_str(line);
    }
    out
}

fn parse_json_separators(value: Value) -> Result<(String, String), RuntimeError> {
    let values = match value {
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(values) => values.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "dumps() separators must be a 2-item tuple",
                ));
            }
        },
        Value::List(obj) => match &*obj.kind() {
            Object::List(values) => values.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "dumps() separators must be a 2-item tuple",
                ));
            }
        },
        _ => {
            return Err(RuntimeError::new(
                "dumps() separators must be a 2-item tuple",
            ));
        }
    };

    if values.len() != 2 {
        return Err(RuntimeError::new(
            "dumps() separators must be a 2-item tuple",
        ));
    }

    let item_sep = match &values[0] {
        Value::Str(text) => text.clone(),
        _ => {
            return Err(RuntimeError::new(
                "dumps() separators items must be strings",
            ));
        }
    };
    let key_sep = match &values[1] {
        Value::Str(text) => text.clone(),
        _ => {
            return Err(RuntimeError::new(
                "dumps() separators items must be strings",
            ));
        }
    };
    Ok((item_sep, key_sep))
}

fn parse_json_indent(value: Value) -> Result<Option<String>, RuntimeError> {
    match value {
        Value::None => Ok(None),
        Value::Str(text) => Ok(Some(text)),
        other => {
            let width = value_to_int(other)?;
            if width <= 0 {
                Ok(Some(String::new()))
            } else {
                Ok(Some(" ".repeat(width as usize)))
            }
        }
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::None => "NoneType",
        Value::Bool(_) => "bool",
        Value::Int(_) | Value::BigInt(_) => "int",
        Value::Float(_) => "float",
        Value::Complex { .. } => "complex",
        Value::Str(_) => "str",
        Value::List(_) => "list",
        Value::Tuple(_) => "tuple",
        Value::Dict(_) => "dict",
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
        Value::BoundMethod(_) => "method",
        Value::Exception(_) | Value::ExceptionType(_) => "exception",
        Value::Code(_) => "code",
        Value::Function(_) | Value::Builtin(_) => "function",
        Value::Cell(_) => "cell",
        Value::Slice { .. } => "slice",
        Value::DictKeys(_) => "dict_keys",
        Value::DictValues(_) => "dict_values",
        Value::DictItems(_) => "dict_items",
    }
}

fn json_serialization_note_for_value(vm: &Vm, value: &Value) -> String {
    let type_name = match value {
        Value::Class(_) => "type".to_string(),
        _ => vm.value_type_name_for_error(value).to_ascii_lowercase(),
    };
    format!("when serializing {type_name} object")
}

fn json_append_exception_note(vm: &mut Vm, err: &mut RuntimeError, note: String) {
    let Some(exception) = err.exception.as_mut() else {
        return;
    };
    let note_value = Value::Str(note.clone());
    let default_notes_list = vm.heap.alloc_list(vec![note_value.clone()]);
    let mut applied = false;
    {
        let mut attrs = exception.attrs.borrow_mut();
        match attrs.get_mut("__notes__") {
            Some(Value::List(notes_obj)) => {
                if let Object::List(notes) = &mut *notes_obj.kind_mut() {
                    notes.push(note_value);
                    applied = true;
                }
            }
            Some(_) => {}
            None => {
                attrs.insert("__notes__".to_string(), default_notes_list);
                applied = true;
            }
        }
    }
    if applied {
        exception.notes.push(note);
    }
}

fn json_marker_id(value: &Value) -> Option<u64> {
    match value {
        Value::List(obj) | Value::Tuple(obj) | Value::Dict(obj) => Some(obj.id()),
        Value::Instance(obj) => Some(obj.id()),
        Value::Class(obj) => Some(obj.id()),
        _ => None,
    }
}

fn json_enter_marker(
    markers: &mut HashSet<u64>,
    value: &Value,
) -> Result<Option<u64>, RuntimeError> {
    let Some(marker_id) = json_marker_id(value) else {
        return Ok(None);
    };
    if !markers.insert(marker_id) {
        return Err(RuntimeError::value_error("Circular reference detected"));
    }
    Ok(Some(marker_id))
}

fn json_exit_marker(markers: &mut HashSet<u64>, marker_id: Option<u64>) {
    if let Some(marker_id) = marker_id {
        markers.remove(&marker_id);
    }
}

fn json_is_builtin_class_named(value: &Value, expected: &str) -> bool {
    match value {
        Value::Class(class) => match &*class.kind() {
            Object::Class(class_data) => {
                class_data.name == expected
                    && matches!(
                        class_data.attrs.get("__module__"),
                        Some(Value::Str(module_name)) if module_name == "builtins"
                    )
            }
            _ => false,
        },
        _ => false,
    }
}

fn json_is_default_parse_float(value: &Value) -> bool {
    matches!(value, Value::Builtin(BuiltinFunction::Float))
        || json_is_builtin_class_named(value, "float")
}

fn json_is_default_parse_int(value: &Value) -> bool {
    matches!(value, Value::Builtin(BuiltinFunction::Int))
        || json_is_builtin_class_named(value, "int")
}

fn json_is_default_parse_constant(value: &Value) -> bool {
    match value {
        Value::BoundMethod(method) => match &*method.kind() {
            Object::BoundMethod(method_data) => {
                matches!(&*method_data.receiver.kind(), Object::Dict(_))
            }
            _ => false,
        },
        _ => false,
    }
}

fn json_escape_string(text: &str, ensure_ascii: bool) -> String {
    let mut out = String::new();
    out.push('"');
    for ch in text.chars() {
        if let Some(code_unit) = surrogate_code_unit_from_internal_char(ch) {
            if ensure_ascii {
                out.push_str(&format!("\\u{code_unit:04x}"));
            } else {
                out.push(ch);
            }
            continue;
        }
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) <= 0x1F => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if ensure_ascii && (!c.is_ascii() || c == '\u{007F}') => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units).iter() {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_nonfinite_float_json_literal(value: f64) -> Option<&'static str> {
    if value.is_nan() {
        Some("NaN")
    } else if value.is_infinite() {
        if value.is_sign_negative() {
            Some("-Infinity")
        } else {
            Some("Infinity")
        }
    } else {
        None
    }
}

fn json_nonfinite_float_error_literal(value: f64) -> Option<&'static str> {
    if value.is_nan() {
        Some("nan")
    } else if value.is_infinite() {
        if value.is_sign_negative() {
            Some("-inf")
        } else {
            Some("inf")
        }
    } else {
        None
    }
}

fn json_format_finite_float(value: f64) -> String {
    format_repr(&Value::Float(value))
}

fn json_encode_object_key(
    vm: &Vm,
    key: &Value,
    options: &JsonDumpsOptions,
) -> Result<Option<String>, RuntimeError> {
    match key {
        Value::Str(text) => Ok(Some(text.clone())),
        Value::Int(value) => Ok(Some(value.to_string())),
        Value::BigInt(value) => Ok(Some(value.to_string())),
        Value::Float(value) => {
            if value.is_finite() {
                Ok(Some(json_format_finite_float(*value)))
            } else if options.allow_nan {
                Ok(Some(
                    json_nonfinite_float_json_literal(*value)
                        .expect("non-finite float should map to a JSON literal")
                        .to_string(),
                ))
            } else {
                let suffix = json_nonfinite_float_error_literal(*value)
                    .expect("non-finite float should map to an error literal");
                Err(RuntimeError::value_error(format!(
                    "Out of range float values are not JSON compliant: {suffix}"
                )))
            }
        }
        Value::Bool(value) => Ok(Some(if *value {
            "true".to_string()
        } else {
            "false".to_string()
        })),
        Value::None => Ok(Some("null".to_string())),
        Value::Instance(instance) => {
            if let Some(backing_str) = vm.instance_backing_str(instance) {
                return Ok(Some(backing_str));
            }
            if let Some(backing_int) = vm.instance_backing_int(instance) {
                return json_encode_object_key(vm, &backing_int, options);
            }
            if let Some(backing_float) = vm.instance_backing_float(instance) {
                return json_encode_object_key(vm, &Value::Float(backing_float), options);
            }
            if options.skipkeys {
                Ok(None)
            } else {
                Err(RuntimeError::type_error(format!(
                    "keys must be str, int, float, bool or None, not {}",
                    json_type_name(key)
                )))
            }
        }
        _ if options.skipkeys => Ok(None),
        _ => Err(RuntimeError::type_error(format!(
            "keys must be str, int, float, bool or None, not {}",
            json_type_name(key)
        ))),
    }
}

fn json_serialize_via_default(
    vm: &mut Vm,
    value: &Value,
    options: &JsonDumpsOptions,
    default: Option<&Value>,
    depth: usize,
    markers: &mut HashSet<u64>,
) -> Result<String, RuntimeError> {
    if let Some(default_callable) = default {
        let marker = json_enter_marker(markers, value)?;
        let call_result = vm.call_internal(
            default_callable.clone(),
            vec![value.clone()],
            HashMap::new(),
        );
        let result = match call_result {
            Ok(InternalCallOutcome::Value(converted)) => {
                match json_serialize_value_at_depth(
                    vm,
                    &converted,
                    options,
                    default,
                    depth.saturating_add(1),
                    markers,
                ) {
                    Ok(encoded) => Ok(encoded),
                    Err(mut err) => {
                        let note = json_serialization_note_for_value(vm, value);
                        json_append_exception_note(vm, &mut err, note);
                        Err(err)
                    }
                }
            }
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                let mut err =
                    vm.runtime_error_from_active_exception("dumps() default callback failed");
                let note = json_serialization_note_for_value(vm, value);
                json_append_exception_note(vm, &mut err, note);
                Err(err)
            }
            Err(err) => Err(err),
        };
        json_exit_marker(markers, marker);
        result
    } else {
        Err(RuntimeError::new(format!(
            "Object of type {} is not JSON serializable",
            json_type_name(value)
        )))
    }
}

fn json_sort_object_items(
    vm: &mut Vm,
    items: &mut [(Value, String, Value)],
) -> Result<(), RuntimeError> {
    for index in 1..items.len() {
        let mut cursor = index;
        while cursor > 0 {
            let right_key = items[cursor].0.clone();
            let left_key = items[cursor - 1].0.clone();
            let less_than = vm.compare_lt_runtime(right_key, left_key)?;
            if !vm.truthy_from_value(&less_than)? {
                break;
            }
            items.swap(cursor - 1, cursor);
            cursor -= 1;
        }
    }
    Ok(())
}

/// Serialize a runtime value into JSON text using current option semantics.
fn json_serialize_value(
    vm: &mut Vm,
    value: &Value,
    options: &JsonDumpsOptions,
    default: Option<&Value>,
) -> Result<String, RuntimeError> {
    let mut markers = HashSet::new();
    json_serialize_value_at_depth(vm, value, options, default, 0, &mut markers)
}

fn json_serialize_value_at_depth(
    vm: &mut Vm,
    value: &Value,
    options: &JsonDumpsOptions,
    default: Option<&Value>,
    depth: usize,
    markers: &mut HashSet<u64>,
) -> Result<String, RuntimeError> {
    // CPython wraps recursive _json encode/decode descent in
    // _Py_EnterRecursiveCall(); use the VM's stack-safe effective limit here
    // so deep accelerator recursion raises RecursionError before native Rust
    // stack overflow.
    let recursion_limit = vm.effective_recursion_limit() as usize;
    if depth >= recursion_limit {
        return Err(vm.recursion_limit_error());
    }

    match value {
        Value::None => Ok("null".to_string()),
        Value::Bool(value) => Ok(if *value {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        Value::Int(value) => Ok(value.to_string()),
        Value::BigInt(value) => Ok(value.to_string()),
        Value::Float(value) => {
            if value.is_finite() {
                Ok(json_format_finite_float(*value))
            } else if options.allow_nan {
                Ok(json_nonfinite_float_json_literal(*value)
                    .expect("non-finite float should map to a JSON literal")
                    .to_string())
            } else {
                let suffix = json_nonfinite_float_error_literal(*value)
                    .expect("non-finite float should map to an error literal");
                Err(RuntimeError::value_error(format!(
                    "Out of range float values are not JSON compliant: {suffix}"
                )))
            }
        }
        Value::Str(value) => Ok(json_escape_string(value, options.ensure_ascii)),
        Value::List(obj) => {
            let marker = json_enter_marker(markers, value)?;
            let result = (|| {
                if !matches!(&*obj.kind(), Object::List(_)) {
                    return Err(RuntimeError::new("json unsupported type"));
                }
                let mut parts = Vec::new();
                let mut index = 0usize;
                loop {
                    let current = {
                        let list_kind = obj.kind();
                        let Object::List(values) = &*list_kind else {
                            return Err(RuntimeError::new("json unsupported type"));
                        };
                        values.get(index).cloned()
                    };
                    let Some(value) = current else {
                        break;
                    };
                    match json_serialize_value_at_depth(
                        vm,
                        &value,
                        options,
                        default,
                        depth + 1,
                        markers,
                    ) {
                        Ok(encoded) => parts.push(encoded),
                        Err(mut err) => {
                            json_append_exception_note(
                                vm,
                                &mut err,
                                format!("when serializing list item {index}"),
                            );
                            return Err(err);
                        }
                    }
                    index += 1;
                }
                if let Some(indent_unit) = options.indent.as_ref() {
                    if parts.is_empty() {
                        return Ok("[]".to_string());
                    }
                    let child_indent = indent_unit.repeat(depth + 1);
                    let parent_indent = indent_unit.repeat(depth);
                    let mut out = String::new();
                    out.push('[');
                    out.push('\n');
                    out.push_str(&child_indent);
                    out.push_str(&parts[0]);
                    for part in parts.iter().skip(1) {
                        out.push_str(&options.item_separator);
                        out.push('\n');
                        out.push_str(&child_indent);
                        out.push_str(part);
                    }
                    out.push('\n');
                    out.push_str(&parent_indent);
                    out.push(']');
                    Ok(out)
                } else {
                    Ok(format!("[{}]", parts.join(&options.item_separator)))
                }
            })();
            json_exit_marker(markers, marker);
            result
        }
        Value::Tuple(obj) => {
            let marker = json_enter_marker(markers, value)?;
            let result = (|| {
                let values = {
                    let tuple_kind = obj.kind();
                    let Object::Tuple(values) = &*tuple_kind else {
                        return Err(RuntimeError::new("json unsupported type"));
                    };
                    values.to_vec()
                };
                let mut parts = Vec::with_capacity(values.len());
                for (index, value) in values.into_iter().enumerate() {
                    match json_serialize_value_at_depth(
                        vm,
                        &value,
                        options,
                        default,
                        depth + 1,
                        markers,
                    ) {
                        Ok(encoded) => parts.push(encoded),
                        Err(mut err) => {
                            json_append_exception_note(
                                vm,
                                &mut err,
                                format!("when serializing tuple item {index}"),
                            );
                            return Err(err);
                        }
                    }
                }
                if let Some(indent_unit) = options.indent.as_ref() {
                    if parts.is_empty() {
                        return Ok("[]".to_string());
                    }
                    let child_indent = indent_unit.repeat(depth + 1);
                    let parent_indent = indent_unit.repeat(depth);
                    let mut out = String::new();
                    out.push('[');
                    out.push('\n');
                    out.push_str(&child_indent);
                    out.push_str(&parts[0]);
                    for part in parts.iter().skip(1) {
                        out.push_str(&options.item_separator);
                        out.push('\n');
                        out.push_str(&child_indent);
                        out.push_str(part);
                    }
                    out.push('\n');
                    out.push_str(&parent_indent);
                    out.push(']');
                    Ok(out)
                } else {
                    Ok(format!("[{}]", parts.join(&options.item_separator)))
                }
            })();
            json_exit_marker(markers, marker);
            result
        }
        Value::Dict(obj) => {
            let marker = json_enter_marker(markers, value)?;
            let result = (|| {
                let mut mapped: Vec<(Value, String, Value)> = {
                    let dict_kind = obj.kind();
                    let Object::Dict(entries) = &*dict_kind else {
                        return Err(RuntimeError::new("json unsupported type"));
                    };
                    let mut mapped: Vec<(Value, String, Value)> = Vec::with_capacity(entries.len());
                    for (key, value) in entries.iter() {
                        if let Some(encoded_key) = json_encode_object_key(vm, key, options)? {
                            mapped.push((key.clone(), encoded_key, value.clone()));
                        }
                    }
                    mapped
                };
                if options.sort_keys {
                    json_sort_object_items(vm, &mut mapped)?;
                }
                let mut parts = Vec::with_capacity(mapped.len());
                for (_raw_key, key, value) in mapped {
                    let encoded_key = json_escape_string(&key, options.ensure_ascii);
                    let encoded_value = match json_serialize_value_at_depth(
                        vm,
                        &value,
                        options,
                        default,
                        depth + 1,
                        markers,
                    ) {
                        Ok(encoded) => encoded,
                        Err(mut err) => {
                            json_append_exception_note(
                                vm,
                                &mut err,
                                format!(
                                    "when serializing dict item {}",
                                    format_repr(&Value::Str(key.clone()))
                                ),
                            );
                            return Err(err);
                        }
                    };
                    parts.push(format!(
                        "{}{sep}{}",
                        encoded_key,
                        encoded_value,
                        sep = options.key_separator
                    ));
                }
                if let Some(indent_unit) = options.indent.as_ref() {
                    if parts.is_empty() {
                        return Ok("{}".to_string());
                    }
                    let child_indent = indent_unit.repeat(depth + 1);
                    let parent_indent = indent_unit.repeat(depth);
                    let mut out = String::new();
                    out.push('{');
                    out.push('\n');
                    out.push_str(&child_indent);
                    out.push_str(&parts[0]);
                    for part in parts.iter().skip(1) {
                        out.push_str(&options.item_separator);
                        out.push('\n');
                        out.push_str(&child_indent);
                        out.push_str(part);
                    }
                    out.push('\n');
                    out.push_str(&parent_indent);
                    out.push('}');
                    Ok(out)
                } else {
                    Ok(format!("{{{}}}", parts.join(&options.item_separator)))
                }
            })();
            json_exit_marker(markers, marker);
            result
        }
        Value::Instance(instance) => {
            if let Some(backing_str) = vm.instance_backing_str(instance) {
                return json_serialize_value_at_depth(
                    vm,
                    &Value::Str(backing_str),
                    options,
                    default,
                    depth,
                    markers,
                );
            }
            if let Some(backing_int) = vm.instance_backing_int(instance) {
                return json_serialize_value_at_depth(
                    vm,
                    &backing_int,
                    options,
                    default,
                    depth,
                    markers,
                );
            }
            if let Some(backing_float) = vm.instance_backing_float(instance) {
                return json_serialize_value_at_depth(
                    vm,
                    &Value::Float(backing_float),
                    options,
                    default,
                    depth,
                    markers,
                );
            }
            if let Some(backing_list) = vm.instance_backing_list(instance) {
                return json_serialize_value_at_depth(
                    vm,
                    &Value::List(backing_list),
                    options,
                    default,
                    depth,
                    markers,
                );
            }
            if let Some(backing_tuple) = vm.instance_backing_tuple(instance) {
                return json_serialize_value_at_depth(
                    vm,
                    &Value::Tuple(backing_tuple),
                    options,
                    default,
                    depth,
                    markers,
                );
            }
            if let Some(backing_dict) = vm.instance_backing_dict(instance) {
                return json_serialize_value_at_depth(
                    vm,
                    &Value::Dict(backing_dict),
                    options,
                    default,
                    depth,
                    markers,
                );
            }
            json_serialize_via_default(vm, value, options, default, depth, markers)
        }
        _ => json_serialize_via_default(vm, value, options, default, depth, markers),
    }
}

fn json_parse_depth_limit(vm: &Vm) -> usize {
    vm.effective_recursion_limit() as usize
}

fn json_parse_error_is_recursion_limit(err: &RuntimeError) -> bool {
    err.message.contains("maximum recursion depth exceeded")
}

fn json_validate_int_digits_limit(digits: &str) -> Result<(), RuntimeError> {
    let limit = runtime_get_int_max_str_digits();
    if limit > 0 && digits.len() > limit as usize {
        return Err(RuntimeError::value_error(format!(
            "Exceeds the limit ({} digits) for integer string conversion: value has {} digits; use sys.set_int_max_str_digits() to increase the limit",
            limit,
            digits.len()
        )));
    }
    Ok(())
}

fn json_decode_error_runtime_error(message: &str, source: &str, pos: usize) -> RuntimeError {
    let prefix_end = source
        .char_indices()
        .take_while(|(idx, _)| *idx < pos)
        .map(|(idx, ch)| idx + ch.len_utf8())
        .last()
        .unwrap_or(0);
    let prefix = &source[..prefix_end.min(source.len())];
    let lineno = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let last_newline = prefix.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
    let colno = source[last_newline..prefix_end.min(source.len())]
        .chars()
        .count()
        + 1;

    let rendered = format!("{message}: line {lineno} column {colno} (char {pos})");
    let exception = ExceptionObject::new("JSONDecodeError", Some(rendered));
    {
        let mut attrs = exception.attrs.borrow_mut();
        attrs.insert("msg".to_string(), Value::Str(message.to_string()));
        attrs.insert("doc".to_string(), Value::Str(source.to_string()));
        attrs.insert("pos".to_string(), Value::Int(pos as i64));
        attrs.insert("lineno".to_string(), Value::Int(lineno as i64));
        attrs.insert("colno".to_string(), Value::Int(colno as i64));
    }
    RuntimeError::from_exception(exception)
}

#[derive(Debug)]
enum JsonNode {
    Null,
    Bool(bool),
    Int(i64),
    BigInt(BigInt),
    Float(f64),
    String(String),
    Array(Vec<JsonNode>),
    Object(Vec<(String, JsonNode)>),
}

const JSON_PARSE_NATIVE_STACK_SAFE_DEPTH: usize =
    super::super::VM_STACK_SAFE_RECURSION_LIMIT as usize;

#[derive(Debug, Clone)]
struct JsonParseError {
    message: String,
    pos: usize,
}

impl JsonParseError {
    fn new(message: impl Into<String>, pos: usize) -> Self {
        Self {
            message: message.into(),
            pos,
        }
    }
}

fn json_runtime_error_from_parse(err: JsonParseError) -> RuntimeError {
    RuntimeError::new(err.message)
}

/// Small recursive-descent parser for JSON text input.
struct JsonParser<'a> {
    source: &'a [u8],
    pos: usize,
    depth_limit: usize,
}

impl<'a> JsonParser<'a> {
    fn new(source: &'a str, depth_limit: usize) -> Self {
        Self {
            source: source.as_bytes(),
            pos: 0,
            depth_limit: depth_limit.max(1),
        }
    }

    /// Parse the full input and require trailing-whitespace-only remainder.
    fn parse(mut self) -> Result<JsonNode, JsonParseError> {
        self.skip_ws();
        let value = self.parse_value(0)?;
        self.skip_ws();
        if self.pos != self.source.len() {
            return Err(JsonParseError::new("Extra data", self.pos));
        }
        Ok(value)
    }

    fn parse_value(&mut self, depth: usize) -> Result<JsonNode, JsonParseError> {
        if depth >= self.depth_limit {
            return Err(JsonParseError::new(
                "maximum recursion depth exceeded",
                self.pos,
            ));
        }
        self.skip_ws();
        if self.source.get(self.pos..self.pos + 9) == Some(b"-Infinity") {
            self.pos += 9;
            return Ok(JsonNode::Float(f64::NEG_INFINITY));
        }
        let byte = self
            .peek()
            .ok_or_else(|| JsonParseError::new("Expecting value", self.pos))?;
        match byte {
            b'n' => self.parse_literal(b"null", JsonNode::Null),
            b't' => self.parse_literal(b"true", JsonNode::Bool(true)),
            b'f' => self.parse_literal(b"false", JsonNode::Bool(false)),
            b'N' => self.parse_literal(b"NaN", JsonNode::Float(f64::NAN)),
            b'I' => self.parse_literal(b"Infinity", JsonNode::Float(f64::INFINITY)),
            b'"' => self.parse_string().map(JsonNode::String),
            b'[' => self.parse_array(depth + 1),
            b'{' => self.parse_object(depth + 1),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(JsonParseError::new("Expecting value", self.pos)),
        }
    }

    fn parse_literal(&mut self, text: &[u8], node: JsonNode) -> Result<JsonNode, JsonParseError> {
        if self.source.get(self.pos..self.pos + text.len()) == Some(text) {
            self.pos += text.len();
            Ok(node)
        } else {
            Err(JsonParseError::new("Expecting value", self.pos))
        }
    }

    fn parse_string(&mut self) -> Result<String, JsonParseError> {
        let start_quote_pos = self.pos;
        self.expect(b'"', "Expecting value")?;
        let mut out = String::new();
        while let Some(byte) = self.next() {
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self
                        .next()
                        .ok_or_else(|| JsonParseError::new("invalid JSON escape", self.pos))?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex_u16()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                let pair_start = self.pos;
                                if self.source.get(self.pos) == Some(&b'\\')
                                    && self.source.get(self.pos + 1) == Some(&b'u')
                                {
                                    self.pos += 2;
                                    let low = self.parse_hex_u16()?;
                                    if (0xDC00..=0xDFFF).contains(&low) {
                                        let scalar = 0x10000
                                            + (((code as u32 - 0xD800) << 10)
                                                | (low as u32 - 0xDC00));
                                        let ch = char::from_u32(scalar).ok_or_else(|| {
                                            JsonParseError::new("invalid unicode escape", self.pos)
                                        })?;
                                        out.push(ch);
                                        continue;
                                    }
                                    // Keep the trailing \uXXXX escape for the next loop pass.
                                    self.pos = pair_start;
                                }
                                let ch =
                                    internal_char_from_codepoint(code as u32).ok_or_else(|| {
                                        JsonParseError::new("invalid unicode escape", self.pos)
                                    })?;
                                out.push(ch);
                            } else if (0xDC00..=0xDFFF).contains(&code) {
                                let ch =
                                    internal_char_from_codepoint(code as u32).ok_or_else(|| {
                                        JsonParseError::new("invalid unicode escape", self.pos)
                                    })?;
                                out.push(ch);
                            } else {
                                let ch = char::from_u32(code as u32).ok_or_else(|| {
                                    JsonParseError::new("invalid unicode escape", self.pos)
                                })?;
                                out.push(ch);
                            }
                        }
                        _ => return Err(JsonParseError::new("invalid JSON escape", self.pos)),
                    }
                }
                b if b < 0x20 => {
                    return Err(JsonParseError::new(
                        "invalid control character in JSON string",
                        self.pos.saturating_sub(1),
                    ));
                }
                b if b < 0x80 => out.push(b as char),
                b => self.push_utf8_char(b, &mut out)?,
            }
        }
        Err(JsonParseError::new(
            "Unterminated string starting at",
            start_quote_pos,
        ))
    }

    fn push_utf8_char(&mut self, first: u8, out: &mut String) -> Result<(), JsonParseError> {
        let width = if first >> 5 == 0b110 {
            2
        } else if first >> 4 == 0b1110 {
            3
        } else if first >> 3 == 0b11110 {
            4
        } else {
            return Err(JsonParseError::new(
                "invalid UTF-8 in JSON string",
                self.pos,
            ));
        };
        let mut bytes = vec![first];
        for _ in 1..width {
            let next = self
                .next()
                .ok_or_else(|| JsonParseError::new("invalid UTF-8 in JSON string", self.pos))?;
            if (next & 0b1100_0000) != 0b1000_0000 {
                return Err(JsonParseError::new(
                    "invalid UTF-8 in JSON string",
                    self.pos,
                ));
            }
            bytes.push(next);
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| JsonParseError::new("invalid UTF-8 in JSON string", self.pos))?;
        let ch = text
            .chars()
            .next()
            .ok_or_else(|| JsonParseError::new("invalid UTF-8 in JSON string", self.pos))?;
        out.push(ch);
        Ok(())
    }

    fn parse_hex_u16(&mut self) -> Result<u16, JsonParseError> {
        let mut value: u16 = 0;
        for _ in 0..4 {
            let byte = self
                .next()
                .ok_or_else(|| JsonParseError::new("invalid unicode escape", self.pos))?;
            value <<= 4;
            value |= match byte {
                b'0'..=b'9' => (byte - b'0') as u16,
                b'a'..=b'f' => (byte - b'a' + 10) as u16,
                b'A'..=b'F' => (byte - b'A' + 10) as u16,
                _ => return Err(JsonParseError::new("invalid unicode escape", self.pos)),
            };
        }
        Ok(value)
    }

    fn parse_array(&mut self, depth: usize) -> Result<JsonNode, JsonParseError> {
        self.expect(b'[', "Expecting value")?;
        self.skip_ws();
        let mut values = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonNode::Array(values));
        }
        let mut trailing_comma_pos: Option<usize> = None;
        loop {
            if let Some(comma_pos) = trailing_comma_pos.take() {
                if self.peek() == Some(b']') {
                    return Err(JsonParseError::new(
                        "Illegal trailing comma before end of array",
                        comma_pos,
                    ));
                }
            }
            values.push(self.parse_value(depth)?);
            self.skip_ws();
            match self.next() {
                Some(b',') => {
                    trailing_comma_pos = Some(self.pos.saturating_sub(1));
                    self.skip_ws();
                    if self.peek().is_none() {
                        return Err(JsonParseError::new("Expecting value", self.pos));
                    }
                }
                Some(b']') => break,
                Some(_) => {
                    return Err(JsonParseError::new(
                        "Expecting ',' delimiter",
                        self.pos.saturating_sub(1),
                    ));
                }
                None => return Err(JsonParseError::new("Expecting ',' delimiter", self.pos)),
            }
        }
        Ok(JsonNode::Array(values))
    }

    fn parse_object(&mut self, depth: usize) -> Result<JsonNode, JsonParseError> {
        self.expect(b'{', "Expecting value")?;
        self.skip_ws();
        let mut values = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonNode::Object(values));
        }
        let mut trailing_comma_pos: Option<usize> = None;
        loop {
            if let Some(comma_pos) = trailing_comma_pos.take() {
                if self.peek() == Some(b'}') {
                    return Err(JsonParseError::new(
                        "Illegal trailing comma before end of object",
                        comma_pos,
                    ));
                }
            }
            if self.peek() != Some(b'"') {
                return Err(JsonParseError::new(
                    "Expecting property name enclosed in double quotes",
                    self.pos,
                ));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':', "Expecting ':' delimiter")?;
            self.skip_ws();
            if self.peek().is_none() {
                return Err(JsonParseError::new("Expecting value", self.pos));
            }
            let value = self.parse_value(depth)?;
            values.push((key, value));
            self.skip_ws();
            match self.next() {
                Some(b',') => {
                    trailing_comma_pos = Some(self.pos.saturating_sub(1));
                    self.skip_ws();
                    if self.peek().is_none() {
                        return Err(JsonParseError::new(
                            "Expecting property name enclosed in double quotes",
                            self.pos,
                        ));
                    }
                }
                Some(b'}') => break,
                Some(_) => {
                    return Err(JsonParseError::new(
                        "Expecting ',' delimiter",
                        self.pos.saturating_sub(1),
                    ));
                }
                None => return Err(JsonParseError::new("Expecting ',' delimiter", self.pos)),
            }
        }
        Ok(JsonNode::Object(values))
    }

    fn parse_number(&mut self) -> Result<JsonNode, JsonParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(JsonParseError::new("invalid JSON number", self.pos));
                }
            }
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(JsonParseError::new("invalid JSON number", self.pos)),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JsonParseError::new("invalid JSON number", self.pos));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JsonParseError::new("invalid JSON number", self.pos));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.source[start..self.pos])
            .map_err(|_| JsonParseError::new("invalid JSON number", self.pos))?;
        if text.contains('.') || text.contains('e') || text.contains('E') {
            let value = text
                .parse::<f64>()
                .map_err(|_| JsonParseError::new("invalid JSON number", self.pos))?;
            Ok(JsonNode::Float(value))
        } else if let Ok(value) = text.parse::<i64>() {
            Ok(JsonNode::Int(value))
        } else {
            let (negative, digits) = match text.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, text),
            };
            json_validate_int_digits_limit(digits)
                .map_err(|err| JsonParseError::new(err.message, self.pos))?;
            let mut value = BigInt::from_str_radix(digits, 10)
                .ok_or_else(|| JsonParseError::new("invalid JSON number", self.pos))?;
            if negative {
                value = value.negated();
            }
            Ok(JsonNode::BigInt(value))
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8, message: &'static str) -> Result<(), JsonParseError> {
        match self.peek() {
            Some(found) if found == byte => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(JsonParseError::new(message, self.pos)),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.pos += 1;
        Some(value)
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn parse_json_node(source: &str) -> Result<JsonNode, RuntimeError> {
    parse_json_node_with_limit(source, JSON_PARSE_NATIVE_STACK_SAFE_DEPTH)
}

fn parse_json_node_with_limit(source: &str, depth_limit: usize) -> Result<JsonNode, RuntimeError> {
    parse_json_node_with_limit_detail(source, depth_limit).map_err(json_runtime_error_from_parse)
}

#[cfg_attr(not(test), allow(dead_code))]
fn parse_json_node_from_index(source: &str, idx: usize) -> Result<(JsonNode, usize), RuntimeError> {
    parse_json_node_from_index_with_limit(source, idx, JSON_PARSE_NATIVE_STACK_SAFE_DEPTH)
}

fn parse_json_node_with_limit_detail(
    source: &str,
    depth_limit: usize,
) -> Result<JsonNode, JsonParseError> {
    JsonParser::new(source, depth_limit).parse()
}

fn parse_json_node_from_index_with_limit(
    source: &str,
    idx: usize,
    depth_limit: usize,
) -> Result<(JsonNode, usize), RuntimeError> {
    parse_json_node_from_index_with_limit_detail(source, idx, depth_limit)
        .map_err(json_runtime_error_from_parse)
}

fn parse_json_node_from_index_with_limit_detail(
    source: &str,
    idx: usize,
    depth_limit: usize,
) -> Result<(JsonNode, usize), JsonParseError> {
    if idx > source.len() {
        return Err(JsonParseError::new("Expecting value", source.len()));
    }
    let mut parser = JsonParser::new(source, depth_limit);
    parser.pos = idx;
    let node = parser.parse_value(0)?;
    Ok((node, parser.pos))
}

fn utf8_char_index_to_byte(source: &str, char_index: usize) -> Option<usize> {
    let char_len = source.chars().count();
    if char_index > char_len {
        return None;
    }
    if char_index == char_len {
        return Some(source.len());
    }
    source
        .char_indices()
        .nth(char_index)
        .map(|(byte_idx, _)| byte_idx)
}

fn utf8_byte_index_to_char(source: &str, byte_index: usize) -> Option<usize> {
    if byte_index > source.len() || !source.is_char_boundary(byte_index) {
        return None;
    }
    Some(source[..byte_index].chars().count())
}

fn json_scan_number_end_byte(source: &str, start_byte: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut pos = start_byte;
    if bytes.get(pos) == Some(&b'-') {
        pos += 1;
    }
    match bytes.get(pos) {
        Some(b'0') => {
            pos += 1;
            if matches!(bytes.get(pos), Some(b'0'..=b'9')) {
                return None;
            }
        }
        Some(b'1'..=b'9') => {
            pos += 1;
            while matches!(bytes.get(pos), Some(b'0'..=b'9')) {
                pos += 1;
            }
        }
        _ => return None,
    }
    if bytes.get(pos) == Some(&b'.') {
        pos += 1;
        if !matches!(bytes.get(pos), Some(b'0'..=b'9')) {
            return None;
        }
        while matches!(bytes.get(pos), Some(b'0'..=b'9')) {
            pos += 1;
        }
    }
    if matches!(bytes.get(pos), Some(b'e' | b'E')) {
        pos += 1;
        if matches!(bytes.get(pos), Some(b'+' | b'-')) {
            pos += 1;
        }
        if !matches!(bytes.get(pos), Some(b'0'..=b'9')) {
            return None;
        }
        while matches!(bytes.get(pos), Some(b'0'..=b'9')) {
            pos += 1;
        }
    }
    Some(pos)
}

fn json_string_has_unescaped_ascii_control_char(source: &str, start: usize, end: usize) -> bool {
    if start >= end || end > source.len() {
        return false;
    }
    let bytes = source.as_bytes();
    let mut idx = start.saturating_add(1);
    let content_end = end.saturating_sub(1);
    while idx < content_end {
        let byte = bytes[idx];
        if byte == b'\\' {
            idx += 1;
            if idx >= content_end {
                break;
            }
            if bytes[idx] == b'u' {
                idx = idx.saturating_add(4).min(content_end);
            }
        } else if byte < 0x20 {
            return true;
        }
        idx += 1;
    }
    false
}

fn json_node_to_value(node: JsonNode, heap: &Heap) -> Value {
    match node {
        JsonNode::Null => Value::None,
        JsonNode::Bool(value) => Value::Bool(value),
        JsonNode::Int(value) => Value::Int(value),
        JsonNode::BigInt(value) => Value::BigInt(Box::new(value)),
        JsonNode::Float(value) => Value::Float(value),
        JsonNode::String(value) => Value::Str(value),
        JsonNode::Array(values) => heap.alloc_list(
            values
                .into_iter()
                .map(|value| json_node_to_value(value, heap))
                .collect(),
        ),
        JsonNode::Object(entries) => heap.alloc_dict(
            entries
                .into_iter()
                .map(|(key, value)| (Value::Str(key), json_node_to_value(value, heap)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        JsonNode, json_escape_string, json_source_text, parse_json_node,
        parse_json_node_from_index, parse_json_separators,
    };
    use crate::runtime::{Heap, Object, Value};
    use crate::unicode::internal_char_from_codepoint;
    use crate::vm::Vm;
    use std::collections::HashMap;

    #[test]
    fn json_escape_string_ensure_ascii_uses_lowercase_hex() {
        let escaped = json_escape_string("A☺😊", true);
        assert_eq!(escaped, "\"A\\u263a\\ud83d\\ude0a\"");
    }

    #[test]
    fn json_escape_string_uses_short_control_escapes_for_backspace_and_formfeed() {
        let escaped = json_escape_string("\u{0008}\u{000C}", true);
        assert_eq!(escaped, "\"\\b\\f\"");
    }

    #[test]
    fn json_escape_string_del_escape_depends_on_ensure_ascii() {
        let text = "\u{007F}";
        assert_eq!(json_escape_string(text, true), "\"\\u007f\"");
        assert_eq!(json_escape_string(text, false), "\"\u{007F}\"");
    }

    #[test]
    fn json_source_text_accepts_utf8_bytes_and_bytearray() {
        let heap = Heap::new();
        let bytes = heap.alloc_bytes(br#"{"ok":1}"#.to_vec());
        let bytearray = heap.alloc_bytearray(br#"{"ok":2}"#.to_vec());

        assert_eq!(
            json_source_text(&bytes).expect("bytes should decode"),
            "{\"ok\":1}"
        );
        assert_eq!(
            json_source_text(&bytearray).expect("bytearray should decode"),
            "{\"ok\":2}"
        );
    }

    #[test]
    fn json_source_text_rejects_invalid_utf8_bytes() {
        let heap = Heap::new();
        let bytes = heap.alloc_bytes(vec![0xFF, 0xFE, 0x00]);

        let err = json_source_text(&bytes).expect_err("invalid bytes should fail");
        assert!(err.message.contains("valid UTF-8"));
    }

    #[test]
    fn parse_json_separators_requires_two_strings() {
        let ok = parse_json_separators(Value::Tuple(Heap::new().alloc(Object::Tuple(vec![
            Value::Str(",".to_string()),
            Value::Str(":".to_string()),
        ]))))
        .expect("tuple separators should parse");
        assert_eq!(ok, (",".to_string(), ":".to_string()));

        let err = parse_json_separators(Value::Tuple(
            Heap::new().alloc(Object::Tuple(vec![Value::Str(",".to_string())])),
        ))
        .expect_err("one element should fail");
        assert!(err.message.contains("2-item tuple"));

        let err = parse_json_separators(Value::Tuple(Heap::new().alloc(Object::Tuple(vec![
            Value::Int(1),
            Value::Str(":".to_string()),
        ]))))
        .expect_err("non-string separator should fail");
        assert!(err.message.contains("must be strings"));
    }

    #[test]
    fn parse_json_node_from_index_respects_offsets() {
        let source = "  [1,2] {\"a\":3}";
        let (array_node, array_end) = parse_json_node_from_index(source, 2).expect("array parse");
        let (object_node, object_end) =
            parse_json_node_from_index(source, 8).expect("object parse");

        match array_node {
            JsonNode::Array(values) => assert_eq!(values.len(), 2),
            other => panic!("expected array node, got {other:?}"),
        }
        assert_eq!(array_end, 7);

        match object_node {
            JsonNode::Object(values) => {
                assert_eq!(values.len(), 1);
                assert_eq!(values[0].0, "a");
            }
            other => panic!("expected object node, got {other:?}"),
        }
        assert_eq!(object_end, 15);
    }

    #[test]
    fn parse_json_node_decodes_surrogate_pair_escape() {
        let node = parse_json_node("\"\\ud83d\\ude0a\"").expect("surrogate pair should decode");
        match node {
            JsonNode::String(value) => assert_eq!(value, "😊"),
            other => panic!("expected string node, got {other:?}"),
        }
    }

    #[test]
    fn json_escape_string_allows_non_ascii_when_disabled() {
        let escaped = json_escape_string("A☺😊", false);
        assert_eq!(escaped, "\"A☺😊\"");
    }

    #[test]
    fn parse_json_node_rejects_invalid_escape_and_trailing_data() {
        let escape_err = parse_json_node("\"\\q\"").expect_err("invalid escape should fail");
        assert!(escape_err.message.contains("invalid JSON escape"));

        let trailing_err =
            parse_json_node("{\"a\": 1} tail").expect_err("trailing data should fail");
        assert!(trailing_err.message.contains("Extra data"));
    }

    #[test]
    fn parse_json_node_rejects_unescaped_control_chars_in_strings() {
        let err = parse_json_node("[\"\ttab\tcharacter\tin\tstring\t\"]")
            .expect_err("unescaped tab should fail");
        assert!(err.message.contains("invalid control character"));
    }

    #[test]
    fn parse_json_node_decodes_unpaired_surrogate_escapes_consistently() {
        let high_surrogate = internal_char_from_codepoint(0xD800).expect("high surrogate");
        let low_surrogate = internal_char_from_codepoint(0xDC00).expect("low surrogate");
        let high_only = parse_json_node("\"\\ud800\"").expect("lone high surrogate");
        assert!(
            matches!(high_only, JsonNode::String(value) if value == high_surrogate.to_string())
        );

        let low_only = parse_json_node("\"\\udc00\"").expect("lone low surrogate");
        assert!(matches!(low_only, JsonNode::String(value) if value == low_surrogate.to_string()));

        let high_then_non_low =
            parse_json_node("\"\\ud800\\u0041\"").expect("high surrogate followed by non-low");
        let expected_high_then_non_low = format!("{high_surrogate}A");
        assert!(
            matches!(high_then_non_low, JsonNode::String(value) if value == expected_high_then_non_low)
        );

        let invalid_following_escape =
            parse_json_node("\"\\ud800\\u00x1\"").expect_err("invalid trailing escape");
        assert!(
            invalid_following_escape
                .message
                .contains("invalid unicode escape")
        );
    }

    #[test]
    fn parse_json_node_enforces_number_grammar() {
        let valid_int = parse_json_node("123").expect("valid integer");
        assert!(matches!(valid_int, JsonNode::Int(123)));
        let valid_float = parse_json_node("-12.5e+2").expect("valid float with exponent");
        assert!(matches!(valid_float, JsonNode::Float(value) if (value + 1250.0).abs() < 1e-9));

        for invalid in ["01", "-01", "1.", "1e", "1e+", "-"] {
            let err = parse_json_node(invalid).expect_err("invalid number should fail");
            assert!(
                err.message.contains("invalid JSON number")
                    || err.message.contains("invalid JSON value"),
                "unexpected error for {invalid:?}: {}",
                err.message
            );
        }
    }

    #[test]
    fn json_source_text_rejects_non_text_input_types() {
        let err = json_source_text(&Value::Int(1)).expect_err("int input should fail");
        assert!(err.message.contains("expects str, bytes, or bytearray"));
    }

    #[test]
    fn builtin_json_dumps_enforces_nan_and_skipkeys_contracts() {
        let mut vm = Vm::new();

        let nan_err = vm
            .builtin_json_dumps(
                vec![Value::Float(f64::NAN)],
                HashMap::from([("allow_nan".to_string(), Value::Bool(false))]),
            )
            .expect_err("allow_nan=False should reject NaN");
        assert!(nan_err.message.contains("not JSON compliant: nan"));

        let bad_key_dict = vm.heap.alloc_dict(vec![(
            vm.heap.alloc_bytes(b"x".to_vec()),
            Value::Str("y".to_string()),
        )]);
        let key_err = vm
            .builtin_json_dumps(vec![bad_key_dict.clone()], HashMap::new())
            .expect_err("unsupported key should fail when skipkeys is false");
        assert!(
            key_err
                .message
                .contains("keys must be str, int, float, bool or None, not bytes")
        );

        let skipped = vm
            .builtin_json_dumps(
                vec![bad_key_dict],
                HashMap::from([("skipkeys".to_string(), Value::Bool(true))]),
            )
            .expect("skipkeys=True should drop unsupported keys");
        assert_eq!(skipped, Value::Str("{}".to_string()));

        let numeric_key = vm
            .builtin_json_dumps(
                vec![
                    vm.heap
                        .alloc_dict(vec![(Value::Int(1), Value::Str("x".to_string()))]),
                ],
                HashMap::new(),
            )
            .expect("int keys should be serialized as JSON object-string keys");
        assert_eq!(numeric_key, Value::Str("{\"1\": \"x\"}".to_string()));

        let nan_key_dict = vm
            .heap
            .alloc_dict(vec![(Value::Float(f64::NAN), Value::Int(1))]);
        let nan_key_err = vm
            .builtin_json_dumps(
                vec![nan_key_dict.clone()],
                HashMap::from([("allow_nan".to_string(), Value::Bool(false))]),
            )
            .expect_err("allow_nan=False should reject non-finite float keys");
        assert!(nan_key_err.message.contains("not JSON compliant: nan"));

        let nan_key_ok = vm
            .builtin_json_dumps(vec![nan_key_dict], HashMap::new())
            .expect("allow_nan=True should serialize non-finite float keys");
        assert_eq!(nan_key_ok, Value::Str("{\"NaN\": 1}".to_string()));
    }

    #[test]
    fn builtin_json_dumps_uses_custom_separators_and_sort_keys() {
        let mut vm = Vm::new();
        let dict = vm.heap.alloc_dict(vec![
            (Value::Str("b".to_string()), Value::Int(2)),
            (Value::Str("a".to_string()), Value::Int(1)),
        ]);
        let kwargs = HashMap::from([
            ("sort_keys".to_string(), Value::Bool(true)),
            (
                "separators".to_string(),
                vm.heap.alloc_tuple(vec![
                    Value::Str(";".to_string()),
                    Value::Str("=".to_string()),
                ]),
            ),
        ]);
        let dumped = vm
            .builtin_json_dumps(vec![dict], kwargs)
            .expect("json.dumps should succeed");
        assert_eq!(dumped, Value::Str("{\"a\"=1;\"b\"=2}".to_string()));
    }
}
